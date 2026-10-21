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
//! uma) e `Ctrl+Shift+F4` (Config). Sub-abas, iguais em todas as abas:
//! `Ctrl+Alt+Tab` passa para a próxima e `Ctrl+Alt+1`–`9` vai direto
//! (`Alt+1`–`9`, `Shift+Alt+1`–`9`, `Ctrl+PgUp/PgDn` e `←`/`→` nas visões
//! também); na aba do agente, um número além da última abre outra instância.
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
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::{Frame, crossterm};
use terraforme_core::accounts::ClaudeAccounts;
use terraforme_core::agent::{Adapter, Mode, Provider};
use terraforme_core::config::Config;
use terraforme_core::extension::{Extension, Report};
use terraforme_core::limits::{self, Limit, Source};
use terraforme_core::space::Preferences;
use terraforme_core::usage::{self, Summary, Tokens, UsageSource};
use tui_term::widget::PseudoTerminal;

/// Linhas de histórico guardadas da saída do agente.
const SCROLLBACK: usize = 5000;
/// Intervalo de atualização automática da aba de consumo.
const USAGE_REFRESH: Duration = Duration::from_secs(5);
/// Sub-abas da aba de consumo.
const USAGE_VIEWS: [&str; 3] = ["Limites", "Tokens", "Subagentes"];
/// Largura da barra de progresso dos limites.
const BAR_WIDTH: usize = 40;
/// Máximo de instâncias do agente abertas ao mesmo tempo.
pub const MAX_INSTANCES: usize = 9;
/// Espera até entregar um prompt pendente à instância: a TUI do agente
/// precisa estar pronta para recebê-lo.
const PROMPT_DELAY: Duration = Duration::from_secs(2);
/// Espera entre ver o agente encerrado e fechar a instância, para a saída
/// que ainda está a caminho chegar (ver [`Agent::exited`]).
const EXIT_GRACE: Duration = Duration::from_millis(500);
/// Linhas roladas por passo da roda do mouse.
const WHEEL_LINES: u16 = 3;
/// Linhas acima do pseudoterminal do agente: o menu e as sub-abas.
const AGENT_TOP: u16 = 2;
/// Tempo que a mensagem de status (ex.: "copiado") fica na tela.
const STATUS_TTL: Duration = Duration::from_secs(3);

/// O que `run` precisa para montar o terminal.
pub struct Options {
    /// Nome da aba do agente (ex.: `claude`).
    pub agent: String,
    /// Texto à direita do menu (provider e modo).
    pub title: String,
    /// Integração com o agente (sessão e consumo).
    pub adapter: &'static dyn Adapter,
    /// Comando de cada instância do agente.
    pub command: Command,
    /// Instâncias abertas ao iniciar.
    pub instances: usize,
    /// Passa um id de sessão próprio a cada instância, para o consumo saber
    /// qual é a sessão dela; desligado quando os argumentos já escolhem a sessão.
    pub session_ids: bool,
    /// Aparecem num aviso inicial, já que a tela alternativa esconderia o
    /// que fosse impresso antes.
    pub notices: Vec<String>,
    /// Extensões do modo; os relatórios delas ficam na aba Extensões.
    pub extensions: Vec<Extension>,
    /// Lock file do daemon MCP; `None` se o MCP não está disponível.
    pub mcp_lock: Option<terraforme_core::mcp::LockFile>,
    /// Id único desta instância da CLI para o registro no MCP.
    pub client_id: String,
    /// Space da pasta atual, mostrado no apelido das instâncias no MCP.
    pub space: Option<String>,
}

/// Ligação do terminal com o daemon MCP: cada instância se registra nele (e
/// some ao fechar) e recebe as mensagens das outras pelo servidor de controle.
struct Link {
    port: u16,
    client_id: String,
    /// Endereço do servidor de controle deste terminal.
    control_addr: String,
    agent: String,
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
    /// Nome da aba: o agente usa o nome dele (`claude`, `opencode`).
    fn label(self, agent: &str) -> String {
        match self {
            Tab::Agent => agent.to_string(),
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
            Tab::Config => "Config",
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
    /// Atalho como tecla do menu (`⌃⇧F4`), sempre igual ao que abre a aba.
    fn tecla(&self) -> String {
        format!("⌃⇧F{}", self.shortcut)
    }

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

/// Campos da aba Config, na ordem da tela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Provider,
    Mode,
    Jail,
    /// Conta do Claude Code: nome registrado em `terraforme config --account`.
    /// Só vale no contexto de um space (grava em `preferences.toml`).
    Account,
    BudgetFiveHours,
    BudgetWeek,
    Mouse,
    CopyOnSelect,
    McpGrace,
    McpTimeout,
    McpMax,
    McpUi,
}

const FIELDS: [Field; 12] = [
    Field::Provider,
    Field::Mode,
    Field::Jail,
    Field::Account,
    Field::BudgetFiveHours,
    Field::BudgetWeek,
    Field::Mouse,
    Field::CopyOnSelect,
    Field::McpGrace,
    Field::McpTimeout,
    Field::McpMax,
    Field::McpUi,
];

const MODES: [Mode; 4] = [Mode::None, Mode::Eco, Mode::High, Mode::Hybrid];

impl Field {
    /// Seção da tela em que o campo aparece.
    fn group(self) -> &'static str {
        match self {
            Field::Provider | Field::Mode | Field::Jail | Field::Account => {
                "Início do agente (vale no próximo terraforme cli)"
            }
            Field::BudgetFiveHours | Field::BudgetWeek => "Orçamento de tokens (aba Consumo)",
            Field::Mouse | Field::CopyOnSelect => "TUI",
            Field::McpGrace | Field::McpTimeout | Field::McpMax | Field::McpUi => "Daemon MCP",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Field::Provider => "Agente",
            Field::Mode => "Modo",
            Field::Jail => "AI Jail (sandbox)",
            Field::Account => "Conta Claude Code",
            Field::BudgetFiveHours => "Tokens em 5h",
            Field::BudgetWeek => "Tokens em 7 dias",
            Field::Mouse => "Capturar o mouse",
            Field::CopyOnSelect => "Copiar ao selecionar",
            Field::McpGrace => "Carência ao sair (s)",
            Field::McpTimeout => "Timeout do subagente (s)",
            Field::McpMax => "Subagentes simultâneos",
            Field::McpUi => "UI web das instâncias",
        }
    }

    /// Campo editado digitando um número (os outros alternam valores).
    fn numeric(self) -> bool {
        matches!(
            self,
            Field::BudgetFiveHours
                | Field::BudgetWeek
                | Field::McpGrace
                | Field::McpTimeout
                | Field::McpMax
        )
    }

    fn value(self, config: &Config) -> String {
        let yes = |on: bool| if on { "sim" } else { "não" }.to_string();
        let tokens = |n: Option<u64>| n.map_or("sem orçamento".into(), thousands);
        match self {
            Field::Provider => config.provider.name().into(),
            Field::Mode => format!(
                "{} ({})",
                config.mode.id(),
                match config.mode {
                    Mode::None => "sem extensões",
                    Mode::Eco => "RTK",
                    Mode::High => "AI Memory",
                    Mode::Hybrid => "RTK + AI Memory",
                }
            ),
            Field::Jail => yes(config.jail),
            // Account é resolvido em Form::account_display(); nunca chamado por este caminho.
            Field::Account => String::new(),
            Field::BudgetFiveHours => tokens(config.budget.five_hours),
            Field::BudgetWeek => tokens(config.budget.week),
            Field::Mouse => yes(config.tui.mouse),
            Field::CopyOnSelect => yes(config.tui.copy_on_select),
            Field::McpGrace => config.mcp.grace_seconds.to_string(),
            Field::McpTimeout => config.mcp.subagent_timeout_seconds.to_string(),
            Field::McpMax => config.mcp.max_simultaneous_subagents.to_string(),
            // Ligada com o daemon no ar: mostra o endereço.
            Field::McpUi => match terraforme_core::mcp::LockFile::read() {
                Ok(Some(lock)) if config.mcp.ui => {
                    format!("sim · {}", terraforme_core::mcp::ui_url(&lock))
                }
                _ => yes(config.mcp.ui),
            },
        }
    }

    /// Passa ao valor seguinte (ou anterior) dos campos que não são números.
    fn step(self, config: &mut Config, delta: isize) {
        match self {
            Field::Provider => {
                let providers = implemented_providers();
                let i = providers
                    .iter()
                    .position(|&p| p == config.provider)
                    .unwrap_or(0);
                config.provider = providers[step_index(providers.len(), i, delta)];
            }
            Field::Mode => {
                let i = MODES.iter().position(|&m| m == config.mode).unwrap_or(0);
                config.mode = MODES[step_index(MODES.len(), i, delta)];
            }
            Field::Jail => config.jail = !config.jail,
            Field::Mouse => config.tui.mouse = !config.tui.mouse,
            Field::CopyOnSelect => config.tui.copy_on_select = !config.tui.copy_on_select,
            Field::McpUi => config.mcp.ui = !config.mcp.ui,
            _ => {}
        }
    }

    /// Passa ao valor seguinte nas preferências do space. `current` são os
    /// valores efetivos, para a sequência continuar de onde a tela mostra.
    fn step_prefs(self, prefs: &mut Preferences, current: &Config, delta: isize) {
        match self {
            Field::Provider => {
                let providers = implemented_providers();
                let i = providers
                    .iter()
                    .position(|&p| p == current.provider)
                    .unwrap_or(0);
                prefs.agent = Some(providers[step_index(providers.len(), i, delta)]);
            }
            Field::Mode => {
                let i = MODES.iter().position(|&m| m == current.mode).unwrap_or(0);
                prefs.mode = Some(MODES[step_index(MODES.len(), i, delta)]);
            }
            Field::Jail => prefs.jail = Some(!current.jail),
            _ => {}
        }
    }

    /// Tira o campo das preferências do space: ele volta a valer o que está
    /// no `config.toml`.
    fn reset_prefs(self, prefs: &mut Preferences) {
        match self {
            Field::Provider => prefs.agent = None,
            Field::Mode => prefs.mode = None,
            Field::Jail => prefs.jail = None,
            Field::Account => prefs.account = None,
            _ => {}
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
            Field::BudgetFiveHours => config.budget.five_hours = n,
            Field::BudgetWeek => config.budget.week = n,
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

    /// Volta o campo ao valor padrão.
    fn reset(self, config: &mut Config) {
        let default = Config::default();
        match self {
            Field::Provider => config.provider = default.provider,
            Field::Mode => config.mode = default.mode,
            Field::Jail => config.jail = default.jail,
            // Account não existe no config.toml (só em preferências de space).
            Field::Account => {}
            Field::BudgetFiveHours => config.budget.five_hours = None,
            Field::BudgetWeek => config.budget.week = None,
            Field::Mouse => config.tui.mouse = default.tui.mouse,
            Field::CopyOnSelect => config.tui.copy_on_select = default.tui.copy_on_select,
            Field::McpGrace => config.mcp.grace_seconds = default.mcp.grace_seconds,
            Field::McpTimeout => {
                config.mcp.subagent_timeout_seconds = default.mcp.subagent_timeout_seconds
            }
            Field::McpMax => {
                config.mcp.max_simultaneous_subagents = default.mcp.max_simultaneous_subagents
            }
            Field::McpUi => config.mcp.ui = default.mcp.ui,
        }
    }
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

/// Estado da aba Config.
struct Form {
    config: Config,
    /// Nome do space quando agente, modo, jail e conta gravam nas preferências
    /// dele (`None`: tudo fica no `config.toml`).
    space: Option<String>,
    /// Preferências do space (vazias fora dele: tudo vem do `config.toml`).
    prefs: Preferences,
    /// Nomes das contas registradas em `claude.toml`, para o campo Account ciclar.
    accounts: Vec<String>,
    /// Campo escolhido em [`FIELDS`].
    index: usize,
    /// Número sendo digitado no campo escolhido.
    editing: Option<String>,
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
        Form {
            config,
            space,
            prefs,
            accounts,
            index: 0,
            editing: None,
            message,
        }
    }

    fn field(&self) -> Field {
        FIELDS[self.index]
    }

    /// Agente, modo, jail e conta gravam nas preferências do space quando há
    /// um; os outros campos são sempre do `config.toml`.
    fn in_space(&self, field: Field) -> bool {
        self.space.is_some()
            && matches!(
                field,
                Field::Provider | Field::Mode | Field::Jail | Field::Account
            )
    }

    /// Valor exibido para o campo Account: nome da conta nas preferências do
    /// space, ou indicação de que está fora de um space (campo inativo).
    fn account_display(&self) -> String {
        if self.space.is_none() {
            return "— (fora de space)".into();
        }
        self.prefs
            .account
            .as_deref()
            .unwrap_or("padrão")
            .to_string()
    }

    /// Avança (ou retrocede) a conta nas preferências do space, ciclando pela
    /// lista de contas registradas mais a opção "padrão" (sem conta fixa).
    fn step_account(&mut self, delta: isize) {
        if self.accounts.is_empty() {
            self.message = Some(Err(
                "nenhuma conta registrada — use `terraforme config --account NOME=PASTA`".into(),
            ));
            return;
        }
        // Opções: [None/"padrão", "conta1", "conta2", ...]
        let len = self.accounts.len() + 1; // +1 para o "padrão"
        let current = self
            .prefs
            .account
            .as_deref()
            .and_then(|name| self.accounts.iter().position(|a| a == name).map(|i| i + 1))
            .unwrap_or(0);
        let next = step_index(len, current, delta);
        self.prefs.account = if next == 0 {
            None
        } else {
            Some(self.accounts[next - 1].clone())
        };
    }

    /// Valores que a sessão vai usar: as preferências do space vencem o
    /// `config.toml`, como em `Choice::resolve`.
    fn effective(&self) -> Config {
        let mut config = self.config;
        if let Some(agent) = self.prefs.agent {
            config.provider = agent;
        }
        if let Some(mode) = self.prefs.mode {
            config.mode = mode;
        }
        if let Some(jail) = self.prefs.jail {
            config.jail = jail;
        }
        config
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

    /// Alterna o valor do campo: nas preferências do space quando elas valem
    /// para ele, senão no `config.toml`.
    fn step(&mut self, field: Field, delta: isize) {
        if self.in_space(field) {
            if field == Field::Account {
                self.step_account(delta);
            } else {
                let current = self.effective();
                field.step_prefs(&mut self.prefs, &current, delta);
            }
            self.save_prefs();
        } else {
            field.step(&mut self.config, delta);
            self.save();
        }
    }

    /// Tira o campo do seu alvo: nas preferências do space, remove o
    /// override (volta a valer o `config.toml`); nos outros, volta ao
    /// padrão.
    fn reset(&mut self, field: Field) {
        if self.in_space(field) {
            field.reset_prefs(&mut self.prefs);
            self.save_prefs();
        } else {
            field.reset(&mut self.config);
            self.save();
        }
    }

    /// Trata a tecla; devolve o campo alterado e gravado, se houver.
    fn key(&mut self, key: KeyEvent) -> Option<Field> {
        let field = self.field();
        if let Some(text) = &mut self.editing {
            match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => text.push(c),
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
            KeyCode::Down | KeyCode::Char('j') => {
                self.index = (self.index + 1).min(FIELDS.len() - 1)
            }
            KeyCode::Home | KeyCode::Char('g') => self.index = 0,
            KeyCode::End | KeyCode::Char('G') => self.index = FIELDS.len() - 1,
            KeyCode::Enter if field.numeric() => self.editing = Some(String::new()),
            KeyCode::Char(c) if field.numeric() && c.is_ascii_digit() => {
                self.editing = Some(c.to_string())
            }
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
            KeyCode::Char('r') => *self = Form::load(),
            _ => {}
        }
        None
    }

    fn text(&self) -> Text<'static> {
        let effective = self.effective();
        let mut lines = Vec::new();
        for (i, &field) in FIELDS.iter().enumerate() {
            if i == 0 || FIELDS[i - 1].group() != field.group() {
                if i > 0 {
                    lines.push(Line::default());
                }
                lines.push(Line::raw(field.group()).bold());
            }
            let selected = i == self.index;
            let value = match (&self.editing, selected) {
                (Some(text), true) => format!("{text}▏"),
                _ if field == Field::Account => self.account_display(),
                _ => field.value(&effective),
            };
            let style = if selected {
                Style::new().black().on_cyan().bold()
            } else {
                Style::new()
            };
            lines.push(Line::from(vec![
                Span::raw(if selected { " › " } else { "   " }),
                Span::styled(format!("{:<26}", field.label()), style),
                Span::raw("  "),
                Span::raw(value).cyan(),
            ]));
        }
        lines.push(Line::default());
        match &self.message {
            Some(Ok(text)) => lines.push(Line::raw(format!("✔ {text}")).green()),
            Some(Err(err)) => lines.push(Line::raw(format!("✘ {err}")).red()),
            None => {}
        }
        let footer = match &self.space {
            Some(name) => format!(
                "Agente, modo, jail e conta valem em {} (preferências do space {name}); \
                 os outros campos valem no config.toml. As flags do terraforme cli têm prioridade.",
                Preferences::path(name)
                    .map_or_else(|_| "preferences.toml".into(), |p| p.display().to_string())
            ),
            None => "As flags do terraforme cli e as preferências do space têm prioridade \
                     sobre o agente, o modo e o jail. Conta só vale dentro de um space."
                .to_string(),
        };
        lines.push(Line::raw(footer).dark_gray());
        Text::from(lines)
    }
}

/// Uma instância do agente, no próprio pseudoterminal.
struct Agent {
    /// Número da sub-aba; não é reaproveitado quando a instância fecha.
    id: usize,
    parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Linhas roladas para trás no histórico (0 = ao vivo).
    scrollback: usize,
    launched: SystemTime,
    /// Id de sessão passado ao agente.
    session: Option<String>,
    /// Instruções a entregar ao agente logo que ele inicializar.
    pending: Option<String>,
    /// Momento da abertura, para o atraso de entregar o prompt.
    booted: Instant,
    /// Id do spawn MCP (Some = este é um subagente MCP).
    spawn_id: Option<String>,
    /// Saída acumulada do subagente para devolver ao MCP.
    output_buf: Vec<u8>,
    /// Porta do MCP para enviar o resultado quando terminar.
    mcp_port: Option<u16>,
    /// Id da instância no registro do MCP (`None`: não registrada).
    instance_id: Option<String>,
    /// Quando o processo foi visto encerrado por [`Agent::exited`].
    exited_at: Option<Instant>,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        id: usize,
        adapter: &dyn Adapter,
        command: &Command,
        cwd: &Path,
        body: Rect,
        session_ids: bool,
        link: Option<&Link>,
        tx: &Sender<Msg>,
    ) -> Result<Agent> {
        let pty = native_pty_system()
            .openpty(pty_size(body))
            .map_err(|e| anyhow!("{e}"))
            .context("falha ao abrir o pseudoterminal")?;
        let mut builder = builder(command, cwd);
        let mut session = None;
        if session_ids {
            let id = uuid::Uuid::new_v4().to_string();
            if let Some(args) = adapter.session_args(&id) {
                builder.args(args);
                session = Some(id);
            }
        }
        let instance_id = link.map(|_| uuid::Uuid::new_v4().to_string());
        if let (Some(link), Some(instance_id)) = (link, &instance_id) {
            let url = format!("http://127.0.0.1:{}/", link.port);
            let headers = [
                ("x-client-id", link.client_id.as_str()),
                ("x-instance-id", instance_id.as_str()),
            ];
            if let Some(args) = adapter.mcp_args(&url, &headers) {
                builder.args(args);
            }
        }
        let launched = SystemTime::now();
        let child = pty
            .slave
            .spawn_command(builder)
            .map_err(|e| anyhow!("{e}"))
            .context("falha ao iniciar o agente")?;
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
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

        if let (Some(link), Some(instance_id)) = (link, &instance_id) {
            let body = serde_json::json!({
                "instance_id": instance_id,
                "client_id": link.client_id,
                "control_addr": link.control_addr,
                "agent": link.agent,
                "number": id,
                "space": link.space,
                "cwd": cwd.display().to_string(),
            });
            let addr = format!("127.0.0.1:{}", link.port);
            thread::spawn(move || http_post_fire(&addr, "/instances/register", &body.to_string()));
        }

        Ok(Agent {
            id,
            parser: vt100::Parser::new(body.height, body.width, SCROLLBACK),
            writer: pty.master.take_writer().map_err(|e| anyhow!("{e}"))?,
            master: pty.master,
            child,
            scrollback: 0,
            launched,
            session,
            pending: None,
            booted: Instant::now(),
            spawn_id: None,
            output_buf: Vec::new(),
            mcp_port: link.map(|l| l.port),
            instance_id,
            exited_at: None,
        })
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
        if queries == 0 {
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

    /// Entrega o prompt pendente ao agente quando ele já
    /// inicializou (primeira saída recebida e ao menos [`PROMPT_DELAY`]).
    fn deliver_pending(&mut self) {
        let Some(prompt) = self.pending.clone() else {
            return;
        };
        if self.booted.elapsed() < PROMPT_DELAY {
            return;
        }
        self.pending = None;
        if self.send(&prompt_bytes(&prompt)).is_err() {
            // Tenta de novo na próxima saída do agente.
            self.pending = Some(prompt);
        }
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
        self.master
            .resize(pty_size(body))
            .map_err(|e| anyhow!("{e}"))
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
    selection: Option<Selection>,
    /// Mensagem curta no canto da tela, até [`STATUS_TTL`].
    status: Option<(String, Instant)>,
}

/// Abre a TUI com o agente dentro e só volta quando todas as instâncias
/// terminam (ou F10).
pub fn run(options: Options) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().context("tamanho do terminal")?;
    let cwd = match options.command.get_current_dir() {
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

        // Heartbeat a cada 2s.
        let hb_client = client_id.clone();
        thread::spawn(move || {
            let body = serde_json::json!({ "client_id": hb_client }).to_string();
            loop {
                thread::sleep(Duration::from_secs(2));
                http_post_fire(&format!("127.0.0.1:{mcp}"), "/heartbeat", &body);
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
            agent: options.agent.clone(),
            space: options.space.clone().unwrap_or_default(),
        });
    }

    let mut app = App {
        agent: options.agent,
        title: options.title,
        extensions: options.extensions,
        adapter: options.adapter,
        command: options.command,
        cwd,
        session_ids: options.session_ids,
        agents: Vec::new(),
        next_id: 1,
        active: Tab::Agent,
        focused: 1,
        view: None,
        form: None,
        popup: (!options.notices.is_empty()).then(|| notices(options.notices.join("\n"))),
        body: body_size(Rect::new(0, 0, cols, rows)),
        tx: tx.clone(),
        quit: false,
        mcp_port,
        link,
        enhanced: false,
        mouse: tui.mouse,
        copy_on_select: tui.copy_on_select,
        selection: None,
        status: None,
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

    // Encerra as instâncias que ainda estiverem rodando.
    drop(app);

    // Desregistra do MCP.
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
        let id = self.next_id;
        let agent = Agent::spawn(
            id,
            self.adapter,
            &self.command,
            &self.cwd,
            self.body,
            self.session_ids,
            self.link.as_ref(),
            &self.tx,
        )?;
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
        match self.spawn() {
            Ok(id) => self.focus(id),
            Err(err) => self.notify(format!("⚠ {err:#}")),
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
            self.adapter,
            &command,
            &cwd,
            self.body,
            false,
            None,
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
        let Some(i) = self.agents.iter().position(|a| a.id == self.focused) else {
            return;
        };
        let id = self.next_id;
        match Agent::spawn(
            id,
            self.adapter,
            &self.command,
            &self.cwd,
            self.body,
            self.session_ids,
            self.link.as_ref(),
            &self.tx,
        ) {
            Ok(agent) => {
                self.next_id += 1;
                self.agents.insert(i + 1, agent);
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
        self.agents.remove(i);
        if self.agents.is_empty() {
            self.quit = true;
            return;
        }
        if self.focused == id {
            self.focused = self.agents[i.min(self.agents.len() - 1)].id;
        }
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
            // Mensagens para um agente ocioso (sem saída que dispare a entrega).
            for agent in &mut self.agents {
                agent.deliver_pending();
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
                    agent.parser.process(&bytes);
                    agent.answer_cursor_queries(&bytes);
                    agent.deliver_pending();
                }
            }
            Msg::Exited(id) => {
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
                let label = self.agent.clone();
                if let Some(agent) = self
                    .agents
                    .iter_mut()
                    .find(|a| a.instance_id.as_deref() == Some(instance_id.as_str()))
                {
                    agent.queue(&text);
                    let id = agent.id;
                    self.flash(format!("✉ {from} → {label} {id}"));
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
                    if agent.parser.screen().bracketed_paste() {
                        agent.send(format!("\x1b[200~{text}\x1b[201~").as_bytes())?;
                    } else {
                        agent.send(text.as_bytes())?;
                    }
                }
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse)?,
            // O redesenho seguinte ajusta as instâncias ao novo tamanho (`fit`).
            _ => {}
        }
        Ok(false)
    }

    /// Roda do mouse e seleção de texto. Sem a captura do mouse o terminal
    /// converte a roda em setas, que no agente navegam o histórico de prompts
    /// em vez de rolar a tela.
    fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        let up = match mouse.kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
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
                    (form.index + 1).min(FIELDS.len() - 1)
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
        let screen = agent.parser.screen();
        if screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None {
            // O agente pediu o mouse: ele mesmo rola.
            let col = mouse.column;
            let row = mouse.row.saturating_sub(AGENT_TOP);
            let bytes = wheel_bytes(up, col, row, screen.mouse_protocol_encoding());
            agent.send(&bytes)?;
        } else if screen.alternate_screen() {
            // Tela alternativa não tem histórico: vira setas, como no xterm.
            let arrow = if up { KeyCode::Up } else { KeyCode::Down };
            let bytes = key_bytes(
                KeyEvent::new(arrow, KeyModifiers::NONE),
                screen.application_cursor(),
            );
            agent.send(&bytes.repeat(WHEEL_LINES as usize))?;
        } else {
            let lines = WHEEL_LINES as isize;
            agent.scroll_history(if up { lines } else { -lines });
        }
        Ok(())
    }

    /// Arrastar com o botão esquerdo seleciona texto da instância aberta e,
    /// ao soltar, copia (se [`App::copy_on_select`]). Um clique sem arrastar
    /// desfaz a seleção.
    fn select_text(&mut self, mouse: MouseEvent) {
        if self.popup.is_some() || self.view.is_some() {
            return;
        }
        let Some(agent) = self.agent(self.focused) else {
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
                            agent.send(&bytes)?;
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
            let tui = form.config.tui;
            match form.key(key) {
                Some(Field::Mouse) if tui.mouse != self.mouse => self.set_mouse(tui.mouse),
                Some(Field::CopyOnSelect) => self.copy_on_select = tui.copy_on_select,
                _ => {}
            }
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
            // ←/→: sub-aba anterior/próxima (nas visões, não chega ao agente).
            KeyCode::Left => self.cycle(-1),
            KeyCode::Right => self.cycle(1),
            _ => {}
        }
    }

    /// Abre a aba pelo jeito que o Registro [`TABS`] diz: o agente fica com
    /// o pseudoterminal, as abas de conteúdo recarregam o conteúdo e a
    /// Config abre o formulário.
    fn select(&mut self, tab: Tab) {
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
        let adapter = self.adapter;
        thread::spawn(move || {
            let result = match (tab, report) {
                (_, Some((ext, report))) => ext.report(report, adapter.bin()).map(Text::from),
                (Tab::Usage, None) => {
                    adapter
                        .usage(&cwd, launched, session.as_deref())
                        .map(|u| match index {
                            0 => format_limits(adapter, &u),
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

    /// Prefixo das teclas das sub-abas mostrado nas barras.
    fn sub_key(&self) -> &'static str {
        if self.enhanced { "⌃⌥" } else { "⌥" }
    }

    fn draw(&self, frame: &mut Frame) {
        let [menu, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());

        frame.render_widget(self.menu(), menu);

        if let Some(view) = &self.view {
            self.draw_view(frame, view, body);
        } else if let Some(agent) = self.agent(self.focused) {
            let [instances, body] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
            frame.render_widget(self.instances(), instances);

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
                let area = Rect::new(body.right().saturating_sub(width), body.y, width, 1)
                    .intersection(body);
                frame.render_widget(Span::raw(hint).black().on_yellow(), area);
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

    fn menu(&self) -> Line<'_> {
        let mut spans = vec![
            Span::styled(" terraforme ", Style::new().black().on_green().bold()),
            Span::raw(" "),
        ];
        for tab in self.tabs() {
            let style = if tab == self.active {
                Style::new().black().on_cyan().bold()
            } else {
                Style::new().white().on_dark_gray()
            };
            // O rótulo da tecla vem do Registro: é o atalho que a abre.
            let key = format!(" {} ", spec(tab).tecla());
            spans.push(Span::styled(key, style.add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!("{} ", tab.label(&self.agent)), style));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(" F10 ", Style::new().white().on_red().bold()));
        spans.push(Span::styled("sair ", Style::new().white().on_red()));
        let mouse = if self.mouse {
            ""
        } else {
            " · 🖱 seleção nativa"
        };
        spans.push(Span::raw(format!("  ⌃⌥K atalhos{mouse} · {}", self.title)).dark_gray());
        Line::from(spans).style(Style::new().bg(Color::Reset))
    }

    /// Sub-abas da aba do agente: uma por instância.
    fn instances(&self) -> Line<'_> {
        let labels: Vec<_> = self
            .agents
            .iter()
            .map(|a| ("", format!("{} {}", self.agent, a.id)))
            .collect();
        let (_, current) = self.sub_tabs();
        let mut spans = vec![Span::raw(" ")];
        spans.extend(sub_tab_spans(&labels, current, self.sub_key()));
        let hint = if self.agents.len() > 1 {
            format!(
                " {k}Tab/{k}1-9 instância · ⌃⌥N nova · ⌃⌥W fecha · ⌃⌥R reinicia",
                k = self.sub_key()
            )
        } else {
            format!(
                " {}{} ou ⌃⌥N nova instância · ⌃⌥W fecha · arraste para copiar",
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
                |_| "Config".to_string(),
                |p| format!("Config · {}", p.display()),
            ),
            (Tab::Extensions, Some((ext, _))) => format!("Extensões · {ext}"),
            (Tab::Extensions, None) => "Extensões".to_string(),
            (_, Some((ext, _))) if !ext.is_empty() => format!("Consumo · {ext}"),
            _ if self.agents.len() > 1 => {
                format!("Consumo de tokens · {} {}", self.agent, self.focused)
            }
            _ => "Consumo de tokens".to_string(),
        };
        let hint = match view.tab {
            Tab::Config => {
                " ↑↓ campo · ←→/Enter altera · dígitos editam · Del padrão · r relê · Esc agente "
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

/// Uma barra por limite, no estilo do Claude Code: o título traz o limite
/// total de tokens (quando há) e a barra verde vai encurtando a cada consumo.
fn format_limits(adapter: &dyn Adapter, usage: &usage::Usage) -> Text<'static> {
    let budget = Config::load().map(|c| c.budget).unwrap_or_default();
    let limits = limits::collect(adapter, usage, &budget);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut lines: Vec<Line> = Vec::new();
    for limit in &limits.items {
        let mut title = vec![Span::raw(limit.label.clone()).bold()];
        if let Some(at) = limit.resets_at {
            title.push(
                Span::raw(format!("  · zera em {}", duration(at.saturating_sub(now)))).dark_gray(),
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
    Text::from(lines)
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

SUB-ABAS (instâncias do agente, visões do consumo, relatórios)
  Ctrl+Alt+Tab           próxima sub-aba (com Shift, a anterior)
  Ctrl+Alt+1–9           vai para a sub-aba (alternativas: Alt+1–9, Shift+Alt+1–9)
  Ctrl+PgUp / Ctrl+PgDn  sub-aba anterior / próxima
  ← / →                  sub-aba anterior / próxima (nas visões: Consumo, Extensões, Config)
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
  Config: ↑↓ campo · ←→/Enter altera · dígitos editam números · Del padrão

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

/// Instruções como teclas: quebra de linha vira `Alt+Enter` (não envia o
/// prompt no meio das instruções) e termina com Enter, que envia.
fn prompt_bytes(instructions: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (i, line) in instructions.split('\n').enumerate() {
        if i > 0 {
            bytes.extend(b"\x1b\r");
        }
        bytes.extend(line.as_bytes());
    }
    bytes.push(b'\r');
    bytes
}

/// Servidor de controle: recebe do MCP `POST /spawn` (subagente) e
/// `POST /message` (mensagem para uma instância) e os repassa ao loop da TUI.
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
    use std::io::Write;
    use std::net::TcpStream;
    if let Ok(mut stream) = TcpStream::connect(addr) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let request = format!(
            "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            path,
            addr,
            body.len(),
            body
        );
        let _ = stream.write_all(request.as_bytes());
    }
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

/// Área do agente: a tela menos a linha do menu e a das sub-abas.
fn body_size(area: Rect) -> Rect {
    Rect::new(
        0,
        0,
        area.width.max(1),
        area.height.saturating_sub(2).max(1),
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
    fn clip_input_is_utf16le_with_bom() {
        assert_eq!(utf16le_with_bom("aç"), [0xFF, 0xFE, b'a', 0, 0xE7, 0]);
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
        assert_eq!(Tab::Agent.label("claude"), "claude");
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
    fn config_fields_cycle_set_and_reset() {
        let mut config = Config::default();
        Field::Mode.step(&mut config, 1);
        assert_eq!(config.mode, Mode::None);
        Field::Mode.step(&mut config, -1);
        assert_eq!(config.mode, Mode::Hybrid);
        Field::Jail.step(&mut config, 1);
        assert!(config.jail);
        Field::Mouse.step(&mut config, 1);
        assert!(!config.tui.mouse);

        Field::BudgetWeek.set(&mut config, "300.000.000").unwrap();
        assert_eq!(config.budget.week, Some(300_000_000));
        assert_eq!(Field::BudgetWeek.value(&config), "300.000.000");
        Field::BudgetWeek.set(&mut config, "").unwrap();
        assert_eq!(config.budget.week, None);
        assert!(Field::McpMax.set(&mut config, "abc").is_err());
        Field::McpMax.set(&mut config, "0").unwrap();
        assert_eq!(config.mcp.max_simultaneous_subagents, 1);

        for field in FIELDS {
            field.reset(&mut config);
        }
        assert_eq!(config, Config::default());
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
    fn thousands_uses_dots() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1.000");
        assert_eq!(thousands(1_234_567), "1.234.567");
    }

    #[test]
    fn body_leaves_room_for_the_menu_and_instances() {
        assert_eq!(body_size(Rect::new(0, 0, 80, 24)), Rect::new(0, 0, 80, 22));
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
    fn prompt_ends_with_enter_and_does_not_submit_midway() {
        // Quebra de linha → Alt+Enter (não envia); só o final envia.
        assert_eq!(prompt_bytes("olá"), "olá\r".as_bytes());
        assert_eq!(prompt_bytes("l1\nl2"), b"l1\x1b\rl2\r");
        assert_eq!(prompt_bytes("l1\n\nl3"), b"l1\x1b\r\x1b\rl3\r");
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
            index: 0,
            editing: None,
            message: None,
        };
        // As preferências do space vencem o config.toml, que completa o resto.
        let effective = form.effective();
        assert_eq!(effective.provider, Provider::OpenCode);
        assert_eq!(effective.mode, Mode::Eco);
        assert!(effective.jail);
        // Só agente, modo e jail vão para as preferências.
        assert!(form.in_space(Field::Provider));
        assert!(form.in_space(Field::Mode));
        assert!(form.in_space(Field::Jail));
        assert!(!form.in_space(Field::BudgetFiveHours));
        assert!(!form.in_space(Field::Mouse));
        // Fora de space, tudo continua no config.toml.
        let global = Form {
            space: None,
            prefs: Preferences::default(),
            ..form
        };
        assert!(!global.in_space(Field::Provider));
        assert_eq!(global.effective().provider, Provider::Claude);
        assert_eq!(global.effective().mode, Mode::Eco);
        assert!(!global.effective().jail);
    }
}
