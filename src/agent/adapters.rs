//! Os provedores de agente (routes) disponíveis para o chat.
//!
//! Para adicionar um provedor: crie um módulo aqui que implemente
//! [`ChatAdapter`] e inclua-o em [`all`]. Nada mais muda.

pub mod claude;

use super::ChatAdapter;

/// Os provedores conhecidos, em ordem de preferência.
pub fn all() -> Vec<Box<dyn ChatAdapter>> {
    vec![Box::new(claude::ClaudeAdapter::detect())]
}

/// O primeiro provedor disponível; sem nenhum, o primeiro da lista (o chat
/// mostra Desconectado com a dica de instalação dele).
pub fn default() -> Box<dyn ChatAdapter> {
    let mut adapters = all();
    let index = adapters.iter().position(|a| a.is_available()).unwrap_or(0);
    adapters.swap_remove(index)
}
