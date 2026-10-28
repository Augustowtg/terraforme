//! Os provedores de agente (routes) disponíveis para o chat.
//!
//! Para adicionar um provedor: crie um módulo aqui que implemente
//! [`ChatAdapter`] e inclua-o em [`all`]. A Config passa a oferecê-lo
//! ([`options`]) e o chat o usa quando ele é o escolhido ([`select`]).

pub mod antigravity;
pub mod claude;

use super::ChatAdapter;

/// Os provedores conhecidos, em ordem de preferência.
pub fn all() -> Vec<Box<dyn ChatAdapter>> {
    vec![
        Box::new(claude::ClaudeAdapter::detect()),
        Box::new(antigravity::AntigravityAdapter::detect()),
    ]
}

/// Um provedor que pode ser escolhido na Config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOption {
    /// O que fica salvo (ex.: `claude`).
    pub id: String,
    /// O que aparece na tela (ex.: `Claude`).
    pub name: String,
    /// O CLI dele está instalado.
    pub available: bool,
}

/// Os provedores para a Config, na ordem de [`all`].
pub fn options() -> Vec<ProviderOption> {
    super::guard::catch(|| {
        all()
            .iter()
            .map(|adapter| ProviderOption {
                id: adapter.id().to_string(),
                name: adapter.name().to_string(),
                available: adapter.is_available(),
            })
            .collect()
    })
    .unwrap_or_default()
}

/// O provedor escolhido na Config (`id`, ex.: `antigravity`), mesmo que não
/// esteja instalado (o chat fica Desconectado com a dica dele). Sem escolha,
/// ou com um id que não existe mais, o primeiro disponível; sem nenhum, o
/// primeiro da lista. Protegido pela [`guard`](super::guard): um provedor
/// quebrado não impede o terra de abrir.
pub fn select(id: Option<&str>) -> Box<dyn ChatAdapter> {
    super::guard::guarded(|| {
        let mut adapters = all();
        let chosen = id.and_then(|id| adapters.iter().position(|a| a.id() == id));
        let index = chosen
            .or_else(|| adapters.iter().position(|a| a.is_available()))
            .unwrap_or(0);
        adapters.swap_remove(index)
    })
}
