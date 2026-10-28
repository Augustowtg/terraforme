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
    /// Tamanho do contexto da sessão na última chamada ao modelo (entrada e
    /// saída, sem o cache) e, quando o provedor informa, o limite do modelo.
    Context { tokens: u64, window: Option<u64> },
    /// O agente compactou a conversa: o contexto volta a ser pequeno. Com o
    /// tamanho dele depois da compactação, quando o provedor informa.
    Compacted { tokens: Option<u64> },
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
