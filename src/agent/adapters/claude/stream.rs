//! Parser do `--output-format stream-json` do Claude CLI: cada linha (um
//! JSON) vira zero ou mais [`AgentEvent`]s.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::parse_mode;
use crate::agent::{
    AgentEvent, CHARS_PER_TOKEN, ModelSpend, ReportedMode, SpendScope, SubagentEvent,
};

/// Tamanho máximo do detalhe de uma ferramenta ou resultado na tela.
const DETAIL_MAX: usize = 80;

/// Os campos de um `usage`: primeiro os do consumo, depois os de cache.
const USAGE_KEYS: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
];

/// Converte as linhas do `--output-format stream-json` em eventos.
#[derive(Default)]
pub struct StreamParser {
    /// Mensagem em streaming no momento.
    current_message: Option<String>,
    /// A mensagem do agente principal em streaming: o `message_delta` não
    /// traz o id, e a de um subagente pode vir no meio.
    main_message: Option<String>,
    /// O uso de cada mensagem do agente principal nesta resposta, na ordem de
    /// [`USAGE_KEYS`]. A mesma mensagem informa o uso várias vezes.
    spent: HashMap<String, [u64; 4]>,
    /// Os caracteres que a mensagem principal já escreveu, e os tokens que ela
    /// já pensou: até o `message_delta`, a saída informada fica parada.
    written_chars: u64,
    thinking_tokens: u64,
    /// O último gasto informado com [`AgentEvent::Usage`].
    reported: (u64, u64),
    /// O último gasto informado com [`AgentEvent::Spent`], sem a estimativa.
    reported_spend: [u64; 4],
    /// Mensagens cujo texto já chegou em trechos; o texto inteiro que vem
    /// depois nelas é ignorado para não duplicar.
    streamed: HashSet<String>,
    /// Modelo do `init`, para achar o limite de contexto dele no `result`.
    model: String,
    /// A conversa foi compactada nesta resposta: o uso do `result` é o da
    /// compactação, não o do contexto que ficou.
    compacted: bool,
    /// As ferramentas chamadas nesta resposta (pelo id do `tool_use`), para
    /// saber a entrada de uma que o modo bloqueou.
    tools: HashMap<String, (String, Value)>,
    /// Lançou algum subagente nesta resposta.
    spawned: bool,
    /// Os subagentes rodando, pelo id da chamada que os lançou.
    subagents: HashSet<String>,
    /// Os que terminaram sem o agente principal ter visto: um em segundo
    /// plano acorda o principal, que continua a resposta.
    notified: HashSet<String>,
    /// Os `init` e os `result` do agente principal. Cada continuação tem os
    /// dois, e o `result` de uma pode chegar antes do da anterior.
    inits: u32,
    results: u32,
}

impl StreamParser {
    pub fn parse(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let usage = self.usage(&value);
        let events = match value["parent_tool_use_id"].as_str() {
            Some(id) => self.subagent_activity(id, &value),
            None => self.events(&value),
        };
        let spend = self.spend();
        usage.into_iter().chain(spend).chain(events).collect()
    }

    /// O [`AgentEvent::Spent`] do que o agente principal informou, se mudou.
    /// Vale até o `result`, que tem o gasto completo; sem ele (a resposta foi
    /// interrompida), é o que fica.
    fn spend(&mut self) -> Option<AgentEvent> {
        let spent = self.spent.values().fold([0; 4], |sum, spent| {
            std::array::from_fn(|i| sum[i] + spent[i])
        });
        if spent == self.reported_spend {
            return None;
        }
        self.reported_spend = spent;
        Some(AgentEvent::Spent {
            models: vec![ModelSpend {
                model: self.model.clone(),
                input: spent[0],
                output: spent[1],
                cache_creation: spent[2],
                cache_read: spent[3],
                ..ModelSpend::default()
            }],
            scope: SpendScope::Turn,
        })
    }

    /// A resposta ainda não acabou, mesmo com um `result`: há subagente
    /// rodando ou o agente principal ainda vai continuar. Sem subagente, o
    /// `result` é sempre o fim.
    pub fn subagents_pending(&self) -> bool {
        self.spawned
            && (self.subagents_running() || !self.notified.is_empty() || self.results < self.inits)
    }

    /// Há subagente rodando, que ainda vai mandar o fim dele.
    pub fn subagents_running(&self) -> bool {
        !self.subagents.is_empty()
    }

    /// O que uma linha de subagente mostra na aba dele: as ferramentas e o
    /// texto, que chega inteiro. O contexto e o uso não são do chat.
    fn subagent_activity(&mut self, id: &str, value: &Value) -> Vec<AgentEvent> {
        let events = match value["type"].as_str() {
            Some("assistant") => self.assistant(&value["message"]),
            Some("user") => tool_results(&value["message"]),
            _ => Vec::new(),
        };
        events
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentEvent::ToolUse { .. }
                        | AgentEvent::ToolResult { .. }
                        | AgentEvent::Text(_)
                )
            })
            .map(|event| {
                AgentEvent::Subagent(SubagentEvent::Activity {
                    id: id.to_string(),
                    event: Box::new(event),
                })
            })
            .collect()
    }

    /// Um subagente começou. As outras tarefas em segundo plano (um `Bash`,
    /// um servidor) ficam de fora: elas não seguram a resposta.
    fn subagent_started(&mut self, value: &Value) -> Option<AgentEvent> {
        if value["task_type"] != "local_agent" {
            return None;
        }
        let id = value["tool_use_id"].as_str()?.to_string();
        self.spawned = true;
        self.subagents.insert(id.clone());
        Some(AgentEvent::Subagent(SubagentEvent::Started {
            id,
            description: text(&value["description"]),
        }))
    }

    /// Um subagente terminou (`completed`, `failed`, `stopped`...).
    fn subagent_finished(&mut self, value: &Value) -> Option<AgentEvent> {
        let id = value["tool_use_id"].as_str()?.to_string();
        if !self.subagents.remove(&id) {
            return None;
        }
        self.notified.insert(id.clone());
        let status = text(&value["status"]);
        let error = (status != "completed").then(|| format!("O subagente terminou: {status}."));
        Some(AgentEvent::Subagent(SubagentEvent::Finished { id, error }))
    }

    /// O gasto da resposta até aqui, quando esta linha o muda. Só o agente
    /// principal: é o que o `result` soma com certeza.
    fn usage(&mut self, value: &Value) -> Option<AgentEvent> {
        if !value["parent_tool_use_id"].is_null() {
            return None;
        }
        let event = &value["event"];
        let (id, usage) = match (value["type"].as_str(), event["type"].as_str()) {
            (Some("stream_event"), Some("message_start")) => {
                self.main_message = event["message"]["id"].as_str().map(str::to_string);
                self.written_chars = 0;
                self.thinking_tokens = 0;
                (self.main_message.clone()?, &event["message"]["usage"])
            }
            (Some("stream_event"), Some("content_block_delta")) => {
                self.count_written(&event["delta"]);
                return self.report_usage();
            }
            (Some("stream_event"), Some("message_delta")) => {
                self.written_chars = 0;
                self.thinking_tokens = 0;
                (self.main_message.clone()?, &event["usage"])
            }
            (Some("assistant"), _) => (
                value["message"]["id"].as_str()?.to_string(),
                &value["message"]["usage"],
            ),
            _ => return None,
        };
        let spent = self.spent.entry(id).or_default();
        // Campo a campo: o `message_delta` pode trazer só a saída, e uma linha
        // atrasada não diminui o que a mensagem já informou.
        let updated = USAGE_KEYS.map(|key| usage[key].as_u64().unwrap_or(0));
        *spent = std::array::from_fn(|i| spent[i].max(updated[i]));
        self.report_usage()
    }

    /// Soma um trecho da mensagem em andamento: o texto, a entrada de uma
    /// ferramenta ou o pensamento, que vem sem o texto, só com a estimativa.
    fn count_written(&mut self, delta: &Value) {
        let chars = ["text", "partial_json"]
            .iter()
            .filter_map(|key| delta[*key].as_str())
            .map(|text| text.chars().count() as u64)
            .sum::<u64>();
        self.written_chars += chars;
        self.thinking_tokens += delta["estimated_tokens"].as_u64().unwrap_or(0);
    }

    /// O [`AgentEvent::Usage`] do gasto até aqui, se ele mudou.
    fn report_usage(&mut self) -> Option<AgentEvent> {
        let (tokens, cache_tokens) = self.spent.values().fold((0, 0), |sum, spent| {
            (sum.0 + spent[0] + spent[1], sum.1 + spent[2] + spent[3])
        });
        let informed = self
            .main_message
            .as_ref()
            .and_then(|id| self.spent.get(id))
            .map_or(0, |spent| spent[1]);
        let estimate = self.thinking_tokens + self.written_chars / CHARS_PER_TOKEN;
        let reported = (tokens + estimate.saturating_sub(informed), cache_tokens);
        if reported == self.reported {
            return None;
        }
        self.reported = reported;
        Some(AgentEvent::Usage {
            tokens: reported.0,
            cache_tokens: reported.1,
        })
    }

    fn events(&mut self, value: &Value) -> Vec<AgentEvent> {
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => {
                self.model = text(&value["model"]);
                self.inits += 1;
                self.notified.clear();
                vec![AgentEvent::Init {
                    session_id: text(&value["session_id"]),
                    model: self.model.clone(),
                    permission_mode: reported_mode(&value["permissionMode"]),
                }]
            }
            Some("system") if value["subtype"] == "compact_boundary" => {
                self.compacted = true;
                let tokens = value["compact_metadata"]["post_tokens"]
                    .as_u64()
                    .filter(|&t| t > 0);
                vec![AgentEvent::Compacted { tokens }]
            }
            Some("system") if value["subtype"] == "permission_denied" => {
                self.blocked(value).into_iter().collect()
            }
            Some("system") if value["subtype"] == "task_started" => {
                self.subagent_started(value).into_iter().collect()
            }
            Some("system") if value["subtype"] == "task_notification" => {
                self.subagent_finished(value).into_iter().collect()
            }
            Some("stream_event") => self.stream_event(&value["event"]),
            Some("assistant") => self.assistant(&value["message"]),
            Some("user") => {
                // O subagente em primeiro plano devolve o resultado aqui e
                // não acorda o principal, que já está esperando.
                for block in blocks(&value["message"]["content"]) {
                    if let Some(id) = block["tool_use_id"].as_str() {
                        self.notified.remove(id);
                    }
                }
                tool_results(&value["message"])
            }
            Some("result") => self.result(value),
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
        let context = context_tokens(&message["usage"]).map(|tokens| AgentEvent::Context {
            tokens,
            window: None,
        });
        context
            .into_iter()
            .chain(
                blocks(&message["content"]).filter_map(|block| match block["type"].as_str() {
                    Some("text") if !streamed => Some(AgentEvent::Text(text(&block["text"]))),
                    Some("tool_use") => {
                        let name = text(&block["name"]);
                        self.tools
                            .insert(text(&block["id"]), (name.clone(), block["input"].clone()));
                        Some(AgentEvent::ToolUse {
                            name,
                            detail: tool_detail(&block["input"]),
                        })
                    }
                    _ => None,
                }),
            )
            .collect()
    }

    /// Uma ferramenta que o classificador do modo `auto` bloqueou sem
    /// perguntar. As negadas por uma regra do usuário (`deny`) ficam
    /// negadas: não viram pedido.
    fn blocked(&self, value: &Value) -> Option<AgentEvent> {
        if value["decision_reason_type"] != "classifier" {
            return None;
        }
        let (name, input) = self.tools.get(value["tool_use_id"].as_str()?)?;
        Some(AgentEvent::Blocked {
            name: name.clone(),
            detail: tool_detail(input),
            reason: first_line(&text(&value["decision_reason"])),
            allow: allow_rule(name, input),
        })
    }

    /// Fim da resposta: o contexto da última chamada, com o limite do
    /// modelo, e o [`AgentEvent::Done`].
    fn result(&mut self, value: &Value) -> Vec<AgentEvent> {
        self.results += 1;
        let usage = &value["usage"];
        // A última chamada ao modelo é o contexto que fica; sem as chamadas,
        // o uso da resposta.
        let last = usage["iterations"]
            .as_array()
            .and_then(|calls| calls.last())
            .unwrap_or(usage);
        let context = context_tokens(last)
            .filter(|_| !self.compacted)
            .map(|tokens| AgentEvent::Context {
                tokens,
                window: context_window(&value["modelUsage"], &self.model),
            });
        let spent = model_spend(&value["modelUsage"]);
        let spent = (!spent.is_empty()).then_some(AgentEvent::Spent {
            models: spent,
            scope: SpendScope::Session,
        });
        spent
            .into_iter()
            .chain(context)
            .chain([result(value)])
            .collect()
    }
}

/// O gasto por modelo do `modelUsage` do `result`. Diferente do `usage`, ele
/// tem os subagentes e o custo, e acumula a sessão inteira: as partes de uma
/// resposta e o que veio antes de um `--resume`.
fn model_spend(models: &Value) -> Vec<ModelSpend> {
    let count = |usage: &Value, key: &str| usage[key].as_u64().unwrap_or(0);
    models
        .as_object()
        .into_iter()
        .flatten()
        .map(|(model, usage)| ModelSpend {
            model: model.clone(),
            input: count(usage, "inputTokens"),
            output: count(usage, "outputTokens"),
            cache_creation: count(usage, "cacheCreationInputTokens"),
            cache_read: count(usage, "cacheReadInputTokens"),
            thinking: count(usage, "thinkingTokens"),
            web_searches: count(usage, "webSearchRequests"),
            cost_micros: usage["costUSD"]
                .as_f64()
                .map(|usd| (usd * 1_000_000.0).round() as u64),
        })
        .filter(|spend| spend.input + spend.output + spend.cache() > 0)
        .collect()
}

/// Tamanho do contexto numa chamada: entrada (com o cache) e saída. Com o
/// cache, quase toda a conversa vem em `cache_read_input_tokens`; sem ele, a
/// barra mostraria só o trecho novo de cada mensagem. `None` quando a chamada
/// não trouxe o uso.
fn context_tokens(usage: &Value) -> Option<u64> {
    const KEYS: [&str; 4] = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ];
    let tokens: u64 = KEYS.iter().filter_map(|key| usage[key].as_u64()).sum();
    (tokens > 0).then_some(tokens)
}

/// O limite de contexto do modelo da sessão, do `modelUsage` do `result`;
/// sem ele, o maior dos modelos usados.
fn context_window(models: &Value, model: &str) -> Option<u64> {
    let window = |usage: &Value| usage["contextWindow"].as_u64();
    window(&models[model]).or_else(|| models.as_object()?.values().filter_map(window).max())
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
    let sum = |keys: &[&str]| keys.iter().filter_map(|key| usage[key].as_u64()).sum();
    let (tokens, cache_tokens) = (sum(&USAGE_KEYS[..2]), sum(&USAGE_KEYS[2..]));
    AgentEvent::Done {
        is_error,
        message,
        tokens,
        cache_tokens,
        denied: value["permission_denials"].as_array().map_or(0, Vec::len),
    }
}

/// O dado mais útil da chamada: o comando, o arquivo, o padrão...
pub fn tool_detail(input: &Value) -> String {
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

/// A regra do `--allowedTools` que libera só esta chamada: o comando exato
/// no `Bash`, o arquivo nas ferramentas de arquivo (`//` é o caminho
/// absoluto) e o domínio no `WebFetch`; nas outras, a ferramenta.
fn allow_rule(name: &str, input: &Value) -> String {
    let specifier = match name {
        "Bash" => input["command"].as_str().map(str::to_string),
        "WebFetch" => input["url"]
            .as_str()
            .and_then(|url| url.split("://").nth(1))
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .map(|host| format!("domain:{host}")),
        _ => input["file_path"]
            .as_str()
            .or(input["notebook_path"].as_str())
            .filter(|path| path.starts_with('/'))
            .map(|path| format!("/{path}")),
    };
    match specifier {
        Some(specifier) => format!("{name}({specifier})"),
        None => name.to_string(),
    }
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

    /// As linhas de um subagente em segundo plano, como o `claude` 2.1.311
    /// manda: o primeiro `result` vem com ele rodando.
    const BACKGROUND: [&str; 9] = [
        r#"{"type":"system","subtype":"init","session_id":"s","model":"m","permissionMode":"default"}"#,
        r#"{"type":"assistant","parent_tool_use_id":null,"message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Rodar"}}]}}"#,
        r#"{"type":"system","subtype":"task_started","task_id":"a","tool_use_id":"t1","description":"Rodar","task_type":"local_agent"}"#,
        r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Async agent launched"}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":1,"output_tokens":2}}"#,
        r#"{"type":"assistant","parent_tool_use_id":"t1","message":{"id":"s1","usage":{"input_tokens":999},"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"echo oi"}},{"type":"text","text":"oi"}]}}"#,
        r#"{"type":"system","subtype":"task_notification","task_id":"a","tool_use_id":"t1","status":"completed","summary":"oi"}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","model":"m","permissionMode":"default"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":4}}"#,
    ];

    #[test]
    fn a_background_subagent_holds_the_turn_until_the_continuation_ends() {
        let mut parser = StreamParser::default();
        let pending: Vec<bool> = BACKGROUND
            .iter()
            .map(|line| {
                parser.parse(line);
                parser.subagents_pending()
            })
            .collect();
        assert_eq!(
            pending,
            [false, false, true, true, true, true, true, true, false]
        );
    }

    #[test]
    fn the_first_result_may_come_after_the_continuation_started() {
        let mut parser = StreamParser::default();
        for line in [
            BACKGROUND[0],
            BACKGROUND[2],
            BACKGROUND[6],
            BACKGROUND[7],
            BACKGROUND[8],
        ] {
            parser.parse(line);
        }
        assert!(parser.subagents_pending(), "falta o segundo result");
        parser.parse(BACKGROUND[8]);
        assert!(!parser.subagents_pending());
    }

    #[test]
    fn a_foreground_subagent_needs_no_continuation() {
        let mut parser = StreamParser::default();
        for line in [
            BACKGROUND[0],
            BACKGROUND[2],
            BACKGROUND[6],
            r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"oi"}]}}"#,
            BACKGROUND[8],
        ] {
            parser.parse(line);
        }
        assert!(!parser.subagents_pending());
    }

    #[test]
    fn other_background_tasks_do_not_hold_the_turn() {
        let mut parser = StreamParser::default();
        let events = [
            BACKGROUND[0],
            r#"{"type":"system","subtype":"task_started","task_id":"b","tool_use_id":"b1","description":"servidor","task_type":"local_bash"}"#,
            BACKGROUND[8],
            r#"{"type":"system","subtype":"task_notification","task_id":"b","tool_use_id":"b1","status":"stopped"}"#,
        ]
        .iter()
        .flat_map(|line| {
            let events = parser.parse(line);
            assert!(!parser.subagents_pending());
            events
        })
        .collect::<Vec<_>>();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::Subagent(_)))
        );
    }

    #[test]
    fn subagent_lines_go_to_its_tab_and_not_to_the_main_chat() {
        let events = parse_all(&BACKGROUND);
        let subagent: Vec<&SubagentEvent> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Subagent(event) => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(
            subagent[0],
            &SubagentEvent::Started {
                id: "t1".into(),
                description: "Rodar".into()
            }
        );
        assert!(matches!(
            subagent[1],
            SubagentEvent::Activity { id, event }
                if id == "t1" && matches!(**event, AgentEvent::ToolUse { .. })
        ));
        assert!(matches!(
            subagent[2],
            SubagentEvent::Activity { event, .. } if **event == AgentEvent::Text("oi".into())
        ));
        assert_eq!(
            subagent[3],
            &SubagentEvent::Finished {
                id: "t1".into(),
                error: None
            }
        );
        // No chat principal, só a chamada do `Agent`, sem o `Bash` nem o
        // contexto do subagente.
        let tools: Vec<&AgentEvent> = events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ToolUse { .. }))
            .collect();
        assert_eq!(tools.len(), 1);
        assert!(!events.contains(&AgentEvent::Context {
            tokens: 999,
            window: None
        }));
    }

    #[test]
    fn a_subagent_that_stops_reports_why() {
        let mut parser = StreamParser::default();
        parser.parse(BACKGROUND[2]);
        let events = parser.parse(
            r#"{"type":"system","subtype":"task_notification","tool_use_id":"t1","status":"failed"}"#,
        );
        assert_eq!(
            events,
            [AgentEvent::Subagent(SubagentEvent::Finished {
                id: "t1".into(),
                error: Some("O subagente terminou: failed.".into())
            })]
        );
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
                AgentEvent::Context {
                    tokens: 21,
                    window: None
                },
                AgentEvent::Done {
                    is_error: false,
                    message: None,
                    tokens: 6,
                    cache_tokens: 15,
                    denied: 0
                },
            ]
        );
    }

    #[test]
    fn usage_is_reported_while_the_answer_runs() {
        let events = parse_all(&[
            // A primeira chamada: a entrada no começo, a saída no fim.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":"m1","usage":{"input_tokens":10,"cache_read_input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"id":"m1","usage":{"input_tokens":10,"cache_read_input_tokens":100,"output_tokens":1},"content":[]}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_delta","usage":{"output_tokens":20}}}"#,
            // Um `assistant` atrasado não diminui a saída.
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"id":"m1","usage":{"input_tokens":10,"cache_read_input_tokens":100,"output_tokens":1},"content":[]}}"#,
            // O subagente fica de fora, e o delta seguinte é do principal.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":"m2","usage":{"input_tokens":5,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":"t1","event":{"type":"message_start","message":{"id":"s1","usage":{"input_tokens":999,"output_tokens":9}}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":"t1","event":{"type":"message_delta","usage":{"output_tokens":99}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_delta","usage":{"output_tokens":3}}}"#,
        ]);
        let usage: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Usage {
                    tokens,
                    cache_tokens,
                } => Some((*tokens, *cache_tokens)),
                _ => None,
            })
            .collect();
        assert_eq!(usage, [(11, 100), (30, 100), (36, 100), (38, 100)]);
    }

    #[test]
    fn usage_grows_while_the_message_is_generated() {
        let events = parse_all(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":"m1","usage":{"input_tokens":2,"cache_read_input_tokens":100,"output_tokens":2}}}}"#,
            // O pensamento vem sem texto, com a estimativa do trecho.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"","estimated_tokens":50}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"","estimated_tokens":null}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"abcdefghijklmnopqrstuvwxyz"}}}"#,
            // O texto e a entrada de uma ferramenta contam pelos caracteres.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"abcdefgh"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"abcd"}}}"#,
            // O subagente fica de fora.
            r#"{"type":"stream_event","parent_tool_use_id":"t1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"abcdefghijklmnopqrstuvwxyz"}}}"#,
            // O `assistant` no meio da mensagem não desfaz a estimativa.
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"id":"m1","usage":{"input_tokens":2,"cache_read_input_tokens":100,"output_tokens":2},"content":[]}}"#,
            // O `message_delta` troca a estimativa pelo uso de verdade.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_delta","usage":{"output_tokens":40}}}"#,
            // A chamada seguinte estima do zero, em cima da anterior.
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":"m2","usage":{"input_tokens":1,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"abcdefgh"}}}"#,
        ]);
        let usage: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Usage { tokens, .. } => Some(*tokens),
                _ => None,
            })
            .collect();
        // 2 + 2; 2 + 50; + 2 do texto; + 1 da ferramenta; 2 + 40; + 2; + 1.
        assert_eq!(usage, [4, 52, 54, 55, 42, 44, 45]);
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
                cache_tokens: 0,
                denied: 1
            }]
        );
    }

    /// O `result` de verdade de uma resposta com subagente em outro modelo.
    #[test]
    fn the_result_reports_the_spend_of_every_model_with_its_cost() {
        let mut parser = StreamParser::default();
        let events = parser.parse(
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":4,"output_tokens":4},"modelUsage":{"claude-haiku-5-5":{"inputTokens":8,"outputTokens":321,"cacheReadInputTokens":53397,"cacheCreationInputTokens":14019,"thinkingTokens":9,"webSearchRequests":1,"costUSD":0.00349907,"contextWindow":1000000},"claude-sonnet-5-5":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":0,"cacheCreationInputTokens":15158,"costUSD":0.037939}}}"#,
        );
        let AgentEvent::Spent { models, scope } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(*scope, SpendScope::Session);
        assert_eq!(
            models[0],
            ModelSpend {
                model: "claude-haiku-5-5".into(),
                input: 8,
                output: 321,
                cache_creation: 14019,
                cache_read: 53397,
                thinking: 9,
                web_searches: 1,
                cost_micros: Some(3499),
            }
        );
        assert_eq!(models[1].cost_micros, Some(37939));
    }

    #[test]
    fn the_spend_of_an_answer_is_reported_only_when_it_changes() {
        let mut parser = StreamParser::default();
        let line = r#"{"type":"assistant","message":{"id":"m1","content":[],"usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":900,"output_tokens":4}}}"#;
        let spent = |events: Vec<AgentEvent>| {
            events
                .into_iter()
                .filter(|e| matches!(e, AgentEvent::Spent { .. }))
                .count()
        };
        assert_eq!(spent(parser.parse(line)), 1);
        assert_eq!(spent(parser.parse(line)), 0, "a mesma mensagem de novo");
    }

    #[test]
    fn parses_context_of_the_last_call_and_the_model_window() {
        let events = parse_all(&[
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-5-5","permissionMode":"auto"}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[],"usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":900,"output_tokens":4}}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":5,"output_tokens":90,"iterations":[{"input_tokens":2,"cache_read_input_tokens":900,"cache_creation_input_tokens":100,"output_tokens":10},{"input_tokens":3,"cache_read_input_tokens":1100,"cache_creation_input_tokens":0,"output_tokens":80}]},"modelUsage":{"claude-haiku-5-5":{"contextWindow":200000},"claude-opus-5-5":{"contextWindow":1000000}}}"#,
        ]);
        assert_eq!(
            events[1],
            AgentEvent::Usage {
                tokens: 6,
                cache_tokens: 1000
            }
        );
        assert_eq!(
            events[3],
            AgentEvent::Context {
                tokens: 1006,
                window: None
            }
        );
        assert_eq!(
            events[4],
            AgentEvent::Context {
                tokens: 1183,
                window: Some(1_000_000)
            }
        );
        assert!(matches!(events[5], AgentEvent::Done { .. }));
    }

    #[test]
    fn compaction_ignores_the_usage_of_the_summary() {
        let events = parse_all(&[
            r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":150000,"post_tokens":8048}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":150000,"output_tokens":900}}"#,
        ]);
        assert_eq!(events[0], AgentEvent::Compacted { tokens: Some(8048) });
        assert!(matches!(events[1..], [AgentEvent::Done { .. }]));

        // Sem o tamanho depois da compactação, ele fica para a próxima chamada.
        let events = parse_all(&[
            r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":150000}}"#,
        ]);
        assert_eq!(events[0], AgentEvent::Compacted { tokens: None });
    }

    #[test]
    fn classifier_denials_become_blocked_with_an_allow_rule() {
        let mut parser = StreamParser::default();
        parser.parse(
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git push --force"}}]}}"#,
        );
        let events = parser.parse(
            r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"t1","decision_reason_type":"classifier","decision_reason":"perigoso\ndetalhes"}"#,
        );
        assert_eq!(
            events,
            [AgentEvent::Blocked {
                name: "Bash".into(),
                detail: "git push --force".into(),
                reason: "perigoso".into(),
                allow: "Bash(git push --force)".into(),
            }]
        );
        // Negada por uma regra do usuário: fica negada.
        let rule = r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"t1","decision_reason_type":"rule"}"#;
        assert_eq!(parser.parse(rule), []);
    }

    #[test]
    fn allow_rules_name_only_the_blocked_call() {
        let rule =
            |name: &str, input: &str| allow_rule(name, &serde_json::from_str(input).unwrap());
        assert_eq!(rule("Bash", r#"{"command":"rm -rf x"}"#), "Bash(rm -rf x)");
        assert_eq!(
            rule("Edit", r#"{"file_path":"/home/a/x.rs"}"#),
            "Edit(//home/a/x.rs)"
        );
        assert_eq!(
            rule("WebFetch", r#"{"url":"https://docs.rs/serde?x=1"}"#),
            "WebFetch(domain:docs.rs)"
        );
        assert_eq!(rule("Task", r#"{"prompt":"x"}"#), "Task");
    }

    #[test]
    fn first_line_skips_blank_and_truncates() {
        assert_eq!(first_line("\n  \n abc \nxyz"), "abc");
        let long = "x".repeat(100);
        assert_eq!(first_line(&long).chars().count(), DETAIL_MAX);
    }
}
