//! As skills instaladas no Claude, de todas as pastas que ele lê:
//!
//! - as do usuário (`<config>/skills`);
//! - as do projeto (o `.claude/skills` do `cwd` e das pastas acima dele);
//! - as compartilhadas entre agentes (`~/.agents/skills`);
//! - as sincronizadas do claude.ai (`<config>/skills/synced/<org>_<conta>`);
//! - as dos plugins (do `plugins/installed_plugins.json`, as do usuário e as
//!   do projeto `cwd`, e dos plugins sincronizados do claude.ai).
//!
//! Cada skill é uma pasta com um `SKILL.md`, cujo cabeçalho (frontmatter)
//! traz o `name` e a `description`. Uma skill com o nome de outra já listada
//! (ex.: a mesma no `~/.claude/skills` e no `~/.agents/skills`) aparece uma vez.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent::ProviderSkill;

/// As skills de `config_dir` (cuja conta está em `json`), de `home` e de
/// `cwd`: as do usuário, as do projeto, as dos agentes, as do claude.ai e as
/// dos plugins, nessa ordem, cada grupo em ordem alfabética.
pub fn installed(
    config_dir: &Path,
    json: &Path,
    home: Option<&Path>,
    cwd: &Path,
) -> Vec<ProviderSkill> {
    let mut found = folder(&config_dir.join("skills"), "usuário", None);
    for dir in cwd.ancestors().take_while(|dir| Some(*dir) != home) {
        found.extend(folder(&dir.join(".claude").join("skills"), "projeto", None));
    }
    if let Some(home) = home {
        found.extend(folder(
            &home.join(".agents").join("skills"),
            "agentes",
            None,
        ));
    }
    let bucket = bucket(json);
    if let Some(bucket) = &bucket {
        let dir = config_dir.join("skills").join("synced").join(bucket);
        found.extend(folder(&dir, "claude.ai", None));
    }
    let mut plugins = plugins(config_dir, cwd);
    if let Some(bucket) = &bucket {
        plugins.extend(synced_plugins(
            &config_dir.join("plugins").join("synced").join(bucket),
        ));
    }
    for (plugin, path) in plugins {
        found.extend(folder(&path.join("skills"), "plugin", Some(&plugin)));
    }
    let mut seen = HashSet::new();
    found.retain(|skill| seen.insert(skill.name.clone()));
    found
}

/// A pasta das coisas sincronizadas do claude.ai para a conta do `json`:
/// `<organizationUuid>_<accountUuid>`.
fn bucket(json: &Path) -> Option<String> {
    let json: Value = serde_json::from_str(&fs::read_to_string(json).ok()?).ok()?;
    let account = &json["oauthAccount"];
    let org = account["organizationUuid"].as_str()?;
    let id = account["accountUuid"].as_str()?;
    Some(format!("{org}_{id}"))
}

/// Os plugins sincronizados do claude.ai em `dir`: o nome (do
/// `.claude-plugin/plugin.json`) e a pasta.
fn synced_plugins(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let manifest = path.join(".claude-plugin").join("plugin.json");
            let json: Value = serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
            Some((json["name"].as_str()?.to_string(), path))
        })
        .collect();
    found.sort();
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
pub(in crate::agent::adapters) fn folder(
    dir: &Path,
    scope: &'static str,
    plugin: Option<&str>,
) -> Vec<ProviderSkill> {
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

        let skills = installed(
            &config,
            &dir.path().join(".claude.json"),
            Some(dir.path()),
            &cwd,
        );
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
        let x = dir.path().join("x");
        assert!(installed(&x, &x.join(".claude.json"), Some(dir.path()), dir.path()).is_empty());
    }

    #[test]
    fn reads_agents_parent_and_synced_skills_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let config = home.join(".claude");
        let cwd = home.join("repo/crate");
        let bucket = "org_conta";
        let json = home.join(".claude.json");
        let account = serde_json::json!({
            "oauthAccount": {"organizationUuid": "org", "accountUuid": "conta"}
        });
        fs::write(&json, account.to_string()).unwrap();
        skill(&config.join("skills"), "docs", "---\nname: docs\n---");
        skill(
            &home.join("repo/.claude/skills"),
            "deploy",
            "---\nname: deploy\n---",
        );
        skill(&home.join(".agents/skills"), "docs", "---\nname: docs\n---");
        skill(
            &home.join(".agents/skills"),
            "memoria",
            "---\nname: memoria\n---",
        );
        skill(
            &config.join("skills/synced").join(bucket),
            "pdf",
            "---\nname: pdf\n---",
        );
        skill(
            &config.join("skills/synced/outra_conta"),
            "xlsx",
            "---\nname: xlsx\n---",
        );
        let plugin = config.join("plugins/synced").join(bucket).join("uuid");
        fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        fs::write(
            plugin.join(".claude-plugin/plugin.json"),
            r#"{"name": "postman"}"#,
        )
        .unwrap();
        skill(&plugin.join("skills"), "flows", "---\nname: flows\n---");

        let skills = installed(&config, &json, Some(home), &cwd);
        let names: Vec<_> = skills.iter().map(|s| (s.name.as_str(), s.scope)).collect();
        assert_eq!(
            names,
            [
                ("docs", "usuário"),
                ("deploy", "projeto"),
                ("memoria", "agentes"),
                ("pdf", "claude.ai"),
                ("postman:flows", "plugin")
            ]
        );
    }
}
