//! Testes da tela principal: abas, foco, confirmação de saída, configurações
//! salvas, interface do provedor e chats abertos.

use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::draw::{CONFIRM_TEXT, SAVE_TEXT};
use super::*;
use crate::theme;

fn press(app: &mut App, code: KeyCode) -> Tab {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    app.tab
}

#[test]
fn tab_key_cycles_tabs() {
    let mut app = App::default();
    assert_eq!(app.tab, Tab::Chat);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Extensoes);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Orquestrador);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
}

#[test]
fn aba_atualizacao_so_com_versao_nova_e_enter_pede_instalar() {
    let mut app = App::default();
    app.update_view.set_update(Some(crate::upgrade::Update {
        version: "9.9.9".into(),
        notes: "## [9.9.9] - 2099-01-01\n\n- novo".into(),
    }));
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Atualizacao);
    assert!(!app.take_update_install());
    press(&mut app, KeyCode::Enter);
    assert!(app.take_update_install());
    assert!(!app.take_update_install());
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
}

fn key(app: &mut App, code: KeyCode) -> bool {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn orquestrador_tab_is_always_between_extensions_and_config() {
    assert_eq!(Tab::Extensoes.next(false), Tab::Orquestrador);
    assert_eq!(Tab::Orquestrador.next(false), Tab::Config);
    assert_eq!(Tab::Config.next(false), Tab::Chat);
    assert_eq!(Tab::Chat.next(false), Tab::Consumo);
    assert_eq!(Tab::Chat.next(true), Tab::Atualizacao);
    assert_eq!(Tab::Atualizacao.next(true), Tab::Consumo);
}

#[test]
fn esc_focuses_quit_and_enter_opens_confirm() {
    let mut app = App::default();
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Quit);
    assert!(!key(&mut app, KeyCode::Enter));
    assert_eq!(app.focus, Focus::Confirm(true));
    assert!(key(&mut app, KeyCode::Enter));
}

#[test]
fn choosing_no_closes_confirm() {
    let mut app = App::default();
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Enter);
    assert!(!key(&mut app, KeyCode::Right));
    assert_eq!(app.focus, Focus::Confirm(false));
    assert!(!key(&mut app, KeyCode::Enter));
    assert_eq!(app.focus, Focus::Quit);
}

#[test]
fn esc_backs_out_of_confirm_and_quit() {
    let mut app = App::default();
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Enter);
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Quit);
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Tabs);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
}

#[test]
fn esc_in_insert_mode_stays_in_chat() {
    let mut app = App::default();
    key(&mut app, KeyCode::Char('i'));
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Tabs);
    // Já no modo NORMAL, o Esc volta a ir para "Sair".
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::Quit);
}

#[test]
fn tab_in_insert_mode_does_not_switch_tabs() {
    let mut app = App::default();
    key(&mut app, KeyCode::Char('i'));
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
}

#[test]
fn config_tab_changes_settings_and_tab_still_switches() {
    use crate::agent::PermissionMode;
    use crate::settings::StartMode;

    let mut app = App::default();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
    // Provedor e Conta são as primeiras; ↓↓ vai para o Modo inicial.
    assert!(!key(&mut app, KeyCode::Down));
    assert!(!key(&mut app, KeyCode::Down));
    assert!(!key(&mut app, KeyCode::Right));
    assert_eq!(
        app.settings.start_mode,
        StartMode::Fixed(PermissionMode::Agente)
    );
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Quit, "Esc continua indo para Sair");
    key(&mut app, KeyCode::Esc);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
}

#[test]
fn default_model_chosen_in_chat_goes_to_settings() {
    let mut app = App::default();
    app.chat
        .default_choice_for_test(("haiku".into(), Some("low".into())));
    app.take_default_model();
    assert_eq!(app.settings.model.as_deref(), Some("haiku"));
    assert_eq!(app.settings.effort.as_deref(), Some("low"));
}

#[test]
fn usage_command_opens_the_consumo_tab() {
    let mut app = App::default();
    app.chat.open_usage_for_test();
    // Qualquer tecla que o chat trate (aqui, entrar no modo INSERT).
    key(&mut app, KeyCode::Char('i'));
    assert_eq!(app.tab, Tab::Consumo);
}

#[test]
fn config_changes_are_saved_in_the_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::settings::config_file(dir.path(), "terra");
    let mut app = App {
        config: Some(config.clone()),
        ..App::default()
    };
    app.tab = Tab::Config;
    key(&mut app, KeyCode::Down); // Conta
    key(&mut app, KeyCode::Down); // Modo inicial
    key(&mut app, KeyCode::Right);
    let saved = AgentSettings::load_file(&config).expect("config.json gravado");
    assert_eq!(saved, app.settings);
    assert_ne!(saved, AgentSettings::default());
}

#[test]
fn extensions_toggled_in_config_are_saved_and_reach_the_chat() {
    use crate::extension::{Contribution, Extension, Hook, Status};

    struct Fake;
    impl Extension for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn description(&self) -> &'static str {
            "faz"
        }
        fn homepage(&self) -> &'static str {
            "https://fake"
        }
        fn status(&self) -> Status {
            Status::Installed { version: None }
        }
        fn contribution(&self) -> Result<Contribution, String> {
            Ok(Contribution {
                hooks: vec![Hook {
                    event: "PreToolUse".into(),
                    matcher: "Bash".into(),
                    command: "fake".into(),
                }],
                mcp_servers: Vec::new(),
                env: Vec::new(),
                launchers: Vec::new(),
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let config = crate::settings::config_file(dir.path(), "terra");
    let mut app = App {
        config: Some(config.clone()),
        db: Some(Db::open_in_memory().unwrap()),
        registry: Registry::new(vec![Box::new(Fake)]),
        tab: Tab::Config,
        ..App::default()
    };
    let saved = |app: &App| {
        let db = app.db.as_ref().unwrap();
        db.extension_settings(&app.extension_account(), &[])
            .unwrap()
    };
    assert!(!key(&mut app, KeyCode::BackTab));
    assert!(!key(&mut app, KeyCode::Char(' ')));
    assert_eq!(saved(&app).enabled, ["fake"]);
    assert_eq!(app.chat.extensions().hooks[0].command, "fake");
    // O agente continua igual.
    assert_eq!(
        AgentSettings::load_file(&config).unwrap_or_default(),
        AgentSettings::default()
    );

    // Na outra conta, só fica a de usar sempre.
    let previous = app.settings.clone();
    app.settings.account = Some(".claude-trabalho".into());
    app.save_settings(&previous);
    assert!(app.extensions.active().is_empty());
    assert_eq!(app.chat.extensions(), &Contribution::default());
    let previous = app.settings.clone();
    app.settings.account = None;
    app.save_settings(&previous);
    assert_eq!(app.extensions.enabled, ["fake"]);

    key(&mut app, KeyCode::Char(' '));
    assert_eq!(saved(&app).always, ["fake"]);
    let previous = app.settings.clone();
    app.settings.account = Some(".claude-trabalho".into());
    app.save_settings(&previous);
    assert_eq!(app.chat.extensions().hooks[0].command, "fake");
    let previous = app.settings.clone();
    app.settings.account = None;
    app.save_settings(&previous);

    key(&mut app, KeyCode::Char(' '));
    assert_eq!(saved(&app), ExtensionSettings::default());
    assert_eq!(app.chat.extensions(), &Contribution::default());

    // A aba Extensões fica entre Consumo e o Orquestrador.
    app.tab = Tab::Consumo;
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Extensoes);
    assert!(!key(&mut app, KeyCode::Down));
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Orquestrador);
}

#[test]
fn provider_interface_opens_with_enter_and_chat_keys_are_not_used() {
    let mut app = App {
        settings: AgentSettings {
            interface: Interface::Provider,
            ..AgentSettings::default()
        },
        ..App::default()
    };
    // `i` entraria no modo INSERT do chat do terra.
    key(&mut app, KeyCode::Char('i'));
    assert!(!app.open_provider);
    key(&mut app, KeyCode::Enter);
    assert!(app.open_provider);
    // Sem provedor (nos testes), não há comando e a aba mostra o porquê.
    app.sync_term(Rect::new(0, 0, 80, 24));
    assert!(app.term.is_none());
    assert!(!app.open_provider);
    assert!(matches!(app.provider_result, Some(Err(_))));
    assert_eq!(
        press(&mut app, KeyCode::Tab),
        Tab::Consumo,
        "Tab troca a aba"
    );
}

#[cfg(unix)]
#[test]
fn switching_interface_keeps_the_terminal_only_with_a_session() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut app = App {
        settings: AgentSettings {
            interface: Interface::Provider,
            ..AgentSettings::default()
        },
        ..App::default()
    };
    let cat = Command::new("cat");
    app.term = Some(Term::spawn(&cat, 20, 80).unwrap());
    app.term_focus = true;

    // Sem mensagem enviada, trocar para o Agente Terraforme fecha.
    app.settings.interface = Interface::Terraforme;
    app.sync_term(screen);
    assert!(app.term.is_none());

    // Com uma mensagem enviada, continua rodando e reaparece.
    app.settings.interface = Interface::Provider;
    app.term = Some(Term::spawn(&cat, 20, 80).unwrap());
    app.term_focus = true;
    key(&mut app, KeyCode::Enter);
    app.settings.interface = Interface::Terraforme;
    app.sync_term(screen);
    assert!(app.term.is_some(), "a sessão não é fechada");
    assert!(!app.captures_keys());
    app.settings.interface = Interface::Provider;
    app.sync_term(screen);
    assert!(app.term.is_some());
}

#[test]
fn opening_creates_the_workspace_and_keeps_what_exists() {
    let dir = tempfile::tempdir().unwrap();
    let files = ConfigFiles {
        workspace: crate::settings::config_file(dir.path(), "projeto"),
        history: crate::settings::history_file(dir.path(), "projeto"),
        global: crate::settings::global_file(dir.path()),
    };

    files.create().unwrap();
    assert_eq!(
        AgentSettings::load_file(&files.workspace),
        Some(AgentSettings::default()),
        "o arquivo novo é o padrão do router"
    );
    assert!(files.history.is_file());

    let settings = AgentSettings {
        model: Some("opus".into()),
        ..AgentSettings::default()
    };
    settings.save_file(&files.workspace).unwrap();
    files.create().unwrap();
    assert_eq!(AgentSettings::load_file(&files.workspace), Some(settings));
}

/// Uma tela num workspace não salvo, com o `config.json` em `dir`.
fn unsaved_app(dir: &std::path::Path) -> App {
    App {
        config: Some(crate::settings::config_file(dir, "projeto")),
        workspace: Some(Workspace {
            dir: dir.join("projeto"),
            saved: false,
        }),
        db: Some(Db::open_in_memory().unwrap()),
        ..App::default()
    }
}

/// Esc, Enter em Sair e Enter no "Sim" da confirmação.
fn confirm_quit(app: &mut App) -> bool {
    key(app, KeyCode::Esc);
    key(app, KeyCode::Enter);
    key(app, KeyCode::Enter)
}

#[test]
fn quitting_an_unsaved_workspace_asks_and_yes_saves_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unsaved_app(dir.path());
    assert!(!confirm_quit(&mut app), "ainda não saiu: pergunta");
    assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::Yes));
    assert!(key(&mut app, KeyCode::Enter), "Sim sai");
    let db = app.db.as_ref().unwrap();
    assert_eq!(
        db.workspace_name(&dir.path().join("projeto")).unwrap(),
        Some("projeto".into())
    );
}

#[test]
fn no_quits_without_saving_and_asks_again_next_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unsaved_app(dir.path());
    confirm_quit(&mut app);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::No));
    assert!(key(&mut app, KeyCode::Enter));
    assert!(!app.workspace.as_ref().unwrap().saved);
    assert!(unsaved_app(dir.path()).asks_to_save());
}

#[test]
fn never_ask_quits_and_stops_asking_in_this_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unsaved_app(dir.path());
    confirm_quit(&mut app);
    // ← dá a volta: do Sim para a última opção.
    key(&mut app, KeyCode::Left);
    assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::NeverAsk));
    assert!(key(&mut app, KeyCode::Enter));

    // Da próxima vez, o Sim da confirmação já sai.
    let mut app = unsaved_app(dir.path());
    assert!(confirm_quit(&mut app));
}

#[test]
fn saved_workspace_quits_without_asking_and_esc_cancels_the_question() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unsaved_app(dir.path());
    confirm_quit(&mut app);
    assert!(!key(&mut app, KeyCode::Esc));
    assert_eq!(app.focus, Focus::Quit, "Esc desiste de sair");

    app.workspace.as_mut().unwrap().saved = true;
    key(&mut app, KeyCode::Enter);
    assert!(key(&mut app, KeyCode::Enter), "salvo: sai direto");
}

#[test]
fn save_question_shows_the_three_options() {
    use ratatui::{Terminal, backend::TestBackend};

    let dir = tempfile::tempdir().unwrap();
    let mut app = unsaved_app(dir.path());
    confirm_quit(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen = (0..14)
        .map(|y| (0..70).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("Salvar workspace"), "{screen}");
    assert!(screen.contains(SAVE_TEXT), "{screen}");
    assert!(
        screen.contains("Sim       Não       Não (não perguntar novamente)"),
        "{screen}"
    );
}

#[test]
fn tab_on_quit_returns_to_tabs_without_switching() {
    let mut app = App::default();
    key(&mut app, KeyCode::Esc);
    assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    assert_eq!(app.focus, Focus::Tabs);
}

#[test]
fn other_keys_do_nothing() {
    let mut app = App::default();
    for code in [
        KeyCode::Right,
        KeyCode::Left,
        KeyCode::BackTab,
        KeyCode::Char('2'),
        KeyCode::Char('q'),
        KeyCode::Enter,
    ] {
        assert!(!app.handle_key(KeyEvent::new(code, KeyModifiers::NONE)));
        assert_eq!(app.tab, Tab::Chat);
    }
}

#[test]
fn nav_bar_highlights_active_tab_and_right_aligns_quit() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut terminal = Terminal::new(TestBackend::new(80, 8)).unwrap();
    let mut app = App::default();
    press(&mut app, KeyCode::Tab);
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buffer = terminal.backend().buffer();
    let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    assert_eq!(row(0), format!("╭{}╮", "─".repeat(78)));
    assert_eq!(
        row(1),
        "│  Chat   Consumo   Extensões   Orquestrador   Configurações             Sair  │"
    );
    assert_eq!(row(2), format!("╰{}╯", "─".repeat(78)));

    // O pill " Consumo " (x = 9..18) usa o estilo da aba ativa; "Chat" não.
    let active_bg = theme::tab_active().bg;
    assert!((9..18).all(|x| buffer[(x, 1)].bg == active_bg.unwrap()));
    assert_ne!(buffer[(3, 1)].bg, active_bg.unwrap());

    // "Sair" (x = 72..78) é vermelho, sem fundo até receber foco.
    let red = theme::quit().fg.unwrap();
    assert!((72..78).all(|x| buffer[(x, 1)].fg == red));
}

#[test]
fn new_chats_show_up_below_the_nav_bar() {
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    let mut app = App::default();
    app.new_chat();
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let row = |y: u16| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    assert!(row(4).starts_with("│  Principal   Chat 2 "), "{}", row(4));
    // Com Alt+N, o chat novo fica ativo; o Principal não.
    assert_eq!(buffer[(17, 4)].bg, theme::tab_active().bg.unwrap());
    assert_ne!(buffer[(3, 4)].bg, theme::tab_active().bg.unwrap());

    let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
    // Alt+N deixa o chat novo ativo; Alt+→ dá a volta ao Principal.
    app.handle_key(alt(KeyCode::Right));
    assert_eq!(app.active_chat, 0);
    app.handle_key(alt(KeyCode::Left));
    assert_eq!(app.active_chat, 1);
    app.handle_key(alt(KeyCode::Char('w')));
    assert_eq!(app.chats.len(), 0, "o chat novo fecha");
    assert_eq!(app.active_chat, 0);
    app.handle_key(alt(KeyCode::Char('w')));
    assert_eq!(app.chats.len(), 0, "o chat do terra não fecha");
}

#[test]
fn alt_n_opens_a_new_chat_in_the_same_terra() {
    use crossterm::event::KeyModifiers;

    let mut app = App::default();
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
    assert_eq!(app.chats.len(), 1);
    assert_eq!(app.active_chat, 1);
    assert_eq!(app.chats[0].title, "Chat 2");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
    assert_eq!(app.active_chat, 2);
    assert_eq!(app.chats[1].title, "Chat 3");
}

#[test]
fn a_dispatched_run_opens_a_read_only_tab_that_follows_its_output() {
    use crate::orchestrator::run::{Run, RunState};
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output.jsonl");
    let mut file = std::fs::File::create(&output).unwrap();
    let run = Run {
        id: "abcdef0123".into(),
        agent: "api".into(),
        message: "m1".into(),
        from: "web".into(),
        level: 2,
        hops: 1,
        note: "/n.md".into(),
        state: RunState::Running,
        created: 0,
        started: None,
        finished: None,
        provider: "claude".into(),
        model: None,
        usage: None,
        error: None,
        summary: None,
    };
    let mut app = App::default();
    app.show_run(&run, &output);
    assert_eq!(app.chats.len(), 1);
    assert_eq!(app.chats[0].title, "▶ api");
    // Em segundo plano por padrão: a aba aberta continua a mesma.
    assert_eq!(app.active_chat, 0);
    let status = app.hub_status();
    assert_eq!(status.tabs[1].run.as_deref(), Some("abcdef0123"));
    assert_eq!(status.tabs[1].to_json()["kind"], "execucao");

    writeln!(
        file,
        r#"{{"type":"result","subtype":"success","is_error":false,"result":"feito","usage":{{}}}}"#
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.chats[0].chat.activity().is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "a aba não viu o resultado"
        );
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn confirm_box_shows_question_and_buttons() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut terminal = Terminal::new(TestBackend::new(50, 14)).unwrap();
    let mut app = App::default();
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buffer = terminal.backend().buffer();
    let screen = (0..14)
        .map(|y| (0..50).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains(CONFIRM_TEXT));
    assert!(screen.contains("Sim"));
    assert!(screen.contains("Não"));
}
