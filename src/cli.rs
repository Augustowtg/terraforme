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
    /// Conclui a execução despachada em que o agente roda, com um resumo
    Done {
        /// O resumo do que foi feito
        summary: Vec<String>,
    },
    /// O servidor MCP do orquestrador (stdio), que todo agente despachado
    /// recebe: as ferramentas `send`, `done`, `handoff_new` e `status`
    Mcp(McpArgs),

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
struct McpArgs {
    /// O agente que usa o servidor (o remetente do `send`); sem ele, o do
    /// ambiente (`TERRA_AGENT`)
    #[arg(long = "agente", value_name = "NOME")]
    agent: Option<String>,
    /// Os saltos da mensagem que gerou a execução; sem ele, o `TERRA_HOPS`
    #[arg(long)]
    hops: Option<u32>,
    /// A execução despachada (para o `done`); sem ela, a do `TERRA_RUN`
    #[arg(long = "execucao", value_name = "ID")]
    run: Option<String>,
    /// A pasta contra a qual os caminhos relativos são resolvidos; sem ela,
    /// o diretório atual
    #[arg(long = "pasta", value_name = "PASTA")]
    cwd: Option<PathBuf>,
}

#[derive(Args)]
struct SendArgs {
    /// O agente de destino: o nome de uma aba (`/agente`) ou de um
    /// `[agents.<nome>]` do orchestrator.toml
    #[arg(required_unless_present = "batch")]
    agent: Option<String>,
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
    /// Manda várias mensagens de nível 1 de um arquivo (`-` é a entrada),
    /// uma por linha: `agente: texto`. Linhas vazias e com `#` são puladas
    #[arg(long = "lote", value_name = "ARQUIVO", conflicts_with_all = ["agent", "refs", "handoff"])]
    batch: Option<PathBuf>,
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
        Command::Done { summary } => {
            let Ok(run) = env::var(orchestrator::RUN_ENV) else {
                anyhow::bail!("o terra done só funciona dentro de uma execução do orquestrador");
            };
            done(&run, &summary.join(" "))?;
            println!("ok");
        }
        Command::Mcp(args) => {
            let (from, hops) = orchestrator::sender();
            let identity = crate::mcp::Identity {
                agent: args.agent.unwrap_or(from),
                hops: args.hops.unwrap_or(hops),
                run: args.run.or_else(|| env::var(orchestrator::RUN_ENV).ok()),
                cwd: match args.cwd {
                    Some(cwd) => cwd,
                    None => current_dir()?,
                },
            };
            crate::mcp::serve(&identity)?;
        }
        Command::Handoff(HandoffCommand::New { agent }) => {
            let (from, _) = orchestrator::sender();
            println!("{}", new_handoff(&agent, &from)?.display());
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
    // As configurações e o histórico são do workspace, que nasce sozinho na
    // primeira vez que o diretório abre.
    let name = db.workspace_folder(&dir).unwrap_or_else(|err| {
        warn(err);
        db::default_name(&dir)
    });
    let root = db::data_dir().unwrap_or_else(|err| {
        warn(err);
        env::temp_dir().join(".terraforme")
    });
    let files = tui::ConfigFiles {
        workspace: settings::config_file(&root, &name),
        history: settings::history_file(&root, &name),
        global: settings::global_file(&root),
    };
    if let Err(err) = files.create() {
        warn(err);
    }
    tui::run_app(db, dir, name, files, warnings)
}

/// O `terra status`: os terminais abertos, por workspace. Sem o
/// orquestrador, ele inicia (e, sem terminais, fecha logo depois).
fn status(as_json: bool) -> Result<()> {
    println!("{}", status_text(as_json)?);
    Ok(())
}

/// O texto do `terra status` (também a ferramenta `status` do MCP).
pub(crate) fn status_text(as_json: bool) -> Result<String> {
    let instances = hub::client::snapshot().context("não foi possível falar com o orquestrador")?;
    if as_json {
        return Ok(format!(
            "{:#}",
            serde_json::json!({ "instances": instances })
        ));
    }
    let instances: Vec<_> = instances
        .iter()
        .filter_map(hub::protocol::Instance::from_json)
        .collect();
    if instances.is_empty() {
        return Ok("Nenhuma instância ativa.".into());
    }
    let mut lines = Vec::new();
    for (i, workspace) in hub::view::by_workspace(&instances).iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.push(format!("{}  ·  {}", workspace.name, workspace.summary()));
        for instance in &workspace.instances {
            lines.push(format!("  {}", hub::view::instance_line(instance)));
            if instance.health != hub::protocol::Health::Gone {
                for tab in &instance.status.tabs {
                    lines.push(format!("    {}", hub::view::tab_line(tab)));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

/// O `terra send`: confere, grava a nota e manda pelo hub. Uma linha de
/// saída, porque ela entra no contexto do agente que mandou.
fn send(args: SendArgs) -> Result<()> {
    use orchestrator::config::Config;
    let endpoint = hub::endpoint::Endpoint::current()?;
    let config = Config::load(&Config::path(&endpoint.home))
        .map_err(|err| anyhow::anyhow!("orchestrator.toml: {err}"))?;
    let Some(batch) = &args.batch else {
        let args = orchestrator::send::Args {
            to: args.agent.unwrap_or_default(),
            text: (!args.text.is_empty()).then(|| args.text.join(" ")),
            refs: args.refs,
            handoff: args.handoff,
            tag: args.tag,
        };
        let (from, hops) = orchestrator::sender();
        let line = send_one(&args, &from, hops, &current_dir()?, &config, &endpoint)?;
        println!("{line}");
        return Ok(());
    };
    // O lote: uma mensagem por linha, sem um LLM no meio. Uma recusada não
    // para as outras; o comando falha no fim se alguma falhou.
    let text = if batch.as_os_str() == "-" {
        std::io::read_to_string(std::io::stdin())?
    } else {
        std::fs::read_to_string(batch).with_context(|| batch.display().to_string())?
    };
    let items = orchestrator::send::batch(&text).map_err(anyhow::Error::msg)?;
    let (from, hops) = orchestrator::sender();
    let cwd = current_dir()?;
    let mut failed = 0;
    for (to, text) in items {
        let args = orchestrator::send::Args {
            to,
            text: Some(text),
            tag: args.tag.clone(),
            ..Default::default()
        };
        match send_one(&args, &from, hops, &cwd, &config, &endpoint) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                failed += 1;
                println!("{}: {err}", args.to);
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} mensagem(ns) do lote recusada(s)");
    }
    Ok(())
}

/// Manda uma mensagem de `from` (com `hops` saltos até aqui; os caminhos
/// relativos são de `cwd`) e devolve a linha de resposta. Também é a
/// ferramenta `send` do MCP.
pub(crate) fn send_one(
    args: &orchestrator::send::Args,
    from: &str,
    hops: u32,
    cwd: &Path,
    config: &orchestrator::config::Config,
    endpoint: &hub::endpoint::Endpoint,
) -> Result<String> {
    use orchestrator::message::State;
    let message = orchestrator::send::prepare(args, from, hops, config, &endpoint.home, cwd)
        .map_err(|reject| anyhow::anyhow!("recusada: {}", reject.text))?;
    let reply = hub::client::send_message(endpoint, true, &message.to_json())
        .context("não foi possível falar com o orquestrador")?;
    match reply["state"].as_str().and_then(State::parse) {
        Some(state) if reply["t"] == "sent" => Ok(orchestrator::send::sent_line(&message, state)),
        _ => anyhow::bail!(
            "recusada: {}",
            reply["text"].as_str().unwrap_or("sem motivo")
        ),
    }
}

/// O `terra done` (e a ferramenta `done` do MCP): o resumo da execução
/// `run`. A execução termina quando o agente sai; o resumo fica no registro
/// dela.
pub(crate) fn done(run: &str, summary: &str) -> Result<()> {
    let endpoint = hub::endpoint::Endpoint::current()?;
    let summary = (!summary.trim().is_empty()).then_some(summary.trim());
    let reply = hub::client::send_done(&endpoint, run, summary)
        .context("não foi possível falar com o orquestrador")?;
    if reply["t"] != "ok" {
        anyhow::bail!(
            "recusado: {}",
            reply["text"].as_str().unwrap_or("sem motivo")
        );
    }
    Ok(())
}

/// O `terra handoff new` (e a ferramenta `handoff_new` do MCP): a nota em
/// branco de `from` para `to`.
pub(crate) fn new_handoff(to: &str, from: &str) -> Result<PathBuf> {
    let home = hub::endpoint::Endpoint::current()?.home;
    orchestrator::send::new_handoff(to, from, &home).map_err(anyhow::Error::msg)
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
