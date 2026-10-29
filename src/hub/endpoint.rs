//! Onde todo mundo se encontra: o socket (ou o pipe, no Windows), o
//! `hub.lock` e o `hub.log`. O usuário vai no nome, para dois usuários na
//! mesma máquina não se misturarem.
//!
//! | SO      | Socket/pipe                                                                 | Lockfile                             |
//! | ------- | --------------------------------------------------------------------------- | ------------------------------------ |
//! | Linux   | `$XDG_RUNTIME_DIR/terraforme/hub.sock` (sem ele, `/tmp/terraforme-<uid>/`)  | `~/.terraforme/hub.lock`             |
//! | macOS   | `/tmp/terraforme-<uid>/hub.sock` (curto de propósito)                       | `~/.terraforme/hub.lock`             |
//! | Windows | `\\.\pipe\terraforme-<usuario>-hub`                                         | `%LOCALAPPDATA%\terraforme\hub.lock` |
//!
//! No Windows, o pipe só abre para o usuário e para o sistema
//! ([`owner_only`]). Com [`DIR_ENV`], tudo fica numa pasta só, inclusive a
//! do orquestrador (os testes).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use super::DIR_ENV;

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
    /// A trava do hub vivo: só um processo a tem (ver `daemon::bind`).
    pub guard: PathBuf,
    /// A pasta do terra (`~/.terraforme`): o `orchestrator.toml`, as
    /// mensagens e as notas.
    pub home: PathBuf,
    /// A pasta de [`DIR_ENV`], quando é ela: o hub iniciado a recebe.
    pub dir: Option<PathBuf>,
    /// Como iniciar o hub em vez do `terra __hub` deste executável (os
    /// testes, que não são o `terra`).
    pub launcher: Option<Vec<OsString>>,
}

impl Endpoint {
    /// Os endereços deste usuário neste SO (ou os da pasta de [`DIR_ENV`]).
    pub fn current() -> Result<Self> {
        if let Some(dir) = std::env::var_os(DIR_ENV).filter(|d| !d.is_empty()) {
            return Ok(Self::in_dir(Path::new(&dir)));
        }
        let data = data_dir()?;
        let (socket, socket_dir) = socket_path();
        Ok(Self {
            socket,
            socket_dir,
            lock: data.join("hub.lock"),
            log: data.join("hub.log"),
            guard: data.join("hub.guard"),
            home: crate::db::data_dir()?,
            dir: None,
            launcher: None,
        })
    }

    /// Tudo dentro de `dir`.
    pub fn in_dir(dir: &Path) -> Self {
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
            guard: dir.join("hub.guard"),
            home: dir.to_path_buf(),
            dir: Some(dir.to_path_buf()),
            launcher: None,
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

/// O descritor de segurança do pipe: acesso total ao usuário deste
/// processo e ao sistema, e a mais ninguém (`P` bloqueia a herança da
/// DACL padrão).
#[cfg(windows)]
pub fn owner_only() -> io::Result<interprocess::os::windows::security_descriptor::SecurityDescriptor>
{
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?);
    let sddl = widestring::U16CString::from_str(&sddl).map_err(io::Error::other)?;
    SecurityDescriptor::deserialize(&sddl)
}

/// O SID do usuário deste processo, em texto (`S-1-5-21-…`).
#[cfg(windows)]
fn user_sid() -> io::Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: as chamadas seguem a documentação do Win32: o buffer tem o
    // tamanho que a primeira chamada pediu (em u64, alinhado para o
    // TOKEN_USER), o token fecha e o texto do SID é liberado com LocalFree.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        let err = io::Error::last_os_error();
        CloseHandle(token);
        if ok == 0 {
            return Err(err);
        }
        let user = &*buf.as_ptr().cast::<TOKEN_USER>();
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(sid)
    }
}
