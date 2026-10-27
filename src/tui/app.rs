//! Tela principal do `terra cli`: barra de abas no topo e o conteúdo da aba
//! ativa logo abaixo. `Esc` leva o foco ao botão "Sair"; `Enter` nele abre a
//! confirmação de saída. Saindo de um workspace que não está salvo, a tela
//! pergunta se ele deve ser salvo (Sim, Não ou Não, sem perguntar de novo).
//!
//! Com o MCP ativado na Config, o terra abre o MCP ao iniciar (se ninguém
//! abriu) e se registra nele; com ele online, aparece a aba MCP, o
//! orquestrador.

use std::path::PathBuf;
use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use super::chat::Chat;
use super::config::{self, ConfigView};
use super::extensions::{self, ExtensionsView};
use super::mcp::{self as mcp_tab, McpView};
use super::term::Term;
use super::usage::{SessionUsage, UsageView};
use crate::agent::Interactive;
use crate::db::Db;
use crate::extension::Registry;
use crate::mcp;
use crate::platform::keyboard;
use crate::settings::{AgentSettings, ExtensionSettings, Interface, McpSettings, StartMode};
use crate::theme;

const QUIT_LABEL: &str = " Sair ";
const TAB_GAP: &str = " ";
const HELP: &str = "Tab trocar aba · Esc ir para Sair";
const HELP_QUIT: &str = "Enter sair · Tab/Esc voltar às abas";
const HELP_PROVIDER: &str = "Enter entrar no terminal · Tab trocar aba · Esc ir para Sair";
const HELP_TERM: &str = "Ctrl+Q voltar às abas";
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
    Extensoes,
    Config,
    /// O orquestrador: só com o MCP online.
    Mcp,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Chat,
        Tab::Consumo,
        Tab::Extensoes,
        Tab::Config,
        Tab::Mcp,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Chat => "Chat",
            Tab::Consumo => "Consumo",
            Tab::Extensoes => "Extensões",
            Tab::Config => "Config",
            Tab::Mcp => "MCP",
        }
    }

    /// Próxima aba, voltando para a primeira depois da última. A aba MCP
    /// só entra com o MCP online (`mcp`).
    fn next(self, mcp: bool) -> Tab {
        let next = Self::ALL[(self as usize + 1) % Self::ALL.len()];
        if next == Tab::Mcp && !mcp {
            return next.next(mcp);
        }
        next
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

/// Os `config.json` do terra: o do workspace e o global (o do MCP).
pub struct ConfigFiles {
    pub workspace: PathBuf,
    pub global: PathBuf,
}

#[derive(Default)]
pub struct App {
    tab: Tab,
    focus: Focus,
    chat: Chat,
    config_view: ConfigView,
    usage: UsageView,
    /// O consumo da sessão, para a barra de consumo do chat.
    session_usage: SessionUsage,
    extensions_view: ExtensionsView,
    settings: AgentSettings,
    /// As extensões conhecidas (estratégias), já detectadas.
    registry: Registry,
    /// As extensões ativas no workspace.
    extensions: ExtensionSettings,
    /// O `config.json` do workspace, onde as configurações são salvas
    /// (`None` nos testes).
    config: Option<PathBuf>,
    /// O MCP, do `config.json` global.
    mcp: McpSettings,
    /// O `config.json` global (`None` nos testes).
    global_config: Option<PathBuf>,
    /// A ligação com o MCP, com ele ativado.
    mcp_link: Option<mcp::Link>,
    mcp_view: McpView,
    /// O nome do workspace, para o MCP.
    name: String,
    /// O diretório aberto (`None` nos testes).
    workspace: Option<Workspace>,
    /// Onde o workspace é salvo, se o usuário pedir ao sair.
    db: Option<Db>,
    /// Abrir a interface original do provedor (na Interface "Original do
    /// provedor": ao iniciar e no `Enter` da aba Chat).
    open_provider: bool,
    /// Como terminou a última interface original, para a aba Chat mostrar.
    provider_result: Option<Result<(), String>>,
    /// A interface original rodando na área de conteúdo da aba Chat.
    term: Option<Term>,
    /// As teclas vão para o terminal (até o `Ctrl+Q`).
    term_focus: bool,
}

impl App {
    /// Tela principal com o chat ligado ao provedor disponível e as
    /// configurações do workspace `name`, em `dir`, lidas dos arquivos
    /// `files` (sem eles, o padrão do router) e salvas neles.
    pub fn new(db: Db, dir: PathBuf, name: String, files: ConfigFiles) -> Self {
        let config = files.workspace;
        let settings = AgentSettings::load_file(&config).unwrap_or_default();
        let extensions = ExtensionSettings::load_file(&config);
        let saved = db.workspace_name(&dir).ok().flatten().is_some();
        let mut app = Self {
            chat: Chat::connect(&settings),
            open_provider: settings.interface == Interface::Provider,
            settings,
            registry: Registry::detect(),
            extensions,
            config: Some(config),
            mcp: McpSettings::load_file(&files.global),
            global_config: Some(files.global),
            name,
            workspace: Some(Workspace { dir, saved }),
            db: Some(db),
            ..Self::default()
        };
        app.apply_extensions();
        if app.mcp.enabled {
            app.start_mcp();
        }
        app
    }

    /// Abre o MCP, se ainda não há um na porta, e liga este terra a ele.
    fn start_mcp(&mut self) {
        // Sem workspace (nos testes), nada abre.
        let Some(workspace) = &self.workspace else {
            return;
        };
        let dir = workspace.dir.clone();
        if let Err(err) = mcp::ensure_running() {
            self.chat.warn(format!("MCP: {err:#}"));
        }
        self.mcp_link = Some(mcp::Link::start(mcp::Instance {
            pid: std::process::id(),
            name: self.name.clone(),
            dir,
            session: self.has_session(),
        }));
    }

    /// Sai do MCP e o fecha (desativado, ele fecha para todos os terra).
    fn stop_mcp(&mut self) {
        if self.mcp_link.take().is_some() {
            let _ = mcp::shutdown();
        }
        if self.tab == Tab::Mcp {
            self.tab = Tab::Config;
        }
    }

    /// Se o MCP está online (e a aba MCP aparece).
    fn mcp_online(&self) -> bool {
        self.mcp_link.as_ref().is_some_and(mcp::Link::is_online)
    }

    /// Há uma conversa com o agente: no chat do terra ou na interface
    /// original do provedor.
    fn has_session(&self) -> bool {
        self.chat.has_session() || self.term.as_ref().is_some_and(Term::has_session)
    }

    /// Passa ao chat o que as extensões ativas acrescentam ao agente (vale
    /// a partir da próxima mensagem); as que falharem viram avisos.
    fn apply_extensions(&mut self) {
        let (contribution, warnings) = self.registry.contribution(&self.extensions.enabled);
        self.chat.set_extensions(contribution);
        for warning in warnings {
            self.chat.warn(format!("Extensão {warning}"));
        }
    }

    /// Um problema ao abrir (ex.: sem o banco), mostrado no chat.
    pub fn warn(&mut self, message: String) {
        self.chat.warn(message);
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
        match self.config_view.handle_key(
            key,
            &mut self.settings,
            &mut self.extensions,
            &mut self.mcp,
            &choices,
        ) {
            config::Outcome::Ignored => false,
            config::Outcome::Moved => true,
            config::Outcome::Changed => {
                self.save_settings(&previous);
                true
            }
            config::Outcome::ExtensionsChanged => {
                if let Some(config) = &self.config {
                    // Sem conseguir salvar, a mudança vale só nesta execução.
                    let _ = self.extensions.save_file(config);
                }
                self.apply_extensions();
                true
            }
            config::Outcome::McpChanged => {
                if let Some(global) = &self.global_config {
                    // Sem conseguir salvar, a mudança vale só nesta execução.
                    let _ = self.mcp.save_file(global);
                }
                if self.mcp.enabled {
                    self.start_mcp();
                } else {
                    self.stop_mcp();
                }
                true
            }
        }
    }

    /// Opções da Config: contas do provedor, modelos do catálogo do chat e
    /// as extensões do registro.
    fn choices(&self) -> config::Choices {
        config::Choices {
            accounts: self
                .chat
                .adapter()
                .map(|a| a.accounts())
                .unwrap_or_default(),
            models: self.chat.model_options(),
            context_window: self.chat.context_window(),
            extensions: self
                .registry
                .all()
                .map(|e| config::ExtensionOption {
                    id: e.id().to_string(),
                    name: e.name().to_string(),
                    description: e.description().to_string(),
                    homepage: e.homepage().to_string(),
                    status: e.status(),
                })
                .collect(),
            mcp: config::McpState {
                settings: self.mcp,
                online: self.mcp_online(),
            },
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
            mode: match settings.start_mode {
                StartMode::Provider => None,
                StartMode::Fixed(mode) => Some(mode),
            },
            extensions: Some(self.chat.extensions()),
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

    /// As teclas vão para o terminal: o `Ctrl+C` também é dele.
    pub fn captures_keys(&self) -> bool {
        self.tab == Tab::Chat && self.focus == Focus::Tabs && self.term_focus && self.term.is_some()
    }

    fn uses_provider(&self) -> bool {
        self.settings.interface == Interface::Provider
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
        } else if self.tab == Tab::Chat && self.focus == Focus::Tabs && self.chat.handle_key(key) {
            // Na aba Chat, o painel do agente tem prioridade (modo INSERT, Shift+Tab).
            self.take_default_model();
            if self.chat.take_open_usage() {
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
        if self.tab == Tab::Mcp && self.focus == Focus::Tabs && self.mcp_view.handle_key(key) {
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
            (Focus::Tabs, KeyCode::Tab) => self.tab = self.tab.next(self.mcp_online()),
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
            Tab::Chat if self.uses_provider() => {}
            Tab::Chat => self.chat.handle_mouse(mouse),
            Tab::Consumo => self.usage.handle_mouse(mouse),
            Tab::Extensoes | Tab::Config | Tab::Mcp => {}
        }
    }

    /// Avança as animações; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        for reason in crate::agent::guard::take_panics() {
            self.chat.provider_failed(&reason);
        }
        self.chat
            .set_caps_lock(keyboard::caps_lock().unwrap_or(false));
        self.chat.tick();
        // O consumo só é carregado com a aba aberta.
        if self.tab == Tab::Consumo {
            self.usage
                .refresh_if_stale(self.chat.adapter(), self.settings.account.as_deref());
        }
        self.usage.tick();
        // A barra de consumo carrega só enquanto está visível.
        if self.settings.usage.visible {
            self.session_usage
                .refresh_if_stale(self.chat.adapter(), self.settings.account.as_deref());
        }
        if self.session_usage.tick() {
            self.chat
                .set_session_usage(self.session_usage.window().cloned());
        }
        // O relatório das extensões, também só com a aba aberta.
        if self.tab == Tab::Extensoes {
            self.extensions_view.refresh_if_stale(&self.registry);
        }
        self.extensions_view.tick();
        self.tick_mcp();
    }

    /// Avisa o MCP se há uma sessão e, com a aba MCP aberta, recarrega os
    /// spaces. Com o MCP offline, a aba some.
    fn tick_mcp(&mut self) {
        let session = self.has_session();
        let Some(link) = &self.mcp_link else {
            return;
        };
        link.set_session(session);
        if self.tab != Tab::Mcp {
            return;
        }
        let Some(instances) = link.instances() else {
            self.tab = Tab::Config;
            return;
        };
        if self.mcp_view.is_stale() {
            let saved = self
                .db
                .as_ref()
                .and_then(|db| db.workspaces(0, usize::from(u16::MAX)).ok())
                .unwrap_or_default();
            self.mcp_view.refresh(&saved, &instances);
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(theme::screen()), area);
        let [nav, content, footer] = layout(area);

        self.draw_nav(frame, nav);
        self.draw_content(frame, content);
        let help = match self.focus {
            Focus::Tabs if self.captures_keys() => HELP_TERM.into(),
            Focus::Tabs if self.tab == Tab::Chat && self.uses_provider() => HELP_PROVIDER.into(),
            Focus::Tabs if self.tab == Tab::Chat => self.chat.help(),
            Focus::Tabs if self.tab == Tab::Config => self.config_view.help().into(),
            Focus::Tabs if self.tab == Tab::Extensoes => extensions::HELP.into(),
            Focus::Tabs if self.tab == Tab::Mcp => mcp_tab::HELP.into(),
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
        let tabs = Tab::ALL
            .into_iter()
            .filter(|tab| *tab != Tab::Mcp || self.mcp_online());
        for (i, tab) in tabs.enumerate() {
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
        if self.tab == Tab::Chat && self.uses_provider() {
            match &self.term {
                Some(term) => term.draw(frame, area, self.captures_keys()),
                None => self.draw_provider(frame, area),
            }
            return;
        }
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
                &self.extensions,
                self.chat.adapter(),
                &self.choices(),
            );
            return;
        }
        if self.tab == Tab::Mcp {
            self.mcp_view.draw(frame, area);
            return;
        }
        if self.tab == Tab::Extensoes {
            self.extensions_view
                .draw(frame, area, &self.registry, &self.extensions);
            return;
        }
        let route = match self.chat.adapter() {
            Some(adapter) => format!("{} · {}", adapter.name(), self.chat.account_label()),
            None => String::new(),
        };
        self.usage.draw(frame, area, self.chat.adapter(), &route);
    }
}

impl App {
    /// A aba Chat na Interface "Original do provedor" sem a interface
    /// rodando: como abrir e como a última terminou.
    fn draw_provider(&self, frame: &mut Frame, area: Rect) {
        let name = self.chat.adapter().map_or("provedor", |a| a.name());
        let mut lines = vec![
            Line::styled(" Interface original", theme::section_title()),
            Line::from(vec![
                Span::styled(format!("  {name}"), theme::text()),
                Span::styled(format!(" · {}", self.chat.account_label()), theme::muted()),
            ]),
            Line::default(),
            Line::styled(
                format!("  Enter abre o chat do {name} neste workspace, aqui na aba Chat."),
                theme::muted(),
            ),
            Line::styled(
                "  Para usar o chat do terra, troque a Interface na aba Config.",
                theme::muted(),
            ),
        ];
        match &self.provider_result {
            Some(Ok(())) => {
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("  {name} encerrado."),
                    theme::variant(theme::Variant::Success),
                ));
            }
            Some(Err(message)) => {
                lines.push(Line::default());
                lines.push(Line::styled(format!("  {message}"), theme::error()));
            }
            None => {}
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
    }
}

/// Barra de abas, conteúdo da aba e rodapé.
fn layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// A área de conteúdo numa tela `screen`.
fn content_area(screen: Rect) -> Rect {
    layout(screen)[1]
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c)
        && key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
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
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Extensoes);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    }

    fn key(app: &mut App, code: KeyCode) -> bool {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn mcp_tab_only_shows_with_the_mcp_online() {
        assert_eq!(Tab::Config.next(false), Tab::Chat);
        assert_eq!(Tab::Config.next(true), Tab::Mcp);
        assert_eq!(Tab::Mcp.next(true), Tab::Chat);
    }

    #[test]
    fn turning_the_mcp_on_in_config_saves_it_in_the_global_file() {
        let dir = tempfile::tempdir().unwrap();
        let global = crate::settings::global_file(dir.path());
        let mut app = App {
            global_config: Some(global.clone()),
            tab: Tab::Config,
            ..App::default()
        };
        // O MCP é a última configuração da seção Agente.
        for _ in 0..20 {
            key(&mut app, KeyCode::Down);
        }
        key(&mut app, KeyCode::Enter);
        assert!(app.mcp.enabled);
        assert!(McpSettings::load_file(&global).enabled);
        // Sem workspace (nos testes), não se liga ao MCP.
        assert!(app.mcp_link.is_none());
        key(&mut app, KeyCode::Enter);
        assert!(!McpSettings::load_file(&global).enabled);
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
        app.tab = Tab::Config;
        key(&mut app, KeyCode::Down); // Modo inicial
        key(&mut app, KeyCode::Right);
        let saved = AgentSettings::load_file(&config).expect("config.json gravado");
        assert_eq!(saved, app.settings);
        assert_ne!(saved, AgentSettings::default());
    }

    #[test]
    fn extensions_toggled_in_config_are_saved_and_reach_the_chat() {
        use crate::extension::{Contribution, Extension, Hook, Status};

        struct Fake;
        impl Extension for Fake {
            fn id(&self) -> &'static str {
                "fake"
            }
            fn name(&self) -> &'static str {
                "Fake"
            }
            fn description(&self) -> &'static str {
                "faz"
            }
            fn homepage(&self) -> &'static str {
                "https://fake"
            }
            fn status(&self) -> Status {
                Status::Installed { version: None }
            }
            fn contribution(&self) -> Result<Contribution, String> {
                Ok(Contribution {
                    hooks: vec![Hook {
                        event: "PreToolUse".into(),
                        matcher: "Bash".into(),
                        command: "fake".into(),
                    }],
                    mcp_servers: Vec::new(),
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let config = crate::settings::config_file(dir.path(), "terra");
        let mut app = App {
            config: Some(config.clone()),
            registry: Registry::new(vec![Box::new(Fake)]),
            tab: Tab::Config,
            ..App::default()
        };
        assert!(!key(&mut app, KeyCode::BackTab));
        assert!(!key(&mut app, KeyCode::Char(' ')));
        assert_eq!(ExtensionSettings::load_file(&config).enabled, ["fake"]);
        assert_eq!(app.chat.extensions().hooks[0].command, "fake");
        // O agente continua igual.
        assert_eq!(
            AgentSettings::load_file(&config).unwrap_or_default(),
            AgentSettings::default()
        );

        key(&mut app, KeyCode::Char(' '));
        assert!(ExtensionSettings::load_file(&config).enabled.is_empty());
        assert_eq!(app.chat.extensions(), &Contribution::default());

        // A aba Extensões fica entre Consumo e Config.
        app.tab = Tab::Consumo;
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Extensoes);
        assert!(!key(&mut app, KeyCode::Down));
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
    }

    #[test]
    fn provider_interface_opens_with_enter_and_chat_keys_are_not_used() {
        let mut app = App {
            settings: AgentSettings {
                interface: Interface::Provider,
                ..AgentSettings::default()
            },
            ..App::default()
        };
        // `i` entraria no modo INSERT do chat do terra.
        key(&mut app, KeyCode::Char('i'));
        assert!(!app.open_provider);
        key(&mut app, KeyCode::Enter);
        assert!(app.open_provider);
        // Sem provedor (nos testes), não há comando e a aba mostra o porquê.
        app.sync_term(Rect::new(0, 0, 80, 24));
        assert!(app.term.is_none());
        assert!(!app.open_provider);
        assert!(matches!(app.provider_result, Some(Err(_))));
        assert_eq!(
            press(&mut app, KeyCode::Tab),
            Tab::Consumo,
            "Tab troca a aba"
        );
    }

    #[cfg(unix)]
    #[test]
    fn switching_interface_keeps_the_terminal_only_with_a_session() {
        let screen = Rect::new(0, 0, 80, 24);
        let mut app = App {
            settings: AgentSettings {
                interface: Interface::Provider,
                ..AgentSettings::default()
            },
            ..App::default()
        };
        let cat = Command::new("cat");
        app.term = Some(Term::spawn(&cat, 20, 80).unwrap());
        app.term_focus = true;

        // Sem mensagem enviada, trocar para o Agente Terraforme fecha.
        app.settings.interface = Interface::Terraforme;
        app.sync_term(screen);
        assert!(app.term.is_none());

        // Com uma mensagem enviada, continua rodando e reaparece.
        app.settings.interface = Interface::Provider;
        app.term = Some(Term::spawn(&cat, 20, 80).unwrap());
        app.term_focus = true;
        key(&mut app, KeyCode::Enter);
        app.settings.interface = Interface::Terraforme;
        app.sync_term(screen);
        assert!(app.term.is_some(), "a sessão não é fechada");
        assert!(!app.captures_keys());
        app.settings.interface = Interface::Provider;
        app.sync_term(screen);
        assert!(app.term.is_some());
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
        assert_eq!(row(1), "│  Chat   Consumo   Extensões   Config     Sair  │");
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
