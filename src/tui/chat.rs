//! Aba Chat: histórico da conversa e, embaixo, o painel de entrada e status
//! do agente. `Enter` envia a mensagem, e o status, a atividade, o modelo e o
//! consumo vêm da resposta.
//!
//! O chat fala com o agente só pelo contrato [`ChatAdapter`]: não sabe qual
//! provedor está por trás (hoje, o Claude CLI). Este módulo liga as partes:
//! [`input`] (caixa de texto), [`history`] (conversa) e [`mode`] (como status
//! e modo aparecem).

mod commands;
mod history;
mod input;
mod mention;
mod mode;
mod model_menu;
mod permission;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Gauge, Paragraph};

use self::history::{Entry, History};
use self::input::{InputBox, InputMode, Outcome};
use self::mode::{AgentStatus, mode_variant};
use self::model_menu::{ModelMenu, Target, effort_label};
use self::permission::Prompts;
use super::usage;
use super::widgets::badge;
use crate::agent::{
    self, AgentEvent, Catalog, ChatAdapter, CommandRoute, Interactive, PermissionMode,
    ReportedMode, Request, Session, UsageWindow,
};
use crate::extension::Contribution;
use crate::mcp;
use crate::platform::clipboard;
use crate::settings::{AgentSettings, ContextSettings, StartMode, UsageSettings};
use crate::theme::{self, Variant};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Altura do painel: atividade, caixa de input e badges.
pub const HEIGHT: u16 = 1 + input::HEIGHT + badge::HEIGHT;

/// Linhas roladas por PageUp/PageDown e pela roda do mouse.
const PAGE_LINES: usize = 10;
const WHEEL_LINES: usize = 3;
/// Ticks que o aviso "Copiado" fica no rodapé (~2 s).
const NOTICE_TICKS: u32 = 20;
/// Limite de contexto enquanto o agente não informa o do modelo.
const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
/// Largura da barra de contexto, com as bordas.
const CONTEXT_WIDTH: u16 = 26;
/// Texto da barra de contexto quando ele chega no ponto de compactar.
/// Largura da barra de consumo, ao lado da de contexto.
const USAGE_WIDTH: u16 = 24;
const COMPACT_HINT: &str = "Compacte a conversa";

const HELP_NORMAL: &str =
    "i escrever · Shift+Tab modo · PgUp/PgDn rolar · Tab trocar aba · Esc ir para Sair";
const HELP_RUNNING: &str = "Esc interromper · i escrever · Shift+Tab modo · PgUp/PgDn rolar";
const HELP_INSERT: &str = "Enter enviar · Esc parar · \\+Enter nova linha · @ arquivo · / comando";
const HELP_PICKER: &str = "↑/↓ escolher · Tab/Enter inserir · Esc fechar · digite para filtrar";
const HELP_MODEL: &str = "↑/↓ escolher · ←/→ esforço · Enter selecionar · Esc voltar/fechar";
const HELP_PERMISSION: &str = "←/→ escolher · Enter confirmar · s permitir · n/Esc negar";
const HELP_COMMANDS: &str = "↑/↓ escolher · Tab/Enter executar · Esc fechar · digite para filtrar";

pub struct Chat {
    /// Provedor do agente (route); o nome vira um botão no título.
    adapter: Option<Box<dyn ChatAdapter>>,
    /// Modelo informado pelo agente (ex.: `claude-opus-5-5`).
    model: Option<String>,
    /// Modelo escolhido na Config, enviado nas próximas mensagens.
    chosen_model: Option<String>,
    /// Esforço padrão (escolhido no Default do `/model`).
    default_effort: Option<String>,
    /// Modelo e esforço desta conversa, escolhidos no `/model`; valem no
    /// lugar dos padrões.
    session_model: Option<String>,
    session_effort: Option<String>,
    /// Menu do `/model`, quando aberto.
    model_menu: Option<ModelMenu>,
    /// Comando do `/login`, para a tela principal rodar fora da tela.
    pending_login: Option<Command>,
    /// A interface original do provedor com um comando que só existe nela
    /// (ex.: `/hooks`), para a tela principal rodar fora da tela.
    pending_original: Option<(Command, String)>,
    /// O `/usage` pediu a aba Consumo, para a tela principal abrir.
    open_usage: bool,
    /// Novo modelo padrão escolhido no `/model`, para a tela principal salvar.
    default_choice: Option<(String, Option<String>)>,
    /// Conta escolhida na Config (`None` é a Default do provedor).
    account: Option<String>,
    /// O que as extensões ativas no workspace acrescentam ao agente.
    extensions: Contribution,
    /// Sessão do agente, para continuar a conversa.
    session_id: Option<String>,
    /// A delegação ([`mcp::ChatRequest::id`]) de um chat aberto por um
    /// agente pelo orquestrador.
    delegation: Option<u64>,
    /// Resposta em andamento.
    run: Option<Box<dyn Session>>,
    /// Pedidos de permissão do agente esperando o usuário.
    prompts: Prompts,
    /// Modo pedido ao agente na resposta em andamento.
    requested_mode: Option<PermissionMode>,
    /// Modo em que o agente disse estar (no `init`) na última resposta.
    agent_mode: Option<PermissionMode>,
    history: History,
    input: InputBox,
    /// Aviso temporário no rodapé (ex.: "Copiado") e ticks restantes.
    notice: Option<(String, u32)>,
    agent_status: AgentStatus,
    /// O que o agente está fazendo; só ele preenche (vazio sem agente).
    agent_activity: Option<String>,
    permission_mode: PermissionMode,
    spinner_frame: usize,
    tokens_used: u64,
    /// Os tokens de cache gastos neste chat.
    cache_tokens_used: u64,
    /// Tamanho do contexto da sessão, depois da primeira chamada ao modelo.
    context: Option<u64>,
    /// Limite de contexto do modelo, quando o agente informa.
    context_window: Option<u64>,
    /// Display da barra, auto compactação e ponto de compactar (Config).
    context_settings: ContextSettings,
    /// Display da barra de consumo (Config).
    usage_settings: UsageSettings,
    /// O consumo da sessão (as 5 horas), quando o provedor informa.
    session_usage: Option<UsageWindow>,
    /// A resposta em andamento é a de uma mensagem do usuário que ainda pode
    /// ser auto compactada: a auto compactação roda no máximo uma vez por
    /// mensagem enviada (nunca no fim de um `/compact`, mesmo se ele falhou).
    auto_compact_armed: bool,
    /// Comandos e modelos do agente, depois de carregados.
    catalog: Option<Catalog>,
    /// Carregamento do catálogo em andamento.
    catalog_loading: Option<Receiver<Result<Catalog, String>>>,
}

/// Sem agente (Desconectado). Use [`Chat::connect`] para usar o provedor
/// disponível.
impl Default for Chat {
    fn default() -> Self {
        Self {
            adapter: None,
            model: None,
            chosen_model: None,
            default_effort: None,
            session_model: None,
            session_effort: None,
            model_menu: None,
            pending_login: None,
            pending_original: None,
            open_usage: false,
            default_choice: None,
            account: None,
            extensions: Contribution::default(),
            session_id: None,
            delegation: None,
            run: None,
            prompts: Prompts::default(),
            requested_mode: None,
            agent_mode: None,
            history: History::default(),
            input: InputBox::default(),
            notice: None,
            agent_status: AgentStatus::default(),
            agent_activity: None,
            permission_mode: PermissionMode::default(),
            spinner_frame: 0,
            tokens_used: 0,
            cache_tokens_used: 0,
            context: None,
            context_window: None,
            context_settings: ContextSettings::default(),
            usage_settings: UsageSettings::default(),
            session_usage: None,
            auto_compact_armed: false,
            catalog: None,
            catalog_loading: None,
        }
    }
}

impl Chat {
    /// Chat ligado ao primeiro provedor disponível (ou Desconectado).
    pub fn connect(settings: &AgentSettings) -> Self {
        Self::with_adapter(agent::adapters::default(), settings)
    }

    /// Chat ligado a `adapter`, com as configurações da seção Agente.
    pub fn with_adapter(adapter: Box<dyn ChatAdapter>, settings: &AgentSettings) -> Self {
        let agent_status = if adapter.is_available() {
            AgentStatus::Ocioso
        } else {
            AgentStatus::Desconectado
        };
        let mut chat = Self {
            adapter: Some(adapter),
            agent_status,
            ..Self::default()
        };
        chat.apply_settings(settings, None);
        chat.load_catalog();
        chat
    }

    /// Aplica as configurações da seção Agente. O modo só muda quando o
    /// modo inicial mudou (ou no início, sem `previous`), para não desfazer
    /// um Shift+Tab; o modelo vale a partir da próxima mensagem.
    ///
    /// Trocar de conta começa uma conversa nova: a sessão anterior é da outra
    /// conta e não pode ser continuada com esta. O aviso só aparece quando
    /// havia uma conversa, para passar pelas contas não encher o histórico.
    pub fn apply_settings(&mut self, settings: &AgentSettings, previous: Option<&AgentSettings>) {
        let account_changed = previous.is_some_and(|p| p.account != settings.account);
        self.account = settings.account.clone();
        if account_changed {
            // Cada conta tem os seus comandos e modelos.
            self.load_catalog();
        }
        if account_changed && self.session_id.is_some() {
            self.session_id = None;
            self.agent_mode = None;
            self.model = None;
            self.context = None;
            let label = self.account_label();
            self.history.push(Entry::Info(format!(
                "Conta: {label}. A próxima mensagem começa uma conversa nova."
            )));
        }
        let start_changed = previous.is_none_or(|p| p.start_mode != settings.start_mode);
        // "Do provedor" depende da conta: a configuração é de cada uma.
        let provider_mode = settings.start_mode == StartMode::Provider && account_changed;
        if start_changed || provider_mode {
            self.permission_mode = self.start_mode(settings.start_mode);
        }
        self.chosen_model = settings.model.clone();
        self.default_effort = settings.effort.clone();
        self.context_settings = settings.context;
        self.usage_settings = settings.usage;
    }

    /// O consumo da sessão (as 5 horas) para a barra de consumo.
    pub fn set_session_usage(&mut self, window: Option<UsageWindow>) {
        self.session_usage = window;
    }

    /// Nome da conta escolhida, como o provedor a mostra.
    pub fn account_label(&self) -> String {
        let accounts = self
            .adapter
            .as_ref()
            .map(|a| a.accounts())
            .unwrap_or_default();
        accounts
            .into_iter()
            .find(|a| a.id == self.account)
            .map_or_else(
                || {
                    self.account
                        .clone()
                        .unwrap_or_else(|| "Default".to_string())
                },
                |a| a.label,
            )
    }

    /// Começa no modo `start` (ex.: o dos chats abertos pelo orquestrador),
    /// no lugar do modo inicial da Config.
    pub fn set_start_mode(&mut self, start: StartMode) {
        self.permission_mode = self.start_mode(start);
    }

    /// Marca o chat como delegado (`id` é a delegação no orquestrador): os
    /// pedidos de permissão do agente vão para quem delegou.
    pub fn set_delegation(&mut self, id: u64) {
        self.delegation = Some(id);
    }

    pub fn delegation(&self) -> Option<u64> {
        self.delegation
    }

    /// A última resposta do agente, para voltar a quem delegou.
    pub fn last_reply(&self) -> Option<String> {
        self.history
            .entries()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::Assistant(text) => Some(text.clone()),
                _ => None,
            })
    }

    /// O modo de um modo inicial. "Do provedor" segue a configuração dele,
    /// para os dois estarem em sincronia desde o início; o Yolo nunca vem do
    /// provedor, então lá o padrão é Agente.
    fn start_mode(&self, start: StartMode) -> PermissionMode {
        match start {
            StartMode::Fixed(mode) => mode,
            StartMode::Provider => std::env::current_dir()
                .ok()
                .zip(self.adapter.as_deref())
                .and_then(|(cwd, adapter)| adapter.configured_mode(&cwd, self.account.as_deref()))
                .filter(|mode| *mode != PermissionMode::Yolo)
                .unwrap_or_default(),
        }
    }

    /// O provedor do chat, para a Config mostrar e oferecer as opções dele.
    /// As extensões ativas mudaram: valem a partir da próxima mensagem.
    pub fn set_extensions(&mut self, extensions: Contribution) {
        self.extensions = extensions;
    }

    /// O que as extensões ativas acrescentam ao agente.
    /// Há uma conversa com o agente (o MCP mostra a sessão ativa).
    /// Os tokens gastos neste chat (entrada e saída, sem os de cache).
    pub fn tokens_used(&self) -> u64 {
        self.tokens_used
    }

    /// Os tokens de cache gastos neste chat (criação e leitura).
    pub fn cache_tokens_used(&self) -> u64 {
        self.cache_tokens_used
    }

    /// O agente está respondendo.
    pub fn is_busy(&self) -> bool {
        self.run.is_some()
    }

    pub fn has_session(&self) -> bool {
        self.session_id.is_some()
    }

    pub fn extensions(&self) -> &Contribution {
        &self.extensions
    }

    pub fn adapter(&self) -> Option<&dyn ChatAdapter> {
        self.adapter.as_deref()
    }

    /// O provedor, se estiver pronto para receber mensagens.
    fn available_adapter(&self) -> Option<&dyn ChatAdapter> {
        self.adapter.as_deref().filter(|a| a.is_available())
    }

    fn unavailable_hint(&self) -> String {
        self.adapter
            .as_ref()
            .map_or("Nenhum agente configurado.".to_string(), |a| {
                a.unavailable_hint()
            })
    }

    /// Trata uma tecla. Retorna `true` se ela foi consumida pelo painel; as
    /// outras (Tab, Esc no modo NORMAL...) ficam para a tela principal.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // O menu do `/model`, aberto, fica com todas as teclas.
        if let Some(menu) = &mut self.model_menu {
            let outcome = menu.handle_key(key, self.catalog.as_ref());
            self.apply_model_menu(outcome);
            return true;
        }
        // Um pedido de permissão aberto fica com as teclas, menos o Tab, que
        // continua trocando de aba.
        if self.prompts.is_open() {
            if key.code == KeyCode::Tab {
                return false;
            }
            if let Some((request, allow)) = self.prompts.handle_key(key) {
                self.answer(request, allow);
            }
            return true;
        }
        match key.code {
            KeyCode::BackTab => {
                self.permission_mode = self.permission_mode.next();
                return true;
            }
            KeyCode::PageUp => {
                self.history.scroll_up(PAGE_LINES);
                return true;
            }
            KeyCode::PageDown => {
                self.history.scroll_down(PAGE_LINES);
                return true;
            }
            // No modo NORMAL, com o agente respondendo, o Esc interrompe (em
            // vez de ir para "Sair").
            KeyCode::Esc if self.input.mode() == InputMode::Normal && self.run.is_some() => {
                self.cancel();
                return true;
            }
            _ => {}
        }
        match self.input.handle_key(key) {
            Outcome::Ignored => false,
            Outcome::Handled => true,
            Outcome::Submit => {
                self.send();
                true
            }
            Outcome::Command(name) => {
                self.run_command(&name);
                true
            }
        }
    }

    /// Começa a carregar os comandos e modelos do agente (da conta atual).
    /// Sem provedor disponível, não há comandos.
    fn load_catalog(&mut self) {
        self.catalog = None;
        self.catalog_loading = self
            .available_adapter()
            .and_then(|adapter| adapter.load_catalog(self.account.as_deref()));
        let loading = self.catalog_loading.is_some();
        self.input.set_commands((!loading).then(Vec::new));
    }

    /// Recebe o catálogo, quando ele chega.
    fn poll_catalog(&mut self) {
        let Some(loading) = &self.catalog_loading else {
            return;
        };
        let catalog = match loading.try_recv() {
            Err(TryRecvError::Empty) => return,
            Ok(Ok(catalog)) => catalog,
            // Sem o catálogo, a lista fica vazia (o chat continua funcionando).
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => Catalog::default(),
        };
        self.catalog_loading = None;
        self.input
            .set_commands(Some(commands::with_terra(catalog.commands.clone())));
        self.catalog = Some(catalog);
        if let Some(menu) = &mut self.model_menu {
            menu.sync(self.catalog.as_ref());
        }
    }

    /// Executa um comando da `/`. A função de cada um é uma interface do
    /// terra (veja [`commands`]); os que ainda não têm avisam no histórico.
    fn run_command(&mut self, name: &str) {
        match name {
            "clear" => self.clear_conversation(),
            "compact" => self.compact(),
            "login" => self.start_login(),
            // O consumo é a aba Consumo do terra.
            "usage" => self.open_usage = true,
            "model" => {
                // O Default do menu é o modelo padrão salvo ou o do agente.
                let default = self
                    .chosen_model
                    .clone()
                    .or_else(|| self.catalog.as_ref().and_then(|c| c.default_model.clone()));
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
    fn route_to_provider(&mut self, command: &str) -> bool {
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
            None => self.history.push(Entry::Info(
                "Este provedor não tem login pelo terra.".to_string(),
            )),
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

    /// Um erro de autenticação do agente: sugere o `/login`.
    fn hint_login(&mut self, message: &str) {
        let lower = message.to_lowercase();
        let auth = ["authenticate", "oauth", "log in again"]
            .iter()
            .any(|word| lower.contains(word));
        if auth {
            let label = self.account_label();
            self.history.push(Entry::Info(format!(
                "Use /login para entrar de novo na conta {label}."
            )));
        }
    }

    /// Aplica a escolha do menu do `/model`.
    fn apply_model_menu(&mut self, outcome: model_menu::Outcome) {
        let (target, model, effort) = match outcome {
            model_menu::Outcome::Handled => return,
            model_menu::Outcome::Close => {
                self.model_menu = None;
                return;
            }
            model_menu::Outcome::Chosen {
                target,
                model,
                effort,
            } => (target, model, effort),
        };
        self.model_menu = None;
        let label = self.model_label_of(&model);
        let effort_text = effort
            .as_deref()
            .map_or(String::new(), |e| format!(" · esforço {}", effort_label(e)));
        match target {
            Target::Session => {
                self.history.push(Entry::Info(format!(
                    "Modelo desta conversa: {label}{effort_text}."
                )));
                // O título já mostra o escolhido; o agente confirma na resposta.
                self.model = self.resolved_model(&model).or(Some(model.clone()));
                self.session_model = Some(model);
                self.session_effort = effort;
            }
            Target::Default => {
                self.history.push(Entry::Info(format!(
                    "Modelo default: {label}{effort_text} (salvo nas Configurações)."
                )));
                // A conversa passa a usar o novo padrão.
                self.session_model = None;
                self.session_effort = None;
                self.model = self.resolved_model(&model);
                self.default_choice = Some((model, effort));
            }
        }
        self.history.follow();
    }

    /// O novo modelo padrão escolhido no `/model`, uma vez, para a tela
    /// principal salvar nas configurações.
    pub fn take_default_choice(&mut self) -> Option<(String, Option<String>)> {
        self.default_choice.take()
    }

    #[cfg(test)]
    pub fn default_choice_for_test(&mut self, choice: (String, Option<String>)) {
        self.default_choice = Some(choice);
    }

    /// O nome real de um modelo do catálogo (ex.: `opus` → `claude-opus-5-5`).
    fn resolved_model(&self, id: &str) -> Option<String> {
        self.catalog
            .as_ref()?
            .models
            .iter()
            .find(|m| m.id == id)?
            .resolved
            .clone()
    }

    /// Nome de um modelo para a tela (ex.: `Opus - 5.5`).
    fn model_label_of(&self, id: &str) -> String {
        let info = self
            .catalog
            .as_ref()
            .and_then(|c| c.models.iter().find(|m| m.id == id));
        match (info, &self.adapter) {
            (Some(info), Some(adapter)) => info
                .resolved
                .as_deref()
                .map_or_else(|| info.label.clone(), |r| adapter.model_label(r)),
            (Some(info), None) => info.label.clone(),
            (None, _) => id.to_string(),
        }
    }

    /// Modelos para a Config: os do catálogo, ou os fixos do provedor
    /// enquanto o catálogo não chega.
    pub fn model_options(&self) -> Vec<agent::ModelOption> {
        match &self.catalog {
            Some(catalog) if !catalog.models.is_empty() => catalog
                .models
                .iter()
                .map(|m| agent::ModelOption {
                    id: m.id.clone(),
                    label: self.model_label_of(&m.id),
                })
                .collect(),
            _ => self
                .adapter
                .as_ref()
                .map(|a| a.models())
                .unwrap_or_default(),
        }
    }

    /// `/clear`: começa uma conversa nova, com o histórico limpo.
    fn clear_conversation(&mut self) {
        self.run = None;
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

    /// A roda rola a conversa em qualquer ponto da aba; arrastar no
    /// histórico seleciona e copia; no input, posiciona o cursor e seleciona.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let (column, row) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollUp => self.history.scroll_up(WHEEL_LINES),
            MouseEventKind::ScrollDown => self.history.scroll_down(WHEEL_LINES),
            MouseEventKind::Down(MouseButton::Left) if self.history.contains(column, row) => {
                if let Some(text) = self.history.mouse_down(column, row) {
                    self.copy(&text);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.history.is_selecting() => {
                self.history.mouse_drag(column, row);
            }
            MouseEventKind::Up(MouseButton::Left) if self.history.is_selecting() => {
                if let Some(text) = self.history.mouse_up(column, row) {
                    self.copy(&text);
                }
            }
            _ => {
                self.input.handle_mouse(mouse);
            }
        }
    }

    /// Envia o texto do input ao agente. Ignora texto vazio e, enquanto o
    /// agente ainda responde, mantém o texto para enviar depois. Um comando
    /// com `/` e argumentos (ex.: `/config model=opus`) vai como está: é o
    /// padrão do router; sem argumentos, um comando com interface no terra
    /// (ex.: `/clear`) roda a interface.
    fn send(&mut self) {
        let prompt = self.input.text().trim().to_string();
        if prompt.is_empty() || self.run.is_some() {
            return;
        }
        if let Some(name) = prompt.strip_prefix('/')
            && commands::has_interface(name)
        {
            self.input.remember();
            self.input.clear();
            self.run_command(name);
            return;
        }
        if prompt.starts_with('/') && self.route_to_provider(&prompt) {
            self.input.remember();
            self.input.clear();
            self.history.follow();
            return;
        }
        if self.start(&prompt, Entry::User(prompt.clone())) {
            self.input.remember();
            self.input.clear();
            // A auto compactação é para as mensagens, não para os comandos.
            self.auto_compact_armed = !prompt.starts_with('/');
        }
    }

    /// Manda `prompt` ao agente como uma mensagem do usuário (ex.: a
    /// primeira de um chat aberto pelo orquestrador).
    pub fn submit(&mut self, prompt: &str) {
        if self.run.is_none() && self.start(prompt, Entry::User(prompt.to_string())) {
            self.auto_compact_armed = true;
        }
    }

    /// Manda `prompt` ao agente na sessão atual e, se ele começou, põe
    /// `entry` no histórico. Retorna se a resposta começou.
    fn start(&mut self, prompt: &str, entry: Entry) -> bool {
        let Some(adapter) = self.available_adapter() else {
            self.history.push(Entry::Error(self.unavailable_hint()));
            return false;
        };
        let started = adapter.start(Request {
            prompt,
            mode: self.permission_mode,
            session_id: self.session_id.as_deref(),
            model: self
                .session_model
                .as_deref()
                .or(self.chosen_model.as_deref()),
            account: self.account.as_deref(),
            effort: self
                .session_effort
                .as_deref()
                .or(self.default_effort.as_deref()),
            extensions: &self.extensions,
            // Num chat delegado, quem delegou decide os pedidos de permissão
            // (pelo MCP do terra, quando ele está ligado ao chat).
            permission_tool: self
                .delegation
                .filter(|_| {
                    self.extensions
                        .mcp_servers
                        .iter()
                        .any(|s| s.name == mcp::SERVER_NAME)
                })
                .map(|_| (mcp::SERVER_NAME, mcp::PERMISSION_TOOL)),
        });
        let ok = started.is_ok();
        match started {
            Ok(run) => {
                self.run = Some(run);
                self.requested_mode = Some(self.permission_mode);
                self.history.push(entry);
                self.agent_status = AgentStatus::Pensando;
                self.agent_activity = Some("Pensando...".to_string());
            }
            Err(err) => {
                self.history.push(Entry::Error(err));
                self.agent_status = AgentStatus::Erro;
            }
        }
        self.history.follow();
        ok
    }

    /// Manda a resposta do usuário a um pedido de permissão e a registra no
    /// histórico.
    fn answer(&mut self, request: permission::Request, allow: bool) {
        if let Some(run) = &mut self.run {
            run.respond(&request.id, allow);
        }
        let verdict = if allow { "Permitido" } else { "Negado" };
        let detail = if request.detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", request.detail)
        };
        self.history
            .push(Entry::Info(format!("{verdict}: {}{detail}.", request.name)));
        self.agent_activity = Some(match self.prompts.current() {
            Some(next) => format!("Aguardando permissão para {}...", next.name),
            None => "Pensando...".to_string(),
        });
        self.history.follow();
    }

    /// Interrompe a resposta em andamento (soltar a sessão a encerra).
    fn cancel(&mut self) {
        self.run = None;
        self.prompts.clear();
        self.auto_compact_armed = false;
        self.history.push(Entry::Info("Interrompido.".to_string()));
        self.agent_status = AgentStatus::Ocioso;
        self.agent_activity = None;
    }

    /// Aplica um evento do agente ao histórico e ao status.
    fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Init {
                session_id,
                model,
                permission_mode,
            } => {
                self.session_id = Some(session_id);
                self.model = Some(model).filter(|m| !m.is_empty());
                self.sync_mode(permission_mode);
            }
            AgentEvent::TextStart => {
                self.history.start_text();
                self.agent_activity = Some("Respondendo...".to_string());
            }
            AgentEvent::TextDelta(text) => self.history.append_text(&text),
            AgentEvent::Text(text) => self.history.push(Entry::Assistant(text)),
            AgentEvent::ToolUse { name, detail } => {
                self.agent_activity = Some(format!("Usando {name}..."));
                self.history.push(Entry::Tool { name, detail });
            }
            AgentEvent::PermissionRequest { id, name, detail } => {
                self.agent_activity = Some(format!("Aguardando permissão para {name}..."));
                self.prompts.push(permission::Request { id, name, detail });
            }
            AgentEvent::ToolResult { is_error, summary } => {
                self.agent_activity = Some("Pensando...".to_string());
                self.history.push(Entry::ToolResult { is_error, summary });
            }
            AgentEvent::Context { tokens, window } => {
                self.context = Some(tokens);
                // O limite só vem no fim da resposta: fica o último informado.
                if window.is_some() {
                    self.context_window = window;
                }
            }
            AgentEvent::Compacted { tokens } => {
                // O tamanho depois da compactação mantém a barra; sem ele, o
                // novo vem na próxima chamada ao modelo. A mensagem já foi
                // compactada (pelo próprio agente): não compacta de novo.
                self.context = tokens;
                self.auto_compact_armed = false;
                self.history
                    .push(Entry::Info("Conversa compactada.".to_string()));
            }
            AgentEvent::Done {
                is_error,
                message,
                tokens,
                cache_tokens,
                denied,
            } => {
                self.tokens_used += tokens;
                self.cache_tokens_used += cache_tokens;
                if denied > 0 {
                    self.history.push(Entry::Info(format!(
                        "{denied} ferramenta(s) negada(s) no modo de permissão {}.",
                        self.permission_mode.label()
                    )));
                }
                if let Some(message) = message {
                    self.history.push(Entry::Error(message.clone()));
                    self.hint_login(&message);
                }
                let armed = std::mem::take(&mut self.auto_compact_armed);
                self.finish(is_error);
                if !is_error && armed {
                    self.auto_compact();
                }
            }
            AgentEvent::Failed(message) => {
                self.history.push(Entry::Error(message.clone()));
                self.hint_login(&message);
                self.finish(true);
            }
        }
    }

    /// Sincroniza com o modo em que o agente disse estar. Se ele não for o
    /// pedido (o provedor recusou ou trocou), o terminal passa a mostrar o
    /// modo real; uma troca feita com Shift+Tab durante a resposta é mantida
    /// para a próxima mensagem.
    /// O Yolo não é sincronizado: o agente roda pedindo permissão e o terra
    /// segue em Yolo, aprovando tudo.
    fn sync_mode(&mut self, reported: ReportedMode) {
        if self.requested_mode == Some(PermissionMode::Yolo) {
            self.agent_mode = Some(PermissionMode::Yolo);
            return;
        }
        let mode = match reported {
            ReportedMode::Known(mode) if mode != PermissionMode::Yolo => mode,
            ReportedMode::Unknown(name) => {
                self.history.push(Entry::Info(format!(
                    "O agente está no modo \"{name}\", que este terminal não conhece."
                )));
                self.agent_mode = None;
                return;
            }
            _ => {
                self.agent_mode = None;
                return;
            }
        };
        self.agent_mode = Some(mode);
        if self
            .requested_mode
            .is_some_and(|requested| requested != mode)
        {
            self.permission_mode = mode;
            self.history.push(Entry::Info(format!(
                "O agente está no modo {}.",
                mode.label()
            )));
        }
    }

    fn finish(&mut self, is_error: bool) {
        self.run = None;
        self.prompts.clear();
        self.auto_compact_armed = false;
        self.agent_activity = None;
        self.agent_status = if is_error {
            AgentStatus::Erro
        } else {
            AgentStatus::Ocioso
        };
    }

    fn copy(&mut self, text: &str) {
        let message = if clipboard::copy(text) {
            format!("Copiado ({} caracteres)", text.chars().count())
        } else {
            "Não foi possível copiar".to_string()
        };
        self.notice = Some((message, NOTICE_TICKS));
    }

    /// Avança o spinner, o cursor piscante e os avisos, e lê o que o agente
    /// mandou; chamado a cada tick do loop.
    /// Um panic numa thread de fundo do provedor: vira um erro no histórico
    /// e encerra a resposta em andamento, que não vai mais terminar.
    pub fn provider_failed(&mut self, reason: &str) {
        let name = self.adapter.as_ref().map_or("O provedor", |a| a.name());
        let message = format!("{name} falhou: {reason}");
        if self.run.is_some() {
            self.apply(AgentEvent::Failed(message));
        } else {
            self.history.push(Entry::Error(message));
        }
    }

    /// Um aviso de fora do chat (ex.: o terra abriu sem o banco).
    pub fn warn(&mut self, message: String) {
        self.history.push(Entry::Error(message));
    }

    pub fn tick(&mut self) {
        self.poll_catalog();
        let events = self
            .run
            .as_mut()
            .map(|session| session.poll())
            .unwrap_or_default();
        for event in events {
            self.apply(event);
        }
        self.history.tick();
        self.input.tick();
        if let Some((_, ticks)) = &mut self.notice {
            *ticks -= 1;
            if *ticks == 0 {
                self.notice = None;
            }
        }
        if self.agent_activity.is_some() {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER.len();
        }
    }

    pub fn set_caps_lock(&mut self, on: bool) {
        self.input.set_caps_lock(on);
    }

    /// Texto do rodapé: o aviso do momento ou as teclas do modo atual.
    pub fn help(&self) -> Cow<'static, str> {
        if let Some((message, _)) = &self.notice {
            return Cow::Owned(message.clone());
        }
        Cow::Borrowed(match self.input.mode() {
            _ if self.model_menu.is_some() => HELP_MODEL,
            _ if self.prompts.is_open() => HELP_PERMISSION,
            _ if self.input.command_picker_open() => HELP_COMMANDS,
            _ if self.input.picker_open() => HELP_PICKER,
            InputMode::Normal if self.run.is_some() => HELP_RUNNING,
            InputMode::Normal => HELP_NORMAL,
            InputMode::Insert => HELP_INSERT,
        })
    }

    /// Histórico da conversa (acima do painel).
    pub fn draw_history(&self, frame: &mut Frame, area: Rect) {
        let hint = match self.available_adapter() {
            Some(adapter) => format!(
                "Converse com o {}: i para escrever, Enter para enviar.",
                adapter.name()
            ),
            None => self.unavailable_hint(),
        };
        let empty = vec![
            Line::styled("Chat", theme::section_title()),
            Line::styled(hint, theme::muted()),
        ];
        self.history.draw(frame, area, empty);
    }

    /// Altura do painel: sem badges para mostrar, a linha deles some e o
    /// espaço fica para a conversa.
    pub fn height(&self) -> u16 {
        if self.badges().is_empty() && self.context_bar().is_none() && self.usage_bar().is_none() {
            HEIGHT - badge::HEIGHT
        } else {
            HEIGHT
        }
    }

    /// Badges visíveis: o modo fora do padrão (Agente) ou com o agente ativo
    /// (respondendo ou com sessão iniciada), e o consumo depois do primeiro
    /// token.
    fn badges(&self) -> Vec<(String, Variant)> {
        let mode = self.permission_mode;
        let active = self.run.is_some() || self.agent_mode.is_some();
        let mut badges = Vec::new();
        if mode != PermissionMode::default() || active {
            // Trocado com Shift+Tab durante a resposta: o agente segue no modo
            // dele até a próxima mensagem.
            let label = match self.agent_mode {
                Some(current) if self.run.is_some() && current != mode => {
                    format!("Modo: {} → {}", current.label(), mode.label())
                }
                _ => format!("Modo: {}", mode.label()),
            };
            badges.push((label, mode_variant(mode)));
        }
        if self.tokens_used > 0 {
            badges.push((
                format!("Consumo: {} Tokens", compact_count(self.tokens_used)),
                Variant::Secondary,
            ));
        }
        badges
    }

    /// Limite de contexto do modelo: o informado pelo agente ou o padrão.
    pub fn context_window(&self) -> u64 {
        self.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW)
    }

    /// Tokens em que a conversa deve ser compactada (o ponto da Config).
    fn compact_tokens(&self) -> u64 {
        self.context_settings
            .compact_tokens(self.context_window())
            .max(1)
    }

    /// Com a auto compactação ligada, compacta a conversa quando o contexto
    /// chega no ponto de compactar. Só no fim da resposta a uma mensagem do
    /// usuário, e uma vez por mensagem.
    fn auto_compact(&mut self) {
        let full = self.context.is_some_and(|t| t >= self.compact_tokens());
        if self.context_settings.auto_compact && full {
            self.start_compact(Entry::Info(format!(
                "O contexto chegou em {}% do limite: compactando a conversa...",
                self.context_settings.compact_at
            )));
        }
    }

    /// A barra de contexto, quando ela está visível e o agente já informou o
    /// tamanho dele: o texto e quanto do ponto de compactar está ocupado (de
    /// 0 a 1).
    fn context_bar(&self) -> Option<(String, f64)> {
        let tokens = self.context.filter(|_| self.context_settings.visible)?;
        let limit = self.compact_tokens();
        let ratio = tokens as f64 / limit as f64;
        let label = if ratio >= 1.0 {
            COMPACT_HINT.to_string()
        } else {
            format!(
                "{} / {} ({:.0}%)",
                compact_count(tokens),
                compact_count(limit),
                (ratio * 100.0).floor()
            )
        };
        Some((label, ratio.min(1.0)))
    }

    /// A barra de consumo, quando ela está visível e o provedor já informou o
    /// consumo da sessão: o texto e quanto dela resta (de 0 a 100).
    fn usage_bar(&self) -> Option<(String, f64)> {
        let window = self
            .session_usage
            .as_ref()
            .filter(|_| self.usage_settings.visible)?;
        let left = (100.0 - window.percent).clamp(0.0, 100.0);
        Some((usage::session_label(window, &TimeZone::system()), left))
    }

    /// Painel: atividade do agente, caixa de input e badges.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        // Os badges ficam com o que sobra: sem eles, o painel encolhe (veja
        // `height`) e o input mantém a altura inteira.
        let [activity, input, badges] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(input::HEIGHT),
            Constraint::Fill(1),
        ])
        .areas(area);

        if let Some(text) = &self.agent_activity {
            let line = format!(" {} {text}", SPINNER[self.spinner_frame]);
            frame.render_widget(
                Paragraph::new(Span::styled(line, theme::activity())),
                activity,
            );
        }
        let status = self.agent_status;
        let status = Line::styled(
            format!(" {} ", status.label()),
            theme::variant(status.variant()),
        );
        self.input.draw(frame, input, self.model_title(), status);
        if let Some(menu) = &self.model_menu {
            let label = |m: &agent::ModelInfo| self.model_label_of(&m.id);
            menu.draw(frame, input, self.catalog.as_ref(), &label);
        }
        self.prompts.draw(frame, input);
        self.draw_badges(frame, badges);
    }

    /// "[Router] Modelo - Versão", com o router no estilo de botão (pill).
    fn model_title(&self) -> Line<'static> {
        // TODO: o botão do router vai abrir a troca de provedor (adapters::all).
        let router = self.adapter.as_ref().map_or("Sem agente", |a| a.name());
        let model = match (&self.model, &self.adapter) {
            (Some(id), Some(adapter)) => adapter.model_label(id),
            _ => "Modelo padrão".to_string(),
        };
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!(" {router} "), theme::badge_text(Variant::Active)),
            Span::styled(format!(" {model} "), theme::text()),
        ])
    }

    /// Os badges à esquerda e, no canto direito, embaixo do status, a barra
    /// de contexto, com a de consumo à esquerda dela.
    fn draw_badges(&self, frame: &mut Frame, area: Rect) {
        let mut area = area;
        if let Some((label, ratio)) = self.context_bar() {
            let [badges, context] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Length(CONTEXT_WIDTH)])
                    .spacing(1)
                    .areas(area);
            area = badges;
            let gauge = Gauge::default()
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(theme::context_border(ratio)),
                )
                .gauge_style(theme::context_bar(ratio))
                .ratio(ratio)
                .label(label);
            frame.render_widget(gauge, context);
        }
        if let Some((label, left)) = self.usage_bar() {
            let [badges, usage] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Length(USAGE_WIDTH)])
                    .spacing(1)
                    .areas(area);
            area = badges;
            let gauge = Gauge::default()
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(theme::usage_border(left)),
                )
                .gauge_style(theme::usage_bar(left))
                .ratio(left / 100.0)
                .label(label);
            frame.render_widget(gauge, usage);
        }
        let badges = self.badges();
        let areas = Layout::horizontal(
            badges
                .iter()
                .map(|(label, _)| Constraint::Length(badge::width(label))),
        )
        .spacing(1)
        .flex(Flex::Start)
        .split(area);
        for ((label, variant), area) in badges.iter().zip(areas.iter()) {
            frame.render_widget(badge::badge(label, *variant), *area);
        }
    }
}

/// Formata contagens de forma compacta: 950, 12.3K, 1M.
pub(crate) fn compact_count(n: u64) -> String {
    let (value, suffix) = match n {
        0..1_000 => return n.to_string(),
        1_000..1_000_000 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{suffix}")
}
