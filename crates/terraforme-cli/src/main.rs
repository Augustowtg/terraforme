mod chat;
mod markdown;
mod select;
mod term;

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, ArgGroup, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::Shell;
use terraforme_core::accounts::{ClaudeAccounts, is_default_dir};
use terraforme_core::agent::{self, CliMode, Mode, Provider};
use terraforme_core::config::Config;
use terraforme_core::extension::{Extension, Outcome};
use terraforme_core::history::{History, Launch};
use terraforme_core::jail;
use terraforme_core::mcp;
use terraforme_core::pet;
use terraforme_core::plugin;
use terraforme_core::skill;
use terraforme_core::space::{self, Preferences, Spaces};
use terraforme_core::status::{self, Component, State};

/// terraforme — CLI
#[derive(Debug, Parser)]
#[command(
    name = "terraforme",
    version,
    about = "Instala e gerencia as extensões dos agentes de IA (RTK, AI Memory, AI Jail) e abre o \
             agente no terminal do terraforme",
    long_about = None,
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    after_help = "\
Primeiros passos:
  terraforme setup           configura a máquina (agentes, extensões e skills)
  terraforme cli             abre o agente no terminal do terraforme
  terraforme space --save    salva a pasta atual como space
  terraforme --historico     pastas abertas pelo `terraforme cli` (spaces e recentes)

Ajuda de um comando: terraforme <comando> --help"
)]
struct Cli {
    /// Mostra as últimas pastas em que o `terraforme cli` foi aberto, divididas entre spaces e
    /// recentes
    #[arg(long, visible_alias = "history", display_order = 800)]
    historico: bool,

    /// Exibe mensagens detalhadas
    #[arg(short, long, global = true, display_order = 900)]
    verbose: bool,

    /// Mostra a ajuda (-h resume, --help detalha)
    #[arg(short, long, global = true, action = ArgAction::Help, display_order = 901)]
    help: Option<bool>,

    /// Mostra a versão
    #[arg(short = 'V', long, action = ArgAction::Version, display_order = 902)]
    version: Option<bool>,

    /// Sem subcomando, mostra a ajuda.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum PluginAction {
    /// Lista os plugins conhecidos e instalados
    List,
    /// Instala um plugin (nome de `plugin list` ou `dono/repo` do GitHub)
    Install {
        #[arg(value_name = "PLUGIN")]
        name: String,
    },
    /// Atualiza um plugin para a última versão
    Update {
        #[arg(value_name = "PLUGIN")]
        name: String,
    },
    /// Remove um plugin instalado pelo terraforme
    Remove {
        #[arg(value_name = "PLUGIN")]
        name: String,
    },
    /// Atualiza só os plugins já instalados e ligados (o mascote desligado fica como está); usado
    /// pelos scripts de instalação e pelo `terraforme upgrade`
    Sync,
    /// Sai com 0 se o plugin está ligado na configuração, 1 se não (para scripts)
    #[command(hide = true)]
    Enabled {
        #[arg(value_name = "PLUGIN")]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Configura a máquina do zero: identifica (ou instala) os agentes, instala e configura as
    /// extensões neles e instala as skills embutidas
    Setup,
    /// Abre o agente de IA no terminal do terraforme (abas do agente, consumo, extensões e config)
    #[command(
        group(ArgGroup::new("modo").args(["eco", "high", "hybrid"])),
        after_help = "\
Sem opções, usa o agente, o modo e a sandbox do space atual ou do `terraforme config`.

Exemplos:
  terraforme cli                    abre com as preferências salvas
  terraforme cli --hybrid --jail    RTK + AI Memory, dentro do AI Jail
  terraforme cli -n 3               três instâncias, cada uma numa sub-aba
  terraforme cli --agent opencode   usa o OpenCode só nesta sessão
  terraforme cli -- --continue      repassa --continue ao agente

Dentro do terminal, Ctrl+Alt+K lista os atalhos e F10 sai."
    )]
    Cli {
        /// Agente desta sessão (o padrão vem de `terraforme config --agent`)
        #[arg(long, visible_alias = "agente", value_enum, help_heading = "Sessão")]
        agent: Option<Provider>,
        /// Abre o agente direto, sem o terminal do terraforme
        #[arg(long, help_heading = "Sessão")]
        plain: bool,
        /// Quantas instâncias do agente abrir, cada uma numa sub-aba (Ctrl+Alt+N abre mais)
        #[arg(
            short = 'n',
            long,
            value_name = "N",
            default_value_t = 1,
            conflicts_with = "plain",
            help_heading = "Sessão",
            value_parser = clap::value_parser!(u8).range(1..=term::MAX_INSTANCES as i64),
        )]
        instances: u8,
        /// Só RTK (economia de tokens)
        #[arg(long, help_heading = "Modo (sobrepõe o padrão)")]
        eco: bool,
        /// Só AI Memory (memória entre sessões)
        #[arg(long, help_heading = "Modo (sobrepõe o padrão)")]
        high: bool,
        /// RTK + AI Memory
        #[arg(long, help_heading = "Modo (sobrepõe o padrão)")]
        hybrid: bool,
        /// Roda o agente dentro do AI Jail
        #[arg(
            long,
            conflicts_with = "no_jail",
            help_heading = "Sandbox (sobrepõe o padrão)"
        )]
        jail: bool,
        /// Roda o agente fora do AI Jail
        #[arg(long, help_heading = "Sandbox (sobrepõe o padrão)")]
        no_jail: bool,
        /// Argumentos repassados ao agente, depois de `--` (ex.: -- --continue)
        #[arg(last = true, value_name = "ARGS")]
        args: Vec<String>,
    },
    /// Pastas de trabalho com nome: abre um shell no space, salva, renomeia, remove e guarda
    /// preferências do `terraforme cli` para cada um
    #[command(
        group(ArgGroup::new("acao").args(["save", "delete", "list", "path", "rename"])),
        after_help = "\
Exemplos:
  terraforme space                         lista e pergunta qual abrir
  terraforme space api                     abre um shell na pasta do space api
  terraforme space api --save              salva a pasta atual como api
  terraforme space api --mode eco --jail true
  terraforme space api --reset             volta às preferências do config
  cd \"$(terraforme space api --path)\"      muda de pasta no shell atual (bash/zsh)
  cd (terraforme space api --path)         idem no PowerShell"
    )]
    Space {
        /// Nome do space (ou número na lista); sem ele, lista e pergunta
        #[arg(value_name = "NOME", help_heading = "Argumentos")]
        name: Option<String>,
        /// Salva a pasta atual como space (sem NOME, pergunta)
        #[arg(long, help_heading = "Ações")]
        save: bool,
        /// Só lista os spaces salvos
        #[arg(long, help_heading = "Ações")]
        list: bool,
        /// Imprime a pasta em vez de abrir um shell
        #[arg(long, help_heading = "Ações")]
        path: bool,
        /// Renomeia o space
        #[arg(
            long,
            value_name = "NOVO-NOME",
            requires = "name",
            help_heading = "Ações"
        )]
        rename: Option<String>,
        /// Remove o space (a pasta não é apagada)
        #[arg(long, requires = "name", help_heading = "Ações")]
        delete: bool,
        /// Não pede confirmação ao substituir ou remover
        #[arg(short, long, help_heading = "Ações")]
        yes: bool,
        /// Agente aberto pelo `terraforme cli` neste space
        #[arg(long, visible_alias = "agente", value_enum, conflicts_with_all = ["delete", "list", "path", "rename"], help_heading = "Preferências do space")]
        agent: Option<Provider>,
        /// Modo (extensões) do `terraforme cli` neste space
        #[arg(long, value_enum, conflicts_with_all = ["delete", "list", "path", "rename"], help_heading = "Preferências do space")]
        mode: Option<Mode>,
        /// Roda (true) ou não (false) o agente no AI Jail neste space
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set, conflicts_with_all = ["delete", "list", "path", "rename"], help_heading = "Preferências do space")]
        jail: Option<bool>,
        /// Conta do Claude Code neste space (registrada com `terraforme config --account`)
        #[arg(long, value_name = "NOME", conflicts_with_all = ["delete", "list", "path", "rename"], help_heading = "Preferências do space")]
        account: Option<String>,
        /// Apaga as preferências do space (volta ao `terraforme config`)
        #[arg(long, conflicts_with_all = ["delete", "list", "path", "rename"], help_heading = "Preferências do space")]
        reset: bool,
    },
    /// Mostra a configuração padrão do `terraforme cli`; com opções, altera e salva
    #[command(after_help = "\
Exemplos:
  terraforme config                          mostra a configuração atual
  terraforme config --agent                  lista os agentes e pergunta o padrão
  terraforme config --cli                    lista os presets de início e pergunta
  terraforme config --cli security           sempre no AI Jail, sem extensões
  terraforme config --mode hybrid --jail false
  terraforme config --mcp-ui true            liga a UI web (a URL aparece no config)
  terraforme config --account work=$HOME/.claude-work

Preferências por pasta: `terraforme space <nome> --mode/--jail/--agent/--account`.")]
    Config {
        /// Agente padrão. Sem valor: lista e pergunta
        #[arg(long, visible_alias = "agente", alias = "provider", value_enum, num_args = 0..=1, help_heading = "Agente e modo")]
        agent: Option<Option<Provider>>,
        /// Preset de início: define o modo e a sandbox de uma vez. Sem valor: lista e pergunta
        #[arg(long, value_name = "PRESET", value_enum, num_args = 0..=1, conflicts_with_all = ["mode", "jail"], help_heading = "Agente e modo")]
        cli: Option<Option<CliMode>>,
        /// Modo padrão: quais extensões usar (custom pergunta)
        #[arg(long, value_enum, help_heading = "Agente e modo")]
        mode: Option<ModeChoice>,
        /// Roda (true) ou não (false) o agente dentro do AI Jail por padrão
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set, help_heading = "Agente e modo")]
        jail: Option<bool>,
        /// Limite de tokens em 5h, usado na aba Consumo quando o agente não informa o do plano (0 remove)
        #[arg(
            long = "budget-5h",
            value_name = "TOKENS",
            help_heading = "Orçamento de tokens"
        )]
        budget_five_hours: Option<u64>,
        /// Limite de tokens em 7 dias, idem (0 remove)
        #[arg(
            long = "budget-week",
            value_name = "TOKENS",
            help_heading = "Orçamento de tokens"
        )]
        budget_week: Option<u64>,
        /// Liga (true) ou desliga (false) a UI web: instâncias de todos os spaces e mensagens
        /// entre elas
        #[arg(long = "mcp-ui", value_name = "BOOL", action = clap::ArgAction::Set, help_heading = "UI web")]
        mcp_ui: Option<bool>,
        /// Abre (true) ou não (false) a janela flutuante com o mascote junto com o
        /// `terraforme cli` (Linux e Windows)
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set, help_heading = "Janela flutuante")]
        pet: Option<bool>,
        /// Sem valor, lista as contas; com `NOME=PASTA`, registra ou atualiza uma. A pasta é o
        /// `CLAUDE_CONFIG_DIR` da conta (credenciais, configurações, sessões) e precisa existir
        #[arg(long, value_name = "NOME=PASTA", num_args = 0..=1, help_heading = "Contas do Claude Code")]
        account: Option<Option<String>>,
        /// Remove a conta registrada
        #[arg(
            long,
            value_name = "NOME",
            conflicts_with = "account",
            help_heading = "Contas do Claude Code"
        )]
        remove_account: Option<String>,
    },
    /// Instala, atualiza, reconfigura, remove ou mostra as extensões (RTK, AI Memory, AI Jail)
    #[command(
        override_usage = "terraforme extension [NOME] <AÇÃO> [-y]",
        group(
            ArgGroup::new("action")
                .required(true)
                .args(["install", "update", "setup", "project", "default", "delete", "status"]),
        ),
        after_help = "\
Exemplos:
  terraforme extension --status            status e versão de todas
  terraforme extension --install           instala todas
  terraforme extension rtk --update        atualiza o RTK
  terraforme extension ai-memory --setup   refaz a configuração do AI Memory
  terraforme extension ai-memory --delete  remove o AI Memory
  terraforme extension ai-jail --project   sandbox na pasta atual
  terraforme extension ai-jail --default true"
    )]
    Extension {
        /// Extensão alvo (sem ela, a ação vale para todas); obrigatória com --update e --delete
        #[arg(value_enum, value_name = "NOME", help_heading = "Argumentos")]
        name: Option<Extension>,
        /// Mostra o status e a versão
        #[arg(long, help_heading = "Ações")]
        status: bool,
        /// Instala
        #[arg(long, help_heading = "Ações")]
        install: bool,
        /// Atualiza para a última versão
        #[arg(long, requires = "name", help_heading = "Ações")]
        update: bool,
        /// Refaz a configuração nos agentes (sem NOME, pergunta; no AI Jail, confere a versão e o
        /// bwrap e completa o ~/.ai-jail)
        #[arg(long, help_heading = "Ações")]
        setup: bool,
        /// Remove a extensão e a configuração dela nos agentes
        #[arg(long, requires = "name", help_heading = "Ações")]
        delete: bool,
        /// Não pede confirmação ao remover
        #[arg(short, long, help_heading = "Ações")]
        yes: bool,
        /// Cria o .ai-jail na pasta atual: o `terraforme cli` passa a rodar no AI Jail nela
        #[arg(long, help_heading = "Só do AI Jail")]
        project: bool,
        /// Liga (true) ou desliga (false) o AI Jail por padrão no `terraforme cli`
        #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set, help_heading = "Só do AI Jail")]
        default: Option<bool>,
    },
    /// Mostra se as extensões estão instaladas e em que versão, e as pastas do terraforme
    Status,
    /// Atualiza o terraforme para a última release do GitHub; com --dev, pelo repositório
    /// (git pull + compila + reinstala)
    #[command(after_help = "\
Exemplos:
  terraforme upgrade                  baixa e instala a última release (sem Rust)
  terraforme upgrade --dev            git pull + compila + reinstala a partir do repositório
  terraforme upgrade --dev --no-pull  só recompila o código local")]
    Upgrade {
        /// Atualiza pelo repositório (git pull + compila + reinstala) em vez da release
        #[arg(long)]
        dev: bool,
        /// Repositório do terraforme, com --dev (padrão: onde este binário foi compilado)
        #[arg(long, env = "TERRAFORME_REPO", value_name = "DIR")]
        repo: Option<std::path::PathBuf>,
        /// Build otimizada (LTO), com --dev; bem mais lenta para compilar
        #[arg(long, requires = "dev")]
        dist: bool,
        /// Não faz git pull, só recompila o código local (com --dev)
        #[arg(long, requires = "dev")]
        no_pull: bool,
        /// Reinicia o terraforme (daemon MCP e janela flutuante) depois, sem perguntar
        #[arg(long)]
        restart: bool,
    },
    /// Reinicia o terraforme em segundo plano — o daemon MCP (e órfãos de versões antigas) e a
    /// janela flutuante — com os binários instalados agora, ex.: depois de um upgrade
    #[command(after_help = "\
Os terraforme cli abertos se registram sozinhos no daemon novo; os agentes já
abertos neles só voltam a usar o MCP depois de reiniciados (Ctrl+Alt+R).
Terminais abertos antes da 0.22.2 não se reconectam: feche e abra de novo.")]
    Restart,
    /// Abre a janela flutuante com o mascote: avisa quando os agentes terminam e, com um
    /// clique, leva à instância escolhida (Linux e Windows)
    #[command(after_help = "\
Para abrir junto com o terraforme cli: terraforme config --pet true")]
    Pet,
    /// Gerencia os plugins: programas opcionais publicados em repositórios próprios (ex.: o
    /// mascote) e baixados sob demanda
    #[command(after_help = "\
Exemplos:
  terraforme plugin list                  plugins conhecidos e instalados
  terraforme plugin install pet           instala a janela flutuante com o mascote
  terraforme plugin install dono/repo     instala um plugin de terceiros (repo com terraforme-plugin.toml)
  terraforme plugin update pet            baixa a última versão
  terraforme plugin sync                  atualiza os instalados e ligados (o que o upgrade faz)
  terraforme plugin remove pet            remove")]
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Mostra a versão do terraforme
    Version,
    /// Gera o script de autocompletar do shell
    #[command(after_help = "\
Exemplo:
  terraforme completions bash > ~/.local/share/bash-completion/completions/terraforme")]
    Completions {
        /// Shell alvo
        #[arg(value_enum, value_name = "SHELL", help_heading = "Argumentos")]
        shell: Shell,
    },
    /// Mostra a ajuda de um comando (ex.: terraforme help space)
    Help {
        /// Comando (e subcomandos) cuja ajuda mostrar
        #[arg(value_name = "COMANDO", help_heading = "Argumentos")]
        command: Vec<String>,
    },
    /// Exibe uma saudação
    #[command(hide = true)]
    Greet {
        /// Nome a saudar
        #[arg(default_value = "mundo")]
        name: String,
    },
}

/// Títulos da ajuda em português: o modelo (`Uso:`), o grupo dos subcomandos (`Comandos`) e o
/// das opções sem grupo próprio (`Opções`), em todos os subcomandos.
fn localized(cmd: clap::Command) -> clap::Command {
    const TEMPLATE: &str = "\
{before-help}{about-with-newline}
Uso: {usage}

{all-args}{after-help}";
    cmd.help_template(TEMPLATE)
        .subcommand_help_heading("Comandos")
        .subcommand_value_name("COMANDO")
        .mut_args(|arg| {
            if arg.get_help_heading().is_some() {
                arg
            } else if arg.is_positional() {
                arg.help_heading("Argumentos")
            } else {
                arg.help_heading("Opções")
            }
        })
        .mut_subcommands(localized)
}

fn cli_command() -> clap::Command {
    localized(Cli::command())
}

fn main() {
    let cli = cli_command()
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches))
        .unwrap_or_else(|err| usage_error(&err));
    if let Err(err) = run(cli) {
        eprintln!("Error: {err:?}");
        let help = help_for(std::env::args().skip(1));
        eprintln!(
            "\nPara ver como usar: {} --help",
            help.get_bin_name().unwrap_or("terraforme")
        );
        std::process::exit(1);
    }
}

/// Erro de uso (argumento inválido, faltando ou desconhecido): mostra o erro e, logo abaixo, a
/// ajuda do subcomando digitado. Pedidos de `--help`/`--version` seguem o fluxo normal do clap.
fn usage_error(err: &clap::Error) -> ! {
    use clap::error::ErrorKind;
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        err.exit();
    }
    // A mensagem do clap termina com `Usage:` e a dica do `--help`; a ajuda completa vem depois.
    let message = err.render().to_string();
    let message = message.split("\n\nUsage:").next().unwrap_or(&message);
    let mut stderr = io::stderr();
    let _ = writeln!(stderr, "{}\n", message.trim_end());
    let _ = help_for(std::env::args().skip(1)).write_help(&mut stderr);
    std::process::exit(err.exit_code());
}

/// O subcomando mais interno citado nos argumentos (ou o comando raiz), já com o nome completo
/// (`terraforme space`) para o `Usage:` da ajuda.
fn plugin_command(action: PluginAction) -> Result<()> {
    match action {
        PluginAction::List => {
            for (manifest, path) in plugin::list()? {
                let state = match (&path, manifest.supported()) {
                    (Some(path), _) => format!("✔ {}", path.display()),
                    (None, true) => "não instalado".into(),
                    (None, false) => format!("não roda em {}", std::env::consts::OS),
                };
                println!("  {:<12} {state}", manifest.name);
                println!("  {:<12} {} ({})", "", manifest.description, manifest.repo);
            }
        }
        PluginAction::Install { name } | PluginAction::Update { name } => {
            let out = plugin::install(&name)?;
            println!(
                "✔ Plugin {} instalado em {} ({})",
                out.manifest.name,
                out.path.display(),
                out.source
            );
            if out.manifest.bin == pet::BIN && pet::running().is_some() {
                println!("  Reabra a janela para usar a versão nova: terraforme restart");
            } else if !out.manifest.enabled(&Config::load()?) {
                // Instalar não liga: o mascote continua opcional.
                println!(
                    "  Desligado: abra com `terraforme pet` ou ligue com `terraforme config --pet true`"
                );
            }
        }
        PluginAction::Sync => {
            for synced in plugin::sync()? {
                match synced {
                    plugin::Synced::Updated(out) => {
                        println!("✔ Plugin {} atualizado ({})", out.manifest.name, out.source)
                    }
                    plugin::Synced::Disabled(m) => println!(
                        "  Plugin {} desligado: não atualizado (ligue com `terraforme config --{} true`)",
                        m.name, m.name
                    ),
                    plugin::Synced::Failed(m, e) => eprintln!(
                        "! Falha ao atualizar o plugin {} — rode `terraforme plugin update {}`: {e:#}",
                        m.name, m.name
                    ),
                }
            }
        }
        PluginAction::Enabled { name } => {
            let manifest = plugin::Manifest::resolve(&name)?;
            if !manifest.enabled(&Config::load()?) {
                std::process::exit(1);
            }
        }
        PluginAction::Remove { name } => {
            if name == "pet" {
                pet::stop()?;
            }
            match plugin::remove(&name)? {
                Some(path) => println!("✔ Plugin {name} removido ({})", path.display()),
                None => println!("O plugin {name} não estava instalado pelo terraforme"),
            }
        }
    }
    Ok(())
}

fn help_for(args: impl IntoIterator<Item = String>) -> clap::Command {
    let mut cmd = cli_command();
    cmd.build();
    for arg in args {
        if arg == "--" {
            break;
        }
        if let Some(sub) = cmd.find_subcommand(&arg).cloned() {
            cmd = sub;
        }
    }
    cmd
}

fn run(cli: Cli) -> Result<()> {
    if cli.verbose {
        eprintln!("[debug] {cli:?}");
    }

    if cli.historico {
        return print_history();
    }
    let Some(command) = cli.command else {
        cli_command().print_help()?;
        return Ok(());
    };

    match command {
        Command::Version => println!("terraforme {}", terraforme_core::VERSION),
        Command::Pet => {
            if pet::start()? {
                println!("✔ Janela flutuante aberta");
            } else {
                println!("A janela flutuante já está aberta");
            }
        }
        Command::Plugin { action } => plugin_command(action)?,
        Command::Status => print_status(),
        Command::Setup => setup_flow()?,
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
                config(
                    None,
                    None,
                    Some(on),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )?;
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
            cli,
            mode,
            jail,
            budget_five_hours,
            budget_week,
            mcp_ui,
            pet,
            account,
            remove_account,
        } => config(
            agent,
            mode,
            jail,
            cli,
            budget_five_hours,
            budget_week,
            mcp_ui,
            pet,
            account,
            remove_account,
        )?,
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
            rename,
            list,
            path,
            agent,
            mode,
            jail,
            account,
            reset,
            yes,
        } => {
            let has_pref =
                agent.is_some() || mode.is_some() || jail.is_some() || account.is_some() || reset;
            let prefs = has_pref.then_some(Preferences {
                agent,
                mode,
                jail,
                account: account.clone(),
            });
            space(
                name.as_deref(),
                save,
                delete,
                rename.as_deref(),
                list,
                path,
                prefs,
                reset,
                yes,
            )?
        }
        Command::Upgrade {
            dev,
            repo,
            dist,
            no_pull,
            restart,
        } => upgrade(dev, repo, dist, no_pull, restart)?,
        Command::Restart => restart()?,
        Command::Help { command } => {
            let mut cmd = help_for(command);
            cmd.print_long_help()?;
        }
        Command::Greet { name } => println!("{}", terraforme_core::greeting(&name)),
        Command::Completions { shell } => {
            let mut cmd = cli_command();
            let bin = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin, &mut io::stdout());
        }
    }

    Ok(())
}

/// Valores do `config --mode`: os modos e `custom`, que pergunta quais
/// extensões usar (RTK, AI Memory, AI Jail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeChoice {
    None,
    Eco,
    High,
    Hybrid,
    Custom,
}

#[allow(clippy::too_many_arguments)]
fn config(
    agent: Option<Option<Provider>>,
    mode: Option<ModeChoice>,
    jail: Option<bool>,
    cli: Option<Option<CliMode>>,
    budget_five_hours: Option<u64>,
    budget_week: Option<u64>,
    mcp_ui: Option<bool>,
    pet: Option<bool>,
    account: Option<Option<String>>,
    remove_account: Option<String>,
) -> Result<()> {
    // Gerenciamento de contas Claude (independente do restante do config).
    if account.is_some() || remove_account.is_some() {
        return config_accounts(account, remove_account);
    }
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
    let cli = match cli {
        // Sem valor: lista os modos de início e, num terminal, pergunta qual
        // vira o padrão.
        Some(None) => {
            print_cli_modes(config.mode, config.jail);
            if !io::stdin().is_terminal() {
                return Ok(());
            }
            let current = CliMode::of(config.mode, config.jail);
            let answer = ask(&format!(
                "Modo de início (1-{} ou nome; Enter mantém {}): ",
                CliMode::ALL.len(),
                current.id()
            ))?;
            if answer.is_empty() {
                return Ok(());
            }
            let Some(cli) = CliMode::pick(&answer) else {
                bail!("modo de início {answer} não encontrado");
            };
            Some(cli)
        }
        Some(Some(cli)) => Some(cli),
        None => None,
    };
    if let Some(cli) = cli {
        match cli.preset() {
            Some((mode, jail)) => {
                config.mode = mode;
                config.jail = jail;
            }
            None => {
                config.mode = custom_cli_preset()?;
            }
        }
        changed = true;
    }
    if let Some(choice) = mode {
        config.mode = match choice {
            ModeChoice::None => Mode::None,
            ModeChoice::Eco => Mode::Eco,
            ModeChoice::High => Mode::High,
            ModeChoice::Hybrid => Mode::Hybrid,
            ModeChoice::Custom => custom_cli_preset()?,
        };
        changed = true;
    }
    if let Some(jail) = jail {
        config.jail = jail;
        changed = true;
    }
    if let Some(ui) = mcp_ui {
        config.mcp.ui = ui;
        changed = true;
    }
    if let Some(pet) = pet {
        config.tui.pet = pet;
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
    let inicio = CliMode::of(config.mode, config.jail);
    println!("  início    {} ({})", inicio.id(), mode_label(config.mode));
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
    let ui = match mcp::LockFile::read() {
        Ok(Some(lock)) if config.mcp.ui => format!("ligada · {}", mcp::ui_url(&lock)),
        _ if config.mcp.ui => "ligada (abre com o terraforme cli)".into(),
        _ => "desligada (liga com `terraforme config --mcp-ui true`)".into(),
    };
    println!("  UI web    {ui}");
    let pet = if !pet::is_supported() {
        "só no Linux e no Windows por enquanto (veja o ROADMAP.md)"
    } else if plugin::locate(pet::BIN).is_none() {
        "não instalada (instale com `terraforme plugin install pet`)"
    } else if config.tui.pet {
        "abre com o terraforme cli"
    } else {
        "desligada (liga com `terraforme config --pet true`)"
    };
    println!("  mascote   {pet}");
    Ok(())
}

/// Gerencia as contas do Claude Code: lista, adiciona ou remove.
fn config_accounts(account: Option<Option<String>>, remove_account: Option<String>) -> Result<()> {
    let mut accounts = ClaudeAccounts::load()?;

    if let Some(name) = remove_account {
        match accounts.remove(&name) {
            Some(dir) => {
                accounts.save()?;
                println!("✔ conta '{name}' removida (era: {})", dir.display());
            }
            None => bail!("conta '{name}' não encontrada"),
        }
        print_accounts(&accounts);
        return Ok(());
    }

    match account {
        // --account sem valor: lista as contas registradas.
        Some(None) => {
            print_accounts(&accounts);
            return Ok(());
        }
        // --account NOME=PASTA: adiciona/atualiza.
        Some(Some(spec)) => {
            let (name, raw_dir) = spec.split_once('=').ok_or_else(|| {
                anyhow::anyhow!("formato esperado: NOME=PASTA (ex.: work=/home/user/.claude-work)")
            })?;
            let dir = std::path::Path::new(raw_dir);
            if !dir.is_dir() {
                bail!(
                    "a pasta '{}' não existe — crie-a antes ou rode `claude` com CLAUDE_CONFIG_DIR='{}'",
                    dir.display(),
                    dir.display()
                );
            }
            let old = accounts.insert(name, dir)?;
            accounts.save()?;
            match old {
                Some(old) => println!(
                    "✔ conta '{name}' atualizada: {} → {}",
                    old.display(),
                    dir.display()
                ),
                None => println!("✔ conta '{name}' registrada: {}", dir.display()),
            }
        }
        None => {}
    }

    print_accounts(&accounts);
    println!("{}", ClaudeAccounts::path()?.display());
    Ok(())
}

fn print_accounts(accounts: &ClaudeAccounts) {
    if accounts.is_empty() {
        println!("Nenhuma conta do Claude Code registrada.");
        println!("  Use `terraforme config --account NOME=PASTA` para registrar uma.");
        return;
    }
    println!("Contas do Claude Code");
    for (i, (name, dir)) in accounts.iter().enumerate() {
        let ok = if dir.is_dir() { "✔" } else { "✘" };
        println!("  {:>2}  {ok}  {name:<16}  {}", i + 1, dir.display());
    }
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
    if names.is_empty() {
        return "sem extensões".into();
    }
    names.join(" + ")
}

fn print_cli_modes(mode: Mode, jail: bool) {
    let current = CliMode::of(mode, jail);
    println!("Modos de início do `terraforme cli`");
    for (i, cli) in CliMode::ALL.into_iter().enumerate() {
        let marker = if cli == current { "●" } else { "○" };
        println!("  {} {marker} {:<9} {}", i + 1, cli.id(), cli.label());
    }
}

/// `--mode custom` / `--cli custom`: pergunta quais extensões do modo (RTK e/ou
/// AI Memory) e devolve o `Mode` derivado. O AI Jail é ortogonal ao modo e é
/// controlado com `--jail` / `config.jail`.
fn custom_cli_preset() -> Result<Mode> {
    if !io::stdin().is_terminal() {
        bail!("`terraforme config --mode custom` precisa de um terminal interativo");
    }
    let extensions = [Extension::Rtk, Extension::AiMemory];
    let labels: Vec<_> = extensions.iter().map(|ext| ext.name()).collect();
    let Some(indices) = ask_selection(
        "Extensões do modo customizado:",
        &labels,
        "ex.: 1 2; Enter: nenhuma; q: cancelar",
    )?
    else {
        bail!("seleção cancelada; nada foi alterado");
    };
    Ok(custom_preset(indices.contains(&0), indices.contains(&1)))
}

fn custom_preset(rtk: bool, ai_memory: bool) -> Mode {
    match (rtk, ai_memory) {
        (true, true) => Mode::Hybrid,
        (true, false) => Mode::Eco,
        (false, true) => Mode::High,
        (false, false) => Mode::None,
    }
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
#[derive(Clone, Copy)]
struct Choice {
    agent: Option<Provider>,
    mode: Option<Mode>,
    jail: Option<bool>,
}

/// Sessão resolvida: a linha de comando vence as preferências do space, que
/// vencem o `.ai-jail` do projeto, que vence o config.toml.
struct Resolved {
    provider: Provider,
    mode: Mode,
    jail: bool,
    launch: agent::Launch,
}

impl Choice {
    fn resolve(self, args: &[String]) -> Result<Resolved> {
        let config = Config::load()?;
        let cwd = std::env::current_dir().ok();
        let space = match &cwd {
            Some(dir) => Preferences::find(dir)?,
            None => None,
        };
        let prefs = space.as_ref().map(|(_, p)| p.clone()).unwrap_or_default();
        let provider = self.agent.or(prefs.agent).unwrap_or(config.provider);
        let mode = self.mode.or(prefs.mode).unwrap_or(config.mode);
        let project = cwd.as_deref().and_then(jail::project_config);
        let jail = self
            .jail
            .or(prefs.jail)
            .unwrap_or(config.jail || project.is_some());
        // Sem AI Jail no Windows: o `.ai-jail` de um projeto versionado, o
        // config ou o space não podem impedir a sessão; só o `--jail` pedido
        // agora vira erro.
        let mut prepared = Vec::new();
        let jail = if jail && !jail::is_supported() {
            if self.jail == Some(true) {
                bail!("o AI Jail não roda no Windows — use-o dentro do WSL2 ou rode sem --jail");
            }
            prepared
                .push("AI Jail ignorado: ele não roda no Windows (use-o dentro do WSL2)".into());
            false
        } else {
            jail
        };

        // Conta do Claude Code: define CLAUDE_CONFIG_DIR antes de prepare()
        // para que Claude::setup() use a pasta certa (hooks, credenciais, MCP).
        // SAFETY: fora da TUI a CLI é single-threaded e está prestes a exec o
        // agente; na TUI (aba Config troca agente/modo) as outras threads são
        // só Rust, que lê o ambiente com o mesmo lock do `set_var`.
        if provider == Provider::Claude
            && let Some(name) = prefs.account.as_deref()
        {
            let accounts = ClaudeAccounts::load()?;
            match accounts.get(name) {
                // A conta da pasta padrão (`~/.claude`) roda sem a variável.
                Some(dir) if is_default_dir(dir) => unsafe {
                    std::env::remove_var("CLAUDE_CONFIG_DIR")
                },
                Some(dir) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) },
                None => bail!(
                    "conta '{name}' não encontrada — registre com \
                     `terraforme config --account {name}=<pasta>`"
                ),
            }
        }

        // Extensões que precisam de algo no projeto (ex.: o `.ai-jail`) são
        // preparadas aqui, só na primeira vez em cada pasta.
        if let Some(dir) = &cwd
            && let Some(adapter) = provider.adapter()
        {
            let mut extensions = mode.extensions();
            if jail {
                extensions.push(Extension::AiJail);
            }
            for ext in extensions {
                prepared.extend(ext.setup_project(dir, adapter.bin())?);
            }
        }
        let mut launch = agent::prepare(provider, mode, jail, args)?;
        launch.notices.extend(prepared);
        // Histórico das pastas (`terraforme --historico`); falhar não impede a sessão.
        if let Some(dir) = &cwd
            && let Err(err) = History::open().and_then(|db| db.record_launch(dir, provider.id()))
        {
            launch
                .warnings
                .push(format!("histórico de pastas não gravado: {err:#}"));
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
    // O daemon sobe antes do comando do agente: com o AI Jail, a porta dele
    // entra no `--forward-port`.
    let config = Config::load()?;
    let mcp_lock = match mcp::ensure_running(&config.mcp) {
        Ok(lock) => Some(lock),
        Err(e) => {
            eprintln!("aviso: terraforme-mcp indisponível: {e}");
            None
        }
    };
    let client_id = uuid::Uuid::new_v4().to_string();
    // A janela flutuante lê o estado das instâncias no daemon.
    let pet_warning = (config.tui.pet && pet::is_supported())
        .then(|| pet::start().err())
        .flatten()
        .map(|e| format!("janela flutuante: {e:#}"));

    let mut launched = resolve_launch(choice, args)?;
    launched
        .notices
        .extend(pet_warning.map(|w| format!("⚠ {w}")));
    let space = std::env::current_dir()
        .ok()
        .and_then(|cwd| Preferences::find(&cwd).ok().flatten())
        .map(|(name, _)| name);

    // A aba Config troca agente e modo: o que ela grava vence as flags.
    let args = args.to_vec();
    term::run(term::Options {
        launched,
        relaunch: Box::new(move |provider, mode, jail| {
            let choice = Choice {
                agent: Some(provider),
                mode: Some(mode),
                jail: Some(jail),
            };
            resolve_launch(choice, &args)
        }),
        instances,
        mcp_lock,
        client_id,
        space,
    })
}

/// Resolve a escolha no que a TUI precisa para abrir as instâncias.
fn resolve_launch(choice: Choice, args: &[String]) -> Result<term::Launched> {
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
    let mut extensions = mode.extensions();
    if jail {
        extensions.push(Extension::AiJail);
    }
    Ok(term::Launched {
        agent: provider.id().to_string(),
        title: launch_title(provider, mode, jail),
        adapter,
        command: launch.command,
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
    // Sem `exec`, o terraforme espera o filho: o Ctrl+C é dele.
    terraforme_core::platform::ignore_ctrl_c();
    let status = command
        .status()
        .with_context(|| format!("falha ao iniciar {}", command.get_program().display()))?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Onde ficam os scripts que instalam as releases do GitHub (sem Rust).
const RELEASE_SCRIPTS: &str = "https://raw.githubusercontent.com/Augustowtg/terraforme/main";

/// Reinicia o daemon MCP e, se estava aberta, a janela flutuante.
fn restart() -> Result<()> {
    let config = Config::load()?;
    let (stopped, lock) = mcp::restart(&config.mcp)?;
    let old = match stopped {
        0 => String::new(),
        1 => " (1 antigo encerrado)".to_string(),
        n => format!(" ({n} antigos encerrados)"),
    };
    println!("✔ terraforme-mcp no ar na porta {}{old}", lock.port);
    if pet::running().is_some() {
        // `stop` espera a janela antiga sair.
        pet::stop()?;
        pet::start()?;
        println!("✔ Janela flutuante reaberta");
    }
    println!(
        "  Os terraforme cli abertos se reconectam sozinhos; reinicie os agentes neles (⌃⌥R)\n  para voltarem a usar o MCP. Terminais de antes da 0.22.2: feche e abra de novo."
    );
    Ok(())
}

/// Atualiza o terraforme. Por padrão instala a última release do GitHub
/// (`install.sh` / `install.ps1`), venha o binário de uma release ou de um
/// clone. Com `dev`, roda o script de instalação do repositório
/// (`install.linux.sh`, ou `install.windows.ps1` no Windows: git pull +
/// compila + reinstala). Os dois rodam no lugar do terraforme (`exec`), para
/// que o binário em uso possa ser substituído.
fn upgrade(
    dev: bool,
    repo: Option<std::path::PathBuf>,
    dist: bool,
    no_pull: bool,
    restart: bool,
) -> Result<()> {
    let windows = cfg!(windows);
    if !windows && !cfg!(target_os = "linux") {
        bail!(
            "o upgrade automático só existe no Linux e no Windows; recompile com `cargo build --release`"
        );
    }
    if !dev {
        return upgrade_release(windows, restart);
    }
    let repo =
        repo.unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let script = repo.join(if windows {
        "install.windows.ps1"
    } else {
        "install.linux.sh"
    });
    if !script.is_file() {
        bail!(
            "{} não encontrado; informe o repositório com --repo ou TERRAFORME_REPO (sem --dev, o upgrade usa a última release)",
            script.display()
        );
    }

    let mut command;
    if windows {
        command = std::process::Command::new("powershell");
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
    } else {
        command = std::process::Command::new("bash");
    }
    command.arg(&script);
    if !no_pull {
        command.arg(if windows { "-Update" } else { "--update" });
    }
    if dist {
        command.arg(if windows { "-Dist" } else { "--dist" });
    }
    set_prefix(&mut command, restart);
    exec(command)
}

/// Atualiza pela última release do GitHub, com o script de instalação dela.
fn upgrade_release(windows: bool, restart: bool) -> Result<()> {
    let mut command;
    if windows {
        command = std::process::Command::new("powershell");
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"]);
        command.arg(format!("irm {RELEASE_SCRIPTS}/install.ps1 | iex"));
    } else {
        command = std::process::Command::new("sh");
        command.arg("-c");
        command.arg(format!("curl -fsSL {RELEASE_SCRIPTS}/install.sh | sh"));
    }
    set_prefix(&mut command, restart);
    exec(command)
}

/// Reinstala no mesmo prefixo do binário atual (`<prefixo>/bin/terraforme`).
/// Os scripts de instalação perguntam se reiniciam o terraforme no fim; com
/// `restart` (`TERRAFORME_RESTART=1`), reiniciam sem perguntar.
fn set_prefix(command: &mut std::process::Command, restart: bool) {
    if restart {
        command.env("TERRAFORME_RESTART", "1");
    }
    if std::env::var_os("PREFIX").is_none()
        && let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
        && bin.file_name().is_some_and(|n| n == "bin")
        && let Some(prefix) = bin.parent()
    {
        command.env("PREFIX", prefix);
    }
}

#[allow(clippy::too_many_arguments)]
fn space(
    name: Option<&str>,
    save: bool,
    delete: bool,
    rename: Option<&str>,
    list: bool,
    path: bool,
    prefs: Option<Preferences>,
    reset: bool,
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
        if let Some(prefs) = prefs {
            space_preferences(name, prefs, reset)?;
        }
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
            Preferences::default().save(name)?;
            println!("✔ space {name} removido");
        }
        return Ok(());
    }

    if let Some(new_name) = rename {
        let old_name = name.unwrap_or_default();
        match spaces.rename(old_name, new_name)? {
            None => bail!("space {old_name} não encontrado"),
            Some(dir) => {
                spaces.save()?;
                Preferences::rename(old_name, new_name)?;
                println!(
                    "✔ space {old_name} renomeado para {new_name}: {}",
                    dir.display()
                );
            }
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
    if let Some(prefs) = prefs {
        return space_preferences(name, prefs, reset);
    }
    if path {
        println!("{}", dir.display());
        return Ok(());
    }
    let summary = preferences_summary(&Preferences::load(name)?)
        .map(|s| format!(" · {s}"))
        .unwrap_or_default();
    eprintln!(
        "==> space {name} · {}{summary} (`exit` volta)",
        dir.display()
    );
    exec(space::shell(name, dir))
}

/// Grava as preferências do space: `reset` apaga as anteriores, o resto
/// substitui só o que foi passado.
fn space_preferences(name: &str, new: Preferences, reset: bool) -> Result<()> {
    if let Some(p) = new.agent
        && !p.is_implemented()
    {
        bail!("o agente {} ainda não foi implementado", p.name());
    }
    if let Some(account_name) = new.account.as_deref() {
        let accounts = ClaudeAccounts::load()?;
        if accounts.get(account_name).is_none() {
            bail!(
                "conta '{account_name}' não encontrada — registre com \
                 `terraforme config --account {account_name}=<pasta>`"
            );
        }
    }
    let mut prefs = if reset {
        Preferences::default()
    } else {
        Preferences::load(name)?
    };
    prefs.agent = new.agent.or(prefs.agent);
    prefs.mode = new.mode.or(prefs.mode);
    prefs.jail = new.jail.or(prefs.jail);
    prefs.account = new.account.or(prefs.account);
    prefs.save(name)?;
    match preferences_summary(&prefs) {
        Some(summary) => println!(
            "✔ preferências do space {name}: {summary}\n{}",
            Preferences::path(name)?.display()
        ),
        None => println!("✔ space {name} sem preferências (usa o `terraforme config`)"),
    }
    Ok(())
}

/// `agente claude · modo eco · AI Jail · conta work`; `None` sem preferências.
fn preferences_summary(prefs: &Preferences) -> Option<String> {
    let parts: Vec<String> = [
        prefs.agent.map(|p| format!("agente {}", p.id())),
        prefs.mode.map(|m| format!("modo {}", m.id())),
        prefs
            .jail
            .map(|j| if j { "AI Jail" } else { "sem AI Jail" }.into()),
        prefs.account.as_deref().map(|a| format!("conta {a}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn print_spaces(spaces: &Spaces, out: &mut impl Write) -> Result<()> {
    let width = spaces
        .iter()
        .map(|(n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    writeln!(out, "Spaces")?;
    for (i, (name, dir)) in spaces.iter().enumerate() {
        let extra = if !dir.is_dir() {
            "  ✘ pasta não existe".to_string()
        } else {
            Preferences::load(name)
                .ok()
                .and_then(|p| preferences_summary(&p))
                .map(|s| format!("  ({s})"))
                .unwrap_or_default()
        };
        writeln!(
            out,
            "  {:>2}  {name:<width$}  {}{extra}",
            i + 1,
            dir.display()
        )?;
    }
    Ok(())
}

/// Sem nenhum agente na máquina: o usuário marca quais instalar (Espaço) e
/// o terraforme roda o instalador oficial de cada um.
fn install_agents() -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("nenhum agente suportado encontrado no PATH — instale o Claude Code ou o OpenCode");
    }
    let providers: Vec<Provider> = Provider::ALL
        .into_iter()
        .filter(|p| p.adapter().is_some())
        .collect();
    let items: Vec<select::Item> = providers
        .iter()
        .map(|p| select::Item {
            label: p.name().to_string(),
            disabled: p
                .installer()
                .is_none()
                .then(|| format!("instale manualmente: {}", p.url())),
        })
        .collect();
    let checked: Vec<bool> = providers.iter().map(|&p| p == Provider::Claude).collect();
    println!();
    let Some(selected) = select::checkboxes(
        "Nenhum agente encontrado. Quais instalar nesta máquina?",
        &items,
        &checked,
    )?
    else {
        bail!("instalação cancelada; nada foi alterado");
    };
    if selected.is_empty() {
        bail!("nenhum agente selecionado — instale o Claude Code ou o OpenCode e rode de novo");
    }
    for provider in selected.into_iter().map(|i| providers[i]) {
        println!("==> instalando {}", provider.name());
        match provider.install() {
            Ok(()) => println!("✔ {} instalado", provider.name()),
            Err(err) => eprintln!("✘ {err:#}"),
        }
        // O instalador põe o binário numa pasta que este processo ainda não
        // tem no PATH (o shell só a lê num terminal novo).
        let current = std::env::var_os("PATH").unwrap_or_default();
        let dirs = provider
            .install_dirs()
            .into_iter()
            .filter(|d| d.is_dir())
            .chain(std::env::split_paths(&current));
        if let Ok(path) = std::env::join_paths(dirs) {
            // Single-thread neste ponto do setup.
            unsafe { std::env::set_var("PATH", path) };
        }
    }
    println!();
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

/// `terraforme setup`: passo a passo, identifica os agentes da máquina,
/// instala e configura as extensões neles e instala as skills embutidas.
fn setup_flow() -> Result<()> {
    println!("Passo 1/3 — Agentes encontrados na máquina");
    for provider in Provider::ALL.into_iter().filter(|p| p.adapter().is_some()) {
        match provider.check().state {
            State::Installed { path, version } => {
                let version = version.map(|v| format!(" v{v}")).unwrap_or_default();
                println!("  ✔ {}{version} ({})", provider.name(), path.display());
            }
            _ => println!("  ✘ {} não instalado — {}", provider.name(), provider.url()),
        }
    }
    let installed = || -> Vec<Provider> {
        Provider::ALL
            .into_iter()
            .filter(|p| p.adapter().is_some() && p.check().state.is_installed())
            .collect()
    };
    let mut agents = installed();
    if agents.is_empty() {
        install_agents()?;
        agents = installed();
        if agents.is_empty() {
            bail!(
                "nenhum agente encontrado no PATH depois da instalação — \
                 abra um novo terminal e rode `terraforme setup` de novo"
            );
        }
    }

    let mut failed = 0usize;

    println!("\nPasso 2/3 — Extensões");
    for ext in Extension::ALL {
        if ext.check().state.is_installed() {
            continue;
        }
        println!("==> instalando {}", ext.name());
        match ext.install() {
            Ok(outcome) => report(ext, outcome),
            Err(err) => {
                eprintln!("✘ {}: {err:#}", ext.name());
                failed += 1;
            }
        }
    }
    let installed: Vec<Extension> = Extension::ALL
        .into_iter()
        .filter(|ext| ext.check().state.is_installed())
        .collect();
    if installed.is_empty() {
        eprintln!("⚠ nenhuma extensão instalada — nada a configurar");
    } else if let Err(err) = setup_selected(&agents, &installed) {
        eprintln!("⚠ {err:#}");
        failed += 1;
    }

    println!("\nPasso 3/3 — Skills");
    if let Err(err) = install_skills(&agents) {
        eprintln!("✘ skills: {err:#}");
        failed += 1;
    }

    if failed > 0 {
        bail!("{failed} etapa(s) do setup falharam; veja os erros acima");
    }
    println!("\n✔ Setup concluído");
    Ok(())
}

/// Instala as skills embutidas na pasta global de skills de cada agente.
fn install_skills(agents: &[Provider]) -> Result<()> {
    let names = skill::names();
    if names.is_empty() {
        println!("  nenhuma skill embutida");
        return Ok(());
    }
    let mut failures = 0usize;
    for &provider in agents {
        let Some(dir) = provider.adapter().and_then(|adapter| adapter.skills_dir()) else {
            eprintln!("  ⚠ {} não tem pasta de skills conhecida", provider.name());
            continue;
        };
        for name in &names {
            match skill::install(name, &dir) {
                Ok(path) => println!(
                    "  ✔ {} · skill {name} → {}",
                    provider.name(),
                    path.display()
                ),
                Err(err) => {
                    eprintln!("  ✘ {} · skill {name}: {err:#}", provider.name());
                    failures += 1;
                }
            }
        }
    }
    if failures > 0 {
        bail!("{failures} skill(s) não foram instaladas");
    }
    Ok(())
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

    println!("\nPastas");
    let dirs = [
        ("local (spaces e skills)", space::root()),
        ("configuração", Config::dir()),
        (
            "dados (daemon MCP)",
            mcp::LockFile::path().map(|p| p.parent().map(Into::into).unwrap_or(p)),
        ),
    ];
    for (label, dir) in dirs {
        match dir {
            Ok(dir) if dir.is_dir() => println!("  ✔ {label}  ({})", dir.display()),
            Ok(dir) => println!("  ✘ {label}  ({}) — ainda não criada", dir.display()),
            Err(err) => println!("  ⚠ {label} — {err:#}"),
        }
    }

    if !status.all_installed() {
        println!("\nAlgum componente está ausente — veja os links acima para instalar.");
    }
}

/// Pastas recentes listadas em `terraforme --historico`, fora dos spaces.
const RECENT_LIMIT: usize = 10;

/// `terraforme --historico`: as pastas em que o `terraforme cli` foi aberto,
/// primeiro os spaces (a abertura mais recente em cada um) e depois as
/// pastas fora de space.
fn print_history() -> Result<()> {
    let launches = History::open()?.launches(500)?;
    let spaces = Spaces::load()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (in_space, recent) = split_history(&launches, |path| {
        spaces.name_containing(path).map(str::to_string)
    });
    let detail = |l: &Launch| format!("{} · {} · {}×", ago(now, l.last), l.agent, l.count);

    println!("Spaces");
    if in_space.is_empty() {
        println!("  nenhum space aberto ainda — salve um com `terraforme space --save`");
    }
    let width = in_space
        .iter()
        .map(|(n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    for (name, launch) in &in_space {
        println!(
            "  {name:<width$}  {}  ({})",
            launch.path.display(),
            detail(launch)
        );
    }

    println!("\nRecentes");
    if recent.is_empty() {
        println!("  nenhuma pasta fora de space");
    }
    for launch in recent.iter().take(RECENT_LIMIT) {
        println!("  {}  ({})", launch.path.display(), detail(launch));
    }
    println!("\nAbrir um space: terraforme space <nome>");
    Ok(())
}

/// Divide as aberturas (da mais nova para a mais antiga) entre os spaces —
/// uma por space, a mais recente — e as pastas fora de space.
fn split_history(
    launches: &[Launch],
    space_of: impl Fn(&std::path::Path) -> Option<String>,
) -> (Vec<(String, &Launch)>, Vec<&Launch>) {
    let mut spaces: Vec<(String, &Launch)> = Vec::new();
    let mut recent = Vec::new();
    for launch in launches {
        match space_of(&launch.path) {
            Some(name) if spaces.iter().all(|(n, _)| *n != name) => spaces.push((name, launch)),
            Some(_) => {}
            None => recent.push(launch),
        }
    }
    (spaces, recent)
}

/// Tempo desde `then`, em português curto (`há 5 min`).
fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        s if s < 60 => "agora".into(),
        s if s < 3600 => format!("há {} min", s / 60),
        s if s < 86400 => format!("há {} h", s / 3600),
        s if s < 2 * 86400 => "ontem".into(),
        s => format!("há {} dias", s / 86400),
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
        cli_command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from([&["terraforme"], args].concat())
    }

    #[test]
    fn help_for_picks_the_typed_subcommand() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(help_for(args(&[])).get_name(), "terraforme");
        let space = help_for(args(&["space", "--bogus"]));
        assert_eq!(space.get_bin_name(), Some("terraforme space"));
        assert_eq!(help_for(args(&["cli", "--", "space"])).get_name(), "cli");
    }

    #[test]
    fn history_flag_is_accepted() {
        assert!(parse(&["--historico"]).unwrap().historico);
        assert!(parse(&["--history"]).unwrap().historico);
    }

    #[test]
    fn history_splits_spaces_and_recent_paths() {
        let launch = |path: &str, last| Launch {
            path: std::path::PathBuf::from(path),
            agent: "claude".into(),
            count: 1,
            last,
        };
        let launches = [
            launch("/w/web/front", 50),
            launch("/tmp/x", 40),
            launch("/w/web", 30),
            launch("/w/api", 20),
        ];
        let (spaces, recent) = split_history(&launches, |path| {
            ["web", "api"]
                .into_iter()
                .find(|name| path.starts_with(format!("/w/{name}")))
                .map(String::from)
        });
        let spaces: Vec<_> = spaces.iter().map(|(n, l)| (n.as_str(), l.last)).collect();
        assert_eq!(spaces, [("web", 50), ("api", 20)]);
        assert_eq!(recent.iter().map(|l| l.last).collect::<Vec<_>>(), [40]);
    }

    #[test]
    fn ago_is_short_portuguese() {
        assert_eq!(ago(100, 90), "agora");
        assert_eq!(ago(1000, 700), "há 5 min");
        assert_eq!(ago(10_000, 2_800), "há 2 h");
        assert_eq!(ago(100_000, 0), "ontem");
        assert_eq!(ago(900_000, 0), "há 10 dias");
    }

    #[test]
    fn setup_subcommand_is_accepted() {
        assert!(matches!(
            parse(&["setup"]).unwrap().command,
            Some(Command::Setup)
        ));
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
    fn upgrade_build_flags_need_dev() {
        assert!(parse(&["upgrade"]).is_ok());
        assert!(parse(&["upgrade", "--restart"]).is_ok());
        assert!(parse(&["upgrade", "--dev", "--dist", "--no-pull"]).is_ok());
        for flag in ["--dist", "--no-pull"] {
            let err = parse(&["upgrade", flag]).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
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
        assert!(parse(&["config", "--mode", "custom"]).is_ok());
        assert!(parse(&["config", "--mode", "turbo"]).is_err());
        assert!(parse(&["config", "--budget-5h", "1000", "--budget-week", "0"]).is_ok());
        assert!(parse(&["config", "--budget-5h", "-1"]).is_err());
    }

    #[test]
    fn config_cli_modes() {
        let cli = parse(&["config", "--cli"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cli: Some(None),
                ..
            })
        ));
        for (name, expected) in [
            ("eco", CliMode::Eco),
            ("custom", CliMode::Custom),
            ("security", CliMode::Security),
            ("high", CliMode::High),
        ] {
            let cli = parse(&["config", "--cli", name]).unwrap();
            assert!(
                matches!(
                    cli.command,
                    Some(Command::Config {
                        cli: Some(Some(mode)),
                        ..
                    }) if mode == expected
                ),
                "esperava {expected:?}",
            );
        }
        assert!(parse(&["config", "--cli", "turbo"]).is_err());
        assert!(parse(&["config", "--cli", "eco", "--mode", "high"]).is_err());
        assert!(parse(&["config", "--cli", "eco", "--jail", "true"]).is_err());
    }

    #[test]
    fn custom_preset_maps_selected_extensions() {
        assert_eq!(custom_preset(true, false), Mode::Eco);
        assert_eq!(custom_preset(false, true), Mode::High);
        assert_eq!(custom_preset(true, true), Mode::Hybrid);
        assert_eq!(custom_preset(false, false), Mode::None);
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
        assert!(parse(&["space", "api", "--agent", "opencode", "--mode", "eco"]).is_ok());
        assert!(parse(&["space", "--save", "--jail", "true"]).is_ok());
        assert!(parse(&["space", "api", "--reset"]).is_ok());
        assert!(parse(&["space", "--list", "--mode", "eco"]).is_err());
        assert!(parse(&["space", "api", "--path", "--jail", "false"]).is_err());
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
