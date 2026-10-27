//! O servidor do MCP (`terra mcp`): JSON-RPC 2.0 por HTTP no endpoint
//! [`PATH`](super::PATH), como o transporte HTTP do MCP (respostas em
//! `application/json`).
//!
//! Métodos do MCP: `initialize`, `ping`, `tools/list` e `tools/call` (a
//! ferramenta `spaces`, os terra abertos). Métodos do terra:
//! `terra/announce` (um terra se registra ou continua aberto),
//! `terra/leave` (saiu), `terra/spaces` (os terra abertos) e
//! `terra/shutdown` (fecha o MCP). Um terra que para de avisar por
//! [`EXPIRE`] sai da lista.
//!
//! As requisições são atendidas uma de cada vez: são pequenas e locais.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{Instance, PATH};

/// Sem avisar por esse tempo, o terra sai da lista.
const EXPIRE: Duration = Duration::from_secs(6);
/// Quanto esperar um cliente mandar a requisição.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// O maior corpo aceito.
const MAX_BODY: usize = 1 << 20;
/// A versão do protocolo do MCP.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Abre o MCP na porta [`PORT`](super::PORT) e atende até o
/// `terra/shutdown`.
pub fn run() -> Result<()> {
    let listener = TcpListener::bind(super::addr()).with_context(|| {
        format!(
            "não foi possível abrir o MCP em {} (a porta já está em uso?)",
            super::addr()
        )
    })?;
    serve(&listener);
    Ok(())
}

/// Atende as conexões de `listener` até o `terra/shutdown`.
pub fn serve(listener: &TcpListener) {
    let mut state = State::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // Um cliente com problema não derruba o MCP.
        if let Ok(Flow::Stop) = handle_connection(stream, &mut state) {
            break;
        }
    }
}

/// Os terra registrados, pelo processo, e quando avisaram pela última vez.
#[derive(Default)]
struct State {
    instances: HashMap<u32, (Instance, Instant)>,
}

impl State {
    /// Os terra que continuam abertos, por nome e processo.
    fn alive(&mut self) -> Vec<Instance> {
        self.instances
            .retain(|_, (_, seen)| seen.elapsed() < EXPIRE);
        let mut alive: Vec<Instance> = self.instances.values().map(|(i, _)| i.clone()).collect();
        alive.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
        alive
    }
}

/// Se o MCP continua atendendo depois de uma requisição.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

fn handle_connection(stream: TcpStream, state: &mut State) -> Result<Flow> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let length = read_headers(&mut reader)?;
    if path != PATH {
        respond(&mut writer, "404 Not Found", "")?;
        return Ok(Flow::Continue);
    }
    if method != "POST" {
        respond(&mut writer, "405 Method Not Allowed", "")?;
        return Ok(Flow::Continue);
    }
    if length > MAX_BODY {
        respond(&mut writer, "413 Payload Too Large", "")?;
        return Ok(Flow::Continue);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let (response, flow) = match serde_json::from_slice::<Value>(&body) {
        Ok(request) => handle(&request, state),
        Err(_) => (
            Some(error(Value::Null, -32700, "JSON inválido")),
            Flow::Continue,
        ),
    };
    match response {
        Some(response) => respond(&mut writer, "200 OK", &response.to_string())?,
        // Uma notificação não tem resposta.
        None => respond(&mut writer, "202 Accepted", "")?,
    }
    Ok(flow)
}

/// Lê os cabeçalhos até a linha vazia e retorna o `Content-Length` (0 sem
/// ele).
pub(super) fn read_headers(reader: &mut impl BufRead) -> Result<usize> {
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("a conexão fechou no meio dos cabeçalhos");
        }
        let line = line.trim_end();
        if line.is_empty() {
            return Ok(length);
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().context("Content-Length inválido")?;
        }
    }
}

fn respond(writer: &mut impl Write, status: &str, body: &str) -> Result<()> {
    let content_type = if body.is_empty() {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    write!(
        writer,
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    writer.flush()?;
    Ok(())
}

/// Responde a uma requisição JSON-RPC; `None` nas notificações (sem `id`).
fn handle(request: &Value, state: &mut State) -> (Option<Value>, Flow) {
    let id = request.get("id").cloned();
    let params = &request["params"];
    let mut flow = Flow::Continue;
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "terraforme", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "O orquestrador do terra: lista os workspaces com o terra cli aberto e se há uma sessão ativa neles.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": [spaces_tool()] })),
        "tools/call" => match params["name"].as_str() {
            Some("spaces") => {
                let instances: Vec<Value> = state.alive().iter().map(Instance::to_json).collect();
                Ok(json!({
                    "content": [{ "type": "text", "text": Value::from(instances.clone()).to_string() }],
                    "structuredContent": { "instances": instances },
                }))
            }
            _ => Err((-32602, "ferramenta desconhecida")),
        },
        "terra/announce" => match Instance::from_json(params) {
            Some(instance) => {
                state
                    .instances
                    .insert(instance.pid, (instance, Instant::now()));
                Ok(json!({}))
            }
            None => Err((-32602, "terra/announce precisa de pid, name e dir")),
        },
        "terra/leave" => {
            if let Some(pid) = params["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) {
                state.instances.remove(&pid);
            }
            Ok(json!({}))
        }
        "terra/spaces" => {
            let instances: Vec<Value> = state.alive().iter().map(Instance::to_json).collect();
            Ok(json!({ "instances": instances }))
        }
        "terra/shutdown" => {
            flow = Flow::Stop;
            Ok(json!({}))
        }
        method if method.starts_with("notifications/") => Ok(Value::Null),
        _ => Err((-32601, "método desconhecido")),
    };
    let Some(id) = id else {
        return (None, flow);
    };
    let response = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error(id, code, message),
    };
    (Some(response), flow)
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A ferramenta `spaces`, para os agentes.
fn spaces_tool() -> Value {
    json!({
        "name": "spaces",
        "description": "Lista os workspaces com o terra cli aberto nesta máquina: o nome, a pasta, o processo e se há uma sessão ativa com o agente.",
        "inputSchema": { "type": "object", "properties": {} },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn announce(state: &mut State, pid: u32, name: &str, session: bool) {
        let me = Instance {
            pid,
            name: name.into(),
            dir: PathBuf::from(format!("/w/{name}")),
            session,
        };
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "terra/announce", "params": me.to_json()});
        assert_eq!(handle(&request, state).1, Flow::Continue);
    }

    #[test]
    fn announced_terra_show_up_until_they_leave_or_expire() {
        let mut state = State::default();
        announce(&mut state, 2, "b", true);
        announce(&mut state, 1, "a", false);
        announce(&mut state, 2, "b", false);
        let names: Vec<_> = state
            .alive()
            .into_iter()
            .map(|i| (i.name, i.session))
            .collect();
        assert_eq!(names, [("a".to_string(), false), ("b".to_string(), false)]);

        let leave =
            json!({"jsonrpc": "2.0", "id": 2, "method": "terra/leave", "params": {"pid": 1}});
        handle(&leave, &mut state);
        assert_eq!(state.alive().len(), 1);

        state.instances.get_mut(&2).unwrap().1 = Instant::now() - EXPIRE;
        assert!(state.alive().is_empty(), "parou de avisar: sai");
    }

    #[test]
    fn speaks_mcp_and_notifications_get_no_response() {
        let mut state = State::default();
        announce(&mut state, 7, "terra", true);
        let (init, _) = handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            &mut state,
        );
        let init = init.unwrap();
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(init["result"]["serverInfo"]["name"], "terraforme");

        let (none, _) = handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &mut state,
        );
        assert_eq!(none, None);

        let (tools, _) = handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &mut state,
        );
        assert_eq!(tools.unwrap()["result"]["tools"][0]["name"], "spaces");

        let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "spaces"}});
        let result = handle(&call, &mut state).0.unwrap();
        assert_eq!(
            result["result"]["structuredContent"]["instances"][0]["pid"],
            7
        );

        let (unknown, _) = handle(
            &json!({"jsonrpc": "2.0", "id": 4, "method": "???"}),
            &mut state,
        );
        assert_eq!(unknown.unwrap()["error"]["code"], -32601);

        let (_, flow) = handle(
            &json!({"jsonrpc": "2.0", "id": 5, "method": "terra/shutdown"}),
            &mut state,
        );
        assert_eq!(flow, Flow::Stop);
    }

    #[test]
    fn serves_over_http_until_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || serve(&listener));
        let me = Instance {
            pid: 9,
            name: "terra".into(),
            dir: PathBuf::from("/w/terra"),
            session: false,
        };
        super::super::call_at(addr, "terra/announce", me.to_json()).unwrap();
        let spaces = super::super::call_at(addr, "terra/spaces", json!({})).unwrap();
        assert_eq!(spaces["instances"][0]["name"], "terra");
        super::super::call_at(addr, "terra/shutdown", json!({})).unwrap();
        server.join().unwrap();
    }
}
