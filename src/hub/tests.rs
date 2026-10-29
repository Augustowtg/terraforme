//! Testes com hubs de verdade, em processos próprios: o hub morto com
//! `kill -9`, dez janelas abrindo juntas e o cliente que não lê.
//!
//! O hub de cada teste é este mesmo executável de testes, rodando só o
//! [`hub_process`] (ignorado no `cargo test` comum), com a pasta do teste
//! em [`DIR_ENV`].

use std::ffi::OsString;
use std::fs;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use interprocess::local_socket::prelude::*;
use serde_json::{Value, json};

use super::client::{self, Reporter};
use super::daemon;
use super::endpoint::Endpoint;
use super::protocol::{self, Health, Instance, Status, recv, send};
use super::{Conn, DIR_ENV, PROTO};

/// O hub dos testes: só roda quando um teste o inicia, com a pasta dele.
#[test]
#[ignore = "o hub dos outros testes, iniciado por eles"]
fn hub_process() {
    if std::env::var_os(DIR_ENV).is_some() {
        daemon::run().unwrap();
    }
}

/// Um endpoint na pasta do teste, que inicia o [`hub_process`].
fn endpoint(dir: &Path) -> Endpoint {
    let mut endpoint = Endpoint::in_dir(dir);
    let exe = std::env::current_exe().unwrap();
    endpoint.launcher = Some(vec![
        exe.into(),
        "hub::tests::hub_process".into(),
        "--exact".into(),
        "--ignored".into(),
        OsString::from("--test-threads=1"),
    ]);
    endpoint
}

/// O pid do hub vivo, pelo `hub.lock`.
fn hub_pid(endpoint: &Endpoint) -> Option<u64> {
    let lock: Value = serde_json::from_str(&fs::read_to_string(&endpoint.lock).ok()?).ok()?;
    lock["pid"].as_u64()
}

fn kill_9(pid: u64) {
    #[cfg(unix)]
    // SAFETY: o kill só manda o sinal ao pid do hub do teste.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .unwrap();
}

/// Espera `ok` valer, por até `secs`.
fn wait<T>(secs: u64, what: &str, mut ok: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(value) = ok() {
            return value;
        }
        assert!(Instant::now() < deadline, "esperando {what}");
        thread::sleep(Duration::from_millis(100));
    }
}

fn instances(endpoint: &Endpoint) -> Vec<Instance> {
    client::snapshot_with(endpoint, false)
        .unwrap_or_default()
        .iter()
        .filter_map(Instance::from_json)
        .collect()
}

fn status(task: Option<&str>) -> Status {
    Status {
        running: task.is_some(),
        task: task.map(str::to_string),
        chats: 1,
        ..Status::default()
    }
}

#[test]
fn every_window_reconnects_and_resends_its_state_after_kill_9_on_the_hub() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = endpoint(dir.path());
    let mut windows: Vec<Reporter> = (0..3)
        .map(|i| {
            Reporter::start_with(
                endpoint.clone(),
                true,
                format!("w{i}"),
                format!("/proj/w{i}"),
                status(None),
            )
        })
        .collect();
    wait(15, "as três janelas no hub", || {
        let all = instances(&endpoint);
        (all.iter().filter(|i| i.health == Health::Alive).count() == 3).then_some(())
    });
    let first = hub_pid(&endpoint).unwrap();

    kill_9(first);
    // O estado muda com o hub fora do ar: o novo hub tem que recebê-lo.
    windows[0].set_status(status(Some("Pensando...")));

    let deadline = Instant::now() + Duration::from_secs(20);
    let all = loop {
        let pid = hub_pid(&endpoint);
        let all = instances(&endpoint);
        let alive = all.iter().filter(|i| i.health == Health::Alive).count();
        if pid.is_some_and(|pid| pid != first) && alive == 3 {
            break all;
        }
        if Instant::now() > deadline {
            let log = fs::read_to_string(&endpoint.log).unwrap_or_default();
            panic!("as janelas não voltaram ao hub novo (pid {pid:?}, vivas {alive})\n{log}");
        }
        thread::sleep(Duration::from_millis(100));
    };
    let w0 = all.iter().find(|i| i.workspace == "w0").unwrap();
    assert_eq!(w0.status.task.as_deref(), Some("Pensando..."));
    drop(windows);
}

#[test]
fn ten_windows_opening_in_the_same_second_share_a_single_hub() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = endpoint(dir.path());
    let start = Arc::new(Barrier::new(10));
    // As dez threads existem antes de qualquer uma conectar (a barreira).
    let threads: Vec<_> = (0..10)
        .map(|_| {
            let (endpoint, start) = (endpoint.clone(), Arc::clone(&start));
            thread::spawn(move || {
                start.wait();
                let stream = client::connect(&endpoint, true).unwrap();
                let conn = Conn::new(stream);
                let mut output = conn.clone();
                let mut input = BufReader::new(conn);
                let hello = json!({"t": "hello", "proto": PROTO, "role": "monitor"});
                send(&mut output, &hello).unwrap();
                let welcome = recv(&mut input).unwrap().unwrap();
                assert_eq!(welcome["t"], "welcome");
                output
            })
        })
        .collect();
    let opened: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    // Os hubs que perderam a corrida saem logo; a espera os deixa sair.
    thread::sleep(Duration::from_millis(500));
    let log = fs::read_to_string(&endpoint.log).unwrap();
    let started = log.lines().filter(|l| l.contains("(proto ")).count();
    assert_eq!(started, 1, "mais de um hub subiu:\n{log}");
    drop(opened);
}

#[test]
fn a_client_that_does_not_read_is_disconnected_without_holding_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::in_dir(dir.path());
    let server = {
        let endpoint = endpoint.clone();
        thread::spawn(move || daemon::serve(&endpoint, Duration::from_millis(300)).unwrap())
    };
    let connect = || {
        wait(5, "o hub subir", || {
            Stream::connect(endpoint.name().unwrap()).ok()
        })
    };
    // O monitor lento assina e nunca mais lê.
    let slow = Conn::new(connect());
    send(
        &mut slow.clone(),
        &json!({"t": "hello", "proto": PROTO, "role": "monitor"}),
    )
    .unwrap();
    send(&mut slow.clone(), &json!({"t": "subscribe"})).unwrap();
    let monitor = client::Monitor::start_with(endpoint.clone(), false);

    // Uma instância manda estados sem parar.
    let mut window = Conn::new(connect());
    let hello = protocol::hello_instance("w", "w", "/w", &status(None));
    send(&mut window, &hello).unwrap();
    for i in 0..5000 {
        let msg = protocol::status_message(&status(Some(&format!("tarefa {i}"))));
        send(&mut window, &msg).unwrap();
    }

    // O monitor que lê recebe tudo até o fim.
    wait(10, "o monitor rápido ver o último estado", || {
        let (_, all) = monitor.instances();
        let last = all.first()?.status.task.as_deref() == Some("tarefa 4999");
        last.then_some(())
    });
    // O lento foi desconectado: depois do que ficou no buffer, vem o fim.
    slow.set_recv_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut rest = Vec::new();
    BufReader::new(slow.clone())
        .read_to_end(&mut rest)
        .expect("o hub devia ter fechado a conexão lenta");
    let log = fs::read_to_string(&endpoint.log).unwrap();
    assert!(log.contains("cliente lento"), "{log}");

    drop((window, slow, monitor));
    server.join().unwrap();
}
