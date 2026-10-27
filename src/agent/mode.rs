//! Modo de permissão: o que o agente pode fazer sem pedir aprovação. Cada
//! adapter traduz o modo para o seu provedor.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// O agente escolhe o que aprovar.
    #[default]
    Agente,
    /// O terra aceita tudo o que o agente pedir. É do terra, não do agente:
    /// nunca vem do provedor.
    Yolo,
    /// Pede aprovação para tudo.
    Manual,
    /// Aceita edições automaticamente.
    AcceptEdits,
    /// Só planeja, não executa.
    Plan,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 5] = [
        PermissionMode::Agente,
        PermissionMode::Yolo,
        PermissionMode::Manual,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
    ];

    /// Próximo modo, voltando para o primeiro depois do último.
    pub fn next(self) -> PermissionMode {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Agente => "Agente",
            PermissionMode::Yolo => "Yolo",
            PermissionMode::Manual => "Manual",
            PermissionMode::AcceptEdits => "Accept Edits",
            PermissionMode::Plan => "Plan Mode",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cycles_in_order() {
        let mut mode = PermissionMode::default();
        assert_eq!(mode, PermissionMode::Agente);
        let mut seen = Vec::new();
        for _ in 0..5 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(
            seen,
            [
                PermissionMode::Yolo,
                PermissionMode::Manual,
                PermissionMode::AcceptEdits,
                PermissionMode::Plan,
                PermissionMode::Agente,
            ]
        );
    }
}
