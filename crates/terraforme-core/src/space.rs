//! Spaces: pastas de trabalho salvas com um nome, em
//! `~/.config/terraforme/spaces.toml`, para voltar a elas depois.
//!
//! Cada space pode ter preferências ([`Preferences`]: agente, modo, sandbox,
//! conta) em `~/.terraforme/<space>/preferences.toml`, fora da pasta do
//! projeto; o `terraforme cli` aberto na pasta do space (ou numa subpasta)
//! usa essas preferências no lugar do `config.toml`.

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
    /// Lê os spaces; sem arquivo, devolve a lista vazia. Preferências ainda
    /// na pasta do space (versões antigas) são movidas para [`root`].
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        let spaces: Self = match fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("lendo {}", path.display()))?
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("lendo {}", path.display())),
        };
        let root = root()?;
        for (name, dir) in spaces.iter() {
            migrate(&root, name, dir)?;
        }
        Ok(spaces)
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

    /// Nome do space salvo mais específico que contém `start` (`None` fora
    /// de space): um de dentro vence um de fora.
    pub fn name_containing(&self, start: &Path) -> Option<&str> {
        self.iter()
            .filter(|(_, dir)| start.starts_with(dir))
            .max_by_key(|(_, dir)| dir.components().count())
            .map(|(name, _)| name)
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

/// Raiz das preferências dos spaces: `~/.terraforme` (`TERRAFORME_HOME`
/// troca). Cada space tem uma pasta com o nome dele.
pub fn root() -> Result<PathBuf> {
    match env::var_os("TERRAFORME_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(crate::agent::home()?.join(".terraforme")),
    }
}

/// Pasta onde as preferências ficavam antes, dentro da pasta do space;
/// [`Spaces::load`] as move para [`root`].
const LEGACY_DIR: &str = ".terraforme";
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
    /// Lê as preferências do space; sem arquivo, devolve as vazias.
    pub fn load(name: &str) -> Result<Self> {
        Self::load_from(&Self::path(name)?)
    }

    fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lendo {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
        }
    }

    /// Grava as preferências; vazias, apaga o arquivo.
    pub fn save(&self, name: &str) -> Result<()> {
        self.save_to(&Self::path(name)?)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        if self.is_empty() {
            return remove_file(path);
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, toml::to_string(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    /// `~/.terraforme/<space>/preferences.toml`.
    pub fn path(name: &str) -> Result<PathBuf> {
        Ok(Self::path_in(&root()?, name))
    }

    fn path_in(root: &Path, name: &str) -> PathBuf {
        root.join(name.trim()).join(PREFERENCES_FILE)
    }

    /// Space salvo que contém `start` (o mais específico), com as
    /// preferências dele; `None` fora de space — aí o `config.toml` é que vale.
    pub fn find(start: &Path) -> Result<Option<(String, Self)>> {
        let spaces = Spaces::load()?;
        let Some(name) = spaces.name_containing(start) else {
            return Ok(None);
        };
        Ok(Some((name.to_string(), Self::load(name)?)))
    }

    /// Leva as preferências do space `old` para `new` (renomeação).
    pub fn rename(old: &str, new: &str) -> Result<()> {
        let prefs = Self::load(old)?;
        if !prefs.is_empty() {
            prefs.save(new)?;
        }
        Self::default().save(old)
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Apaga o arquivo (ausente não é erro) e a pasta dele, se ficou vazia.
fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            return Err(err).with_context(|| format!("removendo {}", path.display()));
        }
        _ => {}
    }
    if let Some(dir) = path.parent() {
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

/// Move as preferências que ficavam em `<pasta do space>/.terraforme/` para
/// `root/<space>/` (só se ainda não houver arquivo lá).
fn migrate(root: &Path, name: &str, dir: &Path) -> Result<()> {
    let old = dir.join(LEGACY_DIR).join(PREFERENCES_FILE);
    let new = Preferences::path_in(root, name);
    if !old.is_file() || new.exists() {
        return Ok(());
    }
    Preferences::load_from(&old)?.save_to(&new)?;
    remove_file(&old)
}

/// O nome não pode ser vazio, nem só número (que escolhe pela posição na
/// lista), nem ter separador de pasta (vira pasta em [`root`]).
fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        bail!("o nome do space não pode ser vazio");
    }
    if name == "." || name == ".." || name.contains(['/', '\\']) {
        bail!("o nome do space não pode ter / nem \\ nem ser . ou ..");
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
    fn name_containing_picks_deepest_saved_space() {
        let mut spaces = spaces();
        spaces.insert("deep", Path::new("/w/web/deep")).unwrap();
        assert_eq!(spaces.name_containing(Path::new("/w/web")), Some("web"));
        assert_eq!(
            spaces.name_containing(Path::new("/w/web/src/app")),
            Some("web")
        );
        assert_eq!(
            spaces.name_containing(Path::new("/w/web/deep/x")),
            Some("deep")
        );
        assert_eq!(spaces.name_containing(Path::new("/w")), None);
        assert_eq!(spaces.name_containing(Path::new("/outro")), None);
        assert_eq!(Spaces::default().name_containing(Path::new("/w/web")), None);
    }

    #[test]
    fn names_cannot_be_paths() {
        let mut spaces = Spaces::default();
        for bad in ["a/b", "a\\b", ".", ".."] {
            assert!(spaces.insert(bad, Path::new("/x")).is_err(), "{bad}");
        }
        assert!(spaces.insert("Claude Skills", Path::new("/x")).is_ok());
    }

    #[test]
    fn preferences_round_trip_save_and_migrate() {
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

        let tmp = env::temp_dir().join(format!("terraforme-space-{}", std::process::id()));
        let root = tmp.join("home");
        let path = Preferences::path_in(&root, " web ");
        assert_eq!(path, root.join("web/preferences.toml"));
        prefs.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap(), prefs);
        Preferences::default().save_to(&path).unwrap();
        assert!(!path.exists() && !path.parent().unwrap().exists());

        // O arquivo antigo, na pasta do space, vai para a raiz.
        let project = tmp.join("project");
        let legacy = project.join(".terraforme/preferences.toml");
        prefs.save_to(&legacy).unwrap();
        migrate(&root, "web", &project).unwrap();
        assert!(!legacy.exists() && !legacy.parent().unwrap().exists());
        assert_eq!(Preferences::load_from(&path).unwrap(), prefs);
        // Com arquivo novo, o antigo fica onde está.
        with_account.save_to(&legacy).unwrap();
        migrate(&root, "web", &project).unwrap();
        assert!(legacy.exists());
        assert_eq!(Preferences::load_from(&path).unwrap(), prefs);
        fs::remove_dir_all(&tmp).unwrap();
    }
}
