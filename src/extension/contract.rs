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

/// Um programa que inicia o agente no lugar do terra (ex.: `rtk claude`,
/// `ai-memory run claude`). Nos `args`, [`Launcher::HARNESS`] vira o nome do
/// provedor no lançador e [`Launcher::BIN`], o executável dele; um lançador
/// que cita o `HARNESS` inicia o agente sozinho, sem o executável no fim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launcher {
    pub program: String,
    pub args: Vec<String>,
    /// Vão antes do [`Launcher::HARNESS`] numa conversa nova (ex.: o
    /// `--fresh` do `ai-memory run`, que sem ele retoma a última sessão).
    pub fresh: Vec<String>,
}

impl Launcher {
    pub const HARNESS: &'static str = "{harness}";
    pub const BIN: &'static str = "{bin}";

    /// Inicia o agente sozinho (cita o [`Launcher::HARNESS`]).
    pub fn starts_agent(&self) -> bool {
        self.args.iter().any(|a| a == Self::HARNESS)
    }
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
    /// Os lançadores, encadeados na ordem do registro (o primeiro por fora).
    pub launchers: Vec<Launcher>,
}

impl Contribution {
    /// Junta o que `other` acrescenta a este.
    pub fn merge(&mut self, other: Contribution) {
        self.hooks.extend(other.hooks);
        self.mcp_servers.extend(other.mcp_servers);
        self.env.extend(other.env);
        self.launchers.extend(other.launchers);
    }

    /// A linha de comando que inicia `bin` pelos lançadores: os que só
    /// repassam o comando (ex.: `rtk`) por fora e o que inicia o agente
    /// sozinho (ex.: `ai-memory run`) por dentro, já que ele não repassa
    /// outro lançador. Sem lançadores, só o `bin`.
    pub fn launch_line(&self, bin: &str, harness: &str, fresh: bool) -> Vec<String> {
        let (starters, wrappers): (Vec<_>, Vec<_>) =
            self.launchers.iter().partition(|l| l.starts_agent());
        let mut line = Vec::new();
        for launcher in wrappers.iter().chain(starters.iter().take(1)) {
            line.push(launcher.program.clone());
            for arg in &launcher.args {
                match arg.as_str() {
                    Launcher::HARNESS => {
                        if fresh {
                            line.extend(launcher.fresh.iter().cloned());
                        }
                        line.push(harness.to_string());
                    }
                    Launcher::BIN => line.push(bin.to_string()),
                    _ => line.push(arg.clone()),
                }
            }
        }
        if starters.is_empty() {
            line.push(bin.to_string());
        }
        line
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rtk() -> Launcher {
        Launcher {
            program: "rtk".into(),
            args: Vec::new(),
            fresh: vec!["--never".into()],
        }
    }

    fn ai_memory() -> Launcher {
        Launcher {
            program: "ai-memory".into(),
            args: ["run", "--executable", Launcher::BIN, Launcher::HARNESS]
                .map(str::to_string)
                .to_vec(),
            fresh: vec!["--fresh".into()],
        }
    }

    fn line(launchers: Vec<Launcher>) -> Vec<String> {
        let contribution = Contribution {
            launchers,
            ..Contribution::default()
        };
        contribution.launch_line("/bin/claude", "claude", false)
    }

    #[test]
    fn without_launchers_runs_the_agent() {
        assert_eq!(line(Vec::new()), ["/bin/claude"]);
    }

    #[test]
    fn a_wrapper_comes_before_the_agent() {
        assert_eq!(line(vec![rtk()]), ["rtk", "/bin/claude"]);
    }

    #[test]
    fn a_starter_runs_the_agent_by_its_harness() {
        assert_eq!(
            line(vec![ai_memory()]),
            ["ai-memory", "run", "--executable", "/bin/claude", "claude"]
        );
    }

    #[test]
    fn both_chain_with_the_starter_inside() {
        let expected = [
            "rtk",
            "ai-memory",
            "run",
            "--executable",
            "/bin/claude",
            "claude",
        ];
        assert_eq!(line(vec![rtk(), ai_memory()]), expected);
        assert_eq!(line(vec![ai_memory(), rtk()]), expected);
    }

    #[test]
    fn a_new_conversation_gets_the_fresh_args_before_the_harness() {
        let contribution = Contribution {
            launchers: vec![rtk(), ai_memory()],
            ..Contribution::default()
        };
        assert_eq!(
            contribution.launch_line("/bin/claude", "claude", true),
            [
                "rtk",
                "ai-memory",
                "run",
                "--executable",
                "/bin/claude",
                "--fresh",
                "claude"
            ]
        );
    }
}
