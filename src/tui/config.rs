//! Aba Config: as configurações do terra, em seções (como as seções Recentes
//! e Salvos do navegador de workspaces). Hoje há a seção Agente.
//!
//! `↑`/`↓` escolhem a configuração e `←`/`→` ou `Enter` trocam o valor; a
//! tela principal salva e aplica a mudança.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use crate::agent::{AccountOption, ChatAdapter, ModelOption};
use crate::settings::{AgentSettings, Interface, StartMode};
use crate::theme::{self, Variant};

pub const HELP: &str = "↑/↓ escolher · ←/→ ou Enter alterar · Tab trocar aba · Esc ir para Sair";
/// Largura da coluna dos nomes das configurações.
const LABEL_WIDTH: usize = 16;

/// Uma configuração que pode ser alterada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Account,
    StartMode,
    Model,
    Interface,
}

const FIELDS: [Field; 4] = [
    Field::Account,
    Field::StartMode,
    Field::Model,
    Field::Interface,
];

/// As opções que vêm do provedor: contas e modelos.
#[derive(Debug, Default, Clone)]
pub struct Choices {
    pub accounts: Vec<AccountOption>,
    pub models: Vec<ModelOption>,
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
        for (i, field) in FIELDS.into_iter().enumerate() {
            let value = value_label(field, settings, choices);
            lines.push(field_row(field_label(field), value, i == self.focus));
        }
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
    fn draws_agent_section_with_focus() {
        let view = ConfigView::default();
        let settings = AgentSettings {
            start_mode: StartMode::Provider,
            model: Some("haiku".into()),
            account: None,
            effort: None,
            interface: Interface::Provider,
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), &settings, None, &Choices::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..50)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(1), "  Agente");
        assert_eq!(row(2), "   Provedor        Nenhum");
        assert_eq!(row(3), " ▌ Conta           ‹ Default ›");
        assert_eq!(row(4), "   Modo inicial      Do provedor");
        // Sem provedor, o id salvo aparece como está.
        assert_eq!(row(5), "   Modelo            haiku");
        assert_eq!(row(6), "   Interface         Original do provedor");
        assert_eq!(buffer[(3, 3)].bg, theme::focused().bg.unwrap());
    }
}
