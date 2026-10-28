//! O servidor do MCP (`terra mcp`): JSON-RPC 2.0 por HTTP no endpoint
//! [`PATH`](super::PATH), como o transporte HTTP do MCP (respostas em
//! `application/json`).
//!
//! Métodos do MCP: `initialize`, `ping`, `tools/list` e `tools/call`. As
//! ferramentas:
//!
//! - `spaces`: os terra abertos;
//! - `chat`: delega uma tarefa a um chat novo num terra e retorna o id da
//!   delegação;
//! - `wait`: quem delegou espera a resposta do chat delegado ou um pedido de
//!   permissão dele;
//! - `decide`: quem delegou aprova ou nega um pedido de permissão;
//! - `permission`: o chat delegado pede permissão (é o
//!   `--permission-prompt-tool` dele) e espera a decisão de quem delegou.
//!
//! Métodos do terra: `terra/announce` (um terra se registra ou continua
//! aberto, manda os chats dele e recebe os chats pedidos para ele),
//! `terra/leave` (saiu), `terra/spaces` (os terra abertos) e
//! `terra/shutdown` (fecha o MCP). Um terra que para de avisar por
//! [`EXPIRE`] sai da lista.
//!
//! O agente de um terra acessa o MCP em `/mcp?from=<pid>` (veja
//! [`agent_url`](super::agent_url)) e o de um chat delegado, em
//! `/mcp?from=<pid>&chat=<id>` ([`delegated_url`](super::delegated_url)):
//! assim o MCP sabe de onde veio o pedido.
//!
//! As requisições curtas são atendidas uma de cada vez; as que esperam
//! (`wait` e `permission`) ganham uma thread, e as outras continuam sendo
//! atendidas enquanto isso.
//!
//! Antes de olhar o corpo, o MCP recusa ([`refusal`]) o que não veio de um
//! terra: sem o token ([`token`](super::token)) no `Authorization`, com um
//! `Host` ou um `Origin` de fora da máquina (um site no navegador, ou um
//! *DNS rebinding*) ou sem `Content-Type: application/json` (o navegador
//! manda `text/plain` sem perguntar ao servidor antes).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{ChatRequest, Instance, PATH, PERMISSION_TOOL};

/// Sem avisar por esse tempo, o terra sai da lista.
const EXPIRE: Duration = Duration::from_secs(6);
/// Quanto esperar um cliente mandar a requisição.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// O maior corpo aceito.
const MAX_BODY: usize = 1 << 20;
/// A versão do protocolo do MCP.
const PROTOCOL_VERSION: &str = "2025-06-18";
/// Sem decisão de quem delegou por esse tempo, o pedido de permissão é
/// negado.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Quanto o `wait` espera sem `timeout_seconds`, e o máximo.
const WAIT_DEFAULT: u64 = 120;
const WAIT_MAX: u64 = 600;
/// De quanto em quanto tempo o `wait` olha de novo (os terra avisam a cada
/// 2 s).
const WAIT_STEP: Duration = Duration::from_millis(500);
/// O erro de um chat delegado que chama uma ferramenta de quem delega.
const DELEGATED_ONLY: &str = "um chat delegado só usa a ferramenta permission do terra";

/// Abre o MCP na porta [`PORT`](super::PORT) e atende até o
/// `terra/shutdown`.
pub fn run() -> Result<()> {
    let dir = crate::db::data_dir()?;
    super::token_in(&dir)?;
    let listener = TcpListener::bind(super::addr()).with_context(|| {
        format!(
            "não foi possível abrir o MCP em {} (a porta já está em uso?)",
            super::addr()
        )
    })?;
    serve(&listener, &dir);
    Ok(())
}

/// Atende as conexões de `listener` até o `terra/shutdown`, com o token da
/// pasta `dir`.
pub fn serve(listener: &TcpListener, dir: &Path) {
    let shared = Arc::new(Shared {
        token_dir: dir.to_path_buf(),
        ..Shared::default()
    });
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // Um cliente com problema não derruba o MCP.
        if let Ok(Flow::Stop) = handle_connection(stream, &shared) {
            break;
        }
    }
}

/// O estado, compartilhado com as threads que esperam, e o aviso de que ele
/// mudou.
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// A pasta do token; ele é lido a cada requisição, e um token novo
    /// (o arquivo foi apagado) vale sem reabrir o MCP.
    token_dir: PathBuf,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// De onde veio um pedido: o terra e, num chat delegado, a delegação.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Caller {
    pid: Option<u32>,
    chat: Option<u64>,
}

/// Um chat delegado: quem delegou e o terra onde ele abre.
struct Delegation {
    parent: Caller,
    target: u32,
    /// O terra já recebeu o pedido (e abriu o chat).
    delivered: bool,
}

/// Um pedido de permissão de um chat delegado, até quem delegou decidir.
struct Permission {
    id: u64,
    /// A delegação que pediu.
    chat: u64,
    tool: String,
    input: Value,
    /// `Ok` aprova; `Err` nega, com o motivo.
    decision: Option<Result<(), String>>,
}

impl Permission {
    fn to_json(&self) -> Value {
        json!({
            "request_id": self.id,
            "chat_id": self.chat,
            "tool": self.tool,
            "input": self.input,
        })
    }
}

/// Os terra registrados, pelo processo, e quando avisaram pela última vez,
/// e as delegações.
#[derive(Default)]
struct State {
    instances: HashMap<u32, (Instance, Instant)>,
    /// Os chats pedidos para cada terra, até ele avisar de novo.
    chats: HashMap<u32, Vec<ChatRequest>>,
    /// O último id dado (às delegações e aos pedidos de permissão).
    last_id: u64,
    delegations: HashMap<u64, Delegation>,
    permissions: Vec<Permission>,
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

    fn next_id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }

    /// Guarda o pedido de um chat novo (`arguments` da ferramenta `chat`),
    /// do agente `from`, para o terra escolhido. Retorna o terra e o id da
    /// delegação.
    fn request_chat(&mut self, arguments: &Value, from: Caller) -> Result<(Instance, u64), String> {
        let alive = self.alive();
        let pid = arguments["pid"]
            .as_u64()
            .and_then(|p| u32::try_from(p).ok());
        let workspace = arguments["workspace"].as_str().filter(|w| !w.is_empty());
        let target = match (pid, workspace) {
            (Some(pid), _) => alive.iter().find(|i| i.pid == pid),
            (None, Some(name)) => alive.iter().find(|i| i.name == name),
            (None, None) => from
                .pid
                .and_then(|from| alive.iter().find(|i| i.pid == from)),
        }
        .cloned()
        .ok_or("nenhum terra aberto com esse pid ou workspace (veja a ferramenta spaces)")?;
        let mut request = ChatRequest::from_json(arguments).ok_or("o chat precisa de message")?;
        request.id = self.next_id();
        request.from = from
            .pid
            .and_then(|from| alive.iter().find(|i| i.pid == from))
            .map(|i| i.name.clone());
        self.delegations.insert(
            request.id,
            Delegation {
                parent: from,
                target: target.pid,
                delivered: false,
            },
        );
        let id = request.id;
        self.chats.entry(target.pid).or_default().push(request);
        Ok((target, id))
    }

    /// Os pedidos de permissão, ainda sem decisão, dos chats que `caller`
    /// delegou.
    fn pending_for(&self, caller: Caller) -> Vec<Value> {
        self.permissions
            .iter()
            .filter(|p| p.decision.is_none())
            .filter(|p| {
                self.delegations
                    .get(&p.chat)
                    .is_some_and(|d| d.parent == caller)
            })
            .map(Permission::to_json)
            .collect()
    }

    /// Como está a delegação `id`: a resposta, quando o chat terminou;
    /// `None` enquanto ele trabalha.
    fn delegation_result(&mut self, id: u64) -> Option<Value> {
        let alive = self.alive();
        let Some(delegation) = self.delegations.get(&id) else {
            return Some(json!({ "status": "unknown", "chat_id": id }));
        };
        let Some(instance) = alive.iter().find(|i| i.pid == delegation.target) else {
            return Some(json!({ "status": "closed", "chat_id": id }));
        };
        match instance.chats.iter().find(|c| c.id == Some(id)) {
            Some(chat) if !chat.busy && chat.reply.is_some() => Some(json!({
                "status": "done",
                "chat_id": id,
                "reply": chat.reply,
                "tokens": chat.tokens,
                "cache_tokens": chat.cache_tokens,
            })),
            Some(_) => None,
            // Entregue e fora da lista: a aba foi fechada.
            None if delegation.delivered => Some(json!({ "status": "closed", "chat_id": id })),
            None => None,
        }
    }

    /// A decisão de `caller` (`arguments` da ferramenta `decide`).
    fn decide(&mut self, arguments: &Value, caller: Caller) -> Result<String, String> {
        let id = arguments["request_id"]
            .as_u64()
            .ok_or("decide precisa de request_id")?;
        let allow = arguments["allow"]
            .as_bool()
            .ok_or("decide precisa de allow")?;
        let delegations = &self.delegations;
        let permission = self
            .permissions
            .iter_mut()
            .find(|p| p.id == id && p.decision.is_none())
            .ok_or("nenhum pedido pendente com esse request_id (veja a ferramenta wait)")?;
        if delegations
            .get(&permission.chat)
            .is_none_or(|d| d.parent != caller)
        {
            return Err("esse pedido é de um chat que você não delegou".to_string());
        }
        permission.decision = Some(if allow {
            Ok(())
        } else {
            Err(arguments["message"]
                .as_str()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or("negado pelo agente principal")
                .to_string())
        });
        Ok(format!(
            "Pedido {id} ({}) {}.",
            permission.tool,
            if allow { "aprovado" } else { "negado" }
        ))
    }
}

/// Se o MCP continua atendendo depois de uma requisição.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

fn handle_connection(stream: TcpStream, shared: &Arc<Shared>) -> Result<Flow> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
            .map(str::to_string)
    };
    let caller = Caller {
        pid: param("from").and_then(|pid| pid.parse().ok()),
        chat: param("chat").and_then(|chat| chat.parse().ok()),
    };
    let headers = parse_headers(&mut reader)?;
    if let Some((status, why)) = refusal(&headers, &shared.token_dir) {
        respond(
            &mut writer,
            status,
            &error(Value::Null, -32001, why).to_string(),
        )?;
        return Ok(Flow::Continue);
    }
    let length = headers.length;
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
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        let response = error(Value::Null, -32700, "JSON inválido");
        respond(&mut writer, "200 OK", &response.to_string())?;
        return Ok(Flow::Continue);
    };
    if waits(&request) {
        // Espera numa thread: o MCP continua atendendo os outros.
        let shared = Arc::clone(shared);
        thread::spawn(move || {
            let response = handle_waiting(&request, &shared, caller);
            let _ = respond(&mut writer, "200 OK", &response.to_string());
        });
        return Ok(Flow::Continue);
    }
    let (response, flow) = handle_as(&request, &mut shared.lock(), caller);
    shared.changed.notify_all();
    match response {
        Some(response) => respond(&mut writer, "200 OK", &response.to_string())?,
        // Uma notificação não tem resposta.
        None => respond(&mut writer, "202 Accepted", "")?,
    }
    Ok(flow)
}

/// A requisição é uma ferramenta que espera (`wait` ou `permission`).
fn waits(request: &Value) -> bool {
    request["method"] == "tools/call"
        && request.get("id").is_some()
        && matches!(
            request["params"]["name"].as_str(),
            Some("wait") | Some(PERMISSION_TOOL)
        )
}

/// Os cabeçalhos de uma requisição que o MCP olha.
#[derive(Debug, Default)]
struct Headers {
    /// O `Content-Length` (0 sem ele).
    length: usize,
    authorization: Option<String>,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
}

/// Lê os cabeçalhos até a linha vazia e retorna o `Content-Length` (0 sem
/// ele).
pub(super) fn read_headers(reader: &mut impl BufRead) -> Result<usize> {
    Ok(parse_headers(reader)?.length)
}

fn parse_headers(reader: &mut impl BufRead) -> Result<Headers> {
    let mut headers = Headers::default();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("a conexão fechou no meio dos cabeçalhos");
        }
        let line = line.trim_end();
        if line.is_empty() {
            return Ok(headers);
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                headers.length = value.parse().context("Content-Length inválido")?
            }
            "authorization" => headers.authorization = Some(value.to_string()),
            "host" => headers.host = Some(value.to_string()),
            "origin" => headers.origin = Some(value.to_string()),
            "content-type" => headers.content_type = Some(value.to_string()),
            _ => {}
        }
    }
}

/// Por que o MCP recusa a requisição (o status e o motivo), ou `None` se ela
/// pode seguir: com o token do terra no `Authorization`, um `Host` e um
/// `Origin` da própria máquina e `Content-Type: application/json`.
fn refusal(headers: &Headers, token_dir: &Path) -> Option<(&'static str, &'static str)> {
    let token = super::token_in(token_dir).ok();
    let sent = headers
        .authorization
        .as_deref()
        .and_then(|value| value.strip_prefix("Bearer "));
    match (token, sent) {
        (Some(token), Some(sent)) if same(&token, sent.trim()) => {}
        _ => return Some(("401 Unauthorized", "token do MCP ausente ou inválido")),
    }
    if !headers.host.as_deref().is_some_and(is_local_host) {
        return Some(("403 Forbidden", "Host de fora da máquina"));
    }
    if let Some(origin) = &headers.origin {
        let host = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"));
        if !host.is_some_and(is_local_host) {
            return Some(("403 Forbidden", "Origin de fora da máquina"));
        }
    }
    let json = headers.content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json {
        return Some((
            "415 Unsupported Media Type",
            "o Content-Type deve ser application/json",
        ));
    }
    None
}

/// `host` (com a porta, se tiver) é a própria máquina.
fn is_local_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// Compara sem parar no primeiro byte diferente.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
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

/// O resultado de uma ferramenta com o texto `text`.
fn tool_text(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

/// O resultado de uma ferramenta com `value` em texto e estruturado.
fn tool_json(value: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "structuredContent": value,
    })
}

fn tool_error(message: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": message.into() }], "isError": true })
}

/// Responde a uma requisição JSON-RPC, vinda do agente do terra `from`
/// (quando se sabe); `None` nas notificações (sem `id`).
#[cfg(test)]
fn handle(request: &Value, state: &mut State, from: Option<u32>) -> (Option<Value>, Flow) {
    handle_as(
        request,
        state,
        Caller {
            pid: from,
            chat: None,
        },
    )
}

/// Responde a uma requisição JSON-RPC que não espera, vinda de `caller`;
/// `None` nas notificações (sem `id`).
fn handle_as(request: &Value, state: &mut State, caller: Caller) -> (Option<Value>, Flow) {
    let id = request.get("id").cloned();
    let params = &request["params"];
    let mut flow = Flow::Continue;
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "terraforme", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "O orquestrador do terra: lista os workspaces com o terra cli aberto e se há uma sessão ativa neles, e delega tarefas a chats novos neles. Quem delega espera a resposta com wait e aprova ou nega, com decide, os pedidos de permissão dos chats delegados.",
        })),
        "ping" => Ok(json!({})),
        // O chat delegado só vê a `permission` (o `--permission-prompt-tool`
        // dele): as outras ocupariam o prompt de cada chamada dele e o
        // deixariam delegar de novo.
        "tools/list" if caller.chat.is_some() => Ok(json!({ "tools": [permission_tool()] })),
        "tools/list" => Ok(json!({
            "tools": [spaces_tool(), chat_tool(), wait_tool(), decide_tool(), permission_tool()]
        })),
        "tools/call" if caller.chat.is_some() => Ok(tool_error(DELEGATED_ONLY)),
        "tools/call" => {
            let arguments = &params["arguments"];
            match params["name"].as_str() {
                Some("spaces") => {
                    let instances: Vec<Value> =
                        state.alive().iter().map(Instance::to_json).collect();
                    Ok(tool_json(json!({ "instances": instances })))
                }
                Some("chat") => Ok(match state.request_chat(arguments, caller) {
                    Ok((target, chat)) => {
                        let mut result = tool_json(json!({
                            "chat_id": chat,
                            "terra": target.name,
                            "pid": target.pid,
                        }));
                        result["content"][0]["text"] = Value::from(format!(
                            "Tarefa delegada ao chat {chat}, no terra {} ({}, pid {}). Chame wait com chat_id {chat} para receber a resposta e os pedidos de permissão dele.",
                            target.name,
                            target.dir.display(),
                            target.pid
                        ));
                        result
                    }
                    Err(message) => tool_error(message),
                }),
                Some("decide") => Ok(match state.decide(arguments, caller) {
                    Ok(message) => tool_text(message),
                    Err(message) => tool_error(message),
                }),
                _ => Err((-32602, "ferramenta desconhecida")),
            }
        }
        "terra/announce" => match Instance::from_json(params) {
            Some(instance) => {
                let delivered = state.chats.remove(&instance.pid).unwrap_or_default();
                for request in &delivered {
                    if let Some(delegation) = state.delegations.get_mut(&request.id) {
                        delegation.delivered = true;
                    }
                }
                let chats: Vec<Value> = delivered.iter().map(ChatRequest::to_json).collect();
                state
                    .instances
                    .insert(instance.pid, (instance, Instant::now()));
                Ok(json!({ "chats": chats }))
            }
            None => Err((-32602, "terra/announce precisa de pid, name e dir")),
        },
        "terra/leave" => {
            if let Some(pid) = params["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) {
                state.instances.remove(&pid);
                state.chats.remove(&pid);
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

/// Responde a uma ferramenta que espera ([`waits`]), vinda de `caller`.
fn handle_waiting(request: &Value, shared: &Shared, caller: Caller) -> Value {
    let id = request["id"].clone();
    let arguments = &request["params"]["arguments"];
    let result = match request["params"]["name"].as_str() {
        Some("wait") if caller.chat.is_some() => tool_error(DELEGATED_ONLY),
        Some("wait") => wait(shared, arguments, caller),
        _ => permission(shared, arguments, caller),
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A ferramenta `wait`: espera, até `timeout_seconds`, um pedido de
/// permissão dos chats que `caller` delegou ou a resposta do `chat_id`.
fn wait(shared: &Shared, arguments: &Value, caller: Caller) -> Value {
    let chat = arguments["chat_id"].as_u64();
    let seconds = arguments["timeout_seconds"]
        .as_u64()
        .unwrap_or(WAIT_DEFAULT)
        .min(WAIT_MAX);
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut state = shared.lock();
    loop {
        let requests = state.pending_for(caller);
        if !requests.is_empty() {
            return tool_json(json!({ "status": "permission", "requests": requests }));
        }
        if let Some(result) = chat.and_then(|chat| state.delegation_result(chat)) {
            return tool_json(result);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return tool_json(json!({ "status": "running", "chat_id": chat }));
        }
        state = shared
            .changed
            .wait_timeout(state, left.min(WAIT_STEP))
            .map_or_else(|poisoned| poisoned.into_inner().0, |(state, _)| state);
    }
}

/// A ferramenta `permission`: o chat delegado `caller` pede permissão para
/// usar uma ferramenta e espera quem delegou decidir (no formato do
/// `--permission-prompt-tool` do Claude).
fn permission(shared: &Shared, arguments: &Value, caller: Caller) -> Value {
    let input = arguments["input"].clone();
    let deny =
        |message: &str| tool_text(json!({ "behavior": "deny", "message": message }).to_string());
    let Some(chat) = caller.chat else {
        return deny("só um chat delegado pede permissão pelo terra");
    };
    let mut state = shared.lock();
    if !state.delegations.contains_key(&chat) {
        return deny("a delegação deste chat não existe mais no MCP");
    }
    let id = state.next_id();
    state.permissions.push(Permission {
        id,
        chat,
        tool: arguments["tool_name"].as_str().unwrap_or("?").to_string(),
        input: input.clone(),
        decision: None,
    });
    shared.changed.notify_all();
    let deadline = Instant::now() + PERMISSION_TIMEOUT;
    let decision = loop {
        let index = state.permissions.iter().position(|p| p.id == id);
        if let Some(decision) = index.and_then(|i| state.permissions[i].decision.clone()) {
            break decision;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Err("o agente principal não decidiu a tempo".to_string());
        }
        state = shared
            .changed
            .wait_timeout(state, left.min(WAIT_STEP))
            .map_or_else(|poisoned| poisoned.into_inner().0, |(state, _)| state);
    };
    state.permissions.retain(|p| p.id != id);
    match decision {
        Ok(()) => tool_text(json!({ "behavior": "allow", "updatedInput": input }).to_string()),
        Err(message) => deny(&message),
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A ferramenta `spaces`, para os agentes.
fn spaces_tool() -> Value {
    json!({
        "name": "spaces",
        "description": "Lista os workspaces com o terra cli aberto nesta máquina: o nome, a pasta, o processo e se há uma sessão ativa com o agente, e os chats abertos em cada um com o consumo: tokens (entrada e saída), cache_tokens (criação e leitura do cache), se ainda está respondendo (busy) e, nos chats delegados, o id da delegação e a última resposta (reply).",
        "inputSchema": { "type": "object", "properties": {} },
    })
}

/// A ferramenta `chat`, para um agente delegar uma tarefa a outro chat.
fn chat_tool() -> Value {
    let tools = super::DELEGATED_TOOLS.join(", ");
    let memory = super::DELEGATED_MCP;
    json!({
        "name": "chat",
        "description": format!("Delega uma tarefa a um agente novo: abre um chat num terra aberto (por padrão, no seu) com a tarefa como primeira mensagem e retorna o chat_id da delegação. O chat aparece numa aba embaixo da barra de abas do terra. Você é o agente principal dele: chame wait com o chat_id para receber a resposta final e os pedidos de permissão dele, que você aprova ou nega com decide. Escolha o terra pelo pid ou pelo nome do workspace (veja a ferramenta spaces). O agente delegado roda enxuto, para gastar menos: só com as ferramentas {tools} e o MCP {memory}, sem os outros MCP, skills e hooks do usuário. Passe em mcp e tools o que mais a tarefa precisar; se ele terminar pedindo uma ferramenta, delegue de novo com ela e com o que ele já fez."),
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "A tarefa: a primeira mensagem do chat novo, com o contexto que o agente delegado precisa." },
                "title": { "type": "string", "description": "O título da aba do chat." },
                "pid": { "type": "integer", "description": "O processo do terra onde abrir o chat." },
                "workspace": { "type": "string", "description": "O nome do workspace onde abrir o chat (sem o pid)." },
                "mcp": { "type": "array", "items": { "type": "string" }, "description": format!("Os servidores MCP do usuário, pelo nome (ex.: git, postman), que o agente delegado precisa além do {memory}.") },
                "tools": { "type": "array", "items": { "type": "string" }, "description": format!("As ferramentas embutidas que o agente delegado precisa além de {tools} (ex.: WebFetch, WebSearch).") },
            },
            "required": ["message"],
        },
    })
}

/// A ferramenta `wait`, para quem delegou.
fn wait_tool() -> Value {
    json!({
        "name": "wait",
        "description": "Espera um chat que você delegou (ferramenta chat). Retorna assim que: um chat delegado por você pedir permissão (status permission, com os requests: request_id, chat_id, tool e input; decida cada um com decide e chame wait de novo); o chat_id terminar (status done, com a resposta final em reply); ou o tempo acabar (status running: chame de novo). status closed: a aba do chat foi fechada.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "chat_id": { "type": "integer", "description": "A delegação a esperar (sem ela, só espera pedidos de permissão)." },
                "timeout_seconds": { "type": "integer", "description": "Quanto esperar, em segundos (padrão 120, máximo 600)." },
            },
        },
    })
}

/// A ferramenta `decide`, para quem delegou.
fn decide_tool() -> Value {
    json!({
        "name": "decide",
        "description": "Aprova ou nega um pedido de permissão de um chat que você delegou (os requests do wait). Ao negar, diga em message o porquê ou o que fazer no lugar: o agente delegado recebe a mensagem.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "request_id": { "type": "integer", "description": "O request_id do pedido." },
                "allow": { "type": "boolean", "description": "true aprova; false nega." },
                "message": { "type": "string", "description": "Ao negar, o motivo ou o que fazer no lugar." },
            },
            "required": ["request_id", "allow"],
        },
    })
}

/// A ferramenta `permission`, o `--permission-prompt-tool` dos chats
/// delegados.
fn permission_tool() -> Value {
    json!({
        "name": PERMISSION_TOOL,
        "description": "Uso interno do terra: pede ao agente principal permissão para um chat delegado usar uma ferramenta, e espera a decisão.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" },
            },
            "required": ["tool_name", "input"],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::mcp::ChatStats;

    fn instance(pid: u32, name: &str, chats: Vec<ChatStats>) -> Instance {
        Instance {
            pid,
            name: name.into(),
            dir: PathBuf::from(format!("/w/{name}")),
            session: false,
            chats,
        }
    }

    fn announce(state: &mut State, pid: u32, name: &str, session: bool) {
        let me = Instance {
            session,
            ..instance(pid, name, Vec::new())
        };
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "terra/announce", "params": me.to_json()});
        assert_eq!(handle(&request, state, None).1, Flow::Continue);
    }

    fn call(name: &str, args: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
    }

    fn headers(token: &str) -> Headers {
        Headers {
            length: 2,
            authorization: Some(format!("Bearer {token}")),
            host: Some("127.0.0.1:30303".into()),
            origin: None,
            content_type: Some("application/json".into()),
        }
    }

    #[test]
    fn refuses_what_did_not_come_from_a_terra() {
        let dir = tempfile::tempdir().unwrap();
        let token = super::super::token_in(dir.path()).unwrap();
        let refused = |h: &Headers| refusal(h, dir.path()).map(|(status, _)| status);
        assert_eq!(refused(&headers(&token)), None);

        let mut h = headers("outro-token");
        assert_eq!(refused(&h), Some("401 Unauthorized"));
        h.authorization = None;
        assert_eq!(refused(&h), Some("401 Unauthorized"));

        let mut h = headers(&token);
        h.host = Some("evil.example:30303".into());
        assert_eq!(refused(&h), Some("403 Forbidden"));
        h.host = Some("localhost:30303".into());
        assert_eq!(refused(&h), None);
        h.host = Some("[::1]:30303".into());
        assert_eq!(refused(&h), None);

        h.origin = Some("https://evil.example".into());
        assert_eq!(refused(&h), Some("403 Forbidden"));
        h.origin = Some("http://localhost:3000".into());
        assert_eq!(refused(&h), None);

        h.content_type = Some("text/plain".into());
        assert_eq!(refused(&h), Some("415 Unsupported Media Type"));
        h.content_type = Some("Application/JSON; charset=utf-8".into());
        assert_eq!(refused(&h), None);
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
        handle(&leave, &mut state, None);
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
            None,
        );
        let init = init.unwrap();
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(init["result"]["serverInfo"]["name"], "terraforme");

        let (none, _) = handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &mut state,
            None,
        );
        assert_eq!(none, None);

        let (tools, _) = handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &mut state,
            None,
        );
        let tools = tools.unwrap();
        let names: Vec<_> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["spaces", "chat", "wait", "decide", "permission"]);

        let result = handle(&call("spaces", json!({})), &mut state, None)
            .0
            .unwrap();
        assert_eq!(
            result["result"]["structuredContent"]["instances"][0]["pid"],
            7
        );

        let (unknown, _) = handle(
            &json!({"jsonrpc": "2.0", "id": 4, "method": "???"}),
            &mut state,
            None,
        );
        assert_eq!(unknown.unwrap()["error"]["code"], -32601);

        let (_, flow) = handle(
            &json!({"jsonrpc": "2.0", "id": 5, "method": "terra/shutdown"}),
            &mut state,
            None,
        );
        assert_eq!(flow, Flow::Stop);
    }

    #[test]
    fn chat_requests_go_to_the_target_on_its_next_announce() {
        let mut state = State::default();
        announce(&mut state, 1, "a", true);
        announce(&mut state, 2, "b", false);
        let to_b = handle(
            &call(
                "chat",
                json!({"workspace": "b", "message": "oi", "title": "B"}),
            ),
            &mut state,
            Some(1),
        );
        let to_b = to_b.0.unwrap();
        assert!(to_b["result"]["isError"].is_null());
        assert_eq!(to_b["result"]["structuredContent"]["chat_id"], 1);
        let mine = handle(&call("chat", json!({"message": "eu"})), &mut state, Some(1))
            .0
            .unwrap();
        assert!(
            mine["result"]["isError"].is_null(),
            "sem pid: no terra que pediu"
        );
        let lost = handle(
            &call("chat", json!({"pid": 9, "message": "x"})),
            &mut state,
            Some(1),
        )
        .0
        .unwrap();
        assert_eq!(lost["result"]["isError"], true);

        let me = instance(2, "b", Vec::new());
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "terra/announce", "params": me.to_json()});
        let answer = handle(&request, &mut state, None).0.unwrap();
        let chats = &answer["result"]["chats"];
        assert_eq!(chats[0]["id"], 1);
        assert_eq!(chats[0]["message"], "oi");
        assert_eq!(chats[0]["from"], "a");
        assert_eq!(chats[0]["title"], "B");
        assert!(state.delegations[&1].delivered);
        let again = handle(&request, &mut state, None).0.unwrap();
        assert_eq!(again["result"]["chats"], json!([]), "entregue uma vez só");
        assert_eq!(state.chats[&1].len(), 1);
    }

    /// O chat delegado só vê a `permission` e não usa as ferramentas de quem
    /// delega (não delega de novo nem espera outros chats).
    #[test]
    fn a_delegated_chat_only_sees_permission() {
        let delegated = Caller {
            pid: Some(1),
            chat: Some(1),
        };
        let mut state = State::default();
        let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let tools = handle_as(&list, &mut state, delegated).0.unwrap();
        let names: Vec<_> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["permission"]);

        for name in ["spaces", "chat", "decide"] {
            let args = json!({"message": "de novo", "request_id": 1, "allow": true});
            let result = handle_as(&call(name, args), &mut state, delegated)
                .0
                .unwrap();
            assert_eq!(result["result"]["isError"], true, "{name}");
        }
        assert!(state.chats.is_empty(), "não delegou de novo");

        let shared = Shared::default();
        let waited = handle_waiting(&call("wait", json!({})), &shared, delegated);
        assert_eq!(waited["result"]["isError"], true);
    }

    /// O principal (terra 1) delega ao terra 2; o chat delegado pede
    /// permissão, o principal vê o pedido no `wait`, aprova, e o `wait`
    /// devolve a resposta final.
    #[test]
    fn the_main_agent_approves_requests_and_gets_the_reply() {
        let shared = Arc::new(Shared::default());
        let main = Caller {
            pid: Some(1),
            chat: None,
        };
        {
            let mut state = shared.lock();
            announce(&mut state, 1, "a", true);
            announce(&mut state, 2, "b", false);
            handle_as(
                &call("chat", json!({"pid": 2, "message": "faça"})),
                &mut state,
                main,
            );
        }
        let delegated = Caller {
            pid: Some(2),
            chat: Some(1),
        };
        let asking = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let args = json!({"tool_name": "Bash", "input": {"command": "cargo test"}});
                handle_waiting(&call(PERMISSION_TOOL, args), &shared, delegated)
            })
        };

        let waited = handle_waiting(&call("wait", json!({"chat_id": 1})), &shared, main);
        let found = &waited["result"]["structuredContent"];
        assert_eq!(found["status"], "permission");
        let request = &found["requests"][0];
        assert_eq!(request["tool"], "Bash");
        assert_eq!(request["chat_id"], 1);

        // Outro terra não decide pelo principal.
        let other = Caller {
            pid: Some(2),
            chat: None,
        };
        let args = json!({"request_id": request["request_id"], "allow": true});
        let stolen = handle_as(&call("decide", args.clone()), &mut shared.lock(), other);
        assert_eq!(stolen.0.unwrap()["result"]["isError"], true);
        handle_as(&call("decide", args), &mut shared.lock(), main);
        shared.changed.notify_all();

        let answer = asking.join().unwrap();
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        let decision: Value = serde_json::from_str(text).unwrap();
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedInput"]["command"], "cargo test");

        let done = ChatStats {
            title: "B".into(),
            id: Some(1),
            reply: Some("pronto".into()),
            ..ChatStats::default()
        };
        announce_with(&mut shared.lock(), instance(2, "b", vec![done]));
        let waited = handle_waiting(&call("wait", json!({"chat_id": 1})), &shared, main);
        let found = &waited["result"]["structuredContent"];
        assert_eq!(found["status"], "done");
        assert_eq!(found["reply"], "pronto");
    }

    #[test]
    fn denied_requests_carry_the_reason_and_strangers_are_denied() {
        let shared = Arc::new(Shared::default());
        let main = Caller {
            pid: Some(1),
            chat: None,
        };
        {
            let mut state = shared.lock();
            announce(&mut state, 1, "a", true);
            handle_as(&call("chat", json!({"message": "faça"})), &mut state, main);
        }
        let asking = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let delegated = Caller {
                    pid: Some(1),
                    chat: Some(1),
                };
                let args = json!({"tool_name": "Bash", "input": {"command": "rm -rf /"}});
                handle_waiting(&call(PERMISSION_TOOL, args), &shared, delegated)
            })
        };
        let waited = handle_waiting(&call("wait", json!({})), &shared, main);
        let id = waited["result"]["structuredContent"]["requests"][0]["request_id"].clone();
        let args = json!({"request_id": id, "allow": false, "message": "não apague nada"});
        handle_as(&call("decide", args), &mut shared.lock(), main);
        shared.changed.notify_all();
        let answer = asking.join().unwrap();
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"deny\"") && text.contains("não apague nada"));

        // Quem não é um chat delegado não pede permissão pelo terra.
        let stranger = handle_waiting(
            &call(PERMISSION_TOOL, json!({"tool_name": "Bash", "input": {}})),
            &shared,
            main,
        );
        let text = stranger["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"deny\""));
    }

    #[test]
    fn wait_times_out_while_the_chat_works() {
        let shared = Shared::default();
        let main = Caller {
            pid: Some(1),
            chat: None,
        };
        {
            let mut state = shared.lock();
            announce(&mut state, 1, "a", true);
            handle_as(&call("chat", json!({"message": "faça"})), &mut state, main);
        }
        let waited = handle_waiting(
            &call("wait", json!({"chat_id": 1, "timeout_seconds": 0})),
            &shared,
            main,
        );
        assert_eq!(waited["result"]["structuredContent"]["status"], "running");
    }

    fn announce_with(state: &mut State, me: Instance) {
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "terra/announce", "params": me.to_json()});
        handle(&request, state, None);
    }

    #[test]
    fn serves_over_http_until_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let token = super::super::token_in(dir.path()).unwrap();
        let path = dir.path().to_path_buf();
        let server = std::thread::spawn(move || serve(&listener, &path));
        let me = instance(9, "terra", Vec::new());
        super::super::call_at(addr, &token, "terra/announce", me.to_json()).unwrap();
        let spaces = super::super::call_at(addr, &token, "terra/spaces", json!({})).unwrap();
        assert_eq!(spaces["instances"][0]["name"], "terra");
        super::super::call_at(addr, &token, "terra/shutdown", json!({})).unwrap();
        server.join().unwrap();
    }
}
