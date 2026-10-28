//! [ai-memory](https://github.com/akitaonrails/ai-memory): memória de longo
//! prazo para agentes, num wiki em markdown versionado com git.
//!
//! No agente, ela é o servidor MCP (as ferramentas de memória) e os hooks
//! do ciclo de vida (que gravam prompts, ferramentas e sessões). As duas
//! partes são as que o próprio `ai-memory` imprime para o Claude Code
//! (`install-mcp` e `install-hooks`, sem `--apply`): o terra só as passa
//! para o agente, sem mudar a configuração do Claude. O relatório vem do
//! `ai-memory status --json`.

use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use super::{find_program, first_json, run, version};
use crate::extension::{Contribution, Extension, Hook, McpServer, Report, Stat, StatValue, Status};

pub struct AiMemory {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    version: Option<String>,
    /// O que ela acrescenta ao agente, lido uma vez do `ai-memory`.
    contribution: OnceCell<Result<Contribution, String>>,
}

impl AiMemory {
    /// Procura o `ai-memory` no `PATH`.
    pub fn detect() -> Self {
        let bin = find_program("ai-memory");
        let version = bin.as_deref().and_then(version);
        Self {
            bin,
            version,
            contribution: OnceCell::new(),
        }
    }

    fn load_contribution(&self) -> Result<Contribution, String> {
        let bin = self.bin.as_deref().ok_or("ai-memory não encontrado")?;
        let hooks = run(bin, &["install-hooks", "--agent", "claude-code"])?;
        let mcp = run(bin, &["install-mcp", "--client", "claude-code"])?;
        Ok(Contribution {
            hooks: parse_hooks(&hooks)?,
            mcp_servers: parse_mcp(&mcp)?,
            env: Vec::new(),
        })
    }
}

impl Extension for AiMemory {
    fn id(&self) -> &'static str {
        "ai-memory"
    }

    fn name(&self) -> &'static str {
        "ai-memory"
    }

    fn description(&self) -> &'static str {
        "Memória de longo prazo entre sessões e agentes (MCP e hooks)"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/akitaonrails/ai-memory"
    }

    fn status(&self) -> Status {
        match self.bin {
            Some(_) => Status::Installed {
                version: self.version.clone(),
            },
            None => Status::Missing,
        }
    }

    fn contribution(&self) -> Result<Contribution, String> {
        self.contribution
            .get_or_init(|| self.load_contribution())
            .clone()
    }

    fn load_report(&self) -> Option<Receiver<Result<Report, String>>> {
        let bin = self.bin.clone()?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let report = run(&bin, &["status", "--json"]).and_then(|text| parse_status(&text));
            let _ = tx.send(report);
        });
        Some(rx)
    }
}

/// Os hooks do `install-hooks --agent claude-code`: um bloco `hooks` no
/// formato dos settings do Claude Code, depois de alguns comentários.
fn parse_hooks(text: &str) -> Result<Vec<Hook>, String> {
    let json = first_json(text).ok_or("o ai-memory não informou os hooks")?;
    let events = json["hooks"]
        .as_object()
        .ok_or("o ai-memory não informou os hooks")?;
    let mut hooks = Vec::new();
    for (event, groups) in events {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group["matcher"].as_str().unwrap_or_default();
            let commands = group["hooks"].as_array().into_iter().flatten();
            for hook in commands.filter(|h| h["type"] == "command") {
                if let Some(command) = hook["command"].as_str() {
                    hooks.push(Hook {
                        event: event.clone(),
                        matcher: matcher.to_string(),
                        command: command.to_string(),
                    });
                }
            }
        }
    }
    Ok(hooks)
}

/// Os servidores do `install-mcp --client claude-code`: o JSON
/// `mcpServers` que ele sugere para o `.claude.json`.
fn parse_mcp(text: &str) -> Result<Vec<McpServer>, String> {
    let servers = first_json(text)
        .and_then(|json| json["mcpServers"].as_object().cloned())
        .ok_or("o ai-memory não informou o servidor MCP")?;
    Ok(servers
        .into_iter()
        .filter_map(|(name, server)| {
            Some(McpServer {
                name,
                command: server["command"].as_str()?.to_string(),
                args: server["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str().map(str::to_string))
                    .collect(),
                url: None,
                headers: Vec::new(),
            })
        })
        .collect())
}

/// O relatório a partir do `ai-memory status --json`.
fn parse_status(text: &str) -> Result<Report, String> {
    let json = first_json(text).ok_or("resposta inesperada do ai-memory")?;
    let counts = &json["counts"];
    let count = |key: &str| StatValue::Count(counts[key].as_u64().unwrap_or(0));
    let provider = |key: &str| {
        let provider = &json["providers"][key];
        let status = provider["status"].as_str().unwrap_or("desconhecido");
        let text = match provider["model"].as_str() {
            Some(model) => format!("{} · {model}", status_label(status)),
            None => status_label(status).to_string(),
        };
        StatValue::Text(text)
    };
    let mut stats = vec![
        Stat::new("Páginas", count("pages_latest")),
        Stat::new("Sessões", count("sessions")),
        Stat::new("Observações", count("observations")),
        Stat::new(
            "Banco",
            StatValue::Bytes(json["storage"]["database_bytes"].as_u64().unwrap_or(0)),
        ),
        Stat::new("Embeddings", provider("embedding")),
        Stat::new("LLM", provider("llm")),
    ];
    if let Some(version) = json["version"].as_str() {
        stats.push(Stat::new("Versão", StatValue::Text(version.to_string())));
    }
    Ok(Report {
        stats,
        ..Report::default()
    })
}

fn status_label(status: &str) -> &str {
    match status {
        "ok" => "ok",
        "disabled" => "desligado",
        "error" => "com erro",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOKS: &str = r#"[ai-memory] capture-policy capability v1 enforced
# Claude Code hook config — merge into /home/eu/.claude/settings.json
# Hook scripts: /home/eu/.local/lib/ai-memory/hooks/claude-code

{
  "hooks": {
    "SessionStart": [
      {"matcher": "", "hooks": [{"type": "command", "command": "ai-memory hook --event session-start"}]}
    ],
    "PreToolUse": [
      {"matcher": "", "hooks": [
        {"type": "command", "command": "ai-memory hook --event pre-tool-use"},
        {"type": "outro", "command": "ignorado"}
      ]}
    ]
  }
}
"#;

    const MCP: &str = r#"# Claude Code — register the MCP server
claude mcp add --transport stdio ai-memory -- \
    ai-memory mcp-bridge --server-url http://127.0.0.1:49374/mcp
#
# Equivalent JSON if you'd rather edit /home/eu/.claude.json directly:
{
  "mcpServers": {
    "ai-memory": {
      "type": "stdio",
      "command": "ai-memory",
      "args": ["mcp-bridge", "--server-url", "http://127.0.0.1:49374/mcp"]
    }
  }
}
"#;

    #[test]
    fn hooks_come_from_the_printed_settings_block() {
        let hooks = parse_hooks(HOOKS).unwrap();
        let events: Vec<_> = hooks.iter().map(|h| h.event.as_str()).collect();
        assert_eq!(events, ["PreToolUse", "SessionStart"]);
        assert_eq!(hooks[0].command, "ai-memory hook --event pre-tool-use");
        assert_eq!(hooks[0].matcher, "");
        assert!(parse_hooks("# nada").is_err());
    }

    #[test]
    fn mcp_server_comes_from_the_printed_json() {
        let servers = parse_mcp(MCP).unwrap();
        assert_eq!(
            servers,
            [McpServer {
                name: "ai-memory".into(),
                command: "ai-memory".into(),
                args: vec![
                    "mcp-bridge".into(),
                    "--server-url".into(),
                    "http://127.0.0.1:49374/mcp".into()
                ],
                url: None,
                headers: Vec::new(),
            }]
        );
        assert!(parse_mcp("claude mcp add").is_err());
    }

    #[test]
    fn status_report_has_counts_storage_and_providers() {
        let json = r#"{"version": "3.1.0",
            "counts": {"pages_latest": 149, "sessions": 310, "observations": 13824},
            "storage": {"database_bytes": 24911872},
            "providers": {"llm": {"status": "disabled", "model": null},
                          "embedding": {"status": "ok", "model": "all-MiniLM-L6-v2"}}}"#;
        let report = parse_status(json).unwrap();
        let value = |label: &str| {
            report
                .stats
                .iter()
                .find(|s| s.label == label)
                .map(|s| s.value.clone())
        };
        assert_eq!(value("Páginas"), Some(StatValue::Count(149)));
        assert_eq!(value("Banco"), Some(StatValue::Bytes(24_911_872)));
        assert_eq!(
            value("Embeddings"),
            Some(StatValue::Text("ok · all-MiniLM-L6-v2".into()))
        );
        assert_eq!(value("LLM"), Some(StatValue::Text("desligado".into())));
        assert_eq!(value("Versão"), Some(StatValue::Text("3.1.0".into())));
        assert!(report.daily.is_empty());
    }

    #[test]
    fn missing_ai_memory_has_no_contribution_or_report() {
        let missing = AiMemory {
            bin: None,
            version: None,
            contribution: OnceCell::new(),
        };
        assert_eq!(missing.status(), Status::Missing);
        assert!(missing.contribution().is_err());
        assert!(missing.load_report().is_none());
    }
}
