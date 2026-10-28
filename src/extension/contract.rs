//! O contrato entre o terra e uma extensão (padrão Strategy).
//!
//! Cada extensão implementa [`Extension`]; o terra só conhece este contrato,
//! então uma extensão nova entra sem mudar a interface: basta implementar o
//! trait e registrá-la em [`super::extensions::all`].

use std::sync::mpsc::Receiver;

/// Se a extensão está instalada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Encontrada, com a versão quando ela informa.
    Installed {
        version: Option<String>,
    },
    Missing,
}

impl Status {
    pub fn is_installed(&self) -> bool {
        matches!(self, Status::Installed { .. })
    }
}

/// Um hook do agente: `command` roda no evento `event` (no formato do
/// Claude Code, ex.: `PreToolUse`) das ferramentas que casam com `matcher`
/// (vazio: todas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub event: String,
    pub matcher: String,
    pub command: String,
}

/// Um servidor MCP que o agente abre (stdio, com `command` e `args`) ou
/// acessa por HTTP (com `url`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// O endereço de um servidor HTTP; com ele, `command` e `args` não valem.
    pub url: Option<String>,
    /// Os cabeçalhos de cada requisição HTTP (ex.: `Authorization`); os
    /// valores podem citar variáveis de [`Contribution::env`] como `${NOME}`.
    pub headers: Vec<(String, String)>,
}

/// O que as extensões acrescentam ao agente. O adapter traduz para o
/// protocolo dele (no Claude, `--settings` e `--mcp-config`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contribution {
    pub hooks: Vec<Hook>,
    pub mcp_servers: Vec<McpServer>,
    /// Variáveis de ambiente do processo do agente: um segredo vai por
    /// aqui, e não pela linha de comando, que outros processos leem.
    pub env: Vec<(String, String)>,
}

impl Contribution {
    /// Junta o que `other` acrescenta a este.
    pub fn merge(&mut self, other: Contribution) {
        self.hooks.extend(other.hooks);
        self.mcp_servers.extend(other.mcp_servers);
        self.env.extend(other.env);
    }
}

/// Um valor do relatório; a tela formata cada tipo.
#[derive(Debug, Clone, PartialEq)]
pub enum StatValue {
    Count(u64),
    Tokens(u64),
    /// De 0 a 100.
    Percent(f64),
    Bytes(u64),
    Text(String),
}

/// Uma linha do relatório (ex.: `Tokens economizados  1.5M`).
#[derive(Debug, Clone, PartialEq)]
pub struct Stat {
    pub label: String,
    pub value: StatValue,
}

impl Stat {
    pub fn new(label: &str, value: StatValue) -> Self {
        Self {
            label: label.to_string(),
            value,
        }
    }
}

/// O que a extensão mostra na aba Extensões.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub stats: Vec<Stat>,
    /// Título do gráfico por dia (ex.: `Tokens economizados por dia`).
    pub daily_title: Option<String>,
    /// Um valor por dia, o mais antigo primeiro; vazio sem gráfico.
    pub daily: Vec<(jiff::civil::Date, u64)>,
}

/// Uma extensão do agente.
pub trait Extension {
    /// O que fica salvo no `config.json` (ex.: `rtk`).
    fn id(&self) -> &'static str;

    /// Nome na tela (ex.: `RTK`).
    fn name(&self) -> &'static str;

    /// Uma frase sobre o que ela faz.
    fn description(&self) -> &'static str;

    /// Onde conhecer e instalar.
    fn homepage(&self) -> &'static str;

    fn status(&self) -> Status;

    /// O que ela acrescenta ao agente quando está ativa no workspace.
    fn contribution(&self) -> Result<Contribution, String>;

    /// Começa a carregar, em segundo plano, o relatório da aba Extensões.
    /// `None` quando ela não tem relatório.
    fn load_report(&self) -> Option<Receiver<Result<Report, String>>> {
        None
    }
}
