//! Histórico do terraforme num SQLite local (`<root>/usage.db`,
//! [`crate::space::root`]): o consumo de tokens por space e agente e as
//! pastas em que o `terraforme cli` foi aberto ([`History::record_launch`]).
//!
//! O `terraforme cli` grava uma linha por sessão de cada instância (o total
//! dela até o momento, substituído a cada gravação) e a aba Consumo e a UI
//! web do MCP leem a soma por space × agente ([`History::totals`]). Só entram
//! as sessões abertas pelo `terraforme cli` cujo agente informa o total da
//! sessão (o OpenCode não informa).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::usage::{Summary, Tokens};

const DAY: u64 = 24 * 60 * 60;
const WEEK: u64 = 7 * DAY;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    key         TEXT PRIMARY KEY,
    agent       TEXT NOT NULL,
    space       TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    started     INTEGER NOT NULL,
    updated     INTEGER NOT NULL,
    input       INTEGER NOT NULL,
    output      INTEGER NOT NULL,
    reasoning   INTEGER NOT NULL,
    cache_write INTEGER NOT NULL,
    cache_read  INTEGER NOT NULL,
    responses   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_space_agent ON sessions (space, agent);
CREATE TABLE IF NOT EXISTS launches (
    path  TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    count INTEGER NOT NULL,
    last  INTEGER NOT NULL
);
";

/// Pasta em que o `terraforme cli` foi aberto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub path: PathBuf,
    /// Agente da última abertura.
    pub agent: String,
    /// Quantas vezes foi aberto ali.
    pub count: u64,
    /// Última abertura (segundos desde a época Unix).
    pub last: u64,
}

/// Consumo de uma sessão a gravar.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// Identifica a sessão: gravar a mesma `key` de novo substitui os totais.
    pub key: &'a str,
    /// Nome do agente (ex.: `claude`).
    pub agent: &'a str,
    /// Nome do space; vazio fora de space.
    pub space: &'a str,
    pub cwd: &'a Path,
    pub started: SystemTime,
    pub summary: &'a Summary,
}

/// Soma das sessões de um agente num space.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpaceUsage {
    /// Vazio fora de space.
    pub space: String,
    pub agent: String,
    pub sessions: u64,
    /// Sessões atualizadas nas últimas 24h.
    pub day: Tokens,
    /// Sessões atualizadas nos últimos 7 dias.
    pub week: Tokens,
    pub total: Tokens,
    pub responses: u64,
    /// Última gravação (segundos desde a época Unix).
    pub updated: u64,
}

pub struct History {
    conn: Connection,
}

impl History {
    /// `<root>/usage.db`.
    pub fn path() -> Result<PathBuf> {
        Ok(crate::space::root()?.join("usage.db"))
    }

    /// Abre (ou cria) o banco em [`History::path`].
    pub fn open() -> Result<Self> {
        Self::open_at(&Self::path()?)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("criando {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("abrindo {}", path.display()))?;
        // Vários `terraforme cli` (e o daemon MCP) usam o mesmo banco.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)
            .with_context(|| format!("criando as tabelas de {}", path.display()))?;
        Ok(Self { conn })
    }

    /// Grava (ou substitui) os totais da sessão.
    pub fn record(&self, record: &Record) -> Result<()> {
        let tokens = &record.summary.tokens;
        self.conn
            .execute(
                "INSERT INTO sessions (key, agent, space, cwd, started, updated, input, output,
                     reasoning, cache_write, cache_read, responses)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (key) DO UPDATE SET
                     updated = excluded.updated, input = excluded.input,
                     output = excluded.output, reasoning = excluded.reasoning,
                     cache_write = excluded.cache_write, cache_read = excluded.cache_read,
                     responses = excluded.responses",
                params![
                    record.key,
                    record.agent,
                    record.space,
                    record.cwd.to_string_lossy(),
                    secs(record.started) as i64,
                    secs(SystemTime::now()) as i64,
                    tokens.input as i64,
                    tokens.output as i64,
                    tokens.reasoning as i64,
                    tokens.cache_write as i64,
                    tokens.cache_read as i64,
                    record.summary.responses as i64,
                ],
            )
            .context("gravando o consumo da sessão")?;
        Ok(())
    }

    /// Registra uma abertura do `terraforme cli` na pasta `path`.
    pub fn record_launch(&self, path: &Path, agent: &str) -> Result<()> {
        self.record_launch_at(path, agent, secs(SystemTime::now()))
    }

    fn record_launch_at(&self, path: &Path, agent: &str, at: u64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO launches (path, agent, count, last) VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT (path) DO UPDATE SET
                     agent = excluded.agent, count = count + 1, last = excluded.last",
                params![path.to_string_lossy(), agent, at as i64],
            )
            .context("gravando a pasta no histórico")?;
        Ok(())
    }

    /// As `limit` pastas abertas mais recentemente, da mais nova para a mais antiga.
    pub fn launches(&self, limit: usize) -> Result<Vec<Launch>> {
        let mut statement = self
            .conn
            .prepare("SELECT path, agent, count, last FROM launches ORDER BY last DESC LIMIT ?1")?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok(Launch {
                path: PathBuf::from(row.get::<_, String>(0)?),
                agent: row.get(1)?,
                count: row.get::<_, i64>(2)? as u64,
                last: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Consumo por space × agente, do space com mais tokens para o com menos.
    pub fn totals(&self) -> Result<Vec<SpaceUsage>> {
        self.totals_at(secs(SystemTime::now()))
    }

    fn totals_at(&self, now: u64) -> Result<Vec<SpaceUsage>> {
        let mut statement = self.conn.prepare(
            "SELECT space, agent, updated, input, output, reasoning, cache_write, cache_read,
                 responses
             FROM sessions",
        )?;
        let rows = statement.query_map([], |row| {
            let tokens = Tokens {
                input: row.get::<_, i64>(3)? as u64,
                output: row.get::<_, i64>(4)? as u64,
                reasoning: row.get::<_, i64>(5)? as u64,
                cache_write: row.get::<_, i64>(6)? as u64,
                cache_read: row.get::<_, i64>(7)? as u64,
            };
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u64,
                tokens,
                row.get::<_, i64>(8)? as u64,
            ))
        })?;
        let mut totals: Vec<SpaceUsage> = Vec::new();
        for row in rows {
            let (space, agent, updated, tokens, responses) = row?;
            let index = match totals
                .iter()
                .position(|t| t.space == space && t.agent == agent)
            {
                Some(index) => index,
                None => {
                    totals.push(SpaceUsage {
                        space,
                        agent,
                        ..SpaceUsage::default()
                    });
                    totals.len() - 1
                }
            };
            let entry = &mut totals[index];
            entry.sessions += 1;
            entry.responses += responses;
            entry.updated = entry.updated.max(updated);
            entry.total.add(&tokens);
            let age = now.saturating_sub(updated);
            if age < WEEK {
                entry.week.add(&tokens);
            }
            if age < DAY {
                entry.day.add(&tokens);
            }
        }
        totals.sort_by(|a, b| {
            b.total
                .total()
                .cmp(&a.total.total())
                .then_with(|| a.space.cmp(&b.space))
                .then_with(|| a.agent.cmp(&b.agent))
        });
        Ok(totals)
    }
}

fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(input: u64, responses: usize) -> Summary {
        Summary {
            tokens: Tokens {
                input,
                ..Tokens::default()
            },
            responses,
            ..Summary::default()
        }
    }

    #[test]
    fn launches_keep_one_row_per_path_newest_first() {
        let path = std::env::temp_dir().join(format!(
            "terraforme-launches-{}/usage.db",
            std::process::id()
        ));
        let db = History::open_at(&path).unwrap();
        db.record_launch_at(Path::new("/w/web"), "claude", 10)
            .unwrap();
        db.record_launch_at(Path::new("/w/api"), "claude", 20)
            .unwrap();
        db.record_launch_at(Path::new("/w/web"), "opencode", 30)
            .unwrap();

        let launches = db.launches(10).unwrap();
        let rows: Vec<_> = launches
            .iter()
            .map(|l| (l.path.to_str().unwrap(), l.agent.as_str(), l.count, l.last))
            .collect();
        assert_eq!(
            rows,
            [("/w/web", "opencode", 2, 30), ("/w/api", "claude", 1, 20)]
        );
        assert_eq!(db.launches(1).unwrap().len(), 1);

        drop(db);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn records_replace_the_session_and_sum_by_space_and_agent() {
        let path = std::env::temp_dir().join(format!(
            "terraforme-history-{}/usage.db",
            std::process::id()
        ));
        let db = History::open_at(&path).unwrap();
        let record = |key: &str, agent: &str, space: &str, input: u64, responses: usize| {
            db.record(&Record {
                key,
                agent,
                space,
                cwd: Path::new("/w"),
                started: SystemTime::now(),
                summary: &summary(input, responses),
            })
            .unwrap()
        };
        record("a", "claude", "web", 10, 1);
        // Mesma sessão: substitui o total, não soma.
        record("a", "claude", "web", 30, 2);
        record("b", "claude", "web", 5, 1);
        record("c", "opencode", "web", 1, 1);
        record("d", "claude", "", 100, 4);

        let now = secs(SystemTime::now());
        let totals = db.totals_at(now).unwrap();
        let keys: Vec<_> = totals
            .iter()
            .map(|t| {
                (
                    t.space.as_str(),
                    t.agent.as_str(),
                    t.sessions,
                    t.total.input,
                )
            })
            .collect();
        assert_eq!(
            keys,
            [
                ("", "claude", 1, 100),
                ("web", "claude", 2, 35),
                ("web", "opencode", 1, 1)
            ]
        );
        assert_eq!(totals[1].responses, 3);
        assert_eq!(totals[1].day.input, 35);

        // Oito dias depois: sai das janelas de 24h e 7 dias, fica no total.
        let later = db.totals_at(now + 8 * DAY).unwrap();
        assert_eq!(later[1].day.input, 0);
        assert_eq!(later[1].week.input, 0);
        assert_eq!(later[1].total.input, 35);

        drop(db);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
