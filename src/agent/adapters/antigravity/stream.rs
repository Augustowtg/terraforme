//! Tradução do `--output-format stream-json` do `agy` para [`AgentEvent`]s.
//!
//! Cada linha é um JSON com `event`:
//! - `init`: a conversa (`conversation_id`) e o modelo;
//! - `step_update`: um passo da conversa (`step_index`, `state` `ACTIVE` ou
//!   `DONE`, `step_type`). Os de `agent_response` trazem o texto aos poucos
//!   (`text_delta`) e, no `DONE`, o `usage` da chamada ao modelo; os de
//!   `tool` trazem `tool_name` e `tool_info` (parâmetros e, no fim, `output`
//!   ou, no `ERROR`, `error.message`). O `checkpoint` é a compactação da
//!   conversa;
//! - `result`: o fim do turno (`status`, `response`, `error`). O `usage` dele
//!   é o da conversa inteira, então os tokens do turno são a soma dos
//!   `usage` dos passos.

use std::collections::HashSet;

use serde_json::Value;

use super::daily::Spend;
use crate::agent::{AgentEvent, ReportedMode};

/// Até quantos caracteres vão no resumo de uma ferramenta.
const SUMMARY_MAX: usize = 80;

/// Lê o stream de um turno, linha a linha.
#[derive(Debug, Default)]
pub struct StreamParser {
    /// O modelo pedido a este turno: o `init` de uma conversa retomada não
    /// traz o modelo, e o chat o apagaria da tela.
    model: String,
    /// O passo de texto em andamento (para o `TextStart` sair uma vez).
    text_step: Option<u64>,
    /// Os passos de ferramenta já anunciados com `ToolUse`.
    tools: HashSet<u64>,
    /// Já saiu algum texto da resposta.
    wrote_text: bool,
    /// Entrada e saída somadas nos passos deste turno.
    tokens: u64,
    /// Leitura de cache somada nos passos deste turno.
    cache_tokens: u64,
    /// A conversa do turno (do `init`, ou do passo).
    conversation: String,
    /// As chamadas ao modelo ainda não entregues ao registro de consumo.
    spends: Vec<Spend>,
}

/// A janela de contexto de um modelo, quando o terra a conhece. O stream e o
/// `agy models` não a informam: os valores são os limites que o `agy` 1.4.8
/// grava, por modelo, nas conversas (Gemini 256k, Claude 160k, GPT-OSS 80k).
/// Um modelo fora da lista fica sem janela, e vale o padrão do chat.
fn context_window(model: &str) -> Option<u64> {
    let model = model.to_ascii_lowercase();
    if model.starts_with("gemini-") {
        Some(256_000)
    } else if model.starts_with("claude-") {
        Some(160_000)
    } else if model.starts_with("gpt-oss") {
        Some(80_000)
    } else {
        None
    }
}

impl StreamParser {
    pub fn new(model: Option<&str>) -> Self {
        Self {
            model: model.unwrap_or_default().to_string(),
            ..Self::default()
        }
    }

    /// As chamadas ao modelo vistas desde a última vez, para o registro de
    /// consumo por dia.
    pub fn take_spends(&mut self) -> Vec<Spend> {
        std::mem::take(&mut self.spends)
    }

    pub fn parse(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match value["event"].as_str() {
            Some("init") => {
                self.conversation = text(&value["conversation_id"]);
                if let Some(model) = Some(text(&value["init"]["model"])).filter(|m| !m.is_empty()) {
                    self.model = model;
                }
                vec![AgentEvent::Init {
                    session_id: self.conversation.clone(),
                    model: self.model.clone(),
                    // O `permission_mode` do `init` é a política de aprovação
                    // (`request-review`, `always-proceed`), não o modo do agente
                    // (`--mode`): não há o que sincronizar.
                    permission_mode: ReportedMode::Missing,
                }]
            }
            Some("step_update") => self.step(&value["step_update"]),
            Some("result") => self.result(&value["result"]),
            _ => Vec::new(),
        }
    }

    fn step(&mut self, step: &Value) -> Vec<AgentEvent> {
        let index = step["step_index"].as_u64().unwrap_or_default();
        let done = step["state"] == "DONE";
        let mut events = Vec::new();
        match step["step_type"].as_str().unwrap_or_default() {
            "agent_response" => {
                if let Some(delta) = step["text_delta"].as_str().filter(|d| !d.is_empty()) {
                    if self.text_step != Some(index) {
                        self.text_step = Some(index);
                        events.push(AgentEvent::TextStart);
                    }
                    self.wrote_text = true;
                    events.push(AgentEvent::TextDelta(delta.to_string()));
                }
                if let Some(usage) = step.get("usage").filter(|u| u.is_object()) {
                    let tokens = count(&usage["input_tokens"]) + count(&usage["output_tokens"]);
                    let cache = count(&usage["cache_read_tokens"]);
                    self.tokens += tokens;
                    self.cache_tokens += cache;
                    if done {
                        let conversation = step["conversation_id"]
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .unwrap_or(&self.conversation);
                        self.spends.push(Spend {
                            conversation: conversation.to_string(),
                            step: index,
                            model: self.model.clone(),
                            tokens,
                            cache,
                        });
                    }
                    events.push(AgentEvent::Context {
                        tokens,
                        window: context_window(&self.model),
                    });
                }
            }
            "tool" => {
                let info = &step["tool_info"];
                if self.tools.insert(index) {
                    let name = step["tool_name"]
                        .as_str()
                        .or_else(|| info["name"].as_str())
                        .unwrap_or("ferramenta");
                    events.push(AgentEvent::ToolUse {
                        name: name.to_string(),
                        detail: tool_detail(&info["parameters"]),
                    });
                }
                let state = step["state"].as_str().unwrap_or_default();
                if done || matches!(state, "ERROR" | "FAILED" | "CANCELED" | "CANCELLED") {
                    events.push(AgentEvent::ToolResult {
                        is_error: !done,
                        summary: summary(
                            info["output"]
                                .as_str()
                                .or_else(|| info["error"]["message"].as_str())
                                .unwrap_or_default(),
                        ),
                    });
                }
            }
            // O `agy` resumiu a conversa (o `/compact` ou a compactação
            // automática) e guardou o resumo num `checkpoint`.
            "checkpoint" | "compaction" if done => {
                events.push(AgentEvent::Compacted { tokens: None })
            }
            _ => {}
        }
        events
    }

    fn result(&mut self, result: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        let response = result["response"].as_str().unwrap_or_default();
        // Sem texto em trechos (ex.: a resposta de um comando da `/`), o
        // texto inteiro vem só aqui.
        if !self.wrote_text && !response.trim().is_empty() {
            events.push(AgentEvent::Text(response.trim_end().to_string()));
        }
        let is_error = result["status"] != "SUCCESS";
        let message = result["error"]
            .as_str()
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .or_else(|| {
                is_error.then(|| format!("o agy terminou com {}", text(&result["status"])))
            });
        events.push(AgentEvent::Done {
            is_error,
            message,
            tokens: self.tokens,
            cache_tokens: self.cache_tokens,
            denied: 0,
        });
        events
    }
}

/// O texto de um valor (string, número...), ou vazio.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn count(value: &Value) -> u64 {
    value.as_u64().unwrap_or_default()
}

/// O parâmetro principal de uma ferramenta: o comando, o caminho, a URL...
/// ou, sem nenhum conhecido, o primeiro texto.
fn tool_detail(parameters: &Value) -> String {
    const KEYS: [&str; 8] = [
        "CommandLine",
        "Path",
        "AbsolutePath",
        "TargetFile",
        "DirectoryPath",
        "Url",
        "Query",
        "SearchPath",
    ];
    let Some(map) = parameters.as_object() else {
        return String::new();
    };
    KEYS.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .or_else(|| map.values().find_map(Value::as_str))
        .map(summary)
        .unwrap_or_default()
}

/// A primeira linha com texto, com no máximo [`SUMMARY_MAX`] caracteres.
fn summary(output: &str) -> String {
    let line = output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= SUMMARY_MAX {
        return line.to_string();
    }
    let mut short: String = line.chars().take(SUMMARY_MAX - 1).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um turno real do `agy` 1.4.7: texto, ferramenta e resultado.
    const TURN: &[&str] = &[
        r#"{"event":"init","conversation_id":"c1","init":{"model":"gemini-3.8-flash-low","cwd":"/tmp","tools":["run_command"],"permission_mode":"request-review"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":0,"state":"DONE","step_type":"user_input"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":2,"state":"DONE","step_type":"agent_response","duration_seconds":3.3,"usage":{"input_tokens":7962,"output_tokens":38,"thinking_tokens":0,"cache_read_tokens":5,"total_tokens":8000}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls -la"}}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":3,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls -la"},"output":"Command exited with code 0.\n\nCommand output:\ntotal 36"}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":6,"state":"ACTIVE","step_type":"agent_response","text_delta":"ok"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c1","step_index":6,"state":"DONE","step_type":"agent_response","text_delta":"\n","usage":{"input_tokens":7840,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":7841}}}"#,
        r#"{"event":"result","result":{"conversation_id":"c1","status":"SUCCESS","response":"ok\n","num_turns":1,"usage":{"input_tokens":99999,"output_tokens":99,"cache_read_tokens":0,"total_tokens":100098}}}"#,
    ];

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = StreamParser::default();
        lines.iter().flat_map(|line| parser.parse(line)).collect()
    }

    #[test]
    fn a_turn_becomes_events_with_the_tokens_of_its_steps() {
        let events = parse_all(TURN);
        assert_eq!(
            events,
            vec![
                AgentEvent::Init {
                    session_id: "c1".into(),
                    model: "gemini-3.8-flash-low".into(),
                    permission_mode: ReportedMode::Missing,
                },
                AgentEvent::Context {
                    tokens: 8000,
                    window: Some(256_000)
                },
                AgentEvent::ToolUse {
                    name: "run_command".into(),
                    detail: "ls -la".into(),
                },
                AgentEvent::ToolResult {
                    is_error: false,
                    summary: "Command exited with code 0.".into(),
                },
                AgentEvent::TextStart,
                AgentEvent::TextDelta("ok".into()),
                AgentEvent::TextDelta("\n".into()),
                AgentEvent::Context {
                    tokens: 7841,
                    window: Some(256_000)
                },
                // O `usage` do `result` é o da conversa: fica de fora.
                AgentEvent::Done {
                    is_error: false,
                    message: None,
                    tokens: 15841,
                    cache_tokens: 5,
                    denied: 0,
                },
            ]
        );
    }

    #[test]
    fn each_finished_model_call_is_a_spend_for_the_daily_log() {
        let mut parser = StreamParser::default();
        for line in TURN {
            parser.parse(line);
        }
        let spend = |step, tokens, cache| Spend {
            conversation: "c1".into(),
            step,
            model: "gemini-3.8-flash-low".into(),
            tokens,
            cache,
        };
        assert_eq!(parser.take_spends(), [spend(2, 8000, 5), spend(6, 7841, 0)]);
        // Entregues uma vez só.
        assert!(parser.take_spends().is_empty());
    }

    #[test]
    fn a_step_in_progress_is_not_a_spend() {
        let mut parser = StreamParser::default();
        parser.parse(
            r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"agent_response","text_delta":"o","usage":{"input_tokens":5,"output_tokens":1}}}"#,
        );
        assert!(parser.take_spends().is_empty());
    }

    #[test]
    fn a_resumed_conversation_spends_with_the_requested_model() {
        let mut parser = StreamParser::new(Some("claude-sonnet-4-6"));
        parser.parse(r#"{"event":"init","conversation_id":"c7","init":{"cwd":"/tmp"}}"#);
        let events = parser.parse(
            r#"{"event":"step_update","step_update":{"step_index":4,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":100,"output_tokens":20,"cache_read_tokens":60}}}"#,
        );
        assert_eq!(
            events,
            [AgentEvent::Context {
                tokens: 120,
                window: Some(160_000)
            }]
        );
        let spends = parser.take_spends();
        assert_eq!(spends.len(), 1);
        assert_eq!(spends[0].conversation, "c7");
        assert_eq!(spends[0].model, "claude-sonnet-4-6");
        assert_eq!((spends[0].tokens, spends[0].cache), (120, 60));
    }

    #[test]
    fn the_context_window_comes_from_the_model_family() {
        assert_eq!(context_window("gemini-3.8-flash-low"), Some(256_000));
        assert_eq!(context_window("claude-opus-4-6-thinking"), Some(160_000));
        assert_eq!(context_window("gpt-oss-120b-medium"), Some(80_000));
        assert_eq!(context_window("outro-modelo"), None);
        assert_eq!(context_window(""), None);
    }

    #[test]
    fn a_response_without_deltas_comes_from_the_result() {
        let events = parse_all(&[
            r#"{"event":"result","result":{"conversation_id":"","status":"SUCCESS","response":"/help\tAjuda\n","num_turns":0}}"#,
        ]);
        assert_eq!(events[0], AgentEvent::Text("/help\tAjuda".into()));
        assert!(matches!(
            events[1],
            AgentEvent::Done {
                is_error: false,
                ..
            }
        ));
    }

    #[test]
    fn an_error_result_carries_the_message() {
        let events = parse_all(&[
            r#"{"event":"result","result":{"conversation_id":"","status":"ERROR","response":"","error":"stream input message is missing the \"event\" field"}}"#,
        ]);
        assert_eq!(
            events,
            vec![AgentEvent::Done {
                is_error: true,
                message: Some(r#"stream input message is missing the "event" field"#.into()),
                tokens: 0,
                cache_tokens: 0,
                denied: 0,
            }]
        );
    }

    #[test]
    fn a_checkpoint_step_compacts_the_context() {
        for step_type in ["checkpoint", "compaction"] {
            let line = format!(
                r#"{{"event":"step_update","step_update":{{"step_index":10,"state":"DONE","step_type":"{step_type}","duration_seconds":2.4}}}}"#
            );
            assert_eq!(
                parse_all(&[&line]),
                vec![AgentEvent::Compacted { tokens: None }]
            );
        }
    }

    #[test]
    fn a_failed_tool_reports_its_error() {
        let events = parse_all(&[
            r#"{"event":"step_update","step_update":{"step_index":7,"state":"ERROR","step_type":"tool","tool_name":"write_file","tool_info":{"name":"write_file","parameters":{"Path":"/etc/x"},"error":{"type":"TOOL_ERROR","message":"open /etc/x: permission denied"}}}}"#,
        ]);
        assert_eq!(
            events,
            vec![
                AgentEvent::ToolUse {
                    name: "write_file".into(),
                    detail: "/etc/x".into(),
                },
                AgentEvent::ToolResult {
                    is_error: true,
                    summary: "open /etc/x: permission denied".into(),
                },
            ]
        );
    }

    #[test]
    fn a_resumed_conversation_keeps_the_requested_model() {
        let line = r#"{"event":"init","conversation_id":"c1","init":{"cwd":"/tmp","permission_mode":"request-review"}}"#;
        let mut parser = StreamParser::new(Some("gemini-3.8-flash-low"));
        let AgentEvent::Init { model, .. } = parser.parse(line).remove(0) else {
            panic!("esperava Init");
        };
        assert_eq!(model, "gemini-3.8-flash-low");
    }

    #[test]
    fn long_details_are_shortened() {
        let long = "x".repeat(200);
        let detail = tool_detail(&serde_json::json!({ "Other": long }));
        assert_eq!(detail.chars().count(), SUMMARY_MAX);
        assert!(detail.ends_with('…'));
    }
}
