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

mod draw;
mod hub_sync;
mod keys;
mod provider;
mod settings;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use ratatui::layout::Rect;

use super::chat::Chat;
use super::config::ConfigView;
use super::extensions::ExtensionsView;
use super::orchestrator::{self as orchestrator_tab, OrchestratorView};
use super::term::Term;
use super::update::UpdateView;
use super::usage::{SessionUsage, UsageView};
use crate::agent::adapters::ProviderOption;
use crate::db::Db;
use crate::extension::Registry;
use crate::hub;
use crate::hub::client::{Monitor, Reporter};
use crate::platform::keyboard;
use crate::settings::{AgentSettings, ExtensionSettings, Interface};

/// O título do chat do terra na barra de chats.
const MAIN_CHAT_TITLE: &str = "Principal";

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

/// Os arquivos do terra: o `config.json` do workspace (as configurações), o
/// SQLite do workspace (o histórico de conversas) e o `config.json` global
/// (as do orquestrador).
pub struct ConfigFiles {
    pub workspace: PathBuf,
    pub history: PathBuf,
    pub global: PathBuf,
}

impl ConfigFiles {
    /// Cria o que falta do workspace (a pasta, o `config.json` e o histórico),
    /// como o `terra.sqlite`, sem mexer no que já existe.
    pub fn create(&self) -> anyhow::Result<()> {
        crate::settings::create_file(&self.workspace)?;
        crate::db::history::move_old_file(&self.history)?;
        crate::db::history::ChatHistory::open(&self.history)?;
        Ok(())
    }
}

/// Um chat novo, aberto com `Alt+N`, além do principal.
struct ChatTab {
    /// O id do chat no hub.
    id: String,
    /// A execução despachada que a aba mostra (só leitura).
    run: Option<String>,
    /// O subagente que a aba mostra (só leitura).
    subagent: Option<SubagentTab>,
    title: String,
    chat: Chat,
}

/// De onde vem o que a aba de um subagente mostra.
struct SubagentTab {
    /// O id da chamada que o lançou.
    id: String,
    /// O id do chat que o lançou.
    parent: String,
    /// Por onde a aba recebe os eventos; `None` depois que ele termina.
    feed: Option<std::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    /// Terminou com erro (ou o chat que o lançou parou antes dele).
    failed: bool,
}

impl SubagentTab {
    /// Em que pé ele está, para o hub.
    fn state(&self) -> crate::hub::protocol::SubagentState {
        use crate::hub::protocol::SubagentState;
        match (&self.feed, self.failed) {
            (Some(_), _) => SubagentState::Running,
            (None, false) => SubagentState::Done,
            (None, true) => SubagentState::Failed,
        }
    }
}

#[derive(Default)]
pub struct App {
    tab: Tab,
    focus: Focus,
    chat: Chat,
    /// O id do chat principal no hub.
    chat_id: String,
    /// Os chats além do principal, abertos com `Alt+N`.
    chats: Vec<ChatTab>,
    /// Os tokens dos chats já fechados, que continuam no consumo geral.
    closed_tokens: u64,
    /// O chat que aparece na aba Chat: 0 é o do terra, os outros são os
    /// de [`App::chats`].
    active_chat: usize,
    /// Onde cada aba foi desenhada na última tela, para o clique do mouse.
    tab_rects: RefCell<Vec<(Rect, Tab)>>,
    /// Onde cada chat da barra de chats foi desenhado (índice como em
    /// [`App::active_chat`]); vazio quando a barra não aparece.
    chat_rects: RefCell<Vec<(Rect, usize)>>,
    /// Onde o botão "Sair" foi desenhado.
    quit_rect: Cell<Rect>,
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
    /// O SQLite do histórico de conversas do workspace (`None` nos testes).
    history: Option<PathBuf>,
    /// A aba Orquestrador, com o que ela lê do disco.
    orchestrator_view: OrchestratorView,
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
        let mut chat = Chat::connect(&settings);
        chat.set_history_file(files.history.clone());
        let folder = dir.to_string_lossy().into_owned();
        chat.set_usage_folder(folder.clone());
        let mut usage = UsageView::default();
        usage.set_session(files.history.clone(), folder);
        let mut app = Self {
            usage,
            chat,
            chat_id: hub::protocol::new_id(),
            providers: crate::agent::adapters::options(),
            open_provider: settings.interface == Interface::Provider,
            settings,
            registry: Registry::detect(),
            config: Some(config),
            history: Some(files.history),
            orchestrator_view: OrchestratorView::new(orchestrator_tab::Data::new(
                hub::endpoint::Endpoint::current().ok().map(|e| e.home),
            )),
            workspace: Some(Workspace { dir, saved }),
            db: Some(db),
            hub,
            ..Self::default()
        };
        app.load_extensions();
        app.update_view.check();
        app.migrate_orchestrator(&files.global);
        app
    }

    /// As antigas configurações do orquestrador saem do `config.json`
    /// global uma vez; o "Ao abrir: mostrar o chat" vai para o
    /// `orchestrator.toml` (`aba_em_primeiro_plano`).
    fn migrate_orchestrator(&mut self, global: &std::path::Path) {
        let focus = crate::settings::take_legacy_orchestrator(global);
        let path = self.orchestrator_view.data.path();
        if let (Some(true), Some(path)) = (focus, path)
            && let Ok(mut editor) = crate::orchestrator::edit::Editor::open(&path)
        {
            editor.set_global("aba_em_primeiro_plano", true);
            let _ = editor.save();
        }
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

    /// Abre um chat novo e vazio neste terra, com as configurações do
    /// workspace, e passa para ele.
    fn new_chat(&mut self) {
        let mut chat = Chat::connect(&self.settings);
        chat.set_extensions(self.chat.extensions().clone());
        chat.set_session_usage(self.session_usage.window().cloned());
        if let Some(file) = &self.history {
            chat.set_history_file(file.clone());
        }
        if let Some(workspace) = &self.workspace {
            chat.set_usage_folder(workspace.dir.to_string_lossy().into_owned());
        }
        let title = format!("Chat {}", self.chats.len() + 2);
        self.chats.push(ChatTab {
            id: hub::protocol::new_id(),
            run: None,
            subagent: None,
            title,
            chat,
        });
        self.active_chat = self.chats.len();
    }

    /// Fecha o chat além do principal que está na aba Chat (o do terra
    /// não fecha).
    fn close_chat(&mut self) {
        if self.active_chat > 0 {
            let mut tab = self.chats.remove(self.active_chat - 1);
            tab.chat.save_history();
            self.closed_tokens += tab.chat.tokens_used();
            self.active_chat -= 1;
        }
    }

    /// Grava o que falta dos chats no histórico do workspace, ao sair.
    pub fn save_history(&mut self) {
        self.chat.save_history();
        for tab in &mut self.chats {
            tab.chat.save_history();
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

    /// As teclas vão para o terminal: o `Ctrl+C` também é dele.
    pub fn captures_keys(&self) -> bool {
        self.tab == Tab::Chat && self.focus == Focus::Tabs && self.term_focus && self.term.is_some()
    }

    fn uses_provider(&self) -> bool {
        self.settings.interface == Interface::Provider
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
        self.sync_subagents();
        // O consumo só é carregado com a aba aberta.
        if self.tab == Tab::Consumo {
            self.usage
                .refresh_if_stale(self.chat.adapter(), self.settings.account.as_deref());
            self.usage.refresh_session();
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
        self.receive_from_hub();
        // No Orquestrador, a aba relê o arquivo e as execuções e assina a
        // lista do hub (só com o hub ligado: não nos testes).
        if self.tab == Tab::Orquestrador {
            if self.hub.is_some() && self.hub_monitor.is_none() {
                self.hub_monitor = Monitor::start();
            }
            self.orchestrator_view.data.refresh_if_stale();
        }
    }
}
