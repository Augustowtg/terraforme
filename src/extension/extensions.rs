//! As extensões conhecidas.
//!
//! Para adicionar uma: crie um módulo aqui que implemente [`Extension`] e
//! inclua-o em [`all`]. Nada mais muda: a Config, a aba Extensões e o agente
//! já usam o que ela informa.

pub mod ai_memory;
pub mod rtk;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::Extension;

/// Quanto um comando da extensão pode demorar antes de o terra desistir.
const TIMEOUT: Duration = Duration::from_secs(10);

/// As extensões conhecidas, na ordem em que aparecem.
pub fn all() -> Vec<Box<dyn Extension>> {
    vec![
        Box::new(rtk::Rtk::detect()),
        Box::new(ai_memory::AiMemory::detect()),
    ]
}

/// Procura o executável `name` no `PATH`.
fn find_program(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_string()]
    };
    env::split_paths(&env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// Roda `bin args` e devolve a saída padrão. Falha quando ele não roda, sai
/// com erro ou passa de [`TIMEOUT`].
fn run(bin: &Path, args: &[&str]) -> Result<String, String> {
    let name = bin
        .file_stem()
        .map_or_else(|| bin.display().to_string(), |s| s.to_string_lossy().into());
    let mut cmd = Command::new(bin);
    cmd.args(args);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    let output = match rx.recv_timeout(TIMEOUT) {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => return Err(format!("não foi possível rodar o {name}: {err}")),
        Err(_) => return Err(format!("o {name} não respondeu")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty());
        return Err(last.map_or_else(
            || format!("o {name} saiu com {}", output.status),
            str::to_string,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A versão que `bin --version` informa (ex.: `rtk 0.45.0` → `0.45.0`).
fn version(bin: &Path) -> Option<String> {
    let text = run(bin, &["--version"]).ok()?;
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    line.split_whitespace().last().map(str::to_string)
}

/// O primeiro objeto JSON de um texto, ignorando o que vem antes da linha
/// em que ele começa (ex.: comentários com `#`).
fn first_json(text: &str) -> Option<Value> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('{') {
            let mut values = serde_json::Deserializer::from_str(&text[offset..]).into_iter();
            return values.next()?.ok();
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_json_skips_comments_and_trailing_text() {
        let text = "# comentário {não}\nclaude mcp add \\\n{\n  \"a\": {\"b\": 1}\n}\n# fim\n";
        assert_eq!(first_json(text), Some(serde_json::json!({"a": {"b": 1}})));
        assert_eq!(first_json("sem json"), None);
    }

    #[test]
    fn all_extensions_have_unique_ids() {
        let all = all();
        let mut ids: Vec<_> = all.iter().map(|e| e.id()).collect();
        assert_eq!(ids, ["rtk", "ai-memory"]);
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }
}
