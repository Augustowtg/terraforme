//! Parser do `opencode run --format json`: cada linha (um evento JSON) vira
//! zero ou mais [`AgentEvent`]s.
//!
//! Os eventos do OpenCode:
//! - `step_start`: começa um passo do agente (o modelo foi chamado);
//! - `text`: um bloco de texto da resposta (inteiro, não em trechos);
//! - `reasoning`: o pensamento do modelo;
//! - `tool_use`: a ferramenta chamada, com o estado dela (`input`, `output`,
//!   `error` e o `status`);
//! - `step_finish`: o fim do passo, com os tokens da chamada e o `reason`
//!   (`tool-calls` quando o agente vai continuar; `stop` no fim);
//! - `error`: uma falha do OpenCode, sem resposta.
//!
//! A sessão (`sessionID`) vem em todas as linhas; o `init` sai na primeira.

use std::collections::HashSet;

use serde_json::Value;

use crate::agent::{AgentEvent, CHARS_PER_TOKEN, ReportedMode};

/// Até quantos caracteres vão no resumo de uma ferramenta.
const DETAIL_MAX: usize = 80;

/// Lê o stream de um turno, linha a linha.
#[derive(Debug, Default)]
pub struct StreamParser {
    /// O modelo pedido (o stream não informa o modelo da sessão).
    model: String,
    /// A sessão do turno, para o `init` sair uma vez.
    session: Option<String>,
    /// Os blocos de texto e de pensamento já entregues (pelo id da parte).
    seen: HashSet<String>,
    /// As ferramentas já anunciadas com `ToolUse` (pelo `callID`).
    tools: HashSet<String>,
    /// As ferramentas cujo resultado já saiu.
    results: HashSet<String>,
    /// Entrada, saída e pensamento somados nos passos deste turno.
    tokens: u64,
    /// Criação e leitura de cache somadas nos passos deste turno.
    cache_tokens: u64,
    /// Os caracteres escritos desde o último `step_finish`, que é quando o
    /// uso de verdade chega.
    written_chars: u64,
    /// O último gasto informado com [`AgentEvent::Usage`].
    reported: (u64, u64),
    /// A resposta já terminou (o `Done` saiu).
    done: bool,
}

impl StreamParser {
    pub fn new(model: Option<&str>) -> Self {
        Self {
            model: model.unwrap_or_default().to_string(),
            ..Self::default()
        }
    }

    pub fn parse(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if self.session.is_none()
            && let Some(id) = value["sessionID"].as_str().filter(|id| !id.is_empty())
        {
            self.session = Some(id.to_string());
            events.push(AgentEvent::Init {
                session_id: id.to_string(),
                model: self.model.clone(),
                // O OpenCode não diz o modo de permissão no stream.
                permission_mode: ReportedMode::Missing,
            });
        }
        let part = &value["part"];
        match value["type"].as_str() {
            Some("text") => self.text(part, &mut events),
            Some("reasoning") => self.reasoning(part, &mut events),
            Some("tool_use") => self.tool(part, &mut events),
            Some("step_finish") => self.step_finish(part, &mut events),
            Some("error") => self.error(&value["error"], &mut events),
            _ => {}
        }
        events
    }

    fn text(&mut self, part: &Value, events: &mut Vec<AgentEvent>) {
        let Some(text) = part["text"].as_str().filter(|text| !text.is_empty()) else {
            return;
        };
        if !first_time(&mut self.seen, part) {
            return;
        }
        events.push(AgentEvent::Text(text.to_string()));
        self.written_chars += text.chars().count() as u64;
        self.report_estimate(events);
    }

    fn reasoning(&mut self, part: &Value, events: &mut Vec<AgentEvent>) {
        let Some(text) = part["text"].as_str().filter(|text| !text.trim().is_empty()) else {
            return;
        };
        if !first_time(&mut self.seen, part) {
            return;
        }
        events.push(AgentEvent::Step(format!("Pensamento: {}", text.trim())));
    }

    fn tool(&mut self, part: &Value, events: &mut Vec<AgentEvent>) {
        let call = part["callID"].as_str().unwrap_or_default();
        let name = part["tool"].as_str().unwrap_or("ferramenta");
        let state = &part["state"];
        let known = !call.is_empty();
        if !known || self.tools.insert(call.to_string()) {
            events.push(AgentEvent::ToolUse {
                name: name.to_string(),
                detail: tool_detail(&state["input"]),
            });
        }
        let status = state["status"].as_str().unwrap_or_default();
        let terminal = matches!(status, "completed" | "error");
        if terminal && (!known || self.results.insert(call.to_string())) {
            let output = state["output"]
                .as_str()
                .or(state["error"].as_str())
                .unwrap_or_default();
            events.push(AgentEvent::ToolResult {
                is_error: status == "error",
                summary: first_line(output),
            });
        }
    }

    fn step_finish(&mut self, part: &Value, events: &mut Vec<AgentEvent>) {
        let tokens = &part["tokens"];
        let input = count(&tokens["input"]);
        let output = count(&tokens["output"]);
        let reasoning = count(&tokens["reasoning"]);
        let cache = count(&tokens["cache"]["read"]) + count(&tokens["cache"]["write"]);
        self.tokens += input + output + reasoning;
        self.cache_tokens += cache;
        self.written_chars = 0;
        let total = count(&tokens["total"]);
        let context = if total > 0 {
            total
        } else {
            input + output + reasoning + cache
        };
        if context > 0 {
            events.push(AgentEvent::Context {
                tokens: context,
                window: None,
            });
        }
        self.report_usage(events);
        let reason = part["reason"].as_str().unwrap_or_default();
        if !self.done && reason != "tool-calls" {
            self.done = true;
            events.push(self.done_event(false, None));
        }
    }

    fn error(&mut self, error: &Value, events: &mut Vec<AgentEvent>) {
        if self.done {
            return;
        }
        let message = error["data"]["message"]
            .as_str()
            .or(error["message"].as_str())
            .or(error["name"].as_str())
            .filter(|message| !message.is_empty())
            .unwrap_or("erro do opencode")
            .to_string();
        self.done = true;
        events.push(self.done_event(true, Some(message)));
    }

    fn done_event(&self, is_error: bool, message: Option<String>) -> AgentEvent {
        AgentEvent::Done {
            is_error,
            message,
            tokens: self.tokens,
            cache_tokens: self.cache_tokens,
            denied: 0,
        }
    }

    /// O [`AgentEvent::Usage`] com a estimativa do texto ainda sem o
    /// `step_finish`, quando ela muda.
    fn report_estimate(&mut self, events: &mut Vec<AgentEvent>) {
        let estimate = self.written_chars / CHARS_PER_TOKEN;
        let reported = (self.tokens + estimate, self.cache_tokens);
        if reported != self.reported {
            self.reported = reported;
            events.push(AgentEvent::Usage {
                tokens: reported.0,
                cache_tokens: reported.1,
            });
        }
    }

    /// O [`AgentEvent::Usage`] do gasto de verdade do passo.
    fn report_usage(&mut self, events: &mut Vec<AgentEvent>) {
        let reported = (self.tokens, self.cache_tokens);
        if reported != self.reported {
            self.reported = reported;
            events.push(AgentEvent::Usage {
                tokens: reported.0,
                cache_tokens: reported.1,
            });
        }
    }
}

/// `true` se é a primeira vez que a parte (pelo id) aparece; sem id, vale
/// sempre.
fn first_time(seen: &mut HashSet<String>, part: &Value) -> bool {
    part["id"]
        .as_str()
        .is_none_or(|id| seen.insert(id.to_string()))
}

fn count(value: &Value) -> u64 {
    value.as_u64().unwrap_or_default()
}

/// O dado mais útil da chamada: o comando, o arquivo, a URL...
fn tool_detail(input: &Value) -> String {
    const KEYS: [&str; 12] = [
        "command",
        "filePath",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
        "content",
        "todos",
        "name",
    ];
    KEYS.iter()
        .find_map(|key| input[key].as_str())
        .map(first_line)
        .unwrap_or_default()
}

/// Primeira linha com texto, cortada em [`DETAIL_MAX`] caracteres.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= DETAIL_MAX {
        return line.to_string();
    }
    let cut: String = line.chars().take(DETAIL_MAX - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str =
        r#"{"type":"step_start","sessionID":"ses_1","part":{"id":"p1","type":"step-start"}}"#;
    const STOP: &str = r#"{"type":"step_finish","sessionID":"ses_1","part":{"id":"p3","type":"step-finish","reason":"stop","tokens":{"total":100,"input":10,"output":5,"reasoning":2,"cache":{"write":1,"read":82}}}}"#;

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = StreamParser::new(Some("opencode/modelo"));
        lines.iter().flat_map(|line| parser.parse(line)).collect()
    }

    #[test]
    fn a_turn_becomes_events_and_ends_at_the_stop() {
        let text =
            r#"{"type":"text","sessionID":"ses_1","part":{"id":"p2","type":"text","text":"oi"}}"#;
        let events = parse_all(&[START, text, STOP]);
        assert_eq!(
            events,
            [
                AgentEvent::Init {
                    session_id: "ses_1".into(),
                    model: "opencode/modelo".into(),
                    permission_mode: ReportedMode::Missing,
                },
                AgentEvent::Text("oi".into()),
                AgentEvent::Context {
                    tokens: 100,
                    window: None
                },
                AgentEvent::Usage {
                    tokens: 17,
                    cache_tokens: 83
                },
                AgentEvent::Done {
                    is_error: false,
                    message: None,
                    tokens: 17,
                    cache_tokens: 83,
                    denied: 0
                },
            ]
        );
    }

    #[test]
    fn tool_steps_do_not_end_the_answer() {
        let tool = r#"{"type":"tool_use","sessionID":"ses_1","part":{"id":"p2","type":"tool","tool":"bash","callID":"c1","state":{"status":"completed","input":{"command":"ls -la"},"output":"a.txt\n"}}}"#;
        let tool_done = r#"{"type":"step_finish","sessionID":"ses_1","part":{"id":"p3","type":"step-finish","reason":"tool-calls","tokens":{"total":50,"input":40,"output":3,"cache":{"read":7}}}}"#;
        let events = parse_all(&[START, tool, tool_done, START, STOP]);
        assert!(matches!(events[0], AgentEvent::Init { .. }));
        assert_eq!(
            events[1],
            AgentEvent::ToolUse {
                name: "bash".into(),
                detail: "ls -la".into()
            }
        );
        assert_eq!(
            events[2],
            AgentEvent::ToolResult {
                is_error: false,
                summary: "a.txt".into()
            }
        );
        // O `tool-calls` soma o uso e segue; o `stop` fecha o turno.
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done { tokens: 60, .. })
        ));
    }

    #[test]
    fn reasoning_and_text_parts_appear_once() {
        let reasoning = r#"{"type":"reasoning","sessionID":"ses_1","part":{"id":"r1","type":"reasoning","text":"vou ler"}}"#;
        let text =
            r#"{"type":"text","sessionID":"ses_1","part":{"id":"t1","type":"text","text":"oi"}}"#;
        let events = parse_all(&[START, reasoning, reasoning, text, text]);
        assert_eq!(
            events[1..],
            [
                AgentEvent::Step("Pensamento: vou ler".into()),
                AgentEvent::Text("oi".into()),
            ]
        );
    }

    #[test]
    fn an_error_ends_the_answer_with_its_message() {
        let error = r#"{"type":"error","sessionID":"ses_1","error":{"name":"UnknownError","data":{"message":"modelo inexistente"}}}"#;
        let events = parse_all(&[error]);
        assert_eq!(
            events,
            [
                AgentEvent::Init {
                    session_id: "ses_1".into(),
                    model: "opencode/modelo".into(),
                    permission_mode: ReportedMode::Missing,
                },
                AgentEvent::Done {
                    is_error: true,
                    message: Some("modelo inexistente".into()),
                    tokens: 0,
                    cache_tokens: 0,
                    denied: 0,
                },
            ]
        );
    }

    #[test]
    fn a_failed_tool_reports_its_error() {
        let tool = r#"{"type":"tool_use","sessionID":"ses_1","part":{"id":"p2","type":"tool","tool":"edit","callID":"c1","state":{"status":"error","input":{"filePath":"/etc/x"},"error":"permissão negada"}}}"#;
        let events = parse_all(&[START, tool]);
        assert_eq!(
            events[1..],
            [
                AgentEvent::ToolUse {
                    name: "edit".into(),
                    detail: "/etc/x".into()
                },
                AgentEvent::ToolResult {
                    is_error: true,
                    summary: "permissão negada".into()
                },
            ]
        );
    }

    #[test]
    fn long_details_are_shortened() {
        let long = "x".repeat(200);
        let detail = tool_detail(&serde_json::json!({ "command": long }));
        assert_eq!(detail.chars().count(), DETAIL_MAX);
        assert!(detail.ends_with('…'));
    }
}
