//! [Headroom](https://github.com/headroomlabs-ai/headroom): comprime a saída
//! das ferramentas, o histórico e os documentos antes de chegarem ao modelo.
//!
//! No agente, ela é o lançador `headroom wrap claude --`: o agente abre atrás
//! do proxy local do Headroom, e o `--` repassa o resto da linha (modo,
//! modelo, sessão...) para o `claude` de dentro, como o próprio Headroom
//! documenta (`headroom wrap copilot -- --model gpt-4o`).

use std::path::PathBuf;

use super::{find_program, version};
use crate::extension::{Contribution, Extension, Launcher, Status};

pub struct Headroom {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    version: Option<String>,
}

impl Headroom {
    /// Procura o `headroom` no `PATH`.
    pub fn detect() -> Self {
        let bin = find_program("headroom");
        let version = bin.as_deref().and_then(version);
        Self { bin, version }
    }
}

impl Extension for Headroom {
    fn id(&self) -> &'static str {
        "headroom"
    }

    fn name(&self) -> &'static str {
        "Headroom"
    }

    fn description(&self) -> &'static str {
        "Comprime a saída das ferramentas antes de chegar ao agente, com os originais recuperáveis"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/headroomlabs-ai/headroom"
    }

    fn status(&self) -> Status {
        match self.bin {
            Some(_) => Status::Installed {
                version: self.version.clone(),
            },
            None => Status::Missing,
        }
    }

    /// O agente inicia pelo `headroom wrap claude --`: o Headroom resolve o
    /// `claude` sozinho (pelo nome, não pelo caminho) e abre atrás do proxy
    /// local; o `--` repassa o resto da linha para ele.
    fn contribution(&self) -> Result<Contribution, String> {
        Ok(Contribution {
            launchers: vec![Launcher {
                program: "headroom".to_string(),
                args: ["wrap", Launcher::HARNESS, "--"]
                    .map(str::to_string)
                    .to_vec(),
                fresh: Vec::new(),
            }],
            ..Contribution::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contribution_wraps_the_agent_with_a_passthrough() {
        let headroom = Headroom {
            bin: Some(PathBuf::from("headroom")),
            version: Some("1.2.0".into()),
        };
        assert_eq!(
            headroom.status(),
            Status::Installed {
                version: Some("1.2.0".into())
            }
        );
        let contribution = headroom.contribution().unwrap();
        assert_eq!(
            contribution.launchers,
            [Launcher {
                program: "headroom".into(),
                args: ["wrap", Launcher::HARNESS, "--"]
                    .map(str::to_string)
                    .to_vec(),
                fresh: Vec::new(),
            }]
        );
        assert!(contribution.hooks.is_empty());
        assert!(contribution.mcp_servers.is_empty());

        let missing = Headroom {
            bin: None,
            version: None,
        };
        assert_eq!(missing.status(), Status::Missing);
        assert!(missing.load_report().is_none());
    }

    #[test]
    fn launch_line_keeps_the_harness_name_and_opens_for_the_rest() {
        let contribution = Headroom {
            bin: Some(PathBuf::from("headroom")),
            version: None,
        }
        .contribution()
        .unwrap();
        assert_eq!(
            contribution.launch_line("/bin/claude", "claude", false),
            ["headroom", "wrap", "claude", "--"]
        );
    }
}
