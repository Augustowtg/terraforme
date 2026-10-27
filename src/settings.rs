//! Configurações do terra (aba Config), salvas no banco.

use anyhow::Result;

use crate::agent::PermissionMode;
use crate::db::Db;

const START_MODE: &str = "agent.start_mode";
const MODEL: &str = "agent.model";
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
        }
    }

    pub fn save(&self, db: &Db) -> Result<()> {
        db.set_setting(START_MODE, Some(self.start_mode.key()))?;
        db.set_setting(MODEL, self.model.as_deref())?;
        Ok(())
    }
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
            };
            settings.save(&db).unwrap();
            assert_eq!(AgentSettings::load(&db), settings);
        }
        // Voltar ao modelo padrão apaga a configuração.
        AgentSettings::default().save(&db).unwrap();
        assert_eq!(AgentSettings::load(&db).model, None);
    }

    #[test]
    fn unknown_saved_mode_falls_back_to_provider() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting(START_MODE, Some("???")).unwrap();
        assert_eq!(AgentSettings::load(&db).start_mode, StartMode::Provider);
    }
}
