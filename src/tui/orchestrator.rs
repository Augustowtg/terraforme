//! Aba Orquestrador: o interruptor dos despachos, as sessões, o `[global]` do
//! `~/.terraforme/orchestrator.toml`, editável, e os custos de hoje.
//!
//! ```text
//!  Orquestrador
//!   Despachos               ● ativos (p pausa todos)
//!
//!  Sessões
//!   ▾ Studio · 2 instâncias · 1 sub agente
//!       ● Studio-4242              Principal, api               rodando
//!       ● Studio-4300              Principal                    parado
//!   ▸ terraforme-cli · 1 instância
//!
//!  Configuração
//!   Modo de execução        Demonstrativo (visível nas abas)
//!   Tempo limite            30 min
//!   …
//!
//!  Custos de hoje
//! ```
//!
//! Um workspace das Sessões fica fechado até o `Enter` (ou `→`) abri-lo.
//! `↑`/`↓` passam pelas linhas escolhíveis; `←`/`→` mudam o valor (números,
//! escolhas, ligado/desligado); `Enter` edita o número num prompt no pé da
//! aba; `p` pausa ou retoma todos os despachos.
//!
//! Os agentes, as regras e os modelos não aparecem aqui: ficam no arquivo,
//! editados à mão. As execuções e as mensagens continuam gravadas em
//! `runs/` e `messages/`, sem aparecer na aba.
//!
//! Toda mudança passa pelo [`Editor`]: ela mantém os comentários do arquivo
//! e só é gravada se o arquivo continuar válido; senão, o erro aparece no pé
//! e nada muda. O hub relê o arquivo a cada segundo. Um arquivo inválido
//! (editado à mão) aparece no topo, e a aba não o edita até ele ser corrigido.

mod rows;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

pub use self::rows::Hub as HubView;
use self::rows::{Action, GlobalKey};
use crate::hub::protocol::Instance;
use crate::orchestrator::config::{Config, INFINITE, RunMode};
use crate::orchestrator::edit::{self, Editor};
use crate::orchestrator::run::{self, Run};
use crate::theme;

pub const HELP: &str =
    "↑/↓ escolher · ←/→ mudar · Enter editar · p pausar · Tab trocar aba · Esc ir para Sair";
const HELP_PROMPT: &str = "Enter salvar · Esc cancelar";
/// De quanto em quanto tempo a aba relê o arquivo e as execuções.
const REFRESH: Duration = Duration::from_secs(1);
/// O que o `PageUp`/`PageDown` anda.
const PAGE: usize = 10;

/// O que a aba mostra, lido do disco.
pub struct Data {
    /// A pasta do terra (`None` sem ela, como nos testes da tela).
    pub home: Option<PathBuf>,
    pub config: Result<Config, String>,
    /// Da mais antiga à mais nova.
    pub runs: Vec<Run>,
    loaded: Option<Instant>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            home: None,
            config: Ok(Config::default()),
            runs: Vec::new(),
            loaded: None,
        }
    }
}

impl Data {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            ..Self::default()
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.home.as_deref().map(Config::path)
    }

    pub fn reload(&mut self) {
        self.loaded = Some(Instant::now());
        let Some(home) = &self.home else {
            return;
        };
        self.config = Config::load(&Config::path(home));
        self.runs = run::list(home);
    }

    pub fn refresh_if_stale(&mut self) {
        if self.loaded.is_none_or(|t| t.elapsed() >= REFRESH) {
            self.reload();
        }
    }
}

/// O prompt do pé da aba: edita uma chave do `[global]`.
struct Prompt {
    label: String,
    text: String,
    key: GlobalKey,
}

#[derive(Default)]
pub struct OrchestratorView {
    pub data: Data,
    /// A linha escolhida, entre as escolhíveis.
    selected: usize,
    /// A primeira linha visível.
    scroll: Cell<usize>,
    prompt: Option<Prompt>,
    /// O aviso do pé: o texto e se é um erro.
    notice: Option<(String, bool)>,
    /// Os workspaces abertos na seção Sessões.
    expanded: Vec<String>,
    /// As instâncias do último desenho: as linhas dos workspaces são
    /// escolhíveis, e as teclas chegam sem o hub.
    hub: RefCell<Option<(bool, Vec<Instance>)>>,
}

impl OrchestratorView {
    pub fn new(data: Data) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }

    pub fn help(&self) -> &'static str {
        if self.prompt.is_some() {
            HELP_PROMPT
        } else {
            HELP
        }
    }

    /// As ações das linhas escolhíveis, na ordem da tela.
    fn actions(&self) -> Vec<Action> {
        let snapshot = self.hub.borrow();
        let hub = HubView {
            instances: snapshot.as_ref(),
            own: None,
        };
        rows::build(&self.data, &hub, &self.expanded, None)
            .into_iter()
            .filter_map(|row| row.action)
            .collect()
    }

    fn current(&self) -> Option<Action> {
        let actions = self.actions();
        actions
            .get(self.selected.min(actions.len().saturating_sub(1)))
            .cloned()
    }

    /// Trata a tecla; `false` se ela não é da aba.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.prompt.is_some() {
            self.prompt_key(key);
            return true;
        }
        let count = self.actions().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(PAGE),
            KeyCode::PageDown => {
                self.selected = (self.selected + PAGE).min(count.saturating_sub(1))
            }
            KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Left | KeyCode::Char('h') => self.change(-1),
            KeyCode::Right | KeyCode::Char('l') => self.change(1),
            KeyCode::Enter => return self.activate(),
            _ => return false,
        }
        true
    }

    fn config(&self) -> Option<&Config> {
        self.data.config.as_ref().ok()
    }

    /// Muda o arquivo com `change` e grava, se ele continuar válido.
    fn edit(&mut self, change: impl FnOnce(&mut Editor)) {
        let Some(path) = self.data.path() else {
            self.notice = Some(("Sem a pasta do terra.".into(), true));
            return;
        };
        let result = Editor::open(&path).and_then(|mut editor| {
            change(&mut editor);
            editor.save()
        });
        self.notice = Some(match result {
            Ok(_) => ("Salvo no orchestrator.toml.".into(), false),
            Err(err) => (format!("Não salvo: {err}"), true),
        });
        self.data.reload();
    }

    fn toggle_pause(&mut self) {
        let Some(paused) = self.config().map(|c| c.global.paused) else {
            return;
        };
        self.edit(|e| e.set_global("pausado", !paused));
    }

    /// `←`/`→`: o próximo valor (`step` é -1 ou 1).
    fn change(&mut self, step: i64) {
        let (Some(action), Some(config)) = (self.current(), self.config().cloned()) else {
            return;
        };
        match action {
            Action::Workspace(name) => {
                let open = self.expanded.contains(&name);
                if step > 0 && !open {
                    self.expanded.push(name);
                } else if step < 0 && open {
                    self.expanded.retain(|w| *w != name);
                }
            }
            Action::Pause => self.toggle_pause(),
            Action::Global(GlobalKey::RunMode) => {
                let next = match config.global.run_mode {
                    RunMode::Tab => "segundo_plano",
                    RunMode::Background => "aba",
                };
                self.edit(|e| e.set_global("modo_execucao", next));
            }
            Action::Global(GlobalKey::FocusRuns) => {
                let focus = config.global.focus_runs;
                self.edit(|e| e.set_global("aba_em_primeiro_plano", !focus));
            }
            Action::Global(GlobalKey::Timeout) => {
                // Abaixo de 1 minuto, infinito; do infinito, 1 minuto.
                let next = match config.global.timeout_min {
                    None if step > 0 => Some(1),
                    None => None,
                    Some(n) => (i64::from(n) + step >= 1).then(|| (i64::from(n) + step) as u32),
                };
                self.edit(|e| match next {
                    Some(n) => e.set_global(GlobalKey::Timeout.key(), edit::number(n)),
                    None => e.set_global(GlobalKey::Timeout.key(), INFINITE),
                });
            }
            Action::Global(key) => {
                let n = key.number(&config).unwrap_or(1);
                let next = (i64::from(n) + step).clamp(1, i64::from(u32::MAX)) as u32;
                self.edit(|e| e.set_global(key.key(), edit::number(next)));
            }
        }
    }

    /// `Enter`: abre um workspace, abre o prompt do número ou passa para o
    /// próximo valor.
    fn activate(&mut self) -> bool {
        let Some(action) = self.current() else {
            return false;
        };
        let config = self.config().cloned().unwrap_or_default();
        let (label, text, key) = match action {
            Action::Workspace(name) => {
                if self.expanded.contains(&name) {
                    self.expanded.retain(|w| *w != name);
                } else {
                    self.expanded.push(name);
                }
                return true;
            }
            Action::Global(GlobalKey::Timeout) => (
                format!("Tempo limite (minutos ou {INFINITE})"),
                config
                    .global
                    .timeout_min
                    .map_or(INFINITE.to_string(), |n| n.to_string()),
                GlobalKey::Timeout,
            ),
            Action::Global(key) if key.number(&config).is_some() => {
                let n = key.number(&config).unwrap_or_default();
                (key_label(key.key()), n.to_string(), key)
            }
            // O resto liga/desliga ou passa pelas escolhas.
            _ => {
                self.change(1);
                return true;
            }
        };
        self.prompt = Some(Prompt { label, text, key });
        true
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                prompt.text.pop();
            }
            KeyCode::Char(c) => prompt.text.push(c),
            KeyCode::Enter => {
                if let Some(prompt) = self.prompt.take() {
                    self.submit(prompt);
                }
            }
            _ => {}
        }
    }

    fn submit(&mut self, prompt: Prompt) {
        let text = prompt.text.trim();
        let key = prompt.key;
        if key == GlobalKey::Timeout && (text.is_empty() || text.eq_ignore_ascii_case(INFINITE)) {
            self.edit(|e| e.set_global(key.key(), INFINITE));
            return;
        }
        match positive(text) {
            Ok(n) => self.edit(|e| e.set_global(key.key(), edit::number(n))),
            Err(err) => self.notice = Some((err, true)),
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, hub: &HubView) {
        let [content, foot] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        *self.hub.borrow_mut() = hub.instances.cloned();
        let selected = self.current();
        let rows = rows::build(&self.data, hub, &self.expanded, selected.as_ref());
        let target = rows
            .iter()
            .position(|row| row.action.is_some() && row.action == selected)
            .unwrap_or(0);
        // A linha escolhida sempre à vista.
        let height = usize::from(content.height.saturating_sub(1)).max(1);
        let mut scroll = self.scroll.get();
        if target < scroll {
            scroll = target.saturating_sub(1);
        } else if target + 3 >= scroll + height {
            scroll = (target + 4).saturating_sub(height);
        }
        self.scroll.set(scroll);
        let lines: Vec<Line> = rows.into_iter().skip(scroll).map(|row| row.line).collect();
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            content,
        );
        let foot_line = match (&self.prompt, &self.notice) {
            (Some(prompt), _) => Line::from(vec![
                Span::styled(format!(" {}: ", prompt.label), theme::section_title()),
                Span::styled(format!("{}▏", prompt.text), theme::text()),
            ]),
            (None, Some((text, true))) => Line::styled(format!(" {text}"), theme::error()),
            (None, Some((text, false))) => Line::styled(format!(" {text}"), theme::muted()),
            (None, None) => Line::default(),
        };
        frame.render_widget(Paragraph::new(foot_line), foot);
    }
}

/// O nome de uma chave do `[global]` para o prompt.
fn key_label(key: &str) -> String {
    format!("{key} (número maior que zero)")
}

fn positive(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("\"{text}\" não é um número maior que zero."))
}

#[cfg(test)]
mod tests;
