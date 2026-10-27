//! Aba Chat: histórico da conversa e, embaixo, o painel de entrada e status
//! do agente. `Enter` envia a mensagem, e o status, a atividade, o modelo e o
//! consumo vêm da resposta.
//!
//! O chat fala com o agente só pelo contrato [`ChatAdapter`]: não sabe qual
//! provedor está por trás (hoje, o Claude CLI). Este módulo liga as partes:
//! [`input`] (caixa de texto), [`history`] (conversa) e [`mode`] (como status
//! e modo aparecem).

mod history;
mod input;
mod mention;
mod mode;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use self::history::{Entry, History};
use self::input::{InputBox, InputMode, Outcome};
use self::mode::{AgentStatus, mode_variant};
use super::widgets::badge;
use crate::agent::{self, AgentEvent, ChatAdapter, PermissionMode, ReportedMode, Request, Session};
use crate::platform::clipboard;
use crate::theme::{self, Variant};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Altura do painel: atividade, caixa de input e badges.
pub const HEIGHT: u16 = 1 + input::HEIGHT + badge::HEIGHT;

/// Linhas roladas por PageUp/PageDown e pela roda do mouse.
const PAGE_LINES: usize = 10;
const WHEEL_LINES: usize = 3;
/// Ticks que o aviso "Copiado" fica no rodapé (~2 s).
const NOTICE_TICKS: u32 = 20;

const HELP_NORMAL: &str =
    "i escrever · Shift+Tab modo · PgUp/PgDn rolar · Tab trocar aba · Esc ir para Sair";
const HELP_RUNNING: &str = "Esc interromper · i escrever · Shift+Tab modo · PgUp/PgDn rolar";
const HELP_INSERT: &str =
    "Enter enviar · Esc parar · Shift+Enter ou \\+Enter nova linha · @ arquivo";
const HELP_PICKER: &str = "↑/↓ escolher · Tab/Enter inserir · Esc fechar · digite para filtrar";

pub struct Chat {
    /// Provedor do agente (route); o nome vira um botão no título.
    adapter: Option<Box<dyn ChatAdapter>>,
    /// Modelo informado pelo agente (ex.: `claude-opus-5-5`).
    model: Option<String>,
    /// Sessão do agente, para continuar a conversa.
    session_id: Option<String>,
    /// Resposta em andamento.
    run: Option<Box<dyn Session>>,
    /// Modo pedido ao agente na resposta em andamento.
    requested_mode: Option<PermissionMode>,
    /// Modo em que o agente disse estar (no `init`) na última resposta.
    agent_mode: Option<PermissionMode>,
    history: History,
    input: InputBox,
    /// Aviso temporário no rodapé (ex.: "Copiado") e ticks restantes.
    notice: Option<(String, u32)>,
    agent_status: AgentStatus,
    /// O que o agente está fazendo; só ele preenche (vazio sem agente).
    agent_activity: Option<String>,
    permission_mode: PermissionMode,
    spinner_frame: usize,
    tokens_used: u64,
}

/// Sem agente (Desconectado). Use [`Chat::connect`] para usar o provedor
/// disponível.
impl Default for Chat {
    fn default() -> Self {
        Self {
            adapter: None,
            model: None,
            session_id: None,
            run: None,
            requested_mode: None,
            agent_mode: None,
            history: History::default(),
            input: InputBox::default(),
            notice: None,
            agent_status: AgentStatus::default(),
            agent_activity: None,
            permission_mode: PermissionMode::default(),
            spinner_frame: 0,
            tokens_used: 0,
        }
    }
}

impl Chat {
    /// Chat ligado ao primeiro provedor disponível (ou Desconectado).
    pub fn connect() -> Self {
        Self::with_adapter(agent::adapters::default())
    }

    /// Chat ligado a `adapter`. Começa no modo padrão configurado no
    /// provedor, para os dois estarem em sincronia desde o início; o Yolo
    /// nunca vem do provedor, então lá o padrão é Agente.
    pub fn with_adapter(adapter: Box<dyn ChatAdapter>) -> Self {
        let agent_status = if adapter.is_available() {
            AgentStatus::Ocioso
        } else {
            AgentStatus::Desconectado
        };
        let permission_mode = std::env::current_dir()
            .ok()
            .and_then(|cwd| adapter.configured_mode(&cwd))
            .filter(|mode| *mode != PermissionMode::Yolo)
            .unwrap_or_default();
        Self {
            adapter: Some(adapter),
            agent_status,
            permission_mode,
            ..Self::default()
        }
    }

    /// O provedor, se estiver pronto para receber mensagens.
    fn available_adapter(&self) -> Option<&dyn ChatAdapter> {
        self.adapter.as_deref().filter(|a| a.is_available())
    }

    fn unavailable_hint(&self) -> String {
        self.adapter
            .as_ref()
            .map_or("Nenhum agente configurado.".to_string(), |a| {
                a.unavailable_hint()
            })
    }

    /// Trata uma tecla. Retorna `true` se ela foi consumida pelo painel; as
    /// outras (Tab, Esc no modo NORMAL...) ficam para a tela principal.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::BackTab => {
                self.permission_mode = self.permission_mode.next();
                return true;
            }
            KeyCode::PageUp => {
                self.history.scroll_up(PAGE_LINES);
                return true;
            }
            KeyCode::PageDown => {
                self.history.scroll_down(PAGE_LINES);
                return true;
            }
            // No modo NORMAL, com o agente respondendo, o Esc interrompe (em
            // vez de ir para "Sair").
            KeyCode::Esc if self.input.mode() == InputMode::Normal && self.run.is_some() => {
                self.cancel();
                return true;
            }
            _ => {}
        }
        match self.input.handle_key(key) {
            Outcome::Ignored => false,
            Outcome::Handled => true,
            Outcome::Submit => {
                self.send();
                true
            }
        }
    }

    /// A roda rola a conversa em qualquer ponto da aba; arrastar no
    /// histórico seleciona e copia; no input, posiciona o cursor e seleciona.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let (column, row) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollUp => self.history.scroll_up(WHEEL_LINES),
            MouseEventKind::ScrollDown => self.history.scroll_down(WHEEL_LINES),
            MouseEventKind::Down(MouseButton::Left) if self.history.contains(column, row) => {
                if let Some(text) = self.history.mouse_down(column, row) {
                    self.copy(&text);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.history.is_selecting() => {
                self.history.mouse_drag(column, row);
            }
            MouseEventKind::Up(MouseButton::Left) if self.history.is_selecting() => {
                if let Some(text) = self.history.mouse_up(column, row) {
                    self.copy(&text);
                }
            }
            _ => {
                self.input.handle_mouse(mouse);
            }
        }
    }

    /// Envia o texto do input ao agente. Ignora texto vazio e, enquanto o
    /// agente ainda responde, mantém o texto para enviar depois.
    fn send(&mut self) {
        let prompt = self.input.text().trim().to_string();
        if prompt.is_empty() || self.run.is_some() {
            return;
        }
        let Some(adapter) = self.available_adapter() else {
            self.history.push(Entry::Error(self.unavailable_hint()));
            return;
        };
        let started = adapter.start(Request {
            prompt: &prompt,
            mode: self.permission_mode,
            session_id: self.session_id.as_deref(),
        });
        match started {
            Ok(run) => {
                self.run = Some(run);
                self.requested_mode = Some(self.permission_mode);
                self.history.push(Entry::User(prompt));
                self.input.clear();
                self.agent_status = AgentStatus::Pensando;
                self.agent_activity = Some("Pensando...".to_string());
            }
            Err(err) => {
                self.history.push(Entry::Error(err));
                self.agent_status = AgentStatus::Erro;
            }
        }
        self.history.follow();
    }

    /// Interrompe a resposta em andamento (soltar a sessão a encerra).
    fn cancel(&mut self) {
        self.run = None;
        self.history.push(Entry::Info("Interrompido.".to_string()));
        self.agent_status = AgentStatus::Ocioso;
        self.agent_activity = None;
    }

    /// Aplica um evento do agente ao histórico e ao status.
    fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Init {
                session_id,
                model,
                permission_mode,
            } => {
                self.session_id = Some(session_id);
                self.model = Some(model).filter(|m| !m.is_empty());
                self.sync_mode(permission_mode);
            }
            AgentEvent::TextStart => {
                self.history.start_text();
                self.agent_activity = Some("Respondendo...".to_string());
            }
            AgentEvent::TextDelta(text) => self.history.append_text(&text),
            AgentEvent::Text(text) => self.history.push(Entry::Assistant(text)),
            AgentEvent::ToolUse { name, detail } => {
                self.agent_activity = Some(format!("Usando {name}..."));
                self.history.push(Entry::Tool { name, detail });
            }
            AgentEvent::ToolResult { is_error, summary } => {
                self.agent_activity = Some("Pensando...".to_string());
                self.history.push(Entry::ToolResult { is_error, summary });
            }
            AgentEvent::Done {
                is_error,
                message,
                tokens,
                denied,
            } => {
                self.tokens_used += tokens;
                if denied > 0 {
                    // TODO: aprovar ferramentas pela interface no modo Manual.
                    self.history.push(Entry::Info(format!(
                        "{denied} ferramenta(s) bloqueada(s) pelo modo de permissão {}.",
                        self.permission_mode.label()
                    )));
                }
                if let Some(message) = message {
                    self.history.push(Entry::Error(message));
                }
                self.finish(is_error);
            }
            AgentEvent::Failed(message) => {
                self.history.push(Entry::Error(message));
                self.finish(true);
            }
        }
    }

    /// Sincroniza com o modo em que o agente disse estar. Se ele não for o
    /// pedido (o provedor recusou ou trocou), o terminal passa a mostrar o
    /// modo real; uma troca feita com Shift+Tab durante a resposta é mantida
    /// para a próxima mensagem.
    /// O Yolo não é sincronizado: o agente roda pedindo permissão e o terra
    /// segue em Yolo, aprovando tudo.
    fn sync_mode(&mut self, reported: ReportedMode) {
        if self.requested_mode == Some(PermissionMode::Yolo) {
            self.agent_mode = Some(PermissionMode::Yolo);
            return;
        }
        let mode = match reported {
            ReportedMode::Known(mode) if mode != PermissionMode::Yolo => mode,
            ReportedMode::Unknown(name) => {
                self.history.push(Entry::Info(format!(
                    "O agente está no modo \"{name}\", que este terminal não conhece."
                )));
                self.agent_mode = None;
                return;
            }
            _ => {
                self.agent_mode = None;
                return;
            }
        };
        self.agent_mode = Some(mode);
        if self
            .requested_mode
            .is_some_and(|requested| requested != mode)
        {
            self.permission_mode = mode;
            self.history.push(Entry::Info(format!(
                "O agente está no modo {}.",
                mode.label()
            )));
        }
    }

    fn finish(&mut self, is_error: bool) {
        self.run = None;
        self.agent_activity = None;
        self.agent_status = if is_error {
            AgentStatus::Erro
        } else {
            AgentStatus::Ocioso
        };
    }

    fn copy(&mut self, text: &str) {
        let message = if clipboard::copy(text) {
            format!("Copiado ({} caracteres)", text.chars().count())
        } else {
            "Não foi possível copiar".to_string()
        };
        self.notice = Some((message, NOTICE_TICKS));
    }

    /// Avança o spinner, o cursor piscante e os avisos, e lê o que o agente
    /// mandou; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        let events = self
            .run
            .as_mut()
            .map(|session| session.poll())
            .unwrap_or_default();
        for event in events {
            self.apply(event);
        }
        self.history.tick();
        self.input.tick();
        if let Some((_, ticks)) = &mut self.notice {
            *ticks -= 1;
            if *ticks == 0 {
                self.notice = None;
            }
        }
        if self.agent_activity.is_some() {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER.len();
        }
    }

    pub fn set_caps_lock(&mut self, on: bool) {
        self.input.set_caps_lock(on);
    }

    /// Texto do rodapé: o aviso do momento ou as teclas do modo atual.
    pub fn help(&self) -> Cow<'static, str> {
        if let Some((message, _)) = &self.notice {
            return Cow::Owned(message.clone());
        }
        Cow::Borrowed(match self.input.mode() {
            _ if self.input.picker_open() => HELP_PICKER,
            InputMode::Normal if self.run.is_some() => HELP_RUNNING,
            InputMode::Normal => HELP_NORMAL,
            InputMode::Insert => HELP_INSERT,
        })
    }

    /// Histórico da conversa (acima do painel).
    pub fn draw_history(&self, frame: &mut Frame, area: Rect) {
        let hint = match self.available_adapter() {
            Some(adapter) => format!(
                "Converse com o {}: i para escrever, Enter para enviar.",
                adapter.name()
            ),
            None => self.unavailable_hint(),
        };
        let empty = vec![
            Line::styled("Chat", theme::section_title()),
            Line::styled(hint, theme::muted()),
        ];
        self.history.draw(frame, area, empty);
    }

    /// Altura do painel: sem badges para mostrar, a linha deles some e o
    /// espaço fica para a conversa.
    pub fn height(&self) -> u16 {
        if self.badges().is_empty() {
            HEIGHT - badge::HEIGHT
        } else {
            HEIGHT
        }
    }

    /// Badges visíveis: o modo fora do padrão (Agente) ou com o agente ativo
    /// (respondendo ou com sessão iniciada), e o consumo depois do primeiro
    /// token.
    fn badges(&self) -> Vec<(String, Variant)> {
        let mode = self.permission_mode;
        let active = self.run.is_some() || self.agent_mode.is_some();
        let mut badges = Vec::new();
        if mode != PermissionMode::default() || active {
            // Trocado com Shift+Tab durante a resposta: o agente segue no modo
            // dele até a próxima mensagem.
            let label = match self.agent_mode {
                Some(current) if self.run.is_some() && current != mode => {
                    format!("Modo: {} → {}", current.label(), mode.label())
                }
                _ => format!("Modo: {}", mode.label()),
            };
            badges.push((label, mode_variant(mode)));
        }
        if self.tokens_used > 0 {
            badges.push((
                format!("Consumo: {} Tokens", compact_count(self.tokens_used)),
                Variant::Secondary,
            ));
        }
        badges
    }

    /// Painel: atividade do agente, caixa de input e badges.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        // Os badges ficam com o que sobra: sem eles, o painel encolhe (veja
        // `height`) e o input mantém a altura inteira.
        let [activity, input, badges] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(input::HEIGHT),
            Constraint::Fill(1),
        ])
        .areas(area);

        if let Some(text) = &self.agent_activity {
            let line = format!(" {} {text}", SPINNER[self.spinner_frame]);
            frame.render_widget(
                Paragraph::new(Span::styled(line, theme::activity())),
                activity,
            );
        }
        let status = self.agent_status;
        let status = Line::styled(
            format!(" {} ", status.label()),
            theme::variant(status.variant()),
        );
        self.input.draw(frame, input, self.model_title(), status);
        self.draw_badges(frame, badges);
    }

    /// "[Router] Modelo - Versão", com o router no estilo de botão (pill).
    fn model_title(&self) -> Line<'static> {
        // TODO: o botão do router vai abrir a troca de provedor (adapters::all).
        let router = self.adapter.as_ref().map_or("Sem agente", |a| a.name());
        let model = match (&self.model, &self.adapter) {
            (Some(id), Some(adapter)) => adapter.model_label(id),
            _ => "Modelo padrão".to_string(),
        };
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!(" {router} "), theme::badge_text(Variant::Active)),
            Span::styled(format!(" {model} "), theme::text()),
        ])
    }

    fn draw_badges(&self, frame: &mut Frame, area: Rect) {
        let badges = self.badges();
        let areas = Layout::horizontal(
            badges
                .iter()
                .map(|(label, _)| Constraint::Length(badge::width(label))),
        )
        .spacing(1)
        .flex(Flex::Start)
        .split(area);
        for ((label, variant), area) in badges.iter().zip(areas.iter()) {
            frame.render_widget(badge::badge(label, *variant), *area);
        }
    }
}

/// Formata contagens de forma compacta: 950, 12.3K, 1M.
fn compact_count(n: u64) -> String {
    let (value, suffix) = match n {
        0..1_000 => return n.to_string(),
        1_000..1_000_000 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{suffix}")
}
