//! Tokens por dia da conta do Claude, somados dos transcripts que o Claude
//! Code grava em `<pasta da conta>/projects/**/*.jsonl` (o terra também usa o
//! `claude`, então as conversas dele entram).
//!
//! Cada resposta do modelo é uma linha `assistant` com o `usage`; uma
//! resposta com vários blocos se repete em várias linhas, com o mesmo
//! `message.id` e `requestId`, e conta uma vez só. Cada dia guarda a entrada
//! e a saída separadas do cache (criação e leitura), para o gráfico mostrar
//! com e sem cache.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::SystemTime;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::Value;

use crate::agent::DailyTokens;

const PROJECTS: &str = "projects";
const TOKEN_KEYS: [&str; 2] = ["input_tokens", "output_tokens"];
const CACHE_KEYS: [&str; 2] = ["cache_creation_input_tokens", "cache_read_input_tokens"];

/// Começa a somar numa thread; o resultado chega pelo canal.
pub fn load(config_dir: PathBuf, days: usize) -> Receiver<Result<Vec<DailyTokens>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let tz = TimeZone::system();
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let _ = tx.send(Ok(sum(&config_dir.join(PROJECTS), today, days, &tz)));
    });
    rx
}

/// Os `days` dias até `today`, o mais antigo primeiro.
fn sum(projects: &Path, today: Date, days: usize, tz: &TimeZone) -> Vec<DailyTokens> {
    let first = today
        .checked_sub((days.saturating_sub(1) as i64).days())
        .unwrap_or(today);
    let mut totals: BTreeMap<Date, DailyTokens> = BTreeMap::new();
    let mut seen = HashSet::new();
    // Arquivos sem mudança desde antes do primeiro dia não têm nada dele.
    let since = first
        .to_zoned(tz.clone())
        .ok()
        .map(|z| SystemTime::from(z.timestamp()));
    for file in jsonl_files(projects) {
        let recent = fs::metadata(&file)
            .and_then(|m| m.modified())
            .map_or(true, |modified| since.is_none_or(|since| modified >= since));
        if recent {
            add_file(&file, first, tz, &mut seen, &mut totals);
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

fn add_file(
    file: &Path,
    first: Date,
    tz: &TimeZone,
    seen: &mut HashSet<String>,
    totals: &mut BTreeMap<Date, DailyTokens>,
) {
    let Ok(file) = fs::File::open(file) else {
        return;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // A maioria das linhas não é resposta do modelo: nem vale ler o JSON.
        if !line.contains("\"usage\"") {
            continue;
        }
        if let Some(reply) = parse_line(&line, tz, seen)
            && reply.date >= first
        {
            let day = totals
                .entry(reply.date)
                .or_insert_with(|| DailyTokens::empty(reply.date));
            day.tokens += reply.tokens;
            day.cache += reply.cache;
            let hour = &mut day.hours[reply.hour];
            hour.tokens += reply.tokens;
            hour.cache += reply.cache;
        }
    }
}

/// Uma resposta do modelo: quando (no fuso local) e quanto gastou.
struct Reply {
    date: Date,
    /// 0 a 23.
    hour: usize,
    tokens: u64,
    cache: u64,
}

/// A resposta de uma linha, se ela for uma que ainda não foi contada.
fn parse_line(line: &str, tz: &TimeZone, seen: &mut HashSet<String>) -> Option<Reply> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["type"] != "assistant" {
        return None;
    }
    let message = &value["message"];
    let usage = message["usage"].as_object()?;
    let key = format!(
        "{}:{}",
        message["id"].as_str().unwrap_or_default(),
        value["requestId"].as_str().unwrap_or_default()
    );
    if key != ":" && !seen.insert(key) {
        return None;
    }
    let at = value["timestamp"]
        .as_str()?
        .parse::<Timestamp>()
        .ok()?
        .to_zoned(tz.clone());
    let sum = |keys: &[&str]| -> u64 {
        keys.iter()
            .filter_map(|key| usage.get(*key).and_then(Value::as_u64))
            .sum()
    };
    Some(Reply {
        date: at.date(),
        hour: at.hour() as usize,
        tokens: sum(&TOKEN_KEYS),
        cache: sum(&CACHE_KEYS),
    })
}

/// Os `.jsonl` dentro de `dir`, em qualquer profundidade.
fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") => {
                    files.push(path)
                }
                _ => {}
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, request: &str, at: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{request}","timestamp":"{at}","message":{{"id":"{id}","usage":{{"input_tokens":1,"output_tokens":{output},"cache_creation_input_tokens":10,"cache_read_input_tokens":100}}}}}}"#
        )
    }

    #[test]
    fn sums_tokens_per_local_day_counting_each_reply_once() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects").join("-home-eu-repo");
        fs::create_dir_all(project.join("subagents")).unwrap();
        let tz = TimeZone::fixed(jiff::tz::offset(-3));
        fs::write(
            project.join("a.jsonl"),
            [
                // 26/10 às 22:00 em UTC-3.
                line("m1", "r1", "2026-10-27T01:00:00Z", 9),
                // A mesma resposta em outra linha (outro bloco): conta uma vez.
                line("m1", "r1", "2026-10-27T01:00:00Z", 9),
                r#"{"type":"user","message":{"content":"oi"}}"#.to_string(),
                "não é json com \"usage\"".to_string(),
                line("m2", "r2", "2026-10-27T15:00:00Z", 89),
                // Antes do primeiro dia: fica de fora.
                line("m0", "r0", "2026-10-20T15:00:00Z", 1000),
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(
            project.join("subagents").join("b.jsonl"),
            line("m3", "r3", "2026-10-27T16:00:00Z", 189),
        )
        .unwrap();
        fs::write(
            project.join("notas.txt"),
            line("m4", "r4", "2026-10-27T16:00:00Z", 5),
        )
        .unwrap();

        let today = jiff::civil::date(2026, 10, 27);
        let days = sum(&dir.path().join("projects"), today, 3, &tz);
        let found: Vec<_> = days
            .iter()
            .map(|d| (d.date.day(), d.tokens, d.cache))
            .collect();
        // Entrada 1 + saída; cache 10 + 100 por resposta.
        assert_eq!(found, [(25, 0, 0), (26, 10, 110), (27, 90 + 190, 220)]);
        // Por hora, no fuso local: 26/10 às 22h; 27/10 às 12h e às 13h.
        let hours = |day: &DailyTokens| -> Vec<(usize, u64)> {
            (0..24)
                .filter(|&h| day.hours[h].tokens > 0)
                .map(|h| (h, day.hours[h].tokens))
                .collect()
        };
        assert_eq!(hours(&days[1]), [(22, 10)]);
        assert_eq!(hours(&days[2]), [(12, 90), (13, 190)]);
        assert_eq!(days[2].hours[13].cache, 110);
    }

    #[test]
    fn missing_folder_gives_empty_days() {
        let dir = tempfile::tempdir().unwrap();
        let days = sum(
            &dir.path().join("projects"),
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
}
