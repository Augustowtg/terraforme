//! Consumo da conta do Claude: o mesmo que o `/usage` do Claude Code mostra.
//!
//! O `claude` não informa o consumo pelo `-p`; ele vem do endpoint de uso da
//! API (`/api/oauth/usage`), com o token OAuth da conta (o
//! `.credentials.json` da pasta dela). Não chama o modelo nem gasta tokens.
//!
//! O token dura poucas horas e quem o renova é o `claude`: vencido, o terra
//! roda `claude auth status` na pasta da conta, que o renova, antes de usar.
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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::agent::{Usage, UsageWindow};

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const CREDENTIALS: &str = ".credentials.json";
/// Segundos até desistir da chamada.
const TIMEOUT: &str = "10";

/// As janelas de nome fixo, na ordem da tela, para a resposta sem `limits`.
/// As semanais por modelo (`seven_day_opus`...) vêm depois, com o nome
/// tirado da chave.
const WINDOWS: [(&str, &str); 2] = [("five_hour", SESSION), ("seven_day", WEEK)];
const SESSION: &str = "Sessão (5 h)";
const WEEK: &str = "Semana (todos os modelos)";
const WEEK_PREFIX: &str = "seven_day_";
const EXTRA: &str = "extra_usage";
/// A lista dos limites da conta, com o nome de cada um: é onde vem o
/// semanal de um modelo (ex.: o do Fable).
const LIMITS: &str = "limits";

/// Começa a carregar numa thread; o resultado chega pelo canal. `config_dir`
/// é a pasta da conta (ex.: `~/.claude`); `refresh` é o `claude auth status`
/// dela, que renova o token vencido.
pub fn load(config_dir: PathBuf, refresh: Command) -> Receiver<Result<Usage, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(fetch_cached(&config_dir, refresh));
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
fn fetch_cached(config_dir: &Path, refresh: Command) -> Result<Usage, String> {
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
    match fetch(config_dir, refresh) {
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

fn fetch(config_dir: &Path, refresh: Command) -> Result<Usage, Fetch> {
    let credentials = fresh_credentials(config_dir, refresh)?;
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
    /// Quando o token vence, em ms desde 1970; `None` se não vence.
    expires_at: Option<u64>,
}

impl Credentials {
    /// Vencido ou vencendo no próximo minuto.
    fn expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.expires_at.is_some_and(|at| at <= now + 60_000)
    }
}

/// As credenciais, renovadas pelo `refresh` se o token venceu. Se a
/// renovação falhar, fica o token velho, e o 401 explica.
fn fresh_credentials(config_dir: &Path, mut refresh: Command) -> Result<Credentials, String> {
    let credentials = read_credentials(config_dir)?;
    if !credentials.expired() {
        return Ok(credentials);
    }
    let _ = refresh
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    read_credentials(config_dir)
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
        expires_at: oauth["expiresAt"].as_u64(),
    })
}

/// A resposta do endpoint. Os limites vêm em `limits` (o da sessão, o da
/// semana e os semanais por modelo); sem ela, valem as janelas soltas, cada
/// uma com `utilization` (0–100) e `resets_at` (ISO 8601), `null` quando não
/// vale para o plano. O crédito extra entra no fim, só quando a conta o tem
/// ativo.
fn parse(body: &str, plan: Option<String>) -> Result<Usage, String> {
    let json: Value =
        serde_json::from_str(body).map_err(|_| "resposta inválida do Claude".to_string())?;
    let mut windows: Vec<UsageWindow> = json[LIMITS]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(limit)
        .collect();
    if windows.is_empty() {
        windows = loose_windows(&json);
    }
    windows.extend(extra_usage(&json[EXTRA]));
    Ok(Usage { plan, windows })
}

/// Um item de `limits`: `kind` diz qual é (`session`, `weekly_all`,
/// `weekly_scoped`) e `scope`, no último, de qual modelo ou superfície.
fn limit(limit: &Value) -> Option<UsageWindow> {
    let kind = limit["kind"].as_str()?;
    let scope = &limit["scope"];
    let scoped = [&scope["model"], &scope["surface"]]
        .into_iter()
        .find_map(|v| v["display_name"].as_str().or_else(|| v.as_str()));
    let label = match (kind, scoped) {
        ("session", _) => SESSION.to_string(),
        ("weekly_all", _) => WEEK.to_string(),
        (_, Some(name)) => format!("Semana ({name})"),
        (kind, None) => readable(kind),
    };
    Some(UsageWindow {
        label,
        percent: limit["percent"].as_f64()?.clamp(0.0, 100.0),
        resets_at: reset(&limit["resets_at"]),
        detail: None,
        session: kind == "session",
    })
}

/// As janelas soltas da resposta sem `limits`.
fn loose_windows(json: &Value) -> Vec<UsageWindow> {
    let fixed = WINDOWS.iter().map(|(key, label)| (*key, label.to_string()));
    let models = json
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, _)| {
            let model = key.strip_prefix(WEEK_PREFIX)?;
            Some((key.as_str(), format!("Semana ({})", readable(model))))
        });
    fixed
        .chain(models)
        .filter_map(|(key, label)| {
            let window = &json[key];
            Some(UsageWindow {
                label,
                percent: window["utilization"].as_f64()?.clamp(0.0, 100.0),
                resets_at: reset(&window["resets_at"]),
                detail: None,
                session: key == "five_hour",
            })
        })
        .collect()
}

fn reset(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        .map(|t| t.as_second())
}

/// `seven_day_opus` vira `Seven day opus`.
fn readable(key: &str) -> String {
    capitalize(&key.replace('_', " "))
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
    fn model_windows_come_from_the_keys() {
        let body = r#"{
            "seven_day_sonnet": {"utilization": 3, "resets_at": null},
            "seven_day_fable": {"utilization": 12, "resets_at": null},
            "seven_day_oauth_apps": null,
            "five_hour": {"utilization": 1, "resets_at": null},
            "extra_usage": {"is_enabled": false, "utilization": 9}
        }"#;
        let usage = parse(body, None).unwrap();
        let labels: Vec<_> = usage.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Sessão (5 h)", "Semana (Fable)", "Semana (Sonnet)"]
        );
        assert!(usage.windows[0].session && !usage.windows[1].session);
    }

    #[test]
    fn limits_list_names_the_model_window() {
        let body = r#"{
            "five_hour": {"utilization": 100.0, "resets_at": null},
            "seven_day": {"utilization": 14.0, "resets_at": null},
            "seven_day_opus": null,
            "tangelo": null,
            "limits": [
                {"kind": "session", "percent": 100, "resets_at": "2026-10-27T18:00:00.123+00:00", "scope": null},
                {"kind": "weekly_all", "percent": 14, "resets_at": null, "scope": null},
                {"kind": "weekly_scoped", "percent": 22, "resets_at": null,
                    "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}},
                {"kind": "weekly_scoped", "percent": 5, "resets_at": null,
                    "scope": {"model": null, "surface": "cowork"}},
                {"kind": "monthly_pool", "percent": 1, "resets_at": null, "scope": null},
                {"kind": "weekly_scoped", "percent": null, "scope": null}
            ]
        }"#;
        let usage = parse(body, None).unwrap();
        let found: Vec<_> = usage
            .windows
            .iter()
            .map(|w| (w.label.as_str(), w.percent, w.session))
            .collect();
        assert_eq!(
            found,
            [
                ("Sessão (5 h)", 100.0, true),
                ("Semana (todos os modelos)", 14.0, false),
                ("Semana (Fable)", 22.0, false),
                ("Semana (cowork)", 5.0, false),
                ("Monthly pool", 1.0, false),
            ]
        );
        assert_eq!(usage.windows[0].resets_at, Some(1_793_124_000));
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
        assert!(!credentials.expired());
    }

    #[test]
    fn renews_an_expired_token_before_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CREDENTIALS);
        fs::write(
            &path,
            r#"{"claudeAiOauth":{"accessToken":"velho","expiresAt":1}}"#,
        )
        .unwrap();
        assert!(read_credentials(dir.path()).unwrap().expired());
        // O `claude auth status` de mentira grava um token que vence em 2286.
        let mut refresh = Command::new("sh");
        refresh.arg("-c").arg(format!(
            r#"echo '{{"claudeAiOauth":{{"accessToken":"novo","expiresAt":9999999999999}}}}' > '{}'"#,
            path.display()
        ));
        let credentials = fresh_credentials(dir.path(), refresh).unwrap();
        assert_eq!(credentials.token, "novo");
        assert!(!credentials.expired());
    }
}
