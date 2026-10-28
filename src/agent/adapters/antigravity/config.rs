//! O que a Config mostra do Antigravity, lido dos arquivos dele (só
//! leitura; o terra não grava nada do `agy`):
//!
//! - o modo do agente: `agentMode` do `~/.gemini/antigravity-cli/settings.json`;
//! - os MCP: `mcpServers` do `~/.gemini/config/mcp_config.json`;
//! - as skills: as embutidas (`~/.gemini/antigravity-cli/builtin/skills`),
//!   as do usuário (`~/.gemini/config/skills`) e as do projeto (o
//!   `.agents/skills` do `cwd` e das pastas acima dele).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::agent::adapters::claude::{mcp, skills};
use crate::agent::{PermissionMode, ProviderMcp, ProviderSkill};

/// O modo do agente salvo no `/config` do `agy` (`agentMode`).
pub fn configured_mode(home: &Path) -> Option<PermissionMode> {
    let settings = read(&home.join(".gemini/antigravity-cli/settings.json"));
    parse_mode(settings["agentMode"].as_str()?)
}

/// `agentMode` → modo do terra. O `default` do `agy` revisa as edições antes
/// de gravar: é o mais próximo do Manual.
pub fn parse_mode(name: &str) -> Option<PermissionMode> {
    Some(match name {
        "auto" => PermissionMode::Agente,
        "default" => PermissionMode::Manual,
        "accept-edits" => PermissionMode::AcceptEdits,
        "plan" => PermissionMode::Plan,
        _ => return None,
    })
}

/// Os MCP globais do `agy`. Um servidor HTTP pode usar a chave antiga
/// `serverUrl` no lugar de `url`.
pub fn mcp_servers(home: &Path) -> Vec<ProviderMcp> {
    let mut config = read(&home.join(".gemini/config/mcp_config.json"));
    if let Some(servers) = config["mcpServers"].as_object_mut() {
        for server in servers.values_mut() {
            if server.get("url").is_none()
                && let Some(url) = server.get("serverUrl").cloned()
            {
                server["url"] = url;
            }
        }
    }
    mcp::servers(&config["mcpServers"], "usuário")
}

/// As skills embutidas, as do usuário e as do projeto, nessa ordem; um nome
/// repetido aparece uma vez.
pub fn skills(home: &Path, cwd: &Path) -> Vec<ProviderSkill> {
    let builtin = home.join(".gemini/antigravity-cli/builtin/skills");
    let mut found = skills::folder(&builtin, "embutida", None);
    found.extend(skills::folder(
        &home.join(".gemini/config/skills"),
        "usuário",
        None,
    ));
    for dir in cwd.ancestors().take_while(|dir| *dir != home) {
        found.extend(skills::folder(
            &dir.join(".agents").join("skills"),
            "projeto",
            None,
        ));
    }
    let mut seen = HashSet::new();
    found.retain(|skill| seen.insert(skill.name.clone()));
    found
}

fn read(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&strip_jsonc(&text)).ok())
        .unwrap_or(Value::Null)
}

/// O `mcp_config.json` do `agy` aceita comentários (`//` e `/* */`) e
/// vírgula sobrando antes de `}` ou `]`; tira os dois para o `serde_json`.
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let without_comments =
        copy_outside_strings(&chars, |chars, i| match (chars[i], chars.get(i + 1)) {
            ('/', Some('/')) => Some(line_end(chars, i)),
            ('/', Some('*')) => Some(comment_end(chars, i)),
            _ => None,
        });
    let chars: Vec<char> = without_comments.chars().collect();
    copy_outside_strings(&chars, |chars, i| {
        let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
        (chars[i] == ',' && matches!(next, Some('}' | ']'))).then_some(i + 1)
    })
}

/// Copia `chars`, pulando o trecho que `skip` indica (devolve onde ele
/// acaba) fora das aspas. Texto entre aspas fica como está (ex.: uma URL
/// com `//`).
fn copy_outside_strings(chars: &[char], skip: impl Fn(&[char], usize) -> Option<usize>) -> String {
    let mut out = String::with_capacity(chars.len());
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if let Some(end) = skip(chars, i) {
            i = end;
            continue;
        } else {
            in_string = c == '"';
            out.push(c);
        }
        i += 1;
    }
    out
}

/// O fim da linha do comentário `//` que começa em `from`.
fn line_end(chars: &[char], from: usize) -> usize {
    (from..chars.len())
        .find(|&i| chars[i] == '\n')
        .unwrap_or(chars.len())
}

/// O que vem depois do `*/` do comentário que começa em `from`.
fn comment_end(chars: &[char], from: usize) -> usize {
    (from + 2..chars.len().saturating_sub(1))
        .find(|&i| chars[i] == '*' && chars[i + 1] == '/')
        .map_or(chars.len(), |i| i + 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn modes_follow_the_agent_mode_of_the_settings() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(configured_mode(home.path()), None);
        let settings = home.path().join(".gemini/antigravity-cli/settings.json");
        write(&settings, r#"{"agentMode":"accept-edits"}"#);
        assert_eq!(
            configured_mode(home.path()),
            Some(PermissionMode::AcceptEdits)
        );
        assert_eq!(parse_mode("auto"), Some(PermissionMode::Agente));
        assert_eq!(parse_mode("default"), Some(PermissionMode::Manual));
        assert_eq!(parse_mode("plan"), Some(PermissionMode::Plan));
        assert_eq!(parse_mode("x"), None);
    }

    #[test]
    fn mcp_servers_accept_stdio_and_http() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".gemini/config/mcp_config.json"),
            r#"{"mcpServers":{
                "mem":{"command":"ai-memory","args":["mcp-bridge"]},
                "web":{"serverUrl":"https://example.com/mcp"}
            }}"#,
        );
        let servers = mcp_servers(home.path());
        assert_eq!(servers.len(), 2);
        let mem = servers.iter().find(|s| s.server.name == "mem").unwrap();
        assert_eq!(mem.server.command, "ai-memory");
        assert_eq!(mem.scope, "usuário");
        let web = servers.iter().find(|s| s.server.name == "web").unwrap();
        assert_eq!(web.server.url.as_deref(), Some("https://example.com/mcp"));
    }

    #[test]
    fn mcp_config_accepts_comments_and_trailing_commas() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".gemini/config/mcp_config.json"),
            "// servidores\n{\"mcpServers\": {\n  /* um */\n  \"web\": {\"url\": \"https://example.com/mcp\", // fim\n  },\n}, }\n",
        );
        let servers = mcp_servers(home.path());
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].server.url.as_deref(),
            Some("https://example.com/mcp")
        );
        assert_eq!(strip_jsonc(r#"["a,]", "b//c"]"#), r#"["a,]", "b//c"]"#);
    }

    #[test]
    fn skills_come_from_builtin_user_and_project() {
        let home = tempfile::tempdir().unwrap();
        let skill = |dir: &Path, name: &str| {
            write(
                &dir.join(name).join("SKILL.md"),
                &format!("---\nname: {name}\ndescription: faz {name}\n---\n"),
            );
        };
        skill(
            &home.path().join(".gemini/antigravity-cli/builtin/skills"),
            "guia",
        );
        skill(&home.path().join(".gemini/config/skills"), "minha");
        let project = home.path().join("projeto");
        skill(&project.join(".agents/skills"), "do-projeto");
        skill(&project.join(".agents/skills"), "guia");
        let found = skills(home.path(), &project);
        let names: Vec<(&str, &str)> = found.iter().map(|s| (s.name.as_str(), s.scope)).collect();
        assert_eq!(
            names,
            [
                ("guia", "embutida"),
                ("minha", "usuário"),
                ("do-projeto", "projeto")
            ]
        );
    }
}
