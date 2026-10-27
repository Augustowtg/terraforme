//! O MCP do terra: um servidor MCP (JSON-RPC por HTTP) em
//! `http://127.0.0.1:30303/mcp`, um só para a máquina, que orquestra os
//! terra abertos.
//!
//! Ele roda num processo próprio (`terra mcp`, veja [`server`]). Com o MCP
//! ativado na Config, o `terra cli` verifica ao iniciar se já há um MCP na
//! porta [`PORT`] ([`is_online`]) e, se não houver, abre um ([`ensure_running`]).
//! Depois, cada terra se registra nele e manda, a cada [`HEARTBEAT`], o
//! workspace e se há uma sessão ativa ([`Link`]); o MCP lista os terra
//! abertos ([`Instance`]) para a aba MCP e para os agentes (a ferramenta
//! `spaces`).

pub mod server;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// A porta do MCP.
pub const PORT: u16 = 30303;
/// O caminho do endpoint JSON-RPC.
pub const PATH: &str = "/mcp";
/// De quanto em quanto tempo cada terra avisa o MCP que continua aberto.
const HEARTBEAT: Duration = Duration::from_secs(2);
/// Quanto esperar o MCP responder.
const TIMEOUT: Duration = Duration::from_millis(500);

/// O endereço do MCP, só na máquina local.
pub fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], PORT))
}

/// O endereço do endpoint, para mostrar.
pub fn url() -> String {
    format!("http://{}{PATH}", addr())
}

/// Um terra aberto, como ele se registra no MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// O processo do terra.
    pub pid: u32,
    /// O nome do workspace (o salvo ou o da pasta).
    pub name: String,
    pub dir: PathBuf,
    /// Há uma conversa com o agente (no chat do terra ou na interface
    /// original do provedor).
    pub session: bool,
}

impl Instance {
    fn to_json(&self) -> Value {
        json!({
            "pid": self.pid,
            "name": self.name,
            "dir": self.dir.to_string_lossy(),
            "session": self.session,
        })
    }

    fn from_json(json: &Value) -> Option<Self> {
        Some(Self {
            pid: u32::try_from(json["pid"].as_u64()?).ok()?,
            name: json["name"].as_str()?.to_string(),
            dir: PathBuf::from(json["dir"].as_str()?),
            session: json["session"].as_bool().unwrap_or(false),
        })
    }
}

/// Se há um MCP do terra respondendo na porta [`PORT`].
pub fn is_online() -> bool {
    call("ping", json!({})).is_ok()
}

/// Abre o MCP num processo novo (`terra mcp`), a não ser que já haja um na
/// porta. Retorna `true` se abriu.
pub fn ensure_running() -> Result<bool> {
    if is_online() {
        return Ok(false);
    }
    let exe = std::env::current_exe().context("não foi possível achar o executável do terra")?;
    let mut command = Command::new(exe);
    command
        .arg("mcp")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    command
        .spawn()
        .context("não foi possível abrir o MCP do terra")?;
    Ok(true)
}

/// O MCP continua rodando depois que o terra que o abriu sai, e o `Ctrl+C`
/// no terminal não chega nele.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

/// Pede ao MCP para fechar (o MCP desativado na Config).
pub fn shutdown() -> Result<()> {
    call("terra/shutdown", json!({})).map(drop)
}

/// Os terra abertos, segundo o MCP.
pub fn instances() -> Result<Vec<Instance>> {
    let result = call("terra/spaces", json!({}))?;
    Ok(result["instances"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Instance::from_json)
        .collect())
}

/// Chama o método `method` do MCP e retorna o `result`.
fn call(method: &str, params: Value) -> Result<Value> {
    call_at(addr(), method, params)
}

/// Chama o método `method` do servidor em `addr`.
fn call_at(addr: SocketAddr, method: &str, params: Value) -> Result<Value> {
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let body = post(addr, &request.to_string())?;
    let response: Value =
        serde_json::from_str(&body).context("o MCP respondeu algo que não é JSON")?;
    if let Some(error) = response.get("error") {
        bail!("o MCP respondeu com erro: {}", error["message"]);
    }
    Ok(response["result"].clone())
}

/// Um `POST` no endpoint de `addr`, com o corpo `body`; retorna o corpo da
/// resposta.
fn post(addr: SocketAddr, body: &str) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    write!(
        stream,
        "POST {PATH} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        addr,
        body.len()
    )?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    if !status
        .split_whitespace()
        .nth(1)
        .is_some_and(|c| c.starts_with('2'))
    {
        bail!("o MCP respondeu {}", status.trim());
    }
    let length = server::read_headers(&mut reader)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// O que o [`Link`] compartilha com a thread dele.
struct Shared {
    me: Mutex<Instance>,
    /// Os terra abertos na última vez que o MCP respondeu; `None` quando ele
    /// não respondeu (offline).
    instances: Mutex<Option<Vec<Instance>>>,
    stop: AtomicBool,
}

/// A ligação deste terra com o MCP: numa thread, registra o terra a cada
/// [`HEARTBEAT`] e lê os terra abertos. Ao sair (no `Drop`), sai do MCP.
pub struct Link {
    shared: Arc<Shared>,
}

impl Link {
    pub fn start(me: Instance) -> Self {
        let shared = Arc::new(Shared {
            me: Mutex::new(me),
            instances: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        thread::spawn(move || heartbeat(&thread_shared));
        Self { shared }
    }

    /// Avisa, na próxima batida, se este terra tem uma sessão ativa.
    pub fn set_session(&self, session: bool) {
        if let Ok(mut me) = self.shared.me.lock() {
            me.session = session;
        }
    }

    /// Os terra abertos; `None` com o MCP offline.
    pub fn instances(&self) -> Option<Vec<Instance>> {
        self.shared.instances.lock().ok()?.clone()
    }

    pub fn is_online(&self) -> bool {
        self.shared
            .instances
            .lock()
            .is_ok_and(|instances| instances.is_some())
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Ok(me) = self.shared.me.lock() {
            let _ = call("terra/leave", json!({ "pid": me.pid }));
        }
    }
}

fn heartbeat(shared: &Shared) {
    const STEP: Duration = Duration::from_millis(100);
    while !shared.stop.load(Ordering::Relaxed) {
        let me = shared.me.lock().map(|me| me.to_json()).ok();
        let seen = me
            .map_or(Ok(Value::Null), |me| call("terra/announce", me))
            .and_then(|_| instances())
            .ok();
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(mut instances) = shared.instances.lock() {
            *instances = seen;
        }
        let mut waited = Duration::ZERO;
        while waited < HEARTBEAT && !shared.stop.load(Ordering::Relaxed) {
            thread::sleep(STEP);
            waited += STEP;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_round_trips_through_json() {
        let instance = Instance {
            pid: 42,
            name: "terra".into(),
            dir: PathBuf::from("/home/eu/terra"),
            session: true,
        };
        assert_eq!(Instance::from_json(&instance.to_json()), Some(instance));
        assert_eq!(Instance::from_json(&json!({"pid": 1})), None);
    }
}
