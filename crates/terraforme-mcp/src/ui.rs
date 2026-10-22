//! UI web do daemon (`/ui`, também em `GET /`): a página principal
//! "Terraforme instâncias" lista, por space, os agentes abertos no computador
//! (`Agente 1 (Claude)`, `Agente 2 (Opencode)`…), seguida das mensagens
//! entre eles e o consumo gravado por space e agente (`/api/usage`). Ligada por `[mcp] ui` no config.toml, lido a cada pedido, então
//! ligar/desligar vale sem reiniciar o daemon.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use terraforme_core::config::Config;
use terraforme_core::history::History;

use crate::hub::WEB;
use crate::server::AppState;

const PAGE: &str = include_str!("ui.html");

fn enabled() -> bool {
    Config::load().map(|c| c.mcp.ui).unwrap_or(false)
}

/// Só aceita a própria página: `Host` local (contra DNS rebinding) e, se
/// houver, `Origin` local (contra outro site postando no daemon).
pub(crate) fn local(headers: &HeaderMap) -> bool {
    let is_local = |value: &str| {
        let host = value
            .strip_prefix("http://")
            .unwrap_or(value)
            .split(':')
            .next()
            .unwrap_or("");
        host == "127.0.0.1" || host == "localhost"
    };
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    get(header::HOST).is_some_and(is_local) && get(header::ORIGIN).is_none_or(is_local)
}

/// Resposta quando a UI está desligada ou o pedido não é local.
fn refuse(headers: &HeaderMap) -> Option<Response> {
    if !local(headers) {
        return Some((StatusCode::FORBIDDEN, "origem não permitida").into_response());
    }
    if !enabled() {
        return Some(
            (
                StatusCode::NOT_FOUND,
                "UI desligada — ligue com `terraforme config --mcp-ui true` ou na aba Config",
            )
                .into_response(),
        );
    }
    None
}

pub async fn page(headers: HeaderMap) -> Response {
    refuse(&headers).unwrap_or_else(|| Html(PAGE).into_response())
}

pub async fn state(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(refused) = refuse(&headers) {
        return refused;
    }
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "instances": state.hub.instances(),
        "messages": state.hub.messages(),
    }))
    .into_response()
}

/// Consumo por space × agente, do histórico em SQLite gravado pelo
/// `terraforme cli` ([`History`]).
pub async fn usage(headers: HeaderMap) -> Response {
    if let Some(refused) = refuse(&headers) {
        return refused;
    }
    let totals = tokio::task::spawn_blocking(|| History::open().and_then(|db| db.totals())).await;
    match totals {
        Ok(Ok(totals)) => Json(json!({ "totals": totals })).into_response(),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SendBody {
    to: String,
    text: String,
}

/// Mensagem do usuário pela UI para uma instância.
pub async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SendBody>,
) -> Response {
    if let Some(refused) = refuse(&headers) {
        return refused;
    }
    match state.hub.send(WEB, &body.to, &body.text).await {
        Ok(to) => Json(json!({ "to": to })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            map.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        map
    }

    #[test]
    fn only_local_pages_are_accepted() {
        assert!(local(&headers("127.0.0.1:4000", None)));
        assert!(local(&headers(
            "localhost:4000",
            Some("http://localhost:4000")
        )));
        assert!(!local(&headers("evil.example:4000", None)));
        assert!(!local(&headers(
            "127.0.0.1:4000",
            Some("https://evil.example")
        )));
    }
}
