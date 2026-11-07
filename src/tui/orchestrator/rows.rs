//! As linhas da aba Orquestrador, seção por seção, a partir do
//! `orchestrator.toml`, das execuções e da lista do hub. Algumas linhas são
//! escolhíveis: a [`Action`] diz o que as teclas fazem nelas.

use ratatui::text::{Line, Span};

use super::Data;
use crate::hub::protocol::{Health, Instance, Tab};
use crate::hub::view;
use crate::orchestrator::config::{Config, INFINITE, RunMode};
use crate::orchestrator::costs::{self, Alert};
use crate::orchestrator::run;
use crate::theme;

/// Largura da coluna dos nomes.
const LABEL: usize = 24;
/// Largura da coluna `workspace-PID` de uma instância.
const INSTANCE_WIDTH: usize = 24;
/// Largura da coluna do nome da instância (a aba principal dela).
const NAME_WIDTH: usize = 28;
/// Largura do título de um sub agente, sob o `workspace-PID` (o `↳` come
/// dois caracteres).
const SUB_WIDTH: usize = INSTANCE_WIDTH - 2;

/// Uma chave do `[global]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalKey {
    Concurrency,
    MaxHops,
    Words,
    RunMode,
    Timeout,
    FocusRuns,
}

impl GlobalKey {
    pub const ALL: [GlobalKey; 6] = [
        GlobalKey::RunMode,
        GlobalKey::Timeout,
        GlobalKey::FocusRuns,
        GlobalKey::Concurrency,
        GlobalKey::MaxHops,
        GlobalKey::Words,
    ];

    pub fn key(self) -> &'static str {
        match self {
            GlobalKey::Concurrency => "max_execucoes_simultaneas",
            GlobalKey::MaxHops => "max_hops",
            GlobalKey::Words => "limite_palavras_mensagem",
            GlobalKey::RunMode => "modo_execucao",
            GlobalKey::Timeout => "tempo_limite_min",
            GlobalKey::FocusRuns => "aba_em_primeiro_plano",
        }
    }

    fn label(self) -> &'static str {
        match self {
            GlobalKey::Concurrency => "Execuções simultâneas",
            GlobalKey::MaxHops => "Máximo de saltos",
            GlobalKey::Words => "Limite de palavras",
            GlobalKey::RunMode => "Modo de execução",
            GlobalKey::Timeout => "Tempo limite",
            GlobalKey::FocusRuns => "Aba da execução",
        }
    }

    /// O número da chave (as que são números; o tempo limite infinito é
    /// `None`).
    pub fn number(self, config: &Config) -> Option<u32> {
        let global = &config.global;
        match self {
            GlobalKey::Concurrency => Some(global.max_concurrent),
            GlobalKey::MaxHops => Some(global.max_hops),
            GlobalKey::Words => Some(global.word_limit),
            GlobalKey::Timeout => global.timeout_min,
            GlobalKey::RunMode | GlobalKey::FocusRuns => None,
        }
    }

    fn value(self, config: &Config) -> String {
        let global = &config.global;
        match self {
            GlobalKey::RunMode if global.run_mode == RunMode::Tab => {
                "Demonstrativo (visível nas abas)".into()
            }
            GlobalKey::RunMode => "Segundo plano (não aparece nas abas)".into(),
            GlobalKey::Timeout => global
                .timeout_min
                .map_or(format!("{INFINITE} (Enter: minutos ou {INFINITE})"), |n| {
                    format!("{n} min")
                }),
            GlobalKey::FocusRuns if global.focus_runs => "abre em primeiro plano".into(),
            GlobalKey::FocusRuns => "abre em segundo plano".into(),
            key => key.number(config).unwrap_or_default().to_string(),
        }
    }
}

/// O que as teclas fazem numa linha escolhível.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// O interruptor geral (`pausado`).
    Pause,
    Global(GlobalKey),
    /// Um workspace da seção Sessões: o `Enter` mostra ou esconde as
    /// instâncias dele.
    Workspace(String),
}

pub struct Row {
    pub line: Line<'static>,
    pub action: Option<Action>,
}

fn info(line: Line<'static>) -> Row {
    Row { line, action: None }
}

fn title(text: &str) -> [Row; 2] {
    [
        info(Line::default()),
        info(Line::styled(format!(" {text}"), theme::section_title())),
    ]
}

/// Uma linha escolhível com um nome e um valor; `selected` a destaca.
fn field(label: &str, value: String, action: Action, selected: bool) -> Row {
    let label = format!(" {label:<LABEL$}");
    let line = if selected {
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
            Span::styled(value, theme::path()),
        ])
    };
    Row {
        line,
        action: Some(action),
    }
}

/// Uma linha escolhível só com texto (ex.: um workspace).
fn item(text: String, style: ratatui::style::Style, action: Action, selected: bool) -> Row {
    let line = if selected {
        Line::from(format!("{} {text}", theme::FOCUS_MARKER)).style(theme::focused())
    } else {
        Line::styled(format!("  {text}"), style)
    };
    Row {
        line,
        action: Some(action),
    }
}

/// As instâncias do hub, como a aba as recebe (`None` antes de assinar).
pub struct Hub<'a> {
    pub instances: Option<&'a (bool, Vec<Instance>)>,
    pub own: Option<&'a str>,
}

/// Todas as linhas da aba. `selected` é a ação da linha escolhida e
/// `expanded` os workspaces abertos na seção Sessões.
pub fn build(data: &Data, hub: &Hub, expanded: &[String], selected: Option<&Action>) -> Vec<Row> {
    let is = |action: &Action| selected == Some(action);
    let mut rows = vec![info(Line::styled(" Orquestrador", theme::section_title()))];
    let config = match &data.config {
        Ok(config) => Some(config),
        Err(err) => {
            rows.push(info(Line::styled(
                format!("   orchestrator.toml inválido: {err}"),
                theme::error(),
            )));
            rows.push(info(Line::styled(
                "   O orquestrador segue com a última configuração válida. Corrija o arquivo para \
                 editar aqui.",
                theme::muted(),
            )));
            None
        }
    };
    if let Some(config) = config {
        let paused = config.global.paused;
        let (text, style) = if paused {
            (
                "⏸ pausados: nenhuma execução começa (p retoma)",
                theme::stale(),
            )
        } else {
            ("● ativos (p pausa todos)", theme::alive())
        };
        rows.push(field_styled(
            "Despachos",
            text,
            style,
            Action::Pause,
            is(&Action::Pause),
        ));
    }
    sessions(&mut rows, hub, expanded, &is);
    if let Some(config) = config {
        settings(&mut rows, config, &is);
        costs(&mut rows, config, data);
    }
    rows
}

fn field_styled(
    label: &str,
    value: &str,
    style: ratatui::style::Style,
    action: Action,
    selected: bool,
) -> Row {
    let mut row = field(label, value.to_string(), action, selected);
    if !selected && let Some(span) = row.line.spans.last_mut() {
        span.style = style;
    }
    row
}

fn costs(rows: &mut Vec<Row>, config: &Config, data: &Data) {
    rows.extend(title("Custos de hoje"));
    let days = costs::by_agent(&data.runs, config, run::today());
    if days.is_empty() {
        rows.push(info(Line::styled(
            "    Nenhum agente despachado.",
            theme::muted(),
        )));
    }
    for day in &days {
        let budget = day.budget.map_or(String::new(), |limit| {
            format!(" · orçamento {}/{limit}", day.runs)
        });
        rows.push(info(Line::styled(
            format!(
                "    {:<16} {} execuções · {} tokens · {}{budget}",
                day.agent,
                day.runs,
                tokens(day.tokens),
                usd(day.cost_usd)
            ),
            theme::path(),
        )));
    }
    let levels = costs::average_by_level(&data.runs);
    let average: Vec<String> = levels
        .iter()
        .enumerate()
        .map(|(i, cost)| format!("{}: {}", i + 1, cost.map_or("—".into(), usd)))
        .collect();
    rows.push(info(Line::styled(
        format!("    Custo médio por nível   {}", average.join(" · ")),
        theme::path(),
    )));
    for day in &days {
        let limit = day.budget.unwrap_or_default();
        match day.alert() {
            Some(Alert::Warning) => rows.push(info(Line::styled(
                format!(
                    "    ⚠ {} já usou 80% do orçamento de hoje ({}/{limit})",
                    day.agent, day.runs
                ),
                theme::stale(),
            ))),
            Some(Alert::Blocked) => rows.push(info(Line::styled(
                format!(
                    "    ⛔ {} sem orçamento hoje ({}/{limit}): novas mensagens são recusadas",
                    day.agent, day.runs
                ),
                theme::error(),
            ))),
            None => {}
        }
    }
}

fn settings(rows: &mut Vec<Row>, config: &Config, is: &dyn Fn(&Action) -> bool) {
    rows.extend(title("Configuração"));
    for key in GlobalKey::ALL {
        let action = Action::Global(key);
        rows.push(field(
            key.label(),
            key.value(config),
            action.clone(),
            is(&action),
        ));
    }
}

/// As sessões ativas: um workspace por linha (`Studio · 2 instâncias`),
/// que o `Enter` abre nas instâncias dele (`Studio-4242  Principal  rodando`).
/// As que saíram não aparecem.
fn sessions(rows: &mut Vec<Row>, hub: &Hub, expanded: &[String], is: &dyn Fn(&Action) -> bool) {
    rows.extend(title("Sessões"));
    let message = match hub.instances {
        None if hub.own.is_none() => Some("   Sem o orquestrador neste terra."),
        None | Some((false, _)) => Some("   Conectando ao orquestrador..."),
        Some(_) => None,
    };
    if let Some(message) = message {
        rows.push(info(Line::styled(message, theme::muted())));
    } else if let Some((_, instances)) = hub.instances {
        let alive: Vec<Instance> = instances
            .iter()
            .filter(|i| i.health != Health::Gone)
            .cloned()
            .collect();
        if alive.is_empty() {
            rows.push(info(Line::styled(
                "   Nenhuma sessão ativa.",
                theme::muted(),
            )));
        }
        for workspace in view::by_workspace(&alive) {
            let open = expanded.iter().any(|w| w == workspace.name);
            let action = Action::Workspace(workspace.name.to_string());
            let count = workspace.instances.len();
            let mut text = format!(
                " {} {} · {count} {}",
                if open { "▾" } else { "▸" },
                workspace.name,
                if count == 1 {
                    "instância"
                } else {
                    "instâncias"
                },
            );
            let agents = workspace.agents();
            match agents {
                0 => {}
                1 => text.push_str(" · 1 sub agente"),
                n => text.push_str(&format!(" · {n} sub agentes")),
            }
            if agents > 0 {
                let subs = workspace.instances.iter().flat_map(|i| subagents(i));
                text.push_str(&format!(" ({})", states(subs)));
            }
            rows.push(item(text, theme::text(), action.clone(), is(&action)));
            if !open {
                continue;
            }
            for instance in &workspace.instances {
                let dot = match instance.health {
                    Health::Stale => theme::stale(),
                    _ => theme::alive(),
                };
                let [doing, _, elapsed] = view::instance_columns(instance);
                let id = format!("{}-{}", instance.workspace, instance.pid);
                let name = instance
                    .status
                    .tabs
                    .first()
                    .map_or(instance.app.clone(), |tab| {
                        tab.name.clone().unwrap_or_else(|| tab.title.clone())
                    });
                rows.push(info(Line::from(vec![
                    Span::styled("      ● ", dot),
                    Span::styled(
                        format!("{:<INSTANCE_WIDTH$} ", short(&id, INSTANCE_WIDTH)),
                        theme::text(),
                    ),
                    Span::styled(
                        format!("{:<NAME_WIDTH$} ", short(&name, NAME_WIDTH)),
                        theme::path(),
                    ),
                    Span::styled(format!("{doing} · {elapsed}"), theme::muted()),
                ])));
                for tab in subagents(instance) {
                    let doing = view::tab_doing(tab);
                    let name = tab
                        .name
                        .as_deref()
                        .map_or(String::new(), |name| format!("@{name}"));
                    rows.push(info(Line::from(vec![
                        Span::styled("        ↳ ", theme::muted()),
                        Span::styled(
                            format!("{:<SUB_WIDTH$} ", short(&tab.title, SUB_WIDTH)),
                            theme::text(),
                        ),
                        Span::styled(
                            format!("{:<NAME_WIDTH$} ", short(&name, NAME_WIDTH)),
                            theme::path(),
                        ),
                        Span::styled(short(doing, NAME_WIDTH), state_style(doing, tab)),
                    ])));
                }
            }
        }
    }
}

/// Os sub agentes de uma instância: as abas além da principal.
fn subagents(instance: &Instance) -> impl Iterator<Item = &Tab> {
    instance.status.tabs.iter().skip(1)
}

/// Quantos sub agentes estão em cada estado (ex.: `2 rodando, 1 concluído`).
fn states<'a>(tabs: impl Iterator<Item = &'a Tab>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for tab in tabs {
        let state = if tab.task.is_some() {
            "rodando"
        } else {
            view::tab_doing(tab)
        };
        match counts.iter_mut().find(|(s, _)| *s == state) {
            Some((_, n)) => *n += 1,
            None => counts.push((state, 1)),
        }
    }
    counts.sort_by_key(|(state, _)| STATES.iter().position(|s| s == state));
    counts
        .iter()
        .map(|(state, n)| format!("{n} {state}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A ordem dos estados no resumo do workspace.
const STATES: [&str; 4] = ["rodando", "parado", "concluído", "falhou"];

fn state_style(doing: &str, tab: &Tab) -> ratatui::style::Style {
    match (doing, tab.task.is_some()) {
        (_, true) | ("rodando", _) => theme::alive(),
        ("falhou", _) => theme::error(),
        _ => theme::muted(),
    }
}

/// Ex.: `US$ 0,0047`.
pub fn usd(value: f64) -> String {
    format!("US$ {value:.4}").replace('.', ",")
}

/// Ex.: `950`, `12,3 mil`, `1,2 mi`.
fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1} mil", n as f64 / 1e3).replace('.', ","),
        _ => format!("{:.1} mi", n as f64 / 1e6).replace('.', ","),
    }
}

fn short(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or_default();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let mut cut: String = first.chars().take(max - 1).collect();
    cut.push('…');
    cut
}
