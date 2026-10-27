//! Aba Config: as configurações do terra, em seções (como as seções Recentes
//! e Salvos do navegador de workspaces). Hoje há a seção Agente e, dentro
//! dela, separada por uma linha, a do Contexto.
//!
//! `↑`/`↓` escolhem a configuração e `←`/`→` ou `Enter` trocam o valor; a
//! tela principal salva e aplica a mudança.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use super::chat::compact_count;
use crate::agent::{AccountOption, ChatAdapter, ModelOption};
use crate::settings::{AgentSettings, ContextSettings, Interface, StartMode};
use crate::theme::{self, Variant};

pub const HELP: &str = "↑/↓ escolher · ←/→ ou Enter alterar · Tab trocar aba · Esc ir para Sair";
/// Largura da coluna dos nomes das configurações.
const LABEL_WIDTH: usize = 18;

/// Uma configuração que pode ser alterada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Account,
    StartMode,
    Model,
    Interface,
    ContextDisplay,
    AutoCompact,
    CompactAt,
}

const FIELDS: [Field; 7] = [
    Field::Account,
    Field::StartMode,
    Field::Model,
    Field::Interface,
    Field::ContextDisplay,
    Field::AutoCompact,
    Field::CompactAt,
];
/// Onde começa a seção Contexto em [`FIELDS`].
const CONTEXT_START: usize = 4;

/// As opções que vêm do provedor: contas e modelos, e o limite de contexto
/// do modelo (para mostrar em tokens o ponto de compactar).
#[derive(Debug, Default, Clone)]
pub struct Choices {
    pub accounts: Vec<AccountOption>,
    pub models: Vec<ModelOption>,
    pub context_window: u64,
}

/// O que a Config fez com uma tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Moved,
    /// Um valor mudou: salvar e aplicar.
    Changed,
}

#[derive(Default)]
pub struct ConfigView {
    /// Posição em [`FIELDS`].
    focus: usize,
}

impl ConfigView {
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        settings: &mut AgentSettings,
        choices: &Choices,
    ) -> Outcome {
        let last = FIELDS.len() - 1;
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

    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        settings: &AgentSettings,
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
        let row = |i: usize, field: Field| {
            let value = value_label(field, settings, choices);
            field_row(field_label(field), value, i == self.focus)
        };
        let (agent, context) = FIELDS.split_at(CONTEXT_START);
        lines.extend(agent.iter().enumerate().map(|(i, f)| row(i, *f)));
        let context: Vec<Line> = context
            .iter()
            .enumerate()
            .map(|(i, f)| row(CONTEXT_START + i, *f))
            .collect();
        // A linha vai só até onde vai a seção, e não a tela inteira.
        let width = lines.iter().chain(&context).map(Line::width).max();
        lines.push(separator(width.unwrap_or(0)));
        lines.push(Line::styled(" Contexto", theme::section_title()));
        lines.extend(context);
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
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
        Field::CompactAt => "Compactar em",
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
        Field::AutoCompact => if settings.context.auto_compact {
            "Sim"
        } else {
            "Não"
        }
        .to_string(),
        Field::CompactAt => {
            let context = settings.context;
            let tokens = context.compact_tokens(choices.context_window);
            format!("{}% · {} tokens", context.compact_at, compact_count(tokens))
        }
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
        Field::AutoCompact => settings.context.auto_compact = !settings.context.auto_compact,
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
    }
}

fn cycle(index: usize, len: usize, step: isize) -> usize {
    (index as isize + step).rem_euclid(len as isize) as usize
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
                    id: Some(".claude-di2win".into()),
                    label: "Trabalho · eu@di2win.com (Di2win)".into(),
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
        }
    }

    fn press(view: &mut ConfigView, settings: &mut AgentSettings, code: KeyCode) -> Outcome {
        view.handle_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            settings,
            &choices(),
        )
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
        assert_eq!(settings.account.as_deref(), Some(".claude-di2win"));
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
        };
        let mut settings = AgentSettings::default();
        press(&mut view, &mut settings, KeyCode::Enter);
        assert!(!settings.context.visible, "Visível → Oculto");
        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Right);
        assert!(settings.context.auto_compact, "Não → Sim");
        press(&mut view, &mut settings, KeyCode::Down);
        press(&mut view, &mut settings, KeyCode::Right);
        assert_eq!(settings.context.compact_at, 80);
        assert_eq!(
            value_label(Field::CompactAt, &settings, &choices()),
            "80% · 160K tokens"
        );
        press(&mut view, &mut settings, KeyCode::Down);
        assert_eq!(view.focus, FIELDS.len() - 1, "a última fica");
        for _ in 0..7 {
            press(&mut view, &mut settings, KeyCode::Left);
        }
        assert_eq!(settings.context.compact_at, 95, "dá a volta");
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
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), &settings, None, &choices()))
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
        assert_eq!(row(9), "   Display             Visível");
        assert_eq!(row(10), "   Auto compactação    Não");
        assert_eq!(row(11), "   Compactar em        75% · 150K tokens");
    }
}
