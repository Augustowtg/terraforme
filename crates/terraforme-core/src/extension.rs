//! Extensões do terraforme (RTK, AI Memory e AI Jail): instalar, atualizar e remover.
//!
//! Cada operação usa o caminho oficial de cada projeto — os scripts de
//! instalação publicados por eles e os subcomandos próprios de
//! atualização/remoção; o AI Jail, que não tem script de instalação, vem do
//! binário publicado nos releases (com o checksum conferido). Os comandos externos herdam o terminal, então o
//! usuário vê o progresso e responde às perguntas que eles fizerem.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::status::{Component, State, find_in_path, run_ok, version_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Extension {
    /// RTK — Rust Token Killer.
    Rtk,
    AiMemory,
    AiJail,
}

/// O que aconteceu numa operação que pode não ter nada a fazer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Installed {
        version: Option<String>,
    },
    AlreadyInstalled {
        version: Option<String>,
    },
    Updated {
        from: Option<String>,
        to: Option<String>,
    },
    UpToDate {
        version: Option<String>,
    },
    Deleted,
}

/// Relatório que a extensão imprime sobre si mesma (economia, status...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub label: &'static str,
    args: &'static [&'static str],
}

const RTK_REPORTS: [Report; 4] = [
    Report {
        label: "Economia",
        args: &["gain"],
    },
    Report {
        label: "Neste projeto",
        args: &["gain", "--project"],
    },
    Report {
        label: "Por dia",
        args: &["gain", "--graph"],
    },
    Report {
        label: "Histórico",
        args: &["gain", "--history"],
    },
];

const AI_MEMORY_REPORTS: [Report; 3] = [
    Report {
        label: "Status",
        args: &["status"],
    },
    Report {
        label: "Projetos",
        args: &["list-projects"],
    },
    Report {
        label: "Handoffs",
        args: &["handoffs"],
    },
];

const AI_JAIL_REPORTS: [Report; 2] = [
    Report {
        label: "Status",
        args: &["status"],
    },
    Report {
        label: "Sandbox",
        args: &["--dry-run", "claude"],
    },
];

const RTK_INSTALL_SH: &str =
    "https://raw.githubusercontent.com/rtk-ai/rtk/refs/heads/master/install.sh";
const AI_MEMORY_INSTALL_SH: &str =
    "https://github.com/akitaonrails/ai-memory/releases/latest/download/install.sh";
const AI_MEMORY_INSTALL_PS1: &str =
    "https://github.com/akitaonrails/ai-memory/releases/latest/download/install.ps1";
const AI_JAIL_RELEASES: &str = "https://github.com/akitaonrails/ai-jail/releases/latest/download";

/// Baixa o binário do release ($1/$2), confere o .sha256 publicado ao lado e
/// o instala em $3.
const AI_JAIL_INSTALL_SH: &str = r#"set -eu
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
curl -fsSLO "$1/$2"
curl -fsSLO "$1/$2.sha256"
if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$2.sha256"; else shasum -a 256 -c "$2.sha256"; fi
tar -xzf "$2" ai-jail
mkdir -p "$3"
install -m 755 ai-jail "$3/ai-jail"
"#;

impl Extension {
    pub const ALL: [Extension; 3] = [Extension::Rtk, Extension::AiMemory, Extension::AiJail];

    /// Identificador usado na linha de comando.
    pub fn id(self) -> &'static str {
        match self {
            Extension::Rtk => "rtk",
            Extension::AiMemory => "ai-memory",
            Extension::AiJail => "ai-jail",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Extension::Rtk => "RTK",
            Extension::AiMemory => "AI Memory",
            Extension::AiJail => "AI Jail",
        }
    }

    pub fn url(self) -> &'static str {
        match self {
            Extension::Rtk => "https://github.com/rtk-ai/rtk",
            Extension::AiMemory => "https://github.com/akitaonrails/ai-memory",
            Extension::AiJail => "https://github.com/akitaonrails/ai-jail",
        }
    }

    fn bin(self) -> &'static str {
        self.id()
    }

    pub fn check(self) -> Component {
        let state = match locate(self.bin()) {
            None => State::NotInstalled,
            // Outro projeto (Rust Type Kit) usa o mesmo nome de binário; o RTK
            // correto responde ao subcomando `gain`.
            Some(path) if self == Extension::Rtk && !run_ok(&path, &["gain", "--help"]) => {
                State::Conflict {
                    path,
                    detail: "`rtk gain` falhou — pode ser o Rust Type Kit".into(),
                }
            }
            Some(path) => State::Installed {
                version: version_of(&path),
                path,
            },
        };

        Component {
            name: self.name(),
            url: self.url(),
            state,
        }
    }

    /// Baixa e instala a versão mais recente e configura os agentes (no AI
    /// Jail, cria o `~/.ai-jail` padrão se não houver um). Não faz nada se já
    /// estiver instalada.
    pub fn install(self) -> Result<Outcome> {
        match self.check().state {
            State::Installed { version, .. } => return Ok(Outcome::AlreadyInstalled { version }),
            State::Conflict { path, detail } => {
                bail!(
                    "{}: {} — remova-o antes de instalar",
                    path.display(),
                    detail
                )
            }
            State::NotInstalled => {}
        }

        self.run_installer()?;
        let path = self.installed_path()?;

        match self {
            Extension::Rtk => run(Command::new(&path).args(["init", "-g"]))?,
            Extension::AiMemory => run(Command::new(&path).arg("setup"))?,
            Extension::AiJail => {
                crate::jail::ensure_default_config()?;
            }
        }

        Ok(Outcome::Installed {
            version: version_of(&path),
        })
    }

    pub fn update(self) -> Result<Outcome> {
        let (path, from) = self.require_installed()?;

        match self {
            // O install.sh do RTK sempre baixa a última versão e sobrescreve.
            Extension::Rtk => self.run_installer()?,
            Extension::AiMemory => run(Command::new(&path).arg("upgrade"))?,
            Extension::AiJail => {
                require_ours(self, &path)?;
                self.run_installer()?;
            }
        }

        let to = version_of(&self.installed_path()?);
        Ok(if from == to {
            Outcome::UpToDate { version: to }
        } else {
            Outcome::Updated { from, to }
        })
    }

    /// Remove a configuração dos agentes e o binário. Os dados do AI Memory
    /// (wiki, banco) e o `~/.ai-jail` são preservados.
    pub fn delete(self) -> Result<Outcome> {
        let (path, _) = self.require_installed()?;

        match self {
            Extension::Rtk => {
                run(Command::new(&path).args(["init", "-g", "--uninstall"]))?;
                remove_file(&path)?;
            }
            Extension::AiMemory => {
                run(Command::new(&path).args(["uninstall", "--apply", "--yes"]))?;
                stop_ai_memory_service()?;
                // O install.sh cria ~/.local/bin/ai-memory como link para
                // ~/.local/lib/ai-memory/ai-memory.
                let target = fs::canonicalize(&path).ok();
                remove_file(&path)?;
                if let Some(lib_dir) = target.as_deref().and_then(Path::parent)
                    && lib_dir.file_name().is_some_and(|n| n == "ai-memory")
                {
                    fs::remove_dir_all(lib_dir)
                        .with_context(|| format!("removendo {}", lib_dir.display()))?;
                }
            }
            Extension::AiJail => {
                require_ours(self, &path)?;
                remove_file(&path)?;
            }
        }

        // Plugin do terraforme com os hooks movidos do Claude (ver `agent`).
        let plugin = crate::agent::plugin_dir(self)?;
        if plugin.exists() {
            fs::remove_dir_all(&plugin)
                .with_context(|| format!("removendo {}", plugin.display()))?;
        }

        Ok(Outcome::Deleted)
    }

    /// Relatórios disponíveis, o primeiro é o principal.
    pub fn reports(self) -> &'static [Report] {
        match self {
            Extension::Rtk => &RTK_REPORTS,
            Extension::AiMemory => &AI_MEMORY_REPORTS,
            Extension::AiJail => &AI_JAIL_REPORTS,
        }
    }

    /// Executa o relatório e devolve a saída como texto puro (sem cores).
    pub fn report(self, report: Report) -> Result<String> {
        let (path, _) = self.require_installed()?;
        let output = Command::new(&path)
            .args(report.args)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("falha ao executar `{}`", path.display()))?;
        if !output.status.success() {
            let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
            bail!(
                "`{} {}` terminou com erro ({}): {}",
                self.bin(),
                report.args.join(" "),
                output.status,
                stderr.trim()
            );
        }
        Ok(strip_ansi(&String::from_utf8_lossy(&output.stdout)))
    }

    fn require_installed(self) -> Result<(PathBuf, Option<String>)> {
        match self.check().state {
            State::Installed { path, version } => Ok((path, version)),
            State::Conflict { path, detail } => bail!("{}: {}", path.display(), detail),
            State::NotInstalled => bail!(
                "{} não está instalado — use `terraforme extension {} --install`",
                self.name(),
                self.id()
            ),
        }
    }

    fn installed_path(self) -> Result<PathBuf> {
        locate(self.bin()).with_context(|| {
            format!(
                "o instalador terminou, mas `{}` não foi encontrado",
                self.bin()
            )
        })
    }

    fn run_installer(self) -> Result<()> {
        if cfg!(windows) {
            return match self {
                Extension::AiMemory => run(Command::new("powershell").args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &format!("irm {AI_MEMORY_INSTALL_PS1} | iex"),
                ])),
                Extension::Rtk => bail!(
                    "instalação automática do RTK não é suportada no Windows — veja {}",
                    self.url()
                ),
                Extension::AiJail => bail!("o AI Jail não roda no Windows — use-o dentro do WSL2"),
            };
        }

        if find_in_path("curl").is_none() {
            bail!("`curl` não encontrado — instale-o para baixar as extensões");
        }
        let url = match self {
            Extension::Rtk => RTK_INSTALL_SH,
            Extension::AiMemory => AI_MEMORY_INSTALL_SH,
            Extension::AiJail => {
                let asset = match (env::consts::OS, env::consts::ARCH) {
                    ("linux", "x86_64") => "ai-jail-linux-x86_64.tar.gz",
                    ("macos", "aarch64") => "ai-jail-macos-aarch64.tar.gz",
                    (os, arch) => bail!(
                        "o AI Jail não publica binário para {os}/{arch} — instale com `cargo install --locked ai-jail`"
                    ),
                };
                let bin_dir = home()
                    .context("pasta do usuário não encontrada")?
                    .join(".local/bin");
                return run(Command::new("sh")
                    .args(["-c", AI_JAIL_INSTALL_SH, "sh", AI_JAIL_RELEASES, asset])
                    .arg(bin_dir));
            }
        };
        run(Command::new("sh").args(["-c", &format!("curl -fsSL {url} | sh")]))
    }
}

/// Procura no PATH e, como os instaladores usam `~/.local/bin`, também lá —
/// assim uma instalação recém-feita é encontrada mesmo fora do PATH.
fn locate(bin: &str) -> Option<PathBuf> {
    find_in_path(bin).or_else(|| {
        let path = home()?
            .join(".local/bin")
            .join(format!("{bin}{}", env::consts::EXE_SUFFIX));
        path.is_file().then_some(path)
    })
}

/// O AI Jail que o terraforme instala fica em `~/.local/bin`; um instalado
/// por Homebrew, AUR, cargo ou Nix é atualizado/removido pelo próprio gerenciador.
fn require_ours(ext: Extension, path: &Path) -> Result<()> {
    let ours = home().is_some_and(|h| path.starts_with(h.join(".local/bin")));
    if !ours {
        bail!(
            "{} foi instalado por outro gerenciador ({}) — atualize ou remova por ele (ex.: `brew upgrade {}`)",
            ext.name(),
            path.display(),
            ext.id()
        );
    }
    Ok(())
}

fn home() -> Option<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn run(cmd: &mut Command) -> Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .with_context(|| format!("falha ao executar `{program}`"))?;
    if !status.success() {
        bail!("`{program}` terminou com erro ({status})");
    }
    Ok(())
}

/// Remove sequências de escape ANSI (cores, título, cursor) de um texto.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: ESC [ … byte final entre '@' e '~'.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … terminado por BEL ou ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn remove_file(path: &Path) -> Result<()> {
    fs::remove_file(path).with_context(|| format!("removendo {}", path.display()))
}

/// `ai-memory setup` cria um serviço de usuário no systemd; `uninstall` não o remove.
fn stop_ai_memory_service() -> Result<()> {
    if cfg!(windows) || find_in_path("systemctl").is_none() {
        return Ok(());
    }
    let Some(unit) = home().map(|h| h.join(".config/systemd/user/ai-memory.service")) else {
        return Ok(());
    };
    if !unit.exists() {
        return Ok(());
    }
    // Falhar ao parar o serviço não deve impedir a remoção.
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", "ai-memory.service"])
        .status();
    remove_file(&unit)?;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_uses_extension_metadata() {
        for ext in Extension::ALL {
            let c = ext.check();
            assert_eq!(c.name, ext.name());
            assert_eq!(c.url, ext.url());
            assert!(!ext.reports().is_empty());
        }
    }

    #[test]
    fn strip_ansi_removes_colors_and_titles() {
        assert_eq!(
            strip_ansi("\x1b[2m2026\x1b[0m \x1b[32m INFO\x1b[0m ok"),
            "2026  INFO ok"
        );
        assert_eq!(strip_ansi("\x1b]0;título\x07texto"), "texto");
        assert_eq!(strip_ansi("\x1b]0;t\x1b\\fim"), "fim");
        assert_eq!(strip_ansi("sem escapes █ ✔"), "sem escapes █ ✔");
    }
}
