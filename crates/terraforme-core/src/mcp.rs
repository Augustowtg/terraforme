//! Ciclo de vida do daemon MCP compartilhado: lock file, tipos comuns e
//! helpers para iniciar ou conectar ao `terraforme-mcp`.

use std::env;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Configuração da seção `[mcp]` do config.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Segundos de tolerância antes de encerrar quando o último cliente sai.
    pub grace_seconds: u64,
    /// Timeout por subagente em segundos.
    pub subagent_timeout_seconds: u64,
    /// Máximo de subagentes simultâneos.
    pub max_simultaneous_subagents: usize,
    /// Serve a UI web das instâncias e mensagens em `http://127.0.0.1:<porta>/ui`.
    pub ui: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            grace_seconds: 5,
            subagent_timeout_seconds: 300,
            max_simultaneous_subagents: 3,
            ui: false,
        }
    }
}

impl McpConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Conteúdo do lock file do daemon MCP.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LockFile {
    pub pid: u32,
    pub port: u16,
}

impl LockFile {
    /// `{data_dir}/terraforme/mcp.lock`.
    pub fn path() -> Result<PathBuf> {
        let base = if cfg!(windows) {
            env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            env::var_os("XDG_DATA_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    crate::agent::home()
                        .ok()
                        .map(|h| h.join(".local").join("share"))
                })
        };
        Ok(base
            .context("pasta de dados do usuário não encontrada")?
            .join("terraforme")
            .join("mcp.lock"))
    }

    /// Lê o lock file; devolve `None` se não existe ou o PID está morto.
    pub fn read() -> Result<Option<Self>> {
        let path = Self::path()?;
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("lendo {}", path.display())),
        };
        let lock: LockFile =
            serde_json::from_str(&text).with_context(|| format!("lendo {}", path.display()))?;
        if !Self::is_pid_alive(lock.pid) {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
        Ok(Some(lock))
    }

    /// Grava o lock file, criando a pasta se necessário.
    pub fn write(pid: u32, port: u16) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(path.parent().context("caminho inválido")?)?;
        let lock = LockFile { pid, port };
        fs::write(&path, serde_json::to_string(&lock)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    /// Remove o lock file, ignorando se não existir.
    pub fn delete() -> Result<()> {
        let path = Self::path()?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removendo {}", path.display())),
        }
    }

    /// `true` se o PID ainda existe no sistema.
    pub fn is_pid_alive(pid: u32) -> bool {
        crate::platform::is_pid_alive(pid)
    }
}

/// Endereço da UI web do daemon que está rodando.
pub fn ui_url(lock: &LockFile) -> String {
    format!("http://127.0.0.1:{}/ui", lock.port)
}

/// `true` se o daemon está rodando e aceitando conexões.
pub fn is_healthy(lock: &LockFile) -> bool {
    if !LockFile::is_pid_alive(lock.pid) {
        return false;
    }
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", lock.port)
            .parse()
            .expect("endereço inválido"),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Verifica se o MCP já está rodando; se não, inicia em background.
pub fn ensure_running(config: &McpConfig) -> Result<LockFile> {
    if let Some(lock) = LockFile::read()? {
        if is_healthy(&lock) {
            return Ok(lock);
        }
        LockFile::delete()?;
    }
    start(config)
}

/// O `terraforme-mcp` instalado ao lado do binário atual (as instalações põem
/// os dois na mesma pasta) ou, sem ele, o do `PATH`.
fn daemon_bin() -> PathBuf {
    let name = format!("terraforme-mcp{}", env::consts::EXE_SUFFIX);
    env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(&name)))
        .filter(|path| path.is_file())
        .or_else(|| crate::status::find_in_path("terraforme-mcp"))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Inicia o `terraforme-mcp` em background e aguarda até 2s pelo lock file.
pub fn start(config: &McpConfig) -> Result<LockFile> {
    let mut cmd = process::Command::new(daemon_bin());
    cmd.arg("--grace-seconds")
        .arg(config.grace_seconds.to_string())
        .arg("--subagent-timeout")
        .arg(config.subagent_timeout_seconds.to_string())
        .arg("--max-subagents")
        .arg(config.max_simultaneous_subagents.to_string())
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());
    // Compartilhado entre as janelas: não pode cair junto com a que o iniciou.
    crate::platform::detach(&mut cmd);
    cmd.spawn()
        .context("falha ao iniciar terraforme-mcp — binário no PATH?")?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        thread::sleep(Duration::from_millis(100));
        if let Some(lock) = LockFile::read()?
            && is_healthy(&lock)
        {
            return Ok(lock);
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    bail!("terraforme-mcp não respondeu em 2s")
}
