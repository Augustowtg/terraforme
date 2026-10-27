//! Tela principal do `terra cli`: barra de abas no topo e o conteúdo da aba
//! ativa logo abaixo. `Esc` leva o foco ao botão "Sair"; `Enter` nele abre a
//! confirmação de saída.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use super::chat::Chat;
use super::config::{self, ConfigView};
use crate::db::Db;
use crate::platform::keyboard;
use crate::settings::AgentSettings;
use crate::theme;

const QUIT_LABEL: &str = " Sair ";
const TAB_GAP: &str = " ";
const HELP: &str = "Tab trocar aba · Esc ir para Sair";
const HELP_QUIT: &str = "Enter sair · Tab/Esc voltar às abas";
const HELP_CONFIRM: &str = "←/→ escolher · Enter confirmar · Esc cancelar";
const CONFIRM_TITLE: &str = " Sair ";
const CONFIRM_TEXT: &str = "Deseja sair do terraforme?";
const YES_LABEL: &str = "  Sim  ";
const NO_LABEL: &str = "  Não  ";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Chat,
    Consumo,
    Config,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Chat, Tab::Consumo, Tab::Config];

    fn title(self) -> &'static str {
        match self {
            Tab::Chat => "Chat",
            Tab::Consumo => "Consumo",
            Tab::Config => "Config",
        }
    }

    /// Próxima aba, voltando para a primeira depois da última.
    fn next(self) -> Tab {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

/// Onde está o foco da tela.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Focus {
    #[default]
    Tabs,
    /// Botão "Sair" selecionado.
    Quit,
    /// Caixa de confirmação aberta; `true` quando "Sim" está selecionado.
    Confirm(bool),
}

#[derive(Default)]
pub struct App {
    tab: Tab,
    focus: Focus,
    chat: Chat,
    config: ConfigView,
    settings: AgentSettings,
    /// Onde as configurações são salvas (`None` nos testes).
    db: Option<Db>,
}

impl App {
    /// Tela principal com o chat ligado ao provedor disponível e as
    /// configurações salvas em `db`.
    pub fn new(db: Db) -> Self {
        let settings = AgentSettings::load(&db);
        Self {
            chat: Chat::connect(&settings),
            settings,
            db: Some(db),
            ..Self::default()
        }
    }

    /// Na aba Config, as setas mudam as configurações; cada mudança é salva
    /// e vale na hora para o chat. Retorna `true` se a tecla foi usada.
    fn handle_config_key(&mut self, key: KeyEvent) -> bool {
        let choices = config::Choices::of(self.chat.adapter());
        let previous = self.settings.clone();
        match self.config.handle_key(key, &mut self.settings, &choices) {
            config::Outcome::Ignored => false,
            config::Outcome::Moved => true,
            config::Outcome::Changed => {
                if let Some(db) = &self.db {
                    // Sem conseguir salvar, a mudança vale só nesta execução.
                    let _ = self.settings.save(db);
                }
                self.chat.apply_settings(&self.settings, Some(&previous));
                true
            }
        }
    }

    /// Trata uma tecla. Retorna `true` quando a saída foi confirmada.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Na aba Chat, o painel do agente tem prioridade (modo INSERT, Shift+Tab).
        if self.tab == Tab::Chat && self.focus == Focus::Tabs && self.chat.handle_key(key) {
            return false;
        }
        if self.tab == Tab::Config && self.focus == Focus::Tabs && self.handle_config_key(key) {
            return false;
        }
        match (self.focus, key.code) {
            (Focus::Tabs, KeyCode::Tab) => self.tab = self.tab.next(),
            (Focus::Tabs, KeyCode::Esc) => self.focus = Focus::Quit,
            (Focus::Quit, KeyCode::Enter) => self.focus = Focus::Confirm(true),
            (Focus::Quit, KeyCode::Tab | KeyCode::Esc) => self.focus = Focus::Tabs,
            (Focus::Confirm(yes), KeyCode::Left | KeyCode::Right | KeyCode::Tab) => {
                self.focus = Focus::Confirm(!yes)
            }
            (Focus::Confirm(true), KeyCode::Enter) => return true,
            (Focus::Confirm(false), KeyCode::Enter) | (Focus::Confirm(_), KeyCode::Esc) => {
                self.focus = Focus::Quit
            }
            _ => {}
        }
        false
    }

    /// Mouse só age no input da aba Chat (posicionar o cursor e selecionar).
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.tab == Tab::Chat && self.focus == Focus::Tabs {
            self.chat.handle_mouse(mouse);
        }
    }

    /// Avança as animações; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        self.chat
            .set_caps_lock(keyboard::caps_lock().unwrap_or(false));
        self.chat.tick();
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(theme::screen()), area);
        let [nav, content, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);

        self.draw_nav(frame, nav);
        self.draw_content(frame, content);
        let help = match self.focus {
            Focus::Tabs if self.tab == Tab::Chat => self.chat.help(),
            Focus::Tabs if self.tab == Tab::Config => config::HELP.into(),
            Focus::Tabs => HELP.into(),
            Focus::Quit => HELP_QUIT.into(),
            Focus::Confirm(_) => HELP_CONFIRM.into(),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {help}"), theme::muted())),
            footer,
        );
        if let Focus::Confirm(yes) = self.focus {
            draw_confirm(frame, area, yes);
        }
    }

    fn draw_nav(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::border())
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [tabs_area, quit_area] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(QUIT_LABEL.len() as u16),
        ])
        .areas(inner);

        let mut spans = Vec::new();
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(TAB_GAP));
            }
            let style = if tab == self.tab && self.focus == Focus::Tabs {
                theme::tab_active()
            } else {
                theme::tab_inactive()
            };
            spans.push(Span::styled(format!(" {} ", tab.title()), style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), tabs_area);

        let quit_style = if self.focus == Focus::Tabs {
            theme::quit()
        } else {
            theme::quit_focused()
        };

        frame.render_widget(
            Paragraph::new(Span::styled(QUIT_LABEL, quit_style)).alignment(Alignment::Right),
            quit_area,
        );
    }

    fn draw_content(&self, frame: &mut Frame, area: Rect) {
        if self.tab == Tab::Chat {
            let [history, panel] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(self.chat.height())])
                    .areas(area);
            self.chat.draw_history(frame, history);
            self.chat.draw(frame, panel);
            return;
        }
        if self.tab == Tab::Config {
            self.config
                .draw(frame, area, &self.settings, self.chat.adapter());
            return;
        }
        // TODO: substituir pelo conteúdo real de Consumo.
        let lines = vec![
            Line::styled(self.tab.title(), theme::section_title()),
            Line::styled("Em construção", theme::muted()),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(2, 2, 1, 0))),
            area,
        );
    }
}

/// Caixa de confirmação de saída, centralizada sobre a tela.
fn draw_confirm(frame: &mut Frame, area: Rect, yes: bool) {
    let [area] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(36)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::quit())
        .title(Span::styled(CONFIRM_TITLE, theme::quit()))
        .style(theme::screen())
        .padding(Padding::vertical(1));
    let lines = vec![
        Line::from(CONFIRM_TEXT),
        Line::default(),
        Line::from(vec![
            Span::styled(YES_LABEL, theme::confirm_yes(yes)),
            Span::raw("   "),
            Span::styled(NO_LABEL, theme::confirm_no(!yes)),
        ]),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(app: &mut App, code: KeyCode) -> Tab {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        app.tab
    }

    #[test]
    fn tab_key_cycles_tabs() {
        let mut app = App::default();
        assert_eq!(app.tab, Tab::Chat);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    }

    fn key(app: &mut App, code: KeyCode) -> bool {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn esc_focuses_quit_and_enter_opens_confirm() {
        let mut app = App::default();
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Quit);
        assert!(!key(&mut app, KeyCode::Enter));
        assert_eq!(app.focus, Focus::Confirm(true));
        assert!(key(&mut app, KeyCode::Enter));
    }

    #[test]
    fn choosing_no_closes_confirm() {
        let mut app = App::default();
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Enter);
        assert!(!key(&mut app, KeyCode::Right));
        assert_eq!(app.focus, Focus::Confirm(false));
        assert!(!key(&mut app, KeyCode::Enter));
        assert_eq!(app.focus, Focus::Quit);
    }

    #[test]
    fn esc_backs_out_of_confirm_and_quit() {
        let mut app = App::default();
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Enter);
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Quit);
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Tabs);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
    }

    #[test]
    fn esc_in_insert_mode_stays_in_chat() {
        let mut app = App::default();
        key(&mut app, KeyCode::Char('i'));
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Tabs);
        // Já no modo NORMAL, o Esc volta a ir para "Sair".
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Quit);
    }

    #[test]
    fn tab_in_insert_mode_does_not_switch_tabs() {
        let mut app = App::default();
        key(&mut app, KeyCode::Char('i'));
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    }

    #[test]
    fn config_tab_changes_settings_and_tab_still_switches() {
        use crate::agent::PermissionMode;
        use crate::settings::StartMode;

        let mut app = App::default();
        press(&mut app, KeyCode::Tab);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
        // Conta é a primeira; ↓ vai para o Modo inicial.
        assert!(!key(&mut app, KeyCode::Down));
        assert!(!key(&mut app, KeyCode::Right));
        assert_eq!(
            app.settings.start_mode,
            StartMode::Fixed(PermissionMode::Agente)
        );
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Quit, "Esc continua indo para Sair");
        key(&mut app, KeyCode::Esc);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    }

    #[test]
    fn tab_on_quit_returns_to_tabs_without_switching() {
        let mut app = App::default();
        key(&mut app, KeyCode::Esc);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
        assert_eq!(app.focus, Focus::Tabs);
    }

    #[test]
    fn other_keys_do_nothing() {
        let mut app = App::default();
        for code in [
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::BackTab,
            KeyCode::Char('2'),
            KeyCode::Char('q'),
            KeyCode::Enter,
        ] {
            assert!(!app.handle_key(KeyEvent::new(code, KeyModifiers::NONE)));
            assert_eq!(app.tab, Tab::Chat);
        }
    }

    #[test]
    fn nav_bar_highlights_active_tab_and_right_aligns_quit() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        let mut app = App::default();
        press(&mut app, KeyCode::Tab);
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let row = |y: u16| (0..50).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), format!("╭{}╮", "─".repeat(48)));
        assert_eq!(row(1), "│  Chat   Consumo   Config                 Sair  │");
        assert_eq!(row(2), format!("╰{}╯", "─".repeat(48)));

        // O pill " Consumo " (x = 9..18) usa o estilo da aba ativa; "Chat" não.
        let active_bg = theme::tab_active().bg;
        assert!((9..18).all(|x| buffer[(x, 1)].bg == active_bg.unwrap()));
        assert_ne!(buffer[(3, 1)].bg, active_bg.unwrap());

        // "Sair" (x = 42..48) é vermelho, sem fundo até receber foco.
        let red = theme::quit().fg.unwrap();
        assert!((42..48).all(|x| buffer[(x, 1)].fg == red));
    }

    #[test]
    fn confirm_box_shows_question_and_buttons() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut terminal = Terminal::new(TestBackend::new(50, 14)).unwrap();
        let mut app = App::default();
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Enter);
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let screen = (0..14)
            .map(|y| (0..50).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains(CONFIRM_TEXT));
        assert!(screen.contains("Sim"));
        assert!(screen.contains("Não"));
    }
}
