//! Instâncias de agente registradas pelos terminais (`terraforme cli`) de
//! todos os spaces e as mensagens trocadas entre elas.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::tools::http_post_json;

/// Mensagens guardadas para a UI (as mais antigas saem primeiro).
const MAX_MESSAGES: usize = 500;

/// Destino especial: a mensagem só aparece na UI web.
pub const WEB: &str = "web";

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
    /// Segundos desde a época Unix.
    pub since: u64,
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
                since: now(),
            },
        );
        handle
    }

    pub fn unregister(&self, instance_id: &str) {
        if let Some(i) = self.state.lock().unwrap().instances.remove(instance_id) {
            info!(handle = i.handle, "instância encerrada");
        }
    }

    /// Remove as instâncias de um terminal que saiu.
    pub fn drop_client(&self, client_id: &str) {
        self.state
            .lock()
            .unwrap()
            .instances
            .retain(|_, i| i.client_id != client_id);
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
        let body = json!({
            "instance_id": target.id,
            "from": from,
            "text": prompt(from, text),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
