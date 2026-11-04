//! Extensões do input: o que aparece na borda e embaixo da caixa de texto,
//! como o nome do provedor e o modelo (no título), o status do agente (no
//! canto direito) e a cor do badge do modo de permissão.

use ratatui::text::{Line, Span};

use crate::agent::PermissionMode;
use crate::theme::{self, Variant};

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

/// "[Provedor] Modelo - Versão", com o provedor no estilo de botão (pill):
/// o título da caixa de input.
pub fn model_title(provider: &str, model: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(format!(" {provider} "), theme::badge_text(Variant::Active)),
        Span::styled(format!(" {model} "), theme::text()),
    ])
}

/// O status do agente no canto direito da borda do input.
pub fn status_line(status: AgentStatus) -> Line<'static> {
    Line::styled(
        format!(" {} ", status.label()),
        theme::variant(status.variant()),
    )
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
#[path = "tests/extensions.rs"]
mod tests;
