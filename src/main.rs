mod agent;
mod cli;
mod db;
mod extension;
mod files;
mod platform;
mod settings;
mod theme;
mod tui;
mod upgrade;

use std::io::{IsTerminal, stderr};
use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let msg = format!("erro: {err:#}");
            eprintln!("{}", theme::paint_error(&msg, stderr().is_terminal()));
            ExitCode::FAILURE
        }
    }
}
