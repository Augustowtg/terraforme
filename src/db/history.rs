//! O histórico de conversas de um workspace, no SQLite
//! `~/.terraforme/workspace/<nome>/<nome>.sql`.
//!
//! Um terra pode ter vários chats, e vários terras podem abrir o mesmo
//! workspace: cada chat tem a sua conexão, e o WAL com o `busy_timeout` deixa
//! elas escreverem no mesmo arquivo.

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::now_millis;
use crate::agent::{ModelSpend, merge_spend};

/// Quanto uma escrita espera por outra conexão antes de desistir.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// Quantas mensagens enviadas o workspace guarda para o ↑ do input.
const SENT_LIMIT: i64 = 10;

/// Quantos dias [`FolderUsage::days`] traz.
const DAYS_KEPT: i64 = 7;

/// Uma mensagem da conversa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    /// `user`, `assistant`, `tool`, `tool_result`, `tool_error`, `error` ou
    /// `info`.
    pub role: &'static str,
    /// O nome da ferramenta (só no `tool`).
    pub tool: Option<&'a str>,
    pub content: &'a str,
}

/// O que o agente informou sobre a conversa até agora.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conversation<'a> {
    pub provider: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub model: Option<&'a str>,
}

/// De quem é um turno gravado com [`ChatHistory::record_turn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnOwner<'a> {
    /// A pasta da sessão. Um workspace pode mudar de pasta, e pastas com o
    /// mesmo nome dividem o arquivo: é ela que separa o consumo.
    pub folder: &'a str,
    /// O id do turno, único entre as linhas (uma por modelo).
    pub turn: &'a str,
    pub provider: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

/// O consumo de um dia, em [`FolderUsage::days`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DayUsage {
    /// Na hora local, como `2026-11-06`.
    pub day: String,
    pub turns: u64,
    /// Entrada e saída.
    pub tokens: u64,
    pub cache: u64,
    pub cost_micros: Option<u64>,
}

/// O consumo da sessão de uma pasta: tudo o que os chats do terra gastaram nela.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderUsage {
    /// Em milissegundos, o primeiro e o último turno.
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub turns: u64,
    pub conversations: u64,
    /// Os turnos de um provedor que não informa o custo.
    pub turns_without_cost: u64,
    pub priciest_turn_micros: Option<u64>,
    /// Do modelo que mais custou (e, sem custo, mais gastou) ao que menos.
    pub models: Vec<ModelSpend>,
    /// Os últimos [`DAYS_KEPT`] dias com consumo, o mais novo primeiro.
    pub days: Vec<DayUsage>,
}

/// Traz o histórico da antiga `sql/history.sqlite`, ao lado de `file`, para
/// `file`, e apaga a pasta. Um `file` que já existe fica como está.
pub fn move_old_file(file: &Path) -> Result<()> {
    let Some(dir) = file.parent() else {
        return Ok(());
    };
    let old_dir = dir.join("sql");
    if file.exists() || !old_dir.join("history.sqlite").exists() {
        return Ok(());
    }
    // O banco por último: se parar no meio, a próxima abertura continua.
    for suffix in ["-wal", "-shm", ""] {
        let old = old_dir.join(format!("history.sqlite{suffix}"));
        if old.exists() {
            let mut new = file.as_os_str().to_owned();
            new.push(suffix);
            fs::rename(&old, &new)
                .with_context(|| format!("não foi possível mover {}", old.display()))?;
        }
    }
    let _ = fs::remove_dir(&old_dir);
    Ok(())
}

pub struct ChatHistory {
    conn: Connection,
}

/// Uma contagem do banco, que o SQLite guarda com sinal.
fn count(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    Ok(row.get::<_, i64>(column)? as u64)
}

fn optional_count(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<Option<u64>> {
    Ok(row.get::<_, Option<i64>>(column)?.map(|n| n as u64))
}

impl ChatHistory {
    /// Abre (criando se preciso) o banco em `file`. A pasta fica só para o
    /// usuário: as conversas podem ter segredos.
    pub fn open(file: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("não foi possível criar {}", dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
        }
        let conn = Connection::open(file)
            .with_context(|| format!("não foi possível abrir {}", file.display()))?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                provider   TEXT,
                session_id TEXT,
                model      TEXT,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL
                                REFERENCES conversations (id) ON DELETE CASCADE,
                role            TEXT    NOT NULL,
                tool            TEXT,
                content         TEXT    NOT NULL,
                created_at      INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS messages_by_conversation
                ON messages (conversation_id, id);
            CREATE TABLE IF NOT EXISTS sent_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                content    TEXT    NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS usage_turns (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                folder         TEXT    NOT NULL,
                turn           TEXT    NOT NULL,
                provider       TEXT,
                session_id     TEXT,
                model          TEXT    NOT NULL,
                input          INTEGER NOT NULL,
                output         INTEGER NOT NULL,
                cache_creation INTEGER NOT NULL,
                cache_read     INTEGER NOT NULL,
                thinking       INTEGER NOT NULL,
                web_searches   INTEGER NOT NULL,
                cost_micros    INTEGER,
                created_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS usage_turns_by_folder
                ON usage_turns (folder, created_at);",
        )
        .context("não foi possível criar as tabelas do histórico")?;
        Ok(Self { conn })
    }

    /// Começa uma conversa e retorna o id dela.
    pub fn start_conversation(&self) -> Result<i64> {
        let now = now_millis();
        self.conn.execute(
            "INSERT INTO conversations (started_at, updated_at) VALUES (?1, ?1)",
            params![now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Acrescenta `messages` à conversa `id` e atualiza o que o agente
    /// informou sobre ela; o que ele ainda não informou fica como está.
    pub fn append(
        &mut self,
        id: i64,
        conversation: Conversation<'_>,
        messages: &[Message<'_>],
    ) -> Result<()> {
        let now = now_millis();
        let tx = self.conn.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO messages (conversation_id, role, tool, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for message in messages {
                insert.execute(params![
                    id,
                    message.role,
                    message.tool,
                    message.content,
                    now
                ])?;
            }
        }
        tx.execute(
            "UPDATE conversations
             SET provider   = COALESCE(?2, provider),
                 session_id = COALESCE(?3, session_id),
                 model      = COALESCE(?4, model),
                 updated_at = ?5
             WHERE id = ?1",
            params![
                id,
                conversation.provider,
                conversation.session_id,
                conversation.model,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Guarda `content` entre as mensagens enviadas (sem repetir a última),
    /// que ficam só nas [`SENT_LIMIT`] mais novas.
    pub fn remember_sent(&self, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sent_messages (content, created_at)
             SELECT ?1, ?2
             WHERE ?1 IS NOT (SELECT content FROM sent_messages ORDER BY id DESC LIMIT 1)",
            params![content, now_millis()],
        )?;
        self.conn.execute(
            "DELETE FROM sent_messages WHERE id NOT IN
             (SELECT id FROM sent_messages ORDER BY id DESC LIMIT ?1)",
            [SENT_LIMIT],
        )?;
        Ok(())
    }

    /// Grava o gasto de um turno, uma linha por modelo.
    pub fn record_turn(&mut self, owner: TurnOwner<'_>, spend: &[ModelSpend]) -> Result<()> {
        let now = now_millis();
        let tx = self.conn.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO usage_turns (folder, turn, provider, session_id, model, input,
                    output, cache_creation, cache_read, thinking, web_searches, cost_micros,
                    created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for item in spend {
                insert.execute(params![
                    owner.folder,
                    owner.turn,
                    owner.provider,
                    owner.session_id,
                    item.model,
                    item.input as i64,
                    item.output as i64,
                    item.cache_creation as i64,
                    item.cache_read as i64,
                    item.thinking as i64,
                    item.web_searches as i64,
                    item.cost_micros.map(|cost| cost as i64),
                    now
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// O consumo da sessão da pasta `folder`.
    pub fn folder_usage(&self, folder: &str) -> Result<FolderUsage> {
        let mut usage = self.conn.query_row(
            "SELECT MIN(created_at), MAX(created_at), COUNT(DISTINCT turn),
                    COUNT(DISTINCT session_id),
                    COUNT(DISTINCT CASE WHEN cost_micros IS NULL THEN turn END)
             FROM usage_turns WHERE folder = ?1",
            [folder],
            |row| {
                Ok(FolderUsage {
                    first_at: row.get(0)?,
                    last_at: row.get(1)?,
                    turns: count(row, 2)?,
                    conversations: count(row, 3)?,
                    turns_without_cost: count(row, 4)?,
                    ..FolderUsage::default()
                })
            },
        )?;
        usage.priciest_turn_micros = self.conn.query_row(
            "SELECT MAX(cost) FROM
             (SELECT SUM(cost_micros) AS cost FROM usage_turns WHERE folder = ?1 GROUP BY turn)",
            [folder],
            |row| optional_count(row, 0),
        )?;
        let mut models = self.conn.prepare_cached(
            "SELECT model, SUM(input), SUM(output), SUM(cache_creation), SUM(cache_read),
                    SUM(thinking), SUM(web_searches), SUM(cost_micros)
             FROM usage_turns WHERE folder = ?1 GROUP BY model
             ORDER BY COALESCE(SUM(cost_micros), 0) DESC,
                      SUM(input) + SUM(output) + SUM(cache_creation) + SUM(cache_read) DESC",
        )?;
        let rows = models.query_map([folder], |row| {
            Ok(ModelSpend {
                model: row.get(0)?,
                input: count(row, 1)?,
                output: count(row, 2)?,
                cache_creation: count(row, 3)?,
                cache_read: count(row, 4)?,
                thinking: count(row, 5)?,
                web_searches: count(row, 6)?,
                cost_micros: optional_count(row, 7)?,
            })
        })?;
        for row in rows {
            merge_spend(&mut usage.models, &[row?]);
        }
        let mut days = self.conn.prepare_cached(
            "SELECT date(created_at / 1000, 'unixepoch', 'localtime') AS day,
                    COUNT(DISTINCT turn), SUM(input) + SUM(output),
                    SUM(cache_creation) + SUM(cache_read), SUM(cost_micros)
             FROM usage_turns WHERE folder = ?1 GROUP BY day ORDER BY day DESC LIMIT ?2",
        )?;
        let rows = days.query_map(params![folder, DAYS_KEPT], |row| {
            Ok(DayUsage {
                day: row.get(0)?,
                turns: count(row, 1)?,
                tokens: count(row, 2)?,
                cache: count(row, 3)?,
                cost_micros: optional_count(row, 4)?,
            })
        })?;
        usage.days = rows.collect::<rusqlite::Result<_>>()?;
        Ok(usage)
    }

    /// As mensagens enviadas guardadas, a mais antiga primeiro.
    pub fn sent(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT content FROM
             (SELECT id, content FROM sent_messages ORDER BY id DESC LIMIT ?1)
             ORDER BY id",
        )?;
        let rows = stmt.query_map([SENT_LIMIT], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &'static str, content: &'static str) -> Message<'static> {
        Message {
            role,
            tool: None,
            content,
        }
    }

    fn spend(model: &str, input: u64, cost: Option<u64>) -> ModelSpend {
        ModelSpend {
            model: model.to_string(),
            input,
            output: 2,
            cache_creation: 3,
            cache_read: 4,
            thinking: 1,
            web_searches: 1,
            cost_micros: cost,
        }
    }

    fn owner<'a>(folder: &'a str, turn: &'a str, session: &'a str) -> TurnOwner<'a> {
        TurnOwner {
            folder,
            turn,
            provider: Some("claude"),
            session_id: Some(session),
        }
    }

    #[test]
    fn the_usage_of_a_folder_adds_its_turns_and_leaves_the_others_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = ChatHistory::open(&dir.path().join("x.sql")).unwrap();
        history
            .record_turn(
                owner("/a", "t1", "s1"),
                &[spend("opus", 10, Some(500)), spend("haiku", 1, Some(100))],
            )
            .unwrap();
        history
            .record_turn(owner("/a", "t2", "s1"), &[spend("opus", 5, Some(200))])
            .unwrap();
        history
            .record_turn(owner("/a", "t3", "s2"), &[spend("gemini", 7, None)])
            .unwrap();
        history
            .record_turn(owner("/b", "t4", "s3"), &[spend("opus", 99, Some(9_000))])
            .unwrap();

        let usage = history.folder_usage("/a").unwrap();
        assert_eq!(usage.turns, 3);
        assert_eq!(usage.conversations, 2);
        assert_eq!(usage.turns_without_cost, 1);
        assert_eq!(usage.priciest_turn_micros, Some(600));
        assert!(usage.first_at.is_some() && usage.last_at.is_some());
        let names: Vec<&str> = usage.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(names, ["opus", "haiku", "gemini"]);
        assert_eq!(usage.models[0].input, 15);
        assert_eq!(usage.models[0].cost_micros, Some(700));
        assert_eq!(usage.models[2].cost_micros, None);
        assert_eq!(usage.days.len(), 1);
        assert_eq!(usage.days[0].turns, 3);
        assert_eq!(usage.days[0].tokens, 23 + 8);
        assert_eq!(usage.days[0].cost_micros, Some(800));

        assert_eq!(
            history.folder_usage("/nada").unwrap(),
            FolderUsage::default(),
            "uma pasta sem turnos não tem consumo"
        );
    }

    #[test]
    fn creates_the_folder_and_keeps_the_messages_of_each_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("workspace/x/x.sql");
        let mut history = ChatHistory::open(&file).unwrap();

        let first = history.start_conversation().unwrap();
        let second = history.start_conversation().unwrap();
        assert_ne!(first, second);
        history
            .append(
                first,
                Conversation {
                    provider: Some("claude"),
                    session_id: Some("s1"),
                    model: Some("opus"),
                },
                &[message("user", "oi"), message("assistant", "olá")],
            )
            .unwrap();
        // O que não veio agora não apaga o que já foi informado.
        history
            .append(
                first,
                Conversation::default(),
                &[Message {
                    role: "tool",
                    tool: Some("Read"),
                    content: "a.rs",
                }],
            )
            .unwrap();
        history
            .append(second, Conversation::default(), &[message("user", "outra")])
            .unwrap();

        // Outra conexão (outro chat) lê o mesmo arquivo.
        let conn = Connection::open(&file).unwrap();
        let (provider, session, model): (String, String, String) = conn
            .query_row(
                "SELECT provider, session_id, model FROM conversations WHERE id = ?1",
                [first],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (provider.as_str(), session.as_str(), model.as_str()),
            ("claude", "s1", "opus")
        );
        let mut rows = conn
            .prepare(
                "SELECT role, tool, content FROM messages
                 WHERE conversation_id = ?1 ORDER BY id",
            )
            .unwrap();
        let messages: Vec<(String, Option<String>, String)> = rows
            .query_map([first], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            messages,
            [
                ("user".to_string(), None, "oi".to_string()),
                ("assistant".to_string(), None, "olá".to_string()),
                (
                    "tool".to_string(),
                    Some("Read".to_string()),
                    "a.rs".to_string()
                ),
            ]
        );
    }

    #[test]
    fn keeps_only_the_last_ten_sent_messages_without_repeating_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        let history = ChatHistory::open(&file).unwrap();
        assert!(history.sent().unwrap().is_empty());

        for i in 0..12 {
            history.remember_sent(&format!("m{i}")).unwrap();
        }
        history.remember_sent("m11").unwrap();

        // Outra conexão (outra execução do terra) lê as mesmas.
        let sent = ChatHistory::open(&file).unwrap().sent().unwrap();
        let expected: Vec<String> = (2..12).map(|i| format!("m{i}")).collect();
        assert_eq!(sent, expected);

        // Só a última não repete: uma mais antiga volta para o fim.
        history.remember_sent("m5").unwrap();
        let sent = history.sent().unwrap();
        assert_eq!(sent.len(), 10);
        assert_eq!(sent.last().unwrap(), "m5");
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_is_only_for_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("x");
        ChatHistory::open(&folder.join("x.sql")).unwrap();
        let mode = fs::metadata(&folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn the_old_file_moves_with_its_messages_and_the_folder_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let old_dir = dir.path().join("sql");
        let old = ChatHistory::open(&old_dir.join("history.sqlite")).unwrap();
        old.remember_sent("antiga").unwrap();
        drop(old);
        // O WAL de um terra que não fechou direito vai junto.
        fs::write(old_dir.join("history.sqlite-wal"), "").unwrap();

        let file = dir.path().join("x.sql");
        move_old_file(&file).unwrap();

        assert!(!old_dir.exists());
        assert!(dir.path().join("x.sql-wal").exists());
        let sent = ChatHistory::open(&file).unwrap().sent().unwrap();
        assert_eq!(sent, ["antiga"]);
    }

    #[test]
    fn the_old_file_does_not_replace_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let old_dir = dir.path().join("sql");
        let old = ChatHistory::open(&old_dir.join("history.sqlite")).unwrap();
        old.remember_sent("antiga").unwrap();
        drop(old);
        let file = dir.path().join("x.sql");
        let new = ChatHistory::open(&file).unwrap();
        new.remember_sent("nova").unwrap();
        drop(new);

        move_old_file(&file).unwrap();

        assert!(old_dir.join("history.sqlite").exists());
        let sent = ChatHistory::open(&file).unwrap().sent().unwrap();
        assert_eq!(sent, ["nova"]);
    }
}
