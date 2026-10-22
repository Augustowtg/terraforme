//! Terminal do terraforme (`terraforme cli`): o agente roda num pseudoterminal
//! dentro de uma TUI com abas no topo — o agente (com uma sub-aba por
//! instância), o consumo de tokens (com os relatórios do RTK), as outras
//! extensões do modo e a configuração geral.
//!
//! As abas são construídas pelo padrão de projeto **Registro**: cada uma é uma
//! única entrada de [`TABS`] ([`TabSpec`]) com o rótulo, o atalho, as sub-abas
//! próprias e o comportamento dela, e o menu, os atalhos, a ajuda e a ordem
//! saem todos dessa lista (ver [`spec`]). Abas: `Ctrl+Shift+F1` (Agente),
//! `Ctrl+Shift+F2` (Consumo), `Ctrl+Shift+F3` (Extensões, quando o modo abre
//! uma) e `Ctrl+Shift+F4` (Configurações), ou `Ctrl+Shift+←/→` para a aba
//! vizinha (`←`/`→` sozinhas nas visões Consumo e Extensões); o menu
//! (blocos `Terraforme` e `Agente (claude)`, `Consumo`, … `Sair`, a aba ativa
//! verde e as outras cinza, atalhos do outro lado) também abre a aba com um clique. Sub-abas, iguais em todas as abas:
//! `Ctrl+Alt+Tab` passa para a próxima e `Ctrl+Alt+1`–`9` vai direto
//! (`Alt+1`–`9`, `Shift+Alt+1`–`9` e `Ctrl+PgUp/PgDn` também); na aba do
//! agente, um número além da última abre outra instância.
//! Os atalhos próprios da TUI usam `Ctrl+Alt`, que o agente não usa
//! (`Ctrl+Alt+K` lista todos). Para distinguir `Ctrl+Shift`/`Ctrl+Alt` de
//! teclas comuns, a TUI liga o protocolo de teclado do kitty quando o terminal
//! o suporta; sem ele, `Shift+Tab` segue para o agente. As instâncias continuam
//! rodando enquanto outra aba ou sub-aba está aberta.
//!
//! Mouse: a roda rola o histórico do agente e o conteúdo das outras abas;
//! arrastar seleciona texto do agente e copia ao soltar (OSC 52 e, no Linux,
//! `wl-copy`/`xclip`/`xsel`). `Ctrl+Alt+S` devolve o mouse ao terminal, para a
//! seleção nativa.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::{Frame, crossterm};
use terraforme_core::accounts::ClaudeAccounts;
use terraforme_core::agent::{Adapter, CliMode, Mode, Provider};
use terraforme_core::config::Config;
use terraforme_core::extension::{Extension, Report};
use terraforme_core::history::{History, Record, SpaceUsage};
use terraforme_core::limits::{self, Budget, Limit, Source};
use terraforme_core::pet::{Activity, Tracker};
use terraforme_core::space::Preferences;
use terraforme_core::usage::{self, Live, Summary, Tokens, UsageSource};
use tui_term::widget::PseudoTerminal;

use crate::chat::Chat;

/// Linhas de histórico guardadas da saída do agente.
const SCROLLBACK: usize = 5000;
/// Intervalo de atualização automática da aba de consumo.
const USAGE_REFRESH: Duration = Duration::from_secs(5);
/// Intervalo da leitura do consumo das instâncias para o rodapé ([`Live`]).
const LIVE_EVERY: Duration = Duration::from_secs(5);
/// Sub-abas da aba de consumo.
const USAGE_VIEWS: [&str; 4] = ["Limites", "Tokens", "Subagentes", "Spaces"];
/// Sub-aba do consumo com o histórico por space e agente.
const SPACES_VIEW: usize = 3;
/// Intervalo entre as gravações do consumo das instâncias no histórico.
const RECORD_EVERY: Duration = Duration::from_secs(60);
/// Largura da barra de progresso dos limites.
const BAR_WIDTH: usize = 40;
/// Máximo de instâncias do agente abertas ao mesmo tempo.
pub const MAX_INSTANCES: usize = 9;
/// Espera mínima até entregar um prompt pendente à instância: a TUI do
/// agente precisa estar pronta para recebê-lo.
const PROMPT_DELAY: Duration = Duration::from_secs(2);
/// Silêncio na saída do agente antes de digitar o prompt pendente (a TUI
/// dele terminou de desenhar).
const PROMPT_QUIET: Duration = Duration::from_millis(500);
/// Até quando esperar o agente ligar o bracketed paste (sinal de que o
/// campo do prompt está pronto) antes de digitar mesmo assim.
const PROMPT_READY_TIMEOUT: Duration = Duration::from_secs(20);
/// Espera entre colar o prompt e mandar o Enter: no mesmo bloco, o agente
/// trata o Enter como parte da colagem e não envia.
const SUBMIT_DELAY: Duration = Duration::from_millis(300);
/// Espera entre ver o agente encerrado e fechar a instância, para a saída
/// que ainda está a caminho chegar (ver [`Agent::exited`]).
const EXIT_GRACE: Duration = Duration::from_millis(500);
/// Linhas roladas por passo da roda do mouse.
const WHEEL_LINES: u16 = 3;
/// Linhas acima do pseudoterminal do agente: o menu e as sub-abas.
const AGENT_TOP: u16 = 2;
/// Linha da tela em que começa o texto da aba Config (menu e borda acima).
const FORM_TOP: u16 = 2;
/// Tempo que a mensagem de status (ex.: "copiado") fica na tela.
const STATUS_TTL: Duration = Duration::from_secs(3);

/// O que `run` precisa para montar o terminal.
pub struct Options {
    /// Agente resolvido para as instâncias abertas ao iniciar.
    pub launched: Launched,
    /// Refaz o [`Launched`] quando a aba Config troca o agente ou o modo.
    pub relaunch: Relaunch,
    /// Instâncias abertas ao iniciar.
    pub instances: usize,
    /// Lock file do daemon MCP; `None` se o MCP não está disponível.
    pub mcp_lock: Option<terraforme_core::mcp::LockFile>,
    /// Id único desta instância da CLI para o registro no MCP.
    pub client_id: String,
    /// Space da pasta atual, mostrado no apelido das instâncias no MCP.
    pub space: Option<String>,
}

/// Agente resolvido pelo `terraforme cli`: o que cada instância nova abre.
pub struct Launched {
    /// Nome da aba do agente (ex.: `claude`).
    pub agent: String,
    /// Texto à direita do menu (provider e modo).
    pub title: String,
    /// Integração com o agente (sessão e consumo).
    pub adapter: &'static dyn Adapter,
    /// Comando de cada instância do agente.
    pub command: Command,
    /// Passa um id de sessão próprio a cada instância, para o consumo saber
    /// qual é a sessão dela; desligado quando os argumentos já escolhem a sessão.
    pub session_ids: bool,
    /// Aparecem num aviso, já que a tela alternativa esconderia o que fosse
    /// impresso antes.
    pub notices: Vec<String>,
    /// Extensões do modo; os relatórios delas ficam na aba Extensões.
    pub extensions: Vec<Extension>,
}

/// Resolve de novo o agente com outro agente, modo e sandbox.
pub type Relaunch = Box<dyn Fn(Provider, Mode, bool) -> Result<Launched>>;

/// Ligação do terminal com o daemon MCP: cada instância se registra nele (e
/// some ao fechar) e recebe as mensagens das outras pelo servidor de controle.
struct Link {
    port: u16,
    client_id: String,
    /// Endereço do servidor de controle deste terminal.
    control_addr: String,
    space: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// O agente, com uma sub-aba por instância.
    Agent,
    /// Consumo de tokens do agente e, se o modo usa o RTK, a economia dele.
    Usage,
    /// Relatórios das outras extensões do modo, um por sub-aba.
    Extensions,
    /// Configuração geral (`config.toml`), editável.
    Config,
}

impl Tab {
    /// Nome da aba no menu: o agente leva o nome dele entre parênteses
    /// (`Agente (claude)`).
    fn label(self, agent: &str) -> String {
        match self {
            Tab::Agent => format!("{} ({agent})", self.nome()),
            other => other.nome().to_string(),
        }
    }

    /// Nome fixo da aba, para a ajuda e para o menu quando não há nome de
    /// agente.
    fn nome(self) -> &'static str {
        match self {
            Tab::Agent => "Agente",
            Tab::Usage => "Consumo",
            Tab::Extensions => "Extensões",
            Tab::Config => "Configurações",
        }
    }

    /// Sub-abas como (grupo, rótulo): as sub-abas próprias da aba, tiradas do
    /// Registro [`TABS`] (sem grupo), e os relatórios, agrupados pela
    /// extensão. As instâncias do agente ficam à parte, porque não carregam
    /// conteúdo.
    fn views(self, extensions: &[Extension]) -> Vec<(&'static str, &'static str)> {
        spec(self)
            .own_views
            .iter()
            .map(|&v| ("", v))
            .chain(
                reports(self, extensions)
                    .into_iter()
                    .map(|(ext, report)| (ext.name(), report.label)),
            )
            .collect()
    }
}

/// Relatórios das extensões mostrados na aba, na ordem das sub-abas: os do
/// RTK (economia de tokens) ficam no Consumo, os demais em Extensões.
fn reports(tab: Tab, extensions: &[Extension]) -> Vec<(Extension, Report)> {
    extensions
        .iter()
        .filter(|&&ext| match tab {
            Tab::Usage => ext == Extension::Rtk,
            Tab::Extensions => ext != Extension::Rtk,
            _ => false,
        })
        .flat_map(|&ext| ext.reports().iter().map(move |&report| (ext, report)))
        .collect()
}

/// Como a aba se comporta, escolhido no Registro [`TABS`] em vez de por
/// `match tab` espalhado pelo arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabKind {
    /// O agente: um pseudoterminal por sub-aba, sem conteúdo carregado.
    Agent,
    /// Conteúdo carregado em segundo plano (Consumo e Extensões).
    Content,
    /// Formulário editável, gravado a cada alteração (Config).
    Form,
}

/// Item clicável do menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    /// Abre a aba.
    Tab(Tab),
    /// Sai (igual a `F10`).
    Quit,
}

/// Descrição de uma aba.
///
/// Padrão de projeto **Registro** (registry / table-driven): as entradas de
/// [`TABS`] são a fonte única das abas — ordem no menu, nome, atalho,
/// sub-abas próprias e comportamento —, e menu, atalhos, ajuda e abertura
/// leem esta lista em vez de espalhar `match tab` pelo arquivo. Para criar
/// uma aba nova basta acrescentar a entrada e dizer o seu [`TabKind`].
struct TabSpec {
    /// Aba descrita.
    tab: Tab,
    /// Número da tecla `Ctrl+Shift+F<n>` que a abre (mais as variantes
    /// legadas `F13`–`F16` com Ctrl e `F25`–`F28` sem modificadores).
    shortcut: u8,
    /// Se a aba aparece no menu com estas extensões.
    show: fn(&[Extension]) -> bool,
    /// Sub-abas próprias da aba, antes dos relatórios (as visões do Consumo).
    own_views: &'static [&'static str],
    /// Recarrega o conteúdo sozinho, a cada [`USAGE_REFRESH`].
    refresh: bool,
    /// Como a aba se constrói.
    kind: TabKind,
}

/// Registro das abas, na ordem do menu e na ordem dos atalhos `F1`–`F4`.
const TABS: [TabSpec; 4] = [
    TabSpec {
        tab: Tab::Agent,
        shortcut: 1,
        show: sempre,
        own_views: &[],
        refresh: false,
        kind: TabKind::Agent,
    },
    TabSpec {
        tab: Tab::Usage,
        shortcut: 2,
        show: sempre,
        own_views: &USAGE_VIEWS,
        refresh: true,
        kind: TabKind::Content,
    },
    TabSpec {
        tab: Tab::Extensions,
        shortcut: 3,
        show: tem_relatorios,
        own_views: &[],
        refresh: false,
        kind: TabKind::Content,
    },
    TabSpec {
        tab: Tab::Config,
        shortcut: 4,
        show: sempre,
        own_views: &[],
        refresh: false,
        kind: TabKind::Form,
    },
];

impl TabSpec {
    /// Atalho por extenso, na janela de atalhos (`Ctrl+Shift+F4`).
    fn tecla_longa(&self) -> String {
        format!("Ctrl+Shift+F{}", self.shortcut)
    }
}

/// Descrição da aba, sempre presente no Registro.
fn spec(tab: Tab) -> &'static TabSpec {
    TABS.iter()
        .find(|s| s.tab == tab)
        .expect("toda aba tem entrada no registro")
}

/// Aba que o atalho `n` (`Ctrl+Shift+F<n>`) abre; `None` quando nenhuma
/// usa esse número.
fn tab_by_shortcut(n: u8) -> Option<Tab> {
    TABS.iter().find(|s| s.shortcut == n).map(|s| s.tab)
}

/// Aba que sempre aparece no menu.
fn sempre(_extensions: &[Extension]) -> bool {
    true
}

/// A aba Extensões só existe quando o modo tem relatórios fora do RTK (que
/// fica no Consumo).
fn tem_relatorios(extensions: &[Extension]) -> bool {
    !reports(Tab::Extensions, extensions).is_empty()
}

/// Pedido de spawn de subagente recebido do servidor de controle MCP.
struct SpawnRequest {
    spawn_id: String,
    task: String,
    cwd: String,
    model: String,
}

enum Msg {
    Output(usize, Vec<u8>),
    /// Linha do stderr de uma instância no modo Middleware.
    Stderr(usize, String),
    Exited(usize),
    Loaded {
        tab: Tab,
        index: usize,
        result: Result<Text<'static>, String>,
    },
    /// Pedido de spawn de subagente vindo do MCP via servidor de controle.
    SpawnSubagent(SpawnRequest),
    /// Mensagem de outra instância (ou da UI web) para uma instância daqui.
    Message {
        instance_id: String,
        from: String,
        text: String,
    },
    /// A janela flutuante pediu para ir a uma instância daqui.
    Focus {
        instance_id: String,
    },
    /// Uma instância (pelo MCP) pediu outra neste terminal, com o id já
    /// escolhido pelo daemon e, se veio, a primeira mensagem dela.
    Open {
        instance_id: String,
        from: String,
        text: Option<String>,
    },
    /// Uma instância (pelo MCP) pediu para fechar uma daqui.
    Close {
        instance_id: String,
        from: String,
    },
    /// Consumo das instâncias lido por [`App::poll_live`], por id.
    Live(Vec<(usize, Live)>),
    /// O daemon MCP em que este terminal se registrou saiu (ou não o conhece
    /// mais) e ele se registrou de novo no daemon desta porta.
    Reconnected(u16),
}

enum Content {
    Loading,
    Ready(Text<'static>),
    Failed(String),
}

/// Estado da aba aberta quando não é a do agente.
struct View {
    tab: Tab,
    /// Sub-aba escolhida (visão do consumo).
    index: usize,
    content: Content,
    scroll: u16,
    loaded_at: Instant,
    loading: bool,
}

/// Janela por cima da tela (avisos, atalhos); qualquer tecla fecha.
struct Popup {
    title: &'static str,
    text: String,
    scroll: u16,
}

/// Seleção de texto feita com o mouse numa instância: posições
/// (linha, coluna) na tela visível do pseudoterminal dela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    agent: usize,
    anchor: (u16, u16),
    head: (u16, u16),
    /// Botão ainda pressionado.
    dragging: bool,
}

impl Selection {
    /// Início e fim, na ordem da leitura.
    fn range(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    fn contains(&self, row: u16, col: u16) -> bool {
        let ((r0, c0), (r1, c1)) = self.range();
        (r0..=r1).contains(&row) && (row != r0 || col >= c0) && (row != r1 || col <= c1)
    }

    fn text(&self, screen: &vt100::Screen) -> String {
        let ((r0, c0), (r1, c1)) = self.range();
        screen.contents_between(r0, c0, r1, c1 + 1)
    }
}

/// Seções da aba Config, na ordem da tela (`docs/tui/config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Agent,
    Budget,
    Plugins,
    Advanced,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Agent => "Agente",
            Section::Budget => "Orçamento",
            Section::Plugins => "Plugins · config global",
            Section::Advanced => "Avançado",
        }
    }

    /// Linha logo abaixo do título.
    fn note(self) -> Option<&'static str> {
        match self {
            Section::Agent => Some(
                "O agente escolhido no Router abre uma instância dele ao sair do campo ou da aba",
            ),
            Section::Budget => {
                Some("Limites de tokens (aba Consumo, quando o plano não informa os dele)")
            }
            Section::Plugins | Section::Advanced => None,
        }
    }
}

/// Campos da aba Config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// Router: o agente das próximas instâncias. A troca só vale ao sair do
    /// campo ou da aba ([`Form::leave_router`]).
    Provider,
    /// Preset de início ([`CliMode`]); no custom, as extensões aparecem
    /// embaixo como caixas.
    Mode,
    /// Caixa de uma extensão do modo custom.
    Extension(Extension),
    /// Conta do agente: nome registrado em `terraforme config --account`.
    /// Só vale no contexto de um space (grava em `preferences.toml`).
    Account,
    /// Middleware (experimental, `[tui] middleware`): as instâncias novas
    /// viram um chat da própria TUI ([`Chat`]). Sempre no `config.toml`.
    Middleware,
    /// Orçamento digitado; o diário e o de 7 dias saem dele.
    BudgetFiveHours,
    BudgetDay,
    BudgetWeek,
    /// UI web das instâncias no daemon MCP (`[mcp] ui`).
    Ui,
    /// Abre a janela flutuante com o mascote junto com o `terraforme cli`.
    Pet,
    /// O daemon MCP em si: no ar ou não.
    Mcp,
    Mouse,
    CopyOnSelect,
    McpGrace,
    McpTimeout,
    McpMax,
}

/// Caixas do modo custom, na ordem da tela.
const CUSTOM: [Extension; 3] = [Extension::AiJail, Extension::Rtk, Extension::AiMemory];

/// Presets do campo Modo, na ordem em que ele alterna (o custom por último,
/// porque abre as caixas embaixo).
const PRESETS: [CliMode; 4] = [
    CliMode::Eco,
    CliMode::High,
    CliMode::Security,
    CliMode::Custom,
];

/// Orçamento de `hours` horas no ritmo do orçamento de 5 horas.
fn budget_for(five_hours: u64, hours: u64) -> u64 {
    five_hours.saturating_mul(hours) / 5
}

/// Nome do preset e a descrição curta mostrada ao lado.
fn preset_text(preset: CliMode) -> (&'static str, &'static str) {
    match preset {
        CliMode::Eco => ("eco", "economiza tokens compactando a saída dos comandos"),
        CliMode::High => ("high", "memória entre sessões e projetos"),
        CliMode::Security => ("security", "agente dentro da sandbox, sem extensões"),
        CliMode::Custom => ("custom", "marque as extensões abaixo com Espaço"),
    }
}

/// Extensões do modo e do sandbox, para mostrar ao lado do nome do modo.
fn mode_extensions(mode: Mode, jail: bool) -> String {
    let mut names: Vec<_> = mode.extensions().iter().map(|e| e.name()).collect();
    if jail {
        names.push(Extension::AiJail.name());
    }
    if names.is_empty() {
        return "sem extensões".into();
    }
    names.join(" + ")
}

impl Field {
    fn section(self) -> Section {
        match self {
            Field::Provider
            | Field::Mode
            | Field::Extension(_)
            | Field::Account
            | Field::Middleware => Section::Agent,
            Field::BudgetFiveHours | Field::BudgetDay | Field::BudgetWeek => Section::Budget,
            Field::Ui | Field::Pet | Field::Mcp => Section::Plugins,
            Field::Mouse
            | Field::CopyOnSelect
            | Field::McpGrace
            | Field::McpTimeout
            | Field::McpMax => Section::Advanced,
        }
    }

    /// Rótulo fixo; o da conta e o das caixas dependem do formulário
    /// ([`Form::label`]).
    fn label(self) -> &'static str {
        match self {
            Field::Provider => "Router",
            Field::Mode => "Modo",
            Field::Extension(ext) => ext.name(),
            Field::Account => "Conta",
            Field::Middleware => "Middleware (Experimental)",
            Field::BudgetFiveHours => "A cada 5 horas",
            Field::BudgetDay => "Tokens diário",
            Field::BudgetWeek => "Tokens em 7 dias",
            Field::Ui => "UI",
            Field::Pet => "Mascote",
            Field::Mcp => "MCP",
            Field::Mouse => "Capturar o mouse",
            Field::CopyOnSelect => "Copiar ao selecionar",
            Field::McpGrace => "Carência ao sair (s)",
            Field::McpTimeout => "Timeout do subagente (s)",
            Field::McpMax => "Subagentes simultâneos",
        }
    }

    /// Campo editado digitando um número (os outros alternam valores).
    fn numeric(self) -> bool {
        matches!(
            self,
            Field::BudgetFiveHours | Field::McpGrace | Field::McpTimeout | Field::McpMax
        )
    }

    /// Valor dos campos que só dependem do `config.toml`.
    fn value(self, config: &Config) -> String {
        let yes = |on: bool| if on { "sim" } else { "não" }.to_string();
        let five_hours = config.budget.five_hours;
        match self {
            Field::BudgetFiveHours => five_hours.map_or("sem orçamento".into(), thousands),
            Field::BudgetDay => five_hours.map_or("—".into(), |n| {
                format!("{} (calculado)", thousands(budget_for(n, 24)))
            }),
            // Sem o de 5 horas, um semanal antigo do config.toml continua valendo.
            Field::BudgetWeek => match (five_hours, config.budget.week) {
                (Some(n), _) => format!("{} (calculado)", thousands(budget_for(n, 24 * 7))),
                (None, Some(n)) => thousands(n),
                (None, None) => "—".into(),
            },
            Field::Ui => match (config.mcp.ui, daemon()) {
                (true, Some(lock)) => {
                    format!("online · {}", terraforme_core::mcp::ui_url(&lock))
                }
                (true, None) => "online (daemon fora do ar)".into(),
                (false, _) => "offline".into(),
            },
            Field::Pet if !terraforme_core::pet::is_supported() => {
                "só no Linux e no Windows".into()
            }
            Field::Pet if config.tui.pet => "ativo".into(),
            Field::Pet => "inativo".into(),
            Field::Mcp => daemon().map_or("offline".into(), |lock| {
                format!("online · porta {}", lock.port)
            }),
            Field::Middleware => yes(config.tui.middleware),
            Field::Mouse => yes(config.tui.mouse),
            Field::CopyOnSelect => yes(config.tui.copy_on_select),
            Field::McpGrace => config.mcp.grace_seconds.to_string(),
            Field::McpTimeout => config.mcp.subagent_timeout_seconds.to_string(),
            Field::McpMax => config.mcp.max_simultaneous_subagents.to_string(),
            Field::Provider | Field::Mode | Field::Extension(_) | Field::Account => String::new(),
        }
    }

    /// Grava o número digitado; vazio tira o orçamento (ou volta ao padrão
    /// nos campos do MCP).
    fn set(self, config: &mut Config, text: &str) -> Result<(), String> {
        let n = match text.trim() {
            "" => None,
            t => Some(
                t.replace('.', "")
                    .parse::<u64>()
                    .map_err(|_| format!("número inválido: {t}"))?,
            ),
        };
        let mcp = terraforme_core::mcp::McpConfig::default();
        match self {
            Field::BudgetFiveHours => {
                config.budget.five_hours = n;
                config.budget.week = n.map(|n| budget_for(n, 24 * 7));
            }
            Field::McpGrace => config.mcp.grace_seconds = n.unwrap_or(mcp.grace_seconds),
            Field::McpTimeout => {
                config.mcp.subagent_timeout_seconds = n.unwrap_or(mcp.subagent_timeout_seconds)
            }
            Field::McpMax => {
                config.mcp.max_simultaneous_subagents =
                    n.map_or(mcp.max_simultaneous_subagents, |n| n.max(1) as usize)
            }
            _ => {}
        }
        Ok(())
    }

    /// Volta ao padrão os campos que só dependem do `config.toml`.
    fn reset(self, config: &mut Config) {
        let default = Config::default();
        match self {
            Field::BudgetFiveHours | Field::BudgetDay | Field::BudgetWeek => {
                config.budget = default.budget
            }
            Field::Ui => config.mcp.ui = default.mcp.ui,
            Field::Pet => config.tui.pet = default.tui.pet,
            Field::Middleware => config.tui.middleware = default.tui.middleware,
            Field::Mouse => config.tui.mouse = default.tui.mouse,
            Field::CopyOnSelect => config.tui.copy_on_select = default.tui.copy_on_select,
            Field::McpGrace => config.mcp.grace_seconds = default.mcp.grace_seconds,
            Field::McpTimeout => {
                config.mcp.subagent_timeout_seconds = default.mcp.subagent_timeout_seconds
            }
            Field::McpMax => {
                config.mcp.max_simultaneous_subagents = default.mcp.max_simultaneous_subagents
            }
            Field::Provider | Field::Mode | Field::Extension(_) | Field::Account | Field::Mcp => {}
        }
    }
}

/// Lock file do daemon MCP, se ele está no ar.
fn daemon() -> Option<terraforme_core::mcp::LockFile> {
    use terraforme_core::mcp::LockFile;
    LockFile::read()
        .ok()
        .flatten()
        .filter(|lock| LockFile::is_pid_alive(lock.pid))
}

/// Agentes implementados, na ordem em que a aba os alterna.
fn implemented_providers() -> Vec<Provider> {
    Provider::ALL
        .into_iter()
        .filter(|p| p.is_implemented())
        .collect()
}

/// Índice seguinte (ou anterior) da sequência, com volta ao fim.
fn step_index(len: usize, i: usize, delta: isize) -> usize {
    (i as isize + delta).rem_euclid(len as isize) as usize
}

/// Valores que a sessão vai usar: as preferências do space vencem o
/// `config.toml`, como em `Choice::resolve`.
fn with_prefs(mut config: Config, prefs: &Preferences) -> Config {
    if let Some(agent) = prefs.agent {
        config.provider = agent;
    }
    if let Some(mode) = prefs.mode {
        config.mode = mode;
    }
    if let Some(jail) = prefs.jail {
        config.jail = jail;
    }
    config
}

/// Estado da aba Config.
struct Form {
    config: Config,
    /// Nome do space quando agente, modo e conta gravam nas preferências
    /// dele (`None`: tudo fica no `config.toml`).
    space: Option<String>,
    /// Preferências do space (vazias fora dele: tudo vem do `config.toml`).
    prefs: Preferences,
    /// Nomes das contas registradas em `claude.toml`, para o campo Account ciclar.
    accounts: Vec<String>,
    /// Modo custom escolhido: as caixas das extensões ficam na tela mesmo
    /// quando a combinação marcada é a de um preset.
    custom: bool,
    /// Campo escolhido em [`Form::fields`].
    index: usize,
    /// Número sendo digitado no campo escolhido.
    editing: Option<String>,
    /// Agente de antes de mexer no Router: a troca fica pendente até sair
    /// do campo ou da aba.
    router: Option<Provider>,
    /// Resultado da última alteração (gravada ou não).
    message: Option<Result<String, String>>,
}

/// Nome do space da pasta atual e as preferências dele; `None` fora de
/// space, quando o `config.toml` é que vale.
fn space_prefs() -> Result<(Option<String>, Preferences)> {
    let cwd = std::env::current_dir().context("pasta atual")?;
    Ok(match Preferences::find(&cwd)? {
        Some((name, prefs)) => (Some(name), prefs),
        None => (None, Preferences::default()),
    })
}

impl Form {
    fn load() -> Form {
        let (config, mut message) = match Config::load() {
            Ok(config) => (config, None),
            Err(err) => (Config::default(), Some(Err(format!("{err:#}")))),
        };
        let (space, prefs) = match space_prefs() {
            Ok(found) => found,
            Err(err) => {
                if message.is_none() {
                    message = Some(Err(format!("{err:#}")));
                }
                (None, Preferences::default())
            }
        };
        let accounts = ClaudeAccounts::load()
            .map(|a| a.iter().map(|(n, _)| n.to_string()).collect())
            .unwrap_or_default();
        let mut form = Form {
            config,
            space,
            prefs,
            accounts,
            custom: false,
            index: 0,
            editing: None,
            router: None,
            message,
        };
        form.custom = form.preset() == CliMode::Custom;
        form
    }

    /// Campos na ordem da tela: as caixas do custom só com ele escolhido.
    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Provider, Field::Mode];
        if self.custom {
            fields.extend(
                CUSTOM
                    .into_iter()
                    .filter(|&e| e != Extension::AiJail || terraforme_core::jail::is_supported())
                    .map(Field::Extension),
            );
        }
        fields.extend([
            Field::Account,
            Field::Middleware,
            Field::BudgetFiveHours,
            Field::BudgetDay,
            Field::BudgetWeek,
            Field::Ui,
            Field::Pet,
            Field::Mcp,
            Field::Mouse,
            Field::CopyOnSelect,
            Field::McpGrace,
            Field::McpTimeout,
            Field::McpMax,
        ]);
        fields
    }

    fn field(&self) -> Field {
        let fields = self.fields();
        fields[self.index.min(fields.len() - 1)]
    }

    /// Agente, modo e conta gravam nas preferências do space quando há um;
    /// os outros campos são sempre do `config.toml`.
    fn in_space(&self, field: Field) -> bool {
        self.space.is_some() && field.section() == Section::Agent && field != Field::Middleware
    }

    /// Valores que a sessão vai usar (space por cima do `config.toml`).
    fn effective(&self) -> Config {
        with_prefs(self.config, &self.prefs)
    }

    /// O preset mostrado no campo Modo.
    fn preset(&self) -> CliMode {
        if self.custom {
            return CliMode::Custom;
        }
        let effective = self.effective();
        CliMode::of(effective.mode, effective.jail)
    }

    fn label(&self, field: Field) -> String {
        match field {
            Field::Account => format!("Conta {}", self.effective().provider.name()),
            Field::Extension(ext) => {
                let effective = self.effective();
                let on = match ext {
                    Extension::AiJail => effective.jail,
                    ext => effective.mode.uses(ext),
                };
                format!("  [{}] {}", if on { "x" } else { " " }, ext.name())
            }
            field => field.label().to_string(),
        }
    }

    /// Valor e, à parte, a descrição curta (em cinza) do campo.
    fn value(&self, field: Field) -> (String, Option<&'static str>) {
        let effective = self.effective();
        match field {
            Field::Provider => (effective.provider.name().into(), None),
            Field::Mode => {
                let (name, description) = preset_text(self.preset());
                let extensions = mode_extensions(effective.mode, effective.jail);
                (format!("{name} ({extensions})"), Some(description))
            }
            Field::Extension(ext) => (
                String::new(),
                Some(match ext {
                    Extension::Rtk => "economia de tokens",
                    Extension::AiMemory => "memória entre sessões",
                    Extension::AiJail => "sandbox do sistema",
                }),
            ),
            Field::Account => (self.account_display(), None),
            Field::Middleware => (
                field.value(&self.config),
                Some("chat do terraforme no lugar do terminal (Claude, Antigravity, OpenCode)"),
            ),
            field => (field.value(&self.config), None),
        }
    }

    /// Valor exibido para a conta: a das preferências do space, ou `default`
    /// quando não há nenhuma configurada.
    fn account_display(&self) -> String {
        self.prefs
            .account
            .as_deref()
            .unwrap_or("default")
            .to_string()
    }

    /// Avança (ou retrocede) a conta nas preferências do space, ciclando pela
    /// lista de contas registradas mais a opção `default` (sem conta fixa).
    fn step_account(&mut self, delta: isize) {
        if self.effective().provider != Provider::Claude {
            self.message = Some(Err("só o Claude Code tem contas registradas".into()));
            return;
        }
        if self.space.is_none() {
            self.message = Some(Err(
                "a conta vale por space — salve a pasta com `terraforme space`".into(),
            ));
            return;
        }
        if self.accounts.is_empty() {
            self.message = Some(Err(
                "nenhuma conta registrada — use `terraforme config --account NOME=PASTA`".into(),
            ));
            return;
        }
        // Opções: [None/"default", "conta1", "conta2", ...]
        let len = self.accounts.len() + 1;
        let current = self
            .prefs
            .account
            .as_deref()
            .and_then(|name| self.accounts.iter().position(|a| a == name).map(|i| i + 1))
            .unwrap_or(0);
        let next = step_index(len, current, delta);
        self.prefs.account = (next > 0).then(|| self.accounts[next - 1].clone());
        self.save_prefs();
    }

    fn save(&mut self) {
        self.message = Some(match self.config.save() {
            Ok(()) => Ok(Config::path()
                .map_or_else(|_| "salvo".into(), |p| format!("salvo em {}", p.display()))),
            Err(err) => Err(format!("{err:#}")),
        });
    }

    /// Grava as preferências do space; vazias, o `Preferences::save` apaga
    /// o arquivo e o space volta a seguir só o `config.toml`.
    fn save_prefs(&mut self) {
        let Some(name) = &self.space else {
            return;
        };
        self.message = Some(match self.prefs.save(name) {
            Ok(()) if self.prefs.is_empty() => {
                Ok("space sem preferências (vale o config.toml)".into())
            }
            Ok(()) => Ok(Preferences::path(name)
                .map_or_else(|_| "salvo".into(), |p| format!("salvo em {}", p.display()))),
            Err(err) => Err(format!("{err:#}")),
        });
    }

    /// Grava o agente: nas preferências do space quando há um, senão no
    /// `config.toml`.
    fn set_provider(&mut self, provider: Provider) {
        if self.space.is_some() {
            self.prefs.agent = Some(provider);
            self.save_prefs();
        } else {
            self.config.provider = provider;
            self.save();
        }
    }

    /// Grava o modo e o sandbox, como o agente.
    fn set_mode(&mut self, mode: Mode, jail: bool) {
        if self.space.is_some() {
            self.prefs.mode = Some(mode);
            self.prefs.jail = Some(jail);
            self.save_prefs();
        } else {
            self.config.mode = mode;
            self.config.jail = jail;
            self.save();
        }
    }

    /// Passa ao preset seguinte (ou anterior). O custom não grava nada: só
    /// abre as caixas, que partem da combinação atual.
    fn step_mode(&mut self, delta: isize) {
        let presets: Vec<_> = PRESETS
            .into_iter()
            .filter(|&p| p != CliMode::Security || terraforme_core::jail::is_supported())
            .collect();
        let i = presets
            .iter()
            .position(|&p| p == self.preset())
            .unwrap_or(0);
        let next = presets[step_index(presets.len(), i, delta)];
        self.custom = next == CliMode::Custom;
        match next.preset() {
            Some((mode, jail)) => self.set_mode(mode, jail),
            None => self.message = Some(Ok("modo custom: marque as extensões com Espaço".into())),
        }
    }

    /// Marca ou desmarca a extensão no modo custom.
    fn toggle_extension(&mut self, ext: Extension) {
        let effective = self.effective();
        let (mut rtk, mut memory, mut jail) = (
            effective.mode.uses(Extension::Rtk),
            effective.mode.uses(Extension::AiMemory),
            effective.jail,
        );
        match ext {
            Extension::Rtk => rtk = !rtk,
            Extension::AiMemory => memory = !memory,
            Extension::AiJail => jail = !jail,
        }
        self.set_mode(Mode::with(rtk, memory), jail);
    }

    /// Liga ou desliga o mascote e já abre (ou fecha) a janela.
    fn toggle_pet(&mut self) {
        use terraforme_core::pet;
        if !pet::is_supported() {
            self.message = Some(Err(
                "a janela flutuante só existe no Linux e no Windows por enquanto (ROADMAP.md)"
                    .into(),
            ));
            return;
        }
        self.config.tui.pet = !self.config.tui.pet;
        self.save();
        let result = if self.config.tui.pet {
            pet::start().map(|_| "mascote aberto")
        } else {
            pet::stop().map(|_| "mascote fechado")
        };
        match result {
            Ok(text) => self.message = Some(Ok(text.into())),
            Err(err) => self.message = Some(Err(format!("{err:#}"))),
        }
    }

    /// Sobe o daemon MCP se ele está fora do ar; no ar, ele fecha sozinho
    /// quando o último terminal sai.
    fn start_mcp(&mut self) {
        self.message = Some(match daemon() {
            Some(lock) => Ok(format!(
                "daemon MCP no ar (porta {}); ele fecha sozinho quando o último terminal sai",
                lock.port
            )),
            None => terraforme_core::mcp::ensure_running(&self.config.mcp)
                .map(|lock| format!("daemon MCP no ar na porta {}", lock.port))
                .map_err(|err| format!("{err:#}")),
        });
    }

    /// Abre a UI web no navegador, se ela está ligada e o daemon no ar.
    fn open_ui(&mut self) {
        let Some(lock) = daemon().filter(|_| self.config.mcp.ui) else {
            self.message = Some(Err("a UI está offline".into()));
            return;
        };
        let url = terraforme_core::mcp::ui_url(&lock);
        self.message = Some(
            terraforme_core::platform::open_url(&url)
                .map(|()| format!("abrindo {url}"))
                .map_err(|err| format!("abrindo {url}: {err}")),
        );
    }

    /// Alterna o valor do campo; os do agente gravam nas preferências do
    /// space quando há um, os outros no `config.toml`.
    fn step(&mut self, field: Field, delta: isize) {
        match field {
            Field::Provider => {
                let providers = implemented_providers();
                let current = self.effective().provider;
                let i = providers.iter().position(|&p| p == current).unwrap_or(0);
                self.set_provider(providers[step_index(providers.len(), i, delta)]);
            }
            Field::Mode => self.step_mode(delta),
            Field::Extension(ext) => self.toggle_extension(ext),
            Field::Account => self.step_account(delta),
            Field::BudgetDay | Field::BudgetWeek => {
                self.message = Some(Err(
                    "calculado a partir do orçamento de 5 horas — edite \"A cada 5 horas\"".into(),
                ))
            }
            Field::Ui => {
                self.config.mcp.ui = !self.config.mcp.ui;
                self.save();
            }
            Field::Pet => self.toggle_pet(),
            Field::Middleware => {
                self.config.tui.middleware = !self.config.tui.middleware;
                self.save();
            }
            Field::Mcp => self.start_mcp(),
            Field::Mouse => {
                self.config.tui.mouse = !self.config.tui.mouse;
                self.save();
            }
            Field::CopyOnSelect => {
                self.config.tui.copy_on_select = !self.config.tui.copy_on_select;
                self.save();
            }
            Field::BudgetFiveHours | Field::McpGrace | Field::McpTimeout | Field::McpMax => {}
        }
    }

    /// Tira o campo do seu alvo: nas preferências do space, remove o
    /// override (volta a valer o `config.toml`); nos outros, volta ao
    /// padrão.
    fn reset(&mut self, field: Field) {
        if self.in_space(field) {
            match field {
                Field::Provider => self.prefs.agent = None,
                Field::Mode | Field::Extension(_) => {
                    self.prefs.mode = None;
                    self.prefs.jail = None;
                }
                Field::Account => self.prefs.account = None,
                _ => {}
            }
            self.save_prefs();
        } else {
            let default = Config::default();
            match field {
                Field::Provider => self.config.provider = default.provider,
                Field::Mode | Field::Extension(_) => {
                    self.config.mode = default.mode;
                    self.config.jail = default.jail;
                }
                field => field.reset(&mut self.config),
            }
            self.save();
        }
        if field.section() == Section::Agent {
            self.custom =
                CliMode::of(self.effective().mode, self.effective().jail) == CliMode::Custom;
        }
    }

    /// Escolhe o campo da linha `line` do texto (clique do mouse); devolve
    /// o campo, se a linha tem um.
    fn click(&mut self, line: usize) -> Option<Field> {
        if self.editing.is_some() {
            return None;
        }
        let field = self.rows().into_iter().nth(line)?.0?;
        self.index = self.fields().iter().position(|&f| f == field)?;
        Some(field)
    }

    /// Sai do Router (ou da aba): devolve `true` se o agente escolhido nele
    /// é outro e a TUI deve abrir uma instância dele.
    fn leave_router(&mut self) -> bool {
        self.router
            .take()
            .is_some_and(|before| before != self.effective().provider)
    }

    /// Trata a tecla; devolve o campo alterado e gravado, se houver. O
    /// Router grava na hora, mas só devolve [`Field::Provider`] quando o
    /// campo escolhido deixa de ser ele.
    fn key(&mut self, key: KeyEvent) -> Option<Field> {
        let provider = self.effective().provider;
        let changed = self.handle(key);
        if changed == Some(Field::Provider) {
            self.router.get_or_insert(provider);
            return None;
        }
        if self.field() != Field::Provider && self.leave_router() {
            return Some(Field::Provider);
        }
        changed
    }

    fn handle(&mut self, key: KeyEvent) -> Option<Field> {
        let field = self.field();
        let last = self.fields().len() - 1;
        if let Some(text) = &mut self.editing {
            match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() => text.push(c),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Enter => {
                    let text = self.editing.take().unwrap_or_default();
                    return match field.set(&mut self.config, &text) {
                        Ok(()) => {
                            self.save();
                            Some(field)
                        }
                        Err(err) => {
                            self.message = Some(Err(err));
                            None
                        }
                    };
                }
                KeyCode::Esc => self.editing = None,
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.index = self.index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.index = (self.index + 1).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.index = 0,
            KeyCode::End | KeyCode::Char('G') => self.index = last,
            KeyCode::Enter if field.numeric() => self.editing = Some(String::new()),
            KeyCode::Char(c) if field.numeric() && c.is_ascii_digit() => {
                self.editing = Some(c.to_string())
            }
            KeyCode::Char('o') if field == Field::Ui => self.open_ui(),
            KeyCode::Left | KeyCode::Char('h') if !field.numeric() => {
                self.step(field, -1);
                return Some(field);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter
                if !field.numeric() =>
            {
                self.step(field, 1);
                return Some(field);
            }
            KeyCode::Delete | KeyCode::Char('d') => {
                self.reset(field);
                return Some(field);
            }
            KeyCode::Char('r') => {
                let router = self.router;
                *self = Form::load();
                self.router = router;
            }
            _ => {}
        }
        None
    }

    /// Linhas da tela, cada uma com o campo dela (para o clique).
    fn rows(&self) -> Vec<(Option<Field>, Line<'static>)> {
        let fields = self.fields();
        let current = self.field();
        let mut rows = Vec::new();
        for (i, &field) in fields.iter().enumerate() {
            let section = field.section();
            if i == 0 || fields[i - 1].section() != section {
                if i > 0 {
                    rows.push((None, Line::default()));
                }
                let title = format!("── {} ", section.title());
                let rule = "─".repeat(48usize.saturating_sub(title.chars().count()));
                rows.push((None, Line::raw(format!("{title}{rule}")).bold().cyan()));
                if let Some(note) = section.note() {
                    rows.push((None, Line::raw(format!("   {note}")).dark_gray()));
                }
            }
            let selected = field == current;
            let (value, description) = match (&self.editing, selected) {
                (Some(text), true) if field == Field::BudgetFiveHours => {
                    let masked = text.parse().map_or_else(|_| text.clone(), thousands);
                    (format!("{masked}▏"), Some("Enter grava · Esc cancela"))
                }
                (Some(text), true) => (format!("{text}▏"), Some("Enter grava · Esc cancela")),
                _ => self.value(field),
            };
            let style = if selected {
                Style::new().black().on_cyan().bold()
            } else {
                Style::new()
            };
            let gap = if value.is_empty() { "" } else { "  " };
            let mut spans = vec![
                Span::raw(if selected { " › " } else { "   " }),
                Span::styled(format!("{:<26}", self.label(field)), style),
                Span::raw("  "),
                Span::raw(value).cyan(),
            ];
            if let Some(description) = description {
                spans.push(Span::raw(format!("{gap}{description}")).dark_gray());
            }
            rows.push((Some(field), Line::from(spans)));
        }
        rows.push((None, Line::default()));
        match &self.message {
            Some(Ok(text)) => rows.push((None, Line::raw(format!("✔ {text}")).green())),
            Some(Err(err)) => rows.push((None, Line::raw(format!("✘ {err}")).red())),
            None => {}
        }
        let footer = match &self.space {
            Some(name) => format!(
                "Agente, modo e conta valem em {} (preferências do space {name}); \
                 os outros campos valem no config.toml. O modo e a conta valem nas \
                 próximas instâncias (⌃⌥N).",
                Preferences::path(name)
                    .map_or_else(|_| "preferences.toml".into(), |p| p.display().to_string())
            ),
            None => "As preferências do space têm prioridade sobre o agente e o modo; \
                     a conta só vale dentro de um space. O modo vale nas próximas \
                     instâncias (⌃⌥N)."
                .to_string(),
        };
        rows.push((None, Line::raw(footer).dark_gray()));
        rows
    }

    fn text(&self) -> Text<'static> {
        Text::from(
            self.rows()
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>(),
        )
    }
}

/// Avisos do agente vistos na saída: o sino (`BEL`) e as notificações de
/// terminal (OSC 9, menos o progresso `9;4`, e OSC 777), que marcam a
/// instância como "precisa de você" na janela flutuante.
#[derive(Default)]
struct Signals {
    bell: bool,
}

impl vt100::Callbacks for Signals {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params {
            [b"9", b"4", ..] => {}
            [b"9", ..] | [b"777", ..] => self.bell = true,
            _ => {}
        }
    }
}

/// Uma instância do agente, no próprio pseudoterminal.
struct Agent {
    /// Número da sub-aba; não é reaproveitado quando a instância fecha.
    id: usize,
    /// Nome do agente (ex.: `claude`): a aba Config pode trocar o agente das
    /// instâncias novas enquanto as antigas continuam abertas.
    name: String,
    adapter: &'static dyn Adapter,
    parser: vt100::Parser<Signals>,
    /// Pseudoterminal; `None` no modo Middleware, em que o agente roda em
    /// pipes e o [`Chat`] desenha a conversa.
    master: Option<Box<dyn MasterPty + Send>>,
    /// Entrada do agente: o pseudoterminal ou, no Middleware, o stdin.
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Conversa do modo Middleware.
    chat: Option<Chat>,
    /// Linhas roladas para trás no histórico (0 = ao vivo).
    scrollback: usize,
    launched: SystemTime,
    /// Id de sessão passado ao agente.
    session: Option<String>,
    /// Instruções a entregar ao agente logo que ele inicializar.
    pending: Option<String>,
    /// Momento da abertura, para o atraso de entregar o prompt.
    booted: Instant,
    /// Última saída recebida do agente.
    last_output: Instant,
    /// Quando mandar o Enter do prompt já colado ([`SUBMIT_DELAY`]).
    submit_at: Option<Instant>,
    /// Id do spawn MCP (Some = este é um subagente MCP).
    spawn_id: Option<String>,
    /// Saída acumulada do subagente para devolver ao MCP.
    output_buf: Vec<u8>,
    /// Porta do MCP para enviar o resultado quando terminar.
    mcp_port: Option<u16>,
    /// Id da instância no registro do MCP (`None`: não registrada).
    instance_id: Option<String>,
    /// Corpo do `/instances/register`, reenviado quando o terminal se
    /// registra num daemon novo ([`Agent::register`]).
    registration: Option<String>,
    /// Quando o processo foi visto encerrado por [`Agent::exited`].
    exited_at: Option<Instant>,
    /// Atividade da instância, informada ao daemon para a janela flutuante.
    activity: Tracker,
    /// Consumo da sessão e do prompt atual, para o rodapé.
    live: Option<Live>,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        id: usize,
        name: &str,
        adapter: &'static dyn Adapter,
        command: &Command,
        cwd: &Path,
        body: Rect,
        session_ids: bool,
        link: Option<&Link>,
        instance_id: Option<String>,
        middleware: bool,
        tx: &Sender<Msg>,
    ) -> Result<Agent> {
        let mut extra: Vec<std::ffi::OsString> = Vec::new();
        let mut session = None;
        if session_ids {
            let id = uuid::Uuid::new_v4().to_string();
            if let Some(args) = adapter.session_args(&id) {
                extra.extend(args.into_iter().map(Into::into));
                session = Some(id);
            }
        }
        // O id vem do daemon quando foi ele quem pediu a instância.
        let instance_id =
            link.map(|_| instance_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
        if let (Some(link), Some(instance_id)) = (link, &instance_id) {
            let url = format!("http://127.0.0.1:{}/", link.port);
            let headers = [
                ("x-client-id", link.client_id.as_str()),
                ("x-instance-id", instance_id.as_str()),
            ];
            if let Some(args) = adapter.mcp_args(&url, &headers) {
                extra.extend(args);
            }
        }
        let chat_args = adapter.chat_args().filter(|_| middleware);
        let launched = SystemTime::now();
        let (master, writer, child, mut reader): (
            _,
            Box<dyn Write + Send>,
            _,
            Box<dyn Read + Send>,
        ) = match chat_args {
            Some(args) => {
                let mut child = piped(command, cwd, adapter.chat_drop_args())
                    .args(extra)
                    .args(args)
                    .spawn()
                    .context("falha ao iniciar o agente")?;
                let stdin = child.stdin.take().context("stdin do agente")?;
                let stdout = child.stdout.take().context("stdout do agente")?;
                if let Some(stderr) = child.stderr.take() {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(stderr).lines() {
                            let Ok(line) = line else { break };
                            if tx.send(Msg::Stderr(id, line)).is_err() {
                                return;
                            }
                        }
                    });
                }
                let child: Box<dyn Child + Send + Sync> = Box::new(child);
                (None, Box::new(stdin), child, Box::new(stdout))
            }
            None => {
                let pty = native_pty_system()
                    .openpty(pty_size(body))
                    .map_err(|e| anyhow!("{e}"))
                    .context("falha ao abrir o pseudoterminal")?;
                let mut builder = builder(command, cwd);
                builder.args(extra);
                let child = pty
                    .slave
                    .spawn_command(builder)
                    .map_err(|e| anyhow!("{e}"))
                    .context("falha ao iniciar o agente")?;
                drop(pty.slave);
                let reader = pty.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
                let writer = pty.master.take_writer().map_err(|e| anyhow!("{e}"))?;
                (Some(pty.master), writer, child, reader)
            }
        };

        let tx = tx.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Msg::Output(id, buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Exited(id));
        });

        let registration = link.zip(instance_id.as_ref()).map(|(link, instance_id)| {
            serde_json::json!({
                "instance_id": instance_id,
                "client_id": link.client_id,
                "control_addr": link.control_addr,
                "agent": name,
                "number": id,
                "space": link.space,
                "cwd": cwd.display().to_string(),
            })
            .to_string()
        });

        let agent = Agent {
            id,
            name: name.to_string(),
            adapter,
            parser: vt100::Parser::new_with_callbacks(
                body.height,
                body.width,
                SCROLLBACK,
                Signals::default(),
            ),
            chat: master
                .is_none()
                .then(|| Chat::new(name, adapter, cwd, &effort_from_command(command))),
            master,
            writer,
            child,
            scrollback: 0,
            launched,
            session,
            pending: None,
            booted: Instant::now(),
            last_output: Instant::now(),
            submit_at: None,
            spawn_id: None,
            output_buf: Vec::new(),
            mcp_port: link.map(|l| l.port),
            instance_id,
            registration,
            exited_at: None,
            activity: Tracker::new(Instant::now()),
            live: None,
        };
        agent.register(false);
        let mut agent = agent;
        agent.flush_chat();
        Ok(agent)
    }

    /// Manda ao agente as linhas que o [`Chat`] pôs na fila (mensagens,
    /// respostas de permissão, pedidos de controle).
    fn flush_chat(&mut self) {
        let Some(chat) = &mut self.chat else {
            return;
        };
        for mut line in chat.outbox() {
            line.push('\n');
            if self.writer.write_all(line.as_bytes()).is_err() {
                return;
            }
        }
        let _ = self.writer.flush();
    }

    /// Registra a instância no daemon de [`Agent::mcp_port`]; com `state`,
    /// informa também a atividade atual (o daemon é novo e não a conhece).
    fn register(&self, state: bool) {
        let (Some(port), Some(body), Some(instance_id)) = (
            self.mcp_port,
            self.registration.clone(),
            self.instance_id.clone(),
        ) else {
            return;
        };
        let activity = self.activity.state();
        thread::spawn(move || {
            let addr = format!("127.0.0.1:{port}");
            http_post_fire(&addr, "/instances/register", &body);
            if state {
                let body = serde_json::json!({ "instance_id": instance_id, "activity": activity });
                http_post_fire(&addr, "/instances/state", &body.to_string());
            }
        });
    }

    /// O que o usuário digita, cola ou rola na instância: também marca o
    /// aviso da janela flutuante como visto.
    fn input(&mut self, bytes: &[u8]) -> Result<()> {
        let change = self.activity.input(Instant::now());
        self.report(change);
        self.send(bytes)
    }

    /// Processa a saída do agente e atualiza a atividade.
    fn output(&mut self, bytes: &[u8]) {
        let now = Instant::now();
        self.last_output = now;
        if let Some(chat) = &mut self.chat {
            // Fim da resposta ou pedido de permissão: o agente espera o usuário.
            let change = if chat.output(bytes) {
                self.activity.bell(now)
            } else {
                self.activity.output(now)
            };
            self.report(change);
            self.flush_chat();
            return;
        }
        self.parser.process(bytes);
        let mut change = self.activity.output(now);
        if std::mem::take(&mut self.parser.callbacks_mut().bell) {
            change = self.activity.bell(now).or(change);
        }
        self.report(change);
    }

    /// Informa ao daemon MCP a mudança de atividade, se houve uma.
    fn report(&self, change: Option<Activity>) {
        let (Some(activity), Some(port), Some(instance_id)) =
            (change, self.mcp_port, &self.instance_id)
        else {
            return;
        };
        let body = serde_json::json!({ "instance_id": instance_id, "activity": activity });
        let addr = format!("127.0.0.1:{port}");
        thread::spawn(move || http_post_fire(&addr, "/instances/state", &body.to_string()));
    }

    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// `true` quando o processo já terminou há [`EXIT_GRACE`]. No Windows a
    /// leitura do ConPTY não chega ao fim quando o agente sai (o
    /// pseudoterminal continua aberto), então a saída é vista por aqui; a
    /// espera deixa chegar o resto da saída (o resultado de um subagente).
    fn exited(&mut self) -> bool {
        if self.exited_at.is_none() && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited_at = Some(Instant::now());
        }
        self.exited_at.is_some_and(|at| at.elapsed() >= EXIT_GRACE)
    }

    /// Responde aos pedidos de posição do cursor (`ESC[6n`) na saída do
    /// agente, como faria um terminal. No Windows o ConPTY manda um ao
    /// iniciar e trava o agente até receber a resposta.
    fn answer_cursor_queries(&mut self, bytes: &[u8]) {
        let queries = cursor_queries(bytes);
        if queries == 0 || self.chat.is_some() {
            return;
        }
        let (row, col) = self.parser.screen().cursor_position();
        let reply = format!("\x1b[{};{}R", row + 1, col + 1).repeat(queries);
        let _ = self.send(reply.as_bytes());
    }

    /// Põe uma mensagem na fila do prompt pendente (entregue por
    /// [`Agent::deliver_pending`]).
    fn queue(&mut self, text: &str) {
        self.pending = Some(match self.pending.take() {
            Some(prev) => format!("{prev}\n\n{text}"),
            None => text.to_string(),
        });
        self.deliver_pending();
    }

    /// Entrega o prompt pendente quando o agente está pronto: passou o
    /// [`PROMPT_DELAY`], ele ligou o bracketed paste (ou passou o
    /// [`PROMPT_READY_TIMEOUT`]) e a saída está parada há [`PROMPT_QUIET`].
    /// O texto é colado e o Enter vai depois, em [`SUBMIT_DELAY`].
    fn deliver_pending(&mut self) {
        if let Some(at) = self.submit_at
            && Instant::now() >= at
        {
            self.submit_at = None;
            let _ = self.send(b"\r");
        }
        let Some(prompt) = self.pending.clone() else {
            return;
        };
        // No Middleware o agente lê a entrada desde o início.
        if let Some(chat) = &mut self.chat {
            chat.message(&prompt);
            self.pending = None;
            self.flush_chat();
            return;
        }
        let bracketed = self.parser.screen().bracketed_paste();
        let ready = bracketed || self.booted.elapsed() >= PROMPT_READY_TIMEOUT;
        if self.submit_at.is_some()
            || self.booted.elapsed() < PROMPT_DELAY
            || !ready
            || self.last_output.elapsed() < PROMPT_QUIET
        {
            return;
        }
        self.pending = None;
        if self.send(&prompt_bytes(&prompt, bracketed)).is_err() {
            // Tenta de novo na próxima volta.
            self.pending = Some(prompt);
            return;
        }
        self.submit_at = Some(Instant::now() + SUBMIT_DELAY);
    }

    /// `isize::MIN` volta ao vivo.
    fn scroll_history(&mut self, delta: isize) {
        self.scrollback = if delta == isize::MIN {
            0
        } else {
            self.scrollback.saturating_add_signed(delta)
        };
        self.parser.screen_mut().set_scrollback(self.scrollback);
        // O vt100 limita ao histórico existente.
        self.scrollback = self.parser.screen().scrollback();
    }

    fn resize(&mut self, body: Rect) -> Result<()> {
        self.parser.screen_mut().set_size(body.height, body.width);
        match &self.master {
            Some(master) => master.resize(pty_size(body)).map_err(|e| anyhow!("{e}")),
            None => Ok(()),
        }
    }

    /// Tecla na conversa do Middleware: edita a entrada e, no `Enter`, manda
    /// a mensagem ao agente.
    fn chat_key(&mut self, key: KeyEvent) -> Result<()> {
        let change = self.activity.input(Instant::now());
        self.report(change);
        if let Some(chat) = &mut self.chat {
            chat.key(key);
        }
        self.flush_chat();
        Ok(())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let (Some(port), Some(instance_id)) = (self.mcp_port, &self.instance_id) {
            let body = serde_json::json!({ "instance_id": instance_id }).to_string();
            http_post_fire(&format!("127.0.0.1:{port}"), "/instances/unregister", &body);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct App {
    agent: String,
    title: String,
    extensions: Vec<Extension>,
    adapter: &'static dyn Adapter,
    command: Command,
    relaunch: Relaunch,
    /// A aba Config mudou o modo ou a conta: a próxima instância resolve o
    /// agente de novo antes de abrir.
    stale: bool,
    cwd: PathBuf,
    session_ids: bool,
    agents: Vec<Agent>,
    next_id: usize,
    active: Tab,
    /// Sub-aba (instância) aberta na aba do agente; é a sessão mostrada no
    /// consumo.
    focused: usize,
    view: Option<View>,
    /// Estado da aba Config, enquanto ela está aberta.
    form: Option<Form>,
    popup: Option<Popup>,
    /// Área do agente (a tela menos o menu e as sub-abas).
    body: Rect,
    tx: Sender<Msg>,
    quit: bool,
    /// Porta do daemon MCP para envio de resultados de subagentes.
    mcp_port: Option<u16>,
    /// Registro das instâncias no MCP; `None` sem o daemon.
    link: Option<Link>,
    /// Protocolo de teclado do kitty ligado: `Ctrl+Shift`/`Ctrl+Alt` chegam
    /// distintos e `Shift+Tab` pode seguir para o agente.
    enhanced: bool,
    /// A TUI captura o mouse (roda e seleção próprias).
    mouse: bool,
    /// Copia a seleção ao soltar o botão do mouse.
    copy_on_select: bool,
    /// Middleware (experimental): as instâncias novas viram um chat
    /// ([`Chat`]) em vez do terminal do agente.
    middleware: bool,
    selection: Option<Selection>,
    /// Mensagem curta no canto da tela, até [`STATUS_TTL`].
    status: Option<(String, Instant)>,
    /// Space da pasta (vazio fora de space), gravado com o consumo.
    space: String,
    /// Última gravação do consumo no histórico ([`RECORD_EVERY`]).
    recorded_at: Instant,
    /// Última leitura do consumo para o rodapé ([`LIVE_EVERY`]) e se ela
    /// ainda está em andamento.
    live_at: Instant,
    live_busy: bool,
    /// Tokens gastos por instância desde que abriu, inclusive as já
    /// fechadas: a soma é o consumo desta sessão do terraforme.
    spent: BTreeMap<usize, u64>,
}

/// Abre a TUI com o agente dentro e só volta quando todas as instâncias
/// terminam (ou F10).
pub fn run(options: Options) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().context("tamanho do terminal")?;
    let cwd = match options.launched.command.get_current_dir() {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let (tx, rx) = mpsc::channel();

    let mcp_port = options.mcp_lock.as_ref().map(|l| l.port);
    let client_id = options.client_id.clone();
    let config = Config::load().unwrap_or_default();
    let tui = config.tui;

    // Servidor de controle e registro no MCP, antes das instâncias, que se
    // registram com o endereço dele.
    let mut link = None;
    if let Some(mcp) = mcp_port {
        let control_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let control_port = control_listener.local_addr()?.port();
        let control_addr = format!("127.0.0.1:{control_port}");

        let reg_body = serde_json::json!({
            "client_id": client_id,
            "pid": std::process::id(),
            "control_addr": control_addr,
        });
        http_post_fire(
            &format!("127.0.0.1:{mcp}"),
            "/register",
            &reg_body.to_string(),
        );

        // Heartbeat a cada 2s; se o daemon saiu (ex.: `terraforme restart`
        // depois de um upgrade) ou não conhece mais este terminal, sobe ou
        // acha o daemon no ar e se registra de novo nele.
        let hb_client = client_id.clone();
        let hb_tx = tx.clone();
        let mcp_config = config.mcp;
        thread::spawn(move || {
            let body = serde_json::json!({ "client_id": hb_client }).to_string();
            let mut port = mcp;
            loop {
                thread::sleep(Duration::from_secs(2));
                let status = http_post(&format!("127.0.0.1:{port}"), "/heartbeat", &body);
                if status.is_some_and(|code| code != 404) {
                    continue;
                }
                let Ok(lock) = terraforme_core::mcp::ensure_running(&mcp_config) else {
                    continue;
                };
                http_post_fire(
                    &format!("127.0.0.1:{}", lock.port),
                    "/register",
                    &reg_body.to_string(),
                );
                port = lock.port;
                if hb_tx.send(Msg::Reconnected(port)).is_err() {
                    return;
                }
            }
        });

        // Recebe `POST /spawn` e `POST /message` do MCP.
        let ctrl_tx = tx.clone();
        thread::spawn(move || {
            control_server(control_listener, ctrl_tx);
        });

        link = Some(Link {
            port: mcp,
            client_id: client_id.clone(),
            control_addr,
            space: options.space.clone().unwrap_or_default(),
        });
    }

    let launched = options.launched;
    let mut app = App {
        agent: launched.agent,
        title: launched.title,
        extensions: launched.extensions,
        adapter: launched.adapter,
        command: launched.command,
        relaunch: options.relaunch,
        stale: false,
        cwd,
        session_ids: launched.session_ids,
        agents: Vec::new(),
        next_id: 1,
        active: Tab::Agent,
        focused: 1,
        view: None,
        form: None,
        popup: (!launched.notices.is_empty()).then(|| notices(launched.notices.join("\n"))),
        body: body_size(Rect::new(0, 0, cols, rows)),
        tx: tx.clone(),
        quit: false,
        mcp_port,
        link,
        enhanced: false,
        mouse: tui.mouse,
        copy_on_select: tui.copy_on_select,
        middleware: tui.middleware,
        selection: None,
        status: None,
        space: options.space.unwrap_or_default(),
        recorded_at: Instant::now(),
        live_at: Instant::now(),
        live_busy: false,
        spent: BTreeMap::new(),
    };
    for _ in 0..options.instances.clamp(1, MAX_INSTANCES) {
        app.spawn()?;
    }

    if config.mcp.ui
        && let Some(lock) = &options.mcp_lock
    {
        app.flash(format!(
            "UI das instâncias: {}",
            terraforme_core::mcp::ui_url(lock)
        ));
    }

    let mut terminal = ratatui::init();
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, EnableBracketedPaste);
    if app.mouse {
        let _ = execute!(stdout, EnableMouseCapture);
    }
    app.enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
        && execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok();
    let result = app.event_loop(&mut terminal, &rx);
    if app.enhanced {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout, DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();

    // Grava o consumo de quem ainda está aberto antes de encerrar.
    app.record_usage(None, true);
    // Encerra as instâncias que ainda estiverem rodando.

    // Desregistra do MCP (do daemon atual, que pode ter mudado).
    let mcp_port = app.mcp_port;
    drop(app);
    if let Some(mcp) = mcp_port {
        let body = format!(r#"{{"client_id":"{}"}}"#, client_id);
        http_post_fire(&format!("127.0.0.1:{mcp}"), "/unregister", &body);
    }

    result
}

/// Abas na ordem do menu: as entradas do Registro [`TABS`] que o modo abre
/// (o agente, o consumo, as extensões — se o modo usa alguma além do RTK,
/// que fica no consumo — e a configuração).
fn tabs(extensions: &[Extension]) -> Vec<Tab> {
    TABS.iter()
        .filter(|spec| (spec.show)(extensions))
        .map(|spec| spec.tab)
        .collect()
}

/// Janela de avisos.
fn notices(text: String) -> Popup {
    Popup {
        title: " Avisos ",
        text,
        scroll: 0,
    }
}

impl App {
    fn tabs(&self) -> Vec<Tab> {
        tabs(&self.extensions)
    }

    /// Abre mais uma instância do agente e devolve o número dela.
    fn spawn(&mut self) -> Result<usize> {
        self.spawn_as(None)
    }

    /// Como [`App::spawn`], com o id da instância no daemon MCP já escolhido.
    fn spawn_as(&mut self, instance_id: Option<String>) -> Result<usize> {
        let id = self.next_id;
        let agent = Agent::spawn(
            id,
            &self.agent,
            self.adapter,
            &self.command,
            &self.cwd,
            self.body,
            self.session_ids,
            self.link.as_ref(),
            instance_id,
            self.middleware,
            &self.tx,
        )?;
        if self.middleware && agent.chat.is_none() {
            self.flash(format!(
                "Middleware ainda não suportado em {}: terminal",
                self.agent
            ));
        }
        self.agents.push(agent);
        self.next_id += 1;
        Ok(id)
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.popup = Some(notices(text.into()));
    }

    /// Mensagem curta no canto da tela, que some sozinha.
    fn flash(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn new_instance(&mut self) {
        if self.agents.len() >= MAX_INSTANCES {
            self.notify(format!("⚠ no máximo {MAX_INSTANCES} instâncias"));
            return;
        }
        if self.stale && !self.reload() {
            return;
        }
        match self.spawn() {
            Ok(id) => self.focus(id),
            Err(err) => self.notify(format!("⚠ {err:#}")),
        }
    }

    /// Resolve o agente de novo com o que a aba Config gravou (agente, modo
    /// e sandbox do space ou do `config.toml`); as instâncias abertas
    /// continuam como estão. Devolve `false` (com o erro na tela) se falhou.
    fn reload(&mut self) -> bool {
        let resolved = Config::load().and_then(|config| {
            let (_, prefs) = space_prefs()?;
            let config = with_prefs(config, &prefs);
            let jail = config.jail && terraforme_core::jail::is_supported();
            (self.relaunch)(config.provider, config.mode, jail)
        });
        let launched = match resolved {
            Ok(launched) => launched,
            Err(err) => {
                self.notify(format!("⚠ {err:#}"));
                return false;
            }
        };
        self.agent = launched.agent;
        self.title = launched.title;
        self.adapter = launched.adapter;
        self.command = launched.command;
        self.session_ids = launched.session_ids;
        self.extensions = launched.extensions;
        self.stale = false;
        if !launched.notices.is_empty() {
            self.notify(launched.notices.join("\n"));
        }
        true
    }

    /// O Router da aba Config trocou o agente: abre uma instância do novo,
    /// sem fechar as que estão rodando.
    fn switch_agent(&mut self) {
        self.stale = true;
        let before = self.agents.len();
        self.new_instance();
        if self.agents.len() > before {
            self.flash(format!("agente trocado: {}", self.agent));
        }
    }

    /// Abre a instância que outra pediu pelo MCP (`open_instance`,
    /// `delegate_task`), sem tirar o foco de onde o usuário está; sem ela,
    /// o daemon avisa quem pediu quando o registro não chega.
    fn open_requested(&mut self, instance_id: String, from: &str, text: Option<String>) {
        if self.agents.len() >= MAX_INSTANCES {
            self.flash(format!(
                "⚠ {from} pediu uma instância: no máximo {MAX_INSTANCES}"
            ));
            return;
        }
        if self.stale && !self.reload() {
            return;
        }
        match self.spawn_as(Some(instance_id)) {
            Ok(id) => {
                if let (Some(text), Some(agent)) = (text, self.agent_mut(id)) {
                    agent.queue(&text);
                }
                self.flash(format!("＋ {} {id} aberto por {from}", self.agent));
            }
            Err(err) => self.notify(format!("⚠ instância pedida por {from}: {err:#}")),
        }
    }

    /// Abre uma instância de subagente para um pedido MCP.
    fn spawn_subagent(&mut self, req: SpawnRequest) {
        if self.agents.len() >= MAX_INSTANCES {
            if let Some(port) = self.mcp_port {
                send_spawn_result(
                    port,
                    &req.spawn_id,
                    &format!("⚠ no máximo {MAX_INSTANCES} instâncias"),
                    1,
                );
            }
            return;
        }
        let cwd = if req.cwd.is_empty() {
            self.cwd.clone()
        } else {
            PathBuf::from(&req.cwd)
        };
        let id = self.next_id;
        // O caminho completo que o `prepare` achou (fora do `PATH` ou um
        // `.cmd` no Windows); com o AI Jail o programa é o `ai-jail`, e aí
        // fica o nome do agente.
        let program = self.command.get_program();
        let bin = if Path::new(program)
            .file_stem()
            .is_some_and(|stem| stem == self.adapter.bin())
        {
            program
        } else {
            self.adapter.bin().as_ref()
        };
        let mut command = Command::new(bin);
        command.arg("--print");
        command.arg("--dangerously-skip-permissions");
        command.arg("-p");
        command.arg(&req.task);
        if !req.model.is_empty() && req.model != "claude-haiku-4-5" {
            command.arg("--model").arg(&req.model);
        }
        // Subagente roda com `--print`: não conversa, então não se registra.
        match Agent::spawn(
            id,
            &self.agent,
            self.adapter,
            &command,
            &cwd,
            self.body,
            false,
            None,
            None,
            false,
            &self.tx,
        ) {
            Ok(mut agent) => {
                agent.spawn_id = Some(req.spawn_id);
                agent.mcp_port = self.mcp_port;
                self.agents.push(agent);
                self.next_id += 1;
                self.focus(id);
                self.notify("ℹ subagente MCP aberto · ⌃⌥W fecha");
            }
            Err(err) => {
                if let Some(port) = self.mcp_port {
                    send_spawn_result(port, &req.spawn_id, &err.to_string(), 1);
                }
                self.notify(format!("⚠ subagente: {err:#}"));
            }
        }
    }

    fn agent(&self, id: usize) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    fn agent_mut(&mut self, id: usize) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Instância da sub-aba aberta, se a aba aberta for a do agente (o
    /// `kind` dela no Registro).
    fn active_agent(&mut self) -> Option<&mut Agent> {
        match spec(self.active).kind {
            TabKind::Agent => self.agent_mut(self.focused),
            _ => None,
        }
    }

    /// Abre a aba do agente na sub-aba da instância.
    fn focus(&mut self, id: usize) {
        if let Some(agent) = self.agent_mut(id) {
            let change = agent.activity.seen();
            agent.report(change);
        }
        self.focused = id;
        self.selection = None;
        self.select(Tab::Agent);
    }

    /// Sub-aba `n` (1–9) da aba aberta; na aba do agente, um número além da
    /// última abre outra instância.
    fn open_sub(&mut self, n: usize) {
        if self.view.is_none() && n > self.agents.len() {
            self.new_instance();
        } else {
            self.pick(n - 1);
        }
    }

    /// Troca a instância aberta por uma nova, no mesmo lugar das sub-abas.
    fn restart_active(&mut self) {
        if !matches!(spec(self.active).kind, TabKind::Agent) {
            return;
        }
        if self.stale && !self.reload() {
            return;
        }
        let Some(i) = self.agents.iter().position(|a| a.id == self.focused) else {
            return;
        };
        let id = self.next_id;
        match Agent::spawn(
            id,
            &self.agent,
            self.adapter,
            &self.command,
            &self.cwd,
            self.body,
            self.session_ids,
            self.link.as_ref(),
            None,
            self.middleware,
            &self.tx,
        ) {
            Ok(agent) => {
                self.next_id += 1;
                self.agents.insert(i + 1, agent);
                self.record_usage(Some(self.focused), false);
                // Encerra a antiga (o `Drop` mata o processo).
                self.agents.remove(i);
                self.focus(id);
                self.flash(format!("↻ {} {id} reiniciado", self.agent));
            }
            Err(err) => self.notify(format!("⚠ {err:#}")),
        }
    }

    /// Sub-abas da aba aberta (instâncias do agente ou visões) e qual está
    /// aberta.
    fn sub_tabs(&self) -> (usize, usize) {
        match &self.view {
            Some(view) => (view.tab.views(&self.extensions).len(), view.index),
            None => (
                self.agents.len(),
                self.agents
                    .iter()
                    .position(|a| a.id == self.focused)
                    .unwrap_or(0),
            ),
        }
    }

    /// Abre a sub-aba `index` da aba aberta.
    fn pick(&mut self, index: usize) {
        let (count, current) = self.sub_tabs();
        if index >= count || index == current {
            return;
        }
        match &mut self.view {
            Some(view) => {
                view.index = index;
                self.load(true);
            }
            None => self.focus(self.agents[index].id),
        }
    }

    /// Passa `delta` sub-abas adiante (ou atrás), em volta.
    fn cycle(&mut self, delta: isize) {
        let (count, current) = self.sub_tabs();
        if count > 1 {
            self.pick((current as isize + delta).rem_euclid(count as isize) as usize);
        }
    }

    /// Tira da lista a instância que terminou; sem nenhuma, sai. Se era a
    /// sub-aba aberta, abre a vizinha.
    fn closed(&mut self, id: usize) {
        let Some(i) = self.agents.iter().position(|a| a.id == id) else {
            return;
        };
        // A última sai junto com a TUI, que grava todas ao encerrar.
        if self.agents.len() > 1 {
            self.record_usage(Some(id), false);
        }
        self.agents.remove(i);
        if self.agents.is_empty() {
            self.quit = true;
            return;
        }
        if self.focused == id {
            self.focused = self.agents[i.min(self.agents.len() - 1)].id;
        }
    }

    /// Grava no histórico por space e agente ([`History`]) o consumo da
    /// sessão de cada instância (só a `only`, se dada). Em segundo plano, a
    /// não ser com `wait` (no encerramento, quando o processo vai sair).
    fn record_usage(&mut self, only: Option<usize>, wait: bool) {
        self.recorded_at = Instant::now();
        let sessions: Vec<_> = self
            .agents
            .iter()
            .filter(|a| only.is_none_or(|id| a.id == id))
            .map(|a| {
                let started = a
                    .launched
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                let key = match &a.session {
                    Some(session) => format!("{}:{session}", a.name),
                    None => format!(
                        "{}:{}-{}-{}",
                        a.name,
                        std::process::id(),
                        a.id,
                        started.as_millis()
                    ),
                };
                (
                    key,
                    a.name.clone(),
                    a.adapter,
                    a.launched,
                    a.session.clone(),
                )
            })
            .collect();
        if sessions.is_empty() {
            return;
        }
        let (cwd, space) = (self.cwd.clone(), self.space.clone());
        let job = move || {
            let Ok(db) = History::open() else {
                return;
            };
            for (key, agent, adapter, launched, session) in sessions {
                let Ok(usage) = adapter.usage(&cwd, launched, session.as_deref()) else {
                    continue;
                };
                if let Some(summary) = &usage.session {
                    let _ = db.record(&Record {
                        key: &key,
                        agent: &agent,
                        space: &space,
                        cwd: &cwd,
                        started: launched,
                        summary,
                    });
                }
            }
        };
        if wait {
            job();
        } else {
            thread::spawn(job);
        }
    }

    /// Lê em segundo plano o consumo de cada instância para o rodapé e o
    /// total da sessão ([`Msg::Live`]); uma leitura por vez.
    fn poll_live(&mut self) {
        if self.live_busy {
            return;
        }
        self.live_at = Instant::now();
        self.live_busy = true;
        let sessions: Vec<_> = self
            .agents
            .iter()
            .map(|a| (a.id, a.adapter, a.launched, a.session.clone()))
            .collect();
        let (cwd, tx) = (self.cwd.clone(), self.tx.clone());
        thread::spawn(move || {
            let live = sessions
                .into_iter()
                .filter_map(|(id, adapter, launched, session)| {
                    let live = adapter.live_usage(&cwd, launched, session.as_deref());
                    Some((id, live.ok()??))
                })
                .collect();
            let _ = tx.send(Msg::Live(live));
        });
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal, rx: &Receiver<Msg>) -> Result<()> {
        let mut dirty = true;
        while !self.quit {
            while let Ok(msg) = rx.try_recv() {
                self.handle_msg(msg);
                dirty = true;
            }
            // Só as abas com `refresh` no Registro se atualizam sozinhas (as
            // visões do consumo, não os relatórios do RTK).
            if self.deve_atualizar() {
                self.load(false);
            }
            if self.recorded_at.elapsed() >= RECORD_EVERY {
                self.record_usage(None, false);
            }
            if self.live_at.elapsed() >= LIVE_EVERY {
                self.poll_live();
            }
            // Mensagens para um agente ocioso (sem saída que dispare a entrega)
            // e o silêncio depois de trabalhar, que vira "terminou".
            let now = Instant::now();
            for agent in &mut self.agents {
                agent.deliver_pending();
                let change = agent.activity.tick(now);
                agent.report(change);
            }
            let exited: Vec<_> = self
                .agents
                .iter_mut()
                .filter_map(|a| a.exited().then_some(a.id))
                .collect();
            for id in exited {
                self.handle_msg(Msg::Exited(id));
                dirty = true;
            }
            if self
                .status
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() >= STATUS_TTL)
            {
                self.status = None;
                dirty = true;
            }
            if dirty {
                self.fit(terminal)?;
                terminal.draw(|f| self.draw(f))?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(16))? {
                // Várias teclas de uma vez (ex.: digitação rápida) num só redesenho.
                loop {
                    if self.handle_event(event::read()?)? {
                        return Ok(());
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
                dirty = true;
            }
        }
        Ok(())
    }

    /// Se o conteúdo da aba aberta deve ser recarregado sozinho: a aba tem
    /// `refresh` no Registro, não está carregando, está numa sub-aba própria
    /// (não num relatório) e já passou o [`USAGE_REFRESH`].
    fn deve_atualizar(&self) -> bool {
        let Some(view) = &self.view else {
            return false;
        };
        let spec = spec(view.tab);
        !view.loading
            && spec.refresh
            && view.index < spec.own_views.len()
            && view.loaded_at.elapsed() >= USAGE_REFRESH
    }

    /// Ajusta as instâncias ao tamanho real da tela. Conferido antes de cada
    /// desenho, e não só no `Event::Resize`, porque o tamanho lido antes de
    /// abrir a TUI pode não ser o da tela em que ela é desenhada.
    fn fit(&mut self, terminal: &DefaultTerminal) -> Result<()> {
        let size = terminal.size()?;
        let body = body_size(Rect::new(0, 0, size.width, size.height));
        if body != self.body {
            self.body = body;
            self.selection = None;
            for agent in &mut self.agents {
                agent.resize(body)?;
            }
        }
        Ok(())
    }

    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Output(id, bytes) => {
                if let Some(agent) = self.agent_mut(id) {
                    if agent.spawn_id.is_some() {
                        agent.output_buf.extend_from_slice(&bytes);
                    }
                    agent.output(&bytes);
                    agent.answer_cursor_queries(&bytes);
                    agent.deliver_pending();
                }
            }
            Msg::Stderr(id, line) => {
                if let Some(chat) = self.agent_mut(id).and_then(|a| a.chat.as_mut()) {
                    chat.stderr(line);
                }
            }
            Msg::Live(list) => {
                self.live_busy = false;
                for (id, live) in list {
                    // A instância pode ter fechado durante a leitura: o gasto
                    // dela continua contando na sessão.
                    self.spent.insert(id, live.launched.tokens.io());
                    if let Some(agent) = self.agent_mut(id) {
                        agent.live = Some(live);
                    }
                }
            }
            Msg::Exited(id) => {
                // No Middleware o agente não tem tela: o erro dele vira aviso.
                if let Some(agent) = self.agent_mut(id)
                    && let Some(chat) = &agent.chat
                    && !chat.errors().is_empty()
                    && !matches!(agent.child.try_wait(), Ok(Some(status)) if status.success())
                {
                    let text = format!("⚠ {} {id} encerrou:\n{}", agent.name, chat.errors());
                    self.notify(text);
                }
                // Se era subagente MCP: envia resultado e, se saiu com sucesso, fecha a aba.
                if let Some(agent) = self.agent_mut(id)
                    && let (Some(spawn_id), Some(mcp_port)) =
                        (agent.spawn_id.clone(), agent.mcp_port)
                {
                    let output = String::from_utf8_lossy(&agent.output_buf).into_owned();
                    let exit_code = agent
                        .child
                        .try_wait()
                        .ok()
                        .flatten()
                        .map(|s| s.exit_code() as i32)
                        .unwrap_or(0);
                    send_spawn_result(mcp_port, &spawn_id, &output, exit_code);
                    if exit_code == 0 {
                        self.closed(id);
                        return;
                    }
                }
                self.closed(id);
            }
            Msg::Loaded { tab, index, result } => {
                if let Some(view) = &mut self.view
                    && view.tab == tab
                    && view.index == index
                {
                    view.content = match result {
                        Ok(text) => Content::Ready(text),
                        Err(err) => Content::Failed(err),
                    };
                    view.loading = false;
                    view.loaded_at = Instant::now();
                }
            }
            Msg::SpawnSubagent(req) => self.spawn_subagent(req),
            Msg::Message {
                instance_id,
                from,
                text,
            } => {
                if let Some(agent) = self
                    .agents
                    .iter_mut()
                    .find(|a| a.instance_id.as_deref() == Some(instance_id.as_str()))
                {
                    agent.queue(&text);
                    let label = format!("{} {}", agent.name, agent.id);
                    self.flash(format!("✉ {from} → {label}"));
                }
            }
            Msg::Focus { instance_id } => {
                let Some((id, name)) = self
                    .agents
                    .iter()
                    .find(|a| a.instance_id.as_deref() == Some(instance_id.as_str()))
                    .map(|a| (a.id, a.name.clone()))
                else {
                    return;
                };
                self.popup = None;
                self.focus(id);
                self.flash(format!("{name} {id}"));
                thread::spawn(terraforme_core::platform::raise_terminal);
            }
            Msg::Open {
                instance_id,
                from,
                text,
            } => self.open_requested(instance_id, &from, text),
            Msg::Close { instance_id, from } => {
                // A aba some quando o processo termina (`Msg::Exited`).
                if let Some(agent) = self
                    .agents
                    .iter_mut()
                    .find(|a| a.instance_id.as_deref() == Some(instance_id.as_str()))
                {
                    let _ = agent.child.kill();
                    let label = format!("{} {}", agent.name, agent.id);
                    self.flash(format!("✕ {label} fechado por {from}"));
                }
            }
            Msg::Reconnected(port) => {
                let moved = self.mcp_port != Some(port);
                self.mcp_port = Some(port);
                if let Some(link) = &mut self.link {
                    link.port = port;
                }
                for agent in &mut self.agents {
                    agent.mcp_port = Some(port);
                    agent.register(true);
                }
                // O agente aberto guarda o endereço antigo no `--mcp-config`.
                if moved && !self.agents.is_empty() {
                    self.flash(
                        "daemon MCP reiniciado · ⌃⌥R reinicia a instância para o agente usá-lo",
                    );
                }
            }
        }
    }

    /// Devolve `true` para sair.
    fn handle_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.popup.is_none() => {
                if let Some(form) = &mut self.form {
                    if let Some(editing) = &mut form.editing {
                        editing.extend(text.chars().filter(|c| c.is_ascii_digit()));
                    }
                    return Ok(false);
                }
                self.selection = None;
                if let Some(agent) = self.active_agent() {
                    if let Some(chat) = &mut agent.chat {
                        chat.paste(&text);
                    } else if agent.parser.screen().bracketed_paste() {
                        agent.input(format!("\x1b[200~{text}\x1b[201~").as_bytes())?;
                    } else {
                        agent.input(text.as_bytes())?;
                    }
                }
            }
            Event::Mouse(mouse) => return self.handle_mouse(mouse),
            // O redesenho seguinte ajusta as instâncias ao novo tamanho (`fit`).
            _ => {}
        }
        Ok(false)
    }

    /// Roda do mouse e seleção de texto. Sem a captura do mouse o terminal
    /// converte a roda em setas, que no agente navegam o histórico de prompts
    /// em vez de rolar a tela.
    fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
        // Clique no menu: abre a aba ou sai.
        if mouse.row == 0 && mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            match self.menu_hit(mouse.column) {
                Some(MenuItem::Quit) => return Ok(true),
                Some(MenuItem::Tab(tab)) if tab != self.active => self.select(tab),
                _ => {}
            }
            return Ok(false);
        }
        self.mouse_event(mouse)?;
        Ok(false)
    }

    /// Roda e seleção fora do menu.
    fn mouse_event(&mut self, mouse: MouseEvent) -> Result<()> {
        let up = match mouse.kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            MouseEventKind::Down(MouseButton::Left) if self.form.is_some() => {
                self.click_form(mouse.row);
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left) => {
                self.select_text(mouse);
                return Ok(());
            }
            _ => return Ok(()),
        };
        if let Some(popup) = &mut self.popup {
            popup.scroll = wheel(popup.scroll, up);
            return Ok(());
        }
        if let Some(form) = &mut self.form {
            if form.editing.is_none() {
                form.index = if up {
                    form.index.saturating_sub(1)
                } else {
                    (form.index + 1).min(form.fields().len() - 1)
                };
            }
            return Ok(());
        }
        if let Some(view) = &mut self.view {
            view.scroll = wheel(view.scroll, up);
            return Ok(());
        }
        self.selection = None;
        let Some(agent) = self.active_agent() else {
            return Ok(());
        };
        if let Some(chat) = &mut agent.chat {
            let lines = WHEEL_LINES as isize;
            chat.scroll_by(if up { lines } else { -lines });
            return Ok(());
        }
        let screen = agent.parser.screen();
        if screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None {
            // O agente pediu o mouse: ele mesmo rola.
            let col = mouse.column;
            let row = mouse.row.saturating_sub(AGENT_TOP);
            let bytes = wheel_bytes(up, col, row, screen.mouse_protocol_encoding());
            agent.input(&bytes)?;
        } else if screen.alternate_screen() {
            // Tela alternativa não tem histórico: vira setas, como no xterm.
            let arrow = if up { KeyCode::Up } else { KeyCode::Down };
            let bytes = key_bytes(
                KeyEvent::new(arrow, KeyModifiers::NONE),
                screen.application_cursor(),
            );
            agent.input(&bytes.repeat(WHEEL_LINES as usize))?;
        } else {
            let lines = WHEEL_LINES as isize;
            agent.scroll_history(if up { lines } else { -lines });
        }
        Ok(())
    }

    /// Clique na aba Config: escolhe o campo da linha; no endereço da UI,
    /// abre o navegador.
    fn click_form(&mut self, row: u16) {
        if self.popup.is_some() {
            return;
        }
        let (Some(form), Some(view)) = (&mut self.form, &self.view) else {
            return;
        };
        // Menu e borda de cima da aba.
        let Some(line) = row.checked_sub(FORM_TOP) else {
            return;
        };
        let clicked = form.click((line + view.scroll) as usize);
        if clicked == Some(Field::Ui) {
            form.open_ui();
        }
        if clicked.is_some_and(|f| f != Field::Provider) && form.leave_router() {
            self.switch_agent();
        }
    }

    /// Arrastar com o botão esquerdo seleciona texto da instância aberta e,
    /// ao soltar, copia (se [`App::copy_on_select`]). Um clique sem arrastar
    /// desfaz a seleção.
    fn select_text(&mut self, mouse: MouseEvent) {
        if self.popup.is_some() || self.view.is_some() {
            return;
        }
        let Some(agent) = self.agent(self.focused).filter(|a| a.chat.is_none()) else {
            return;
        };
        let (rows, cols) = agent.parser.screen().size();
        let pos = (
            mouse
                .row
                .saturating_sub(AGENT_TOP)
                .min(rows.saturating_sub(1)),
            mouse.column.min(cols.saturating_sub(1)),
        );
        match mouse.kind {
            MouseEventKind::Down(_) => {
                self.selection = (mouse.row >= AGENT_TOP).then_some(Selection {
                    agent: self.focused,
                    anchor: pos,
                    head: pos,
                    dragging: true,
                });
            }
            MouseEventKind::Drag(_) => {
                if let Some(selection) = &mut self.selection
                    && selection.dragging
                {
                    selection.head = pos;
                }
            }
            _ => {
                let Some(selection) = &mut self.selection else {
                    return;
                };
                selection.dragging = false;
                if selection.anchor == selection.head {
                    self.selection = None;
                } else if self.copy_on_select {
                    self.copy_selection();
                }
            }
        }
    }

    /// `Ctrl+Alt+C`: a seleção ou, sem ela, a tela visível da instância; nas
    /// outras abas, o conteúdo delas.
    fn copy(&mut self) {
        let text = match (&self.view, &self.form) {
            (_, Some(form)) => plain(&form.text()),
            (Some(view), None) => match &view.content {
                Content::Ready(text) => plain(text),
                Content::Failed(err) => err.clone(),
                Content::Loading => String::new(),
            },
            (None, None) => match self.agent(self.focused) {
                Some(Agent {
                    chat: Some(chat), ..
                }) => chat.transcript(),
                Some(agent) => match self.selection.filter(|s| s.agent == agent.id) {
                    Some(selection) => selection.text(agent.parser.screen()),
                    None => agent.parser.screen().contents(),
                },
                None => String::new(),
            },
        };
        self.copy_text(&text);
    }

    fn copy_selection(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(agent) = self.agent(selection.agent) else {
            return;
        };
        let text = selection.text(agent.parser.screen());
        self.copy_text(&text);
    }

    fn copy_text(&mut self, text: &str) {
        let text = text.trim_end();
        if text.is_empty() {
            self.flash("nada para copiar");
            return;
        }
        let chars = text.chars().count();
        self.flash(if clipboard(text) {
            format!("✔ copiado · {chars} caracteres")
        } else {
            format!("✔ copiado pelo terminal (OSC 52) · {chars} caracteres")
        });
    }

    /// Liga ou desliga a captura do mouse; desligada, o terminal faz a
    /// seleção nativa (e a roda vira setas).
    fn set_mouse(&mut self, on: bool) {
        self.mouse = on;
        self.selection = None;
        let mut stdout = std::io::stdout();
        let _ = if on {
            execute!(stdout, EnableMouseCapture)
        } else {
            execute!(stdout, DisableMouseCapture)
        };
        self.flash(if on {
            "🖱 mouse na TUI: arrastar seleciona e copia · ⌃⌥S devolve ao terminal"
        } else {
            "🖱 mouse no terminal: seleção nativa · ⌃⌥S volta para a TUI"
        });
    }

    fn help(&mut self) {
        self.popup = Some(Popup {
            title: " Atalhos ",
            text: shortcuts(self.enhanced),
            scroll: 0,
        });
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(delta) = tab_step(key) {
            self.step_tab(delta);
            return Ok(false);
        }
        if let Some(tab) = tab_shortcut(key) {
            // Uma aba fora do menu (Extensões sem relatórios) não tem para
            // onde ir; a tecla não passa para o agente.
            if self.tabs().contains(&tab) && self.active != tab {
                self.select(tab);
            }
            return Ok(false);
        }
        if let Some(n) = sub_tab_number(key) {
            self.open_sub(n);
            return Ok(false);
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let back = key.code == KeyCode::BackTab || shift;
        match key.code {
            KeyCode::F(10) => return Ok(true),
            // Ctrl+Alt+Tab: próxima sub-aba (com Shift, a anterior). Sem o
            // protocolo do kitty chega como Alt+Tab.
            KeyCode::Tab | KeyCode::BackTab if alt => self.cycle(if back { -1 } else { 1 }),
            KeyCode::PageUp if ctrl => self.cycle(-1),
            KeyCode::PageDown if ctrl => self.cycle(1),
            // Comandos da TUI: Ctrl+Alt+letra, que o agente não usa.
            KeyCode::Char(c) if ctrl && alt && "nwrcsk".contains(c.to_ascii_lowercase()) => {
                match c.to_ascii_lowercase() {
                    'n' => self.new_instance(),
                    'w' => self.close_active(),
                    'r' => self.restart_active(),
                    'c' => self.copy(),
                    's' => self.set_mouse(!self.mouse),
                    _ => self.help(),
                }
            }
            // Alguns terminais mandam Shift+F12 como F24.
            KeyCode::F(12) if shift => self.close_active(),
            KeyCode::F(24) => self.close_active(),
            KeyCode::F(12) => self.new_instance(),
            _ if self.popup.is_some() => self.popup_key(key),
            _ if self.view.is_some() => self.view_key(key),
            _ => {
                let Some(agent) = self.active_agent() else {
                    return Ok(false);
                };
                if agent.chat.is_some() {
                    agent.chat_key(key)?;
                    return Ok(false);
                }
                let half = agent.parser.screen().size().0 as isize / 2;
                match key.code {
                    KeyCode::PageUp if shift => agent.scroll_history(half),
                    KeyCode::PageDown if shift => agent.scroll_history(-half),
                    KeyCode::Home if shift => agent.scroll_history(isize::MAX),
                    KeyCode::End if shift => agent.scroll_history(isize::MIN),
                    _ => {
                        let bytes = key_bytes(key, agent.parser.screen().application_cursor());
                        if !bytes.is_empty() {
                            agent.scroll_history(isize::MIN);
                            agent.input(&bytes)?;
                        }
                    }
                }
                self.selection = None;
            }
        }
        Ok(false)
    }

    /// Encerra a instância aberta; a aba some quando o processo termina.
    fn close_active(&mut self) {
        if let Some(agent) = self.active_agent() {
            let _ = agent.child.kill();
        }
    }

    fn popup_key(&mut self, key: KeyEvent) {
        let Some(popup) = &mut self.popup else {
            return;
        };
        match key.code {
            KeyCode::Up => popup.scroll = popup.scroll.saturating_sub(1),
            KeyCode::Down => popup.scroll = popup.scroll.saturating_add(1),
            _ => self.popup = None,
        }
    }

    fn view_key(&mut self, key: KeyEvent) {
        if let Some(form) = &mut self.form {
            if form.editing.is_none() && matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.select(Tab::Agent);
                return;
            }
            // A TUI aplica na hora as preferências dela.
            let changed = form.key(key);
            let tui = form.config.tui;
            self.config_changed(changed, tui);
            return;
        }
        let Some(view) = &mut self.view else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.select(Tab::Agent),

            // Rolagem e recarregar.
            KeyCode::Up | KeyCode::Char('k') => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => view.scroll = view.scroll.saturating_add(1),
            KeyCode::PageUp => view.scroll = view.scroll.saturating_sub(10),
            KeyCode::PageDown => view.scroll = view.scroll.saturating_add(10),
            KeyCode::Home | KeyCode::Char('g') => view.scroll = 0,
            KeyCode::Char('r') => self.load(true),
            // ←/→: aba anterior/próxima (nas visões, não chega ao agente).
            KeyCode::Left => self.step_tab(-1),
            KeyCode::Right => self.step_tab(1),
            _ => {}
        }
    }

    /// Aplica na TUI o campo que a aba Config acabou de gravar.
    fn config_changed(&mut self, field: Option<Field>, tui: terraforme_core::config::TuiConfig) {
        match field {
            Some(Field::Mouse) if tui.mouse != self.mouse => self.set_mouse(tui.mouse),
            Some(Field::CopyOnSelect) => self.copy_on_select = tui.copy_on_select,
            Some(Field::Provider) => self.switch_agent(),
            Some(Field::Mode | Field::Extension(_) | Field::Account) => self.stale = true,
            Some(Field::Middleware) => {
                self.middleware = tui.middleware;
                self.flash("Middleware vale nas próximas instâncias (⌃⌥N, ⌃⌥R)");
            }
            _ => {}
        }
    }

    /// Abre a aba `delta` posições adiante (ou atrás) no menu, em volta.
    fn step_tab(&mut self, delta: isize) {
        let tabs = self.tabs();
        let current = tabs.iter().position(|&t| t == self.active).unwrap_or(0);
        let next = tabs[(current as isize + delta).rem_euclid(tabs.len() as isize) as usize];
        if next != self.active {
            self.select(next);
        }
    }

    /// Abre a aba pelo jeito que o Registro [`TABS`] diz: o agente fica com
    /// o pseudoterminal, as abas de conteúdo recarregam o conteúdo e a
    /// Config abre o formulário.
    fn select(&mut self, tab: Tab) {
        // Sair da Config com o Router mudado troca o agente.
        let switch = self.form.as_mut().is_some_and(Form::leave_router);
        let kind = spec(tab).kind;
        self.active = tab;
        self.form = (kind == TabKind::Form).then(Form::load);
        self.view = match kind {
            TabKind::Agent => None,
            TabKind::Content | TabKind::Form => Some(View {
                tab,
                index: 0,
                content: Content::Loading,
                scroll: 0,
                loaded_at: Instant::now(),
                loading: false,
            }),
        };
        if self.view.is_some() && self.form.is_none() {
            self.load(true);
        }
        if switch {
            self.switch_agent();
        }
    }

    /// Carrega o conteúdo da aba aberta em segundo plano. Sem `reset`, o
    /// conteúdo atual fica na tela até o novo chegar.
    fn load(&mut self, reset: bool) {
        let (launched, session) = self
            .agent(self.focused)
            .map(|a| (a.launched, a.session.clone()))
            .unwrap_or((SystemTime::now(), None));
        let Some(view) = &mut self.view else {
            return;
        };
        if reset {
            view.content = Content::Loading;
            view.scroll = 0;
        }
        view.loading = true;
        let (tab, index) = (view.tab, view.index);
        // Os relatórios vêm depois das sub-abas próprias da aba (as visões
        // do consumo), que o Registro guarda.
        let report = index
            .checked_sub(spec(tab).own_views.len())
            .and_then(|i| reports(tab, &self.extensions).get(i).copied());
        let tx = self.tx.clone();
        let cwd = self.cwd.clone();
        // O consumo é o da instância aberta, que pode ser de outro agente.
        let adapter = self.agent(self.focused).map_or(self.adapter, |a| a.adapter);
        let routers = self.routers(launched, session.clone());
        thread::spawn(move || {
            let result = match (tab, report) {
                (_, Some((ext, report))) => ext.report(report, adapter.bin()).map(Text::from),
                (Tab::Usage, None) if index == SPACES_VIEW => History::open()
                    .and_then(|db| db.totals())
                    .map(|totals| format_spaces(&totals)),
                (Tab::Usage, None) if index == 0 => Ok(format_routers(&routers, &cwd)),
                (Tab::Usage, None) => {
                    adapter
                        .usage(&cwd, launched, session.as_deref())
                        .map(|u| match index {
                            1 => format_usage(&u),
                            _ => format_agents(&u),
                        })
                }
                _ => return,
            };
            let result = result.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Loaded { tab, index, result });
        });
    }

    /// Um router por agente com instância aberta (instâncias do mesmo agente
    /// dividem os limites), o da instância em foco primeiro; sem instância,
    /// o agente da aba.
    fn routers(&self, launched: SystemTime, session: Option<String>) -> Vec<Router> {
        let mut routers = vec![match self.agent(self.focused) {
            Some(a) => (a.name.clone(), a.adapter, launched, session),
            None => (self.agent.clone(), self.adapter, launched, session),
        }];
        for a in &self.agents {
            if !routers.iter().any(|(name, ..)| *name == a.name) {
                routers.push((a.name.clone(), a.adapter, a.launched, a.session.clone()));
            }
        }
        routers
    }

    /// Prefixo das teclas das sub-abas mostrado nas barras.
    fn sub_key(&self) -> &'static str {
        if self.enhanced { "⌃⌥" } else { "⌥" }
    }

    fn draw(&self, frame: &mut Frame) {
        let [menu, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.draw_menu(frame, menu);
        self.draw_footer(frame, footer);

        if let Some(view) = &self.view {
            self.draw_view(frame, view, body);
        } else if let Some(agent) = self.agent(self.focused) {
            let [instances, body] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
            // O total da sessão fica à direita; sem espaço, sai a dica de atalhos.
            let spent = self.spent_line();
            let fits = |line: &Line| line.width() + spent.width() < instances.width as usize;
            let line = Some(self.instances(true))
                .filter(fits)
                .unwrap_or_else(|| self.instances(false));
            let room = fits(&line);
            frame.render_widget(line, instances);
            if room {
                frame.render_widget(spent, instances);
            }

            if let Some(chat) = &agent.chat {
                chat.render(frame, body, self.popup.is_none());
            } else {
                self.draw_terminal(frame, agent, body);
            }
        }

        if let Some(popup) = &self.popup {
            let area = body.inner(Margin::new(
                (body.width / 12).max(1),
                (body.height / 4).max(1),
            ));
            frame.render_widget(Clear, area);
            let block = Block::bordered()
                .title(Line::from(popup.title).bold().cyan())
                .title_bottom(Line::from(" ↑↓ rolar · qualquer tecla fecha ").dark_gray())
                .border_style(Style::new().cyan());
            frame.render_widget(
                Paragraph::new(popup.text.as_str())
                    .wrap(Wrap { trim: false })
                    .scroll((popup.scroll, 0))
                    .block(block),
                area,
            );
        }

        if let Some((text, _)) = &self.status {
            let span = Span::raw(format!(" {text} ")).black().on_green();
            let full = frame.area();
            let width = (span.width() as u16).min(full.width);
            let area = Rect::new(
                full.right().saturating_sub(width),
                full.bottom().saturating_sub(1),
                width,
                1,
            );
            frame.render_widget(Clear, area);
            frame.render_widget(span, area);
        }
    }

    /// O pseudoterminal da instância, com a seleção e o aviso do histórico.
    fn draw_terminal(&self, frame: &mut Frame, agent: &Agent, body: Rect) {
        let mut term = PseudoTerminal::new(agent.parser.screen());
        if self.popup.is_some() || agent.scrollback > 0 {
            let mut cursor = tui_term::widget::Cursor::default();
            cursor.hide();
            term = term.cursor(cursor);
        }
        frame.render_widget(term, body);

        if let Some(selection) = self.selection.filter(|s| s.agent == agent.id) {
            let buf = frame.buffer_mut();
            for row in 0..body.height {
                for col in 0..body.width {
                    if selection.contains(row, col) {
                        buf[(body.x + col, body.y + row)]
                            .set_style(Style::new().black().on_light_cyan());
                    }
                }
            }
        }

        if agent.scrollback > 0 {
            let hint = format!(
                " histórico −{} · Shift+PgDn/Shift+End volta ",
                agent.scrollback
            );
            let width = hint.chars().count() as u16;
            let area =
                Rect::new(body.right().saturating_sub(width), body.y, width, 1).intersection(body);
            frame.render_widget(Span::raw(hint).black().on_yellow(), area);
        }
    }

    /// Itens do menu com o texto de cada um, na ordem da tela: os blocos
    /// `Terraforme`, `Agente (claude)`, `Consumo`, … e `Sair`, separados por um espaço.
    fn menu_items(&self) -> Vec<(Option<MenuItem>, String)> {
        let mut items = vec![(None, " Terraforme ".to_string())];
        for tab in self.tabs() {
            items.push((None, " ".to_string()));
            // Com o Middleware as instâncias de todos os routers são
            // sub-abas de uma aba só, a do chat.
            let label = match tab {
                Tab::Agent if self.middleware || self.agents.iter().any(|a| a.chat.is_some()) => {
                    "Middleware".to_string()
                }
                tab => tab.label(&self.agent),
            };
            items.push((Some(MenuItem::Tab(tab)), format!(" {label} ")));
        }
        items.push((None, " ".to_string()));
        items.push((Some(MenuItem::Quit), " Sair ".to_string()));
        items
    }

    /// Item do menu na coluna `col`.
    fn menu_hit(&self, col: u16) -> Option<MenuItem> {
        let mut x = 0u16;
        for (item, text) in self.menu_items() {
            let width = Span::raw(text.as_str()).width() as u16;
            if (x..x + width).contains(&col) {
                return item;
            }
            x += width;
        }
        None
    }

    /// Menu de abas à esquerda e, do outro lado, o atalho da lista de atalhos.
    fn draw_menu(&self, frame: &mut Frame, area: Rect) {
        let spans: Vec<_> = self
            .menu_items()
            .into_iter()
            .map(|(item, text)| {
                let style = match item {
                    None if text == " Terraforme " => Style::new().black().on_green().bold(),
                    None => Style::new(),
                    // Mesmo bloco do Terraforme: verde na aba ativa, cinza nas outras.
                    Some(MenuItem::Tab(tab)) if tab == self.active => {
                        Style::new().black().on_green().bold()
                    }
                    Some(MenuItem::Tab(_)) => Style::new().black().on_dark_gray().bold(),
                    Some(MenuItem::Quit) => Style::new().light_red().on_dark_gray().bold(),
                };
                Span::styled(text, style)
            })
            .collect();
        frame.render_widget(Line::from(spans).style(Style::new().bg(Color::Reset)), area);

        let mouse = if self.mouse {
            ""
        } else {
            "🖱 seleção nativa · "
        };
        let right = Line::from(vec![
            Span::raw(format!("{mouse}{} · ", self.title)).dark_gray(),
            Span::styled(" ⌃⌥K ", Style::new().black().on_dark_gray().bold()),
            Span::raw(" Atalhos ").dark_gray(),
        ])
        .right_aligned();
        // Só cabe se não cobrir as abas.
        let left = self
            .menu_items()
            .iter()
            .map(|(_, t)| Span::raw(t.as_str()).width() as u16)
            .sum::<u16>();
        if left + 1 + right.width() as u16 <= area.width {
            frame.render_widget(right, area);
        }
    }

    /// Tokens gastos nesta sessão do terraforme, por todas as instâncias
    /// (inclusive as já fechadas), alinhado à direita.
    fn spent_line(&self) -> Line<'static> {
        let total: u64 = self.spent.values().sum();
        Line::from(vec![
            Span::raw(" Sessão terraforme ").dark_gray(),
            Span::styled(
                format!(" {} tokens ", thousands(total)),
                Style::new().black().on_dark_gray().bold(),
            ),
        ])
        .right_aligned()
    }

    /// Rodapé da instância aberta: à esquerda o consumo do prompt atual (o
    /// em andamento ou o último), à direita o total da sessão dela.
    /// No modo Middleware mostra o modo do Claude (esquerda) e só o total (direita).
    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let Some(agent) = self.agent(self.focused) else {
            return;
        };
        // Sem leitura ainda (ou agente que não informa o consumo): rodapé vazio.
        let Some(live) = &agent.live else {
            return;
        };
        if let Some(chat) = &agent.chat {
            let left = Line::from(format!(" {} ", chat.mode())).dark_gray();
            let right = Line::from(Span::styled(
                format!(" {} tokens ", thousands(live.session.tokens.io())),
                Style::new().black().on_dark_gray().bold(),
            ))
            .right_aligned();
            if left.width() + right.width() < area.width as usize {
                frame.render_widget(left, area);
            }
            frame.render_widget(right, area);
            return;
        }
        let turn = &live.turn.tokens;
        let left = Line::from(vec![
            Span::styled(" Prompt atual ", Style::new().black().on_dark_gray().bold()),
            Span::raw(format!(
                " {} tokens · entrada {} · saída {}",
                thousands(turn.io()),
                thousands(turn.input),
                thousands(turn.output + turn.reasoning),
            ))
            .dark_gray(),
        ]);
        let right = Line::from(vec![
            Span::raw(format!(" {} {} ", agent.name, agent.id)).dark_gray(),
            Span::styled(
                format!(" {} tokens ", thousands(live.session.tokens.io())),
                Style::new().black().on_dark_gray().bold(),
            ),
        ])
        .right_aligned();
        // Sem espaço para os dois, o total da instância tem prioridade.
        if left.width() + right.width() < area.width as usize {
            frame.render_widget(left, area);
        }
        frame.render_widget(right, area);
    }

    /// Sub-abas da aba do agente: uma por instância e, com `hint`, a dica
    /// dos atalhos.
    fn instances(&self, hint: bool) -> Line<'_> {
        let labels: Vec<_> = self
            .agents
            .iter()
            .map(|a| ("", format!("{} {}", a.name, a.id)))
            .collect();
        let (_, current) = self.sub_tabs();
        let mut spans = vec![Span::raw(" ")];
        spans.extend(sub_tab_spans(&labels, current, self.sub_key()));
        if !hint {
            return Line::from(spans);
        }
        let hint = if self.agents.len() > 1 {
            format!(
                " {k}Tab/{k}1-9 instância · ⌃⌥N nova · ⌃⌥W fecha · ⌃⌥R reinicia",
                k = self.sub_key()
            )
        } else {
            // No Middleware não há tela do agente para selecionar: ⌃⌥C copia a conversa.
            let copy = match self.agent(self.focused) {
                Some(agent) if agent.chat.is_some() => "⌃⌥C copia a conversa",
                _ => "arraste para copiar",
            };
            format!(
                " {}{} ou ⌃⌥N nova instância · ⌃⌥W fecha · {copy}",
                self.sub_key(),
                self.agents.len() + 1
            )
        };
        spans.push(Span::raw(hint).dark_gray());
        Line::from(spans)
    }

    fn draw_view(&self, frame: &mut Frame, view: &View, area: Rect) {
        let views = view.tab.views(&self.extensions);
        let title = match (view.tab, views.get(view.index)) {
            (Tab::Config, _) => Config::path().map_or_else(
                |_| "Configurações".to_string(),
                |p| format!("Configurações · {}", p.display()),
            ),
            (Tab::Extensions, Some((ext, _))) => format!("Extensões · {ext}"),
            (Tab::Extensions, None) => "Extensões".to_string(),
            (_, Some((ext, _))) if !ext.is_empty() => format!("Consumo · {ext}"),
            _ if self.agents.len() > 1 => match self.agent(self.focused) {
                Some(agent) => format!("Consumo de tokens · {} {}", agent.name, agent.id),
                None => "Consumo de tokens".to_string(),
            },
            _ => "Consumo de tokens".to_string(),
        };
        let hint = match view.tab {
            Tab::Config => {
                " ↑↓ campo · ←→/Espaço altera · dígitos editam · Del padrão · o/clique abre a UI · r relê · Esc agente "
                    .to_string()
            }
            _ if views.len() > 1 => format!(
                " {k}Tab/{k}1-9 sub-aba · ↑↓ rolar · r recarregar · ⌃⌥C copia · Esc agente ",
                k = self.sub_key()
            ),
            _ => " ↑↓ rolar · r recarregar · ⌃⌥C copia · Esc agente ".to_string(),
        };
        let block = Block::bordered()
            .title(Line::from(format!(" {title} ")).bold().cyan())
            .title(self.spent_line())
            .title_bottom(Line::from(hint).dark_gray())
            .border_style(Style::new().cyan());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let inner = if views.len() > 1 {
            let labels: Vec<_> = views
                .iter()
                .map(|&(group, name)| (group, name.to_string()))
                .collect();
            let tabs = sub_tab_spans(&labels, view.index, self.sub_key());
            let [header, rest] =
                Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
            frame.render_widget(
                Paragraph::new(Line::from(tabs)).wrap(Wrap { trim: false }),
                header,
            );
            rest
        } else {
            inner
        };

        let paragraph = match (&self.form, &view.content) {
            (Some(form), _) => Paragraph::new(form.text()),
            (None, Content::Loading) => Paragraph::new("carregando…").dark_gray(),
            (None, Content::Ready(text)) => Paragraph::new(text.clone()),
            (None, Content::Failed(err)) => Paragraph::new(format!("✘ {err}")).red(),
        };
        frame.render_widget(
            paragraph
                .wrap(Wrap { trim: false })
                .scroll((view.scroll, 0)),
            inner,
        );
    }
}

/// Barra de sub-abas, igual em todas as abas: `⌃⌥N rótulo` (`key` é o
/// prefixo da tecla), a aberta em destaque e o nome do grupo (ex.: a
/// extensão) antes do primeiro item dele.
fn sub_tab_spans(labels: &[(&str, String)], current: usize, key: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (group, name)) in labels.iter().enumerate() {
        if !group.is_empty() && (i == 0 || labels[i - 1].0 != *group) {
            if i > 0 {
                spans.push(Span::raw("│ ").dark_gray());
            }
            spans.push(Span::raw(format!("{group} ")).bold());
        }
        let label = format!(" {key}{} {name} ", i + 1);
        spans.push(if i == current {
            Span::styled(label, Style::new().black().on_cyan().bold())
        } else {
            Span::raw(label).cyan()
        });
        spans.push(Span::raw(" "));
    }
    spans
}

/// Agente com instância aberta na sub-aba Limites: nome, adapter, início e
/// sessão (os da instância usada para ler o consumo).
type Router = (String, &'static dyn Adapter, SystemTime, Option<String>);

/// Sub-aba Limites (`docs/tui/consumo/consumo.feature.md`): um bloco
/// `Router <nome>` por agente aberto, com as barras dos limites dele.
fn format_routers(routers: &[Router], cwd: &Path) -> Text<'static> {
    let budget = Config::load().map(|c| c.budget).unwrap_or_default();
    let mut lines: Vec<Line> = Vec::new();
    for (name, adapter, launched, session) in routers {
        lines.push(Line::from(vec![
            Span::raw("Router ").dark_gray(),
            Span::raw(name.clone()).cyan().bold(),
        ]));
        lines.push(Line::default());
        match adapter.usage(cwd, *launched, session.as_deref()) {
            Ok(usage) => lines.extend(format_limits(*adapter, &usage, &budget)),
            Err(err) => lines.push(Line::raw(format!("✘ {err:#}")).red()),
        }
        lines.push(Line::default());
    }
    Text::from(lines)
}

/// Uma barra por limite, no estilo do Claude Code: o título traz quando o
/// limite zera, o limite total de tokens (quando há) e a barra verde vai
/// encurtando a cada consumo.
fn format_limits(
    adapter: &dyn Adapter,
    usage: &usage::Usage,
    budget: &Budget,
) -> Vec<Line<'static>> {
    let limits = limits::collect(adapter, usage, budget);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut lines: Vec<Line> = Vec::new();
    for limit in &limits.items {
        let mut title = vec![Span::raw(limit.label.clone()).bold()];
        if let Some(at) = limit.resets_at {
            title.push(
                Span::raw(format!(
                    " - zera em {} - {}",
                    duration(at.saturating_sub(now)),
                    local_date(at)
                ))
                .dark_gray(),
            );
        }
        if let Some((used, max)) = limit.tokens {
            title.push(
                Span::raw(format!(
                    "  · {} de {} tokens",
                    thousands(used),
                    thousands(max)
                ))
                .dark_gray(),
            );
        }
        lines.push(Line::from(title));
        lines.push(bar(limit));
        lines.push(Line::default());
    }

    if limits.items.is_empty() {
        lines.push(Line::raw("Nenhum limite disponível.").yellow());
        match usage.source {
            UsageSource::Claude => {
                lines.push(Line::raw(
                    "Sem login OAuth no Claude Code (assinatura Pro/Max/Team), defina um orçamento de tokens:",
                ));
                lines.push(
                    Line::raw("  terraforme config --budget-5h 20000000 --budget-week 300000000")
                        .cyan(),
                );
            }
            UsageSource::OpenCode => {
                lines.push(Line::raw(
                    "Sem limites de plano, a janela de 5h vale só os tokens desta máquina (sem orçamento):",
                ));
                lines.push(Line::raw("  terraforme config --budget-week 300000000").cyan());
            }
            UsageSource::Antigravity => {
                lines.push(Line::raw(
                    "Sem limites do agy (abra o `agy` para entrar na conta), defina um orçamento de tokens:",
                ));
                lines.push(Line::raw("  terraforme config --budget-week 300000000").cyan());
            }
        }
        lines.push(Line::default());
    }
    if let Some(note) = limits.note {
        lines.push(Line::raw(format!("⚠ {note}")).yellow());
    }
    lines.push(
        Line::raw(match limits.source {
            Source::Plan => match usage.source {
                UsageSource::Antigravity => {
                    "Fonte: limites da conta do Antigravity (`agy -p /usage`) · atualiza a cada 60s"
                }
                _ => {
                    "Fonte: limites do plano (api.anthropic.com, como o /usage) · atualiza a cada 60s"
                }
            },
            Source::Budget => match usage.source {
                UsageSource::Claude => {
                    "Fonte: orçamento [budget] do config.toml × transcripts do Claude Code"
                }
                UsageSource::OpenCode => {
                    "Fonte: orçamento semanal [budget] × opencode stats --json · sem janela de 5h"
                }
                UsageSource::Antigravity => {
                    "Fonte: orçamento semanal [budget] × transcripts do Antigravity (agy)"
                }
            },
        })
        .dark_gray(),
    );
    lines
}

/// Segundos desde a época Unix → `DD/MM/AAAA - HH:MM` no fuso local.
fn local_date(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d/%m/%Y - %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Barra no estilo do Claude Code: o verde começa cheio em 100% e vai
/// diminuindo a cada consumo — `████████░░░░  75% restante` —, com a cor
/// esquentando quando o que falta aperta.
fn bar(limit: &Limit) -> Line<'static> {
    let remaining = limit.remaining();
    let filled = ((remaining / 100.0) * BAR_WIDTH as f64).round() as usize;
    let color = if remaining > 50.0 {
        Color::Green
    } else if remaining > 20.0 {
        Color::Yellow
    } else {
        Color::Red
    };
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::new().fg(color)),
        Span::raw("░".repeat(BAR_WIDTH - filled)).dark_gray(),
        Span::raw("  "),
        Span::styled(
            format!("{remaining:.0}% restante"),
            Style::new().fg(color).bold(),
        ),
    ])
}

/// `9000` → `2h30min`; `200000` → `2d 7h`.
fn duration(secs: u64) -> String {
    let (days, hours, mins) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("{mins}min"),
        (0, _) => format!("{hours}h{mins:02}min"),
        _ => format!("{days}d {hours}h"),
    }
}

const USAGE_HEADER: [&str; 7] = [
    "Entrada",
    "Saída",
    "Raciocínio",
    "Cache (grava)",
    "Cache (lê)",
    "Total",
    "Respostas",
];

fn usage_header() -> String {
    let [a, b, c, d, e, f, g] = USAGE_HEADER;
    format!(
        "{:<22} {a:>12} {b:>12} {c:>12} {d:>14} {e:>14} {f:>14} {g:>9}",
        ""
    )
}

fn usage_row(label: &str, tokens: &Tokens, responses: Option<usize>) -> String {
    format!(
        "{:<22} {:>12} {:>12} {:>12} {:>14} {:>14} {:>14} {:>9}",
        truncate(label, 22),
        thousands(tokens.input),
        thousands(tokens.output),
        thousands(tokens.reasoning),
        thousands(tokens.cache_write),
        thousands(tokens.cache_read),
        thousands(tokens.total()),
        responses.map(|r| thousands(r as u64)).unwrap_or_default()
    )
}

/// Tabela do consumo: uma linha por período e, embaixo, a sessão por modelo.
fn format_usage(usage: &usage::Usage) -> Text<'static> {
    if usage.source == UsageSource::OpenCode {
        return format_opencode_usage(usage);
    }
    let header = usage_header();
    let summary = |label: &str, s: &Summary| usage_row(label, &s.tokens, Some(s.responses));

    let empty = Summary::default();
    let mut lines = vec![
        header.clone(),
        summary("Sessão atual", usage.session.as_ref().unwrap_or(&empty)),
        summary("Projeto · 24h", &usage.project_day),
        summary("Todos · 5h", &usage.all_five_hours),
        summary("Todos · 24h", &usage.all_day),
        summary("Todos · 7 dias", &usage.all_week),
        String::new(),
    ];
    match &usage.session {
        Some(session) => {
            lines.push("Sessão atual por modelo".into());
            lines.push(header);
            for (model, tokens) in &session.by_model {
                lines.push(usage_row(model, tokens, None));
            }
        }
        None => lines.push("A sessão atual ainda não tem respostas do modelo.".into()),
    }
    lines.push(String::new());
    let fonte = match usage.source {
        UsageSource::Antigravity => format!(
            "Fonte: transcripts do Antigravity (agy) · atualiza a cada {}s",
            USAGE_REFRESH.as_secs()
        ),
        _ => format!(
            "Fonte: transcripts do Claude Code · atualiza a cada {}s",
            USAGE_REFRESH.as_secs()
        ),
    };
    lines.push(fonte);
    Text::from(lines.join("\n"))
}

fn format_opencode_usage(usage: &usage::Usage) -> Text<'static> {
    let header = usage_header();
    let summary = |label: &str, s: &Summary| usage_row(label, &s.tokens, Some(s.responses));
    let mut lines = vec![
        header.clone(),
        summary("Projeto · hoje", &usage.project_day),
        summary("Todos · hoje", &usage.all_day),
        summary("Todos · 7 dias", &usage.all_week),
        String::new(),
        "Projeto · hoje por modelo".into(),
        header,
    ];
    if usage.project_day.by_model.is_empty() {
        lines.push("Sem consumo registrado hoje neste projeto.".into());
    } else {
        for (model, tokens) in &usage.project_day.by_model {
            lines.push(usage_row(model, tokens, None));
        }
    }
    if let Some(stats) = &usage.opencode {
        lines.push(String::new());
        lines.push(format!(
            "Custo · projeto hoje: ${:.4}",
            stats.project_today_cost
        ));
        lines.push(format!("Custo · todos hoje: ${:.4}", stats.today_cost));
        lines.push(format!("Custo · todos 7 dias: ${:.4}", stats.week_cost));
    }
    lines.push(String::new());
    lines.push(format!(
        "Fonte: opencode stats --json · atualiza a cada {}s",
        USAGE_REFRESH.as_secs()
    ));
    lines.push(
        "O OpenCode não oferece consumo da sessão atual nem janela móvel de 5h; raciocínio é informado separadamente."
            .into(),
    );
    Text::from(lines.join("\n"))
}

/// A sessão atual dividida entre o agente principal e cada subagente.
fn format_agents(usage: &usage::Usage) -> Text<'static> {
    if usage.source == UsageSource::OpenCode {
        let Some(stats) = &usage.opencode else {
            return Text::from("Estatísticas de subagentes indisponíveis.");
        };
        return Text::from(format!(
            "O OpenCode não detalha tokens por subagente.\n\nSubagentes contabilizados nos últimos 7 dias: {}\nSessões: {} · prompts: {}\n\nFonte: opencode stats --json · atualiza a cada {}s",
            thousands(stats.subagents),
            thousands(stats.sessions),
            thousands(stats.prompts),
            USAGE_REFRESH.as_secs()
        ));
    }
    if usage.source == UsageSource::Antigravity {
        return Text::from(format!(
            "O Antigravity não detalha tokens por subagente nesta versão.\n\nFonte: transcripts do agy · atualiza a cada {}s",
            USAGE_REFRESH.as_secs()
        ));
    }
    if usage.session_agents.is_empty() {
        return Text::from("A sessão atual ainda não tem respostas do modelo.");
    }
    let mut lines = vec![usage_header()];
    let mut descriptions = Vec::new();
    for agent in &usage.session_agents {
        let label = match (&agent.id, &agent.kind) {
            (None, _) => "Principal".to_string(),
            (Some(_), Some(kind)) => kind.clone(),
            (Some(id), None) => format!("subagente {}", truncate(id, 8)),
        };
        lines.push(usage_row(
            &label,
            &agent.summary.tokens,
            Some(agent.summary.responses),
        ));
        if let Some(description) = &agent.description {
            descriptions.push(format!("  {} — {description}", truncate(&label, 22)));
        }
    }
    let subagents = usage
        .session_agents
        .iter()
        .filter(|a| a.id.is_some())
        .count();
    lines.push(String::new());
    lines.push(format!("{subagents} subagente(s) nesta sessão"));
    lines.extend(descriptions);
    lines.push(String::new());
    lines.push(format!(
        "Fonte: transcripts do Claude Code (<sessão>/subagents) · atualiza a cada {}s",
        USAGE_REFRESH.as_secs()
    ));
    Text::from(lines.join("\n"))
}

/// Histórico do consumo: uma linha por space × agente.
fn format_spaces(totals: &[SpaceUsage]) -> Text<'static> {
    let fonte = format!(
        "Fonte: {} · sessões abertas pelo terraforme cli, gravadas a cada {}s e ao fechar",
        History::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "usage.db".into()),
        RECORD_EVERY.as_secs()
    );
    if totals.is_empty() {
        return Text::from(format!("Nenhuma sessão gravada ainda.\n\n{fonte}"));
    }
    let mut lines = vec![format!(
        "{:<22} {:<12} {:>8} {:>14} {:>14} {:>14} {:>9}",
        "Space", "Agente", "Sessões", "24h", "7 dias", "Total", "Respostas"
    )];
    for row in totals {
        let space = if row.space.is_empty() {
            "(fora de space)"
        } else {
            &row.space
        };
        lines.push(format!(
            "{:<22} {:<12} {:>8} {:>14} {:>14} {:>14} {:>9}",
            truncate(space, 22),
            truncate(&row.agent, 12),
            thousands(row.sessions),
            thousands(row.day.total()),
            thousands(row.week.total()),
            thousands(row.total.total()),
            thousands(row.responses)
        ));
    }
    lines.push(String::new());
    lines.push("24h e 7 dias: sessões atualizadas no período (o total inteiro de cada uma)".into());
    lines.push(fonte);
    Text::from(lines.join("\n"))
}

/// Corta em `max` caracteres, terminando com `…`.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Atalho das abas: `Ctrl+Shift+F<n>`, na ordem do Registro [`TABS`] — a
/// primeira função é só o agente, a segunda o consumo, a terceira as
/// extensões e a quarta a configuração. As variantes `F13`–`F16` (com Ctrl)
/// e `F25`–`F28` (sem modificadores) cobrem os terminais legados.
fn tab_shortcut(key: KeyEvent) -> Option<Tab> {
    let m = key.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let shift = m.contains(KeyModifiers::SHIFT);
    let alt = m.contains(KeyModifiers::ALT);
    if alt {
        return None;
    }
    let n = match key.code {
        KeyCode::F(n @ 1..=4) if ctrl && shift => n,
        KeyCode::F(n @ 13..=16) if ctrl => n - 12,
        KeyCode::F(n @ 25..=28) => n - 24,
        _ => return None,
    };
    tab_by_shortcut(n)
}

/// Troca de aba pelas setas: `Ctrl+Shift+←/→` em qualquer aba (o `←`/`→`
/// sozinho continua indo para o agente; nas visões ele também troca de aba).
fn tab_step(key: KeyEvent) -> Option<isize> {
    let m = key.modifiers;
    if !m.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Left => Some(-1),
        KeyCode::Right => Some(1),
        _ => None,
    }
}

/// Sub-aba 1–9 pedida pela tecla: `Ctrl+Alt+1`–`9` ou `Alt+1`–`9` (sem o
/// protocolo do kitty, `Ctrl+Alt+dígito` costuma chegar como `Alt+dígito`).
fn sub_tab_number(key: KeyEvent) -> Option<usize> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c @ '1'..='9') if alt => Some(c as usize - '0' as usize),
        // Shift+Alt+1–9 sem o protocolo do kitty: chega como símbolo + Alt (sem
        // Ctrl, para não confundir com AltGr no Windows que chega como Ctrl+Alt).
        KeyCode::Char(c) if alt && !ctrl => "!@#$%^&*(".find(c).map(|i| i + 1),
        _ => None,
    }
}

/// Texto da janela de atalhos (`Ctrl+Alt+K`); as abas saem do Registro
/// [`TABS`], igual ao menu.
fn shortcuts(enhanced: bool) -> String {
    let protocol = if enhanced {
        "Protocolo de teclado do kitty ligado: Ctrl+Shift e Ctrl+Alt chegam distintos e Shift+Tab vai para o agente."
    } else {
        "Este terminal não tem o protocolo de teclado do kitty: Ctrl+Alt+1–9 pode chegar como Alt+1–9."
    };
    let abas = TABS
        .iter()
        .map(|entrada| {
            let nota = if entrada.tab == Tab::Extensions {
                " (só com extensões)"
            } else {
                ""
            };
            format!(
                "  {:<23}{}{nota}",
                entrada.tecla_longa(),
                entrada.tab.nome()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "ABAS
{abas}
  Ctrl+Shift+← / →       aba anterior / próxima (nas visões Consumo e Extensões, ← / → sozinhas)
  clique no menu         abre a aba · Sair encerra

SUB-ABAS (instâncias do agente, visões do consumo, relatórios)
  Ctrl+Alt+Tab           próxima sub-aba (com Shift, a anterior)
  Ctrl+Alt+1–9           vai para a sub-aba (alternativas: Alt+1–9, Shift+Alt+1–9)
  Ctrl+PgUp / Ctrl+PgDn  sub-aba anterior / próxima
  Na aba do agente, Ctrl+Alt+<número> além da última instância abre uma nova.

INSTÂNCIAS DO AGENTE
  Ctrl+Alt+N  ou  F12    nova instância (até {MAX_INSTANCES})
  Ctrl+Alt+W  ou  ⇧F12   fecha a instância aberta
  Ctrl+Alt+R             reinicia a instância aberta

TEXTO E MOUSE
  arrastar               seleciona e copia ao soltar (clique desfaz)
  Ctrl+Alt+C             copia a seleção, a tela do agente ou o conteúdo da aba
  Ctrl+Alt+S             mouse para o terminal (seleção nativa) e de volta
  Shift+arrastar         seleção nativa do terminal, sem desligar o mouse

HISTÓRICO DO AGENTE
  roda do mouse          rola o histórico
  Shift+PgUp / PgDn      meia tela para trás / para frente
  Shift+Home / End       início do histórico / volta ao vivo

ABAS DE CONTEÚDO
  ↑↓ j k PgUp PgDn g     rolar · r recarregar · Esc ou q volta ao agente
  Configurações: ↑↓ campo · ←→/Enter altera · dígitos editam números · Del padrão

GERAL
  Ctrl+Alt+K             esta lista
  F10                    sair (encerra todas as instâncias)

Tab, F1–F9, Ctrl+←→ e as demais teclas seguem para o agente. Os atalhos evitam
os do ambiente Linux (Ctrl+Alt+T/L/D/Del, Ctrl+Alt+F1–F12 e Ctrl+Alt+setas).

{protocol}"
    )
}

/// Texto sem estilos, como é copiado.
fn plain(text: &Text) -> String {
    text.lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Copia para a área de transferência pelo terminal (OSC 52, que funciona
/// também por SSH) e pelo sistema: `clip.exe` no Windows (o console clássico
/// ignora o OSC 52) e `wl-copy`/`xclip`/`xsel` no Linux (o GNOME Terminal
/// também ignora). Devolve se uma ferramenta copiou.
fn clipboard(text: &str) -> bool {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{encoded}\x07");
    let _ = stdout.flush();
    if cfg!(windows) {
        return pipe_to("clip", &[], &utf16le_with_bom(text));
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    let tools: [(&str, &[&str], bool); 3] = [
        ("wl-copy", &[], wayland),
        ("xclip", &["-selection", "clipboard"], x11),
        ("xsel", &["--clipboard", "--input"], x11),
    ];
    tools
        .into_iter()
        .any(|(bin, args, available)| available && pipe_to(bin, args, text.as_bytes()))
}

/// Manda `input` para a entrada de `bin`, sem esperar: as ferramentas do
/// Linux ficam servindo a área de transferência. Devolve se ele iniciou.
fn pipe_to(bin: &str, args: &[&str], input: &[u8]) -> bool {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    thread::spawn(move || {
        let _ = child.wait();
    });
    true
}

/// O `clip.exe` lê a entrada na página de código do console, a não ser que
/// ela venha em UTF-16LE com BOM.
fn utf16le_with_bom(text: &str) -> Vec<u8> {
    [0xFEFF_u16]
        .into_iter()
        .chain(text.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// `1234567` → `1.234.567`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Quantos pedidos de posição do cursor (`ESC[6n`, DSR) há em `bytes`.
fn cursor_queries(bytes: &[u8]) -> usize {
    bytes.windows(4).filter(|w| w == b"\x1b[6n").count()
}

/// Instruções como colagem, sem o Enter (mandado depois, ver
/// [`Agent::deliver_pending`]): com `bracketed`, entre `ESC[200~`/`ESC[201~`;
/// sem, cada quebra de linha vira `Alt+Enter` (não envia no meio).
fn prompt_bytes(instructions: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        return format!("\x1b[200~{instructions}\x1b[201~").into_bytes();
    }
    let mut bytes = Vec::new();
    for (i, line) in instructions.split('\n').enumerate() {
        if i > 0 {
            bytes.extend(b"\x1b\r");
        }
        bytes.extend(line.as_bytes());
    }
    bytes
}

/// Servidor de controle: recebe do MCP `POST /spawn` (subagente),
/// `POST /message` (mensagem para uma instância), `POST /open` e
/// `POST /close` (abrir e fechar instâncias) e `POST /focus` (a janela
/// flutuante pediu a instância) e os repassa ao loop da TUI.
fn control_server(listener: std::net::TcpListener, tx: Sender<Msg>) {
    use std::io::Write;

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let Some((path, body)) = read_request(&mut stream) else {
            continue;
        };
        let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let text = |key: &str| v[key].as_str().unwrap_or("").to_string();
        let msg = match path.as_str() {
            "/spawn" => Msg::SpawnSubagent(SpawnRequest {
                spawn_id: text("spawn_id"),
                task: text("task"),
                cwd: text("cwd"),
                model: v
                    .get("model")
                    .and_then(|m| m.as_str())
                    .unwrap_or("claude-haiku-4-5")
                    .to_string(),
            }),
            "/message" => Msg::Message {
                instance_id: text("instance_id"),
                from: text("from"),
                text: text("text"),
            },
            "/focus" => Msg::Focus {
                instance_id: text("instance_id"),
            },
            "/open" => Msg::Open {
                instance_id: text("instance_id"),
                from: text("from"),
                text: v["text"].as_str().map(str::to_string),
            },
            "/close" => Msg::Close {
                instance_id: text("instance_id"),
                from: text("from"),
            },
            _ => continue,
        };
        if tx.send(msg).is_err() {
            break;
        }
    }
}

/// Lê um pedido HTTP/1.0 inteiro: o caminho e o corpo (até o `Content-Length`).
fn read_request(stream: &mut impl std::io::Read) -> Option<(String, String)> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).ok()?;
        raw.extend_from_slice(&buf[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = std::str::from_utf8(&raw[..end]).ok()?;
            let path = head.split_whitespace().nth(1)?.to_string();
            let length = head
                .lines()
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok());
            let start = end + 4;
            match length {
                Some(len) if raw.len() - start < len && n > 0 => continue,
                Some(len) => raw.truncate((start + len).min(raw.len())),
                None if n > 0 => continue,
                None => {}
            }
            return Some((path, String::from_utf8_lossy(&raw[start..]).into_owned()));
        }
        if n == 0 {
            return None;
        }
    }
}

/// Faz um POST HTTP simples sem esperar a resposta (fire-and-forget).
fn http_post_fire(addr: &str, path: &str, body: &str) {
    http_post(addr, path, body);
}

/// POST sem esperar mais que 500ms; devolve o status HTTP da resposta, ou
/// `None` se o daemon não respondeu (fora do ar).
fn http_post(addr: &str, path: &str, body: &str) -> Option<u16> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(addr).ok()?;
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let request = format!(
        "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        addr,
        body.len(),
        body
    );
    // Espera a resposta antes de fechar: o hyper do daemon trata o fim
    // da conexão como pedido cancelado e descarta o handler, então um
    // `/register` ou `/instances/register` sem resposta lida se perdia.
    stream.write_all(request.as_bytes()).ok()?;
    let mut head = [0; 64];
    let n = stream.read(&mut head).ok()?;
    // `HTTP/1.1 404 Not Found`
    std::str::from_utf8(&head[..n])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Envia o resultado de um subagente de volta ao daemon MCP.
fn send_spawn_result(mcp_port: u16, spawn_id: &str, output: &str, exit_code: i32) {
    let body = serde_json::json!({
        "spawn_id": spawn_id,
        "output": output,
        "exit_code": exit_code,
    })
    .to_string();
    let addr = format!("127.0.0.1:{mcp_port}");
    // Envia em thread separada para não bloquear o loop da TUI.
    thread::spawn(move || {
        http_post_fire(&addr, "/spawn-result", &body);
    });
}

/// Área do agente: a tela menos a linha do menu, a das sub-abas e o rodapé.
fn body_size(area: Rect) -> Rect {
    Rect::new(
        0,
        0,
        area.width.max(1),
        area.height.saturating_sub(3).max(1),
    )
}

/// Rolagem de um passo da roda do mouse.
fn wheel(scroll: u16, up: bool) -> u16 {
    if up {
        scroll.saturating_sub(WHEEL_LINES)
    } else {
        scroll.saturating_add(WHEEL_LINES)
    }
}

/// Evento da roda do mouse (botões 64/65) no protocolo que o agente pediu;
/// `col`/`row` contam a partir de 0 no pseudoterminal.
fn wheel_bytes(up: bool, col: u16, row: u16, encoding: vt100::MouseProtocolEncoding) -> Vec<u8> {
    let button: u32 = if up { 64 } else { 65 };
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    match encoding {
        vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}M").into_bytes(),
        vt100::MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for value in [button, x, y] {
                let c = char::from_u32(32 + value).unwrap_or(' ');
                out.extend(c.to_string().as_bytes());
            }
            out
        }
        vt100::MouseProtocolEncoding::Default => {
            let mut out = b"\x1b[M".to_vec();
            out.extend([button, x, y].map(|value| (32 + value).min(255) as u8));
            out
        }
    }
}

fn pty_size(body: Rect) -> PtySize {
    PtySize {
        rows: body.height,
        cols: body.width,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Converte o `Command` montado pelo core para o formato do pseudoterminal.
fn builder(command: &Command, cwd: &std::path::Path) -> CommandBuilder {
    let mut builder = CommandBuilder::new(command.get_program());
    builder.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => builder.env(key, value),
            None => builder.env_remove(key),
        }
    }
    builder.cwd(cwd);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder
}

/// O mesmo `Command` do core com a entrada e as saídas em pipes, para o
/// modo Middleware; `ignored` são argumentos que o modo não quer (ex.: o
/// `--standalone` do OpenCode, que o `acp` recusa).
fn piped(command: &Command, cwd: &std::path::Path, ignored: &[&str]) -> Command {
    let mut piped = Command::new(command.get_program());
    piped.args(
        command
            .get_args()
            .filter(|arg| !ignored.contains(&arg.to_str().unwrap_or_default())),
    );
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => piped.env(key, value),
            None => piped.env_remove(key),
        };
    }
    piped
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    piped
}

/// Extrai o valor de `--effort` do comando, se presente.
fn effort_from_command(command: &Command) -> String {
    let args: Vec<_> = command.get_args().collect();
    for pair in args.windows(2) {
        if pair[0] == "--effort" {
            return pair[1].to_string_lossy().into_owned();
        }
    }
    String::new()
}

/// `Ctrl+Alt` com um caractere que não é letra é AltGr no Windows (`@`, `\`,
/// `{`, `/` no ABNT2…): Ctrl+Alt+letra continua sendo atalho.
fn is_altgr(c: char, ctrl: bool, alt: bool) -> bool {
    ctrl && alt && !c.is_ascii_alphabetic()
}

/// Bytes que um terminal xterm enviaria para a tecla.
fn key_bytes(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // Parâmetro xterm de modificadores: 1 + shift + 2·alt + 4·ctrl.
    let modifier = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    let cursor = |c: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let bytes = match key.code {
        // AltGr no Windows chega como Ctrl+Alt com o caractere já pronto.
        KeyCode::Char(c) if cfg!(windows) && is_altgr(c, ctrl, alt) => {
            return c.to_string().into_bytes();
        }
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![c as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            ']' | '5' => vec![0x1d],
            '^' | '6' => vec![0x1e],
            '_' | '-' | '7' => vec![0x1f],
            _ => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift+Enter quebra a linha no Claude Code, como Alt+Enter.
        KeyCode::Enter if shift => return b"\x1b\r".to_vec(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return cursor('A'),
        KeyCode::Down => return cursor('B'),
        KeyCode::Right => return cursor('C'),
        KeyCode::Left => return cursor('D'),
        KeyCode::Home => return cursor('H'),
        KeyCode::End => return cursor('F'),
        KeyCode::Insert => return tilde(2),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let c = (b'P' + n - 1) as char;
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{c}").into_bytes()
            } else {
                format!("\x1bO{c}").into_bytes()
            };
        }
        KeyCode::F(n @ 5..=12) => return tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        _ => return Vec::new(),
    };

    if alt {
        [&[0x1b][..], &bytes].concat()
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_post_returns_the_status_or_none_when_offline() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 512]);
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
        });
        assert_eq!(http_post(&addr, "/heartbeat", "{}"), Some(404));
        server.join().unwrap();
        // A porta foi fechada junto com o listener.
        assert_eq!(http_post(&addr, "/heartbeat", "{}"), None);
    }

    #[test]
    fn clip_input_is_utf16le_with_bom() {
        assert_eq!(utf16le_with_bom("aç"), [0xFF, 0xFE, b'a', 0, 0xE7, 0]);
    }

    #[test]
    fn piped_drops_the_args_the_middleware_does_not_want() {
        // O `--standalone` é do `opencode run`; o `opencode acp` recusa a flag.
        let mut command = std::process::Command::new("opencode");
        command.args(["--standalone", "acp"]);
        let piped = piped(&command, std::path::Path::new("."), &["--standalone"]);
        let args: Vec<String> = piped
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["acp"]);
    }

    #[test]
    fn altgr_symbols_are_not_control_keys() {
        assert!(is_altgr('@', true, true));
        assert!(is_altgr('\\', true, true));
        assert!(is_altgr('€', true, true));
        assert!(!is_altgr('n', true, true));
        assert!(!is_altgr('@', true, false));
        assert!(!is_altgr('@', false, true));
    }

    #[test]
    fn wheel_bytes_follow_the_agent_encoding() {
        use vt100::MouseProtocolEncoding::{Default, Sgr, Utf8};
        assert_eq!(wheel_bytes(true, 0, 0, Sgr), b"\x1b[<64;1;1M");
        assert_eq!(wheel_bytes(false, 9, 4, Sgr), b"\x1b[<65;10;5M");
        assert_eq!(wheel_bytes(true, 0, 0, Default), b"\x1b[M`!!");
        assert_eq!(wheel_bytes(false, 300, 0, Default), b"\x1b[Ma\xff!");
        assert_eq!(wheel_bytes(true, 0, 0, Utf8), b"\x1b[M`!!");
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        key_bytes(KeyEvent::new(code, modifiers), false)
    }

    #[test]
    fn plain_and_control_keys() {
        assert_eq!(key(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(key(KeyCode::Char('ç'), KeyModifiers::NONE), "ç".as_bytes());
        assert_eq!(key(KeyCode::Char('c'), KeyModifiers::CONTROL), [3]);
        assert_eq!(key(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        assert_eq!(key(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(key(KeyCode::Enter, KeyModifiers::SHIFT), b"\x1b\r");
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::NONE), [0x7f]);
        assert_eq!(key(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
    }

    #[test]
    fn cursor_and_function_keys() {
        assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(key(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D");
        assert_eq!(key(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(key(KeyCode::F(4), KeyModifiers::NONE), b"\x1bOS");
        assert_eq!(key(KeyCode::F(12), KeyModifiers::NONE), b"\x1b[24~");
    }

    #[test]
    fn tabs_list_agent_then_usage_and_extensions() {
        assert_eq!(tabs(&[]), [Tab::Agent, Tab::Usage, Tab::Config]);
        // O RTK fica no consumo: sozinho, não abre a aba Extensões.
        assert_eq!(
            tabs(&[Extension::Rtk]),
            [Tab::Agent, Tab::Usage, Tab::Config]
        );
        assert_eq!(
            tabs(&[Extension::Rtk, Extension::AiMemory]),
            [Tab::Agent, Tab::Usage, Tab::Extensions, Tab::Config]
        );
        let both = [Extension::Rtk, Extension::AiMemory];
        assert_eq!(Tab::Extensions.views(&both)[0], ("AI Memory", "Status"));
        let usage = Tab::Usage.views(&both);
        assert_eq!(
            usage.len(),
            USAGE_VIEWS.len() + Extension::Rtk.reports().len()
        );
        assert_eq!(usage[USAGE_VIEWS.len()], ("RTK", "Economia"));
        assert_eq!(Tab::Usage.views(&[]).len(), USAGE_VIEWS.len());
        assert!(Tab::Config.views(&both).is_empty());
        assert_eq!(Tab::Agent.label("claude"), "Agente (claude)");
        assert_eq!(Tab::Config.label("claude"), "Configurações");
    }

    #[test]
    fn ctrl_shift_arrows_step_tabs() {
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(tab_step(KeyEvent::new(KeyCode::Left, cs)), Some(-1));
        assert_eq!(tab_step(KeyEvent::new(KeyCode::Right, cs)), Some(1));
        // Setas sozinhas, com Ctrl (palavra) ou Ctrl+Alt (desktop) seguem adiante.
        assert_eq!(
            tab_step(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            tab_step(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            tab_step(KeyEvent::new(KeyCode::Left, cs | KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn registry_is_the_single_source_of_the_tabs() {
        // Menu, nome e comportamento vêm todos do Registro `TABS`.
        let todas = [Extension::Rtk, Extension::AiMemory];
        assert_eq!(
            tabs(&todas),
            [Tab::Agent, Tab::Usage, Tab::Extensions, Tab::Config]
        );
        let mut atalhos = Vec::new();
        for tab in tabs(&todas) {
            let entrada = spec(tab);
            assert!(
                !atalhos.contains(&entrada.shortcut),
                "atalho repetido: Ctrl+Shift+F{}",
                entrada.shortcut
            );
            atalhos.push(entrada.shortcut);
            // O menu mostra o atalho que esta mesma entrada abre.
            assert_eq!(
                tab_by_shortcut(entrada.shortcut),
                Some(tab),
                "Ctrl+Shift+F{} não devolve {}",
                entrada.shortcut,
                tab.nome()
            );
            assert_eq!(entrada.tab, tab);
        }
        // F1 só o agente, F2 o consumo, F3 as extensões e F4 a configuração.
        assert_eq!(atalhos, [1, 2, 3, 4]);
        assert_eq!(spec(Tab::Agent).kind, TabKind::Agent);
        assert_eq!(spec(Tab::Config).kind, TabKind::Form);
        assert_eq!(spec(Tab::Usage).kind, TabKind::Content);
        assert!(spec(Tab::Usage).refresh);
        assert!(!spec(Tab::Extensions).refresh);
        assert_eq!(spec(Tab::Usage).own_views.len(), USAGE_VIEWS.len());
        assert_eq!(spec(Tab::Agent).own_views, &[] as &[&str]);
    }

    #[test]
    fn help_lists_every_registered_tab() {
        let text = shortcuts(true);
        for entrada in &TABS {
            // A ajuda mostra o mesmo atalho do menu e da abertura da aba.
            assert!(
                text.contains(&entrada.tecla_longa()),
                "ajuda sem {}",
                entrada.tecla_longa()
            );
            assert!(
                text.contains(entrada.tab.nome()),
                "ajuda sem {}",
                entrada.tab.nome()
            );
        }
        // A terceira função é só as extensões (marcada) e a quarta a configuração.
        assert!(text.contains("Extensões (só com extensões)"));
        assert!(text.contains(Tab::Config.nome()));
        assert!(!text.contains("terceira função"));
    }

    #[test]
    fn shortcut_keys_pick_tabs_and_sub_tabs() {
        let t = |code, m| tab_shortcut(KeyEvent::new(code, m));
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        // Cada função abre a aba da entrada dela no Registro: F1 é o agente
        // (só o agente), F2 o consumo, F3 as extensões e F4 a configuração.
        assert_eq!(t(KeyCode::F(1), ctrl_shift), Some(Tab::Agent));
        assert_eq!(t(KeyCode::F(2), ctrl_shift), Some(Tab::Usage));
        assert_eq!(t(KeyCode::F(3), ctrl_shift), Some(Tab::Extensions));
        assert_eq!(t(KeyCode::F(4), ctrl_shift), Some(Tab::Config));
        assert_eq!(t(KeyCode::F(5), ctrl_shift), None);
        // F13..16 com Ctrl (terminais legados)
        assert_eq!(t(KeyCode::F(13), KeyModifiers::CONTROL), Some(Tab::Agent));
        assert_eq!(t(KeyCode::F(14), KeyModifiers::CONTROL), Some(Tab::Usage));
        assert_eq!(
            t(KeyCode::F(15), KeyModifiers::CONTROL),
            Some(Tab::Extensions)
        );
        assert_eq!(t(KeyCode::F(16), KeyModifiers::CONTROL), Some(Tab::Config));
        assert_eq!(t(KeyCode::F(25), KeyModifiers::NONE), Some(Tab::Agent));
        assert_eq!(t(KeyCode::F(26), KeyModifiers::NONE), Some(Tab::Usage));
        assert_eq!(t(KeyCode::F(27), KeyModifiers::NONE), Some(Tab::Extensions));
        assert_eq!(t(KeyCode::F(28), KeyModifiers::NONE), Some(Tab::Config));

        // Atalhos desativados: F1..F4 puros, Shift+F1..F4, dígitos, símbolos
        assert_eq!(t(KeyCode::F(1), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::F(2), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::F(3), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::F(4), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::F(1), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::F(2), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::F(3), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::F(4), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::F(13), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::Char('1'), ctrl_shift), None);
        assert_eq!(t(KeyCode::Char('2'), ctrl_shift), None);
        assert_eq!(t(KeyCode::Char('3'), ctrl_shift), None);
        assert_eq!(t(KeyCode::Char('4'), ctrl_shift), None);
        assert_eq!(t(KeyCode::Char('1'), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::Char('2'), KeyModifiers::SHIFT), None);
        assert_eq!(t(KeyCode::Char('!'), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::Char('@'), KeyModifiers::NONE), None);
        assert_eq!(t(KeyCode::Char('!'), ctrl_shift), None);

        let s = |c, m| sub_tab_number(KeyEvent::new(KeyCode::Char(c), m));
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let shift_alt = KeyModifiers::SHIFT | KeyModifiers::ALT;
        assert_eq!(s('2', ctrl_alt), Some(2));
        assert_eq!(s('9', KeyModifiers::ALT), Some(9));
        assert_eq!(s('0', KeyModifiers::ALT), None);
        assert_eq!(s('2', KeyModifiers::NONE), None);
        // AltGr no Windows chega como Ctrl+Alt com o símbolo: não é sub-aba.
        assert_eq!(s('@', ctrl_alt), None);
        // Shift+Alt+1–9 sem kitty: chega como símbolo + Alt.
        assert_eq!(s('!', KeyModifiers::ALT), Some(1));
        assert_eq!(s('@', KeyModifiers::ALT), Some(2));
        assert_eq!(s('#', KeyModifiers::ALT), Some(3));
        // Shift+Alt+1 com kitty: chega como dígito + Shift+Alt.
        assert_eq!(s('1', shift_alt), Some(1));
        assert_eq!(s('3', shift_alt), Some(3));
    }

    #[test]
    fn selections_cover_cells_in_reading_order() {
        let selection = Selection {
            agent: 1,
            anchor: (2, 5),
            head: (0, 3),
            dragging: false,
        };
        assert_eq!(selection.range(), ((0, 3), (2, 5)));
        assert!(!selection.contains(0, 2));
        assert!(selection.contains(0, 3));
        assert!(selection.contains(1, 0));
        assert!(selection.contains(2, 5));
        assert!(!selection.contains(2, 6));
        assert!(!selection.contains(3, 0));

        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"hello\r\nworld");
        let selection = Selection {
            agent: 1,
            anchor: (0, 1),
            head: (1, 2),
            dragging: false,
        };
        assert_eq!(selection.text(parser.screen()), "ello\nwor");
    }

    #[test]
    fn config_fields_set_and_reset() {
        let mut config = Config::default();
        config.tui.mouse = false;
        config.mcp.ui = true;
        config.tui.middleware = true;
        assert_eq!(Field::Mouse.value(&config), "não");
        assert_eq!(Field::Middleware.value(&config), "sim");
        assert_eq!(Field::Middleware.section(), Section::Agent);

        Field::BudgetFiveHours.set(&mut config, "300.000").unwrap();
        assert_eq!(Field::BudgetFiveHours.value(&config), "300.000");
        assert_eq!(Field::BudgetDay.value(&config), "1.440.000 (calculado)");
        assert_eq!(Field::BudgetWeek.value(&config), "10.080.000 (calculado)");
        assert!(Field::McpMax.set(&mut config, "abc").is_err());
        Field::McpMax.set(&mut config, "0").unwrap();
        assert_eq!(config.mcp.max_simultaneous_subagents, 1);

        let form = Form {
            config,
            space: None,
            prefs: Preferences::default(),
            accounts: Vec::new(),
            custom: true,
            index: 0,
            editing: None,
            router: None,
            message: None,
        };
        for field in form.fields() {
            field.reset(&mut config);
        }
        assert_eq!(config, Config::default());
    }

    #[test]
    fn router_switches_only_to_another_agent() {
        let mut form = Form {
            config: Config::default(),
            space: None,
            prefs: Preferences::default(),
            accounts: Vec::new(),
            custom: false,
            index: 0,
            editing: None,
            router: None,
            message: None,
        };
        assert!(!form.leave_router());
        // Voltou ao agente de antes: nada a trocar.
        form.router = Some(form.config.provider);
        assert!(!form.leave_router());
        assert_eq!(form.router, None);
        form.router = Some(Provider::OpenCode);
        form.config.provider = Provider::Claude;
        assert!(form.leave_router());
        assert_eq!(form.router, None);
    }

    #[test]
    fn plain_text_drops_styles() {
        let text = Text::from(vec![
            Line::from(vec![Span::raw("a").bold(), Span::raw("b")]),
            Line::raw("c"),
        ]);
        assert_eq!(plain(&text), "ab\nc");
    }

    #[test]
    fn sub_tabs_share_one_style() {
        let text = |spans: Vec<Span>| {
            spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        let instances = [("", "claude 1".to_string()), ("", "claude 2".to_string())];
        assert_eq!(
            text(sub_tab_spans(&instances, 1, "⌃⌥")),
            " ⌃⌥1 claude 1   ⌃⌥2 claude 2  "
        );
        let reports = [
            ("RTK", "gain".to_string()),
            ("AI Memory", "Status".to_string()),
        ];
        let spans = sub_tab_spans(&reports, 0, "⌥");
        assert_eq!(
            text(spans.clone()),
            "RTK  ⌥1 gain  │ AI Memory  ⌥2 Status  "
        );
        assert_eq!(spans[1].style.bg, Some(Color::Cyan));
    }

    #[test]
    fn durations_and_truncation() {
        assert_eq!(duration(59), "0min");
        assert_eq!(duration(9000), "2h30min");
        assert_eq!(duration(200_000), "2d 7h");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn spaces_view_lists_space_and_agent_rows() {
        let text = |totals: &[SpaceUsage]| {
            format_spaces(totals)
                .lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(text(&[])[0], "Nenhuma sessão gravada ainda.");
        let row = SpaceUsage {
            agent: "claude".into(),
            sessions: 2,
            total: Tokens {
                input: 1500,
                ..Tokens::default()
            },
            ..SpaceUsage::default()
        };
        let lines = text(&[row]);
        assert!(lines[0].starts_with("Space"));
        assert!(lines[1].starts_with("(fora de space)"));
        assert!(lines[1].contains("claude") && lines[1].contains("1.500"));
    }

    #[test]
    fn thousands_uses_dots() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1.000");
        assert_eq!(thousands(1_234_567), "1.234.567");
    }

    #[test]
    fn body_leaves_room_for_the_menu_and_instances() {
        assert_eq!(body_size(Rect::new(0, 0, 80, 24)), Rect::new(0, 0, 80, 21));
        assert_eq!(body_size(Rect::new(0, 0, 0, 0)), Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn reads_the_whole_control_request() {
        let body = r#"{"instance_id":"i1","text":"oi"}"#;
        let raw = format!(
            "POST /message HTTP/1.0\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        // Chega em pedaços pequenos, como num socket.
        let mut chunks = std::io::Cursor::new(raw.into_bytes());
        let mut slow = std::io::Read::take(&mut chunks, u64::MAX);
        let (path, read) = read_request(&mut slow).unwrap();
        assert_eq!(path, "/message");
        assert_eq!(read, body);
        assert!(read_request(&mut std::io::Cursor::new(b"lixo".to_vec())).is_none());
    }

    #[test]
    fn counts_cursor_position_queries() {
        // O ConPTY abre com `ESC[6n` e espera a resposta.
        assert_eq!(cursor_queries(b"\x1b[6n"), 1);
        assert_eq!(cursor_queries(b"oi\x1b[6n\x1b[?25l\x1b[6n"), 2);
        assert_eq!(cursor_queries(b"\x1b[5n\x1b[6"), 0);
    }

    #[test]
    fn prompt_is_pasted_without_enter() {
        // O Enter vai separado; sem bracketed paste, quebra de linha → Alt+Enter.
        assert_eq!(prompt_bytes("olá", false), "olá".as_bytes());
        assert_eq!(prompt_bytes("l1\nl2", false), b"l1\x1b\rl2");
        assert_eq!(prompt_bytes("l1\n\nl3", false), b"l1\x1b\r\x1b\rl3");
        assert_eq!(prompt_bytes("l1\nl2", true), b"\x1b[200~l1\nl2\x1b[201~");
    }

    #[test]
    fn form_prefers_space_preferences_for_agent_fields() {
        let form = Form {
            config: Config {
                provider: Provider::Claude,
                mode: Mode::Eco,
                jail: false,
                ..Config::default()
            },
            space: Some("web".into()),
            prefs: Preferences {
                agent: Some(Provider::OpenCode),
                mode: None,
                jail: Some(true),
                account: None,
            },
            accounts: Vec::new(),
            custom: false,
            index: 0,
            editing: None,
            router: None,
            message: None,
        };
        // As preferências do space vencem o config.toml, que completa o resto.
        let effective = form.effective();
        assert_eq!(effective.provider, Provider::OpenCode);
        assert_eq!(effective.mode, Mode::Eco);
        assert!(effective.jail);
        // RTK + AI Jail não é preset: o modo aparece como custom.
        assert_eq!(form.preset(), CliMode::Custom);
        // Só agente, modo e conta vão para as preferências.
        assert!(form.in_space(Field::Provider));
        assert!(form.in_space(Field::Mode));
        assert!(form.in_space(Field::Extension(Extension::Rtk)));
        assert!(form.in_space(Field::Account));
        assert!(!form.in_space(Field::BudgetFiveHours));
        assert!(!form.in_space(Field::Pet));
        // Fora de space, tudo continua no config.toml.
        let global = Form {
            space: None,
            prefs: Preferences::default(),
            ..form
        };
        assert!(!global.in_space(Field::Provider));
        assert_eq!(global.effective().provider, Provider::Claude);
        assert_eq!(global.preset(), CliMode::Eco);
        assert_eq!(global.account_display(), "default");
    }

    #[test]
    fn form_shows_custom_boxes_only_in_custom_mode() {
        let mut form = Form {
            config: Config::default(),
            space: None,
            prefs: Preferences::default(),
            accounts: Vec::new(),
            custom: false,
            index: 0,
            editing: None,
            router: None,
            message: None,
        };
        assert!(!form.fields().contains(&Field::Extension(Extension::Rtk)));
        form.custom = true;
        assert!(form.fields().contains(&Field::Extension(Extension::Rtk)));
        assert!(
            form.fields()
                .contains(&Field::Extension(Extension::AiMemory))
        );
    }

    #[test]
    fn budget_follows_the_five_hour_window() {
        let mut config = Config::default();
        Field::BudgetFiveHours
            .set(&mut config, "1.000.000")
            .unwrap();
        assert_eq!(config.budget.five_hours, Some(1_000_000));
        assert_eq!(budget_for(1_000_000, 24), 4_800_000);
        assert_eq!(config.budget.week, Some(33_600_000));
        Field::BudgetFiveHours.set(&mut config, "").unwrap();
        assert_eq!(config.budget.five_hours, None);
        assert_eq!(config.budget.week, None);
    }
}
