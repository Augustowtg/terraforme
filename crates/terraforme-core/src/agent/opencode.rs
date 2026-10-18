//! Adapter do OpenCode (`opencode`, versão 2 ou mais nova).
//!
//! Mesma ideia do Claude Code: RTK e AI Memory viram plugins do terraforme
//! (`~/.config/terraforme/plugins/opencode/terraforme-<extensão>`), ligados
//! por sessão conforme o modo. O OpenCode carrega sozinho tudo o que estiver
//! em `~/.config/opencode/plugins/`, então os plugins que o `rtk init
//! --opencode` e o `ai-memory install-hooks --agent opencode2` gravam lá saem
//! de lá (ficam como `<arquivo>.terraforme-bak`), e o mesmo vale para o
//! servidor MCP do AI Memory no `opencode.json` (backup em
//! `opencode.json.terraforme-bak`). A sessão recebe os plugins e o MCP do modo
//! pela `OPENCODE_CONFIG_CONTENT`, uma camada de configuração a mais que não
//! mexe em arquivo nenhum.
//!
//! - RTK: o plugin do `rtk init --opencode` usa o formato do OpenCode 1
//!   (export nomeado), que o 2 recusa; o terraforme usa o dele,
//!   [`RTK_PLUGIN`], que faz o mesmo (`rtk rewrite` em cada comando do shell).
//! - AI Memory: o plugin é o do próprio AI Memory (`ai-memory-opencode2.ts`,
//!   gerado na primeira vez se ainda não existir); o MCP vai pela ponte stdio
//!   (`ai-memory mcp-bridge`), que se autentica sozinha.
//!
//! Sem `--standalone`, o `opencode` conversa com um serviço em segundo plano
//! que já roda com a configuração dele; com a flag, cada sessão sobe o próprio
//! servidor, que é quem lê a `OPENCODE_CONFIG_CONTENT` (e fica dentro do AI
//! Jail junto com o agente).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use super::{Adapter, Mode, Setup, home, read_json, write_json, write_private};
use crate::config::Config;
use crate::extension::{Extension, strip_ansi};
use crate::status::{State, find_in_path, version_of};
use crate::usage::{OpenCodeStats, Summary, Tokens, Usage, UsageSource};

/// Variável com a configuração extra da sessão (JSON).
const CONFIG_CONTENT: &str = "OPENCODE_CONFIG_CONTENT";
/// Arquivo de entrada de cada plugin do terraforme.
const ENTRY: &str = "index.ts";
/// Servidor do AI Memory quando o plugin não diz outro.
const AI_MEMORY_SERVER: &str = "http://127.0.0.1:49374";
/// Sufixo dos arquivos globais tirados do caminho (o OpenCode os ignora).
const BACKUP_SUFFIX: &str = "terraforme-bak";

/// Plugin do RTK no formato do OpenCode 2.
pub(crate) const RTK_PLUGIN: &str = r#"// Plugin do RTK para o OpenCode 2, gravado pelo terraforme a cada execução —
// não edite. O `rtk init --opencode` gera um plugin no formato do OpenCode 1,
// que o 2 não carrega; este faz o mesmo: passa cada comando do shell pelo
// `rtk rewrite`, que é quem decide a reescrita.
import { spawnSync } from "node:child_process"

export default {
  id: "terraforme-rtk",
  setup: async (ctx: any) => {
    const registration = await ctx.tool.hook("execute.before", async (event: any) => {
      const tool = String(event?.tool ?? "").toLowerCase()
      if (tool !== "shell" && tool !== "bash") return
      const input = event?.input
      if (!input || typeof input.command !== "string" || !input.command) return
      // O `rtk rewrite` sai com código diferente de 0 mesmo quando reescreve.
      const result = spawnSync("rtk", ["rewrite", input.command], { encoding: "utf8", timeout: 5000 })
      const rewritten = String(result.stdout ?? "").trim()
      if (rewritten && rewritten !== input.command) input.command = rewritten
    })
    return async () => {
      await registration.dispose()
    }
  },
}
"#;

/// OpenCode (`opencode`).
pub struct OpenCode;

impl Adapter for OpenCode {
    fn bin(&self) -> &'static str {
        "opencode"
    }

    fn setup(&self, mode: Mode) -> Result<Setup> {
        require_v2()?;
        let mut setup = Setup::default();
        let root = plugins_root()?;
        let globals = global_dirs()?;
        move_globals(&globals, &root, &mut setup)?;

        let mut plugins = Vec::new();
        let mut mcp = Map::new();
        for ext in mode.extensions() {
            let dir = plugin_dir(&root, ext);
            match ext {
                Extension::Rtk => write_plugin(ext, &dir, Some(RTK_PLUGIN))?,
                Extension::AiMemory => {
                    if !dir.join(ENTRY).is_file() {
                        generate_ai_memory_plugin(&mut setup);
                        move_globals(&globals, &root, &mut setup)?;
                    }
                    if !dir.join(ENTRY).is_file() {
                        setup.warnings.push(
                            "plugin do AI Memory para o OpenCode não encontrado — rode `ai-memory install-hooks --agent opencode2 --apply`".into(),
                        );
                        continue;
                    }
                    write_plugin(ext, &dir, None)?;
                    mcp.insert("memory".into(), ai_memory_bridge(&dir)?);
                }
                Extension::AiJail => continue,
            }
            plugins.push(Value::String(dir.display().to_string()));
            setup.plugins.push((ext, dir));
        }

        let mut content = json!({ "plugins": plugins });
        if !mcp.is_empty() {
            content["mcp"] = json!({ "servers": mcp });
        }
        setup.env.push((CONFIG_CONTENT, content.to_string().into()));
        // Depois dos argumentos repassados: o `opencode run` só aceita a flag
        // depois do subcomando.
        setup.trailing_args.push("--standalone".into());

        if let Some(bin) = find_in_path(self.bin()) {
            let (launcher, maps) = write_jail_launcher(&root, &bin)?;
            setup.jail_path = launcher.parent().map(Path::to_path_buf);
            setup.jail_maps = maps;
        }
        Ok(setup)
    }

    /// `--continue` e `--session` já escolhem a sessão.
    fn picks_session(&self, args: &[String]) -> bool {
        args.iter().any(|arg| {
            let flag = arg.split('=').next().unwrap_or_default();
            matches!(flag, "-c" | "--continue" | "-s" | "--session")
        })
    }

    fn usage(
        &self,
        cwd: &Path,
        _launched: std::time::SystemTime,
        _session: Option<&str>,
    ) -> Result<Usage> {
        collect_usage(cwd)
    }

    fn forget(&self, ext: Extension) -> Result<()> {
        let plugin = plugin_dir(&plugins_root()?, ext);
        if plugin.exists() {
            fs::remove_dir_all(&plugin)
                .with_context(|| format!("removendo {}", plugin.display()))?;
        }
        Ok(())
    }
}

/// Consulta as estatísticas oficiais do OpenCode. `stats` não oferece uma
/// janela de cinco horas nem o detalhamento por sessão/subagente do Claude.
fn collect_usage(cwd: &Path) -> Result<Usage> {
    let (project_day, project_stats) = fetch_stats(cwd, "0", true)?;
    let (all_day, day_stats) = fetch_stats(cwd, "0", false)?;
    let (all_week, week_stats) = fetch_stats(cwd, "7", false)?;

    Ok(Usage {
        source: UsageSource::OpenCode,
        project_day,
        all_day,
        all_week,
        opencode: Some(OpenCodeStats {
            sessions: week_stats.sessions,
            prompts: week_stats.prompts,
            subagents: week_stats.subagents,
            project_today_cost: project_stats.cost,
            today_cost: day_stats.cost,
            week_cost: week_stats.cost,
        }),
        ..Usage::default()
    })
}

#[derive(Debug, Default, PartialEq)]
struct StatsMeta {
    sessions: u64,
    prompts: u64,
    subagents: u64,
    cost: f64,
}

fn fetch_stats(cwd: &Path, days: &str, project_only: bool) -> Result<(Summary, StatsMeta)> {
    let mut command = Command::new("opencode");
    command
        .args(["stats", "--json", "--days", days, "--models", "--cost"])
        .current_dir(cwd);
    if project_only {
        command.args(["--project", "."]);
    }
    let output = command.output().context("executando `opencode stats`")?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "`opencode stats` terminou com erro{}",
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .context("a saída de `opencode stats --json` não é um JSON válido")?;
    parse_stats(&value)
}

fn parse_stats(value: &Value) -> Result<(Summary, StatsMeta)> {
    let token_value = value
        .get("tokens")
        .context("a saída de `opencode stats` não contém tokens")?;
    let token_count =
        |parent: &Value, key: &str| parent.get(key).and_then(Value::as_u64).unwrap_or_default();
    let tokens = |value: &Value| Tokens {
        input: token_count(value, "input"),
        output: token_count(value, "output"),
        reasoning: token_count(value, "reasoning"),
        cache_write: value
            .pointer("/cache/write")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_read: value
            .pointer("/cache/read")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    };
    let mut summary = Summary {
        tokens: tokens(token_value),
        responses: token_count(value, "steps") as usize,
        ..Summary::default()
    };
    if let Some(models) = value.get("models").and_then(Value::as_array) {
        for model in models {
            let Some(model_id) = model.pointer("/model/id").and_then(Value::as_str) else {
                continue;
            };
            let provider = model
                .pointer("/model/providerID")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = if provider.is_empty() {
                model_id.to_string()
            } else {
                format!("{provider}/{model_id}")
            };
            summary
                .by_model
                .insert(name, tokens(model.get("tokens").unwrap_or(&Value::Null)));
        }
    }
    let stats = StatsMeta {
        sessions: token_count(value, "sessions"),
        prompts: token_count(value, "prompts"),
        subagents: token_count(value, "subagents"),
        cost: value
            .get("cost")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
    };
    Ok((summary, stats))
}

/// Os plugins do OpenCode 1 têm outro formato, e o terraforme não os gera.
fn require_v2() -> Result<()> {
    let Some(bin) = find_in_path("opencode") else {
        return Ok(());
    };
    let major = version_of(&bin).and_then(|v| v.split('.').next()?.parse::<u32>().ok());
    if let Some(major) = major
        && major < 2
    {
        bail!("o terraforme precisa do OpenCode 2 ou mais novo — rode `opencode upgrade`");
    }
    Ok(())
}

/// `~/.config/terraforme/plugins/opencode`.
fn plugins_root() -> Result<PathBuf> {
    Ok(Config::dir()?.join("plugins").join("opencode"))
}

/// Grava `<root>/bin/opencode`, que roda o `opencode` real no AI Jail, e
/// devolve o caminho dele e o que a sandbox precisa montar para ele.
///
/// Na rede filtrada do ai-jail, o `HTTP_PROXY` aponta para um proxy que só
/// aceita CONNECT e responde 405 ao HTTP simples que o cliente do OpenCode
/// manda para o próprio servidor (`127.0.0.1`), e o `NO_PROXY` vem vazio. Sem
/// `HTTP_PROXY` essa conexão vai direto; a API (HTTPS) continua passando pelo
/// proxy, e HTTP para fora não tem rota na sandbox. A pasta do lançador vai
/// na frente do `PATH` da sandbox e o comando continua `opencode`, para o
/// ai-jail aplicar o preset e a seção `[commands.opencode]` do `~/.ai-jail`.
fn write_jail_launcher(root: &Path, bin: &Path) -> Result<(PathBuf, Vec<PathBuf>)> {
    let real = fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let script = format!(
        "#!/bin/sh\n# Gerado pelo terraforme: roda o OpenCode no AI Jail sem o HTTP_PROXY da\n# rede filtrada, que recusa a conexão do cliente com o próprio servidor.\nunset HTTP_PROXY http_proxy\nexec '{}' \"$@\"\n",
        real.display().to_string().replace('\'', r"'\''")
    );
    let launcher = root.join("bin").join("opencode");
    if fs::read_to_string(&launcher).ok().as_deref() != Some(script.as_str()) {
        write_private(&launcher, script.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700))?;
    }
    // O preset do ai-jail monta ~/.opencode/bin; uma instalação em outro
    // lugar (npm, Homebrew) também precisa aparecer.
    let mut maps = vec![root.join("bin")];
    if let Some(dir) = real.parent() {
        maps.push(dir.to_path_buf());
    }
    Ok((launcher, maps))
}

fn plugin_dir(root: &Path, ext: Extension) -> PathBuf {
    root.join(format!("terraforme-{}", ext.id()))
}

/// Pastas de configuração global do OpenCode que podem ter plugins das
/// extensões: a que ele usa (`OPENCODE_CONFIG_DIR`, `XDG_CONFIG_HOME`) e
/// `~/.config/opencode`, onde o AI Memory sempre grava.
fn global_dirs() -> Result<Vec<PathBuf>> {
    let default = home()?.join(".config").join("opencode");
    let current = env::var_os("OPENCODE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(|d| PathBuf::from(d).join("opencode"))
        })
        .unwrap_or_else(|| default.clone());
    let mut dirs = vec![current];
    if !dirs.contains(&default) {
        dirs.push(default);
    }
    Ok(dirs)
}

/// Move das pastas globais os plugins e o MCP das extensões, avisando o que
/// mudou.
fn move_globals(globals: &[PathBuf], root: &Path, setup: &mut Setup) -> Result<()> {
    for dir in globals {
        for (file, ext) in move_global_plugins(dir, root)? {
            setup.notices.push(format!(
                "plugin do {} tirado de {} — o terraforme o liga por sessão (backup: {}.{BACKUP_SUFFIX})",
                ext.name(),
                file.parent().unwrap_or(dir).display(),
                file.display(),
            ));
        }
        let config = dir.join("opencode.json");
        match move_global_mcp(&config) {
            Ok(Some(backup)) => setup.notices.push(format!(
                "servidor MCP do AI Memory tirado de {} — o terraforme o liga por sessão (backup: {})",
                config.display(),
                backup.display()
            )),
            Ok(None) => {}
            // Ex.: JSON com comentários — melhor não reescrever.
            Err(err) => setup.warnings.push(format!(
                "{err:#} — se houver um servidor MCP do AI Memory lá, ele vale em todos os modos"
            )),
        }
    }
    Ok(())
}

/// Tira de `<global>/plugins` (e `plugin`) os plugins do RTK e do AI Memory,
/// renomeando-os para `<arquivo>.terraforme-bak` (o OpenCode só carrega
/// `.ts`/`.js`). O do AI Memory no formato do OpenCode 2 vira o plugin do
/// terraforme. Devolve os arquivos movidos.
fn move_global_plugins(global: &Path, root: &Path) -> Result<Vec<(PathBuf, Extension)>> {
    let mut moved = Vec::new();
    for sub in ["plugins", "plugin"] {
        let Ok(entries) = fs::read_dir(global.join(sub)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for file in files {
            let Some(ext) = file
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(plugin_owner)
            else {
                continue;
            };
            if ext == Extension::AiMemory {
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("lendo {}", file.display()))?;
                if is_v2_plugin(&text) {
                    let dir = plugin_dir(root, ext);
                    write_private(&dir.join(ENTRY), text.as_bytes())?;
                    write_plugin(ext, &dir, None)?;
                }
            }
            // Só mexe no global depois que o plugin do terraforme está gravado.
            let mut backup = file.as_os_str().to_owned();
            backup.push(format!(".{BACKUP_SUFFIX}"));
            fs::rename(&file, &backup).with_context(|| format!("movendo {}", file.display()))?;
            moved.push((file, ext));
        }
    }
    Ok(moved)
}

/// `rtk.ts` → RTK; `ai-memory-opencode2.ts`, `ai-memory.js`… → AI Memory.
fn plugin_owner(file_name: &str) -> Option<Extension> {
    let stem = file_name
        .strip_suffix(".ts")
        .or_else(|| file_name.strip_suffix(".js"))?;
    if stem == "rtk" {
        Some(Extension::Rtk)
    } else if stem.starts_with("ai-memory") {
        Some(Extension::AiMemory)
    } else {
        None
    }
}

/// O OpenCode 2 exige `export default { id, setup }`; o 1 usava export nomeado.
fn is_v2_plugin(text: &str) -> bool {
    text.contains("export default")
}

/// Tira do `opencode.json` o servidor MCP `ai-memory` (`mcp.servers` no
/// OpenCode 2, direto em `mcp` no 1). Devolve o caminho do backup, ou `None`
/// se não havia nada.
fn move_global_mcp(config: &Path) -> Result<Option<PathBuf>> {
    let mut value = read_json(config)?;
    let mut removed = false;
    if let Some(mcp) = value.get_mut("mcp").and_then(Value::as_object_mut) {
        removed |= mcp.remove("ai-memory").is_some();
        if let Some(servers) = mcp.get_mut("servers").and_then(Value::as_object_mut) {
            removed |= servers.remove("ai-memory").is_some();
        }
    }
    if !removed {
        return Ok(None);
    }
    let backup = config.with_extension(format!("json.{BACKUP_SUFFIX}"));
    fs::copy(config, &backup).with_context(|| format!("criando backup {}", backup.display()))?;
    write_json(config, &value)?;
    Ok(Some(backup))
}

/// Grava o `package.json` do plugin e, com `entry`, o `index.ts` (só se mudou).
fn write_plugin(ext: Extension, dir: &Path, entry: Option<&str>) -> Result<()> {
    if let Some(entry) = entry {
        let path = dir.join(ENTRY);
        if fs::read_to_string(&path).ok().as_deref() != Some(entry) {
            write_private(&path, entry.as_bytes())?;
        }
    }
    let manifest = json!({
        "name": format!("terraforme-{}", ext.id()),
        "version": crate::VERSION,
        "description": format!("{} gerenciado pelo terraforme", ext.name()),
        "type": "module",
        "main": ENTRY,
    });
    let path = dir.join("package.json");
    if read_json(&path).ok().as_ref() != Some(&manifest) {
        write_json(&path, &manifest)?;
    }
    Ok(())
}

/// Pede ao AI Memory o plugin dele para o OpenCode 2, que vai para
/// `~/.config/opencode/plugins` (de onde [`move_globals`] o tira).
fn generate_ai_memory_plugin(setup: &mut Setup) {
    let State::Installed { path, .. } = Extension::AiMemory.check().state else {
        return;
    };
    let output = Command::new(&path)
        .args(["install-hooks", "--agent", "opencode2", "--apply"])
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => setup
            .notices
            .push("plugin do AI Memory para o OpenCode criado (`ai-memory install-hooks --agent opencode2`)".into()),
        Ok(out) => setup.warnings.push(format!(
            "`ai-memory install-hooks --agent opencode2` falhou: {}",
            strip_ansi(&String::from_utf8_lossy(&out.stderr)).trim()
        )),
        Err(err) => setup
            .warnings
            .push(format!("falha ao executar `{}`: {err}", path.display())),
    }
}

/// Servidor MCP `memory` pela ponte stdio do AI Memory, no mesmo servidor que
/// o plugin usa.
fn ai_memory_bridge(plugin: &Path) -> Result<Value> {
    let bin = match Extension::AiMemory.check().state {
        State::Installed { path, .. } => path,
        _ => bail!("AI Memory não está instalado"),
    };
    let text = fs::read_to_string(plugin.join(ENTRY)).unwrap_or_default();
    let server = plugin_server(&text).unwrap_or(AI_MEMORY_SERVER);
    Ok(json!({
        "type": "local",
        "command": [
            bin.display().to_string(),
            "mcp-bridge",
            "--server-url",
            format!("{}/mcp", server.trim_end_matches('/')),
        ],
    }))
}

/// `const SERVER = "http://127.0.0.1:49374"…` no plugin do AI Memory.
fn plugin_server(text: &str) -> Option<&str> {
    let rest = &text[text.find("const SERVER = \"")? + "const SERVER = \"".len()..];
    let server = &rest[..rest.find('"')?];
    (!server.is_empty()).then_some(server)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            env::temp_dir().join(format!("terraforme-opencode-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn identifies_plugin_owner() {
        assert_eq!(plugin_owner("rtk.ts"), Some(Extension::Rtk));
        assert_eq!(
            plugin_owner("ai-memory-opencode2.ts"),
            Some(Extension::AiMemory)
        );
        assert_eq!(plugin_owner("ai-memory.js"), Some(Extension::AiMemory));
        assert_eq!(plugin_owner("rtk.ts.terraforme-bak"), None);
        assert_eq!(plugin_owner("outro.ts"), None);
        assert_eq!(plugin_owner("rtk-extra.ts"), None);
    }

    #[test]
    fn picks_session_detects_session_flags() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!OpenCode.picks_session(&args(&["--prompt", "oi"])));
        assert!(OpenCode.picks_session(&args(&["-c"])));
        assert!(OpenCode.picks_session(&args(&["--session=ses_1"])));
    }

    #[test]
    fn opencode_stats_json_maps_tokens_models_and_cost() {
        let value: Value = serde_json::from_str(
            r#"{
                "sessions": 3,
                "subagents": 2,
                "prompts": 4,
                "steps": 9,
                "cost": 0.125,
                "tokens": {"input": 10, "output": 20, "reasoning": 30,
                    "cache": {"read": 40, "write": 50}},
                "models": [{"model": {"id": "model-x", "providerID": "provider-y"},
                    "tokens": {"input": 10, "output": 20, "reasoning": 30,
                        "cache": {"read": 40, "write": 50}}}]
            }"#,
        )
        .unwrap();
        let (summary, stats) = parse_stats(&value).unwrap();
        assert_eq!(summary.tokens.total(), 150);
        assert_eq!(summary.responses, 9);
        assert_eq!(summary.by_model["provider-y/model-x"].total(), 150);
        assert_eq!(stats.sessions, 3);
        assert_eq!(stats.prompts, 4);
        assert_eq!(stats.subagents, 2);
        assert_eq!(stats.cost, 0.125);
    }

    #[test]
    fn rtk_plugin_uses_the_v2_contract() {
        assert!(is_v2_plugin(RTK_PLUGIN));
        assert!(RTK_PLUGIN.contains("\"execute.before\""));
        assert!(RTK_PLUGIN.contains("\"rewrite\""));
    }

    #[test]
    fn reads_server_from_ai_memory_plugin() {
        let text = "const AGENT = \"x\";\nconst SERVER = \"http://127.0.0.1:5000\".replace(/\\/+$/, \"\");";
        assert_eq!(plugin_server(text), Some("http://127.0.0.1:5000"));
        assert_eq!(plugin_server("nada"), None);
    }

    #[test]
    fn moves_global_plugins_into_terraforme() {
        let base = tmp("plugins");
        let global = base.join("opencode");
        let root = base.join("terraforme");
        fs::create_dir_all(global.join("plugins")).unwrap();
        fs::write(
            global.join("plugins/rtk.ts"),
            "export const Rtk = async () => ({})",
        )
        .unwrap();
        let aim = "export default { id: \"ai-memory-opencode2\", setup: async () => {} }";
        fs::write(global.join("plugins/ai-memory-opencode2.ts"), aim).unwrap();
        fs::write(global.join("plugins/meu.ts"), "export default {}").unwrap();

        let moved = move_global_plugins(&global, &root).unwrap();
        let exts: Vec<_> = moved.iter().map(|(_, e)| *e).collect();
        assert_eq!(exts, [Extension::AiMemory, Extension::Rtk]);
        assert!(global.join("plugins/meu.ts").is_file());
        assert!(global.join("plugins/rtk.ts.terraforme-bak").is_file());
        assert!(!global.join("plugins/rtk.ts").exists());
        let dir = plugin_dir(&root, Extension::AiMemory);
        assert_eq!(fs::read_to_string(dir.join(ENTRY)).unwrap(), aim);
        assert_eq!(read_json(&dir.join("package.json")).unwrap()["main"], ENTRY);
        // O do RTK (formato do OpenCode 1) não é aproveitado.
        assert!(!plugin_dir(&root, Extension::Rtk).join(ENTRY).exists());
        // Segunda execução: nada mais a mover.
        assert!(move_global_plugins(&global, &root).unwrap().is_empty());

        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn jail_launcher_drops_http_proxy_and_runs_the_real_binary() {
        let base = tmp("launcher");
        let real = base.join("o'pencode-real");
        fs::write(&real, "#!/bin/sh\necho \"$HTTP_PROXY|$HTTPS_PROXY|$*\"\n").unwrap();
        fs::set_permissions(&real, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let (launcher, maps) = write_jail_launcher(&base, &real).unwrap();
        assert_eq!(launcher.file_name().unwrap(), "opencode");
        assert!(maps.contains(&base.join("bin")));
        let out = Command::new(&launcher)
            .args(["run", "a b"])
            .env("HTTP_PROXY", "http://p")
            .env("HTTPS_PROXY", "http://s")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "|http://s|run a b\n");

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn moves_ai_memory_mcp_out_of_opencode_json() {
        let base = tmp("mcp");
        let config = base.join("opencode.json");
        let original = json!({
            "model": "x/y",
            "mcp": { "servers": {
                "ai-memory": { "type": "remote", "url": "http://127.0.0.1:49374/mcp" },
                "outro": { "type": "local", "command": ["outro"] }
            } }
        });
        write_json(&config, &original).unwrap();

        let backup = move_global_mcp(&config).unwrap().unwrap();
        assert_eq!(read_json(&backup).unwrap(), original);
        let now = read_json(&config).unwrap();
        assert!(now.pointer("/mcp/servers/ai-memory").is_none());
        assert!(now.pointer("/mcp/servers/outro").is_some());
        assert!(move_global_mcp(&config).unwrap().is_none());
        // Sem arquivo, nada a fazer.
        assert!(
            move_global_mcp(&base.join("nao-existe.json"))
                .unwrap()
                .is_none()
        );

        fs::remove_dir_all(&base).unwrap();
    }
}
