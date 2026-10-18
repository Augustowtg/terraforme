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

fn exe_name(bin: &str) -> String {
    format!("{bin}{}", env::consts::EXE_SUFFIX)
}

pub(crate) fn find_in_path(bin: &str) -> Option<PathBuf> {
    let name = exe_name(bin);
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
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
