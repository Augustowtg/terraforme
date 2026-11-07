//! Teclado, colagem e mouse da tela principal: quem recebe cada tecla, conforme
//! o foco e a aba ativa.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::{App, Focus, SaveChoice, Tab};
use crate::tui::config;

impl App {
    /// Na aba Config, as setas mudam as configurações; cada mudança é salva
    /// e vale na hora para o chat. Retorna `true` se a tecla foi usada.
    fn handle_config_key(&mut self, key: KeyEvent) -> bool {
        let choices = self.choices();
        let previous = self.settings.clone();
        let previous_extensions = self.extensions.clone();
        match self
            .config_view
            .handle_key(key, &mut self.settings, &mut self.extensions, &choices)
        {
            config::Outcome::Ignored => false,
            config::Outcome::Moved => true,
            config::Outcome::Changed => {
                self.save_settings(&previous);
                true
            }
            config::Outcome::ExtensionsChanged => {
                self.save_extensions(&previous_extensions);
                self.apply_extensions();
                true
            }
        }
    }

    /// Trata uma tecla. Retorna `true` quando a saída foi confirmada.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.captures_keys() {
            if is_ctrl(&key, 'q') {
                self.term_focus = false;
            } else if let Some(term) = &mut self.term {
                term.send_key(key);
            }
            return false;
        }
        // Na Interface "Original do provedor", o Enter da aba Chat entra no
        // terminal (abrindo a interface, se ela não está rodando).
        if self.tab == Tab::Chat && self.focus == Focus::Tabs && self.uses_provider() {
            if key.code == KeyCode::Enter {
                if self.term.is_some() {
                    self.term_focus = true;
                } else {
                    self.open_provider = true;
                }
                return false;
            }
        } else if self.tab == Tab::Chat && self.focus == Focus::Tabs && self.handle_chats_key(key) {
            return false;
        } else if self.tab == Tab::Chat
            && self.focus == Focus::Tabs
            && self.current_chat_mut().handle_key(key)
        {
            // Na aba Chat, o painel do agente tem prioridade (modo INSERT, Shift+Tab).
            self.take_default_model();
            if self.current_chat_mut().take_open_usage() {
                self.tab = Tab::Consumo;
            }
            return false;
        }
        if self.tab == Tab::Config && self.focus == Focus::Tabs && self.handle_config_key(key) {
            return false;
        }
        if self.tab == Tab::Extensoes
            && self.focus == Focus::Tabs
            && self.extensions_view.handle_key(key, &self.registry)
        {
            return false;
        }
        if self.tab == Tab::Orquestrador
            && self.focus == Focus::Tabs
            && self.orchestrator_view.handle_key(key)
        {
            return false;
        }
        if self.tab == Tab::Atualizacao
            && self.focus == Focus::Tabs
            && self.update_view.handle_key(key)
        {
            return false;
        }
        if self.tab == Tab::Consumo
            && self.focus == Focus::Tabs
            && self
                .usage
                .handle_key(key, self.chat.adapter(), self.settings.account.as_deref())
        {
            return false;
        }
        match (self.focus, key.code) {
            (Focus::Tabs, KeyCode::Tab) => self.tab = self.tab.next(self.update_view.available()),
            (Focus::Tabs, KeyCode::Esc) => self.focus = Focus::Quit,
            (Focus::Quit, KeyCode::Enter) => self.focus = Focus::Confirm(true),
            (Focus::Quit, KeyCode::Tab | KeyCode::Esc) => self.focus = Focus::Tabs,
            (Focus::Confirm(yes), KeyCode::Left | KeyCode::Right | KeyCode::Tab) => {
                self.focus = Focus::Confirm(!yes)
            }
            (Focus::Confirm(true), KeyCode::Enter) if self.asks_to_save() => {
                self.focus = Focus::SaveAsk(SaveChoice::Yes)
            }
            (Focus::Confirm(true), KeyCode::Enter) => return true,
            (Focus::Confirm(false), KeyCode::Enter) | (Focus::Confirm(_), KeyCode::Esc) => {
                self.focus = Focus::Quit
            }
            (Focus::SaveAsk(choice), KeyCode::Left) => self.focus = Focus::SaveAsk(choice.step(-1)),
            (Focus::SaveAsk(choice), KeyCode::Right | KeyCode::Tab) => {
                self.focus = Focus::SaveAsk(choice.step(1))
            }
            (Focus::SaveAsk(choice), KeyCode::Enter) => {
                self.answer_save(choice);
                return true;
            }
            // Esc desiste de sair.
            (Focus::SaveAsk(_), KeyCode::Esc) => self.focus = Focus::Quit,
            _ => {}
        }
        false
    }

    /// Texto colado no terminal (bracketed paste, ou uma rajada de teclas
    /// com quebra de linha): vai para o terminal do provedor ou para o input
    /// do chat; nas outras abas, é ignorado.
    pub fn handle_paste(&mut self, text: &str) {
        if self.captures_keys() {
            if let Some(term) = &mut self.term {
                term.send_paste(text);
            }
            return;
        }
        if self.tab == Tab::Chat && self.focus == Focus::Tabs && !self.uses_provider() {
            self.current_chat_mut().handle_paste(text);
        }
    }

    /// Na barra de chats, `Alt+N` abre um chat novo, `Alt+←/→` troca de chat
    /// e `Alt+W` fecha o chat aberto. Retorna `true` se a tecla foi usada.
    fn handle_chats_key(&mut self, key: KeyEvent) -> bool {
        if !key.modifiers.contains(crossterm::event::KeyModifiers::ALT) {
            return false;
        }
        if let KeyCode::Char('n' | 'N') = key.code {
            self.new_chat();
            return true;
        }
        if self.chats.is_empty() {
            return false;
        }
        let total = self.chats.len() + 1;
        match key.code {
            KeyCode::Left => self.active_chat = (self.active_chat + total - 1) % total,
            KeyCode::Right => self.active_chat = (self.active_chat + 1) % total,
            KeyCode::Char('w' | 'W') => self.close_chat(),
            _ => return false,
        }
        true
    }

    /// O clique esquerdo nas abas, na barra de chats e no "Sair" (com as
    /// caixas de confirmação fechadas). Retorna `true` se o clique foi usado.
    fn click_bars(&mut self, mouse: MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left)
            || matches!(self.focus, Focus::Confirm(_) | Focus::SaveAsk(_))
        {
            return false;
        }
        let at = Position::new(mouse.column, mouse.row);
        let tab = (self.tab_rects.borrow().iter())
            .find(|(rect, _)| rect.contains(at))
            .map(|&(_, tab)| tab);
        if let Some(tab) = tab {
            self.tab = tab;
            self.focus = Focus::Tabs;
            self.term_focus = false;
            return true;
        }
        let chat = (self.chat_rects.borrow().iter())
            .find(|(rect, _)| rect.contains(at))
            .map(|&(_, i)| i);
        if let (Some(i), Tab::Chat) = (chat, self.tab) {
            self.active_chat = i;
            self.focus = Focus::Tabs;
            return true;
        }
        if self.quit_rect.get().contains(at) {
            self.focus = Focus::Confirm(true);
            self.term_focus = false;
            return true;
        }
        false
    }

    /// Mouse: no input da aba Chat (posicionar o cursor e selecionar) e nos
    /// botões do gráfico da aba Consumo.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.click_bars(mouse) {
            return;
        }
        if self.focus != Focus::Tabs {
            return;
        }
        match self.tab {
            Tab::Chat if self.uses_provider() => {}
            Tab::Chat => self.current_chat_mut().handle_mouse(mouse),
            Tab::Consumo => self.usage.handle_mouse(mouse),
            Tab::Extensoes | Tab::Config | Tab::Orquestrador | Tab::Atualizacao => {}
        }
    }
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c)
        && key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
}
