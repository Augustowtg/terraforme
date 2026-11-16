//! [Ponytail](https://github.com/dietrichgebert/ponytail): skill que
//! incentiva o agente a escrever menos código, com comandos de revisão
//! (`/ponytail-review`, `/ponytail-audit`) e um ledger de atalhos.
//!
//! Sem executável próprio: ela é a skill `ponytail` instalada no Claude Code,
//! direto em `~/.claude/skills/ponytail` ou, pelo jeito que o README dela
//! recomenda (`claude plugin install ponytail@ponytail`), como plugin — caso
//! em que o Claude Code não espelha o `SKILL.md` em `~/.claude/skills`, só
//! guarda o plugin habilitado em `~/.claude/settings.json` e o caminho dele
//! em `~/.claude/plugins/installed_plugins.json`. De um jeito ou de outro, o
//! agente já lê a skill sozinho; aqui ela só aparece na aba Extensões, para o
//! usuário ver se está instalada, e não há hook, MCP ou lançador para
//! acrescentar ao agente.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::extension::{Contribution, Extension, Status};

pub struct Ponytail {
    /// O `SKILL.md`, direto ou pelo plugin; `None` quando não está instalada.
    skill: Option<PathBuf>,
}

impl Ponytail {
    /// Procura a skill em `~/.claude/skills/ponytail` ou, instalada como
    /// plugin do Claude Code, pelo `installPath` dele.
    pub fn detect() -> Self {
        let skill = dirs::home_dir().and_then(|home| Self::find(&home));
        Self { skill }
    }

    /// A skill em `home/.claude/skills/ponytail`, ou a de um plugin.
    fn find(home: &Path) -> Option<PathBuf> {
        let direct = home.join(".claude/skills/ponytail/SKILL.md");
        if direct.is_file() {
            Some(direct)
        } else {
            Self::plugin_skill(home)
        }
    }

    /// O `SKILL.md` de um plugin `ponytail@<marketplace>` habilitado em
    /// `~/.claude/settings.json`, pelo caminho dele no
    /// `~/.claude/plugins/installed_plugins.json`.
    fn plugin_skill(home: &Path) -> Option<PathBuf> {
        let settings = read_json(&home.join(".claude/settings.json"))?;
        let id = settings["enabledPlugins"]
            .as_object()?
            .iter()
            .find(|(id, enabled)| id.starts_with("ponytail@") && enabled.as_bool() == Some(true))?
            .0;
        let installed = read_json(&home.join(".claude/plugins/installed_plugins.json"))?;
        let install_path = installed["plugins"][id][0]["installPath"].as_str()?;
        let skill = Path::new(install_path).join("skills/ponytail/SKILL.md");
        skill.is_file().then_some(skill)
    }
}

/// O JSON de `path`, ou `None` sem o arquivo ou com um JSON inválido.
fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

impl Extension for Ponytail {
    fn id(&self) -> &'static str {
        "ponytail"
    }

    fn name(&self) -> &'static str {
        "Ponytail"
    }

    fn description(&self) -> &'static str {
        "Skill que faz o agente escrever menos código, com revisão de diffs e ledger de atalhos"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/dietrichgebert/ponytail"
    }

    fn status(&self) -> Status {
        match self.skill {
            Some(_) => Status::Installed { version: None },
            None => Status::Missing,
        }
    }

    /// A skill já é lida pelo Claude Code sozinha; o terra só confirma que
    /// ela está instalada.
    fn contribution(&self) -> Result<Contribution, String> {
        Ok(Contribution::default())
    }

    /// Como plugin do Claude Code, do jeito que o README dela recomenda.
    fn setup(&self) -> Option<String> {
        Some(
            "claude plugin marketplace add DietrichGebert/ponytail\nclaude plugin install ponytail@ponytail"
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn status_follows_the_skill_file_and_has_no_contribution() {
        let installed = Ponytail {
            skill: Some(PathBuf::from("/home/eu/.claude/skills/ponytail/SKILL.md")),
        };
        assert_eq!(installed.status(), Status::Installed { version: None });
        assert_eq!(installed.contribution(), Ok(Contribution::default()));

        let missing = Ponytail { skill: None };
        assert_eq!(missing.status(), Status::Missing);
        assert!(missing.load_report().is_none());
    }

    /// Um `~/.claude` com o plugin `ponytail@ponytail` habilitado e instalado
    /// em `plugin_dir`, como o `claude plugin install` deixa.
    fn plugin_home(home: &Path, plugin_dir: &Path) {
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(
            claude.join("settings.json"),
            r#"{"enabledPlugins": {"ponytail@ponytail": true}}"#,
        )
        .unwrap();
        fs::create_dir_all(claude.join("plugins")).unwrap();
        fs::write(
            claude.join("plugins/installed_plugins.json"),
            format!(
                r#"{{"plugins": {{"ponytail@ponytail": [{{"installPath": {:?}}}]}}}}"#,
                plugin_dir.to_str().unwrap()
            ),
        )
        .unwrap();
    }

    #[test]
    fn finds_the_skill_of_an_enabled_plugin() {
        let home = tempfile::tempdir().unwrap();
        let plugin_dir = tempfile::tempdir().unwrap();
        plugin_home(home.path(), plugin_dir.path());
        let skill_dir = plugin_dir.path().join("skills/ponytail");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "...").unwrap();

        assert_eq!(
            Ponytail::plugin_skill(home.path()),
            Some(skill_dir.join("SKILL.md"))
        );
    }

    #[test]
    fn ignores_a_disabled_or_unknown_plugin() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude")).unwrap();
        fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"enabledPlugins": {"ponytail@ponytail": false}}"#,
        )
        .unwrap();
        assert_eq!(Ponytail::plugin_skill(home.path()), None);

        let bare = tempfile::tempdir().unwrap();
        assert_eq!(Ponytail::plugin_skill(bare.path()), None);
    }

    #[test]
    fn ignores_an_enabled_plugin_whose_skill_file_is_missing() {
        let home = tempfile::tempdir().unwrap();
        let plugin_dir = tempfile::tempdir().unwrap();
        plugin_home(home.path(), plugin_dir.path());

        assert_eq!(Ponytail::plugin_skill(home.path()), None);
    }

    #[test]
    fn direct_install_wins_over_a_plugin() {
        let home = tempfile::tempdir().unwrap();
        let plugin_dir = tempfile::tempdir().unwrap();
        plugin_home(home.path(), plugin_dir.path());
        let direct = home.path().join(".claude/skills/ponytail");
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join("SKILL.md"), "...").unwrap();

        assert_eq!(Ponytail::find(home.path()), Some(direct.join("SKILL.md")));
    }
}
