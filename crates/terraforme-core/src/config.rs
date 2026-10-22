//! Configuração do terraforme, em `~/.config/terraforme/config.toml`
//! (`%APPDATA%\terraforme\config.toml` no Windows).

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::agent::{Mode, Provider};
use crate::limits::Budget;
use crate::mcp::McpConfig;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Agente de IA executado por `terraforme cli`.
    pub provider: Provider,
    /// Modo usado quando `terraforme cli` é chamado sem --eco/--high/--hybrid.
    pub mode: Mode,
    /// Roda o agente dentro do AI Jail (sandbox) quando `terraforme cli` é
    /// chamado sem --jail/--no-jail.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub jail: bool,
    /// Orçamento de tokens da aba de consumo, sem os limites do plano.
    #[serde(skip_serializing_if = "Budget::is_empty")]
    pub budget: Budget,
    /// Configuração do daemon MCP compartilhado.
    #[serde(default, skip_serializing_if = "McpConfig::is_default")]
    pub mcp: McpConfig,
    /// Preferências da TUI do `terraforme cli` (seção `[tui]`).
    #[serde(default, skip_serializing_if = "TuiConfig::is_default")]
    pub tui: TuiConfig,
}

/// Seção `[tui]` do config.toml, editável na aba Config da TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Captura o mouse ao abrir: a roda rola o histórico e arrastar seleciona
    /// pela própria TUI. Desligado, a seleção é a nativa do terminal.
    pub mouse: bool,
    /// Copia o texto para a área de transferência ao soltar o botão do mouse.
    pub copy_on_select: bool,
    /// Abre a janela flutuante com o mascote (`terraforme pet`) junto com o
    /// `terraforme cli`. Só no Linux por enquanto.
    pub pet: bool,
    /// Middleware (experimental): as instâncias novas conversam com o agente
    /// por um chat desenhado pela própria TUI ([`crate::chat`]) em vez do
    /// terminal dele. Só nos agentes com `Adapter::chat_args`.
    pub middleware: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            mouse: true,
            copy_on_select: true,
            pet: false,
            middleware: false,
        }
    }
}

impl TuiConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Config {
    /// Lê a configuração; sem arquivo, devolve os valores padrão.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lendo {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(Self::dir()?)?;
        fs::write(&path, toml::to_string(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    pub fn path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("config.toml"))
    }

    pub fn dir() -> Result<PathBuf> {
        let base = if cfg!(windows) {
            env::var_os("APPDATA").map(PathBuf::from)
        } else {
            env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| crate::agent::home().ok().map(|h| h.join(".config")))
        };
        Ok(base
            .context("não foi possível descobrir a pasta de configuração do usuário")?
            .join("terraforme"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip() {
        let config = Config {
            provider: Provider::Claude,
            mode: Mode::Eco,
            jail: false,
            budget: Budget::default(),
            mcp: McpConfig::default(),
            tui: TuiConfig::default(),
        };
        let text = toml::to_string(&config).unwrap();
        assert_eq!(text, "provider = \"claude\"\nmode = \"eco\"\n");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);

        let jailed = Config {
            jail: true,
            ..config
        };
        let text = toml::to_string(&jailed).unwrap();
        assert_eq!(text, "provider = \"claude\"\nmode = \"eco\"\njail = true\n");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), jailed);

        let config = Config {
            budget: Budget {
                five_hours: Some(1_000),
                week: None,
            },
            ..config
        };
        let text = toml::to_string(&config).unwrap();
        assert!(text.contains("[budget]\nfive_hours = 1000\n"), "{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let config: Config = toml::from_str("mode = \"high\"").unwrap();
        assert_eq!(config.provider, Provider::Claude);
        assert_eq!(config.mode, Mode::High);
        assert!(!config.jail);
        assert_eq!(config.mcp, McpConfig::default());
        assert_eq!(config.tui, TuiConfig::default());

        let config: Config = toml::from_str("[tui]\nmouse = false").unwrap();
        assert!(!config.tui.mouse);
        assert!(config.tui.copy_on_select);
        assert!(!config.tui.middleware);
    }
}
