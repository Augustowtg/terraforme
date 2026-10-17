//! Adapter do Claude Code.
//!
//! RTK e AI Memory viram plugins locais do terraforme
//! (`~/.config/terraforme/plugins/terraforme-<extensão>`), carregados por
//! sessão com `--plugin-dir` conforme o modo. Para isso os hooks deles saem de
//! `~/.claude/settings.json` (com backup) — e voltam a sair sempre que um
//! `rtk init -g`/`ai-memory setup` os recolocar lá. O servidor MCP global do
//! AI Memory fica no `~/.claude.json`, que não é alterado: ele é bloqueado por
//! sessão (`deniedMcpServers`) e o plugin traz uma cópia quando o modo o usa.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use super::{Adapter, Mode, Setup};
use crate::config::Config;
use crate::extension::Extension;
use crate::limits::Limit;
use crate::usage::Usage;

/// Nome do servidor MCP que o `ai-memory setup` registra no Claude.
const AI_MEMORY_MCP: &str = "ai-memory";

/// Claude Code (`claude`).
pub struct Claude;

impl Adapter for Claude {
    fn bin(&self) -> &'static str {
        "claude"
    }

    fn setup(&self, mode: Mode) -> Result<Setup> {
        let claude_dir = claude_config_dir()?;
        let mut setup = Setup::default();

        let settings_path = claude_dir.join("settings.json");
        if let Some((exts, backup)) = migrate_global_hooks(&settings_path)? {
            let names: Vec<_> = exts.iter().map(|e| e.name()).collect();
            setup.notices.push(format!(
                "hooks de {} movidos de {} para os plugins do terraforme (backup: {})",
                names.join(" e "),
                settings_path.display(),
                backup.display()
            ));
        }

        for ext in mode.extensions() {
            let dir = plugin_dir(ext)?;
            ensure_plugin(ext, &dir)?;
            if ext == Extension::AiMemory {
                let claude_json = read_json(&claude_json_path(&claude_dir)?)?;
                match claude_json.pointer(&format!("/mcpServers/{AI_MEMORY_MCP}")) {
                    Some(server) => write_json(
                        &dir.join(".mcp.json"),
                        &json!({ "mcpServers": { "memory": server } }),
                    )?,
                    None if !dir.join(".mcp.json").exists() => setup.warnings.push(
                        "servidor MCP do AI Memory não encontrado — rode `ai-memory setup`".into(),
                    ),
                    None => {}
                }
            }
            if !has_hooks(&dir) {
                setup.warnings.push(format!(
                    "nenhum hook do {} encontrado — rode `{}`",
                    ext.name(),
                    setup_command(ext)
                ));
            }
            setup.args.push("--plugin-dir".into());
            setup.args.push(dir.clone().into());
            setup.plugins.push((ext, dir));
        }

        // O MCP global do AI Memory valeria em todos os modos; o do plugin o substitui.
        setup.args.push("--settings".into());
        setup.args.push(
            json!({ "deniedMcpServers": [{ "serverName": AI_MEMORY_MCP }] })
                .to_string()
                .into(),
        );
        Ok(setup)
    }

    fn session_args(&self, session: &str) -> Option<Vec<String>> {
        Some(vec!["--session-id".into(), session.into()])
    }

    /// `--continue`, `--resume` e `--session-id` já escolhem a sessão.
    fn picks_session(&self, args: &[String]) -> bool {
        args.iter().any(|arg| {
            let flag = arg.split('=').next().unwrap_or_default();
            matches!(
                flag,
                "-c" | "--continue" | "-r" | "--resume" | "--session-id"
            )
        })
    }

    fn usage(&self, cwd: &Path, launched: SystemTime, session: Option<&str>) -> Result<Usage> {
        crate::usage::collect(cwd, launched, session)
    }

    fn plan_limits(&self) -> Result<Option<Vec<Limit>>> {
        crate::limits::fetch_claude_plan()
    }

    fn forget(&self, ext: Extension) -> Result<()> {
        let plugin = plugin_dir(ext)?;
        if plugin.exists() {
            fs::remove_dir_all(&plugin)
                .with_context(|| format!("removendo {}", plugin.display()))?;
        }
        Ok(())
    }
}

/// Pasta do plugin do terraforme para a extensão.
fn plugin_dir(ext: Extension) -> Result<PathBuf> {
    Ok(Config::dir()?
        .join("plugins")
        .join(format!("terraforme-{}", ext.id())))
}

fn setup_command(ext: Extension) -> &'static str {
    match ext {
        Extension::Rtk => "rtk init -g",
        Extension::AiMemory => "ai-memory setup",
        Extension::AiJail => "terraforme extension ai-jail --install",
    }
}

/// Tira de `settings.json` os hooks do RTK e do AI Memory e os grava nos
/// plugins. Devolve as extensões movidas e o caminho do backup, ou `None` se
/// não havia nada a mover.
fn migrate_global_hooks(settings_path: &Path) -> Result<Option<(Vec<Extension>, PathBuf)>> {
    let mut settings = read_json(settings_path)?;
    let extracted = extract_hooks(&mut settings);
    if extracted.is_empty() {
        return Ok(None);
    }

    let backup = settings_path.with_extension("json.terraforme-bak");
    fs::copy(settings_path, &backup)
        .with_context(|| format!("criando backup {}", backup.display()))?;

    let mut moved = Vec::new();
    for (ext, hooks) in extracted {
        let dir = plugin_dir(ext)?;
        ensure_plugin(ext, &dir)?;
        write_json(&dir.join("hooks/hooks.json"), &json!({ "hooks": hooks }))?;
        moved.push(ext);
    }

    // Só altera o global depois que os plugins estão gravados.
    write_json(settings_path, &settings)?;
    Ok(Some((moved, backup)))
}

/// Remove de `settings` os hooks de cada extensão, devolvendo-os no mesmo
/// formato (`evento → [{ matcher, hooks }]`).
fn extract_hooks(settings: &mut Value) -> Vec<(Extension, Map<String, Value>)> {
    let mut extracted: Vec<(Extension, Map<String, Value>)> = Vec::new();
    let Some(events) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return extracted;
    };

    for (event, groups) in events.iter_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        for group in groups.iter_mut() {
            let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            let mut owned: Vec<(Extension, Value)> = Vec::new();
            hooks.retain(|hook| {
                let command = hook.get("command").and_then(Value::as_str).unwrap_or("");
                match hook_owner(command) {
                    Some(ext) => {
                        owned.push((ext, hook.clone()));
                        false
                    }
                    None => true,
                }
            });
            for ext in Extension::ALL {
                let mine: Vec<Value> = owned
                    .iter()
                    .filter(|(e, _)| *e == ext)
                    .map(|(_, h)| h.clone())
                    .collect();
                if mine.is_empty() {
                    continue;
                }
                let mut copy = group.clone();
                copy["hooks"] = Value::Array(mine);
                let slot = match extracted.iter().position(|(e, _)| *e == ext) {
                    Some(i) => i,
                    None => {
                        extracted.push((ext, Map::new()));
                        extracted.len() - 1
                    }
                };
                let list = extracted[slot]
                    .1
                    .entry(event.clone())
                    .or_insert_with(|| json!([]));
                list.as_array_mut().expect("lista de grupos").push(copy);
            }
        }
        groups.retain(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|h| !h.is_empty())
        });
    }
    events.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));

    extracted
}

/// `rtk hook claude` → RTK; `/…/ai-memory --data-dir … hook --event …` → AI Memory.
fn hook_owner(command: &str) -> Option<Extension> {
    let mut words = command
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '"' || c == '\''));
    let program = Path::new(words.next()?).file_stem()?.to_str()?;
    // Só as extensões dos modos têm hooks (o AI Jail envolve o agente).
    let ext = Mode::Hybrid
        .extensions()
        .into_iter()
        .find(|e| e.id() == program)?;
    words.any(|w| w == "hook").then_some(ext)
}

/// Cria o manifesto do plugin e, para o RTK, o hook padrão do `rtk init -g`
/// quando ainda não há nenhum.
fn ensure_plugin(ext: Extension, dir: &Path) -> Result<()> {
    let manifest = dir.join(".claude-plugin/plugin.json");
    if !manifest.exists() {
        write_json(
            &manifest,
            &json!({
                "name": format!("terraforme-{}", ext.id()),
                "version": crate::VERSION,
                "description": format!("{} gerenciado pelo terraforme", ext.name()),
            }),
        )?;
    }
    if ext == Extension::Rtk && !has_hooks(dir) {
        write_json(
            &dir.join("hooks/hooks.json"),
            &json!({ "hooks": { "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rtk hook claude" }] }
            ] } }),
        )?;
    }
    Ok(())
}

fn has_hooks(plugin: &Path) -> bool {
    read_json(&plugin.join("hooks/hooks.json"))
        .ok()
        .and_then(|v| {
            v.get("hooks")
                .and_then(Value::as_object)
                .map(|h| !h.is_empty())
        })
        .unwrap_or(false)
}

pub(crate) fn claude_config_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(home()?.join(".claude"))
}

/// `~/.claude.json`, ou `$CLAUDE_CONFIG_DIR/.claude.json` quando definido.
fn claude_json_path(claude_dir: &Path) -> Result<PathBuf> {
    if env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
        return Ok(claude_dir.join(".claude.json"));
    }
    Ok(home()?.join(".claude.json"))
}

fn home() -> Result<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("pasta do usuário não encontrada")
}

fn read_json(path: &Path) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("lendo {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
    }
}

/// Grava num arquivo temporário e renomeia, para nunca deixar um JSON pela
/// metade. Os arquivos podem ter segredos (o `.mcp.json` copia o servidor do
/// AI Memory), então só o próprio usuário os lê.
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let dir = path.parent().context("caminho sem pasta")?;
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options
        .open(&tmp)
        .with_context(|| format!("gravando {}", tmp.display()))?;
    serde_json::to_writer_pretty(&file, value)?;
    file.sync_all()?;

    fs::rename(&tmp, path).with_context(|| format!("gravando {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_session_detects_session_flags() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!Claude.picks_session(&args(&[])));
        assert!(!Claude.picks_session(&args(&["-p", "oi", "--model", "opus"])));
        assert!(Claude.picks_session(&args(&["--continue"])));
        assert!(Claude.picks_session(&args(&["-r"])));
        assert!(Claude.picks_session(&args(&["--resume=abc"])));
        assert!(Claude.picks_session(&args(&["--session-id", "x"])));
    }

    fn settings() -> Value {
        json!({
            "model": "opus",
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rtk hook claude" }] },
                    { "matcher": "", "hooks": [{ "type": "command",
                        "command": "/home/u/.local/lib/ai-memory/ai-memory --data-dir /d hook --event pre-tool-use" }] }
                ],
                "SessionStart": [
                    { "hooks": [{ "type": "command", "command": "\"/home/u/.claude/hooks/heal.mjs\"" }] }
                ],
                "Stop": [
                    { "matcher": "", "hooks": [
                        { "type": "command", "command": "ai-memory hook --event stop" },
                        { "type": "command", "command": "/x/timeline-stop-hook" }
                    ] }
                ]
            }
        })
    }

    #[test]
    fn identifies_hook_owner() {
        assert_eq!(hook_owner("rtk hook claude"), Some(Extension::Rtk));
        assert_eq!(
            hook_owner("/x/ai-memory --data-dir /d hook --event stop"),
            Some(Extension::AiMemory)
        );
        assert_eq!(hook_owner("rtk gain"), None);
        assert_eq!(hook_owner("ai-jail hook"), None);
        assert_eq!(hook_owner("\"/home/u/.claude/hooks/heal.mjs\""), None);
    }

    #[test]
    fn extracts_extension_hooks_and_keeps_the_rest() {
        let mut s = settings();
        let extracted = extract_hooks(&mut s);

        assert_eq!(
            s,
            json!({
                "model": "opus",
                "hooks": {
                    "SessionStart": [
                        { "hooks": [{ "type": "command", "command": "\"/home/u/.claude/hooks/heal.mjs\"" }] }
                    ],
                    "Stop": [
                        { "matcher": "", "hooks": [{ "type": "command", "command": "/x/timeline-stop-hook" }] }
                    ]
                }
            })
        );

        let get = |ext| {
            extracted
                .iter()
                .find(|(e, _)| *e == ext)
                .map(|(_, h)| Value::Object(h.clone()))
                .unwrap()
        };
        assert_eq!(
            get(Extension::Rtk),
            json!({ "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rtk hook claude" }] }
            ] })
        );
        let aim = get(Extension::AiMemory);
        assert_eq!(aim["PreToolUse"][0]["matcher"], "");
        assert_eq!(
            aim["Stop"][0]["hooks"],
            json!([{ "type": "command", "command": "ai-memory hook --event stop" }])
        );
    }

    #[test]
    fn nothing_to_extract_leaves_settings_untouched() {
        let mut s = json!({ "model": "opus" });
        assert!(extract_hooks(&mut s).is_empty());
        assert_eq!(s, json!({ "model": "opus" }));
    }

    #[test]
    fn migration_moves_hooks_into_plugins() {
        let tmp = env::temp_dir().join(format!("terraforme-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let settings_path = tmp.join("settings.json");
        write_json(&settings_path, &settings()).unwrap();

        // SAFETY: único teste que altera XDG_CONFIG_HOME/APPDATA.
        unsafe {
            env::set_var("XDG_CONFIG_HOME", &tmp);
            env::set_var("APPDATA", &tmp);
        }
        let (moved, backup) = migrate_global_hooks(&settings_path).unwrap().unwrap();
        assert_eq!(moved, [Extension::Rtk, Extension::AiMemory]);
        assert_eq!(read_json(&backup).unwrap(), settings());
        assert!(
            !fs::read_to_string(&settings_path)
                .unwrap()
                .contains("ai-memory")
        );
        for ext in moved {
            let dir = plugin_dir(ext).unwrap();
            assert!(dir.join(".claude-plugin/plugin.json").is_file());
            assert!(has_hooks(&dir));
        }
        // Segunda execução: nada mais a mover.
        assert!(migrate_global_hooks(&settings_path).unwrap().is_none());

        fs::remove_dir_all(&tmp).unwrap();
    }
}
