//! Ferramenta MCP `spawn_subagent`: delega uma tarefa a uma nova aba do
//! terminal do terraforme e devolve o resultado ao agente principal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing::info;

use crate::server::AppState;
use crate::tools::http_post_json;

pub struct SpawnResult {
    pub output: String,
    pub exit_code: i32,
}

pub type PendingSpawns = Arc<Mutex<HashMap<String, oneshot::Sender<SpawnResult>>>>;

/// Definição JSON da ferramenta para `tools/list`.
pub fn definition() -> Value {
    json!({
        "name": "spawn_subagent",
        "description": "Delega uma tarefa para uma nova instância do Claude Code aberta em aba separada no terminal do terraforme. Retorna o resultado quando o subagente concluir.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Descrição completa da tarefa a executar"
                },
                "cwd": {
                    "type": "string",
                    "description": "Diretório de trabalho (caminho absoluto); opcional"
                },
                "model": {
                    "type": "string",
                    "description": "Modelo Claude a usar (padrão: claude-haiku-4-5)"
                }
            },
            "required": ["task"]
        }
    })
}

/// Executa `spawn_subagent`: pede ao TUI do cliente que abra uma nova aba,
/// aguarda o resultado e devolve ao agente principal.
pub async fn handle(client_id: &str, params: &Value, state: &AppState) -> Result<Value, String> {
    let task = params
        .get("task")
        .and_then(Value::as_str)
        .ok_or("parâmetro 'task' obrigatório")?
        .to_string();
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let model = params
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-haiku-4-5")
        .to_string();

    // Verifica limite de subagentes simultâneos.
    {
        let pending = state.pending_spawns.lock().unwrap();
        if pending.len() >= state.config.max_simultaneous_subagents {
            return Err(format!(
                "limite de {} subagentes simultâneos atingido",
                state.config.max_simultaneous_subagents
            ));
        }
    }

    let control_addr = state
        .registry
        .control_addr(client_id)
        .ok_or_else(|| "cliente não encontrado no registro".to_string())?;

    let spawn_id = uuid::Uuid::new_v4().to_string();
    info!(spawn_id, client_id, "spawn_subagent iniciado");

    let (tx, rx) = oneshot::channel::<SpawnResult>();
    state
        .pending_spawns
        .lock()
        .unwrap()
        .insert(spawn_id.clone(), tx);

    let body = json!({
        "task": task,
        "cwd": cwd,
        "model": model,
        "spawn_id": spawn_id,
    });

    if let Err(e) = http_post_json(&control_addr, "/spawn", &body).await {
        state.pending_spawns.lock().unwrap().remove(&spawn_id);
        return Err(format!("falha ao contatar o terminal: {e}"));
    }

    let timeout = Duration::from_secs(state.config.subagent_timeout_seconds);
    let result = tokio::time::timeout(timeout, rx)
        .await
        .map_err(|_| {
            state.pending_spawns.lock().unwrap().remove(&spawn_id);
            format!("timeout após {}s", state.config.subagent_timeout_seconds)
        })?
        .map_err(|_| "canal do subagente fechado inesperadamente".to_string())?;

    info!(
        spawn_id,
        exit_code = result.exit_code,
        "spawn_subagent concluído"
    );
    Ok(json!({
        "output": result.output,
        "exit_code": result.exit_code
    }))
}
