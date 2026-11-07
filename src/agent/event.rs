//! Eventos de uma resposta do agente: o contrato entre quem fala com o agente
//! e a interface.

use super::PermissionMode;

/// Modo em que o agente disse estar, já traduzido pelo adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportedMode {
    Known(PermissionMode),
    /// Um modo do provedor sem equivalente no terra (ex.: `dontAsk`).
    Unknown(String),
    /// O agente não informou.
    Missing,
}

/// Quantos caracteres fazem um token, na conta aproximada do que o modelo está
/// escrevendo: o uso de verdade só vem no fim de cada chamada.
pub const CHARS_PER_TOKEN: u64 = 4;

/// O que um modelo gastou. O custo vem em milionésimos de dólar e é o que o
/// provedor informa (`None` quando ele não informa).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSpend {
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    /// O pensamento do modelo, que já está em `output`.
    pub thinking: u64,
    pub web_searches: u64,
    pub cost_micros: Option<u64>,
}

impl ModelSpend {
    pub fn cache(&self) -> u64 {
        self.cache_creation + self.cache_read
    }

    /// Soma `other`, menos o nome do modelo.
    pub fn add(&mut self, other: &ModelSpend) {
        self.input += other.input;
        self.output += other.output;
        self.cache_creation += other.cache_creation;
        self.cache_read += other.cache_read;
        self.thinking += other.thinking;
        self.web_searches += other.web_searches;
        self.cost_micros = match (self.cost_micros, other.cost_micros) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
    }

    fn since(&self, base: &ModelSpend) -> ModelSpend {
        ModelSpend {
            model: self.model.clone(),
            input: self.input.saturating_sub(base.input),
            output: self.output.saturating_sub(base.output),
            cache_creation: self.cache_creation.saturating_sub(base.cache_creation),
            cache_read: self.cache_read.saturating_sub(base.cache_read),
            thinking: self.thinking.saturating_sub(base.thinking),
            web_searches: self.web_searches.saturating_sub(base.web_searches),
            cost_micros: self
                .cost_micros
                .map(|cost| cost.saturating_sub(base.cost_micros.unwrap_or(0))),
        }
    }

    fn is_empty(&self) -> bool {
        self.input + self.output + self.cache() + self.web_searches == 0
            && self.cost_micros.unwrap_or(0) == 0
    }
}

/// Soma `spend` em `into`, modelo a modelo.
pub fn merge_spend(into: &mut Vec<ModelSpend>, spend: &[ModelSpend]) {
    for item in spend {
        match into.iter_mut().find(|known| known.model == item.model) {
            Some(known) => known.add(item),
            None => into.push(item.clone()),
        }
    }
}

/// O que `now`, um acumulado, gastou depois de `base`.
pub fn spend_since(now: &[ModelSpend], base: &[ModelSpend]) -> Vec<ModelSpend> {
    now.iter()
        .map(|item| match base.iter().find(|b| b.model == item.model) {
            Some(known) => item.since(known),
            None => item.clone(),
        })
        .filter(|item| !item.is_empty())
        .collect()
}

/// A soma de todos os modelos.
pub fn total_spend(spend: &[ModelSpend]) -> ModelSpend {
    let mut total = ModelSpend::default();
    for item in spend {
        total.add(item);
    }
    total
}

/// A que período um [`AgentEvent::Spent`] se refere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendScope {
    /// Só esta resposta, até aqui.
    Turn,
    /// A sessão do agente inteira, mesmo com o que veio antes de um `--resume`:
    /// o chat tira o que já contou.
    Session,
}

/// Uma pergunta do agente ao usuário.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// O rótulo curto da pergunta (ex.: `Cor`).
    pub header: String,
    pub text: String,
    /// O usuário pode marcar mais de uma opção.
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

/// O que acontece com um subagente que o agente lançou. O `id` é o da chamada
/// da ferramenta que o lançou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentEvent {
    /// O subagente começou.
    Started { id: String, description: String },
    /// O subagente usou uma ferramenta, recebeu o resultado ou escreveu.
    Activity { id: String, event: Box<AgentEvent> },
    /// O subagente terminou; `error` diz o motivo quando não deu certo.
    Finished { id: String, error: Option<String> },
    /// O agente principal parou, mas a resposta segue: falta subagente.
    Waiting,
}

/// Uma opção de resposta de uma [`Question`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// Sessão iniciada: id, modelo (ex.: `claude-opus-5-5`) e modo de
    /// permissão em que o agente está de fato.
    Init {
        session_id: String,
        model: String,
        permission_mode: ReportedMode,
    },
    /// Começou um bloco de texto da resposta.
    TextStart,
    /// Trecho de texto da resposta, chegando aos poucos.
    TextDelta(String),
    /// Bloco de texto inteiro, quando o CLI não mandou os trechos.
    Text(String),
    /// O agente chamou uma ferramenta (ex.: `Bash` com `ls -la`).
    ToolUse { name: String, detail: String },
    /// O agente pede permissão para usar uma ferramenta: o chat pergunta ao
    /// usuário e responde com [`Session::respond`](super::Session::respond)
    /// e o `id`.
    PermissionRequest {
        id: String,
        name: String,
        detail: String,
    },
    /// O agente faz perguntas ao usuário e espera: o chat mostra as opções e
    /// responde com [`Session::answer`](super::Session::answer) e o `id`.
    Question {
        id: String,
        questions: Vec<Question>,
    },
    /// O modo do agente bloqueou uma ferramenta sem perguntar (ex.: o
    /// classificador do modo Agente). O agente segue sem ela; se o usuário
    /// permitir, `allow` vai em
    /// [`Request::allowed_tools`](super::Request::allowed_tools) numa
    /// mensagem nova, que libera a ferramenta.
    Blocked {
        name: String,
        detail: String,
        reason: String,
        allow: String,
    },
    /// Resultado de uma ferramenta (primeira linha).
    ToolResult { is_error: bool, summary: String },
    /// Tamanho do contexto da sessão na última chamada ao modelo (entrada,
    /// cache e saída) e, quando o provedor informa, o limite do modelo.
    Context { tokens: u64, window: Option<u64> },
    /// O agente compactou a conversa: o contexto volta a ser pequeno. Com o
    /// tamanho dele depois da compactação, quando o provedor informa.
    Compacted { tokens: Option<u64> },
    /// O que este turno gastou até aqui, enquanto o provedor trabalha, com uma
    /// estimativa da chamada em andamento. O [`AgentEvent::Done`] traz o total
    /// que vale.
    Usage { tokens: u64, cache_tokens: u64 },
    /// O gasto separado por tipo de token e por modelo; cada um substitui o
    /// anterior do mesmo turno.
    Spent {
        models: Vec<ModelSpend>,
        scope: SpendScope,
    },
    /// Um subagente começou, trabalhou ou terminou.
    Subagent(SubagentEvent),
    /// Fim da resposta.
    Done {
        is_error: bool,
        message: Option<String>,
        tokens: u64,
        /// Os tokens de cache (criação e leitura), fora de `tokens`.
        cache_tokens: u64,
        /// Ferramentas negadas por falta de permissão.
        denied: usize,
    },
    /// O processo falhou sem chegar ao fim da resposta.
    Failed(String),
}
