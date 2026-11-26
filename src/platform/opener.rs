//! O Ctrl+clique do histórico: abre um link no navegador e um arquivo no
//! VS Code (`code -g arquivo:linha`) ou, sem ele, no app padrão do sistema.
//! O terra captura o mouse, então o terminal não abre o link sozinho.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Url(String),
    /// Um arquivo ou pasta que existe, com a linha (`arquivo.rs:12`).
    File {
        path: PathBuf,
        line: Option<u32>,
    },
}

/// O que abrir para a palavra `word` (um link, `[texto](url)` do markdown,
/// ou um caminho relativo a `cwd`); `None` se não é link nem arquivo.
pub fn target(word: &str, cwd: &Path) -> Option<Target> {
    // `[texto](url)`: o que vale é a url.
    let word = word.rsplit_once("](").map_or(word, |(_, url)| url);
    let word = word
        .trim_start_matches(['(', '[', '<', '\'', '"', '`'])
        .trim_end_matches(['.', ',', ';', ':', ')', ']', '>', '\'', '"', '`', '!', '?']);
    if word.starts_with("http://") || word.starts_with("https://") {
        return Some(Target::Url(word.to_string()));
    }
    let word = word.strip_prefix("file://").unwrap_or(word);
    // `arquivo:linha` ou `arquivo:linha:coluna`.
    let mut parts = word.splitn(3, ':');
    let file = parts.next()?;
    let line = parts.next().and_then(|n| n.parse().ok());
    let path = cwd.join(file);
    (!file.is_empty() && path.exists()).then_some(Target::File { path, line })
}

/// Abre `target` num processo à parte; `false` se não deu para rodar.
pub fn open(target: &Target) -> bool {
    if cfg!(test) {
        return true;
    }
    match target {
        Target::Url(url) => system_open(url),
        Target::File { path, line } => {
            let mut arg = path.to_string_lossy().into_owned();
            if let Some(line) = line {
                arg = format!("{arg}:{line}");
            }
            spawn(shell("code", &["-g", &arg])) || system_open(&path.to_string_lossy())
        }
    }
}

fn system_open(target: &str) -> bool {
    if cfg!(windows) {
        spawn(shell("start", &["", target]))
    } else {
        spawn({
            let mut c = Command::new("xdg-open");
            c.arg(target);
            c
        })
    }
}

/// No Windows, `code` e `start` só existem pelo `cmd`.
fn shell(bin: &str, args: &[&str]) -> Command {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/c", bin]);
        c
    } else {
        Command::new(bin)
    };
    command.args(args);
    command
}

fn spawn(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_existing_files_are_targets() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let url = |u: &str| Some(Target::Url(u.to_string()));
        assert_eq!(target("https://a.com/x.", cwd), url("https://a.com/x"));
        assert_eq!(
            target("[docs](https://a.com/d)", cwd),
            url("https://a.com/d")
        );
        assert_eq!(
            target("`src/main.rs:12`", cwd),
            Some(Target::File {
                path: cwd.join("src/main.rs"),
                line: Some(12)
            })
        );
        assert_eq!(target("palavra", cwd), None);
    }
}
