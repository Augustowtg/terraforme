//! Servidor HTTP+JSON-RPC 2.0 do daemon MCP e endpoints de controle do ciclo de vida.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use terraforme_core::mcp::McpConfig;

use crate::lifecycle::ClientRegistry;
use crate::tools::spawn_subagent::{self, PendingSpawns, SpawnResult};

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<ClientRegistry>,
    pub pending_spawns: PendingSpawns,
    pub config: McpConfig,
}

/// Monta o router axum com todas as rotas.
pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/", post(mcp_handler))
        .route("/register", post(register_handler))
        .route("/heartbeat", post(heartbeat_handler))
        .route("/unregister", post(unregister_handler))
        .route("/spawn-result", post(spawn_result_handler))
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
    let client_id = headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let id = req.id.unwrap_or(Value::Null);
    let params = req.params.unwrap_or(Value::Null);

    let response = match req.method.as_str() {
        "initialize" => JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "terraforme-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "notifications/initialized" => {
            return (StatusCode::NO_CONTENT, axum::body::Body::empty()).into_response();
        }
        "tools/list" => JsonRpcResponse::ok(id, json!({ "tools": [spawn_subagent::definition()] })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").unwrap_or(&Value::Null);
            match name {
                "spawn_subagent" => match spawn_subagent::handle(&client_id, args, &state).await {
                    Ok(result) => JsonRpcResponse::ok(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": result.to_string()
                            }]
                        }),
                    ),
                    Err(e) => JsonRpcResponse::err(id, -32000, e),
                },
                other => {
                    JsonRpcResponse::err(id, -32601, format!("ferramenta '{other}' não encontrada"))
                }
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
    state.registry.heartbeat(&body.client_id);
    StatusCode::OK
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
