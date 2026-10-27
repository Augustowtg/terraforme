//! Consumo da conta do Claude: o mesmo que o `/usage` do Claude Code mostra.
//!
//! O `claude` não informa o consumo pelo `-p`; ele vem do endpoint de uso da
//! API (`/api/oauth/usage`), com o token OAuth da conta (o
//! `.credentials.json` da pasta dela). Não chama o modelo nem gasta tokens.
//!
//! A chamada é feita pelo `curl`, com os headers pelo stdin para o token não
//! aparecer na linha de comando (`ps`).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

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
        let _ = tx.send(fetch_cached(&config_dir));
    });
    rx
}

/// Quanto uma resposta vale para as outras chamadas (a aba e a barra de
/// consumo pedem juntas).
const FRESH: Duration = Duration::from_secs(50);
/// Espera depois de um HTTP 429; dobra a cada 429 seguido, até [`MAX_BACKOFF`].
const BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
struct Cached {
    /// A última resposta boa e quando chegou.
    usage: Option<(Instant, Usage)>,
    /// Com 429: até quando não chamar, e quantos 429 seguidos.
    blocked_until: Option<Instant>,
    strikes: u32,
}

static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// Uma chamada por vez e por conta: quem pede logo depois recebe a mesma
/// resposta, e depois de um 429 ninguém chama até passar a espera.
fn fetch_cached(config_dir: &Path) -> Result<Usage, String> {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let entry = guard
        .get_or_insert_with(HashMap::new)
        .entry(config_dir.to_path_buf())
        .or_default();
    let now = Instant::now();
    if let Some((at, usage)) = &entry.usage
        && now.duration_since(*at) < FRESH
    {
        return Ok(usage.clone());
    }
    if let Some(until) = entry.blocked_until
        && until > now
    {
        return Err(rate_limited(until - now));
    }
    match fetch(config_dir) {
        Ok(usage) => {
            entry.usage = Some((Instant::now(), usage.clone()));
            entry.blocked_until = None;
            entry.strikes = 0;
            Ok(usage)
        }
        Err(Fetch::RateLimited) => {
            let wait = BACKOFF
                .saturating_mul(1 << entry.strikes.min(4))
                .min(MAX_BACKOFF);
            entry.strikes += 1;
            entry.blocked_until = Some(Instant::now() + wait);
            Err(rate_limited(wait))
        }
        Err(Fetch::Other(error)) => Err(error),
    }
}

fn rate_limited(wait: Duration) -> String {
    let secs = wait.as_secs().max(1);
    let wait = if secs >= 60 {
        format!("{} min", secs.div_ceil(60))
    } else {
        format!("{secs} s")
    };
    format!("o Claude limitou as consultas de consumo (HTTP 429): nova tentativa em {wait}")
}

enum Fetch {
    RateLimited,
    Other(String),
}

impl From<String> for Fetch {
    fn from(error: String) -> Self {
        Fetch::Other(error)
    }
}

fn fetch(config_dir: &Path) -> Result<Usage, Fetch> {
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
        return Err(Fetch::Other(format!(
            "sem conexão com o Claude: {}",
            stderr.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout.rsplit_once('\n').unwrap_or(("", &stdout));
    match status.trim() {
        "200" => Ok(parse(body, credentials.plan)?),
        "429" => Err(Fetch::RateLimited),
        "401" | "403" => Err(Fetch::Other(
            "o login do Claude expirou ou foi revogado: use /login no chat".to_string(),
        )),
        status => Err(Fetch::Other(format!("o Claude respondeu HTTP {status}"))),
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
/// O crédito extra entra no fim, só quando a conta o tem ativo.
fn parse(body: &str, plan: Option<String>) -> Result<Usage, String> {
    let json: Value =
        serde_json::from_str(body).map_err(|_| "resposta inválida do Claude".to_string())?;
    let mut windows: Vec<UsageWindow> = WINDOWS
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
                detail: None,
                session: *key == "five_hour",
            })
        })
        .collect();
    windows.extend(extra_usage(&json["extra_usage"]));
    Ok(Usage { plan, windows })
}

/// O crédito extra (cobrado quando os limites do plano acabam): o gasto do
/// mês contra o limite mensal, os dois em centavos (`decimal_places`).
fn extra_usage(extra: &Value) -> Option<UsageWindow> {
    if extra["is_enabled"] != true {
        return None;
    }
    let limit = extra["monthly_limit"].as_f64()?;
    let used = extra["used_credits"].as_f64().unwrap_or(0.0);
    let percent = extra["utilization"]
        .as_f64()
        .or_else(|| (limit > 0.0).then(|| used / limit * 100.0))?
        .clamp(0.0, 100.0);
    let places = extra["decimal_places"].as_u64().unwrap_or(2) as i32;
    let currency = extra["currency"].as_str().unwrap_or("USD");
    let money = |minor: f64| format_money(minor / 10f64.powi(places), currency, places);
    let left = (limit - used).max(0.0);
    let mut detail = format!("{} restantes de {} no mês", money(left), money(limit));
    if extra["spend_limit_reached"] == true {
        detail.push_str(" · limite atingido");
    }
    Some(UsageWindow {
        label: "Crédito extra (mensal)".to_string(),
        percent,
        resets_at: None,
        detail: Some(detail),
        session: false,
    })
}

/// `US$ 1.234,56`: símbolo da moeda e número no formato brasileiro.
fn format_money(value: f64, currency: &str, places: i32) -> String {
    let symbol = match currency {
        "USD" => "US$",
        "BRL" => "R$",
        "EUR" => "€",
        other => other,
    };
    let places = places.max(0) as usize;
    let text = format!("{value:.places$}");
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    let digits: Vec<char> = int.chars().collect();
    let mut grouped = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(*c);
    }
    if frac.is_empty() {
        format!("{symbol} {grouped}")
    } else {
        format!("{symbol} {grouped},{frac}")
    }
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
                    detail: None,
                    session: true,
                },
                UsageWindow {
                    label: "Semana (todos os modelos)".into(),
                    percent: 100.0,
                    resets_at: None,
                    detail: None,
                    session: false,
                },
            ]
        );
    }

    #[test]
    fn shows_extra_credit_only_when_enabled() {
        let body = r#"{
            "five_hour": {"utilization": 7, "resets_at": null},
            "extra_usage": {"is_enabled": true, "monthly_limit": 4000, "used_credits": 178.0,
                "utilization": 4.45, "currency": "USD", "decimal_places": 2,
                "spend_limit_reached": false}
        }"#;
        let usage = parse(body, None).unwrap();
        assert_eq!(
            usage.windows.last(),
            Some(&UsageWindow {
                label: "Crédito extra (mensal)".into(),
                percent: 4.45,
                resets_at: None,
                detail: Some("US$ 38,22 restantes de US$ 40,00 no mês".into()),
                session: false,
            })
        );

        let disabled = body.replace(r#""is_enabled": true"#, r#""is_enabled": false"#);
        assert_eq!(parse(&disabled, None).unwrap().windows.len(), 1);
        let missing = r#"{"five_hour": {"utilization": 7, "resets_at": null}}"#;
        assert_eq!(parse(missing, None).unwrap().windows.len(), 1);
    }

    #[test]
    fn extra_credit_shows_when_limit_is_reached() {
        let extra = serde_json::json!({
            "is_enabled": true, "monthly_limit": 123456, "used_credits": 123456,
            "currency": "BRL", "decimal_places": 2, "spend_limit_reached": true
        });
        let window = extra_usage(&extra).unwrap();
        assert_eq!(window.percent, 100.0);
        assert_eq!(
            window.detail.as_deref(),
            Some("R$ 0,00 restantes de R$ 1.234,56 no mês · limite atingido")
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
