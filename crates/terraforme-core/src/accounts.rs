//! Contas nomeadas do Claude Code: nome → `CLAUDE_CONFIG_DIR`, em
//! `~/.config/terraforme/claude.toml`.
//!
//! Cada conta é uma pasta de configuração do Claude Code distinta — com suas
//! próprias credenciais, sessões e configurações. Trocar de conta numa sessão
//! equivale a definir `CLAUDE_CONFIG_DIR` para a pasta da conta escolhida.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Registro de contas do Claude Code: nome → pasta de configuração
/// (`CLAUDE_CONFIG_DIR`), em `~/.config/terraforme/claude.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeAccounts {
    /// Nome → pasta, em ordem alfabética.
    accounts: BTreeMap<String, PathBuf>,
}

impl ClaudeAccounts {
    /// Lê as contas; sem arquivo, devolve a lista vazia.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lendo {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(Config::dir()?)?;
        fs::write(&path, toml::to_string(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    pub fn path() -> Result<PathBuf> {
        Ok(Config::dir()?.join("claude.toml"))
    }

    /// Adiciona ou substitui a conta. Devolve a pasta anterior, se havia.
    pub fn insert(&mut self, name: &str, dir: &Path) -> Result<Option<PathBuf>> {
        let name = validate_name(name)?;
        Ok(self.accounts.insert(name.to_string(), dir.to_path_buf()))
    }

    pub fn remove(&mut self, name: &str) -> Option<PathBuf> {
        self.accounts.remove(name.trim())
    }

    pub fn get(&self, name: &str) -> Option<&Path> {
        self.accounts.get(name.trim()).map(PathBuf::as_path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.accounts.iter().map(|(n, p)| (n.as_str(), p.as_path()))
    }

    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        bail!("o nome da conta não pode ser vazio");
    }
    if name.contains('=') {
        bail!("o nome da conta não pode conter '='");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip_keeps_accounts_sorted() {
        let mut a = ClaudeAccounts::default();
        a.insert("work", Path::new("/home/u/.claude-work")).unwrap();
        a.insert("personal", Path::new("/home/u/.claude")).unwrap();
        let text = toml::to_string(&a).unwrap();
        assert!(text.contains("[accounts]"), "{text}");
        assert!(
            text.find("personal").unwrap() < text.find("work").unwrap(),
            "deve estar em ordem alfabética: {text}"
        );
        assert_eq!(toml::from_str::<ClaudeAccounts>(&text).unwrap(), a);
        assert!(toml::from_str::<ClaudeAccounts>("").unwrap().is_empty());
    }

    #[test]
    fn insert_validates_name() {
        let mut a = ClaudeAccounts::default();
        assert!(a.insert("", Path::new("/x")).is_err());
        assert!(a.insert("  ", Path::new("/x")).is_err());
        assert!(a.insert("bad=name", Path::new("/x")).is_err());
        a.insert("ok", Path::new("/x")).unwrap();
        assert_eq!(a.get("ok"), Some(Path::new("/x")));
        assert_eq!(a.remove("ok"), Some(PathBuf::from("/x")));
        assert!(a.is_empty());
    }
}
