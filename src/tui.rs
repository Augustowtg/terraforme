//! Interfaces ratatui: tela principal com abas (`terra cli`) e navegador de
//! workspaces (`terra workspace`).

mod app;
mod chat;
mod config;
mod login;
mod term;
mod usage;
mod widgets;
mod workspace;

use std::io::stdout;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
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

/// Tela principal com a barra de abas; sai pelo botão "Sair" (`Esc`, `Enter` e
/// confirmação) ou direto com `Ctrl+C`.
pub fn run_app(db: Db, dir: PathBuf, config: PathBuf, warnings: Vec<String>) -> Result<()> {
    let mut app = app::App::new(db, dir, config);
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

/// Mouse para selecionar o texto do input, cursor em barra (`|`) e, quando
/// o terminal tem, o protocolo de teclado do kitty.
fn enter_app_mode(enhanced: bool) -> Result<()> {
    execute!(stdout(), EnableMouseCapture, SetCursorStyle::SteadyBar)?;
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
    loop {
        let size = terminal.size()?;
        app.sync_term(ratatui::layout::Rect::new(0, 0, size.width, size.height));
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(TICK.saturating_sub(last_tick.elapsed()))? {
            match event::read()? {
                // No Windows também chegam eventos de Release; só interessa o Press.
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if (is_ctrl_c(&key) && !app.captures_keys()) || app.handle_key(key) {
                        return Ok(());
                    }
                    if let Some((login, account)) = app.take_login() {
                        let result = run_login(terminal, login, &account, enhanced)?;
                        app.login_finished(result);
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
