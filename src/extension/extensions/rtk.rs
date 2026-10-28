//! [RTK](https://github.com/rtk-ai/rtk) (Rust Token Killer): filtra e
//! comprime a saída dos comandos antes de ela chegar ao agente.
//!
//! No agente, ela é o hook `PreToolUse` das chamadas ao Bash
//! (`rtk hook claude`), que troca o comando pelo equivalente do RTK (ex.:
//! `git status` → `rtk git status`). O relatório vem do `rtk gain`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use jiff::civil::Date;
use serde_json::Value;

use super::{find_program, run, version};
use crate::extension::{Contribution, Extension, Hook, Launcher, Report, Stat, StatValue, Status};

/// Quantos dias o gráfico mostra.
const DAYS: usize = 14;

pub struct Rtk {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    version: Option<String>,
}

impl Rtk {
    /// Procura o `rtk` no `PATH`.
    pub fn detect() -> Self {
        let bin = find_program("rtk");
        let version = bin.as_deref().and_then(version);
        Self { bin, version }
    }
}

impl Extension for Rtk {
    fn id(&self) -> &'static str {
        "rtk"
    }

    fn name(&self) -> &'static str {
        "RTK"
    }

    fn description(&self) -> &'static str {
        "Enxuga a saída dos comandos do agente (60-90% menos tokens)"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/rtk-ai/rtk"
    }

    fn status(&self) -> Status {
        match self.bin {
            Some(_) => Status::Installed {
                version: self.version.clone(),
            },
            None => Status::Missing,
        }
    }

    /// O hook roda pelo `PATH` do agente, que é o do terra. O agente também
    /// inicia pelo `rtk` (`rtk claude`), que repassa o comando.
    fn contribution(&self) -> Result<Contribution, String> {
        Ok(Contribution {
            hooks: vec![Hook {
                event: "PreToolUse".to_string(),
                matcher: "Bash".to_string(),
                command: "rtk hook claude".to_string(),
            }],
            mcp_servers: Vec::new(),
            env: Vec::new(),
            launchers: vec![Launcher {
                program: "rtk".to_string(),
                args: Vec::new(),
                fresh: Vec::new(),
            }],
        })
    }

    fn load_report(&self) -> Option<Receiver<Result<Report, String>>> {
        let bin = self.bin.clone()?;
        let version = self.version.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let today = jiff::Zoned::now().date();
            let report = run(&bin, &["gain", "--all", "--format", "json"])
                .and_then(|json| parse_gain(&json, today, version.as_deref()));
            let _ = tx.send(report);
        });
        Some(rx)
    }
}

/// O relatório a partir do `rtk gain --all --format json`, com os
/// [`DAYS`] dias até `today` no gráfico (os sem uso com 0).
fn parse_gain(json: &str, today: Date, version: Option<&str>) -> Result<Report, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|err| format!("resposta inesperada do rtk: {err}"))?;
    let summary = &value["summary"];
    let count = |key: &str| summary[key].as_u64().unwrap_or(0);
    let mut stats = vec![
        Stat::new(
            "Tokens economizados",
            StatValue::Tokens(count("total_saved")),
        ),
        Stat::new(
            "Economia média",
            StatValue::Percent(summary["avg_savings_pct"].as_f64().unwrap_or(0.0)),
        ),
        Stat::new("Comandos", StatValue::Count(count("total_commands"))),
        Stat::new("Saída original", StatValue::Tokens(count("total_input"))),
        Stat::new("Saída enxuta", StatValue::Tokens(count("total_output"))),
    ];
    if let Some(version) = version {
        stats.push(Stat::new("Versão", StatValue::Text(version.to_string())));
    }

    let saved = |date: Date| {
        value["daily"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|day| day["date"].as_str() == Some(&date.to_string()))
            .filter_map(|day| day["saved_tokens"].as_u64())
            .sum()
    };
    let daily = (0..DAYS)
        .rev()
        .filter_map(|ago| today.checked_sub(jiff::Span::new().days(ago as i64)).ok())
        .map(|date| (date, saved(date)))
        .collect();
    Ok(Report {
        stats,
        daily_title: Some("Tokens economizados por dia".to_string()),
        daily,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn contribution_is_the_bash_pre_tool_use_hook() {
        let rtk = Rtk {
            bin: Some(PathBuf::from("rtk")),
            version: Some("0.45.0".into()),
        };
        assert_eq!(
            rtk.status(),
            Status::Installed {
                version: Some("0.45.0".into())
            }
        );
        let contribution = rtk.contribution().unwrap();
        assert_eq!(
            contribution.hooks,
            [Hook {
                event: "PreToolUse".into(),
                matcher: "Bash".into(),
                command: "rtk hook claude".into(),
            }]
        );
        assert!(contribution.mcp_servers.is_empty());

        let missing = Rtk {
            bin: None,
            version: None,
        };
        assert_eq!(missing.status(), Status::Missing);
        assert!(missing.load_report().is_none());
    }

    #[test]
    fn gain_report_has_totals_and_the_last_days() {
        let json = r#"{
            "summary": {"total_commands": 8047, "total_input": 3495328, "total_output": 2020257,
                        "total_saved": 1476310, "avg_savings_pct": 42.2},
            "daily": [
                {"date": "2026-10-01", "saved_tokens": 999},
                {"date": "2026-10-26", "saved_tokens": 500},
                {"date": "2026-10-27", "saved_tokens": 1200}
            ]
        }"#;
        let report = parse_gain(json, date(2026, 10, 27), Some("0.45.0")).unwrap();
        assert_eq!(
            report.stats[0],
            Stat::new("Tokens economizados", StatValue::Tokens(1_476_310))
        );
        assert_eq!(report.stats[1].value, StatValue::Percent(42.2));
        assert_eq!(report.stats[2].value, StatValue::Count(8047));
        assert_eq!(
            report.stats.last().unwrap().value,
            StatValue::Text("0.45.0".into())
        );
        assert_eq!(report.daily.len(), DAYS);
        assert_eq!(report.daily[0], (date(2026, 10, 14), 0));
        assert_eq!(report.daily[12], (date(2026, 10, 26), 500));
        assert_eq!(report.daily[13], (date(2026, 10, 27), 1200));

        assert!(parse_gain("não é json", date(2026, 10, 27), None).is_err());
    }
}
