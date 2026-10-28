//! O chat delegado pelo orquestrador roda enxuto: sem o ambiente do Claude
//! Code do usuário, que ocupa uns 30k tokens no prompt de cada chamada.
//!
//! - MCP: só o `terra` (que mostra ao delegado só a `permission`), o
//!   [`DELEGATED_MCP`] e os que quem delegou pediu (o `mcp` da ferramenta
//!   `chat`), num arquivo ([`McpFile`]) com `--strict-mcp-config`;
//! - sem os settings do usuário (`--setting-sources project,local`): sem os
//!   hooks, os plugins e as permissões dele; o `--settings` desliga todos os
//!   hooks;
//! - sem as skills e os comandos (`--disable-slash-commands`);
//! - só as ferramentas [`DELEGATED_TOOLS`] e as que quem delegou pediu
//!   (`--tools`), mais a [`TOOL_SEARCH`];
//! - sem modelo escolhido no Orquestrador, [`MODEL`], e, sem esforço
//!   também, [`EFFORT`].
//!
//! O arquivo do MCP fica em `~/.terraforme/delegados/` (só do usuário), e
//! não na linha de comando, porque leva os servidores do usuário como estão,
//! com os segredos do `env` e dos cabeçalhos. Ele some quando a resposta
//! termina.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value};

use super::server_config;
use crate::agent::Delegated;
use crate::extension::Contribution;
use crate::mcp::{DELEGATED_MCP, DELEGATED_TOOLS, SERVER_NAME};

/// O modelo de um chat delegado sem modelo escolhido no Orquestrador.
pub const MODEL: &str = "sonnet";
/// O esforço de um chat delegado sem esforço escolhido no Orquestrador.
pub const EFFORT: &str = "medium";
/// A ferramenta com que o `claude` adia os schemas das ferramentas MCP (o
/// agente carrega só as que for usar). Sem ela no `--tools`, os schemas vão
/// inteiros no prompt: só o ai-memory soma uns 24k tokens (medido no
/// `claude` 2.1.308: o prefixo de 11k vira 33k). Sem MCP, ela não custa nada.
const TOOL_SEARCH: &str = "ToolSearch";
/// Os settings de um chat delegado: nenhum hook.
pub const SETTINGS: &str = r#"{"disableAllHooks":true}"#;

/// Os servidores do chat delegado: dos das extensões (`extensions`), o
/// `terra`, o [`DELEGATED_MCP`] e os pedidos; dos do usuário (`user`, como
/// estão nos arquivos), os que faltarem desses.
pub fn servers(
    extensions: &Contribution,
    user: &Map<String, Value>,
    delegated: Delegated<'_>,
) -> Map<String, Value> {
    let wanted =
        |name: &str| name == DELEGATED_MCP || delegated.mcp.iter().any(|m| m.as_str() == name);
    let mut servers: Map<String, Value> = extensions
        .mcp_servers
        .iter()
        .filter(|server| server.name == SERVER_NAME || wanted(&server.name))
        .map(|server| (server.name.clone(), server_config(server)))
        .collect();
    for (name, config) in user {
        if wanted(name) && !servers.contains_key(name) {
            servers.insert(name.clone(), config.clone());
        }
    }
    servers
}

/// Os argumentos do chat delegado, com o MCP no arquivo `mcp`.
pub fn args(mcp: &Path, delegated: Delegated<'_>) -> Vec<String> {
    let mut tools: Vec<&str> = DELEGATED_TOOLS.to_vec();
    tools.push(TOOL_SEARCH);
    for tool in delegated.tools {
        if !tools.contains(&tool.as_str()) {
            tools.push(tool);
        }
    }
    vec![
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        mcp.to_string_lossy().into_owned(),
        "--setting-sources".into(),
        "project,local".into(),
        "--disable-slash-commands".into(),
        "--tools".into(),
        tools.join(","),
    ]
}

/// O arquivo do `--mcp-config` de um chat delegado; some ao ser solto.
pub struct McpFile(PathBuf);

impl McpFile {
    /// Grava `servers` num arquivo novo em `dir`, que é criado só do
    /// usuário.
    pub fn create(dir: &Path, servers: Map<String, Value>) -> io::Result<Self> {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        let path = dir.join(format!(
            "{}-{}.json",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let text = serde_json::json!({ "mcpServers": servers }).to_string();
        write_private(&path, &text)?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for McpFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Grava o arquivo com permissão só do usuário (0600 no Unix); no Windows,
/// a pasta do usuário já é só dele.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::McpServer;

    fn server(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            command: format!("{name}-mcp"),
            ..Default::default()
        }
    }

    fn user() -> Map<String, Value> {
        serde_json::json!({
            "ai-memory": {"command": "user-memory"},
            "git": {"command": "git-mcp", "env": {"TOKEN": "x"}},
            "postman": {"type": "http", "url": "http://x/mcp"}
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn keeps_terra_memory_and_only_what_was_asked() {
        let extensions = Contribution {
            mcp_servers: vec![server(SERVER_NAME), server("rtk"), server("ai-memory")],
            ..Default::default()
        };
        let asked = ["git".to_string()];
        let delegated = Delegated {
            mcp: &asked,
            tools: &[],
        };
        let servers = servers(&extensions, &user(), delegated);
        let mut names: Vec<_> = servers.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, ["ai-memory", "git", "terra"]);
        // O da extensão vence o do usuário, e o do usuário vai como está.
        assert_eq!(servers["ai-memory"]["command"], "ai-memory-mcp");
        assert_eq!(servers["git"]["env"]["TOKEN"], "x");
    }

    #[test]
    fn without_the_extension_memory_comes_from_the_user() {
        let delegated = Delegated {
            mcp: &[],
            tools: &[],
        };
        let servers = servers(&Contribution::default(), &user(), delegated);
        assert_eq!(servers.keys().collect::<Vec<_>>(), ["ai-memory"]);
        assert_eq!(servers["ai-memory"]["command"], "user-memory");
    }

    #[test]
    fn args_are_lean_and_add_the_asked_tools() {
        let tools = ["WebFetch".to_string(), "Bash".to_string()];
        let delegated = Delegated {
            mcp: &[],
            tools: &tools,
        };
        let args = args(Path::new("/tmp/m.json"), delegated);
        assert_eq!(
            args,
            [
                "--strict-mcp-config",
                "--mcp-config",
                "/tmp/m.json",
                "--setting-sources",
                "project,local",
                "--disable-slash-commands",
                "--tools",
                "Bash,Read,Edit,Write,Glob,Grep,ToolSearch,WebFetch",
            ]
        );
    }

    #[test]
    fn the_file_is_private_and_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("delegados");
        let file = McpFile::create(&dir, user()).unwrap();
        let path = file.path().to_path_buf();
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["mcpServers"]["git"]["command"], "git-mcp");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }
        drop(file);
        assert!(!path.exists());
    }
}
