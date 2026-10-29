//! Definição dos argumentos (clap) e execução dos comandos.

use std::env;
use std::fs;
use std::io::{IsTerminal, stdout};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::db::{self, Db, SaveOutcome};
use crate::hub;
use crate::orchestrator;
use crate::settings;
use crate::theme;
use crate::tui;

#[derive(Parser)]
#[command(
    name = "terraforme",
    bin_name = "terra",
    version,
    about = "Navegue entre seus workspaces pelo terminal"
)]
struct Cli {
    /// Arquivo onde o path escolhido é gravado; usado pela função de shell `terra`.
    #[arg(long, global = true, value_name = "ARQUIVO", hide = true)]
    cwd_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Abre a interface e registra o diretório atual nos recentes
    Cli,
    /// Lista os workspaces recentes e salvos e entra no escolhido
    Workspace(WorkspaceArgs),
    /// Atualiza o terra pelo instalador do GitHub (a última release)
    Upgrade(UpgradeArgs),
    /// Mostra os terminais do terra abertos, por workspace (pelo orquestrador)
    Status(StatusArgs),
    /// Manda uma mensagem a outro agente (pelo orquestrador)
    Send(SendArgs),
    /// Notas de handoff entre agentes
    #[command(subcommand)]
    Handoff(HandoffCommand),

    /// O daemon orquestrador: um por usuário, iniciado pelo primeiro terra e
    /// fechado com o último (uso interno)
    #[command(name = "__hub", hide = true)]
    Hub,

    /// O gancho de permissão do Antigravity (`agy`): o `agy` o roda antes de
    /// cada ferramenta e ele leva o pedido ao usuário (uso interno)
    #[command(hide = true)]
    AgyPermissionHook(AgyHookArgs),
}

#[derive(Args)]
struct AgyHookArgs {
    /// O arquivo do canal do chat do terra, onde o usuário decide
    #[arg(long, value_name = "ARQUIVO")]
    channel: PathBuf,

    /// Edições de arquivo passam sem perguntar (modo Accept Edits)
    #[arg(long)]
    accept_edits: bool,
}

#[derive(Args)]
struct SendArgs {
    /// O agente de destino: o nome de uma aba (`/agente`) ou de um
    /// `[agents.<nome>]` do orchestrator.toml
    agent: String,
    /// O texto da mensagem (nível 1)
    text: Vec<String>,
    /// Um arquivo para o agente ler (nível 2); pode repetir
    #[arg(long = "ref", value_name = "ARQUIVO")]
    refs: Vec<PathBuf>,
    /// A nota de handoff (nível 3), criada com `terra handoff new`
    #[arg(long, value_name = "ARQUIVO")]
    handoff: Option<PathBuf>,
    /// Uma etiqueta para a mensagem (ex.: testes)
    #[arg(long)]
    tag: Option<String>,
}

#[derive(Subcommand)]
enum HandoffCommand {
    /// Cria uma nota de handoff em branco para o agente e mostra o caminho
    New {
        /// O agente de destino
        agent: String,
    },
}

#[derive(Args)]
struct StatusArgs {
    /// Imprime a lista do orquestrador em JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct UpgradeArgs {
    /// Usa o instalador do repositório do diretório atual (install-linux.sh
    /// no Linux, install.ps1 no Windows)
    #[arg(long)]
    repo: bool,

    /// Argumentos passados ao instalador (ex.: -y, --dev)
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    args: Vec<String>,
}

#[derive(Args)]
struct WorkspaceArgs {
    /// Salva o diretório atual como workspace
    #[arg(long)]
    save: bool,

    /// Nome do workspace (padrão: nome da pasta)
    #[arg(long, value_name = "NOME", requires = "save")]
    name: Option<String>,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Cli => open_app()?,
        Command::Workspace(args) if args.save => save_workspace(args.name.as_deref())?,
        Command::Workspace(_) => open_workspace(cli.cwd_file.as_deref())?,
        Command::Upgrade(args) => crate::upgrade::run(args.repo, &args.args)?,
        Command::Status(args) => status(args.json)?,
        Command::Send(args) => send(args)?,
        Command::Handoff(HandoffCommand::New { agent }) => {
            let (from, _) = orchestrator::sender();
            let home = hub::endpoint::Endpoint::current()?.home;
            let path = orchestrator::send::new_handoff(&agent, &from, &home)
                .map_err(anyhow::Error::msg)?;
            println!("{}", path.display());
        }
        Command::Hub => hub::daemon::run()?,
        Command::AgyPermissionHook(args) => {
            use crate::agent::adapters::antigravity::permission;
            permission::hook(&args.channel, args.accept_edits);
        }
    }
    Ok(())
}

/// Abre a tela principal. Ela sempre abre: um problema com o banco, o
/// diretório ou o home vira um aviso no chat, e o que faltou fica só nesta
/// execução (banco em memória, configurações numa pasta temporária).
fn open_app() -> Result<()> {
    let mut warnings = Vec::new();
    let mut warn = |err: anyhow::Error| warnings.push(format!("{err:#}"));
    let db = match Db::open_default() {
        Ok(db) => db,
        Err(err) => {
            warn(err.context("o terra abriu sem o banco; recentes e salvos não serão gravados"));
            Db::open_in_memory()?
        }
    };
    let dir = current_dir().unwrap_or_else(|err| {
        warn(err);
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    if let Err(err) = db.touch_recent(&dir) {
        warn(err);
    }
    // As configurações do agente são do workspace: o nome salvo deste
    // diretório ou, sem um, o nome da pasta.
    let name = db
        .workspace_name(&dir)
        .ok()
        .flatten()
        .unwrap_or_else(|| db::default_name(&dir));
    let root = db::data_dir().unwrap_or_else(|err| {
        warn(err);
        env::temp_dir().join(".terraforme")
    });
    let files = tui::ConfigFiles {
        workspace: settings::config_file(&root, &name),
        global: settings::global_file(&root),
    };
    tui::run_app(db, dir, name, files, warnings)
}

/// O `terra status`: os terminais abertos, por workspace. Sem o
/// orquestrador, ele inicia (e, sem terminais, fecha logo depois).
fn status(as_json: bool) -> Result<()> {
    let instances = hub::client::snapshot().context("não foi possível falar com o orquestrador")?;
    if as_json {
        println!("{:#}", serde_json::json!({ "instances": instances }));
        return Ok(());
    }
    let instances: Vec<_> = instances
        .iter()
        .filter_map(hub::protocol::Instance::from_json)
        .collect();
    if instances.is_empty() {
        println!("Nenhuma instância ativa.");
        return Ok(());
    }
    for (i, workspace) in hub::view::by_workspace(&instances).iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{}  ·  {}", workspace.name, workspace.summary());
        for instance in &workspace.instances {
            println!("  {}", hub::view::instance_line(instance));
            if instance.health != hub::protocol::Health::Gone {
                for tab in &instance.status.tabs {
                    println!("    {}", hub::view::tab_line(tab));
                }
            }
        }
    }
    Ok(())
}

/// O `terra send`: confere, grava a nota e manda pelo hub. Uma linha de
/// saída, porque ela entra no contexto do agente que mandou.
fn send(args: SendArgs) -> Result<()> {
    use orchestrator::config::Config;
    use orchestrator::message::State;
    let endpoint = hub::endpoint::Endpoint::current()?;
    let config = Config::load(&Config::path(&endpoint.home))
        .map_err(|err| anyhow::anyhow!("orchestrator.toml: {err}"))?;
    let (from, hops) = orchestrator::sender();
    let args = orchestrator::send::Args {
        to: args.agent,
        text: (!args.text.is_empty()).then(|| args.text.join(" ")),
        refs: args.refs,
        handoff: args.handoff,
        tag: args.tag,
    };
    let message =
        orchestrator::send::prepare(&args, &from, hops, &config, &endpoint.home, &current_dir()?)
            .map_err(|reject| anyhow::anyhow!("recusada: {}", reject.text))?;
    let reply = hub::client::send_message(&endpoint, true, &message.to_json())
        .context("não foi possível falar com o orquestrador")?;
    match reply["state"].as_str().and_then(State::parse) {
        Some(state) if reply["t"] == "sent" => {
            println!("{}", orchestrator::send::sent_line(&message, state));
            Ok(())
        }
        _ => anyhow::bail!(
            "recusada: {}",
            reply["text"].as_str().unwrap_or("sem motivo")
        ),
    }
}

fn save_workspace(name: Option<&str>) -> Result<()> {
    let db = Db::open_default()?;
    let path = current_dir()?;
    match db.save_workspace(&path, name)? {
        SaveOutcome::Saved(name) => {
            let msg = format!("Workspace \"{name}\" salvo: {}", path.display());
            println!("{}", theme::paint_success(&msg, stdout().is_terminal()));
        }
        SaveOutcome::AlreadySaved(name) => println!("Este path já está salvo como {name}"),
    }
    Ok(())
}

fn open_workspace(cwd_file: Option<&Path>) -> Result<()> {
    let db = Db::open_default()?;
    let Some(path) = tui::run_workspace(&db)? else {
        return Ok(());
    };
    match cwd_file {
        Some(file) => fs::write(file, path.to_string_lossy().as_bytes())
            .with_context(|| format!("não foi possível gravar {}", file.display()))?,
        // Sem a função de shell não há como trocar de diretório: mostra o path.
        None => println!("{}", path.display()),
    }
    Ok(())
}

fn current_dir() -> Result<PathBuf> {
    let dir = env::current_dir().context("não foi possível ler o diretório atual")?;
    db::canonical_path(&dir).with_context(|| format!("não foi possível resolver {}", dir.display()))
}
