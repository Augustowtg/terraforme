//! `terraforme-mcp` — daemon MCP compartilhado. Inicia sob demanda pela CLI,
//! aceita múltiplos clientes `terraforme-cli` e encerra quando o último sai.

mod hub;
mod lifecycle;
mod server;
mod tools;
mod ui;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::EnvFilter;

use terraforme_core::mcp::{LockFile, McpConfig};

use crate::hub::Hub;
use crate::lifecycle::ClientRegistry;
use crate::server::AppState;

#[derive(Debug)]
struct Args {
    port: u16,
    grace_seconds: u64,
    log_level: String,
    subagent_timeout: u64,
    max_subagents: usize,
}

fn parse_args() -> Args {
    let mut args = Args {
        port: 0,
        grace_seconds: 5,
        log_level: "info".into(),
        subagent_timeout: 300,
        max_subagents: 3,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--port" => {
                i += 1;
                args.port = raw.get(i).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--grace-seconds" => {
                i += 1;
                args.grace_seconds = raw.get(i).and_then(|v| v.parse().ok()).unwrap_or(5);
            }
            "--log-level" => {
                i += 1;
                if let Some(v) = raw.get(i) {
                    args.log_level = v.clone();
                }
            }
            "--subagent-timeout" => {
                i += 1;
                args.subagent_timeout = raw.get(i).and_then(|v| v.parse().ok()).unwrap_or(300);
            }
            "--max-subagents" => {
                i += 1;
                args.max_subagents = raw.get(i).and_then(|v| v.parse().ok()).unwrap_or(3);
            }
            _ => {}
        }
        i += 1;
    }
    args
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&args.log_level))
        .init();

    info!("terraforme-mcp v{} iniciando", env!("CARGO_PKG_VERSION"));

    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    let port = listener.local_addr()?.port();

    info!(port, "ouvindo em 127.0.0.1:{}", port);
    LockFile::write(std::process::id(), port)?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let shutdown_tx = Arc::new(shutdown_tx);

    let config = McpConfig {
        grace_seconds: args.grace_seconds,
        subagent_timeout_seconds: args.subagent_timeout,
        max_simultaneous_subagents: args.max_subagents,
        ..McpConfig::default()
    };

    let hub = Hub::default();
    let registry = Arc::new(ClientRegistry::new(
        args.grace_seconds,
        shutdown_tx.clone(),
        hub.clone(),
    ));
    registry.spawn_watchdog();

    let state = AppState {
        registry,
        hub,
        pending_spawns: Arc::new(Mutex::new(HashMap::new())),
        config,
    };

    let router = server::router(state);

    // Handler de Ctrl-C / SIGTERM.
    let shutdown_tx_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("sinal recebido — encerrando");
            let _ = shutdown_tx_signal.send(true);
        }
    });

    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        loop {
            if shutdown_rx.changed().await.is_err() || *shutdown_rx.borrow() {
                break;
            }
        }
    });

    server.await?;

    LockFile::delete()?;
    info!("terraforme-mcp encerrado");
    Ok(())
}
