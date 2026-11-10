//! A conversa com o [`crate::hub`]: o que este terra conta a ele a cada tick e
//! o que recebe de volta, como as execuções despachadas.

use super::{App, ChatTab, MAIN_CHAT_TITLE};
use crate::hub;
use crate::hub::client::Event;
use crate::tui::chat::Chat;

impl App {
    /// O que este terra está fazendo, para o hub: algum chat rodando, o que
    /// o agente faz e os chats abertos (os além do principal são os sub
    /// agentes).
    pub(super) fn hub_status(&self) -> hub::protocol::Status {
        let main = (&self.chat_id, MAIN_CHAT_TITLE, &self.chat, None);
        let chats = std::iter::once(main).chain(
            self.chats
                .iter()
                .map(|tab| (&tab.id, tab.title.as_str(), &tab.chat, tab.run.clone())),
        );
        let tabs: Vec<_> = chats
            .map(|(id, title, chat, run)| hub::protocol::Tab {
                id: id.clone(),
                run,
                title: title.to_string(),
                name: chat.agent_name().map(str::to_string),
                task: chat.activity().map(str::to_string),
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
    /// do chat. Os pedidos de permissão das execuções vão para o chat
    /// principal, e as respostas a eles voltam ao hub. Nada disso vai ao
    /// agente do chat.
    pub(super) fn receive_from_hub(&mut self) {
        let answers = self.chat.take_remote_answers();
        let events = match &self.hub {
            Some(hub) => {
                for (run, id, allow) in answers {
                    hub.permission_answer(&run, &id, allow);
                }
                hub.events()
            }
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
                Event::PermissionRequest {
                    run,
                    agent,
                    id,
                    name,
                    detail,
                } => self.chat.remote_request(run, id, agent, name, detail),
                Event::RunFinished { run } => self.chat.run_finished(&run),
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
            title: format!("▶ {}", run.agent),
            chat: Chat::viewer(session, intro, provider),
        });
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
