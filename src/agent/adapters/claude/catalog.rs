//! Comandos e modelos do Claude, pelo pedido `initialize` do protocolo do
//! Agent SDK: o `claude` responde em ~1 s com o que tem, sem chamar a API
//! (não gasta tokens).
//!
//! Entram todos os comandos que o `claude` informa (os embutidos, as skills
//! e os plugins) e os do modo interativo que o `-p` não informa
//! ([`interactive::COMMANDS`], ex.: `/help`, `/status`), para a lista da `/`
//! mostrar todos os comandos do Claude, menos os de pagamento e plano
//! ([`interactive::BILLING`]).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

use super::{command, interactive, text};
use crate::agent::{Catalog, CommandInfo, ModelInfo};

const REQUEST_ID: &str = "terra-catalog";
/// O id que o Claude usa para "o modelo padrão" (não é um modelo próprio).
const DEFAULT_ID: &str = "default";

/// Começa a carregar numa thread; o resultado chega pelo canal.
pub fn load(bin: PathBuf, config_dir: Option<PathBuf>) -> Receiver<Result<Catalog, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(fetch(bin, config_dir));
    });
    rx
}

fn fetch(bin: PathBuf, config_dir: Option<PathBuf>) -> Result<Catalog, String> {
    let mut cmd = command(&bin);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--strict-mcp-config",
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ]);
    if let Some(dir) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("não foi possível iniciar o claude: {err}"))?;

    let request = serde_json::json!({
        "type": "control_request",
        "request_id": REQUEST_ID,
        "request": { "subtype": "initialize" },
    });
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = writeln!(stdin, "{request}");
    }

    let stdout = child.stdout.take().expect("stdout configurado como pipe");
    let catalog = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .find(|value| {
            value["type"] == "control_response" && value["response"]["request_id"] == REQUEST_ID
        })
        .map(|value| parse(&value["response"]["response"]));
    let _ = child.kill();
    let _ = child.wait();
    catalog.ok_or_else(|| "o claude não informou os comandos".to_string())
}

/// Lê a resposta do `initialize`.
fn parse(response: &Value) -> Catalog {
    let list = |key: &str| response[key].as_array().cloned().unwrap_or_default();
    let mut commands: Vec<CommandInfo> = list("commands")
        .iter()
        .map(|c| CommandInfo {
            name: text(&c["name"]),
            description: text(&c["description"]),
            skill: c["builtin"] != true,
        })
        .filter(|c| !c.name.is_empty() && !c.name.starts_with("__"))
        .filter(|c| !interactive::BILLING.contains(&c.name.as_str()))
        .collect();
    // Os do modo interativo, quando o `claude` não informou (o dele vence).
    for (name, description) in interactive::COMMANDS {
        if !commands.iter().any(|c| c.name == *name) {
            commands.push(CommandInfo {
                name: name.to_string(),
                description: description.to_string(),
                skill: false,
            });
        }
    }

    let models: Vec<Value> = list("models");
    let default_model = models
        .iter()
        .find(|m| m["value"] == DEFAULT_ID)
        .and_then(|m| m["resolvedModel"].as_str())
        .map(str::to_string);
    let models = models
        .iter()
        .filter(|m| m["value"] != DEFAULT_ID)
        .map(|m| ModelInfo {
            id: text(&m["value"]),
            label: text(&m["displayName"]),
            description: text(&m["description"]),
            resolved: m["resolvedModel"].as_str().map(str::to_string),
            efforts: m["supportedEffortLevels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect(),
        })
        .collect();

    Catalog {
        commands,
        models,
        default_model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands_and_models() {
        let response: Value = serde_json::from_str(
            r#"{
            "commands": [
                {"name": "model", "description": "Set the AI model", "builtin": true},
                {"name": "graphify", "description": "skill de plugin"},
                {"name": "__remote-workflow", "description": "interno", "builtin": true}
            ],
            "models": [
                {"value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)"},
                {"value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus 5.5",
                 "description": "For complex work", "supportedEffortLevels": ["low", "high", "max"]},
                {"value": "claude-haiku-4-5", "displayName": "Haiku 4.5", "description": "Fastest"}
            ]
        }"#,
        )
        .unwrap();
        let catalog = parse(&response);
        let names: Vec<_> = catalog.commands.iter().map(|c| c.name.as_str()).collect();
        // Os do `claude` primeiro (embutidos e skills; os internos `__` não),
        // depois os do modo interativo.
        assert_eq!(names[..2], ["model", "graphify"]);
        assert!(!catalog.commands[0].skill);
        assert!(catalog.commands[1].skill);
        assert!(!names.contains(&"__remote-workflow"));
        for name in ["help", "status", "resume", "login", "logout", "permissions"] {
            assert!(names.contains(&name), "/{name}");
        }
        assert_eq!(names.len(), 2 + interactive::COMMANDS.len());
        assert_eq!(catalog.default_model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(
            catalog.models.len(),
            2,
            "o \"default\" não é um modelo próprio"
        );
        assert_eq!(catalog.models[0].efforts, ["low", "high", "max"]);
        assert_eq!(catalog.models[1].resolved, None);
        assert!(catalog.models[1].efforts.is_empty());
    }
}
