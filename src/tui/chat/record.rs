//! Grava a conversa do chat no histórico do workspace ([`ChatHistory`]).
//!
//! A conversa só começa na primeira mensagem do usuário, e o banco só é criado
//! nela: um chat aberto e fechado sem conversar não deixa nada.

use std::path::PathBuf;

use anyhow::Result;

use crate::agent::ModelSpend;
use crate::db::history::{ChatHistory, Conversation, Message, TurnOwner};
use crate::tui::history::Entry;

pub(super) struct Recorder {
    file: PathBuf,
    store: Option<ChatHistory>,
    /// A conversa em andamento no banco.
    conversation: Option<i64>,
    /// Quantas entradas do histórico da tela já foram vistas.
    written: usize,
    /// A pasta da sessão, para onde vai o consumo dos turnos.
    folder: Option<String>,
}

impl Recorder {
    pub(super) fn new(file: PathBuf) -> Self {
        Self {
            file,
            store: None,
            conversation: None,
            written: 0,
            folder: None,
        }
    }

    pub(super) fn set_folder(&mut self, folder: String) {
        self.folder = Some(folder);
    }

    /// Grava o gasto de um turno na sessão da pasta.
    pub(super) fn record_turn(
        &mut self,
        info: Conversation<'_>,
        spend: &[ModelSpend],
    ) -> Result<()> {
        let Some(folder) = self.folder.clone() else {
            return Ok(());
        };
        let owner = TurnOwner {
            folder: &folder,
            turn: &crate::hub::protocol::new_id(),
            provider: info.provider,
            session_id: info.session_id,
        };
        self.store()?.record_turn(owner, spend)
    }

    /// Grava as entradas novas de `entries`. Antes da primeira mensagem do
    /// usuário elas são só avisos da tela, e ficam de fora.
    pub(super) fn save(&mut self, entries: &[Entry], info: Conversation<'_>) -> Result<()> {
        let mut pending = entries.get(self.written..).unwrap_or_default();
        if self.conversation.is_none() {
            let first = pending.iter().position(|e| matches!(e, Entry::User(_)));
            pending = first.map_or(&[], |first| &pending[first..]);
        }
        if pending.is_empty() {
            self.written = entries.len();
            return Ok(());
        }
        let conversation = self.conversation;
        let store = self.store()?;
        let id = match conversation {
            Some(id) => id,
            None => store.start_conversation()?,
        };
        let messages: Vec<Message<'_>> = pending.iter().map(message).collect();
        store.append(id, info, &messages)?;
        self.conversation = Some(id);
        self.written = entries.len();
        Ok(())
    }

    /// As últimas mensagens enviadas no workspace, a mais antiga primeiro. Sem
    /// o banco ainda não há nenhuma, e ele continua sem ser criado.
    pub(super) fn sent(&mut self) -> Result<Vec<String>> {
        if !self.file.is_file() {
            return Ok(Vec::new());
        }
        self.store()?.sent()
    }

    /// Guarda uma mensagem enviada, para voltar a ela nas próximas execuções.
    pub(super) fn remember_sent(&mut self, content: &str) -> Result<()> {
        self.store()?.remember_sent(content)
    }

    fn store(&mut self) -> Result<&mut ChatHistory> {
        Ok(match &mut self.store {
            Some(store) => store,
            none => none.insert(ChatHistory::open(&self.file)?),
        })
    }

    /// A próxima mensagem começa outra conversa; `written` é quantas
    /// entradas o histórico da tela já tem.
    pub(super) fn new_conversation(&mut self, written: usize) {
        self.conversation = None;
        self.written = written;
    }
}

fn message(entry: &Entry) -> Message<'_> {
    let (role, tool, content) = match entry {
        Entry::User(text) => ("user", None, text),
        Entry::Assistant(text) => ("assistant", None, text),
        Entry::Tool { name, detail } => ("tool", Some(name.as_str()), detail),
        Entry::ToolResult {
            is_error: false,
            summary,
        } => ("tool_result", None, summary),
        Entry::ToolResult {
            is_error: true,
            summary,
        } => ("tool_error", None, summary),
        Entry::Error(text) => ("error", None, text),
        Entry::Info(text) => ("info", None, text),
    };
    Message {
        role,
        tool,
        content: content.as_str(),
    }
}
