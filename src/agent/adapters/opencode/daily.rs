//! Tokens por dia do OpenCode, somados do banco dele
//! (`<data do OpenCode>/opencode.db`).
//!
//! Cada sessão guarda o total gasto (`tokens_input`, `tokens_output`,
//! `tokens_reasoning`, `tokens_cache_read` e `tokens_cache_write`), o modelo
//! e a pasta em que rodou. O banco não separa o gasto por dia: o dia e a hora
//! de uma sessão são os da última atividade dela (`time_updated`), então uma
//! sessão que atravessa a meia-noite entra toda no dia em que terminou.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::agent::{DailyTokens, ModelTokens, TokenReport};

/// O banco do OpenCode na pasta de dados do usuário.
pub fn default_file() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("opencode").join("opencode.db"))
}

/// Começa a somar numa thread; o resultado chega pelo canal.
pub fn load(
    file: PathBuf,
    days: usize,
    workspace: PathBuf,
) -> Receiver<Result<TokenReport, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let tz = TimeZone::system();
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let _ = tx.send(sum(&file, today, days, &tz, &workspace));
    });
    rx
}

/// Os `days` dias até `today`, o mais antigo primeiro, e os totais por modelo.
fn sum(
    file: &Path,
    today: Date,
    days: usize,
    tz: &TimeZone,
    workspace: &Path,
) -> Result<TokenReport, String> {
    let conn = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|err| format!("não foi possível abrir {}: {err}", file.display()))?;
    let first = today
        .checked_sub((days.saturating_sub(1) as i64).days())
        .unwrap_or(today);
    let since = first
        .to_zoned(tz.clone())
        .ok()
        .map(|at| at.timestamp().as_millisecond())
        .unwrap_or(0);
    let mut statement = conn
        .prepare(
            "SELECT directory, model, tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, time_updated \
             FROM session WHERE time_updated >= ?1",
        )
        .map_err(|err| err.to_string())?;
    let rows = statement
        .query_map([since], |row| {
            Ok(Session {
                directory: row.get(0).unwrap_or_default(),
                model: row.get(1).unwrap_or_default(),
                input: row.get(2).unwrap_or_default(),
                output: row.get(3).unwrap_or_default(),
                reasoning: row.get(4).unwrap_or_default(),
                cache_read: row.get(5).unwrap_or_default(),
                cache_write: row.get(6).unwrap_or_default(),
                updated: row.get(7).unwrap_or_default(),
            })
        })
        .map_err(|err| err.to_string())?;
    let mut totals: BTreeMap<Date, DailyTokens> = BTreeMap::new();
    let mut report = TokenReport::default();
    for row in rows {
        let session = row.map_err(|err| err.to_string())?;
        let Some(at) = Timestamp::from_millisecond(session.updated)
            .ok()
            .map(|at| at.to_zoned(tz.clone()))
        else {
            continue;
        };
        let date = at.date();
        if date < first {
            continue;
        }
        let tokens = positive(session.input + session.output + session.reasoning);
        let cache = positive(session.cache_read + session.cache_write);
        let hour = at.hour().clamp(0, 23) as usize;
        let day = totals
            .entry(date)
            .or_insert_with(|| DailyTokens::empty(date));
        day.tokens += tokens;
        day.cache += cache;
        day.hours[hour].tokens += tokens;
        day.hours[hour].cache += cache;
        let model = model_name(&session.model);
        ModelTokens::add(&mut report.models, &model, tokens, cache);
        if Path::new(&session.directory).starts_with(workspace) {
            ModelTokens::add(&mut report.workspace, &model, tokens, cache);
        }
    }
    report.days = (0..days)
        .filter_map(|i| first.checked_add((i as i64).days()).ok())
        .map(|date| {
            totals
                .get(&date)
                .copied()
                .unwrap_or_else(|| DailyTokens::empty(date))
        })
        .collect();
    Ok(report)
}

/// Uma linha da tabela `session`.
struct Session {
    directory: String,
    model: String,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    /// Milissegundos Unix.
    updated: i64,
}

fn positive(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

/// O id completo do modelo (`provider/modelo`), do JSON da sessão.
fn model_name(json: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return "desconhecido".to_string();
    };
    match (
        value["providerID"].as_str().filter(|id| !id.is_empty()),
        value["id"].as_str().filter(|id| !id.is_empty()),
    ) {
        (Some(provider), Some(id)) => format!("{provider}/{id}"),
        (None, Some(id)) => id.to_string(),
        _ => "desconhecido".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use rusqlite::params;

    fn database(dir: &Path) -> PathBuf {
        let file = dir.join("opencode.db");
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                directory TEXT, model TEXT,
                tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
                tokens_cache_read INTEGER, tokens_cache_write INTEGER,
                time_updated INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                "/proj/api",
                r#"{"id":"modelo","providerID":"opencode"}"#,
                100_i64,
                20,
                5,
                30,
                2,
                at(date(2026, 11, 20), 12)
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                "/outra",
                r#"{"id":"outro","providerID":"anthropic"}"#,
                1_i64,
                2,
                0,
                0,
                0,
                at(date(2026, 11, 20), 9)
            ],
        )
        .unwrap();
        file
    }

    /// O instante do dia `day` na hora `hour`, em UTC, em milissegundos.
    fn at(day: Date, hour: i8) -> i64 {
        jiff::civil::DateTime::from_parts(day, jiff::civil::time(hour, 0, 0, 0))
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn sums_the_sessions_of_the_last_days_by_model_and_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let file = database(dir.path());
        let report = sum(
            &file,
            date(2026, 11, 20),
            2,
            &TimeZone::UTC,
            Path::new("/proj"),
        )
        .unwrap();
        let days: Vec<u64> = report.days.iter().map(|day| day.tokens).collect();
        assert_eq!(days, [0, 128]);
        assert_eq!(report.days[1].hours[12].tokens, 125);
        assert_eq!(report.days[1].cache, 32);
        assert_eq!(
            report.models,
            [
                ModelTokens {
                    model: "opencode/modelo".into(),
                    tokens: 125,
                    cache: 32
                },
                ModelTokens {
                    model: "anthropic/outro".into(),
                    tokens: 3,
                    cache: 0
                },
            ]
        );
        assert_eq!(
            report.workspace,
            [ModelTokens {
                model: "opencode/modelo".into(),
                tokens: 125,
                cache: 32
            }]
        );
    }

    #[test]
    fn a_missing_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = sum(
            &dir.path().join("nao-existe.db"),
            date(2026, 11, 20),
            1,
            &TimeZone::UTC,
            Path::new("/proj"),
        )
        .unwrap_err();
        assert!(error.contains("não foi possível abrir"));
    }
}
