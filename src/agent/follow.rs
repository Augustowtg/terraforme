//! Acompanha a saída de uma execução despachada (`output.jsonl`, em
//! stream-json) como uma [`Session`]: a aba de execução é um chat só de
//! leitura que mostra o que o agente faz enquanto ele roda. Cada provedor
//! passa o leitor do stream dele.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use super::{AgentEvent, Session};

/// A espera entre o fim do arquivo e a próxima leitura.
const POLL: Duration = Duration::from_millis(200);

struct Follow {
    events: Receiver<AgentEvent>,
}

impl Session for Follow {
    fn poll(&mut self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }

    /// A execução roda sem perguntar nada: não há a quem responder.
    fn respond(&mut self, _id: &str, _allow: bool) {}
}

/// Segue `path` até a linha do fim (a que contém `end`) ou até a sessão ser
/// fechada, transformando cada linha em eventos com `parse`. Os pedidos de
/// permissão não aparecem: a execução nega sozinha o que não está liberado,
/// e o resultado diz o que foi negado.
pub fn follow(
    path: &Path,
    end: &'static str,
    mut parse: impl FnMut(&str) -> Vec<AgentEvent> + Send + 'static,
) -> Box<dyn Session> {
    let (tx, events) = mpsc::channel();
    let path: PathBuf = path.to_path_buf();
    thread::spawn(move || {
        let mut reader = None;
        let mut line = String::new();
        loop {
            if reader.is_none() {
                reader = File::open(&path).ok().map(BufReader::new);
            }
            let Some(input) = reader.as_mut() else {
                thread::sleep(POLL);
                continue;
            };
            match input.read_line(&mut line) {
                // Uma linha pela metade continua na próxima leitura.
                Ok(0) | Err(_) => thread::sleep(POLL),
                Ok(_) if !line.ends_with('\n') => thread::sleep(POLL),
                Ok(_) => {
                    let last = line.contains(end);
                    for event in parse(line.trim_end()) {
                        if matches!(
                            event,
                            AgentEvent::PermissionRequest { .. } | AgentEvent::Blocked { .. }
                        ) {
                            continue;
                        }
                        if tx.send(event).is_err() {
                            return;
                        }
                    }
                    line.clear();
                    if last {
                        return;
                    }
                }
            }
        }
    });
    Box::new(Follow { events })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    #[test]
    fn follows_the_file_while_it_grows_until_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.jsonl");
        let mut session = crate::agent::adapters::claude::follow(&path);
        let mut file = File::create(&path).unwrap();
        writeln!(file, r#"{{"type":"system","subtype":"init","session_id":"s","model":"claude-haiku-5-5","permissionMode":"dontAsk"}}"#).unwrap();
        write!(
            file,
            r#"{{"type":"result","subtype":"success","is_error":false,"result":"pronto","#
        )
        .unwrap();
        file.flush().unwrap();
        thread::sleep(Duration::from_millis(300));
        writeln!(file, r#""usage":{{"input_tokens":1,"output_tokens":2}}}}"#).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while !events.iter().any(|e| matches!(e, AgentEvent::Done { .. })) {
            assert!(Instant::now() < deadline, "{events:?}");
            events.extend(session.poll());
            thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(events[0], AgentEvent::Init { .. }));
    }
}
