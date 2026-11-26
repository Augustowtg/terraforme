//! Servidor HTTP para monitoramento e controle remoto do terra pelo app mobile.
//!
//! O app mobile conecta via Wi-Fi (`http://<ip-do-computador>:8765`) ou via USB
//! (`adb reverse tcp:8765 tcp:8765` -> `http://localhost:8765`).

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::hub;
use crate::hub::endpoint::Endpoint;
use crate::orchestrator::config::Config;
use crate::orchestrator::costs;
use crate::orchestrator::run;

/// A porta padrão, que o app procura na rede.
pub const PORT: u16 = 8765;

/// Abre o servidor na rede (todas as interfaces, porta [`PORT`]) sem
/// mensagens: o hub o liga ao iniciar.
// shortcut: sem senha, qualquer um na rede digita nas abas; pôr token antes de abrir fora da LAN.
pub fn listen_default() -> Result<()> {
    accept(bind("0.0.0.0", PORT)?)
}

fn bind(bind: &str, port: u16) -> Result<TcpListener> {
    let addr = format!("{bind}:{port}");
    TcpListener::bind(&addr).with_context(|| format!("não foi possível abrir o servidor em {addr}"))
}

/// Inicia o servidor HTTP e escuta requisições na porta configurada.
pub fn serve(bind_addr: &str, port: u16) -> Result<()> {
    let addr = format!("{bind_addr}:{port}");
    let listener = bind(bind_addr, port)?;

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║              TERRAFORME REMOTE SERVER                    ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Ouvindo em http://{addr}");
    println!();
    println!("Para conectar o celular via USB (ADB):");
    println!("  adb reverse tcp:{port} tcp:{port}");
    println!("  -> No app, use: http://localhost:{port}");
    println!();
    println!("Para conectar o celular via Wi-Fi:");
    println!("  -> No app, use o IP deste computador (porta {port})");
    println!();
    println!("Pressione Ctrl+C para encerrar.");
    accept(listener)
}

fn accept(listener: TcpListener) -> Result<()> {
    let endpoint = Arc::new(Endpoint::current()?);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let ep = Arc::clone(&endpoint);
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, ep) {
                        eprintln!("[remote] erro na requisição: {e:#}");
                    }
                });
            }
            Err(e) => eprintln!("[remote] erro de conexão: {e}"),
        }
    }

    Ok(())
}

fn handle_client(mut stream: TcpStream, endpoint: Arc<Endpoint>) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];

    // Lê os cabeçalhos
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:") {
            if let Some(val) = line.split(':').nth(1) {
                content_length = val.trim().parse().unwrap_or(0);
            }
        }
    }

    // Lê o corpo (se houver)
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    // CORS preflight
    if method == "OPTIONS" {
        return send_response(&mut stream, 204, "No Content", &json!({}));
    }

    let (status_code, status_text, response_json) = match (method, path) {
        ("GET", "/api/health") => (
            200,
            "OK",
            json!({
                "status": "ok",
                "app": "terraforme",
                "version": env!("CARGO_PKG_VERSION"),
                "tailscale": tailscale_ip()
            }),
        ),

        // O histórico do chat da aba `id`, gravado no SQLite do workspace.
        ("GET", p) if p.starts_with("/api/history/") => {
            match history(&endpoint, &p["/api/history/".len()..]) {
                Ok(messages) => (200, "OK", json!({ "messages": messages })),
                Err(e) => (404, "Not Found", json!({ "error": format!("{e:#}") })),
            }
        }

        ("GET", "/api/status") => match hub::client::snapshot_with(&endpoint, true) {
            Ok(instances) => (
                200,
                "OK",
                json!({
                    "instances": instances,
                    "timestamp": hub::protocol::now()
                }),
            ),
            Err(e) => (
                500,
                "Internal Server Error",
                json!({
                    "error": format!("erro ao obter snapshot do hub: {e:#}")
                }),
            ),
        },

        ("GET", "/api/agents") => {
            let config_path = Config::path(&endpoint.home);
            match Config::load(&config_path) {
                Ok(config) => {
                    let agents: Vec<Value> = config
                        .agents
                        .iter()
                        .map(|agent| {
                            json!({
                                "name": agent.name,
                                "cwd": agent.cwd.display().to_string(),
                                "model": agent.model,
                                "max_turns": agent.max_turns,
                                "daily_budget": agent.daily_budget,
                                "tools": agent.tools,
                                "description": agent.description,
                            })
                        })
                        .collect();
                    (200, "OK", json!({ "agents": agents }))
                }
                Err(e) => (
                    500,
                    "Internal Server Error",
                    json!({
                        "error": format!("erro ao carregar orchestrator.toml: {e:#}")
                    }),
                ),
            }
        }

        ("GET", "/api/runs") => {
            let runs_dir = endpoint.home.join(run::DIR);
            let mut list = Vec::new();
            if let Ok(entries) = fs::read_dir(&runs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let json_file = path.join("run.json");
                        if let Ok(content) = fs::read_to_string(&json_file) {
                            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                                list.push(val);
                            }
                        }
                    }
                }
            }
            // Ordena pelo created ou started decrescente
            list.sort_by(|a, b| {
                let ta = a["created"]
                    .as_u64()
                    .or_else(|| a["started"].as_u64())
                    .unwrap_or(0);
                let tb = b["created"]
                    .as_u64()
                    .or_else(|| b["started"].as_u64())
                    .unwrap_or(0);
                tb.cmp(&ta)
            });
            (200, "OK", json!({ "runs": list }))
        }

        ("GET", p) if p.starts_with("/api/runs/") => {
            let run_id = &p["/api/runs/".len()..];
            let run_dir = endpoint.home.join(run::DIR).join(run_id);
            if !run_dir.exists() {
                (
                    404,
                    "Not Found",
                    json!({ "error": "execução não encontrada" }),
                )
            } else {
                let run_meta = fs::read_to_string(run_dir.join("run.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .unwrap_or(json!({}));

                let stderr = fs::read_to_string(run_dir.join(run::STDERR)).unwrap_or_default();

                let mut output_events = Vec::new();
                if let Ok(file) = fs::File::open(run_dir.join(run::OUTPUT)) {
                    let r = BufReader::new(file);
                    for line in r.lines().flatten() {
                        if let Ok(ev) = serde_json::from_str::<Value>(&line) {
                            output_events.push(ev);
                        }
                    }
                }

                (
                    200,
                    "OK",
                    json!({
                        "run": run_meta,
                        "stderr": stderr,
                        "output": output_events,
                    }),
                )
            }
        }

        ("GET", "/api/workspaces") => match crate::db::Db::open_default() {
            Ok(db) => {
                const ALL: usize = 100;
                let saved = db.workspaces(0, ALL).unwrap_or_default();
                let recents = db.recents(0, ALL).unwrap_or_default();
                let saved_json: Vec<_> = saved
                    .iter()
                    .map(|w| json!({"name": w.name, "path": w.path}))
                    .collect();
                let recents_json: Vec<_> = recents
                    .iter()
                    .map(|r| json!({"path": r.path, "last_used_at": r.last_used_at}))
                    .collect();
                (
                    200,
                    "OK",
                    json!({ "saved": saved_json, "recents": recents_json }),
                )
            }
            Err(e) => (
                500,
                "Internal Server Error",
                json!({ "error": format!("{e:#}") }),
            ),
        },

        ("GET", "/api/usage") => {
            let config_path = Config::path(&endpoint.home);
            match Config::load(&config_path) {
                Ok(config) => {
                    let runs = run::list(&endpoint.home);
                    let today = run::today();
                    let days = costs::by_agent(&runs, &config, today);
                    let agents: Vec<Value> = days
                        .iter()
                        .map(|d| {
                            json!({
                                "agent": d.agent,
                                "runs": d.runs,
                                "tokens": d.tokens,
                                "cost_usd": d.cost_usd,
                                "budget": d.budget,
                                "alert": d.alert().map(|a| match a {
                                    costs::Alert::Warning => "warning",
                                    costs::Alert::Blocked => "blocked",
                                }),
                            })
                        })
                        .collect();
                    let running = runs
                        .iter()
                        .filter(|r| r.state == run::RunState::Running)
                        .count();
                    (
                        200,
                        "OK",
                        json!({
                            "agents": agents,
                            "total_runs": days.iter().map(|d| d.runs).sum::<u32>(),
                            "total_tokens": days.iter().map(|d| d.tokens).sum::<u64>(),
                            "total_cost_usd": days.iter().map(|d| d.cost_usd).sum::<f64>(),
                            "running": running,
                        }),
                    )
                }
                Err(e) => (
                    500,
                    "Internal Server Error",
                    json!({
                        "error": format!("erro ao carregar orchestrator.toml: {e:#}")
                    }),
                ),
            }
        }

        // Digita na aba `tab` do terra aberto, como no teclado do PC.
        ("POST", "/api/input") => {
            let req: Value = serde_json::from_slice(&body).unwrap_or_default();
            let (Some(tab), Some(text)) = (req["tab"].as_str(), req["text"].as_str()) else {
                return send_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({ "error": "campos 'tab' e 'text' são obrigatórios" }),
                );
            };
            match hub::client::send_input(&endpoint, tab, text) {
                Ok(reply) if reply["t"] == "ok" => (200, "OK", json!({ "ok": true })),
                Ok(reply) => (
                    404,
                    "Not Found",
                    json!({ "error": reply["text"].as_str().unwrap_or("aba recusada") }),
                ),
                Err(e) => (
                    500,
                    "Internal Server Error",
                    json!({ "error": format!("erro ao falar com o hub: {e:#}") }),
                ),
            }
        }

        ("POST", "/api/send") => {
            let req_body: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return send_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({ "error": format!("JSON inválido: {e}") }),
                    );
                }
            };

            let mut to = req_body["agent"].as_str().unwrap_or("").to_string();
            // `tab`: o id de uma aba aberta; vira o nome de agente dela.
            if let Some(tab) = req_body["tab"].as_str() {
                let found = hub::client::snapshot_with(&endpoint, true)
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|i| i["status"]["tabs"].as_array().cloned().unwrap_or_default())
                    .find(|t| t["id"] == tab);
                match found.as_ref().and_then(|t| t["name"].as_str()) {
                    Some(name) => to = name.to_string(),
                    None if found.is_some() => {
                        return send_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            &json!({ "error": "a aba não tem nome de agente; use /agente no terra para nomeá-la" }),
                        );
                    }
                    None => {
                        return send_response(
                            &mut stream,
                            404,
                            "Not Found",
                            &json!({ "error": "aba não encontrada" }),
                        );
                    }
                }
            }
            let text = req_body["text"].as_str().map(str::to_string);
            let refs: Vec<PathBuf> = req_body["refs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(PathBuf::from))
                        .collect()
                })
                .unwrap_or_default();
            let tag = req_body["tag"].as_str().map(str::to_string);

            if to.is_empty() {
                return send_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({ "error": "campo 'agent' é obrigatório" }),
                );
            }

            let config_path = Config::path(&endpoint.home);
            let config = match Config::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    return send_response(
                        &mut stream,
                        500,
                        "Internal Server Error",
                        &json!({ "error": format!("orchestrator.toml: {e}") }),
                    );
                }
            };

            let send_args = crate::orchestrator::send::Args {
                to: to.clone(),
                text,
                refs,
                handoff: None,
                tag,
            };

            let (from, hops) = crate::orchestrator::sender();
            let cwd = std::env::current_dir().unwrap_or_default();

            match crate::orchestrator::send::prepare(
                &send_args,
                &from,
                hops,
                &config,
                &endpoint.home,
                &cwd,
            ) {
                Ok(message) => {
                    let msg_id = message.id.clone();
                    match hub::client::send_message(&endpoint, true, &message.to_json()) {
                        Ok(reply) => (
                            200,
                            "OK",
                            json!({
                                "ok": true,
                                "id": msg_id,
                                "reply": reply
                            }),
                        ),
                        Err(e) => (
                            500,
                            "Internal Server Error",
                            json!({ "error": format!("erro ao enviar mensagem: {e:#}") }),
                        ),
                    }
                }
                Err(reject) => (400, "Bad Request", json!({ "error": reject.text })),
            }
        }

        _ => (
            404,
            "Not Found",
            json!({ "error": format!("rota não encontrada: {method} {path}") }),
        ),
    };

    send_response(&mut stream, status_code, status_text, &response_json)
}

/// As mensagens da conversa da aba `tab`: a aba diz o workspace e a
/// conversa, e o histórico está no banco do workspace.
fn history(endpoint: &Endpoint, tab: &str) -> Result<Vec<Value>> {
    let instances = hub::client::snapshot_with(endpoint, true)?;
    let (workspace, conversation) = instances
        .iter()
        .find_map(|i| {
            let t = i["status"]["tabs"]
                .as_array()?
                .iter()
                .find(|t| t["id"] == tab)?;
            Some((i["workspace"].as_str()?, t["conversation"].as_i64()))
        })
        .context("aba não encontrada")?;
    let Some(conversation) = conversation else {
        return Ok(Vec::new());
    };
    let file = crate::settings::history_file(&endpoint.home, workspace);
    let store = crate::db::history::ChatHistory::open(&file)?;
    Ok(store
        .messages(conversation)?
        .into_iter()
        .map(|(role, tool, content)| json!({ "role": role, "tool": tool, "content": content }))
        .collect())
}

/// O IP desta máquina no Tailscale, que o app prefere ao da rede local. A rota
/// para o DNS do Tailscale (100.100.100.100) sai pela interface dele; um UDP
/// `connect` não manda nada.
fn tailscale_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("100.100.100.100:53").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if ip.octets()[0] == 100 && ip.octets()[1] & 0xC0 == 64 => {
            Some(ip.to_string())
        }
        _ => None,
    }
}

fn send_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    body: &Value,
) -> Result<()> {
    let body_str = serde_json::to_string_pretty(body)?;
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\n\
        Content-Type: application/json; charset=utf-8\r\n\
        Content-Length: {}\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
        Access-Control-Allow-Headers: *\r\n\
        Connection: close\r\n\
        \r\n\
        {}",
        body_str.len(),
        body_str
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}
