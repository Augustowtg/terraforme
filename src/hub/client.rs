//! O lado de quem conversa com o hub: a instância ([`Reporter`]), o monitor
//! da aba Orquestrador ([`Monitor`]) e o `terra status` ([`snapshot`]).
//!
//! A regra de ouro: **o trabalho do terra nunca espera o hub**. O
//! [`Reporter::set_status`] só põe o estado na fila e volta; a conversa com o
//! hub fica numa thread. Com o hub fora do ar, a fila fica só com o estado
//! mais novo e o terra segue normalmente.

use std::io::{self, BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use interprocess::local_socket::prelude::*;
use serde_json::{Value, json};

use super::endpoint::Endpoint;
use super::protocol::{self, Instance, Status, recv, send};
use super::{Conn, HEARTBEAT, PROTO};

/// Quanto o cliente espera o hub que ele iniciou subir.
const START_WAIT: Duration = Duration::from_secs(3);
/// A espera entre uma conexão perdida e a próxima tentativa.
const RETRY: Duration = Duration::from_secs(2);
/// Quanto o fim do terra espera o `bye` sair.
const BYE_WAIT: Duration = Duration::from_millis(300);
/// O limite de uma leitura do monitor, para ele notar que foi fechado.
const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// Conecta ao hub. Sem ele e com `start`, inicia o hub desacoplado e tenta
/// de novo com backoff (50 ms, 100 ms, 200 ms…) até [`START_WAIT`].
pub fn connect(endpoint: &Endpoint, start: bool) -> io::Result<Stream> {
    match Stream::connect(endpoint.name()?) {
        Ok(stream) => return Ok(stream),
        Err(err) if !start => return Err(err),
        Err(_) => {}
    }
    spawn_hub()?;
    let deadline = Instant::now() + START_WAIT;
    let mut wait = Duration::from_millis(50);
    loop {
        thread::sleep(wait);
        match Stream::connect(endpoint.name()?) {
            Ok(stream) => return Ok(stream),
            Err(err) if Instant::now() >= deadline => return Err(err),
            Err(_) => wait = (wait * 2).min(Duration::from_millis(800)),
        }
    }
}

/// Inicia o `terra __hub` desacoplado: numa sessão nova (Unix) ou sem
/// console e num grupo próprio (Windows), com a entrada e as saídas no
/// nada. Fechar o terminal ou um `Ctrl+C` não derrubam o hub.
fn spawn_hub() -> io::Result<()> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("__hub")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: o setsid é seguro entre o fork e o exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = cmd.spawn()?;
    // Recolhe o hub quando ele sair antes deste terra (sem zumbi).
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Lê o `welcome` (ou o `reject`) do hub.
fn handshake(stream: Stream, hello: &Value) -> io::Result<(BufReader<Conn>, Conn, Value)> {
    let conn = Conn::new(stream);
    // O pipe do Windows não tem tempo limite de leitura.
    let _ = conn.set_recv_timeout(Some(START_WAIT));
    let mut output = conn.clone();
    let mut input = BufReader::new(conn);
    send(&mut output, hello)?;
    let answer = recv(&mut input)?.ok_or(io::ErrorKind::UnexpectedEof)?;
    Ok((input, output, answer))
}

enum Msg {
    Status(Status),
    /// O terra está saindo: manda o `bye` e avisa no canal.
    Bye(Sender<()>),
}

/// A instância: o terra contando ao hub o que está fazendo.
pub struct Reporter {
    tx: Sender<Msg>,
    last: Status,
    id: String,
}

impl Reporter {
    /// Conecta este terra ao hub (iniciando o hub, se preciso).
    pub fn start(workspace: String, cwd: String, status: Status) -> Option<Self> {
        let endpoint = Endpoint::current().ok()?;
        Some(Self::start_with(endpoint, true, workspace, cwd, status))
    }

    pub fn start_with(
        endpoint: Endpoint,
        autostart: bool,
        workspace: String,
        cwd: String,
        status: Status,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let last = status.clone();
        let id = protocol::new_id();
        let hello = Hello {
            id: id.clone(),
            workspace,
            cwd,
        };
        thread::spawn(move || report(&endpoint, autostart, &hello, status, &rx));
        Self { tx, last, id }
    }

    /// O id desta instância no hub.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Manda o estado ao hub, se mudou. Nunca bloqueia.
    pub fn set_status(&mut self, status: Status) {
        if status != self.last {
            self.last = status.clone();
            let _ = self.tx.send(Msg::Status(status));
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let (ack, done) = mpsc::channel();
        if self.tx.send(Msg::Bye(ack)).is_ok() {
            let _ = done.recv_timeout(BYE_WAIT);
        }
    }
}

struct Hello {
    id: String,
    workspace: String,
    cwd: String,
}

/// O que a fila trouxe.
enum Next {
    /// Chegou um estado novo (já em `latest`).
    Status,
    Bye(Sender<()>),
    /// Nada no tempo dado.
    Timeout,
    /// O [`Reporter`] sumiu sem o `bye`.
    Closed,
}

/// Espera a fila por até `timeout` e junta os estados que chegaram: só o
/// mais novo importa.
fn next(rx: &Receiver<Msg>, timeout: Duration, latest: &mut Status) -> Next {
    let first = match rx.recv_timeout(timeout) {
        Ok(msg) => msg,
        Err(RecvTimeoutError::Timeout) => return Next::Timeout,
        Err(RecvTimeoutError::Disconnected) => return Next::Closed,
    };
    let mut msg = first;
    loop {
        match msg {
            Msg::Bye(ack) => return Next::Bye(ack),
            Msg::Status(status) => *latest = status,
        }
        match rx.try_recv() {
            Ok(more) => msg = more,
            Err(_) => return Next::Status,
        }
    }
}

/// A thread da instância: conecta, manda o `hello` com o estado inteiro e
/// depois os estados novos, ou um `heartbeat` a cada [`HEARTBEAT`] sem
/// novidade. Caiu a conexão, reconecta.
fn report(
    endpoint: &Endpoint,
    autostart: bool,
    hello: &Hello,
    mut latest: Status,
    rx: &Receiver<Msg>,
) {
    let mut retry = Duration::ZERO;
    loop {
        // Entre as tentativas, a fila continua andando.
        let deadline = Instant::now() + retry;
        while let Some(left) = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        {
            match next(rx, left, &mut latest) {
                Next::Bye(ack) => {
                    let _ = ack.send(());
                    return;
                }
                Next::Closed => return,
                Next::Status | Next::Timeout => {}
            }
        }
        retry = RETRY;
        let Ok(stream) = connect(endpoint, autostart) else {
            continue;
        };
        let msg = protocol::hello_instance(&hello.id, &hello.workspace, &hello.cwd, &latest);
        let Ok((_input, mut output, answer)) = handshake(stream, &msg) else {
            continue;
        };
        match answer["t"].as_str() {
            Some("welcome") => {}
            // O hub é mais antigo e saiu: o próximo `connect` inicia este.
            Some("reject") if answer["action"] == "restart_hub" => {
                retry = Duration::from_millis(200);
                continue;
            }
            // Este terra é mais antigo que o hub: segue sem ele.
            Some("reject") => return,
            _ => continue,
        }
        loop {
            let sent = match next(rx, HEARTBEAT, &mut latest) {
                Next::Status => send(&mut output, &protocol::status_message(&latest)),
                Next::Timeout => send(&mut output, &json!({"t": "heartbeat"})),
                Next::Bye(ack) => {
                    let _ = send(&mut output, &json!({"t": "bye", "reason": "exit"}));
                    let _ = ack.send(());
                    return;
                }
                Next::Closed => return,
            };
            if sent.is_err() {
                break;
            }
        }
    }
}

/// O que o monitor sabe: se está ligado ao hub e as instâncias (os objetos
/// do protocolo, com as mudanças aplicadas).
#[derive(Default)]
struct View {
    connected: bool,
    instances: Vec<Value>,
    /// A conexão de agora, para o `bye` do fim.
    conn: Option<Conn>,
}

impl View {
    fn apply(&mut self, msg: &Value) {
        match msg["t"].as_str() {
            Some("snapshot") => {
                self.instances = msg["instances"].as_array().cloned().unwrap_or_default();
            }
            Some("instance_added") => {
                let instance = msg["instance"].clone();
                self.instances.retain(|i| i["id"] != instance["id"]);
                self.instances.push(instance);
            }
            Some("instance_updated") => {
                if let Some(instance) = self.instances.iter_mut().find(|i| i["id"] == msg["id"]) {
                    protocol::merge(instance, &msg["changes"]);
                }
            }
            Some("instance_removed") => self.instances.retain(|i| i["id"] != msg["id"]),
            _ => {}
        }
    }
}

/// O monitor da aba Orquestrador: assina as instâncias e acompanha o que
/// muda numa thread. Fechado (drop), a thread sai e a conexão fecha.
pub struct Monitor {
    view: Arc<Mutex<View>>,
}

impl Monitor {
    pub fn start() -> Option<Self> {
        Some(Self::start_with(Endpoint::current().ok()?, true))
    }

    pub fn start_with(endpoint: Endpoint, autostart: bool) -> Self {
        let view = Arc::new(Mutex::new(View::default()));
        let weak = Arc::downgrade(&view);
        thread::spawn(move || {
            while weak.strong_count() > 0 {
                follow(&endpoint, autostart, &weak);
                if let Some(view) = weak.upgrade() {
                    let mut view = view.lock().unwrap();
                    view.connected = false;
                    view.conn = None;
                }
                thread::sleep(if autostart {
                    RETRY
                } else {
                    Duration::from_millis(50)
                });
            }
        });
        Self { view }
    }

    /// Ligado ao hub? E as instâncias, na ordem em que chegaram.
    pub fn instances(&self) -> (bool, Vec<Instance>) {
        let view = self.view.lock().unwrap();
        let instances = view
            .instances
            .iter()
            .filter_map(Instance::from_json)
            .collect();
        (view.connected, instances)
    }
}

impl Drop for Monitor {
    /// O `bye` faz o hub fechar a conexão, e a leitura da thread acaba (o
    /// pipe do Windows não tem tempo limite para ela notar sozinha).
    fn drop(&mut self) {
        if let Some(mut conn) = self.view.lock().unwrap().conn.take() {
            let _ = send(&mut conn, &json!({"t": "bye"}));
        }
    }
}

/// Uma conexão do monitor, até ela cair ou o [`Monitor`] ser fechado.
fn follow(endpoint: &Endpoint, autostart: bool, view: &std::sync::Weak<Mutex<View>>) {
    let Ok((mut input, output)) = subscribe(endpoint, autostart) else {
        return;
    };
    match view.upgrade() {
        Some(view) => view.lock().unwrap().conn = Some(output),
        None => return,
    }
    let _ = input.get_ref().set_recv_timeout(Some(READ_TIMEOUT));
    let mut line = Vec::new();
    loop {
        // O `read_until` guarda o que já leu quando a leitura estoura o
        // tempo; a linha continua na próxima volta.
        match input.read_until(b'\n', &mut line) {
            Ok(0) => return,
            Ok(_) => {
                let msg = serde_json::from_slice(&line).unwrap_or(Value::Null);
                line.clear();
                let Some(view) = view.upgrade() else {
                    return;
                };
                let mut view = view.lock().unwrap();
                view.connected = true;
                view.apply(&msg);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if view.strong_count() == 0 {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Conecta como monitor e assina as instâncias.
fn subscribe(endpoint: &Endpoint, autostart: bool) -> io::Result<(BufReader<Conn>, Conn)> {
    let stream = connect(endpoint, autostart)?;
    let hello = json!({"t": "hello", "proto": PROTO, "role": "monitor"});
    let (input, mut output, answer) = handshake(stream, &hello)?;
    if answer["t"] != "welcome" {
        return Err(io::Error::other("o hub recusou o monitor"));
    }
    send(
        &mut output,
        &json!({"t": "subscribe", "topic": "instances"}),
    )?;
    Ok((input, output))
}

/// As instâncias de agora, de uma vez (o `terra status`), como o hub as
/// manda.
pub fn snapshot() -> io::Result<Vec<Value>> {
    let endpoint = Endpoint::current().map_err(io::Error::other)?;
    snapshot_with(&endpoint, true)
}

pub fn snapshot_with(endpoint: &Endpoint, autostart: bool) -> io::Result<Vec<Value>> {
    let (mut input, _output) = subscribe(endpoint, autostart)?;
    loop {
        let msg = recv(&mut input)?.ok_or(io::ErrorKind::UnexpectedEof)?;
        if msg["t"] == "snapshot" {
            return Ok(msg["instances"].as_array().cloned().unwrap_or_default());
        }
    }
}
