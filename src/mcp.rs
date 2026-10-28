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
//!
//! Um agente também pode delegar uma tarefa a um chat novo num terra (a
//! ferramenta `chat`): o MCP guarda o pedido ([`ChatRequest`]) e o entrega ao
//! terra na próxima batida, e o terra abre o chat numa aba embaixo da barra
//! de abas. O agente que delegou espera a resposta (a ferramenta `wait`), e
//! os pedidos de permissão do chat delegado chegam a ele para aprovar ou
//! negar (as ferramentas `wait` e `decide`).
//!
//! Só quem tem o token ([`token`], em `~/.terraforme/mcp.token`) usa o MCP:
//! os terra e os agentes deles. Um site aberto no navegador ou outro usuário
//! da máquina não consegue abrir chats nem aprovar pedidos de permissão.

pub mod server;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
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

/// As ferramentas embutidas de um chat delegado, que roda enxuto: quem
/// delega acrescenta outras no `tools` da ferramenta `chat`.
pub const DELEGATED_TOOLS: &[&str] = &["Bash", "Read", "Edit", "Write", "Glob", "Grep"];
/// O servidor MCP do usuário que todo chat delegado leva (a memória entre
/// sessões); quem delega acrescenta outros no `mcp` da ferramenta `chat`.
pub const DELEGATED_MCP: &str = "ai-memory";

/// O arquivo do token, na pasta de dados do terra.
const TOKEN_FILE: &str = "mcp.token";
/// A variável de ambiente com o token para o agente: o `--mcp-config` cita
/// `${TERRA_MCP_TOKEN}`, e o token não aparece na linha de comando (`ps`).
pub const TOKEN_ENV: &str = "TERRA_MCP_TOKEN";

/// O token do MCP (`~/.terraforme/mcp.token`), criado na primeira vez.
pub fn token() -> Result<String> {
    token_in(&crate::db::data_dir()?)
}

/// O token do MCP na pasta `dir`: 32 bytes aleatórios em hexadecimal, num
/// arquivo que só o usuário lê.
fn token_in(dir: &Path) -> Result<String> {
    let path = dir.join(TOKEN_FILE);
    if let Some(token) = read_token(&path) {
        return Ok(token);
    }
    fs::create_dir_all(dir).with_context(|| format!("não foi possível criar {}", dir.display()))?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| anyhow::anyhow!("sem bytes aleatórios: {err}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    // Grava ao lado e renomeia: quem lê nunca vê o arquivo pela metade. Com
    // dois terra criando ao mesmo tempo, vale o último, e os dois o leem.
    let tmp = dir.join(format!("{TOKEN_FILE}.{}", std::process::id()));
    write_private(&tmp, &token)
        .and_then(|()| fs::rename(&tmp, &path))
        .with_context(|| format!("não foi possível gravar {}", path.display()))?;
    read_token(&path).context("o token do MCP sumiu logo depois de gravado")
}

fn read_token(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let token = text.trim();
    (token.len() >= 32).then(|| token.to_string())
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(text.as_bytes())
}

/// No Windows, a pasta do usuário já é só dele.
#[cfg(windows)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    fs::write(path, text)
}

/// O MCP do terra para o agente, em `url` (veja [`agent_url`] e
/// [`delegated_url`]): o token vai pela variável [`TOKEN_ENV`].
pub fn agent_server(url: String, token: &str) -> crate::extension::Contribution {
    crate::extension::Contribution {
        mcp_servers: vec![crate::extension::McpServer {
            name: SERVER_NAME.to_string(),
            url: Some(url),
            headers: vec![(
                "Authorization".to_string(),
                format!("Bearer ${{{TOKEN_ENV}}}"),
            )],
            ..Default::default()
        }],
        env: vec![(TOKEN_ENV.to_string(), token.to_string())],
        ..Default::default()
    }
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
    /// Os chats abertos neste terra, com o consumo de cada um.
    pub chats: Vec<ChatStats>,
}

/// Um chat aberto num terra, como ele aparece no MCP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatStats {
    pub title: String,
    /// Tokens de entrada e saída, sem os de cache.
    pub tokens: u64,
    /// Tokens de cache (criação e leitura).
    pub cache_tokens: u64,
    /// O agente está respondendo.
    pub busy: bool,
    /// A delegação ([`ChatRequest::id`]) quando o chat foi aberto por um
    /// agente.
    pub id: Option<u64>,
    /// A última resposta do agente do chat, para voltar a quem delegou.
    pub reply: Option<String>,
}

impl ChatStats {
    fn to_json(&self) -> Value {
        json!({
            "title": self.title,
            "tokens": self.tokens,
            "cache_tokens": self.cache_tokens,
            "busy": self.busy,
            "id": self.id,
            "reply": self.reply,
        })
    }

    fn from_json(json: &Value) -> Option<Self> {
        Some(Self {
            title: json["title"].as_str()?.to_string(),
            tokens: json["tokens"].as_u64().unwrap_or(0),
            cache_tokens: json["cache_tokens"].as_u64().unwrap_or(0),
            busy: json["busy"].as_bool().unwrap_or(false),
            id: json["id"].as_u64(),
            reply: json["reply"].as_str().map(str::to_string),
        })
    }
}

impl Instance {
    fn to_json(&self) -> Value {
        json!({
            "pid": self.pid,
            "name": self.name,
            "dir": self.dir.to_string_lossy(),
            "session": self.session,
            "chats": self.chats.iter().map(ChatStats::to_json).collect::<Vec<_>>(),
        })
    }

    fn from_json(json: &Value) -> Option<Self> {
        Some(Self {
            pid: u32::try_from(json["pid"].as_u64()?).ok()?,
            name: json["name"].as_str()?.to_string(),
            dir: PathBuf::from(json["dir"].as_str()?),
            session: json["session"].as_bool().unwrap_or(false),
            chats: json["chats"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(ChatStats::from_json)
                .collect(),
        })
    }
}

/// O endereço do MCP para o agente do terra `pid`: assim o MCP sabe de qual
/// terra veio o pedido.
pub fn agent_url(pid: u32) -> String {
    format!("{}?from={pid}", url())
}

/// O endereço do MCP para o agente de um chat delegado (`chat` é o
/// [`ChatRequest::id`]): assim o MCP sabe de qual delegação vêm os pedidos
/// de permissão dele.
pub fn delegated_url(pid: u32, chat: u64) -> String {
    format!("{}&chat={chat}", agent_url(pid))
}

/// O nome do MCP do terra para os agentes.
pub const SERVER_NAME: &str = "terra";
/// A ferramenta que um chat delegado usa para pedir permissão a quem delegou.
pub const PERMISSION_TOOL: &str = "permission";

/// Um pedido de um agente para abrir um chat novo num terra.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatRequest {
    /// A delegação, dada pelo MCP: com ela, quem delegou espera a resposta
    /// (a ferramenta `wait`) e aprova os pedidos de permissão do chat.
    pub id: u64,
    /// O nome do workspace de onde veio o pedido, quando se sabe.
    pub from: Option<String>,
    /// O título da aba do chat.
    pub title: Option<String>,
    /// A primeira mensagem do chat.
    pub message: String,
    /// Os servidores MCP do usuário que o chat leva além do
    /// [`DELEGATED_MCP`].
    pub mcp: Vec<String>,
    /// As ferramentas embutidas que o chat tem além das [`DELEGATED_TOOLS`].
    pub tools: Vec<String>,
}

impl ChatRequest {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "from": self.from,
            "title": self.title,
            "message": self.message,
            "mcp": self.mcp,
            "tools": self.tools,
        })
    }

    fn from_json(json: &Value) -> Option<Self> {
        let text = |key: &str| {
            json[key]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        };
        let names = |key: &str| -> Vec<String> {
            json[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| Some(name.as_str()?.trim().to_string()))
                .filter(|name| !name.is_empty())
                .collect()
        };
        Some(Self {
            id: json["id"].as_u64().unwrap_or(0),
            from: text("from"),
            title: text("title"),
            message: text("message")?,
            mcp: names("mcp"),
            tools: names("tools"),
        })
    }

    /// O prompt da primeira mensagem: diz ao agente de onde ela veio, que
    /// a resposta final volta para quem delegou e como pedir uma ferramenta
    /// que ele não tem.
    pub fn prompt(&self) -> String {
        let from = self.from.as_deref().unwrap_or("principal");
        format!(
            "[Tarefa delegada pelo agente do workspace {from}. A sua última resposta volta para ele: termine com o resultado. Você roda só com as ferramentas e os MCP que ele escolheu; se precisar de outro, pare e termine dizendo qual e por quê, e ele delega de novo com ele.]\n\n{}",
            self.message
        )
    }
}

/// Se há um MCP do terra respondendo na porta [`PORT`].
pub fn is_online() -> bool {
    call("ping", json!({})).is_ok()
}

/// A versão do terra que abriu o MCP da porta, se houver um respondendo.
fn running_version() -> Option<String> {
    let result = call("initialize", json!({})).ok()?;
    Some(result["serverInfo"]["version"].as_str()?.to_string())
}

/// Abre o MCP num processo novo (`terra mcp`), a não ser que já haja um na
/// porta da mesma versão. Um MCP de outra versão (o terra foi atualizado com
/// ele aberto) é fechado e aberto de novo. Retorna `true` se abriu.
pub fn ensure_running() -> Result<bool> {
    match running_version() {
        Some(version) if version == env!("CARGO_PKG_VERSION") => return Ok(false),
        Some(_) => {
            shutdown()?;
            let deadline = std::time::Instant::now() + TIMEOUT;
            while is_online() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        None if is_online() => return Ok(false),
        None => {}
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

/// Quanto o pedido de permissão espera quem delegou decidir: o MCP nega
/// depois de 10 minutos, e este espera um pouco mais para ler a negativa.
const PERMISSION_WAIT: Duration = Duration::from_secs(11 * 60);

/// De onde vem o pedido de um chat delegado: o terra (`from`) e a delegação
/// (`chat`) que a URL do servidor `terra` ([`delegated_url`]) carrega.
pub fn delegated_ids(url: &str) -> Option<(u32, u64)> {
    let query = url.split_once('?')?.1;
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
    };
    Some((param("from")?.parse().ok()?, param("chat")?.parse().ok()?))
}

/// Pede, pelo MCP, a quem delegou o chat `chat` (do terra `pid`) para usar a
/// ferramenta `tool` com `input`, e espera a decisão. `Ok(Ok(()))` é
/// aprovado; `Ok(Err(motivo))`, negado. Para os provedores sem
/// `--permission-prompt-tool`, que chamam isso de um gancho.
pub fn ask_permission(
    pid: u32,
    chat: u64,
    tool: &str,
    input: &Value,
) -> Result<std::result::Result<(), String>> {
    let target = format!("{PATH}?from={pid}&chat={chat}");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": PERMISSION_TOOL,
            "arguments": { "tool_name": tool, "input": input },
        },
    });
    let body = post(
        addr(),
        &token()?,
        &target,
        PERMISSION_WAIT,
        &request.to_string(),
    )?;
    let response: Value =
        serde_json::from_str(&body).context("o MCP respondeu algo que não é JSON")?;
    if let Some(error) = response.get("error") {
        bail!("o MCP respondeu com erro: {}", error["message"]);
    }
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let decision: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    if decision["behavior"] == "allow" {
        return Ok(Ok(()));
    }
    let reason = decision["message"]
        .as_str()
        .unwrap_or("negado por quem delegou");
    Ok(Err(reason.to_string()))
}

/// Chama o método `method` do MCP e retorna o `result`.
fn call(method: &str, params: Value) -> Result<Value> {
    call_at(addr(), &token()?, method, params)
}

/// Chama o método `method` do servidor em `addr`, com o token `token`.
fn call_at(addr: SocketAddr, token: &str, method: &str, params: Value) -> Result<Value> {
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let body = post(addr, token, PATH, TIMEOUT, &request.to_string())?;
    let response: Value =
        serde_json::from_str(&body).context("o MCP respondeu algo que não é JSON")?;
    if let Some(error) = response.get("error") {
        bail!("o MCP respondeu com erro: {}", error["message"]);
    }
    Ok(response["result"].clone())
}

/// Um `POST` em `target` (o caminho e a query) de `addr`, com o corpo `body`,
/// esperando a resposta por `wait`; retorna o corpo da resposta.
fn post(addr: SocketAddr, token: &str, target: &str, wait: Duration, body: &str) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    stream.set_read_timeout(Some(wait))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    write!(
        stream,
        "POST {target} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nAuthorization: Bearer {token}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    /// Os chats pedidos pelos agentes, ainda não abertos.
    chats: Mutex<Vec<ChatRequest>>,
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
            chats: Mutex::new(Vec::new()),
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

    /// Avisa, na próxima batida, os chats abertos e o consumo deles.
    pub fn set_chats(&self, chats: Vec<ChatStats>) {
        if let Ok(mut me) = self.shared.me.lock() {
            me.chats = chats;
        }
    }

    /// Os terra abertos; `None` com o MCP offline.
    pub fn instances(&self) -> Option<Vec<Instance>> {
        self.shared.instances.lock().ok()?.clone()
    }

    /// Os chats que os agentes pediram para abrir neste terra.
    pub fn take_chats(&self) -> Vec<ChatRequest> {
        self.shared
            .chats
            .lock()
            .map(|mut chats| std::mem::take(&mut *chats))
            .unwrap_or_default()
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
            .map(|answer| {
                let chats = answer["chats"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(ChatRequest::from_json);
                if let Ok(mut pending) = shared.chats.lock() {
                    pending.extend(chats);
                }
            })
            .and_then(|()| instances())
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
            chats: vec![ChatStats {
                title: "Chat 2".into(),
                tokens: 10,
                cache_tokens: 100,
                busy: false,
                id: Some(3),
                reply: Some("feito".into()),
            }],
        };
        assert_eq!(Instance::from_json(&instance.to_json()), Some(instance));
        assert_eq!(Instance::from_json(&json!({"pid": 1})), None);
    }

    #[test]
    fn chat_request_round_trips_and_needs_a_message() {
        let request = ChatRequest {
            id: 7,
            from: Some("a".into()),
            title: None,
            message: "oi".into(),
            mcp: vec!["git".into()],
            tools: vec!["WebFetch".into()],
        };
        assert_eq!(
            ChatRequest::from_json(&request.to_json()),
            Some(request.clone())
        );
        assert!(request.prompt().contains("workspace a") && request.prompt().ends_with("oi"));
        assert_eq!(ChatRequest::from_json(&json!({"message": "  "})), None);
    }
}
