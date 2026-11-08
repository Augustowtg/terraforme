//! As mensagens entre agentes no hub: os nomes das abas, o `send` de quem
//! manda (o `terra send`) e a entrega às abas manuais.
//!
//! ```text
//! terra send → hub: {"t":"hello","proto":2,"role":"sender"}
//!                   {"t":"send","message":{ … }}
//! hub → terra send: {"t":"sent","id":"…","state":"enviada"|"na_fila"}
//!                   {"t":"rejected","id":"…","code":"hops","text":"…"}
//! terra done → hub: {"t":"hello","proto":2,"role":"sender"}
//!                   {"t":"done","run":"…","summary":"…"}
//! hub → terra done: {"t":"ok"} ou {"t":"rejected",…}
//! hub → janela:     {"t":"deliver","tab":"<id da aba>","message":{ … }}
//!                   {"t":"name_rejected","tab":"…","name":"…","reason":"…"}
//! janela → hub:     {"t":"ack","id":"…"}
//! ```
//!
//! Um nome é de uma aba só na máquina, e o nome de um agente da
//! configuração não serve para uma aba manual. A mensagem para um agente da
//! configuração fica na fila dele; a para uma aba manual vai à janela dela,
//! que só mostra a mensagem (nunca a manda ao agente da aba).

use std::io::BufReader;

use serde_json::{Value, json};

use super::{Hub, Outbox, PROTO, State};
use crate::hub::protocol::Health;
use crate::hub::{APP_VERSION, Conn};
use crate::orchestrator::config::{self, Config};
use crate::orchestrator::message::{self, Message, Reject, State as MessageState};

impl State {
    /// A aba viva que tem o nome `name`, fora a instância `except`: a fila
    /// de saída da janela e o id da aba.
    fn holder(&self, name: &str, except: &str) -> Option<(&Outbox, String)> {
        self.instances
            .iter()
            .filter(|(id, entry)| id != except && entry.health != Health::Gone)
            .find_map(|(_, entry)| {
                let tabs = entry.info["status"]["tabs"].as_array()?;
                let tab = tabs.iter().find(|tab| tab["name"] == name)?;
                Some((&entry.outbox, tab["id"].as_str()?.to_string()))
            })
    }
}

impl Hub {
    /// A configuração do `orchestrator.toml`, lida de novo a cada uso. Com
    /// o arquivo inválido, fica a última válida, e o erro vai ao log (uma
    /// vez por erro).
    pub(super) fn config(&self) -> Config {
        let loaded = Config::load(&Config::path(&self.home));
        let mut last = self.config.lock().unwrap_or_else(|e| e.into_inner());
        match loaded {
            Ok(config) => {
                last.0 = config;
                last.1 = None;
            }
            Err(err) => {
                if last.1.as_ref() != Some(&err) {
                    self.log.line(&format!(
                        "orchestrator.toml inválido; fica o anterior: {err}"
                    ));
                    last.1 = Some(err);
                }
            }
        }
        last.0.clone()
    }

    /// Os nomes das abas do `status` da instância `id`: o repetido, o de um
    /// agente da configuração e o inválido saem do status, e voltam as
    /// mensagens `name_rejected` para a janela.
    pub(super) fn claim_names(&self, id: &str, status: &mut Value) -> Vec<Value> {
        let state = self.state();
        self.claim_names_in(&state, id, status)
    }

    pub(super) fn claim_names_in(&self, state: &State, id: &str, status: &mut Value) -> Vec<Value> {
        let Some(tabs) = status["tabs"].as_array_mut() else {
            return Vec::new();
        };
        let config = self.config();
        let mut taken = Vec::new();
        let mut rejected = Vec::new();
        for tab in tabs {
            let Some(name) = tab["name"].as_str().map(str::to_string) else {
                continue;
            };
            let reason = if !config::valid_name(&name) {
                Some("nome inválido")
            } else if config.agent(&name).is_some() {
                Some("é o nome de um agente da configuração")
            } else if taken.contains(&name) || state.holder(&name, id).is_some() {
                Some("outra aba já tem esse nome")
            } else {
                None
            };
            match reason {
                Some(reason) => {
                    rejected.push(json!({
                        "t": "name_rejected",
                        "tab": tab["id"],
                        "name": name,
                        "reason": reason,
                    }));
                    tab["name"] = Value::Null;
                }
                None => taken.push(name),
            }
        }
        rejected
    }

    /// Quem manda (o `terra send`): um `send` e a resposta.
    pub(super) fn sender(&self, mut input: BufReader<Conn>, output: &Outbox) {
        if !output.push(json!({"t": "welcome", "proto": PROTO, "hub": APP_VERSION})) {
            return;
        }
        let Some(msg) = self.read(&mut input) else {
            return;
        };
        if msg["t"] == "done" {
            let run = msg["run"].as_str().unwrap_or_default();
            let summary = msg["summary"].as_str().map(str::to_string);
            let reply = match self.done(run, summary) {
                Ok(()) => json!({"t": "ok"}),
                Err(reject) => json!({"t": "rejected", "code": reject.code, "text": reject.text}),
            };
            output.push(reply);
            return;
        }
        let reply = match Message::from_json(&msg["message"]).filter(|m| valid_id(&m.id)) {
            None => json!({"t": "rejected", "code": "mensagem", "text": "mensagem inválida"}),
            Some(message) => match self.route(&message) {
                Ok(state) => json!({"t": "sent", "id": message.id, "state": state.key()}),
                Err(reject) => json!({
                    "t": "rejected",
                    "id": message.id,
                    "code": reject.code,
                    "text": reject.text,
                }),
            },
        };
        output.push(reply);
    }

    /// Confere a mensagem de novo e a leva: à fila do agente da
    /// configuração ou à janela da aba com o nome. Grava o estado e conta
    /// aos monitores.
    pub(super) fn route(&self, message: &Message) -> Result<MessageState, Reject> {
        let config = self.config();
        let result = message.check(&config.global).and_then(|()| {
            if config.agent(&message.to).is_some() {
                // Gravada antes: a execução pode começar já no `enqueue`.
                self.record(message, MessageState::Queued, None);
                self.enqueue(message, &config)?;
                return Ok(MessageState::Queued);
            }
            let state = self.state();
            let Some((outbox, tab)) = state.holder(&message.to, "") else {
                return Err(Reject::new(
                    "agente",
                    format!(
                        "nenhuma aba aberta se chama \"{0}\" e não há [agents.{0}] na configuração",
                        message.to
                    ),
                ));
            };
            let deliver = json!({"t": "deliver", "tab": tab, "message": message.to_json()});
            if !outbox.push(deliver) {
                return Err(Reject::new("agente", "a janela do agente não respondeu"));
            }
            Ok(MessageState::Sent)
        });
        match &result {
            Ok(MessageState::Queued) => {}
            Ok(state) => self.record(message, *state, None),
            Err(reject) => self.record(message, MessageState::Rejected, Some(reject)),
        }
        result
    }

    /// A janela recebeu a mensagem: ela está na aba, esperando o usuário.
    pub(super) fn ack(&self, id: &str) {
        if !valid_id(id) {
            return;
        }
        if let Some(message) = message::load(&self.home, id) {
            self.record(&message, MessageState::Delivered, None);
        }
    }

    /// Grava o estado da mensagem, anota no log e conta aos monitores.
    pub(super) fn record(&self, message: &Message, state: MessageState, reason: Option<&Reject>) {
        if let Err(err) = message::save(message, state, reason, &self.home) {
            self.log
                .line(&format!("não foi possível gravar a mensagem: {err}"));
        }
        self.log.line(&format!(
            "mensagem {} {} → {} (nível {}, {} saltos): {}{}",
            &message.id[..message.id.len().min(8)],
            message.from,
            message.to,
            message.level,
            message.hops,
            state.key(),
            reason.map_or(String::new(), |r| format!(" ({})", r.code)),
        ));
        self.state().broadcast(&json!({
            "t": "message",
            "message": message.to_json(),
            "state": state.key(),
            "reason": reason.map(|r| json!({"code": r.code, "text": r.text})),
        }));
    }
}

/// Um id de mensagem: hexadecimal, como os do terra (e seguro como nome de
/// pasta).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit())
}
