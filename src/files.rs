//! Índice dos arquivos e pastas de um diretório, sempre atualizado.
//!
//! Uma thread lista o diretório (respeitando o `.gitignore` e escondendo
//! arquivos ocultos, como o `ripgrep`) e lista de novo sempre que algo muda:
//! um observador do sistema (`notify`: inotify no Linux, ReadDirectoryChanges
//! no Windows) avisa de criações, remoções e renomeações. Se o observador não
//! puder ser criado, a lista é refeita a cada [`POLL_FALLBACK`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Limite de entradas, para um diretório enorme (ex.: a home) não pesar.
const MAX_ENTRIES: usize = 50_000;
/// De quanto em quanto tempo a thread confere se precisa listar de novo.
const CHECK_EVERY: Duration = Duration::from_millis(150);
/// Sem observador, lista de novo a cada 2 s.
const POLL_FALLBACK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Caminho relativo ao diretório, com `/` (ex.: `src/main.rs`).
    pub path: String,
    pub is_dir: bool,
}

pub struct FileIndex {
    entries: Arc<RwLock<Arc<Vec<FileEntry>>>>,
    /// Desligado ao soltar o índice, para a thread terminar.
    alive: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
}

impl FileIndex {
    /// Começa a listar e observar `root`. A primeira lista chega logo depois.
    pub fn watch(root: PathBuf) -> Self {
        let entries = Arc::new(RwLock::new(Arc::new(Vec::new())));
        let alive = Arc::new(AtomicBool::new(true));
        let dirty = Arc::new(AtomicBool::new(true));

        let watcher = {
            let dirty = Arc::clone(&dirty);
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    dirty.store(true, Ordering::Relaxed);
                }
            })
            .ok()
            .and_then(|mut watcher| {
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .ok()
                    .map(|()| watcher)
            })
        };
        let polling = watcher.is_none();

        {
            let entries = Arc::clone(&entries);
            let alive = Arc::clone(&alive);
            thread::spawn(move || {
                let mut last_scan = Instant::now();
                while alive.load(Ordering::Relaxed) {
                    let due = polling && last_scan.elapsed() >= POLL_FALLBACK;
                    if dirty.swap(false, Ordering::Relaxed) || due {
                        let scanned = Arc::new(scan(&root));
                        if let Ok(mut current) = entries.write() {
                            *current = scanned;
                        }
                        last_scan = Instant::now();
                    }
                    thread::sleep(CHECK_EVERY);
                }
            });
        }

        Self {
            entries,
            alive,
            _watcher: watcher,
        }
    }

    /// Índice fixo, sem thread nem observador (para testes).
    #[cfg(test)]
    pub fn fixed(entries: Vec<FileEntry>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(Arc::new(entries))),
            alive: Arc::new(AtomicBool::new(false)),
            _watcher: None,
        }
    }

    /// A lista mais recente.
    pub fn snapshot(&self) -> Arc<Vec<FileEntry>> {
        self.entries
            .read()
            .map(|entries| Arc::clone(&entries))
            .unwrap_or_default()
    }
}

impl Drop for FileIndex {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

/// Lista `root` inteiro, ordenado por caminho.
fn scan(root: &Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = WalkBuilder::new(root)
        .hidden(true)
        // O `.gitignore` vale mesmo fora de um repositório git.
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() > 0)
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?;
            let path = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            Some(FileEntry { path, is_dir })
        })
        .take(MAX_ENTRIES)
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn paths(entries: &[FileEntry]) -> Vec<(&str, bool)> {
        entries
            .iter()
            .map(|e| (e.path.as_str(), e.is_dir))
            .collect()
    }

    #[test]
    fn scan_lists_files_and_dirs_respecting_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("src/ui/app.rs"), "").unwrap();
        fs::write(root.join("target/debug/terra"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();

        assert_eq!(
            paths(&scan(root)),
            [
                ("src", true),
                ("src/main.rs", false),
                ("src/ui", true),
                ("src/ui/app.rs", false),
            ]
        );
    }

    #[test]
    fn watch_picks_up_new_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        let index = FileIndex::watch(dir.path().to_path_buf());

        let wait_for = |path: &str| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if index.snapshot().iter().any(|e| e.path == path) {
                    return true;
                }
                thread::sleep(Duration::from_millis(50));
            }
            false
        };
        assert!(wait_for("a.txt"), "lista inicial");
        fs::create_dir(dir.path().join("novo")).unwrap();
        fs::write(dir.path().join("novo/b.rs"), "").unwrap();
        assert!(wait_for("novo/b.rs"), "arquivo criado depois aparece");
    }
}
