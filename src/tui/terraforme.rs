//! Aba Terraforme: a conversa com o agente à esquerda (o mesmo chat
//! principal da aba Chat, com o mesmo histórico e sessão), as sessões do
//! terra abertas na máquina à direita e, embaixo delas, uma caixa por sub
//! agente (as execuções do orquestrador) rodando ou que terminou há pouco:
//!
//! ```text
//! ╭──────────────────────────────╮
//! │ ● busca · 1.2K tokens        │   ● verde pulsando: trabalhando
//! │ Grep: subagent_model         │   ● amarelo: espera uma permissão
//! ╰──────────────────────────────╯   ● vermelho: falhou
//! ```

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};

use super::chat::Chat;
use super::chat::compact_count;
use super::orchestrator::HubView;
use crate::hub::protocol::{Health, Instance, now};
use crate::hub::view;
use crate::orchestrator::run::{self, Run, RunState};
use crate::theme::{self, Variant};

/// Quanto da largura da tela a conversa ocupa; as sessões ficam com o resto,
/// um pouco mais para a direita.
const CHAT_PERCENT: u16 = 45;

/// De quantos ticks em quantos ticks a bolinha de quem trabalha pulsa
/// (mesmo ritmo do cursor do input).
const PULSE_TICKS: u32 = 5;
/// De quantos em quantos ticks (de 100 ms) as execuções são relidas do
/// disco, com a aba aberta.
const RELOAD_TICKS: u32 = 10;
/// Por quanto tempo uma execução que terminou continua na tela.
const RECENT_SECS: u64 = 10 * 60;
/// A altura de uma caixa: a borda e as duas linhas.
const BOX_HEIGHT: u16 = 4;

/// O estado de um sub agente na caixa: cada um tem a sua cor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Working,
    Permission,
    Error,
    Queued,
    Done,
}

impl Tier {
    fn style(self) -> Style {
        match self {
            Tier::Working => theme::variant(Variant::Success),
            Tier::Permission => theme::variant(Variant::Warning),
            Tier::Error => theme::variant(Variant::Danger),
            Tier::Queued => theme::muted(),
            Tier::Done => theme::variant(Variant::Active),
        }
    }
}

/// A caixa de um sub agente: o nome, os tokens sem o cache e o que ele faz.
#[derive(Debug, Clone, PartialEq)]
struct RunBox {
    run: String,
    name: String,
    tier: Tier,
    tokens: u64,
    description: String,
}

/// As caixas das execuções rodando, na fila ou que terminaram há menos de
/// [`RECENT_SECS`] (em `now`), a mais nova primeiro.
fn load_boxes(home: &Path, now: u64) -> Vec<RunBox> {
    run::list(home)
        .into_iter()
        .rev()
        .filter(|run| {
            matches!(run.state, RunState::Queued | RunState::Running)
                || run
                    .finished
                    .is_some_and(|f| now.saturating_sub(f) < RECENT_SECS)
        })
        .map(|run| run_box(home, run))
        .collect()
}

fn run_box(home: &Path, run: Run) -> RunBox {
    let output = || fs::read_to_string(run.dir(home).join(run::OUTPUT)).unwrap_or_default();
    let used = run.usage.as_ref().map_or(0, |u| u.input + u.output);
    let (tier, tokens, description) = match run.state {
        RunState::Queued => (Tier::Queued, 0, "Na fila".to_string()),
        RunState::Running => {
            let live = run::live(&output());
            let activity = live.activity.unwrap_or_else(|| "Pensando...".into());
            (Tier::Working, live.tokens, activity)
        }
        RunState::Done => {
            let text = run.summary.clone().or_else(|| run::final_text(&output()));
            let first = text
                .as_deref()
                .and_then(|t| t.lines().find(|l| !l.trim().is_empty()));
            (
                Tier::Done,
                used,
                first.unwrap_or("Concluída").trim().to_string(),
            )
        }
        RunState::Failed | RunState::Timeout | RunState::Rejected => (
            Tier::Error,
            used,
            run.error.clone().unwrap_or_else(|| "Falhou".into()),
        ),
    };
    RunBox {
        run: run.id,
        name: run.agent,
        tier,
        tokens,
        description,
    }
}

/// As caixas com os pedidos de permissão (`waiting`: a execução e o pedido)
/// por cima: quem espera fica amarelo, com o pedido na descrição.
fn with_permissions(boxes: &[RunBox], waiting: &[(String, String)]) -> Vec<RunBox> {
    boxes
        .iter()
        .map(|b| match waiting.iter().find(|(run, _)| *run == b.run) {
            Some((_, request)) if b.tier == Tier::Working => RunBox {
                tier: Tier::Permission,
                description: format!("Permissão: {request}"),
                ..b.clone()
            },
            _ => b.clone(),
        })
        .collect()
}

/// As caixas, uma embaixo da outra, até onde cabem em `area`.
fn draw_boxes(frame: &mut Frame, area: Rect, boxes: &[RunBox], pulse_on: bool) {
    let mut y = area.y;
    for b in boxes {
        if y + BOX_HEIGHT > area.bottom() {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, BOX_HEIGHT);
        let dot = match b.tier {
            Tier::Working => theme::pulse(pulse_on),
            tier => tier.style(),
        };
        let lines = vec![
            Line::from(vec![
                Span::styled("● ", dot),
                Span::styled(b.name.clone(), theme::text()),
                Span::styled(
                    format!(" · {} tokens", compact_count(b.tokens)),
                    theme::muted(),
                ),
            ]),
            Line::styled(b.description.clone(), theme::path()),
        ];
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(b.tier.style())
            .padding(Padding::horizontal(1));
        frame.render_widget(Paragraph::new(lines).block(block), rect);
        y += BOX_HEIGHT;
    }
}

#[derive(Default)]
pub struct TerraformeView {
    /// Os workspaces abertos na lista de sessões.
    expanded: Vec<String>,
    /// O workspace escolhido, entre os do último desenho.
    selected: usize,
    /// Os workspaces do último desenho, na ordem da tela (as teclas chegam
    /// sem o hub).
    workspaces: RefCell<Vec<String>>,
    /// Ticks desde a abertura, para a bolinha pulsar.
    pulse_ticks: u32,
    /// Ticks até reler as execuções; 0 relê no próximo tick com a aba aberta.
    reload_in: u32,
    /// A pasta do terra, onde ficam as execuções.
    home: Option<PathBuf>,
    /// As caixas da última leitura.
    boxes: Vec<RunBox>,
}

impl TerraformeView {
    /// Retorna `true` se a tecla foi usada pela lista de sessões (`↑`/`↓`
    /// escolhe um workspace, `Enter` abre ou fecha). Chamado só depois do
    /// chat não ter consumido a tecla (ver [`crate::tui::chat::Chat::handle_key`]):
    /// no modo INSERT, ↑/↓/Enter são do chat, não da lista.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                let count = self.workspaces.borrow().len();
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
                true
            }
            KeyCode::Enter => {
                if let Some(name) = self.workspaces.borrow().get(self.selected).cloned() {
                    if self.expanded.contains(&name) {
                        self.expanded.retain(|w| *w != name);
                    } else {
                        self.expanded.push(name);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Com a aba aberta (`visible`), relê as execuções a cada
    /// [`RELOAD_TICKS`]; fechada, só marca para reler ao abrir.
    pub fn tick(&mut self, visible: bool) {
        self.pulse_ticks = self.pulse_ticks.wrapping_add(1);
        if !visible {
            self.reload_in = 0;
            return;
        }
        if self.reload_in > 0 {
            self.reload_in -= 1;
            return;
        }
        self.reload_in = RELOAD_TICKS;
        if self.home.is_none() {
            self.home = crate::hub::endpoint::Endpoint::current()
                .ok()
                .map(|e| e.home);
        }
        if let Some(home) = &self.home {
            self.boxes = load_boxes(home, now());
        }
    }

    /// Desenha a aba em `area`: a conversa (o `chat` principal) à esquerda,
    /// com histórico e painel de entrada, e as sessões abertas (do `hub`) à
    /// direita.
    pub fn draw(&self, frame: &mut Frame, area: Rect, hub: &HubView, chat: &Chat) {
        let [chat_col, sessions] =
            Layout::horizontal([Constraint::Percentage(CHAT_PERCENT), Constraint::Fill(1)])
                .areas(area);

        let [history, panel] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(chat.height())])
                .areas(chat_col);
        chat.draw_history(frame, history);
        chat.draw(frame, panel);

        let [_, sessions] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(sessions);
        let pulse_on = (self.pulse_ticks / PULSE_TICKS).is_multiple_of(2);
        let (lines, workspaces) = session_lines(hub, &self.expanded, self.selected);
        *self.workspaces.borrow_mut() = workspaces;
        let height = u16::try_from(lines.len() + 1).unwrap_or(u16::MAX);
        let [list, boxes] =
            Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(sessions);
        frame.render_widget(Paragraph::new(lines), list);
        let boxes_area = Rect {
            width: boxes.width.saturating_sub(1),
            ..boxes
        };
        let shown = with_permissions(&self.boxes, &chat.waiting_runs());
        draw_boxes(frame, boxes_area, &shown, pulse_on);
    }
}

/// As sessões do terra abertas na máquina, em grupos por workspace: cada
/// grupo fica fechado até o `Enter` abri-lo. Retorna as linhas e os nomes
/// dos workspaces, na ordem da tela (para o `handle_key` escolher entre
/// eles sem o hub).
fn session_lines(
    hub: &HubView,
    expanded: &[String],
    selected: usize,
) -> (Vec<Line<'static>>, Vec<String>) {
    let mut lines = vec![Line::styled(" Sessões abertas", theme::section_title())];
    let message = match hub.instances {
        None if hub.own.is_none() => Some("Sem o orquestrador neste terra."),
        None | Some((false, _)) => Some("Conectando ao orquestrador..."),
        Some(_) => None,
    };
    if let Some(message) = message {
        lines.push(Line::styled(format!(" {message}"), theme::muted()));
        return (lines, Vec::new());
    }
    let Some((_, instances)) = hub.instances else {
        return (lines, Vec::new());
    };
    let alive: Vec<Instance> = instances
        .iter()
        .filter(|i| i.health != Health::Gone)
        .cloned()
        .collect();
    if alive.is_empty() {
        lines.push(Line::styled(" Nenhuma sessão ativa.", theme::muted()));
        return (lines, Vec::new());
    }
    let groups = view::by_workspace(&alive);
    let names: Vec<String> = groups.iter().map(|w| w.name.to_string()).collect();
    for (i, workspace) in groups.iter().enumerate() {
        let open = expanded.iter().any(|w| w == workspace.name);
        let marker = if open { "▾" } else { "▸" };
        let header = format!(" {marker} {} · {}", workspace.name, workspace.summary());
        let style = if i == selected {
            theme::focused()
        } else {
            theme::path()
        };
        lines.push(Line::styled(header, style));
        if !open {
            continue;
        }
        for instance in &workspace.instances {
            let dot = match instance.health {
                Health::Stale => theme::stale(),
                _ => theme::alive(),
            };
            let [doing, _, elapsed] = view::instance_columns(instance);
            lines.push(Line::from(vec![
                Span::styled("   ● ", dot),
                Span::styled(format!("{doing} "), doing_style(&doing)),
                Span::styled(elapsed, theme::muted()),
            ]));
        }
    }
    (lines, names)
}

/// Rodando é verde, parado é neutro, e quem saiu é vermelho; uma tarefa (o
/// que o agente está fazendo) fica na cor de destaque.
fn doing_style(doing: &str) -> Style {
    match doing {
        "rodando" => theme::variant(Variant::Success),
        "parado" => theme::path(),
        _ if doing.starts_with("saiu") => theme::variant(Variant::Danger),
        _ => theme::variant(Variant::Accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_key_navega_e_expande_sessoes() {
        use crossterm::event::KeyModifiers;

        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let mut view = TerraformeView::default();
        *view.workspaces.borrow_mut() = vec!["a".to_string(), "b".to_string()];
        assert!(view.handle_key(key(KeyCode::Down)));
        assert_eq!(view.selected, 1);
        assert!(view.handle_key(key(KeyCode::Enter)));
        assert_eq!(view.expanded, vec!["b".to_string()]);
        assert!(!view.handle_key(key(KeyCode::Char('x'))));
    }

    #[test]
    fn doing_colore_por_estado() {
        assert_eq!(doing_style("rodando"), theme::variant(Variant::Success));
        assert_eq!(doing_style("parado"), theme::path());
        assert_eq!(doing_style("saiu (crash)"), theme::variant(Variant::Danger));
        assert_eq!(doing_style("uma tarefa"), theme::variant(Variant::Accent));
    }

    #[test]
    fn sem_hub_avisa_na_lista_de_sessoes() {
        let hub = HubView {
            instances: None,
            own: None,
        };
        let (lines, workspaces) = session_lines(&hub, &[], 0);
        assert!(
            lines
                .iter()
                .any(|l| l.to_string().contains("Sem o orquestrador"))
        );
        assert!(workspaces.is_empty());
    }

    fn instance(workspace: &str, pid: u64) -> Instance {
        Instance {
            id: format!("id{pid}"),
            pid,
            app: "terra".into(),
            cwd: String::new(),
            workspace: workspace.into(),
            started: 0,
            last_seen: 0,
            health: Health::Alive,
            reason: None,
            status: crate::hub::protocol::Status {
                running: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn workspace_comeca_fechado_e_abre_com_enter() {
        let instances = (true, vec![instance("Studio", 1), instance("cli", 2)]);
        let hub = HubView {
            instances: Some(&instances),
            own: None,
        };
        let (lines, workspaces) = session_lines(&hub, &[], 0);
        assert_eq!(workspaces, vec!["Studio".to_string(), "cli".to_string()]);
        assert!(lines.iter().any(|l| l.to_string().contains("▸ Studio")));
        assert!(!lines.iter().any(|l| l.to_string().contains("rodando")));

        let (lines, _) = session_lines(&hub, &["Studio".to_string()], 0);
        assert!(lines.iter().any(|l| l.to_string().contains("▾ Studio")));
        assert!(lines.iter().any(|l| l.to_string().contains("rodando")));
    }

    fn run(id: &str, agent: &str, state: RunState, finished: Option<u64>) -> Run {
        Run {
            id: id.into(),
            agent: agent.into(),
            message: format!("m{id}"),
            from: "usuario".into(),
            level: 1,
            hops: 1,
            note: PathBuf::from("/n.md"),
            state,
            created: finished.unwrap_or(1000),
            started: None,
            finished,
            provider: "claude".into(),
            model: None,
            usage: Some(run::Usage {
                input: 200,
                output: 1000,
                cache_read: 90_000,
                ..Default::default()
            }),
            error: None,
            summary: None,
        }
    }

    #[test]
    fn as_caixas_sao_das_execucoes_rodando_e_das_recentes() {
        let home = tempfile::tempdir().unwrap();
        let now = 10_000;
        let working = run("a1", "busca", RunState::Running, None);
        working.save(home.path()).unwrap();
        fs::write(
            working.dir(home.path()).join(run::OUTPUT),
            r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":40,"output_tokens":60,"cache_read_input_tokens":5000},"content":[{"type":"tool_use","name":"Grep","input":{"pattern":"subagent"}}]}}"#,
        )
        .unwrap();
        let mut failed = run("b2", "testes", RunState::Failed, Some(now - 30));
        failed.error = Some("passou do tempo_limite_min".into());
        failed.save(home.path()).unwrap();
        let mut done = run("c3", "qa", RunState::Done, Some(now - 60));
        done.summary = Some("lint ok\nresto".into());
        done.save(home.path()).unwrap();
        run("d4", "velho", RunState::Done, Some(now - RECENT_SECS - 1))
            .save(home.path())
            .unwrap();

        let boxes = load_boxes(home.path(), now);
        let names: Vec<&str> = boxes.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["testes", "qa", "busca"]);
        let busca = &boxes[2];
        assert_eq!((busca.tier, busca.tokens), (Tier::Working, 100));
        assert_eq!(busca.description, "Grep: subagent");
        // Os tokens são entrada e saída, sem o cache.
        assert_eq!((boxes[1].tier, boxes[1].tokens), (Tier::Done, 1200));
        assert_eq!(boxes[1].description, "lint ok");
        assert_eq!(boxes[0].tier, Tier::Error);
        assert_eq!(boxes[0].description, "passou do tempo_limite_min");

        let waiting = vec![("a1".to_string(), "Bash rm -rf build".to_string())];
        let shown = with_permissions(&boxes, &waiting);
        assert_eq!(shown[2].tier, Tier::Permission);
        assert_eq!(shown[2].description, "Permissão: Bash rm -rf build");
        assert_eq!(shown[0], boxes[0]);
    }

    fn screen(width: u16, view: &TerraformeView) -> (Vec<String>, ratatui::buffer::Buffer) {
        use ratatui::{Terminal, backend::TestBackend};

        let instances = (true, vec![instance("Studio", 1)]);
        let hub = HubView {
            instances: Some(&instances),
            own: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal
            .draw(|f| view.draw(f, f.area(), &hub, &Chat::default()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..30)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (rows, buffer)
    }

    #[test]
    fn cada_sub_agente_tem_uma_caixa_embaixo_das_sessoes_com_a_cor_do_estado() {
        let (rows, _) = screen(140, &TerraformeView::default());
        assert!(!rows.iter().any(|r| r.contains(" tokens")));

        let boxes = vec![
            RunBox {
                run: "a1".into(),
                name: "busca".into(),
                tier: Tier::Working,
                tokens: 1234,
                description: "Grep: subagent".into(),
            },
            RunBox {
                run: "b2".into(),
                name: "testes".into(),
                tier: Tier::Error,
                tokens: 0,
                description: "falhou".into(),
            },
        ];
        let mut view = TerraformeView {
            boxes,
            ..Default::default()
        };
        let (rows, buffer) = screen(140, &view);
        let sessions = rows
            .iter()
            .position(|r| r.contains("Sessões abertas"))
            .unwrap();
        let title = rows
            .iter()
            .position(|r| r.contains("● busca · 1.2K tokens"))
            .unwrap();
        assert!(title > sessions);
        assert!(rows[title + 1].contains("Grep: subagent"));
        assert!(rows[title + 4].contains("● testes · 0 tokens"));
        // À direita da conversa, com a borda na cor do estado.
        let x = rows[title - 1].chars().position(|c| c == '╭').unwrap();
        assert!(x >= 140 * usize::from(CHAT_PERCENT) / 100);
        let corner = |y: usize| {
            let col = rows[y].chars().position(|c| c == '╭').unwrap();
            buffer[(col as u16, y as u16)].fg
        };
        assert_eq!(Some(corner(title - 1)), Tier::Working.style().fg);
        assert_eq!(Some(corner(title + 3)), Tier::Error.style().fg);

        // A bolinha de quem trabalha pulsa.
        let dot = |view: &TerraformeView| {
            let (rows, buffer) = screen(140, view);
            let y = rows.iter().position(|r| r.contains("● busca")).unwrap();
            let col = rows[y].chars().position(|c| c == '●').unwrap();
            buffer[(col as u16, y as u16)].style()
        };
        let first = dot(&view);
        view.pulse_ticks = PULSE_TICKS;
        assert_ne!(first, dot(&view));
    }
}
