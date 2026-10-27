//! O contrato entre o chat e um provedor de agente (padrão Adapter).
//!
//! Cada provedor (route) — Claude CLI, uma API, outro CLI... — implementa
//! [`ChatAdapter`] traduzindo o seu protocolo para [`Request`] na ida e
//! [`AgentEvent`]s na volta. O chat só conhece este contrato, então um
//! provedor novo entra sem mudar a interface: basta implementar o trait e
//! registrá-lo em [`super::adapters::all`].

use std::path::Path;

use super::{AgentEvent, PermissionMode};

/// Uma mensagem para o agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub prompt: &'a str,
    pub mode: PermissionMode,
    /// Sessão a continuar (o `session_id` do último [`AgentEvent::Init`]).
    pub session_id: Option<&'a str>,
}

/// Um provedor de agente para o chat.
pub trait ChatAdapter {
    /// Nome no botão do título do input (ex.: "Claude").
    fn name(&self) -> &str;

    /// `false` quando o provedor não está instalado ou configurado; o chat
    /// fica Desconectado.
    fn is_available(&self) -> bool;

    /// O que mostrar quando o provedor não está disponível.
    fn unavailable_hint(&self) -> String {
        format!("{} não está disponível.", self.name())
    }

    /// Modo padrão configurado no provedor para o diretório `cwd`, para o
    /// chat começar em sincronia com ele.
    fn configured_mode(&self, _cwd: &Path) -> Option<PermissionMode> {
        None
    }

    /// Nome do modelo para a tela (ex.: `claude-opus-5-5` → `Opus - 5.5`).
    fn model_label(&self, id: &str) -> String {
        id.to_string()
    }

    /// Começa uma resposta. No [`PermissionMode::Yolo`], o adapter deve
    /// aprovar tudo o que o agente pedir.
    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String>;
}

/// Uma resposta em andamento. Soltar (drop) interrompe a resposta.
pub trait Session {
    /// Eventos que chegaram desde a última chamada, sem esperar. A resposta
    /// termina com [`AgentEvent::Done`] ou [`AgentEvent::Failed`].
    fn poll(&mut self) -> Vec<AgentEvent>;
}
