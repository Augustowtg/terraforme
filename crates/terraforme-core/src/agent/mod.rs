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

pub(crate) mod antigravity;
pub(crate) mod claude;
pub(crate) mod opencode;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::extension::Extension;
use crate::limits::Quota;
use crate::status::{Component, State, find_in_path, version_of};
use crate::usage::Usage;

pub use antigravity::Antigravity;
pub use claude::Claude;
pub use opencode::OpenCode;

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

    /// Limites de uso do plano/conta no formato padronizado do terraforme
    /// ([`Quota`]): o adapter só traduz o que o CLI informa (a API do plano,
    /// os transcripts, o `stats`) e o terraforme calcula o quanto falta e
    /// desenha a barra. `None` quando o CLI não tem limites (ex.: sem login),
    /// e aí o consumo é comparado com o orçamento do config.toml.
    fn quota(&self) -> Result<Option<Vec<Quota>>> {
        Ok(None)
    }

    /// Desfaz o que [`Adapter::setup`] criou para a extensão (chamado quando
    /// ela é removida).
    fn forget(&self, _ext: Extension) -> Result<()> {
        Ok(())
    }

    /// Pasta global de skills do agente, onde o [`crate::skill`] instala as
    /// skills embutidas; `None` se o agente não tem uma.
    fn skills_dir(&self) -> Option<PathBuf> {
        None
    }
}

/// O que [`Adapter::setup`] devolve.
#[derive(Debug, Default)]
pub struct Setup {
    /// Argumentos do agente que ligam as extensões do modo.
    pub args: Vec<OsString>,
    /// Argumentos que vão depois dos repassados pelo usuário (ex.: flags que
    /// o agente só aceita depois do subcomando).
    pub trailing_args: Vec<OsString>,
    /// Variáveis de ambiente do agente (repassadas para dentro do AI Jail).
    pub env: Vec<(&'static str, OsString)>,
    /// Pastas que o agente precisa enxergar para cada extensão (montadas no
    /// AI Jail).
    pub plugins: Vec<(Extension, PathBuf)>,
    /// Pasta posta na frente do `PATH` no AI Jail, com um executável de mesmo
    /// nome do agente que roda no lugar dele. O comando continua sendo só o
    /// nome (`-- opencode`), porque é por ele que o ai-jail escolhe o preset
    /// e a seção `[commands.<nome>]` da configuração.
    pub jail_path: Option<PathBuf>,
    /// Outros caminhos que o AI Jail monta (somente leitura).
    pub jail_maps: Vec<PathBuf>,
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
    #[cfg_attr(feature = "clap", value(name = "opencode"))]
    OpenCode,
    Cursor,
    Antigravity,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Claude,
        Provider::OpenCode,
        Provider::Cursor,
        Provider::Antigravity,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::OpenCode => "opencode",
            Provider::Cursor => "cursor",
            Provider::Antigravity => "antigravity",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::OpenCode => "OpenCode",
            Provider::Cursor => "Cursor",
            Provider::Antigravity => "Antigravity",
        }
    }

    pub fn url(self) -> &'static str {
        match self {
            Provider::Claude => "https://claude.com/claude-code",
            Provider::OpenCode => "https://opencode.ai",
            Provider::Cursor => "https://cursor.com/cli",
            Provider::Antigravity => "https://antigravity.com",
        }
    }

    /// Integração com o agente; `None` se ainda não foi implementada.
    pub fn adapter(self) -> Option<&'static dyn Adapter> {
        match self {
            Provider::Claude => Some(&Claude),
            Provider::OpenCode => Some(&OpenCode),
            Provider::Cursor => None,
            Provider::Antigravity => Some(&Antigravity),
        }
    }

    pub fn is_implemented(self) -> bool {
        self.adapter().is_some()
    }

    /// `claude`, `opencode`… — o id, como no config.toml; o número da lista
    /// também vale (1 = o primeiro de [`Provider::ALL`]).
    pub fn pick(choice: &str) -> Option<Provider> {
        let choice = choice.trim().to_lowercase();
        if let Ok(n) = choice.parse::<usize>() {
            return n.checked_sub(1).and_then(|i| Provider::ALL.get(i).copied());
        }
        Provider::ALL
            .into_iter()
            .find(|p| p.id() == choice || p.name().to_lowercase() == choice)
    }

    pub fn check(self) -> Component {
        let path = self.adapter().and_then(|a| find_in_path(a.bin()));
        Component {
            name: self.name(),
            url: self.url(),
            state: match path {
                Some(path) => State::Installed {
                    version: version_of(&path),
                    path,
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
    /// Nenhuma extensão: o agente roda puro (é o modo do preset `security`).
    None,
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
            Mode::None => "none",
            Mode::Eco => "eco",
            Mode::High => "high",
            Mode::Hybrid => "hybrid",
        }
    }

    pub fn uses(self, ext: Extension) -> bool {
        match self {
            Mode::None => false,
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

/// Presets de início do `terraforme cli`, mostrados por `terraforme config --cli`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum CliMode {
    /// Eco: só RTK.
    Eco,
    /// Custom: o usuário escolhe quais extensões usar.
    Custom,
    /// Security: roda o agente dentro do AI Jail, sem extensões.
    Security,
    /// High: só AI Memory.
    High,
    /// Doc: AI Memory com MCP para geração de documentação.
    Doc,
}

impl CliMode {
    pub const ALL: [CliMode; 5] = [
        CliMode::Eco,
        CliMode::Custom,
        CliMode::Security,
        CliMode::High,
        CliMode::Doc,
    ];

    pub fn id(self) -> &'static str {
        match self {
            CliMode::Eco => "eco",
            CliMode::Custom => "custom",
            CliMode::Security => "security",
            CliMode::High => "high",
            CliMode::Doc => "doc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CliMode::Eco => "Eco (RTK)",
            CliMode::Custom => "Customizado (você escolhe as extensões)",
            CliMode::Security => "Security (AI Jail)",
            CliMode::High => "High (AI Memory)",
            CliMode::Doc => "Doc (AI Memory + MCP)",
        }
    }

    /// `(modo, sandbox)` fixo do preset; `None` no `Custom`, que pergunta as
    /// extensões na hora.
    pub fn preset(self) -> Option<(Mode, bool)> {
        match self {
            CliMode::Eco => Some((Mode::Eco, false)),
            CliMode::Custom => None,
            CliMode::Security => Some((Mode::None, true)),
            CliMode::High => Some((Mode::High, false)),
            CliMode::Doc => Some((Mode::High, false)),
        }
    }

    /// O preset que corresponde a este modo e sandbox (o contrário de
    /// [`CliMode::preset`]).
    pub fn of(mode: Mode, jail: bool) -> CliMode {
        match (mode, jail) {
            (Mode::Eco, false) => CliMode::Eco,
            (Mode::High, false) => CliMode::High,
            (Mode::None, true) => CliMode::Security,
            _ => CliMode::Custom,
        }
    }

    /// O preset pelo id ou pelo número da lista.
    pub fn pick(choice: &str) -> Option<CliMode> {
        let choice = choice.trim().to_lowercase();
        if let Ok(n) = choice.parse::<usize>() {
            return n.checked_sub(1).and_then(|i| CliMode::ALL.get(i).copied());
        }
        CliMode::ALL
            .into_iter()
            .find(|c| c.id() == choice || c.label().to_lowercase().starts_with(&choice))
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
        trailing_args,
        env,
        plugins,
        jail_path,
        jail_maps,
        mut notices,
        warnings,
    } = adapter.setup(mode)?;
    agent_args.extend(args.iter().map(OsString::from));
    agent_args.extend(trailing_args);

    let mut command = if jail {
        let names: Vec<_> = env.iter().map(|(name, _)| *name).collect();
        let (mut command, jail_notices) =
            crate::jail::wrap(adapter.bin(), &plugins, &jail_maps, &names, &agent_args)?;
        notices.extend(jail_notices);
        // O ai-jail leva o PATH do host para dentro da sandbox.
        if let Some(dir) = jail_path {
            let rest = env::var_os("PATH").unwrap_or_default();
            let path = env::join_paths(std::iter::once(dir).chain(env::split_paths(&rest)))
                .context("PATH inválido")?;
            command.env("PATH", path);
        }
        command
    } else {
        let mut command = Command::new(bin);
        command.args(&agent_args);
        command
    };
    // Com o AI Jail, o `--env NOME` de `jail::wrap` copia o valor daqui.
    command.envs(env);

    Ok(Launch {
        command,
        notices,
        warnings,
    })
}

pub(crate) fn home() -> Result<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("pasta do usuário não encontrada")
}

/// Lê um JSON; arquivo ausente vale como objeto vazio.
pub(crate) fn read_json(path: &Path) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("lendo {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(err) => Err(err).with_context(|| format!("lendo {}", path.display())),
    }
}

/// Grava num arquivo temporário e renomeia, para nunca deixar um JSON pela
/// metade. Os arquivos podem ter segredos (o `.mcp.json` copia o servidor do
/// AI Memory), então só o próprio usuário os lê.
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_private(path, serde_json::to_string_pretty(value)?.as_bytes())
}

/// Grava `content` em `path` (atômico, só o usuário lê), criando a pasta.
pub(crate) fn write_private(path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;

    let dir = path.parent().context("caminho sem pasta")?;
    fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("gravando {}", tmp.display()))?;
    file.write_all(content)?;
    file.sync_all()?;

    fs::rename(&tmp, path).with_context(|| format!("gravando {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_provider_has_no_adapter() {
        assert!(Provider::Claude.is_implemented());
        assert!(Provider::OpenCode.is_implemented());
        assert!(!Provider::Cursor.is_implemented());
        let err = prepare(Provider::Cursor, Mode::Eco, false, &[]).unwrap_err();
        assert!(err.to_string().contains("não foi implementado"));
    }

    #[test]
    fn picks_provider_by_id_name_or_number() {
        assert_eq!(Provider::pick("opencode"), Some(Provider::OpenCode));
        assert_eq!(Provider::pick(" Claude Code "), Some(Provider::Claude));
        assert_eq!(Provider::pick("2"), Some(Provider::OpenCode));
        assert_eq!(Provider::pick("0"), None);
        assert_eq!(Provider::pick("9"), None);
        assert_eq!(Provider::pick("gemini"), None);
    }

    #[test]
    fn provider_ids_round_trip_in_toml() {
        for p in Provider::ALL {
            let text = toml::to_string(&crate::config::Config {
                provider: p,
                ..Default::default()
            })
            .unwrap();
            assert!(
                text.contains(&format!("provider = \"{}\"", p.id())),
                "{text}"
            );
        }
    }

    #[test]
    fn cli_presets_map_to_mode_and_jail() {
        assert_eq!(CliMode::Eco.preset(), Some((Mode::Eco, false)));
        assert_eq!(CliMode::High.preset(), Some((Mode::High, false)));
        assert_eq!(CliMode::Security.preset(), Some((Mode::None, true)));
        assert_eq!(CliMode::Custom.preset(), None);
        // Doc = AI Memory (com o MCP dela), sem sandbox.
        assert_eq!(CliMode::Doc.preset(), Some((Mode::High, false)));

        assert_eq!(CliMode::of(Mode::Eco, false), CliMode::Eco);
        assert_eq!(CliMode::of(Mode::High, false), CliMode::High);
        assert_eq!(CliMode::of(Mode::None, true), CliMode::Security);
        assert_eq!(CliMode::of(Mode::Hybrid, false), CliMode::Custom);
        assert_eq!(CliMode::of(Mode::Eco, true), CliMode::Custom);
    }

    #[test]
    fn picks_cli_mode_by_id_or_number() {
        assert_eq!(CliMode::pick("eco"), Some(CliMode::Eco));
        assert_eq!(CliMode::pick(" security "), Some(CliMode::Security));
        assert_eq!(CliMode::pick("3"), Some(CliMode::Security));
        assert_eq!(CliMode::pick("doc"), Some(CliMode::Doc));
        assert_eq!(CliMode::pick("5"), Some(CliMode::Doc));
        assert_eq!(CliMode::pick("0"), None);
        assert_eq!(CliMode::pick("9"), None);
        assert_eq!(CliMode::pick("turbo"), None);
    }

    #[test]
    fn none_mode_uses_no_extension() {
        assert!(Mode::None.extensions().is_empty());
        for ext in Extension::ALL {
            assert!(!Mode::None.uses(ext));
        }
    }
}
