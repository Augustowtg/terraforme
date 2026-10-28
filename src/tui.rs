//! Interfaces ratatui: tela principal com abas (`terra cli`) e navegador de
//! workspaces (`terra workspace`).

mod app;
mod chat;
mod config;
mod extensions;
mod login;
mod mcp;
mod term;
mod usage;
mod widgets;
mod workspace;

use std::collections::VecDeque;
use std::io::stdout;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, supports_keyboard_enhancement,
};
use ratatui::backend::CrosstermBackend;
use ratatui::{DefaultTerminal, Terminal};

use crate::db::Db;

pub use workspace::run_workspace;

/// Intervalo das animações da tela principal (spinner e cursor piscante).
const TICK: Duration = Duration::from_millis(100);

pub use app::ConfigFiles;

/// Tela principal com a barra de abas do workspace `name`, em `dir`; sai
/// pelo botão "Sair" (`Esc`, `Enter` e confirmação) ou direto com `Ctrl+C`.
pub fn run_app(
    db: Db,
    dir: PathBuf,
    name: String,
    files: ConfigFiles,
    warnings: Vec<String>,
) -> Result<()> {
    let mut app = app::App::new(db, dir, name, files);
    for warning in warnings {
        app.warn(warning);
    }
    with_terminal(|terminal| {
        // Por cima do hook do ratatui: um panic do provedor não fecha a tela.
        crate::agent::guard::install_hook();
        // Nos terminais com o protocolo de teclado do kitty, o Shift+Enter chega
        // diferente do Enter; nos outros, Alt+Enter e Ctrl+J pulam linha.
        let enhanced = supports_keyboard_enhancement().unwrap_or(false);
        enter_app_mode(enhanced)?;
        let result = app_loop(terminal, &mut app, enhanced);
        leave_app_mode(enhanced)?;
        result
    })
}

/// Mouse para selecionar o texto do input, cursor em barra (`|`), o texto
/// colado num evento só (bracketed paste) e, quando o terminal tem, o
/// protocolo de teclado do kitty.
fn enter_app_mode(enhanced: bool) -> Result<()> {
    execute!(stdout(), EnableMouseCapture, SetCursorStyle::SteadyBar)?;
    // O console do Windows não tem; lá a colagem chega como teclas
    // ([`paste_burst`]).
    let _ = execute!(stdout(), EnableBracketedPaste);
    if enhanced {
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    Ok(())
}

fn leave_app_mode(enhanced: bool) -> Result<()> {
    if enhanced {
        execute!(stdout(), PopKeyboardEnhancementFlags)?;
    }
    let _ = execute!(stdout(), DisableBracketedPaste);
    execute!(
        stdout(),
        DisableMouseCapture,
        SetCursorStyle::DefaultUserShape
    )?;
    Ok(())
}

/// Roda o login do `/login` no terminal normal: o terra sai da tela, mostra
/// o link e lê o código (veja [`login`]) e a tela volta no fim.
fn run_login(
    terminal: &mut DefaultTerminal,
    login: Command,
    account: &str,
    enhanced: bool,
) -> Result<std::result::Result<(), String>> {
    outside_screen(terminal, enhanced, || {
        login::run(login, account, &mut std::io::stdin().lock(), &mut stdout())
    })
}

/// Roda a interface original do provedor no terminal normal até ela sair.
fn run_original(mut original: std::process::Command) -> std::result::Result<(), String> {
    original
        .status()
        .map_err(|err| format!("não foi possível iniciar: {err}"))
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("saiu com {status}"))
        })
}

/// Sai da tela do terra, roda `f` no terminal normal e volta.
fn outside_screen<T>(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    f: impl FnOnce() -> T,
) -> Result<T> {
    leave_app_mode(enhanced)?;
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    let result = f();
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    enter_app_mode(enhanced)?;
    // Um terminal novo desenha a tela inteira de novo. O `clear()` do
    // ratatui pergunta a posição do cursor, o que pode falhar aqui.
    *terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    Ok(result)
}

fn app_loop(terminal: &mut DefaultTerminal, app: &mut app::App, enhanced: bool) -> Result<()> {
    let mut last_tick = Instant::now();
    // Eventos já lidos por [`paste_burst`] que não eram uma colagem.
    let mut pending = VecDeque::new();
    loop {
        let size = terminal.size()?;
        app.sync_term(ratatui::layout::Rect::new(0, 0, size.width, size.height));
        terminal.draw(|frame| app.draw(frame))?;
        let next = match pending.pop_front() {
            Some(event) => Some(event),
            None if event::poll(TICK.saturating_sub(last_tick.elapsed()))? => {
                let event = event::read()?;
                match &event {
                    Event::Key(key) if !app.captures_keys() && is_text_key(key) => {
                        match paste_burst(*key, &mut pending)? {
                            Some(text) => Some(Event::Paste(text)),
                            None => pending.pop_front(),
                        }
                    }
                    _ => Some(event),
                }
            }
            None => None,
        };
        if let Some(event) = next {
            match event {
                Event::Paste(text) => app.handle_paste(&text),
                // No Windows também chegam eventos de Release; só interessa o Press.
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if (is_ctrl_c(&key) && !app.captures_keys()) || app.handle_key(key) {
                        return Ok(());
                    }
                    if let Some((login, account)) = app.take_login() {
                        let result = run_login(terminal, login, &account, enhanced)?;
                        app.login_finished(result);
                    }
                    if let Some((original, command)) = app.take_original() {
                        let result = outside_screen(terminal, enhanced, || run_original(original))?;
                        app.original_finished(&command, result);
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
        if last_tick.elapsed() >= TICK {
            app.tick();
            last_tick = Instant::now();
        }
    }
}

/// Uma tecla que um texto colado gera: caractere, Enter ou Tab, sem Ctrl
/// nem Alt.
fn is_text_key(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab)
}

/// Nos terminais sem bracketed paste (ex.: o console do Windows), o texto
/// colado chega como teclas, uma atrás da outra, e cada Enter enviaria a
/// mensagem. Com `first` e as teclas que já estão na fila, junta a rajada: se
/// ela tem uma quebra de linha ou passa de 100 caracteres, é uma colagem.
/// Senão, as teclas (e o evento que interrompeu a rajada) vão para `pending`,
/// para serem tratadas uma a uma.
fn paste_burst(first: KeyEvent, pending: &mut VecDeque<Event>) -> Result<Option<String>> {
    let mut keys = vec![first];
    while event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Key(key) if is_text_key(&key) => keys.push(key),
            // As de Release (Windows) não contam.
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            other => {
                pending.extend(keys.drain(..).map(Event::Key));
                pending.push_back(other);
                return Ok(None);
            }
        }
    }
    let text: String = keys
        .iter()
        .map(|key| match key.code {
            KeyCode::Char(c) => c,
            KeyCode::Tab => '\t',
            _ => '\n',
        })
        .collect();
    if keys.len() > 1 && (text.contains('\n') || text.chars().count() > chat::PASTE_LIMIT) {
        return Ok(Some(text));
    }
    pending.extend(keys.into_iter().map(Event::Key));
    Ok(None)
}

fn with_terminal<T>(f: impl FnOnce(&mut DefaultTerminal) -> Result<T>) -> Result<T> {
    let mut terminal = ratatui::init();
    let result = f(&mut terminal);
    ratatui::restore();
    result
}

/// Em raw mode o Ctrl+C não gera SIGINT; tratamos como saída em todas as telas.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}
