//! Agentes de IA (providers) e os modos de execução com as extensões.
//!
//! Cada agente é um [`Adapter`]: o que muda de um agente para outro (como
//! ligar as extensões do modo, como escolher a sessão, onde ler o consumo)
//! fica atrás dele, e [`prepare`] monta o comando igual para todos. Para
//! suportar um agente novo:
//!
//! 1. crie um módulo aqui (ex.: `cursor.rs`) com um tipo que implemente
//!    [`Adapter`];
//! 2. adicione a variante em [`Provider`] (o `id` é o nome no config.toml e na
//!    CLI) e devolva o adapter em [`Provider::adapter`].
//!
//! O AI Jail não entra nos modos: com `jail`, o comando inteiro roda dentro
//! dele (ver `jail`), qualquer que seja o agente.

pub(crate) mod claude;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::extension::Extension;
use crate::limits::Limit;
use crate::status::{Component, State, find_in_path};
use crate::usage::Usage;

pub use claude::Claude;

/// Integração do terraforme com um agente de IA.
pub trait Adapter: Sync {
    /// Executável do agente, procurado no `PATH`.
    fn bin(&self) -> &'static str;

    /// Prepara a configuração do agente para as extensões do modo (já
    /// instaladas) e devolve os argumentos que as ligam nesta sessão.
    fn setup(&self, mode: Mode) -> Result<Setup>;

    /// Argumentos que fixam o id da sessão, para o consumo saber qual é a de
    /// cada instância. `None` se o agente não aceita escolher o id.
    fn session_args(&self, _session: &str) -> Option<Vec<String>> {
        None
    }

    /// `true` se os argumentos repassados ao agente já escolhem a sessão —
    /// aí o terraforme não passa a dele.
    fn picks_session(&self, _args: &[String]) -> bool {
        false
    }

    /// Consumo de tokens do agente; `session` é o id passado por
    /// [`Adapter::session_args`], se houver.
    fn usage(&self, _cwd: &Path, _launched: SystemTime, _session: Option<&str>) -> Result<Usage> {
        bail!("consumo ainda não disponível para este agente")
    }

    /// Limites do plano do usuário; `None` quando não há (ex.: sem login),
    /// e aí o consumo é comparado com o orçamento do config.toml.
    fn plan_limits(&self) -> Result<Option<Vec<Limit>>> {
        Ok(None)
    }

    /// Desfaz o que [`Adapter::setup`] criou para a extensão (chamado quando
    /// ela é removida).
    fn forget(&self, _ext: Extension) -> Result<()> {
        Ok(())
    }
}

/// O que [`Adapter::setup`] devolve.
#[derive(Debug, Default)]
pub struct Setup {
    /// Argumentos do agente que ligam as extensões do modo.
    pub args: Vec<OsString>,
    /// Pastas que o agente precisa enxergar para cada extensão (montadas no
    /// AI Jail).
    pub plugins: Vec<(Extension, PathBuf)>,
    /// O que mudou nas configurações globais nesta execução.
    pub notices: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    Cursor,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Cursor];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Cursor => "cursor",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Cursor => "Cursor",
        }
    }

    /// Integração com o agente; `None` se ainda não foi implementada.
    pub fn adapter(self) -> Option<&'static dyn Adapter> {
        match self {
            Provider::Claude => Some(&Claude),
            Provider::Cursor => None,
        }
    }

    pub fn is_implemented(self) -> bool {
        self.adapter().is_some()
    }

    pub fn check(self) -> Component {
        let path = self.adapter().and_then(|a| find_in_path(a.bin()));
        Component {
            name: self.name(),
            url: "",
            state: match path {
                Some(path) => State::Installed {
                    path,
                    version: None,
                },
                None => State::NotInstalled,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Só RTK: economia de tokens.
    Eco,
    /// Só AI Memory: memória entre sessões.
    High,
    /// RTK + AI Memory.
    #[default]
    Hybrid,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Eco => "eco",
            Mode::High => "high",
            Mode::Hybrid => "hybrid",
        }
    }

    pub fn uses(self, ext: Extension) -> bool {
        match self {
            Mode::Eco => ext == Extension::Rtk,
            Mode::High => ext == Extension::AiMemory,
            Mode::Hybrid => matches!(ext, Extension::Rtk | Extension::AiMemory),
        }
    }

    pub fn extensions(self) -> Vec<Extension> {
        Extension::ALL
            .into_iter()
            .filter(|&e| self.uses(e))
            .collect()
    }
}

/// Comando pronto para iniciar o agente, mais mensagens para o usuário.
#[derive(Debug)]
pub struct Launch {
    pub command: Command,
    /// O que mudou nas configurações globais nesta execução.
    pub notices: Vec<String>,
    pub warnings: Vec<String>,
}

/// Monta o comando do agente no modo pedido, dentro do AI Jail se `jail`.
/// `args` são repassados ao agente.
pub fn prepare(provider: Provider, mode: Mode, jail: bool, args: &[String]) -> Result<Launch> {
    let Some(adapter) = provider.adapter() else {
        bail!("o provider {} ainda não foi implementado", provider.name());
    };
    let Some(bin) = find_in_path(adapter.bin()) else {
        bail!("`{}` não encontrado no PATH", adapter.bin());
    };

    for ext in mode.extensions() {
        if !ext.check().state.is_installed() {
            bail!(
                "o modo {} usa {}, que não está instalado — rode `terraforme extension {} --install`",
                mode.id(),
                ext.name(),
                ext.id()
            );
        }
    }

    let Setup {
        args: mut agent_args,
        plugins,
        mut notices,
        warnings,
    } = adapter.setup(mode)?;
    agent_args.extend(args.iter().map(OsString::from));

    let command = if jail {
        let (command, jail_notices) = crate::jail::wrap(adapter.bin(), &plugins, &agent_args)?;
        notices.extend(jail_notices);
        command
    } else {
        let mut command = Command::new(bin);
        command.args(&agent_args);
        command
    };

    Ok(Launch {
        command,
        notices,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_provider_has_no_adapter() {
        assert!(Provider::Claude.is_implemented());
        assert!(!Provider::Cursor.is_implemented());
        let err = prepare(Provider::Cursor, Mode::Eco, false, &[]).unwrap_err();
        assert!(err.to_string().contains("não foi implementado"));
    }
}
