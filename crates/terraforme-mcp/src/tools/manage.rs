//! Ferramentas MCP que gerenciam instâncias: `open_instance`,
//! `close_instance` e `delegate_task`.

use serde_json::{Value, json};

use crate::hub::{DELEGATE_TIMEOUT, Instance};
use crate::server::AppState;

pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "open_instance",
            "description": "Abre uma nova instância do seu agente numa sub-aba do seu terminal do terraforme (conversa normal, que fica aberta) e devolve o apelido dela. Com text, ela recebe o texto como uma mensagem sua. Para entregar uma tarefa e esperar o resultado, prefira delegate_task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Primeira mensagem da nova instância (opcional); ela não vê esta conversa"
                    }
                }
            }
        }),
        json!({
            "name": "close_instance",
            "description": "Fecha (encerra o processo de) outra instância de agente do terraforme. Não fecha a sua.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Apelido da instância a fechar, como em list_instances"
                    }
                },
                "required": ["to"]
            }
        }),
        json!({
            "name": "delegate_task",
            "description": "Delega uma tarefa a outra instância do terraforme: a uma já aberta (to) ou, sem to, a uma nova aberta no seu terminal. A chamada espera a instância terminar e devolve o resultado dela em result (até 30 min; depois disso o resultado chega como um prompt).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "Tarefa completa, com o contexto necessário: a outra instância não vê esta conversa"
                    },
                    "to": {
                        "type": "string",
                        "description": "Apelido de uma instância aberta (opcional; sem ele abre uma nova)"
                    }
                },
                "required": ["task"]
            }
        }),
    ]
}

/// Instância de quem chamou (cabeçalho `x-instance-id`).
fn caller(instance_id: &str, state: &AppState) -> Result<Instance, String> {
    state
        .hub
        .get(instance_id)
        .ok_or_else(|| "instância não registrada no terraforme".into())
}

fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

pub async fn open_instance(
    instance_id: &str,
    params: &Value,
    state: &AppState,
) -> Result<Value, String> {
    let from = caller(instance_id, state)?;
    let opened = state.hub.open(&from, text(params, "text")).await?;
    Ok(json!({ "opened": opened.handle }))
}

pub async fn close_instance(
    instance_id: &str,
    params: &Value,
    state: &AppState,
) -> Result<Value, String> {
    let to = text(params, "to").ok_or("parâmetro 'to' obrigatório")?;
    let from = caller(instance_id, state)?;
    let closed = state.hub.close(&from.handle, to).await?;
    Ok(json!({ "closed": closed }))
}

pub async fn delegate_task(
    instance_id: &str,
    params: &Value,
    state: &AppState,
) -> Result<Value, String> {
    let task = text(params, "task")
        .filter(|t| !t.trim().is_empty())
        .ok_or("parâmetro 'task' obrigatório")?;
    let from = caller(instance_id, state)?;
    let to = text(params, "to").filter(|t| !t.trim().is_empty());
    let opened = to.is_none();
    let (worker, rx) = match to {
        Some(to) => state.hub.delegate(&from, to, task).await?,
        None => state.hub.open_delegated(&from, task).await?,
    };
    match state.hub.wait(&worker, rx).await {
        Some(Ok(result)) => Ok(json!({
            "delegated_to": worker.handle,
            "opened": opened,
            "result": result,
        })),
        Some(Err(e)) => Err(e),
        None => Ok(json!({
            "delegated_to": worker.handle,
            "opened": opened,
            "result": null,
            "note": format!(
                "{} ainda não terminou em {} min; o resultado chega depois como um prompt",
                worker.handle,
                DELEGATE_TIMEOUT.as_secs() / 60
            ),
        })),
    }
}
