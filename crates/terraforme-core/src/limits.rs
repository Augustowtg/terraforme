//! Quanto falta dos limites de uso do agente. Com login OAuth no Claude Code
//! (assinaturas Pro/Max/Team), lê a utilização do plano em
//! `api.anthropic.com/api/oauth/usage` — o mesmo dado do `/usage`; sem login,
//! ou se a consulta falhar, compara o consumo dos transcripts com o orçamento
//! de `[budget]` do config.toml.

use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::Adapter;
use crate::agent::claude::claude_config_dir;
use crate::usage::{Usage, parse_timestamp};

const PLAN_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// A aba de consumo atualiza a cada poucos segundos; a API não precisa.
const PLAN_TTL: Duration = Duration::from_secs(60);

/// Orçamento de tokens (total, como na tabela de consumo) para quando os
/// limites do plano não estão disponíveis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    /// Tokens nas últimas 5 horas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hours: Option<u64>,
    /// Tokens nos últimos 7 dias.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub week: Option<u64>,
}

impl Budget {
    pub fn is_empty(&self) -> bool {
        self.five_hours.is_none() && self.week.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Limites da assinatura, informados pela Anthropic.
    Plan,
    /// Orçamento do config.toml comparado com os transcripts.
    Budget,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub label: String,
    /// Porcentagem já usada (0–100; pode passar de 100 no orçamento).
    pub used: f64,
    /// Quando o limite zera, em segundos desde a época Unix.
    pub resets_at: Option<u64>,
    /// Tokens usados e o máximo, no orçamento.
    pub tokens: Option<(u64, u64)>,
}

impl Limit {
    /// Porcentagem que ainda falta (0–100).
    pub fn remaining(&self) -> f64 {
        (100.0 - self.used).clamp(0.0, 100.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub source: Source,
    pub items: Vec<Limit>,
    /// Por que os limites do plano não foram usados, quando houve erro.
    pub note: Option<String>,
}

/// Limites do plano do agente quando há (ex.: login OAuth no Claude Code);
/// senão, o orçamento.
pub fn collect(adapter: &dyn Adapter, usage: &Usage, budget: &Budget) -> Limits {
    let note = match plan(adapter) {
        Ok(Some(items)) if !items.is_empty() => {
            return Limits {
                source: Source::Plan,
                items,
                note: None,
            };
        }
        Ok(_) => None,
        Err(err) => Some(format!("limites do plano indisponíveis: {err}")),
    };
    Limits {
        source: Source::Budget,
        items: budget_limits(usage, budget),
        note,
    }
}

fn budget_limits(usage: &Usage, budget: &Budget) -> Vec<Limit> {
    [
        ("Últimas 5h", budget.five_hours, &usage.all_five_hours),
        ("Últimos 7 dias", budget.week, &usage.all_week),
    ]
    .into_iter()
    .filter_map(|(label, max, summary)| {
        let max = max.filter(|&m| m > 0)?;
        let used = summary.tokens.total();
        Some(Limit {
            label: label.into(),
            used: used as f64 * 100.0 / max as f64,
            resets_at: None,
            tokens: Some((used, max)),
        })
    })
    .collect()
}

/// Resultado da última consulta ao plano e quando ela foi feita.
type PlanCache = Option<(Instant, Result<Option<Vec<Limit>>, String>)>;

static PLAN: Mutex<PlanCache> = Mutex::new(None);

/// [`Adapter::plan_limits`], guardado por `PLAN_TTL` (o terraforme roda um
/// agente só por vez).
fn plan(adapter: &dyn Adapter) -> Result<Option<Vec<Limit>>> {
    let mut cache = PLAN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, result)) = cache.as_ref()
        && at.elapsed() < PLAN_TTL
    {
        return result.clone().map_err(anyhow::Error::msg);
    }
    let result = adapter.plan_limits().map_err(|e| format!("{e:#}"));
    *cache = Some((Instant::now(), result.clone()));
    result.map_err(anyhow::Error::msg)
}

/// Limites do plano do Claude Code; `None` sem login OAuth.
pub(crate) fn fetch_claude_plan() -> Result<Option<Vec<Limit>>> {
    let path = claude_config_dir()?.join(".credentials.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    let credentials: Value =
        serde_json::from_str(&text).with_context(|| format!("lendo {}", path.display()))?;
    let Some(oauth) = credentials.get("claudeAiOauth") else {
        return Ok(None);
    };
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .context("credenciais sem accessToken")?;
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    if oauth
        .get("expiresAt")
        .and_then(Value::as_u64)
        .is_some_and(|exp| exp <= now_ms)
    {
        // Renovar aqui trocaria o refresh token do Claude Code.
        bail!("token OAuth expirado; abra o claude para renová-lo");
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let body = agent
        .get(PLAN_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(
            "User-Agent",
            concat!("terraforme/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .context("consultando o uso do plano")?
        .body_mut()
        .read_to_string()?;
    let body: Value = serde_json::from_str(&body).context("resposta inválida da API")?;
    Ok(Some(parse_plan(&body)))
}

fn parse_plan(body: &Value) -> Vec<Limit> {
    [
        ("five_hour", "Sessão · 5h"),
        ("seven_day", "Semana · todos os modelos"),
        ("seven_day_opus", "Semana · Opus"),
        ("seven_day_sonnet", "Semana · Sonnet"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        let limit = body.get(key)?;
        Some(Limit {
            label: label.into(),
            used: limit.get("utilization")?.as_f64()?,
            resets_at: limit
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(parse_timestamp),
            tokens: None,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Summary, Tokens};

    #[test]
    fn parses_plan_usage() {
        let body: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":10.0,"resets_at":"1970-01-01T01:00:00.240634+00:00"},
                "seven_day":{"utilization":48.0,"resets_at":null},
                "seven_day_opus":null}"#,
        )
        .unwrap();
        let limits = parse_plan(&body);
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].label, "Sessão · 5h");
        assert_eq!(limits[0].resets_at, Some(3600));
        assert_eq!(limits[0].remaining(), 90.0);
        assert_eq!(limits[1].used, 48.0);
        assert_eq!(limits[1].resets_at, None);
    }

    #[test]
    fn budget_compares_transcript_totals() {
        let summary = |input| Summary {
            tokens: Tokens {
                input,
                ..Tokens::default()
            },
            ..Summary::default()
        };
        let usage = Usage {
            all_five_hours: summary(250),
            all_week: summary(1_500),
            ..Usage::default()
        };
        let budget = Budget {
            five_hours: Some(1_000),
            week: Some(0),
        };
        let limits = budget_limits(&usage, &budget);
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].used, 25.0);
        assert_eq!(limits[0].tokens, Some((250, 1_000)));

        let over = budget_limits(
            &usage,
            &Budget {
                week: Some(1_000),
                ..Budget::default()
            },
        );
        assert_eq!(over[0].used, 150.0);
        assert_eq!(over[0].remaining(), 0.0);
    }
}
