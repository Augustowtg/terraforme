//! Instância de documentação do terminal (`terraforme cli`).
//!
//! Quando o agente precisa gerar uma documentação (README, referência de
//! API…), ele grava um pedido em
//! `~/.config/terraforme/doc-request.json` com as instruções da tarefa. O
//! terminal descobre o arquivo, abre uma nova sub-aba do agente já com esse
//! prompt, e apaga o arquivo. A instância fica aberta até o usuário fechá-la
//! (`Shift+F12`) ou até o terraforme ser encerrado.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Arquivo do pedido pendente de instância de documentação.
pub fn request_path() -> Result<PathBuf> {
    Ok(Config::dir()?.join("doc-request.json"))
}

/// Pedido gravado pelo agente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocRequest {
    /// Instruções passadas ao agente da documentação.
    pub instructions: String,
}

/// Resultado de [`take_request`]: o que consumiu (ou por que não consumiu).
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Instruções do pedido consumido.
    pub instructions: Option<String>,
    /// Erro ao consumir (arquivo inválido); o arquivo já foi descartado.
    pub error: Option<String>,
}

impl Outcome {
    /// `true` quando há instruções para abrir a instância.
    pub fn is_some(&self) -> bool {
        self.instructions.is_some()
    }
}

/// Consome o pedido pendente: lê o arquivo e o apaga, para que um pedido não
/// abra duas instâncias. Sem arquivo, devolve [`Outcome::default`].
pub fn take_request() -> Result<Outcome> {
    take_request_at(&request_path()?)
}

/// [`take_request`] de um caminho qualquer (o terminal passa o resolvido).
pub fn take_request_at(path: &Path) -> Result<Outcome> {
    use std::io::ErrorKind;

    let mut outcome = Outcome::default();
    match fs::read_to_string(path) {
        Ok(text) => {
            let _ = fs::remove_file(path);
            match serde_json::from_str::<DocRequest>(&text) {
                Ok(request) => outcome.instructions = Some(request.instructions),
                Err(err) => outcome.error = Some(format!("pedido de documentação inválido: {err}")),
            }
        }
        // Sem pedido pendente — o comum.
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => outcome.error = Some(format!("lendo {}: {err}", path.display())),
    }
    Ok(outcome)
}

/// Grava o pedido que abre a instância de documentação (sobrescreve pendente).
pub fn write_request(instructions: &str) -> Result<()> {
    write_request_at(&request_path()?, instructions)
}

/// [`write_request`] de um caminho qualquer (o agente grava no arquivo dele).
pub fn write_request_at(path: &Path, instructions: &str) -> Result<()> {
    crate::agent::write_json(path, &serde_json::json!({ "instructions": instructions }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("terraforme-doc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_request_is_none() {
        let base = tmp("nenhum");
        // O arquivo vai em `<base>/terraforme/doc-request.json`.
        let path = base.join("terraforme").join("doc-request.json");
        assert_eq!(take_request_at(&path).unwrap(), Outcome::default());
    }

    #[test]
    fn write_then_take_round_trip() {
        let base = tmp("round-trip");
        let path = base.join("doc-request.json");
        write_request_at(&path, "# Doc do módulo X\n- item").unwrap();
        let outcome = take_request_at(&path).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                instructions: Some("# Doc do módulo X\n- item".into()),
                error: None,
            }
        );
        // Consumido: não abre de novo.
        assert_eq!(take_request_at(&path).unwrap(), Outcome::default());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn invalid_request_is_consumed_with_error() {
        let base = tmp("invalido");
        let path = base.join("doc-request.json");
        fs::write(&path, "não é json").unwrap();
        let outcome = take_request_at(&path).unwrap();
        assert!(outcome.error.is_some());
        assert!(!path.exists(), "pedido inválido não fica pendente");

        fs::remove_dir_all(&base).unwrap();
    }
}
