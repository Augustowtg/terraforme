//! Instâncias de agente registradas pelos terminais (`terraforme cli`) de
//! todos os spaces e as mensagens trocadas entre elas.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;
use tracing::info;

use terraforme_core::pet::Activity;

use crate::tools::http_post_json;

/// Mensagens guardadas para a UI (as mais antigas saem primeiro).
const MAX_MESSAGES: usize = 500;

/// Destino especial: a mensagem só aparece na UI web.
pub const WEB: &str = "web";

/// Instâncias por terminal (o `MAX_INSTANCES` da TUI).
const MAX_PER_TERMINAL: usize = 9;

/// Quanto esperar a instância aberta por [`Hub::open`] se registrar.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Quanto quem delegou espera o resultado ([`Hub::wait`]); depois disso o
/// resultado chega como um prompt comum.
pub const DELEGATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Resultado de uma tarefa delegada: o texto devolvido ou o motivo de não
/// haver um.
pub type Outcome = Result<String, String>;

/// Quem espera o resultado de uma tarefa delegada.
struct Waiter {
    /// Id da instância que delegou.
    caller: String,
    tx: oneshot::Sender<Outcome>,
}

/// Pedido de registro de uma instância, enviado pelo terminal.
#[derive(Debug, Deserialize)]
pub struct Registration {
    pub instance_id: String,
    pub client_id: String,
    pub control_addr: String,
    /// Agente da instância (`claude`, `opencode`…).
    pub agent: String,
    /// Número da sub-aba no terminal (`claude 2` → 2).
    pub number: usize,
    /// Nome do space; vazio fora de um space.
    #[serde(default)]
    pub space: String,
    pub cwd: String,
}

/// Instância conhecida pelo daemon.
#[derive(Debug, Clone, Serialize)]
pub struct Instance {
    pub id: String,
    /// Apelido único usado para endereçar mensagens (`space/claude-1`).
    pub handle: String,
    #[serde(skip)]
    pub client_id: String,
    #[serde(skip)]
    pub control_addr: String,
    pub agent: String,
    pub space: String,
    pub cwd: String,
    /// Número da sub-aba no terminal.
    pub number: usize,
    /// Segundos desde a época Unix.
    pub since: u64,
    /// O que a instância está fazendo, informado pelo terminal.
    pub activity: Activity,
    /// Segundos desde a época Unix da última mudança de `activity`.
    pub changed: u64,
}

/// Mensagem trocada (ou tentada) entre instâncias.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub at: u64,
    pub from: String,
    pub to: String,
    pub text: String,
    /// Motivo da falha na entrega; `None` quando foi entregue.
    pub error: Option<String>,
}

#[derive(Default)]
struct State {
    instances: BTreeMap<String, Instance>,
    messages: VecDeque<Message>,
    /// Tarefas delegadas em andamento, pelo id da instância que trabalha.
    waiters: BTreeMap<String, Waiter>,
}

#[derive(Clone, Default)]
pub struct Hub {
    state: Arc<Mutex<State>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Parte do apelido sem espaços nem barras (`Meu Space` → `meu-space`).
fn slug(text: &str) -> String {
    let slug: String = text
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-').to_string()
}

/// Apelido da instância: `<space ou pasta>/<agente>-<n>`.
fn base_handle(reg: &Registration) -> String {
    let place = if reg.space.is_empty() {
        std::path::Path::new(&reg.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        reg.space.clone()
    };
    let place = match slug(&place) {
        p if p.is_empty() => "sem-space".to_string(),
        p => p,
    };
    format!("{place}/{}-{}", slug(&reg.agent), reg.number)
}

impl Hub {
    /// Registra (ou atualiza) a instância e devolve o apelido dela.
    pub fn register(&self, reg: Registration) -> String {
        let mut state = self.state.lock().unwrap();
        let base = base_handle(&reg);
        let taken = |h: &str| {
            state
                .instances
                .values()
                .any(|i| i.handle == h && i.id != reg.instance_id)
        };
        let mut handle = base.clone();
        let mut n = 2;
        while taken(&handle) {
            handle = format!("{base}#{n}");
            n += 1;
        }
        info!(
            handle,
            instance_id = reg.instance_id,
            "instância registrada"
        );
        let (activity, changed) = state
            .instances
            .get(&reg.instance_id)
            .map_or((Activity::Idle, now()), |i| (i.activity, i.changed));
        state.instances.insert(
            reg.instance_id.clone(),
            Instance {
                id: reg.instance_id,
                handle: handle.clone(),
                client_id: reg.client_id,
                control_addr: reg.control_addr,
                agent: reg.agent,
                space: reg.space,
                cwd: reg.cwd,
                number: reg.number,
                since: now(),
                activity,
                changed,
            },
        );
        handle
    }

    /// Guarda o estado de atividade informado pelo terminal da instância.
    pub fn set_activity(&self, instance_id: &str, activity: Activity) {
        if let Some(i) = self.state.lock().unwrap().instances.get_mut(instance_id)
            && i.activity != activity
        {
            i.activity = activity;
            i.changed = now();
        }
    }

    /// Pede ao terminal da instância para abrir a sub-aba dela.
    pub async fn focus(&self, instance_id: &str) -> Result<(), String> {
        let target = self
            .get(instance_id)
            .ok_or_else(|| format!("instância '{instance_id}' não encontrada"))?;
        let body = json!({ "instance_id": target.id });
        http_post_json(&target.control_addr, "/focus", &body)
            .await
            .map_err(|e| format!("terminal de {} não respondeu: {e}", target.handle))
    }

    pub fn unregister(&self, instance_id: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(i) = state.instances.remove(instance_id) {
            info!(handle = i.handle, "instância encerrada");
        }
        // Quem esperava a tarefa dela recebe o erro (o `tx` cai).
        state.waiters.remove(instance_id);
    }

    /// Remove as instâncias de um terminal que saiu.
    pub fn drop_client(&self, client_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.instances.retain(|_, i| i.client_id != client_id);
        let State {
            instances, waiters, ..
        } = &mut *state;
        waiters.retain(|id, _| instances.contains_key(id));
    }

    /// Passa a esperar o resultado da tarefa que `caller` delegou a
    /// `worker`: a próxima `send_message` de `worker` para `caller` chega
    /// por aqui em vez de ser digitada.
    fn expect(&self, worker: &str, caller: &str) -> oneshot::Receiver<Outcome> {
        let (tx, rx) = oneshot::channel();
        let waiter = Waiter {
            caller: caller.into(),
            tx,
        };
        self.state
            .lock()
            .unwrap()
            .waiters
            .insert(worker.into(), waiter);
        rx
    }

    /// Espera o resultado de `worker` por até [`DELEGATE_TIMEOUT`]; `None`
    /// quando o tempo acaba (o resultado então chega como prompt).
    pub async fn wait(&self, worker: &Instance, rx: oneshot::Receiver<Outcome>) -> Option<Outcome> {
        match tokio::time::timeout(DELEGATE_TIMEOUT, rx).await {
            Ok(Ok(outcome)) => Some(outcome),
            Ok(Err(_)) => Some(Err(format!(
                "{} fechou sem devolver o resultado",
                worker.handle
            ))),
            Err(_) => {
                self.state.lock().unwrap().waiters.remove(&worker.id);
                None
            }
        }
    }

    /// Entrega `text` a quem espera o resultado de `from`, se `to` for quem
    /// delegou. Devolve o texto de volta quando ninguém espera.
    fn resolve(&self, from: &str, to: &str, text: String) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        if state.waiters.get(from).is_none_or(|w| w.caller != to) {
            return Some(text);
        }
        let waiter = state.waiters.remove(from)?;
        // Quem delegou desistiu (chamada cancelada): vira prompt comum.
        waiter.tx.send(Ok(text)).err().and_then(Result::ok)
    }

    /// Instâncias ordenadas pelo apelido.
    pub fn instances(&self) -> Vec<Instance> {
        let mut list: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .instances
            .values()
            .cloned()
            .collect();
        list.sort_by(|a, b| a.handle.cmp(&b.handle));
        list
    }

    pub fn get(&self, instance_id: &str) -> Option<Instance> {
        self.state
            .lock()
            .unwrap()
            .instances
            .get(instance_id)
            .cloned()
    }

    /// Instância pelo apelido (sem diferenciar maiúsculas) ou pelo id.
    pub fn find(&self, to: &str) -> Option<Instance> {
        let to = to.trim();
        self.state
            .lock()
            .unwrap()
            .instances
            .values()
            .find(|i| i.id == to || i.handle.eq_ignore_ascii_case(to))
            .cloned()
    }

    pub fn messages(&self) -> Vec<Message> {
        self.state
            .lock()
            .unwrap()
            .messages
            .iter()
            .cloned()
            .collect()
    }

    fn log(&self, message: Message) {
        let mut state = self.state.lock().unwrap();
        if state.messages.len() >= MAX_MESSAGES {
            state.messages.pop_front();
        }
        state.messages.push_back(message);
    }

    /// Entrega `text` de `from` (apelido, ou [`WEB`] para a UI) à instância
    /// `to`: o terminal dela digita a mensagem como um prompt. Devolve o
    /// apelido do destino.
    pub async fn send(&self, from: &str, to: &str, text: &str) -> Result<String, String> {
        self.deliver(from, to, text, prompt).await
    }

    /// Como [`Hub::send`], mas a mensagem chega como uma tarefa delegada,
    /// com o pedido de devolver o resultado a `from`, cuja resposta chega
    /// pelo receptor devolvido (ver [`Hub::wait`]).
    pub async fn delegate(
        &self,
        from: &Instance,
        to: &str,
        task: &str,
    ) -> Result<(Instance, oneshot::Receiver<Outcome>), String> {
        let target = self
            .find(to)
            .ok_or_else(|| format!("instância '{to}' não encontrada — veja list_instances"))?;
        if target.id == from.id {
            return Err("a instância não pode delegar a si mesma".into());
        }
        let rx = self.expect(&target.id, &from.id);
        if let Err(e) = self
            .deliver(&from.handle, &target.id, task, delegation)
            .await
        {
            self.state.lock().unwrap().waiters.remove(&target.id);
            return Err(e);
        }
        Ok((target, rx))
    }

    /// Pede ao terminal de `from` uma instância nova do mesmo agente e
    /// espera ela se registrar. Com `text`, ela recebe o texto como
    /// mensagem de `from`.
    pub async fn open(&self, from: &Instance, text: Option<&str>) -> Result<Instance, String> {
        let instance_id = uuid::Uuid::new_v4().to_string();
        self.spawn(from, &instance_id, text, false).await
    }

    /// Como [`Hub::open`], mas o texto chega como tarefa delegada e a
    /// resposta pelo receptor devolvido (ver [`Hub::wait`]).
    pub async fn open_delegated(
        &self,
        from: &Instance,
        task: &str,
    ) -> Result<(Instance, oneshot::Receiver<Outcome>), String> {
        let instance_id = uuid::Uuid::new_v4().to_string();
        let rx = self.expect(&instance_id, &from.id);
        match self.spawn(from, &instance_id, Some(task), true).await {
            Ok(opened) => Ok((opened, rx)),
            Err(e) => {
                self.state.lock().unwrap().waiters.remove(&instance_id);
                Err(e)
            }
        }
    }

    async fn spawn(
        &self,
        from: &Instance,
        instance_id: &str,
        text: Option<&str>,
        delegated: bool,
    ) -> Result<Instance, String> {
        let open = self
            .instances()
            .iter()
            .filter(|i| i.client_id == from.client_id)
            .count();
        if open >= MAX_PER_TERMINAL {
            return Err(format!(
                "o terminal de {} já tem {MAX_PER_TERMINAL} instâncias — feche uma com close_instance",
                from.handle
            ));
        }
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        let body = json!({
            "instance_id": instance_id,
            "from": from.handle,
            "text": text.map(|t| if delegated {
                delegation(&from.handle, t)
            } else {
                prompt(&from.handle, t)
            }),
        });
        http_post_json(&from.control_addr, "/open", &body)
            .await
            .map_err(|e| format!("terminal de {} não respondeu: {e}", from.handle))?;
        let deadline = Instant::now() + OPEN_TIMEOUT;
        let opened = loop {
            if let Some(opened) = self.get(instance_id) {
                break opened;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "o terminal de {} não abriu a instância em {}s (veja o aviso na TUI)",
                    from.handle,
                    OPEN_TIMEOUT.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if let Some(text) = text {
            self.log(Message {
                at: now(),
                from: from.handle.clone(),
                to: opened.handle.clone(),
                text: text.into(),
                error: None,
            });
        }
        Ok(opened)
    }

    /// Pede ao terminal da instância `to` para encerrá-la. Devolve o apelido
    /// dela.
    pub async fn close(&self, from: &str, to: &str) -> Result<String, String> {
        let target = self
            .find(to)
            .ok_or_else(|| format!("instância '{to}' não encontrada — veja list_instances"))?;
        if target.handle == from {
            return Err("a instância não pode fechar a si mesma".into());
        }
        let body = json!({ "instance_id": target.id, "from": from });
        http_post_json(&target.control_addr, "/close", &body)
            .await
            .map_err(|e| format!("terminal de {} não respondeu: {e}", target.handle))?;
        self.log(Message {
            at: now(),
            from: from.into(),
            to: target.handle.clone(),
            text: "[instância fechada]".into(),
            error: None,
        });
        Ok(target.handle)
    }

    async fn deliver(
        &self,
        from: &str,
        to: &str,
        text: &str,
        wrap: fn(&str, &str) -> String,
    ) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("mensagem vazia".into());
        }
        if to.trim().eq_ignore_ascii_case(WEB) {
            self.log(Message {
                at: now(),
                from: from.into(),
                to: WEB.into(),
                text: text.into(),
                error: None,
            });
            return Ok(WEB.into());
        }
        let Some(target) = self.find(to) else {
            let error = format!("instância '{to}' não encontrada — veja list_instances");
            self.log(Message {
                at: now(),
                from: from.into(),
                to: to.into(),
                text: text.into(),
                error: Some(error.clone()),
            });
            return Err(error);
        };
        if target.handle == from {
            return Err("a instância não pode mandar mensagem para si mesma".into());
        }
        // Resultado de uma tarefa delegada: volta na chamada de quem delegou.
        let sender = self.find(from).map(|i| i.id).unwrap_or_default();
        let Some(text) = self.resolve(&sender, &target.id, text.to_string()) else {
            self.log(Message {
                at: now(),
                from: from.into(),
                to: target.handle.clone(),
                text: text.into(),
                error: None,
            });
            return Ok(target.handle);
        };
        let text = text.as_str();
        let body = json!({
            "instance_id": target.id,
            "from": from,
            "text": wrap(from, text),
        });
        let error = http_post_json(&target.control_addr, "/message", &body)
            .await
            .err()
            .map(|e| format!("terminal de {} não respondeu: {e}", target.handle));
        self.log(Message {
            at: now(),
            from: from.into(),
            to: target.handle.clone(),
            text: text.into(),
            error: error.clone(),
        });
        match error {
            Some(error) => Err(error),
            None => Ok(target.handle),
        }
    }
}

/// O que é digitado na instância de destino: a mensagem com o remetente e
/// como responder.
fn prompt(from: &str, text: &str) -> String {
    let from_label = if from == WEB {
        "usuário (UI web)"
    } else {
        from
    };
    format!(
        "[terraforme] Mensagem de {from_label}:\n{text}\n\n\
         Para responder, use a ferramenta send_message do MCP terraforme com to=\"{from}\"."
    )
}

/// O que é digitado na instância que recebe uma tarefa: a tarefa e o pedido
/// de devolver o resultado a quem delegou.
fn delegation(from: &str, task: &str) -> String {
    format!(
        "[terraforme] Tarefa delegada por {from}:\n{task}\n\n\
         {from} está esperando: ao terminar, mande o resultado final (ou o que impediu a \
         tarefa) numa única chamada da ferramenta send_message do MCP terraforme com \
         to=\"{from}\"."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn reg(id: &str, space: &str, number: usize) -> Registration {
        Registration {
            instance_id: id.into(),
            client_id: "c1".into(),
            control_addr: "127.0.0.1:1".into(),
            agent: "claude".into(),
            number,
            space: space.into(),
            cwd: "/home/u/Meu Projeto".into(),
        }
    }

    #[test]
    fn handles_use_space_or_folder_and_are_unique() {
        let hub = Hub::default();
        assert_eq!(
            hub.register(reg("a", "Terraforme", 1)),
            "terraforme/claude-1"
        );
        assert_eq!(hub.register(reg("b", "", 1)), "meu-projeto/claude-1");
        // Mesmo space aberto em dois terminais.
        assert_eq!(
            hub.register(reg("c", "Terraforme", 1)),
            "terraforme/claude-1#2"
        );
        // Registrar de novo mantém o apelido.
        assert_eq!(
            hub.register(reg("a", "Terraforme", 1)),
            "terraforme/claude-1"
        );

        assert_eq!(hub.find("TERRAFORME/claude-1").unwrap().id, "a");
        assert_eq!(hub.find("b").unwrap().handle, "meu-projeto/claude-1");

        hub.set_activity("a", Activity::Done);
        assert_eq!(hub.get("a").unwrap().activity, Activity::Done);
        // Registrar de novo não perde o estado.
        hub.register(reg("a", "Terraforme", 1));
        assert_eq!(hub.get("a").unwrap().activity, Activity::Done);

        hub.unregister("b");
        assert!(hub.find("b").is_none());
        hub.drop_client("c1");
        assert!(hub.instances().is_empty());
    }

    #[test]
    fn prompt_tells_how_to_reply() {
        let text = prompt("web-app/claude-1", "oi");
        assert!(text.starts_with("[terraforme] Mensagem de web-app/claude-1:\noi\n"));
        assert!(text.ends_with("to=\"web-app/claude-1\"."));
        assert!(prompt(WEB, "oi").contains("usuário (UI web)"));
    }

    /// Terminal falso: a cada pedido guarda o caminho e o corpo e, num
    /// `/open`, registra a instância pedida como faria a TUI.
    async fn fake_terminal(hub: Hub) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (control_addr, log) = (addr.clone(), seen.clone());
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = String::new();
                stream.read_to_string(&mut raw).await.unwrap();
                let (head, body) = raw.split_once("\r\n\r\n").unwrap();
                let path = head.split_whitespace().nth(1).unwrap().to_string();
                let body: Value = serde_json::from_str(body).unwrap();
                if path == "/open" {
                    let mut new = reg(body["instance_id"].as_str().unwrap(), "Terraforme", 2);
                    new.control_addr = control_addr.clone();
                    hub.register(new);
                }
                log.lock().unwrap().push((path, body));
            }
        });
        (addr, seen)
    }

    #[tokio::test]
    async fn opens_delegates_and_closes_instances() {
        let hub = Hub::default();
        let (addr, seen) = fake_terminal(hub.clone()).await;
        let mut first = reg("a", "Terraforme", 1);
        first.control_addr = addr;
        hub.register(first);
        let from = hub.get("a").unwrap();

        let (opened, rx) = hub.open_delegated(&from, "rode os testes").await.unwrap();
        assert_eq!(opened.handle, "terraforme/claude-2");
        {
            let seen = seen.lock().unwrap();
            let (path, body) = &seen[0];
            assert_eq!(path, "/open");
            assert_eq!(body["from"], "terraforme/claude-1");
            assert!(
                body["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("[terraforme] Tarefa delegada por terraforme/claude-1:")
            );
        }

        // A resposta de quem trabalha volta para a chamada, sem ser digitada.
        hub.send(&opened.handle, &from.handle, "2 falharam")
            .await
            .unwrap();
        assert_eq!(hub.wait(&opened, rx).await, Some(Ok("2 falharam".into())));
        assert_eq!(seen.lock().unwrap().len(), 1);

        assert!(
            hub.close("terraforme/claude-2", "terraforme/claude-2")
                .await
                .is_err()
        );
        let closed = hub
            .close(&from.handle, "terraforme/claude-2")
            .await
            .unwrap();
        assert_eq!(closed, "terraforme/claude-2");
        // O pedido já foi enviado; o terminal falso o lê em paralelo.
        while seen.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1].0, "/close");
        assert_eq!(seen[1].1["instance_id"], opened.id);
        let log = hub.messages();
        assert_eq!(log[0].text, "rode os testes");
        assert_eq!(log[1].text, "2 falharam");
        assert_eq!(log[2].text, "[instância fechada]");
    }

    #[test]
    fn delegation_asks_for_the_result() {
        let text = delegation("web-app/claude-1", "rode os testes");
        assert!(
            text.starts_with(
                "[terraforme] Tarefa delegada por web-app/claude-1:\nrode os testes\n"
            )
        );
        assert!(text.ends_with("to=\"web-app/claude-1\"."));
    }
}
