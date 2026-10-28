//! O registro das extensões (padrão Registry): guarda as estratégias e junta
//! o que as ativas (pelo id) acrescentam ao agente.

use super::{Contribution, Extension};

#[derive(Default)]
pub struct Registry {
    extensions: Vec<Box<dyn Extension>>,
}

impl Registry {
    pub fn new(extensions: Vec<Box<dyn Extension>>) -> Self {
        Self { extensions }
    }

    /// As extensões conhecidas ([`super::extensions::all`]), já detectadas.
    pub fn detect() -> Self {
        Self::new(super::extensions::all())
    }

    /// Todas, na ordem do registro.
    pub fn all(&self) -> impl Iterator<Item = &dyn Extension> {
        self.extensions.iter().map(Box::as_ref)
    }

    /// O que as extensões `enabled` (ids) acrescentam ao agente, na ordem do
    /// registro. As que não estão instaladas (ou não existem) ficam de fora;
    /// as que falham, também, com o erro na lista de avisos.
    pub fn contribution(&self, enabled: &[String]) -> (Contribution, Vec<String>) {
        let mut total = Contribution::default();
        let mut warnings = Vec::new();
        let active = self
            .all()
            .filter(|e| enabled.iter().any(|id| id == e.id()))
            .filter(|e| e.status().is_installed());
        for extension in active {
            match extension.contribution() {
                Ok(contribution) => total.merge(contribution),
                Err(error) => warnings.push(format!("{}: {error}", extension.name())),
            }
        }
        (total, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{Hook, McpServer, Status};

    struct Fake {
        id: &'static str,
        installed: bool,
        result: Result<Contribution, String>,
    }

    impl Extension for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn homepage(&self) -> &'static str {
            ""
        }
        fn status(&self) -> Status {
            if self.installed {
                Status::Installed { version: None }
            } else {
                Status::Missing
            }
        }
        fn contribution(&self) -> Result<Contribution, String> {
            self.result.clone()
        }
    }

    fn hook(command: &str) -> Contribution {
        Contribution {
            hooks: vec![Hook {
                event: "PreToolUse".into(),
                matcher: String::new(),
                command: command.into(),
            }],
            mcp_servers: Vec::new(),
            env: Vec::new(),
            launchers: Vec::new(),
        }
    }

    fn registry() -> Registry {
        let mcp = Contribution {
            hooks: Vec::new(),
            mcp_servers: vec![McpServer {
                name: "b".into(),
                command: "b".into(),
                args: Vec::new(),
                ..Default::default()
            }],
            env: Vec::new(),
            launchers: Vec::new(),
        };
        Registry::new(vec![
            Box::new(Fake {
                id: "a",
                installed: true,
                result: Ok(hook("a")),
            }),
            Box::new(Fake {
                id: "b",
                installed: true,
                result: Ok(mcp),
            }),
            Box::new(Fake {
                id: "faltando",
                installed: false,
                result: Ok(hook("faltando")),
            }),
            Box::new(Fake {
                id: "quebrada",
                installed: true,
                result: Err("falhou".into()),
            }),
        ])
    }

    #[test]
    fn keeps_extensions_in_registry_order() {
        let registry = registry();
        let ids: Vec<_> = registry.all().map(|e| e.id()).collect();
        assert_eq!(ids, ["a", "b", "faltando", "quebrada"]);
    }

    #[test]
    fn contribution_joins_only_enabled_and_installed_extensions() {
        let registry = registry();
        let enabled = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let (total, warnings) = registry.contribution(&[]);
        assert_eq!(total, Contribution::default());
        assert!(warnings.is_empty());

        let (total, warnings) =
            registry.contribution(&enabled(&["b", "a", "faltando", "inexistente"]));
        assert!(warnings.is_empty());
        assert_eq!(total.hooks, hook("a").hooks, "a não instalada fica de fora");
        assert_eq!(total.mcp_servers.len(), 1);

        let (total, warnings) = registry.contribution(&enabled(&["quebrada", "a"]));
        assert_eq!(warnings, ["quebrada: falhou"]);
        assert_eq!(total.hooks.len(), 1);
    }
}
