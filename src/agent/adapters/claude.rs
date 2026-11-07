//! O Claude CLI (`claude`) como agente: [`ClaudeAdapter`] adapta o CLI ao
//! contrato [`ChatAdapter`] do chat.
//!
//! Cada mensagem roda `claude -p` com entrada e saída em `stream-json`; a
//! partir da segunda, com `--resume <sessão>` para manter a conversa. Uma
//! thread lê a saída (um JSON por linha) e a transforma em [`AgentEvent`]s,
//! que a interface consome sem travar. Os pedidos de permissão do agente
//! chegam pela mesma saída: no modo Yolo, a thread aprova; nos outros, viram
//! [`AgentEvent::PermissionRequest`] e o usuário decide no chat. As perguntas
//! do agente (`AskUserQuestion`) chegam do mesmo jeito e viram
//! [`AgentEvent::Question`] em todo modo, porque só o usuário pode responder.

mod accounts;
mod catalog;
mod daily;
mod interactive;
pub(super) mod skills;
mod stream;
mod usage;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use self::stream::{StreamParser, text, tool_detail};
use crate::agent::{
    AccountOption, AgentEvent, Catalog, ChatAdapter, CommandRoute, DailyTokens, Interactive,
    ModelOption, PermissionMode, ProviderSkill, Question, QuestionOption, Request, Session,
    SubagentEvent, Usage, kill_tree, launch, own_group,
};
use crate::extension::{Contribution, Hook};

const NOT_FOUND: &str =
    "Claude CLI não encontrado no PATH. Instale em https://claude.com/claude-code";
/// A variável com que o `claude` escolhe o modelo dos sub agentes.
const SUBAGENT_MODEL_ENV: &str = "CLAUDE_CODE_SUBAGENT_MODEL";
/// A ferramenta com que o agente faz perguntas ao usuário.
const ASK_TOOL: &str = "AskUserQuestion";

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
        NAME
    }

    fn id(&self) -> &str {
        "claude"
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
        accounts::discover(home, &default_json(home))
    }

    /// As skills da pasta da conta (do usuário, do claude.ai e dos
    /// plugins), as do `~/.agents/skills` e as do `.claude/skills` do `cwd`
    /// e das pastas acima dele (do projeto).
    fn skills(&self, cwd: &Path, account: Option<&str>) -> Vec<ProviderSkill> {
        let (dir, json) = match self.account_dir(account) {
            Ok(Some(dir)) => {
                let json = dir.join(".claude.json");
                (dir, json)
            }
            Ok(None) => match (default_config_dir(), &self.home) {
                (Some(dir), Some(home)) => (dir, default_json(home)),
                _ => return Vec::new(),
            },
            Err(_) => return Vec::new(),
        };
        skills::installed(&dir, &json, self.home.as_deref(), cwd)
    }

    /// `claude auth login` na pasta da conta, com o e-mail dela já
    /// preenchido na página de login.
    fn login(&self, account: Option<&str>) -> Option<Command> {
        let bin = self.bin.as_ref()?;
        let dir = self.account_dir(account).ok()?;
        let json = match &dir {
            Some(dir) => dir.join(".claude.json"),
            None => default_json(self.home.as_deref()?),
        };
        let mut cmd = command(bin);
        cmd.args(["auth", "login"]);
        if let Some(email) = accounts::email(&json) {
            cmd.args(["--email", &email]);
        }
        if let Some(dir) = dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        Some(cmd)
    }

    /// Os avisos de login do `claude`: "Failed to authenticate", "OAuth token
    /// revoked" e "Please log in again".
    fn login_error_words(&self) -> &[&str] {
        &["authenticate", "oauth", "log in again"]
    }

    /// Os comandos só do modo interativo abrem o `claude` original; os de
    /// pagamento e plano não rodam.
    fn command_route(&self, name: &str) -> CommandRoute {
        if interactive::BILLING.contains(&name) {
            CommandRoute::Blocked
        } else if interactive::opens_original(name) {
            CommandRoute::Original
        } else {
            CommandRoute::Agent
        }
    }

    /// O `claude` interativo, com o ambiente do usuário (MCP, skills e
    /// hooks) e o que as extensões acrescentam. No Yolo,
    /// `bypassPermissions`: fora da tela, o terra não tem como aprovar os
    /// pedidos.
    fn interactive(&self, cwd: &Path, options: Interactive<'_>) -> Option<Command> {
        let bin = self.bin.as_ref()?;
        let dir = self.account_dir(options.account).ok()?;
        let mut cmd = match options.extensions {
            Some(extensions) => launch(
                bin,
                HARNESS,
                extensions,
                options.session_id.is_none(),
                command,
            ),
            None => command(bin),
        };
        cmd.current_dir(cwd);
        if let Some(mode) = options.mode {
            let name = match mode {
                PermissionMode::Yolo => "bypassPermissions",
                mode => cli_mode(mode),
            };
            cmd.args(["--permission-mode", name]);
        }
        if let Some(model) = options.model {
            cmd.args(["--model", model]);
        }
        if let Some(effort) = options.effort {
            cmd.args(["--effort", effort]);
        }
        if let Some(model) = options.subagent_model {
            cmd.env(SUBAGENT_MODEL_ENV, model);
        }
        if let Some(extensions) = options.extensions {
            if !extensions.hooks.is_empty() {
                cmd.args(["--settings", &hook_settings(&extensions.hooks)]);
            }
            cmd.args(mcp_args(extensions));
            cmd.envs(extensions.env.iter().map(|(k, v)| (k, v)));
        }
        if let Some(session) = options.session_id {
            cmd.args(["--resume", session]);
        }
        if let Some(command) = options.command {
            cmd.arg(command);
        }
        if let Some(dir) = dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        Some(cmd)
    }

    /// O `claude` responde ao `initialize` do protocolo do Agent SDK com os
    /// comandos e modelos, sem chamar a API.
    fn load_catalog(&self, account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        let bin = self.bin.clone()?;
        let dir = self.account_dir(account).ok().flatten();
        Some(catalog::load(bin, dir))
    }

    /// O consumo da conta, do endpoint de uso do Claude (sem gastar tokens).
    fn load_usage(&self, account: Option<&str>) -> Option<Receiver<Result<Usage, String>>> {
        let dir = match self.account_dir(account).ok()? {
            Some(dir) => dir,
            None => default_config_dir()?,
        };
        Some(usage::load(dir))
    }

    /// Os tokens por dia, dos transcripts do Claude Code na pasta da conta.
    fn load_daily(
        &self,
        account: Option<&str>,
        days: usize,
    ) -> Option<Receiver<Result<Vec<DailyTokens>, String>>> {
        let dir = match self.account_dir(account).ok()? {
            Some(dir) => dir,
            None => default_config_dir()?,
        };
        Some(daily::load(dir, days))
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
/// Com o ambiente do Claude Code do usuário: os servidores MCP dele (do
/// usuário, os locais e os do projeto) e as skills e os comandos ficam
/// ligados, como no `claude` interativo.
///
/// Com `stdio`, os pedidos de permissão do agente chegam pelo stdin/stdout
/// (`--permission-prompt-tool stdio`): o terra aprova (Yolo) ou pergunta ao
/// usuário (ver [`Run::start`]).
///
/// `settings` vai no `--settings` (ver [`hook_settings`]); os servidores MCP
/// das extensões, em [`mcp_args`], somam aos do usuário.
fn cli_args<'a>(
    mode: &'a str,
    session: Option<&'a str>,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    settings: &'a str,
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
        "--settings",
        settings,
        "--permission-prompt-tool",
        "stdio",
    ];
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

/// Os settings de uma mensagem: sem hooks das extensões, todos os hooks
/// ficam desligados (`disableAllHooks`, que vale até para os do
/// `--settings`); com eles, os hooks ficam ligados — inclusive os do
/// usuário — e os das extensões entram por aqui.
fn hook_settings(hooks: &[Hook]) -> String {
    if hooks.is_empty() {
        return r#"{"disableAllHooks":true}"#.to_string();
    }
    let mut events = serde_json::Map::new();
    for hook in hooks {
        let groups = events
            .entry(hook.event.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(groups) = groups {
            groups.push(serde_json::json!({
                "matcher": hook.matcher,
                "hooks": [{ "type": "command", "command": hook.command }],
            }));
        }
    }
    serde_json::json!({ "hooks": events }).to_string()
}

/// `--mcp-config` com os servidores MCP das extensões (vazio sem nenhum),
/// somados aos do usuário.
pub(crate) fn mcp_args(extensions: &Contribution) -> Vec<String> {
    if extensions.mcp_servers.is_empty() {
        return Vec::new();
    }
    let servers: serde_json::Map<String, Value> = extensions
        .mcp_servers
        .iter()
        .map(|server| (server.name.clone(), server_config(server)))
        .collect();
    let config = serde_json::json!({ "mcpServers": servers }).to_string();
    vec!["--mcp-config".to_string(), config]
}

/// Um servidor MCP das extensões no formato do `--mcp-config`.
fn server_config(server: &crate::extension::McpServer) -> Value {
    match &server.url {
        Some(url) if server.headers.is_empty() => {
            serde_json::json!({ "type": "http", "url": url })
        }
        // O `claude` troca os `${NOME}` dos cabeçalhos pelas variáveis de
        // ambiente dele.
        Some(url) => serde_json::json!({
            "type": "http",
            "url": url,
            "headers": server
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), Value::from(value.as_str())))
                .collect::<serde_json::Map<_, _>>(),
        }),
        None => serde_json::json!({
            "type": "stdio",
            "command": server.command,
            "args": server.args,
        }),
    }
}

/// A mensagem do usuário no formato do `--input-format stream-json`.
fn user_message(prompt: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": prompt },
    })
    .to_string()
}

/// Um pedido de permissão (`can_use_tool`) do agente: o id do pedido, a
/// ferramenta e a entrada dela. `None` se a linha não é um.
fn tool_request(line: &str) -> Option<(String, String, Value)> {
    let value: Value = serde_json::from_str(line).ok()?;
    let request = &value["request"];
    if value["type"] != "control_request" || request["subtype"] != "can_use_tool" {
        return None;
    }
    Some((
        text(&value["request_id"]),
        text(&request["tool_name"]),
        request["input"].clone(),
    ))
}

/// As perguntas de um pedido do `AskUserQuestion`; `None` em outra ferramenta
/// ou sem perguntas (fica como um pedido de permissão comum).
fn questions(tool: &str, input: &Value) -> Option<Vec<Question>> {
    if tool != ASK_TOOL {
        return None;
    }
    let questions: Vec<Question> = input["questions"]
        .as_array()?
        .iter()
        .map(|question| Question {
            header: text(&question["header"]),
            text: text(&question["question"]),
            multi_select: question["multiSelect"].as_bool().unwrap_or(false),
            options: question["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|option| QuestionOption {
                    label: text(&option["label"]),
                    description: text(&option["description"]),
                })
                .collect(),
        })
        .collect();
    (!questions.is_empty()).then_some(questions)
}

/// A resposta a um `AskUserQuestion`: a entrada aprovada, com `answers` (o
/// texto de cada pergunta e a resposta dela), que o `claude` entrega ao modelo.
fn answer_response(request_id: &str, input: &Value, answers: &[String]) -> String {
    let answers: serde_json::Map<String, Value> = input["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(answers)
        .map(|(question, answer)| (text(&question["question"]), Value::from(answer.as_str())))
        .collect();
    let mut input = input.clone();
    input["answers"] = Value::Object(answers);
    permission_response(request_id, true, &input)
}

/// A resposta a um pedido de permissão: aprovado com a entrada como veio, ou
/// negado.
fn permission_response(request_id: &str, allow: bool, input: &Value) -> String {
    let decision = if allow {
        serde_json::json!({ "behavior": "allow", "updatedInput": input })
    } else {
        serde_json::json!({ "behavior": "deny", "message": "negado pelo usuário no terra" })
    };
    serde_json::json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": decision },
    })
    .to_string()
}

/// Resposta de erro a um pedido de controle do agente (`control_request`) que
/// o terra não suporta, ou `None` se a linha não é um. Os pedidos de
/// permissão vêm antes, por [`tool_request`].
fn unsupported_response(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["type"] != "control_request" {
        return None;
    }
    let response = serde_json::json!({
        "subtype": "error",
        "request_id": value["request_id"],
        "error": format!("pedido não suportado pelo terra: {}", text(&value["request"]["subtype"])),
    });
    Some(serde_json::json!({ "type": "control_response", "response": response }).to_string())
}

/// Modo de permissão padrão configurado no Claude (`permissions.defaultMode`)
/// para o diretório `cwd`, como o próprio `claude` leria. `account_dir` é a
/// pasta da conta escolhida; sem ela, a padrão.
fn configured_mode(cwd: &Path, account_dir: Option<&Path>) -> Option<String> {
    let config_dir = account_dir
        .map(Path::to_path_buf)
        .or_else(default_config_dir)?;
    configured_mode_in(&config_dir, cwd)
}

/// O `.claude.json` da conta Default (com o e-mail dela).
fn default_json(home: &Path) -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(|dir| PathBuf::from(dir).join(".claude.json"))
        .unwrap_or_else(|| home.join(".claude.json"))
}

/// A pasta que o `claude` usa sem conta escolhida.
fn default_config_dir() -> Option<PathBuf> {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
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
    /// O stdin do `claude`, para responder aos pedidos de permissão; `None`
    /// depois do fim da resposta.
    stdin: Stdin,
    /// Os pedidos de permissão esperando o usuário: a entrada da ferramenta
    /// de cada um, que volta na aprovação.
    pending: Arc<Mutex<HashMap<String, Value>>>,
}

type Stdin = Arc<Mutex<Option<ChildStdin>>>;

/// Quanto o agente principal tem para continuar depois de um subagente acabar.
const CONTINUATION_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(15)
};

impl Run {
    /// Inicia o `claude` com a mensagem, na conta da pasta `config_dir`. No
    /// Yolo, o terra aprova tudo o que o agente pedir; nos outros, cada
    /// pedido vira um [`AgentEvent::PermissionRequest`] para o usuário.
    fn start(
        claude: &Path,
        request: Request<'_>,
        config_dir: Option<&Path>,
    ) -> Result<Run, String> {
        let auto_allow = request.mode == PermissionMode::Yolo;
        let settings = hook_settings(&request.extensions.hooks);
        let mut cmd = launch(
            claude,
            HARNESS,
            request.extensions,
            request.session_id.is_none(),
            command,
        );
        cmd.args(cli_args(
            cli_mode(request.mode),
            request.session_id,
            request.model,
            request.effort,
            &settings,
        ));
        cmd.args(mcp_args(request.extensions));
        cmd.envs(request.extensions.env.iter().map(|(k, v)| (k, v)));
        if let Some(model) = request.subagent_model {
            cmd.env(SUBAGENT_MODEL_ENV, model);
        }
        // As ferramentas que o usuário liberou depois de o modo bloquear: uma
        // regra por argumento, porque a do Bash tem espaços.
        if !request.allowed_tools.is_empty() {
            cmd.arg("--allowedTools").args(request.allowed_tools);
        }
        // A conta escolhida: o `claude` lê credenciais e settings dessa pasta.
        if let Some(dir) = config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        // Num grupo próprio, para o fim do turno levar junto o que os
        // lançadores abrem.
        own_group(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("não foi possível iniciar o claude: {err}"))?;

        // O stdin fica aberto durante a resposta, para responder aos pedidos
        // de permissão; fechar no fim é o que encerra o `claude`.
        let stdin: Stdin = Arc::new(Mutex::new(child.stdin.take()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let message = user_message(request.prompt);

        let mut stderr = child.stderr.take().expect("stderr configurado como pipe");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });

        let stdout = child.stdout.take().expect("stdout configurado como pipe");
        let (tx, events) = mpsc::channel();
        let (thread_stdin, thread_pending) = (Arc::clone(&stdin), Arc::clone(&pending));
        thread::spawn(move || {
            let (stdin, pending) = (thread_stdin, thread_pending);
            send_line(&stdin, &message);
            let mut parser = StreamParser::default();
            let mut done = false;
            // O fim da resposta, enquanto falta subagente: as continuações
            // somam o uso, e o stdin segue aberto para os pedidos dele.
            let mut held = None;
            let activity = Arc::new(AtomicU64::new(0));
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                activity.fetch_add(1, Ordering::Relaxed);
                if let Some((id, name, input)) = tool_request(&line) {
                    if let Some(questions) = questions(&name, &input) {
                        pending.lock().unwrap().insert(id.clone(), input);
                        if tx.send(AgentEvent::Question { id, questions }).is_err() {
                            return;
                        }
                        continue;
                    }
                    if auto_allow {
                        send_line(&stdin, &permission_response(&id, true, &input));
                        continue;
                    }
                    let detail = tool_detail(&input);
                    pending.lock().unwrap().insert(id.clone(), input);
                    if tx
                        .send(AgentEvent::PermissionRequest { id, name, detail })
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                if let Some(response) = unsupported_response(&line) {
                    send_line(&stdin, &response);
                    continue;
                }
                for event in parser.parse(&line) {
                    // O último subagente acabou depois do fim do principal:
                    // a continuação tem o mesmo prazo para começar.
                    if held.is_some()
                        && !parser.subagents_running()
                        && matches!(event, AgentEvent::Subagent(SubagentEvent::Finished { .. }))
                    {
                        close_when_silent(&stdin, &activity);
                    }
                    let event = if matches!(event, AgentEvent::Done { .. }) {
                        let event = merge_done(held.take(), event);
                        if parser.subagents_pending() {
                            if !parser.subagents_running() {
                                close_when_silent(&stdin, &activity);
                            }
                            held = Some(event);
                            AgentEvent::Subagent(SubagentEvent::Waiting)
                        } else {
                            done = true;
                            *stdin.lock().unwrap() = None;
                            event
                        }
                    } else {
                        event
                    };
                    if tx.send(event).is_err() {
                        return;
                    }
                }
            }
            if let Some(event) = held.filter(|_| !done) {
                done = true;
                let _ = tx.send(event);
            }
            if !done {
                let stderr = stderr.join().unwrap_or_default();
                let message = last_lines(&stderr, 3)
                    .unwrap_or_else(|| "o claude encerrou sem responder".to_string());
                let _ = tx.send(AgentEvent::Failed(message));
            }
        });

        Ok(Run {
            child,
            events,
            stdin,
            pending,
        })
    }
}

impl Session for Run {
    fn poll(&mut self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }

    fn respond(&mut self, id: &str, allow: bool) {
        let Some(input) = self.pending.lock().unwrap().remove(id) else {
            return;
        };
        send_line(&self.stdin, &permission_response(id, allow, &input));
    }

    fn answer(&mut self, id: &str, answers: Option<Vec<String>>) {
        let Some(input) = self.pending.lock().unwrap().remove(id) else {
            return;
        };
        let response = match answers {
            Some(answers) => answer_response(id, &input, &answers),
            None => permission_response(id, false, &input),
        };
        send_line(&self.stdin, &response);
    }
}

/// Segue a saída de uma execução despachada (ver [`crate::agent::follow`]).
pub fn follow(path: &Path) -> Box<dyn Session> {
    let mut parser = StreamParser::default();
    crate::agent::follow(path, "\"type\":\"result\"", move |line| parser.parse(line))
}

/// O nome do provedor, como o adapter mostra.
pub const NAME: &str = "Claude";

/// O `claude` do `PATH` para uma execução despachada pelo orquestrador, sem
/// os lançadores das extensões; `None` sem ele.
pub fn run_command() -> Option<Command> {
    find_claude().map(|claude| command(&claude))
}

/// O Claude no `ai-memory run`.
const HARNESS: &str = "claude";

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

/// O fim da resposta com o uso das partes que já acabaram: cada `result` traz
/// só o da parte dele.
fn merge_done(held: Option<AgentEvent>, last: AgentEvent) -> AgentEvent {
    match (held, last) {
        (
            Some(AgentEvent::Done {
                tokens: spent,
                cache_tokens: cache_spent,
                denied: denied_before,
                ..
            }),
            AgentEvent::Done {
                is_error,
                message,
                tokens,
                cache_tokens,
                denied,
            },
        ) => AgentEvent::Done {
            is_error,
            message,
            tokens: spent + tokens,
            cache_tokens: cache_spent + cache_tokens,
            denied: denied_before + denied,
        },
        (_, last) => last,
    }
}

/// Fecha o stdin, que encerra o `claude`, se nada chegar em [`CONTINUATION_WAIT`].
/// É a saída para uma continuação que não vem depois de o subagente acabar.
fn close_when_silent(stdin: &Stdin, activity: &Arc<AtomicU64>) {
    let (stdin, activity) = (Arc::clone(stdin), Arc::clone(activity));
    let seen = activity.load(Ordering::Relaxed);
    thread::spawn(move || {
        thread::sleep(CONTINUATION_WAIT);
        if activity.load(Ordering::Relaxed) == seen {
            *stdin.lock().unwrap() = None;
        }
    });
}

/// Escreve uma linha JSON no stdin do `claude`. Se ele já fechou, desiste.
fn send_line(stdin: &Stdin, line: &str) {
    let mut stdin = stdin.lock().unwrap();
    if let Some(pipe) = stdin.as_mut()
        && writeln!(pipe, "{line}")
            .and_then(|()| pipe.flush())
            .is_err()
    {
        *stdin = None;
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        kill_tree(&mut self.child);
    }
}

fn last_lines(text: &str, n: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    (!lines.is_empty()).then(|| lines[lines.len().saturating_sub(n)..].join("\n"))
}

/// "claude-opus-5-5" → "Opus - 5.5". Ids em outro formato ficam como estão.
pub fn display_model(id: &str) -> String {
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

    const NO_HOOKS: &str = r#"{"disableAllHooks":true}"#;

    #[test]
    fn cli_args_keep_user_environment_and_resume_session() {
        let args = cli_args("plan", None, None, None, NO_HOOKS);
        assert!(args.contains(&r#"{"disableAllHooks":true}"#));
        // Os MCP e as skills do usuário ficam ligados.
        assert!(!args.contains(&"--strict-mcp-config"));
        assert!(!args.contains(&"--disable-slash-commands"));
        assert!(!args.contains(&"--resume"));
        assert!(!args.contains(&"--model"));
        assert!(!args.contains(&"--effort"));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));

        let args = cli_args("manual", Some("s1"), Some("haiku"), Some("max"), NO_HOOKS);
        assert!(args.windows(2).any(|w| w == ["--model", "haiku"]));
        assert!(args.windows(2).any(|w| w == ["--effort", "max"]));
        assert!(args.ends_with(&["--resume", "s1"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--permission-prompt-tool", "stdio"])
        );
    }

    #[test]
    fn extensions_become_hook_settings_and_mcp_config() {
        assert_eq!(hook_settings(&[]), NO_HOOKS);
        assert!(mcp_args(&Contribution::default()).is_empty());

        let hook = |event: &str, matcher: &str, command: &str| Hook {
            event: event.into(),
            matcher: matcher.into(),
            command: command.into(),
        };
        let extensions = Contribution {
            hooks: vec![
                hook("PreToolUse", "Bash", "rtk hook claude"),
                hook("PreToolUse", "", "memoria pre"),
                hook("Stop", "", "memoria stop"),
            ],
            mcp_servers: vec![crate::extension::McpServer {
                name: "ai-memory".into(),
                command: "ai-memory".into(),
                args: vec!["mcp-bridge".into()],
                ..Default::default()
            }],
            env: Vec::new(),
            launchers: Vec::new(),
        };
        let settings: Value = serde_json::from_str(&hook_settings(&extensions.hooks)).unwrap();
        assert_eq!(settings.get("disableAllHooks"), None);
        assert_eq!(
            settings["hooks"]["PreToolUse"],
            serde_json::json!([
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "rtk hook claude"}]},
                {"matcher": "", "hooks": [{"type": "command", "command": "memoria pre"}]},
            ])
        );
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            "memoria stop"
        );

        let args = mcp_args(&extensions);
        assert_eq!(args[0], "--mcp-config");
        let config: Value = serde_json::from_str(&args[1]).unwrap();
        assert_eq!(
            config["mcpServers"]["ai-memory"],
            serde_json::json!({"type": "stdio", "command": "ai-memory", "args": ["mcp-bridge"]})
        );

        let http = Contribution {
            mcp_servers: vec![crate::extension::McpServer {
                name: "terra".into(),
                url: Some("http://127.0.0.1:30303/mcp".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let config: Value = serde_json::from_str(&mcp_args(&http)[1]).unwrap();
        assert_eq!(
            config["mcpServers"]["terra"],
            serde_json::json!({"type": "http", "url": "http://127.0.0.1:30303/mcp"})
        );
    }

    #[test]
    fn interactive_gets_the_extensions_without_disabling_user_hooks() {
        let adapter = ClaudeAdapter {
            bin: Some(PathBuf::from("claude")),
            home: None,
        };
        let extensions = Contribution {
            hooks: vec![Hook {
                event: "PreToolUse".into(),
                matcher: "Bash".into(),
                command: "rtk hook claude".into(),
            }],
            mcp_servers: Vec::new(),
            env: Vec::new(),
            launchers: Vec::new(),
        };
        let options = Interactive {
            extensions: Some(&extensions),
            ..Interactive::default()
        };
        let cmd = adapter.interactive(Path::new("."), options).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
        assert_eq!(args[0], "--settings");
        assert!(args[1].contains("rtk hook claude"));
        assert!(!args.iter().any(|a| a.contains("disableAllHooks")));
        assert!(!args.iter().any(|a| a == "--mcp-config"));

        let cmd = adapter
            .interactive(Path::new("."), Interactive::default())
            .unwrap();
        assert_eq!(cmd.get_args().count(), 0);
    }

    #[test]
    fn user_message_is_stream_json() {
        let value: Value = serde_json::from_str(&user_message("oi \"você\"")).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "oi \"você\"");
    }

    #[test]
    fn permission_requests_are_parsed_and_answered() {
        let request = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}"#;
        let (id, name, input) = tool_request(request).unwrap();
        assert_eq!((id.as_str(), name.as_str()), ("r1", "Bash"));
        assert_eq!(input["command"], "ls");

        let allow: Value = serde_json::from_str(&permission_response(&id, true, &input)).unwrap();
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "ls"
        );

        let deny: Value = serde_json::from_str(&permission_response(&id, false, &input)).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");

        let other =
            r#"{"type":"control_request","request_id":"r2","request":{"subtype":"hook_callback"}}"#;
        assert_eq!(tool_request(other), None);
        let error: Value = serde_json::from_str(&unsupported_response(other).unwrap()).unwrap();
        assert_eq!(error["response"]["subtype"], "error");
        assert_eq!(error["response"]["request_id"], "r2");

        assert_eq!(tool_request(r#"{"type":"assistant"}"#), None);
        assert_eq!(unsupported_response(r#"{"type":"assistant"}"#), None);
    }

    const ASK_INPUT: &str = r#"{"questions":[
        {"question":"Qual cor?","header":"Cor","multiSelect":false,"options":[
            {"label":"Azul","description":"O céu"},{"label":"Verde"}]},
        {"question":"Quais?","header":"Itens","multiSelect":true,"options":[{"label":"A"}]}
    ]}"#;

    #[test]
    fn ask_user_question_becomes_questions_and_is_answered_with_the_input() {
        let input: Value = serde_json::from_str(ASK_INPUT).unwrap();
        let parsed = questions(ASK_TOOL, &input).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].header, "Cor");
        assert_eq!(parsed[0].text, "Qual cor?");
        assert!(!parsed[0].multi_select);
        assert_eq!(
            parsed[0].options,
            [
                QuestionOption {
                    label: "Azul".into(),
                    description: "O céu".into()
                },
                QuestionOption {
                    label: "Verde".into(),
                    description: String::new()
                },
            ]
        );
        assert!(parsed[1].multi_select);

        // Outra ferramenta, ou sem perguntas, é um pedido de permissão comum.
        assert_eq!(questions("Bash", &input), None);
        assert_eq!(
            questions(ASK_TOOL, &serde_json::json!({ "questions": [] })),
            None
        );
        assert_eq!(questions(ASK_TOOL, &Value::Null), None);

        let answers = ["Verde".to_string(), "A, B".to_string()];
        let reply: Value = serde_json::from_str(&answer_response("r1", &input, &answers)).unwrap();
        assert_eq!(reply["response"]["request_id"], "r1");
        let decision = &reply["response"]["response"];
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedInput"]["questions"], input["questions"]);
        assert_eq!(
            decision["updatedInput"]["answers"],
            serde_json::json!({ "Qual cor?": "Verde", "Quais?": "A, B" })
        );
    }

    /// Mesmo no Yolo, que aprova o resto sozinho, a pergunta espera o usuário.
    #[cfg(unix)]
    #[test]
    fn questions_wait_for_the_user_even_in_yolo() {
        let dir = tempfile::tempdir().unwrap();
        let reply = dir.path().join("resposta");
        let request = serde_json::json!({
            "type": "control_request",
            "request_id": "r1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": ASK_TOOL,
                "input": serde_json::from_str::<Value>(ASK_INPUT).unwrap(),
            },
        });
        let script = format!(
            "#!/bin/sh\nread -r message\necho '{request}'\nread -r answer\n\
             printf '%s' \"$answer\" > '{}'\n\
             echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{{}}}}'\n",
            reply.display()
        );
        let mut run = start_fake(dir.path(), &script);

        let id = loop {
            if let Some(AgentEvent::Question { id, questions }) = run.poll().into_iter().next() {
                assert_eq!(questions[0].text, "Qual cor?");
                break id;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(!reply.exists());
        run.answer(&id, Some(vec!["Verde".to_string(), "A".to_string()]));
        while !run
            .poll()
            .iter()
            .any(|event| matches!(event, AgentEvent::Done { .. }))
        {
            thread::sleep(std::time::Duration::from_millis(5));
        }
        let sent: Value = serde_json::from_str(&fs::read_to_string(&reply).unwrap()).unwrap();
        assert_eq!(
            sent["response"]["response"]["updatedInput"]["answers"]["Qual cor?"],
            "Verde"
        );
    }

    /// Grava `script` como o `claude` em `dir` e o inicia no Yolo. Repete o
    /// início enquanto der "Text file busy": um teste em paralelo pode fazer
    /// `fork` com o arquivo ainda aberto para escrita, e o `exec` falha.
    #[cfg(unix)]
    fn start_fake(dir: &Path, script: &str) -> Run {
        use std::os::unix::fs::PermissionsExt;
        let fake = dir.join("claude");
        fs::write(&fake, script).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let extensions = Contribution::default();
        let mut attempts = 0;
        loop {
            let started = Run::start(
                &fake,
                Request {
                    prompt: "oi",
                    mode: PermissionMode::Yolo,
                    session_id: None,
                    model: None,
                    account: None,
                    effort: None,
                    subagent_model: None,
                    extensions: &extensions,
                    allowed_tools: &[],
                },
                None,
            );
            match started {
                Err(err) if err.contains("os error 26") && attempts < 50 => {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(10));
                }
                started => return started.unwrap(),
            }
        }
    }

    /// Um `claude` de mentira que escreve `lines`, com uma pausa entre as
    /// linhas e, no fim, espera o stdin fechar. Devolve os eventos da resposta
    /// até o fim dela.
    #[cfg(unix)]
    fn run_fake(lines: &[&str]) -> Vec<AgentEvent> {
        let dir = tempfile::tempdir().unwrap();
        let mut script = String::from("#!/bin/sh\nread -r message\n");
        for line in lines {
            script.push_str(&format!("echo '{line}'\nsleep 0.05\n"));
        }
        script.push_str("cat > /dev/null\n");
        let mut run = start_fake(dir.path(), &script);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        while !events
            .iter()
            .any(|event| matches!(event, AgentEvent::Done { .. } | AgentEvent::Failed(_)))
        {
            assert!(std::time::Instant::now() < deadline, "{events:?}");
            events.extend(run.poll());
            thread::sleep(Duration::from_millis(5));
        }
        events
    }

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s","model":"m"}"#;
    const STARTED: &str = r#"{"type":"system","subtype":"task_started","tool_use_id":"t1","description":"Rodar","task_type":"local_agent"}"#;
    const FINISHED: &str = r#"{"type":"system","subtype":"task_notification","tool_use_id":"t1","status":"completed"}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":1,"output_tokens":2}}"#;

    #[cfg(unix)]
    #[test]
    fn the_first_result_does_not_end_a_turn_with_a_running_subagent() {
        let events = run_fake(&[INIT, STARTED, RESULT, FINISHED, INIT, RESULT]);
        let position = |wanted: fn(&AgentEvent) -> bool| events.iter().position(wanted).unwrap();
        let finished =
            position(|event| matches!(event, AgentEvent::Subagent(SubagentEvent::Finished { .. })));
        let done = position(|event| matches!(event, AgentEvent::Done { .. }));
        assert!(finished < done, "{events:?}");
        // As duas partes da resposta entram no uso.
        assert!(matches!(events[done], AgentEvent::Done { tokens: 6, .. }));
        assert!(events.contains(&AgentEvent::Subagent(SubagentEvent::Waiting)));
    }

    #[cfg(unix)]
    #[test]
    fn a_continuation_that_never_comes_still_ends_the_turn() {
        let events = run_fake(&[INIT, STARTED, FINISHED, RESULT]);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                is_error: false,
                tokens: 3,
                ..
            })
        ));
    }

    /// O caso comum do subagente em segundo plano: o `result` chega com ele
    /// rodando, e ele termina depois. Sem continuação, a resposta fecha igual.
    #[cfg(unix)]
    #[test]
    fn a_continuation_that_never_comes_after_a_late_subagent_still_ends_the_turn() {
        let events = run_fake(&[INIT, STARTED, RESULT, FINISHED]);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                is_error: false,
                tokens: 3,
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_turn_without_subagents_ends_on_its_result() {
        let events = run_fake(&[INIT, RESULT]);
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
        assert!(!events.contains(&AgentEvent::Subagent(SubagentEvent::Waiting)));
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
            subagent_model: None,
            extensions: &Contribution::default(),
            allowed_tools: &[],
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

    #[test]
    fn login_runs_auth_login_in_the_account_folder_with_its_email() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude-trabalho");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"eu@empresa.com"}}"#,
        )
        .unwrap();
        let adapter = ClaudeAdapter {
            bin: Some(PathBuf::from("claude")),
            home: Some(home.path().to_path_buf()),
        };

        let login = adapter.login(Some(".claude-trabalho")).unwrap();
        let args: Vec<_> = login.get_args().collect();
        assert_eq!(args, ["auth", "login", "--email", "eu@empresa.com"]);
        let env: Vec<_> = login.get_envs().collect();
        assert_eq!(env, [("CLAUDE_CONFIG_DIR".as_ref(), Some(dir.as_os_str()))]);

        assert!(adapter.login(Some(".claude-inexistente")).is_none());
    }
}
