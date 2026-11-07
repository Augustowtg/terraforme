//! O MCP do orquestrador (`terra mcp`): um servidor MCP por stdio (JSON-RPC
//! 2.0, uma mensagem por linha) que todo agente despachado recebe por
//! padrão, com as mesmas operações do `terra send`, `terra done`,
//! `terra handoff new` e `terra status`:
//!
//! | Ferramenta | O que faz |
//! | --- | --- |
//! | `send` | Manda uma mensagem a outro agente (níveis 1 a 3) |
//! | `done` | Conclui a execução despachada, com o resumo |
//! | `handoff_new` | Cria a nota de handoff em branco e devolve o caminho |
//! | `status` | Os terminais do terra abertos, por workspace |
//!
//! Quem usa o servidor (o remetente, os saltos e a execução) vem da
//! [`Identity`], que o despachante passa nos argumentos: assim ela não
//! depende de o agente repassar o ambiente dele ao servidor.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

use crate::hub::endpoint::Endpoint;
use crate::orchestrator::{self, config::Config};

/// O nome do servidor na configuração do agente: as ferramentas aparecem
/// como `mcp__terra__send`, `mcp__terra__done`...
pub const NAME: &str = "terra";
/// A versão do protocolo MCP quando o cliente não diz a dele.
const PROTOCOL: &str = "2025-06-18";

/// Quem usa o servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// O remetente do `send` e da nota de handoff.
    pub agent: String,
    /// Os saltos da mensagem que gerou a execução.
    pub hops: u32,
    /// A execução despachada, para o `done`.
    pub run: Option<String>,
    /// A pasta dos caminhos relativos do `send`.
    pub cwd: PathBuf,
}

/// Atende o agente pela entrada e pela saída até a entrada fechar.
pub fn serve(identity: &Identity) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line, identity) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// A resposta a uma linha do cliente; `None` para uma notificação (sem
/// `id`), que não tem resposta.
pub fn handle(line: &str, identity: &Identity) -> Option<Value> {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return Some(error(Value::Null, -32700, "JSON inválido"));
    };
    let id = request.get("id").cloned()?;
    let params = &request["params"];
    let reply = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": NAME, "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let (text, is_error) = match call(name, &params["arguments"], identity) {
                Ok(text) => (text, false),
                Err(err) => (format!("{err:#}"), true),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        method => return Some(error(id, -32601, &format!("método desconhecido: {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": reply }))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// As ferramentas, com o schema dos argumentos. As descrições são curtas:
/// elas entram no prompt de toda execução.
fn tools() -> Value {
    json!([
        {
            "name": "send",
            "description": "Manda uma mensagem a outro agente (o nome de uma aba ou de um [agents.<nome>] do orchestrator.toml). Nível 1: text; nível 2: text e refs; nível 3: handoff (a nota do handoff_new, preenchida).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "O agente de destino" },
                    "text": { "type": "string", "description": "O texto da mensagem" },
                    "refs": { "type": "array", "items": { "type": "string" }, "description": "Arquivos para o agente ler" },
                    "handoff": { "type": "string", "description": "O caminho da nota de handoff" },
                    "tag": { "type": "string", "description": "Uma etiqueta (ex.: testes)" }
                },
                "required": ["agent"]
            }
        },
        {
            "name": "done",
            "description": "Conclui esta execução despachada com um resumo em uma linha.",
            "inputSchema": {
                "type": "object",
                "properties": { "summary": { "type": "string", "description": "O resumo do que foi feito" } },
                "required": ["summary"]
            }
        },
        {
            "name": "handoff_new",
            "description": "Cria uma nota de handoff em branco para o agente e devolve o caminho; preencha e mande com send (handoff).",
            "inputSchema": {
                "type": "object",
                "properties": { "agent": { "type": "string", "description": "O agente de destino" } },
                "required": ["agent"]
            }
        },
        {
            "name": "status",
            "description": "Os terminais do terra abertos, por workspace, com os agentes de cada um.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

/// Roda a ferramenta `name` e devolve o texto da resposta.
fn call(name: &str, args: &Value, identity: &Identity) -> Result<String> {
    let text = |key: &str| {
        args[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    match name {
        "send" => {
            let endpoint = Endpoint::current()?;
            let config = Config::load(&Config::path(&endpoint.home))
                .map_err(|err| anyhow::anyhow!("orchestrator.toml: {err}"))?;
            let send = orchestrator::send::Args {
                to: text("agent").unwrap_or_default(),
                text: text("text"),
                refs: args["refs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect(),
                handoff: text("handoff").map(PathBuf::from),
                tag: text("tag"),
            };
            crate::cli::send_one(
                &send,
                &identity.agent,
                identity.hops,
                &identity.cwd,
                &config,
                &endpoint,
            )
        }
        "done" => {
            let Some(run) = &identity.run else {
                anyhow::bail!("o done só funciona dentro de uma execução do orquestrador");
            };
            crate::cli::done(run, &text("summary").unwrap_or_default())?;
            Ok("ok".into())
        }
        "handoff_new" => {
            let to = text("agent").unwrap_or_default();
            let path = crate::cli::new_handoff(&to, &identity.agent)?;
            Ok(path.display().to_string())
        }
        "status" => crate::cli::status_text(false),
        _ => anyhow::bail!("ferramenta desconhecida: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            agent: "api".into(),
            hops: 1,
            run: None,
            cwd: PathBuf::from("/p"),
        }
    }

    fn ask(line: &str) -> Value {
        handle(line, &identity()).expect("uma resposta")
    }

    #[test]
    fn initialize_echoes_the_protocol_and_offers_tools() {
        let reply = ask(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        );
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], NAME);
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_have_no_reply() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle(line, &identity()).is_none());
    }

    #[test]
    fn the_tools_are_the_orchestrator_commands() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["send", "done", "handoff_new", "status"]);
    }

    #[test]
    fn done_outside_a_run_is_a_tool_error() {
        let reply = ask(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"done","arguments":{"summary":"x"}}}"#,
        );
        assert_eq!(reply["result"]["isError"], true);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("execução do orquestrador"));
    }

    #[test]
    fn unknown_methods_and_bad_json_are_errors() {
        assert_eq!(
            ask(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#)["error"]["code"],
            -32601
        );
        assert_eq!(ask("{nao")["error"]["code"], -32700);
    }
}
