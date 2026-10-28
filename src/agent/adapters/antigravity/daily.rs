//! Tokens por dia do Antigravity, somados de um registro do próprio terra.
//!
//! As conversas do `agy` ficam em bancos SQLite (`~/.gemini/antigravity-cli/
//! conversations/<id>.db`, com WAL) cujas linhas são protobuf sem esquema
//! público: ler daria um decodificador adivinhado, que quebra a cada versão.
//! O que o terra tem de confiável é o `usage` de cada passo do stream, então
//! cada chamada ao modelo que o terra vê vira uma linha de
//! `~/.terraforme/antigravity-usage.jsonl`:
//!
//! ```json
//! {"at":1793000000,"conversation":"c1","step":2,"model":"gemini-3.8-flash-low","tokens":8000,"cache":5}
//! ```
//!
//! `tokens` é a entrada mais a saída, e `cache` a leitura de cache (como nos
//! badges do chat). Fica de fora o que o `agy` gasta fora do terra (a
//! interface original e o uso anterior ao registro). Uma chamada repetida
//! (mesma conversa e passo) conta uma vez.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::{Value, json};

use crate::agent::DailyTokens;

const FILE: &str = "antigravity-usage.jsonl";

/// O registro padrão: `~/.terraforme/antigravity-usage.jsonl`.
pub fn default_file() -> Option<PathBuf> {
    crate::db::data_dir().ok().map(|dir| dir.join(FILE))
}

/// O gasto de uma chamada ao modelo, como o stream a informou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spend {
    pub conversation: String,
    pub step: u64,
    pub model: String,
    /// Entrada e saída.
    pub tokens: u64,
    /// Leitura de cache.
    pub cache: u64,
}

/// Acrescenta os gastos ao registro, com a hora de agora. Erros de gravação
/// são ignorados: o consumo é só informativo, e não pode derrubar o chat.
pub fn record(file: &Path, spends: &[Spend]) {
    record_at(file, spends, Timestamp::now().as_second());
}

fn record_at(file: &Path, spends: &[Spend], at: i64) {
    if spends.is_empty() {
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // Uma só gravação por chamada: com `O_APPEND`, vários chats ao mesmo
    // tempo não misturam as linhas.
    let lines: String = spends
        .iter()
        .map(|s| {
            let line = json!({
                "at": at,
                "conversation": s.conversation,
                "step": s.step,
                "model": s.model,
                "tokens": s.tokens,
                "cache": s.cache,
            });
            format!("{line}\n")
        })
        .collect();
    if let Ok(mut out) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = out.write_all(lines.as_bytes());
    }
}

/// Começa a somar numa thread; o resultado chega pelo canal.
pub fn load(file: PathBuf, days: usize) -> Receiver<Result<Vec<DailyTokens>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let tz = TimeZone::system();
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let _ = tx.send(Ok(sum(&file, today, days, &tz)));
    });
    rx
}

/// Os `days` dias até `today`, o mais antigo primeiro.
fn sum(file: &Path, today: Date, days: usize, tz: &TimeZone) -> Vec<DailyTokens> {
    let first = today
        .checked_sub((days.saturating_sub(1) as i64).days())
        .unwrap_or(today);
    let mut totals: BTreeMap<Date, DailyTokens> = BTreeMap::new();
    let mut seen = HashSet::new();
    if let Ok(file) = fs::File::open(file) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Some(entry) = parse_line(&line, tz, &mut seen) else {
                continue;
            };
            if entry.date < first {
                continue;
            }
            let day = totals
                .entry(entry.date)
                .or_insert_with(|| DailyTokens::empty(entry.date));
            day.tokens += entry.tokens;
            day.cache += entry.cache;
            let hour = &mut day.hours[entry.hour];
            hour.tokens += entry.tokens;
            hour.cache += entry.cache;
        }
    }
    (0..days)
        .filter_map(|i| first.checked_add((i as i64).days()).ok())
        .map(|date| {
            totals
                .get(&date)
                .copied()
                .unwrap_or_else(|| DailyTokens::empty(date))
        })
        .collect()
}

struct Entry {
    date: Date,
    /// 0 a 23.
    hour: usize,
    tokens: u64,
    cache: u64,
}

/// A chamada de uma linha, se ela for válida e ainda não tiver sido contada.
/// Linhas quebradas (ex.: gravação cortada) são ignoradas.
fn parse_line(line: &str, tz: &TimeZone, seen: &mut HashSet<(String, u64)>) -> Option<Entry> {
    let value: Value = serde_json::from_str(line).ok()?;
    let at = Timestamp::from_second(value["at"].as_i64()?)
        .ok()?
        .to_zoned(tz.clone());
    let key = (
        value["conversation"].as_str()?.to_string(),
        value["step"].as_u64()?,
    );
    if !seen.insert(key) {
        return None;
    }
    Some(Entry {
        date: at.date(),
        hour: at.hour() as usize,
        tokens: value["tokens"].as_u64().unwrap_or_default(),
        cache: value["cache"].as_u64().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spend(conversation: &str, step: u64, tokens: u64, cache: u64) -> Spend {
        Spend {
            conversation: conversation.into(),
            step,
            model: "gemini-3.8-flash-low".into(),
            tokens,
            cache,
        }
    }

    fn at(rfc3339: &str) -> i64 {
        rfc3339.parse::<Timestamp>().unwrap().as_second()
    }

    #[test]
    fn sums_per_local_day_and_hour_counting_each_step_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("antigravity-usage.jsonl");
        let tz = TimeZone::fixed(jiff::tz::offset(-3));
        // 26/10 às 22h em UTC-3.
        record_at(&file, &[spend("c1", 2, 10, 1)], at("2026-10-27T01:00:00Z"));
        // A mesma chamada gravada de novo: conta uma vez.
        record_at(&file, &[spend("c1", 2, 10, 1)], at("2026-10-27T01:00:00Z"));
        // 27/10 às 12h e às 13h; dois passos de uma só gravação.
        record_at(
            &file,
            &[spend("c1", 6, 90, 5), spend("c2", 2, 190, 0)],
            at("2026-10-27T15:00:00Z"),
        );
        record_at(&file, &[spend("c2", 6, 100, 7)], at("2026-10-27T16:00:00Z"));
        // Antes do primeiro dia: fica de fora.
        record_at(
            &file,
            &[spend("c0", 2, 1000, 0)],
            at("2026-10-20T15:00:00Z"),
        );

        let days = sum(&file, jiff::civil::date(2026, 10, 27), 3, &tz);
        let found: Vec<_> = days
            .iter()
            .map(|d| (d.date.day(), d.tokens, d.cache))
            .collect();
        assert_eq!(found, [(25, 0, 0), (26, 10, 1), (27, 380, 12)]);
        let hours = |day: &DailyTokens| -> Vec<(usize, u64, u64)> {
            (0..24)
                .filter(|&h| day.hours[h].tokens > 0)
                .map(|h| (h, day.hours[h].tokens, day.hours[h].cache))
                .collect()
        };
        assert_eq!(hours(&days[1]), [(22, 10, 1)]);
        assert_eq!(hours(&days[2]), [(12, 280, 5), (13, 100, 7)]);
    }

    #[test]
    fn broken_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("antigravity-usage.jsonl");
        record_at(&file, &[spend("c1", 2, 10, 0)], at("2026-10-27T12:00:00Z"));
        let mut out = OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(out, "{{\"at\":17930").unwrap();
        writeln!(out, "não é json").unwrap();
        writeln!(out, r#"{{"conversation":"c9","step":1,"tokens":5}}"#).unwrap();
        drop(out);
        record_at(&file, &[spend("c1", 4, 20, 0)], at("2026-10-27T12:30:00Z"));

        let days = sum(&file, jiff::civil::date(2026, 10, 27), 1, &TimeZone::UTC);
        assert_eq!(days[0].tokens, 30);
    }

    #[test]
    fn missing_file_gives_empty_days() {
        let dir = tempfile::tempdir().unwrap();
        let days = sum(
            &dir.path().join("nada.jsonl"),
            jiff::civil::date(2026, 1, 2),
            2,
            &TimeZone::UTC,
        );
        assert_eq!(
            days,
            [
                DailyTokens::empty(jiff::civil::date(2026, 1, 1)),
                DailyTokens::empty(jiff::civil::date(2026, 1, 2)),
            ]
        );
    }

    #[test]
    fn nothing_to_record_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("antigravity-usage.jsonl");
        record_at(&file, &[], 0);
        assert!(!file.exists());
    }
}
