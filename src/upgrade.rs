//! `terra upgrade`: atualiza o terra pelo instalador.
//!
//! Sem `--repo`, roda o instalador do GitHub sem clonar o repositório, como na
//! instalação: `curl ... install.sh | bash` (Linux) ou `irm ... install.ps1 |
//! iex` (Windows), que instalam a última release. Com `--repo`, procura o
//! instalador no repositório do diretório atual (ou de um diretório acima):
//! o `install-linux.sh` no Linux e o `install.ps1` no Windows.
//!
//! Pelo GitHub, o terra lê antes o CHANGELOG.md da `main`: sem versão mais
//! nova que a atual, avisa e não instala; com versão nova, instala sem
//! perguntar (o `-y` vai ao instalador) e mostra depois as notas das versões
//! novas.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const RAW: &str = "https://raw.githubusercontent.com/Augustowtg/terraforme/main";

/// O instalador deste sistema no repositório.
#[cfg(windows)]
const INSTALLER: &str = "install.ps1";
#[cfg(not(windows))]
const INSTALLER: &str = "install-linux.sh";

const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub fn run(repo: bool, args: &[String]) -> Result<()> {
    let mut notes = None;
    let mut command = if repo {
        let dir = env::current_dir().context("não foi possível ler o diretório atual")?;
        let Some(script) = find_installer(&dir) else {
            bail!(
                "{INSTALLER} não encontrado em {} nem nos diretórios acima",
                dir.display()
            );
        };
        println!("Atualizando o terra com {}...", script.display());
        local_command(&script, args)
    } else {
        notes = match fetch_changelog() {
            Ok(changelog) => Some(newer_notes(&changelog, CURRENT)),
            Err(err) => {
                eprintln!("aviso: não foi possível ler o CHANGELOG ({err:#})");
                None
            }
        };
        if let Some(notes) = &notes
            && notes.is_empty()
        {
            println!("O terra já está na última versão ({CURRENT}).");
            return Ok(());
        }
        println!("Atualizando o terra {CURRENT} pelo instalador do GitHub...");
        let mut args = args.to_vec();
        if !args.iter().any(|arg| arg == "-y" || arg == "--yes") {
            args.push("-y".into());
        }
        remote_command(&args)
    };
    let replaced = release_exe();
    let status = command
        .status()
        .context("não foi possível rodar o instalador");
    restore_exe(replaced);
    let status = status?;
    if !status.success() {
        bail!("o instalador terminou com erro ({status})");
    }
    if let Some(notes) = notes {
        println!("\nO que mudou desde a {CURRENT}:\n\n{notes}");
    }
    Ok(())
}

/// O CHANGELOG.md da `main`, pelo `curl` (no Windows 10+, o `curl.exe`).
fn fetch_changelog() -> Result<String> {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let output = Command::new(curl)
        .args(["-fsSL", &format!("{RAW}/CHANGELOG.md")])
        .output()
        .with_context(|| format!("não foi possível rodar o {curl}"))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// As seções `## [x.y.z]` do CHANGELOG de versão maior que `current`, na ordem
/// do arquivo (da mais nova para a mais velha). Vazio se não houver.
fn newer_notes(changelog: &str, current: &str) -> String {
    let current = parse_version(current);
    let mut notes = String::new();
    let mut keep = false;
    for line in changelog.lines() {
        if let Some(rest) = line.strip_prefix("## [") {
            let version = rest.split(']').next().unwrap_or_default();
            keep = parse_version(version) > current;
        }
        if keep {
            notes.push_str(line);
            notes.push('\n');
        }
    }
    notes.trim_end().to_string()
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().splitn(3, '.').map(|part| part.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// O primeiro `INSTALLER` de `dir` para cima.
fn find_installer(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|dir| dir.join(INSTALLER))
        .find(|path| path.is_file())
}

#[cfg(not(windows))]
fn local_command(script: &Path, args: &[String]) -> Command {
    let mut command = Command::new("bash");
    command.arg(script).args(args);
    command
}

#[cfg(windows)]
fn local_command(script: &Path, args: &[String]) -> Command {
    let mut command = powershell();
    command.arg("-File").arg(script).args(args);
    command
}

/// `curl ... | bash -s -- <args>`: o instalador não fica em disco, e o
/// install.sh lê as teclas do terminal.
#[cfg(not(windows))]
fn remote_command(args: &[String]) -> Command {
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(r#"curl -fsSL "$0" | bash -s -- "$@""#)
        .arg(format!("{RAW}/install.sh"))
        .args(args);
    command
}

/// O `irm ... | iex` com os argumentos: o script vira um scriptblock, que
/// recebe os argumentos como o `@args` do install.ps1.
#[cfg(windows)]
fn remote_command(args: &[String]) -> Command {
    let mut script = format!("& ([scriptblock]::Create((irm '{RAW}/install.ps1')))");
    for arg in args {
        script.push_str(&format!(" '{}'", arg.replace('\'', "''")));
    }
    let mut command = powershell();
    command.arg("-Command").arg(script);
    command
}

#[cfg(windows)]
fn powershell() -> Command {
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass"]);
    command
}

/// No Windows, o executável em uso não pode ser sobrescrito, mas pode ser
/// renomeado: ele sai do caminho para o instalador gravar o novo. Devolve o
/// path original e o novo nome.
#[cfg(windows)]
fn release_exe() -> Option<(PathBuf, PathBuf)> {
    let exe = env::current_exe().ok()?;
    let old = exe.with_extension("exe.old");
    // O de uma atualização anterior, que estava em uso.
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).ok()?;
    Some((exe, old))
}

#[cfg(not(windows))]
fn release_exe() -> Option<(PathBuf, PathBuf)> {
    None
}

/// Se o instalador não gravou um executável novo no lugar, o antigo volta.
fn restore_exe(replaced: Option<(PathBuf, PathBuf)>) {
    if let Some((exe, old)) = replaced
        && !exe.exists()
    {
        let _ = std::fs::rename(old, exe);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_installer_sobe_ate_o_repositorio() {
        let root = env::temp_dir().join(format!("terra-upgrade-{}", std::process::id()));
        let sub = root.join("src").join("tui");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(find_installer(&sub), None);
        std::fs::write(root.join(INSTALLER), "").unwrap();
        assert_eq!(find_installer(&sub), Some(root.join(INSTALLER)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn newer_notes_pega_so_as_versoes_novas() {
        let changelog = "# Changelog\n\nintro\n\n## [Unreleased]\n\n- x\n\n## [0.9.0] - 2026-11-01\n\n- b\n\n## [0.8.25] - 2026-10-28\n\n- a\n\n## [0.8.24] - 2026-10-28\n\n- velho\n";
        assert_eq!(
            newer_notes(changelog, "0.8.24"),
            "## [0.9.0] - 2026-11-01\n\n- b\n\n## [0.8.25] - 2026-10-28\n\n- a"
        );
        assert_eq!(newer_notes(changelog, "0.9.0"), "");
    }
}
