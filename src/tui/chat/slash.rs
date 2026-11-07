//! Comandos de `/` do chat: os do terra, os que vão ao provedor, o login e a
//! limpeza ou compactação da conversa.

use std::process::Command;

use super::Chat;
use super::model_menu::ModelMenu;
use crate::agent::{CommandRoute, Interactive};
use crate::tui::history::{Entry, History};
use crate::tui::input::extensions::AgentStatus;

impl Chat {
    /// Executa um comando da `/`. A função de cada um é uma interface do
    /// terra (veja [`commands`](crate::tui::input::commands)); os que ainda não têm avisam no histórico.
    pub(super) fn run_command(&mut self, name: &str) {
        match name {
            "clear" => self.clear_conversation(),
            "compact" => self.compact(),
            "login" => self.start_login(),
            "agente" => self.agent_command(""),
            "mensagens" => self.show_messages(),
            // O consumo é a aba Consumo do terra.
            "usage" => self.open_usage = true,
            "model" => {
                // O Default do menu é o modelo padrão salvo ou o do agente.
                let default = self.default_model();
                let effort = self.session_effort.clone().or(self.default_effort.clone());
                let mut menu = ModelMenu::new(default, effort);
                menu.sync(self.catalog.as_ref());
                self.model_menu = Some(menu);
            }
            // Sem interface no terra, fica o padrão do router: o comando vai
            // para o agente, que responde no chat.
            _ if self.run.is_some() => self.history.push(Entry::Info(format!(
                "Espere o agente terminar para rodar /{name}."
            ))),
            _ => {
                let command = format!("/{name}");
                if !self.route_to_provider(&command) {
                    self.start(&command, Entry::User(command.clone()));
                }
            }
        }
        self.history.follow();
    }

    /// O comando `command` (com a `/` e os argumentos) que só existe na
    /// interface original abre nela, na sessão do chat; um de pagamento ou
    /// plano não roda. Retorna se o comando foi tratado aqui.
    pub(super) fn route_to_provider(&mut self, command: &str) -> bool {
        let name = command
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default();
        let Some(adapter) = self.available_adapter() else {
            return false;
        };
        match adapter.command_route(name) {
            CommandRoute::Agent => false,
            CommandRoute::Blocked => {
                self.history.push(Entry::Info(format!(
                    "O terra não roda /{name}: pagamento e plano ficam no {}.",
                    adapter.name()
                )));
                true
            }
            CommandRoute::Original => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
                let options = Interactive {
                    account: self.account.as_deref(),
                    model: self
                        .session_model
                        .as_deref()
                        .or(self.chosen_model.as_deref()),
                    effort: self
                        .session_effort
                        .as_deref()
                        .or(self.default_effort.as_deref()),
                    subagent_model: self.subagent_model.as_deref(),
                    mode: Some(self.permission_mode),
                    extensions: Some(&self.extensions),
                    session_id: self.session_id.as_deref(),
                    command: Some(command),
                };
                match adapter.interactive(&cwd, options) {
                    Some(cmd) => self.pending_original = Some((cmd, command.to_string())),
                    None => self.history.push(Entry::Info(format!(
                        "{} não tem uma interface original para {command}.",
                        adapter.name()
                    ))),
                }
                true
            }
        }
    }

    /// A interface original pedida por um comando e o comando, para a tela
    /// principal rodar fora da tela.
    pub fn take_original(&mut self) -> Option<(Command, String)> {
        self.pending_original.take()
    }

    /// Volta da interface original: avisa no histórico e recarrega os
    /// comandos (ex.: um `/plugin` pode ter instalado skills).
    pub fn finish_original(&mut self, command: &str, result: Result<(), String>) {
        match result {
            Ok(()) => self.history.push(Entry::Info(format!(
                "{command} rodou na interface original."
            ))),
            Err(reason) => self
                .history
                .push(Entry::Error(format!("{command} não rodou: {reason}"))),
        }
        self.load_catalog();
        self.history.follow();
    }

    /// O `/login`: pede à tela principal para rodar o login do provedor na
    /// conta escolhida (ele é interativo, fora da tela do terra).
    fn start_login(&mut self) {
        let login = self
            .available_adapter()
            .and_then(|adapter| adapter.login(self.account.as_deref()));
        match login {
            Some(command) => self.pending_login = Some(command),
            None => {
                let help = self.available_adapter().and_then(|a| a.login_help());
                self.history.push(Entry::Info(
                    help.unwrap_or_else(|| "Este provedor não tem login pelo terra.".to_string()),
                ));
            }
        }
    }

    #[cfg(test)]
    pub fn open_usage_for_test(&mut self) {
        self.open_usage = true;
    }

    /// O `/usage` pediu a aba Consumo desde a última chamada.
    pub fn take_open_usage(&mut self) -> bool {
        std::mem::take(&mut self.open_usage)
    }

    /// O comando de login pedido pelo `/login` e o nome da conta, para a
    /// tela principal rodar.
    pub fn take_login(&mut self) -> Option<(Command, String)> {
        let login = self.pending_login.take()?;
        Some((login, self.account_label()))
    }

    /// Volta do login: avisa no histórico e recarrega os comandos e modelos
    /// (que dependem do login).
    pub fn finish_login(&mut self, result: Result<(), String>) {
        let label = self.account_label();
        match result {
            Ok(()) => {
                self.history
                    .push(Entry::Info(format!("Login feito na conta {label}.")));
                self.load_catalog();
            }
            Err(reason) => self.history.push(Entry::Error(format!(
                "O login na conta {label} não terminou: {reason}"
            ))),
        }
        self.history.follow();
    }

    /// Um erro de autenticação do agente: sugere o `/login`. As palavras que
    /// denunciam o erro são do provedor ([`ChatAdapter::login_error_words`](crate::agent::ChatAdapter::login_error_words)).
    pub(super) fn hint_login(&mut self, message: &str) {
        let lower = message.to_lowercase();
        let auth = self.adapter.as_deref().is_some_and(|adapter| {
            adapter
                .login_error_words()
                .iter()
                .any(|word| lower.contains(word))
        });
        if auth {
            let label = self.account_label();
            self.history.push(Entry::Info(format!(
                "Use /login para entrar de novo na conta {label}."
            )));
        }
    }

    /// Encerra a sessão do agente e mantém a conversa na tela: a próxima
    /// mensagem começa uma conversa nova.
    pub(super) fn end_session(&mut self) {
        self.save_history();
        let written = self.history.entries().len();
        if let Some(recorder) = &mut self.recorder {
            recorder.new_conversation(written);
        }
        self.session_id = None;
        self.session_spend.clear();
        self.agent_mode = None;
        self.model = None;
        self.context = None;
    }

    /// `/clear`: começa uma conversa nova, com o histórico limpo.
    pub(super) fn clear_conversation(&mut self) {
        self.run = None;
        self.close_turn_spend();
        self.session_spend.clear();
        self.save_history();
        if let Some(recorder) = &mut self.recorder {
            recorder.new_conversation(0);
        }
        self.prompts.clear();
        self.allowed.clear();
        self.session_id = None;
        self.agent_mode = None;
        self.model = None;
        self.agent_activity = None;
        if self.agent_status != AgentStatus::Desconectado {
            self.agent_status = AgentStatus::Ocioso;
        }
        self.context = None;
        self.history = History::default();
        self.history.push(Entry::Info("Nova conversa.".to_string()));
    }

    /// `/compact`: pede ao agente para resumir a conversa, liberando o
    /// contexto.
    fn compact(&mut self) {
        if self.session_id.is_none() {
            self.history.push(Entry::Info(
                "Ainda não há conversa para compactar.".to_string(),
            ));
        } else if self.run.is_some() {
            self.history.push(Entry::Info(
                "Espere o agente terminar para compactar a conversa.".to_string(),
            ));
        } else {
            self.start_compact(Entry::Info("Compactando a conversa...".to_string()));
        }
    }

    fn start_compact(&mut self, entry: Entry) {
        self.start("/compact", entry);
    }
}
