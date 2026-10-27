//! Configurações do terra (aba Config).
//!
//! As do agente são de cada workspace: ficam em
//! `~/.terraforme/workspace/<nome do workspace>/config.json` ([`config_file`]).
//! As do banco são as globais de antes; um workspace sem arquivo começa com
//! elas.
//!
//! ```json
//! {
//!   "agent": {
//!     "start_mode": "accept_edits",
//!     "model": "sonnet",
//!     "account": ".claude-trabalho",
//!     "effort": "high"
//!   }
//! }
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::agent::PermissionMode;
use crate::db::Db;

const START_MODE: &str = "agent.start_mode";
const MODEL: &str = "agent.model";
const ACCOUNT: &str = "agent.account";
const EFFORT: &str = "agent.effort";
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

/// Configurações da seção Agente.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AgentSettings {
    pub start_mode: StartMode,
    /// Modelo escolhido (um id dos modelos do provedor); `None` é o padrão.
    pub model: Option<String>,
    /// Conta escolhida (um id das contas do provedor); `None` é a Default.
    pub account: Option<String>,
    /// Esforço padrão (escolhido no `/model`); `None` é o do provedor.
    pub effort: Option<String>,
}

impl AgentSettings {
    /// Lê do banco; o que faltar (ou não for reconhecido) fica no padrão.
    pub fn load(db: &Db) -> Self {
        let get = |key| db.setting(key).ok().flatten();
        Self {
            start_mode: get(START_MODE)
                .and_then(|key| StartMode::from_key(&key))
                .unwrap_or_default(),
            model: get(MODEL),
            account: get(ACCOUNT),
            effort: get(EFFORT),
        }
    }

    /// Grava as configurações globais. O terra grava no `config.json` do
    /// workspace ([`Self::save_file`]); o banco só é lido.
    #[cfg(test)]
    pub fn save(&self, db: &Db) -> Result<()> {
        db.set_setting(START_MODE, Some(self.start_mode.key()))?;
        db.set_setting(MODEL, self.model.as_deref())?;
        db.set_setting(ACCOUNT, self.account.as_deref())?;
        db.set_setting(EFFORT, self.effort.as_deref())?;
        Ok(())
    }

    /// Lê o `config.json` de um workspace. `None` quando ele não existe ou
    /// não é JSON; o que faltar (ou não for reconhecido) fica no padrão.
    pub fn load_file(path: &Path) -> Option<Self> {
        let json: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let agent = &json["agent"];
        let text = |key: &str| agent[key].as_str().map(str::to_string);
        Some(Self {
            start_mode: text("start_mode")
                .and_then(|key| StartMode::from_key(&key))
                .unwrap_or_default(),
            model: text("model"),
            account: text("account"),
            effort: text("effort"),
        })
    }

    /// Grava o `config.json` de um workspace, criando a pasta. Outras chaves
    /// do arquivo (de fora da seção do agente) são mantidas.
    pub fn save_file(&self, path: &Path) -> Result<()> {
        let mut json: Value = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        json["agent"] = json!({
            "start_mode": self.start_mode.key(),
            "model": self.model,
            "account": self.account,
            "effort": self.effort,
        });
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("não foi possível criar {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&json)? + "\n";
        fs::write(path, text).with_context(|| format!("não foi possível gravar {}", path.display()))
    }
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
    fn defaults_when_nothing_saved() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(AgentSettings::load(&db), AgentSettings::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let db = Db::open_in_memory().unwrap();
        for start_mode in StartMode::ALL {
            let settings = AgentSettings {
                start_mode,
                model: Some("haiku".into()),
                account: Some(".claude-trabalho".into()),
                effort: Some("max".into()),
            };
            settings.save(&db).unwrap();
            assert_eq!(AgentSettings::load(&db), settings);
        }
        // Voltar ao modelo e à conta padrão apaga as configurações.
        AgentSettings::default().save(&db).unwrap();
        assert_eq!(AgentSettings::load(&db).model, None);
        assert_eq!(AgentSettings::load(&db).account, None);
    }

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
            start_mode: StartMode::Fixed(PermissionMode::AcceptEdits),
            model: Some("sonnet".into()),
            account: Some(".claude-trabalho".into()),
            effort: None,
        };
        settings.save_file(&path).unwrap();
        assert_eq!(AgentSettings::load_file(&path), Some(settings.clone()));
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["agent"]["start_mode"], "accept_edits");
        assert_eq!(json["agent"]["effort"], Value::Null);

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
    fn unknown_saved_mode_falls_back_to_provider() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting(START_MODE, Some("???")).unwrap();
        assert_eq!(AgentSettings::load(&db).start_mode, StartMode::Provider);
    }
}
