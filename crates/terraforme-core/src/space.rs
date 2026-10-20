//! Spaces: pastas de trabalho salvas com um nome, em
//! `~/.config/terraforme/spaces.toml`, para voltar a elas depois.
//!
//! Cada space pode ter preferências ([`Preferences`]: agente, modo, sandbox)
//! em `<pasta do space>/.terraforme/preferences.toml`; o `terraforme cli`
//! aberto nela (ou numa subpasta) usa essas preferências no lugar do
//! `config.toml`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::agent::{Mode, Provider};
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

    /// Renomeia o space: remove `old`, insere com `new` e a mesma pasta.
    /// Devolve `None` se `old` não existia.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<Option<PathBuf>> {
        let new = validate_name(new)?;
        match self.spaces.remove(old.trim()) {
            None => Ok(None),
            Some(dir) => {
                self.spaces.insert(new.to_string(), dir.clone());
                Ok(Some(dir))
            }
        }
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

    /// Raiz do space salvo mais específico que contém `start` (`None` fora
    /// de space): um de dentro vence um de fora, como a pasta mais próxima
    /// com `preferences.toml`.
    pub fn containing(&self, start: &Path) -> Option<&Path> {
        self.spaces
            .values()
            .filter(|dir| start.starts_with(dir))
            .max_by_key(|dir| dir.components().count())
            .map(PathBuf::as_path)
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

/// Pasta do terraforme dentro da pasta do space.
pub const DIR: &str = ".terraforme";
const PREFERENCES_FILE: &str = "preferences.toml";

/// Preferências do space: o que não estiver definido vem do `config.toml`
/// (e, na sandbox, do `.ai-jail` da pasta).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Agente aberto pelo `terraforme cli` no space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Provider>,
    /// Modo usado sem --eco/--high/--hybrid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// Roda o agente no AI Jail sem --jail/--no-jail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jail: Option<bool>,
    /// Conta do Claude Code (nome registrado em `terraforme config --account`);
    /// define `CLAUDE_CONFIG_DIR` ao abrir o agente neste space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl Preferences {
    /// Lê as preferências da pasta do space; sem arquivo, devolve as vazias.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lendo {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
        }
    }

    /// Grava as preferências; vazias, apaga o arquivo.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = Self::path(dir);
        if self.is_empty() {
            return match fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    Err(err).with_context(|| format!("removendo {}", path.display()))
                }
                _ => Ok(()),
            };
        }
        fs::create_dir_all(dir.join(DIR))?;
        fs::write(&path, toml::to_string(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(DIR).join(PREFERENCES_FILE)
    }

    /// Preferências da pasta ou da mais próxima acima dela que tenha o
    /// arquivo, com o caminho dele.
    pub fn find(start: &Path) -> Result<Option<(PathBuf, Self)>> {
        for dir in start.ancestors() {
            let path = Self::path(dir);
            if path.is_file() {
                return Ok(Some((path, Self::load(dir)?)));
            }
        }
        Ok(None)
    }

    /// Pasta cujas preferências valem para `start`: a da mais próxima acima
    /// dela com `preferences.toml`, ou a da raiz de space salvo mais
    /// próxima (`None` fora de space — aí o `config.toml` é que vale).
    ///
    /// É o mesmo que o `terraforme cli` usa na abertura (`find`), com o
    /// acréscimo da raiz de space sem preferências ainda: gravar nelas faz
    /// o space passar a ter voz própria.
    pub fn dir_for(start: &Path) -> Result<Option<PathBuf>> {
        if let Some((path, _)) = Self::find(start)?
            && let Some(dir) = path.parent().and_then(Path::parent)
        {
            return Ok(Some(dir.to_path_buf()));
        }
        Ok(Spaces::load()?.containing(start).map(Path::to_path_buf))
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
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

    #[test]
    fn containing_picks_deepest_saved_space() {
        let mut spaces = spaces();
        spaces.insert("deep", Path::new("/w/web/deep")).unwrap();
        assert_eq!(
            spaces.containing(Path::new("/w/web")),
            Some(Path::new("/w/web"))
        );
        assert_eq!(
            spaces.containing(Path::new("/w/web/src/app")),
            Some(Path::new("/w/web"))
        );
        assert_eq!(
            spaces.containing(Path::new("/w/web/deep/x")),
            Some(Path::new("/w/web/deep"))
        );
        assert_eq!(spaces.containing(Path::new("/w")), None);
        assert_eq!(spaces.containing(Path::new("/outro")), None);
        assert_eq!(Spaces::default().containing(Path::new("/w/web")), None);
    }

    #[test]
    fn preferences_round_trip_and_find() {
        let prefs = Preferences {
            agent: Some(Provider::Claude),
            mode: Some(Mode::Eco),
            jail: None,
            account: None,
        };
        let text = toml::to_string(&prefs).unwrap();
        assert_eq!(text, "agent = \"claude\"\nmode = \"eco\"\n");
        assert_eq!(toml::from_str::<Preferences>(&text).unwrap(), prefs);

        let with_account = Preferences {
            account: Some("work".into()),
            ..prefs.clone()
        };
        let text_with_account = toml::to_string(&with_account).unwrap();
        assert!(
            text_with_account.contains("account = \"work\""),
            "{text_with_account}"
        );
        assert_eq!(
            toml::from_str::<Preferences>(&text_with_account).unwrap(),
            with_account
        );
        assert!(toml::from_str::<Preferences>("").unwrap().is_empty());

        let dir = env::temp_dir().join(format!("terraforme-space-{}", std::process::id()));
        let sub = dir.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        assert!(
            Preferences::find(&sub)
                .unwrap()
                .is_none_or(|(p, _)| !p.starts_with(&dir))
        );
        prefs.save(&dir).unwrap();
        let (path, found) = Preferences::find(&sub).unwrap().unwrap();
        assert_eq!(path, dir.join(".terraforme/preferences.toml"));
        assert_eq!(found, prefs);
        Preferences::default().save(&dir).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
