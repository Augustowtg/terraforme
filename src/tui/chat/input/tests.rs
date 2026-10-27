//! Testes da caixa de input: edição, várias linhas, seleção, Insert,
//! indicadores e menções de arquivo com `@`.

use super::*;
use ratatui::{Terminal, backend::TestBackend};

fn press(input: &mut InputBox, code: KeyCode) -> Outcome {
    input.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn press_with(input: &mut InputBox, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
    input.handle_key(KeyEvent::new(code, modifiers))
}

fn type_text(input: &mut InputBox, text: &str) {
    for c in text.chars() {
        press(input, KeyCode::Char(c));
    }
}

/// Desenha a caixa com 60 colunas a partir de y = 1 (como no painel, abaixo
/// da linha de atividade): o texto começa em x = 2 e y = 2, e a borda de
/// baixo fica em y = 5.
fn render(input: &InputBox) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(60, 1 + HEIGHT)).unwrap();
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 1, 60, HEIGHT);
            input.draw(frame, area, Line::default(), Line::default());
        })
        .unwrap();
    terminal
}

fn mouse(input: &mut InputBox, kind: MouseEventKind, column: u16, row: u16) -> bool {
    input.handle_mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Caixa no modo NORMAL com `text`, já desenhada.
fn input_with(text: &str) -> InputBox {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, text);
    press(&mut input, KeyCode::Esc);
    render(&input);
    input
}

fn select(input: &mut InputBox, from: u16, to: u16) {
    mouse(input, MouseEventKind::Down(MouseButton::Left), 2 + from, 2);
    mouse(input, MouseEventKind::Drag(MouseButton::Left), 2 + to, 2);
    mouse(input, MouseEventKind::Up(MouseButton::Left), 2 + to, 2);
}

#[test]
fn normal_mode_ignores_typing() {
    let mut input = InputBox::default();
    assert_eq!(press(&mut input, KeyCode::Char('x')), Outcome::Ignored);
    assert_eq!(press(&mut input, KeyCode::Esc), Outcome::Ignored);
    assert!(input.text().is_empty());
}

#[test]
fn insert_mode_edits_buffer_and_esc_returns_to_normal() {
    let mut input = InputBox::default();
    assert_eq!(press(&mut input, KeyCode::Char('i')), Outcome::Handled);
    assert_eq!(input.mode(), InputMode::Insert);
    type_text(&mut input, "olá");
    press(&mut input, KeyCode::Left);
    press(&mut input, KeyCode::Backspace);
    press(&mut input, KeyCode::Char('L'));
    assert_eq!(input.text(), "oLá");
    assert_eq!(input.cursor, 2);

    assert_eq!(press(&mut input, KeyCode::Esc), Outcome::Handled);
    assert_eq!(input.mode(), InputMode::Normal);
    assert_eq!(input.text(), "oLá");
}

#[test]
fn enter_submits_and_clear_empties() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "oi");
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Submit);
    assert_eq!(input.text(), "oi");
    input.clear();
    assert_eq!(input.text(), "");
    assert_eq!(input.mode(), InputMode::Insert);
}

#[test]
fn insert_key_toggles_overwrite() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "abcd");
    press(&mut input, KeyCode::Home);
    press(&mut input, KeyCode::Insert);
    type_text(&mut input, "XY");
    assert_eq!(input.text(), "XYcd");
    // No fim do texto, sobrescrever só acrescenta.
    press(&mut input, KeyCode::End);
    type_text(&mut input, "!");
    assert_eq!(input.text(), "XYcd!");
    // Insert de novo volta a inserir.
    press(&mut input, KeyCode::Home);
    press(&mut input, KeyCode::Insert);
    type_text(&mut input, "_");
    assert_eq!(input.text(), "_XYcd!");
}

#[test]
fn insert_mode_ignores_ctrl_chords() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    press_with(&mut input, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(input.text().is_empty());
}

#[test]
fn cursor_blinks_every_blink_ticks() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    for _ in 0..BLINK_TICKS - 1 {
        input.tick();
    }
    assert!(input.cursor_visible);
    input.tick();
    assert!(!input.cursor_visible);
    press(&mut input, KeyCode::Char('x'));
    assert!(input.cursor_visible);
}

#[test]
fn shift_enter_inserts_newline() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "ab");
    press_with(&mut input, KeyCode::Enter, KeyModifiers::SHIFT);
    type_text(&mut input, "cd");
    press_with(&mut input, KeyCode::Enter, KeyModifiers::ALT);
    press_with(&mut input, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(input.text(), "ab\ncd\n\n");
    // Enter sozinho não pula linha (envia a mensagem).
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Submit);
    assert_eq!(input.text(), "ab\ncd\n\n");
}

#[test]
fn backslash_enter_inserts_newline() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "ab\\");
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Handled);
    type_text(&mut input, "cd");
    assert_eq!(input.text(), "ab\ncd");
    assert_eq!(input.cursor, 5);

    // Só a `\` logo antes do cursor conta.
    type_text(&mut input, "\\x");
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Submit);
    assert_eq!(input.text(), "ab\ncd\\x");
}

#[test]
fn overwrite_does_not_eat_newline() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "a");
    press_with(&mut input, KeyCode::Enter, KeyModifiers::SHIFT);
    type_text(&mut input, "b");
    press(&mut input, KeyCode::Home);
    press(&mut input, KeyCode::Insert);
    type_text(&mut input, "XY");
    assert_eq!(input.text(), "XY\nb");
}

#[test]
fn newlines_render_on_separate_rows_with_cursor_below() {
    let mut input = InputBox::default();
    press(&mut input, KeyCode::Char('i'));
    type_text(&mut input, "ab");
    press_with(&mut input, KeyCode::Enter, KeyModifiers::SHIFT);
    type_text(&mut input, "cd");
    let mut terminal = render(&input);
    let buffer = terminal.backend().buffer().clone();
    let row = |y: u16| (2..10).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    assert_eq!(row(2).trim_end(), "ab");
    assert_eq!(row(3).trim_end(), "cd");
    assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(4, 3));

    // Clicar na segunda linha posiciona depois do `\n`.
    mouse(&mut input, MouseEventKind::Down(MouseButton::Left), 3, 3);
    assert_eq!(input.cursor, 4);
    // Depois do fim da primeira linha, fica antes do `\n`.
    mouse(&mut input, MouseEventKind::Down(MouseButton::Left), 30, 2);
    assert_eq!(input.cursor, 2);
}

#[test]
fn wrap_rows_breaks_by_display_width() {
    let plain = |c: char| c.width().unwrap_or(0);
    assert_eq!(wrap_rows("", 4, plain), vec![Range { start: 0, end: 0 }]);
    assert_eq!(wrap_rows("abcdef", 4, plain), [0..4, 4..6]);
    // Cada ideograma ocupa duas colunas.
    assert_eq!(wrap_rows("日本語", 4, plain), [0..2, 2..3]);
    assert_eq!(wrap_rows("ab\n\ncd\n", 10, plain), [0..2, 3..3, 4..6, 7..7]);
}

#[test]
fn click_places_cursor_and_starts_typing() {
    let mut input = input_with("hello world");
    assert!(mouse(
        &mut input,
        MouseEventKind::Down(MouseButton::Left),
        2 + 6,
        2
    ));
    mouse(&mut input, MouseEventKind::Up(MouseButton::Left), 2 + 6, 2);
    assert_eq!(input.mode(), InputMode::Insert);
    assert_eq!(input.cursor, 6);
    assert_eq!(input.selection(), None);

    // Depois do fim da linha, o cursor vai para o fim do texto.
    mouse(&mut input, MouseEventKind::Down(MouseButton::Left), 40, 3);
    assert_eq!(input.cursor, 11);
}

#[test]
fn click_outside_input_is_ignored() {
    let mut input = input_with("hello");
    assert!(!mouse(
        &mut input,
        MouseEventKind::Down(MouseButton::Left),
        2,
        0
    ));
    assert_eq!(input.mode(), InputMode::Normal);
}

#[test]
fn drag_selects_and_typing_replaces_selection() {
    let mut input = input_with("hello world");
    select(&mut input, 6, 11);
    assert_eq!(input.selection(), Some(6..11));
    type_text(&mut input, "X");
    assert_eq!(input.text(), "hello X");
    assert_eq!(input.selection(), None);
}

#[test]
fn drag_backwards_selects_too() {
    let mut input = input_with("hello world");
    select(&mut input, 5, 0);
    assert_eq!(input.selection(), Some(0..5));
    press(&mut input, KeyCode::Backspace);
    assert_eq!(input.text(), " world");
}

#[test]
fn arrow_keys_clear_selection() {
    let mut input = input_with("hello world");
    select(&mut input, 0, 5);
    press(&mut input, KeyCode::Left);
    assert_eq!(input.selection(), None);
    assert_eq!(input.cursor, 4);
    press(&mut input, KeyCode::Delete);
    assert_eq!(input.text(), "hell world");
}

#[test]
fn selection_is_highlighted_and_cursor_is_terminal_bar() {
    let mut input = input_with("hello world");
    select(&mut input, 0, 5);
    let mut terminal = render(&input);
    let buffer = terminal.backend().buffer().clone();
    let bg = theme::selection().bg;
    assert!((2..7).all(|x| buffer[(x, 2)].bg == bg.unwrap()));
    assert_ne!(buffer[(8, 2)].bg, bg.unwrap());
    // O cursor do terminal fica na ponta da seleção, sem ocupar coluna.
    assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(7, 2));
}

/// Caixa escrevendo, com um índice fixo de arquivos.
fn input_with_files() -> InputBox {
    let mut input = InputBox::default();
    input.set_files(FileIndex::fixed(
        ["README.md", "src/", "src/main.rs", "src/tui/app.rs"]
            .iter()
            .map(|p| FileEntry {
                path: p.trim_end_matches('/').to_string(),
                is_dir: p.ends_with('/'),
            })
            .collect(),
    ));
    press(&mut input, KeyCode::Char('i'));
    input
}

#[test]
fn at_opens_picker_and_tab_inserts_badge() {
    let mut input = input_with_files();
    type_text(&mut input, "veja @");
    assert!(input.picker_open());
    type_text(&mut input, "main");
    assert_eq!(
        input
            .found()
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>(),
        ["src/main.rs"]
    );
    assert_eq!(press(&mut input, KeyCode::Tab), Outcome::Handled);
    assert!(!input.picker_open());
    // O badge vai como `@caminho` no texto enviado, seguido de um espaço.
    assert_eq!(input.text(), "veja @src/main.rs ");
    type_text(&mut input, "agora");
    assert_eq!(input.text(), "veja @src/main.rs agora");
}

#[test]
fn badge_renders_as_gray_pill_and_backspace_removes_it_whole() {
    let mut input = input_with_files();
    type_text(&mut input, "@main");
    press(&mut input, KeyCode::Enter);
    let terminal = render(&input);
    let buffer = terminal.backend().buffer();
    let row = (2..20).map(|x| buffer[(x, 2)].symbol()).collect::<String>();
    assert!(row.starts_with(" @/src/main.rs "), "{row}");
    assert_eq!(buffer[(3, 2)].bg, theme::mention().bg.unwrap());

    press(&mut input, KeyCode::Backspace); // o espaço
    press(&mut input, KeyCode::Backspace); // o badge inteiro
    assert_eq!(input.text(), "");
}

#[test]
fn arrows_choose_and_enter_accepts_without_sending() {
    let mut input = input_with_files();
    type_text(&mut input, "@");
    // Sem consulta, os mais rasos primeiro: src, README.md, src/main.rs...
    press(&mut input, KeyCode::Down);
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Handled);
    assert_eq!(input.text(), "@README.md ");
}

#[test]
fn picker_closes_on_esc_space_or_email() {
    let mut input = input_with_files();
    type_text(&mut input, "@ma");
    press(&mut input, KeyCode::Esc);
    assert!(!input.picker_open());
    assert_eq!(input.mode(), InputMode::Insert, "Esc só fecha a lista");
    assert_eq!(input.text(), "@ma");

    type_text(&mut input, " @x y");
    assert!(!input.picker_open(), "espaço encerra a consulta");

    input.clear();
    type_text(&mut input, "eu@exemplo.com");
    assert!(!input.picker_open(), "@ no meio de palavra não abre");
}

#[test]
fn enter_with_no_match_closes_without_sending() {
    let mut input = input_with_files();
    type_text(&mut input, "@zzz");
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Handled);
    assert!(!input.picker_open());
    assert_eq!(input.text(), "@zzz");
    assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Submit);
}

#[test]
fn picker_is_drawn_above_the_input() {
    let mut input = input_with_files();
    type_text(&mut input, "@src");
    let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 15, 40, HEIGHT);
            input.draw(frame, area, Line::default(), Line::default());
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let row = |y: u16| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    // 3 resultados + bordas, logo acima da caixa (y = 15).
    assert!(row(10).starts_with("╭ Arquivos (3) "), "{}", row(10));
    assert!(row(11).contains("▌ src/"));
    assert!(row(12).contains("src/main.rs"));
    assert!(row(13).contains("src/tui/app.rs"));
    assert!(row(14).starts_with("╰"));
}

#[test]
fn lock_indicators_show_only_when_active() {
    let green = theme::variant(Variant::Success).fg.unwrap();
    // Borda de baixo da caixa: y = 5.
    let border = |input: &InputBox| {
        let terminal = render(input);
        let buffer = terminal.backend().buffer();
        let line = (0..60).map(|x| buffer[(x, 5)].symbol()).collect::<String>();
        let ball = line.chars().position(|c| c == '●');
        let green_ball = ball.is_some_and(|x| buffer[(x as u16, 5)].fg == green);
        (line, green_ball)
    };

    let mut input = InputBox::default();
    let (line, _) = border(&input);
    assert!(!line.contains('●'), "nada ativo, nada aparece: {line}");

    input.set_caps_lock(true);
    let (line, green_ball) = border(&input);
    assert!(line.ends_with(" ● Caps ╯"), "{line}");
    assert!(green_ball);

    press(&mut input, KeyCode::Char('i'));
    press(&mut input, KeyCode::Insert);
    assert!(border(&input).0.ends_with(" ● Caps ● Insert ╯"));

    input.set_caps_lock(false);
    assert!(border(&input).0.ends_with(" ● Insert ╯"));
}
