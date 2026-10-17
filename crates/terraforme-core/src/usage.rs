//! Consumo de tokens do Claude Code, lido dos transcripts que ele grava em
//! `~/.claude/projects/<projeto>/<sessão>.jsonl`. Cada resposta do modelo traz
//! um `message.usage`; como uma mesma resposta aparece em várias linhas (uma
//! por bloco de conteúdo), as linhas são agrupadas por `message.id` + `requestId`.
//! Subagentes gravam em `<sessão>/subagents/agent-<id>.jsonl`, com o mesmo
//! `sessionId` da sessão que os chamou e o próprio `agentId`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde_json::Value;

use crate::agent::claude::claude_config_dir;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const FIVE_HOURS: Duration = Duration::from_secs(5 * 60 * 60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// Soma de um conjunto de respostas do modelo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub tokens: Tokens,
    /// Respostas do modelo (chamadas à API).
    pub responses: usize,
    pub by_model: BTreeMap<String, Tokens>,
}

impl Summary {
    fn add(&mut self, entry: &Entry) {
        self.tokens.add(&entry.tokens);
        self.responses += 1;
        self.by_model
            .entry(entry.model.clone())
            .or_default()
            .add(&entry.tokens);
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// Sessão aberta por esta execução; `None` enquanto ela não tem respostas.
    pub session: Option<Summary>,
    /// A sessão dividida entre o agente principal (primeiro) e os subagentes,
    /// do que mais consumiu para o que menos consumiu.
    pub session_agents: Vec<AgentUsage>,
    pub project_day: Summary,
    pub all_five_hours: Summary,
    pub all_day: Summary,
    pub all_week: Summary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentUsage {
    /// `None` no agente principal.
    pub id: Option<String>,
    /// Tipo do subagente (ex.: `Explore`), do `.meta.json` dele.
    pub kind: Option<String>,
    pub description: Option<String>,
    pub summary: Summary,
}

#[derive(Debug, Clone)]
struct Entry {
    session: String,
    /// `agentId` do subagente; `None` no agente principal.
    agent: Option<String>,
    model: String,
    /// Segundos desde a época Unix.
    time: u64,
    tokens: Tokens,
}

/// Junta o consumo da sessão `session` (sem ela, a iniciada em `launched`: a
/// transcript mais recente do projeto de `cwd` modificada desde então), do
/// projeto nas últimas 24h e de todos os projetos nas últimas 5h, 24h e 7 dias.
pub fn collect(cwd: &Path, launched: SystemTime, session: Option<&str>) -> Result<Usage> {
    let projects = claude_config_dir()?.join("projects");
    let project = projects.join(project_dir_name(cwd));
    let now = SystemTime::now();
    let week_ago = now.checked_sub(7 * DAY).unwrap_or(UNIX_EPOCH);
    let day_ago = secs(now.checked_sub(DAY).unwrap_or(UNIX_EPOCH));
    let five_hours_ago = secs(now.checked_sub(FIVE_HOURS).unwrap_or(UNIX_EPOCH));

    let session = match session {
        Some(id) => Some(id.to_string()),
        None => newest_transcript(&project, launched),
    };

    let mut usage = Usage::default();
    let mut session_summary = Summary::default();
    let mut agents: HashMap<Option<String>, Summary> = HashMap::new();
    for dir in read_dirs(&projects) {
        let in_project = dir == project;
        for entry in read_transcripts(&dir, week_ago) {
            usage.all_week.add(&entry);
            if entry.time >= day_ago {
                usage.all_day.add(&entry);
                if in_project {
                    usage.project_day.add(&entry);
                }
            }
            if entry.time >= five_hours_ago {
                usage.all_five_hours.add(&entry);
            }
            if in_project && session.as_deref() == Some(entry.session.as_str()) {
                session_summary.add(&entry);
                agents.entry(entry.agent.clone()).or_default().add(&entry);
            }
        }
    }
    if let Some(session) = &session {
        let subagents = project.join(session).join("subagents");
        usage.session_agents = session_agents(agents, &subagents);
    }
    usage.session = (session_summary.responses > 0).then_some(session_summary);
    Ok(usage)
}

/// Principal primeiro; subagentes por consumo, com tipo e descrição lidos de
/// `subagents/agent-<id>.meta.json`.
fn session_agents(agents: HashMap<Option<String>, Summary>, subagents: &Path) -> Vec<AgentUsage> {
    let mut list: Vec<AgentUsage> = agents
        .into_iter()
        .map(|(id, summary)| {
            let meta = id
                .as_ref()
                .and_then(|id| {
                    fs::read_to_string(subagents.join(format!("agent-{id}.meta.json"))).ok()
                })
                .and_then(|text| serde_json::from_str::<Value>(&text).ok());
            let field = |name: &str| {
                meta.as_ref()?
                    .get(name)?
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            AgentUsage {
                kind: field("agentType"),
                description: field("description"),
                id,
                summary,
            }
        })
        .collect();
    list.sort_by(|a, b| {
        (a.id.is_some(), std::cmp::Reverse(a.summary.tokens.total()))
            .cmp(&(b.id.is_some(), std::cmp::Reverse(b.summary.tokens.total())))
    });
    list
}

/// Pasta em que o Claude Code guarda as sessões de `cwd`: o caminho com tudo
/// que não for letra ou número trocado por `-`.
fn project_dir_name(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Id da sessão (nome do arquivo) mais recente modificada desde `since`.
fn newest_transcript(project: &Path, since: SystemTime) -> Option<String> {
    fs::read_dir(project)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().ok()?.modified().ok()?;
            (path.extension()? == "jsonl" && modified >= since).then_some((modified, path))
        })
        .max()
        .and_then(|(_, path)| Some(path.file_stem()?.to_string_lossy().into_owned()))
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// Respostas de todas as transcripts do projeto (inclusive as de subagentes,
/// em subpastas) modificadas desde `since`.
fn read_transcripts(project: &Path, since: SystemTime) -> Vec<Entry> {
    let mut files = Vec::new();
    find_jsonl(project, since, &mut files);

    let mut entries: HashMap<String, Entry> = HashMap::new();
    for file in files {
        let Ok(file) = fs::File::open(&file) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some((key, entry)) = parse_line(&line) {
                // A última linha da resposta traz o uso final.
                entries.insert(key, entry);
            }
        }
    }
    let since = secs(since);
    entries.into_values().filter(|e| e.time >= since).collect()
}

fn find_jsonl(dir: &Path, since: SystemTime, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_jsonl(&path, since, out);
        } else if path.extension().is_some_and(|e| e == "jsonl")
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= since)
        {
            out.push(path);
        }
    }
}

fn parse_line(line: &str) -> Option<(String, Entry)> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    let model = message.get("model")?.as_str()?;
    if model.starts_with('<') {
        // `<synthetic>`: mensagens geradas pelo próprio Claude Code, sem custo.
        return None;
    }
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    let key = format!(
        "{}:{}",
        message.get("id").and_then(Value::as_str).unwrap_or(""),
        value.get("requestId").and_then(Value::as_str).unwrap_or("")
    );
    Some((
        key,
        Entry {
            session: value.get("sessionId")?.as_str()?.to_string(),
            agent: value
                .get("agentId")
                .and_then(Value::as_str)
                .map(str::to_string),
            model: model.to_string(),
            time: parse_timestamp(value.get("timestamp")?.as_str()?)?,
            tokens: Tokens {
                input: field("input_tokens"),
                output: field("output_tokens"),
                cache_write: field("cache_creation_input_tokens"),
                cache_read: field("cache_read_input_tokens"),
            },
        },
    ))
}

fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `2026-10-17T02:45:14.612Z` → segundos desde a época Unix (UTC).
pub(crate) fn parse_timestamp(ts: &str) -> Option<u64> {
    let num = |range: std::ops::Range<usize>| ts.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, min, s) = (num(11..13)?, num(14..16)?, num(17..19)?);

    // Dias desde 1970-01-01 (algoritmo "days from civil", de Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_name_replaces_non_alphanumerics() {
        assert_eq!(
            project_dir_name(Path::new("/home/a/kaizen/cli/terraforme")),
            "-home-a-kaizen-cli-terraforme"
        );
        assert_eq!(
            project_dir_name(Path::new("/home/a/documentação_x")),
            "-home-a-documenta--o-x"
        );
    }

    #[test]
    fn timestamps_are_parsed_as_utc() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_timestamp("2026-10-17T02:45:14.612Z"),
            Some(1_792_205_114)
        );
        assert_eq!(parse_timestamp("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(parse_timestamp("ontem"), None);
    }

    #[test]
    fn parses_assistant_usage_lines() {
        let line = r#"{"message":{"model":"claude-opus-5-5","id":"msg_1","usage":{"input_tokens":2,"cache_creation_input_tokens":10,"cache_read_input_tokens":20,"output_tokens":5}},"requestId":"req_1","sessionId":"s1","timestamp":"1970-01-01T00:00:10Z"}"#;
        let (key, entry) = parse_line(line).unwrap();
        assert_eq!(key, "msg_1:req_1");
        assert_eq!(entry.session, "s1");
        assert_eq!(entry.agent, None);
        assert_eq!(entry.time, 10);
        assert_eq!(entry.tokens.total(), 37);

        assert!(parse_line(r#"{"type":"user","message":{"content":"oi"}}"#).is_none());
        let synthetic = line.replace("claude-opus-5-5", "<synthetic>");
        assert!(parse_line(&synthetic).is_none());

        let sub = line.replace(r#""sessionId""#, r#""agentId":"a1","sessionId""#);
        assert_eq!(parse_line(&sub).unwrap().1.agent.as_deref(), Some("a1"));
    }

    #[test]
    fn session_agents_put_main_first_and_read_meta() {
        let dir = std::env::temp_dir().join(format!("terraforme-usage-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("agent-a2.meta.json"),
            r#"{"agentType":"Explore","description":"busca"}"#,
        )
        .unwrap();
        let summary = |input| Summary {
            tokens: Tokens {
                input,
                ..Tokens::default()
            },
            responses: 1,
            ..Summary::default()
        };
        let agents = HashMap::from([
            (Some("a1".to_string()), summary(10)),
            (None, summary(1)),
            (Some("a2".to_string()), summary(50)),
        ]);
        let list = session_agents(agents, &dir);
        fs::remove_dir_all(&dir).unwrap();

        let ids: Vec<_> = list.iter().map(|a| a.id.as_deref()).collect();
        assert_eq!(ids, [None, Some("a2"), Some("a1")]);
        assert_eq!(list[1].kind.as_deref(), Some("Explore"));
        assert_eq!(list[1].description.as_deref(), Some("busca"));
        assert_eq!(list[2].kind, None);
    }
}
