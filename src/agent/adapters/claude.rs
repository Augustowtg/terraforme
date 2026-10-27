//! O Claude CLI (`claude`) como agente: [`ClaudeAdapter`] adapta o CLI ao
//! contrato [`ChatAdapter`] do chat.
//!
//! Cada mensagem roda `claude -p` com entrada e saída em `stream-json`; a
//! partir da segunda, com `--resume <sessão>` para manter a conversa. Uma
//! thread lê a saída (um JSON por linha) e a transforma em [`AgentEvent`]s,
//! que a interface consome sem travar. No modo Yolo, a mesma thread aprova os
//! pedidos de permissão do agente.

mod accounts;
mod catalog;
mod stream;

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

use self::stream::{StreamParser, text};
use crate::agent::{
    AccountOption, AgentEvent, Catalog, ChatAdapter, ModelOption, PermissionMode, Request, Session,
};

const NOT_FOUND: &str =
    "Claude CLI não encontrado no PATH. Instale em https://claude.com/claude-code";

/// O Claude CLI do `PATH` como provedor do chat.
pub struct ClaudeAdapter {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    /// Onde ficam as pastas das contas.
    home: Option<PathBuf>,
}

impl ClaudeAdapter {
    /// Procura o `claude` no `PATH`.
    pub fn detect() -> Self {
        Self {
            bin: find_claude(),
            home: dirs::home_dir(),
        }
    }

    /// A pasta de configuração da conta, ou `None` para a padrão.
    fn account_dir(&self, account: Option<&str>) -> Result<Option<PathBuf>, String> {
        let Some(id) = account else {
            return Ok(None);
        };
        self.home
            .as_deref()
            .and_then(|home| accounts::config_dir(home, id))
            .map(Some)
            .ok_or_else(|| format!("Conta do Claude não encontrada: {id}"))
    }
}

impl ChatAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "Claude"
    }

    fn is_available(&self) -> bool {
        self.bin.is_some()
    }

    fn unavailable_hint(&self) -> String {
        NOT_FOUND.to_string()
    }

    fn configured_mode(&self, cwd: &Path, account: Option<&str>) -> Option<PermissionMode> {
        let dir = self.account_dir(account).ok().flatten();
        configured_mode(cwd, dir.as_deref()).and_then(|name| parse_mode(&name))
    }

    /// A Default e as pastas `~/.claude*` com credenciais.
    fn accounts(&self) -> Vec<AccountOption> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        let default_json = env::var_os("CLAUDE_CONFIG_DIR")
            .map(|dir| PathBuf::from(dir).join(".claude.json"))
            .unwrap_or_else(|| home.join(".claude.json"));
        accounts::discover(home, &default_json)
    }

    /// O `claude` responde ao `initialize` do protocolo do Agent SDK com os
    /// comandos e modelos, sem chamar a API.
    fn load_catalog(&self, account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        let bin = self.bin.clone()?;
        let dir = self.account_dir(account).ok().flatten();
        Some(catalog::load(bin, dir))
    }

    /// Os apelidos que o `claude --model` aceita.
    fn models(&self) -> Vec<ModelOption> {
        [("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")]
            .into_iter()
            .map(|(id, label)| ModelOption {
                id: id.to_string(),
                label: label.to_string(),
            })
            .collect()
    }

    fn model_label(&self, id: &str) -> String {
        display_model(id)
    }

    /// No Yolo, o `claude` roda pedindo permissão (`manual`) e cada pedido é
    /// aprovado na hora pela thread que lê a resposta.
    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String> {
        let bin = self.bin.as_ref().ok_or(NOT_FOUND)?;
        let account_dir = self.account_dir(request.account)?;
        let run = Run::start(bin, request, account_dir.as_deref())?;
        Ok(Box::new(run))
    }
}

/// Valor do `--permission-mode`. No Yolo, o agente pede permissão
/// normalmente e o terra aprova cada pedido.
fn cli_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Agente => "auto",
        PermissionMode::Yolo | PermissionMode::Manual => "manual",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
    }
}

/// Modo a partir do nome do Claude (no `init` e no `defaultMode` dos
/// settings). `default` é o nome antigo de `manual`. O Yolo nunca vem do
/// Claude: `bypassPermissions` não tem equivalente.
fn parse_mode(name: &str) -> Option<PermissionMode> {
    Some(match name {
        "auto" => PermissionMode::Agente,
        "manual" | "default" => PermissionMode::Manual,
        "acceptEdits" => PermissionMode::AcceptEdits,
        "plan" => PermissionMode::Plan,
        _ => return None,
    })
}

/// Procura o executável do Claude CLI no `PATH`.
fn find_claude() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    env::split_paths(&env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// Argumentos do `claude` para uma mensagem.
///
/// Sem MCP, skills e hooks: são o ambiente do Claude Code do usuário (ex.:
/// servidores MCP e plugins instalados) e entram no contexto de toda chamada.
/// Num "oi" isso caía de ~26k para ~13k tokens; as ferramentas embutidas do
/// Claude Code continuam disponíveis.
///
/// Com `auto_allow`, os pedidos de permissão do agente chegam pelo stdin/stdout
/// (`--permission-prompt-tool stdio`) e são aprovados por [`control_response`].
fn cli_args<'a>(
    mode: &'a str,
    session: Option<&'a str>,
    auto_allow: bool,
    model: Option<&'a str>,
    effort: Option<&'a str>,
) -> Vec<&'a str> {
    let mut args = vec![
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        mode,
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ];
    if auto_allow {
        args.extend(["--permission-prompt-tool", "stdio"]);
    }
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    if let Some(effort) = effort {
        args.extend(["--effort", effort]);
    }
    if let Some(id) = session {
        args.extend(["--resume", id]);
    }
    args
}

/// A mensagem do usuário no formato do `--input-format stream-json`.
fn user_message(prompt: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": prompt },
    })
    .to_string()
}

/// Resposta a um pedido de controle do agente (`control_request`), ou `None`
/// se a linha não é um. Com `auto_allow`, todo pedido de permissão é aprovado
/// como veio; sem ele, é negado. Outros pedidos não são suportados.
fn control_response(line: &str, auto_allow: bool) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["type"] != "control_request" {
        return None;
    }
    let request_id = &value["request_id"];
    let request = &value["request"];
    let response = if request["subtype"] == "can_use_tool" {
        let decision = if auto_allow {
            serde_json::json!({ "behavior": "allow", "updatedInput": request["input"] })
        } else {
            serde_json::json!({ "behavior": "deny", "message": "negado pelo terra" })
        };
        serde_json::json!({ "subtype": "success", "request_id": request_id, "response": decision })
    } else {
        serde_json::json!({
            "subtype": "error",
            "request_id": request_id,
            "error": format!("pedido não suportado pelo terra: {}", text(&request["subtype"])),
        })
    };
    Some(serde_json::json!({ "type": "control_response", "response": response }).to_string())
}

/// Modo de permissão padrão configurado no Claude (`permissions.defaultMode`)
/// para o diretório `cwd`, como o próprio `claude` leria. `account_dir` é a
/// pasta da conta escolhida; sem ela, a padrão.
fn configured_mode(cwd: &Path, account_dir: Option<&Path>) -> Option<String> {
    let config_dir = account_dir
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))?;
    configured_mode_in(&config_dir, cwd)
}

/// O arquivo mais específico que define o modo vence: local, projeto,
/// usuário.
fn configured_mode_in(config_dir: &Path, cwd: &Path) -> Option<String> {
    [
        cwd.join(".claude").join("settings.local.json"),
        cwd.join(".claude").join("settings.json"),
        config_dir.join("settings.json"),
    ]
    .iter()
    .find_map(|path| {
        let json: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        json["permissions"]["defaultMode"]
            .as_str()
            .map(str::to_string)
    })
}

/// Uma resposta do agente em andamento. Encerrar (drop) mata o processo.
struct Run {
    child: Child,
    events: Receiver<AgentEvent>,
}

impl Run {
    /// Inicia o `claude` com a mensagem, na conta da pasta `config_dir`. No
    /// Yolo, o terra aprova tudo o que o agente pedir.
    fn start(
        claude: &Path,
        request: Request<'_>,
        config_dir: Option<&Path>,
    ) -> Result<Run, String> {
        let auto_allow = request.mode == PermissionMode::Yolo;
        let mut cmd = command(claude);
        cmd.args(cli_args(
            cli_mode(request.mode),
            request.session_id,
            auto_allow,
            request.model,
            request.effort,
        ));
        // A conta escolhida: o `claude` lê credenciais e settings dessa pasta.
        if let Some(dir) = config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("não foi possível iniciar o claude: {err}"))?;

        // O stdin fica aberto durante a resposta, para responder aos pedidos
        // de permissão; fechar no fim é o que encerra o `claude`.
        let mut stdin = child.stdin.take();
        let message = user_message(request.prompt);

        let mut stderr = child.stderr.take().expect("stderr configurado como pipe");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });

        let stdout = child.stdout.take().expect("stdout configurado como pipe");
        let (tx, events) = mpsc::channel();
        thread::spawn(move || {
            send_line(&mut stdin, &message);
            let mut parser = StreamParser::default();
            let mut done = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(response) = control_response(&line, auto_allow) {
                    send_line(&mut stdin, &response);
                    continue;
                }
                for event in parser.parse(&line) {
                    if matches!(event, AgentEvent::Done { .. }) {
                        done = true;
                        stdin = None;
                    }
                    if tx.send(event).is_err() {
                        return;
                    }
                }
            }
            if !done {
                let stderr = stderr.join().unwrap_or_default();
                let message = last_lines(&stderr, 3)
                    .unwrap_or_else(|| "o claude encerrou sem responder".to_string());
                let _ = tx.send(AgentEvent::Failed(message));
            }
        });

        Ok(Run { child, events })
    }
}

impl Session for Run {
    fn poll(&mut self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }
}

/// O processo do `claude`. Instalado pelo npm no Windows, ele é um `.cmd`,
/// que precisa do `cmd.exe`.
fn command(claude: &Path) -> Command {
    if claude.extension().is_some_and(|ext| ext == "cmd") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(claude);
        cmd
    } else {
        Command::new(claude)
    }
}

/// Escreve uma linha JSON no stdin do `claude`. Se ele já fechou, desiste.
fn send_line(stdin: &mut Option<std::process::ChildStdin>, line: &str) {
    if let Some(pipe) = stdin
        && writeln!(pipe, "{line}")
            .and_then(|()| pipe.flush())
            .is_err()
    {
        *stdin = None;
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn last_lines(text: &str, n: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    (!lines.is_empty()).then(|| lines[lines.len().saturating_sub(n)..].join("\n"))
}

/// "claude-opus-5-5" → "Opus - 5.5". Ids em outro formato ficam como estão.
fn display_model(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else {
        return id.to_string();
    };
    let mut parts = rest.split('-');
    let Some(family) = parts.next().filter(|f| !f.is_empty()) else {
        return id.to_string();
    };
    // Datas (ex.: 20250805) ficam de fora da versão.
    let version: Vec<&str> = parts
        .take_while(|p| p.len() < 8 && p.chars().all(|c| c.is_ascii_digit()))
        .collect();
    let mut name = family[..1].to_uppercase() + &family[1..];
    if !version.is_empty() {
        name = format!("{name} - {}", version.join("."));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_args_skip_user_environment_and_resume_session() {
        let args = cli_args("plan", None, false, None, None);
        for flag in [
            "--strict-mcp-config",
            "--disable-slash-commands",
            r#"{"disableAllHooks":true}"#,
        ] {
            assert!(args.contains(&flag), "faltou {flag}");
        }
        assert!(!args.contains(&"--resume"));
        assert!(!args.contains(&"--permission-prompt-tool"));
        assert!(!args.contains(&"--model"));
        assert!(!args.contains(&"--effort"));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));

        let args = cli_args("manual", Some("s1"), true, Some("haiku"), Some("max"));
        assert!(args.windows(2).any(|w| w == ["--model", "haiku"]));
        assert!(args.windows(2).any(|w| w == ["--effort", "max"]));
        assert!(args.ends_with(&["--resume", "s1"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--permission-prompt-tool", "stdio"])
        );
    }

    #[test]
    fn user_message_is_stream_json() {
        let value: Value = serde_json::from_str(&user_message("oi \"você\"")).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "oi \"você\"");
    }

    #[test]
    fn control_requests_are_allowed_only_with_auto_allow() {
        let request = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}"#;
        let allow: Value = serde_json::from_str(&control_response(request, true).unwrap()).unwrap();
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "ls"
        );

        let deny: Value = serde_json::from_str(&control_response(request, false).unwrap()).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");

        let other =
            r#"{"type":"control_request","request_id":"r2","request":{"subtype":"hook_callback"}}"#;
        let error: Value = serde_json::from_str(&control_response(other, true).unwrap()).unwrap();
        assert_eq!(error["response"]["subtype"], "error");

        assert_eq!(control_response(r#"{"type":"assistant"}"#, true), None);
    }

    #[test]
    fn configured_mode_follows_settings_precedence() {
        let config = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let write = |path: PathBuf, mode: &str| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let json = format!(r#"{{"permissions":{{"defaultMode":"{mode}"}}}}"#);
            fs::write(path, json).unwrap();
        };
        let cfg = config.path();
        let cwd = project.path();
        assert_eq!(configured_mode_in(cfg, cwd), None);

        write(cfg.join("settings.json"), "acceptEdits");
        assert_eq!(configured_mode_in(cfg, cwd).as_deref(), Some("acceptEdits"));

        write(cwd.join(".claude/settings.json"), "plan");
        assert_eq!(configured_mode_in(cfg, cwd).as_deref(), Some("plan"));

        write(cwd.join(".claude/settings.local.json"), "bypassPermissions");
        assert_eq!(
            configured_mode_in(cfg, cwd).as_deref(),
            Some("bypassPermissions")
        );

        // Arquivo sem defaultMode (ou inválido) não apaga o anterior.
        fs::write(cwd.join(".claude/settings.local.json"), "{}").unwrap();
        assert_eq!(configured_mode_in(cfg, cwd).as_deref(), Some("plan"));
    }

    #[test]
    fn permission_modes_map_to_cli() {
        let names: Vec<_> = PermissionMode::ALL.iter().map(|&m| cli_mode(m)).collect();
        assert_eq!(names, ["auto", "manual", "manual", "acceptEdits", "plan"]);
    }

    #[test]
    fn permission_modes_round_trip_cli_names() {
        for mode in PermissionMode::ALL {
            if mode != PermissionMode::Yolo {
                assert_eq!(parse_mode(cli_mode(mode)), Some(mode));
            }
        }
        // O Yolo nunca vem do Claude nem dos settings.
        assert_eq!(parse_mode("bypassPermissions"), None);
        assert_eq!(parse_mode("default"), Some(PermissionMode::Manual));
        assert_eq!(parse_mode("dontAsk"), None);
    }

    #[test]
    fn adapter_without_claude_is_unavailable() {
        let adapter = ClaudeAdapter {
            bin: None,
            home: None,
        };
        assert!(!adapter.is_available());
        assert_eq!(adapter.name(), "Claude");
        let request = Request {
            prompt: "oi",
            mode: PermissionMode::Agente,
            session_id: None,
            model: None,
            account: None,
            effort: None,
        };
        assert_eq!(adapter.start(request).err().as_deref(), Some(NOT_FOUND));
        assert!(adapter.accounts().is_empty());
        let models: Vec<_> = adapter.models().into_iter().map(|m| m.id).collect();
        assert_eq!(models, ["opus", "sonnet", "haiku"]);
        assert_eq!(adapter.model_label("claude-opus-5-5"), "Opus - 5.5");
    }

    #[test]
    fn display_model_formats_ids() {
        assert_eq!(display_model("claude-opus-5-5"), "Opus - 5.5");
        assert_eq!(display_model("claude-haiku-5-5"), "Haiku - 5.5");
        assert_eq!(display_model("claude-opus-4-1-20250805"), "Opus - 4.1");
        assert_eq!(display_model("claude-fable-5-1"), "Fable - 5.1");
        assert_eq!(display_model("gpt-x"), "gpt-x");
    }
}
