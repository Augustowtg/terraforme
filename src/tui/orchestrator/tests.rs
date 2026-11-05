//! A aba Orquestrador sobre uma pasta do terra temporária: as teclas mudam
//! o `orchestrator.toml` (só quando ele continua válido) e a tela mostra o
//! que está no disco.

use std::fs;

use crossterm::event::KeyModifiers;
use ratatui::{Terminal, backend::TestBackend};

use super::*;
use crate::hub::protocol::{Health, Status, Tab as HubTab};
use crate::orchestrator::run::{RunState, Usage};

struct Tab {
    dir: tempfile::TempDir,
    view: OrchestratorView,
}

impl Tab {
    fn new(config: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("orchestrator.toml"), config).unwrap();
        let mut data = Data::new(Some(dir.path().to_path_buf()));
        data.reload();
        Self {
            dir,
            view: OrchestratorView::new(data),
        }
    }

    fn file(&self) -> String {
        fs::read_to_string(self.dir.path().join("orchestrator.toml")).unwrap()
    }

    fn config(&self) -> Config {
        Config::parse(&self.file()).unwrap()
    }

    fn press(&mut self, code: KeyCode) -> bool {
        self.view
            .handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.press(KeyCode::Char(c));
        }
    }

    /// Vai até a linha com a ação `action`.
    fn select(&mut self, action: &Action) {
        let i = self.view.actions().iter().position(|a| a == action);
        self.view.selected =
            i.unwrap_or_else(|| panic!("sem {action:?}: {:?}", self.view.actions()));
    }

    fn screen(&self) -> String {
        self.screen_with(None)
    }

    fn screen_with(&self, instances: Option<&(bool, Vec<Instance>)>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 80)).unwrap();
        let hub = HubView {
            instances,
            own: None,
        };
        terminal
            .draw(|frame| self.view.draw(frame, frame.area(), &hub))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..80)
            .map(|y| {
                (0..110)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[test]
fn p_pauses_and_resumes_every_dispatch() {
    let mut tab = Tab::new("# meus agentes\n");
    assert!(tab.screen().contains("● ativos"));
    tab.press(KeyCode::Char('p'));
    assert!(tab.config().global.paused);
    assert!(
        tab.file().starts_with("# meus agentes\n"),
        "o comentário fica"
    );
    assert!(tab.screen().contains("⏸ pausados"));
    tab.press(KeyCode::Char('p'));
    assert!(!tab.config().global.paused);
}

#[test]
fn dispatched_agents_routing_messages_and_runs_stay_on_disk_and_off_the_tab() {
    let config = "[agents.api]\ncwd = \"/p\"\n\n[modelos]\nforte = \"opus\"\n\n\
                  [roteamento]\n[[roteamento.regra]]\ntag = \"testes\"\nmodelo = \"forte\"\n";
    let mut tab = Tab::new(config);
    let now = jiff::Timestamp::now().as_second() as u64;
    let run = Run {
        id: "aa01".into(),
        agent: "api".into(),
        message: "ab12".into(),
        from: "usuario".into(),
        level: 1,
        hops: 1,
        note: "/n.md".into(),
        state: RunState::Running,
        created: now,
        started: Some(now),
        finished: None,
        provider: "claude".into(),
        model: None,
        usage: None,
        error: None,
        summary: None,
    };
    run.save(tab.dir.path()).unwrap();
    let saved = tab.dir.path().join("messages/ab12");
    fs::create_dir_all(&saved).unwrap();
    fs::write(saved.join("message.json"), "{\"id\":\"ab12\"}").unwrap();
    tab.view.data.reload();

    let screen = tab.screen();
    for hidden in [
        "Agentes despachados",
        "Filas e execuções",
        "Mensagens",
        "Roteamento",
        "Sub agentes em execução",
        "aa01",
        "ab12",
        "Regra #testes",
        "Novo agente",
    ] {
        assert!(!screen.contains(hidden), "{hidden}: {screen}");
    }
    assert!(screen.contains("Custos de hoje"), "{screen}");

    // Mudar o que a aba mostra não mexe no que ela esconde.
    tab.press(KeyCode::Char('p'));
    assert!(tab.config().global.paused);
    let file = tab.config();
    assert!(file.agent("api").is_some());
    assert_eq!(file.routing.rules.len(), 1);
    assert_eq!(file.model_id("forte"), "opus");
    assert!(tab.dir.path().join("runs/aa01/run.json").exists());
    assert!(saved.join("message.json").exists());
}

#[test]
fn an_invalid_change_shows_the_error_and_the_file_stays() {
    let mut tab = Tab::new("[global]\nmax_hops = 5 # cuidado com laços\n");
    tab.select(&Action::Global(GlobalKey::MaxHops));
    tab.press(KeyCode::Right);
    assert_eq!(tab.config().global.max_hops, 6);
    assert!(tab.file().contains("# cuidado com laços"));

    let before = tab.file();
    tab.press(KeyCode::Enter);
    tab.press(KeyCode::Backspace);
    tab.type_text("0");
    tab.press(KeyCode::Enter);
    assert_eq!(tab.file(), before);
    assert!(tab.screen().contains("não é um número maior que zero"));
}

#[test]
fn a_broken_file_is_shown_and_not_edited() {
    let mut tab = Tab::new("[global\n");
    let screen = tab.screen();
    assert!(screen.contains("orchestrator.toml inválido"), "{screen}");
    assert!(screen.contains("última configuração válida"));
    tab.press(KeyCode::Char('p'));
    assert_eq!(tab.file(), "[global\n");
}

#[test]
fn runs_costs_and_budget_alerts_come_from_the_disk() {
    let mut tab = Tab::new("[agents.api]\ncwd = \"/p\"\norcamento_diario = 2\n");
    let now = jiff::Timestamp::now().as_second() as u64;
    for (id, level, cost) in [("aa01", 1, 0.25), ("bb02", 3, 0.75)] {
        Run {
            id: id.into(),
            agent: "api".into(),
            message: "m".into(),
            from: "usuario".into(),
            level,
            hops: 1,
            note: "/n.md".into(),
            state: RunState::Done,
            created: now,
            started: Some(now),
            finished: Some(now),
            provider: "claude".into(),
            model: None,
            usage: Some(Usage {
                input: 1000,
                output: 500,
                cost_usd: cost,
                ..Usage::default()
            }),
            error: None,
            summary: Some("feito".into()),
        }
        .save(tab.dir.path())
        .unwrap();
    }
    tab.view.data.reload();
    let screen = tab.screen();
    assert!(
        screen.contains("2 execuções · 3,0 mil tokens · US$ 1,0000 · orçamento 2/2"),
        "{screen}"
    );
    assert!(
        screen.contains("1: US$ 0,2500 · 2: — · 3: US$ 0,7500"),
        "{screen}"
    );
    assert!(screen.contains("api sem orçamento hoje (2/2)"), "{screen}");
}

fn instance(workspace: &str, pid: u64, health: Health, tabs: &[&str]) -> Instance {
    Instance {
        id: format!("id{pid}"),
        pid,
        app: "terra".into(),
        cwd: String::new(),
        workspace: workspace.into(),
        started: 0,
        last_seen: 0,
        health,
        reason: None,
        status: Status {
            chats: tabs.len(),
            tabs: tabs
                .iter()
                .map(|title| HubTab {
                    title: (*title).into(),
                    ..HubTab::default()
                })
                .collect(),
            ..Status::default()
        },
    }
}

#[test]
fn a_workspace_shows_its_live_instances_only_when_opened() {
    let mut tab = Tab::new("");
    let hub = (
        true,
        vec![
            instance("Studio", 4242, Health::Alive, &["Principal", "Chat 2"]),
            instance("Studio", 4300, Health::Stale, &["Principal"]),
            instance("Studio", 5000, Health::Gone, &["Principal"]),
            instance("velho", 6000, Health::Gone, &["Principal"]),
        ],
    );
    let screen = tab.screen_with(Some(&hub));
    assert!(screen.contains("▸ Studio · 2 instâncias"), "{screen}");
    assert!(!screen.contains("Studio-4242"), "{screen}");
    // O que saiu não aparece, nem o workspace sem ninguém vivo.
    assert!(!screen.contains("velho"), "{screen}");

    tab.select(&Action::Workspace("Studio".into()));
    tab.press(KeyCode::Enter);
    let screen = tab.screen_with(Some(&hub));
    assert!(screen.contains("▾ Studio · 2 instâncias"), "{screen}");
    assert!(screen.contains("Studio-4242"), "{screen}");
    assert!(screen.contains("Principal, Chat 2"), "{screen}");
    assert!(screen.contains("Studio-4300"), "{screen}");
    assert!(!screen.contains("Studio-5000"), "{screen}");

    tab.press(KeyCode::Left);
    assert!(!tab.screen_with(Some(&hub)).contains("Studio-4242"));
}

#[test]
fn the_timeout_can_be_infinite() {
    let mut tab = Tab::new("");
    assert!(tab.screen().contains("Demonstrativo (visível nas abas)"));
    tab.select(&Action::Global(GlobalKey::Timeout));
    tab.press(KeyCode::Enter);
    for _ in 0..2 {
        tab.press(KeyCode::Backspace);
    }
    tab.type_text("infinito");
    tab.press(KeyCode::Enter);
    assert_eq!(tab.config().global.timeout_min, None);
    assert!(tab.file().contains("tempo_limite_min = \"infinito\""));

    // → sai do infinito para 1 minuto; ← volta.
    tab.press(KeyCode::Right);
    assert_eq!(tab.config().global.timeout_min, Some(1));
    tab.press(KeyCode::Left);
    assert_eq!(tab.config().global.timeout_min, None);

    // O prompt começa com "infinito": apaga e escreve os minutos.
    tab.press(KeyCode::Enter);
    for _ in 0..8 {
        tab.press(KeyCode::Backspace);
    }
    tab.type_text("45");
    tab.press(KeyCode::Enter);
    assert_eq!(tab.config().global.timeout_min, Some(45));
    assert!(tab.screen().contains("45 min"));
}
