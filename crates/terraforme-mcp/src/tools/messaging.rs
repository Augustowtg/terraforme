//! Ferramentas MCP de conversa entre instâncias: `list_instances` e
//! `send_message`.

use serde_json::{Value, json};

use crate::server::AppState;

pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_instances",
            "description": "Lista as instâncias de agente abertas no terraforme em todos os spaces (apelido, agente, space e pasta). A sua vem marcada com you=true. Use o apelido em send_message.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "send_message",
            "description": "Manda uma mensagem para outra instância de agente do terraforme; ela chega como um prompt na instância de destino, com o seu apelido para a resposta. Use to=\"web\" para escrever só na UI web.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Apelido da instância de destino (ex.: meu-space/claude-2), como em list_instances"
                    },
                    "text": {
                        "type": "string",
                        "description": "Mensagem completa: a outra instância não vê esta conversa"
                    }
                },
                "required": ["to", "text"]
            }
        }),
    ]
}

/// Apelido de quem chamou (cabeçalho `x-instance-id`).
fn caller(instance_id: &str, state: &AppState) -> Result<String, String> {
    state
        .hub
        .get(instance_id)
        .map(|i| i.handle)
        .ok_or_else(|| "instância não registrada no terraforme".into())
}

pub fn list_instances(instance_id: &str, state: &AppState) -> Value {
    let list: Vec<_> = state
        .hub
        .instances()
        .into_iter()
        .map(|i| {
            json!({
                "handle": i.handle,
                "agent": i.agent,
                "space": i.space,
                "cwd": i.cwd,
                "you": i.id == instance_id,
            })
        })
        .collect();
    json!({ "instances": list })
}

pub async fn send_message(
    instance_id: &str,
    params: &Value,
    state: &AppState,
) -> Result<Value, String> {
    let to = params
        .get("to")
        .and_then(Value::as_str)
        .ok_or("parâmetro 'to' obrigatório")?;
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .ok_or("parâmetro 'text' obrigatório")?;
    let from = caller(instance_id, state)?;
    let to = state.hub.send(&from, to, text).await?;
    Ok(json!({ "delivered": true, "to": to }))
}
