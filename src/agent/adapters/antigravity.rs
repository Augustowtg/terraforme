//! O Antigravity CLI (`agy`, do Google) como agente: [`AntigravityAdapter`]
//! adapta o CLI ao contrato [`ChatAdapter`] do chat.
//!
//! Cada mensagem roda `agy -p` com entrada e saída em `stream-json`; a partir
//! da segunda, com `--conversation <id>` para manter a conversa. Uma thread
//! lê a saída (um JSON por linha) e a transforma em [`AgentEvent`]s.
//!
//! Permissões: o stream do `-p` não traz pedidos de permissão (o `agy`
//! nega sozinho o que pediria), então o equivalente do
//! `--permission-prompt-tool` é um gancho `PreToolUse` num plugin temporário
//! (veja [`permission`]). Nos modos Manual e Accept Edits, ele leva cada
//! pedido ao chat do terra, que o mostra ao usuário como o do Claude (um
//! [`AgentEvent::PermissionRequest`], respondido em [`Session::respond`]);
//! nos chats delegados, leva a quem delegou. No Yolo, o `agy` aprova tudo;
//! no Plan e no Agente, ele decide sozinho pelo modo.
//!
//! O que o `agy` (1.4.8) não tem, e por isso fica de fora:
//! - MCP por processo: o `agy` não tem flag para isso. Os servidores das
//!   extensões vão num plugin temporário passado em `--add-dir` (veja
//!   [`plugin`]), sem gravar nos arquivos do usuário;
//! - hooks das extensões: são do Claude Code (evento, matcher, comandos como
//!   `rtk hook claude`), e o `agy` manda outro payload. Não vão; o usuário
//!   instala os do ai-memory no `hooks.json` com o `ai-memory install-hooks
//!   --agent antigravity-cli --apply`;
//! - sub agentes por processo;
//! - contas: um login só (o do Google), guardado no chaveiro do sistema. A
//!   pasta `~/.gemini` (ou outro `HOME`) separa só as conversas e a
//!   configuração: o `agy` entra na mesma conta do chaveiro de qualquer
//!   jeito, então o terra não tem conta por pasta como no Claude;
//! - login fora da tela: o `agy` não tem subcomando de login (`agy login`
//!   é um erro); o `/login` só existe na interface dele;
//! - tokens por dia direto do `agy`: as conversas ficam em bancos SQLite com
//!   protobuf sem esquema público. O terra soma o que ele mesmo vê no
//!   stream (o `usage` de cada chamada ao modelo), num registro em
//!   `~/.terraforme/antigravity-usage.jsonl` (veja [`daily`]); o gasto fora
//!   do terra (a interface original) e o anterior ao registro não entram;
//! - a janela de contexto: o stream e o `agy models` não a informam. Vale
//!   uma tabela por família de modelo (os limites que o `agy` grava nas
//!   conversas) e, fora dela, o padrão de 200k do chat.

mod catalog;
mod channel;
mod config;
mod daily;
pub mod permission;
mod plugin;
mod stream;

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use self::catalog::{Families, Labels};
use self::permission::Gate;
use self::plugin::Overlay;
use self::stream::StreamParser;
use crate::agent::{
    AgentEvent, Catalog, ChatAdapter, CommandRoute, DailyTokens, Interactive, PermissionMode,
    ProviderMcp, ProviderSkill, Request, Session, Usage, kill_tree, launch, own_group,
};
use crate::extension::Contribution;

const NOT_FOUND: &str =
    "Antigravity CLI (agy) não encontrado no PATH. Instale em https://antigravity.google/docs/cli";

/// Os comandos do `agy` que só têm interface no modo interativo (os
/// painéis do `/help` do 1.4.7): abrem o `agy` original.
const INTERACTIVE: &[&str] = &[
    "agents",
    "changelog",
    "config",
    "settings",
    "effort",
    "help",
    "hooks",
    "permissions",
    "plugin",
    "plugins",
    "skills",
    // O apelido do `/usage`, que o terra intercepta só com esse nome.
    "quota",
];
/// Os de créditos e compra, que o terra não roda.
const BILLING: &[&str] = &["credits"];

/// O Antigravity CLI do `PATH` como provedor do chat.
pub struct AntigravityAdapter {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    /// Onde fica o `~/.gemini`.
    home: Option<PathBuf>,
    /// Os nomes dos modelos, do último catálogo.
    labels: Labels,
    /// Os esforços dos modelos que o catálogo agrupou (ver [`catalog`]).
    families: Families,
}

impl AntigravityAdapter {
    /// Procura o `agy` no `PATH`.
    pub fn detect() -> Self {
        Self {
            bin: find_agy(),
            home: dirs::home_dir(),
            labels: Arc::new(Mutex::new(HashMap::new())),
            families: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// O modelo que o `agy` recebe. O `--effort` do `agy` conflita com os
    /// ids que já levam o nível (`gemini-3.8-flash-high`) e falha nos
    /// modelos sem esforço, então o esforço escolhido vira o sufixo do id
    /// (`gemini-3.8-flash` + `low` = `gemini-3.8-flash-low`). Um esforço que
    /// o modelo não tem, ou sem modelo, é ignorado: vale o do `agy`.
    fn model_id(&self, model: Option<&str>, effort: Option<&str>) -> Option<String> {
        let model = model?;
        let families = self.families.lock().unwrap();
        let supported = families
            .get(model)
            .zip(effort)
            .is_some_and(|(levels, effort)| levels.iter().any(|level| level == effort));
        Some(match effort {
            Some(effort) if supported => format!("{model}-{effort}"),
            _ => model.to_string(),
        })
    }
}

impl ChatAdapter for AntigravityAdapter {
    fn name(&self) -> &str {
        "Antigravity"
    }

    fn id(&self) -> &str {
        "antigravity"
    }

    fn is_available(&self) -> bool {
        self.bin.is_some()
    }

    fn unavailable_hint(&self) -> String {
        NOT_FOUND.to_string()
    }

    /// O erro do `agy` sem login: "You are not logged into Antigravity".
    fn login_error_words(&self) -> &[&str] {
        &["not logged in"]
    }

    /// O `agy` não tem comando de login: o `/login` fica na interface dele.
    fn login_help(&self) -> Option<String> {
        Some(
            "O Antigravity não tem login pelo terra: rode `agy` num terminal e use /login lá."
                .to_string(),
        )
    }

    /// O `agentMode` salvo no `/config` do `agy` (é um só, sem conta).
    fn configured_mode(&self, _cwd: &Path, _account: Option<&str>) -> Option<PermissionMode> {
        config::configured_mode(self.home.as_deref()?)
    }

    fn mcp_servers(&self, _cwd: &Path, _account: Option<&str>) -> Vec<ProviderMcp> {
        self.home
            .as_deref()
            .map(config::mcp_servers)
            .unwrap_or_default()
    }

    fn skills(&self, cwd: &Path, _account: Option<&str>) -> Vec<ProviderSkill> {
        self.home
            .as_deref()
            .map(|home| config::skills(home, cwd))
            .unwrap_or_default()
    }

    /// Os painéis do `agy` abrem a interface dele; os créditos não rodam; o
    /// resto (ex.: as skills) vai para o agente.
    fn command_route(&self, name: &str) -> CommandRoute {
        if BILLING.contains(&name) {
            CommandRoute::Blocked
        } else if INTERACTIVE.contains(&name) {
            CommandRoute::Original
        } else {
            CommandRoute::Agent
        }
    }

    /// O `agy` interativo, na conversa do chat e com o comando como primeiro
    /// prompt (`--prompt-interactive`). No Yolo,
    /// `--dangerously-skip-permissions`: fora da tela, o terra não aprova
    /// nada.
    fn interactive(&self, cwd: &Path, options: Interactive<'_>) -> Option<Command> {
        let bin = self.bin.as_ref()?;
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
            cmd.args(mode_args(mode));
        }
        if let Some(model) = self.model_id(options.model, options.effort) {
            cmd.args(["--model", &model]);
        }
        if let Some(extensions) = options.extensions {
            cmd.envs(extensions.env.iter().map(|(k, v)| (k, v)));
            // O plugin com o MCP das extensões; quem o apaga é o próximo
            // `interactive` (a pasta não acompanha o `Command`). Se não der
            // para montar, o `agy` abre sem ele.
            if let Ok(Some(overlay)) = self.overlay(extensions) {
                cmd.arg("--add-dir").arg(overlay.keep_as_interactive());
            }
        }
        if let Some(session) = options.session_id {
            cmd.args(["--conversation", session]);
        }
        if let Some(command) = options.command {
            // Com `=`, o comando não é lido como uma flag.
            cmd.arg(format!("--prompt-interactive={command}"));
        }
        Some(cmd)
    }

    /// Os modelos do `agy models` e os comandos e skills do `/help` e do
    /// `/skills`, sem chamar o modelo.
    fn load_catalog(&self, _account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        let bin = self.bin.clone()?;
        Some(catalog::load(
            bin,
            Arc::clone(&self.labels),
            Arc::clone(&self.families),
        ))
    }

    /// As cotas do `/usage` (sem gastar tokens).
    fn load_usage(&self, _account: Option<&str>) -> Option<Receiver<Result<Usage, String>>> {
        Some(catalog::load_usage(self.bin.clone()?))
    }

    /// Os tokens por dia, do registro que o terra grava dos chats com o
    /// `agy` (não há conta: o login é um só).
    fn load_daily(
        &self,
        _account: Option<&str>,
        days: usize,
    ) -> Option<Receiver<Result<Vec<DailyTokens>, String>>> {
        Some(daily::load(daily::default_file()?, days))
    }

    /// O nome do catálogo (ex.: `Gemini 3.8 Flash (High)`); antes dele, o id.
    fn model_label(&self, id: &str) -> String {
        self.labels
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String> {
        let bin = self.bin.as_ref().ok_or(NOT_FOUND)?;
        let overlay = self.overlay(request.extensions)?;
        let model = self.model_id(request.model, request.effort);
        let request = Request {
            model: model.as_deref(),
            effort: None,
            ..request
        };
        let gate = permission_gate(&request)?;
        Ok(Box::new(Run::start(bin, request, overlay, gate)?))
    }
}

impl AntigravityAdapter {
    /// O plugin temporário com os MCP das extensões, menos os que o usuário
    /// já tem no `mcp_config.json` (o mesmo nome entraria duas vezes).
    fn overlay(&self, extensions: &Contribution) -> Result<Option<Overlay>, String> {
        let user: HashSet<String> = self
            .home
            .as_deref()
            .map(config::mcp_servers)
            .unwrap_or_default()
            .into_iter()
            .map(|mcp| mcp.server.name)
            .collect();
        Overlay::create(extensions, &user)
    }
}

/// O gancho de permissão da mensagem: leva os pedidos a quem delegou, num
/// chat delegado, ou ao usuário, no Manual e no Accept Edits (o `agy` não
/// tem um `--permission-prompt-tool`). `None`: o `agy` decide sozinho.
fn permission_gate(request: &Request<'_>) -> Result<Option<Gate>, String> {
    let delegated = match request.permission_tool {
        Some((server, _)) if permission::gated(request.mode) => {
            Gate::create(request.extensions, server, request.mode)?
        }
        _ => None,
    };
    match delegated {
        None if permission::asks_user(request.mode) => {
            Gate::create_for_user(request.mode).map(Some)
        }
        gate => Ok(gate),
    }
}

/// As flags do modo. O `agy` só tem `--mode` `auto`, `accept-edits` e
/// `plan`; sem `--mode`, ele revisa as edições (o mais próximo do Manual).
/// O Yolo é `--dangerously-skip-permissions`: no `-p` não há pedidos de
/// permissão para o terra aprovar. Uma conta sem o `auto` fica no modo
/// padrão (o `agy` avisa no stderr).
fn mode_args(mode: PermissionMode) -> Vec<&'static str> {
    match mode {
        PermissionMode::Agente => vec!["--mode", "auto"],
        PermissionMode::Yolo => vec!["--dangerously-skip-permissions"],
        PermissionMode::Manual => Vec::new(),
        PermissionMode::AcceptEdits => vec!["--mode", "accept-edits"],
        PermissionMode::Plan => vec!["--mode", "plan"],
    }
}

/// Argumentos do `agy` para uma mensagem. O prompt vai no stdin; `-p=`
/// vazio liga o modo sem prompt na linha de comando (um `-p` sem valor
/// tomaria a flag seguinte como prompt).
fn cli_args<'a>(
    mode: PermissionMode,
    session: Option<&'a str>,
    model: Option<&'a str>,
    effort: Option<&'a str>,
) -> Vec<&'a str> {
    let mut args = vec![
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ];
    args.extend(mode_args(mode));
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    if let Some(effort) = effort {
        args.extend(["--effort", effort]);
    }
    if let Some(id) = session {
        args.extend(["--conversation", id]);
    }
    args.push("-p=");
    args
}

/// A mensagem do usuário no formato do `--input-format stream-json` do `agy`.
fn user_message(prompt: &str) -> String {
    serde_json::json!({
        "event": "user",
        "message": { "role": "user", "content": prompt },
    })
    .to_string()
}

/// Procura o executável do Antigravity CLI no `PATH`.
fn find_agy() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["agy.exe", "agy.cmd"]
    } else {
        &["agy"]
    };
    env::split_paths(&env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// O Antigravity no `ai-memory run`.
const HARNESS: &str = "antigravity";

/// O processo do `agy`. Um `.cmd` no Windows precisa do `cmd.exe`.
fn command(agy: &Path) -> Command {
    if agy.extension().is_some_and(|ext| ext == "cmd") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(agy);
        cmd
    } else {
        Command::new(agy)
    }
}

/// Uma resposta do agente em andamento. Encerrar (drop) mata o processo.
struct Run {
    child: Child,
    events: Receiver<AgentEvent>,
    /// O plugin com o MCP das extensões; o `child` morre antes (os campos
    /// caem na ordem), e a pasta some depois.
    _overlay: Option<Overlay>,
    /// O plugin do gancho de permissão e, quando o usuário decide, o canal
    /// dos pedidos.
    gate: Option<Gate>,
}

type Stdin = Arc<Mutex<Option<ChildStdin>>>;

impl Run {
    fn start(
        agy: &Path,
        request: Request<'_>,
        overlay: Option<Overlay>,
        gate: Option<Gate>,
    ) -> Result<Run, String> {
        // O delegado não passa pelos lançadores: o `ai-memory run` só deixa
        // um lançador por workstream, e ele roda junto com quem delegou.
        let mut cmd = match request.delegated {
            Some(_) => command(agy),
            None => launch(
                agy,
                HARNESS,
                request.extensions,
                request.session_id.is_none(),
                command,
            ),
        };
        cmd.args(cli_args(
            request.mode,
            request.session_id,
            request.model,
            request.effort,
        ));
        if let Some(overlay) = &overlay {
            cmd.arg("--add-dir").arg(overlay.dir());
        }
        if let Some(gate) = &gate {
            // O gancho decide por cada ferramenta: sem isto, o `agy` negaria
            // sozinho o que pede permissão.
            cmd.arg("--dangerously-skip-permissions")
                .arg("--add-dir")
                .arg(gate.dir());
        }
        cmd.envs(request.extensions.env.iter().map(|(k, v)| (k, v)));
        // Num grupo próprio, para o fim do turno levar junto o que os
        // lançadores abrem.
        own_group(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("não foi possível iniciar o agy: {err}"))?;

        // O stdin fica aberto até o fim do turno: fechar é o que encerra o
        // `agy` (ele roda um turno por mensagem do stdin).
        let stdin: Stdin = Arc::new(Mutex::new(child.stdin.take()));
        let message = user_message(request.prompt);
        let model = request.model.map(str::to_string);

        let mut stderr = child.stderr.take().expect("stderr configurado como pipe");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });

        let stdout = child.stdout.take().expect("stdout configurado como pipe");
        let (tx, events) = mpsc::channel();
        thread::spawn(move || {
            send_line(&stdin, &message);
            let mut parser = StreamParser::new(model.as_deref());
            let usage_file = daily::default_file();
            let mut done = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let events = parser.parse(&line);
                // Na hora, para o consumo de um turno interrompido não se perder.
                if let Some(file) = &usage_file {
                    daily::record(file, &parser.take_spends());
                }
                for event in events {
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
                    .unwrap_or_else(|| "o agy encerrou sem responder".to_string());
                let _ = tx.send(AgentEvent::Failed(message));
            }
        });

        Ok(Run {
            child,
            events,
            _overlay: overlay,
            gate,
        })
    }
}

impl Session for Run {
    fn poll(&mut self) -> Vec<AgentEvent> {
        let mut events: Vec<AgentEvent> = self.events.try_iter().collect();
        if let Some(gate) = &self.gate {
            // Os pedidos entram antes do `Done`, que fecha a resposta.
            let at = events
                .iter()
                .position(|e| matches!(e, AgentEvent::Done { .. } | AgentEvent::Failed(_)))
                .unwrap_or(events.len());
            events.splice(at..at, gate.poll());
        }
        events
    }

    fn respond(&mut self, id: &str, allow: bool) {
        if let Some(gate) = &self.gate {
            gate.respond(id, allow);
        }
    }
}

/// Escreve uma linha JSON no stdin do `agy`. Se ele já fechou, desiste.
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

#[cfg(test)]
mod tests {
    use super::channel::ask as channel_ask;
    use super::*;
    use std::fs;

    #[test]
    fn has_no_accounts_and_login_is_only_in_the_agy_interface() {
        let agy = AntigravityAdapter {
            bin: None,
            home: None,
            labels: Arc::new(Mutex::new(HashMap::new())),
            families: Arc::default(),
        };
        assert!(agy.accounts().is_empty(), "um login só, no chaveiro");
        assert!(agy.login(None).is_none(), "o agy não tem `agy login`");
        assert!(agy.login_help().unwrap().contains("/login"));
        assert!(
            agy.login_error_words()
                .iter()
                .any(|w| "you are not logged into antigravity".contains(w))
        );
    }

    #[test]
    fn cli_args_read_the_prompt_from_stdin_and_resume_the_conversation() {
        let args = cli_args(PermissionMode::Plan, Some("c1"), Some("m"), Some("high"));
        assert_eq!(
            args,
            [
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--mode",
                "plan",
                "--model",
                "m",
                "--effort",
                "high",
                "--conversation",
                "c1",
                "-p=",
            ]
        );
        let args = cli_args(PermissionMode::Manual, None, None, None);
        assert!(!args.contains(&"--mode"));
        assert!(!args.contains(&"--conversation"));
        assert_eq!(args.last(), Some(&"-p="));
    }

    #[test]
    fn modes_become_agy_flags() {
        assert_eq!(mode_args(PermissionMode::Agente), ["--mode", "auto"]);
        assert_eq!(
            mode_args(PermissionMode::Yolo),
            ["--dangerously-skip-permissions"]
        );
        assert!(mode_args(PermissionMode::Manual).is_empty());
        assert_eq!(
            mode_args(PermissionMode::AcceptEdits),
            ["--mode", "accept-edits"]
        );
    }

    #[test]
    fn the_user_message_has_the_event_field() {
        let message: serde_json::Value = serde_json::from_str(&user_message("oi")).unwrap();
        assert_eq!(message["event"], "user");
        assert_eq!(message["message"]["content"], "oi");
    }

    #[test]
    fn panels_open_the_original_and_credits_are_blocked() {
        let adapter = AntigravityAdapter {
            bin: None,
            home: None,
            labels: Arc::default(),
            families: Arc::default(),
        };
        assert_eq!(adapter.command_route("config"), CommandRoute::Original);
        assert_eq!(adapter.command_route("credits"), CommandRoute::Blocked);
        assert_eq!(adapter.command_route("automation"), CommandRoute::Agent);
        assert_eq!(adapter.id(), "antigravity");
    }

    #[test]
    fn the_effort_becomes_the_model_level_suffix() {
        let adapter = AntigravityAdapter {
            bin: None,
            home: None,
            labels: Arc::default(),
            families: Arc::default(),
        };
        adapter.families.lock().unwrap().insert(
            "gemini-x".into(),
            vec!["low".into(), "medium".into(), "high".into()],
        );
        let id = |model, effort| adapter.model_id(model, effort);
        assert_eq!(
            id(Some("gemini-x"), Some("low")).as_deref(),
            Some("gemini-x-low")
        );
        // Esforço que o modelo não tem, ou sem esforço: o do `agy`.
        assert_eq!(
            id(Some("gemini-x"), Some("max")).as_deref(),
            Some("gemini-x")
        );
        assert_eq!(id(Some("gemini-x"), None).as_deref(), Some("gemini-x"));
        // Sem nível no catálogo (ex.: Claude), o esforço do terra não vai.
        assert_eq!(
            id(Some("claude-y"), Some("high")).as_deref(),
            Some("claude-y")
        );
        // Sem modelo, vale o padrão do `agy`, que já tem o nível.
        assert_eq!(id(None, Some("high")), None);
    }

    #[test]
    fn model_labels_come_from_the_catalog() {
        let adapter = AntigravityAdapter {
            bin: None,
            home: None,
            labels: Arc::default(),
            families: Arc::default(),
        };
        assert_eq!(adapter.model_label("gemini-x"), "gemini-x");
        adapter
            .labels
            .lock()
            .unwrap()
            .insert("gemini-x".into(), "Gemini X".into());
        assert_eq!(adapter.model_label("gemini-x"), "Gemini X");
    }

    /// Um `agy` de mentira que só espera.
    #[cfg(unix)]
    fn sleeping_agy() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = plugin::make_dir().unwrap();
        let path = dir.join("agy");
        fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn request(mode: PermissionMode, extensions: &Contribution) -> Request<'_> {
        Request {
            prompt: "oi",
            mode,
            session_id: None,
            model: None,
            account: None,
            effort: None,
            subagent_model: None,
            extensions,
            permission_tool: None,
            allowed_tools: &[],
            delegated: None,
        }
    }

    #[test]
    #[cfg(unix)]
    fn manual_and_accept_edits_show_the_agy_permission_requests_to_the_user() {
        let agy = sleeping_agy();
        let extensions = Contribution::default();
        for mode in [PermissionMode::Manual, PermissionMode::AcceptEdits] {
            let mut run = Run::start(
                &agy,
                request(mode, &extensions),
                None,
                Some(Gate::create_for_user(mode).unwrap()),
            )
            .unwrap();
            let file = run.gate.as_ref().unwrap().dir().join("channel");
            let hook = thread::spawn(move || {
                channel_ask(
                    &file,
                    "run_command",
                    &serde_json::json!({ "CommandLine": "ls" }),
                )
            });
            // O pedido chega pela sessão, como o do Claude, e a resposta do
            // usuário volta ao gancho.
            let (id, name, detail) = loop {
                if let Some(AgentEvent::PermissionRequest { id, name, detail }) =
                    run.poll().into_iter().next()
                {
                    break (id, name, detail);
                }
                thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!((name.as_str(), detail.as_str()), ("run_command", "ls"));
            run.respond(&id, mode == PermissionMode::Manual);
            assert_eq!(
                hook.join().unwrap().unwrap().is_ok(),
                mode == PermissionMode::Manual
            );
        }
        let _ = fs::remove_dir_all(agy.parent().unwrap());
    }

    #[test]
    fn only_manual_and_accept_edits_get_the_user_gate_in_a_normal_chat() {
        let extensions = Contribution::default();
        let gate = |mode| permission_gate(&request(mode, &extensions)).unwrap();
        for mode in [PermissionMode::Manual, PermissionMode::AcceptEdits] {
            let gate = gate(mode).expect("o gancho do usuário");
            assert!(gate.dir().join("channel").exists());
        }
        for mode in [
            PermissionMode::Agente,
            PermissionMode::Yolo,
            PermissionMode::Plan,
        ] {
            assert!(gate(mode).is_none(), "{mode:?} decide sem o gancho");
        }
    }
}
