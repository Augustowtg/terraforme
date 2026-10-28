//! Onde todo mundo se encontra: o socket (ou o pipe, no Windows), o
//! `hub.lock` e o `hub.log`. O usuário vai no nome, para dois usuários na
//! mesma máquina não se misturarem.
//!
//! | SO      | Socket/pipe                                                                 | Lockfile                             |
//! | ------- | --------------------------------------------------------------------------- | ------------------------------------ |
//! | Linux   | `$XDG_RUNTIME_DIR/terraforme/hub.sock` (sem ele, `/tmp/terraforme-<uid>/`)  | `~/.terraforme/hub.lock`             |
//! | macOS   | `/tmp/terraforme-<uid>/hub.sock` (curto de propósito)                       | `~/.terraforme/hub.lock`             |
//! | Windows | `\\.\pipe\terraforme-<usuario>-hub`                                         | `%LOCALAPPDATA%\terraforme\hub.lock` |

use std::fs;
use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

/// Os endereços do hub deste usuário.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// O caminho do socket (Unix) ou o nome do pipe (Windows).
    pub socket: PathBuf,
    /// A pasta do socket, criada com 0700 (só no Unix).
    pub socket_dir: Option<PathBuf>,
    /// O `hub.lock`, com o pid, o endereço e as versões do hub vivo.
    pub lock: PathBuf,
    /// O log do hub, que não tem terminal.
    pub log: PathBuf,
}

impl Endpoint {
    /// Os endereços deste usuário neste SO.
    pub fn current() -> Result<Self> {
        let data = data_dir()?;
        let (socket, socket_dir) = socket_path();
        Ok(Self {
            socket,
            socket_dir,
            lock: data.join("hub.lock"),
            log: data.join("hub.log"),
        })
    }

    /// Tudo dentro de `dir` (para os testes).
    #[cfg(test)]
    pub fn in_dir(dir: &std::path::Path) -> Self {
        #[cfg(unix)]
        let (socket, socket_dir) = (dir.join("hub.sock"), Some(dir.to_path_buf()));
        #[cfg(windows)]
        let (socket, socket_dir) = {
            let tag = dir.file_name().unwrap_or_default().to_string_lossy();
            (
                PathBuf::from(format!(r"\\.\pipe\terraforme-test-{tag}")),
                None,
            )
        };
        Self {
            socket,
            socket_dir,
            lock: dir.join("hub.lock"),
            log: dir.join("hub.log"),
        }
    }

    /// O nome do socket/pipe para o `interprocess`.
    pub fn name(&self) -> io::Result<Name<'_>> {
        self.socket.as_path().to_fs_name::<GenericFilePath>()
    }

    /// Cria as pastas do socket (0700) e do lock.
    pub fn prepare(&self) -> Result<()> {
        if let Some(dir) = &self.socket_dir {
            fs::create_dir_all(dir)
                .with_context(|| format!("não foi possível criar {}", dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
        }
        if let Some(dir) = self.lock.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("não foi possível criar {}", dir.display()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("não foi possível encontrar o %LOCALAPPDATA%")?
        .join("terraforme"))
}

#[cfg(not(windows))]
fn data_dir() -> Result<PathBuf> {
    crate::db::data_dir()
}

#[cfg(target_os = "linux")]
fn socket_path() -> (PathBuf, Option<PathBuf>) {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("terraforme"))
        .unwrap_or_else(tmp_dir);
    (dir.join("hub.sock"), Some(dir))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn socket_path() -> (PathBuf, Option<PathBuf>) {
    let dir = tmp_dir();
    (dir.join("hub.sock"), Some(dir))
}

#[cfg(unix)]
fn tmp_dir() -> PathBuf {
    // SAFETY: o getuid não falha nem tem efeitos colaterais.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/terraforme-{uid}"))
}

#[cfg(windows)]
fn socket_path() -> (PathBuf, Option<PathBuf>) {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
    let user: String = user
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    (
        PathBuf::from(format!(r"\\.\pipe\terraforme-{user}-hub")),
        None,
    )
}
