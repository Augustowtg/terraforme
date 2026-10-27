//! O `/login` no terminal normal, fora da tela do terra.
//!
//! O comando do provedor (ex.: `claude auth login`) roda com pipes e o terra
//! fica no meio:
//! - o link de login sai inteiro como hiperlink (OSC 8): clicar abre o link
//!   todo, mesmo quebrado em várias linhas, e ele também vai para a área de
//!   transferência;
//! - o código é lido pelo terra, com eco (o prompt do Claude não mostra o que
//!   se digita), e mandado ao comando. Código inválido pede de novo.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::clipboard;

/// Onde o link de login aparece na saída do comando.
const URL_MARKER: &str = "visit: ";
const INVALID_MARKER: &str = "Invalid code";
/// O prompt do código do Claude, que o terra substitui pelo dele.
const PROMPT: &str = "Paste code here if prompted >";
/// Quanto esperar o link antes de pedir o código mesmo assim.
const URL_TIMEOUT: Duration = Duration::from_secs(15);
/// Quanto esperar a resposta a um código.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_millis(100);

/// Roda o `login` da conta `account`, lendo o código de `input` e
/// escrevendo em `out`. `Ok` quando o login deu certo; `Err` com o motivo.
pub fn run(
    mut login: Command,
    account: &str,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<(), String> {
    let mut child = login
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("não foi possível iniciar o login: {err}"))?;
    let (tx, lines) = mpsc::channel();
    read_lines(child.stdout.take().expect("stdout em pipe"), tx.clone());
    read_lines(child.stderr.take().expect("stderr em pipe"), tx);
    let mut session = Login {
        child,
        lines,
        out,
        last_line: None,
    };
    let _ = writeln!(session.out, "terra · login na conta {account}\n");
    let result = session.converse(input);
    // Cancelado ou com erro, o comando não fica rodando.
    let _ = session.child.kill();
    let _ = session.child.wait();
    result
}

struct Login<'a, W: Write> {
    child: Child,
    lines: Receiver<String>,
    out: &'a mut W,
    /// Última linha útil da saída, para explicar uma falha.
    last_line: Option<String>,
}

/// Como uma espera terminou.
enum Wait {
    /// O comando terminou: `true` se deu certo.
    Exited(bool),
    /// O comando recusou o código e espera outro.
    Invalid,
    /// Passou o tempo (ou chegou o link).
    Ready,
}

impl<W: Write> Login<'_, W> {
    fn converse(&mut self, input: &mut impl BufRead) -> Result<(), String> {
        if let Wait::Exited(ok) = self.wait(URL_TIMEOUT, true) {
            return self.finish(ok);
        }
        loop {
            let _ = write!(
                self.out,
                "\nCole o código que aparece depois de entrar e aperte Enter (vazio cancela): "
            );
            let _ = self.out.flush();
            let mut code = String::new();
            if input.read_line(&mut code).unwrap_or(0) == 0 {
                return Err("login cancelado".to_string());
            }
            // O código colado não tem espaços; quebras e espaços do terminal saem.
            let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            // O comando pode ter terminado enquanto o terra esperava o código.
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain();
                return self.finish(status.success());
            }
            if code.is_empty() {
                return Err("login cancelado".to_string());
            }
            let sent = self.child.stdin.as_mut().is_some_and(|stdin| {
                writeln!(stdin, "{code}")
                    .and_then(|()| stdin.flush())
                    .is_ok()
            });
            if !sent {
                return Err("o login parou antes do código".to_string());
            }
            let _ = writeln!(self.out, "Verificando o código...");
            match self.wait(ANSWER_TIMEOUT, false) {
                Wait::Exited(ok) => return self.finish(ok),
                Wait::Invalid => {
                    let _ = writeln!(
                        self.out,
                        "Código inválido: copie o código inteiro e cole de novo."
                    );
                }
                Wait::Ready => return Err("o login não respondeu ao código".to_string()),
            }
        }
    }

    /// Mostra a saída do comando até ele terminar, recusar o código ou
    /// passar `timeout`. Com `until_url`, para assim que o link chega.
    fn wait(&mut self, timeout: Duration, until_url: bool) -> Wait {
        let start = Instant::now();
        while start.elapsed() < timeout {
            match self.lines.recv_timeout(STEP) {
                Ok(line) => match self.show(&line) {
                    Shown::Url if until_url => return Wait::Ready,
                    Shown::Invalid => return Wait::Invalid,
                    _ => {}
                },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => thread::sleep(STEP),
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain();
                return Wait::Exited(status.success());
            }
        }
        Wait::Ready
    }

    /// O que ainda estava nos pipes quando o comando terminou.
    fn drain(&mut self) {
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
            self.show(&line);
        }
    }

    /// Escreve uma linha da saída do comando.
    fn show(&mut self, line: &str) -> Shown {
        // O prompt não tem quebra de linha: a resposta vem grudada nele.
        let line = line.trim();
        let line = line.strip_prefix(PROMPT).unwrap_or(line).trim();
        if line.is_empty() {
            return Shown::Other;
        }
        if let Some((_, url)) = line.split_once(URL_MARKER) {
            let url = url.trim();
            let copied = if clipboard::copy(url) {
                " (já copiado para a área de transferência)"
            } else {
                ""
            };
            let _ = writeln!(
                self.out,
                "Se o navegador não abriu, clique ou abra o link{copied}:\n\n{}\n",
                hyperlink(url)
            );
            return Shown::Url;
        }
        if line.contains(INVALID_MARKER) {
            return Shown::Invalid;
        }
        let _ = writeln!(self.out, "{line}");
        self.last_line = Some(line.to_string());
        Shown::Other
    }

    fn finish(&mut self, ok: bool) -> Result<(), String> {
        if ok {
            return Ok(());
        }
        Err(self
            .last_line
            .clone()
            .unwrap_or_else(|| "o login terminou sem concluir".to_string()))
    }
}

/// O que uma linha da saída era.
enum Shown {
    Url,
    Invalid,
    Other,
}

/// O link como hiperlink do terminal (OSC 8): o clique abre o link inteiro,
/// mesmo quando ele ocupa várias linhas.
fn hyperlink(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}

/// Manda cada linha de `pipe` pelo canal, numa thread.
fn read_lines(pipe: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Um "login" de mentira: imprime o link, lê o código (sem quebra de
    /// linha no prompt, como o Claude) e aceita só `bom#estado`.
    fn fake_login() -> Command {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            r##"echo "If the browser didn't open, visit: https://exemplo/auth?a=1&b=2"
               printf 'Paste code here if prompted > '
               while read code; do
                 if [ "$code" = "bom#estado" ]; then echo "Login successful."; exit 0; fi
                 case "$code" in
                   *"#"*) echo "Login failed: status 400" >&2; exit 1 ;;
                 esac
                 echo "Invalid code. Please make sure the full code was copied."
                 printf 'Paste code here if prompted > '
               done
               exit 1"##,
        ]);
        cmd
    }

    fn login(input: &str) -> (Result<(), String>, String) {
        let mut out = Vec::new();
        let result = run(fake_login(), "Default", &mut input.as_bytes(), &mut out);
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn shows_full_link_retries_invalid_code_and_logs_in() {
        let (result, out) = login("ruim\n  bom#estado \n");
        assert_eq!(result, Ok(()));
        assert!(out.starts_with("terra · login na conta Default"), "{out}");
        // O link inteiro, dentro do hiperlink.
        let url = "https://exemplo/auth?a=1&b=2";
        assert!(
            out.contains(&format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")),
            "{out}"
        );
        assert!(out.contains("Código inválido"), "{out}");
        assert!(out.contains("Login successful."), "{out}");
        assert!(!out.contains(PROMPT), "o prompt do Claude não aparece");
    }

    #[test]
    fn failure_reports_the_reason() {
        let (result, _) = login("errado#x\n");
        assert_eq!(result, Err("Login failed: status 400".into()));
    }

    #[test]
    fn empty_code_or_end_of_input_cancels() {
        assert_eq!(login("\n").0, Err("login cancelado".into()));
        assert_eq!(login("").0, Err("login cancelado".into()));
    }

    #[test]
    fn missing_command_is_an_error() {
        let mut out = Vec::new();
        let result = run(
            Command::new("terra-login-inexistente"),
            "Default",
            &mut "".as_bytes(),
            &mut out,
        );
        assert!(
            result
                .unwrap_err()
                .starts_with("não foi possível iniciar o login")
        );
    }
}
