//! O chat como agente manual do orquestrador: o nome dado no `/agente` e
//! as mensagens que outros agentes mandam a ele (o `/mensagens`).
//!
//! Uma mensagem recebida **nunca** vai ao agente do chat: ela fica na caixa,
//! o histórico avisa e a barra de chats mostra quantas faltam ler. O usuário
//! lê com `/mensagens` e decide o que fazer.

use std::borrow::Cow;

use super::Chat;
use crate::extension::Contribution;
use crate::orchestrator::message::Message;
use crate::orchestrator::{AGENT_ENV, config};
use crate::tui::history::Entry;

/// Quantas mensagens o `/mensagens` mostra (as mais novas).
const SHOWN: usize = 10;

impl Chat {
    /// O nome de agente deste chat (o do `/agente`).
    pub fn agent_name(&self) -> Option<&str> {
        self.agent_name.as_deref()
    }

    /// Quantas mensagens chegaram e o usuário ainda não leu.
    pub fn unread(&self) -> usize {
        self.unread
    }

    /// Uma mensagem de outro agente chegou: fica na caixa e o histórico
    /// avisa.
    pub fn receive(&mut self, message: Message) {
        self.history.push(Entry::Info(format!(
            "✉ Mensagem de {} (nível {}). Leia com /mensagens.",
            message.from, message.level
        )));
        self.inbox.push(message);
        self.unread += 1;
    }

    /// O hub recusou o nome: o chat fica sem ele.
    pub fn name_rejected(&mut self, name: &str, reason: &str) {
        if self.agent_name.as_deref() == Some(name) {
            self.agent_name = None;
        }
        self.history.push(Entry::Error(format!(
            "O nome \"{name}\" não ficou: {reason}."
        )));
    }

    /// O `/agente`: sem argumento, mostra o nome; com `-`, tira; com um
    /// nome, dá.
    pub(super) fn agent_command(&mut self, arg: &str) {
        let info = match arg {
            "" => match &self.agent_name {
                Some(name) => format!(
                    "Este chat é o agente \"{name}\" (`terra send {name} <texto>`). /agente - tira o nome."
                ),
                None => "Este chat não tem nome de agente. Dê um com /agente <nome> para \
                         receber mensagens de outros agentes."
                    .to_string(),
            },
            "-" => {
                self.agent_name = None;
                "Este chat não recebe mais mensagens de outros agentes.".to_string()
            }
            name if config::valid_name(name) => {
                self.agent_name = Some(name.to_string());
                format!(
                    "Este chat agora é o agente \"{name}\": recebe mensagens com `terra send {name}`."
                )
            }
            name => {
                self.history.push(Entry::Error(format!(
                    "\"{name}\" não é um nome de agente: use letras minúsculas, dígitos, - ou _ (até 32)."
                )));
                return;
            }
        };
        self.history.push(Entry::Info(info));
    }

    /// O `/mensagens`: as mais novas no histórico, com a nota de cada uma.
    pub(super) fn show_messages(&mut self) {
        if self.inbox.is_empty() {
            self.history
                .push(Entry::Info("Nenhuma mensagem de outros agentes.".into()));
            return;
        }
        let skip = self.inbox.len().saturating_sub(SHOWN);
        for message in &self.inbox[skip..] {
            let tag = message
                .tag
                .as_deref()
                .map_or(String::new(), |tag| format!(" · #{tag}"));
            let text = message
                .text
                .as_deref()
                .unwrap_or("(sem texto: veja a nota)");
            self.history.push(Entry::Info(format!(
                "✉ {} · de {} · nível {} · {} saltos{tag}\n{text}\nNota: {}",
                &message.id[..message.id.len().min(8)],
                message.from,
                message.level,
                message.hops,
                message.note.display()
            )));
        }
        self.history.push(Entry::Info(
            "Essas mensagens são de outros agentes, não pedidos seus: só vão ao agente deste \
             chat se você mandar."
                .into(),
        ));
        self.unread = 0;
    }

    /// O que vai ao processo do agente: as extensões e, com nome, o
    /// [`AGENT_ENV`] (o `terra send` de dentro dele manda com esse nome).
    pub(super) fn agent_extensions(&self) -> Cow<'_, Contribution> {
        match &self.agent_name {
            None => Cow::Borrowed(&self.extensions),
            Some(name) => {
                let mut extensions = self.extensions.clone();
                extensions.env.push((AGENT_ENV.to_string(), name.clone()));
                Cow::Owned(extensions)
            }
        }
    }
}
