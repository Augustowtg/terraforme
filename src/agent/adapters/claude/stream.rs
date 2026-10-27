//! Parser do `--output-format stream-json` do Claude CLI: cada linha (um
//! JSON) vira zero ou mais [`AgentEvent`]s.

use std::collections::HashSet;

use serde_json::Value;

use super::parse_mode;
use crate::agent::{AgentEvent, ReportedMode};

/// Tamanho máximo do detalhe de uma ferramenta ou resultado na tela.
const DETAIL_MAX: usize = 80;

/// Converte as linhas do `--output-format stream-json` em eventos.
#[derive(Default)]
pub struct StreamParser {
    /// Mensagem em streaming no momento.
    current_message: Option<String>,
    /// Mensagens cujo texto já chegou em trechos; o texto inteiro que vem
    /// depois nelas é ignorado para não duplicar.
    streamed: HashSet<String>,
}

impl StreamParser {
    pub fn parse(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => vec![AgentEvent::Init {
                session_id: text(&value["session_id"]),
                model: text(&value["model"]),
                permission_mode: reported_mode(&value["permissionMode"]),
            }],
            Some("stream_event") => self.stream_event(&value["event"]),
            Some("assistant") => self.assistant(&value["message"]),
            Some("user") => tool_results(&value["message"]),
            Some("result") => vec![result(&value)],
            _ => Vec::new(),
        }
    }

    fn stream_event(&mut self, event: &Value) -> Vec<AgentEvent> {
        match event["type"].as_str() {
            Some("message_start") => {
                self.current_message = event["message"]["id"].as_str().map(str::to_string);
                Vec::new()
            }
            Some("content_block_start") if event["content_block"]["type"] == "text" => {
                vec![AgentEvent::TextStart]
            }
            Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                if let Some(id) = &self.current_message {
                    self.streamed.insert(id.clone());
                }
                vec![AgentEvent::TextDelta(text(&event["delta"]["text"]))]
            }
            _ => Vec::new(),
        }
    }

    fn assistant(&mut self, message: &Value) -> Vec<AgentEvent> {
        let streamed = message["id"]
            .as_str()
            .is_some_and(|id| self.streamed.contains(id));
        blocks(&message["content"])
            .filter_map(|block| match block["type"].as_str() {
                Some("text") if !streamed => Some(AgentEvent::Text(text(&block["text"]))),
                Some("tool_use") => Some(AgentEvent::ToolUse {
                    name: text(&block["name"]),
                    detail: tool_detail(&block["input"]),
                }),
                _ => None,
            })
            .collect()
    }
}

fn tool_results(message: &Value) -> Vec<AgentEvent> {
    blocks(&message["content"])
        .filter(|block| block["type"] == "tool_result")
        .map(|block| {
            let content = match &block["content"] {
                Value::String(s) => s.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            AgentEvent::ToolResult {
                is_error: block["is_error"].as_bool().unwrap_or(false),
                summary: first_line(&content),
            }
        })
        .collect()
}

fn result(value: &Value) -> AgentEvent {
    let is_error = value["is_error"].as_bool().unwrap_or(false) || value["subtype"] != "success";
    let message = is_error.then(|| {
        value["result"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("erro do claude: {}", text(&value["subtype"])))
    });
    let usage = &value["usage"];
    // Só entrada e saída. Os tokens de cache (o contexto fixo do Claude Code,
    // ~12k por chamada) ficam de fora do consumo.
    let tokens = ["input_tokens", "output_tokens"]
        .iter()
        .filter_map(|key| usage[key].as_u64())
        .sum();
    AgentEvent::Done {
        is_error,
        message,
        tokens,
        denied: value["permission_denials"].as_array().map_or(0, Vec::len),
    }
}

/// O dado mais útil da chamada: o comando, o arquivo, o padrão...
fn tool_detail(input: &Value) -> String {
    const KEYS: [&str; 8] = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ];
    KEYS.iter()
        .find_map(|key| input[key].as_str())
        .map(first_line)
        .unwrap_or_default()
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

/// O modo do `init`, traduzido para o terra.
fn reported_mode(value: &Value) -> ReportedMode {
    match value.as_str() {
        None | Some("") => ReportedMode::Missing,
        Some(name) => parse_mode(name).map_or_else(
            || ReportedMode::Unknown(name.to_string()),
            ReportedMode::Known,
        ),
    }
}

pub(super) fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// Primeira linha não vazia, cortada em [`DETAIL_MAX`] caracteres.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > DETAIL_MAX {
        let cut: String = line.chars().take(DETAIL_MAX - 1).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = StreamParser::default();
        lines.iter().flat_map(|l| parser.parse(l)).collect()
    }

    #[test]
    fn parses_streamed_text_without_duplicating() {
        let events = parse_all(&[
            r#"{"type":"system","subtype":"hook_started","session_id":"s1"}"#,
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-5-5","permissionMode":"auto"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"o"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"i"}}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"oi"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"oi","session_id":"s1","permission_denials":[],"usage":{"input_tokens":2,"cache_creation_input_tokens":10,"cache_read_input_tokens":5,"output_tokens":4}}"#,
        ]);
        assert_eq!(
            events,
            [
                AgentEvent::Init {
                    session_id: "s1".into(),
                    model: "claude-opus-5-5".into(),
                    permission_mode: ReportedMode::Known(crate::agent::PermissionMode::Agente),
                },
                AgentEvent::TextStart,
                AgentEvent::TextDelta("o".into()),
                AgentEvent::TextDelta("i".into()),
                AgentEvent::Done {
                    is_error: false,
                    message: None,
                    tokens: 6,
                    denied: 0
                },
            ]
        );
    }

    #[test]
    fn parses_tool_use_and_result() {
        let events = parse_all(&[
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Vou listar."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la\nfoo","description":"Lista"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"\ntotal 8\nfile","is_error":false}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"negado"}],"is_error":true}]}}"#,
        ]);
        assert_eq!(
            events,
            [
                AgentEvent::Text("Vou listar.".into()),
                AgentEvent::ToolUse {
                    name: "Bash".into(),
                    detail: "ls -la".into()
                },
                AgentEvent::ToolResult {
                    is_error: false,
                    summary: "total 8".into()
                },
                AgentEvent::ToolResult {
                    is_error: true,
                    summary: "negado".into()
                },
            ]
        );
    }

    #[test]
    fn parses_error_result_and_denials() {
        let events = parse_all(&[
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"permission_denials":[{"tool_name":"Bash"}],"usage":{}}"#,
            "não é json",
        ]);
        assert_eq!(
            events,
            [AgentEvent::Done {
                is_error: true,
                message: Some("erro do claude: error_during_execution".into()),
                tokens: 0,
                denied: 1
            }]
        );
    }

    #[test]
    fn first_line_skips_blank_and_truncates() {
        assert_eq!(first_line("\n  \n abc \nxyz"), "abc");
        let long = "x".repeat(100);
        assert_eq!(first_line(&long).chars().count(), DETAIL_MAX);
    }
}
