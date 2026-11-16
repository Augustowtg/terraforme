//! `terra upgrade`: atualiza o terra pelo instalador.
//!
//! Sem `--repo`, roda o instalador do GitHub sem clonar o repositório, como na
//! instalação: `curl ... install.sh | bash` (Linux) ou `irm ... install.ps1 |
//! iex` (Windows), que instalam a última release. Com `--repo`, procura o
//! instalador no repositório do diretório atual (ou de um diretório acima):
//! o `install-linux.sh` no Linux e o `install.ps1` no Windows.
//!
//! Pelo GitHub, o terra lê antes a última release publicada, que é a que o
//! instalador baixa: se ela não é mais nova que a atual, avisa e não instala;
//! se é, instala sem perguntar (o `-y` vai ao instalador) e mostra depois as
//! notas das versões novas, do CHANGELOG.md da `main`. Uma versão que está
//! no CHANGELOG mas ainda não tem release não conta: o instalador baixaria a
//! release antiga de novo.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const RAW: &str = "https://raw.githubusercontent.com/Augustowtg/terraforme/main";
const RELEASES: &str = "https://github.com/Augustowtg/terraforme/releases";

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
        match fetch_latest_release() {
            Ok(latest) if !is_newer(&latest, CURRENT) => {
                println!(
                    "O terra ({CURRENT}) já está em dia: a última release publicada é a {latest}."
                );
                return Ok(());
            }
            Ok(latest) => {
                println!("Atualizando o terra da {CURRENT} para a {latest}...");
                notes = Some(fetch_notes(&latest));
            }
            Err(err) => {
                eprintln!("aviso: não foi possível ler a última release ({err:#})");
                println!("Atualizando o terra {CURRENT} pelo instalador do GitHub...");
            }
        }
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
    if let Some(notes) = notes
        && !notes.is_empty()
    {
        println!("\nO que mudou desde a {CURRENT}:\n\n{notes}");
    }
    Ok(())
}

/// Uma versão mais nova que a atual: a da última release e as notas de todas
/// as que vieram depois da atual, até ela.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes: String,
}

/// A versão atual do terra.
pub fn current() -> &'static str {
    CURRENT
}

/// Lê a última release do GitHub; `None` se o terra já está nela.
pub fn check() -> Result<Option<Update>> {
    let version = fetch_latest_release()?;
    Ok(is_newer(&version, CURRENT).then(|| Update {
        notes: fetch_notes(&version),
        version,
    }))
}

fn is_newer(version: &str, current: &str) -> bool {
    parse_version(version) > parse_version(current)
}

/// A saída do `curl` (no Windows 10+, o `curl.exe`) com `args`.
fn curl(args: &[&str]) -> Result<String> {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let output = Command::new(curl)
        .args(["-fsSL", "--max-time", "15"])
        .args(args)
        .output()
        .with_context(|| format!("não foi possível rodar o {curl}"))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A versão da última release publicada, a que o instalador baixa: o
/// `/releases/latest` redireciona para `/releases/tag/vX.Y.Z` (como no
/// install.sh). A URL final vem na última linha, depois dos cabeçalhos.
fn fetch_latest_release() -> Result<String> {
    let output = curl(&[
        "-I",
        "-w",
        "\n%{url_effective}",
        &format!("{RELEASES}/latest"),
    ])?;
    release_version(output.lines().last().unwrap_or_default())
        .map(str::to_string)
        .context("o repositório não tem release publicada")
}

fn release_version(url: &str) -> Option<&str> {
    Some(url.trim().rsplit_once("/releases/tag/v")?.1)
}

/// As notas das versões depois da atual até a `latest`, do CHANGELOG.md da
/// `main`; vazio sem conseguir ler (a atualização segue sem as notas).
fn fetch_notes(latest: &str) -> String {
    curl(&[&format!("{RAW}/CHANGELOG.md")])
        .map(|changelog| notes_between(&changelog, CURRENT, latest))
        .unwrap_or_default()
}

/// As seções `## [x.y.z]` do CHANGELOG de versão maior que `current` e até a
/// `latest`, na ordem do arquivo (da mais nova para a mais velha). Vazio se
/// não houver.
fn notes_between(changelog: &str, current: &str, latest: &str) -> String {
    let (current, latest) = (parse_version(current), parse_version(latest));
    let mut notes = String::new();
    let mut keep = false;
    for line in changelog.lines() {
        if let Some(rest) = line.strip_prefix("## [") {
            let version = parse_version(rest.split(']').next().unwrap_or_default());
            keep = version > current && version <= latest;
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
    fn notes_between_pega_as_versoes_novas_ate_a_release() {
        let changelog = "# Changelog\n\nintro\n\n## [Unreleased]\n\n- x\n\n## [0.9.0] - 2026-11-01\n\n- b\n\n## [0.8.25] - 2026-10-28\n\n- a\n\n## [0.8.24] - 2026-10-28\n\n- velho\n";
        assert_eq!(
            notes_between(changelog, "0.8.24", "0.9.0"),
            "## [0.9.0] - 2026-11-01\n\n- b\n\n## [0.8.25] - 2026-10-28\n\n- a"
        );
        // A 0.9.0 está no CHANGELOG, mas a última release é a 0.8.25.
        assert_eq!(
            notes_between(changelog, "0.8.24", "0.8.25"),
            "## [0.8.25] - 2026-10-28\n\n- a"
        );
        assert_eq!(notes_between(changelog, "0.9.0", "0.9.0"), "");
    }

    #[test]
    fn a_release_so_e_nova_se_for_maior_que_a_atual() {
        let url = "https://github.com/Augustowtg/terraforme/releases/tag/v0.13.23\n";
        assert_eq!(release_version(url), Some("0.13.23"));
        // Sem release, o /releases/latest redireciona para /releases.
        assert_eq!(release_version("https://github.com/x/y/releases"), None);
        assert!(is_newer("0.13.26", "0.13.25"));
        assert!(is_newer("0.14.0", "0.13.25"));
        // Compilado da main, na frente da última release: nada a instalar.
        assert!(!is_newer("0.13.23", "0.13.25"));
        assert!(!is_newer("0.13.25", "0.13.25"));
    }
}
