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

// As mensagens entre agentes, num hub nesta mesma thread de testes.
mod messages {
    use super::*;
    use crate::hub::client::Event;
    use crate::hub::protocol::Tab;
    use crate::orchestrator::config::Config;
    use crate::orchestrator::message::{self, Message, State};
    use crate::orchestrator::send;

    struct Hub {
        dir: tempfile::TempDir,
        endpoint: Endpoint,
        server: Option<thread::JoinHandle<()>>,
    }

    impl Hub {
        fn start(config: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("orchestrator.toml"), config).unwrap();
            let mut endpoint = Endpoint::in_dir(dir.path());
            // Nenhum teste roda o claude de verdade.
            endpoint.claude = Some(dir.path().join("sem-claude"));
            let server = {
                let endpoint = endpoint.clone();
                thread::spawn(move || daemon::serve(&endpoint, Duration::from_millis(300)).unwrap())
            };
            Self {
                dir,
                endpoint,
                server: Some(server),
            }
        }

        /// Uma janela com uma aba chamada `name` (o id da aba é `tab`).
        fn window(&self, tab: &str, name: &str) -> Reporter {
            let status = Status {
                chats: 1,
                tabs: vec![Tab {
                    id: tab.into(),
                    run: None,
                    title: "Principal".into(),
                    name: Some(name.into()),
                    task: None,
                }],
                ..Status::default()
            };
            let window = Reporter::start_with(
                self.endpoint.clone(),
                false,
                "w".into(),
                "/w".into(),
                status,
            );
            wait(5, "a janela no hub", || {
                instances(&self.endpoint)
                    .iter()
                    .any(|i| i.status.tabs.iter().any(|t| t.id == tab))
                    .then_some(())
            });
            window
        }

        /// Prepara a mensagem (com a nota) como o `terra send` faria.
        fn message(&self, to: &str, hops: u32) -> Message {
            let args = send::Args {
                to: to.into(),
                text: Some("Rode os testes".into()),
                ..send::Args::default()
            };
            let config = Config::default();
            send::prepare(
                &args,
                "usuario",
                hops,
                &config,
                self.dir.path(),
                self.dir.path(),
            )
            .unwrap()
        }

        fn send(&self, message: &Message) -> Value {
            client::send_message(&self.endpoint, false, &message.to_json()).unwrap()
        }

        fn state(&self, id: &str) -> Option<State> {
            message::state(self.dir.path(), id)
        }
    }

    impl Drop for Hub {
        fn drop(&mut self) {
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
        }
    }

    fn next_event(window: &Reporter) -> Event {
        wait(5, "um evento do hub", || window.events().into_iter().next())
    }

    #[test]
    fn a_message_reaches_the_named_tab_and_the_ack_marks_it_delivered() {
        let hub = Hub::start("");
        let window = hub.window("t1", "frontend");
        let msg = hub.message("frontend", 0);
        let reply = hub.send(&msg);
        assert_eq!(
            (reply["t"].as_str(), reply["state"].as_str()),
            (Some("sent"), Some("enviada"))
        );
        assert_eq!(hub.state(&msg.id), Some(State::Sent));

        let Event::Deliver { tab, message } = next_event(&window) else {
            panic!("esperava a entrega");
        };
        assert_eq!(tab, "t1");
        assert_eq!(message, msg);
        window.ack(&msg.id);
        wait(5, "o ack", || {
            (hub.state(&msg.id) == Some(State::Delivered)).then_some(())
        });
        drop(window);
    }

    #[test]
    fn a_name_belongs_to_one_tab_and_never_to_a_configured_agent() {
        // Pausado: a mensagem fica na fila (o despacho é testado em `dispatch`).
        let hub = Hub::start("[global]\npausado = true\n[agents.backend]\ncwd = \"/proj/api\"\n");
        let first = hub.window("t1", "frontend");
        let second = hub.window("t2", "frontend");
        let Event::NameRejected { tab, name, .. } = next_event(&second) else {
            panic!("esperava a recusa do nome");
        };
        assert_eq!((tab.as_str(), name.as_str()), ("t2", "frontend"));
        // O hub tirou o nome repetido da lista.
        let all = instances(&hub.endpoint);
        let named = all
            .iter()
            .flat_map(|i| &i.status.tabs)
            .filter(|t| t.name.is_some())
            .count();
        assert_eq!(named, 1);

        let third = hub.window("t3", "backend");
        let Event::NameRejected { reason, .. } = next_event(&third) else {
            panic!("esperava a recusa do nome da configuração");
        };
        assert!(reason.contains("configuração"));

        // Para o agente da configuração, a mensagem vai para a fila dele.
        let msg = hub.message("backend", 0);
        assert_eq!(hub.send(&msg)["state"], "na_fila");
        assert_eq!(hub.state(&msg.id), Some(State::Queued));
        drop((first, second, third));
    }

    #[test]
    fn too_many_hops_and_unknown_agents_are_rejected_by_the_hub() {
        let hub = Hub::start("[global]\nmax_hops = 2\n");
        let window = hub.window("t1", "frontend");
        // O `terra send` recusaria antes; aqui ela chega ao hub com 3 saltos.
        let mut looping = hub.message("frontend", 0);
        looping.hops = 3;
        let reply = hub.send(&looping);
        assert_eq!(
            (reply["t"].as_str(), reply["code"].as_str()),
            (Some("rejected"), Some("hops"))
        );
        assert_eq!(hub.state(&looping.id), Some(State::Rejected));

        let lost = hub.message("ninguem", 0);
        assert_eq!(hub.send(&lost)["code"], "agente");
        // Nada chegou à aba.
        thread::sleep(Duration::from_millis(200));
        assert!(window.events().is_empty());
        drop(window);
    }
}

// O despachante, com um `claude` falso (um script de shell): ele grava o
// prompt, os argumentos e o ambiente que recebeu e se comporta pelo arquivo
// `mode` da pasta do agente (`fail`, `wait` até existir `go`, ou sucesso).
#[cfg(unix)]
mod dispatch {
    use super::*;
    use crate::orchestrator::config::Config;
    use crate::orchestrator::message::{self, Message, State};
    use crate::orchestrator::run::{self, Run, RunState};
    use crate::orchestrator::send;

    const FAKE_CLAUDE: &str = r#"#!/bin/sh
cat > prompt.txt
echo "$@" > args.txt
env | grep '^TERRA_' | sort > env.txt
echo '{"type":"system","subtype":"init","session_id":"s","model":"claude-falso"}'
case "$(cat mode 2>/dev/null)" in
  fail)
    echo '{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":2,"errors":["Reached maximum number of turns (1)"]}'
    exit 1 ;;
  wait)
    while [ ! -f go ]; do sleep 0.05; done ;;
esac
echo '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":3,"output_tokens":4}}'
"#;

    struct Hub {
        dir: tempfile::TempDir,
        endpoint: Endpoint,
        server: Option<thread::JoinHandle<()>>,
        /// Uma janela aberta: sem ela, o hub sai quando nada roda.
        window: Option<client::Monitor>,
    }

    impl Hub {
        /// Um hub com os agentes `agents` (nome e `mode`), cada um na sua
        /// pasta, e o resto da configuração em `extra`.
        fn start(agents: &[(&str, &str)], extra: &str) -> Self {
            Self::start_with(agents, extra, |_| {})
        }

        /// Como o [`Hub::start`], com `seed` preparando a pasta antes do hub
        /// subir (o que um hub anterior deixou).
        fn start_with(agents: &[(&str, &str)], extra: &str, seed: impl FnOnce(&Path)) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join("claude");
            fs::write(&fake, FAKE_CLAUDE).unwrap();
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
            let mut config = format!("{extra}\n[modelos]\nforte = \"opus\"\n");
            for (name, mode) in agents {
                let cwd = dir.path().join(name);
                fs::create_dir(&cwd).unwrap();
                fs::write(cwd.join("mode"), mode).unwrap();
                config.push_str(&format!(
                    "[agents.{name}]\ncwd = \"{}\"\nmodelo = \"forte\"\n\
                     ferramentas = [\"Read\"]\nmax_turnos = 3\n",
                    cwd.display()
                ));
                if *name == "api" && extra.contains("orcamento") {
                    config.push_str("orcamento_diario = 1\n");
                }
            }
            fs::write(dir.path().join("orchestrator.toml"), config).unwrap();
            seed(dir.path());
            let mut endpoint = Endpoint::in_dir(dir.path());
            endpoint.claude = Some(fake);
            let server = {
                let endpoint = endpoint.clone();
                thread::spawn(move || daemon::serve(&endpoint, Duration::from_millis(300)).unwrap())
            };
            wait(5, "o hub subir", || {
                Stream::connect(endpoint.name().unwrap()).ok().map(|_| ())
            });
            let window = Some(client::Monitor::start_with(endpoint.clone(), false));
            Self {
                dir,
                endpoint,
                server: Some(server),
                window,
            }
        }

        fn home(&self) -> &Path {
            self.dir.path()
        }

        fn send(&self, from: &str, to: &str) -> (Message, Value) {
            let args = send::Args {
                to: to.into(),
                text: Some("Rode os testes".into()),
                ..send::Args::default()
            };
            let message =
                send::prepare(&args, from, 0, &Config::default(), self.home(), self.home())
                    .unwrap();
            let reply = client::send_message(&self.endpoint, false, &message.to_json()).unwrap();
            (message, reply)
        }

        /// A execução da mensagem `id`.
        fn run_of(&self, id: &str) -> Option<Run> {
            run::list(self.home())
                .into_iter()
                .find(|run| run.message == id)
        }

        fn wait_state(&self, id: &str, state: RunState) -> Run {
            wait(10, &format!("a execução ficar {}", state.key()), || {
                self.run_of(id).filter(|run| run.state == state)
            })
        }

        fn agent_file(&self, agent: &str, file: &str) -> String {
            fs::read_to_string(self.home().join(agent).join(file)).unwrap_or_default()
        }
    }

    impl Drop for Hub {
        fn drop(&mut self) {
            self.window.take();
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
        }
    }

    #[test]
    fn a_configured_agent_runs_the_message_with_the_fixed_prompt_and_its_usage_is_saved() {
        let hub = Hub::start(&[("api", "ok")], "");
        let (message, reply) = hub.send("usuario", "api");
        assert_eq!(reply["state"], "na_fila");
        let run = hub.wait_state(&message.id, RunState::Done);

        let usage = run.usage.clone().unwrap();
        assert_eq!((usage.input, usage.output, usage.turns), (3, 4, 1));
        assert!((usage.cost_usd - 0.01).abs() < 1e-9);
        assert_eq!(run.model.as_deref(), Some("claude-falso"));
        let output = run.dir(hub.home()).join(run::OUTPUT);
        assert!(fs::read_to_string(output).unwrap().contains("\"result\""));
        wait(5, "a mensagem concluída", || {
            (message::state(hub.home(), &message.id) == Some(State::Done)).then_some(())
        });

        let prompt = hub.agent_file("api", "prompt.txt");
        assert!(prompt.starts_with("Você é o agente `api`. Leia `"));
        assert!(prompt.contains(&message.note.display().to_string()));
        let args = hub.agent_file("api", "args.txt");
        for flag in [
            "-p",
            "--permission-mode dontAsk",
            "--model opus",
            "--max-turns 3",
            "--allowedTools Read",
        ] {
            assert!(args.contains(flag), "{flag} em {args}");
        }
        let env = hub.agent_file("api", "env.txt");
        assert!(env.contains("TERRA_AGENT=api"));
        assert!(env.contains("TERRA_HOPS=1"));
        assert!(env.contains(&format!("TERRA_RUN={}", run.id)));
    }

    #[test]
    fn two_sends_to_the_same_agent_run_one_after_the_other() {
        let hub = Hub::start(&[("api", "wait")], "");
        let (first, _) = hub.send("usuario", "api");
        let (second, _) = hub.send("usuario", "api");
        hub.wait_state(&first.id, RunState::Running);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(hub.run_of(&second.id).unwrap().state, RunState::Queued);

        fs::write(hub.home().join("api").join("go"), "").unwrap();
        let first = hub.wait_state(&first.id, RunState::Done);
        let second = hub.wait_state(&second.id, RunState::Done);
        assert!(second.started.unwrap() >= first.finished.unwrap());
    }

    #[test]
    fn an_exhausted_budget_blocks_new_runs() {
        let hub = Hub::start(&[("api", "ok")], "# orcamento");
        let (first, _) = hub.send("usuario", "api");
        hub.wait_state(&first.id, RunState::Done);
        let (second, reply) = hub.send("usuario", "api");
        assert_eq!(
            (reply["t"].as_str(), reply["code"].as_str()),
            (Some("rejected"), Some("orcamento"))
        );
        assert_eq!(
            message::state(hub.home(), &second.id),
            Some(State::Rejected)
        );
        assert_eq!(hub.run_of(&second.id).unwrap().state, RunState::Rejected);
        let log = fs::read_to_string(&hub.endpoint.log).unwrap();
        assert!(log.contains("orçamento de api esgotado"), "{log}");
    }

    #[test]
    fn a_failed_level_1_run_asks_the_sender_for_a_full_handoff() {
        let hub = Hub::start(&[("api", "fail"), ("web", "ok")], "");
        let (message, _) = hub.send("web", "api");
        let failed = hub.wait_state(&message.id, RunState::Failed);
        assert_eq!(
            failed.error.as_deref(),
            Some("Reached maximum number of turns (1)")
        );

        // O terra pede ao remetente (web, um agente da configuração) a nota
        // completa, numa execução dele.
        let asked = wait(10, "o pedido de handoff", || {
            fs::read_dir(hub.home().join("messages"))
                .ok()?
                .flatten()
                .filter_map(|entry| message::load(hub.home(), &entry.file_name().to_string_lossy()))
                .find(|m| m.from == "terra")
        });
        assert_eq!((asked.to.as_str(), asked.level, asked.hops), ("web", 1, 2));
        assert!(asked.text.unwrap().contains("terra handoff new api"));
        hub.wait_state(&asked.id, RunState::Done);
    }

    #[test]
    fn paused_dispatches_wait_and_resume_when_the_switch_is_turned_off() {
        let hub = Hub::start(&[("api", "ok")], "[global]\npausado = true\n");
        let (message, reply) = hub.send("usuario", "api");
        assert_eq!(reply["state"], "na_fila");
        thread::sleep(Duration::from_millis(1500));
        assert_eq!(hub.run_of(&message.id).unwrap().state, RunState::Queued);

        let path = hub.home().join("orchestrator.toml");
        let config = fs::read_to_string(&path)
            .unwrap()
            .replace("pausado = true", "pausado = false");
        fs::write(&path, config).unwrap();
        hub.wait_state(&message.id, RunState::Done);
    }

    #[test]
    fn a_new_hub_resumes_the_queue_and_fails_what_was_running() {
        let mut ids = (String::new(), String::new());
        let hub = Hub::start_with(&[("api", "ok")], "", |home| {
            // O que o hub anterior deixou: uma execução na fila e uma rodando.
            let args = send::Args {
                to: "api".into(),
                text: Some("x".into()),
                ..send::Args::default()
            };
            let config = Config::default();
            for (i, state) in [RunState::Queued, RunState::Running]
                .into_iter()
                .enumerate()
            {
                let message = send::prepare(&args, "usuario", 0, &config, home, home).unwrap();
                message::save(&message, State::Queued, None, home).unwrap();
                let run = Run {
                    id: format!("{i}a"),
                    agent: "api".into(),
                    message: message.id.clone(),
                    from: "usuario".into(),
                    level: 1,
                    hops: 1,
                    note: message.note.clone(),
                    state,
                    created: protocol::now(),
                    started: None,
                    finished: None,
                    model: None,
                    usage: None,
                    error: None,
                    summary: None,
                };
                run.save(home).unwrap();
                match state {
                    RunState::Queued => ids.0 = message.id,
                    _ => ids.1 = message.id,
                }
            }
        });
        hub.wait_state(&ids.0, RunState::Done);
        let lost = hub.run_of(&ids.1).unwrap();
        assert_eq!(lost.state, RunState::Failed);
        assert_eq!(lost.error.as_deref(), Some("o hub saiu durante a execução"));
        assert_eq!(message::state(hub.home(), &ids.1), Some(State::Failed));
    }
}
