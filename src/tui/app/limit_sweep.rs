//! Os terras abertos com uma versÃ£o anterior nÃ£o continuam sozinhos depois do
//! limite de uso. Um terra desta versÃ£o vasculha as abas deles pelo hub a
//! cada minuto e digita o `continue` (como o app remoto) na aba parada cuja
//! Ãºltima mensagem no histÃ³rico Ã© o erro do limite, 5 min depois do reset.
//! Com vÃ¡rios terras desta versÃ£o abertos, sÃ³ o de menor id vasculha.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::agent::limit;
use crate::db::history::ChatHistory;
use crate::hub::endpoint::Endpoint;
use crate::hub::protocol::{Health, Instance};
use crate::hub::{APP_VERSION, client};

/// De quanto em quanto tempo a vasculha roda.
const EVERY: Duration = Duration::from_secs(60);

/// Uma aba de um terra de versÃ£o anterior que parou no limite de uso.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Stuck {
    pub tab: String,
    pub title: String,
    pub app: String,
    /// Quando o erro do limite foi gravado (segundos Unix).
    pub error_at: i64,
    /// Quando mandar o `continue`.
    pub resume_at: i64,
}

/// ComeÃ§a a vasculha numa thread; `own` Ã© o id deste terra no hub. O que ela
/// fez chega no canal, para o chat mostrar.
pub(super) fn start(own: String) -> Option<Receiver<String>> {
    let endpoint = Endpoint::current().ok()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        // A aba e o erro que jÃ¡ receberam o `continue`.
        let mut sent = HashSet::new();
        loop {
            thread::sleep(EVERY);
            let Ok(values) = client::snapshot_with(&endpoint, false) else {
                continue;
            };
            let instances: Vec<Instance> = values.iter().filter_map(Instance::from_json).collect();
            for stuck in stuck_tabs(&instances, &own, &endpoint.home) {
                let key = (stuck.tab.clone(), stuck.error_at);
                if stuck.resume_at > limit::now() || sent.contains(&key) {
                    continue;
                }
                let typed = client::send_input(&endpoint, &stuck.tab, limit::RESUME_PROMPT)
                    .is_ok_and(|answer| answer["t"] == "ok");
                if !typed {
                    continue;
                }
                sent.insert(key);
                let note = format!(
                    "Mandei `{}` Ã  aba {} de um terra da versÃ£o {}, que tinha parado no limite de uso.",
                    limit::RESUME_PROMPT,
                    stuck.title,
                    stuck.app
                );
                if tx.send(note).is_err() {
                    return;
                }
            }
        }
    });
    Some(rx)
}

/// As abas paradas no limite nos terras vivos de versÃ£o anterior. Vazio
/// quando outro terra desta versÃ£o (o de menor id) Ã© quem vasculha.
pub(super) fn stuck_tabs(instances: &[Instance], own: &str, home: &Path) -> Vec<Stuck> {
    let alive = |instance: &&Instance| instance.health != Health::Gone;
    let leader = instances
        .iter()
        .filter(alive)
        .filter(|instance| instance.app == APP_VERSION)
        .map(|instance| instance.id.as_str())
        .min();
    if leader.is_some_and(|leader| leader != own) {
        return Vec::new();
    }
    instances
        .iter()
        .filter(alive)
        .filter(|instance| older(&instance.app))
        .flat_map(|instance| {
            instance
                .status
                .tabs
                .iter()
                .filter(|tab| tab.run.is_none() && tab.task.is_none() && tab.ask.is_none())
                .filter_map(move |tab| {
                    let file = crate::settings::history_file(home, &instance.workspace);
                    if !file.is_file() {
                        return None;
                    }
                    let history = ChatHistory::open(&file).ok()?;
                    let (role, content, created) =
                        history.last_message(tab.conversation?).ok()??;
                    if role != "error" || !limit::is_limit_message(&content) {
                        return None;
                    }
                    let error_at = created / 1000;
                    let reset = limit::reset_from_message(&content, error_at);
                    Some(Stuck {
                        tab: tab.id.clone(),
                        title: tab.title.clone(),
                        app: instance.app.clone(),
                        error_at,
                        resume_at: limit::resume_at(reset, error_at),
                    })
                })
        })
        .collect()
}

/// A versÃ£o `app` Ã© anterior Ã  deste terra.
fn older(app: &str) -> bool {
    let version = |text: &str| -> Option<[u64; 3]> {
        let mut parts = text.trim_start_matches('v').split('.').map(str::parse);
        Some([
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        ])
    };
    matches!((version(app), version(APP_VERSION)), (Some(a), Some(b)) if a < b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history::{Conversation, Message};
    use crate::hub::protocol::{Status, Tab};

    fn instance(id: &str, app: &str, tab: Tab) -> Instance {
        Instance {
            id: id.into(),
            pid: 1,
            app: app.into(),
            cwd: String::new(),
            workspace: "projeto".into(),
            started: 0,
            last_seen: 0,
            health: Health::Alive,
            reason: None,
            status: Status {
                chats: 1,
                tabs: vec![tab],
                ..Status::default()
            },
        }
    }

    #[test]
    fn an_older_terra_stopped_in_the_limit_gets_the_continue() {
        let home = tempfile::tempdir().unwrap();
        let file = crate::settings::history_file(home.path(), "projeto");
        let mut history = ChatHistory::open(&file).unwrap();
        let conversation = history.start_conversation().unwrap();
        let message = |role, content| Message {
            role,
            tool: None,
            content,
        };
        history
            .append(
                conversation,
                Conversation::default(),
                &[
                    message("user", "faz a tarefa"),
                    message("error", "Claude AI usage limit reached|4000000000"),
                ],
            )
            .unwrap();
        let tab = Tab {
            id: "aba-antiga".into(),
            title: "Principal".into(),
            conversation: Some(conversation),
            ..Tab::default()
        };
        let instances = [
            instance("novo", APP_VERSION, Tab::default()),
            instance("antigo", "0.14.30", tab),
        ];

        let stuck = stuck_tabs(&instances, "novo", home.path());
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].tab, "aba-antiga");
        assert_eq!(stuck[0].resume_at, 4_000_000_000 + limit::RESUME_DELAY);
    }
}
