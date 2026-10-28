//! Aba Orquestrador: as configurações dos chats delegados (os sub agentes)
//! — o provedor, o modelo, o modo e como eles começam e terminam.
//!
//! Por enquanto, elas só ficam salvas: o terra ainda não delega chats.
//!
//! ```text
//!  Orquestrador
//!
//!  Configurações
//!    Provedor          Claude
//!    Modelo            Default (Opus - 5.5)
//!    Modo dos chats    Accept Edits
//!    Ao terminar       Fechar o chat
//!    Ao abrir          Em segundo plano
//! ```
//!
//! `↑`/`↓` passam pelas configurações; `←`/`→` (ou `Enter`) mudam a
//! escolhida. No Provedor, `←`/`→` passam pelo do workspace e pelos
//! provedores conhecidos. No Modelo, `Enter` abre o mesmo menu do `/model`,
//! com o esforço, com o catálogo do provedor escolhido (o do workspace ou,
//! com outro, o que o terra carrega para ele: [`Delegated`]).

use std::sync::mpsc::{Receiver, TryRecvError};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use super::chat::model_menu::{self, ModelMenu, effort_label};
use crate::agent::adapters::{self, ProviderOption};
use crate::agent::{Catalog, ChatAdapter, ModelInfo};
use crate::settings::{OrchestratorSettings, StartMode};
use crate::theme::{self};

pub const HELP: &str = "↑/↓ escolher · ←/→ mudar · Tab trocar aba · Esc ir para Sair";
/// Com o menu do modelo aberto.
const HELP_MODEL: &str = "↑/↓ escolher · ←/→ esforço · Enter selecionar · Esc fechar";
/// Largura da coluna dos nomes das configurações.
const LABEL_WIDTH: usize = 18;

/// As configurações do orquestrador, na ordem da tela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    /// O provedor dos chats delegados: o do workspace ou outro.
    Provider,
    /// O modelo e o esforço dos chats delegados.
    Model,
    /// O modo com que os chats delegados começam.
    SpawnMode,
    /// Fechar o chat delegado quando ele termina a tarefa.
    CloseFinished,
    /// Mostrar o chat delegado ao abrir.
    FocusDelegated,
}

const SETTINGS: [Setting; 5] = [
    Setting::Provider,
    Setting::Model,
    Setting::SpawnMode,
    Setting::CloseFinished,
    Setting::FocusDelegated,
];

impl Setting {
    fn label(self) -> &'static str {
        match self {
            Setting::Provider => "Provedor",
            Setting::Model => "Modelo",
            Setting::SpawnMode => "Modo dos chats",
            Setting::CloseFinished => "Ao terminar",
            Setting::FocusDelegated => "Ao abrir",
        }
    }

    fn value(self, settings: &OrchestratorSettings, agent: &Agent) -> String {
        match self {
            Setting::Provider => match &settings.provider {
                None => format!("Do workspace ({})", agent.provider),
                Some(id) => agent
                    .providers
                    .iter()
                    .find(|p| &p.id == id)
                    .map_or_else(|| id.clone(), |p| p.name.clone()),
            },
            Setting::Model if agent.catalog.is_none() && settings.provider.is_some() => {
                "Padrão do provedor".into()
            }
            Setting::Model => match &settings.model {
                Some(model) => {
                    let effort = settings
                        .effort
                        .as_deref()
                        .map_or(String::new(), |e| format!(" · esforço {}", effort_label(e)));
                    format!("{}{effort}", (agent.label)(model))
                }
                None => {
                    let default = agent
                        .default_model
                        .as_deref()
                        .map_or("padrão do agente".to_string(), agent.label);
                    format!("Default ({default})")
                }
            },
            Setting::SpawnMode => settings.spawn_mode.label().to_string(),
            Setting::CloseFinished if settings.close_finished => "Fechar o chat".into(),
            Setting::CloseFinished => "Manter aberto".into(),
            Setting::FocusDelegated if settings.focus_delegated => "Mostrar o chat".into(),
            Setting::FocusDelegated => "Em segundo plano".into(),
        }
    }

    /// Muda a configuração um passo (`step` é -1 ou 1).
    fn change(self, settings: &mut OrchestratorSettings, agent: &Agent, step: isize) {
        match self {
            Setting::Provider => {
                // O do workspace (None) e depois os provedores conhecidos.
                let options: Vec<Option<String>> = std::iter::once(None)
                    .chain(agent.providers.iter().map(|p| Some(p.id.clone())))
                    .collect();
                let i = options
                    .iter()
                    .position(|p| *p == settings.provider)
                    .unwrap_or(0);
                let next = (i as isize + step).rem_euclid(options.len() as isize) as usize;
                if options[next] != settings.provider {
                    // O modelo e o esforço são ids do provedor anterior.
                    settings.model = None;
                    settings.effort = None;
                }
                settings.provider = options[next].clone();
            }
            // O modelo é escolhido no menu.
            Setting::Model => {}
            Setting::SpawnMode => {
                let all = StartMode::ALL;
                let i = all
                    .iter()
                    .position(|m| *m == settings.spawn_mode)
                    .unwrap_or(0);
                let next = (i as isize + step).rem_euclid(all.len() as isize) as usize;
                settings.spawn_mode = all[next];
            }
            Setting::CloseFinished => settings.close_finished = !settings.close_finished,
            Setting::FocusDelegated => settings.focus_delegated = !settings.focus_delegated,
        }
    }
}

/// O agente dos chats delegados, para a tela: o provedor do workspace, os
/// provedores conhecidos, o catálogo de modelos (`None` quando os delegados
/// usam outro provedor), o modelo padrão e o nome de um modelo (ex.:
/// `Opus - 5.5`).
pub struct Agent<'a> {
    pub provider: &'a str,
    pub providers: &'a [ProviderOption],
    pub catalog: Option<&'a Catalog>,
    pub default_model: Option<String>,
    pub label: &'a dyn Fn(&str) -> String,
}

/// O provedor dos chats delegados quando ele não é o do workspace: o terra
/// não tem o catálogo dele, então carrega o próprio (modelos e esforço
/// válidos para ele, e o nome de cada modelo).
pub struct Delegated {
    /// O `id` do provedor.
    id: String,
    adapter: Box<dyn ChatAdapter>,
    loading: Option<Receiver<Result<Catalog, String>>>,
    catalog: Option<Catalog>,
}

impl Delegated {
    fn new(id: &str) -> Self {
        let adapter = adapters::select(Some(id));
        let loading = adapter.load_catalog(None);
        Self {
            id: id.to_string(),
            adapter,
            loading,
            catalog: None,
        }
    }

    /// Acompanha o provedor escolhido no Orquestrador: `chosen` é o `id`
    /// dele, e `own` diz que é o do workspace (que já tem o catálogo).
    /// Retorna `true` quando o catálogo chegou agora.
    pub fn sync(slot: &mut Option<Self>, chosen: Option<&str>, own: bool) -> bool {
        let wanted = chosen.filter(|_| !own);
        if slot.as_ref().map(|d| d.id.as_str()) != wanted {
            *slot = wanted.map(Self::new);
        }
        let Some(this) = slot else {
            return false;
        };
        let Some(loading) = &this.loading else {
            return false;
        };
        let catalog = match loading.try_recv() {
            Err(TryRecvError::Empty) => return false,
            Ok(Ok(catalog)) => catalog,
            // Sem o catálogo, o delegado usa o modelo padrão do provedor.
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => Catalog::default(),
        };
        this.loading = None;
        this.catalog = Some(catalog);
        true
    }

    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref()
    }

    /// Nome de um modelo do provedor para a tela.
    pub fn label(&self, id: &str) -> String {
        model_label(self.catalog.as_ref(), Some(self.adapter.as_ref()), id)
    }
}

/// Nome de um modelo do `catalog` para a tela (ex.: `Opus - 5.5`).
pub fn model_label(
    catalog: Option<&Catalog>,
    adapter: Option<&dyn ChatAdapter>,
    id: &str,
) -> String {
    let info = catalog.and_then(|c| find_model(c, id));
    match (info, adapter) {
        (Some(info), Some(adapter)) => info
            .resolved
            .as_deref()
            .map_or_else(|| info.label.clone(), |r| adapter.model_label(r)),
        (Some(info), None) => info.label.clone(),
        (None, _) => id.to_string(),
    }
}

fn find_model<'a>(catalog: &'a Catalog, id: &str) -> Option<&'a ModelInfo> {
    catalog
        .models
        .iter()
        .find(|m| m.id == id || m.resolved.as_deref() == Some(id))
}

/// O modelo e o esforço salvos valem no `catalog`? Sem catálogo (ainda não
/// chegou), não há como saber e eles ficam. O esforço sem modelo no
/// catálogo, ou que o modelo não aceita, cai.
pub fn valid_choice(
    catalog: Option<&Catalog>,
    model: Option<&str>,
    effort: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(catalog) = catalog.filter(|c| !c.models.is_empty()) else {
        return (model.map(str::to_string), effort.map(str::to_string));
    };
    let Some(info) = model.and_then(|m| find_model(catalog, m)) else {
        return (None, None);
    };
    let effort = effort.filter(|e| info.efforts.iter().any(|x| x == e));
    (model.map(str::to_string), effort.map(str::to_string))
}

/// O que uma tecla fez na aba.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Moved,
    /// Uma configuração mudou: salvar.
    Changed,
}

#[derive(Default)]
pub struct OrchestratorView {
    /// A configuração escolhida.
    selected: usize,
    /// O menu do `/model` dos chats delegados, quando aberto.
    model_menu: Option<ModelMenu>,
}

impl OrchestratorView {
    pub fn help(&self) -> &'static str {
        if self.model_menu.is_some() {
            HELP_MODEL
        } else {
            HELP
        }
    }

    /// Ajusta o menu do modelo, aberto, ao catálogo que chegou.
    pub fn sync(&mut self, catalog: Option<&Catalog>) {
        if let Some(menu) = &mut self.model_menu {
            menu.sync(catalog);
        }
    }

    /// `↑`/`↓` passam pelas configurações; `←`/`→` e `Enter`
    /// mudam a configuração escolhida. `Enter` no Modelo abre o menu do
    /// `/model`, que fica com as teclas enquanto aberto.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        settings: &mut OrchestratorSettings,
        agent: &Agent,
    ) -> Outcome {
        if let Some(menu) = &mut self.model_menu {
            return match menu.handle_key(key, agent.catalog) {
                model_menu::Outcome::Handled => Outcome::Moved,
                model_menu::Outcome::Close => {
                    self.model_menu = None;
                    Outcome::Moved
                }
                model_menu::Outcome::Chosen { model, effort, .. } => {
                    self.model_menu = None;
                    settings.model = Some(model);
                    settings.effort = effort;
                    Outcome::Changed
                }
                model_menu::Outcome::Unset(_) => {
                    self.model_menu = None;
                    settings.model = None;
                    settings.effort = None;
                    Outcome::Changed
                }
            };
        }
        // Com outro provedor, até o catálogo dele chegar (ou sem ele): o padrão.
        let other_provider = agent.catalog.is_none() && settings.provider.is_some();
        if SETTINGS.get(self.selected) == Some(&Setting::Model)
            && key.code == KeyCode::Enter
            && !other_provider
        {
            let mut menu = ModelMenu::delegated(
                settings.model.clone(),
                settings.effort.clone(),
                agent.default_model.clone(),
                agent.catalog,
            );
            menu.sync(agent.catalog);
            self.model_menu = Some(menu);
            return Outcome::Moved;
        }
        let step = match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                return Outcome::Moved;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(SETTINGS.len() - 1);
                return Outcome::Moved;
            }
            KeyCode::Left | KeyCode::Char('h') => -1,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => 1,
            _ => return Outcome::Ignored,
        };
        match SETTINGS.get(self.selected) {
            Some(Setting::Model) => Outcome::Ignored,
            Some(setting) => {
                setting.change(settings, agent, step);
                Outcome::Changed
            }
            None => Outcome::Ignored,
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        settings: &OrchestratorSettings,
        agent: &Agent,
    ) {
        let mut lines = vec![
            Line::styled(" Orquestrador", theme::section_title()),
            Line::styled(
                "   Por enquanto, as configurações só ficam salvas.",
                theme::muted(),
            ),
            Line::default(),
            Line::styled(" Configurações", theme::section_title()),
        ];
        for (i, setting) in SETTINGS.iter().enumerate() {
            lines.push(setting_line(*setting, settings, agent, i == self.selected));
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
        if let Some(menu) = &self.model_menu {
            // Como o do chat, mas acima do pé da aba.
            let bottom = Rect {
                y: area.bottom(),
                height: 0,
                ..area
            };
            let label = |m: &ModelInfo| (agent.label)(&m.id);
            menu.draw(frame, bottom, agent.catalog, &label);
        }
    }
}

fn setting_line(
    setting: Setting,
    settings: &OrchestratorSettings,
    agent: &Agent,
    selected: bool,
) -> Line<'static> {
    let label = format!("{:<LABEL_WIDTH$}", setting.label());
    let value = setting.value(settings, agent);
    if selected {
        // O modelo muda no menu do `Enter` (só com o catálogo do provedor).
        let value = match setting {
            Setting::Model if agent.catalog.is_none() && settings.provider.is_some() => value,
            Setting::Model => format!("{value}  (Enter)"),
            _ => format!("‹ {value} ›"),
        };
        Line::from(vec![
            Span::raw(format!("{} ", theme::FOCUS_MARKER)),
            Span::raw(label),
            Span::raw(value),
        ])
        .style(theme::focused())
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(label, theme::text()),
            Span::styled(format!("  {value}"), theme::path()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn label(id: &str) -> String {
        format!("{id} - 5.5")
    }

    fn catalog() -> Catalog {
        let model = |id: &str| ModelInfo {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            efforts: vec!["low".into(), "high".into()],
            resolved: None,
        };
        Catalog {
            models: vec![model("opus"), model("sonnet")],
            default_model: Some("opus".into()),
            ..Catalog::default()
        }
    }

    #[test]
    fn saved_model_and_effort_must_belong_to_the_provider_catalog() {
        let catalog = catalog();
        let valid = |m, e| valid_choice(Some(&catalog), m, e);
        assert_eq!(
            valid(Some("opus"), Some("high")),
            (Some("opus".into()), Some("high".into()))
        );
        // O esforço que o modelo não aceita cai; o modelo fica.
        assert_eq!(
            valid(Some("opus"), Some("max")),
            (Some("opus".into()), None)
        );
        // Um modelo de outro provedor cai, com o esforço.
        assert_eq!(valid(Some("gemini-x"), Some("high")), (None, None));
        // Sem catálogo (ainda não chegou), não há como saber.
        assert_eq!(
            valid_choice(None, Some("gemini-x"), Some("high")),
            (Some("gemini-x".into()), Some("high".into()))
        );
    }

    fn agent(catalog: &Catalog) -> Agent<'_> {
        let providers = vec![
            ProviderOption {
                id: "claude".into(),
                name: "Claude".into(),
                available: true,
            },
            ProviderOption {
                id: "antigravity".into(),
                name: "Antigravity".into(),
                available: true,
            },
        ];
        Agent {
            provider: "Claude",
            providers: providers.leak(),
            catalog: Some(catalog),
            default_model: Some("opus".into()),
            label: &label,
        }
    }

    #[test]
    fn settings_change_and_ask_to_save() {
        let mut view = OrchestratorView::default();
        let catalog = catalog();
        let agent = agent(&catalog);
        let mut settings = OrchestratorSettings::default();
        assert!(settings.close_finished, "fecha ao terminar por padrão");
        assert!(!settings.focus_delegated);
        // O Provedor passa pelo do workspace e pelos conhecidos.
        assert_eq!(
            view.handle_key(right(), &mut settings, &agent),
            Outcome::Changed
        );
        assert_eq!(settings.provider.as_deref(), Some("claude"));
        assert_eq!(Setting::Provider.value(&settings, &agent), "Claude");
        view.handle_key(right(), &mut settings, &agent);
        assert_eq!(settings.provider.as_deref(), Some("antigravity"));
        view.handle_key(right(), &mut settings, &agent);
        assert_eq!(
            settings.provider, None,
            "depois do último, volta ao do workspace"
        );
        // No Modelo, ←/→ não mudam nada e o Enter abre o menu do `/model`.
        view.handle_key(down(), &mut settings, &agent);
        assert_eq!(
            view.handle_key(right(), &mut settings, &agent),
            Outcome::Ignored
        );
        assert_eq!(
            view.handle_key(enter(), &mut settings, &agent),
            Outcome::Moved
        );
        assert_eq!(view.help(), HELP_MODEL);
        // Default → 1. opus → 2. sonnet, com o esforço.
        view.handle_key(down(), &mut settings, &agent);
        view.handle_key(down(), &mut settings, &agent);
        view.handle_key(left(), &mut settings, &agent);
        assert_eq!(
            view.handle_key(enter(), &mut settings, &agent),
            Outcome::Changed
        );
        assert_eq!(settings.model.as_deref(), Some("sonnet"));
        assert_eq!(settings.effort.as_deref(), Some("low"));
        assert_eq!(view.help(), HELP);
        assert_eq!(
            Setting::Model.value(&settings, &agent),
            "sonnet - 5.5 · esforço baixo"
        );
        // O Default volta ao modelo padrão.
        view.handle_key(enter(), &mut settings, &agent);
        view.handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut settings,
            &agent,
        );
        view.handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut settings,
            &agent,
        );
        assert_eq!(
            view.handle_key(enter(), &mut settings, &agent),
            Outcome::Changed
        );
        assert_eq!(
            (settings.model.clone(), settings.effort.clone()),
            (None, None)
        );
        // Modo dos chats: Accept Edits → Plan.
        view.handle_key(down(), &mut settings, &agent);
        assert_eq!(
            view.handle_key(right(), &mut settings, &agent),
            Outcome::Changed
        );
        assert_eq!(
            settings.spawn_mode,
            StartMode::Fixed(crate::agent::PermissionMode::Plan)
        );
        view.handle_key(down(), &mut settings, &agent);
        assert_eq!(
            view.handle_key(enter(), &mut settings, &agent),
            Outcome::Changed
        );
        assert!(!settings.close_finished);
        view.handle_key(down(), &mut settings, &agent);
        assert_eq!(
            view.handle_key(right(), &mut settings, &agent),
            Outcome::Changed
        );
        assert!(settings.focus_delegated);
        // E ↓ para na última.
        assert_eq!(
            view.handle_key(down(), &mut settings, &agent),
            Outcome::Moved
        );
        assert_eq!(view.selected, SETTINGS.len() - 1);
    }

    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn left() -> KeyEvent {
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
    }

    fn right() -> KeyEvent {
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
}
