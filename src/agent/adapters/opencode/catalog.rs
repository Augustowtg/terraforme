//! O catálogo do OpenCode: os modelos do `opencode models`.
//!
//! O `opencode models` imprime um modelo por linha, no formato
//! `provider/modelo` (ex.: `opencode/deepseek-v4.1-flash-free`). O comando
//! não chama o modelo, então carregar o catálogo não gasta tokens.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::agent::{Catalog, ModelInfo};

use super::command;

/// Começa a carregar o catálogo numa thread; o resultado chega pelo canal.
pub fn load(bin: std::path::PathBuf) -> Receiver<Result<Catalog, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cmd = command(&bin);
        let output = cmd.arg("models").output();
        let result = match output {
            Ok(output) if output.status.success() => {
                Ok(parse(&String::from_utf8_lossy(&output.stdout)))
            }
            Ok(output) => Err(format!(
                "o opencode models falhou: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(err) => Err(format!("não foi possível rodar o opencode: {err}")),
        };
        let _ = tx.send(result);
    });
    rx
}

/// Os modelos de cada linha não vazia, na ordem em que vieram.
fn parse(text: &str) -> Catalog {
    let models: Vec<ModelInfo> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|id| ModelInfo {
            id: id.to_string(),
            label: id.to_string(),
            description: String::new(),
            resolved: None,
            efforts: Vec::new(),
        })
        .collect();
    Catalog {
        commands: Vec::new(),
        models,
        default_model: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_is_a_model() {
        let catalog = parse("opencode/modelo-a\n\n anthropic/claude-sonnet-4 \n");
        let ids: Vec<&str> = catalog.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["opencode/modelo-a", "anthropic/claude-sonnet-4"]);
        assert_eq!(catalog.models[0].label, "opencode/modelo-a");
        assert!(catalog.commands.is_empty());
    }
}
