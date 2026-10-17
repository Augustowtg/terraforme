//! AI Jail: roda o agente numa sandbox do sistema (bubblewrap + Landlock no
//! Linux, `sandbox-exec` no macOS), com a pasta do projeto como único lugar
//! gravável.
//!
//! Diferente do RTK e do AI Memory, não é um modo: liga e desliga à parte
//! (`jail` na configuração, `--jail`/`--no-jail` no `terraforme cli`) e
//! combina com qualquer modo. O agente vira `ai-jail … -- claude …`, e o
//! terraforme passa a cada execução o que as extensões do modo precisam
//! enxergar lá dentro: os plugins, os binários, as pastas de dados e a porta
//! do servidor local do AI Memory. Esses mapeamentos dependem da máquina, então
//! ficam fora do `~/.ai-jail` — que o terraforme só cria, com valores padrão,
//! quando ainda não existe.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::extension::Extension;
use crate::status::{State, find_in_path};

/// Primeira versão com `--forward-port`, que o AI Memory usa para falar com
/// o servidor local.
const MIN_VERSION: (u32, u32) = (2, 9);

/// Conteúdo do `~/.ai-jail` criado pelo terraforme.
pub const DEFAULT_CONFIG: &str = r#"# Configuração global do ai-jail, criada pelo terraforme — pode editar à vontade.
# O terraforme passa a cada execução os mapeamentos das extensões (plugins,
# RTK, AI Memory), então eles não precisam ficar aqui.
# Referência: https://github.com/akitaonrails/ai-jail#configuration

# Não grava um .ai-jail na pasta de cada projeto aberto.
no_save_config = true
# O terminal do terraforme já tem abas e consumo; a barra do ai-jail ocuparia
# a última linha do agente.
no_status_bar = true

# Rede: sem nada aqui, só a API do agente e os registros de pacotes passam.
# allow_hosts = ["github.com"]
# network = true

# Chaves SSH (somente leitura) e agente SSH, para git por SSH.
# ssh = true

[commands.claude]
# Credenciais e estado do Claude Code (~/.claude, ~/.claude.json): a sessão
# já começa logada.
agent_state = true
"#;

/// `~/.ai-jail`, a configuração global (confiável) do ai-jail.
pub fn config_path() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".ai-jail"))
        .context("pasta do usuário não encontrada")
}

/// Cria o `~/.ai-jail` com [`DEFAULT_CONFIG`] se ainda não existir. Devolve
/// o caminho quando criou.
pub fn ensure_default_config() -> Result<Option<PathBuf>> {
    let path = config_path()?;
    if path.exists() || path.is_symlink() {
        return Ok(None);
    }
    fs::write(&path, DEFAULT_CONFIG).with_context(|| format!("gravando {}", path.display()))?;
    Ok(Some(path))
}

/// Comando que roda `agent` com `agent_args` dentro do ai-jail, enxergando os
/// `plugins` (extensão → pasta do plugin) do modo. Devolve também avisos para
/// o usuário.
pub(crate) fn wrap(
    agent: &str,
    plugins: &[(Extension, PathBuf)],
    agent_args: &[OsString],
) -> Result<(Command, Vec<String>)> {
    let ext = Extension::AiJail;
    let (bin, version) = match ext.check().state {
        State::Installed { path, version } => (path, version),
        _ => bail!(
            "{} não está instalado — rode `terraforme extension {} --install` ou use --no-jail",
            ext.name(),
            ext.id()
        ),
    };
    if let Some(v) = version.as_deref()
        && parse_version(v).is_some_and(|v| v < MIN_VERSION)
    {
        bail!(
            "o ai-jail v{v} é antigo — o terraforme precisa da {}.{} ou mais nova (`terraforme extension {} --update`)",
            MIN_VERSION.0,
            MIN_VERSION.1,
            ext.id()
        );
    }
    if cfg!(target_os = "linux") && find_in_path("bwrap").is_none() {
        bail!("o ai-jail precisa do `bwrap` — instale o pacote bubblewrap");
    }

    let mut notices = Vec::new();
    if let Some(path) = ensure_default_config()? {
        notices.push(format!(
            "configuração padrão do ai-jail criada em {}",
            path.display()
        ));
    }

    let mut command = Command::new(bin);
    // Sem isso, os mapeamentos abaixo iriam parar num .ai-jail no projeto.
    command.arg("--no-save-config");
    let mut ports = Vec::new();
    for (ext, plugin) in plugins {
        command.arg("--map").arg(plugin);
        let mounts = mounts(*ext)?;
        for path in mounts.read_only {
            command.arg("--map").arg(path);
        }
        for path in mounts.writable {
            command.arg("--rw-map").arg(path);
        }
        // As URLs do servidor local ficam nos hooks e no .mcp.json do plugin.
        for file in ["hooks/hooks.json", ".mcp.json"] {
            if let Ok(text) = fs::read_to_string(plugin.join(file)) {
                ports.extend(loopback_ports(&text));
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    for port in ports {
        command.arg("--forward-port").arg(port.to_string());
    }
    command.arg("--").arg(agent).args(agent_args);

    Ok((command, notices))
}

#[derive(Debug, Default)]
struct Mounts {
    read_only: Vec<PathBuf>,
    writable: Vec<PathBuf>,
}

/// O que a extensão precisa enxergar na sandbox: o binário (e, se for um
/// link, a pasta de destino), a configuração e os dados que ela grava (o
/// histórico do `rtk gain`, a fila de eventos do AI Memory).
fn mounts(ext: Extension) -> Result<Mounts> {
    let mut mounts = Mounts::default();
    if let State::Installed { path, .. } = ext.check().state {
        if let Ok(target) = fs::canonicalize(&path)
            && target != path
        {
            match target.parent() {
                // ~/.local/bin/ai-memory → ~/.local/lib/ai-memory/ai-memory
                Some(dir) if dir.file_name().is_some_and(|n| n == ext.id()) => {
                    mounts.read_only.push(dir.to_path_buf())
                }
                _ => mounts.read_only.push(target),
            }
        }
        mounts.read_only.push(path);
    }
    match ext {
        Extension::Rtk => {
            mounts.read_only.push(config_dir()?.join("rtk"));
            mounts.writable.push(data_dir()?.join("rtk"));
        }
        Extension::AiMemory => mounts.writable.push(data_dir()?.join("ai-memory")),
        Extension::AiJail => {}
    }
    mounts.read_only.retain(|p| p.exists());
    mounts.writable.retain(|p| p.exists());
    Ok(mounts)
}

fn data_dir() -> Result<PathBuf> {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

fn config_dir() -> Result<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

fn xdg_dir(var: &str, fallback: &str) -> Result<PathBuf> {
    if let Some(dir) = env::var_os(var).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    env::var_os("HOME")
        .map(|h| Path::new(&h).join(fallback))
        .context("pasta do usuário não encontrada")
}

/// Portas de `127.0.0.1:PORTA` e `localhost:PORTA` citadas no texto.
fn loopback_ports(text: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for host in ["127.0.0.1:", "localhost:"] {
        for (i, _) in text.match_indices(host) {
            let rest = &text[i + host.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if let Ok(port) = rest[..end].parse() {
                ports.push(port);
            }
        }
    }
    ports
}

/// `2.10.0` → `(2, 10)`.
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.trim_start_matches('v').split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_loopback_ports() {
        let hooks = r#"{"command": "/x/ai-memory hook --server-url http://127.0.0.1:49374"}"#;
        assert_eq!(loopback_ports(hooks), [49374]);
        assert_eq!(
            loopback_ports("http://localhost:8080/mcp e http://127.0.0.1:/x"),
            [8080]
        );
        assert!(loopback_ports("http://example.com:443").is_empty());
    }

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("2.10.0"), Some((2, 10)));
        assert_eq!(parse_version("v1.20.1"), Some((1, 20)));
        assert!(parse_version("1.20.1").unwrap() < MIN_VERSION);
        assert!(parse_version("2.10.0").unwrap() >= MIN_VERSION);
        assert_eq!(parse_version("?"), None);
    }

    #[test]
    fn default_config_is_valid_toml() {
        let value: toml::Table = toml::from_str(DEFAULT_CONFIG).unwrap();
        assert_eq!(value["no_save_config"].as_bool(), Some(true));
        assert_eq!(
            value["commands"]["claude"]["agent_state"].as_bool(),
            Some(true)
        );
    }
}
