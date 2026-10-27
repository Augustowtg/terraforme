//! Consumo da conta do Claude: o mesmo que o `/usage` do Claude Code mostra.
//!
//! O `claude` não informa o consumo pelo `-p`; ele vem do endpoint de uso da
//! API (`/api/oauth/usage`), com o token OAuth da conta (o
//! `.credentials.json` da pasta dela). Não chama o modelo nem gasta tokens.
//!
//! A chamada é feita pelo `curl`, com os headers pelo stdin para o token não
//! aparecer na linha de comando (`ps`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

use crate::agent::{Usage, UsageWindow};

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const CREDENTIALS: &str = ".credentials.json";
/// Segundos até desistir da chamada.
const TIMEOUT: &str = "10";

/// As janelas que o endpoint informa, na ordem da tela.
const WINDOWS: [(&str, &str); 4] = [
    ("five_hour", "Sessão (5 h)"),
    ("seven_day", "Semana (todos os modelos)"),
    ("seven_day_opus", "Semana (Opus)"),
    ("seven_day_sonnet", "Semana (Sonnet)"),
];

/// Começa a carregar numa thread; o resultado chega pelo canal. `config_dir`
/// é a pasta da conta (ex.: `~/.claude`).
pub fn load(config_dir: PathBuf) -> Receiver<Result<Usage, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(fetch(&config_dir));
    });
    rx
}

fn fetch(config_dir: &Path) -> Result<Usage, String> {
    let credentials = read_credentials(config_dir)?;
    let mut child = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            TIMEOUT,
            "-H",
            "@-",
            "-w",
            "\n%{http_code}",
            URL,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("não foi possível iniciar o curl: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let headers = format!(
            "Authorization: Bearer {}\nanthropic-beta: {BETA}\n",
            credentials.token
        );
        let _ = stdin.write_all(headers.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("o curl falhou: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("sem conexão com o Claude: {}", stderr.trim()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout.rsplit_once('\n').unwrap_or(("", &stdout));
    match status.trim() {
        "200" => parse(body, credentials.plan),
        "401" | "403" => {
            Err("o login do Claude expirou: envie uma mensagem no chat para renová-lo".to_string())
        }
        status => Err(format!("o Claude respondeu HTTP {status}")),
    }
}

/// O token e o plano da conta.
struct Credentials {
    token: String,
    plan: Option<String>,
}

fn read_credentials(config_dir: &Path) -> Result<Credentials, String> {
    let path = config_dir.join(CREDENTIALS);
    let json: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| {
            format!(
                "credenciais do Claude não encontradas em {}",
                path.display()
            )
        })?;
    let oauth = &json["claudeAiOauth"];
    let token = oauth["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("a conta não tem login do Claude (claude.ai)")?;
    Ok(Credentials {
        token: token.to_string(),
        plan: oauth["subscriptionType"].as_str().map(capitalize),
    })
}

/// A resposta do endpoint: cada janela tem `utilization` (0–100) e
/// `resets_at` (ISO 8601); janelas que não valem para o plano vêm `null`.
fn parse(body: &str, plan: Option<String>) -> Result<Usage, String> {
    let json: Value =
        serde_json::from_str(body).map_err(|_| "resposta inválida do Claude".to_string())?;
    let windows = WINDOWS
        .iter()
        .filter_map(|(key, label)| {
            let window = &json[key];
            Some(UsageWindow {
                label: label.to_string(),
                percent: window["utilization"].as_f64()?.clamp(0.0, 100.0),
                resets_at: window["resets_at"]
                    .as_str()
                    .and_then(|s| s.parse::<jiff::Timestamp>().ok())
                    .map(|t| t.as_second()),
            })
        })
        .collect();
    Ok(Usage { plan, windows })
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_skipping_null_ones() {
        let body = r#"{
            "five_hour": {"utilization": 42.5, "resets_at": "2026-10-27T18:00:00.123+00:00"},
            "seven_day": {"utilization": 130, "resets_at": null},
            "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": null, "resets_at": null},
            "extra_usage": {"is_enabled": false}
        }"#;
        let usage = parse(body, Some("Max".into())).unwrap();
        assert_eq!(usage.plan.as_deref(), Some("Max"));
        assert_eq!(
            usage.windows,
            [
                UsageWindow {
                    label: "Sessão (5 h)".into(),
                    percent: 42.5,
                    resets_at: Some(1_793_124_000),
                },
                UsageWindow {
                    label: "Semana (todos os modelos)".into(),
                    percent: 100.0,
                    resets_at: None,
                },
            ]
        );
    }

    #[test]
    fn rejects_invalid_body() {
        assert!(parse("<html>", None).is_err());
    }

    #[test]
    fn reads_token_and_plan_from_credentials() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_credentials(dir.path()).is_err());
        fs::write(
            dir.path().join(CREDENTIALS),
            r#"{"claudeAiOauth":{"accessToken":"tok","subscriptionType":"max"}}"#,
        )
        .unwrap();
        let credentials = read_credentials(dir.path()).unwrap();
        assert_eq!(credentials.token, "tok");
        assert_eq!(credentials.plan.as_deref(), Some("Max"));
    }
}
