//! Como o status do agente e o modo de permissão aparecem na tela.

use crate::agent::PermissionMode;
use crate::theme::Variant;

// TODO: "Aguardando" vai indicar um pedido de aprovação no modo Manual.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    #[default]
    Desconectado,
    Ocioso,
    Pensando,
    Aguardando,
    Erro,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Desconectado => "Desconectado",
            AgentStatus::Ocioso => "Ocioso",
            AgentStatus::Pensando => "Pensando",
            AgentStatus::Aguardando => "Aguardando",
            AgentStatus::Erro => "Erro",
        }
    }

    pub fn variant(self) -> Variant {
        match self {
            AgentStatus::Desconectado => Variant::Secondary,
            AgentStatus::Ocioso => Variant::Success,
            AgentStatus::Pensando => Variant::Accent,
            AgentStatus::Aguardando => Variant::Warning,
            AgentStatus::Erro => Variant::Danger,
        }
    }
}

/// Cor do badge de cada modo de permissão.
pub fn mode_variant(mode: PermissionMode) -> Variant {
    match mode {
        PermissionMode::Agente => Variant::Active,
        PermissionMode::Yolo => Variant::Warning,
        PermissionMode::Manual => Variant::Accent,
        PermissionMode::AcceptEdits => Variant::Success,
        PermissionMode::Plan => Variant::Secondary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_has_its_own_color() {
        let variants: Vec<_> = PermissionMode::ALL.into_iter().map(mode_variant).collect();
        for (i, v) in variants.iter().enumerate() {
            assert!(!variants[i + 1..].contains(v), "cor repetida: {v:?}");
        }
    }
}
