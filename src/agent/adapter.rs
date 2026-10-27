//! O contrato entre o chat e um provedor de agente (padrão Adapter).
//!
//! Cada provedor (route) — Claude CLI, uma API, outro CLI... — implementa
//! [`ChatAdapter`] traduzindo o seu protocolo para [`Request`] na ida e
//! [`AgentEvent`]s na volta. O chat só conhece este contrato, então um
//! provedor novo entra sem mudar a interface: basta implementar o trait e
//! registrá-lo em [`super::adapters::all`].

use std::path::Path;
use std::sync::mpsc::Receiver;

use super::{AgentEvent, PermissionMode};

/// Uma mensagem para o agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub prompt: &'a str,
    pub mode: PermissionMode,
    /// Sessão a continuar (o `session_id` do último [`AgentEvent::Init`]).
    pub session_id: Option<&'a str>,
    /// Modelo escolhido na Config (um [`ModelOption::id`]); `None` usa o
    /// padrão do provedor.
    pub model: Option<&'a str>,
    /// Conta escolhida na Config (um [`AccountOption::id`]); `None` usa a
    /// conta padrão do provedor.
    pub account: Option<&'a str>,
}

/// Uma conta do provedor que pode ser escolhida na Config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountOption {
    /// O que fica salvo e vai no [`Request`]; `None` é a conta padrão.
    pub id: Option<String>,
    /// O que aparece na tela (ex.: `Trabalho · eu@empresa.com (Empresa)`).
    pub label: String,
}

/// Um modelo que o provedor oferece na Config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// O que vai para o provedor (ex.: `opus`).
    pub id: String,
    /// O que aparece na tela (ex.: `Opus`).
    pub label: String,
}

/// Um comando que o agente informa (ex.: `/model`). A função de cada um é
/// uma interface do terra; o agente só diz quais existem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    /// Sem a `/` (ex.: `model`).
    pub name: String,
    pub description: String,
}

/// Um modelo que o agente informa, com o que ele aceita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// O que vai para o provedor (ex.: `opus`).
    pub id: String,
    /// Nome para a tela (ex.: `Opus 5.5`).
    pub label: String,
    pub description: String,
    /// O modelo de fato (ex.: `claude-opus-5-5`), quando o id é um apelido.
    pub resolved: Option<String>,
    /// Níveis de esforço aceitos, do menor para o maior (vazio: não tem).
    pub efforts: Vec<String>,
}

/// Comandos e modelos do agente, carregados uma vez por conta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub commands: Vec<CommandInfo>,
    pub models: Vec<ModelInfo>,
    /// O modelo que o provedor usa quando nenhum é escolhido.
    pub default_model: Option<String>,
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

    /// Modo padrão configurado no provedor para o diretório `cwd` e a conta
    /// `account`, para o chat começar em sincronia com ele.
    fn configured_mode(&self, _cwd: &Path, _account: Option<&str>) -> Option<PermissionMode> {
        None
    }

    /// Contas que podem ser escolhidas na Config, a padrão primeiro. Vazio
    /// quando o provedor não tem contas.
    fn accounts(&self) -> Vec<AccountOption> {
        Vec::new()
    }

    /// Modelos que podem ser escolhidos na Config, além do padrão.
    fn models(&self) -> Vec<ModelOption> {
        Vec::new()
    }

    /// Nome do modelo para a tela (ex.: `claude-opus-5-5` → `Opus - 5.5`).
    fn model_label(&self, id: &str) -> String {
        id.to_string()
    }

    /// Começa a carregar, em segundo plano, os comandos e modelos do agente
    /// para a conta `account`. `None` quando o provedor não informa nada.
    fn load_catalog(&self, _account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        None
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
