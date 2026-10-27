//! Interfaces ratatui: tela principal com abas (`terra cli`) e navegador de
//! workspaces (`terra workspace`).

mod app;
mod chat;
mod config;
mod usage;
mod widgets;
mod workspace;

use std::io::stdout;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::DefaultTerminal;

use crate::db::Db;

pub use workspace::run_workspace;

/// Intervalo das animações da tela principal (spinner e cursor piscante).
const TICK: Duration = Duration::from_millis(100);

/// Tela principal com a barra de abas; sai pelo botão "Sair" (`Esc`, `Enter` e
/// confirmação) ou direto com `Ctrl+C`.
pub fn run_app(db: Db) -> Result<()> {
    let mut app = app::App::new(db);
    with_terminal(|terminal| {
        // Mouse para selecionar o texto do input; cursor em barra (`|`).
        execute!(stdout(), EnableMouseCapture, SetCursorStyle::SteadyBar)?;
        // Nos terminais com o protocolo de teclado do kitty, o Shift+Enter chega
        // diferente do Enter; nos outros, Alt+Enter e Ctrl+J pulam linha.
        let enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            execute!(
                stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        let result = app_loop(terminal, &mut app);
        if enhanced {
            execute!(stdout(), PopKeyboardEnhancementFlags)?;
        }
        execute!(
            stdout(),
            DisableMouseCapture,
            SetCursorStyle::DefaultUserShape
        )?;
        result
    })
}

fn app_loop(terminal: &mut DefaultTerminal, app: &mut app::App) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(TICK.saturating_sub(last_tick.elapsed()))? {
            match event::read()? {
                // No Windows também chegam eventos de Release; só interessa o Press.
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if is_ctrl_c(&key) || app.handle_key(key) {
                        return Ok(());
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
