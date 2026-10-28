//! Configurações do terra (aba Config).
//!
//! As do agente são de cada workspace: ficam em
//! `~/.terraforme/workspace/<nome do workspace>/config.json` ([`config_file`]),
//! lido ao iniciar. Um workspace sem o arquivo começa com o padrão do router
//! (o provedor): o modo, o modelo, a conta e o esforço que ele usa.
//!
//! ```json
//! {
//!   "agent": {
//!     "provider": "claude",
//!     "start_mode": "accept_edits",
//!     "model": "sonnet",
//!     "account": ".claude-trabalho",
//!     "effort": "high",
//!     "interface": "terraforme",
//!     "context": { "display": "hidden", "auto_compact": false, "limit": 200000, "compact_at": 75 },
//!     "usage": { "display": "hidden", "total_display": "visible" }
//!   },
//!   "extensions": { "enabled": ["rtk", "ai-memory"] }
//! }
//! ```
//!
//! As extensões ativas ([`ExtensionSettings`]) também são de cada workspace;
//! sem a chave, nenhuma.
//!
//! O MCP ([`McpSettings`]) é um só para a máquina, então fica no
//! `~/.terraforme/config.json` ([`global_file`]):
//! `{"mcp": {"enabled": true, "spawn_mode": "accept_edits"}}`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::agent::PermissionMode;

const PROVIDER: &str = "provider";

/// Modo de permissão com que o chat começa.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// O modo padrão configurado no provedor (ex.: o `defaultMode` do Claude).
    #[default]
    Provider,
    Fixed(PermissionMode),
}

impl StartMode {
    pub const ALL: [StartMode; 6] = [
        StartMode::Provider,
        StartMode::Fixed(PermissionMode::Agente),
        StartMode::Fixed(PermissionMode::Yolo),
        StartMode::Fixed(PermissionMode::Manual),
        StartMode::Fixed(PermissionMode::AcceptEdits),
        StartMode::Fixed(PermissionMode::Plan),
    ];

    pub fn label(self) -> &'static str {
        match self {
            StartMode::Provider => "Do provedor",
            StartMode::Fixed(mode) => mode.label(),
        }
    }

    fn key(self) -> &'static str {
        match self {
            StartMode::Provider => PROVIDER,
            StartMode::Fixed(mode) => mode.key(),
        }
    }

    fn from_key(key: &str) -> Option<StartMode> {
        if key == PROVIDER {
            return Some(StartMode::Provider);
        }
        PermissionMode::from_key(key).map(StartMode::Fixed)
    }
}

/// A interface do chat e do input: a do terra ou a original do provedor
/// (ex.: o `claude` interativo), aberta com as mesmas configurações.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    #[default]
    Terraforme,
    Provider,
}

impl Interface {
    pub const ALL: [Interface; 2] = [Interface::Terraforme, Interface::Provider];

    pub fn label(self) -> &'static str {
        match self {
            Interface::Terraforme => "Agente Terraforme",
            Interface::Provider => "Original do provedor",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Interface::Terraforme => "terraforme",
            Interface::Provider => PROVIDER,
        }
    }

    fn from_key(key: &str) -> Option<Interface> {
        Interface::ALL.into_iter().find(|i| i.key() == key)
    }
}

/// Configurações do contexto da sessão (a barra embaixo do chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSettings {
    /// A barra de contexto aparece no chat.
    pub visible: bool,
    /// Compactar a conversa sozinho quando o contexto chega em `compact_at`.
    pub auto_compact: bool,
    /// Limite de tokens do contexto; `None` é o do modelo.
    pub limit: Option<u64>,
    /// Ponto de compactar, em % do limite: a barra fica cheia e
    /// mostra "Compacte a conversa" ao chegar nele.
    pub compact_at: u8,
}

impl ContextSettings {
    /// Pontos de compactar que a Config oferece (% do limite).
    pub const COMPACT_AT: [u8; 10] = [50, 55, 60, 65, 70, 75, 80, 85, 90, 95];

    /// Limites de tokens que a Config oferece, além do do modelo.
    pub const LIMITS: [u64; 10] = [
        50_000, 100_000, 150_000, 200_000, 250_000, 300_000, 400_000, 500_000, 750_000, 1_000_000,
    ];

    /// O limite de tokens: o escolhido ou, sem ele, o do modelo (`window`).
    pub fn limit(self, window: u64) -> u64 {
        self.limit.unwrap_or(window)
    }

    /// Tokens em que a conversa deve ser compactada, com o limite do modelo
    /// em `window`.
    pub fn compact_tokens(self, window: u64) -> u64 {
        self.limit(window) * u64::from(self.compact_at) / 100
    }

    fn load(json: &Value) -> Self {
        let default = Self::default();
        Self {
            visible: json["display"]
                .as_str()
                .map_or(default.visible, |d| d != "hidden"),
            auto_compact: json["auto_compact"]
                .as_bool()
                .unwrap_or(default.auto_compact),
            // Sem a chave, o padrão; `null` é o do modelo. Qualquer valor
            // positivo vale, mesmo fora das opções da Config.
            limit: match json.get("limit") {
                None => default.limit,
                Some(limit) => limit.as_u64().filter(|l| *l > 0),
            },
            compact_at: json["compact_at"]
                .as_u64()
                .and_then(|p| u8::try_from(p).ok())
                .filter(|p| Self::COMPACT_AT.contains(p))
                .unwrap_or(default.compact_at),
        }
    }

    fn to_json(self) -> Value {
        json!({
            "display": if self.visible { "visible" } else { "hidden" },
            "auto_compact": self.auto_compact,
            "limit": self.limit,
            "compact_at": self.compact_at,
        })
    }
}

/// Oculto, sem compactar sozinho, com o limite em 200K tokens e o ponto de
/// compactar em 75% dele.
impl Default for ContextSettings {
    fn default() -> Self {
        Self {
            visible: false,
            auto_compact: false,
            limit: Some(200_000),
            compact_at: 75,
        }
    }
}

/// Configurações do consumo (a barra do consumo da sessão de 5 horas, ao
/// lado da de contexto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageSettings {
    /// A barra de consumo aparece no chat.
    pub visible: bool,
    /// O consumo geral (a soma dos chats do terra) aparece ao lado do "Sair".
    pub total_visible: bool,
}

impl UsageSettings {
    fn load(json: &Value) -> Self {
        Self {
            visible: json["display"]
                .as_str()
                .map_or(Self::default().visible, |d| d != "hidden"),
            total_visible: json["total_display"]
                .as_str()
                .map_or(Self::default().total_visible, |d| d != "hidden"),
        }
    }

    fn to_json(self) -> Value {
        let display = |visible: bool| if visible { "visible" } else { "hidden" };
        json!({ "display": display(self.visible), "total_display": display(self.total_visible) })
    }
}

/// A barra oculta e o consumo geral visível.
impl Default for UsageSettings {
    fn default() -> Self {
        Self {
            visible: false,
            total_visible: true,
        }
    }
}

/// Configurações da seção Agente. O padrão ([`Default`]) é o do router: tudo
/// fica com o provedor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AgentSettings {
    /// Provedor escolhido (o `id` de um adapter, ex.: `claude` ou
    /// `antigravity`); `None` é o primeiro disponível. A conta, os modelos e
    /// o esforço são ids desse provedor.
    pub provider: Option<String>,
    pub start_mode: StartMode,
    /// Modelo escolhido (um id dos modelos do provedor); `None` é o padrão.
    pub model: Option<String>,
    /// Conta escolhida (um id das contas do provedor); `None` é a Default.
    pub account: Option<String>,
    /// Esforço padrão (escolhido no `/model`); `None` é o do provedor.
    pub effort: Option<String>,
    /// Modelo dos sub agentes (escolhido no menu do `/model` da Config);
    /// `None` deixa o agente decidir.
    pub subagent_model: Option<String>,
    pub interface: Interface,
    pub context: ContextSettings,
    pub usage: UsageSettings,
}

impl AgentSettings {
    /// Lê o `config.json` de um workspace. `None` quando ele não existe ou
    /// não é JSON; o que faltar (ou não for reconhecido) fica no padrão.
    pub fn load_file(path: &Path) -> Option<Self> {
        let json: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let agent = &json["agent"];
        let text = |key: &str| agent[key].as_str().map(str::to_string);
        Some(Self {
            provider: text("provider"),
            start_mode: text("start_mode")
                .and_then(|key| StartMode::from_key(&key))
                .unwrap_or_default(),
            model: text("model"),
            account: text("account"),
            effort: text("effort"),
            subagent_model: text("subagent_model"),
            interface: text("interface")
                .and_then(|key| Interface::from_key(&key))
                .unwrap_or_default(),
            context: ContextSettings::load(&agent["context"]),
            usage: UsageSettings::load(&agent["usage"]),
        })
    }

    /// Grava o `config.json` de um workspace, criando a pasta. Outras chaves
    /// do arquivo (de fora da seção do agente) são mantidas.
    pub fn save_file(&self, path: &Path) -> Result<()> {
        update_file(path, |json| {
            json["agent"] = json!({
                "provider": self.provider,
                "start_mode": self.start_mode.key(),
                "model": self.model,
                "account": self.account,
                "effort": self.effort,
                "subagent_model": self.subagent_model,
                "interface": self.interface.key(),
                "context": self.context.to_json(),
                "usage": self.usage.to_json(),
            });
        })
    }
}

/// As extensões ativas no workspace (seção Extensões da Config).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExtensionSettings {
    /// Ids das extensões ativas, na ordem em que foram escolhidas.
    pub enabled: Vec<String>,
}

impl ExtensionSettings {
    /// Lê as extensões do `config.json` de um workspace; sem o arquivo ou a
    /// chave, nenhuma.
    pub fn load_file(path: &Path) -> Self {
        let enabled = read_file(path)["extensions"]["enabled"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_string))
            .collect();
        Self { enabled }
    }

    /// Grava as extensões no `config.json`, mantendo as outras chaves.
    pub fn save_file(&self, path: &Path) -> Result<()> {
        update_file(path, |json| {
            json["extensions"] = json!({ "enabled": self.enabled });
        })
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.iter().any(|e| e == id)
    }

    /// Liga a extensão `id`, ou desliga se ela já estava ativa.
    pub fn toggle(&mut self, id: &str) {
        if self.is_enabled(id) {
            self.enabled.retain(|e| e != id);
        } else {
            self.enabled.push(id.to_string());
        }
    }
}

/// O MCP do terra (seção MCP da Config): o servidor orquestrador, um só para
/// a máquina.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSettings {
    /// Ao iniciar, o terra abre o MCP (se ninguém abriu) e se registra nele.
    /// Desativado por padrão.
    pub enabled: bool,
    /// O modo com que começam os chats abertos pelo orquestrador (a
    /// ferramenta `chat`). Accept Edits por padrão.
    pub spawn_mode: StartMode,
    /// O chat delegado fecha quando termina a tarefa (a resposta continua
    /// indo para quem delegou). Ativado por padrão.
    pub close_finished: bool,
    /// O chat delegado aparece na aba Chat ao abrir, em vez de ficar em
    /// segundo plano na barra de chats. Desativado por padrão.
    pub focus_delegated: bool,
    /// O modelo (um id do catálogo) e o esforço dos chats delegados,
    /// escolhidos no menu do `/model` da aba Orquestrador. Sem modelo, eles
    /// usam o modelo padrão.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// O provedor dos chats delegados (o `id` de um adapter); `None` é o
    /// mesmo do workspace. O modelo e o esforço são ids dele.
    pub provider: Option<String>,
}

impl McpSettings {
    pub const DEFAULT_SPAWN_MODE: StartMode = StartMode::Fixed(PermissionMode::AcceptEdits);

    /// Lê o MCP do `config.json` global; sem o arquivo ou a chave, desativado,
    /// com os chats abertos em Accept Edits, em segundo plano, e fechados ao
    /// terminar.
    pub fn load_file(path: &Path) -> Self {
        let json = read_file(path);
        Self {
            enabled: json["mcp"]["enabled"] == true,
            spawn_mode: json["mcp"]["spawn_mode"]
                .as_str()
                .and_then(StartMode::from_key)
                .unwrap_or(Self::DEFAULT_SPAWN_MODE),
            close_finished: json["mcp"]["close_finished"] != false,
            focus_delegated: json["mcp"]["focus_delegated"] == true,
            model: json["mcp"]["model"].as_str().map(str::to_string),
            effort: json["mcp"]["effort"].as_str().map(str::to_string),
            provider: json["mcp"]["provider"].as_str().map(str::to_string),
        }
    }

    /// Grava o MCP no `config.json` global, mantendo as outras chaves.
    pub fn save_file(&self, path: &Path) -> Result<()> {
        update_file(path, |json| {
            json["mcp"] = json!({
                "enabled": self.enabled,
                "spawn_mode": self.spawn_mode.key(),
                "close_finished": self.close_finished,
                "focus_delegated": self.focus_delegated,
                "model": self.model,
                "effort": self.effort,
                "provider": self.provider,
            });
        })
    }
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            spawn_mode: Self::DEFAULT_SPAWN_MODE,
            close_finished: true,
            focus_delegated: false,
            model: None,
            effort: None,
            provider: None,
        }
    }
}

/// Se o terra pergunta, ao sair, se o workspace deve ser salvo. O "Não
/// (não perguntar novamente)" grava `"workspace": {"ask_to_save": false}`.
pub fn asks_to_save(path: &Path) -> bool {
    read_file(path)["workspace"]["ask_to_save"] != false
}

/// O "Não (não perguntar novamente)": não pergunta mais neste workspace.
pub fn stop_asking_to_save(path: &Path) -> Result<()> {
    update_file(path, |json| {
        if !json["workspace"].is_object() {
            json["workspace"] = json!({});
        }
        json["workspace"]["ask_to_save"] = json!(false);
    })
}

/// O `config.json` como objeto (vazio quando não existe ou não é JSON).
fn read_file(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Lê o `config.json`, aplica `change` e grava, criando a pasta. As chaves
/// que `change` não mexe ficam como estavam.
fn update_file(path: &Path, change: impl FnOnce(&mut Value)) -> Result<()> {
    let mut json = read_file(path);
    change(&mut json);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("não foi possível criar {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&json)? + "\n";
    fs::write(path, text).with_context(|| format!("não foi possível gravar {}", path.display()))
}

/// O `config.json` global, das configurações da máquina toda, dentro de
/// `root` (o `~/.terraforme`).
pub fn global_file(root: &Path) -> PathBuf {
    root.join("config.json")
}

/// O `config.json` do workspace `name` dentro de `root` (o
/// `~/.terraforme`). Separadores no nome viram `-`, para o arquivo ficar
/// sempre dentro de `workspace/`.
pub fn config_file(root: &Path, name: &str) -> PathBuf {
    let name: String = name
        .trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\') { '-' } else { c })
        .collect();
    let name = match name.as_str() {
        "" | "." | ".." => "root".to_string(),
        _ => name,
    };
    root.join("workspace").join(name).join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_file_round_trip_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "terraforme-cli");
        assert_eq!(
            path,
            dir.path().join("workspace/terraforme-cli/config.json")
        );
        assert_eq!(AgentSettings::load_file(&path), None, "sem arquivo");

        let settings = AgentSettings {
            provider: Some("antigravity".into()),
            start_mode: StartMode::Fixed(PermissionMode::AcceptEdits),
            model: Some("sonnet".into()),
            account: Some(".claude-trabalho".into()),
            effort: None,
            subagent_model: Some("haiku".into()),
            interface: Interface::Provider,
            context: ContextSettings {
                visible: false,
                auto_compact: true,
                limit: Some(300_000),
                compact_at: 90,
            },
            usage: UsageSettings {
                visible: true,
                total_visible: false,
            },
        };
        settings.save_file(&path).unwrap();
        assert_eq!(AgentSettings::load_file(&path), Some(settings.clone()));
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["agent"]["provider"], "antigravity");
        assert_eq!(json["agent"]["start_mode"], "accept_edits");
        assert_eq!(json["agent"]["effort"], Value::Null);
        assert_eq!(json["agent"]["subagent_model"], "haiku");
        assert_eq!(json["agent"]["interface"], "provider");
        assert_eq!(
            json["agent"]["context"],
            json!({"display": "hidden", "auto_compact": true, "limit": 300_000, "compact_at": 90})
        );
        assert_eq!(
            json["agent"]["usage"],
            json!({"display": "visible", "total_display": "hidden"})
        );

        // Chaves de fora do agente ficam no arquivo.
        fs::write(&path, r#"{"outra": 1, "agent": {"model": "opus"}}"#).unwrap();
        assert_eq!(
            AgentSettings::load_file(&path).unwrap().model.as_deref(),
            Some("opus")
        );
        settings.save_file(&path).unwrap();
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["outra"], 1);
        assert_eq!(json["agent"]["model"], "sonnet");
    }

    #[test]
    fn extensions_toggle_and_round_trip_with_the_agent_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "terra");
        assert_eq!(
            ExtensionSettings::load_file(&path),
            ExtensionSettings::default()
        );

        let mut extensions = ExtensionSettings::default();
        extensions.toggle("rtk");
        extensions.toggle("ai-memory");
        assert!(extensions.is_enabled("rtk"));
        extensions.toggle("rtk");
        assert_eq!(extensions.enabled, ["ai-memory"]);

        let agent = AgentSettings {
            model: Some("opus".into()),
            ..AgentSettings::default()
        };
        agent.save_file(&path).unwrap();
        extensions.save_file(&path).unwrap();
        agent.save_file(&path).unwrap();
        assert_eq!(ExtensionSettings::load_file(&path), extensions);
        assert_eq!(AgentSettings::load_file(&path), Some(agent));
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["extensions"], json!({"enabled": ["ai-memory"]}));
    }

    #[test]
    fn stop_asking_to_save_keeps_the_agent_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "terra");
        assert!(asks_to_save(&path), "pergunta por padrão");
        let settings = AgentSettings {
            model: Some("opus".into()),
            ..AgentSettings::default()
        };
        settings.save_file(&path).unwrap();
        stop_asking_to_save(&path).unwrap();
        assert!(!asks_to_save(&path));
        assert_eq!(AgentSettings::load_file(&path), Some(settings.clone()));
        // Salvar o agente de novo não volta a perguntar.
        settings.save_file(&path).unwrap();
        assert!(!asks_to_save(&path));
    }

    #[test]
    fn mcp_defaults_to_disabled_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = global_file(dir.path());
        assert_eq!(path, dir.path().join("config.json"));
        assert!(
            !McpSettings::load_file(&path).enabled,
            "desativado por padrão"
        );
        fs::write(&path, r#"{"outra": 1}"#).unwrap();
        assert_eq!(
            McpSettings::load_file(&path).spawn_mode,
            StartMode::Fixed(PermissionMode::AcceptEdits),
            "os chats do orquestrador começam em Accept Edits"
        );
        assert!(
            McpSettings::load_file(&path).close_finished,
            "os chats do orquestrador fecham ao terminar por padrão"
        );
        assert!(!McpSettings::load_file(&path).focus_delegated);
        fs::write(&path, r#"{"outra": 1}"#).unwrap();
        let mcp = McpSettings {
            enabled: true,
            spawn_mode: StartMode::Fixed(PermissionMode::Yolo),
            close_finished: false,
            focus_delegated: true,
            model: Some("sonnet".into()),
            effort: Some("high".into()),
            provider: Some("claude".into()),
        };
        mcp.save_file(&path).unwrap();
        assert_eq!(McpSettings::load_file(&path), mcp);
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            json!({"outra": 1, "mcp": {
                "enabled": true,
                "spawn_mode": "yolo",
                "close_finished": false,
                "focus_delegated": true,
                "model": "sonnet",
                "effort": "high",
                "provider": "claude",
            }})
        );
    }

    #[test]
    fn usage_bar_defaults_to_hidden() {
        assert!(!UsageSettings::default().visible);
        assert_eq!(UsageSettings::load(&json!({})), UsageSettings::default());
        assert!(UsageSettings::load(&json!({"display": "visible"})).visible);
    }

    #[test]
    fn context_defaults_to_hidden_without_auto_compact_at_75_of_200k() {
        let context = ContextSettings::default();
        assert!(!context.visible);
        assert!(!context.auto_compact);
        assert_eq!(context.compact_at, 75);
        assert_eq!(context.limit, Some(200_000));
        // O limite padrão vale mesmo com o modelo informando outro.
        assert_eq!(context.compact_tokens(1_000_000), 150_000);
        // Um arquivo sem as chaves fica no padrão.
        assert_eq!(ContextSettings::load(&json!({})), context);
    }

    #[test]
    fn custom_limit_replaces_the_model_window() {
        let context = ContextSettings {
            limit: Some(100_000),
            ..ContextSettings::default()
        };
        assert_eq!(context.limit(1_000_000), 100_000);
        assert_eq!(context.compact_tokens(1_000_000), 75_000);
        let load = |text: &str| ContextSettings::load(&serde_json::from_str(text).unwrap());
        assert_eq!(load(r#"{"limit": 123456}"#).limit, Some(123_456));
        assert_eq!(load(r#"{"limit": 0}"#).limit, None);
        assert_eq!(load(r#"{"limit": null}"#).limit, None, "o do modelo");
    }

    #[test]
    fn config_file_stays_inside_workspace_folder() {
        let root = Path::new("/r");
        assert_eq!(
            config_file(root, "a/b"),
            Path::new("/r/workspace/a-b/config.json")
        );
        assert_eq!(
            config_file(root, ".."),
            Path::new("/r/workspace/root/config.json")
        );
    }

    #[test]
    fn unknown_or_missing_values_fall_back_to_the_router_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path(), "terra");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"agent": {"start_mode": "???", "interface": "???", "context": {"compact_at": 42}}}"#,
        )
        .unwrap();
        assert_eq!(
            AgentSettings::load_file(&path),
            Some(AgentSettings::default())
        );
        fs::write(&path, "não é json").unwrap();
        assert_eq!(AgentSettings::load_file(&path), None);
    }
}
