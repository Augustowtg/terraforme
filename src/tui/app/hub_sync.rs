//! A conversa com o [`crate::hub`]: o que este terra conta a ele a cada tick e
//! o que recebe de volta, como as execuções despachadas.

use super::{App, ChatTab, MAIN_CHAT_TITLE, SubagentTab};
use crate::agent::{AgentEvent, SubagentEvent};
use crate::hub;
use crate::hub::client::Event;
use crate::tui::chat::Chat;

/// Quantos caracteres da descrição de um subagente vão no título da aba.
const SUBAGENT_TITLE: usize = 24;

impl App {
    /// O que este terra está fazendo, para o hub: algum chat rodando, o que
    /// o agente faz e os chats abertos (os além do principal são os sub
    /// agentes).
    pub(super) fn hub_status(&self) -> hub::protocol::Status {
        let main = (&self.chat_id, MAIN_CHAT_TITLE, &self.chat, None, None);
        let chats = std::iter::once(main).chain(self.chats.iter().map(|tab| {
            (
                &tab.id,
                tab.title.as_str(),
                &tab.chat,
                tab.run.clone(),
                tab.subagent.as_ref().map(SubagentTab::state),
            )
        }));
        let tabs: Vec<_> = chats
            .map(|(id, title, chat, run, subagent)| hub::protocol::Tab {
                id: id.clone(),
                run,
                title: title.to_string(),
                name: chat.agent_name().map(str::to_string),
                task: chat.activity().map(str::to_string),
                subagent,
            })
            .collect();
        let task = tabs.iter().find_map(|tab| tab.task.clone());
        hub::protocol::Status {
            running: task.is_some(),
            task,
            chats: tabs.len(),
            agents: tabs.len() - 1,
            tabs,
        }
    }

    /// O que o hub mandou: as mensagens de outros agentes vão para a caixa
    /// do chat da aba (o `ack` confirma ao hub) e os nomes recusados saem
    /// do chat. Nada disso vai ao agente do chat.
    pub(super) fn receive_from_hub(&mut self) {
        let events = match &self.hub {
            Some(hub) => hub.events(),
            None => return,
        };
        for event in events {
            match event {
                Event::Deliver { tab, message } => {
                    let id = message.id.clone();
                    if let Some(chat) = self.chat_by_id_mut(&tab) {
                        chat.receive(message);
                        if let Some(hub) = &self.hub {
                            hub.ack(&id);
                        }
                    }
                }
                Event::RunStarted { run, output } => self.show_run(&run, &output),
                Event::NameRejected { tab, name, reason } => {
                    if let Some(chat) = self.chat_by_id_mut(&tab) {
                        chat.name_rejected(&name, &reason);
                    }
                }
            }
        }
    }

    /// Uma execução despachada começou: abre uma aba só de leitura que
    /// acompanha a saída do agente. Ela vai para a frente só com
    /// `aba_em_primeiro_plano` no `orchestrator.toml`.
    pub(super) fn show_run(
        &mut self,
        run: &crate::orchestrator::run::Run,
        output: &std::path::Path,
    ) {
        self.push_run(run, output);
        let focus = self
            .orchestrator_view
            .data
            .path()
            .and_then(|path| crate::orchestrator::config::Config::load(&path).ok())
            .is_some_and(|config| config.global.focus_runs);
        if focus {
            self.active_chat = self.chats.len();
        }
    }

    /// A aba só de leitura de uma execução, no fim da barra de chats.
    fn push_run(&mut self, run: &crate::orchestrator::run::Run, output: &std::path::Path) {
        let intro = format!(
            "▶ Execução {} do agente {}: mensagem de {} (nível {}, {} saltos). Nota: {}",
            &run.id[..run.id.len().min(8)],
            run.agent,
            run.from,
            run.level,
            run.hops,
            run.note.display()
        );
        use crate::agent::adapters::{antigravity, claude};
        // A saída de cada provedor tem o leitor dele.
        let (session, provider): (_, crate::tui::chat::ViewerProvider) = if run.is_antigravity() {
            (
                antigravity::follow(output),
                (antigravity::NAME, str::to_string),
            )
        } else {
            (
                claude::follow(output),
                (claude::NAME, claude::display_model),
            )
        };
        self.chats.push(ChatTab {
            id: hub::protocol::new_id(),
            run: Some(run.id.clone()),
            subagent: None,
            title: format!("▶ {}", run.agent),
            chat: Chat::viewer(session, intro, provider),
        });
    }

    /// Leva às abas o que os subagentes dos chats fizeram: cada um que começa
    /// abre uma aba só de leitura, em segundo plano. A aba de um subagente
    /// cujo chat parou (o `Esc`, por exemplo, mata os dois) termina junto.
    pub(super) fn sync_subagents(&mut self) {
        let mut events: Vec<_> = self
            .chat
            .take_subagent_events()
            .into_iter()
            .map(|event| (self.chat_id.clone(), event))
            .collect();
        for tab in &mut self.chats {
            let id = tab.id.clone();
            events.extend(
                tab.chat
                    .take_subagent_events()
                    .into_iter()
                    .map(|event| (id.clone(), event)),
            );
        }
        for (parent, event) in events {
            self.apply_subagent(parent, event);
        }
        let stopped: Vec<String> = self
            .chats
            .iter()
            .filter_map(|tab| tab.subagent.as_ref())
            .filter(|sub| sub.feed.is_some())
            .map(|sub| sub.parent.clone())
            .filter(|parent| {
                !self
                    .chat_by_id(parent)
                    .is_some_and(|chat| chat.is_running())
            })
            .collect();
        for tab in &mut self.chats {
            if let Some(sub) = &mut tab.subagent
                && stopped.contains(&sub.parent)
                && let Some(feed) = sub.feed.take()
            {
                sub.failed = true;
                let _ = feed.send(AgentEvent::Failed(
                    "O chat que lançou este subagente parou antes dele.".to_string(),
                ));
            }
        }
    }

    fn apply_subagent(&mut self, parent: String, event: SubagentEvent) {
        match event {
            SubagentEvent::Started { id, description } => {
                use crate::agent::adapters::claude;
                let (tx, session) = crate::agent::feed();
                let title: String = description.chars().take(SUBAGENT_TITLE).collect();
                let intro = format!("▶ Subagente: {description}");
                let provider = (claude::NAME, claude::display_model as fn(&str) -> String);
                self.chats.push(ChatTab {
                    id: hub::protocol::new_id(),
                    run: None,
                    subagent: Some(SubagentTab {
                        id,
                        parent: parent.clone(),
                        feed: Some(tx),
                        failed: false,
                    }),
                    title: format!("▶ {title}"),
                    chat: Chat::viewer(session, intro, provider),
                });
            }
            SubagentEvent::Activity { id, event } => {
                if let Some(feed) = self
                    .subagent_tab(&parent, &id)
                    .and_then(|tab| tab.feed.as_ref())
                {
                    let _ = feed.send(*event);
                }
            }
            SubagentEvent::Finished { id, error } => {
                if let Some(tab) = self.subagent_tab(&parent, &id)
                    && let Some(feed) = tab.feed.take()
                {
                    tab.failed = error.is_some();
                    let _ = feed.send(match error {
                        Some(error) => AgentEvent::Failed(error),
                        None => AgentEvent::Done {
                            is_error: false,
                            message: None,
                            tokens: 0,
                            cache_tokens: 0,
                            denied: 0,
                        },
                    });
                }
            }
            SubagentEvent::Waiting => {}
        }
    }

    /// A aba do subagente `id` que o chat `parent` lançou.
    fn subagent_tab(&mut self, parent: &str, id: &str) -> Option<&mut SubagentTab> {
        self.chats
            .iter_mut()
            .filter_map(|tab| tab.subagent.as_mut())
            .find(|sub| sub.id == id && sub.parent == parent)
    }

    /// O chat com o id `id` no hub, só para ler.
    fn chat_by_id(&self, id: &str) -> Option<&Chat> {
        if self.chat_id == id {
            return Some(&self.chat);
        }
        self.chats
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| &tab.chat)
    }

    /// O chat com o id `id` no hub: o principal ou um do `Alt+N`.
    fn chat_by_id_mut(&mut self, id: &str) -> Option<&mut Chat> {
        if self.chat_id == id {
            return Some(&mut self.chat);
        }
        self.chats
            .iter_mut()
            .find(|tab| tab.id == id)
            .map(|tab| &mut tab.chat)
    }

    /// As mensagens de outros agentes ainda não lidas, em todos os chats.
    pub(super) fn unread(&self) -> usize {
        self.chat.unread()
            + self
                .chats
                .iter()
                .map(|tab| tab.chat.unread())
                .sum::<usize>()
    }

    /// Manda o estado ao hub; o [`Reporter`](crate::hub::client::Reporter) só manda quando mudou e nunca
    /// espera.
    pub(super) fn report_to_hub(&mut self) {
        let status = self.hub_status();
        if let Some(hub) = &mut self.hub {
            hub.set_status(status);
        }
    }
}
