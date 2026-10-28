//! Tela principal do `terra cli`: barra de abas no topo e o conteúdo da aba
//! ativa logo abaixo. `Esc` leva o foco ao botão "Sair"; `Enter` nele abre a
//! confirmação de saída. Saindo de um workspace que não está salvo, a tela
//! pergunta se ele deve ser salvo (Sim, Não ou Não, sem perguntar de novo).
//!
//! O orquestrador é a aba com os terminais do terra abertos na máquina,
//! separados por workspace (vindos do daemon orquestrador, o [`hub`]), e as
//! configurações dos chats delegados (os sub agentes), que por enquanto só
//! ficam salvas. Cada terra conta ao hub o que está fazendo a cada tick. Chats além do principal se abrem
//! com `Alt+N`: aparecem numa barra de chats embaixo da barra de abas, no
//! mesmo tema, e `Alt+←/→` troca de chat.

use std::borrow::Cow;
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
use super::orchestrator::{self as orchestrator_tab, OrchestratorView};
use super::term::Term;
use super::update::{self, UpdateView};
use super::usage::{SessionUsage, UsageView};
use crate::agent::Interactive;
use crate::agent::adapters::ProviderOption;
use crate::db::Db;
use crate::extension::Registry;
use crate::hub::{
    self,
    client::{Monitor, Reporter},
};
use crate::platform::keyboard;
use crate::settings::{
    AgentSettings, ExtensionSettings, Interface, OrchestratorSettings, StartMode,
};
use crate::theme;

const QUIT_LABEL: &str = " Sair ";
const TAB_GAP: &str = " ";
const HELP: &str = "Tab trocar aba · Esc ir para Sair";
const HELP_QUIT: &str = "Enter sair · Tab/Esc voltar às abas";
const HELP_PROVIDER: &str = "Enter entrar no terminal · Tab trocar aba · Esc ir para Sair";
const HELP_CHATS: &str = "Alt+N novo chat · Alt+←/→ trocar chat · Alt+W fechar chat";
const HELP_NEW_CHAT: &str = "Alt+N novo chat";
/// O título do chat do terra na barra de chats.
const MAIN_CHAT_TITLE: &str = "Principal";
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
    /// Só com uma versão nova do terra no GitHub.
    Atualizacao,
    Consumo,
    Extensoes,
    /// As configurações dos chats delegados.
    Orquestrador,
    Config,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Chat,
        Tab::Atualizacao,
        Tab::Consumo,
        Tab::Extensoes,
        Tab::Orquestrador,
        Tab::Config,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Chat => "Chat",
            Tab::Atualizacao => "Atualização",
            Tab::Consumo => "Consumo",
            Tab::Extensoes => "Extensões",
            Tab::Config => "Configurações",
            Tab::Orquestrador => "Orquestrador",
        }
    }

    /// Próxima aba, voltando para a primeira depois da última. A aba
    /// Atualização só entra com versão nova (`update`).
    fn next(self, update: bool) -> Tab {
        let next = Self::ALL[(self as usize + 1) % Self::ALL.len()];
        if !next.visible(update) {
            return next.next(update);
        }
        next
    }

    fn visible(self, update: bool) -> bool {
        match self {
            Tab::Atualizacao => update,
            _ => true,
        }
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

/// Os `config.json` do terra: o do workspace (as configurações) e o global
/// (as do orquestrador).
pub struct ConfigFiles {
    pub workspace: PathBuf,
    pub global: PathBuf,
}

/// Um chat novo, aberto com `Alt+N`, além do principal.
struct ChatTab {
    title: String,
    chat: Chat,
}

#[derive(Default)]
pub struct App {
    tab: Tab,
    focus: Focus,
    chat: Chat,
    /// Os chats além do principal, abertos com `Alt+N`.
    chats: Vec<ChatTab>,
    /// Os tokens dos chats já fechados, que continuam no consumo geral.
    closed_tokens: u64,
    /// O chat que aparece na aba Chat: 0 é o do terra, os outros são os
    /// de [`App::chats`].
    active_chat: usize,
    config_view: ConfigView,
    usage: UsageView,
    /// O consumo da sessão, para a barra de consumo do chat.
    session_usage: SessionUsage,
    extensions_view: ExtensionsView,
    /// A aba Atualização, com a versão nova do terra.
    update_view: UpdateView,
    settings: AgentSettings,
    /// As extensões conhecidas (estratégias), já detectadas.
    registry: Registry,
    /// As extensões ativas na conta do agente (e as de usar sempre).
    extensions: ExtensionSettings,
    /// O `config.json` do workspace, onde as configurações são salvas
    /// (`None` nos testes).
    config: Option<PathBuf>,
    /// As configurações do orquestrador, do `config.json` global.
    orchestrator: OrchestratorSettings,
    /// O `config.json` global (`None` nos testes).
    global_config: Option<PathBuf>,
    orchestrator_view: OrchestratorView,
    /// O provedor dos chats delegados, com o catálogo dele, quando não é o
    /// do workspace.
    delegated: Option<orchestrator_tab::Delegated>,
    /// Os provedores conhecidos (Config e Orquestrador), detectados ao abrir.
    providers: Vec<ProviderOption>,
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
    /// Este terra no daemon orquestrador (`None` nos testes ou sem o hub).
    hub: Option<Reporter>,
    /// A lista do orquestrador, assinada ao abrir a aba Orquestrador.
    hub_monitor: Option<Monitor>,
}

impl App {
    /// Tela principal com o chat ligado ao provedor disponível e as
    /// configurações do workspace `name`, em `dir`, lidas dos arquivos
    /// `files` (sem eles, o padrão do router) e salvas neles.
    pub fn new(db: Db, dir: PathBuf, name: String, files: ConfigFiles) -> Self {
        let config = files.workspace;
        let settings = AgentSettings::load_file(&config).unwrap_or_default();
        let saved = db.workspace_name(&dir).ok().flatten().is_some();
        let hub = Reporter::start(
            name,
            dir.to_string_lossy().into_owned(),
            hub::protocol::Status {
                chats: 1,
                ..Default::default()
            },
        );
        let mut app = Self {
            chat: Chat::connect(&settings),
            providers: crate::agent::adapters::options(),
            open_provider: settings.interface == Interface::Provider,
            settings,
            registry: Registry::detect(),
            config: Some(config),
            orchestrator: OrchestratorSettings::load_file(&files.global),
            global_config: Some(files.global),
            workspace: Some(Workspace { dir, saved }),
            db: Some(db),
            hub,
            ..Self::default()
        };
        app.load_extensions();
        app.update_view.check();
        app
    }

    /// O chat que aparece na aba Chat.
    fn current_chat(&self) -> &Chat {
        match self.active_chat {
            0 => &self.chat,
            i => &self.chats[i - 1].chat,
        }
    }

    fn current_chat_mut(&mut self) -> &mut Chat {
        match self.active_chat {
            0 => &mut self.chat,
            i => &mut self.chats[i - 1].chat,
        }
    }

    /// O provedor escolhido no Orquestrador é o do chat do workspace (ou
    /// não há escolha).
    fn delegated_is_own(&self) -> bool {
        self.orchestrator
            .provider
            .as_deref()
            .is_none_or(|id| self.chat.provider_id() == Some(id))
    }

    /// Carrega o catálogo do provedor dos chats delegados, quando é outro.
    fn sync_delegated(&mut self) {
        let own = self.delegated_is_own();
        orchestrator_tab::Delegated::sync(
            &mut self.delegated,
            self.orchestrator.provider.as_deref(),
            own,
        );
    }

    /// Abre um chat novo e vazio neste terra, com as configurações do
    /// workspace, e passa para ele.
    fn new_chat(&mut self) {
        let mut chat = Chat::connect(&self.settings);
        chat.set_extensions(self.chat.extensions().clone());
        chat.set_session_usage(self.session_usage.window().cloned());
        let title = format!("Chat {}", self.chats.len() + 2);
        self.chats.push(ChatTab { title, chat });
        self.active_chat = self.chats.len();
    }

    /// Fecha o chat além do principal que está na aba Chat (o do terra
    /// não fecha).
    fn close_chat(&mut self) {
        if self.active_chat > 0 {
            let tab = self.chats.remove(self.active_chat - 1);
            self.closed_tokens += tab.chat.tokens_used();
            self.active_chat -= 1;
        }
    }

    /// A conta das extensões: o provedor e a conta do agente (ex.:
    /// `claude:.claude-trabalho`; a Default fica sem nada depois do `:`).
    fn extension_account(&self) -> String {
        format!(
            "{}:{}",
            self.chat.provider_id().unwrap_or_default(),
            self.settings.account.as_deref().unwrap_or_default()
        )
    }

    /// Lê do banco as extensões da conta do agente e as aplica. Na primeira
    /// vez da conta, ela fica com as do `config.json` do workspace, de antes
    /// de as extensões irem para o banco.
    fn load_extensions(&mut self) {
        if let Some(db) = &self.db {
            let legacy = self
                .config
                .as_deref()
                .map(ExtensionSettings::load_file)
                .unwrap_or_default();
            match db.extension_settings(&self.extension_account(), &legacy) {
                Ok(extensions) => self.extensions = extensions,
                Err(err) => self.chat.warn(format!("Extensões: {err:#}")),
            }
        }
        self.apply_extensions();
    }

    /// Grava no banco as extensões que mudaram de modo desde `previous`.
    fn save_extensions(&mut self, previous: &ExtensionSettings) {
        let Some(db) = &self.db else {
            return;
        };
        let account = self.extension_account();
        let mut ids = previous.active();
        ids.extend(self.extensions.active());
        for id in ids {
            let mode = self.extensions.mode(&id);
            if mode != previous.mode(&id)
                && let Err(err) = db.set_extension(&account, &id, mode)
            {
                // Sem conseguir salvar, a mudança vale só nesta execução.
                self.chat.warn(format!("Extensões: {err:#}"));
                return;
            }
        }
    }

    /// Passa aos chats o que as extensões ativas acrescentam ao agente (vale
    /// a partir da próxima mensagem); as que falharem viram avisos.
    fn apply_extensions(&mut self) {
        let (contribution, warnings) = self.registry.contribution(&self.extensions.active());
        for tab in &mut self.chats {
            tab.chat.set_extensions(contribution.clone());
        }
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

    /// Opções da Config: contas do provedor, modelos do catálogo do chat e
    /// as extensões do registro.
    fn choices(&self) -> config::Choices {
        config::Choices {
            providers: self.providers.clone(),
            provider: self.chat.provider_id().map(str::to_string),
            accounts: self
                .chat
                .adapter()
                .map(|a| a.accounts())
                .unwrap_or_default(),
            models: self.chat.model_options(),
            catalog: self.chat.catalog().cloned(),
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
            skills: self.skills(),
        }
    }

    /// As skills instaladas no agente, para a seção Skills da Config: as das
    /// pastas do provedor e, depois, as que só o catálogo do agente informa
    /// (ex.: de uma pasta que o terra não conhece).
    fn skills(&self) -> Vec<config::SkillInfo> {
        let Some(adapter) = self.chat.adapter() else {
            return Vec::new();
        };
        let mut skills: Vec<config::SkillInfo> = std::env::current_dir()
            .map(|cwd| adapter.skills(&cwd, self.settings.account.as_deref()))
            .unwrap_or_default()
            .into_iter()
            .map(|skill| config::SkillInfo {
                name: skill.name,
                origin: format!("{} · {}", adapter.name(), skill.scope),
                description: skill.description,
            })
            .collect();
        let commands = self.chat.catalog().map_or(&[][..], |c| &c.commands[..]);
        let mut builtin: Vec<config::SkillInfo> = commands
            .iter()
            .filter(|c| c.skill && !skills.iter().any(|s| s.name == c.name))
            .map(|c| config::SkillInfo {
                name: c.name.clone(),
                origin: format!("{} · catálogo", adapter.name()),
                description: c.description.clone(),
            })
            .collect();
        builtin.sort_by(|a, b| a.name.cmp(&b.name));
        skills.extend(builtin);
        skills
    }

    /// Salva as configurações e as aplica ao chat.
    fn save_settings(&mut self, previous: &AgentSettings) {
        let provider = self.chat.provider_id().map(str::to_string);
        if let Some(config) = &self.config {
            // Sem conseguir salvar, a mudança vale só nesta execução.
            let _ = self.settings.save_file(config);
        }
        self.chat.apply_settings(&self.settings, Some(previous));
        for tab in &mut self.chats {
            tab.chat.apply_settings(&self.settings, Some(previous));
        }
        // Outra conta (ou provedor): as extensões passam a ser as dela.
        if self.settings.account != previous.account
            || self.chat.provider_id() != provider.as_deref()
        {
            self.load_extensions();
        }
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
        if self.tab == Tab::Orquestrador && self.focus == Focus::Tabs {
            self.sync_delegated();
            let chat = &self.chat;
            let delegated = self.delegated.as_ref();
            let settings = self.orchestrator.clone();
            let label = |id: &str| delegated_label(chat, delegated, &settings, id);
            let agent = delegated_agent(chat, &settings, &self.providers, delegated, &label);
            match self
                .orchestrator_view
                .handle_key(key, &mut self.orchestrator, &agent)
            {
                orchestrator_tab::Outcome::Ignored => {}
                orchestrator_tab::Outcome::Moved => return false,
                orchestrator_tab::Outcome::Changed => {
                    if let Some(global) = &self.global_config {
                        // Sem conseguir salvar, a mudança vale só nesta execução.
                        let _ = self.orchestrator.save_file(global);
                    }
                    return false;
                }
            }
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

    /// Mouse: no input da aba Chat (posicionar o cursor e selecionar) e nos
    /// botões do gráfico da aba Consumo.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
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

    /// Avança as animações; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        for reason in crate::agent::guard::take_panics() {
            self.chat.provider_failed(&reason);
        }
        self.chat
            .set_caps_lock(keyboard::caps_lock().unwrap_or(false));
        self.chat.tick();
        for tab in &mut self.chats {
            tab.chat.tick();
        }
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
            let window = self.session_usage.window().cloned();
            self.chat.set_session_usage(window.clone());
            for tab in &mut self.chats {
                tab.chat.set_session_usage(window.clone());
            }
        }
        // O relatório das extensões, também só com a aba aberta.
        if self.tab == Tab::Extensoes {
            self.extensions_view.refresh_if_stale(&self.registry);
        }
        self.extensions_view.tick();
        self.update_view.tick();
        self.report_to_hub();
        // No Orquestrador, a aba carrega o catálogo do provedor escolhido e
        // assina a lista do hub (só com o hub ligado: não nos testes).
        if self.tab == Tab::Orquestrador {
            if self.hub.is_some() && self.hub_monitor.is_none() {
                self.hub_monitor = Monitor::start();
            }
            self.sync_delegated();
            let own = self.delegated_is_own();
            self.orchestrator_view.sync(match &self.delegated {
                Some(d) if !own => d.catalog(),
                _ => self.chat.catalog(),
            });
        }
    }

    /// O que este terra está fazendo, para o hub: algum chat rodando, o que
    /// o agente faz e os chats abertos (os além do principal são os sub
    /// agentes).
    fn hub_status(&self) -> hub::protocol::Status {
        let chats = std::iter::once(&self.chat).chain(self.chats.iter().map(|c| &c.chat));
        let task = chats.clone().find_map(Chat::activity);
        hub::protocol::Status {
            running: task.is_some(),
            task: task.map(str::to_string),
            chats: self.chats.len() + 1,
            agents: self.chats.len(),
        }
    }

    /// Manda o estado ao hub; o [`Reporter`] só manda quando mudou e nunca
    /// espera.
    fn report_to_hub(&mut self) {
        let status = self.hub_status();
        if let Some(hub) = &mut self.hub {
            hub.set_status(status);
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(theme::screen()), area);
        let [nav, content, footer] = layout(area);

        self.draw_nav(frame, nav);
        self.draw_content(frame, content);
        let help: Cow<'static, str> = match self.focus {
            Focus::Tabs if self.captures_keys() => HELP_TERM.into(),
            Focus::Tabs if self.tab == Tab::Chat && self.uses_provider() => HELP_PROVIDER.into(),
            Focus::Tabs if self.tab == Tab::Chat && !self.chats.is_empty() => {
                format!("{} · {HELP_CHATS}", self.current_chat().help()).into()
            }
            Focus::Tabs if self.tab == Tab::Chat => {
                format!("{} · {HELP_NEW_CHAT}", self.current_chat().help()).into()
            }
            Focus::Tabs if self.tab == Tab::Config => self.config_view.help().into(),
            Focus::Tabs if self.tab == Tab::Extensoes => extensions::HELP.into(),
            Focus::Tabs if self.tab == Tab::Orquestrador => self.orchestrator_view.help().into(),
            Focus::Tabs if self.tab == Tab::Consumo => self.usage.help().into(),
            Focus::Tabs if self.tab == Tab::Atualizacao => update::HELP.into(),
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

        let total = self.total_label();
        let [tabs_area, total_area, quit_area] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(total.as_ref().map_or(0, |t| t.chars().count() as u16)),
            Constraint::Length(QUIT_LABEL.len() as u16),
        ])
        .areas(inner);
        if let Some(total) = total {
            frame.render_widget(
                Paragraph::new(Span::styled(total, theme::muted())),
                total_area,
            );
        }

        let mut spans = Vec::new();
        let tabs = Tab::ALL
            .into_iter()
            .filter(|tab| tab.visible(self.update_view.available()));
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

    /// O consumo geral, ao lado do "Sair": os tokens de todos os chats deste
    /// terra (entrada e saída, sem os de cache, como no consumo do chat).
    fn total_label(&self) -> Option<String> {
        let tokens = self.closed_tokens
            + self.chat.tokens_used()
            + self.chats.iter().map(|c| c.chat.tokens_used()).sum::<u64>();
        (self.settings.usage.total_visible && tokens > 0).then(|| {
            format!(
                "Consumo geral: {} Tokens  ",
                super::chat::compact_count(tokens)
            )
        })
    }

    /// A barra de chats, embaixo da barra de abas e no mesmo tema: o chat
    /// do terra e os abertos com `Alt+N`.
    fn draw_chats(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::border())
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let titles =
            std::iter::once(MAIN_CHAT_TITLE).chain(self.chats.iter().map(|c| c.title.as_str()));
        let mut spans = Vec::new();
        for (i, title) in titles.enumerate() {
            if i > 0 {
                spans.push(Span::raw(TAB_GAP));
            }
            let style = if i == self.active_chat && self.focus == Focus::Tabs {
                theme::tab_active()
            } else {
                theme::tab_inactive()
            };
            spans.push(Span::styled(format!(" {title} "), style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
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
            let bar = if self.chats.is_empty() { 0 } else { 3 };
            let chat = self.current_chat();
            let [chats, history, panel] = Layout::vertical([
                Constraint::Length(bar),
                Constraint::Fill(1),
                Constraint::Length(chat.height()),
            ])
            .areas(area);
            if bar > 0 {
                self.draw_chats(frame, chats);
            }
            chat.draw_history(frame, history);
            chat.draw(frame, panel);
            return;
        }
        if self.tab == Tab::Config {
            self.config_view.draw(
                frame,
                area,
                &self.settings,
                &self.extensions,
                &self.choices(),
            );
            return;
        }
        if self.tab == Tab::Orquestrador {
            let delegated = self.delegated.as_ref();
            let label = |id: &str| delegated_label(&self.chat, delegated, &self.orchestrator, id);
            let agent = delegated_agent(
                &self.chat,
                &self.orchestrator,
                &self.providers,
                delegated,
                &label,
            );
            let instances = self.hub_monitor.as_ref().map(Monitor::instances);
            let hub = orchestrator_tab::HubView {
                instances: instances.as_ref(),
                own: self.hub.as_ref().map(Reporter::id),
            };
            self.orchestrator_view
                .draw(frame, area, &self.orchestrator, &agent, &hub);
            return;
        }
        if self.tab == Tab::Atualizacao {
            self.update_view.draw(frame, area);
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
                "  Para usar o chat do terra, troque a Interface na aba Configurações.",
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

/// O agente dos chats delegados, para a aba Orquestrador: o provedor e o
/// catálogo do chat do terra ou, com outro provedor escolhido para os
/// delegados, o catálogo dele (`None` até chegar).
fn delegated_agent<'a>(
    chat: &'a Chat,
    settings: &OrchestratorSettings,
    providers: &'a [ProviderOption],
    delegated: Option<&'a orchestrator_tab::Delegated>,
    label: &'a dyn Fn(&str) -> String,
) -> orchestrator_tab::Agent<'a> {
    let own = settings
        .provider
        .as_deref()
        .is_none_or(|id| chat.provider_id() == Some(id));
    let catalog = if own {
        chat.catalog()
    } else {
        delegated.and_then(|d| d.catalog())
    };
    orchestrator_tab::Agent {
        provider: chat.adapter().map_or("Sem agente", |a| a.name()),
        providers,
        catalog,
        default_model: if own {
            chat.default_model()
        } else {
            catalog.and_then(|c| c.default_model.clone())
        },
        label,
    }
}

/// O nome de um modelo dos chats delegados para a tela.
fn delegated_label(
    chat: &Chat,
    delegated: Option<&orchestrator_tab::Delegated>,
    settings: &OrchestratorSettings,
    id: &str,
) -> String {
    let own = settings
        .provider
        .as_deref()
        .is_none_or(|p| chat.provider_id() == Some(p));
    match delegated {
        Some(d) if !own => d.label(id),
        _ => chat.model_label_of(id),
    }
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
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Orquestrador);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Chat);
    }

    #[test]
    fn aba_atualizacao_so_com_versao_nova_e_enter_pede_instalar() {
        let mut app = App::default();
        app.update_view.set_update(Some(crate::upgrade::Update {
            version: "9.9.9".into(),
            notes: "## [9.9.9] - 2099-01-01\n\n- novo".into(),
        }));
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Atualizacao);
        assert!(!app.take_update_install());
        press(&mut app, KeyCode::Enter);
        assert!(app.take_update_install());
        assert!(!app.take_update_install());
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Consumo);
    }

    fn key(app: &mut App, code: KeyCode) -> bool {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn orquestrador_tab_is_always_between_extensions_and_config() {
        assert_eq!(Tab::Extensoes.next(false), Tab::Orquestrador);
        assert_eq!(Tab::Orquestrador.next(false), Tab::Config);
        assert_eq!(Tab::Config.next(false), Tab::Chat);
        assert_eq!(Tab::Chat.next(false), Tab::Consumo);
        assert_eq!(Tab::Chat.next(true), Tab::Atualizacao);
        assert_eq!(Tab::Atualizacao.next(true), Tab::Consumo);
    }

    #[test]
    fn changing_the_orquestrador_saves_it_in_the_global_file() {
        let dir = tempfile::tempdir().unwrap();
        let global = crate::settings::global_file(dir.path());
        // A antiga chave `mcp` ainda está no arquivo de outro terra.
        std::fs::write(
            &global,
            r#"{"outra": 1, "mcp": {"enabled": true, "model": "opus"}}"#,
        )
        .unwrap();
        let mut app = App {
            global_config: Some(global.clone()),
            tab: Tab::Orquestrador,
            orchestrator: OrchestratorSettings::load_file(&global),
            ..App::default()
        };
        // O Provedor é a primeira configuração, e ←/→ muda e salva.
        assert!(!key(&mut app, KeyCode::Right));
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&global).unwrap()).unwrap();
        // A antiga chave `mcp` saiu; os modos do orquestrador ficaram.
        assert!(saved.get("orchestrator").is_some());
        assert!(saved.get("mcp").is_none(), "{}", saved);
        assert_eq!(
            OrchestratorSettings::load_file(&global).model.as_deref(),
            Some("opus")
        );
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
        press(&mut app, KeyCode::Tab);
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Config);
        // Provedor e Conta são as primeiras; ↓↓ vai para o Modo inicial.
        assert!(!key(&mut app, KeyCode::Down));
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
        key(&mut app, KeyCode::Down); // Conta
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
                    env: Vec::new(),
                    launchers: Vec::new(),
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let config = crate::settings::config_file(dir.path(), "terra");
        let mut app = App {
            config: Some(config.clone()),
            db: Some(Db::open_in_memory().unwrap()),
            registry: Registry::new(vec![Box::new(Fake)]),
            tab: Tab::Config,
            ..App::default()
        };
        let saved = |app: &App| {
            let db = app.db.as_ref().unwrap();
            db.extension_settings(&app.extension_account(), &[])
                .unwrap()
        };
        assert!(!key(&mut app, KeyCode::BackTab));
        assert!(!key(&mut app, KeyCode::Char(' ')));
        assert_eq!(saved(&app).enabled, ["fake"]);
        assert_eq!(app.chat.extensions().hooks[0].command, "fake");
        // O agente continua igual.
        assert_eq!(
            AgentSettings::load_file(&config).unwrap_or_default(),
            AgentSettings::default()
        );

        // Na outra conta, só fica a de usar sempre.
        let previous = app.settings.clone();
        app.settings.account = Some(".claude-trabalho".into());
        app.save_settings(&previous);
        assert!(app.extensions.active().is_empty());
        assert_eq!(app.chat.extensions(), &Contribution::default());
        let previous = app.settings.clone();
        app.settings.account = None;
        app.save_settings(&previous);
        assert_eq!(app.extensions.enabled, ["fake"]);

        key(&mut app, KeyCode::Char(' '));
        assert_eq!(saved(&app).always, ["fake"]);
        let previous = app.settings.clone();
        app.settings.account = Some(".claude-trabalho".into());
        app.save_settings(&previous);
        assert_eq!(app.chat.extensions().hooks[0].command, "fake");
        let previous = app.settings.clone();
        app.settings.account = None;
        app.save_settings(&previous);

        key(&mut app, KeyCode::Char(' '));
        assert_eq!(saved(&app), ExtensionSettings::default());
        assert_eq!(app.chat.extensions(), &Contribution::default());

        // A aba Extensões fica entre Consumo e o Orquestrador.
        app.tab = Tab::Consumo;
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Extensoes);
        assert!(!key(&mut app, KeyCode::Down));
        assert_eq!(press(&mut app, KeyCode::Tab), Tab::Orquestrador);
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

        let mut terminal = Terminal::new(TestBackend::new(80, 8)).unwrap();
        let mut app = App::default();
        press(&mut app, KeyCode::Tab);
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), format!("╭{}╮", "─".repeat(78)));
        assert_eq!(
            row(1),
            "│  Chat   Consumo   Extensões   Orquestrador   Configurações             Sair  │"
        );
        assert_eq!(row(2), format!("╰{}╯", "─".repeat(78)));

        // O pill " Consumo " (x = 9..18) usa o estilo da aba ativa; "Chat" não.
        let active_bg = theme::tab_active().bg;
        assert!((9..18).all(|x| buffer[(x, 1)].bg == active_bg.unwrap()));
        assert_ne!(buffer[(3, 1)].bg, active_bg.unwrap());

        // "Sair" (x = 72..78) é vermelho, sem fundo até receber foco.
        let red = theme::quit().fg.unwrap();
        assert!((72..78).all(|x| buffer[(x, 1)].fg == red));
    }

    #[test]
    fn new_chats_show_up_below_the_nav_bar() {
        use crossterm::event::KeyModifiers;
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = App::default();
        app.new_chat();
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(4).starts_with("│  Principal   Chat 2 "), "{}", row(4));
        // Com Alt+N, o chat novo fica ativo; o Principal não.
        assert_eq!(buffer[(17, 4)].bg, theme::tab_active().bg.unwrap());
        assert_ne!(buffer[(3, 4)].bg, theme::tab_active().bg.unwrap());

        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        // Alt+N deixa o chat novo ativo; Alt+→ dá a volta ao Principal.
        app.handle_key(alt(KeyCode::Right));
        assert_eq!(app.active_chat, 0);
        app.handle_key(alt(KeyCode::Left));
        assert_eq!(app.active_chat, 1);
        app.handle_key(alt(KeyCode::Char('w')));
        assert_eq!(app.chats.len(), 0, "o chat novo fecha");
        assert_eq!(app.active_chat, 0);
        app.handle_key(alt(KeyCode::Char('w')));
        assert_eq!(app.chats.len(), 0, "o chat do terra não fecha");
    }

    #[test]
    fn alt_n_opens_a_new_chat_in_the_same_terra() {
        use crossterm::event::KeyModifiers;

        let mut app = App::default();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
        assert_eq!(app.chats.len(), 1);
        assert_eq!(app.active_chat, 1);
        assert_eq!(app.chats[0].title, "Chat 2");
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
        assert_eq!(app.active_chat, 2);
        assert_eq!(app.chats[1].title, "Chat 3");
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
