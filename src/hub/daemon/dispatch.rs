//! O despachante: as mensagens para os agentes da configuração viram
//! execuções do `claude` em modo não interativo.
//!
//! - Uma fila por agente e no máximo uma execução rodando por agente.
//! - No máximo `max_execucoes_simultaneas` rodando ao todo; com `pausado`,
//!   nenhuma começa (as que rodam terminam).
//! - O orçamento do dia (`orcamento_diario`, em execuções) recusa a
//!   mensagem quando acaba, e avisa a 80%.
//! - Uma execução que passa do `tempo_limite_min` é encerrada (`timeout`),
//!   a não ser com `tempo_limite_min = "infinito"`.
//! - Uma execução de nível 1 ou 2 que falha pede ao remetente, num nível 1,
//!   o handoff completo (`escalonar_falhas`); a nota dele volta no nível 3.
//!
//! O hub não espera as execuções numa thread: o laço principal chama
//! [`Hub::poll_runs`], que vê quem terminou e começa as próximas.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use super::Hub;
use crate::agent::adapters::{antigravity, claude};
use crate::agent::{kill_tree, own_group};
use crate::hub::protocol::{Health, new_id, now};
use crate::orchestrator::config::{Config, RunMode};
use crate::orchestrator::message::{self, Message, Reject, State as MessageState};
use crate::orchestrator::run::{self, Run, RunState};
use crate::orchestrator::{AGENT_ENV, HOPS_ENV, RUN_ENV, USER, note};

/// O remetente das mensagens do próprio terra (o escalonamento).
const TERRA: &str = "terra";

/// Uma execução rodando.
struct Running {
    run: Run,
    child: Child,
    /// `None`: sem tempo limite.
    deadline: Option<Instant>,
    /// No Antigravity, o plugin com o MCP do orquestrador; a pasta some
    /// junto com a execução.
    _overlay: Option<antigravity::Overlay>,
}

#[derive(Default)]
pub(super) struct Dispatcher {
    /// As execuções esperando, por agente, na ordem em que chegaram.
    queues: BTreeMap<String, VecDeque<Run>>,
    /// A execução rodando de cada agente.
    running: BTreeMap<String, Running>,
}

impl Dispatcher {
    /// Há execução rodando: o hub não sai sozinho. A fila não segura o hub
    /// (pausada, ela esperaria para sempre): ela está em disco, e o próximo
    /// hub a retoma ([`Hub::recover`]).
    pub(super) fn busy(&self) -> bool {
        !self.running.is_empty()
    }
}

impl Hub {
    fn dispatcher(&self) -> std::sync::MutexGuard<'_, Dispatcher> {
        self.dispatcher.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn busy(&self) -> bool {
        self.dispatcher().busy()
    }

    /// A mensagem para o agente da configuração vira uma execução na fila
    /// dele, se o orçamento do dia deixa.
    pub(super) fn enqueue(&self, message: &Message, config: &Config) -> Result<(), Reject> {
        let mut run = Run {
            id: new_id(),
            agent: message.to.clone(),
            message: message.id.clone(),
            from: message.from.clone(),
            level: message.level,
            hops: message.hops,
            note: message.note.clone(),
            state: RunState::Queued,
            created: now(),
            started: None,
            finished: None,
            provider: config
                .agent(&message.to)
                .map_or("claude", |a| a.provider())
                .to_string(),
            model: None,
            usage: None,
            error: None,
            summary: None,
        };
        if let Some(limit) = config.agent(&message.to).and_then(|a| a.daily_budget) {
            let used = run::count_on(&run::list(&self.home), &message.to, run::today());
            if used >= limit {
                run.state = RunState::Rejected;
                run.error = Some("orçamento do dia esgotado".into());
                self.save_run(&run);
                self.budget_alert(&message.to, used, limit, true);
                return Err(Reject::new(
                    "orcamento",
                    format!(
                        "o agente {} já usou as {limit} execuções de hoje (orcamento_diario)",
                        message.to
                    ),
                ));
            }
            // Esta é a `used + 1`: a partir de 80%, o aviso.
            if (used + 1) * 5 >= limit * 4 {
                self.budget_alert(&message.to, used + 1, limit, false);
            }
        }
        self.save_run(&run);
        self.dispatcher()
            .queues
            .entry(run.agent.clone())
            .or_default()
            .push_back(run);
        self.pump();
        Ok(())
    }

    /// O aviso de orçamento: no log e aos monitores (a aba mostra).
    fn budget_alert(&self, agent: &str, used: u32, limit: u32, blocked: bool) {
        let what = if blocked { "esgotado" } else { "a 80%" };
        self.log.line(&format!(
            "orçamento de {agent} {what}: {used} de {limit} execuções hoje"
        ));
        self.state().broadcast(&json!({
            "t": "budget",
            "agent": agent,
            "used": used,
            "limit": limit,
            "blocked": blocked,
        }));
    }

    /// Começa as execuções que podem começar: uma por agente, até o limite
    /// global, e nenhuma com os despachos pausados.
    pub(super) fn pump(&self) {
        let config = self.config();
        if config.global.paused {
            return;
        }
        loop {
            let next = {
                let mut dispatcher = self.dispatcher();
                if dispatcher.running.len() >= config.global.max_concurrent as usize {
                    return;
                }
                let free = dispatcher
                    .queues
                    .iter()
                    .find(|(agent, queue)| {
                        !queue.is_empty() && !dispatcher.running.contains_key(*agent)
                    })
                    .map(|(agent, _)| agent.clone());
                let Some(agent) = free else {
                    return;
                };
                dispatcher
                    .queues
                    .get_mut(&agent)
                    .and_then(VecDeque::pop_front)
            };
            if let Some(run) = next {
                self.start(run, &config);
            }
        }
    }

    /// Lança o agente da execução (o `claude` ou, no Antigravity, o `agy`)
    /// na pasta dele, com a saída em `runs/<id>/output.jsonl` e o prompt
    /// pela entrada.
    fn start(&self, mut run: Run, config: &Config) {
        let Some(agent) = config.agent(&run.agent) else {
            return self.fail(run, "o agente saiu do orchestrator.toml");
        };
        // O provedor de agora: a configuração pode ter mudado na fila.
        run.provider = agent.provider().to_string();
        let (bin, command) = match &self.claude {
            Some(fake) => ("claude", Some(std::process::Command::new(fake))),
            None if agent.is_antigravity() => ("agy", antigravity::run_command()),
            None => ("claude", claude::run_command()),
        };
        let Some(mut cmd) = command else {
            return self.fail(run, &format!("o {bin} não está no PATH"));
        };
        let dir = run.dir(&self.home);
        let files = fs::create_dir_all(&dir).and_then(|()| {
            Ok((
                File::create(dir.join(run::OUTPUT))?,
                File::create(dir.join(run::STDERR))?,
            ))
        });
        let (stdout, stderr) = match files {
            Ok(files) => files,
            Err(err) => return self.fail(run, &format!("{}: {err}", dir.display())),
        };
        // O MCP do orquestrador vai em toda execução: no Claude, pelo
        // `--mcp-config`; no Antigravity, por um plugin no `--add-dir`.
        let mcp = run::terra_mcp(&run, &agent.cwd);
        cmd.args(run::args(agent, config, &mcp));
        let overlay = if agent.is_antigravity() {
            match antigravity::Overlay::create(&run::contribution(&mcp), &Default::default()) {
                Ok(overlay) => overlay,
                Err(err) => return self.fail(run, &err),
            }
        } else {
            None
        };
        if let Some(overlay) = &overlay {
            cmd.arg("--add-dir").arg(overlay.dir());
        }
        cmd.current_dir(&agent.cwd)
            .env(AGENT_ENV, &run.agent)
            .env(HOPS_ENV, run.hops.to_string())
            .env(RUN_ENV, &run.id)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(stderr);
        // Num grupo próprio, para o timeout levar junto o que ele abrir.
        own_group(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                let cwd = agent.cwd.display();
                return self.fail(
                    run,
                    &format!("não foi possível iniciar o {bin} em {cwd}: {err}"),
                );
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let note = fs::read_to_string(&run.note).ok();
            let prompt = run::prompt(&run.agent, &run.note, note.as_deref());
            let _ = stdin.write_all(run::input(agent, &prompt).as_bytes());
        }
        run.state = RunState::Running;
        run.started = Some(now());
        run.model = agent
            .model
            .as_deref()
            .map(|m| config.model_id(m).to_string());
        self.save_run(&run);
        self.message_state(&run, MessageState::Running, None);
        if config.global.run_mode == RunMode::Tab {
            self.show_run(&run, &agent.cwd.to_string_lossy());
        }
        let deadline = config
            .global
            .timeout_min
            .map(|min| Instant::now() + Duration::from_secs(u64::from(min) * 60));
        self.dispatcher().running.insert(
            run.agent.clone(),
            Running {
                run,
                child,
                deadline,
                _overlay: overlay,
            },
        );
    }

    /// No modo `aba`, a execução aparece numa aba só de leitura: na janela
    /// aberta na pasta do agente ou, sem ela, na janela mais nova.
    fn show_run(&self, run: &Run, cwd: &str) {
        let state = self.state();
        let alive = state
            .instances
            .iter()
            .filter(|(_, entry)| entry.health != Health::Gone)
            .map(|(_, entry)| entry);
        let window = alive
            .clone()
            .find(|entry| entry.info["cwd"] == cwd)
            .or_else(|| alive.max_by_key(|entry| entry.info["started"].as_u64()));
        if let Some(window) = window {
            let output = run.dir(&self.home).join(run::OUTPUT);
            window
                .outbox
                .push(json!({"t": "run_started", "run": run.to_json(), "output": output}));
        }
    }

    /// Vê as execuções que terminaram (ou passaram do tempo), registra e
    /// começa as próximas.
    pub(super) fn poll_runs(&self) {
        let finished: Vec<(Run, bool)> = {
            let mut dispatcher = self.dispatcher();
            let mut finished = Vec::new();
            let agents: Vec<String> = dispatcher.running.keys().cloned().collect();
            for agent in agents {
                let Some(running) = dispatcher.running.get_mut(&agent) else {
                    continue;
                };
                let timed_out = match running.child.try_wait() {
                    Ok(Some(_)) | Err(_) => false,
                    Ok(None) if running.deadline.is_some_and(|d| Instant::now() >= d) => {
                        kill_tree(&mut running.child);
                        true
                    }
                    Ok(None) => continue,
                };
                if let Some(running) = dispatcher.running.remove(&agent) {
                    finished.push((running.run, timed_out));
                }
            }
            finished
        };
        let any = !finished.is_empty();
        for (run, timed_out) in finished {
            self.finish(run, timed_out);
        }
        if any {
            self.pump();
        }
    }

    /// O fim de uma execução: o resultado, o uso e o erro da saída do
    /// agente, e o escalonamento quando ela falhou.
    fn finish(&self, mut run: Run, timed_out: bool) {
        let dir = run.dir(&self.home);
        let output = fs::read_to_string(dir.join(run::OUTPUT)).unwrap_or_default();
        let outcome = run::outcome(&output, run.is_antigravity());
        run.finished = Some(now());
        run.usage = outcome.usage;
        if outcome.model.is_some() {
            run.model = outcome.model;
        }
        run.state = match (timed_out, outcome.success) {
            (true, _) => RunState::Timeout,
            (false, true) => RunState::Done,
            (false, false) => RunState::Failed,
        };
        run.error = match run.state {
            RunState::Timeout => Some("passou do tempo_limite_min".into()),
            RunState::Failed => outcome.error.or_else(|| {
                let stderr = fs::read_to_string(dir.join(run::STDERR)).unwrap_or_default();
                stderr
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_string)
            }),
            _ => None,
        };
        self.save_run(&run);
        let state = match run.state {
            RunState::Done => MessageState::Done,
            _ => MessageState::Failed,
        };
        let reason = run.error.as_deref().map(|e| Reject::new("execucao", e));
        self.message_state(&run, state, reason.as_ref());
        if run.state != RunState::Done {
            self.escalate(&run);
        }
    }

    /// Uma execução que nem começou.
    fn fail(&self, mut run: Run, error: &str) {
        run.state = RunState::Failed;
        run.finished = Some(now());
        run.error = Some(error.to_string());
        self.save_run(&run);
        let reason = Reject::new("execucao", error);
        self.message_state(&run, MessageState::Failed, Some(&reason));
        self.escalate(&run);
    }

    /// A execução de nível 1 ou 2 falhou: o remetente recebe, num nível 1,
    /// o pedido do handoff completo. Um agente da configuração o processa
    /// numa execução; uma aba manual o recebe na caixa. O usuário (fora de
    /// um agente) e o próprio terra não recebem nada: fica o log.
    fn escalate(&self, run: &Run) {
        let config = self.config();
        if run.level > 2 || run.from == TERRA || !config.routing.escalate_failures {
            return;
        }
        if run.from == USER {
            self.log.line(&format!(
                "execução {} de {} falhou; o remetente é o usuário, sem escalonamento",
                short(&run.id),
                run.agent
            ));
            return;
        }
        let id = new_id();
        let text = format!(
            "A execução {} do agente {} falhou ({}) com a sua mensagem {}. Escreva uma nota \
             completa com `terra handoff new {}` e mande com `terra send {} --handoff <arquivo>`.",
            short(&run.id),
            run.agent,
            run.error.as_deref().unwrap_or("sem detalhe"),
            short(&run.message),
            run.agent,
            run.agent,
        );
        let escalation = Message {
            id: id.clone(),
            from: TERRA.into(),
            to: run.from.clone(),
            level: 1,
            hops: run.hops + 1,
            tag: Some("escalonamento".into()),
            text: Some(text.clone()),
            refs: Vec::new(),
            note: message::dir(&self.home, &id).join("note.md"),
            created: now(),
        };
        let note = note::mini(&id, TERRA, &run.from, &text, &[]);
        let written = fs::create_dir_all(message::dir(&self.home, &id))
            .and_then(|()| fs::write(&escalation.note, note));
        if let Err(err) = written {
            self.log.line(&format!("não foi possível escalonar: {err}"));
            return;
        }
        // O mesmo caminho de qualquer mensagem: as regras e a entrega.
        let _ = self.route(&escalation);
    }

    /// O `terra done` de dentro de uma execução: guarda o resumo.
    pub(super) fn done(&self, run_id: &str, summary: Option<String>) -> Result<(), Reject> {
        let mut dispatcher = self.dispatcher();
        let running = dispatcher
            .running
            .values_mut()
            .find(|running| running.run.id == run_id)
            .ok_or_else(|| Reject::new("execucao", "esta execução não está rodando"))?;
        running.run.summary = Some(summary.unwrap_or_default());
        let run = running.run.clone();
        drop(dispatcher);
        self.save_run(&run);
        Ok(())
    }

    /// Ao subir, o hub retoma as execuções que ficaram na fila e marca como
    /// falhas as que rodavam quando o hub anterior saiu.
    pub(super) fn recover(&self) {
        for mut run in run::list(&self.home) {
            match run.state {
                RunState::Queued => self
                    .dispatcher()
                    .queues
                    .entry(run.agent.clone())
                    .or_default()
                    .push_back(run),
                RunState::Running => {
                    run.state = RunState::Failed;
                    run.finished = Some(now());
                    run.error = Some("o hub saiu durante a execução".into());
                    self.save_run(&run);
                    let reason = Reject::new("execucao", "o hub saiu durante a execução");
                    self.message_state(&run, MessageState::Failed, Some(&reason));
                }
                _ => {}
            }
        }
        self.pump();
    }

    /// Grava a execução, anota no log e conta aos monitores.
    fn save_run(&self, run: &Run) {
        if let Err(err) = run.save(&self.home) {
            self.log
                .line(&format!("não foi possível gravar a execução: {err}"));
        }
        self.log.line(&format!(
            "execução {} de {} (mensagem {}): {}{}",
            short(&run.id),
            run.agent,
            short(&run.message),
            run.state.key(),
            run.error
                .as_deref()
                .map_or(String::new(), |e| format!(" ({e})")),
        ));
        self.state()
            .broadcast(&json!({"t": "run", "run": run.to_json()}));
    }

    /// O estado da mensagem da execução.
    fn message_state(&self, run: &Run, state: MessageState, reason: Option<&Reject>) {
        if let Some(message) = message::load(&self.home, &run.message) {
            self.record(&message, state, reason);
        }
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}
