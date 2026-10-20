//! AI Jail: roda o agente numa sandbox do sistema (bubblewrap + Landlock no
//! Linux, `sandbox-exec` no macOS), com a pasta do projeto como único lugar
//! gravável.
//!
//! Diferente do RTK e do AI Memory, não é um modo: liga e desliga à parte
//! (`jail` na configuração, `--jail`/`--no-jail` no `terraforme cli`) e
//! combina com qualquer modo e qualquer agente. O agente vira `ai-jail … --
//! claude …` (ou `opencode`), e o terraforme passa a cada execução o que as
//! extensões do modo precisam enxergar lá dentro: os plugins, os binários, as
//! pastas de dados, a porta do servidor local do AI Memory e as variáveis de
//! ambiente do agente. Esses mapeamentos dependem da máquina, então ficam fora
//! do `~/.ai-jail` — que o terraforme só cria, com valores padrão, quando
//! ainda não existe (o [`setup`] completa um que já exista com as seções dos
//! agentes que faltarem).
//!
//! Um `.ai-jail` na pasta do projeto ([`init_project`]) liga a sandbox só
//! nele: o `terraforme cli` aberto ali roda no AI Jail mesmo com `jail =
//! false` na configuração.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::agent::Provider;
use crate::extension::Extension;
use crate::status::{State, find_in_path};

/// Primeira versão com `--forward-port`, que o AI Memory usa para falar com
/// o servidor local.
const MIN_VERSION: (u32, u32) = (2, 9);

/// Arquivo de configuração do ai-jail, tanto o global (`~`) quanto o do projeto.
pub const CONFIG_FILE: &str = ".ai-jail";

/// Início do `~/.ai-jail` criado pelo terraforme; depois vêm as seções dos
/// agentes ([`agent_section`]).
const CONFIG_HEADER: &str = r#"# Configuração global do ai-jail, criada pelo terraforme — pode editar à vontade.
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
"#;

/// Seção `[commands.<agente>]` do `~/.ai-jail` para o agente.
fn agent_section(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some(
            r#"
[commands.claude]
# Credenciais e estado do Claude Code (~/.claude, ~/.claude.json): a sessão
# já começa logada.
agent_state = true
"#,
        ),
        Provider::OpenCode => Some(
            r#"
[commands.opencode]
# Credenciais, sessões e configuração do OpenCode (~/.config/opencode,
# ~/.local/share/opencode): a sessão já começa logada.
agent_state = true
# O ai-jail não libera nenhum host do OpenCode: opencode.ai (Zen e modelos
# grátis) e models.dev (catálogo). Acrescente o do seu provedor, ex.:
# "api.anthropic.com", "api.openai.com", "openrouter.ai".
allow_hosts = ["opencode.ai", "models.dev"]
"#,
        ),
        Provider::Cursor => None,
        Provider::Antigravity => None,
    }
}

/// Conteúdo do `~/.ai-jail` criado pelo terraforme.
pub fn default_config() -> String {
    let mut text = CONFIG_HEADER.to_string();
    for section in Provider::ALL.into_iter().filter_map(agent_section) {
        text.push_str(section);
    }
    text
}

/// `~/.ai-jail`, a configuração global (confiável) do ai-jail.
pub fn config_path() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(CONFIG_FILE))
        .context("pasta do usuário não encontrada")
}

/// Cria o `~/.ai-jail` com [`default_config`] se ainda não existir. Devolve
/// o caminho quando criou.
pub fn ensure_default_config() -> Result<Option<PathBuf>> {
    let path = config_path()?;
    if path.exists() || path.is_symlink() {
        return Ok(None);
    }
    fs::write(&path, default_config()).with_context(|| format!("gravando {}", path.display()))?;
    Ok(Some(path))
}

/// Configura o AI Jail para o terraforme: confere a versão e o `bwrap`, cria
/// o `~/.ai-jail` ou acrescenta nele a seção dos agentes que faltam. Devolve
/// o que foi verificado/feito, uma linha por item.
pub fn setup() -> Result<Vec<String>> {
    let (bin, version) = require()?;
    let mut done = vec![format!(
        "{} {} ({})",
        Extension::AiJail.name(),
        version.map(|v| format!("v{v}")).unwrap_or_default(),
        bin.display()
    )];
    if cfg!(target_os = "linux") {
        done.push("bubblewrap (`bwrap`) encontrado".into());
    }

    let path = config_path()?;
    if ensure_default_config()?.is_some() {
        done.push(format!("{} criado", path.display()));
        return Ok(done);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("lendo {}", path.display()))?;
    let missing = missing_sections(&text)
        .with_context(|| format!("{} não é um TOML válido", path.display()))?;
    if missing.is_empty() {
        done.push(format!(
            "{} já tem a seção de todos os agentes",
            path.display()
        ));
        return Ok(done);
    }
    let mut new = text.clone();
    for &provider in &missing {
        if !new.ends_with('\n') {
            new.push('\n');
        }
        new.push_str(agent_section(provider).unwrap_or_default());
    }
    fs::write(&path, new).with_context(|| format!("gravando {}", path.display()))?;
    let names: Vec<_> = missing.iter().map(|p| p.name()).collect();
    done.push(format!(
        "{}: seção de {} adicionada",
        path.display(),
        names.join(" e ")
    ));
    Ok(done)
}

/// Agentes implementados sem `[commands.<bin>]` na configuração.
fn missing_sections(text: &str) -> Result<Vec<Provider>> {
    let table: toml::Table = toml::from_str(text)?;
    let commands = table.get("commands").and_then(toml::Value::as_table);
    Ok(Provider::ALL
        .into_iter()
        .filter(|p| agent_section(*p).is_some())
        .filter(|p| {
            let bin = p.adapter().map(|a| a.bin()).unwrap_or(p.id());
            commands.is_none_or(|c| !c.contains_key(bin))
        })
        .collect())
}

/// `.ai-jail` da pasta, se houver — aí o `terraforme cli` roda no AI Jail ali.
pub fn project_config(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(CONFIG_FILE);
    path.is_file().then_some(path)
}

/// Cria o `.ai-jail` do projeto em `dir` com [`init_project`] se ainda não
/// houver um; devolve o caminho quando criou. Na pasta do usuário não faz
/// nada, porque ali o `.ai-jail` é a configuração global.
pub fn ensure_project(dir: &Path, agent: &str) -> Result<Option<PathBuf>> {
    if project_config(dir).is_some() || config_path().is_ok_and(|p| p == dir.join(CONFIG_FILE)) {
        return Ok(None);
    }
    init_project(dir, agent).map(Some)
}

/// Cria (ou atualiza) o `.ai-jail` do projeto em `dir` para o `agent`, com o
/// `ai-jail --init`. A partir daí o `terraforme cli` roda nele dentro do AI
/// Jail, e as regras do projeto (pastas, hosts liberados) ficam nesse arquivo.
pub fn init_project(dir: &Path, agent: &str) -> Result<PathBuf> {
    let (bin, _) = require()?;
    // Só a mensagem "Config saved" sai no stdout; os erros continuam no stderr.
    let status = Command::new(&bin)
        .args(["--init", agent])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .status()
        .with_context(|| format!("falha ao executar `{}`", bin.display()))?;
    if !status.success() {
        bail!("`ai-jail --init {agent}` terminou com erro ({status})");
    }
    project_config(dir)
        .with_context(|| format!("o ai-jail não criou {CONFIG_FILE} em {}", dir.display()))
}

/// O ai-jail instalado (caminho e versão), com a versão e o `bwrap` que o
/// terraforme precisa.
fn require() -> Result<(PathBuf, Option<String>)> {
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
    Ok((bin, version))
}

/// Comando que roda `agent` com `agent_args` dentro do ai-jail, enxergando os
/// `plugins` (extensão → pasta do plugin) do modo e as variáveis `env` (o
/// ai-jail só repassa uma lista segura; o valor vem do ambiente do comando).
/// Devolve também avisos para o usuário.
pub(crate) fn wrap(
    agent: &str,
    plugins: &[(Extension, PathBuf)],
    maps: &[PathBuf],
    env: &[&str],
    agent_args: &[OsString],
) -> Result<(Command, Vec<String>)> {
    let (bin, _) = require()?;

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
    // Só o nome: o valor (que pode ser grande) não aparece na linha de comando.
    for name in env {
        command.arg("--env").arg(name);
    }
    for path in maps {
        command.arg("--map").arg(path);
    }
    for map in tool_maps() {
        command.arg("--map").arg(map);
    }
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
        // As URLs do servidor local ficam nos hooks e no .mcp.json do plugin
        // do Claude, e no próprio código do plugin do OpenCode.
        for file in ["hooks/hooks.json", ".mcp.json", "index.ts"] {
            if let Ok(text) = fs::read_to_string(plugin.join(file)) {
                ports.extend(loopback_ports(&text));
            }
        }
    }
    // O daemon MCP do terraforme (conversa entre instâncias, subagentes).
    if let Ok(Some(lock)) = crate::mcp::LockFile::read() {
        ports.push(lock.port);
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

/// Ferramentas que o ai-jail não monta sozinho (ele só traz toolchains como o
/// cargo e o node do nvm): o bun. O instalador dele põe tudo em `~/.bun/bin`,
/// que fica fora do `PATH` na sandbox (a home é privada e o `.bashrc` não
/// entra), então os binários vão para `~/.local/bin`, que já está no `PATH`.
fn tool_maps() -> Vec<OsString> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let bun_dir = env::var_os("BUN_INSTALL")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".bun"), PathBuf::from)
        .join("bin");
    let bin_dir = home.join(".local/bin");
    let mut maps = Vec::new();
    for name in ["bun", "bunx"] {
        let Some(source) = find_in_path(name).or_else(|| {
            let path = bun_dir.join(name);
            path.is_file().then_some(path)
        }) else {
            continue;
        };
        maps.push(bind_into(&source, &bin_dir));
    }
    maps
}

/// Argumento do `--map` que monta `source` em `dir` com o mesmo nome
/// (`ORIGEM:DESTINO`), ou só `source` quando ele já está em `dir`.
fn bind_into(source: &Path, dir: &Path) -> OsString {
    if source.parent() == Some(dir) {
        return source.as_os_str().to_owned();
    }
    let mut map = source.as_os_str().to_owned();
    map.push(":");
    map.push(dir.join(source.file_name().unwrap_or_default()));
    map
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
    fn binds_tools_into_dir() {
        let bin = Path::new("/home/u/.local/bin");
        assert_eq!(
            bind_into(Path::new("/home/u/.bun/bin/bun"), bin),
            "/home/u/.bun/bin/bun:/home/u/.local/bin/bun"
        );
        assert_eq!(
            bind_into(Path::new("/home/u/.local/bin/bun"), bin),
            "/home/u/.local/bin/bun"
        );
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
        let value: toml::Table = toml::from_str(&default_config()).unwrap();
        assert_eq!(value["no_save_config"].as_bool(), Some(true));
        for agent in ["claude", "opencode"] {
            assert_eq!(
                value["commands"][agent]["agent_state"].as_bool(),
                Some(true)
            );
        }
        assert!(missing_sections(&default_config()).unwrap().is_empty());
    }

    #[test]
    fn finds_missing_agent_sections() {
        let only_claude = format!(
            "{CONFIG_HEADER}{}",
            agent_section(Provider::Claude).unwrap()
        );
        assert_eq!(
            missing_sections(&only_claude).unwrap(),
            [Provider::OpenCode]
        );
        assert_eq!(
            missing_sections("no_save_config = true").unwrap(),
            [Provider::Claude, Provider::OpenCode]
        );
        assert!(missing_sections("[commands").is_err());
        // Acrescentar as que faltam dá um TOML válido e completo.
        let fixed = format!(
            "{only_claude}{}",
            agent_section(Provider::OpenCode).unwrap()
        );
        assert!(missing_sections(&fixed).unwrap().is_empty());
    }

    #[test]
    fn project_config_needs_the_file() {
        let dir = env::temp_dir().join(format!("terraforme-jail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(project_config(&dir), None);
        fs::write(dir.join(CONFIG_FILE), "command = [\"claude\"]\n").unwrap();
        assert_eq!(project_config(&dir), Some(dir.join(CONFIG_FILE)));
        // Projeto já configurado: nada a fazer (nem roda o ai-jail).
        assert_eq!(ensure_project(&dir, "claude").unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
