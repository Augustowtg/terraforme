//! O canal entre o gancho de permissão do `agy` e o chat do terra.
//!
//! No chat normal (modos Manual e Accept Edits), quem decide é o usuário do
//! terra, na tela. O gancho `PreToolUse` roda num processo à parte
//! (`terra agy-permission-hook`), então precisa de um jeito de falar com o
//! chat: o [`Channel`] abre um servidor em `127.0.0.1` numa porta qualquer e
//! grava a porta e um token aleatório num arquivo só do usuário (0600, numa
//! pasta 0700) dentro do plugin temporário. O gancho lê o arquivo, manda o
//! pedido com o token e espera a decisão ([`ask`]); quem não tem o arquivo
//! não consegue fazer um pedido.
//!
//! Cada pedido vira um [`AgentEvent::PermissionRequest`], que o chat mostra
//! como o do Claude; a resposta dele chega em [`Channel::respond`] e volta
//! ao gancho. Sem resposta em [`WAIT`], com o canal fechado (o chat foi
//! interrompido) ou com qualquer erro, a decisão é negar.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::plugin::write_private;
use crate::agent::AgentEvent;

/// O arquivo do canal (porta e token) dentro da pasta do plugin.
pub const FILE: &str = "channel";
/// Quanto o canal espera o usuário decidir.
const WAIT: Duration = Duration::from_secs(10 * 60);
/// Quanto o servidor espera o gancho mandar o pedido.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// O maior pedido aceito.
const MAX_REQUEST: u64 = 256 * 1024;

struct Shared {
    token: String,
    closed: AtomicBool,
    next: AtomicU64,
    /// Os pedidos esperando o usuário, pelo id do [`AgentEvent`].
    pending: Mutex<HashMap<String, Sender<bool>>>,
    events: Sender<AgentEvent>,
}

/// O servidor local dos pedidos do gancho; fecha ao soltar.
pub struct Channel {
    shared: Arc<Shared>,
    addr: SocketAddr,
    events: Receiver<AgentEvent>,
}

impl Channel {
    /// Abre o servidor e grava o arquivo [`FILE`] em `dir`.
    pub fn open(dir: &Path) -> io::Result<Channel> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|err| io::Error::other(err.to_string()))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        write_private(&dir.join(FILE), &format!("{} {token}", addr.port()))?;
        let (tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            token,
            closed: AtomicBool::new(false),
            next: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            events: tx,
        });
        let server = Arc::clone(&shared);
        thread::spawn(move || {
            for stream in listener.incoming() {
                if server.closed.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let server = Arc::clone(&server);
                thread::spawn(move || serve(stream, &server));
            }
        });
        Ok(Channel {
            shared,
            addr,
            events,
        })
    }

    /// Os pedidos de permissão que chegaram desde a última chamada.
    pub fn poll(&self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }

    /// A decisão do usuário a um pedido; o gancho a recebe na hora. Um id
    /// que não está esperando é ignorado.
    pub fn respond(&self, id: &str, allow: bool) {
        if let Some(waiting) = self.shared.pending.lock().unwrap().remove(id) {
            let _ = waiting.send(allow);
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        // Os pedidos esperando são negados (o remetente some), e uma conexão
        // acorda o `accept` para o servidor ver que fechou.
        self.shared.pending.lock().unwrap().clear();
        let _ = TcpStream::connect_timeout(&self.addr, REQUEST_TIMEOUT);
    }
}

/// Atende uma conexão do gancho: lê o pedido, mostra ao usuário e responde.
fn serve(mut stream: TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let mut line = String::new();
    let read = match stream.try_clone() {
        Ok(clone) => BufReader::new(clone.take(MAX_REQUEST)).read_line(&mut line),
        Err(err) => Err(err),
    };
    let reply = match read.map(|_| parse(&line, &shared.token)) {
        Ok(Some((tool, input))) => ask_user(shared, &tool, &input),
        _ => json!({ "allow": false, "reason": "o pedido ao terra não é válido" }),
    };
    let _ = writeln!(stream, "{reply}");
}

/// A ferramenta e a entrada do pedido, se o token confere.
fn parse(line: &str, token: &str) -> Option<(String, Value)> {
    let request: Value = serde_json::from_str(line).ok()?;
    if request["token"].as_str() != Some(token) {
        return None;
    }
    Some((
        request["tool"].as_str()?.to_string(),
        request["input"].clone(),
    ))
}

/// Põe o pedido na tela do chat e espera o usuário.
fn ask_user(shared: &Shared, tool: &str, input: &Value) -> Value {
    let id = format!("agy-{}", shared.next.fetch_add(1, Ordering::Relaxed) + 1);
    let (tx, waiting) = mpsc::channel();
    shared.pending.lock().unwrap().insert(id.clone(), tx);
    let event = AgentEvent::PermissionRequest {
        id: id.clone(),
        name: tool.to_string(),
        detail: detail(input),
    };
    let sent = shared.events.send(event).is_ok();
    let allow = sent && !shared.closed.load(Ordering::Relaxed) && {
        waiting.recv_timeout(WAIT).unwrap_or(false)
    };
    shared.pending.lock().unwrap().remove(&id);
    if allow {
        json!({ "allow": true })
    } else {
        json!({ "allow": false, "reason": "negado pelo usuário no terra" })
    }
}

/// O parâmetro principal do pedido para a tela: o comando, o caminho, a
/// URL... ou, sem nenhum conhecido, o primeiro texto.
fn detail(input: &Value) -> String {
    const KEYS: [&str; 8] = [
        "CommandLine",
        "Path",
        "AbsolutePath",
        "TargetFile",
        "DirectoryPath",
        "Url",
        "Query",
        "SearchPath",
    ];
    let Some(map) = input.as_object() else {
        return String::new();
    };
    KEYS.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .or_else(|| map.values().find_map(Value::as_str))
        .and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|line| match line.char_indices().nth(200) {
            Some((end, _)) => format!("{}…", &line[..end]),
            None => line.to_string(),
        })
        .unwrap_or_default()
}

/// O lado do gancho: pergunta ao chat do terra pelo canal do arquivo `file`.
/// `Ok(Ok(()))` é aprovado; `Ok(Err(motivo))`, negado; `Err`, o canal não
/// respondeu (o gancho nega).
pub fn ask(file: &Path, tool: &str, input: &Value) -> Result<Result<(), String>, String> {
    let text = fs::read_to_string(file)
        .map_err(|err| format!("o canal do chat ({}): {err}", file.display()))?;
    let (port, token) = text
        .trim()
        .split_once(' ')
        .ok_or("o arquivo do canal está malformado")?;
    let port: u16 = port.parse().map_err(|_| "a porta do canal é inválida")?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, REQUEST_TIMEOUT)
        .map_err(|err| format!("o chat do terra não atende ({err})"))?;
    stream
        .set_read_timeout(Some(WAIT + REQUEST_TIMEOUT))
        .map_err(|err| err.to_string())?;
    let request = json!({ "token": token, "tool": tool, "input": input });
    writeln!(stream, "{request}").map_err(|err| format!("o chat do terra não recebeu: {err}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|err| format!("o chat do terra não respondeu: {err}"))?;
    let reply: Value =
        serde_json::from_str(&line).map_err(|_| "o chat do terra respondeu algo inválido")?;
    if reply["allow"] == true {
        return Ok(Ok(()));
    }
    let reason = reply["reason"].as_str().unwrap_or("negado pelo usuário");
    Ok(Err(reason.to_string()))
}

#[cfg(test)]
mod tests {
    use super::super::plugin::make_dir;
    use super::*;

    /// Um canal numa pasta temporária; devolve também a pasta para apagar.
    fn open() -> (Channel, std::path::PathBuf) {
        let dir = make_dir().unwrap();
        (Channel::open(&dir).unwrap(), dir)
    }

    /// Espera o próximo pedido do gancho no chat.
    fn next_request(channel: &Channel) -> AgentEvent {
        for _ in 0..500 {
            if let Some(event) = channel.poll().into_iter().next() {
                return event;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("o pedido não chegou ao chat");
    }

    fn hook(dir: &Path, tool: &str) -> thread::JoinHandle<Result<Result<(), String>, String>> {
        let file = dir.join(FILE);
        let tool = tool.to_string();
        thread::spawn(move || ask(&file, &tool, &json!({ "CommandLine": "rm -rf x\nlixo" })))
    }

    #[test]
    fn the_user_allows_a_request_from_the_hook() {
        let (channel, dir) = open();
        let asked = hook(&dir, "run_command");
        let AgentEvent::PermissionRequest { id, name, detail } = next_request(&channel) else {
            panic!("esperava PermissionRequest");
        };
        assert_eq!(
            (name.as_str(), detail.as_str()),
            ("run_command", "rm -rf x")
        );
        channel.respond(&id, true);
        assert_eq!(asked.join().unwrap(), Ok(Ok(())));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_user_denies_a_request_from_the_hook() {
        let (channel, dir) = open();
        let asked = hook(&dir, "run_command");
        let AgentEvent::PermissionRequest { id, .. } = next_request(&channel) else {
            panic!("esperava PermissionRequest");
        };
        channel.respond(&id, false);
        let Ok(Err(reason)) = asked.join().unwrap() else {
            panic!("esperava negado");
        };
        assert!(reason.contains("negado pelo usuário"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn requests_are_answered_each_by_its_id() {
        let (channel, dir) = open();
        let first = hook(&dir, "run_command");
        let AgentEvent::PermissionRequest { id: one, .. } = next_request(&channel) else {
            panic!("esperava PermissionRequest");
        };
        let second = hook(&dir, "write_file");
        let AgentEvent::PermissionRequest { id: two, name, .. } = next_request(&channel) else {
            panic!("esperava PermissionRequest");
        };
        assert_eq!(name, "write_file");
        assert_ne!(one, two);
        channel.respond(&two, false);
        channel.respond(&one, true);
        assert!(second.join().unwrap().unwrap().is_err());
        assert!(first.join().unwrap().unwrap().is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_the_channel_denies_what_was_waiting() {
        let (channel, dir) = open();
        let asked = hook(&dir, "run_command");
        next_request(&channel);
        drop(channel);
        assert!(matches!(asked.join().unwrap(), Ok(Err(_))));
        // Sem o servidor, o gancho não consegue perguntar: nega com `Err`.
        assert!(ask(&dir.join(FILE), "run_command", &json!({})).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_wrong_token_or_a_broken_request_is_denied_without_asking_the_user() {
        let (channel, dir) = open();
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        let (port, _) = text.split_once(' ').unwrap();
        for body in [
            json!({ "token": "errado", "tool": "run_command", "input": {} }).to_string(),
            "isto não é json".to_string(),
            json!({ "tool": "run_command" }).to_string(),
        ] {
            let mut stream =
                TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
            writeln!(stream, "{body}").unwrap();
            let mut reply = String::new();
            BufReader::new(stream).read_line(&mut reply).unwrap();
            let reply: Value = serde_json::from_str(&reply).unwrap();
            assert_eq!(reply["allow"], false);
        }
        assert!(channel.poll().is_empty(), "o usuário não foi incomodado");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_channel_file_is_private() {
        let (_channel, dir) = open();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(FILE)), 0o600);
        }
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        let (port, token) = text.split_once(' ').unwrap();
        assert!(port.parse::<u16>().is_ok());
        assert_eq!(token.len(), 64);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_detail_is_the_main_parameter_in_one_short_line() {
        assert_eq!(
            detail(&json!({ "CommandLine": "ls -la", "Cwd": "/x" })),
            "ls -la"
        );
        assert_eq!(detail(&json!({ "Cwd": "/x" })), "/x");
        assert_eq!(detail(&json!({ "CommandLine": "\n  a\nb" })), "a");
        assert_eq!(detail(&json!(null)), "");
        let long = "x".repeat(300);
        assert_eq!(detail(&json!({ "CommandLine": long })).chars().count(), 201);
    }
}
