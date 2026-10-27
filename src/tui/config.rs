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

use crate::agent::{ChatAdapter, ModelOption};
use crate::settings::{AgentSettings, StartMode};
use crate::theme::{self, Variant};

pub const HELP: &str = "↑/↓ escolher · ←/→ ou Enter alterar · Tab trocar aba · Esc ir para Sair";
/// Largura da coluna dos nomes das configurações.
const LABEL_WIDTH: usize = 16;

/// Uma configuração que pode ser alterada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    StartMode,
    Model,
}

const FIELDS: [Field; 2] = [Field::StartMode, Field::Model];

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
        models: &[ModelOption],
    ) -> Outcome {
        let last = FIELDS.len() - 1;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.focus = self.focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.focus = (self.focus + 1).min(last),
            KeyCode::Right | KeyCode::Enter | KeyCode::Char('l') => {
                change(FIELDS[self.focus], settings, models, 1);
                return Outcome::Changed;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                change(FIELDS[self.focus], settings, models, -1);
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
    ) {
        let models = adapter.map(|a| a.models()).unwrap_or_default();
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
            let value = value_label(field, settings, &models);
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
        Field::StartMode => "Modo inicial",
        Field::Model => "Modelo",
    }
}

fn value_label(field: Field, settings: &AgentSettings, models: &[ModelOption]) -> String {
    match field {
        Field::StartMode => settings.start_mode.label().to_string(),
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
fn change(field: Field, settings: &mut AgentSettings, models: &[ModelOption], step: isize) {
    match field {
        Field::StartMode => {
            let all = StartMode::ALL;
            let i = all
                .iter()
                .position(|m| *m == settings.start_mode)
                .unwrap_or(0);
            settings.start_mode = all[cycle(i, all.len(), step)];
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

    fn models() -> Vec<ModelOption> {
        ["opus", "haiku"]
            .iter()
            .map(|id| ModelOption {
                id: id.to_string(),
                label: id.to_uppercase(),
            })
            .collect()
    }

    fn press(view: &mut ConfigView, settings: &mut AgentSettings, code: KeyCode) -> Outcome {
        view.handle_key(KeyEvent::new(code, KeyModifiers::NONE), settings, &models())
    }

    #[test]
    fn arrows_change_start_mode_and_model() {
        let mut view = ConfigView::default();
        let mut settings = AgentSettings::default();

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
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 6)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), &settings, None))
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
        assert_eq!(row(3), " ▌ Modo inicial    ‹ Do provedor ›");
        // Sem provedor, o id salvo aparece como está.
        assert_eq!(row(4), "   Modelo            haiku");
        assert_eq!(buffer[(3, 3)].bg, theme::focused().bg.unwrap());
    }
}
