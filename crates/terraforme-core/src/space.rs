//! Spaces: pastas de trabalho salvas com um nome, em
//! `~/.config/terraforme/spaces.toml`, para voltar a elas depois.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spaces {
    /// Nome → pasta, em ordem alfabética.
    spaces: BTreeMap<String, PathBuf>,
}

impl Spaces {
    /// Lê os spaces; sem arquivo, devolve a lista vazia.
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
        Ok(Config::dir()?.join("spaces.toml"))
    }

    /// Salva (ou substitui) o space. Devolve a pasta que ele tinha antes.
    pub fn insert(&mut self, name: &str, dir: &Path) -> Result<Option<PathBuf>> {
        let name = validate_name(name)?;
        Ok(self.spaces.insert(name.to_string(), dir.to_path_buf()))
    }

    pub fn remove(&mut self, name: &str) -> Option<PathBuf> {
        self.spaces.remove(name.trim())
    }

    pub fn get(&self, name: &str) -> Option<&Path> {
        self.spaces.get(name.trim()).map(PathBuf::as_path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.spaces.iter().map(|(n, p)| (n.as_str(), p.as_path()))
    }

    pub fn len(&self) -> usize {
        self.spaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spaces.is_empty()
    }

    /// Space escolhido pelo número da lista (a partir de 1) ou pelo nome.
    pub fn pick(&self, choice: &str) -> Option<(&str, &Path)> {
        let choice = choice.trim();
        match choice.parse::<usize>() {
            Ok(n) if n >= 1 => self.iter().nth(n - 1),
            _ => self
                .spaces
                .get_key_value(choice)
                .map(|(n, p)| (n.as_str(), p.as_path())),
        }
    }
}

/// O nome não pode ser vazio, nem só número (que escolhe pela posição na lista).
fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        bail!("o nome do space não pode ser vazio");
    }
    if name.parse::<usize>().is_ok() {
        bail!("o nome do space não pode ser só um número");
    }
    Ok(name)
}

/// Shell do usuário aberto na pasta do space (`$SHELL`; `%COMSPEC%` no Windows).
/// `TERRAFORME_SPACE` fica com o nome dele.
pub fn shell(name: &str, dir: &Path) -> Command {
    let shell = if cfg!(windows) {
        env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
    } else {
        env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into())
    };
    let mut command = Command::new(shell);
    command.current_dir(dir).env("TERRAFORME_SPACE", name);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaces() -> Spaces {
        let mut spaces = Spaces::default();
        spaces.insert("web", Path::new("/w/web")).unwrap();
        spaces.insert(" api ", Path::new("/w/api")).unwrap();
        spaces
    }

    #[test]
    fn toml_round_trip_keeps_names_sorted() {
        let spaces = spaces();
        let text = toml::to_string(&spaces).unwrap();
        assert_eq!(text, "[spaces]\napi = \"/w/api\"\nweb = \"/w/web\"\n");
        assert_eq!(toml::from_str::<Spaces>(&text).unwrap(), spaces);
        assert!(toml::from_str::<Spaces>("").unwrap().is_empty());
    }

    #[test]
    fn pick_by_number_or_name() {
        let spaces = spaces();
        assert_eq!(spaces.pick("1"), Some(("api", Path::new("/w/api"))));
        assert_eq!(spaces.pick(" web "), Some(("web", Path::new("/w/web"))));
        assert_eq!(spaces.pick("0"), None);
        assert_eq!(spaces.pick("3"), None);
        assert_eq!(spaces.pick("outro"), None);
    }

    #[test]
    fn insert_validates_and_replaces() {
        let mut spaces = spaces();
        assert!(spaces.insert("  ", Path::new("/x")).is_err());
        assert!(spaces.insert("42", Path::new("/x")).is_err());
        let old = spaces.insert("web", Path::new("/novo")).unwrap();
        assert_eq!(old, Some(PathBuf::from("/w/web")));
        assert_eq!(spaces.get("web"), Some(Path::new("/novo")));
        assert_eq!(spaces.remove("web"), Some(PathBuf::from("/novo")));
        assert_eq!(spaces.len(), 1);
    }
}
