//! Um terminal dentro da tela do terra: a interface original do provedor
//! (ex.: o `claude` interativo) roda num pseudo-terminal e a tela dele é
//! desenhada na área de conteúdo, com a barra de abas e o rodapé do terra.
//!
//! A saída do processo é lida numa thread e interpretada pelo [`vt100`]; as
//! teclas viram as sequências que um terminal mandaria ([`key_bytes`]).

use std::io::{Read, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

pub struct Term {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    size: (u16, u16),
    /// O cliente já enviou uma mensagem (um `Enter`): há uma sessão.
    sent: bool,
}

impl Term {
    /// Roda `command` num pseudo-terminal de `rows` x `cols`.
    pub fn spawn(command: &Command, rows: u16, cols: u16) -> Result<Self, String> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .map_err(|e| format!("Não foi possível abrir o terminal: {e}"))?;
        let child = pair
            .slave
            .spawn_command(builder(command))
            .map_err(|e| format!("Não foi possível abrir a interface do provedor: {e}"))?;
        // O processo já tem o lado dele; sem soltar, a leitura nunca termina.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            nonzero(rows),
            nonzero(cols),
            0,
        )));
        let output = Arc::clone(&parser);
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut parser) = output.lock() {
                    parser.process(&buf[..n]);
                }
            }
        });
        Ok(Self {
            master: pair.master,
            writer,
            child,
            parser,
            size: (rows, cols),
            sent: false,
        })
    }

    /// Acompanha o tamanho da área onde o terminal é desenhado.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(pty_size(rows, cols));
        if let Ok(mut parser) = self.parser.lock() {
            parser.set_size(nonzero(rows), nonzero(cols));
        }
    }

    /// Se o cliente já enviou alguma mensagem: aí a sessão não deve ser
    /// fechada sem ele pedir.
    pub fn has_session(&self) -> bool {
        self.sent
    }

    pub fn send_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            self.sent = true;
        }
        let app_cursor = self
            .parser
            .lock()
            .is_ok_and(|p| p.screen().application_cursor());
        if let Some(bytes) = key_bytes(key, app_cursor) {
            let _ = self.writer.write_all(&bytes);
            let _ = self.writer.flush();
        }
    }

    /// Cola `text` no processo, entre as marcas do bracketed paste quando ele
    /// pediu esse modo (o `claude` pede), para as quebras de linha não
    /// enviarem a mensagem.
    pub fn send_paste(&mut self, text: &str) {
        let bracketed = self
            .parser
            .lock()
            .is_ok_and(|p| p.screen().bracketed_paste());
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        let _ = self.writer.write_all(bytes.as_bytes());
        let _ = self.writer.flush();
    }

    /// `Some` quando o processo terminou: `Ok` se saiu com sucesso.
    pub fn finished(&mut self) -> Option<Result<(), String>> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) if status.success() => Some(Ok(())),
            Ok(Some(status)) => Some(Err(format!(
                "A interface do provedor terminou com o código {}.",
                status.exit_code()
            ))),
            Err(err) => Some(Err(err.to_string())),
        }
    }

    /// Desenha a tela do processo em `area`; com `focused`, o cursor também.
    pub fn draw(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let Ok(parser) = self.parser.lock() else {
            return;
        };
        let screen = parser.screen();
        let buffer = frame.buffer_mut();
        for row in 0..area.height {
            for col in 0..area.width {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let target = &mut buffer[(area.x + col, area.y + row)];
                let symbol = cell.contents();
                target.set_symbol(if symbol.is_empty() { " " } else { symbol });
                target.set_style(cell_style(cell));
            }
        }
        if focused && !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            if row < area.height && col < area.width {
                frame.set_cursor_position(Position::new(area.x + col, area.y + row));
            }
        }
    }
}

impl Drop for Term {
    /// Sair do terra (ou trocar a Interface) encerra o processo.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn nonzero(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN)
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// O `Command` do adapter para o pseudo-terminal: programa, argumentos,
/// variáveis e diretório.
fn builder(command: &Command) -> CommandBuilder {
    let mut builder = CommandBuilder::new(command.get_program());
    builder.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => builder.env(key, value),
            None => builder.env_remove(key),
        }
    }
    if let Some(dir) = command.get_current_dir() {
        builder.cwd(dir);
    }
    builder.env("TERM", "xterm-256color");
    builder
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::new()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()));
    for (on, modifier) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        if on {
            style = style.add_modifier(modifier);
        }
    }
    style
}

/// A sequência que um terminal (xterm) manda para a tecla. Com
/// `app_cursor`, as setas vão no modo de aplicação (`ESC O A`).
pub fn key_bytes(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let arrow = |c: char| {
        let prefix = if app_cursor { "\x1bO" } else { "\x1b[" };
        format!("{prefix}{c}").into_bytes()
    };
    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => {
            vec![c.to_ascii_lowercase() as u8 - b'a' + 1]
        }
        KeyCode::Char(' ') if ctrl => vec![0],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift+Enter (no protocolo do kitty) pula linha, como o Alt+Enter.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => arrow('A'),
        KeyCode::Down => arrow('B'),
        KeyCode::Right => arrow('C'),
        KeyCode::Left => arrow('D'),
        KeyCode::Home => arrow('H'),
        KeyCode::End => arrow('F'),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        _ => return None,
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        key_bytes(KeyEvent::new(code, modifiers), false).unwrap()
    }

    #[test]
    fn keys_become_terminal_sequences() {
        let none = KeyModifiers::NONE;
        assert_eq!(bytes(KeyCode::Char('a'), none), b"a");
        assert_eq!(bytes(KeyCode::Char('c'), KeyModifiers::CONTROL), [3]);
        assert_eq!(bytes(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        assert_eq!(bytes(KeyCode::Enter, none), b"\r");
        assert_eq!(bytes(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
        assert_eq!(bytes(KeyCode::Up, none), b"\x1b[A");
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, none), true).unwrap(),
            b"\x1bOA"
        );
        assert!(key_bytes(KeyEvent::new(KeyCode::F(1), none), false).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn enter_starts_a_session() {
        let mut term = Term::spawn(&Command::new("cat"), 3, 20).unwrap();
        assert!(!term.has_session());
        term.send_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        term.send_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(!term.has_session(), "Shift+Enter só pula linha");
        term.send_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(term.has_session());
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_command_and_draws_its_output() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut command = Command::new("printf");
        command.arg("oi terra");
        let mut term = Term::spawn(&command, 3, 20).unwrap();
        let start = std::time::Instant::now();
        while term.finished().is_none() && start.elapsed().as_secs() < 5 {
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(term.finished(), Some(Ok(())));
        thread::sleep(std::time::Duration::from_millis(100));
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| term.draw(frame, frame.area(), false))
            .unwrap();
        let row: String = (0..20)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert_eq!(row.trim_end(), "oi terra");
    }
}
