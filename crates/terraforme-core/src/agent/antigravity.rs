//! Integração do terraforme com o agente Antigravity (CLI: `agy`).
//!
//! O consumo é lido dos transcripts que o `agy` grava em
//! `~/.gemini/antigravity-cli/brain/<conv-id>/.system_generated/logs/transcript.jsonl`.
//! Cada resposta do modelo aparece como uma linha com `type: PLANNER_RESPONSE` e
//! os campos `input_tokens`, `output_tokens` e `cache_read_tokens`.
//!
//! A sessão corrente é identificada pelo `conversation_id` gravado em
//! `~/.gemini/antigravity-cli/cache/last_conversations.json` para o diretório
//! de trabalho atual, ou, quando não encontrado, pelo transcript modificado
//! mais recentemente desde o instante de lançamento.
//!
//! Os limites da conta vêm do próprio agente: `agy -p "/usage" --output-format
//! json` devolve, em `command.data.groups`, cada grupo de modelos com suas
//! janelas (`buckets`): quanto da janela ainda resta (`remaining_fraction`) e
//! quando ela zera (`reset_time`).
//!
//! No modo Middleware ([`crate::chat`]) o `agy` fala o mesmo `stream-json` nos
//! dois sentidos, sem `-p`: o processo abre no stdin e continua vivo entre os
//! turnos, como no terminal. Ele não tem permissão interativa — no modo print
//! as ferramentas rodam liberadas.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{Adapter, Mode, Setup, home};
use crate::chat;
use crate::limits::Quota;
use crate::usage::{Summary, Tokens, Usage, UsageSource, parse_timestamp};
use anyhow::{Context, Result, bail};
use serde_json::Value;

const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
const FIVE_HOURS: std::time::Duration = std::time::Duration::from_secs(5 * 60 * 60);

pub struct Antigravity;

impl Adapter for Antigravity {
    fn bin(&self) -> &'static str {
        "agy"
    }

    fn setup(&self, _mode: Mode) -> Result<Setup> {
        Ok(Setup::default())
    }

    fn chat_args(&self) -> Option<Vec<String>> {
        Some(chat::AGY_ARGS.iter().map(|a| a.to_string()).collect())
    }

    fn chat_input(&self, text: &str, _ctx: &chat::ChatCtx) -> String {
        chat::agy_input(text)
    }

    fn chat_events(&self, line: &str, _ctx: &chat::ChatCtx) -> Vec<chat::ChatEvent> {
        chat::agy_events(line)
    }

    /// O `agy` gerencia sessões internamente; não passamos `--session`.
    fn picks_session(&self, _args: &[String]) -> bool {
        true
    }

    fn usage(&self, cwd: &Path, launched: SystemTime, _session: Option<&str>) -> Result<Usage> {
        collect_usage(cwd, launched)
    }

    /// Os limites da conta são os que o próprio `agy` informa em
    /// `agy -p "/usage"` (janelas semanais por grupo de modelos).
    fn quota(&self) -> Result<Option<Vec<Quota>>> {
        fetch_quota().map(Some)
    }
}

/// Raiz de dados do `agy`: `$ANTIGRAVITY_APP_DATA_DIR` ou `~/.gemini/antigravity-cli`.
fn app_data_dir() -> Result<PathBuf> {
    if let Some(val) = std::env::var_os("ANTIGRAVITY_APP_DATA_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(val));
    }
    Ok(home()?.join(".gemini").join("antigravity-cli"))
}

/// Pasta dos brains de cada conversa: `<app_data>/brain`.
fn brain_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("brain"))
}

/// `conversation_id` da conversa mais recente para `cwd` em
/// `<app_data>/cache/last_conversations.json`, ou `None` se não encontrado.
fn last_conversation_id(cwd: &Path, app_data: &Path) -> Option<String> {
    let path = app_data.join("cache/last_conversations.json");
    let text = fs::read_to_string(&path).ok()?;
    let map: Value = serde_json::from_str(&text).ok()?;
    let key = cwd.to_string_lossy();
    map.get(key.as_ref())?.as_str().map(str::to_string)
}

/// Arquivo de transcript de uma conversa.
fn transcript_path(brain: &Path, conv_id: &str) -> PathBuf {
    brain
        .join(conv_id)
        .join(".system_generated/logs/transcript.jsonl")
}

/// `conversation_id` cuja transcript foi modificada mais recentemente após
/// `since` (fallback quando `last_conversations.json` não tem a entrada).
fn newest_transcript_id(brain: &Path, since: SystemTime) -> Option<String> {
    fs::read_dir(brain)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let conv_id = e.file_name().to_string_lossy().to_string();
            let transcript = transcript_path(brain, &conv_id);
            let modified = fs::metadata(&transcript).ok()?.modified().ok()?;
            (modified >= since).then_some((modified, conv_id))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, id)| id)
}

/// Itera todos os `brain/<conv-id>/.system_generated/logs/transcript.jsonl`
/// existentes e chama `f(conv_id, path)` para cada um encontrado.
fn iter_transcripts(brain: &Path, mut f: impl FnMut(&str, &Path)) {
    let Ok(entries) = fs::read_dir(brain) else {
        return;
    };
    for entry in entries.flatten() {
        let conv_id = entry.file_name().to_string_lossy().to_string();
        let path = transcript_path(brain, &conv_id);
        if path.is_file() {
            f(&conv_id, &path);
        }
    }
}

/// Lê o transcript de uma conversa e devolve um `Summary` com os tokens
/// de todas as respostas do modelo com `created_at` >= `since_secs`.
fn read_transcript_since(path: &Path, since_secs: u64) -> Summary {
    let Ok(file) = fs::File::open(path) else {
        return Summary::default();
    };
    let mut summary = Summary::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some((tokens, ts)) = parse_transcript_line(&line)
            && ts >= since_secs
        {
            summary.tokens.add(&tokens);
            summary.responses += 1;
        }
    }
    summary
}

/// Lê o transcript e devolve tokens agrupados por modelo com timestamp >= since_secs.
fn read_transcript_by_model(path: &Path, since_secs: u64) -> (Summary, BTreeMap<String, Tokens>) {
    let Ok(file) = fs::File::open(path) else {
        return (Summary::default(), BTreeMap::new());
    };
    let mut summary = Summary::default();
    let mut by_model: BTreeMap<String, Tokens> = BTreeMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some((tokens, ts)) = parse_transcript_line(&line)
            && ts >= since_secs
        {
            summary.tokens.add(&tokens);
            summary.responses += 1;
            let model = extract_model(&line).unwrap_or_default();
            by_model.entry(model).or_default().add(&tokens);
        }
    }
    summary.by_model = by_model.clone();
    (summary, by_model)
}

/// `PLANNER_RESPONSE` com tokens → `(Tokens, timestamp_secs)`.
fn parse_transcript_line(line: &str) -> Option<(Tokens, u64)> {
    if !line.contains("input_tokens") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    // Só linhas de resposta do modelo têm os campos de tokens.
    let tp = v.get("type")?.as_str()?;
    if tp != "PLANNER_RESPONSE" {
        return None;
    }
    let field = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let tokens = Tokens {
        input: field("input_tokens"),
        output: field("output_tokens"),
        reasoning: 0,
        cache_write: 0,
        cache_read: field("cache_read_tokens"),
    };
    if tokens.total() == 0 {
        return None;
    }
    let ts = v.get("created_at")?.as_str()?;
    let secs = parse_timestamp(ts)?;
    Some((tokens, secs))
}

/// Extrai o nome do modelo de um modelo fictício baseado no endpoint da API.
/// Os transcripts do `agy` não têm campo de modelo — usamos uma string genérica.
fn extract_model(line: &str) -> Option<String> {
    // Os transcripts do agy atualmente não expõem o modelo por linha;
    // devolvemos None para não poluir o `by_model` com uma chave vazia.
    let v: Value = serde_json::from_str(line).ok()?;
    v.get("model")?.as_str().map(str::to_string)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Coleta o consumo do Antigravity lendo os transcripts do `agy`.
fn collect_usage(cwd: &Path, launched: SystemTime) -> Result<Usage> {
    let app_data = app_data_dir().context("localizando pasta de dados do agy")?;
    let brain = brain_dir().context("localizando brain do agy")?;

    let now = SystemTime::now();
    let week_ago = secs(now.checked_sub(7 * DAY).unwrap_or(std::time::UNIX_EPOCH));
    let day_ago = secs(now.checked_sub(DAY).unwrap_or(std::time::UNIX_EPOCH));
    let five_hours_ago = secs(now.checked_sub(FIVE_HOURS).unwrap_or(std::time::UNIX_EPOCH));

    // Conversa da sessão atual.
    let session_conv =
        last_conversation_id(cwd, &app_data).or_else(|| newest_transcript_id(&brain, launched));

    let mut usage = Usage {
        source: UsageSource::Antigravity,
        ..Usage::default()
    };

    // Lê todos os transcripts para os totais globais.
    iter_transcripts(&brain, |conv_id, path| {
        // Otimização: ignora arquivos não modificados nos últimos 7 dias.
        if fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|m| secs(m) < week_ago)
            .unwrap_or(false)
        {
            return;
        }
        let is_session = session_conv.as_deref() == Some(conv_id);
        let (week_summary, _) = read_transcript_by_model(path, week_ago);
        let day_summary = read_transcript_since(path, day_ago);
        let five_h_summary = read_transcript_since(path, five_hours_ago);

        usage.all_week.add_summary(&week_summary);
        usage.all_day.add_summary(&day_summary);
        usage.all_five_hours.add_summary(&five_h_summary);

        if is_session {
            let (session_summary, _) = read_transcript_by_model(path, 0);
            usage.session = Some(session_summary);
            usage.project_day = day_summary;
        }
    });

    Ok(usage)
}

/// Limites da conta segundo o próprio agente: `agy -p "/usage" --output-format
/// json` é o `/usage` da interface do `agy` e responde sem chamar o modelo.
fn fetch_quota() -> Result<Vec<Quota>> {
    let out = crate::status::command("agy")
        .args([
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--print-timeout",
            "30s",
        ])
        .output()
        .context("rodando o agy")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("agy -p /usage falhou: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let body: Value = serde_json::from_str(stdout.trim()).context("resposta inválida do agy")?;
    if body.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        let response = body
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default();
        bail!("agy -p /usage não completou: {}", response.trim());
    }
    Ok(parse_quota(&body))
}

/// Cada `bucket` de cada grupo de modelos do `/usage` vira uma [`Quota`]: a
/// porcentagem já usada sai do `remaining_fraction` e o instante em que a
/// janela zera vem no `reset_time` (UTC).
fn parse_quota(body: &Value) -> Vec<Quota> {
    let Some(groups) = body
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut quota = Vec::new();
    for group in groups {
        let group_name = group
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let (Some(name), Some(remaining)) = (
                bucket.get("name").and_then(Value::as_str),
                bucket.get("remaining_fraction").and_then(Value::as_f64),
            ) else {
                continue;
            };
            quota.push(Quota {
                label: if group_name.is_empty() {
                    name.into()
                } else {
                    format!("{group_name} · {name}")
                },
                used: Some(((1.0 - remaining) * 100.0).clamp(0.0, 100.0)),
                tokens: None,
                resets_at: bucket
                    .get("reset_time")
                    .and_then(Value::as_str)
                    .and_then(parse_timestamp),
            });
        }
    }
    quota
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, input: u64, output: u64, cache_read: u64) -> String {
        format!(
            r#"{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"{ts}","input_tokens":{input},"cache_read_tokens":{cache_read},"output_tokens":{output}}}"#,
        )
    }

    #[test]
    fn parse_transcript_line_works() {
        let l = line("1970-01-01T00:00:10Z", 100, 20, 50);
        let (tokens, ts) = parse_transcript_line(&l).unwrap();
        assert_eq!(ts, 10);
        assert_eq!(tokens.input, 100);
        assert_eq!(tokens.output, 20);
        assert_eq!(tokens.cache_read, 50);
        assert_eq!(tokens.total(), 170);
    }

    #[test]
    fn ignores_non_planner_lines() {
        let l = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"1970-01-01T00:00:01Z","input_tokens":10,"output_tokens":5}"#;
        assert!(parse_transcript_line(l).is_none());
    }

    #[test]
    fn ignores_zero_token_lines() {
        let l = r#"{"type":"PLANNER_RESPONSE","created_at":"1970-01-01T00:00:01Z","input_tokens":0,"output_tokens":0,"cache_read_tokens":0}"#;
        assert!(parse_transcript_line(l).is_none());
    }

    #[test]
    fn parses_usage_command_quota_with_reset_times() {
        let body: Value = serde_json::from_str(
            r#"{"status":"SUCCESS","response":"…","command":{"name":"usage","data":{
                "description":"Within each group, models share a weekly limit.",
                "groups":[
                    {"name":"Gemini Models","description":"…","buckets":[
                        {"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly",
                         "remaining_fraction":0,"reset_time":"1970-01-01T05:00:00Z"}]},
                    {"name":"Claude and GPT models","buckets":[
                        {"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly",
                         "remaining_fraction":0.30665600299835205,"reset_time":"1970-01-01T05:10:00Z"}]},
                    {"name":"Sem buckets"}
                ]}}}"#,
        )
        .unwrap();
        let quota = parse_quota(&body);
        assert_eq!(quota.len(), 2);
        assert_eq!(quota[0].label, "Gemini Models · Weekly Limit Remaining");
        assert_eq!(quota[0].used, Some(100.0));
        assert_eq!(quota[0].resets_at, Some(18_000));
        assert_eq!(
            quota[1].label,
            "Claude and GPT models · Weekly Limit Remaining"
        );
        assert!((quota[1].used.unwrap() - 69.334_399_700_164_8).abs() < 1e-9);
        assert_eq!(quota[1].resets_at, Some(18_600));
        assert_eq!(quota[0].tokens, None);

        assert!(parse_quota(&serde_json::json!({"status": "SUCCESS"})).is_empty());
    }
}
