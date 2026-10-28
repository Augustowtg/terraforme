//! O Claude CLI (`claude`) como agente: [`ClaudeAdapter`] adapta o CLI ao
//! contrato [`ChatAdapter`] do chat.
//!
//! Cada mensagem roda `claude -p` com entrada e saída em `stream-json`; a
//! partir da segunda, com `--resume <sessão>` para manter a conversa. Uma
//! thread lê a saída (um JSON por linha) e a transforma em [`AgentEvent`]s,
//! que a interface consome sem travar. Os pedidos de permissão do agente
//! chegam pela mesma saída: no modo Yolo, a thread aprova; nos outros, viram
//! [`AgentEvent::PermissionRequest`] e o usuário decide no chat.

mod accounts;
mod catalog;
mod daily;
mod delegated;
mod interactive;
pub(super) mod mcp;
pub(super) mod skills;
mod stream;
mod usage;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use self::stream::{StreamParser, text, tool_detail};
use crate::agent::{
    AccountOption, AgentEvent, Catalog, ChatAdapter, CommandRoute, DailyTokens, Interactive,
    ModelOption, PermissionMode, ProviderMcp, ProviderSkill, Request, Session, Usage, kill_tree,
    launch, own_group,
};
use crate::extension::{Contribution, Hook};

const NOT_FOUND: &str =
    "Claude CLI não encontrado no PATH. Instale em https://claude.com/claude-code";
/// A variável com que o `claude` escolhe o modelo dos sub agentes.
const SUBAGENT_MODEL_ENV: &str = "CLAUDE_CODE_SUBAGENT_MODEL";

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
    /// O arquivo do `--mcp-config` de um chat delegado: os MCP das
    /// extensões e do usuário (da conta em `account_dir` e da pasta atual)
    /// que ele leva (veja [`delegated::servers`]).
    fn delegated_mcp(
        &self,
        extensions: &Contribution,
        delegated: crate::agent::Delegated<'_>,
        account_dir: Option<&Path>,
    ) -> Result<delegated::McpFile, String> {
        let json = match (account_dir, &self.home) {
            (Some(dir), _) => Some(dir.join(".claude.json")),
            (None, Some(home)) => Some(default_json(home)),
            (None, None) => None,
        };
        let user = match (json, env::current_dir()) {
            (Some(json), Ok(cwd)) => mcp::raw(&json, &cwd),
            _ => serde_json::Map::new(),
        };
        let servers = delegated::servers(extensions, &user, delegated);
        let dir = crate::db::data_dir()
            .map_err(|err| format!("{err:#}"))?
            .join("delegados");
        delegated::McpFile::create(&dir, servers)
            .map_err(|err| format!("não foi possível gravar o MCP do chat delegado: {err}"))
    }

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

    /// Os MCP do `.claude.json` da conta (do usuário e, do `cwd`, os
    /// locais) e os do `.mcp.json` do `cwd` (do projeto).
    fn mcp_servers(&self, cwd: &Path, account: Option<&str>) -> Vec<ProviderMcp> {
        let json = match self.account_dir(account) {
            Ok(Some(dir)) => dir.join(".claude.json"),
            Ok(None) => match &self.home {
                Some(home) => default_json(home),
                None => return Vec::new(),
            },
            Err(_) => return Vec::new(),
        };
        mcp::configured(&json, cwd)
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
        let mcp_file = match request.delegated {
            Some(delegated) => {
                Some(self.delegated_mcp(request.extensions, delegated, account_dir.as_deref())?)
            }
            None => None,
        };
        let run = Run::start(bin, request, account_dir.as_deref(), mcp_file)?;
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
    stdio: bool,
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
    ];
    if stdio {
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
fn mcp_args(extensions: &Contribution) -> Vec<String> {
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
    /// O `--mcp-config` de um chat delegado: some depois do processo.
    _mcp_file: Option<delegated::McpFile>,
}

type Stdin = Arc<Mutex<Option<ChildStdin>>>;

impl Run {
    /// Inicia o `claude` com a mensagem, na conta da pasta `config_dir`. No
    /// Yolo, o terra aprova tudo o que o agente pedir; num chat delegado, quem
    /// delegou decide (pela ferramenta `permission_tool`); nos outros, cada
    /// pedido vira um [`AgentEvent::PermissionRequest`] para o usuário.
    ///
    /// Um chat delegado (`request.delegated`) roda enxuto, com o MCP de
    /// `mcp_file` (veja [`delegated`]).
    fn start(
        claude: &Path,
        request: Request<'_>,
        config_dir: Option<&Path>,
        mcp_file: Option<delegated::McpFile>,
    ) -> Result<Run, String> {
        let auto_allow = request.mode == PermissionMode::Yolo;
        let asks_parent = !auto_allow && request.permission_tool.is_some();
        let lean = request.delegated.zip(mcp_file.as_ref());
        let settings = match lean {
            Some(_) => delegated::SETTINGS.to_string(),
            None => hook_settings(&request.extensions.hooks),
        };
        // Sem modelo escolhido, o delegado usa o dele, com o esforço dele
        // quando também não há esforço.
        let (model, effort) = match (lean, request.model) {
            (Some(_), None) => (
                Some(delegated::MODEL),
                request.effort.or(Some(delegated::EFFORT)),
            ),
            _ => (request.model, request.effort),
        };
        // O delegado roda enxuto: sem os lançadores, como sem os hooks. O
        // `ai-memory run` também só deixa um lançador por workstream, e os
        // delegados rodam junto com o chat que delegou.
        let mut cmd = match lean {
            Some(_) => command(claude),
            None => launch(
                claude,
                HARNESS,
                request.extensions,
                request.session_id.is_none(),
                command,
            ),
        };
        cmd.args(cli_args(
            cli_mode(request.mode),
            request.session_id,
            !asks_parent,
            model,
            effort,
            &settings,
        ));
        match lean {
            Some((delegated, file)) => cmd.args(delegated::args(file.path(), delegated)),
            None => cmd.args(mcp_args(request.extensions)),
        };
        cmd.envs(request.extensions.env.iter().map(|(k, v)| (k, v)));
        if let Some(model) = request.subagent_model {
            cmd.env(SUBAGENT_MODEL_ENV, model);
        }
        // As ferramentas que o usuário liberou depois de o modo bloquear: uma
        // regra por argumento, porque a do Bash tem espaços.
        if !request.allowed_tools.is_empty() {
            cmd.arg("--allowedTools").args(request.allowed_tools);
        }
        if let (false, Some((server, tool))) = (auto_allow, request.permission_tool) {
            cmd.args([
                "--permission-prompt-tool",
                &format!("mcp__{server}__{tool}"),
            ]);
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
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((id, name, input)) = tool_request(&line) {
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
                    if matches!(event, AgentEvent::Done { .. }) {
                        done = true;
                        *stdin.lock().unwrap() = None;
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

        Ok(Run {
            child,
            events,
            stdin,
            pending,
            _mcp_file: mcp_file,
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

    const NO_HOOKS: &str = r#"{"disableAllHooks":true}"#;

    #[test]
    fn cli_args_keep_user_environment_and_resume_session() {
        let args = cli_args("plan", None, false, None, None, NO_HOOKS);
        assert!(args.contains(&r#"{"disableAllHooks":true}"#));
        // Os MCP e as skills do usuário ficam ligados.
        assert!(!args.contains(&"--strict-mcp-config"));
        assert!(!args.contains(&"--disable-slash-commands"));
        assert!(!args.contains(&"--resume"));
        assert!(!args.contains(&"--permission-prompt-tool"));
        assert!(!args.contains(&"--model"));
        assert!(!args.contains(&"--effort"));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));

        let args = cli_args(
            "manual",
            Some("s1"),
            true,
            Some("haiku"),
            Some("max"),
            NO_HOOKS,
        );
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
            permission_tool: None,
            allowed_tools: &[],
            delegated: None,
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
