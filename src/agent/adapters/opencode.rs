//! O OpenCode CLI (`opencode`) como agente: [`OpenCodeAdapter`] adapta o CLI
//! ao contrato [`ChatAdapter`] do chat.
//!
//! Cada mensagem roda `opencode run --format json`; a partir da segunda, com
//! `-s <sessão>` para manter a conversa. O prompt vai pela entrada, que é
//! fechada em seguida (é o fim da entrada que manda o OpenCode responder).
//! Uma thread lê a saída (um JSON por linha) e a transforma em
//! [`AgentEvent`]s.
//!
//! O que o `run` não tem, e por isso fica de fora:
//! - pedidos de permissão: o `--format json` não os expõe. O OpenCode segue a
//!   permissão da configuração dele; no Yolo, o terra passa `--auto`;
//! - contas por pasta: o OpenCode guarda as credenciais por provider, e o
//!   login (`opencode auth login`) é interativo no terminal dele. O `/login`
//!   só mostra como entrar;
//! - hooks e MCP das extensões: são do Claude Code. As variáveis de ambiente
//!   das extensões entram, como nos outros provedores;
//! - prompt de sistema por processo: o `run` não tem flag para isso. A
//!   instrução dos sub agentes (`terra send`) vai na frente da primeira
//!   mensagem da conversa (veja [`with_system_prompt`]);
//! - os comandos da `/`: os do OpenCode não são listados no catálogo, e os
//!   que o terra não implementa vão para o agente.

mod catalog;
mod daily;
mod stream;

use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use self::stream::StreamParser;
use crate::agent::{
    AgentEvent, Catalog, ChatAdapter, Interactive, PermissionMode, Request, Session, TokenReport,
    kill_tree, launch, own_group,
};

const NOT_FOUND: &str = "OpenCode CLI não encontrado no PATH. Instale em https://opencode.ai";

/// O OpenCode CLI do `PATH` como provedor do chat.
pub struct OpenCodeAdapter {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
}

impl OpenCodeAdapter {
    /// Procura o `opencode` no `PATH`.
    pub fn detect() -> Self {
        Self {
            bin: find_opencode(),
        }
    }
}

impl ChatAdapter for OpenCodeAdapter {
    fn name(&self) -> &str {
        NAME
    }

    fn id(&self) -> &str {
        "opencode"
    }

    fn is_available(&self) -> bool {
        self.bin.is_some()
    }

    fn unavailable_hint(&self) -> String {
        NOT_FOUND.to_string()
    }

    /// Os avisos de credencial do OpenCode.
    fn login_error_words(&self) -> &[&str] {
        &["not authenticated", "unauthorized", "authentication"]
    }

    /// O login do OpenCode é interativo no terminal dele; o `/login` só
    /// orienta.
    fn login_help(&self) -> Option<String> {
        Some(
            "O OpenCode não tem login pelo terra: rode `opencode auth login` no terminal e abra \
             o terra de novo."
                .to_string(),
        )
    }

    /// O `opencode` interativo, na sessão do chat e com o modelo escolhido.
    /// No Yolo, `--auto`: fora da tela, o terra não aprova nada.
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
        if let Some(model) = options.model {
            cmd.args(["-m", model]);
        }
        if options.mode == Some(PermissionMode::Yolo) {
            cmd.arg("--auto");
        }
        if let Some(extensions) = options.extensions {
            cmd.envs(extensions.env.iter().map(|(k, v)| (k, v)));
        }
        if let Some(session) = options.session_id {
            cmd.args(["-s", session]);
        }
        Some(cmd)
    }

    /// Os modelos do `opencode models`, sem chamar o modelo.
    fn load_catalog(&self, _account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        Some(catalog::load(self.bin.clone()?))
    }

    /// Os tokens por dia do banco do OpenCode.
    fn load_daily(
        &self,
        _account: Option<&str>,
        days: usize,
        workspace: &Path,
    ) -> Option<Receiver<Result<TokenReport, String>>> {
        Some(daily::load(
            daily::default_file()?,
            days,
            workspace.to_path_buf(),
        ))
    }

    /// O nome do modelo como o `opencode models` o informa.
    fn model_label(&self, id: &str) -> String {
        display_model(id)
    }

    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String> {
        let bin = self.bin.as_ref().ok_or(NOT_FOUND)?;
        Ok(Box::new(Run::start(bin, request)?))
    }
}

/// Argumentos do `opencode run` para uma mensagem. O prompt vai pela entrada.
/// No Yolo, `--auto` aprova o que o OpenCode pediria.
fn cli_args<'a>(
    mode: PermissionMode,
    session: Option<&'a str>,
    model: Option<&'a str>,
) -> Vec<&'a str> {
    let mut args = vec!["run", "--format", "json"];
    if mode == PermissionMode::Yolo {
        args.push("--auto");
    }
    if let Some(model) = model {
        args.extend(["-m", model]);
    }
    if let Some(id) = session {
        args.extend(["-s", id]);
    }
    args
}

/// Põe `system` na frente do prompt só na primeira mensagem da conversa
/// (`first`); nas seguintes o OpenCode já o tem no histórico.
fn with_system_prompt(prompt: &str, system: Option<&str>, first: bool) -> String {
    match system {
        Some(system) if first => format!("{system}\n\n{prompt}"),
        _ => prompt.to_string(),
    }
}

/// Procura o executável do OpenCode no `PATH`.
fn find_opencode() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["opencode.exe", "opencode.cmd"]
    } else {
        &["opencode"]
    };
    env::split_paths(&env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// O nome do provedor, como o adapter mostra.
const NAME: &str = "OpenCode";

/// O OpenCode no `ai-memory run`.
const HARNESS: &str = "opencode";

/// O processo do `opencode`. Instalado no npm no Windows, ele é um `.cmd`,
/// que precisa do `cmd.exe`.
fn command(bin: &Path) -> Command {
    if bin.extension().is_some_and(|ext| ext == "cmd") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(bin);
        cmd
    } else {
        Command::new(bin)
    }
}

/// Uma resposta do agente em andamento. Encerrar (drop) mata o processo.
struct Run {
    child: Child,
    events: Receiver<AgentEvent>,
}

impl Run {
    fn start(bin: &Path, request: Request<'_>) -> Result<Run, String> {
        let mut cmd = launch(
            bin,
            HARNESS,
            request.extensions,
            request.session_id.is_none(),
            command,
        );
        cmd.args(cli_args(request.mode, request.session_id, request.model));
        cmd.envs(request.extensions.env.iter().map(|(k, v)| (k, v)));
        // Num grupo próprio, para o fim do turno levar junto o que os
        // lançadores abrem.
        own_group(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("não foi possível iniciar o opencode: {err}"))?;

        let stdin = child.stdin.take();
        let message = with_system_prompt(
            request.prompt,
            request.system_prompt,
            request.session_id.is_none(),
        );
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
            // A entrada é fechada logo depois do prompt: é o fim dela que
            // manda o `opencode run` responder.
            if let Some(mut pipe) = stdin {
                let _ = writeln!(pipe, "{message}");
                let _ = pipe.flush();
            }
            let mut parser = StreamParser::new(model.as_deref());
            let mut done = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                for event in parser.parse(&line) {
                    if matches!(event, AgentEvent::Done { .. }) {
                        done = true;
                    }
                    if tx.send(event).is_err() {
                        return;
                    }
                }
                if done {
                    return;
                }
            }
            if !done {
                let stderr = stderr.join().unwrap_or_default();
                let message = last_lines(&stderr, 3)
                    .unwrap_or_else(|| "o opencode encerrou sem responder".to_string());
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

impl Drop for Run {
    fn drop(&mut self) {
        kill_tree(&mut self.child);
    }
}

fn last_lines(text: &str, n: usize) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines[lines.len().saturating_sub(n)..].join("\n"))
}

/// O id do modelo como o OpenCode o mostra (`provider/modelo`).
fn display_model(id: &str) -> String {
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::Contribution;

    #[test]
    fn cli_args_run_json_and_resume_the_session() {
        let args = cli_args(PermissionMode::Plan, Some("ses_1"), Some("opencode/modelo"));
        assert_eq!(
            args,
            [
                "run",
                "--format",
                "json",
                "-m",
                "opencode/modelo",
                "-s",
                "ses_1"
            ]
        );
        assert!(!args.contains(&"--auto"));

        let args = cli_args(PermissionMode::Yolo, None, None);
        assert_eq!(args, ["run", "--format", "json", "--auto"]);
    }

    #[test]
    fn the_system_prompt_goes_only_in_front_of_the_first_message() {
        assert_eq!(with_system_prompt("oi", Some("sub"), true), "sub\n\noi");
        assert_eq!(with_system_prompt("oi", Some("sub"), false), "oi");
        assert_eq!(with_system_prompt("oi", None, true), "oi");
    }

    #[test]
    fn adapter_without_opencode_is_unavailable() {
        let adapter = OpenCodeAdapter { bin: None };
        assert!(!adapter.is_available());
        assert_eq!(adapter.name(), "OpenCode");
        assert_eq!(adapter.id(), "opencode");
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
            system_prompt: None,
        };
        assert_eq!(adapter.start(request).err().as_deref(), Some(NOT_FOUND));
        assert!(adapter.login(None).is_none());
        assert!(adapter.login_help().is_some());
        assert!(adapter.accounts().is_empty());
    }
}
