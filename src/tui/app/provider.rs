//! Processos do provedor fora da tela: login, interface original, instalação da
//! atualização e o terminal embutido.

use std::process::Command;

use ratatui::layout::Rect;

use super::App;
use super::draw::content_area;
use crate::agent::Interactive;
use crate::settings::StartMode;
use crate::tui::term::Term;

impl App {
    /// O comando do `/login`, para o loop rodar fora da tela.
    /// Com o nome da conta, para o login mostrar.
    pub fn take_login(&mut self) -> Option<(Command, String)> {
        self.chat.take_login()
    }

    /// O `Enter` da aba Atualização, para o loop instalar fora da tela.
    pub fn take_update_install(&mut self) -> bool {
        self.update_view.take_install()
    }

    pub fn update_finished(&mut self, result: Result<(), String>) {
        self.update_view.finish_install(result);
    }

    /// A interface original pedida por um comando da `/` do chat.
    pub fn take_original(&mut self) -> Option<(Command, String)> {
        self.current_chat_mut().take_original()
    }

    /// Volta da interface original: o chat avisa e o consumo é recarregado.
    pub fn original_finished(&mut self, command: &str, result: Result<(), String>) {
        self.current_chat_mut().finish_original(command, result);
        self.usage.invalidate();
        self.session_usage.invalidate();
    }

    /// Volta do login: o chat avisa e o consumo é recarregado com o novo
    /// login.
    pub fn login_finished(&mut self, result: Result<(), String>) {
        self.chat.finish_login(result);
        self.usage.invalidate();
        self.session_usage.invalidate();
    }

    /// O comando da interface original do provedor, com as configurações
    /// do workspace.
    fn provider_command(&self) -> Result<Command, String> {
        let dir = self
            .workspace
            .as_ref()
            .map(|w| w.dir.clone())
            .ok_or("Nenhum workspace aberto.")?;
        let settings = &self.settings;
        let options = Interactive {
            account: settings.account.as_deref(),
            model: settings.model.as_deref(),
            effort: settings.effort.as_deref(),
            subagent_model: settings.subagent_model.as_deref(),
            mode: match settings.start_mode {
                StartMode::Provider => None,
                StartMode::Fixed(mode) => Some(mode),
            },
            extensions: Some(self.chat.extensions()),
            ..Interactive::default()
        };
        self.chat
            .adapter()
            .and_then(|a| a.interactive(&dir, options))
            .ok_or_else(|| self.unavailable_provider())
    }

    fn unavailable_provider(&self) -> String {
        match self.chat.adapter() {
            Some(a) if !a.is_available() => a.unavailable_hint(),
            Some(a) => format!("{} não tem uma interface original.", a.name()),
            None => "Nenhum provedor disponível.".to_string(),
        }
    }

    /// Mantém a interface original em dia com a tela de tamanho `screen`:
    /// abre quando pedido, acompanha o tamanho da área de conteúdo e nota
    /// quando ela sai. Quando a Interface volta ao Agente Terraforme, ela só
    /// é fechada se o cliente ainda não enviou nenhuma mensagem; com uma
    /// sessão, continua rodando e reaparece ao voltar a Interface.
    pub fn sync_term(&mut self, screen: Rect) {
        let area = content_area(screen);
        if let Some(term) = &mut self.term
            && let Some(result) = term.finished()
        {
            self.term = None;
            self.term_focus = false;
            self.provider_result = Some(result);
            self.usage.invalidate();
            self.session_usage.invalidate();
        }
        if !self.uses_provider() {
            if self.term.as_ref().is_some_and(|t| !t.has_session()) {
                self.term = None;
            }
            self.term_focus = false;
            self.open_provider = false;
            return;
        }
        if std::mem::take(&mut self.open_provider) {
            match self
                .provider_command()
                .and_then(|cmd| Term::spawn(&cmd, area.height, area.width))
            {
                Ok(term) => {
                    self.term = Some(term);
                    self.term_focus = true;
                    self.provider_result = None;
                }
                Err(message) => self.provider_result = Some(Err(message)),
            }
        }
        let Some(term) = &mut self.term else {
            return;
        };
        term.resize(area.height, area.width);
    }
}
