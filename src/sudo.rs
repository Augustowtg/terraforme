//! O `sudo` dos agentes, com a senha pedida no chat do terra (Unix).
//!
//! O agente roda os comandos sem terminal, então o `sudo` não tem onde pedir
//! a senha e falha. Com o [`Askpass`] aberto, o processo do agente recebe:
//! - no começo do `PATH`, um `sudo` que chama o de verdade com `-A` (o
//!   `sudo-rs` só usa o programa de senha com `-A`);
//! - em `SUDO_ASKPASS`, um script que roda `terra sudo-askpass`.
//!
//! O `terra sudo-askpass` ([`askpass`]) pergunta ao chat por um servidor em
//! `127.0.0.1`, com a porta e um token aleatório num arquivo só do usuário
//! (0600, numa pasta 0700), como o canal de permissão do `agy`. O chat
//! mostra o comando e um campo de senha oculto; negado, sem resposta em
//! [`WAIT`] ou com o chat fechado, o `sudo` não recebe senha e falha. A
//! senha não fica guardada: cada `sudo` pede de novo.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// Quanto o `sudo` espera o usuário digitar a senha.
const WAIT: Duration = Duration::from_secs(10 * 60);
/// Quanto o servidor espera o pedido chegar.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST: u64 = 64 * 1024;
const CHANNEL: &str = "channel";

/// Um `sudo` do agente esperando a senha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoRequest {
    pub id: String,
    /// O comando que vai rodar como root.
    pub command: String,
}

struct Shared {
    token: String,
    closed: AtomicBool,
    next: AtomicU64,
    pending: Mutex<HashMap<String, Sender<String>>>,
    requests: Sender<SudoRequest>,
}

/// O servidor dos pedidos de senha de um chat; fecha e apaga a pasta ao
/// soltar.
pub struct Askpass {
    dir: PathBuf,
    addr: SocketAddr,
    shared: Arc<Shared>,
    requests: Receiver<SudoRequest>,
}

impl Askpass {
    /// Abre o servidor e monta a pasta com o canal, o `sudo` e o askpass.
    /// Falha sem `sudo` no `PATH` ou fora do Unix.
    pub fn open() -> io::Result<Askpass> {
        if !cfg!(unix) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "só no Unix"));
        }
        let sudo = find_sudo().ok_or_else(|| io::Error::other("sudo não encontrado"))?;
        let terra = std::env::current_exe()?;
        let dir = make_dir()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|err| io::Error::other(err.to_string()))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let channel = dir.join(CHANNEL);
        write_private(&channel, &format!("{} {token}", addr.port()), false)?;
        write_private(
            &dir.join("sudo"),
            &format!("#!/bin/sh\nexec {} -A \"$@\"\n", quote(&sudo)),
            true,
        )?;
        write_private(
            &dir.join("askpass"),
            &format!(
                "#!/bin/sh\nexec {} sudo-askpass {}\n",
                quote(&terra),
                quote(&channel)
            ),
            true,
        )?;
        let (tx, requests) = mpsc::channel();
        let shared = Arc::new(Shared {
            token,
            closed: AtomicBool::new(false),
            next: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            requests: tx,
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
        Ok(Askpass {
            dir,
            addr,
            shared,
            requests,
        })
    }

    /// O que vai ao ambiente do processo do agente.
    pub fn env(&self) -> Vec<(String, String)> {
        let path = std::env::var("PATH").unwrap_or_default();
        vec![
            ("PATH".to_string(), format!("{}:{path}", self.dir.display())),
            (
                "SUDO_ASKPASS".to_string(),
                self.dir.join("askpass").display().to_string(),
            ),
        ]
    }

    /// Os pedidos de senha que chegaram desde a última chamada.
    pub fn poll(&self) -> Vec<SudoRequest> {
        self.requests.try_iter().collect()
    }

    /// A resposta ao pedido `id`: a senha, ou `None` para negar.
    pub fn answer(&self, id: &str, password: Option<String>) {
        let waiting = self.shared.pending.lock().unwrap().remove(id);
        if let (Some(waiting), Some(password)) = (waiting, password) {
            let _ = waiting.send(password);
        }
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        self.shared.pending.lock().unwrap().clear();
        let _ = TcpStream::connect_timeout(&self.addr, REQUEST_TIMEOUT);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let mut line = String::new();
    let read = match stream.try_clone() {
        Ok(clone) => BufReader::new(clone.take(MAX_REQUEST)).read_line(&mut line),
        Err(err) => Err(err),
    };
    let command = read.ok().and_then(|_| {
        let request: Value = serde_json::from_str(&line).ok()?;
        (request["token"].as_str() == Some(shared.token.as_str()))
            .then(|| request["command"].as_str().unwrap_or_default().to_string())
    });
    let password = command.and_then(|command| ask_user(shared, command));
    let reply = match password {
        Some(password) => json!({ "password": password }),
        None => json!({}),
    };
    let _ = writeln!(stream, "{reply}");
}

fn ask_user(shared: &Shared, command: String) -> Option<String> {
    let id = format!("sudo-{}", shared.next.fetch_add(1, Ordering::Relaxed) + 1);
    let (tx, waiting) = mpsc::channel();
    shared.pending.lock().unwrap().insert(id.clone(), tx);
    let sent = shared
        .requests
        .send(SudoRequest {
            id: id.clone(),
            command,
        })
        .is_ok();
    let password = (sent && !shared.closed.load(Ordering::Relaxed))
        .then(|| waiting.recv_timeout(WAIT).ok())
        .flatten();
    shared.pending.lock().unwrap().remove(&id);
    password
}

/// O lado do `sudo`: pede a senha ao chat pelo canal do arquivo `file`.
/// `None` é negado ou sem resposta.
pub fn ask(file: &Path, command: &str) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    let (port, token) = text.trim().split_once(' ')?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port.parse::<u16>().ok()?));
    let mut stream = TcpStream::connect_timeout(&addr, REQUEST_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(WAIT + REQUEST_TIMEOUT)).ok()?;
    writeln!(stream, "{}", json!({ "token": token, "command": command })).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let reply: Value = serde_json::from_str(&line).ok()?;
    reply["password"].as_str().map(str::to_string)
}

/// O `terra sudo-askpass`: o `sudo` o roda e lê a senha do stdout. O
/// comando é o do processo pai (o `sudo`). Sem senha, sai com erro (1).
pub fn askpass(file: &Path) -> i32 {
    match ask(file, &parent_command()) {
        Some(password) => {
            println!("{password}");
            0
        }
        None => 1,
    }
}

/// Os argumentos do `sudo` que chamou o askpass, sem o caminho dele e o
/// `-A` do wrapper.
fn parent_command() -> String {
    #[cfg(unix)]
    {
        let pid = std::os::unix::process::parent_id();
        if let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) {
            let args: Vec<String> = raw
                .split(|b| *b == 0)
                .filter(|arg| !arg.is_empty())
                .skip(1)
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect();
            let args = args.strip_prefix(&["-A".to_string()]).unwrap_or(&args);
            return args.join(" ");
        }
    }
    String::new()
}

/// O `sudo` de verdade no `PATH`.
fn find_sudo() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("sudo"))
        .find(|path| path.is_file())
}

/// Um caminho entre aspas simples para o `sh`.
fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

fn make_dir() -> io::Result<PathBuf> {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "terraforme-sudo-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(dir)
}

/// Grava o arquivo só do usuário (0600, ou 0700 se `executable`).
fn write_private(path: &Path, text: &str, executable: bool) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, if executable { 0o700 } else { 0o600 });
    let _ = executable;
    options.open(path)?.write_all(text.as_bytes())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn next_request(askpass: &Askpass) -> SudoRequest {
        for _ in 0..500 {
            if let Some(request) = askpass.poll().pop() {
                return request;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("o pedido não chegou ao chat");
    }

    #[test]
    fn the_password_typed_in_the_chat_goes_to_sudo() {
        let askpass = Askpass::open().unwrap();
        let file = askpass.dir.join(CHANNEL);
        let asked = thread::spawn(move || ask(&file, "apt install x"));
        let request = next_request(&askpass);
        assert_eq!(request.command, "apt install x");
        askpass.answer(&request.id, Some("segredo".to_string()));
        assert_eq!(asked.join().unwrap().as_deref(), Some("segredo"));
    }

    #[test]
    fn denied_sudo_gets_no_password() {
        let askpass = Askpass::open().unwrap();
        let file = askpass.dir.join(CHANNEL);
        let asked = thread::spawn(move || ask(&file, "rm -rf /x"));
        let request = next_request(&askpass);
        askpass.answer(&request.id, None);
        assert_eq!(asked.join().unwrap(), None);
    }
}
