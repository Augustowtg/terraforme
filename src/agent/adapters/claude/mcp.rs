//! Os servidores MCP configurados no Claude: os do usuário e os locais (do
//! `.claude.json`) e os do projeto (do `.mcp.json`).

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::agent::ProviderMcp;
use crate::extension::McpServer;

/// Os MCP do `.claude.json` em `json` e do `.mcp.json` de `cwd`: os do
/// usuário, os locais e os do projeto, nessa ordem.
pub fn configured(json: &Path, cwd: &Path) -> Vec<ProviderMcp> {
    let config = read(json);
    let project = read(&cwd.join(".mcp.json"));
    let local = &config["projects"][cwd.to_string_lossy().as_ref()]["mcpServers"];
    let mut found = servers(&config["mcpServers"], "usuário");
    found.extend(servers(local, "local"));
    found.extend(servers(&project["mcpServers"], "projeto"));
    found
}

/// Os MCP de [`configured`] como estão nos arquivos (com `env`, `type` e o
/// que mais houver), pelo nome. No nome repetido, vale o mais específico:
/// local, projeto, usuário.
pub fn raw(json: &Path, cwd: &Path) -> serde_json::Map<String, Value> {
    let config = read(json);
    let project = read(&cwd.join(".mcp.json"));
    let local = &config["projects"][cwd.to_string_lossy().as_ref()]["mcpServers"];
    let mut found = serde_json::Map::new();
    for servers in [&config["mcpServers"], &project["mcpServers"], local] {
        if let Some(servers) = servers.as_object() {
            found.extend(servers.clone());
        }
    }
    found
}

fn read(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// Os servidores de um objeto `mcpServers`.
pub(in crate::agent::adapters) fn servers(value: &Value, scope: &'static str) -> Vec<ProviderMcp> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    map.iter()
        .map(|(name, server)| ProviderMcp {
            server: McpServer {
                name: name.clone(),
                command: server["command"].as_str().unwrap_or_default().to_string(),
                args: server["args"]
                    .as_array()
                    .map(|args| {
                        args.iter()
                            .filter_map(|a| a.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                url: server["url"].as_str().map(str::to_string),
                headers: server["headers"]
                    .as_object()
                    .map(|headers| {
                        headers
                            .iter()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            scope,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_user_local_and_project_servers() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir(&cwd).unwrap();
        let json = dir.path().join(".claude.json");
        let config = serde_json::json!({
            "mcpServers": {
                "ai-memory": {"command": "ai-memory", "args": ["mcp-bridge"]}
            },
            "projects": {
                cwd.to_string_lossy(): {
                    "mcpServers": {"docs": {"type": "http", "url": "http://x/mcp"}}
                }
            }
        });
        fs::write(&json, config.to_string()).unwrap();
        fs::write(
            cwd.join(".mcp.json"),
            r#"{"mcpServers": {"db": {"command": "db-mcp"}}}"#,
        )
        .unwrap();

        let servers = configured(&json, &cwd);
        let names: Vec<_> = servers
            .iter()
            .map(|s| (s.server.name.as_str(), s.scope))
            .collect();
        assert_eq!(
            names,
            [
                ("ai-memory", "usuário"),
                ("docs", "local"),
                ("db", "projeto")
            ]
        );
        assert_eq!(servers[0].server.args, ["mcp-bridge"]);
        assert_eq!(servers[1].server.url.as_deref(), Some("http://x/mcp"));
    }

    #[test]
    fn raw_keeps_every_field_and_the_most_specific_wins() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir(&cwd).unwrap();
        let json = dir.path().join(".claude.json");
        let config = serde_json::json!({
            "mcpServers": {
                "git": {"command": "git-mcp", "env": {"TOKEN": "x"}},
                "docs": {"command": "user-docs"}
            },
            "projects": {
                cwd.to_string_lossy(): {"mcpServers": {"docs": {"command": "local-docs"}}}
            }
        });
        fs::write(&json, config.to_string()).unwrap();
        fs::write(
            cwd.join(".mcp.json"),
            r#"{"mcpServers": {"docs": {"command": "project-docs"}}}"#,
        )
        .unwrap();

        let servers = raw(&json, &cwd);
        assert_eq!(servers["git"]["env"]["TOKEN"], "x");
        assert_eq!(servers["docs"]["command"], "local-docs");
    }

    #[test]
    fn without_files_there_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(configured(&dir.path().join("x.json"), dir.path()).is_empty());
    }
}
