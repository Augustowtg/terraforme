//! Verificação do ambiente do terraforme: extensões.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::extension::Extension;

/// Algo que o terraforme verifica se está instalado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: &'static str,
    pub url: &'static str,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Encontrado; versão quando for possível descobri-la.
    Installed {
        path: PathBuf,
        version: Option<String>,
    },
    /// Existe um executável com o nome esperado, mas não é a ferramenta certa.
    Conflict {
        path: PathBuf,
        detail: String,
    },
    NotInstalled,
}

impl State {
    pub fn is_installed(&self) -> bool {
        matches!(self, State::Installed { .. })
    }
}

/// Resultado completo de `terraforme status`.
#[derive(Debug, Clone)]
pub struct Status {
    pub version: &'static str,
    pub extensions: Vec<Component>,
}

impl Status {
    pub fn all_installed(&self) -> bool {
        self.extensions.iter().all(|c| c.state.is_installed())
    }
}

pub fn check() -> Status {
    Status {
        version: crate::VERSION,
        extensions: Extension::ALL.iter().map(|e| e.check()).collect(),
    }
}

/// Roda `<bin> --version` e devolve o último termo da primeira linha que
/// parece uma versão (`rtk 0.45.0` → `0.45.0`, `2.1.3 (Claude Code)` → `2.1.3`).
pub(crate) fn version_of(bin: &Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn parse_version(output: &str) -> Option<String> {
    let words: Vec<_> = output
        .lines()
        .next()?
        .split_whitespace()
        .map(|v| v.trim_start_matches('v'))
        .collect();
    words
        .iter()
        .rev()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))
        .or(words.last())
        .map(|v| v.to_string())
}

pub(crate) fn run_ok(bin: &Path, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Nomes que `bin` pode ter no disco. No Windows, um para cada extensão do
/// `PATHEXT`: CLIs instaladas pelo npm (o OpenCode, por exemplo) são `.cmd`.
fn exe_names(bin: &str) -> Vec<String> {
    if !cfg!(windows) {
        return vec![bin.to_string()];
    }
    let exts = env::var("PATHEXT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    exe_names_with(bin, &exts)
}

fn exe_names_with(bin: &str, pathext: &str) -> Vec<String> {
    pathext
        .split(';')
        .map(str::trim)
        .filter(|ext| ext.starts_with('.') && ext.len() > 1)
        .map(|ext| format!("{bin}{}", ext.to_ascii_lowercase()))
        .collect()
}

/// Caminho completo de `bin` no `PATH`. Rode sempre o caminho devolvido: o
/// `Command` do Windows só completa `.exe`, então `opencode` sozinho não
/// acharia o `opencode.cmd`.
pub(crate) fn find_in_path(bin: &str) -> Option<PathBuf> {
    let names = exe_names(bin);
    env::split_paths(&env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|p| p.is_file())
}

/// `Command` para `bin`, pelo caminho encontrado no `PATH` (ou o nome, para o
/// erro de "não encontrado" sair do próprio `spawn`).
pub(crate) fn command(bin: &str) -> Command {
    Command::new(find_in_path(bin).unwrap_or_else(|| PathBuf::from(bin)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_takes_last_word_of_first_line() {
        assert_eq!(parse_version("rtk 0.45.0\n"), Some("0.45.0".into()));
        assert_eq!(parse_version("ai-memory v3.1.0"), Some("3.1.0".into()));
        assert_eq!(
            parse_version("2.1.302 (Claude Code)"),
            Some("2.1.302".into())
        );
        assert_eq!(parse_version("opencode v2.0.32"), Some("2.0.32".into()));
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn pathext_gives_one_name_per_extension() {
        assert_eq!(
            exe_names_with("opencode", ".COM;.EXE;;.CMD; .Bat "),
            [
                "opencode.com",
                "opencode.exe",
                "opencode.cmd",
                "opencode.bat"
            ]
        );
        assert!(exe_names_with("opencode", "").is_empty());
    }

    #[test]
    fn missing_binary_is_not_found() {
        assert_eq!(find_in_path("terraforme-binario-inexistente"), None);
    }

    #[test]
    fn check_reports_all_components() {
        let status = check();
        assert_eq!(status.version, crate::VERSION);
        let names: Vec<_> = status.extensions.iter().map(|c| c.name).collect();
        assert_eq!(names, ["RTK", "AI Memory", "AI Jail"]);
    }
}
