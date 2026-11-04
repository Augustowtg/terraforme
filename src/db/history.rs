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

/// Quanto uma escrita espera por outra conexão antes de desistir.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// Quantas mensagens enviadas o workspace guarda para o ↑ do input.
const SENT_LIMIT: i64 = 10;

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
            );",
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
