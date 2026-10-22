//! Middleware (experimental): o agente roda sem a TUI dele, em modo de
//! texto estruturado, e o próprio terraforme desenha a conversa como um chat
//! (`docs/tui/agente/agente.feature.md`). Cada [`crate::agent::Adapter`] que
//! suporta o modo devolve os argumentos em `Adapter::chat_args` e traduz a
//! entrada e a saída do agente com [`Adapter::chat_input`] e
//! [`Adapter::chat_events`] para o formato comum daqui ([`ChatEvent`]).
//!
//! O Claude Code usa o `stream-json` (`claude -p --input-format stream-json
//! --output-format stream-json --verbose --include-partial-messages`): uma
//! mensagem JSON por linha nos dois sentidos, com o processo vivo entre as
//! mensagens, então a sessão continua como no terminal. Pelo mesmo canal vão
//! os pedidos de controle do SDK: `initialize` (devolve os comandos `/`) e,
//! com `--permission-prompt-tool stdio`, o `can_use_tool` de cada ferramenta
//! que precisa de permissão, respondido pelo chat ([`claude_permission`]).
//!
//! O Antigravity (`agy`) também fala `stream-json` nos dois sentidos
//! ([`AGY_ARGS`]), sem `-p`: o `user` de [`agy_input`] abre cada turno, os
//! pedaços da resposta vêm em `step_update` e o `result` fecha o turno. Não
//! há permissão interativa — as ferramentas rodam liberadas.
//!
//! O OpenCode (`opencode acp`) é um servidor do Agent Client Protocol
//! ([`OPENCODE_ARGS`]): um JSON-RPC por linha no stdio, com o handshake
//! `initialize` → `session/new` ([`acp_new_session`]) e cada mensagem num
//! `session/prompt` ([`acp_input`]) que devolve a `sessionId` usada daí em
//! diante ([`ChatEvent::Session`]). Os pedidos de permissão do agente chegam
//! como requisição no meio da conversa e são respondidos com
//! [`acp_permission`].
//!
//! Os `Adapter::chat_*` recebem o [`ChatCtx`] da instância: a pasta de
//! trabalho, que o handshake do OpenCode precisa, e a sessão aberta, que os
//! pedidos seguintes usam.
//!
//! [`Adapter::chat_input`]: crate::agent::Adapter::chat_input
//! [`Adapter::chat_events`]: crate::agent::Adapter::chat_events

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Um comando `/` do agente, para completar na caixa de mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// Dica dos argumentos (ex.: `[arquivo]`); vazia sem argumentos.
    pub hint: String,
}

/// O que a instância tem de contexto no modo Middleware, repassado aos
/// `Adapter::chat_*`: a pasta de trabalho (o handshake do OpenCode pede) e a
/// sessão aberta pelo agente, quando o protocolo tem uma
/// ([`ChatEvent::Session`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatCtx {
    pub cwd: PathBuf,
    pub session: Option<String>,
}

/// Pedido de permissão de uma ferramenta, respondido com
/// [`crate::agent::Adapter::chat_permission`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission {
    /// Id do pedido, devolvido como está na resposta (texto no stream-json,
    /// número no JSON-RPC).
    pub id: Value,
    pub tool: String,
    /// Resumo da chamada (o comando, o arquivo…).
    pub detail: String,
    /// Entrada da ferramenta, devolvida como está quando liberada.
    pub input: Value,
    /// Regras sugeridas pelo agente para "sempre permitir"; `Null` sem elas.
    pub always: Value,
}

/// Resposta do usuário a uma [`Permission`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Once,
    /// Libera e grava as regras sugeridas ([`Permission::always`]).
    Always,
    Deny,
}

/// O que o chat mostra de uma linha da saída do agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    /// Sessão aberta, com o modelo em uso (só a primeira vale) e os nomes
    /// dos comandos `/`.
    Init {
        model: String,
        commands: Vec<String>,
    },
    /// Comandos `/` com a descrição (resposta do `initialize`).
    Commands(Vec<SlashCommand>),
    /// Sessão aberta pelo agente (só o Agent Client Protocol tem uma): o id
    /// que os pedidos seguintes usam ([`ChatCtx`]).
    Session(String),
    /// Uma ferramenta pede permissão; o chat pergunta ao usuário.
    Permission(Permission),
    /// Linha a devolver ao agente sem passar pelo usuário (ex.: recusa de
    /// um pedido de controle que o chat não conhece).
    Reply(String),
    /// Começa um bloco novo de texto do agente.
    TextStart,
    /// Pedaço do texto do agente, a juntar ao bloco aberto.
    Text(String),
    /// O agente chamou uma ferramenta (ex.: `Bash` com o comando). `id` liga
    /// os avisos da mesma ferramenta quando o agente avisa mais de uma vez
    /// (Agent Client Protocol); vazia, cada aviso vira um item novo. Um
    /// aviso com `name` vazio só atualiza o resumo do começo.
    Tool {
        id: String,
        name: String,
        detail: String,
    },
    /// Resultado de uma ferramenta: primeira linha, e se deu erro.
    ToolResult { error: bool, text: String },
    /// Fim da resposta: erro, se houve, e as ferramentas negadas.
    Done {
        error: Option<String>,
        denied: Vec<String>,
    },
}

/// Argumentos do `claude` para o modo Middleware. Com
/// `--permission-prompt-tool stdio` as permissões chegam como pedidos de
/// controle no stdout ([`ChatEvent::Permission`]) em vez de serem negadas.
pub const CLAUDE_ARGS: [&str; 9] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--permission-prompt-tool",
    "stdio",
];

/// Pedido `initialize` do SDK, mandado ao abrir: a resposta traz os
/// comandos `/` com a descrição ([`ChatEvent::Commands`]) antes da primeira
/// mensagem (o `init` da sessão só vem depois dela).
pub fn claude_initialize() -> String {
    json!({
        "type": "control_request",
        "request_id": "terraforme-init",
        "request": { "subtype": "initialize" },
    })
    .to_string()
}

/// Resposta a um [`ChatEvent::Permission`] (uma linha).
pub fn claude_permission(permission: &Permission, answer: Answer) -> String {
    let response = match answer {
        Answer::Deny => json!({
            "behavior": "deny",
            "message": "O usuário negou o uso desta ferramenta.",
        }),
        Answer::Once => json!({ "behavior": "allow", "updatedInput": permission.input }),
        Answer::Always => json!({
            "behavior": "allow",
            "updatedInput": permission.input,
            "updatedPermissions": permission.always,
        }),
    };
    control_response(&permission.id, response)
}

fn control_response(id: &Value, response: Value) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": id, "response": response },
    })
    .to_string()
}

/// Mensagem do usuário no `stream-json` do Claude Code (uma linha).
pub fn claude_input(text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
    })
    .to_string()
}

/// Traduz uma linha do `stream-json` do Claude Code. As mensagens dos
/// subagentes (`parent_tool_use_id` preenchido) ficam de fora: o chat mostra
/// só a conversa principal.
pub fn claude_events(line: &str) -> Vec<ChatEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    if !value["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    match value["type"].as_str() {
        Some("system") if value["subtype"] == "init" => vec![ChatEvent::Init {
            model: value["model"].as_str().unwrap_or_default().to_string(),
            commands: value["slash_commands"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect(),
        }],
        Some("control_request") => control_request(&value).into_iter().collect(),
        Some("control_response") => value["response"]["response"]["commands"]
            .as_array()
            .map(|commands| {
                ChatEvent::Commands(
                    commands
                        .iter()
                        .filter_map(|c| {
                            Some(SlashCommand {
                                name: c["name"].as_str()?.to_string(),
                                description: first_line(c["description"].as_str().unwrap_or("")),
                                hint: c["argumentHint"].as_str().unwrap_or("").to_string(),
                            })
                        })
                        .collect(),
                )
            })
            .into_iter()
            .collect(),
        Some("stream_event") => stream_event(&value["event"]).into_iter().collect(),
        Some("assistant") => content(&value["message"])
            .filter(|block| block["type"] == "tool_use")
            .map(|block| ChatEvent::Tool {
                id: String::new(),
                name: block["name"].as_str().unwrap_or("?").to_string(),
                detail: tool_detail(&block["input"]),
            })
            .collect(),
        Some("user") => content(&value["message"])
            .filter(|block| block["type"] == "tool_result")
            .map(|block| ChatEvent::ToolResult {
                error: block["is_error"].as_bool().unwrap_or(false),
                text: first_line(&tool_result_text(&block["content"])),
            })
            .collect(),
        Some("result") => {
            let denied = value["permission_denials"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["tool_name"].as_str().map(str::to_string))
                .collect();
            let error = value["is_error"].as_bool().unwrap_or(false).then(|| {
                value["result"]
                    .as_str()
                    .or(value["subtype"].as_str())
                    .unwrap_or("erro")
                    .to_string()
            });
            vec![ChatEvent::Done { error, denied }]
        }
        _ => Vec::new(),
    }
}

/// Pedido de controle do agente: a permissão de uma ferramenta vai ao
/// usuário; os outros (ganchos e MCP do SDK, que o chat não registra) são
/// recusados na hora para o agente não ficar esperando.
fn control_request(value: &Value) -> Option<ChatEvent> {
    let id = value["request_id"].clone();
    if !id.is_string() {
        return None;
    }
    let request = &value["request"];
    if request["subtype"] != "can_use_tool" {
        let line = json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": id, "error": "não suportado" },
        });
        return Some(ChatEvent::Reply(line.to_string()));
    }
    let suggestions = request["permission_suggestions"].as_array();
    // "Sempre" grava só as regras da ferramenta, como no terminal; sem
    // elas, o que o agente sugeriu (pastas, modo).
    let rules: Vec<&Value> = suggestions
        .into_iter()
        .flatten()
        .filter(|s| s["type"] == "addRules")
        .collect();
    let always = match (rules.is_empty(), suggestions) {
        (false, _) => json!(rules),
        (true, Some(all)) if !all.is_empty() => json!(all),
        _ => Value::Null,
    };
    Some(ChatEvent::Permission(Permission {
        id,
        tool: request["tool_name"].as_str().unwrap_or("?").to_string(),
        detail: tool_detail(&request["input"]),
        input: request["input"].clone(),
        always,
    }))
}

/// Eventos parciais da resposta (`--include-partial-messages`): só o texto,
/// que chega aos pedaços.
fn stream_event(event: &Value) -> Option<ChatEvent> {
    match event["type"].as_str()? {
        "content_block_start" if event["content_block"]["type"] == "text" => {
            Some(ChatEvent::TextStart)
        }
        "content_block_delta" if event["delta"]["type"] == "text_delta" => Some(ChatEvent::Text(
            event["delta"]["text"].as_str()?.to_string(),
        )),
        _ => None,
    }
}

fn content(message: &Value) -> impl Iterator<Item = &Value> {
    message["content"].as_array().into_iter().flatten()
}

/// Argumentos do `agy` para o modo Middleware: o stream-json do Antigravity,
/// sem `-p` — o processo abre no stdin e fica vivo entre as mensagens.
pub const AGY_ARGS: [&str; 4] = [
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
];

/// Mensagem do usuário no `stream-json` do Antigravity (uma linha).
pub fn agy_input(text: &str) -> String {
    json!({
        "event": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    })
    .to_string()
}

/// Traduz uma linha do `stream-json` do Antigravity: o `init` abre a
/// conversa, cada `step_update` anda um passo (pedaços de texto ou uma
/// ferramenta, do começo ao fim) e o `result` fecha o turno.
pub fn agy_events(line: &str) -> Vec<ChatEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match value["event"].as_str() {
        Some("step_update") => agy_step(&value["step_update"]).into_iter().collect(),
        Some("result") => {
            let result = &value["result"];
            let error = (result["status"] != "SUCCESS").then(|| {
                first_line(
                    result["error"]
                        .as_str()
                        .or(result["response"].as_str())
                        .unwrap_or("erro"),
                )
            });
            vec![ChatEvent::Done {
                error,
                denied: Vec::new(),
            }]
        }
        _ => Vec::new(),
    }
}

/// Um passo da resposta do Antigravity: texto em qualquer estado, e a
/// ferramenta no começo (`ACTIVE`) e no fim (`DONE`, com a saída).
fn agy_step(step: &Value) -> Option<ChatEvent> {
    let tool = &step["tool_info"];
    if let Some(text) = step["text_delta"].as_str() {
        return Some(ChatEvent::Text(text.to_string()));
    }
    let name = tool["name"].as_str()?;
    if step["state"] == "DONE" {
        Some(ChatEvent::ToolResult {
            error: false,
            text: first_line(tool["output"].as_str().unwrap_or_default()),
        })
    } else {
        Some(ChatEvent::Tool {
            id: String::new(),
            name: name.to_string(),
            detail: tool_detail(&tool["parameters"]),
        })
    }
}

/// Argumentos do `opencode` para o modo Middleware: o servidor do Agent
/// Client Protocol, um JSON-RPC por linha no stdio.
pub const OPENCODE_ARGS: [&str; 1] = ["acp"];

/// Id dos pedidos que o terraforme faz no Agent Client Protocol; a resposta
/// de cada um tem um papel no handshake ([`acp_events`]).
const ACP_INIT: u64 = 1;
const ACP_SESSION: u64 = 2;
const ACP_PROMPT: u64 = 3;

/// Pedido `initialize` do Agent Client Protocol, mandado ao abrir
/// ([`crate::agent::Adapter::chat_start`]).
pub fn acp_initialize() -> String {
    json!({
        "jsonrpc": "2.0",
        "id": ACP_INIT,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false } },
        },
    })
    .to_string()
}

/// Pedido `session/new` do Agent Client Protocol, mandado quando o agente
/// responde ao [`acp_initialize`]: a resposta traz a `sessionId` e o modelo
/// em uso ([`ChatEvent::Session`] e [`ChatEvent::Init`]).
pub fn acp_new_session(cwd: &Path) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": ACP_SESSION,
        "method": "session/new",
        "params": { "cwd": cwd.display().to_string(), "mcpServers": [] },
    })
    .to_string()
}

/// Mensagem do usuário num `session/prompt` do Agent Client Protocol (uma
/// linha); o id é o mesmo daqui para a sessão toda.
pub fn acp_input(text: &str, session: Option<&str>) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": ACP_PROMPT,
        "method": "session/prompt",
        "params": {
            "sessionId": session.unwrap_or_default(),
            "prompt": [{ "type": "text", "text": text }],
        },
    })
    .to_string()
}

/// Resposta a um pedido de permissão do Agent Client Protocol (uma linha):
/// a opção escolhida, ou a recusa, que o agente vira `reject`.
pub fn acp_permission(permission: &Permission, answer: Answer) -> String {
    let outcome = match answer {
        Answer::Deny => json!({ "outcome": { "outcome": "cancelled" } }),
        Answer::Once => json!({ "outcome": { "outcome": "selected", "optionId": "once" } }),
        Answer::Always => json!({ "outcome": { "outcome": "selected", "optionId": "always" } }),
    };
    json!({
        "jsonrpc": "2.0",
        "id": permission.id.clone(),
        "result": outcome,
    })
    .to_string()
}

/// Traduz uma linha do JSON-RPC do Agent Client Protocol: as respostas dos
/// pedidos do terraforme (o handshake e o fim da mensagem), as notificações
/// `session/update` e os pedidos do agente no meio da conversa.
pub fn acp_events(line: &str, ctx: &ChatCtx) -> Vec<ChatEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match value["method"].as_str() {
        Some("session/update") => acp_update(&value["params"]["update"]),
        Some("session/request_permission") => acp_ask(&value),
        // Notificação: sem id para responder.
        Some(_) if value["id"].is_null() => Vec::new(),
        // Pedido que o chat não conhece: recusa na hora, para o agente não
        // ficar esperando.
        Some(_) => vec![ChatEvent::Reply(
            json!({
                "jsonrpc": "2.0",
                "id": value["id"].clone(),
                "error": { "code": -32601, "message": "método não suportado" },
            })
            .to_string(),
        )],
        None => acp_response(&value, ctx),
    }
}

/// Resposta a um pedido do terraforme: a do `initialize` pede a sessão, a da
/// sessão guarda o id e mostra o modelo, e a da mensagem fecha o turno (os
/// erros chegam aqui também, como resposta com `error`).
fn acp_response(value: &Value, ctx: &ChatCtx) -> Vec<ChatEvent> {
    if let Some(error) = value["error"]["message"].as_str() {
        return vec![ChatEvent::Done {
            error: Some(first_line(error)),
            denied: Vec::new(),
        }];
    }
    match value["id"].as_u64() {
        Some(ACP_INIT) => vec![ChatEvent::Reply(acp_new_session(&ctx.cwd))],
        Some(ACP_SESSION) => {
            let result = &value["result"];
            let model = result["configOptions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|option| option["id"] == "model")
                .and_then(|option| option["currentValue"].as_str())
                .unwrap_or_default()
                .to_string();
            vec![
                ChatEvent::Session(result["sessionId"].as_str().unwrap_or_default().to_string()),
                ChatEvent::Init {
                    model,
                    commands: Vec::new(),
                },
            ]
        }
        // O fim da mensagem traz o `stopReason` do turno.
        _ => {
            let error = match value["result"]["stopReason"].as_str().unwrap_or("end_turn") {
                "end_turn" | "cancelled" => None,
                "max_tokens" => Some("resposta cortada no limite de tokens".to_string()),
                "refusal" => Some("o modelo recusou a resposta".to_string()),
                other => Some(other.to_string()),
            };
            vec![ChatEvent::Done {
                error,
                denied: Vec::new(),
            }]
        }
    }
}

/// Notificação `session/update`: o texto da resposta aos pedaços, os comandos
/// `/` do catálogo e as ferramentas chamadas.
fn acp_update(update: &Value) -> Vec<ChatEvent> {
    match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => update["content"]["text"]
            .as_str()
            .map(|text| vec![ChatEvent::Text(text.to_string())])
            .unwrap_or_default(),
        Some("available_commands_update") => vec![ChatEvent::Commands(
            update["availableCommands"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|command| {
                    Some(SlashCommand {
                        name: command["name"].as_str()?.to_string(),
                        description: first_line(command["description"].as_str().unwrap_or("")),
                        hint: String::new(),
                    })
                })
                .collect(),
        )],
        Some("tool_call") => vec![ChatEvent::Tool {
            id: acp_id(update),
            name: update["name"].as_str().unwrap_or("?").to_string(),
            detail: acp_title(update),
        }],
        Some("tool_call_update") => acp_tool_update(update),
        _ => Vec::new(),
    }
}

/// Uma ferramenta do Agent Client Protocol: o aviso de começo traz o nome e
/// o do meio, o título já com a entrada; o do fim traz a saída (`completed`)
/// ou o erro (`failed`). Os avisos do meio repetem a chamada sem o nome, que
/// o chat mantém ([`ChatEvent::Tool`]).
fn acp_tool_update(update: &Value) -> Vec<ChatEvent> {
    match update["status"].as_str() {
        Some("in_progress") => vec![ChatEvent::Tool {
            id: acp_id(update),
            name: String::new(),
            detail: acp_title(update),
        }],
        Some("completed" | "failed") => {
            let text = update["rawOutput"]["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| acp_content(&update["content"]));
            vec![ChatEvent::ToolResult {
                error: update["status"] == "failed",
                text: first_line(&text),
            }]
        }
        _ => Vec::new(),
    }
}

/// Pedido de permissão do agente: a ferramenta, o resumo da chamada e as
/// opções que ele oferece (o "sempre" é uma delas).
fn acp_ask(value: &Value) -> Vec<ChatEvent> {
    let call = &value["params"]["toolCall"];
    let always = value["params"]["options"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|option| option["kind"] == "allow_always");
    vec![ChatEvent::Permission(Permission {
        id: value["id"].clone(),
        tool: call["name"].as_str().unwrap_or("?").to_string(),
        detail: acp_title(call),
        input: call["rawInput"].clone(),
        // O "sempre" do Agent Client Protocol é uma opção que o agente já
        // conhece, sem regra para guardar.
        always: if always { json!(true) } else { Value::Null },
    })]
}

/// Id da chamada, que liga os avisos da mesma ferramenta.
fn acp_id(update: &Value) -> String {
    update["toolCallId"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Título da chamada: o comando, o arquivo…; sem título, a entrada.
fn acp_title(update: &Value) -> String {
    match update["title"].as_str() {
        Some(title) if !title.is_empty() => first_line(title),
        _ => tool_detail(&update["rawInput"]),
    }
}

/// Texto dos blocos de conteúdo de uma chamada (só os de texto).
fn acp_content(content: &Value) -> String {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["content"]["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resumo da chamada: o campo mais revelador de cada ferramenta comum, a
/// primeira entrada de texto (o Antigravity usa chaves em PascalCase,
/// `CommandLine`…), ou o JSON da entrada.
fn tool_detail(input: &Value) -> String {
    let field = [
        "command",
        "file_path",
        "pattern",
        "path",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|key| input[key].as_str());
    let text = field.or_else(|| {
        input
            .as_object()
            .and_then(|fields| fields.values().find_map(Value::as_str))
    });
    match text {
        Some(text) => first_line(text),
        None if input.as_object().is_some_and(|o| o.is_empty()) => String::new(),
        None => first_line(&input.to_string()),
    }
}

/// O conteúdo do resultado vem como texto ou como lista de blocos.
fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Primeira linha não vazia, cortada em 160 caracteres.
fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(160).collect();
    if line.chars().count() > 160 || text.trim().lines().count() > 1 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_input_is_one_json_line() {
        let line = claude_input("oi\n\"mundo\"");
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["content"], "oi\n\"mundo\"");
    }

    #[test]
    fn reads_text_tools_and_result() {
        let init = r#"{"type":"system","subtype":"init","model":"claude-haiku","parent_tool_use_id":null}"#;
        assert_eq!(
            claude_events(init),
            vec![ChatEvent::Init {
                model: "claude-haiku".into(),
                commands: Vec::new(),
            }]
        );
        let start = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}"#;
        assert_eq!(claude_events(start), vec![ChatEvent::TextStart]);
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Oi"}},"parent_tool_use_id":null}"#;
        assert_eq!(claude_events(delta), vec![ChatEvent::Text("Oi".into())]);
        // O texto já veio aos pedaços: a mensagem completa só traz as ferramentas.
        let assistant = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Oi"},{"type":"tool_use","name":"Bash","input":{"command":"ls -la","description":"lista"}}]},"parent_tool_use_id":null}"#;
        assert_eq!(
            claude_events(assistant),
            vec![ChatEvent::Tool {
                id: String::new(),
                name: "Bash".into(),
                detail: "ls -la".into()
            }]
        );
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"a\nb","is_error":false}]},"parent_tool_use_id":null}"#;
        assert_eq!(
            claude_events(result),
            vec![ChatEvent::ToolResult {
                error: false,
                text: "a…".into()
            }]
        );
        let done = r#"{"type":"result","subtype":"success","is_error":false,"result":"Oi","permission_denials":[{"tool_name":"Bash"}]}"#;
        assert_eq!(
            claude_events(done),
            vec![ChatEvent::Done {
                error: None,
                denied: vec!["Bash".into()]
            }]
        );
    }

    #[test]
    fn reads_commands_from_initialize() {
        assert!(claude_initialize().contains("\"initialize\""));
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"terraforme-init","response":{"commands":[{"name":"compact","description":"Resume a conversa","argumentHint":"[foco]"}]}}}"#;
        assert_eq!(
            claude_events(line),
            vec![ChatEvent::Commands(vec![SlashCommand {
                name: "compact".into(),
                description: "Resume a conversa".into(),
                hint: "[foco]".into(),
            }])]
        );
    }

    #[test]
    fn asks_and_answers_permissions() {
        let line = r#"{"type":"control_request","request_id":"p1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch a"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch a"}],"behavior":"allow","destination":"localSettings"},{"type":"setMode","mode":"acceptEdits","destination":"session"}]}}"#;
        let Some(ChatEvent::Permission(permission)) = claude_events(line).pop() else {
            panic!("esperava um pedido de permissão");
        };
        assert_eq!(
            (permission.tool.as_str(), permission.detail.as_str()),
            ("Bash", "touch a")
        );
        assert_eq!(permission.always.as_array().unwrap().len(), 1);
        let reply: Value =
            serde_json::from_str(&claude_permission(&permission, Answer::Once)).unwrap();
        assert_eq!(reply["response"]["request_id"], "p1");
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(
            reply["response"]["response"]["updatedInput"]["command"],
            "touch a"
        );
        let reply: Value =
            serde_json::from_str(&claude_permission(&permission, Answer::Always)).unwrap();
        assert_eq!(
            reply["response"]["response"]["updatedPermissions"][0]["type"],
            "addRules"
        );
        let reply: Value =
            serde_json::from_str(&claude_permission(&permission, Answer::Deny)).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
        // Outros pedidos de controle são recusados na hora.
        let hook =
            r#"{"type":"control_request","request_id":"h1","request":{"subtype":"hook_callback"}}"#;
        let Some(ChatEvent::Reply(reply)) = claude_events(hook).pop() else {
            panic!("esperava uma recusa");
        };
        assert!(reply.contains("\"error\"") && reply.contains("h1"));
    }

    #[test]
    fn skips_subagents_and_noise() {
        let sub = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}},"parent_tool_use_id":"toolu_1"}"#;
        assert!(claude_events(sub).is_empty());
        assert!(claude_events("não é json").is_empty());
        assert!(claude_events(r#"{"type":"rate_limit_event"}"#).is_empty());
    }

    #[test]
    fn antigravity_user_message_is_one_json_line() {
        let line = agy_input("oi\n\"mundo\"");
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["event"], "user");
        assert_eq!(value["message"]["content"][0]["text"], "oi\n\"mundo\"");
    }

    #[test]
    fn reads_antigravity_steps_and_result() {
        let text = r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"agent_response","text_delta":"Oi "}}"#;
        assert_eq!(agy_events(text), vec![ChatEvent::Text("Oi ".into())]);
        let tool = r#"{"event":"step_update","step_update":{"step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls -la"}}}}"#;
        assert_eq!(
            agy_events(tool),
            vec![ChatEvent::Tool {
                id: String::new(),
                name: "run_command".into(),
                detail: "ls -la".into()
            }]
        );
        // O mesmo passo no fim traz a saída (a primeira linha, como no resto).
        let done = r#"{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"tool","tool_info":{"name":"run_command","output":"Command exited with code 0.\n\nCommand output:\nterraforme\n"}}}"#;
        assert_eq!(
            agy_events(done),
            vec![ChatEvent::ToolResult {
                error: false,
                text: "Command exited with code 0.…".into()
            }]
        );
        let ok = r#"{"event":"result","result":{"status":"SUCCESS","response":"Oi\n"}}"#;
        assert_eq!(
            agy_events(ok),
            vec![ChatEvent::Done {
                error: None,
                denied: Vec::new()
            }]
        );
        let failed =
            r#"{"event":"result","result":{"status":"ERROR","error":"provedor fora do ar"}}"#;
        assert_eq!(
            agy_events(failed),
            vec![ChatEvent::Done {
                error: Some("provedor fora do ar".into()),
                denied: Vec::new()
            }]
        );
        // Passo sem texto nem ferramenta (o `user_input`, o `unknown`).
        assert!(agy_events(
            r#"{"event":"step_update","step_update":{"step_index":0,"state":"DONE","step_type":"user_input"}}"#
        )
        .is_empty());
        assert!(agy_events("lixo").is_empty());
    }

    #[test]
    fn opencode_handshake_opens_the_session_and_sends_the_prompt() {
        // O `initialize` manda o `session/new` na pasta da instância.
        let ctx = ChatCtx {
            cwd: "/projetos/x".into(),
            ..ChatCtx::default()
        };
        let start = serde_json::from_str::<Value>(&acp_initialize()).unwrap();
        assert_eq!(start["method"], "initialize");
        assert_eq!(start["params"]["protocolVersion"], 1);
        // A resposta ao `initialize` manda o `session/new` na pasta da instância.
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"OpenCode","version":"2.0.32"}}}"#;
        assert_eq!(
            acp_events(init, &ctx),
            vec![ChatEvent::Reply(acp_new_session(Path::new("/projetos/x")))]
        );
        let session = r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_1","configOptions":[{"id":"model","currentValue":"9router/mimo"}]}}"#;
        assert_eq!(
            acp_events(session, &ctx),
            vec![
                ChatEvent::Session("ses_1".into()),
                ChatEvent::Init {
                    model: "9router/mimo".into(),
                    commands: Vec::new()
                }
            ]
        );
        // A mensagem usa a sessão aberta e o fim dela chega na resposta.
        let prompt: Value = serde_json::from_str(&acp_input("oi", Some("ses_1"))).unwrap();
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(prompt["params"]["sessionId"], "ses_1");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "oi");
        let end = r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#;
        assert_eq!(
            acp_events(end, &ctx),
            vec![ChatEvent::Done {
                error: None,
                denied: Vec::new()
            }]
        );
        let error = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"Internal error: ConnectionRefused"}}"#;
        assert_eq!(
            acp_events(error, &ctx),
            vec![ChatEvent::Done {
                error: Some("Internal error: ConnectionRefused".into()),
                denied: Vec::new()
            }]
        );
    }

    #[test]
    fn reads_opencode_updates_and_commands() {
        let ctx = ChatCtx::default();
        let chunk = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"ses_1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Olá"}}}}"#;
        assert_eq!(acp_events(chunk, &ctx), vec![ChatEvent::Text("Olá".into())]);
        let commands = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"compact","description":"Compacta a sessão"}]}}}"#;
        assert_eq!(
            acp_events(commands, &ctx),
            vec![ChatEvent::Commands(vec![SlashCommand {
                name: "compact".into(),
                description: "Compacta a sessão".into(),
                hint: String::new(),
            }])]
        );
        // A ferramenta aparece no começo, ganha o título no meio e a saída
        // no fim — pelo mesmo id.
        let tool = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","name":"shell","title":"shell","status":"pending"}}}"#;
        assert_eq!(
            acp_events(tool, &ctx),
            vec![ChatEvent::Tool {
                id: "t1".into(),
                name: "shell".into(),
                detail: "shell".into()
            }]
        );
        let running = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"in_progress","title":"ls -la"}}}"#;
        assert_eq!(
            acp_events(running, &ctx),
            vec![ChatEvent::Tool {
                id: "t1".into(),
                name: String::new(),
                detail: "ls -la".into()
            }]
        );
        let completed = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"main.rs"}}]}}}"#;
        assert_eq!(
            acp_events(completed, &ctx),
            vec![ChatEvent::ToolResult {
                error: false,
                text: "main.rs".into()
            }]
        );
        // O raciocínio e o resto do que o chat não mostra ficam de fora.
        let thought = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"pensando"}}}}"#;
        assert!(acp_events(thought, &ctx).is_empty());
    }

    #[test]
    fn asks_and_answers_opencode_permissions() {
        let line = r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":"ses_1","toolCall":{"toolCallId":"t1","name":"shell","title":"ls -la","status":"pending","rawInput":{"command":"ls -la"}},"options":[{"optionId":"once","kind":"allow_once"},{"optionId":"always","kind":"allow_always"},{"optionId":"reject","kind":"reject_once"}]}}"#;
        let Some(ChatEvent::Permission(permission)) = acp_events(line, &ChatCtx::default()).pop()
        else {
            panic!("esperava um pedido de permissão");
        };
        assert_eq!(
            (permission.tool.as_str(), permission.detail.as_str()),
            ("shell", "ls -la")
        );
        assert_eq!(permission.id, 7);
        assert!(!permission.always.is_null());
        // A resposta repassa o id do pedido e a opção escolhida.
        let reply: Value =
            serde_json::from_str(&acp_permission(&permission, Answer::Always)).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["outcome"]["optionId"], "always");
        let reply: Value =
            serde_json::from_str(&acp_permission(&permission, Answer::Deny)).unwrap();
        assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
        // Outros pedidos do agente são recusados na hora.
        let Some(ChatEvent::Reply(reply)) = acp_events(
            r#"{"jsonrpc":"2.0","id":8,"method":"elication/create","params":{}}"#,
            &ChatCtx::default(),
        )
        .pop() else {
            panic!("esperava uma recusa");
        };
        assert!(reply.contains("-32601") && reply.contains("\"id\":8"));
        // Notificação sem resposta.
        assert!(
            acp_events(
                r#"{"jsonrpc":"2.0","method":"session/cancel","params":{}}"#,
                &ChatCtx::default()
            )
            .is_empty()
        );
    }
}
