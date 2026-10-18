mod term;

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use terraforme_core::agent::{self, Mode, Provider};
use terraforme_core::config::Config;
use terraforme_core::extension::{Extension, Outcome};
use terraforme_core::jail;
use terraforme_core::space::{self, Spaces};
use terraforme_core::status::{self, Component, State};

/// terraforme — CLI
#[derive(Debug, Parser)]
#[command(name = "terraforme", version, about, long_about = None)]
struct Cli {
    /// Exibe mensagens detalhadas
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Sem subcomando, mostra a ajuda.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Exibe a versão do terraforme
    Version,
    /// Verifica as extensões (RTK, AI Memory, AI Jail)
    Status,
    /// Instala, atualiza, configura, remove ou mostra as extensões (RTK, AI Memory, AI Jail)
    #[command(group(
        ArgGroup::new("action")
            .required(true)
            .args(["install", "update", "setup", "project", "default", "delete", "status"]),
    ))]
    Extension {
        /// Extensão alvo; obrigatória com --update e --delete
        #[arg(value_enum)]
        name: Option<Extension>,
        /// Instala a extensão (sem nome: instala todas)
        #[arg(long)]
        install: bool,
        /// Atualiza a extensão para a última versão
        #[arg(long, requires = "name")]
        update: bool,
        /// Refaz a configuração da extensão (no AI Jail: confere a versão e o bwrap e completa o ~/.ai-jail)
        #[arg(long)]
        setup: bool,
        /// AI Jail: cria o .ai-jail na pasta atual, e o `terraforme cli` passa a rodar no AI Jail nela
        #[arg(long)]
        project: bool,
        /// AI Jail: liga (true) ou desliga (false) a sandbox por padrão no `terraforme cli`
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set)]
        default: Option<bool>,
        /// Remove a extensão e a configuração dela nos agentes
        #[arg(long, requires = "name")]
        delete: bool,
        /// Mostra o status e a versão (sem nome: todas)
        #[arg(long)]
        status: bool,
        /// Não pede confirmação ao remover
        #[arg(short, long)]
        yes: bool,
    },
    /// Mostra ou altera a configuração (agente, modo padrão, sandbox e orçamento)
    Config {
        /// Agente padrão do `terraforme cli` (claude, opencode). Sem valor: lista e pergunta
        #[arg(long, visible_alias = "agente", alias = "provider", value_enum, num_args = 0..=1)]
        agent: Option<Option<Provider>>,
        /// Define o modo padrão do `terraforme cli`
        #[arg(long, value_enum)]
        mode: Option<Mode>,
        /// Roda o agente dentro do AI Jail (sandbox) por padrão
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set)]
        jail: Option<bool>,
        /// Orçamento de tokens nas últimas 5h, para a aba de consumo sem login OAuth (0 remove)
        #[arg(long = "budget-5h", value_name = "TOKENS")]
        budget_five_hours: Option<u64>,
        /// Orçamento de tokens nos últimos 7 dias (0 remove)
        #[arg(long = "budget-week", value_name = "TOKENS")]
        budget_week: Option<u64>,
    },
    /// Abre o agente de IA no terminal do terraforme (abas de consumo, RTK e AI Memory)
    #[command(group(ArgGroup::new("modo").args(["eco", "high", "hybrid"])))]
    Cli {
        /// Agente desta sessão, qualquer que seja o padrão (`terraforme config --agent`)
        #[arg(long, visible_alias = "agente", value_enum)]
        agent: Option<Provider>,
        /// Só RTK (economia de tokens)
        #[arg(long)]
        eco: bool,
        /// Só AI Memory (memória entre sessões)
        #[arg(long)]
        high: bool,
        /// RTK + AI Memory
        #[arg(long)]
        hybrid: bool,
        /// Roda o agente dentro do AI Jail (sandbox), qualquer que seja o padrão
        #[arg(long, conflicts_with = "no_jail")]
        jail: bool,
        /// Roda o agente fora do AI Jail, qualquer que seja o padrão
        #[arg(long)]
        no_jail: bool,
        /// Abre o agente direto, sem o terminal do terraforme
        #[arg(long)]
        plain: bool,
        /// Instâncias do agente abertas ao iniciar, cada uma numa sub-aba da aba do agente (F12 abre mais)
        #[arg(
            short = 'n',
            long,
            default_value_t = 1,
            conflicts_with = "plain",
            value_parser = clap::value_parser!(u8).range(1..=term::MAX_INSTANCES as i64),
        )]
        instances: u8,
        /// Argumentos repassados ao agente, depois de `--` (ex.: -- --continue)
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Pastas de trabalho salvas com um nome: lista e abre um shell no space escolhido
    #[command(group(ArgGroup::new("acao").args(["save", "delete", "list", "path"])))]
    Space {
        /// Nome do space (ou número da lista); sem ele, pergunta
        name: Option<String>,
        /// Salva a pasta atual como space (sem nome, pergunta)
        #[arg(long)]
        save: bool,
        /// Remove o space
        #[arg(long, requires = "name")]
        delete: bool,
        /// Só lista os spaces salvos
        #[arg(long)]
        list: bool,
        /// Imprime a pasta em vez de abrir um shell (ex.: cd "$(terraforme space --path)")
        #[arg(long)]
        path: bool,
        /// Não pede confirmação ao substituir ou remover
        #[arg(short, long)]
        yes: bool,
    },
    /// Atualiza o próprio terraforme: git pull no repositório, compila e reinstala
    Upgrade {
        /// Repositório do terraforme (padrão: onde este binário foi compilado)
        #[arg(long, env = "TERRAFORME_REPO", value_name = "DIR")]
        repo: Option<std::path::PathBuf>,
        /// Build otimizada (LTO); bem mais lenta para compilar
        #[arg(long)]
        dist: bool,
        /// Não faz git pull, só recompila o código local
        #[arg(long)]
        no_pull: bool,
    },
    /// Exibe uma saudação
    Greet {
        /// Nome a saudar
        #[arg(default_value = "mundo")]
        name: String,
    },
    /// Gera o script de autocompletar para o shell (bash, zsh, fish, powershell, elvish)
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        eprintln!("[debug] {cli:?}");
    }

    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };

    match command {
        Command::Version => println!("terraforme {}", terraforme_core::VERSION),
        Command::Status => print_status(),
        Command::Extension {
            name,
            install,
            update,
            setup,
            project,
            default,
            delete,
            status: _,
            yes,
        } => {
            let targets: Vec<Extension> = match name {
                Some(name) => vec![name],
                None => Extension::ALL.to_vec(),
            };
            if (project || default.is_some()) && name != Some(Extension::AiJail) {
                bail!(
                    "--project e --default são do AI Jail: `terraforme extension ai-jail --project`"
                );
            }
            if install {
                install_all(&targets)?;
            } else if update {
                report(targets[0], targets[0].update()?);
            } else if setup {
                if let Some(ext) = name {
                    for line in ext.setup()? {
                        println!("✔ {line}");
                    }
                    println!("✔ {} configurado", ext.name());
                } else {
                    setup_interactive()?;
                }
            } else if project {
                jail_project()?;
            } else if let Some(on) = default {
                config(None, None, Some(on), None, None)?;
            } else if delete {
                let ext = targets[0];
                if yes || confirm(&format!("Remover {}?", ext.name()))? {
                    report(ext, ext.delete()?);
                }
            } else {
                for ext in targets {
                    print_component(&ext.check());
                }
            }
        }
        Command::Config {
            agent,
            mode,
            jail,
            budget_five_hours,
            budget_week,
        } => config(agent, mode, jail, budget_five_hours, budget_week)?,
        Command::Cli {
            agent,
            eco,
            high,
            hybrid,
            jail,
            no_jail,
            plain,
            instances,
            args,
        } => {
            let choice = Choice {
                agent,
                mode: pick_mode(eco, high, hybrid),
                jail: (jail || no_jail).then_some(jail),
            };
            if plain {
                run_agent(choice, &args)?
            } else {
                run_term(choice, instances.into(), &args)?
            }
        }
        Command::Space {
            name,
            save,
            delete,
            list,
            path,
            yes,
        } => space(name.as_deref(), save, delete, list, path, yes)?,
        Command::Upgrade {
            repo,
            dist,
            no_pull,
        } => upgrade(repo, dist, no_pull)?,
        Command::Greet { name } => println!("{}", terraforme_core::greeting(&name)),
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let bin = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin, &mut io::stdout());
        }
    }

    Ok(())
}

fn config(
    agent: Option<Option<Provider>>,
    mode: Option<Mode>,
    jail: Option<bool>,
    budget_five_hours: Option<u64>,
    budget_week: Option<u64>,
) -> Result<()> {
    let mut config = Config::load()?;
    let mut changed = false;

    let agent = match agent {
        // Sem valor: lista e, num terminal, pergunta qual vira o padrão.
        Some(None) => {
            print_providers(config.provider);
            if !io::stdin().is_terminal() {
                return Ok(());
            }
            let answer = ask(&format!(
                "Agente padrão (1-{} ou nome; Enter mantém {}): ",
                Provider::ALL.len(),
                config.provider.id()
            ))?;
            if answer.is_empty() {
                return Ok(());
            }
            let Some(p) = Provider::pick(&answer) else {
                bail!("agente {answer} não encontrado");
            };
            Some(p)
        }
        Some(Some(p)) => Some(p),
        None => None,
    };
    match agent {
        Some(p) if !p.is_implemented() => {
            bail!("o agente {} ainda não foi implementado", p.name())
        }
        Some(p) => {
            if !p.check().state.is_installed() {
                eprintln!(
                    "⚠ `{}` não está no PATH — instale o {}: {}",
                    p.id(),
                    p.name(),
                    p.url()
                );
            }
            config.provider = p;
            changed = true;
        }
        None => {}
    }
    if let Some(mode) = mode {
        config.mode = mode;
        changed = true;
    }
    if let Some(jail) = jail {
        config.jail = jail;
        changed = true;
    }
    for (value, field) in [
        (budget_five_hours, &mut config.budget.five_hours),
        (budget_week, &mut config.budget.week),
    ] {
        if let Some(value) = value {
            *field = (value > 0).then_some(value);
            changed = true;
        }
    }

    if changed {
        config.save()?;
        println!("✔ Configuração salva");
    }
    println!("{}", Config::path()?.display());
    println!(
        "  agente    {} ({})",
        config.provider.id(),
        config.provider.name()
    );
    println!(
        "  modo      {} ({})",
        config.mode.id(),
        mode_label(config.mode)
    );
    println!("  sandbox   {}", if config.jail { "AI Jail" } else { "—" });
    if let Ok(dir) = std::env::current_dir()
        && let Some(path) = jail::project_config(&dir)
    {
        println!("            AI Jail nesta pasta ({})", path.display());
    }
    let budget = |value: Option<u64>| value.map_or("—".to_string(), |v| format!("{v} tokens"));
    println!("  orçamento 5h {}", budget(config.budget.five_hours));
    println!("  orçamento 7d {}", budget(config.budget.week));
    Ok(())
}

fn print_providers(current: Provider) {
    println!("Agentes");
    for (i, p) in Provider::ALL.into_iter().enumerate() {
        let marker = if p == current { "●" } else { "○" };
        let state = if !p.is_implemented() {
            "não implementado".to_string()
        } else {
            match p.check().state {
                State::Installed { path, version } => {
                    let version = version.map(|v| format!(" v{v}")).unwrap_or_default();
                    format!("✔ instalado{version} ({})", path.display())
                }
                _ => format!("✘ não instalado — {}", p.url()),
            }
        };
        println!(
            "  {} {marker} {:<8} {:<12} {state}",
            i + 1,
            p.id(),
            p.name()
        );
    }
}

/// `terraforme extension ai-jail --project`.
fn jail_project() -> Result<()> {
    let dir = std::env::current_dir().context("pasta atual")?;
    let agent = Config::load()?.provider;
    let bin = agent.adapter().context("agente sem adapter")?.bin();
    let path = jail::init_project(&dir, bin)?;
    println!("✔ {} criado", path.display());
    println!(
        "  o `terraforme cli` aberto nesta pasta roda no AI Jail (--no-jail desliga numa sessão);"
    );
    println!("  regras do projeto (pastas, hosts liberados) vão nesse arquivo");
    Ok(())
}

fn mode_label(mode: Mode) -> String {
    let names: Vec<_> = mode.extensions().iter().map(|e| e.name()).collect();
    names.join(" + ")
}

/// Título da sessão: agente, modo e, se for o caso, a sandbox.
fn launch_title(provider: Provider, mode: Mode, jail: bool) -> String {
    let sandbox = if jail { " · AI Jail" } else { "" };
    format!(
        "{} · modo {} ({}){sandbox}",
        provider.name(),
        mode.id(),
        mode_label(mode)
    )
}

fn pick_mode(eco: bool, high: bool, hybrid: bool) -> Option<Mode> {
    match (eco, high, hybrid) {
        (true, _, _) => Some(Mode::Eco),
        (_, true, _) => Some(Mode::High),
        (_, _, true) => Some(Mode::Hybrid),
        _ => None,
    }
}

/// Agente, modo e sandbox pedidos na linha de comando (`None` = o padrão).
struct Choice {
    agent: Option<Provider>,
    mode: Option<Mode>,
    jail: Option<bool>,
}

/// Sessão resolvida: a linha de comando vence o `.ai-jail` do projeto, que
/// vence o config.toml.
struct Resolved {
    provider: Provider,
    mode: Mode,
    jail: bool,
    launch: agent::Launch,
}

impl Choice {
    fn resolve(self, args: &[String]) -> Result<Resolved> {
        let config = Config::load()?;
        let provider = self.agent.unwrap_or(config.provider);
        let mode = self.mode.unwrap_or(config.mode);
        let project = std::env::current_dir()
            .ok()
            .and_then(|dir| jail::project_config(&dir));
        let jail = self.jail.unwrap_or(config.jail || project.is_some());
        let mut launch = agent::prepare(provider, mode, jail, args)?;
        if let Some(path) = project
            && jail
            && self.jail.is_none()
            && !config.jail
        {
            launch.notices.push(format!(
                "AI Jail ligado pelo {} desta pasta",
                path.display()
            ));
        }
        Ok(Resolved {
            provider,
            mode,
            jail,
            launch,
        })
    }
}

fn run_term(choice: Choice, instances: usize, args: &[String]) -> Result<()> {
    let Resolved {
        provider,
        mode,
        jail,
        launch,
    } = choice.resolve(args)?;
    let adapter = provider.adapter().context("agente sem adapter")?;

    let notices = launch
        .notices
        .iter()
        .map(|n| format!("ℹ {n}"))
        .chain(launch.warnings.iter().map(|w| format!("⚠ {w}")))
        .collect();
    let title = launch_title(provider, mode, jail);
    let mut extensions = mode.extensions();
    if jail {
        extensions.push(Extension::AiJail);
    }
    term::run(term::Options {
        agent: provider.id().to_string(),
        title,
        adapter,
        command: launch.command,
        instances,
        session_ids: !adapter.picks_session(args),
        notices,
        extensions,
    })
}

fn run_agent(choice: Choice, args: &[String]) -> Result<()> {
    let Resolved {
        provider,
        mode,
        jail,
        launch,
    } = choice.resolve(args)?;

    for notice in &launch.notices {
        eprintln!("ℹ {notice}");
    }
    for warning in &launch.warnings {
        eprintln!("⚠ {warning}");
    }
    eprintln!("==> {}", launch_title(provider, mode, jail));

    exec(launch.command)
}

/// Substitui o processo do terraforme pelo comando (agente ou shell do
/// space), que fica com o terminal.
#[cfg(unix)]
fn exec(mut command: std::process::Command) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let err = command.exec();
    Err(err).with_context(|| format!("falha ao iniciar {}", command.get_program().display()))
}

#[cfg(not(unix))]
fn exec(mut command: std::process::Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("falha ao iniciar {}", command.get_program().display()))?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Roda o `install.linux.sh` do repositório no lugar do terraforme (`exec`),
/// para que o binário em uso possa ser substituído.
fn upgrade(repo: Option<std::path::PathBuf>, dist: bool, no_pull: bool) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("o upgrade automático só existe no Linux; recompile com `cargo build --release`");
    }
    let repo =
        repo.unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let script = repo.join("install.linux.sh");
    if !script.is_file() {
        bail!(
            "{} não encontrado; informe o repositório com --repo ou TERRAFORME_REPO",
            script.display()
        );
    }

    let mut command = std::process::Command::new("bash");
    command.arg(&script);
    if !no_pull {
        command.arg("--update");
    }
    if dist {
        command.arg("--dist");
    }
    // Reinstala no mesmo prefixo do binário atual (<prefixo>/bin/terraforme).
    if std::env::var_os("PREFIX").is_none()
        && let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
        && bin.file_name().is_some_and(|n| n == "bin")
        && let Some(prefix) = bin.parent()
    {
        command.env("PREFIX", prefix);
    }
    exec(command)
}

fn space(
    name: Option<&str>,
    save: bool,
    delete: bool,
    list: bool,
    path: bool,
    yes: bool,
) -> Result<()> {
    let mut spaces = Spaces::load()?;

    if save {
        let dir = std::env::current_dir().context("pasta atual")?;
        let name = match name {
            Some(name) => name.to_string(),
            None => {
                let default = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let answer = ask(&format!("Nome do space [{default}]: "))?;
                if answer.is_empty() { default } else { answer }
            }
        };
        let name = name.trim();
        if let Some(old) = spaces.get(name)
            && old != dir
            && !yes
            && !confirm(&format!(
                "O space {name} já aponta para {} — substituir?",
                old.display()
            ))?
        {
            return Ok(());
        }
        spaces.insert(name, &dir)?;
        spaces.save()?;
        println!("✔ space {name} salvo: {}", dir.display());
        return Ok(());
    }

    if delete {
        let name = name.unwrap_or_default();
        if spaces.get(name).is_none() {
            bail!("space {name} não encontrado");
        }
        if yes || confirm(&format!("Remover o space {name}?"))? {
            spaces.remove(name);
            spaces.save()?;
            println!("✔ space {name} removido");
        }
        return Ok(());
    }

    if spaces.is_empty() {
        bail!("nenhum space salvo — entre na pasta e rode `terraforme space --save`");
    }
    if list {
        print_spaces(&spaces, &mut io::stdout())?;
        return Ok(());
    }

    // Lista e pergunta no stderr, para `--path` deixar só a pasta no stdout.
    let choice = match name {
        Some(name) => name.to_string(),
        None => {
            print_spaces(&spaces, &mut io::stderr())?;
            ask(&format!("Space (1-{} ou nome): ", spaces.len()))?
        }
    };
    let Some((name, dir)) = spaces.pick(&choice) else {
        bail!("space {} não encontrado", choice.trim());
    };
    if !dir.is_dir() {
        bail!(
            "a pasta do space {name} não existe mais: {} — remova com `terraforme space {name} --delete`",
            dir.display()
        );
    }
    if path {
        println!("{}", dir.display());
        return Ok(());
    }
    eprintln!("==> space {name} · {} (`exit` volta)", dir.display());
    exec(space::shell(name, dir))
}

fn print_spaces(spaces: &Spaces, out: &mut impl Write) -> Result<()> {
    let width = spaces
        .iter()
        .map(|(n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    writeln!(out, "Spaces")?;
    for (i, (name, dir)) in spaces.iter().enumerate() {
        let missing = if dir.is_dir() {
            ""
        } else {
            "  ✘ pasta não existe"
        };
        writeln!(
            out,
            "  {:>2}  {name:<width$}  {}{missing}",
            i + 1,
            dir.display()
        )?;
    }
    Ok(())
}

/// Pergunta no stderr e devolve a resposta sem espaços nas pontas.
fn ask(question: &str) -> Result<String> {
    eprint!("{question}");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().to_string())
}

/// Seleciona agentes e extensões e só então executa as configurações escolhidas.
fn setup_interactive() -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("`terraforme extension --setup` precisa de um terminal interativo");
    }

    let agents: Vec<_> = Provider::ALL
        .into_iter()
        .filter(|provider| provider.adapter().is_some() && provider.check().state.is_installed())
        .collect();
    let selected_agents = if agents.is_empty() {
        eprintln!("Nenhum agente suportado encontrado no PATH.");
        Vec::new()
    } else {
        let labels: Vec<_> = agents.iter().map(|provider| provider.name()).collect();
        let Some(indices) = ask_selection(
            "Agentes encontrados (selecione um ou mais):",
            &labels,
            "Enter: nenhum; q: cancelar",
        )?
        else {
            println!("Configuração cancelada; nada foi alterado.");
            return Ok(());
        };
        indices.into_iter().map(|index| agents[index]).collect()
    };

    let available_extensions: Vec<_> = if selected_agents.is_empty() {
        vec![Extension::AiJail]
    } else {
        Extension::ALL.to_vec()
    };
    let labels: Vec<_> = available_extensions
        .iter()
        .map(|extension| extension.name())
        .collect();
    let Some(indices) = ask_selection(
        "Extensões para configurar:",
        &labels,
        "Enter: nenhuma; q: cancelar",
    )?
    else {
        println!("Configuração cancelada; nada foi alterado.");
        return Ok(());
    };
    let extensions: Vec<_> = indices
        .into_iter()
        .map(|index| available_extensions[index])
        .collect();

    if extensions.is_empty() {
        println!("Nenhuma extensão selecionada; nada foi alterado.");
        return Ok(());
    }
    setup_selected(&selected_agents, &extensions)
}

fn ask_selection(title: &str, choices: &[&str], hint: &str) -> Result<Option<Vec<usize>>> {
    eprintln!("{title}");
    for (index, choice) in choices.iter().enumerate() {
        eprintln!("  {}) {choice}", index + 1);
    }
    eprintln!("{hint}");
    let answer = ask("Seleção: ")?;
    select_indices(&answer, choices.len())
}

/// Aplica uma configuração selecionada uma extensão/agente por vez para que
/// uma falha não esconda o resultado das outras.
fn setup_selected(agents: &[Provider], extensions: &[Extension]) -> Result<()> {
    let mut failures = 0;
    for &extension in extensions {
        if extension == Extension::AiJail {
            eprintln!("==> {} (configuração da máquina)", extension.name());
            match extension.setup() {
                Ok(lines) => {
                    for line in lines {
                        println!("  {line}");
                    }
                    println!("✔ {} configurado", extension.name());
                }
                Err(error) => {
                    eprintln!("✘ {}: {error:#}", extension.name());
                    failures += 1;
                }
            }
            continue;
        }

        for &provider in agents {
            eprintln!("==> {} · {}", extension.name(), provider.name());
            match extension.setup_for(provider) {
                Ok(messages) => {
                    println!(
                        "✔ {} configurado para {}",
                        extension.name(),
                        provider.name()
                    );
                    for notice in messages.notices {
                        eprintln!("  ℹ {notice}");
                    }
                    for warning in messages.warnings {
                        eprintln!("  ⚠ {warning}");
                    }
                }
                Err(error) => {
                    eprintln!("✘ {} para {}: {error:#}", extension.name(), provider.name());
                    failures += 1;
                }
            }
        }
    }
    if failures > 0 {
        bail!("{failures} etapa(s) de configuração falharam; veja os erros acima");
    }
    Ok(())
}

fn select_indices(answer: &str, count: usize) -> Result<Option<Vec<usize>>> {
    let answer = answer.trim();
    if answer.eq_ignore_ascii_case("q") || answer.eq_ignore_ascii_case("cancelar") {
        return Ok(None);
    }
    if answer.is_empty()
        || answer.eq_ignore_ascii_case("nenhum")
        || answer.eq_ignore_ascii_case("nenhuma")
    {
        return Ok(Some(Vec::new()));
    }

    let mut indices = Vec::new();
    for item in answer.split(|c: char| c == ',' || c.is_whitespace()) {
        if item.is_empty() {
            continue;
        }
        let number: usize = item
            .parse()
            .with_context(|| format!("seleção inválida: {item}"))?;
        if number == 0 || number > count {
            bail!("selecione um número entre 1 e {count}: {number}");
        }
        let index = number - 1;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    Ok(Some(indices))
}

/// Tenta instalar todas antes de falhar, para uma não bloquear a outra.
fn install_all(targets: &[Extension]) -> Result<()> {
    let mut failed = Vec::new();
    for &ext in targets {
        println!("==> {}", ext.name());
        match ext.install() {
            Ok(outcome) => report(ext, outcome),
            Err(err) => {
                eprintln!("✘ {}: {err:#}", ext.name());
                failed.push(ext.name());
            }
        }
    }
    if !failed.is_empty() {
        bail!("falha ao instalar: {}", failed.join(", "));
    }
    Ok(())
}

fn report(ext: Extension, outcome: Outcome) {
    let v = |v: &Option<String>| v.as_deref().map(|v| format!("v{v}")).unwrap_or("?".into());
    let name = ext.name();
    match outcome {
        Outcome::Installed { version } => println!("✔ {name} {} instalado", v(&version)),
        Outcome::AlreadyInstalled { version } => println!(
            "✔ {name} {} já está instalado — use `terraforme extension {} --update`",
            v(&version),
            ext.id()
        ),
        Outcome::Updated { from, to } => {
            println!("✔ {name} atualizado: {} → {}", v(&from), v(&to))
        }
        Outcome::UpToDate { version } => {
            println!("✔ {name} {} já está na última versão", v(&version))
        }
        Outcome::Deleted => println!("✔ {name} removido"),
    }
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [s/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "sim" | "y" | "yes"
    ))
}

fn print_status() {
    let status = status::check();

    println!("terraforme v{}\n", status.version);
    println!("Extensões");
    for ext in &status.extensions {
        print_component(ext);
    }

    if !status.all_installed() {
        println!("\nAlgum componente está ausente — veja os links acima para instalar.");
    }
}

fn print_component(c: &Component) {
    match &c.state {
        State::Installed { path, version } => {
            let version = version
                .as_deref()
                .map(|v| format!(" v{v}"))
                .unwrap_or_default();
            println!("  ✔ {}{version}  ({})", c.name, path.display());
        }
        State::Conflict { path, detail } => {
            println!("  ⚠ {}  ({}) — {detail}", c.name, path.display());
        }
        State::NotInstalled if c.url.is_empty() => println!("  ✘ {}  não instalado", c.name),
        State::NotInstalled => println!("  ✘ {}  não instalado — {}", c.name, c.url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from([&["terraforme"], args].concat())
    }

    #[test]
    fn extension_requires_an_action() {
        assert!(parse(&["extension"]).is_err());
        assert!(parse(&["extension", "rtk"]).is_err());
        assert!(parse(&["extension", "--install", "--status"]).is_err());
    }

    #[test]
    fn no_subcommand_is_accepted() {
        assert!(parse(&[]).unwrap().command.is_none());
    }

    #[test]
    fn version_flag_prints_version() {
        for flag in ["--version", "-V"] {
            let err = parse(&[flag]).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn install_and_status_accept_no_name() {
        assert!(parse(&["extension", "--install"]).is_ok());
        assert!(parse(&["extension", "--status"]).is_ok());
        assert!(parse(&["extension", "ai-memory", "--status"]).is_ok());
    }

    #[test]
    fn config_agent_value_is_optional() {
        let cli = parse(&["config", "--agent"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                agent: Some(None),
                ..
            })
        ));
        // --agente e o antigo --provider continuam valendo.
        for flag in ["--agent", "--agente", "--provider"] {
            let cli = parse(&["config", flag, "opencode"]).unwrap();
            assert!(matches!(
                cli.command,
                Some(Command::Config {
                    agent: Some(Some(Provider::OpenCode)),
                    ..
                })
            ));
        }
        assert!(parse(&["config", "--agent", "claude"]).is_ok());
        assert!(parse(&["config", "--agent", "gemini"]).is_err());
        assert!(parse(&["config", "--mode", "eco"]).is_ok());
        assert!(parse(&["config", "--mode", "turbo"]).is_err());
        assert!(parse(&["config", "--budget-5h", "1000", "--budget-week", "0"]).is_ok());
        assert!(parse(&["config", "--budget-5h", "-1"]).is_err());
    }

    #[test]
    fn cli_accepts_one_mode_and_passthrough_args() {
        let cli = parse(&["cli", "--eco", "--", "--continue", "-p", "oi"]).unwrap();
        let Some(Command::Cli { eco, args, .. }) = cli.command else {
            panic!("esperava Command::Cli");
        };
        assert!(eco);
        assert_eq!(args, ["--continue", "-p", "oi"]);
        assert!(parse(&["cli", "--eco", "--high"]).is_err());
    }

    #[test]
    fn jail_flags() {
        let cli = parse(&["cli", "--jail", "--eco"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Cli {
                jail: true,
                no_jail: false,
                ..
            })
        ));
        assert!(parse(&["cli", "--no-jail"]).is_ok());
        assert!(parse(&["cli", "--jail", "--no-jail"]).is_err());
        assert!(matches!(
            parse(&["config", "--jail", "true"]).unwrap().command,
            Some(Command::Config {
                jail: Some(true),
                ..
            })
        ));
        assert!(parse(&["config", "--jail"]).is_err());
        assert!(parse(&["extension", "ai-jail", "--update"]).is_ok());
    }

    #[test]
    fn extension_setup_project_and_default() {
        assert!(parse(&["extension", "ai-jail", "--setup"]).is_ok());
        assert!(parse(&["extension", "rtk", "--setup"]).is_ok());
        assert!(parse(&["extension", "--setup"]).is_ok());
        assert!(parse(&["extension", "ai-jail", "--project"]).is_ok());
        assert!(matches!(
            parse(&["extension", "ai-jail", "--default", "true"])
                .unwrap()
                .command,
            Some(Command::Extension {
                default: Some(true),
                ..
            })
        ));
        assert!(parse(&["extension", "ai-jail", "--default"]).is_err());
        assert!(parse(&["extension", "ai-jail", "--project", "--setup"]).is_err());
    }

    #[test]
    fn setup_selection_accepts_multiple_choices_and_cancel() {
        assert_eq!(select_indices("1, 3", 3).unwrap(), Some(vec![0, 2]));
        assert_eq!(select_indices("2 2", 3).unwrap(), Some(vec![1]));
        assert_eq!(select_indices("", 3).unwrap(), Some(vec![]));
        assert_eq!(select_indices("nenhum", 3).unwrap(), Some(vec![]));
        assert_eq!(select_indices("q", 3).unwrap(), None);
        assert!(select_indices("0", 3).is_err());
        assert!(select_indices("4", 3).is_err());
        assert!(select_indices("claude", 3).is_err());
    }

    #[test]
    fn cli_agent_overrides_the_default() {
        let cli = parse(&["cli", "--agent", "opencode", "--eco"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Cli {
                agent: Some(Provider::OpenCode),
                eco: true,
                ..
            })
        ));
        assert!(matches!(
            parse(&["cli"]).unwrap().command,
            Some(Command::Cli { agent: None, .. })
        ));
    }

    #[test]
    fn cli_plain_skips_the_terminal() {
        let cli = parse(&["cli", "--plain", "--high"]).unwrap();
        let Some(Command::Cli { plain, high, .. }) = cli.command else {
            panic!("esperava Command::Cli");
        };
        assert!(plain && high);
    }

    #[test]
    fn cli_instances_are_limited() {
        let cli = parse(&["cli", "-n", "3"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Cli { instances: 3, .. })
        ));
        let cli = parse(&["cli"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Cli { instances: 1, .. })
        ));
        assert!(parse(&["cli", "--instances", "0"]).is_err());
        assert!(parse(&["cli", "--instances", "10"]).is_err());
        assert!(parse(&["cli", "--plain", "-n", "2"]).is_err());
    }

    #[test]
    fn space_actions() {
        assert!(parse(&["space"]).is_ok());
        assert!(parse(&["space", "api"]).is_ok());
        assert!(parse(&["space", "--save"]).is_ok());
        assert!(parse(&["space", "api", "--save"]).is_ok());
        assert!(parse(&["space", "--path"]).is_ok());
        assert!(parse(&["space", "--delete"]).is_err());
        assert!(parse(&["space", "api", "--delete", "-y"]).is_ok());
        assert!(parse(&["space", "--save", "--list"]).is_err());
    }

    #[test]
    fn update_and_delete_require_a_name() {
        assert!(parse(&["extension", "--update"]).is_err());
        assert!(parse(&["extension", "--delete"]).is_err());
        assert!(parse(&["extension", "rtk", "--update"]).is_ok());
        assert!(parse(&["extension", "ai-memory", "--delete"]).is_ok());
        assert!(parse(&["extension", "outra", "--delete"]).is_err());
    }
}
