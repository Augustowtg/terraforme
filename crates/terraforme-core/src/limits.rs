//! Quanto falta dos limites de uso do agente. Os limites chegam pelo
//! [`Adapter::quota`], no formato padronizado do terraforme ([`Quota`]):
//! cada CLI traduz o que o dele informa (a API OAuth do Claude Code, o
//! `opencode stats`…) e o cálculo de quanto falta é sempre deste módulo.
//! Sem limites de plano — ou se a consulta falhar —, o consumo dos
//! transcripts é comparado com o orçamento de `[budget]` do config.toml.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::agent::Adapter;
use crate::usage::{Usage, UsageSource};

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
    /// Tokens usados e o limite total, quando há (orçamento ou o próprio
    /// provedor).
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

/// Informação padronizada de um limite que o adapter de um CLI devolve ao
/// terraforme ([`Adapter::quota`]): o adapter só traduz o que o dele informa
/// (tokens usados, limite total, quando zera), e o terraforme calcula o
/// quanto falta e desenha a barra.
#[derive(Debug, Clone, PartialEq)]
pub struct Quota {
    /// Janela do limite (ex.: `Sessão · 5h`).
    pub label: String,
    /// Porcentagem já usada, calculada pelo próprio provedor (0–100);
    /// `None` quando ela deve sair de [`Quota::tokens`].
    pub used: Option<f64>,
    /// Tokens usados e o limite total da janela, quando o provedor informa.
    pub tokens: Option<(u64, u64)>,
    /// Quando o limite zera, em segundos desde a época Unix.
    pub resets_at: Option<u64>,
}

impl From<Quota> for Limit {
    fn from(quota: Quota) -> Self {
        let used = quota.used.unwrap_or_else(|| match quota.tokens {
            Some((used, max)) if max > 0 => used as f64 * 100.0 / max as f64,
            _ => 0.0,
        });
        Limit {
            label: quota.label,
            used,
            resets_at: quota.resets_at,
            tokens: quota.tokens,
        }
    }
}

/// Limites do plano do agente quando há (ex.: login OAuth no Claude Code);
/// senão, o orçamento.
pub fn collect(adapter: &dyn Adapter, usage: &Usage, budget: &Budget) -> Limits {
    let note = match quotas(adapter) {
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
        (
            "Últimas 5h",
            (usage.source != UsageSource::OpenCode)
                .then_some(budget.five_hours)
                .flatten(),
            &usage.all_five_hours,
        ),
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

/// [`Adapter::quota`], guardado por `PLAN_TTL` (o terraforme roda um agente
/// só por vez) e convertido nos [`Limit`] prontos para a interface.
fn quotas(adapter: &dyn Adapter) -> Result<Option<Vec<Limit>>> {
    let mut cache = PLAN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, result)) = cache.as_ref()
        && at.elapsed() < PLAN_TTL
    {
        return result.clone().map_err(anyhow::Error::msg);
    }
    let result = adapter
        .quota()
        .map(|quota| quota.map(|items| items.into_iter().map(Limit::from).collect()))
        .map_err(|e| format!("{e:#}"));
    *cache = Some((Instant::now(), result.clone()));
    result.map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Summary, Tokens};

    #[test]
    fn quota_becomes_a_limit_with_tokens_or_a_provider_percentage() {
        // O provedor já calculou a porcentagem (ex.: Claude Code).
        let from_api = Limit::from(Quota {
            label: "Sessão · 5h".into(),
            used: Some(10.0),
            tokens: None,
            resets_at: Some(3600),
        });
        assert_eq!(from_api.used, 10.0);
        assert_eq!(from_api.remaining(), 90.0);
        assert_eq!(from_api.resets_at, Some(3600));

        // O terraforme calcula a porcentagem dos tokens informados.
        let from_tokens = Limit::from(Quota {
            label: "Últimos 7 dias".into(),
            used: None,
            tokens: Some((250, 1_000)),
            resets_at: None,
        });
        assert_eq!(from_tokens.used, 25.0);
        assert_eq!(from_tokens.tokens, Some((250, 1_000)));

        // Sem tokens e sem porcentagem não há usage para mostrar.
        let empty = Limit::from(Quota {
            label: "vazio".into(),
            used: None,
            tokens: None,
            resets_at: None,
        });
        assert_eq!(empty.used, 0.0);
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

    #[test]
    fn opencode_budget_omits_the_unavailable_five_hour_window() {
        let usage = Usage {
            source: UsageSource::OpenCode,
            all_five_hours: Summary {
                tokens: Tokens {
                    input: 500,
                    ..Tokens::default()
                },
                ..Summary::default()
            },
            all_week: Summary {
                tokens: Tokens {
                    input: 2_000,
                    ..Tokens::default()
                },
                ..Summary::default()
            },
            ..Usage::default()
        };
        let budget = Budget {
            five_hours: Some(1_000),
            week: Some(10_000),
        };

        let limits = budget_limits(&usage, &budget);
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].label, "Últimos 7 dias");
        assert_eq!(limits[0].tokens, Some((2_000, 10_000)));
    }
}
