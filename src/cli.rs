//! Definição dos argumentos (clap) e execução dos comandos.

use std::env;
use std::fs;
use std::io::{IsTerminal, stdout};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::db::{self, Db, SaveOutcome};
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
    let config = settings::config_file(&root, &name);
    tui::run_app(db, dir, config, warnings)
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
