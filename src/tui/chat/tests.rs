//! Testes do Chat: envio, eventos do agente, sincronia do modo, badges e
//! seleção no histórico. A edição do texto é testada em [`super::input`].
//!
//! O agente é um [`FakeAdapter`]: o chat só conhece o contrato
//! [`ChatAdapter`], então qualquer provedor serve.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use crossterm::event::KeyModifiers;
use ratatui::{Terminal, backend::TestBackend};

use super::*;

/// O que o chat pediu ao provedor: texto, modo, sessão e modelo.
type Sent = Rc<RefCell<Vec<(String, PermissionMode, Option<String>, Option<String>)>>>;

/// Provedor de mentira: responde cada mensagem com os eventos de `reply` e
/// guarda o que recebeu.
struct FakeAdapter {
    available: bool,
    reply: Vec<AgentEvent>,
    sent: Sent,
}

struct FakeSession(Vec<AgentEvent>);

impl Session for FakeSession {
    fn poll(&mut self) -> Vec<AgentEvent> {
        std::mem::take(&mut self.0)
    }
}

impl ChatAdapter for FakeAdapter {
    fn name(&self) -> &str {
        "Claude"
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn unavailable_hint(&self) -> String {
        "Claude CLI não encontrado no PATH.".to_string()
    }

    fn model_label(&self, id: &str) -> String {
        format!("modelo {id}")
    }

    fn load_catalog(
        &self,
        _account: Option<&str>,
    ) -> Option<mpsc::Receiver<Result<Catalog, String>>> {
        let (tx, rx) = mpsc::channel();
        let commands = ["clear", "compact", "model"]
            .iter()
            .map(|name| agent::CommandInfo {
                name: name.to_string(),
                description: format!("faz {name}"),
            })
            .collect();
        let model = |id: &str, efforts: &[&str]| agent::ModelInfo {
            id: id.to_string(),
            label: id.to_string(),
            description: format!("sobre {id}"),
            resolved: Some(format!("claude-{id}-5-5")),
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
        };
        let all = ["low", "medium", "high", "xhigh", "max"];
        tx.send(Ok(Catalog {
            commands,
            models: vec![model("opus", &all), model("haiku", &[])],
            default_model: Some("claude-opus-5-5".into()),
        }))
        .unwrap();
        Some(rx)
    }

    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String> {
        self.sent.borrow_mut().push((
            request.prompt.to_string(),
            request.mode,
            request.session_id.map(str::to_string),
            request.model.map(str::to_string),
        ));
        Ok(Box::new(FakeSession(self.reply.clone())))
    }
}

fn fake_chat(available: bool, reply: Vec<AgentEvent>) -> (Chat, Sent) {
    fake_chat_with(available, reply, &AgentSettings::default())
}

fn fake_chat_with(
    available: bool,
    reply: Vec<AgentEvent>,
    settings: &AgentSettings,
) -> (Chat, Sent) {
    let sent = Sent::default();
    let adapter = FakeAdapter {
        available,
        reply,
        sent: Rc::clone(&sent),
    };
    (Chat::with_adapter(Box::new(adapter), settings), sent)
}

fn done(tokens: u64) -> AgentEvent {
    AgentEvent::Done {
        is_error: false,
        message: None,
        tokens,
        denied: 0,
    }
}

fn press(chat: &mut Chat, code: KeyCode) -> bool {
    chat.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_text(chat: &mut Chat, text: &str) {
    for c in text.chars() {
        press(chat, KeyCode::Char(c));
    }
}

fn mouse(chat: &mut Chat, kind: MouseEventKind, column: u16, row: u16) {
    chat.handle_mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

/// Desenha o painel num terminal de teste (60 colunas).
fn render(chat: &Chat) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(60, HEIGHT)).unwrap();
    terminal
        .draw(|frame| chat.draw(frame, frame.area()))
        .unwrap();
    terminal
}

fn init(mode: PermissionMode) -> AgentEvent {
    AgentEvent::Init {
        session_id: "s1".into(),
        model: "opus".into(),
        permission_mode: ReportedMode::Known(mode),
    }
}

#[test]
fn sends_through_adapter_and_continues_the_session() {
    let reply = vec![
        init(PermissionMode::Agente),
        AgentEvent::TextStart,
        AgentEvent::TextDelta("oi!".into()),
        done(7),
    ];
    let (mut chat, sent) = fake_chat(true, reply);
    assert_eq!(chat.agent_status, AgentStatus::Ocioso);

    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "olá");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(chat.input.text(), "", "o input esvazia ao enviar");
    assert_eq!(chat.agent_status, AgentStatus::Pensando);

    chat.tick();
    assert_eq!(chat.agent_status, AgentStatus::Ocioso);
    assert_eq!(
        chat.history.entries(),
        [Entry::User("olá".into()), Entry::Assistant("oi!".into())]
    );
    assert_eq!(chat.tokens_used, 7);
    assert_eq!(chat.model_title().to_string(), "  Claude  modelo opus ");

    // A segunda mensagem continua a sessão informada no init.
    press(&mut chat, KeyCode::BackTab);
    type_text(&mut chat, "e agora?");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(
        *sent.borrow(),
        [
            ("olá".to_string(), PermissionMode::Agente, None, None),
            (
                "e agora?".to_string(),
                PermissionMode::Yolo,
                Some("s1".to_string()),
                None
            ),
        ]
    );
}

#[test]
fn settings_choose_start_mode_and_model() {
    let settings = AgentSettings {
        start_mode: StartMode::Fixed(PermissionMode::Plan),
        model: Some("haiku".into()),
        account: None,
        effort: None,
    };
    let (mut chat, sent) = fake_chat_with(true, vec![done(1)], &settings);
    assert_eq!(chat.permission_mode, PermissionMode::Plan);

    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "oi");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(sent.borrow()[0].3.as_deref(), Some("haiku"));
}

#[test]
fn changing_only_the_model_keeps_shift_tab_choice() {
    let settings = AgentSettings::default();
    let (mut chat, _) = fake_chat_with(true, Vec::new(), &settings);
    press(&mut chat, KeyCode::BackTab);
    assert_eq!(chat.permission_mode, PermissionMode::Yolo);

    let changed = AgentSettings {
        model: Some("opus".into()),
        ..settings.clone()
    };
    chat.apply_settings(&changed, Some(&settings));
    assert_eq!(chat.permission_mode, PermissionMode::Yolo);
    assert_eq!(chat.chosen_model.as_deref(), Some("opus"));

    // Trocar o modo inicial muda o modo na hora.
    let fixed = AgentSettings {
        start_mode: StartMode::Fixed(PermissionMode::AcceptEdits),
        ..changed.clone()
    };
    chat.apply_settings(&fixed, Some(&changed));
    assert_eq!(chat.permission_mode, PermissionMode::AcceptEdits);
}

#[test]
fn switching_account_starts_a_new_conversation() {
    let reply = vec![init(PermissionMode::Agente), done(1)];
    let (mut chat, _) = fake_chat(true, reply);
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "oi");
    press(&mut chat, KeyCode::Enter);
    chat.tick();
    assert_eq!(chat.session_id.as_deref(), Some("s1"));

    let before = AgentSettings::default();
    let work = AgentSettings {
        account: Some(".claude-trabalho".into()),
        ..before.clone()
    };
    chat.apply_settings(&work, Some(&before));
    assert_eq!(chat.account.as_deref(), Some(".claude-trabalho"));
    assert_eq!(chat.session_id, None, "a sessão era da outra conta");
    assert_eq!(chat.model, None);
    assert_eq!(
        chat.history.entries().last(),
        Some(&Entry::Info(
            "Conta: .claude-trabalho. A próxima mensagem começa uma conversa nova.".into()
        ))
    );

    // Sem conversa, trocar de novo não repete o aviso.
    let entries = chat.history.entries().len();
    chat.apply_settings(&before, Some(&work));
    assert_eq!(chat.account, None);
    assert_eq!(chat.history.entries().len(), entries);
    chat.apply_settings(&work, Some(&before));

    // Mudar outra configuração não troca a conta nem a sessão.
    chat.session_id = Some("s2".into());
    let model = AgentSettings {
        model: Some("opus".into()),
        ..work.clone()
    };
    chat.apply_settings(&model, Some(&work));
    assert_eq!(chat.session_id.as_deref(), Some("s2"));
}

#[test]
fn slash_commands_come_from_the_agent_and_run_terra_interfaces() {
    let reply = vec![init(PermissionMode::Agente), done(1)];
    let (mut chat, _) = fake_chat(true, reply);
    chat.tick(); // chega o catálogo
    assert_eq!(chat.catalog.as_ref().map(|c| c.commands.len()), Some(3));

    // Uma conversa, e depois /clear.
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "oi");
    press(&mut chat, KeyCode::Enter);
    chat.tick();
    assert_eq!(chat.session_id.as_deref(), Some("s1"));
    type_text(&mut chat, "/clear");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(chat.session_id, None);
    assert_eq!(
        chat.history.entries(),
        [Entry::Info("Nova conversa.".into())]
    );

    // Um comando do agente que ainda não tem interface no terra.
    type_text(&mut chat, "/compact");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(
        chat.history.entries().last(),
        Some(&Entry::Info(
            "/compact ainda não tem interface no terra.".into()
        ))
    );
}

/// Abre o `/model` num chat com o catálogo carregado, escrevendo.
fn chat_with_model_menu() -> (Chat, Sent) {
    let (mut chat, sent) = fake_chat(true, vec![done(1)]);
    chat.tick();
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "/model");
    press(&mut chat, KeyCode::Enter);
    assert!(chat.model_menu.is_some(), "/model abre o menu");
    (chat, sent)
}

#[test]
fn model_menu_chooses_model_and_effort_for_this_conversation() {
    let (mut chat, sent) = chat_with_model_menu();
    assert_eq!(chat.help(), HELP_MODEL);
    press(&mut chat, KeyCode::Down); // 1. opus
    press(&mut chat, KeyCode::Right); // high -> xhigh
    press(&mut chat, KeyCode::Enter);
    assert!(chat.model_menu.is_none());
    assert_eq!(
        chat.history.entries().last(),
        Some(&Entry::Info(
            "Modelo desta conversa: modelo claude-opus-5-5 · esforço muito alto.".into()
        ))
    );
    assert_eq!(chat.take_default_choice(), None, "não mexe no padrão");

    type_text(&mut chat, "oi");
    press(&mut chat, KeyCode::Enter);
    let sent = sent.borrow();
    assert_eq!(sent[0].3.as_deref(), Some("opus"));
    assert_eq!(chat.session_effort.as_deref(), Some("xhigh"));
}

#[test]
fn model_menu_default_saves_a_new_default() {
    let (mut chat, _) = chat_with_model_menu();
    press(&mut chat, KeyCode::Enter); // Default -> menu do default, no opus
    press(&mut chat, KeyCode::Down); // haiku
    press(&mut chat, KeyCode::Enter);
    assert_eq!(chat.take_default_choice(), Some(("haiku".into(), None)));
    assert_eq!(chat.session_model, None, "a conversa segue o novo padrão");
}

#[test]
fn model_menu_esc_closes_without_changes() {
    let (mut chat, _) = chat_with_model_menu();
    press(&mut chat, KeyCode::Esc);
    assert!(chat.model_menu.is_none());
    assert_eq!(chat.session_model, None);
    assert_eq!(
        chat.model_options().len(),
        2,
        "a Config vê os modelos do catálogo"
    );
}

#[test]
fn without_adapter_the_commands_list_is_empty_not_loading() {
    let (mut chat, _) = fake_chat(false, Vec::new());
    assert!(chat.catalog_loading.is_none());
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "/");
    assert!(chat.input.picker_open());
}

#[test]
fn esc_cancels_reply_in_progress() {
    let (mut chat, _) = fake_chat(true, vec![init(PermissionMode::Agente)]);
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "olá");
    press(&mut chat, KeyCode::Enter);
    press(&mut chat, KeyCode::Esc); // sai do modo de escrita
    assert!(press(&mut chat, KeyCode::Esc)); // interrompe
    assert!(chat.run.is_none());
    assert_eq!(chat.agent_status, AgentStatus::Ocioso);
    assert_eq!(
        chat.history.entries().last(),
        Some(&Entry::Info("Interrompido.".into()))
    );
}

#[test]
fn shift_tab_cycles_permission_modes() {
    let mut chat = Chat::default();
    assert!(press(&mut chat, KeyCode::BackTab));
    assert_eq!(chat.permission_mode, PermissionMode::Yolo);
    // Também enquanto escreve.
    press(&mut chat, KeyCode::Char('i'));
    assert!(press(&mut chat, KeyCode::BackTab));
    assert_eq!(chat.permission_mode, PermissionMode::Manual);
}

#[test]
fn spinner_advances_only_with_activity() {
    let mut chat = Chat {
        agent_activity: Some("Pensando...".to_string()),
        ..Chat::default()
    };
    chat.tick();
    assert_eq!(chat.spinner_frame, 1);
    chat.agent_activity = None;
    chat.tick();
    assert_eq!(chat.spinner_frame, 1);
}

#[test]
fn agent_events_update_history_status_and_consumption() {
    let mut chat = Chat {
        agent_status: AgentStatus::Pensando,
        ..Chat::default()
    };
    for event in [
        init(PermissionMode::Agente),
        AgentEvent::TextStart,
        AgentEvent::TextDelta("o".into()),
        AgentEvent::TextDelta("i".into()),
        AgentEvent::ToolUse {
            name: "Bash".into(),
            detail: "ls".into(),
        },
    ] {
        chat.apply(event);
    }
    assert_eq!(chat.agent_activity.as_deref(), Some("Usando Bash..."));
    assert_eq!(chat.session_id.as_deref(), Some("s1"));
    assert_eq!(
        chat.model_title().to_string(),
        "  Sem agente  Modelo padrão "
    );

    chat.apply(AgentEvent::Done {
        is_error: false,
        message: None,
        tokens: 1_500,
        denied: 1,
    });
    assert_eq!(chat.agent_status, AgentStatus::Ocioso);
    assert_eq!(chat.agent_activity, None);
    assert_eq!(chat.tokens_used, 1_500);
    assert_eq!(
        chat.history.entries(),
        [
            Entry::Assistant("oi".into()),
            Entry::Tool {
                name: "Bash".into(),
                detail: "ls".into()
            },
            Entry::Info("1 ferramenta(s) bloqueada(s) pelo modo de permissão Agente.".into()),
        ]
    );
}

#[test]
fn failure_marks_error() {
    let mut chat = Chat::default();
    chat.apply(AgentEvent::Failed("sem rede".into()));
    assert_eq!(chat.agent_status, AgentStatus::Erro);
    assert_eq!(chat.history.entries(), [Entry::Error("sem rede".into())]);
}

#[test]
fn enter_without_claude_keeps_text_and_shows_error() {
    let (mut chat, sent) = fake_chat(false, Vec::new());
    assert_eq!(chat.agent_status, AgentStatus::Desconectado);
    press(&mut chat, KeyCode::Char('i'));
    type_text(&mut chat, "oi");
    press(&mut chat, KeyCode::Enter);
    assert_eq!(chat.input.text(), "oi");
    assert!(
        matches!(chat.history.entries(), [Entry::Error(msg)] if msg.contains("não encontrado"))
    );
    assert!(sent.borrow().is_empty());
}

#[test]
fn selecting_history_copies_and_shows_notice() {
    let mut chat = Chat::default();
    chat.history
        .push(Entry::Assistant("resposta do agente".into()));
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
    terminal
        .draw(|frame| chat.draw_history(frame, frame.area()))
        .unwrap();
    // "⏺ resposta do agente" na linha 1, texto a partir de x = 4.
    mouse(&mut chat, MouseEventKind::Down(MouseButton::Left), 4, 1);
    mouse(&mut chat, MouseEventKind::Drag(MouseButton::Left), 11, 1);
    mouse(&mut chat, MouseEventKind::Up(MouseButton::Left), 11, 1);
    assert_eq!(chat.help(), "Copiado (8 caracteres)");
    // O clique no histórico não começa a escrever no input.
    assert_eq!(chat.input.mode(), InputMode::Normal);
    for _ in 0..NOTICE_TICKS {
        chat.tick();
    }
    assert_eq!(chat.help(), HELP_NORMAL);
}

#[test]
fn compact_count_formats() {
    assert_eq!(compact_count(950), "950");
    assert_eq!(compact_count(1_000), "1K");
    assert_eq!(compact_count(12_300), "12.3K");
    assert_eq!(compact_count(1_000_000), "1M");
    assert_eq!(compact_count(1_500_000), "1.5M");
}

#[test]
fn panel_renders_titles_and_badges() {
    let (chat, _) = fake_chat(false, Vec::new());
    let terminal = render(&chat);
    let buffer = terminal.backend().buffer();
    let row = |y: u16| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>();

    // Sem agente, a linha de atividade fica vazia.
    assert_eq!(row(0).trim(), "");
    assert!(row(1).starts_with("╭  Claude  Modelo padrão ─"));
    assert!(row(1).trim_end().ends_with(" Desconectado ╮"));
    assert!(!row(5).contains("NORMAL"));
    // Modo padrão e nada consumido: sem badges, e o painel encolhe.
    assert_eq!(row(7).trim(), "");
    assert_eq!(chat.height(), HEIGHT - badge::HEIGHT);
}

#[test]
fn input_keeps_full_height_when_badges_are_hidden() {
    // Regressão: sem badges o painel encolhe, e o input ficava com 1 linha.
    let (chat, _) = fake_chat(true, Vec::new());
    let height = chat.height();
    let mut terminal = Terminal::new(TestBackend::new(60, height)).unwrap();
    terminal
        .draw(|frame| chat.draw(frame, frame.area()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let first = |y: u16| buffer[(0, y)].symbol().to_string();
    // Atividade (y = 0), borda de cima (1), três linhas de texto e a borda de
    // baixo na última linha.
    assert_eq!(first(1), "╭");
    assert_eq!([first(2), first(3), first(4)], ["│", "│", "│"]);
    assert_eq!(first(height - 1), "╰");
}

#[test]
fn init_syncs_mode_when_agent_differs() {
    let mut chat = Chat {
        requested_mode: Some(PermissionMode::Plan),
        permission_mode: PermissionMode::Plan,
        ..Chat::default()
    };
    chat.apply(init(PermissionMode::AcceptEdits));
    assert_eq!(chat.agent_mode, Some(PermissionMode::AcceptEdits));
    assert_eq!(chat.permission_mode, PermissionMode::AcceptEdits);
    assert_eq!(
        chat.history.entries(),
        [Entry::Info("O agente está no modo Accept Edits.".into())]
    );
}

#[test]
fn yolo_is_not_synced_with_agent() {
    // No Yolo o agente roda em "manual" (default) e pede; o terra aprova.
    let mut chat = Chat {
        requested_mode: Some(PermissionMode::Yolo),
        permission_mode: PermissionMode::Yolo,
        ..Chat::default()
    };
    chat.apply(init(PermissionMode::Manual));
    assert_eq!(chat.permission_mode, PermissionMode::Yolo);
    assert_eq!(chat.agent_mode, Some(PermissionMode::Yolo));
    assert!(chat.history.entries().is_empty());
    assert_eq!(chat.badges()[0].0, "Modo: Yolo");
}

#[test]
fn init_keeps_shift_tab_choice_made_during_reply() {
    // Pediu Agente, trocou para Plan com Shift+Tab antes do init chegar.
    let mut chat = Chat {
        requested_mode: Some(PermissionMode::Agente),
        permission_mode: PermissionMode::Plan,
        ..Chat::default()
    };
    chat.apply(init(PermissionMode::Agente));
    assert_eq!(chat.agent_mode, Some(PermissionMode::Agente));
    assert_eq!(chat.permission_mode, PermissionMode::Plan);
    assert!(chat.history.entries().is_empty());
}

#[test]
fn unknown_agent_mode_is_reported() {
    let mut chat = Chat::default();
    chat.apply(AgentEvent::Init {
        session_id: "s1".into(),
        model: "opus".into(),
        permission_mode: ReportedMode::Unknown("dontAsk".into()),
    });
    assert_eq!(chat.agent_mode, None);
    assert!(matches!(chat.history.entries(), [Entry::Info(msg)] if msg.contains("dontAsk")));
}

#[test]
fn mode_badge_shows_once_agent_is_active() {
    let mut chat = Chat::default();
    assert!(chat.badges().is_empty());
    chat.apply(init(PermissionMode::Agente));
    assert_eq!(
        chat.badges(),
        [("Modo: Agente".to_string(), Variant::Active)]
    );
}

#[test]
fn badges_appear_only_when_active() {
    let badge_row = |chat: &Chat| {
        let terminal = render(chat);
        let buffer = terminal.backend().buffer();
        (0..60)
            .map(|x| buffer[(x, 7)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let mut chat = Chat::default();

    // Fora do modo padrão, aparece o modo.
    press(&mut chat, KeyCode::BackTab);
    assert_eq!(badge_row(&chat), "│ Modo: Yolo │");
    assert_eq!(chat.height(), HEIGHT);

    // Depois de consumir, aparece o consumo também.
    chat.tokens_used = 1_500;
    assert_eq!(badge_row(&chat), "│ Modo: Yolo │ │ Consumo: 1.5K Tokens │");

    // De volta ao padrão, fica só o consumo.
    for _ in 0..4 {
        press(&mut chat, KeyCode::BackTab);
    }
    assert_eq!(chat.permission_mode, PermissionMode::Agente);
    assert_eq!(badge_row(&chat), "│ Consumo: 1.5K Tokens │");
}
