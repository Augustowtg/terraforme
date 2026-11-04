//! As linhas da aba Orquestrador, seção por seção, a partir do
//! `orchestrator.toml`, das execuções, das mensagens e da lista do hub.
//! Algumas linhas são escolhíveis: a [`Action`] diz o que as teclas fazem
//! nelas.

use ratatui::text::{Line, Span};

use super::super::chat::model_menu::matches_id;
use super::Data;
use crate::agent::ModelInfo;
use crate::agent::adapters::claude;
use crate::hub::protocol::{Health, Instance};
use crate::hub::view;
use crate::orchestrator::config::{Agent, Config, INFINITE, RunMode};
use crate::orchestrator::costs::{self, Alert};
use crate::orchestrator::run::{self, Run, RunState};
use crate::theme::{self, Variant};

/// Largura da coluna dos nomes.
const LABEL: usize = 24;
/// Quantas execuções e mensagens a aba mostra (as mais novas).
pub const RECENT: usize = 10;
/// Largura da coluna `workspace-PID` de uma instância.
const INSTANCE_WIDTH: usize = 24;
/// Largura da coluna do nome da instância (as abas dela).
const NAME_WIDTH: usize = 28;

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

/// Um campo de um `[agents.<nome>]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKey {
    Cwd,
    Provider,
    Model,
    Tools,
    MaxTurns,
    Budget,
}

impl AgentKey {
    pub const ALL: [AgentKey; 6] = [
        AgentKey::Cwd,
        AgentKey::Provider,
        AgentKey::Model,
        AgentKey::Tools,
        AgentKey::MaxTurns,
        AgentKey::Budget,
    ];

    pub fn key(self) -> &'static str {
        match self {
            AgentKey::Cwd => "cwd",
            AgentKey::Provider => "provedor",
            AgentKey::Model => "modelo",
            AgentKey::Tools => "ferramentas",
            AgentKey::MaxTurns => "max_turnos",
            AgentKey::Budget => "orcamento_diario",
        }
    }

    fn label(self) -> &'static str {
        match self {
            AgentKey::Cwd => "Pasta",
            AgentKey::Provider => "Provedor",
            AgentKey::Model => "Modelo",
            AgentKey::Tools => "Ferramentas",
            AgentKey::MaxTurns => "Máximo de turnos",
            AgentKey::Budget => "Orçamento",
        }
    }

    /// O texto do campo para editar (o que o prompt começa mostrando).
    pub fn text(self, agent: &Agent) -> String {
        match self {
            AgentKey::Cwd => agent.cwd.display().to_string(),
            AgentKey::Provider => agent.provider().to_string(),
            AgentKey::Model => agent.model.clone().unwrap_or_default(),
            AgentKey::Tools => agent.tools.join(", "),
            AgentKey::MaxTurns => agent.max_turns.map_or(String::new(), |n| n.to_string()),
            AgentKey::Budget => agent.daily_budget.map_or(String::new(), |n| n.to_string()),
        }
    }

    pub fn number(self, agent: &Agent) -> Option<u32> {
        match self {
            AgentKey::MaxTurns => agent.max_turns,
            AgentKey::Budget => agent.daily_budget,
            _ => None,
        }
    }

    fn value(self, agent: &Agent, config: &Config, data: &Data) -> String {
        let provider = agent.provider();
        let name = data.provider_name(provider);
        match self {
            AgentKey::Provider => match data.providers.iter().find(|p| p.id == provider) {
                Some(option) if !option.available => format!("{name} (não instalado)"),
                _ => name.to_string(),
            },
            AgentKey::Model => {
                let label = |id: &str| model_name(data, provider, id);
                match &agent.model {
                    None => {
                        let default = data
                            .catalogs
                            .get(provider)
                            .and_then(|c| c.default_model.as_deref());
                        match default {
                            Some(id) => format!("padrão do {name} ({})", label(id)),
                            None => format!("padrão do {name}"),
                        }
                    }
                    Some(model) if config.model_id(model) != model => {
                        format!("{model} ({})", label(config.model_id(model)))
                    }
                    Some(model) => label(model),
                }
            }
            AgentKey::Tools | AgentKey::MaxTurns if agent.is_antigravity() => {
                format!("não vale no {name}")
            }
            AgentKey::Tools if agent.tools.is_empty() => {
                "nenhuma (tudo o que pedir permissão é negado)".into()
            }
            AgentKey::MaxTurns if agent.max_turns.is_none() => "sem limite".into(),
            AgentKey::Budget => agent
                .daily_budget
                .map_or("sem limite".into(), |n| format!("{n} execuções por dia")),
            key => key.text(agent),
        }
    }
}

/// O nome de um modelo do catálogo para a tela (ex.: `Opus 5.5`).
pub fn model_label(provider: &str, model: &ModelInfo) -> String {
    if !model.label.is_empty() {
        model.label.clone()
    } else if provider == "claude" {
        claude::display_model(&model.id)
    } else {
        model.id.clone()
    }
}

/// O nome do modelo `id` do `provider`: o do catálogo ou, fora dele, o id
/// (no Claude, como o título do input: `Opus - 5.5`).
fn model_name(data: &Data, provider: &str, id: &str) -> String {
    let found = data
        .catalogs
        .get(provider)
        .and_then(|c| c.models.iter().find(|m| matches_id(m, id)));
    match found {
        Some(model) => model_label(provider, model),
        None if provider == "claude" => claude::display_model(id),
        None => id.to_string(),
    }
}

/// Uma chave do `[roteamento]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingKey {
    Active,
    Default,
    Escalate,
}

/// O que as teclas fazem numa linha escolhível.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// O interruptor geral (`pausado`).
    Pause,
    Global(GlobalKey),
    AddAgent,
    /// A linha do agente (o `d` o tira).
    Agent(String),
    AgentField(String, AgentKey),
    /// Um workspace da seção Sessões: o `Enter` mostra ou esconde as
    /// instâncias dele.
    Workspace(String),
    /// Uma execução: o `Enter` abre a saída dela.
    Run(String),
    Routing(RoutingKey),
    Rule(usize),
    AddRule,
    Model(String),
    AddModel,
}

impl Action {
    /// O que se tira com o `d`.
    pub fn removable(&self) -> bool {
        matches!(self, Action::Agent(_) | Action::Rule(_) | Action::Model(_))
    }
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
fn field(label: &str, value: String, indent: usize, action: Action, selected: bool) -> Row {
    let label = format!("{:indent$}{label:<LABEL$}", "");
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

/// Uma linha escolhível só com texto (ex.: `+ Novo agente`).
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

/// Todas as linhas da aba. `selected` é a ação da linha escolhida,
/// `deleting` a que espera o segundo `d` e `expanded` os workspaces abertos
/// na seção Sessões.
pub fn build(
    data: &Data,
    hub: &Hub,
    expanded: &[String],
    selected: Option<&Action>,
    deleting: Option<&Action>,
) -> Vec<Row> {
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
    sessions(&mut rows, data, hub, expanded, &is);
    if let Some(config) = config {
        settings(&mut rows, config, &is);
        agents(&mut rows, config, data, &is);
    }
    runs(&mut rows, data, config, &is);
    messages(&mut rows, data);
    if let Some(config) = config {
        costs(&mut rows, config, data);
        routing(&mut rows, config, &is);
    }
    if let Some(deleting) = deleting
        && let Some(row) = rows
            .iter_mut()
            .find(|row| row.action.as_ref() == Some(deleting))
    {
        row.line
            .spans
            .push(Span::styled("   d de novo para tirar", theme::error()));
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
    let mut row = field(label, value.to_string(), 1, action, selected);
    if !selected && let Some(span) = row.line.spans.last_mut() {
        span.style = style;
    }
    row
}

fn agents(rows: &mut Vec<Row>, config: &Config, data: &Data, is: &dyn Fn(&Action) -> bool) {
    rows.extend(title("Agentes despachados"));
    let days = costs::by_agent(&data.runs, config, run::today());
    for (agent, day) in config.agents.iter().zip(&days) {
        let queued = data
            .runs
            .iter()
            .filter(|r| r.agent == agent.name && r.state == RunState::Queued)
            .count();
        let running = data
            .runs
            .iter()
            .any(|r| r.agent == agent.name && r.state == RunState::Running);
        let mut state = vec![if running {
            "rodando".to_string()
        } else {
            "parado".to_string()
        }];
        if queued > 0 {
            state.push(format!("{queued} na fila"));
        }
        if let Some(limit) = day.budget {
            state.push(format!("{}/{limit} hoje", day.runs));
        }
        let action = Action::Agent(agent.name.clone());
        let text = format!("{:<16} despachado · {}", agent.name, state.join(" · "));
        let style = if running {
            theme::alive()
        } else {
            theme::text()
        };
        rows.push(item(format!(" {text}"), style, action.clone(), is(&action)));
        for key in AgentKey::ALL {
            let action = Action::AgentField(agent.name.clone(), key);
            let selected = is(&action);
            let mut row = field(
                key.label(),
                key.value(agent, config, data),
                5,
                action,
                selected,
            );
            let off = agent.is_antigravity() && matches!(key, AgentKey::Tools | AgentKey::MaxTurns);
            if let Some(value) = row.line.spans.last_mut()
                && !selected
                && off
            {
                value.style = theme::muted();
            }
            // O modelo com o provedor no estilo de botão, como o título do
            // input: `[Claude] Opus - 5.5`.
            if key == AgentKey::Model {
                let name = data.provider_name(agent.provider()).to_string();
                let at = row.line.spans.len() - 1;
                row.line.spans.insert(
                    at,
                    Span::styled(format!(" {name} "), theme::badge_text(Variant::Active)),
                );
                row.line.spans.insert(at + 1, Span::raw(" "));
            }
            rows.push(row);
        }
    }
    let action = Action::AddAgent;
    rows.push(item(
        " + Novo agente".into(),
        theme::muted(),
        action.clone(),
        is(&action),
    ));
}

fn runs(rows: &mut Vec<Row>, data: &Data, config: Option<&Config>, is: &dyn Fn(&Action) -> bool) {
    rows.extend(title("Filas e execuções"));
    if let Some(config) = config {
        for agent in &config.agents {
            let queued = data
                .runs
                .iter()
                .filter(|r| r.agent == agent.name && r.state == RunState::Queued)
                .count();
            let running = data
                .runs
                .iter()
                .filter(|r| r.agent == agent.name && r.state == RunState::Running)
                .count();
            if queued + running > 0 {
                rows.push(info(Line::styled(
                    format!(
                        "    {:<16} {running} rodando · {queued} na fila",
                        agent.name
                    ),
                    theme::path(),
                )));
            }
        }
    }
    if data.runs.is_empty() {
        rows.push(info(Line::styled("    Nenhuma execução.", theme::muted())));
        return;
    }
    for run in data.runs.iter().rev().take(RECENT) {
        let action = Action::Run(run.id.clone());
        let selected = is(&action);
        rows.push(item(run_line(run), run_style(run.state), action, selected));
        if selected {
            let dir = run.dir(data.home.as_deref().unwrap_or(std::path::Path::new("")));
            let mut details = vec![format!("nota: {}", run.note.display())];
            details.push(format!("saída: {}", dir.join(run::OUTPUT).display()));
            if let Some(summary) = run.summary.as_deref().filter(|s| !s.is_empty()) {
                details.push(format!("resumo: {summary}"));
            }
            for detail in details {
                rows.push(info(Line::styled(
                    format!("        {detail}"),
                    theme::muted(),
                )));
            }
        }
    }
}

fn run_line(run: &Run) -> String {
    let icon = match run.state {
        RunState::Queued => "…",
        RunState::Running => "▶",
        RunState::Done => "✓",
        RunState::Failed => "✗",
        RunState::Timeout => "⏱",
        RunState::Rejected => "⊘",
    };
    let cost = run
        .usage
        .as_ref()
        .map_or(String::new(), |u| format!(" · {}", usd(u.cost_usd)));
    let error = run
        .error
        .as_deref()
        .map_or(String::new(), |e| format!(" · {}", short(e, 40)));
    format!(
        "  {icon} {}  {:<12} {:<8} nível {}{cost}{error}",
        &run.id[..run.id.len().min(8)],
        run.agent,
        run.state.key(),
        run.level,
    )
}

fn run_style(state: RunState) -> ratatui::style::Style {
    match state {
        RunState::Running => theme::alive(),
        RunState::Failed | RunState::Timeout | RunState::Rejected => theme::error(),
        RunState::Queued => theme::stale(),
        RunState::Done => theme::path(),
    }
}

fn messages(rows: &mut Vec<Row>, data: &Data) {
    rows.extend(title("Mensagens"));
    if data.messages.is_empty() {
        rows.push(info(Line::styled("    Nenhuma mensagem.", theme::muted())));
    }
    for saved in &data.messages {
        let m = &saved.message;
        let reason = saved
            .reason
            .as_deref()
            .map_or(String::new(), |r| format!(" · {}", short(r, 40)));
        rows.push(info(Line::from(vec![
            Span::styled(
                format!(
                    "    {}  {} → {}  nível {} · {} saltos  ",
                    &m.id[..m.id.len().min(8)],
                    m.from,
                    m.to,
                    m.level,
                    m.hops
                ),
                theme::path(),
            ),
            Span::styled(format!("{}{reason}", saved.state.key()), theme::muted()),
        ])));
    }
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
            1,
            action.clone(),
            is(&action),
        ));
    }
}

fn routing(rows: &mut Vec<Row>, config: &Config, is: &dyn Fn(&Action) -> bool) {
    rows.extend(title("Roteamento"));
    let routing = &config.routing;
    let fields = [
        (
            RoutingKey::Active,
            "Regras por tag",
            if routing.active {
                "ligadas"
            } else {
                "desligadas (Fase C)"
            }
            .to_string(),
        ),
        (
            RoutingKey::Default,
            "Modelo padrão",
            routing.default.clone().unwrap_or("—".into()),
        ),
        (
            RoutingKey::Escalate,
            "Escalonar falhas",
            if routing.escalate_failures {
                "sim: a falha de nível 1 ou 2 pede o handoff ao remetente"
            } else {
                "não"
            }
            .to_string(),
        ),
    ];
    for (key, label, value) in fields {
        let action = Action::Routing(key);
        rows.push(field(label, value, 1, action.clone(), is(&action)));
    }
    for (i, rule) in routing.rules.iter().enumerate() {
        let action = Action::Rule(i);
        rows.push(field(
            &format!("Regra #{}", rule.tag),
            format!("modelo {}", rule.model),
            3,
            action.clone(),
            is(&action),
        ));
    }
    let action = Action::AddRule;
    rows.push(item(
        "   + Nova regra (tag modelo)".into(),
        theme::muted(),
        action.clone(),
        is(&action),
    ));
    rows.push(info(Line::styled("   Modelos", theme::text())));
    for (alias, id) in &config.models {
        let action = Action::Model(alias.clone());
        rows.push(field(alias, id.clone(), 3, action.clone(), is(&action)));
    }
    let action = Action::AddModel;
    rows.push(item(
        "   + Novo modelo (apelido id)".into(),
        theme::muted(),
        action.clone(),
        is(&action),
    ));
}

/// As sessões ativas: um workspace por linha (`Studio · 2 instâncias`),
/// que o `Enter` abre nas instâncias dele (`Studio-4242  Principal  rodando`).
/// As que saíram não aparecem. Os sub agentes despachados que estão rodando
/// vêm embaixo.
fn sessions(
    rows: &mut Vec<Row>,
    data: &Data,
    hub: &Hub,
    expanded: &[String],
    is: &dyn Fn(&Action) -> bool,
) {
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
            match workspace.agents() {
                0 => {}
                1 => text.push_str(" · 1 sub agente"),
                n => text.push_str(&format!(" · {n} sub agentes")),
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
                let names: Vec<&str> = instance
                    .status
                    .tabs
                    .iter()
                    .map(|tab| tab.name.as_deref().unwrap_or(&tab.title))
                    .collect();
                let name = if names.is_empty() {
                    instance.app.clone()
                } else {
                    names.join(", ")
                };
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
            }
        }
    }
    let running: Vec<&Run> = data
        .runs
        .iter()
        .filter(|r| r.state == RunState::Running)
        .collect();
    if !running.is_empty() {
        rows.push(info(Line::styled(
            "   Sub agentes em execução",
            theme::text(),
        )));
    }
    for run in running {
        rows.push(info(Line::styled(
            format!(
                "      ▶ {:<16} {}  nível {}",
                run.agent,
                &run.id[..run.id.len().min(8)],
                run.level
            ),
            theme::alive(),
        )));
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
