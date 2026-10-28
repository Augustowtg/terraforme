//! O processo do hub (`terra __hub`): escuta no [`Endpoint`], guarda o
//! estado das instâncias e repassa o que muda aos monitores.
//!
//! ```text
//! serve():
//!     bind(endpoint); se o endereço está em uso:
//!         conecta? outro hub está vivo → sai
//!         senão, socket órfão → apaga e tenta de novo
//!     grava o hub.lock (temporário + rename)
//!     loop: aceita conexões (uma thread por cliente); a cada 1 s marca as
//!           instâncias stale e tira as gone; sem conexões por IDLE_EXIT,
//!           apaga o lock e sai (o socket sai com o listener)
//! ```
//!
//! Dois hubs iniciando juntos se resolvem sozinhos: só um consegue o bind.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerNonblockingMode, ListenerOptions, Stream};
use serde_json::{Value, json};

use super::endpoint::Endpoint;
use super::protocol::{Health, now, recv, send};
use super::{APP_VERSION, Conn, GONE_KEEP, PROTO, STALE_AFTER};

/// De quanto em quanto tempo o hub revê a saúde das instâncias.
const SWEEP: Duration = Duration::from_secs(1);
/// A espera entre um `accept` vazio e o próximo.
const POLL: Duration = Duration::from_millis(50);

/// Roda o hub deste usuário até ele ficar sem conexões.
pub fn run() -> Result<()> {
    serve(&Endpoint::current()?, super::IDLE_EXIT)
}

/// Roda o hub em `endpoint`; sai depois de `idle_exit` sem conexões, ou
/// logo, se outro hub já está vivo.
pub fn serve(endpoint: &Endpoint, idle_exit: Duration) -> Result<()> {
    endpoint.prepare()?;
    let log = Log::open(endpoint);
    let Some(listener) = bind(endpoint)? else {
        log.line("outro hub já está vivo; saindo");
        return Ok(());
    };
    write_lock(endpoint).context("não foi possível gravar o hub.lock")?;
    log.line(&format!(
        "hub {APP_VERSION} (proto {PROTO}) em {}",
        endpoint.socket.display()
    ));
    let hub = Arc::new(Hub {
        state: Mutex::default(),
        quit: AtomicBool::new(false),
        clients: AtomicU64::new(0),
        idle_since: Mutex::new(Some(Instant::now())),
        next_conn: AtomicU64::new(1),
        log,
    });
    let mut last_sweep = Instant::now();
    loop {
        match listener.accept() {
            Ok(stream) => {
                hub.clients.fetch_add(1, Ordering::SeqCst);
                *hub.idle_since.lock().unwrap() = None;
                let hub = Arc::clone(&hub);
                thread::spawn(move || {
                    hub.handle(stream);
                    if hub.clients.fetch_sub(1, Ordering::SeqCst) == 1 {
                        *hub.idle_since.lock().unwrap() = Some(Instant::now());
                    }
                });
                continue;
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(err) => {
                hub.log.line(&format!("accept falhou: {err}"));
                thread::sleep(POLL);
            }
        }
        if last_sweep.elapsed() >= SWEEP {
            hub.sweep();
            last_sweep = Instant::now();
        }
        if hub.quit.load(Ordering::SeqCst) {
            hub.log
                .line("um terra com o protocolo mais novo pediu a troca do hub");
            break;
        }
        let idle = hub.idle_since.lock().unwrap().map(|t| t.elapsed());
        if idle.is_some_and(|idle| idle >= idle_exit) {
            hub.log.line("nenhum terminal aberto; saindo");
            break;
        }
    }
    remove_lock(endpoint);
    Ok(())
}

/// O listener do hub, ou `None` se outro hub já está vivo.
fn bind(endpoint: &Endpoint) -> Result<Option<LocalSocketListener>> {
    // No Windows, um segundo servidor do mesmo pipe não falha: a pergunta
    // vem antes.
    if Stream::connect(endpoint.name()?).is_ok() {
        return Ok(None);
    }
    let create = || -> io::Result<LocalSocketListener> {
        ListenerOptions::new()
            .name(endpoint.name()?)
            .nonblocking(ListenerNonblockingMode::Accept)
            .create_sync()
    };
    match create() {
        Ok(listener) => Ok(Some(listener)),
        // No Windows, o pipe de outro hub chega como acesso negado.
        Err(err) if in_use(&err) => {
            if Stream::connect(endpoint.name()?).is_ok() {
                return Ok(None);
            }
            // Ninguém atende: o socket ficou de um hub que morreu.
            let _ = fs::remove_file(&endpoint.socket);
            match create() {
                Ok(listener) => Ok(Some(listener)),
                // Outro hub ganhou a corrida.
                Err(err) if in_use(&err) => Ok(None),
                Err(err) => Err(err.into()),
            }
        }
        Err(err) => Err(err)
            .with_context(|| format!("não foi possível escutar em {}", endpoint.socket.display())),
    }
}

type LocalSocketListener = interprocess::local_socket::Listener;

fn in_use(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    )
}

/// Grava o `hub.lock` de forma atômica (temporário + rename).
fn write_lock(endpoint: &Endpoint) -> io::Result<()> {
    let lock = json!({
        "pid": std::process::id(),
        "start_time": now(),
        "endpoint": endpoint.socket.to_string_lossy(),
        "proto": PROTO,
        "versao_app": APP_VERSION,
    });
    let tmp = endpoint.lock.with_extension("lock.tmp");
    fs::write(&tmp, format!("{lock:#}\n"))?;
    fs::rename(&tmp, &endpoint.lock)
}

/// Apaga o `hub.lock`, se ainda é o deste hub.
fn remove_lock(endpoint: &Endpoint) {
    let ours = fs::read_to_string(&endpoint.lock)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|lock| lock["pid"] == std::process::id());
    if ours {
        let _ = fs::remove_file(&endpoint.lock);
    }
}

/// O `hub.log`: o hub não tem terminal.
struct Log(Mutex<Option<File>>);

impl Log {
    fn open(endpoint: &Endpoint) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&endpoint.log)
            .ok();
        Self(Mutex::new(file))
    }

    fn line(&self, text: &str) {
        if let Some(file) = self.0.lock().unwrap().as_mut() {
            let _ = writeln!(
                file,
                "{} [{}] {text}",
                jiff::Timestamp::now(),
                std::process::id()
            );
        }
    }
}

/// Uma instância no hub.
struct Entry {
    /// O objeto que os monitores recebem.
    info: Value,
    health: Health,
    last_signal: Instant,
    gone_at: Option<Instant>,
    /// A conexão dona do registro: uma instância que reconecta troca de
    /// conexão, e o fim da antiga não pode marcá-la como `gone`.
    conn: u64,
}

#[derive(Default)]
struct State {
    /// Na ordem em que chegaram.
    instances: Vec<(String, Entry)>,
    monitors: Vec<(u64, Conn)>,
}

impl State {
    fn entry(&mut self, id: &str) -> Option<&mut Entry> {
        self.instances
            .iter_mut()
            .find(|(i, _)| i == id)
            .map(|(_, e)| e)
    }

    /// Manda `msg` a todos os monitores; o que não recebe sai da lista.
    fn broadcast(&mut self, msg: &Value) {
        self.monitors.retain_mut(|(_, out)| send(out, msg).is_ok());
    }

    fn update(&mut self, id: &str, changes: Value) {
        self.broadcast(&json!({"t": "instance_updated", "id": id, "changes": changes}));
    }
}

struct Hub {
    state: Mutex<State>,
    /// Pedido de saída (um terra com o protocolo mais novo).
    quit: AtomicBool,
    /// As conexões abertas, de qualquer papel.
    clients: AtomicU64,
    /// Desde quando o hub está sem conexões.
    idle_since: Mutex<Option<Instant>>,
    next_conn: AtomicU64,
    log: Log,
}

impl Hub {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn handle(&self, stream: Stream) {
        let conn = self.next_conn.fetch_add(1, Ordering::SeqCst);
        let output = Conn::new(stream);
        let mut input = BufReader::new(output.clone());
        let Ok(Some(hello)) = recv(&mut input) else {
            return;
        };
        if hello["t"] != "hello" {
            return;
        }
        match hello["role"].as_str() {
            Some("instance") => self.instance(conn, hello, input, output),
            Some("monitor") => self.monitor(conn, input, output),
            _ => {}
        }
    }

    fn instance(&self, conn: u64, hello: Value, mut input: BufReader<Conn>, mut output: Conn) {
        let proto = hello["proto"].as_u64().unwrap_or(0);
        if proto != PROTO {
            // O mais novo vence: com o terra mais novo, o hub sai e ele
            // inicia outro; com o mais antigo, ele segue sem o hub.
            let newer = proto > PROTO;
            let action = if newer {
                "restart_hub"
            } else {
                "upgrade_instance"
            };
            let _ = send(
                &mut output,
                &json!({"t": "reject", "reason": "proto_mismatch", "action": action}),
            );
            if newer {
                self.quit.store(true, Ordering::SeqCst);
            }
            return;
        }
        let Some(id) = hello["id"].as_str().map(str::to_string) else {
            return;
        };
        let mut info = hello;
        if let Some(fields) = info.as_object_mut() {
            for key in ["t", "proto", "role"] {
                fields.remove(key);
            }
            fields.insert("health".into(), Health::Alive.as_str().into());
            fields.insert("last_seen".into(), now().into());
            fields.insert("reason".into(), Value::Null);
        }
        {
            let mut state = self.state();
            state.instances.retain(|(i, _)| *i != id);
            state.broadcast(&json!({"t": "instance_added", "instance": info}));
            state.instances.push((
                id.clone(),
                Entry {
                    info,
                    health: Health::Alive,
                    last_signal: Instant::now(),
                    gone_at: None,
                    conn,
                },
            ));
        }
        if send(
            &mut output,
            &json!({"t": "welcome", "proto": PROTO, "hub": APP_VERSION}),
        )
        .is_err()
        {
            return self.gone(&id, conn, None);
        }
        let mut reason = None;
        while let Ok(Some(msg)) = recv(&mut input) {
            match msg["t"].as_str() {
                Some("heartbeat") => self.signal(&id, None),
                Some("status") => self.signal(&id, Some(&msg)),
                Some("bye") => {
                    reason = Some(msg["reason"].as_str().unwrap_or("exit").to_string());
                }
                _ => {}
            }
        }
        self.gone(&id, conn, reason);
    }

    /// Um sinal da instância: o `heartbeat` ou um `status`.
    fn signal(&self, id: &str, status: Option<&Value>) {
        let mut state = self.state();
        let Some(entry) = state.entry(id) else {
            return;
        };
        entry.last_signal = Instant::now();
        let seen = now();
        entry.info["last_seen"] = seen.into();
        let mut changes = serde_json::Map::new();
        if entry.health == Health::Stale {
            entry.health = Health::Alive;
            entry.info["health"] = Health::Alive.as_str().into();
            changes.insert("health".into(), Health::Alive.as_str().into());
        }
        if let Some(msg) = status {
            let mut fields = msg.clone();
            if let Some(fields) = fields.as_object_mut() {
                fields.remove("t");
            }
            entry.info["status"] = fields.clone();
            changes.insert("status".into(), fields);
        }
        if !changes.is_empty() {
            changes.insert("last_seen".into(), seen.into());
            state.update(id, Value::Object(changes));
        }
    }

    /// A conexão da instância fechou: o sinal principal de morte (o SO
    /// fecha o socket até num `kill -9`).
    fn gone(&self, id: &str, conn: u64, reason: Option<String>) {
        let mut state = self.state();
        let Some(entry) = state.entry(id).filter(|e| e.conn == conn) else {
            return;
        };
        let reason = reason.unwrap_or_else(|| "crash".into());
        entry.health = Health::Gone;
        entry.gone_at = Some(Instant::now());
        entry.info["health"] = Health::Gone.as_str().into();
        entry.info["reason"] = reason.clone().into();
        state.update(id, json!({"health": "gone", "reason": reason}));
    }

    fn monitor(&self, conn: u64, mut input: BufReader<Conn>, mut output: Conn) {
        if send(
            &mut output,
            &json!({"t": "welcome", "proto": PROTO, "hub": APP_VERSION}),
        )
        .is_err()
        {
            return;
        }
        loop {
            match recv(&mut input) {
                Ok(Some(msg)) if msg["t"] == "subscribe" => break,
                Ok(Some(_)) => {}
                _ => return,
            }
        }
        {
            let mut state = self.state();
            let instances: Vec<&Value> = state.instances.iter().map(|(_, e)| &e.info).collect();
            if send(
                &mut output,
                &json!({"t": "snapshot", "instances": instances}),
            )
            .is_err()
            {
                return;
            }
            state.monitors.push((conn, output));
        }
        // Depois do `subscribe`, o monitor só manda o `bye`, ao fechar.
        while let Ok(Some(msg)) = recv(&mut input) {
            if msg["t"] == "bye" {
                break;
            }
        }
        self.state().monitors.retain(|(c, _)| *c != conn);
    }

    /// Marca como `stale` as instâncias sem sinal e tira as `gone` antigas.
    fn sweep(&self) {
        let mut state = self.state();
        let mut stale = Vec::new();
        let mut removed = Vec::new();
        for (id, entry) in &mut state.instances {
            match entry.health {
                Health::Alive if entry.last_signal.elapsed() > STALE_AFTER => {
                    entry.health = Health::Stale;
                    entry.info["health"] = Health::Stale.as_str().into();
                    stale.push(id.clone());
                }
                Health::Gone if entry.gone_at.is_some_and(|t| t.elapsed() > GONE_KEEP) => {
                    removed.push((id.clone(), entry.info["reason"].clone()));
                }
                _ => {}
            }
        }
        for id in stale {
            state.update(&id, json!({"health": "stale"}));
        }
        for (id, reason) in removed {
            state.instances.retain(|(i, _)| *i != id);
            state.broadcast(&json!({"t": "instance_removed", "id": id, "reason": reason}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::client;
    use crate::hub::protocol::Status;

    #[test]
    fn instances_join_update_and_leave_while_the_monitor_follows() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::in_dir(dir.path());
        let server = {
            let endpoint = endpoint.clone();
            thread::spawn(move || serve(&endpoint, Duration::from_millis(300)).unwrap())
        };
        // O monitor espera o hub subir (sem iniciar outro).
        let monitor = client::Monitor::start_with(endpoint.clone(), false);
        let wait = |what: &str, ok: &dyn Fn(&[crate::hub::protocol::Instance]) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let (_, instances) = monitor.instances();
                if ok(&instances) {
                    return instances;
                }
                assert!(Instant::now() < deadline, "esperando {what}: {instances:?}");
                thread::sleep(Duration::from_millis(20));
            }
        };
        let status = Status {
            chats: 1,
            ..Status::default()
        };
        let mut reporter = client::Reporter::start_with(
            endpoint.clone(),
            false,
            "terraforme-cli".into(),
            "/proj/terraforme-cli".into(),
            status,
        );
        let instances = wait("a instância entrar", &|i| i.len() == 1);
        assert_eq!(instances[0].workspace, "terraforme-cli");
        assert_eq!(instances[0].health, Health::Alive);
        assert!(!instances[0].status.running);

        reporter.set_status(Status {
            running: true,
            task: Some("Pensando...".into()),
            chats: 3,
            agents: 2,
        });
        let instances = wait("o status chegar", &|i| i[0].status.running);
        assert_eq!(instances[0].status.agents, 2);
        assert_eq!(instances[0].status.task.as_deref(), Some("Pensando..."));

        // O `terra status`: a lista de uma vez.
        let snapshot = client::snapshot_with(&endpoint, false).unwrap();
        assert_eq!(snapshot.len(), 1);

        // Fechar o terra manda o `bye`.
        drop(reporter);
        let instances = wait("a instância sair", &|i| i[0].health == Health::Gone);
        assert_eq!(instances[0].reason.as_deref(), Some("exit"));

        // Sem conexões, o hub sai sozinho e leva o lock.
        drop(monitor);
        server.join().unwrap();
        assert!(!endpoint.lock.exists());
    }
}
