//! A configuração do orquestrador, a fonte única da verdade:
//! `~/.terraforme/orchestrator.toml`.
//!
//! ```toml
//! [global]
//! max_execucoes_simultaneas = 3
//! max_hops = 5
//! limite_palavras_mensagem = 500
//! modo_execucao = "aba"          # "aba" ou "segundo_plano"
//! pausado = false
//! tempo_limite_min = 30          # depois disso, a execução é encerrada (timeout); "infinito" não encerra
//! aba_em_primeiro_plano = false  # a aba de uma execução vai para a frente ao abrir
//!
//! [agents.backend]
//! cwd = "/proj/api"
//! provedor = "claude"            # "claude" (padrão) ou "antigravity"
//! modelo = "forte"
//! ferramentas = ["Read", "Edit", "Bash(npm test:*)", "Bash(terra:*)"]
//! max_turnos = 30
//! orcamento_diario = 40          # execuções por dia
//! ambiente = "minimo"            # "minimo" (padrão) ou "completo"
//!
//! [modelos]
//! forte = "opus"
//! leve = "haiku"
//!
//! [roteamento]                   # desligado por padrão
//! ativo = false
//! padrao = "forte"
//! escalonar_falhas = true
//! # [[roteamento.regra]]
//! # tag = "testes"
//! # modelo = "leve"
//! ```
//!
//! Sem o arquivo, vale o padrão (sem agentes despachados). Uma chave
//! desconhecida ou de tipo errado é um erro com o lugar dela, para um erro
//! de digitação não passar calado.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, TableLike};

/// O nome do arquivo, na pasta do terra.
pub const FILE: &str = "orchestrator.toml";

/// Como roda uma execução despachada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunMode {
    /// Numa aba de execução, visível, na janela do workspace do agente.
    #[default]
    Tab,
    /// Em segundo plano, sem aba.
    Background,
}

/// Os provedores que o hub sabe lançar (os ids de
/// [`crate::agent::adapters::options`]); o primeiro é o padrão.
pub const PROVIDERS: [&str; 2] = ["claude", "antigravity"];

/// O valor do `tempo_limite_min` sem limite.
pub const INFINITE: &str = "infinito";

/// O `[global]`: os limites de todos os agentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Global {
    pub max_concurrent: u32,
    pub max_hops: u32,
    pub word_limit: u32,
    pub run_mode: RunMode,
    /// Os despachos ficam parados (o interruptor geral).
    pub paused: bool,
    /// Minutos que uma execução pode rodar antes do timeout; `None` é
    /// sem limite (`"infinito"`).
    pub timeout_min: Option<u32>,
    /// A aba de uma execução vai para a frente ao abrir (no modo `aba`).
    pub focus_runs: bool,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            max_hops: 5,
            word_limit: 500,
            run_mode: RunMode::Tab,
            paused: false,
            timeout_min: Some(30),
            focus_runs: false,
        }
    }
}

/// Um agente despachado (`[agents.<nome>]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub name: String,
    /// A pasta onde a execução roda.
    pub cwd: PathBuf,
    /// Um dos [`PROVIDERS`]; `None` é o Claude.
    pub provider: Option<String>,
    /// Um apelido de `[modelos]` ou o id de um modelo do provedor; `None` é
    /// o padrão dele.
    pub model: Option<String>,
    /// As ferramentas liberadas (`--allowedTools`).
    pub tools: Vec<String>,
    pub max_turns: Option<u32>,
    /// Execuções por dia; `None` é sem limite.
    pub daily_budget: Option<u32>,
    /// Sem o `ambiente = "completo"`, a execução sobe enxuta: só as
    /// ferramentas da lista, sem MCP, skills nem hooks.
    pub full_env: bool,
}

impl Agent {
    /// O id do provedor: o escolhido ou o Claude.
    pub fn provider(&self) -> &str {
        self.provider.as_deref().unwrap_or(PROVIDERS[0])
    }

    /// Roda no Antigravity, que não tem lista de ferramentas nem limite de
    /// turnos no `-p`.
    pub fn is_antigravity(&self) -> bool {
        self.provider() == "antigravity"
    }
}

/// Uma regra do `[[roteamento.regra]]`: a tag escolhe o modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tag: String,
    pub model: String,
}

/// O `[roteamento]`. As regras por tag (a Fase C) ficam desligadas por
/// padrão (`ativo`); o escalonamento por falha vale mesmo assim e fica
/// ligado por padrão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub active: bool,
    pub default: Option<String>,
    /// Uma execução de nível 1 ou 2 que falha pede ao remetente o handoff
    /// completo.
    pub escalate_failures: bool,
    pub rules: Vec<Rule>,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            active: false,
            default: None,
            escalate_failures: true,
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub global: Global,
    /// Na ordem do arquivo.
    pub agents: Vec<Agent>,
    /// Os apelidos de modelo (`forte = "opus"`), na ordem do arquivo.
    pub models: Vec<(String, String)>,
    pub routing: Routing,
}

impl Config {
    /// O caminho do arquivo na pasta `home`.
    pub fn path(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    /// Lê o arquivo; sem ele, o padrão. O erro diz o que está errado.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!("não foi possível ler {}: {err}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|err| format!("TOML inválido: {err}"))?;
        let mut config = Self::default();
        for (key, item) in doc.iter() {
            match key {
                "global" => config.global = global(table(item, "global")?)?,
                "agents" => {
                    for (name, item) in table(item, "agents")?.iter() {
                        let at = format!("agents.{name}");
                        config.agents.push(agent(name, table(item, &at)?, &at)?);
                    }
                }
                "modelos" => {
                    for (alias, item) in table(item, "modelos")?.iter() {
                        let model = text_of(item, &format!("modelos.{alias}"))?;
                        config.models.push((alias.to_string(), model));
                    }
                }
                "roteamento" => config.routing = routing(table(item, "roteamento")?)?,
                other => return Err(format!("seção desconhecida: [{other}]")),
            }
        }
        config.check()?;
        Ok(config)
    }

    /// As regras que valem entre as seções.
    fn check(&self) -> Result<(), String> {
        let aliases: Vec<&str> = self.models.iter().map(|(a, _)| a.as_str()).collect();
        if let Some(default) = &self.routing.default
            && !aliases.contains(&default.as_str())
        {
            return Err(format!(
                "roteamento.padrao: \"{default}\" não está em [modelos]"
            ));
        }
        for rule in &self.routing.rules {
            if !aliases.contains(&rule.model.as_str()) {
                return Err(format!(
                    "roteamento.regra (tag \"{}\"): o modelo \"{}\" não está em [modelos]",
                    rule.tag, rule.model
                ));
            }
        }
        Ok(())
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.name == name)
    }

    /// O id do modelo: o do apelido de `[modelos]` ou, sem apelido, o
    /// próprio nome.
    pub fn model_id<'a>(&'a self, model: &'a str) -> &'a str {
        self.models
            .iter()
            .find(|(alias, _)| alias == model)
            .map_or(model, |(_, id)| id.as_str())
    }
}

/// Os nomes que não são de agentes: o remetente de fora de um agente e o
/// do próprio terra (o escalonamento).
pub const RESERVED: [&str; 2] = ["usuario", "terra"];

/// Um nome de agente: de 1 a 32 letras minúsculas, dígitos, `-` e `_`,
/// começando por letra ou dígito (ex.: `backend`, `front-2`), fora os
/// [`RESERVED`].
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    !RESERVED.contains(&name)
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn global(table: &dyn TableLike) -> Result<Global, String> {
    let mut global = Global::default();
    for (key, item) in table.iter() {
        let at = format!("global.{key}");
        match key {
            "max_execucoes_simultaneas" => global.max_concurrent = positive(item, &at)?,
            "max_hops" => global.max_hops = positive(item, &at)?,
            "limite_palavras_mensagem" => global.word_limit = positive(item, &at)?,
            "modo_execucao" => {
                global.run_mode = match text_of(item, &at)?.as_str() {
                    "aba" => RunMode::Tab,
                    "segundo_plano" => RunMode::Background,
                    other => {
                        return Err(format!(
                            "{at}: \"{other}\" não é \"aba\" nem \"segundo_plano\""
                        ));
                    }
                }
            }
            "pausado" => global.paused = flag(item, &at)?,
            "tempo_limite_min" => {
                global.timeout_min = match item.as_str() {
                    Some(INFINITE) => None,
                    _ => Some(
                        positive(item, &at).map_err(|err| format!("{err} ou \"{INFINITE}\""))?,
                    ),
                }
            }
            "aba_em_primeiro_plano" => global.focus_runs = flag(item, &at)?,
            _ => return Err(format!("chave desconhecida: {at}")),
        }
    }
    Ok(global)
}

fn agent(name: &str, table: &dyn TableLike, at: &str) -> Result<Agent, String> {
    if !valid_name(name) {
        return Err(format!(
            "[{at}]: o nome deve ter letras minúsculas, dígitos, - ou _ (até 32)"
        ));
    }
    let mut agent = Agent {
        name: name.to_string(),
        cwd: PathBuf::new(),
        provider: None,
        model: None,
        tools: Vec::new(),
        max_turns: None,
        daily_budget: None,
        full_env: false,
    };
    for (key, item) in table.iter() {
        let at = format!("{at}.{key}");
        match key {
            "cwd" => agent.cwd = PathBuf::from(text_of(item, &at)?),
            "provedor" => {
                let provider = text_of(item, &at)?;
                if !PROVIDERS.contains(&provider.as_str()) {
                    return Err(format!(
                        "{at}: \"{provider}\" não é {}",
                        PROVIDERS.map(|p| format!("\"{p}\"")).join(" nem ")
                    ));
                }
                agent.provider = Some(provider);
            }
            "modelo" => agent.model = Some(text_of(item, &at)?),
            "ferramentas" => agent.tools = texts(item, &at)?,
            "max_turnos" => agent.max_turns = Some(positive(item, &at)?),
            "orcamento_diario" => agent.daily_budget = Some(positive(item, &at)?),
            "ambiente" => {
                agent.full_env = match text_of(item, &at)?.as_str() {
                    "minimo" => false,
                    "completo" => true,
                    _ => return Err(format!("{at}: deve ser \"minimo\" ou \"completo\"")),
                }
            }
            _ => return Err(format!("chave desconhecida: {at}")),
        }
    }
    if agent.cwd.as_os_str().is_empty() {
        return Err(format!("[{at}]: falta o cwd (a pasta do agente)"));
    }
    if !agent.cwd.is_absolute() {
        return Err(format!("{at}.cwd: o caminho deve ser absoluto"));
    }
    Ok(agent)
}

fn routing(table: &dyn TableLike) -> Result<Routing, String> {
    let mut routing = Routing::default();
    for (key, item) in table.iter() {
        let at = format!("roteamento.{key}");
        match key {
            "ativo" => routing.active = flag(item, &at)?,
            "padrao" => routing.default = Some(text_of(item, &at)?),
            "escalonar_falhas" => routing.escalate_failures = flag(item, &at)?,
            "regra" => {
                let rules = item
                    .as_array_of_tables()
                    .ok_or_else(|| format!("{at}: deve ser [[roteamento.regra]]"))?;
                for rule in rules {
                    let (mut tag, mut model) = (None, None);
                    for (key, item) in rule.iter() {
                        let at = format!("{at}.{key}");
                        match key {
                            "tag" => tag = Some(text_of(item, &at)?),
                            "modelo" => model = Some(text_of(item, &at)?),
                            _ => return Err(format!("chave desconhecida: {at}")),
                        }
                    }
                    let (Some(tag), Some(model)) = (tag, model) else {
                        return Err(format!("{at}: cada regra precisa de tag e modelo"));
                    };
                    routing.rules.push(Rule { tag, model });
                }
            }
            _ => return Err(format!("chave desconhecida: {at}")),
        }
    }
    Ok(routing)
}

fn table<'a>(item: &'a Item, at: &str) -> Result<&'a dyn TableLike, String> {
    item.as_table_like()
        .ok_or_else(|| format!("{at}: deve ser uma tabela ([{at}])"))
}

fn text_of(item: &Item, at: &str) -> Result<String, String> {
    item.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{at}: deve ser um texto entre aspas"))
}

fn flag(item: &Item, at: &str) -> Result<bool, String> {
    item.as_bool()
        .ok_or_else(|| format!("{at}: deve ser true ou false"))
}

fn positive(item: &Item, at: &str) -> Result<u32, String> {
    item.as_integer()
        .filter(|n| *n > 0)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("{at}: deve ser um número inteiro maior que zero"))
}

fn texts(item: &Item, at: &str) -> Result<Vec<String>, String> {
    let array = item
        .as_array()
        .ok_or_else(|| format!("{at}: deve ser uma lista de textos"))?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{at}: deve ser uma lista de textos"))
        })
        .collect()
}

/// O `toml` dos testes com os `cwd = "/..."` absolutos também no Windows,
/// onde um caminho sem o drive não é absoluto (lá vira `C:/...`).
#[cfg(test)]
pub(crate) fn portable(toml: &str) -> String {
    if cfg!(windows) {
        toml.replace("cwd = \"/", "cwd = \"C:/")
    } else {
        toml.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[global]
max_execucoes_simultaneas = 3
max_hops = 5
limite_palavras_mensagem = 500
modo_execucao = "aba"          # "aba" ou "segundo_plano"
pausado = false

[agents.backend]
cwd = "/proj/api"
modelo = "forte"
ferramentas = ["Read", "Edit", "Bash(npm test:*)", "Bash(terra:*)"]
max_turnos = 30
orcamento_diario = 40

[modelos]
forte = "opus"
leve = "haiku"

[roteamento]
ativo = false
padrao = "forte"
escalonar_falhas = true
[[roteamento.regra]]
tag = "testes"
modelo = "leve"
"#;

    #[test]
    fn the_example_of_the_spec_parses() {
        let config = Config::parse(&portable(EXAMPLE)).unwrap();
        assert_eq!(config.global, Global::default());
        let backend = config.agent("backend").unwrap();
        assert!(backend.cwd.is_absolute() && backend.cwd.ends_with("proj/api"));
        assert_eq!(backend.tools.len(), 4);
        assert_eq!(backend.max_turns, Some(30));
        assert_eq!(backend.daily_budget, Some(40));
        assert_eq!(backend.provider(), "claude");
        assert!(!backend.is_antigravity());
        assert_eq!(config.model_id("forte"), "opus");
        assert_eq!(config.model_id("sonnet"), "sonnet");
        assert!(!config.routing.active);
        assert_eq!(config.routing.rules[0].tag, "testes");
    }

    #[test]
    fn the_provider_is_claude_or_antigravity() {
        let config = Config::parse(&portable(
            "[agents.web]\ncwd = \"/w\"\nprovedor = \"antigravity\"\n",
        ))
        .unwrap();
        let web = config.agent("web").unwrap();
        assert_eq!(web.provider(), "antigravity");
        assert!(web.is_antigravity());
    }

    #[test]
    fn missing_file_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&Config::path(dir.path())).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.global.max_hops, 5);
        assert_eq!(config.global.word_limit, 500);
    }

    #[test]
    fn errors_say_where() {
        let err = |text: &str| Config::parse(text).unwrap_err();
        assert!(err("[global]\nmax_hop = 3").contains("global.max_hop"));
        assert!(err("[global]\nmax_hops = 0").contains("global.max_hops"));
        assert!(err("[global]\nmodo_execucao = \"janela\"").contains("janela"));
        assert!(err("[agents.Back]\ncwd = \"/x\"").contains("agents.Back"));
        assert!(err("[agents.api]\nmodelo = \"x\"").contains("falta o cwd"));
        assert!(err("[agents.api]\ncwd = \"rel\"").contains("absoluto"));
        assert!(err("[agents.api]\ncwd = \"/x\"\nferramentas = \"Read\"").contains("lista"));
        assert!(err("[roteamento]\npadrao = \"forte\"").contains("[modelos]"));
        assert!(err("[outra]").contains("[outra]"));
        assert!(
            err("[agents.api]\ncwd = \"/x\"\nprovedor = \"gpt\"").contains("agents.api.provedor")
        );
        assert!(err("[global").contains("TOML inválido"));
    }

    #[test]
    fn agent_names_are_short_lowercase_slugs() {
        for ok in ["backend", "front-2", "a", "x_y"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            "Backend",
            "-x",
            "com espaço",
            &"a".repeat(33),
            "usuario",
            "terra",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
