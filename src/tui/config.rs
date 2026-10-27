//! Aba Config: as configurações do terra, em seções (como as seções Recentes
//! e Salvos do navegador de workspaces). Há a seção Agente e, dentro dela,
//! separadas por uma linha, a do Contexto e a do Consumo; embaixo, separada
//! do Agente, a seção MCP (com uma bola verde com o MCP online e os
//! servidores MCP conectados ao agente, do provedor e do terraforme) e, embaixo
//! dela, a seção Skills (as skills instaladas no agente); à direita
//! (embaixo, em telas estreitas), a seção Extensões.
//!
//! `↑`/`↓` escolhem a configuração e `←`/`→` ou `Enter` trocam o valor (nos
//! servidores MCP e nas skills, passam de um para outro, com os detalhes
//! embaixo); a
//! tela principal salva e aplica a mudança. `Shift+Tab` alterna entre o
//! Agente e as Extensões. Nas Extensões, a lista é paginada (como a dos
//! modelos do `/model`, mas com páginas): `↑`/`↓` escolhem, `←`/`→` trocam
//! de página e `Espaço` (ou `Enter`) liga ou desliga a extensão.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use super::chat::compact_count;
use crate::agent::{AccountOption, ChatAdapter, ModelOption};
use crate::extension::Status;
use crate::mcp;
use crate::settings::{
    AgentSettings, ContextSettings, ExtensionSettings, Interface, McpSettings, StartMode,
};
use crate::theme::{self, Variant};

const HELP: &str =
    "↑/↓ escolher · ←/→ ou Enter alterar · Shift+Tab extensões · Tab trocar aba · Esc ir para Sair";
const HELP_EXTENSIONS: &str = "↑/↓ escolher · Espaço ativar · ←/→ página · Shift+Tab agente · Tab trocar aba · Esc ir para Sair";
/// Largura da coluna dos nomes das configurações.
const LABEL_WIDTH: usize = 18;
/// Extensões em cada página.
const PAGE: usize = 5;
/// Largura mínima da seção Extensões ao lado do Agente; com menos, ela fica
/// embaixo.
const EXTENSIONS_MIN_WIDTH: u16 = 40;
/// Largura máxima do comando de um servidor MCP.
const DETAIL_WIDTH: usize = 36;

/// Uma configuração que pode ser alterada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Account,
    StartMode,
    Model,
    Interface,
    ContextDisplay,
    AutoCompact,
    Limit,
    CompactAt,
    UsageDisplay,
    TotalUsageDisplay,
    Mcp,
    McpSpawnMode,
    McpServers,
    Skills,
}

const FIELDS: [Field; 14] = [
    Field::Account,
    Field::StartMode,
    Field::Model,
    Field::Interface,
    Field::ContextDisplay,
    Field::AutoCompact,
    Field::Limit,
    Field::CompactAt,
    Field::UsageDisplay,
    Field::TotalUsageDisplay,
    Field::Mcp,
    Field::McpSpawnMode,
    Field::McpServers,
    Field::Skills,
];
/// Onde começa a seção Contexto em [`FIELDS`].
const CONTEXT_START: usize = 4;
/// Onde começa a seção Consumo em [`FIELDS`].
const USAGE_START: usize = 8;
/// Onde começa a seção MCP em [`FIELDS`].
const MCP_START: usize = 10;

/// As opções que vêm do provedor: contas e modelos, e o limite de contexto
/// do modelo (para mostrar em tokens o ponto de compactar). E as extensões
/// do registro e o MCP.
#[derive(Debug, Default, Clone)]
pub struct Choices {
    pub accounts: Vec<AccountOption>,
    pub models: Vec<ModelOption>,
    pub context_window: u64,
    pub extensions: Vec<ExtensionOption>,
    pub mcp: McpState,
    pub skills: Vec<SkillInfo>,
}

/// Uma skill instalada no agente: de onde vem (ex.: `Claude · plugin`) e o
/// que ela faz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInfo {
    pub name: String,
    pub origin: String,
    pub description: String,
}

/// Uma extensão que pode ser ativada na seção Extensões.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub homepage: String,
    pub status: Status,
}

/// O que a Config fez com uma tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Moved,
    /// Um valor do Agente mudou: salvar e aplicar.
    Changed,
    /// Uma extensão foi ligada ou desligada: salvar e aplicar.
    ExtensionsChanged,
    /// O MCP foi ativado ou desativado: salvar e abrir ou fechar.
    McpChanged,
    /// O modo dos chats abertos pelo orquestrador mudou: salvar.
    McpSpawnModeChanged,
}

/// O MCP como a seção MCP mostra: a configuração, se ele está online e os
/// servidores MCP conectados ao agente.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct McpState {
    pub settings: McpSettings,
    pub online: bool,
    pub servers: Vec<McpServerInfo>,
}

/// Um servidor MCP conectado ao agente: de onde vem (ex.: `Claude ·
/// usuário`, `terraforme`) e o comando ou o endereço.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerInfo {
    pub name: String,
    pub origin: String,
    pub detail: String,
}

/// A seção em foco.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Section {
    #[default]
    Agent,
    Extensions,
}

#[derive(Default)]
pub struct ConfigView {
    section: Section,
    /// Posição em [`FIELDS`].
    focus: usize,
    /// Posição na lista das extensões.
    extension: usize,
    /// O servidor MCP em foco.
    mcp_server: usize,
    /// A skill em foco.
    skill: usize,
}

impl ConfigView {
    pub fn help(&self) -> &'static str {
        match self.section {
            Section::Agent => HELP,
            Section::Extensions => HELP_EXTENSIONS,
        }
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        settings: &mut AgentSettings,
        extensions: &mut ExtensionSettings,
        mcp: &mut McpSettings,
        choices: &Choices,
    ) -> Outcome {
        if key.code == KeyCode::BackTab {
            self.section = match self.section {
                Section::Agent => Section::Extensions,
                Section::Extensions => Section::Agent,
            };
            return Outcome::Moved;
        }
        if self.section == Section::Extensions {
            return self.handle_extension_key(key, extensions, &choices.extensions);
        }
        let last = FIELDS.len() - 1;
        let changes = matches!(
            key.code,
            KeyCode::Right
                | KeyCode::Left
                | KeyCode::Enter
                | KeyCode::Char('l')
                | KeyCode::Char('h')
        );
        if FIELDS[self.focus] == Field::Mcp && changes {
            mcp.enabled = !mcp.enabled;
            return Outcome::McpChanged;
        }
        if FIELDS[self.focus] == Field::McpSpawnMode && changes {
            let step = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                -1
            } else {
                1
            };
            let all = StartMode::ALL;
            let i = all.iter().position(|m| *m == mcp.spawn_mode).unwrap_or(0);
            mcp.spawn_mode = all[cycle(i, all.len(), step)];
            return Outcome::McpSpawnModeChanged;
        }
        if FIELDS[self.focus] == Field::McpServers && changes {
            let len = choices.mcp.servers.len().max(1);
            let step = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                -1
            } else {
                1
            };
            self.mcp_server = cycle(self.mcp_server.min(len - 1), len, step);
            return Outcome::Moved;
        }
        if FIELDS[self.focus] == Field::Skills && changes {
            let len = choices.skills.len().max(1);
            let step = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                -1
            } else {
                1
            };
            self.skill = cycle(self.skill.min(len - 1), len, step);
            return Outcome::Moved;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.focus = self.focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.focus = (self.focus + 1).min(last),
            KeyCode::Right | KeyCode::Enter | KeyCode::Char('l') => {
                change(FIELDS[self.focus], settings, choices, 1);
                return Outcome::Changed;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                change(FIELDS[self.focus], settings, choices, -1);
                return Outcome::Changed;
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Moved
    }

    /// `↑`/`↓` andam pela lista (trocando de página), `←`/`→` trocam de
    /// página e `Espaço`/`Enter` ligam ou desligam. Uma extensão que não
    /// está instalada não liga, mas desliga.
    fn handle_extension_key(
        &mut self,
        key: KeyEvent,
        settings: &mut ExtensionSettings,
        options: &[ExtensionOption],
    ) -> Outcome {
        let Some(last) = options.len().checked_sub(1) else {
            return match key.code {
                KeyCode::Tab | KeyCode::Esc => Outcome::Ignored,
                _ => Outcome::Moved,
            };
        };
        self.extension = self.extension.min(last);
        let page = self.extension / PAGE;
        let pages = last / PAGE;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.extension = self.extension.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.extension = (self.extension + 1).min(last),
            KeyCode::Left | KeyCode::Char('h') => {
                self.extension = page.saturating_sub(1) * PAGE;
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.extension = ((page + 1).min(pages) * PAGE).min(last);
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                let option = &options[self.extension];
                if !option.status.is_installed() && !settings.is_enabled(&option.id) {
                    return Outcome::Moved;
                }
                settings.toggle(&option.id);
                return Outcome::ExtensionsChanged;
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Moved
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        settings: &AgentSettings,
        extensions: &ExtensionSettings,
        adapter: Option<&dyn ChatAdapter>,
        choices: &Choices,
    ) {
        let provider = match adapter {
            Some(a) if a.is_available() => Span::styled(
                format!("{} · disponível", a.name()),
                theme::variant(Variant::Success),
            ),
            Some(a) => Span::styled(format!("{} · não encontrado", a.name()), theme::error()),
            None => Span::styled("Nenhum", theme::muted()),
        };

        let mut lines = vec![
            Line::styled(" Agente", theme::section_title()),
            info_row("Provedor", provider),
        ];
        let agent_focus = self.section == Section::Agent;
        let row = |i: usize, field: Field| {
            let value = value_label(field, settings, choices);
            field_row(field_label(field), value, agent_focus && i == self.focus)
        };
        let rows = |range: std::ops::Range<usize>| -> Vec<Line> {
            range.map(|i| row(i, FIELDS[i])).collect()
        };
        lines.extend(rows(0..CONTEXT_START));
        let context = rows(CONTEXT_START..USAGE_START);
        let usage = rows(USAGE_START..MCP_START);
        // A linha vai só até onde vai a seção, e não a tela inteira.
        let width = lines
            .iter()
            .chain(&context)
            .chain(&usage)
            .map(Line::width)
            .max();
        lines.push(separator(width.unwrap_or(0)));
        lines.push(Line::styled(" Contexto", theme::section_title()));
        lines.extend(context);
        lines.push(separator(width.unwrap_or(0)));
        lines.push(Line::styled(" Consumo", theme::section_title()));
        lines.extend(usage);

        // O MCP fica separado do Agente: é da máquina, não do workspace.
        let mcp = &choices.mcp;
        let mut title = vec![Span::styled(" MCP", theme::section_title())];
        if mcp.online {
            title.push(Span::styled(" ●", theme::variant(Variant::Success)));
        }
        let enabled = if mcp.settings.enabled {
            "Ativado"
        } else {
            "Desativado"
        };
        let mut mcp_lines = vec![
            Line::from(title),
            field_row(
                field_label(Field::Mcp),
                enabled.to_string(),
                agent_focus && FIELDS[self.focus] == Field::Mcp,
            ),
            field_row(
                field_label(Field::McpSpawnMode),
                mcp.settings.spawn_mode.label().to_string(),
                agent_focus && FIELDS[self.focus] == Field::McpSpawnMode,
            ),
            info_row("Endereço", Span::styled(mcp::url(), theme::path())),
        ];
        mcp_lines.extend(self.mcp_server_lines(
            &mcp.servers,
            agent_focus && FIELDS[self.focus] == Field::McpServers,
        ));
        let skill_lines = self.skill_lines(
            &choices.skills,
            agent_focus && FIELDS[self.focus] == Field::Skills,
        );
        let width = width
            .max(mcp_lines.iter().map(Line::width).max())
            .max(skill_lines.iter().map(Line::width).max());
        lines.push(Line::default());
        lines.extend(mcp_lines);
        lines.push(Line::default());
        lines.extend(skill_lines);

        let extension_lines = self.extension_lines(extensions, &choices.extensions);
        // A seção Agente com o recuo (1) e a folga (2) do bloco.
        let agent_width = width.unwrap_or(0) as u16 + 3;
        let (agent_area, extensions_area) = if area.width >= agent_width + EXTENSIONS_MIN_WIDTH {
            let [left, right] =
                Layout::horizontal([Constraint::Length(agent_width), Constraint::Fill(1)])
                    .areas(area);
            (left, right)
        } else {
            let [top, bottom] = Layout::vertical([
                Constraint::Length(lines.len() as u16 + 1),
                Constraint::Fill(1),
            ])
            .areas(area);
            (top, bottom)
        };
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            agent_area,
        );
        frame.render_widget(
            Paragraph::new(extension_lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            extensions_area,
        );
    }

    /// Os servidores MCP conectados ao agente: o em foco, `‹ nome · 1/3 ›`,
    /// com de onde vem e o comando ou o endereço.
    fn mcp_server_lines(&self, servers: &[McpServerInfo], focused: bool) -> Vec<Line<'static>> {
        let label = field_label(Field::McpServers);
        let Some(last) = servers.len().checked_sub(1) else {
            return vec![field_row(label, "Nenhum".to_string(), focused)];
        };
        let server = &servers[self.mcp_server.min(last)];
        let position = format!(
            "{} · {}/{}",
            server.name,
            self.mcp_server.min(last) + 1,
            servers.len()
        );
        vec![
            field_row(label, position, focused),
            info_row("Origem", Span::styled(server.origin.clone(), theme::text())),
            info_row(
                "Comando",
                Span::styled(shorten(&server.detail, DETAIL_WIDTH), theme::path()),
            ),
        ]
    }

    /// A seção Skills: a skill em foco, `‹ nome · 1/3 ›`, com de onde vem e
    /// o que ela faz.
    fn skill_lines(&self, skills: &[SkillInfo], focused: bool) -> Vec<Line<'static>> {
        let mut lines = vec![Line::styled(" Skills", theme::section_title())];
        let label = field_label(Field::Skills);
        let Some(last) = skills.len().checked_sub(1) else {
            lines.push(field_row(label, "Nenhuma".to_string(), focused));
            return lines;
        };
        let current = self.skill.min(last);
        let skill = &skills[current];
        let position = format!("{} · {}/{}", skill.name, current + 1, skills.len());
        lines.extend([
            field_row(label, position, focused),
            info_row("Origem", Span::styled(skill.origin.clone(), theme::text())),
            info_row(
                "Descrição",
                Span::styled(shorten(&skill.description, DETAIL_WIDTH), theme::muted()),
            ),
        ]);
        lines
    }

    /// A seção Extensões: a página da extensão em foco, como a lista do
    /// `/model` (`1. RTK - descrição`), com `[x]` nas ativas, os detalhes da
    /// que está em foco e a página.
    fn extension_lines(
        &self,
        settings: &ExtensionSettings,
        options: &[ExtensionOption],
    ) -> Vec<Line<'static>> {
        let mut lines = vec![Line::styled(" Extensões", theme::section_title())];
        if options.is_empty() {
            lines.push(Line::styled(
                "  Nenhuma extensão disponível.",
                theme::muted(),
            ));
            return lines;
        }
        let focused = self.section == Section::Extensions;
        let current = self.extension.min(options.len() - 1);
        let page = current / PAGE;
        let pages = options.len().div_ceil(PAGE);
        for (i, option) in options.iter().enumerate().skip(page * PAGE).take(PAGE) {
            let enabled = settings.is_enabled(&option.id);
            let check = if enabled { "[x]" } else { "[ ]" };
            let mut text = format!("{}. {} - {}", i + 1, option.name, option.description);
            if !option.status.is_installed() {
                text.push_str(" (indisponível)");
            }
            lines.push(if focused && i == current {
                Line::from(vec![
                    Span::raw(format!("{} ", theme::FOCUS_MARKER)),
                    Span::raw(format!("{check} {text}")),
                ])
                .style(theme::variant(Variant::Success).add_modifier(Modifier::BOLD))
            } else {
                let check_style = if enabled {
                    theme::variant(Variant::Success)
                } else {
                    theme::muted()
                };
                let text_style = if option.status.is_installed() {
                    theme::text()
                } else {
                    theme::muted()
                };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(check, check_style),
                    Span::styled(format!(" {text}"), text_style),
                ])
            });
        }
        lines.push(Line::styled(
            format!("  ‹ Página {}/{pages} ›", page + 1),
            theme::muted(),
        ));
        lines.push(Line::default());

        let option = &options[current];
        let state = if settings.is_enabled(&option.id) {
            Span::styled("ativa neste workspace", theme::variant(Variant::Success))
        } else {
            Span::styled("inativa", theme::muted())
        };
        let status = match &option.status {
            Status::Installed { version: Some(v) } => {
                Span::styled(format!("{} {v} · ", option.name), theme::text())
            }
            Status::Installed { version: None } => {
                Span::styled(format!("{} · ", option.name), theme::text())
            }
            Status::Missing => Span::styled(
                format!("{} indisponível (não instalada) · ", option.name),
                theme::error(),
            ),
        };
        lines.push(Line::from(vec![Span::raw("  "), status, state]));
        lines.push(Line::styled(
            format!("  {}", option.homepage),
            theme::path(),
        ));
        lines
    }
}

fn field_label(field: Field) -> &'static str {
    match field {
        Field::Account => "Conta",
        Field::StartMode => "Modo inicial",
        Field::Model => "Modelo",
        Field::Interface => "Interface",
        Field::ContextDisplay => "Display",
        Field::AutoCompact => "Auto compactação",
        Field::Limit => "Limite de tokens",
        Field::CompactAt => "Compactar em",
        Field::UsageDisplay => "Display",
        Field::TotalUsageDisplay => "Consumo geral",
        Field::Mcp => "Servidor",
        Field::McpSpawnMode => "Modo dos chats",
        Field::McpServers => "Conectados",
        Field::Skills => "Instaladas",
    }
}

fn value_label(field: Field, settings: &AgentSettings, choices: &Choices) -> String {
    let models = &choices.models;
    match field {
        Field::Account => choices
            .accounts
            .iter()
            .find(|a| a.id == settings.account)
            .map_or_else(
                || {
                    settings
                        .account
                        .clone()
                        .unwrap_or_else(|| "Default".to_string())
                },
                |a| a.label.clone(),
            ),
        Field::StartMode => settings.start_mode.label().to_string(),
        Field::Interface => settings.interface.label().to_string(),
        Field::ContextDisplay => if settings.context.visible {
            "Visível"
        } else {
            "Oculto"
        }
        .to_string(),
        Field::UsageDisplay => if settings.usage.visible {
            "Visível"
        } else {
            "Oculto"
        }
        .to_string(),
        Field::TotalUsageDisplay => if settings.usage.total_visible {
            "Visível"
        } else {
            "Oculto"
        }
        .to_string(),
        Field::AutoCompact => if settings.context.auto_compact {
            "Sim"
        } else {
            "Não"
        }
        .to_string(),
        Field::Limit => match settings.context.limit {
            None => format!("Do modelo · {}", compact_count(choices.context_window)),
            Some(limit) => compact_count(limit),
        },
        Field::CompactAt => {
            let context = settings.context;
            let tokens = context.compact_tokens(choices.context_window);
            format!("{}% · {} tokens", context.compact_at, compact_count(tokens))
        }
        Field::Mcp | Field::McpSpawnMode | Field::McpServers | Field::Skills => String::new(),
        Field::Model => match &settings.model {
            None => "Padrão".to_string(),
            Some(id) => models
                .iter()
                .find(|m| &m.id == id)
                .map_or_else(|| id.clone(), |m| m.label.clone()),
        },
    }
}

/// Avança (`step = 1`) ou volta (`-1`) o valor, dando a volta no fim.
fn change(field: Field, settings: &mut AgentSettings, choices: &Choices, step: isize) {
    let models = &choices.models;
    match field {
        Field::Account => {
            // Sem contas do provedor, só a Default.
            let mut options: Vec<Option<String>> =
                choices.accounts.iter().map(|a| a.id.clone()).collect();
            if options.is_empty() {
                options.push(None);
            }
            let i = options
                .iter()
                .position(|a| *a == settings.account)
                .unwrap_or(0);
            settings.account = options[cycle(i, options.len(), step)].clone();
        }
        Field::StartMode => {
            let all = StartMode::ALL;
            let i = all
                .iter()
                .position(|m| *m == settings.start_mode)
                .unwrap_or(0);
            settings.start_mode = all[cycle(i, all.len(), step)];
        }
        Field::Interface => {
            let all = Interface::ALL;
            let i = all
                .iter()
                .position(|m| *m == settings.interface)
                .unwrap_or(0);
            settings.interface = all[cycle(i, all.len(), step)];
        }
        Field::ContextDisplay => settings.context.visible = !settings.context.visible,
        Field::UsageDisplay => settings.usage.visible = !settings.usage.visible,
        Field::TotalUsageDisplay => {
            settings.usage.total_visible = !settings.usage.total_visible;
        }
        Field::AutoCompact => settings.context.auto_compact = !settings.context.auto_compact,
        Field::Limit => {
            // Do modelo (None) e depois os limites fixos. Um valor do arquivo
            // fora da lista volta para o do modelo.
            let options: Vec<Option<u64>> = std::iter::once(None)
                .chain(ContextSettings::LIMITS.map(Some))
                .collect();
            let limit = &mut settings.context.limit;
            let i = options.iter().position(|l| l == limit).unwrap_or(0);
            *limit = options[cycle(i, options.len(), step)];
        }
        Field::CompactAt => {
            let all = ContextSettings::COMPACT_AT;
            let at = &mut settings.context.compact_at;
            let i = all.iter().position(|p| p == at).unwrap_or(0);
            *at = all[cycle(i, all.len(), step)];
        }
        Field::Model => {
            // Padrão (None) e depois os modelos do provedor.
            let options: Vec<Option<String>> = std::iter::once(None)
                .chain(models.iter().map(|m| Some(m.id.clone())))
                .collect();
            let i = options
                .iter()
                .position(|m| *m == settings.model)
                .unwrap_or(0);
            settings.model = options[cycle(i, options.len(), step)].clone();
        }
        // Fica com a Config, que tem o MCP.
        Field::Mcp | Field::McpSpawnMode | Field::McpServers | Field::Skills => {}
    }
}

fn cycle(index: usize, len: usize, step: isize) -> usize {
    (index as isize + step).rem_euclid(len as isize) as usize
}

/// `text` com no máximo `width` caracteres, terminando em `…` quando corta.
fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut short: String = text.chars().take(width - 1).collect();
    short.push('…');
    short
}

/// A linha que separa as seções, com `width` colunas.
fn separator(width: usize) -> Line<'static> {
    Line::styled(
        format!(" {}", "─".repeat(width.saturating_sub(1))),
        theme::border(),
    )
}

fn label_span(label: &str) -> String {
    format!("{label:<LABEL_WIDTH$}")
}

/// Uma informação que não muda pela Config.
fn info_row(label: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(label_span(label), theme::muted()),
        value,
    ])
}

/// Uma configuração; em foco, com o marcador, o destaque e `‹ valor ›`.
fn field_row(label: &str, value: String, focused: bool) -> Line<'static> {
    if focused {
        Line::from(vec![
            Span::raw(format!("{} ", theme::FOCUS_MARKER)),
            Span::raw(label_span(label)),
            Span::raw(format!("‹ {value} ›")),
        ])
        .style(theme::focused())
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(label_span(label), theme::text()),
            Span::styled(format!("  {value}"), theme::path()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::PermissionMode;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn choices() -> Choices {
        Choices {
            accounts: vec![
                AccountOption {
                    id: None,
                    label: "Default · eu@gmail.com".into(),
                },
                AccountOption {
                    id: Some(".claude-empresa".into()),
                    label: "Trabalho · eu@empresa.com (Empresa)".into(),
                },
            ],
            models: ["opus", "haiku"]
                .iter()
                .map(|id| ModelOption {
                    id: id.to_string(),
                    label: id.to_uppercase(),
                })
                .collect(),
            context_window: 200_000,
            extensions: Vec::new(),
            mcp: McpState::default(),
            skills: Vec::new(),
        }
    }

    fn press(view: &mut ConfigView, settings: &mut AgentSettings, code: KeyCode) -> Outcome {
        view.handle_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            settings,
            &mut ExtensionSettings::default(),
            &mut McpSettings::default(),
            &choices(),
        )
    }

    fn extension_options(n: usize) -> Vec<ExtensionOption> {
        (1..=n)
            .map(|i| ExtensionOption {
                id: format!("ext{i}"),
                name: format!("Ext{i}"),
                description: format!("faz {i}"),
                homepage: format!("https://ext{i}"),
                status: if i == 2 {
                    Status::Missing
                } else {
                    Status::Installed {
                        version: Some("1.0".into()),
                    }
                },
            })
            .collect()
    }

    #[test]
    fn shift_tab_moves_to_extensions_where_space_toggles_and_arrows_page() {
        let mut view = ConfigView::default();
        let mut settings = AgentSettings::default();
        let mut extensions = ExtensionSettings::default();
        let choices = Choices {
            extensions: extension_options(7),
            ..choices()
        };
        let mut press = |view: &mut ConfigView, code: KeyCode| {
            view.handle_key(
                KeyEvent::new(code, KeyModifiers::NONE),
                &mut settings,
                &mut extensions,
                &mut McpSettings::default(),
                &choices,
            )
        };
        assert_eq!(press(&mut view, KeyCode::BackTab), Outcome::Moved);
        assert_eq!(view.help(), HELP_EXTENSIONS);
        assert_eq!(
            press(&mut view, KeyCode::Char(' ')),
            Outcome::ExtensionsChanged
        );
        press(&mut view, KeyCode::Down);
        // A segunda não está instalada: não liga.
        assert_eq!(press(&mut view, KeyCode::Char(' ')), Outcome::Moved);
        // → vai para a próxima página; ← volta para o começo da anterior.
        press(&mut view, KeyCode::Right);
        assert_eq!(view.extension, 5);
        press(&mut view, KeyCode::Right);
        assert_eq!(view.extension, 5, "na última página fica");
        press(&mut view, KeyCode::Down);
        press(&mut view, KeyCode::Down);
        assert_eq!(view.extension, 6, "a última fica");
        press(&mut view, KeyCode::Enter);
        press(&mut view, KeyCode::Left);
        assert_eq!(view.extension, 0);
        press(&mut view, KeyCode::Char(' '));
        assert_eq!(press(&mut view, KeyCode::Tab), Outcome::Ignored);
        assert_eq!(press(&mut view, KeyCode::BackTab), Outcome::Moved);
        assert_eq!(view.help(), HELP);
        assert_eq!(extensions.enabled, ["ext7"]);
        assert_eq!(settings, AgentSettings::default());
    }

    #[test]
    fn missing_extension_can_still_be_turned_off() {
        let mut view = ConfigView {
            section: Section::Extensions,
            extension: 1,
            ..ConfigView::default()
        };
        let mut extensions = ExtensionSettings {
            enabled: vec!["ext2".into()],
        };
        let outcome = view.handle_key(
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut AgentSettings::default(),
            &mut extensions,
            &mut McpSettings::default(),
            &Choices {
                extensions: extension_options(2),
                ..choices()
            },
        );
        assert_eq!(outcome, Outcome::ExtensionsChanged);
        assert!(extensions.enabled.is_empty());
    }

    #[test]
    fn draws_extensions_page_beside_the_agent() {
        let view = ConfigView {
            section: Section::Extensions,
            extension: 6,
            ..ConfigView::default()
        };
        let extensions = ExtensionSettings {
            enabled: vec!["ext7".into()],
        };
        let choices = Choices {
            extensions: extension_options(7),
            ..choices()
        };
        let mut terminal = Terminal::new(TestBackend::new(110, 15)).unwrap();
        terminal
            .draw(|frame| {
                let settings = AgentSettings::default();
                view.draw(frame, frame.area(), &settings, &extensions, None, &choices)
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..110)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert!(row(1).ends_with("  Extensões"), "{}", row(1));
        assert!(row(2).ends_with("[ ] 6. Ext6 - faz 6"), "{}", row(2));
        assert!(row(3).ends_with("▌ [x] 7. Ext7 - faz 7"), "{}", row(3));
        assert!(row(4).ends_with("‹ Página 2/2 ›"), "{}", row(4));
        assert!(
            row(6).ends_with("Ext7 1.0 · ativa neste workspace"),
            "{}",
            row(6)
        );
        assert!(row(7).ends_with("https://ext7"), "{}", row(7));
        // Sem foco no Agente: a Conta não tem o marcador.
        assert!(row(3).starts_with("   Conta"), "{}", row(3));
    }

    #[test]
    fn missing_extension_shows_as_unavailable() {
        let view = ConfigView {
            section: Section::Extensions,
            extension: 1,
            ..ConfigView::default()
        };
        let choices = Choices {
            extensions: extension_options(2),
            ..choices()
        };
        let lines = view.extension_lines(&ExtensionSettings::default(), &choices.extensions);
        let text = |line: &Line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        };
        assert_eq!(text(&lines[1]), "  [ ] 1. Ext1 - faz 1");
        assert_eq!(
            text(&lines[2]),
            format!("{} [ ] 2. Ext2 - faz 2 (indisponível)", theme::FOCUS_MARKER)
        );
        assert_eq!(
            text(&lines[5]),
            "  Ext2 indisponível (não instalada) · inativa"
        );
    }

    #[test]
    fn account_cycles_through_provider_accounts() {
        let mut view = ConfigView::default();
        let mut settings = AgentSettings::default();
        // A Conta é a primeira configuração.
        assert_eq!(
            press(&mut view, &mut settings, KeyCode::Right),
            Outcome::Changed
        );
        assert_eq!(settings.account.as_deref(), Some(".claude-empresa"));
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.account, None, "depois da última, volta à Default");
        assert_eq!(
            value_label(Field::Account, &settings, &choices()),
            "Default · eu@gmail.com"
        );
    }

    #[test]
    fn without_accounts_only_default_exists() {
        let mut settings = AgentSettings::default();
        change(Field::Account, &mut settings, &Choices::default(), 1);
        assert_eq!(settings.account, None);
        assert_eq!(
            value_label(Field::Account, &settings, &Choices::default()),
            "Default"
        );
    }

    #[test]
    fn arrows_change_start_mode_and_model() {
        let mut view = ConfigView::default();
        let mut settings = AgentSettings::default();
        // Pula a Conta, a primeira configuração.
        assert_eq!(
            press(&mut view, &mut settings, KeyCode::Down),
            Outcome::Moved
        );

        assert_eq!(
            press(&mut view, &mut settings, KeyCode::Right),
            Outcome::Changed
        );
        assert_eq!(
            settings.start_mode,
            StartMode::Fixed(PermissionMode::Agente)
        );
        press(&mut view, &mut settings, KeyCode::Left);
        press(&mut view, &mut settings, KeyCode::Left);
        // Dá a volta: do primeiro para o último.
        assert_eq!(settings.start_mode, StartMode::Fixed(PermissionMode::Plan));

        assert_eq!(
            press(&mut view, &mut settings, KeyCode::Down),
            Outcome::Moved
        );
        press(&mut view, &mut settings, KeyCode::Enter);
        assert_eq!(settings.model.as_deref(), Some("opus"));
        press(&mut view, &mut settings, KeyCode::Right);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.model, None, "depois do último, volta ao Padrão");

        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.interface, Interface::Provider);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.interface, Interface::Terraforme, "dá a volta");

        assert_eq!(
            press(&mut view, &mut settings, KeyCode::Tab),
            Outcome::Ignored
        );
    }

    #[test]
    fn context_settings_toggle_and_cycle() {
        let mut view = ConfigView {
            focus: CONTEXT_START,
            ..ConfigView::default()
        };
        let mut settings = AgentSettings::default();
        press(&mut view, &mut settings, KeyCode::Enter);
        assert!(settings.context.visible, "Oculto → Visível");
        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Right);
        assert!(settings.context.auto_compact, "Não → Sim");

        press(&mut view, &mut settings, KeyCode::Down);
        assert_eq!(value_label(Field::Limit, &settings, &choices()), "200K");
        for _ in 0..4 {
            press(&mut view, &mut settings, KeyCode::Left);
        }
        assert_eq!(
            value_label(Field::Limit, &settings, &choices()),
            "Do modelo · 200K"
        );
        press(&mut view, &mut settings, KeyCode::Left);
        assert_eq!(settings.context.limit, Some(1_000_000), "dá a volta");
        press(&mut view, &mut settings, KeyCode::Right);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.context.limit, Some(50_000));
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(value_label(Field::Limit, &settings, &choices()), "100K");

        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.context.compact_at, 80);
        // O ponto de compactar é do limite escolhido, não do modelo.
        assert_eq!(
            value_label(Field::CompactAt, &settings, &choices()),
            "80% · 80K tokens"
        );
        for _ in 0..7 {
            press(&mut view, &mut settings, KeyCode::Left);
        }
        assert_eq!(settings.context.compact_at, 95, "dá a volta");

        // A seção Consumo.
        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Enter);
        assert!(settings.usage.visible, "Oculto → Visível");
    }

    #[test]
    fn draws_agent_section_with_focus() {
        let view = ConfigView::default();
        let settings = AgentSettings {
            start_mode: StartMode::Provider,
            model: Some("haiku".into()),
            account: None,
            effort: None,
            interface: Interface::Provider,
            context: ContextSettings::default(),
            usage: Default::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 22)).unwrap();
        terminal
            .draw(|frame| {
                let extensions = ExtensionSettings::default();
                view.draw(
                    frame,
                    frame.area(),
                    &settings,
                    &extensions,
                    None,
                    &Choices {
                        mcp: McpState {
                            settings: McpSettings {
                                enabled: true,
                                ..McpSettings::default()
                            },
                            online: true,
                            ..McpState::default()
                        },
                        ..choices()
                    },
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..60)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(1), "  Agente");
        assert_eq!(row(2), "   Provedor          Nenhum");
        assert_eq!(row(3), " ▌ Conta             ‹ Default · eu@gmail.com ›");
        assert_eq!(row(4), "   Modo inicial        Do provedor");
        assert_eq!(row(5), "   Modelo              HAIKU");
        assert_eq!(row(6), "   Interface           Original do provedor");
        assert_eq!(buffer[(3, 3)].bg, theme::focused().bg.unwrap());
        // A linha que separa o Contexto vai até onde vai a seção.
        assert!(row(7).starts_with("  ─"));
        assert_eq!(row(7).chars().count(), row(3).chars().count());
        assert_eq!(row(8), "  Contexto");
        assert_eq!(row(9), "   Display             Oculto");
        assert_eq!(row(10), "   Auto compactação    Não");
        assert_eq!(row(11), "   Limite de tokens    200K");
        assert_eq!(row(12), "   Compactar em        75% · 150K tokens");
        assert!(row(13).starts_with("  ─"));
        assert_eq!(row(14), "  Consumo");
        assert_eq!(row(15), "   Display             Oculto");
        assert_eq!(row(16), "   Consumo geral       Visível");
        // O MCP, separado, com a bola verde de online.
        assert_eq!(row(17), "");
        assert_eq!(row(18), "  MCP ●");
        assert_eq!(row(19), "   Servidor            Ativado");
        assert_eq!(row(20), "   Modo dos chats      Accept Edits");
        assert_eq!(row(21), "   Endereço          http://127.0.0.1:30303/mcp");
    }

    #[test]
    fn spawn_mode_cycles_and_asks_to_save() {
        let mut view = ConfigView {
            focus: MCP_START + 1,
            ..ConfigView::default()
        };
        let mut mcp = McpSettings::default();
        let outcome = view.handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &mut AgentSettings::default(),
            &mut ExtensionSettings::default(),
            &mut mcp,
            &choices(),
        );
        assert_eq!(outcome, Outcome::McpSpawnModeChanged);
        assert_eq!(mcp.spawn_mode, StartMode::Fixed(PermissionMode::Plan));
        assert!(!mcp.enabled, "não liga nem desliga o MCP");
    }

    #[test]
    fn arrows_walk_through_the_connected_mcp_servers() {
        let server = |name: &str, origin: &str| McpServerInfo {
            name: name.into(),
            origin: origin.into(),
            detail: format!("{name} serve"),
        };
        let choices = Choices {
            mcp: McpState {
                servers: vec![
                    server("ai-memory", "Claude · usuário"),
                    server("terra", "terraforme"),
                ],
                ..McpState::default()
            },
            ..choices()
        };
        let mut view = ConfigView {
            focus: MCP_START + 2,
            ..ConfigView::default()
        };
        let press = |view: &mut ConfigView, code: KeyCode| {
            view.handle_key(
                KeyEvent::new(code, KeyModifiers::NONE),
                &mut AgentSettings::default(),
                &mut ExtensionSettings::default(),
                &mut McpSettings::default(),
                &choices,
            )
        };
        let text = |view: &ConfigView| -> Vec<String> {
            view.mcp_server_lines(&choices.mcp.servers, true)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        assert!(text(&view)[0].contains("‹ ai-memory · 1/2 ›"));
        assert!(text(&view)[1].contains("Claude · usuário"));
        assert_eq!(press(&mut view, KeyCode::Right), Outcome::Moved);
        assert!(text(&view)[0].contains("‹ terra · 2/2 ›"));
        assert!(text(&view)[2].contains("terra serve"));
        press(&mut view, KeyCode::Right);
        assert!(text(&view)[0].contains("ai-memory · 1/2"), "dá a volta");
        press(&mut view, KeyCode::Left);
        assert!(text(&view)[0].contains("terra · 2/2"));
    }

    #[test]
    fn skills_are_the_last_setting_and_toggle_mcp_above() {
        let mut view = ConfigView::default();
        let mut mcp = McpSettings::default();
        let mut press = |view: &mut ConfigView, code: KeyCode| {
            view.handle_key(
                KeyEvent::new(code, KeyModifiers::NONE),
                &mut AgentSettings::default(),
                &mut ExtensionSettings::default(),
                &mut mcp,
                &choices(),
            )
        };
        for _ in 0..FIELDS.len() {
            press(&mut view, KeyCode::Down);
        }
        assert_eq!(view.focus, MCP_START + 3, "a última fica");
        assert_eq!(FIELDS[view.focus], Field::Skills);
        for _ in 0..3 {
            press(&mut view, KeyCode::Up);
        }
        assert_eq!(press(&mut view, KeyCode::Enter), Outcome::McpChanged);
        assert_eq!(press(&mut view, KeyCode::Up), Outcome::Moved);
        assert!(mcp.enabled, "Desativado → Ativado");
    }

    #[test]
    fn arrows_walk_through_the_installed_skills() {
        let skill = |name: &str| SkillInfo {
            name: name.into(),
            origin: "Claude · usuário".into(),
            description: format!("{name} faz algo"),
        };
        let choices = Choices {
            skills: vec![skill("docs"), skill("postman:api-mocking")],
            ..choices()
        };
        let mut view = ConfigView {
            focus: MCP_START + 3,
            ..ConfigView::default()
        };
        let text = |view: &ConfigView| -> Vec<String> {
            view.skill_lines(&choices.skills, true)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        assert!(text(&view)[1].contains("‹ docs · 1/2 ›"));
        let outcome = view.handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &mut AgentSettings::default(),
            &mut ExtensionSettings::default(),
            &mut McpSettings::default(),
            &choices,
        );
        assert_eq!(outcome, Outcome::Moved);
        assert!(text(&view)[1].contains("postman:api-mocking · 2/2"));
        assert!(text(&view)[3].contains("postman:api-mocking faz algo"));
        let empty: String = ConfigView::default().skill_lines(&[], false)[1]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(empty.trim_end().ends_with("Nenhuma"));
    }
}
