//! Endpoints da janela flutuante (`terraforme pet`): o estado das instâncias
//! de todos os terminais e o pedido de foco numa delas. Só pedidos locais,
//! mas sem depender de `[mcp] ui`.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::server::AppState;
use crate::ui::local;

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "origem não permitida").into_response()
}

pub async fn state(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !local(&headers) {
        return forbidden();
    }
    Json(json!({ "instances": state.hub.instances() })).into_response()
}

#[derive(Deserialize)]
pub struct FocusBody {
    instance_id: String,
}

/// Repassa ao terminal da instância o pedido de abrir a sub-aba dela.
pub async fn focus(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<FocusBody>,
) -> Response {
    if !local(&headers) {
        return forbidden();
    }
    match state.hub.focus(&body.instance_id).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}
