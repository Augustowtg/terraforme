//! Servidor HTTP+JSON-RPC 2.0 do daemon MCP e endpoints de controle do ciclo de vida.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use terraforme_core::mcp::McpConfig;

use terraforme_core::pet::Activity;

use crate::hub::{Hub, Registration};
use crate::lifecycle::ClientRegistry;
use crate::tools::spawn_subagent::{self, PendingSpawns, SpawnResult};
use crate::tools::{manage, messaging};

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<ClientRegistry>,
    /// Instâncias de todos os terminais e as mensagens entre elas.
    pub hub: Hub,
    pub pending_spawns: PendingSpawns,
    pub config: McpConfig,
}

/// Monta o router axum com todas as rotas.
pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/", post(mcp_handler).get(crate::ui::page))
        .route("/register", post(register_handler))
        .route("/heartbeat", post(heartbeat_handler))
        .route("/unregister", post(unregister_handler))
        .route("/spawn-result", post(spawn_result_handler))
        .route("/instances/register", post(register_instance_handler))
        .route("/instances/unregister", post(unregister_instance_handler))
        .route("/instances/state", post(instance_state_handler))
        .route("/pet/state", get(crate::pet::state))
        .route("/pet/focus", post(crate::pet::focus))
        .route("/ui", get(crate::ui::page))
        .route("/api/state", get(crate::ui::state))
        .route("/api/usage", get(crate::ui::usage))
        .route("/api/send", post(crate::ui::send))
        .with_state(state)
}

// ── Tipos JSON-RPC ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Handler principal MCP: recebe JSON-RPC 2.0 e despacha pelo método.
async fn mcp_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let client_id = header("x-client-id");
    let instance_id = header("x-instance-id");

    let id = req.id.unwrap_or(Value::Null);
    let params = req.params.unwrap_or(Value::Null);

    let response = match req.method.as_str() {
        "initialize" => JsonRpcResponse::ok(
            id,
            json!({
                // Ecoa a versão pedida pelo cliente: o daemon só usa tools.
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05"),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "terraforme-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "ping" => JsonRpcResponse::ok(id, json!({})),
        method if method.starts_with("notifications/") => {
            return (StatusCode::ACCEPTED, axum::body::Body::empty()).into_response();
        }
        "tools/list" => {
            let mut tools = messaging::definitions();
            tools.extend(manage::definitions());
            tools.push(spawn_subagent::definition());
            JsonRpcResponse::ok(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").unwrap_or(&Value::Null);
            let result = match name {
                "spawn_subagent" => spawn_subagent::handle(&client_id, args, &state).await,
                "list_instances" => Ok(messaging::list_instances(&instance_id, &state)),
                "send_message" => messaging::send_message(&instance_id, args, &state).await,
                "open_instance" => manage::open_instance(&instance_id, args, &state).await,
                "close_instance" => manage::close_instance(&instance_id, args, &state).await,
                "delegate_task" => manage::delegate_task(&instance_id, args, &state).await,
                other => {
                    let message = format!("ferramenta '{other}' não encontrada");
                    return Json(JsonRpcResponse::err(id, -32601, message)).into_response();
                }
            };
            match result {
                Ok(result) => JsonRpcResponse::ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": result.to_string() }] }),
                ),
                // Erro da ferramenta: o agente vê o motivo e pode corrigir a chamada.
                Err(e) => JsonRpcResponse::ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                ),
            }
        }
        other => {
            warn!(method = other, "método MCP desconhecido");
            JsonRpcResponse::err(id, -32601, format!("método '{other}' não suportado"))
        }
    };

    Json(response).into_response()
}

// ── Endpoints de controle de ciclo de vida ──────────────────────────────────

#[derive(Deserialize)]
struct RegisterBody {
    client_id: String,
    pid: u32,
    control_addr: String,
}

async fn register_handler(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> StatusCode {
    state
        .registry
        .register(body.client_id, body.pid, body.control_addr);
    StatusCode::OK
}

#[derive(Deserialize)]
struct HeartbeatBody {
    client_id: String,
}

async fn heartbeat_handler(
    State(state): State<AppState>,
    Json(body): Json<HeartbeatBody>,
) -> StatusCode {
    if state.registry.heartbeat(&body.client_id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

#[derive(Deserialize)]
struct UnregisterBody {
    client_id: String,
}

async fn unregister_handler(
    State(state): State<AppState>,
    Json(body): Json<UnregisterBody>,
) -> StatusCode {
    state.registry.unregister(&body.client_id);
    StatusCode::OK
}

#[derive(Deserialize)]
struct SpawnResultBody {
    spawn_id: String,
    output: String,
    exit_code: i32,
}

async fn spawn_result_handler(
    State(state): State<AppState>,
    Json(body): Json<SpawnResultBody>,
) -> StatusCode {
    let tx = state.pending_spawns.lock().unwrap().remove(&body.spawn_id);
    if let Some(tx) = tx {
        let _ = tx.send(SpawnResult {
            output: body.output,
            exit_code: body.exit_code,
        });
    }
    StatusCode::OK
}

async fn register_instance_handler(
    State(state): State<AppState>,
    Json(body): Json<Registration>,
) -> StatusCode {
    state.hub.register(body);
    StatusCode::OK
}

#[derive(Deserialize)]
struct UnregisterInstanceBody {
    instance_id: String,
}

async fn unregister_instance_handler(
    State(state): State<AppState>,
    Json(body): Json<UnregisterInstanceBody>,
) -> StatusCode {
    state.hub.unregister(&body.instance_id);
    StatusCode::OK
}

#[derive(Deserialize)]
struct InstanceStateBody {
    instance_id: String,
    activity: Activity,
}

/// O terminal informa a mudança de atividade de uma instância (para a janela
/// flutuante).
async fn instance_state_handler(
    State(state): State<AppState>,
    Json(body): Json<InstanceStateBody>,
) -> StatusCode {
    state.hub.set_activity(&body.instance_id, body.activity);
    StatusCode::OK
}
