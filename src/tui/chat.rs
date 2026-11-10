//! Aba Chat: histórico da conversa e, embaixo, o painel de entrada e status
//! do agente. `Enter` envia a mensagem, e o status, a atividade, o modelo e o
//! consumo vêm da resposta.
//!
//! O chat fala com o agente só pelo contrato [`ChatAdapter`]: não sabe qual
//! provedor está por trás (hoje, o Claude CLI). Este módulo liga as partes:
//! o componente [`input`] (caixa de texto, comandos de `/` e as extensões,
//! como o provedor, o modelo e o status do agente) e o componente
//! [`history`] (conversa).

mod draw;
mod inbox;
mod model;
pub(super) mod model_menu;
mod permission;
mod record;
mod slash;
#[cfg(test)]
mod tests;
mod turn;

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Receiver;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use self::model_menu::ModelMenu;
use self::permission::Prompts;
use self::record::Recorder;
use super::history::{Entry, History};
use super::input::extensions::AgentStatus;
use super::input::{self, InputBox, InputMode, Outcome};
use super::widgets::badge;
use crate::agent::{self, AgentEvent, Catalog, ChatAdapter, PermissionMode, Session, UsageWindow};
use crate::db::history::Conversation;
use crate::extension::Contribution;
use crate::platform::clipboard;
use crate::settings::{AgentSettings, ContextSettings, StartMode, UsageSettings};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Acima disso (em caracteres), um texto colado vira um badge no input.
pub use super::input::PASTE_LIMIT;

/// Altura do painel: atividade, caixa de input e badges.
pub const HEIGHT: u16 = 1 + input::HEIGHT + badge::HEIGHT;

/// Linhas roladas por PageUp/PageDown e pela roda do mouse.
const PAGE_LINES: usize = 10;
const WHEEL_LINES: usize = 3;
/// Ticks que o aviso "Copiado" fica no rodapé (~2 s).
const NOTICE_TICKS: u32 = 20;
/// Limite de contexto enquanto o agente não informa o do modelo.
const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

pub struct Chat {
    /// Provedor do agente (route); o nome vira um botão no título.
    adapter: Option<Box<dyn ChatAdapter>>,
    /// Modelo informado pelo agente (ex.: `claude-opus-5-5`).
    model: Option<String>,
    /// Modelo escolhido na Config, enviado nas próximas mensagens.
    chosen_model: Option<String>,
    /// Esforço padrão (escolhido no Default do `/model`).
    default_effort: Option<String>,
    /// Modelo e esforço desta conversa, escolhidos no `/model`; valem no
    /// lugar dos padrões.
    session_model: Option<String>,
    session_effort: Option<String>,
    /// Modelo dos sub agentes, escolhido na Config.
    subagent_model: Option<String>,
    /// Menu do `/model`, quando aberto.
    model_menu: Option<ModelMenu>,
    /// Comando do `/login`, para a tela principal rodar fora da tela.
    pending_login: Option<Command>,
    /// O login pendente (ou em curso) é o de uma conta nova (`/new-login`).
    new_login: bool,
    /// A interface original do provedor com um comando que só existe nela
    /// (ex.: `/hooks`), para a tela principal rodar fora da tela.
    pending_original: Option<(Command, String)>,
    /// O `/usage` pediu a aba Consumo, para a tela principal abrir.
    open_usage: bool,
    /// Novo modelo padrão escolhido no `/model`, para a tela principal salvar.
    default_choice: Option<(String, Option<String>)>,
    /// Conta escolhida na Config (`None` é a Default do provedor).
    account: Option<String>,
    /// O que as extensões ativas no workspace acrescentam ao agente.
    extensions: Contribution,
    /// Sessão do agente, para continuar a conversa.
    session_id: Option<String>,
    /// Resposta em andamento.
    run: Option<Box<dyn Session>>,
    /// Pedidos de permissão do agente esperando o usuário.
    prompts: Prompts,
    /// As respostas aos pedidos das execuções despachadas (a execução, o id
    /// do pedido e se foi permitido), para a tela principal mandar ao hub.
    remote_answers: Vec<(String, String, bool)>,
    /// Ferramentas bloqueadas pelo modo que o usuário permitiu: vão
    /// liberadas na próxima mensagem (ver [`Chat::resume_allowed`]).
    allowed: Vec<String>,
    /// Modo pedido ao agente na resposta em andamento.
    requested_mode: Option<PermissionMode>,
    /// Modo em que o agente disse estar (no `init`) na última resposta.
    agent_mode: Option<PermissionMode>,
    history: History,
    /// Grava a conversa no histórico do workspace (`None` nas abas de uma
    /// execução despachada e nos testes).
    recorder: Option<Recorder>,
    input: InputBox,
    /// Aviso temporário no rodapé (ex.: "Copiado") e ticks restantes.
    notice: Option<(String, u32)>,
    agent_status: AgentStatus,
    /// O que o agente está fazendo; só ele preenche (vazio sem agente).
    agent_activity: Option<String>,
    permission_mode: PermissionMode,
    spinner_frame: usize,
    tokens_used: u64,
    /// Os tokens de cache gastos neste chat.
    cache_tokens_used: u64,
    /// O que o turno em andamento já somou nos dois acima (fora e de cache).
    turn_spent: (u64, u64),
    /// Tamanho do contexto da sessão, depois da primeira chamada ao modelo.
    context: Option<u64>,
    /// Limite de contexto do modelo, quando o agente informa.
    context_window: Option<u64>,
    /// Display da barra, auto compactação e ponto de compactar (Config).
    context_settings: ContextSettings,
    /// Display da barra de consumo (Config).
    usage_settings: UsageSettings,
    /// O consumo da sessão (a janela da barra de consumo), quando o provedor
    /// informa.
    session_usage: Option<UsageWindow>,
    /// A resposta em andamento é a de uma mensagem do usuário que ainda pode
    /// ser auto compactada: a auto compactação roda no máximo uma vez por
    /// mensagem enviada (nunca no fim de um `/compact`, mesmo se ele falhou).
    auto_compact_armed: bool,
    /// Comandos e modelos do agente, depois de carregados.
    catalog: Option<Catalog>,
    /// Carregamento do catálogo em andamento.
    catalog_loading: Option<Receiver<Result<Catalog, String>>>,
    /// O nome de agente do chat, dado no `/agente` (ver [`inbox`]).
    agent_name: Option<String>,
    /// As mensagens de outros agentes que chegaram ao chat.
    inbox: Vec<crate::orchestrator::message::Message>,
    /// Quantas da caixa o usuário ainda não leu.
    unread: usize,
    /// A aba de uma execução despachada: só mostra, não manda nada.
    read_only: bool,
    /// Na aba de uma execução, que não tem adapter: o nome do provedor e
    /// como ele mostra os modelos.
    viewer_provider: Option<ViewerProvider>,
    /// Na aba de uma execução: o modo informado pelo agente, mesmo sem
    /// equivalente no terra (ex.: `dontAsk`).
    viewer_mode: Option<String>,
}

/// O provedor de uma aba de execução: o nome e como ele mostra os modelos.
pub type ViewerProvider = (&'static str, fn(&str) -> String);

/// Sem agente (Desconectado). Use [`Chat::connect`] para usar o provedor
/// disponível.
impl Default for Chat {
    fn default() -> Self {
        Self {
            adapter: None,
            model: None,
            chosen_model: None,
            default_effort: None,
            session_model: None,
            session_effort: None,
            subagent_model: None,
            model_menu: None,
            pending_login: None,
            new_login: false,
            pending_original: None,
            open_usage: false,
            default_choice: None,
            account: None,
            extensions: Contribution::default(),
            session_id: None,
            run: None,
            prompts: Prompts::default(),
            remote_answers: Vec::new(),
            allowed: Vec::new(),
            requested_mode: None,
            agent_mode: None,
            history: History::default(),
            recorder: None,
            input: InputBox::default(),
            notice: None,
            agent_status: AgentStatus::default(),
            agent_activity: None,
            permission_mode: PermissionMode::default(),
            spinner_frame: 0,
            tokens_used: 0,
            cache_tokens_used: 0,
            turn_spent: (0, 0),
            context: None,
            context_window: None,
            context_settings: ContextSettings::default(),
            usage_settings: UsageSettings::default(),
            session_usage: None,
            auto_compact_armed: false,
            catalog: None,
            catalog_loading: None,
            agent_name: None,
            inbox: Vec::new(),
            unread: 0,
            read_only: false,
            viewer_provider: None,
            viewer_mode: None,
        }
    }
}

impl Chat {
    /// Chat ligado ao provedor escolhido na Config ou, sem escolha, ao
    /// primeiro disponível (ou Desconectado).
    pub fn connect(settings: &AgentSettings) -> Self {
        Self::with_adapter(
            agent::adapters::select(settings.provider.as_deref()),
            settings,
        )
    }

    /// Chat ligado a `adapter`, com as configurações da seção Agente.
    /// A aba de uma execução despachada: mostra o que `session` (a saída do
    /// `claude`) traz e não manda nada ao agente.
    pub fn viewer(session: Box<dyn Session>, title: String, provider: ViewerProvider) -> Self {
        let mut chat = Self {
            run: Some(session),
            read_only: true,
            viewer_provider: Some(provider),
            agent_status: AgentStatus::Pensando,
            agent_activity: Some("Rodando...".into()),
            ..Self::default()
        };
        chat.history.push(Entry::Info(title));
        chat
    }

    pub fn with_adapter(adapter: Box<dyn ChatAdapter>, settings: &AgentSettings) -> Self {
        let agent_status = if adapter.is_available() {
            AgentStatus::Ocioso
        } else {
            AgentStatus::Desconectado
        };
        let mut chat = Self {
            adapter: Some(adapter),
            agent_status,
            ..Self::default()
        };
        chat.apply_settings(settings, None);
        chat.load_catalog();
        chat
    }

    /// Aplica as configurações da seção Agente. O modo só muda quando o
    /// modo inicial mudou (ou no início, sem `previous`), para não desfazer
    /// um Shift+Tab; o modelo vale a partir da próxima mensagem.
    ///
    /// Trocar de conta começa uma conversa nova: a sessão anterior é da outra
    /// conta e não pode ser continuada com esta. O aviso só aparece quando
    /// havia uma conversa, para passar pelas contas não encher o histórico.
    pub fn apply_settings(&mut self, settings: &AgentSettings, previous: Option<&AgentSettings>) {
        let provider_changed = previous.is_some_and(|p| p.provider != settings.provider);
        if provider_changed {
            self.switch_provider(settings.provider.as_deref());
        }
        let account_changed =
            provider_changed || previous.is_some_and(|p| p.account != settings.account);
        self.account = settings.account.clone();
        if account_changed {
            // Cada conta (e cada provedor) tem os seus comandos e modelos.
            self.load_catalog();
        }
        if account_changed && self.session_id.is_some() {
            self.end_session();
            let message = if provider_changed {
                let name = self.adapter.as_ref().map_or("Nenhum", |a| a.name());
                format!("Provedor: {name}. A próxima mensagem começa uma conversa nova.")
            } else {
                let label = self.account_label();
                format!("Conta: {label}. A próxima mensagem começa uma conversa nova.")
            };
            self.history.push(Entry::Info(message));
        }
        let start_changed = previous.is_none_or(|p| p.start_mode != settings.start_mode);
        // "Do provedor" depende da conta: a configuração é de cada uma.
        let provider_mode = settings.start_mode == StartMode::Provider && account_changed;
        if start_changed || provider_mode {
            self.permission_mode = self.start_mode(settings.start_mode);
        }
        self.chosen_model = settings.model.clone();
        self.default_effort = settings.effort.clone();
        self.subagent_model = settings.subagent_model.clone();
        self.context_settings = settings.context;
        self.usage_settings = settings.usage;
    }

    /// Troca o provedor (o `id` de um adapter; `None`, o primeiro
    /// disponível). A resposta em andamento é do provedor anterior: para.
    fn switch_provider(&mut self, id: Option<&str>) {
        let adapter = agent::adapters::select(id);
        if self.run.take().is_some() {
            self.prompts.clear_asks();
            self.agent_activity = None;
        }
        self.agent_status = if adapter.is_available() {
            AgentStatus::Ocioso
        } else {
            AgentStatus::Desconectado
        };
        self.adapter = Some(adapter);
        self.session_model = None;
        self.session_effort = None;
        self.context_window = None;
        self.session_usage = None;
    }

    /// O `id` do provedor do chat (ex.: `claude`).
    pub fn provider_id(&self) -> Option<&str> {
        self.adapter.as_deref().map(|a| a.id())
    }

    /// O consumo da sessão (a janela do provedor que tem `session`) para a
    /// barra de consumo.
    pub fn set_session_usage(&mut self, window: Option<UsageWindow>) {
        self.session_usage = window;
    }

    /// Nome da conta escolhida, como o provedor a mostra.
    pub fn account_label(&self) -> String {
        let accounts = self
            .adapter
            .as_ref()
            .map(|a| a.accounts())
            .unwrap_or_default();
        accounts
            .into_iter()
            .find(|a| a.id == self.account)
            .map_or_else(
                || {
                    self.account
                        .clone()
                        .unwrap_or_else(|| "Default".to_string())
                },
                |a| a.label,
            )
    }

    /// O modelo padrão: o salvo na Config ou o do agente.
    pub fn default_model(&self) -> Option<String> {
        self.chosen_model
            .clone()
            .or_else(|| self.catalog.as_ref().and_then(|c| c.default_model.clone()))
    }

    /// O modo de um modo inicial. "Do provedor" segue a configuração dele,
    /// para os dois estarem em sincronia desde o início; o Yolo nunca vem do
    /// provedor, então lá o padrão é Agente.
    fn start_mode(&self, start: StartMode) -> PermissionMode {
        match start {
            StartMode::Fixed(mode) => mode,
            StartMode::Provider => std::env::current_dir()
                .ok()
                .zip(self.adapter.as_deref())
                .and_then(|(cwd, adapter)| adapter.configured_mode(&cwd, self.account.as_deref()))
                .filter(|mode| *mode != PermissionMode::Yolo)
                .unwrap_or_default(),
        }
    }

    /// O provedor do chat, para a Config mostrar e oferecer as opções dele.
    /// As extensões ativas mudaram: valem a partir da próxima mensagem.
    pub fn set_extensions(&mut self, extensions: Contribution) {
        self.extensions = extensions;
    }

    /// O que as extensões ativas acrescentam ao agente.
    /// Há uma conversa com o agente (o MCP mostra a sessão ativa).
    /// Os tokens gastos neste chat (entrada e saída, sem os de cache).
    /// O que o agente está fazendo agora (`None` com o chat parado).
    pub fn activity(&self) -> Option<&str> {
        self.agent_activity.as_deref()
    }

    pub fn tokens_used(&self) -> u64 {
        self.tokens_used
    }

    pub fn extensions(&self) -> &Contribution {
        &self.extensions
    }

    pub fn adapter(&self) -> Option<&dyn ChatAdapter> {
        self.adapter.as_deref()
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
        if is_copy_key(&key) {
            self.copy_selection();
            return true;
        }
        // O menu do `/model`, aberto, fica com todas as teclas.
        if let Some(menu) = &mut self.model_menu {
            let outcome = menu.handle_key(key, self.catalog.as_ref());
            self.apply_model_menu(outcome);
            return true;
        }
        // Um pedido de permissão aberto fica com as teclas, menos o Tab, que
        // continua trocando de aba.
        if self.prompts.is_open() {
            if key.code == KeyCode::Tab {
                return false;
            }
            if let Some((request, allow)) = self.prompts.handle_key(key) {
                self.answer(request, allow);
            }
            return true;
        }
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
            Outcome::Command(name) => {
                self.run_command(&name);
                true
            }
        }
    }

    /// Texto colado no terminal: vai para o input, menos com o menu do
    /// `/model` ou um pedido de permissão abertos.
    pub fn handle_paste(&mut self, text: &str) {
        if self.model_menu.is_none() && !self.prompts.is_open() {
            self.input.paste(text);
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
                self.input.clear_selection();
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
                let pressed = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
                let released = matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left));
                if self.input.handle_mouse(mouse) {
                    // Uma seleção de cada vez: começar uma no input desfaz a do
                    // histórico; soltar o botão copia o trecho, como no histórico.
                    if pressed {
                        self.history.clear_selection();
                    }
                    if released && let Some(text) = self.input.selected_text() {
                        self.copy(&text);
                    }
                }
            }
        }
    }

    /// Ctrl+Shift+C: copia o trecho selecionado no histórico ou no input.
    fn copy_selection(&mut self) {
        match self
            .history
            .selected_text()
            .or_else(|| self.input.selected_text())
        {
            Some(text) => self.copy(&text),
            None => {
                self.notice = Some((
                    "Selecione um trecho com o mouse para copiar".to_string(),
                    NOTICE_TICKS,
                ))
            }
        }
    }

    fn copy(&mut self, text: &str) {
        let message = if clipboard::copy(text) {
            format!("Copiado ({} caracteres)", text.chars().count())
        } else {
            "Não foi possível copiar".to_string()
        };
        self.notice = Some((message, NOTICE_TICKS));
    }

    /// Grava a conversa no SQLite `file`, o histórico do workspace, e traz de
    /// lá as últimas mensagens enviadas para o ↑.
    pub fn set_history_file(&mut self, file: PathBuf) {
        let mut recorder = Recorder::new(file);
        match recorder.sent() {
            Ok(sent) => self.input.set_sent(sent),
            Err(err) => self.history.push(Entry::Error(format!(
                "As últimas mensagens enviadas não foram carregadas: {err:#}"
            ))),
        }
        self.recorder = Some(recorder);
    }

    /// Grava o que falta da conversa, mesmo uma resposta pela metade.
    pub fn save_history(&mut self) {
        self.record(self.history.entries().len());
    }

    /// Grava as `complete` primeiras entradas do histórico. Se não der, avisa
    /// uma vez e para de gravar: a conversa continua na tela.
    fn record(&mut self, complete: usize) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let info = Conversation {
            provider: self.adapter.as_ref().map(|a| a.id()),
            session_id: self.session_id.as_deref(),
            model: self.model.as_deref(),
        };
        if let Err(err) = recorder.save(&self.history.entries()[..complete], info) {
            self.recording_failed(err);
        }
    }

    fn recording_failed(&mut self, err: anyhow::Error) {
        self.recorder = None;
        self.history.push(Entry::Error(format!(
            "O histórico de conversas não foi salvo: {err:#}"
        )));
    }

    /// Avança o spinner, o cursor piscante e os avisos, e lê o que o agente
    /// mandou; chamado a cada tick do loop.
    /// Um panic numa thread de fundo do provedor: vira um erro no histórico
    /// e encerra a resposta em andamento, que não vai mais terminar.
    pub fn provider_failed(&mut self, reason: &str) {
        let name = self.adapter.as_ref().map_or("O provedor", |a| a.name());
        let message = format!("{name} falhou: {reason}");
        if self.run.is_some() {
            self.apply(AgentEvent::Failed(message));
        } else {
            self.history.push(Entry::Error(message));
        }
    }

    /// Um aviso de fora do chat (ex.: o terra abriu sem o banco).
    pub fn warn(&mut self, message: String) {
        self.history.push(Entry::Error(message));
    }

    /// O agente `agent` da execução despachada `run` pede para usar a
    /// ferramenta `name`: o pedido entra na fila com os do agente deste
    /// chat, e a resposta sai em [`Chat::take_remote_answers`]. Um pedido
    /// que já está na fila (o hub manda de novo numa reconexão) não repete.
    pub fn remote_request(
        &mut self,
        run: String,
        id: String,
        agent: String,
        name: String,
        detail: String,
    ) {
        if self.prompts.has_remote(&run, &id) {
            return;
        }
        self.prompts.push(permission::Request {
            name,
            detail,
            kind: permission::Kind::Remote { run, id, agent },
        });
    }

    /// A execução `run` terminou: os pedidos dela sem resposta saem.
    pub fn run_finished(&mut self, run: &str) {
        self.prompts.clear_run(run);
    }

    /// As respostas aos pedidos das execuções desde a última vez, para o
    /// hub.
    pub fn take_remote_answers(&mut self) -> Vec<(String, String, bool)> {
        std::mem::take(&mut self.remote_answers)
    }

    pub fn tick(&mut self) {
        self.poll_catalog();
        let events = self
            .run
            .as_mut()
            .map(|session| session.poll())
            .unwrap_or_default();
        for event in events {
            self.apply(event);
        }
        // A resposta que ainda está chegando é gravada quando terminar.
        let entries = self.history.entries();
        let streaming = self.run.is_some() && matches!(entries.last(), Some(Entry::Assistant(_)));
        let complete = entries.len() - usize::from(streaming);
        self.record(complete);
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
}

/// Formata contagens de forma compacta: 950, 12.3K, 1M.
pub(crate) fn compact_count(n: u64) -> String {
    let (value, suffix) = match n {
        0..1_000 => return n.to_string(),
        1_000..1_000_000 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{suffix}")
}

/// Ctrl+Shift+C, o copiar dos terminais. Só chega aqui nos terminais que o
/// repassam (kitty, WezTerm, Windows Terminal...); no GNOME Terminal e no
/// Ptyxis ele é do próprio terminal.
fn is_copy_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c' | 'C'))
        && key
            .modifiers
            .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
}
