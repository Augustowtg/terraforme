//! Persistência em SQLite (`~/.terraforme/terra.sqlite`).
//!
//! Todas as funções que recebem um path esperam que ele já tenha passado por
//! [`canonical_path`]; a comparação de duplicados é feita sobre esse valor.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::settings::{ExtensionMode, ExtensionSettings};

/// No Windows a comparação de paths não diferencia maiúsculas/minúsculas.
#[cfg(windows)]
const PATH_COLLATE: &str = "COLLATE NOCASE";
#[cfg(not(windows))]
const PATH_COLLATE: &str = "";

/// A "conta" das extensões "usar sempre", que valem em todas.
const ALL_ACCOUNTS: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    pub last_used_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Workspace criado com o nome informado.
    Saved(String),
    /// O path já estava salvo com este nome; nada foi alterado.
    AlreadySaved(String),
}

#[derive(Debug)]
pub enum SaveError {
    EmptyName,
    NameTaken { name: String, path: PathBuf },
    Db(rusqlite::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::EmptyName => write!(f, "o nome do workspace não pode ser vazio"),
            SaveError::NameTaken { name, path } => write!(
                f,
                "o nome \"{name}\" já está em uso pelo workspace {}",
                path.display()
            ),
            SaveError::Db(err) => write!(f, "erro no banco de dados: {err}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<rusqlite::Error> for SaveError {
    fn from(err: rusqlite::Error) -> Self {
        SaveError::Db(err)
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Abre (criando se preciso) `~/.terraforme/terra.sqlite`.
    pub fn open_default() -> Result<Self> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("não foi possível criar {}", dir.display()))?;
        Self::open(&dir.join("terra.sqlite"))
    }

    pub fn open(file: &Path) -> Result<Self> {
        let conn = Connection::open(file)
            .with_context(|| format!("não foi possível abrir {}", file.display()))?;
        Self::init(conn)
    }

    /// Um banco só desta execução: o terra abre mesmo sem conseguir abrir o
    /// `terra.sqlite`.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS recents (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                path         TEXT    NOT NULL UNIQUE {PATH_COLLATE},
                last_used_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS workspaces (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT    NOT NULL UNIQUE,
                path       TEXT    NOT NULL UNIQUE {PATH_COLLATE},
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS extensions (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                account   TEXT NOT NULL,
                extension TEXT NOT NULL,
                UNIQUE (account, extension)
            );
            CREATE TABLE IF NOT EXISTS extension_accounts (
                account TEXT PRIMARY KEY
            );"
        ))
        .context("não foi possível criar as tabelas")?;
        Ok(Self { conn })
    }

    /// Registra o uso de um path em `recents` (upsert de `last_used_at`).
    pub fn touch_recent(&self, path: &Path) -> Result<()> {
        self.touch_recent_at(path, now_millis())
    }

    pub fn touch_recent_at(&self, path: &Path, timestamp: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO recents (path, last_used_at) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET last_used_at = excluded.last_used_at",
            params![path_str(path), timestamp],
        )?;
        Ok(())
    }

    /// Recentes do mais novo para o mais antigo.
    pub fn recents(&self, offset: usize, limit: usize) -> Result<Vec<Recent>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, last_used_at FROM recents
             ORDER BY last_used_at DESC, id DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            Ok(Recent {
                path: PathBuf::from(row.get::<_, String>(0)?),
                last_used_at: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Workspaces salvos em ordem alfabética (sem diferenciar maiúsculas).
    pub fn workspaces(&self, offset: usize, limit: usize) -> Result<Vec<Workspace>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, path FROM workspaces
             ORDER BY name COLLATE NOCASE ASC, name ASC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            Ok(Workspace {
                name: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Salva `path` como workspace.
    ///
    /// - Path já salvo: não duplica, retorna [`SaveOutcome::AlreadySaved`].
    /// - Sem `name`: usa o basename (ou `root`) e sufixa `-2`, `-3`... se o
    ///   nome já existir.
    /// - Com `name`: se o nome já existir, falha com [`SaveError::NameTaken`].
    pub fn save_workspace(
        &self,
        path: &Path,
        name: Option<&str>,
    ) -> Result<SaveOutcome, SaveError> {
        let name = name.map(str::trim);
        if name == Some("") {
            return Err(SaveError::EmptyName);
        }

        let path_s = path_str(path);
        if let Some(existing) = self.workspace_name_by_path(&path_s)? {
            return Ok(SaveOutcome::AlreadySaved(existing));
        }

        let name = match name {
            Some(name) => {
                if let Some(other) = self.workspace_path_by_name(name)? {
                    return Err(SaveError::NameTaken {
                        name: name.to_string(),
                        path: PathBuf::from(other),
                    });
                }
                name.to_string()
            }
            None => self.free_name(&default_name(path))?,
        };

        self.conn.execute(
            "INSERT INTO workspaces (name, path, created_at) VALUES (?1, ?2, ?3)",
            params![name, path_s, now_millis()],
        )?;
        Ok(SaveOutcome::Saved(name))
    }

    /// Primeiro nome livre entre `base`, `base-2`, `base-3`...
    fn free_name(&self, base: &str) -> rusqlite::Result<String> {
        if self.workspace_path_by_name(base)?.is_none() {
            return Ok(base.to_string());
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}-{n}");
            if self.workspace_path_by_name(&candidate)?.is_none() {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    /// O nome do workspace salvo em `path`, se houver.
    pub fn workspace_name(&self, path: &Path) -> Result<Option<String>> {
        Ok(self.workspace_name_by_path(&path_str(path))?)
    }

    fn workspace_name_by_path(&self, path: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT name FROM workspaces WHERE path = ?1",
                [path],
                |row| row.get(0),
            )
            .optional()
    }

    fn workspace_path_by_name(&self, name: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT path FROM workspaces WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
    }

    /// As extensões da conta `account` (ex.: `claude:.claude-trabalho`),
    /// mais as "usar sempre". Na primeira vez da conta, ela começa com as
    /// `legacy` (as do `config.json` do workspace, de antes do banco).
    pub fn extension_settings(
        &self,
        account: &str,
        legacy: &[String],
    ) -> Result<ExtensionSettings> {
        let known = self.conn.execute(
            "INSERT OR IGNORE INTO extension_accounts (account) VALUES (?1)",
            [account],
        )? == 0;
        if !known {
            for id in legacy {
                self.set_extension(account, id, ExtensionMode::Account)?;
            }
        }
        Ok(ExtensionSettings {
            enabled: self.extensions_of(account)?,
            always: self.extensions_of(ALL_ACCOUNTS)?,
        })
    }

    /// Grava o modo de uma extensão na conta `account`: inativa sai da conta
    /// e das "usar sempre"; "usar sempre" vale para todas as contas.
    pub fn set_extension(&self, account: &str, id: &str, mode: ExtensionMode) -> Result<()> {
        let keep = match mode {
            ExtensionMode::Off => None,
            ExtensionMode::Account => Some(account),
            ExtensionMode::Always => Some(ALL_ACCOUNTS),
        };
        self.conn.execute(
            "DELETE FROM extensions
             WHERE extension = ?1 AND account IN (?2, ?3) AND account IS NOT ?4",
            params![id, account, ALL_ACCOUNTS, keep],
        )?;
        if let Some(keep) = keep {
            self.conn.execute(
                "INSERT OR IGNORE INTO extensions (account, extension) VALUES (?1, ?2)",
                params![keep, id],
            )?;
        }
        Ok(())
    }

    fn extensions_of(&self, account: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT extension FROM extensions WHERE account = ?1 ORDER BY id")?;
        let rows = stmt.query_map([account], |row| row.get(0))?;
        rows.collect()
    }
}

/// A pasta de dados do terra: `~/.terraforme`.
pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("não foi possível encontrar o diretório home do usuário")?
        .join(".terraforme"))
}

/// Caminho absoluto, sem `..`, sem barra final e, no Windows, sem o prefixo
/// `\\?\` e com separadores `\`.
pub fn canonical_path(path: &Path) -> io::Result<PathBuf> {
    let path = dunce::canonicalize(path)?;
    #[cfg(windows)]
    let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
    Ok(path)
}

/// Nome padrão de um workspace: basename do path, ou `root` na raiz do disco.
pub fn default_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".to_string())
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    fn count(db: &Db, table: &str) -> i64 {
        db.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn recents_upsert_updates_without_duplicating() {
        let db = db();
        let a = Path::new("/home/user/a");
        let b = Path::new("/home/user/b");

        db.touch_recent_at(a, 100).unwrap();
        db.touch_recent_at(b, 200).unwrap();
        db.touch_recent_at(a, 300).unwrap();

        assert_eq!(count(&db, "recents"), 2);
        let recents = db.recents(0, 5).unwrap();
        assert_eq!(
            recents[0],
            Recent {
                path: a.into(),
                last_used_at: 300
            }
        );
        assert_eq!(recents[1].path, b);
    }

    #[test]
    fn recents_paginate_newest_first() {
        let db = db();
        for i in 0..7 {
            db.touch_recent_at(&PathBuf::from(format!("/p/{i}")), i)
                .unwrap();
        }
        let first: Vec<_> = db
            .recents(0, 5)
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        let rest: Vec<_> = db
            .recents(5, 5)
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(first[0], PathBuf::from("/p/6"));
        assert_eq!(first.len(), 5);
        assert_eq!(rest, vec![PathBuf::from("/p/1"), PathBuf::from("/p/0")]);
    }

    #[test]
    fn duplicate_path_is_not_saved_twice() {
        let db = db();
        let path = Path::new("/home/user/projeto");

        assert_eq!(
            db.save_workspace(path, None).unwrap(),
            SaveOutcome::Saved("projeto".into())
        );
        assert_eq!(
            db.save_workspace(path, Some("outro")).unwrap(),
            SaveOutcome::AlreadySaved("projeto".into())
        );
        assert_eq!(count(&db, "workspaces"), 1);
    }

    #[test]
    fn duplicate_default_name_gets_numeric_suffix() {
        let db = db();
        let saved: Vec<_> = ["/a/projeto", "/b/projeto", "/c/projeto"]
            .iter()
            .map(|p| db.save_workspace(Path::new(p), None).unwrap())
            .collect();
        assert_eq!(
            saved,
            vec![
                SaveOutcome::Saved("projeto".into()),
                SaveOutcome::Saved("projeto-2".into()),
                SaveOutcome::Saved("projeto-3".into()),
            ]
        );
    }

    #[test]
    fn duplicate_explicit_name_fails() {
        let db = db();
        db.save_workspace(Path::new("/a/x"), Some("api")).unwrap();
        let err = db
            .save_workspace(Path::new("/b/y"), Some("  api "))
            .unwrap_err();
        assert!(matches!(err, SaveError::NameTaken { ref name, .. } if name == "api"));
        assert_eq!(count(&db, "workspaces"), 1);
    }

    #[test]
    fn explicit_name_is_trimmed_and_not_empty() {
        let db = db();
        assert!(matches!(
            db.save_workspace(Path::new("/a"), Some("   ")),
            Err(SaveError::EmptyName)
        ));
        assert_eq!(
            db.save_workspace(Path::new("/a"), Some("  meu app "))
                .unwrap(),
            SaveOutcome::Saved("meu app".into())
        );
    }

    #[test]
    fn root_path_defaults_to_root_name() {
        assert_eq!(default_name(Path::new("/")), "root");
        assert_eq!(default_name(Path::new("/home/user/projeto")), "projeto");
    }

    #[test]
    fn workspaces_are_sorted_case_insensitively() {
        let db = db();
        for (name, path) in [("beta", "/1"), ("Alpha", "/2"), ("charlie", "/3")] {
            db.save_workspace(Path::new(path), Some(name)).unwrap();
        }
        let names: Vec<_> = db
            .workspaces(0, 5)
            .unwrap()
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta", "charlie"]);
    }

    #[test]
    fn extensions_are_per_account_and_always_ones_follow_every_account() {
        let db = db();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // Na primeira vez, a conta começa com as do workspace; depois, não.
        let work = db
            .extension_settings("claude:.claude-trabalho", &ids(&["rtk"]))
            .unwrap();
        assert_eq!(work.enabled, ["rtk"]);
        db.set_extension("claude:.claude-trabalho", "rtk", ExtensionMode::Off)
            .unwrap();
        let work = db
            .extension_settings("claude:.claude-trabalho", &ids(&["rtk"]))
            .unwrap();
        assert_eq!(work, ExtensionSettings::default());

        db.set_extension("claude:", "rtk", ExtensionMode::Account)
            .unwrap();
        db.set_extension("claude:", "ai-memory", ExtensionMode::Always)
            .unwrap();
        let personal = db.extension_settings("claude:", &[]).unwrap();
        assert_eq!(personal.enabled, ["rtk"]);
        assert_eq!(personal.always, ["ai-memory"]);

        // Trocando de conta, só a "usar sempre" continua.
        let work = db
            .extension_settings("claude:.claude-trabalho", &[])
            .unwrap();
        assert_eq!(work.active(), ["ai-memory"]);

        // De "usar sempre" para só a conta, e desligada em todas.
        db.set_extension(
            "claude:.claude-trabalho",
            "ai-memory",
            ExtensionMode::Account,
        )
        .unwrap();
        assert_eq!(
            db.extension_settings("claude:", &[]).unwrap().active(),
            ["rtk"]
        );
        db.set_extension("claude:.claude-trabalho", "ai-memory", ExtensionMode::Off)
            .unwrap();
        assert_eq!(count(&db, "extensions"), 1);
    }
}
