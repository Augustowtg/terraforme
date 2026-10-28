//! Interface do terra para o `/model`: escolher o modelo e o esforço.
//!
//! A lista de modelos e os níveis de esforço de cada um vêm do agente (o
//! catálogo do adapter); a tela é do terra. Abre acima do input:
//!
//! ```text
//! Selecione o modelo                       (amarelo)
//! Escolha o modelo desta conversa...       (descrição)
//! ▌ Default (Opus - 5.5)                   (em foco: verde)
//!   1. Opus - 5.5 - For complex work
//!   ...                                    (até 5)
//!   ... 12 modelos disponíveis
//! Esforço  ████████████░░░░░░░░  alto     (barra verde)
//! ```
//!
//! `Enter` no Default abre o mesmo menu para o modelo padrão (salvo na
//! Config); num modelo, escolhe o modelo desta conversa. A Config abre o
//! mesmo menu para o modelo dos sub agentes, sem o esforço, e a aba
//! Orquestrador para o modelo e o esforço dos chats delegados.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::agent::{Catalog, ModelInfo};
use crate::theme::{self, Variant};

/// Modelos visíveis de cada vez.
const VISIBLE: usize = 5;
/// Colunas de cada nível na barra de esforço.
const LEVEL_WIDTH: usize = 4;
/// Esforço escolhido quando o modelo aceita e nada foi escolhido antes.
const PREFERRED_EFFORT: &str = "high";

/// Para que o menu escolhe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// O modelo desta conversa (o `/model`).
    Session,
    /// O modelo padrão, salvo na Config (o Default do `/model`).
    Default,
    /// O modelo dos sub agentes, salvo na Config (aberto pela Config).
    Subagent,
    /// O modelo dos chats delegados, salvo no `config.json` global (aberto
    /// pela aba Orquestrador).
    Delegated,
}

/// O que o menu fez com uma tecla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Handled,
    Close,
    /// Escolheu `model` (um id do catálogo) com o esforço `effort`.
    Chosen {
        target: Target,
        model: String,
        effort: Option<String>,
    },
    /// Escolheu o Default: o alvo volta ao padrão do agente.
    Unset(Target),
}

/// Um item da lista: o Default (só no menu da conversa) ou um modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Default,
    Model(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMenu {
    target: Target,
    selected: usize,
    /// Nível de esforço escolhido (um nome, ex.: `high`).
    effort: Option<String>,
    /// O modelo padrão (id), para mostrar no Default e começar nele.
    default_model: Option<String>,
}

impl ModelMenu {
    /// Abre o menu da conversa. `default_model` e `effort` são o padrão
    /// atual e o esforço em uso.
    pub fn new(default_model: Option<String>, effort: Option<String>) -> Self {
        Self {
            target: Target::Session,
            selected: 0,
            effort,
            default_model,
        }
    }

    /// Abre o menu do modelo dos sub agentes, com o foco em `current`
    /// (`None` é o Default).
    pub fn subagents(current: Option<String>, catalog: Option<&Catalog>) -> Self {
        let models = catalog.map_or(&[][..], |c| &c.models[..]);
        let selected = current
            .as_ref()
            .and_then(|id| models.iter().position(|m| matches_id(m, id)))
            .map_or(0, |i| i + 1);
        Self {
            target: Target::Subagent,
            selected,
            effort: None,
            default_model: None,
        }
    }

    /// Abre o menu do modelo dos chats delegados, com o foco em `current`
    /// (`None` é o Default, que mostra `default_model`) e o esforço `effort`.
    pub fn delegated(
        current: Option<String>,
        effort: Option<String>,
        default_model: Option<String>,
        catalog: Option<&Catalog>,
    ) -> Self {
        let models = catalog.map_or(&[][..], |c| &c.models[..]);
        let selected = current
            .as_ref()
            .and_then(|id| models.iter().position(|m| matches_id(m, id)))
            .map_or(0, |i| i + 1);
        Self {
            target: Target::Delegated,
            selected,
            effort,
            default_model,
        }
    }

    /// Ajusta o menu ao catálogo (ao abrir e quando ele chega): o esforço
    /// passa a ser um que o modelo em foco aceite.
    pub fn sync(&mut self, catalog: Option<&Catalog>) {
        let models = catalog.map_or(&[][..], |c| &c.models[..]);
        self.fit_effort(models);
    }

    fn items(&self, models: &[ModelInfo]) -> Vec<Item> {
        let default = (self.target != Target::Default).then_some(Item::Default);
        default
            .into_iter()
            .chain((0..models.len()).map(Item::Model))
            .collect()
    }

    /// O modelo do item: o próprio, ou o padrão para o Default.
    fn model_of<'a>(&self, item: Item, models: &'a [ModelInfo]) -> Option<&'a ModelInfo> {
        match item {
            Item::Model(i) => models.get(i),
            Item::Default => self
                .default_model
                .as_ref()
                .and_then(|id| models.iter().find(|m| matches_id(m, id))),
        }
    }

    fn focused<'a>(&self, models: &'a [ModelInfo]) -> (Option<Item>, Option<&'a ModelInfo>) {
        let item = self.items(models).get(self.selected).copied();
        (item, item.and_then(|i| self.model_of(i, models)))
    }

    /// Garante um esforço que o modelo em foco aceite.
    fn fit_effort(&mut self, models: &[ModelInfo]) {
        if self.target == Target::Subagent {
            return;
        }
        let Some(model) = self.focused(models).1 else {
            return;
        };
        if model.efforts.is_empty() {
            return;
        }
        let supported = self
            .effort
            .as_ref()
            .is_some_and(|e| model.efforts.contains(e));
        if !supported {
            self.effort = model
                .efforts
                .iter()
                .find(|e| *e == PREFERRED_EFFORT)
                .or(model.efforts.last())
                .cloned();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, catalog: Option<&Catalog>) -> Outcome {
        let models = catalog.map_or(&[][..], |c| &c.models[..]);
        let total = self.items(models).len();
        match key.code {
            KeyCode::Esc if self.target == Target::Default => {
                // Volta ao menu da conversa, com o foco no Default.
                self.target = Target::Session;
                self.selected = 0;
            }
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') if total > 0 => {
                self.selected = (self.selected + total - 1) % total;
            }
            KeyCode::Down | KeyCode::Char('j') if total > 0 => {
                self.selected = (self.selected + 1) % total;
            }
            KeyCode::Left | KeyCode::Char('h') => self.step_effort(models, -1),
            KeyCode::Right | KeyCode::Char('l') => self.step_effort(models, 1),
            KeyCode::Enter => match self.focused(models) {
                (Some(Item::Default), _)
                    if matches!(self.target, Target::Subagent | Target::Delegated) =>
                {
                    return Outcome::Unset(self.target);
                }
                (Some(Item::Default), _) => {
                    self.target = Target::Default;
                    // Começa no modelo padrão atual.
                    self.selected = self
                        .default_model
                        .as_ref()
                        .and_then(|id| models.iter().position(|m| matches_id(m, id)))
                        .unwrap_or(0);
                }
                (Some(Item::Model(_)), Some(model)) => {
                    let effort = self
                        .effort
                        .clone()
                        .filter(|e| model.efforts.contains(e) && self.target != Target::Subagent);
                    return Outcome::Chosen {
                        target: self.target,
                        model: model.id.clone(),
                        effort,
                    };
                }
                _ => {}
            },
            _ => {}
        }
        self.fit_effort(models);
        Outcome::Handled
    }

    fn step_effort(&mut self, models: &[ModelInfo], step: isize) {
        if self.target == Target::Subagent {
            return;
        }
        let Some(model) = self.focused(models).1 else {
            return;
        };
        let levels = &model.efforts;
        if levels.is_empty() {
            return;
        }
        let current = self
            .effort
            .as_ref()
            .and_then(|e| levels.iter().position(|l| l == e))
            .unwrap_or(0);
        let next = (current as isize + step).clamp(0, levels.len() as isize - 1) as usize;
        self.effort = Some(levels[next].clone());
    }

    /// Desenha acima de `input`. `label` dá o nome do modelo para a tela
    /// (ex.: `Opus - 5.5`).
    pub fn draw(
        &self,
        frame: &mut Frame,
        input: Rect,
        catalog: Option<&Catalog>,
        label: &dyn Fn(&ModelInfo) -> String,
    ) {
        let models = catalog.map_or(&[][..], |c| &c.models[..]);
        let (title, description) = match self.target {
            Target::Session => (
                "Selecione o modelo",
                "Escolha o modelo desta conversa. Default usa o modelo padrão.",
            ),
            Target::Default => (
                "Selecione o modelo default",
                "O modelo padrão vale para as próximas conversas e fica salvo nas Configurações.",
            ),
            Target::Subagent => (
                "Selecione o modelo dos sub agentes",
                "Os sub agentes usam este modelo e ele fica salvo nas Configurações.",
            ),
            Target::Delegated => (
                "Selecione o modelo dos chats delegados",
                "Os chats abertos pelo orquestrador usam este modelo. Default usa o modelo padrão.",
            ),
        };
        let mut lines = vec![
            Line::styled(
                title,
                theme::variant(Variant::Warning).add_modifier(Modifier::BOLD),
            ),
            Line::styled(description, theme::path()),
            Line::default(),
        ];

        let items = self.items(models);
        if catalog.is_none() {
            lines.push(Line::styled(
                "Carregando os modelos do agente...",
                theme::muted(),
            ));
        } else if models.is_empty() {
            lines.push(Line::styled(
                "Nenhum modelo informado pelo agente.",
                theme::muted(),
            ));
        }
        let first = self.selected.saturating_sub(VISIBLE - 1);
        for (i, item) in items.iter().enumerate().skip(first).take(VISIBLE) {
            let text = match item {
                Item::Default if self.target == Target::Subagent => {
                    "Default (o agente escolhe)".to_string()
                }
                Item::Default => {
                    let current = self
                        .model_of(*item, models)
                        .map_or("padrão do agente".to_string(), label);
                    format!("Default ({current})")
                }
                Item::Model(index) => {
                    let model = &models[*index];
                    format!("{}. {} - {}", index + 1, label(model), model.description)
                }
            };
            lines.push(if i == self.selected {
                Line::from(vec![
                    Span::raw(format!("{} ", theme::FOCUS_MARKER)),
                    Span::raw(text),
                ])
                .style(theme::variant(Variant::Success).add_modifier(Modifier::BOLD))
            } else {
                Line::from(vec![Span::raw("  "), Span::styled(text, theme::text())])
            });
        }
        if items.len() > VISIBLE {
            lines.push(Line::styled(
                format!("  ... {} modelos disponíveis", models.len()),
                theme::muted(),
            ));
        }
        // O esforço dos sub agentes não é ajustável.
        if self.target != Target::Subagent {
            lines.push(Line::default());
            lines.push(self.effort_line(self.focused(models).1));
        }

        let height = (lines.len() as u16 + 2).min(input.y);
        if height < 3 {
            return;
        }
        let area = Rect {
            y: input.y - height,
            height,
            ..input
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::border())
            .padding(Padding::horizontal(1))
            .style(theme::screen());
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// `Esforço  ████████░░░░  alto`, com os níveis do modelo em foco.
    fn effort_line(&self, model: Option<&ModelInfo>) -> Line<'static> {
        let label = Span::styled("Esforço  ", theme::text());
        let levels = model.map_or(&[][..], |m| &m.efforts[..]);
        if levels.is_empty() {
            return Line::from(vec![
                label,
                Span::styled("não ajustável neste modelo", theme::muted()),
            ]);
        }
        let filled = self
            .effort
            .as_ref()
            .and_then(|e| levels.iter().position(|l| l == e))
            .map_or(0, |i| i + 1);
        let bar_on = "█".repeat(filled * LEVEL_WIDTH);
        let bar_off = "░".repeat((levels.len() - filled) * LEVEL_WIDTH);
        let name = self.effort.as_deref().map_or("", effort_label);
        Line::from(vec![
            label,
            Span::styled(bar_on, theme::variant(Variant::Success)),
            Span::styled(bar_off, theme::muted()),
            Span::styled(format!("  {name}"), theme::variant(Variant::Success)),
            Span::styled("   ←/→", theme::muted()),
        ])
    }
}

/// O modelo é `id`, pelo apelido (`opus`) ou pelo nome real
/// (`claude-opus-5-5`, como o agente informa o padrão).
fn matches_id(model: &ModelInfo, id: &str) -> bool {
    model.id == id || model.resolved.as_deref() == Some(id)
}

/// Nome do nível de esforço para a tela.
pub fn effort_label(effort: &str) -> &str {
    match effort {
        "low" => "baixo",
        "medium" => "médio",
        "high" => "alto",
        "xhigh" => "muito alto",
        "max" => "máximo",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn model(id: &str, efforts: &[&str]) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: id.into(),
            description: format!("descrição de {id}"),
            resolved: None,
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
        }
    }

    fn catalog() -> Catalog {
        let all = ["low", "medium", "high", "xhigh", "max"];
        Catalog {
            models: vec![
                model("opus", &all),
                model("sonnet", &all),
                model("fable", &all),
                model("haiku", &all),
                model("old-haiku", &[]),
                model("opus-4-6", &["low", "medium", "high", "max"]),
                model("sonnet-4-6", &["low", "medium", "high", "max"]),
            ],
            ..Catalog::default()
        }
    }

    fn press(menu: &mut ModelMenu, code: KeyCode) -> Outcome {
        menu.handle_key(KeyEvent::new(code, KeyModifiers::NONE), Some(&catalog()))
    }

    fn screen(menu: &ModelMenu) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(70, 22)).unwrap();
        terminal
            .draw(|frame| {
                let input = Rect::new(0, 20, 70, 2);
                menu.draw(frame, input, Some(&catalog()), &|m: &ModelInfo| {
                    m.label.clone()
                });
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..20)
            .map(|y| {
                (0..70)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .filter(|l| !l.is_empty())
            .collect()
    }

    #[test]
    fn draws_header_default_up_to_five_models_count_and_effort() {
        let menu = ModelMenu::new(Some("opus".into()), Some("high".into()));
        let lines = screen(&menu);
        let text = lines.join("\n");
        assert!(lines[1].contains("Selecione o modelo"), "{text}");
        assert!(text.contains("▌ Default (opus)"));
        assert!(text.contains("1. opus - descrição de opus"));
        assert!(text.contains("4. haiku - descrição de haiku"));
        assert!(!text.contains("5. old-haiku"), "só 5 itens visíveis");
        assert!(text.contains("... 7 modelos disponíveis"));
        // Default aponta para o opus, com esforço alto: 3 de 5 níveis.
        assert!(text.contains(&format!(
            "Esforço  {}{}  alto",
            "█".repeat(12),
            "░".repeat(8)
        )));
    }

    #[test]
    fn header_is_yellow_and_focus_is_green() {
        let menu = ModelMenu::new(None, None);
        let mut terminal = Terminal::new(TestBackend::new(70, 22)).unwrap();
        terminal
            .draw(|frame| {
                menu.draw(
                    frame,
                    Rect::new(0, 20, 70, 2),
                    Some(&catalog()),
                    &|m: &ModelInfo| m.label.clone(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let find = |text: &str| {
            (0..20)
                .find_map(|y| {
                    let row: String = (0..70).map(|x| buffer[(x, y)].symbol()).collect();
                    row.find(text).map(|x| (row[..x].chars().count() as u16, y))
                })
                .unwrap()
        };
        let (x, y) = find("Selecione");
        assert_eq!(
            buffer[(x, y)].fg,
            theme::variant(Variant::Warning).fg.unwrap()
        );
        let (x, y) = find("Default (");
        assert_eq!(
            buffer[(x, y)].fg,
            theme::variant(Variant::Success).fg.unwrap()
        );
    }

    #[test]
    fn opening_fills_the_effort_of_the_focused_model() {
        let mut menu = ModelMenu::new(Some("claude-opus-5-5".into()), None);
        menu.sync(Some(&Catalog {
            models: vec![ModelInfo {
                resolved: Some("claude-opus-5-5".into()),
                ..model("opus", &["low", "high", "max"])
            }],
            ..Catalog::default()
        }));
        // O Default aponta para o opus pelo nome real; esforço alto (2 de 3).
        assert_eq!(menu.effort.as_deref(), Some("high"));
    }

    #[test]
    fn enter_on_model_chooses_it_with_effort() {
        let mut menu = ModelMenu::new(None, None);
        press(&mut menu, KeyCode::Down); // 1. opus
        press(&mut menu, KeyCode::Right); // high -> xhigh
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Chosen {
                target: Target::Session,
                model: "opus".into(),
                effort: Some("xhigh".into())
            }
        );
    }

    #[test]
    fn effort_follows_the_levels_of_each_model() {
        let mut menu = ModelMenu::new(None, Some("xhigh".into()));
        for _ in 0..6 {
            press(&mut menu, KeyCode::Down); // 6. opus-4-6, sem xhigh
        }
        assert_eq!(menu.effort.as_deref(), Some("high"));
        press(&mut menu, KeyCode::Right);
        press(&mut menu, KeyCode::Right);
        assert_eq!(menu.effort.as_deref(), Some("max"), "para no último nível");
        // Um modelo sem esforço escolhe sem esforço.
        press(&mut menu, KeyCode::Up); // 5. old-haiku
        let text = screen(&menu).join("\n");
        assert!(
            text.contains("Esforço  não ajustável neste modelo"),
            "{text}"
        );
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Chosen {
                target: Target::Session,
                model: "old-haiku".into(),
                effort: None
            }
        );
    }

    #[test]
    fn default_opens_the_default_menu_and_esc_goes_back() {
        let mut menu = ModelMenu::new(Some("sonnet".into()), None);
        assert_eq!(press(&mut menu, KeyCode::Enter), Outcome::Handled);
        let text = screen(&menu).join("\n");
        assert!(text.contains("Selecione o modelo default"), "{text}");
        assert!(
            !text.contains("Default ("),
            "sem o item Default dentro dele"
        );
        assert!(text.contains("▌ 2. sonnet"), "começa no padrão atual");
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Chosen {
                target: Target::Default,
                model: "sonnet".into(),
                effort: Some("high".into())
            }
        );
        press(&mut menu, KeyCode::Esc);
        assert_eq!(menu.target, Target::Session);
        assert_eq!(press(&mut menu, KeyCode::Esc), Outcome::Close);
    }

    #[test]
    fn subagent_menu_starts_on_current_and_chooses_without_effort() {
        let mut menu = ModelMenu::subagents(Some("haiku".into()), Some(&catalog()));
        let text = screen(&menu).join("\n");
        assert!(
            text.contains("Selecione o modelo dos sub agentes"),
            "{text}"
        );
        assert!(text.contains("Default (o agente escolhe)"), "{text}");
        assert!(text.contains("▌ 4. haiku"), "começa no atual: {text}");
        assert!(!text.contains("Esforço"), "sem esforço: {text}");
        press(&mut menu, KeyCode::Right);
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Chosen {
                target: Target::Subagent,
                model: "haiku".into(),
                effort: None
            }
        );
    }

    #[test]
    fn subagent_default_unsets_and_esc_closes() {
        let mut menu = ModelMenu::subagents(None, Some(&catalog()));
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Unset(Target::Subagent)
        );
        assert_eq!(press(&mut menu, KeyCode::Esc), Outcome::Close);
    }

    #[test]
    fn delegated_menu_is_the_model_menu_with_effort() {
        let mut menu = ModelMenu::delegated(
            Some("haiku".into()),
            Some("low".into()),
            Some("opus".into()),
            Some(&catalog()),
        );
        menu.sync(Some(&catalog()));
        let text = screen(&menu).join("\n");
        assert!(
            text.contains("Selecione o modelo dos chats delegados"),
            "{text}"
        );
        assert!(
            text.contains("Default (opus)"),
            "o Default mostra o padrão: {text}"
        );
        assert!(text.contains("▌ 4. haiku"), "começa no atual: {text}");
        assert!(text.contains("Esforço"), "com esforço: {text}");
        press(&mut menu, KeyCode::Up);
        press(&mut menu, KeyCode::Up);
        assert!(matches!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Chosen {
                target: Target::Delegated,
                effort: Some(_),
                ..
            }
        ));
        let mut menu = ModelMenu::delegated(None, None, None, Some(&catalog()));
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            Outcome::Unset(Target::Delegated)
        );
        assert_eq!(press(&mut menu, KeyCode::Esc), Outcome::Close);
    }

    #[test]
    fn shows_loading_without_catalog() {
        let menu = ModelMenu::new(None, None);
        let mut terminal = Terminal::new(TestBackend::new(70, 22)).unwrap();
        terminal
            .draw(|frame| {
                menu.draw(frame, Rect::new(0, 20, 70, 2), None, &|m: &ModelInfo| {
                    m.label.clone()
                })
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..20)
            .flat_map(|y| (0..70).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains("Carregando os modelos do agente..."));
    }
}
