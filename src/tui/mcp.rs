//! Aba Orquestrador. Aparece com o MCP online e mostra, em Configurações,
//! o provedor e o modelo dos chats delegados (os sub agentes) e como eles
//! começam e terminam e, em
//! Sessões, os spaces (os workspaces salvos e os abertos em algum terra) e,
//! embaixo de cada um, os terra abertos nele, com sessão ativa ou só o terra
//! cli aberto.
//!
//! ```text
//!  Orquestrador · http://127.0.0.1:30303/mcp
//!    3 terra cli abertos · 1 com sessão ativa
//!
//!  Configurações
//!    Provedor          Claude
//!    Modelo            Default (Opus - 5.5)
//!    Modo dos chats    Accept Edits
//!    Ao terminar       Fechar o chat
//!    Ao abrir          Em segundo plano
//!
//!  Sessões
//!  ▌ ● terraforme-cli  sessão ativa      /home/eu/terraforme-cli
//!        ● pid 4210     sessão ativa
//!        ● pid 4388     terra cli aberto
//!    ● site            terra cli aberto  /home/eu/site
//!        ● pid 5120     terra cli aberto
//!    ○ notas           fechado           /home/eu/notas
//! ```
//!
//! `↑`/`↓` passam pelas configurações e pelos spaces; `←`/`→` (ou `Enter`)
//! mudam a configuração escolhida. No Provedor, `←`/`→` passam pelo do
//! workspace e pelos provedores conhecidos. No Modelo, `Enter` abre o mesmo
//! menu do `/model`, com o esforço, com o catálogo do provedor escolhido (o
//! do workspace ou, com outro, o que o terra carrega para ele: [`Delegated`]).
//! A lista é atualizada a cada [`STALE`] enquanto a aba
//! está aberta.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::chat::model_menu::{self, ModelMenu, effort_label};
use crate::agent::adapters::{self, ProviderOption};
use crate::agent::{Catalog, ChatAdapter, ModelInfo};
use crate::db;
use crate::mcp::{self, Instance};
use crate::settings::{McpSettings, StartMode};
use crate::theme::{self, Variant};

pub const HELP: &str = "↑/↓ escolher · ←/→ mudar · Tab trocar aba · Esc ir para Sair";
/// Com o menu do modelo aberto.
const HELP_MODEL: &str = "↑/↓ escolher · ←/→ esforço · Enter selecionar · Esc fechar";
/// Depois disso, a lista é recarregada.
pub const STALE: Duration = Duration::from_secs(1);
/// A maior largura da coluna dos nomes.
const NAME_MAX: usize = 28;
/// Largura da coluna do estado.
const STATUS_WIDTH: usize = 18;
/// As linhas antes da lista (título, resumo, as configurações com o título
/// e a linha vazia, e o título Sessões) e o recuo de cima.
const HEADER: u16 = 6 + SETTINGS.len() as u16;
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

    fn value(self, mcp: &McpSettings, agent: &Agent) -> String {
        match self {
            Setting::Provider => match &mcp.provider {
                None => format!("Do workspace ({})", agent.provider),
                Some(id) => agent
                    .providers
                    .iter()
                    .find(|p| &p.id == id)
                    .map_or_else(|| id.clone(), |p| p.name.clone()),
            },
            Setting::Model if agent.catalog.is_none() && mcp.provider.is_some() => {
                "Padrão do provedor".into()
            }
            Setting::Model => match &mcp.model {
                Some(model) => {
                    let effort = mcp
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
            Setting::SpawnMode => mcp.spawn_mode.label().to_string(),
            Setting::CloseFinished if mcp.close_finished => "Fechar o chat".into(),
            Setting::CloseFinished => "Manter aberto".into(),
            Setting::FocusDelegated if mcp.focus_delegated => "Mostrar o chat".into(),
            Setting::FocusDelegated => "Em segundo plano".into(),
        }
    }

    /// Muda a configuração um passo (`step` é -1 ou 1).
    fn change(self, mcp: &mut McpSettings, agent: &Agent, step: isize) {
        match self {
            Setting::Provider => {
                // O do workspace (None) e depois os provedores conhecidos.
                let options: Vec<Option<String>> = std::iter::once(None)
                    .chain(agent.providers.iter().map(|p| Some(p.id.clone())))
                    .collect();
                let i = options.iter().position(|p| *p == mcp.provider).unwrap_or(0);
                let next = (i as isize + step).rem_euclid(options.len() as isize) as usize;
                if options[next] != mcp.provider {
                    // O modelo e o esforço são ids do provedor anterior.
                    mcp.model = None;
                    mcp.effort = None;
                }
                mcp.provider = options[next].clone();
            }
            // O modelo é escolhido no menu.
            Setting::Model => {}
            Setting::SpawnMode => {
                let all = StartMode::ALL;
                let i = all.iter().position(|m| *m == mcp.spawn_mode).unwrap_or(0);
                let next = (i as isize + step).rem_euclid(all.len() as isize) as usize;
                mcp.spawn_mode = all[next];
            }
            Setting::CloseFinished => mcp.close_finished = !mcp.close_finished,
            Setting::FocusDelegated => mcp.focus_delegated = !mcp.focus_delegated,
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

/// O que está aberto num space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpaceStatus {
    /// Um terra aberto com uma conversa com o agente.
    Session,
    /// Um terra aberto, sem conversa.
    Open,
    /// Nenhum terra aberto (um workspace salvo).
    Closed,
}

impl SpaceStatus {
    fn label(self) -> &'static str {
        match self {
            SpaceStatus::Session => "sessão ativa",
            SpaceStatus::Open => "terra cli aberto",
            SpaceStatus::Closed => "fechado",
        }
    }

    fn dot(self) -> Span<'static> {
        match self {
            SpaceStatus::Session => Span::styled("●", theme::variant(Variant::Success)),
            SpaceStatus::Open => Span::styled("●", theme::variant(Variant::Warning)),
            SpaceStatus::Closed => Span::styled("○", theme::muted()),
        }
    }
}

impl From<&Instance> for SpaceStatus {
    fn from(instance: &Instance) -> Self {
        if instance.session {
            SpaceStatus::Session
        } else {
            SpaceStatus::Open
        }
    }
}

/// Um space da lista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub name: String,
    pub dir: PathBuf,
    pub status: SpaceStatus,
    /// Os terra abertos nele, os com sessão ativa primeiro.
    pub instances: Vec<Instance>,
}

/// Junta os workspaces salvos com os terra abertos: um space por pasta, com
/// os terra dele agrupados, as sessões ativas primeiro, depois os abertos e
/// os fechados, cada grupo por nome.
pub fn spaces(saved: &[db::Workspace], instances: &[Instance]) -> Vec<Space> {
    let mut spaces: Vec<Space> = saved
        .iter()
        .map(|w| Space {
            name: w.name.clone(),
            dir: w.path.clone(),
            status: SpaceStatus::Closed,
            instances: Vec::new(),
        })
        .collect();
    for instance in instances {
        let status = SpaceStatus::from(instance);
        match spaces.iter_mut().find(|s| s.dir == instance.dir) {
            Some(space) => {
                space.status = space.status.min(status);
                space.instances.push(instance.clone());
            }
            None => spaces.push(Space {
                name: instance.name.clone(),
                dir: instance.dir.clone(),
                status,
                instances: vec![instance.clone()],
            }),
        }
    }
    for space in &mut spaces {
        space
            .instances
            .sort_by_key(|i| (SpaceStatus::from(i), i.pid));
    }
    spaces.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    spaces
}

#[derive(Default)]
pub struct McpView {
    /// A linha escolhida: as configurações primeiro, depois os spaces.
    selected: usize,
    spaces: Vec<Space>,
    /// Quando a lista foi carregada.
    refreshed: Option<Instant>,
    /// O menu do `/model` dos chats delegados, quando aberto.
    model_menu: Option<ModelMenu>,
}

impl McpView {
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

    /// Se a lista deve ser recarregada.
    pub fn is_stale(&self) -> bool {
        self.refreshed.is_none_or(|at| at.elapsed() >= STALE)
    }

    pub fn refresh(&mut self, saved: &[db::Workspace], instances: &[Instance]) {
        self.spaces = spaces(saved, instances);
        self.selected = self.selected.min(self.last());
        self.refreshed = Some(Instant::now());
    }

    /// A última linha que dá para escolher.
    fn last(&self) -> usize {
        SETTINGS.len() + self.spaces.len().saturating_sub(1)
    }

    /// O space escolhido, com o foco nos spaces.
    fn selected_space(&self) -> Option<usize> {
        self.selected.checked_sub(SETTINGS.len())
    }

    /// `↑`/`↓` passam pelas configurações e pelos spaces; `←`/`→` e `Enter`
    /// mudam a configuração escolhida. `Enter` no Modelo abre o menu do
    /// `/model`, que fica com as teclas enquanto aberto.
    pub fn handle_key(&mut self, key: KeyEvent, mcp: &mut McpSettings, agent: &Agent) -> Outcome {
        if let Some(menu) = &mut self.model_menu {
            return match menu.handle_key(key, agent.catalog) {
                model_menu::Outcome::Handled => Outcome::Moved,
                model_menu::Outcome::Close => {
                    self.model_menu = None;
                    Outcome::Moved
                }
                model_menu::Outcome::Chosen { model, effort, .. } => {
                    self.model_menu = None;
                    mcp.model = Some(model);
                    mcp.effort = effort;
                    Outcome::Changed
                }
                model_menu::Outcome::Unset(_) => {
                    self.model_menu = None;
                    mcp.model = None;
                    mcp.effort = None;
                    Outcome::Changed
                }
            };
        }
        // Com outro provedor, até o catálogo dele chegar (ou sem ele): o padrão.
        let other_provider = agent.catalog.is_none() && mcp.provider.is_some();
        if SETTINGS.get(self.selected) == Some(&Setting::Model)
            && key.code == KeyCode::Enter
            && !other_provider
        {
            let mut menu = ModelMenu::delegated(
                mcp.model.clone(),
                mcp.effort.clone(),
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
                self.selected = (self.selected + 1).min(self.last());
                return Outcome::Moved;
            }
            KeyCode::Left | KeyCode::Char('h') => -1,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => 1,
            _ => return Outcome::Ignored,
        };
        match SETTINGS.get(self.selected) {
            Some(Setting::Model) => Outcome::Ignored,
            Some(setting) => {
                setting.change(mcp, agent, step);
                Outcome::Changed
            }
            None => Outcome::Ignored,
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, mcp: &McpSettings, agent: &Agent) {
        let open: usize = self.spaces.iter().map(|s| s.instances.len()).sum();
        let sessions = self
            .spaces
            .iter()
            .filter(|s| s.status == SpaceStatus::Session)
            .count();
        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Orquestrador", theme::section_title()),
                Span::styled(format!(" · {}", mcp::url()), theme::path()),
            ]),
            Line::styled(
                format!(
                    "   {} · {sessions} com sessão ativa",
                    if open == 1 {
                        "1 terra cli aberto".to_string()
                    } else {
                        format!("{open} terra cli abertos")
                    },
                ),
                theme::muted(),
            ),
            Line::default(),
            Line::styled(" Configurações", theme::section_title()),
        ];
        for (i, setting) in SETTINGS.iter().enumerate() {
            lines.push(setting_line(*setting, mcp, agent, i == self.selected));
        }
        lines.push(Line::default());
        lines.push(Line::styled(" Sessões", theme::section_title()));
        if self.spaces.is_empty() {
            lines.push(Line::styled(
                "   Nenhum space: salve um com terra workspace --save.",
                theme::muted(),
            ));
        }
        let name_width = self
            .spaces
            .iter()
            .map(|s| s.name.width())
            .max()
            .unwrap_or(0)
            .min(NAME_MAX);
        // Cada space, com os terra dele embaixo.
        let mut list = Vec::new();
        let mut selected = 0..0;
        for (i, space) in self.spaces.iter().enumerate() {
            let start = list.len();
            let chosen = self.selected_space() == Some(i);
            list.push(space_line(space, name_width, chosen));
            for instance in &space.instances {
                list.push(instance_line(instance, name_width));
            }
            if chosen {
                selected = start..list.len();
            }
        }
        // A lista rola para o space escolhido (e os terra dele) ficar na tela.
        let rows = usize::from(area.height.saturating_sub(HEADER)).max(1);
        let offset = selected.end.saturating_sub(rows).min(selected.start);
        lines.extend(list.into_iter().skip(offset).take(rows));
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
    mcp: &McpSettings,
    agent: &Agent,
    selected: bool,
) -> Line<'static> {
    let label = format!("{:<LABEL_WIDTH$}", setting.label());
    let value = setting.value(mcp, agent);
    if selected {
        // O modelo muda no menu do `Enter` (só com o catálogo do provedor).
        let value = match setting {
            Setting::Model if agent.catalog.is_none() && mcp.provider.is_some() => value,
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

fn space_line(space: &Space, name_width: usize, selected: bool) -> Line<'static> {
    let name: String = truncate(&space.name, name_width);
    let pad = name_width - name.width();
    let status = space.status.label();
    let text = format!(" {name}{}  {status:<STATUS_WIDTH$}", " ".repeat(pad));
    let dir = space.dir.display().to_string();
    if selected {
        Line::from(vec![
            Span::raw(format!("{} ", theme::FOCUS_MARKER)),
            space.status.dot(),
            Span::raw(text),
            Span::raw(dir),
        ])
        .style(theme::focused().add_modifier(Modifier::BOLD))
    } else {
        let style = if space.status == SpaceStatus::Closed {
            theme::muted()
        } else {
            theme::text()
        };
        Line::from(vec![
            Span::raw("  "),
            space.status.dot(),
            Span::styled(text, style),
            Span::styled(dir, theme::path()),
        ])
    }
}

/// Um terra aberto, embaixo do space dele.
fn instance_line(instance: &Instance, name_width: usize) -> Line<'static> {
    let status = SpaceStatus::from(instance);
    // Recuado sob o nome do space; com espaço, o estado alinha com o dele.
    let pid = format!("pid {}", instance.pid);
    let width = name_width.saturating_sub(3).max(pid.width());
    Line::from(vec![
        Span::raw("      "),
        status.dot(),
        Span::styled(
            format!(" {pid:<width$}  {}", status.label()),
            theme::muted(),
        ),
    ])
}

/// `text` com no máximo `width` colunas, com `…` quando corta.
fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

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

    fn saved(name: &str) -> db::Workspace {
        db::Workspace {
            name: name.into(),
            path: PathBuf::from(format!("/w/{name}")),
        }
    }

    fn open(pid: u32, name: &str, session: bool) -> Instance {
        Instance {
            pid,
            name: name.into(),
            dir: PathBuf::from(format!("/w/{name}")),
            session,
            chats: Vec::new(),
        }
    }

    #[test]
    fn spaces_join_saved_workspaces_with_open_terra() {
        let list = spaces(
            &[saved("notas"), saved("terra"), saved("Site")],
            &[
                open(1, "Site", false),
                open(2, "terra", false),
                open(3, "terra", true),
                open(4, "novo", false),
            ],
        );
        let rows: Vec<_> = list
            .iter()
            .map(|s| {
                let pids: Vec<_> = s.instances.iter().map(|i| i.pid).collect();
                (s.name.as_str(), s.status, pids)
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("terra", SpaceStatus::Session, vec![3, 2]),
                ("novo", SpaceStatus::Open, vec![4]),
                ("Site", SpaceStatus::Open, vec![1]),
                ("notas", SpaceStatus::Closed, vec![]),
            ]
        );
    }

    #[test]
    fn draws_the_sessions_grouped_by_workspace() {
        let mut view = McpView::default();
        view.refresh(
            &[saved("notas")],
            &[
                open(1, "terra", false),
                open(2, "site", false),
                open(3, "terra", true),
            ],
        );
        let catalog = catalog();
        let agent = agent(&catalog);
        let mut mcp = McpSettings::default();
        for _ in 0..SETTINGS.len() + 1 {
            assert_eq!(view.handle_key(down(), &mut mcp, &agent), Outcome::Moved);
        }
        assert_eq!(view.handle_key(enter(), &mut mcp, &agent), Outcome::Ignored);
        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), &mcp, &agent))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..70)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(1), "  Orquestrador · http://127.0.0.1:30303/mcp");
        assert_eq!(row(2), "    3 terra cli abertos · 1 com sessão ativa");
        assert_eq!(row(4), "  Configurações");
        assert_eq!(row(5), "   Provedor            Do workspace (Claude)");
        assert_eq!(row(6), "   Modelo              Default (opus - 5.5)");
        assert_eq!(row(7), "   Modo dos chats      Accept Edits");
        assert_eq!(row(8), "   Ao terminar         Fechar o chat");
        assert_eq!(row(9), "   Ao abrir            Em segundo plano");
        assert_eq!(row(11), "  Sessões");
        assert_eq!(row(12), "   ● terra  sessão ativa      /w/terra");
        assert_eq!(row(13), "       ● pid 3  sessão ativa");
        assert_eq!(row(14), "       ● pid 1  terra cli aberto");
        assert_eq!(row(15), " ▌ ● site   terra cli aberto  /w/site");
        assert_eq!(row(16), "       ● pid 2  terra cli aberto");
        assert_eq!(row(17), "   ○ notas  fechado           /w/notas");
    }

    #[test]
    fn settings_change_and_ask_to_save() {
        let mut view = McpView::default();
        view.refresh(&[saved("notas")], &[]);
        let catalog = catalog();
        let agent = agent(&catalog);
        let mut mcp = McpSettings::default();
        assert!(mcp.close_finished, "fecha ao terminar por padrão");
        assert!(!mcp.focus_delegated);
        // O Provedor passa pelo do workspace e pelos conhecidos.
        assert_eq!(view.handle_key(right(), &mut mcp, &agent), Outcome::Changed);
        assert_eq!(mcp.provider.as_deref(), Some("claude"));
        assert_eq!(Setting::Provider.value(&mcp, &agent), "Claude");
        view.handle_key(right(), &mut mcp, &agent);
        assert_eq!(mcp.provider.as_deref(), Some("antigravity"));
        view.handle_key(right(), &mut mcp, &agent);
        assert_eq!(
            mcp.provider, None,
            "depois do último, volta ao do workspace"
        );
        // No Modelo, ←/→ não mudam nada e o Enter abre o menu do `/model`.
        view.handle_key(down(), &mut mcp, &agent);
        assert_eq!(view.handle_key(right(), &mut mcp, &agent), Outcome::Ignored);
        assert_eq!(view.handle_key(enter(), &mut mcp, &agent), Outcome::Moved);
        assert_eq!(view.help(), HELP_MODEL);
        // Default → 1. opus → 2. sonnet, com o esforço.
        view.handle_key(down(), &mut mcp, &agent);
        view.handle_key(down(), &mut mcp, &agent);
        view.handle_key(left(), &mut mcp, &agent);
        assert_eq!(view.handle_key(enter(), &mut mcp, &agent), Outcome::Changed);
        assert_eq!(mcp.model.as_deref(), Some("sonnet"));
        assert_eq!(mcp.effort.as_deref(), Some("low"));
        assert_eq!(view.help(), HELP);
        assert_eq!(
            Setting::Model.value(&mcp, &agent),
            "sonnet - 5.5 · esforço baixo"
        );
        // O Default volta ao modelo padrão.
        view.handle_key(enter(), &mut mcp, &agent);
        view.handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut mcp,
            &agent,
        );
        view.handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut mcp,
            &agent,
        );
        assert_eq!(view.handle_key(enter(), &mut mcp, &agent), Outcome::Changed);
        assert_eq!((mcp.model.clone(), mcp.effort.clone()), (None, None));
        // Modo dos chats: Accept Edits → Plan.
        view.handle_key(down(), &mut mcp, &agent);
        assert_eq!(view.handle_key(right(), &mut mcp, &agent), Outcome::Changed);
        assert_eq!(
            mcp.spawn_mode,
            StartMode::Fixed(crate::agent::PermissionMode::Plan)
        );
        view.handle_key(down(), &mut mcp, &agent);
        assert_eq!(view.handle_key(enter(), &mut mcp, &agent), Outcome::Changed);
        assert!(!mcp.close_finished);
        view.handle_key(down(), &mut mcp, &agent);
        assert_eq!(view.handle_key(right(), &mut mcp, &agent), Outcome::Changed);
        assert!(mcp.focus_delegated);
        // Nos spaces, ←/→ não mudam nada.
        view.handle_key(down(), &mut mcp, &agent);
        assert_eq!(view.handle_key(right(), &mut mcp, &agent), Outcome::Ignored);
        // E ↓ para no último space.
        assert_eq!(view.handle_key(down(), &mut mcp, &agent), Outcome::Moved);
        assert_eq!(view.selected_space(), Some(0));
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

    #[test]
    fn long_names_are_cut() {
        assert_eq!(truncate("terraforme", 6), "terra…");
        assert_eq!(truncate("terra", 6), "terra");
    }
}
