//! Copia texto para a área de transferência do sistema e lê dela.
//!
//! No Linux usa `wl-copy` (Wayland), `xclip` ou `xsel`. Sem nenhum deles, e
//! no macOS e no Windows, usa a área de transferência do sistema direto (o
//! `arboard`; no Linux, pelo X11, que no Wayland é o XWayland). Se nem isso
//! der, manda a sequência OSC 52, que o próprio terminal transforma em cópia
//! (Windows Terminal, kitty, WezTerm, iTerm2...); o GNOME Terminal e o
//! Ptyxis a ignoram.
//!
//! Para colar ([`paste`]): `wl-paste`, `xclip` ou `xsel` no Linux, `pbpaste`
//! no macOS e o `Get-Clipboard` do PowerShell no Windows; sem eles, o
//! `arboard`.

use std::env;
use std::io::{Write, stdout};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;

/// A área de transferência do `arboard`, aberta na primeira vez e mantida
/// até o terra sair: no X11 o texto copiado só fica disponível enquanto
/// quem copiou está vivo para entregá-lo.
fn system() -> &'static Mutex<Option<arboard::Clipboard>> {
    static CLIPBOARD: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()))
}

fn system_copy(text: &str) -> bool {
    let mut clipboard = system().lock().unwrap_or_else(|e| e.into_inner());
    clipboard
        .as_mut()
        .is_some_and(|c| c.set_text(text.to_string()).is_ok())
}

fn system_paste() -> Option<String> {
    let mut clipboard = system().lock().unwrap_or_else(|e| e.into_inner());
    clipboard
        .as_mut()?
        .get_text()
        .ok()
        .filter(|text| !text.is_empty())
}

/// Copia `text`. Retorna `false` se não houve como tentar.
pub fn copy(text: &str) -> bool {
    // Nos testes, não mexe na área de transferência de quem roda.
    if cfg!(test) {
        return true;
    }
    if let Some((bin, args)) = native_tool()
        && spawn_copy(bin, args, text)
    {
        return true;
    }
    system_copy(text) || osc52(text)
}

fn native_tool() -> Option<(&'static str, &'static [&'static str])> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    let tools: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    tools
        .into_iter()
        .filter(|(bin, _)| wayland || *bin != "wl-copy")
        .find(|(bin, _)| in_path(bin))
}

/// O texto da área de transferência; `None` se ela está vazia ou não há
/// como ler.
pub fn paste() -> Option<String> {
    // Nos testes, não lê a área de transferência de quem roda.
    if cfg!(test) {
        return None;
    }
    paste_tool()
        .and_then(|(bin, args)| tool_paste(bin, args))
        .or_else(system_paste)
}

fn tool_paste(bin: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    // O PowerShell põe uma quebra de linha no fim da saída.
    if cfg!(windows) && text.ends_with("\r\n") {
        text.truncate(text.len() - 2);
    }
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn paste_tool() -> Option<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        return Some(("pbpaste", &[]));
    }
    if cfg!(windows) {
        return Some((
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Clipboard -Raw",
            ],
        ));
    }
    let wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    let tools: [(&str, &[&str]); 3] = [
        ("wl-paste", &["--no-newline"]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
    tools
        .into_iter()
        .filter(|(bin, _)| wayland || *bin != "wl-paste")
        .find(|(bin, _)| in_path(bin))
}

fn in_path(bin: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
}

fn spawn_copy(bin: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let mut stdin = child.stdin.take().expect("stdin configurado como pipe");
    let text = text.to_string();
    // Escreve e espera em outra thread para não travar a tela.
    thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
        drop(stdin);
        let _ = child.wait();
    });
    true
}

fn osc52(text: &str) -> bool {
    let mut out = stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).is_ok() && out.flush().is_ok()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encodes_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("olá".as_bytes()), "b2zDoQ==");
    }
}
