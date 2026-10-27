//! Tela principal do `terra cli`: barra de abas no topo e o conteúdo da aba
//! ativa logo abaixo. `Esc` leva o foco ao botão "Sair"; `Enter` nele abre a
//! confirmação de saída. Saindo de um workspace que não está salvo, a tela
//! pergunta se ele deve ser salvo (Sim, Não ou Não, sem perguntar de novo).

use std::path::PathBuf;
use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use super::chat::Chat;
use super::config::{self, ConfigView};
use super::usage::UsageView;
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
const SAVE_TITLE: &str = " Salvar workspace ";
const SAVE_TEXT: &str = "Este workspace não está salvo. Deseja salvar?";
/// As opções da pergunta de salvar, na ordem de [`SaveChoice`].
const SAVE_LABELS: [&str; 3] = ["  Sim  ", "  Não  ", "  Não (não perguntar novamente)  "];

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
    /// Pergunta, ao sair, se o workspace deve ser salvo; a opção selecionada.
    SaveAsk(SaveChoice),
}

/// As opções da pergunta de salvar o workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveChoice {
    Yes,
    No,
    /// Não, e não perguntar mais neste workspace.
    NeverAsk,
}

impl SaveChoice {
    const ALL: [SaveChoice; 3] = [SaveChoice::Yes, SaveChoice::No, SaveChoice::NeverAsk];

    /// A opção ao lado (`step` 1 ou -1), dando a volta.
    fn step(self, step: isize) -> SaveChoice {
        let i = (self as isize + step).rem_euclid(Self::ALL.len() as isize);
        Self::ALL[i as usize]
    }
}

/// O diretório aberto e se ele já é um workspace salvo.
struct Workspace {
    dir: PathBuf,
    saved: bool,
}

#[derive(Default)]
pub struct App {
    tab: Tab,
    focus: Focus,
    chat: Chat,
    config_view: ConfigView,
    usage: UsageView,
    settings: AgentSettings,
    /// O `config.json` do workspace, onde as configurações são salvas
    /// (`None` nos testes).
    config: Option<PathBuf>,
    /// O diretório aberto (`None` nos testes).
    workspace: Option<Workspace>,
    /// Onde o workspace é salvo, se o usuário pedir ao sair.
    db: Option<Db>,
}

impl App {
    /// Tela principal com o chat ligado ao provedor disponível e as
    /// configurações do workspace `dir`, lidas de `config` (sem o arquivo, o
    /// padrão do router) e salvas nele.
    pub fn new(db: Db, dir: PathBuf, config: PathBuf) -> Self {
        let settings = AgentSettings::load_file(&config).unwrap_or_default();
        let saved = db.workspace_name(&dir).ok().flatten().is_some();
        Self {
            chat: Chat::connect(&settings),
            settings,
            config: Some(config),
            workspace: Some(Workspace { dir, saved }),
            db: Some(db),
            ..Self::default()
        }
    }

    /// Ao sair, pergunta se salva o workspace: só quando ele não está salvo e
    /// ninguém escolheu "não perguntar novamente".
    fn asks_to_save(&self) -> bool {
        self.workspace.as_ref().is_some_and(|w| !w.saved)
            && self
                .config
                .as_deref()
                .is_none_or(crate::settings::asks_to_save)
    }

    /// Aplica a resposta da pergunta de salvar. Saindo, um erro não tem onde
    /// aparecer: o terra sai do mesmo jeito.
    fn answer_save(&mut self, choice: SaveChoice) {
        match choice {
            SaveChoice::Yes => {
                if let (Some(db), Some(workspace)) = (&self.db, &mut self.workspace) {
                    workspace.saved = db.save_workspace(&workspace.dir, None).is_ok();
                }
            }
            SaveChoice::No => {}
            SaveChoice::NeverAsk => {
                if let Some(config) = &self.config {
                    let _ = crate::settings::stop_asking_to_save(config);
                }
            }
        }
    }

    /// Na aba Config, as setas mudam as configurações; cada mudança é salva
    /// e vale na hora para o chat. Retorna `true` se a tecla foi usada.
    fn handle_config_key(&mut self, key: KeyEvent) -> bool {
        let choices = self.choices();
        let previous = self.settings.clone();
        match self
            .config_view
            .handle_key(key, &mut self.settings, &choices)
        {
            config::Outcome::Ignored => false,
            config::Outcome::Moved => true,
            config::Outcome::Changed => {
                self.save_settings(&previous);
                true
            }
        }
    }

    /// Opções da Config: contas do provedor e modelos do catálogo do chat.
    fn choices(&self) -> config::Choices {
        config::Choices {
            accounts: self
                .chat
                .adapter()
                .map(|a| a.accounts())
                .unwrap_or_default(),
            models: self.chat.model_options(),
        }
    }

    /// Salva as configurações e as aplica ao chat.
    fn save_settings(&mut self, previous: &AgentSettings) {
        if let Some(config) = &self.config {
            // Sem conseguir salvar, a mudança vale só nesta execução.
            let _ = self.settings.save_file(config);
        }
        self.chat.apply_settings(&self.settings, Some(previous));
    }

    /// O `/model` escolheu um novo modelo padrão: vai para as configurações.
    fn take_default_model(&mut self) {
        if let Some((model, effort)) = self.chat.take_default_choice() {
            let previous = self.settings.clone();
            self.settings.model = Some(model);
            self.settings.effort = effort;
            self.save_settings(&previous);
        }
    }

    /// O comando do `/login`, para o loop rodar fora da tela.
    /// Com o nome da conta, para o login mostrar.
    pub fn take_login(&mut self) -> Option<(Command, String)> {
        self.chat.take_login()
    }

    /// Volta do login: o chat avisa e o consumo é recarregado com o novo
    /// login.
    pub fn login_finished(&mut self, result: Result<(), String>) {
        self.chat.finish_login(result);
        self.usage.invalidate();
    }

    /// Trata uma tecla. Retorna `true` quando a saída foi confirmada.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Na aba Chat, o painel do agente tem prioridade (modo INSERT, Shift+Tab).
        if self.tab == Tab::Chat && self.focus == Focus::Tabs && self.chat.handle_key(key) {
            self.take_default_model();
            if self.chat.take_open_usage() {
                self.tab = Tab::Consumo;
            }
            return false;
        }
        if self.tab == Tab::Config && self.focus == Focus::Tabs && self.handle_config_key(key) {
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
            (Focus::Tabs, KeyCode::Tab) => self.tab = self.tab.next(),
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

    /// Mouse: no input da aba Chat (posicionar o cursor e selecionar) e nos
    /// botões do gráfico da aba Consumo.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.focus != Focus::Tabs {
            return;
        }
        match self.tab {
            Tab::Chat => self.chat.handle_mouse(mouse),
            Tab::Consumo => self.usage.handle_mouse(mouse),
            Tab::Config => {}
        }
    }

    /// Avança as animações; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        self.chat
            .set_caps_lock(keyboard::caps_lock().unwrap_or(false));
        self.chat.tick();
        // O consumo só é carregado com a aba aberta.
        if self.tab == Tab::Consumo {
            self.usage
                .refresh_if_stale(self.chat.adapter(), self.settings.account.as_deref());
        }
        self.usage.tick();
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
            Focus::Tabs if self.tab == Tab::Consumo => self.usage.help().into(),
            Focus::Tabs => HELP.into(),
            Focus::Quit => HELP_QUIT.into(),
            Focus::Confirm(_) | Focus::SaveAsk(_) => HELP_CONFIRM.into(),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {help}"), theme::muted())),
            footer,
        );
        match self.focus {
            Focus::Confirm(yes) => draw_confirm(frame, area, yes),
            Focus::SaveAsk(choice) => draw_save_ask(frame, area, choice),
            _ => {}
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
            self.config_view.draw(
                frame,
                area,
                &self.settings,
                self.chat.adapter(),
                &self.choices(),
            );
            return;
        }
        let route = match self.chat.adapter() {
            Some(adapter) => format!("{} · {}", adapter.name(), self.chat.account_label()),
            None => String::new(),
        };
        self.usage.draw(frame, area, self.chat.adapter(), &route);
    }
}

/// A pergunta de salvar o workspace, centralizada sobre a tela.
fn draw_save_ask(frame: &mut Frame, area: Rect, choice: SaveChoice) {
    let [area] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(62)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Span::styled(SAVE_TITLE, theme::section_title()))
        .style(theme::screen())
        .padding(Padding::vertical(1));
    let mut buttons = Vec::new();
    for (option, label) in SaveChoice::ALL.into_iter().zip(SAVE_LABELS) {
        if !buttons.is_empty() {
            buttons.push(Span::raw("   "));
        }
        let selected = option == choice;
        let style = match option {
            SaveChoice::Yes => theme::confirm_yes(selected),
            SaveChoice::No | SaveChoice::NeverAsk => theme::confirm_no(selected),
        };
        buttons.push(Span::styled(label, style));
    }
    let lines = vec![Line::from(SAVE_TEXT), Line::default(), Line::from(buttons)];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        area,
    );
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
    fn default_model_chosen_in_chat_goes_to_settings() {
        let mut app = App::default();
        app.chat
            .default_choice_for_test(("haiku".into(), Some("low".into())));
        app.take_default_model();
        assert_eq!(app.settings.model.as_deref(), Some("haiku"));
        assert_eq!(app.settings.effort.as_deref(), Some("low"));
    }

    #[test]
    fn usage_command_opens_the_consumo_tab() {
        let mut app = App::default();
        app.chat.open_usage_for_test();
        // Qualquer tecla que o chat trate (aqui, entrar no modo INSERT).
        key(&mut app, KeyCode::Char('i'));
        assert_eq!(app.tab, Tab::Consumo);
    }

    #[test]
    fn config_changes_are_saved_in_the_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::settings::config_file(dir.path(), "terra");
        let mut app = App {
            config: Some(config.clone()),
            ..App::default()
        };
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Down); // Modo inicial
        key(&mut app, KeyCode::Right);
        let saved = AgentSettings::load_file(&config).expect("config.json gravado");
        assert_eq!(saved, app.settings);
        assert_ne!(saved, AgentSettings::default());
    }

    /// Uma tela num workspace não salvo, com o `config.json` em `dir`.
    fn unsaved_app(dir: &std::path::Path) -> App {
        App {
            config: Some(crate::settings::config_file(dir, "projeto")),
            workspace: Some(Workspace {
                dir: dir.join("projeto"),
                saved: false,
            }),
            db: Some(Db::open_in_memory().unwrap()),
            ..App::default()
        }
    }

    /// Esc, Enter em Sair e Enter no "Sim" da confirmação.
    fn confirm_quit(app: &mut App) -> bool {
        key(app, KeyCode::Esc);
        key(app, KeyCode::Enter);
        key(app, KeyCode::Enter)
    }

    #[test]
    fn quitting_an_unsaved_workspace_asks_and_yes_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = unsaved_app(dir.path());
        assert!(!confirm_quit(&mut app), "ainda não saiu: pergunta");
        assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::Yes));
        assert!(key(&mut app, KeyCode::Enter), "Sim sai");
        let db = app.db.as_ref().unwrap();
        assert_eq!(
            db.workspace_name(&dir.path().join("projeto")).unwrap(),
            Some("projeto".into())
        );
    }

    #[test]
    fn no_quits_without_saving_and_asks_again_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = unsaved_app(dir.path());
        confirm_quit(&mut app);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::No));
        assert!(key(&mut app, KeyCode::Enter));
        assert!(!app.workspace.as_ref().unwrap().saved);
        assert!(unsaved_app(dir.path()).asks_to_save());
    }

    #[test]
    fn never_ask_quits_and_stops_asking_in_this_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = unsaved_app(dir.path());
        confirm_quit(&mut app);
        // ← dá a volta: do Sim para a última opção.
        key(&mut app, KeyCode::Left);
        assert_eq!(app.focus, Focus::SaveAsk(SaveChoice::NeverAsk));
        assert!(key(&mut app, KeyCode::Enter));

        // Da próxima vez, o Sim da confirmação já sai.
        let mut app = unsaved_app(dir.path());
        assert!(confirm_quit(&mut app));
    }

    #[test]
    fn saved_workspace_quits_without_asking_and_esc_cancels_the_question() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = unsaved_app(dir.path());
        confirm_quit(&mut app);
        assert!(!key(&mut app, KeyCode::Esc));
        assert_eq!(app.focus, Focus::Quit, "Esc desiste de sair");

        app.workspace.as_mut().unwrap().saved = true;
        key(&mut app, KeyCode::Enter);
        assert!(key(&mut app, KeyCode::Enter), "salvo: sai direto");
    }

    #[test]
    fn save_question_shows_the_three_options() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = tempfile::tempdir().unwrap();
        let mut app = unsaved_app(dir.path());
        confirm_quit(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen = (0..14)
            .map(|y| (0..70).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Salvar workspace"), "{screen}");
        assert!(screen.contains(SAVE_TEXT), "{screen}");
        assert!(
            screen.contains("Sim       Não       Não (não perguntar novamente)"),
            "{screen}"
        );
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
