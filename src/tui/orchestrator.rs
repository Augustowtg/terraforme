//! Aba Orquestrador: tudo o que está no `~/.terraforme/orchestrator.toml`,
//! editável, e as visões ao vivo do orquestrador.
//!
//! ```text
//!  Orquestrador
//!   Despachos               ● ativos (p pausa todos)
//!
//!  Sessões
//!   ▾ Studio · 2 instâncias · 1 sub agente
//!       ● Studio-4242              Principal, api               rodando
//!       ● Studio-4300              Principal                    parado
//!   ▸ terraforme-cli · 1 instância
//!    Sub agentes em execução
//!       ▶ api              1a2b3c4d  nível 2
//!
//!  Configuração
//!   Modo de execução        Demonstrativo (visível nas abas)
//!   Tempo limite            30 min
//!   …
//!
//!  Agentes despachados
//!   api              despachado · rodando · 3/40 hoje
//!       Pasta                   /proj/api
//!       Provedor                Claude
//!       Modelo                  [Claude] Opus - 5.5
//!       …
//!   + Novo agente
//!
//!  Filas e execuções · Mensagens · Custos de hoje · Roteamento
//! ```
//!
//! Um workspace das Sessões fica fechado até o `Enter` (ou `→`) abri-lo.
//! `↑`/`↓` passam pelas linhas escolhíveis; `←`/`→` mudam o valor (números,
//! escolhas, ligado/desligado); `Enter` edita o texto num prompt no pé da
//! aba (ou abre a saída de uma execução); `d` duas vezes tira um agente,
//! uma regra ou um modelo; `p` pausa ou retoma todos os despachos.
//!
//! O provedor de um agente passa entre os que o hub sabe lançar, e trocá-lo
//! tira o modelo (o id de um não vale no outro). No Modelo, o `Enter` abre o
//! menu do `/model` com o catálogo ao vivo do provedor do agente, carregado
//! em segundo plano como o do chat; o `←`/`→` passa pelo padrão, pelos
//! modelos do catálogo e pelos apelidos de `[modelos]`.
//!
//! Toda mudança passa pelo [`Editor`]: ela mantém os comentários do arquivo
//! e só é gravada se o arquivo continuar válido; senão, o erro aparece no pé
//! e nada muda. O hub relê o arquivo a cada segundo. Um arquivo inválido
//! (editado à mão) aparece no topo, e a aba não o edita até ele ser corrigido.

mod rows;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

pub use self::rows::Hub as HubView;
use self::rows::{Action, AgentKey, GlobalKey, RoutingKey};
use super::chat::model_menu::{self, ModelMenu};
use crate::agent::Catalog;
use crate::agent::adapters::{self, ProviderOption};
use crate::hub::protocol::Instance;
use crate::orchestrator::config::{Config, INFINITE, PROVIDERS, RunMode};
use crate::orchestrator::edit::{self, Editor};
use crate::orchestrator::message::{self, Saved};
use crate::orchestrator::run::{self, Run};
use crate::theme;

pub const HELP: &str = "↑/↓ escolher · ←/→ mudar · Enter editar · d tirar · p pausar · Tab trocar aba · Esc ir para Sair";
const HELP_PROMPT: &str = "Enter salvar · Esc cancelar";
const HELP_MODEL: &str = "↑/↓ escolher · Enter selecionar · Esc fechar";
/// De quanto em quanto tempo a aba relê o arquivo, as execuções e as
/// mensagens.
const REFRESH: Duration = Duration::from_secs(1);
/// O que o `PageUp`/`PageDown` anda.
const PAGE: usize = 10;

/// O que a aba mostra, lido do disco.
pub struct Data {
    /// A pasta do terra (`None` sem ela, como nos testes da tela).
    pub home: Option<PathBuf>,
    /// A pasta do workspace: a de um agente novo.
    pub cwd: PathBuf,
    pub config: Result<Config, String>,
    /// Da mais antiga à mais nova.
    pub runs: Vec<Run>,
    /// As mais novas primeiro.
    pub messages: Vec<Saved>,
    /// Os provedores, com o nome e se estão instalados.
    pub providers: Vec<ProviderOption>,
    /// Os catálogos que já chegaram, por provedor.
    pub catalogs: HashMap<String, Catalog>,
    loaded: Option<Instant>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            home: None,
            cwd: PathBuf::new(),
            config: Ok(Config::default()),
            runs: Vec::new(),
            messages: Vec::new(),
            providers: Vec::new(),
            catalogs: HashMap::new(),
            loaded: None,
        }
    }
}

impl Data {
    pub fn new(home: Option<PathBuf>, cwd: PathBuf) -> Self {
        Self {
            home,
            cwd,
            ..Self::default()
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.home.as_deref().map(Config::path)
    }

    pub fn reload(&mut self) {
        self.loaded = Some(Instant::now());
        let Some(home) = &self.home else {
            return;
        };
        self.config = Config::load(&Config::path(home));
        self.runs = run::list(home);
        self.messages = message::recent(home, rows::RECENT);
    }

    pub fn refresh_if_stale(&mut self) {
        if self.loaded.is_none_or(|t| t.elapsed() >= REFRESH) {
            self.reload();
        }
    }

    /// O nome do provedor para a tela (ex.: `Claude`).
    pub fn provider_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.providers
            .iter()
            .find(|p| p.id == id)
            .map_or(id, |p| p.name.as_str())
    }
}

/// O que um prompt edita.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    NewAgent,
    AgentField(String, AgentKey),
    Global(GlobalKey),
    /// Uma regra (`None` é a nova).
    Rule(Option<usize>),
    /// Um modelo (`None` é o novo).
    Model(Option<String>),
}

struct Prompt {
    label: String,
    text: String,
    target: Target,
}

/// O que uma tecla fez na aba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Handled,
    /// Abrir a saída da execução (o app abre a aba dela).
    OpenRun(String),
}

#[derive(Default)]
pub struct OrchestratorView {
    pub data: Data,
    /// A linha escolhida, entre as escolhíveis.
    selected: usize,
    /// A primeira linha visível.
    scroll: Cell<usize>,
    prompt: Option<Prompt>,
    /// A ação que espera o segundo `d`.
    deleting: Option<Action>,
    /// O aviso do pé: o texto e se é um erro.
    notice: Option<(String, bool)>,
    /// Os workspaces abertos na seção Sessões.
    expanded: Vec<String>,
    /// As instâncias do último desenho: as linhas dos workspaces são
    /// escolhíveis, e as teclas chegam sem o hub.
    hub: RefCell<Option<(bool, Vec<Instance>)>>,
    /// O menu do `/model` aberto e o agente que ele escolhe.
    model_menu: Option<(String, ModelMenu)>,
    /// Os catálogos que estão carregando, por provedor.
    loading: HashMap<String, Receiver<Result<Catalog, String>>>,
}

impl OrchestratorView {
    pub fn new(data: Data) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }

    pub fn help(&self) -> &'static str {
        if self.model_menu.is_some() {
            HELP_MODEL
        } else if self.prompt.is_some() {
            HELP_PROMPT
        } else {
            HELP
        }
    }

    /// O catálogo do provedor `id`, se já chegou.
    fn catalog(&self, id: &str) -> Option<&Catalog> {
        self.data.catalogs.get(id)
    }

    /// Um catálogo que o app já tem (o do chat, do provedor `id`): a aba não
    /// precisa carregar de novo.
    pub fn offer_catalog(&mut self, id: &str, catalog: &Catalog) {
        if !self.data.catalogs.contains_key(id) {
            self.loading.remove(id);
            self.data.catalogs.insert(id.to_string(), catalog.clone());
        }
    }

    /// Começa a carregar, em segundo plano, o catálogo dos provedores dos
    /// agentes que ainda não o têm, e recebe os que chegaram. Um erro deixa
    /// o catálogo vazio, para não carregar de novo a cada tick.
    pub fn load_catalogs(&mut self) {
        let used: Vec<String> = self
            .config()
            .map(|c| c.agents.iter().map(|a| a.provider().to_string()).collect())
            .unwrap_or_default();
        for id in used {
            if self.data.catalogs.contains_key(&id) || self.loading.contains_key(&id) {
                continue;
            }
            let receiver = adapters::select(Some(&id)).load_catalog(None);
            match receiver {
                Some(receiver) => {
                    self.loading.insert(id, receiver);
                }
                None => {
                    self.data.catalogs.insert(id, Catalog::default());
                }
            }
        }
        let arrived: Vec<(String, Catalog)> = self
            .loading
            .iter()
            .filter_map(|(id, rx)| {
                rx.try_recv()
                    .ok()
                    .map(|r| (id.clone(), r.unwrap_or_default()))
            })
            .collect();
        for (id, catalog) in arrived {
            self.loading.remove(&id);
            self.data.catalogs.insert(id, catalog);
        }
    }

    /// As ações das linhas escolhíveis, na ordem da tela.
    fn actions(&self) -> Vec<Action> {
        let snapshot = self.hub.borrow();
        let hub = HubView {
            instances: snapshot.as_ref(),
            own: None,
        };
        rows::build(&self.data, &hub, &self.expanded, None, None)
            .into_iter()
            .filter_map(|row| row.action)
            .collect()
    }

    fn current(&self) -> Option<Action> {
        let actions = self.actions();
        actions
            .get(self.selected.min(actions.len().saturating_sub(1)))
            .cloned()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if self.model_menu.is_some() {
            self.menu_key(key);
            return Outcome::Handled;
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return Outcome::Handled;
        }
        let deleting = self.deleting.take();
        let count = self.actions().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(PAGE),
            KeyCode::PageDown => {
                self.selected = (self.selected + PAGE).min(count.saturating_sub(1))
            }
            KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Left | KeyCode::Char('h') => self.change(-1),
            KeyCode::Right | KeyCode::Char('l') => self.change(1),
            KeyCode::Enter => return self.activate(),
            KeyCode::Char('d') | KeyCode::Delete => {
                let Some(action) = self.current().filter(Action::removable) else {
                    return Outcome::Handled;
                };
                if deleting.as_ref() == Some(&action) {
                    self.remove(&action);
                } else {
                    self.deleting = Some(action);
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Handled
    }

    fn config(&self) -> Option<&Config> {
        self.data.config.as_ref().ok()
    }

    /// Muda o arquivo com `change` e grava, se ele continuar válido.
    fn edit(&mut self, change: impl FnOnce(&mut Editor)) {
        self.try_edit(|editor| {
            change(editor);
            Ok(())
        });
    }

    /// Como o [`Self::edit`], com uma mudança que pode ser recusada antes.
    fn try_edit(&mut self, change: impl FnOnce(&mut Editor) -> Result<(), String>) {
        let Some(path) = self.data.path() else {
            self.notice = Some(("Sem a pasta do terra.".into(), true));
            return;
        };
        let result = Editor::open(&path).and_then(|mut editor| {
            change(&mut editor)?;
            editor.save()
        });
        self.notice = Some(match result {
            Ok(_) => ("Salvo no orchestrator.toml.".into(), false),
            Err(err) => (format!("Não salvo: {err}"), true),
        });
        self.data.reload();
    }

    fn toggle_pause(&mut self) {
        let Some(paused) = self.config().map(|c| c.global.paused) else {
            return;
        };
        self.edit(|e| e.set_global("pausado", !paused));
    }

    /// `←`/`→`: o próximo valor (`step` é -1 ou 1).
    fn change(&mut self, step: i64) {
        let (Some(action), Some(config)) = (self.current(), self.config().cloned()) else {
            return;
        };
        match action {
            Action::Workspace(name) => {
                let open = self.expanded.contains(&name);
                if step > 0 && !open {
                    self.expanded.push(name);
                } else if step < 0 && open {
                    self.expanded.retain(|w| *w != name);
                }
            }
            Action::Pause => self.toggle_pause(),
            Action::Global(GlobalKey::RunMode) => {
                let next = match config.global.run_mode {
                    RunMode::Tab => "segundo_plano",
                    RunMode::Background => "aba",
                };
                self.edit(|e| e.set_global("modo_execucao", next));
            }
            Action::Global(GlobalKey::FocusRuns) => {
                let focus = config.global.focus_runs;
                self.edit(|e| e.set_global("aba_em_primeiro_plano", !focus));
            }
            Action::Global(GlobalKey::Timeout) => {
                // Abaixo de 1 minuto, infinito; do infinito, 1 minuto.
                let next = match config.global.timeout_min {
                    None if step > 0 => Some(1),
                    None => None,
                    Some(n) => (i64::from(n) + step >= 1).then(|| (i64::from(n) + step) as u32),
                };
                self.edit(|e| match next {
                    Some(n) => e.set_global(GlobalKey::Timeout.key(), edit::number(n)),
                    None => e.set_global(GlobalKey::Timeout.key(), INFINITE),
                });
            }
            Action::Global(key) => {
                let n = key.number(&config).unwrap_or(1);
                let next = (i64::from(n) + step).clamp(1, i64::from(u32::MAX)) as u32;
                self.edit(|e| e.set_global(key.key(), edit::number(next)));
            }
            Action::AgentField(name, AgentKey::Provider) => {
                let Some(agent) = config.agent(&name) else {
                    return;
                };
                let i = PROVIDERS
                    .iter()
                    .position(|p| *p == agent.provider())
                    .unwrap_or(0);
                let next = PROVIDERS[(i as i64 + step).rem_euclid(PROVIDERS.len() as i64) as usize];
                // O id do modelo de um provedor não vale no outro.
                self.edit(|e| {
                    e.set_agent(&name, "provedor", Some(next.into()));
                    e.set_agent(&name, "modelo", None);
                });
            }
            Action::AgentField(name, AgentKey::Model) => {
                let Some(agent) = config.agent(&name) else {
                    return;
                };
                // O padrão, os modelos do catálogo e os apelidos.
                let models = self
                    .catalog(agent.provider())
                    .map(|c| {
                        c.models
                            .iter()
                            .map(|m| Some(m.id.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let options: Vec<Option<String>> = std::iter::once(None)
                    .chain(models)
                    .chain(config.models.iter().map(|(alias, _)| Some(alias.clone())))
                    .collect();
                let i = options.iter().position(|o| *o == agent.model).unwrap_or(0);
                let next =
                    options[(i as i64 + step).rem_euclid(options.len() as i64) as usize].clone();
                self.edit(|e| e.set_agent(&name, "modelo", next.map(|m| m.as_str().into())));
            }
            Action::AgentField(name, key @ (AgentKey::MaxTurns | AgentKey::Budget)) => {
                let Some(agent) = config.agent(&name) else {
                    return;
                };
                // Abaixo de 1, sem limite.
                let next = i64::from(key.number(agent).unwrap_or(0)) + step;
                let value = (next >= 1).then(|| edit::number(next as u32));
                self.edit(|e| e.set_agent(&name, key.key(), value));
            }
            Action::Routing(RoutingKey::Active) => {
                let active = config.routing.active;
                self.edit(|e| e.set_routing("ativo", Some((!active).into())));
            }
            Action::Routing(RoutingKey::Escalate) => {
                let escalate = config.routing.escalate_failures;
                self.edit(|e| e.set_routing("escalonar_falhas", Some((!escalate).into())));
            }
            Action::Routing(RoutingKey::Default) => {
                let options: Vec<Option<String>> = std::iter::once(None)
                    .chain(config.models.iter().map(|(alias, _)| Some(alias.clone())))
                    .collect();
                let i = options
                    .iter()
                    .position(|o| *o == config.routing.default)
                    .unwrap_or(0);
                let next =
                    options[(i as i64 + step).rem_euclid(options.len() as i64) as usize].clone();
                self.edit(|e| e.set_routing("padrao", next.map(|m| m.as_str().into())));
            }
            _ => {}
        }
    }

    /// `Enter`: liga/desliga, abre o prompt do texto ou a execução.
    fn activate(&mut self) -> Outcome {
        let Some(action) = self.current() else {
            return Outcome::Ignored;
        };
        let config = self.config().cloned().unwrap_or_default();
        let (label, text, target) = match action {
            Action::Run(id) => return Outcome::OpenRun(id),
            Action::Workspace(name) => {
                if self.expanded.contains(&name) {
                    self.expanded.retain(|w| *w != name);
                } else {
                    self.expanded.push(name);
                }
                return Outcome::Handled;
            }
            Action::Global(GlobalKey::Timeout) => (
                format!("Tempo limite (minutos ou {INFINITE})"),
                config
                    .global
                    .timeout_min
                    .map_or(INFINITE.to_string(), |n| n.to_string()),
                Target::Global(GlobalKey::Timeout),
            ),
            Action::Global(key) if key.number(&config).is_some() => {
                let n = key.number(&config).unwrap_or_default();
                (key_label(key.key()), n.to_string(), Target::Global(key))
            }
            Action::AddAgent => (
                "Novo agente: nome [pasta]".to_string(),
                String::new(),
                Target::NewAgent,
            ),
            Action::AgentField(name, AgentKey::Model) => {
                let Some(agent) = config.agent(&name) else {
                    return Outcome::Handled;
                };
                let catalog = self.catalog(agent.provider());
                let mut menu = ModelMenu::dispatched(agent.model.clone(), catalog);
                menu.sync(catalog);
                self.model_menu = Some((name, menu));
                return Outcome::Handled;
            }
            Action::AgentField(_, AgentKey::Provider) => {
                self.change(1);
                return Outcome::Handled;
            }
            Action::AgentField(name, key) => {
                let Some(agent) = config.agent(&name) else {
                    return Outcome::Handled;
                };
                let label = match key {
                    AgentKey::Tools => format!("{name}: ferramentas, separadas por vírgula"),
                    AgentKey::Model | AgentKey::Provider => format!("{name}: {}", key.key()),
                    AgentKey::MaxTurns | AgentKey::Budget => {
                        format!("{name}: {} (vazio é sem limite)", key.key())
                    }
                    AgentKey::Cwd => format!("{name}: pasta"),
                };
                (label, key.text(agent), Target::AgentField(name, key))
            }
            Action::Rule(i) => {
                let Some(rule) = config.routing.rules.get(i) else {
                    return Outcome::Handled;
                };
                (
                    "Regra: tag modelo".to_string(),
                    format!("{} {}", rule.tag, rule.model),
                    Target::Rule(Some(i)),
                )
            }
            Action::AddRule => (
                "Nova regra: tag modelo".into(),
                String::new(),
                Target::Rule(None),
            ),
            Action::Model(alias) => {
                let id = config.model_id(&alias).to_string();
                (
                    format!("Modelo {alias}: id"),
                    id,
                    Target::Model(Some(alias)),
                )
            }
            Action::AddModel => (
                "Novo modelo: apelido id".into(),
                String::new(),
                Target::Model(None),
            ),
            // O resto liga/desliga ou passa pelas escolhas.
            _ => {
                self.change(1);
                return Outcome::Handled;
            }
        };
        self.prompt = Some(Prompt {
            label,
            text,
            target,
        });
        Outcome::Handled
    }

    /// As teclas do menu do `/model`: escolher grava o `modelo` do agente;
    /// o Default o tira.
    fn menu_key(&mut self, key: KeyEvent) {
        let Some((name, mut menu)) = self.model_menu.take() else {
            return;
        };
        let provider = self
            .config()
            .and_then(|c| c.agent(&name))
            .map(|a| a.provider().to_string())
            .unwrap_or_default();
        match menu.handle_key(key, self.data.catalogs.get(&provider)) {
            model_menu::Outcome::Handled => self.model_menu = Some((name, menu)),
            model_menu::Outcome::Close => {}
            model_menu::Outcome::Chosen { model, .. } => {
                self.edit(|e| e.set_agent(&name, "modelo", Some(model.as_str().into())));
            }
            model_menu::Outcome::Unset(_) => self.edit(|e| e.set_agent(&name, "modelo", None)),
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                prompt.text.pop();
            }
            KeyCode::Char(c) => prompt.text.push(c),
            KeyCode::Enter => {
                if let Some(prompt) = self.prompt.take() {
                    self.submit(prompt);
                }
            }
            _ => {}
        }
    }

    fn submit(&mut self, prompt: Prompt) {
        let text = prompt.text.trim().to_string();
        let cwd = self.data.cwd.clone();
        match prompt.target {
            Target::NewAgent => {
                let mut words = text.splitn(2, char::is_whitespace);
                let name = words.next().unwrap_or_default().to_string();
                let dir = words
                    .next()
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .map_or(cwd.clone(), |d| cwd.join(d));
                self.try_edit(|e| e.add_agent(&name, &dir));
            }
            Target::AgentField(name, key) => {
                let value = match key {
                    AgentKey::Cwd => Some(cwd.join(&text).to_string_lossy().as_ref().into()),
                    AgentKey::Model | AgentKey::Provider => {
                        (!text.is_empty()).then(|| text.as_str().into())
                    }
                    AgentKey::Tools => {
                        let tools: Vec<String> = text
                            .split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect();
                        (!tools.is_empty()).then(|| edit::texts(&tools))
                    }
                    AgentKey::MaxTurns | AgentKey::Budget if text.is_empty() => None,
                    AgentKey::MaxTurns | AgentKey::Budget => match positive(&text) {
                        Ok(n) => Some(edit::number(n)),
                        Err(err) => return self.notice = Some((err, true)),
                    },
                };
                self.edit(|e| e.set_agent(&name, key.key(), value));
            }
            Target::Global(GlobalKey::Timeout)
                if text.is_empty() || text.eq_ignore_ascii_case(INFINITE) =>
            {
                self.edit(|e| e.set_global(GlobalKey::Timeout.key(), INFINITE));
            }
            Target::Global(key) => match positive(&text) {
                Ok(n) => self.edit(|e| e.set_global(key.key(), edit::number(n))),
                Err(err) => self.notice = Some((err, true)),
            },
            Target::Rule(index) => {
                let Some((tag, model)) = pair(&text) else {
                    return self.notice = Some(("Escreva: tag modelo".into(), true));
                };
                self.edit(|e| match index {
                    Some(i) => e.set_rule(i, &tag, &model),
                    None => e.add_rule(&tag, &model),
                });
            }
            Target::Model(Some(alias)) if !text.is_empty() => {
                self.edit(|e| e.set_model(&alias, &text));
            }
            Target::Model(Some(_)) => self.notice = Some(("Escreva o id do modelo.".into(), true)),
            Target::Model(None) => {
                let Some((alias, id)) = pair(&text) else {
                    return self.notice = Some(("Escreva: apelido id".into(), true));
                };
                self.edit(|e| e.set_model(&alias, &id));
            }
        }
    }

    fn remove(&mut self, action: &Action) {
        match action {
            Action::Agent(name) => self.edit(|e| e.remove_agent(name)),
            Action::Rule(i) => self.edit(|e| e.remove_rule(*i)),
            Action::Model(alias) => self.edit(|e| e.remove_model(alias)),
            _ => {}
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, hub: &HubView) {
        let [content, foot] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        *self.hub.borrow_mut() = hub.instances.cloned();
        let selected = self.current();
        let rows = rows::build(
            &self.data,
            hub,
            &self.expanded,
            selected.as_ref(),
            self.deleting.as_ref(),
        );
        let target = rows
            .iter()
            .position(|row| row.action.is_some() && row.action == selected)
            .unwrap_or(0);
        // A linha escolhida sempre à vista (com as de detalhe dela).
        let height = usize::from(content.height.saturating_sub(1)).max(1);
        let mut scroll = self.scroll.get();
        if target < scroll {
            scroll = target.saturating_sub(1);
        } else if target + 3 >= scroll + height {
            scroll = (target + 4).saturating_sub(height);
        }
        self.scroll.set(scroll);
        let lines: Vec<Line> = rows.into_iter().skip(scroll).map(|row| row.line).collect();
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            content,
        );
        let foot_line = match (&self.prompt, &self.notice) {
            (Some(prompt), _) => Line::from(vec![
                Span::styled(format!(" {}: ", prompt.label), theme::section_title()),
                Span::styled(format!("{}▏", prompt.text), theme::text()),
            ]),
            (None, Some((text, true))) => Line::styled(format!(" {text}"), theme::error()),
            (None, Some((text, false))) => Line::styled(format!(" {text}"), theme::muted()),
            (None, None) => Line::default(),
        };
        frame.render_widget(Paragraph::new(foot_line), foot);
        if let Some((name, menu)) = &self.model_menu {
            // Como o da Config: acima do pé da aba.
            let bottom = Rect {
                y: area.bottom(),
                height: 0,
                ..area
            };
            let provider = self
                .config()
                .and_then(|c| c.agent(name))
                .map_or("claude", |a| a.provider());
            let label = |m: &crate::agent::ModelInfo| rows::model_label(provider, m);
            menu.draw(frame, bottom, self.catalog(provider), &label);
        }
    }
}

/// O nome de uma chave do `[global]` para o prompt.
fn key_label(key: &str) -> String {
    format!("{key} (número maior que zero)")
}

fn positive(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("\"{text}\" não é um número maior que zero."))
}

/// `"a b"` → `("a", "b")` (também `a = b`).
fn pair(text: &str) -> Option<(String, String)> {
    let text = text.replacen('=', " ", 1);
    let mut words = text.split_whitespace();
    let (a, b) = (words.next()?, words.next()?);
    words
        .next()
        .is_none()
        .then(|| (a.to_string(), b.to_string()))
}

/// A saída de uma execução, para a aba dela.
pub fn run_output(home: &Path, id: &str) -> Option<(Run, PathBuf)> {
    let run = Run::load(home, id)?;
    let output = run.dir(home).join(run::OUTPUT);
    Some((run, output))
}

#[cfg(test)]
mod tests;
