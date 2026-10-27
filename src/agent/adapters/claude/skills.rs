//! As skills instaladas no Claude: as do usuário (`<config>/skills`), as do
//! projeto (`<cwd>/.claude/skills`) e as dos plugins (do
//! `plugins/installed_plugins.json`, as do usuário e as do projeto `cwd`).
//!
//! Cada skill é uma pasta com um `SKILL.md`, cujo cabeçalho (frontmatter)
//! traz o `name` e a `description`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent::ProviderSkill;

/// As skills de `config_dir` e de `cwd`: as do usuário, as do projeto e as
/// dos plugins, nessa ordem, cada grupo em ordem alfabética.
pub fn installed(config_dir: &Path, cwd: &Path) -> Vec<ProviderSkill> {
    let mut found = folder(&config_dir.join("skills"), "usuário", None);
    found.extend(folder(&cwd.join(".claude").join("skills"), "projeto", None));
    for (plugin, path) in plugins(config_dir, cwd) {
        found.extend(folder(&path.join("skills"), "plugin", Some(&plugin)));
    }
    found
}

/// Os plugins instalados para o usuário ou para o projeto `cwd`: o nome
/// (sem o `@marketplace`) e a pasta.
fn plugins(config_dir: &Path, cwd: &Path) -> Vec<(String, PathBuf)> {
    let path = config_dir.join("plugins").join("installed_plugins.json");
    let Some(json) = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    else {
        return Vec::new();
    };
    let Some(map) = json["plugins"].as_object() else {
        return Vec::new();
    };
    let cwd = cwd.to_string_lossy();
    let mut found: Vec<(String, PathBuf)> = map
        .iter()
        .filter_map(|(id, installs)| {
            let install = installs.as_array()?.iter().find(|i| {
                i["scope"] == "user" || i["projectPath"].as_str() == Some(cwd.as_ref())
            })?;
            let name = id.split('@').next().unwrap_or(id).to_string();
            Some((name, PathBuf::from(install["installPath"].as_str()?)))
        })
        .collect();
    found.sort();
    found
}

/// As skills das pastas de `dir`; as de plugin ficam `plugin:nome`.
fn folder(dir: &Path, scope: &'static str, plugin: Option<&str>) -> Vec<ProviderSkill> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<ProviderSkill> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path().join("SKILL.md");
            let text = fs::read_to_string(&path).ok()?;
            let (name, description) = frontmatter(&text);
            let name = name.unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            Some(ProviderSkill {
                name: match plugin {
                    Some(plugin) => format!("{plugin}:{name}"),
                    None => name,
                },
                description: description.unwrap_or_default(),
                scope,
                path,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// O `name` e a `description` do cabeçalho entre `---`.
fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let (mut name, mut description) = (None, None);
    for line in lines.take_while(|l| l.trim() != "---") {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        match key.trim() {
            "name" if !value.is_empty() => name = Some(value),
            "description" if !value.is_empty() => description = Some(value),
            _ => {}
        }
    }
    (name, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, folder: &str, body: &str) {
        let dir = dir.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn reads_user_project_and_plugin_skills() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".claude");
        let cwd = dir.path().join("repo");
        let plugin = dir.path().join("cache/postman");
        skill(
            &config.join("skills"),
            "docs",
            "---\nname: docs\ndescription: \"Escreve docs\"\n---\n# Docs",
        );
        skill(&cwd.join(".claude/skills"), "deploy", "# sem cabeçalho");
        skill(
            &plugin.join("skills"),
            "mock",
            "---\nname: api-mocking\ndescription: Mocka\n---",
        );
        fs::create_dir_all(config.join("plugins")).unwrap();
        let plugins_json = serde_json::json!({
            "plugins": {
                "postman@official": [{"scope": "user", "installPath": plugin}],
                "github@official": [{
                    "scope": "project",
                    "projectPath": "/outro",
                    "installPath": dir.path().join("cache/github")
                }]
            }
        });
        fs::write(
            config.join("plugins/installed_plugins.json"),
            plugins_json.to_string(),
        )
        .unwrap();

        let skills = installed(&config, &cwd);
        let names: Vec<_> = skills.iter().map(|s| (s.name.as_str(), s.scope)).collect();
        assert_eq!(
            names,
            [
                ("docs", "usuário"),
                ("deploy", "projeto"),
                ("postman:api-mocking", "plugin")
            ]
        );
        assert_eq!(skills[0].description, "Escreve docs");
        assert!(skills[1].description.is_empty());
    }

    #[test]
    fn without_folders_there_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(installed(&dir.path().join("x"), dir.path()).is_empty());
    }
}
