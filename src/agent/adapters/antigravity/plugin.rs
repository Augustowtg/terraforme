//! Os MCP das extensões no `agy`, sem gravar nos arquivos do usuário.
//!
//! O `agy` não tem flag nem variável de ambiente para um MCP por processo,
//! mas descobre plugins em `<pasta>/.agents/plugins/<nome>/` de cada pasta
//! passada em `--add-dir`. Então o terra monta, numa pasta temporária, um
//! plugin com o `mcp_config.json` da [`Contribution`] e passa a pasta no
//! `--add-dir`. A pasta some quando o [`Overlay`] é solto.
//!
//! O `agy` não resolve `${VAR}` nos cabeçalhos (vão literais), então o terra
//! substitui aqui, pelo `env` da Contribution e depois pelo ambiente. Por
//! isso o arquivo guarda o segredo: a pasta é só do usuário (0700) e o
//! arquivo também (0600).
//!
//! Os hooks da Contribution não vão: são do Claude Code (evento, matcher
//! `Bash`, comandos como `rtk hook claude`), e o `agy` manda outro payload e
//! espera outra resposta.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value, json};

use crate::extension::{Contribution, McpServer};

/// O nome do plugin do terra no `agy`.
const PLUGIN: &str = "terraforme";

/// A interface original em andamento. Fica aqui porque o `Command` que o
/// adapter devolve não carrega a pasta: a anterior é apagada quando uma
/// nova abre.
static INTERACTIVE: Mutex<Option<Overlay>> = Mutex::new(None);

/// A pasta temporária com o plugin; apagada ao soltar.
pub struct Overlay {
    dir: PathBuf,
}

impl Overlay {
    /// Monta o plugin com os servidores MCP de `extensions` que o usuário
    /// ainda não tem (`skip`, pelo nome). `None` quando não sobra nenhum.
    pub fn create(
        extensions: &Contribution,
        skip: &HashSet<String>,
    ) -> Result<Option<Overlay>, String> {
        let servers: Vec<&McpServer> = extensions
            .mcp_servers
            .iter()
            .filter(|server| !skip.contains(&server.name))
            .collect();
        if servers.is_empty() {
            return Ok(None);
        }
        let config = mcp_config(&servers, &extensions.env);
        let dir = make_dir().map_err(|err| format!("pasta temporária do plugin do agy: {err}"))?;
        let overlay = Overlay { dir };
        overlay
            .write(&config)
            .map_err(|err| format!("plugin do terra para o agy: {err}"))?;
        Ok(Some(overlay))
    }

    /// A pasta do `--add-dir`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn write(&self, config: &Value) -> io::Result<()> {
        let plugin = self.dir.join(".agents").join("plugins").join(PLUGIN);
        fs::create_dir_all(&plugin)?;
        fs::write(
            plugin.join("plugin.json"),
            json!({ "name": PLUGIN }).to_string(),
        )?;
        write_private(&plugin.join("mcp_config.json"), &config.to_string())
    }

    /// Guarda este plugin como o da interface original em andamento e apaga
    /// o anterior. Devolve a pasta.
    pub fn keep_as_interactive(self) -> PathBuf {
        let dir = self.dir.clone();
        *INTERACTIVE.lock().unwrap() = Some(self);
        dir
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// O `mcp_config.json` do `agy`: stdio com `command` e `args`, HTTP com
/// `url` e `headers` (já sem os `${VAR}`).
fn mcp_config(servers: &[&McpServer], env: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for server in servers {
        let entry = match &server.url {
            Some(url) => {
                let mut entry = json!({ "url": url });
                if !server.headers.is_empty() {
                    let headers: Map<String, Value> = server
                        .headers
                        .iter()
                        .map(|(name, value)| (name.clone(), expand(value, env).into()))
                        .collect();
                    entry["headers"] = headers.into();
                }
                entry
            }
            None => json!({ "command": server.command, "args": server.args }),
        };
        map.insert(server.name.clone(), entry);
    }
    json!({ "mcpServers": map })
}

/// Troca cada `${NOME}` pelo valor em `env` ou, na falta, no ambiente do
/// terra. Um nome sem valor fica como está.
fn expand(text: &str, env: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        let value = env
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var(name).ok());
        match value {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + 3 + end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Uma pasta nova em `temp`, só do usuário (0700 no Unix).
pub(super) fn make_dir() -> io::Result<PathBuf> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "terraforme-agy-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(dir)
}

/// Grava o arquivo com permissão só do usuário (0600 no Unix).
pub(super) fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            command: "ai-memory".into(),
            args: vec!["mcp-bridge".into()],
            ..Default::default()
        }
    }

    fn http(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            url: Some("http://127.0.0.1:1/mcp".into()),
            headers: vec![("Authorization".into(), "Bearer ${TERRA_TOKEN}".into())],
            ..Default::default()
        }
    }

    fn contribution(servers: Vec<McpServer>) -> Contribution {
        Contribution {
            mcp_servers: servers,
            env: vec![("TERRA_TOKEN".into(), "segredo".into())],
            ..Default::default()
        }
    }

    #[test]
    fn config_has_stdio_and_http_with_the_headers_resolved() {
        let config = mcp_config(&[&stdio("mem"), &http("terra")], &contribution(vec![]).env);
        assert_eq!(config["mcpServers"]["mem"]["command"], "ai-memory");
        assert_eq!(config["mcpServers"]["mem"]["args"][0], "mcp-bridge");
        assert_eq!(
            config["mcpServers"]["terra"]["url"],
            "http://127.0.0.1:1/mcp"
        );
        assert_eq!(
            config["mcpServers"]["terra"]["headers"]["Authorization"],
            "Bearer segredo"
        );
        assert!(config["mcpServers"]["mem"].get("headers").is_none());
    }

    #[test]
    fn expand_uses_env_then_the_environment_and_keeps_unknown_names() {
        let env = vec![("A".to_string(), "1".to_string())];
        assert_eq!(expand("x-${A}-y", &env), "x-1-y");
        assert_eq!(
            expand("${TERRAFORME_SEM_ESSA_VAR}", &env),
            "${TERRAFORME_SEM_ESSA_VAR}"
        );
        assert_eq!(expand("${A}${A}", &env), "11");
        assert_eq!(expand("aberto ${A", &env), "aberto ${A");
        assert!(!expand("${PATH}", &env).contains("${"));
    }

    #[test]
    fn the_overlay_writes_a_private_plugin_and_removes_it_on_drop() {
        let extensions = contribution(vec![http("terra")]);
        let overlay = Overlay::create(&extensions, &HashSet::new())
            .unwrap()
            .unwrap();
        let dir = overlay.dir().to_path_buf();
        let plugin = dir.join(".agents/plugins/terraforme");
        let manifest = fs::read_to_string(plugin.join("plugin.json")).unwrap();
        assert!(manifest.contains("terraforme"));
        let config = fs::read_to_string(plugin.join("mcp_config.json")).unwrap();
        assert!(config.contains("Bearer segredo"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&plugin.join("mcp_config.json")), 0o600);
        }
        drop(overlay);
        assert!(!dir.exists());
    }

    #[test]
    fn servers_the_user_already_has_are_not_repeated() {
        let extensions = contribution(vec![stdio("ai-memory"), http("terra")]);
        let skip = HashSet::from(["ai-memory".to_string()]);
        let overlay = Overlay::create(&extensions, &skip).unwrap().unwrap();
        let config = fs::read_to_string(
            overlay
                .dir()
                .join(".agents/plugins/terraforme/mcp_config.json"),
        )
        .unwrap();
        assert!(config.contains("terra") && !config.contains("ai-memory"));
        let only_user = contribution(vec![stdio("ai-memory")]);
        assert!(Overlay::create(&only_user, &skip).unwrap().is_none());
        assert!(
            Overlay::create(&Contribution::default(), &skip)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_new_interactive_overlay_removes_the_previous_one() {
        let extensions = contribution(vec![http("terra")]);
        let make = || {
            Overlay::create(&extensions, &HashSet::new())
                .unwrap()
                .unwrap()
        };
        let first = make().keep_as_interactive();
        assert!(first.exists());
        let second = make().keep_as_interactive();
        assert!(!first.exists());
        assert!(second.exists());
    }
}
