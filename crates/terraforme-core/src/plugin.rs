//! Plugins: programas opcionais do terraforme publicados em repositórios
//! próprios (ex.: o mascote, `terraforme-pet`) e baixados sob demanda por
//! `terraforme plugin install <nome>`.
//!
//! Cada plugin é descrito por um [`Manifest`]. Os conhecidos ficam em
//! [`KNOWN`] (adicionar um plugin = uma entrada lá); qualquer outro repositório
//! do GitHub com um `terraforme-plugin.toml` na raiz também pode ser instalado
//! por `dono/repo`. Como instalar fica com [`sources`], a fábrica que escolhe,
//! em ordem de preferência, as [`Source`]s possíveis nesta máquina (binário do
//! GitHub Release, depois `cargo install --git`). Os instalados ficam em
//! `<config dir>/plugins.toml`, para `list`/`update`/`remove` funcionarem
//! também com os de terceiros.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::status::find_in_path;

/// Arquivo que descreve o plugin na raiz do repositório dele.
pub const MANIFEST_FILE: &str = "terraforme-plugin.toml";

/// Descrição de um plugin: de onde baixar e qual binário ele instala.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Nome curto, usado nos comandos (`terraforme plugin install pet`).
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Executável instalado (sem `.exe`).
    pub bin: String,
    /// Repositório no GitHub, `dono/repo`.
    pub repo: String,
    /// Versão mínima do terraforme (`X.Y.Z`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_terraforme: Option<String>,
    /// Sistemas suportados (`env::consts::OS`); vazio = todos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
}

/// Plugin conhecido pelo terraforme, instalável só pelo nome.
struct Known {
    name: &'static str,
    description: &'static str,
    bin: &'static str,
    repo: &'static str,
    min_terraforme: Option<&'static str>,
    platforms: &'static [&'static str],
}

/// Registro dos plugins conhecidos: um novo plugin oficial é só uma entrada.
const KNOWN: &[Known] = &[Known {
    name: "pet",
    description: "Janela flutuante com o mascote do terraforme",
    bin: "terraforme-pet",
    repo: "Augustowtg/terraforme-pet",
    min_terraforme: Some("0.25.0"),
    platforms: &["linux", "windows"],
}];

impl From<&Known> for Manifest {
    fn from(k: &Known) -> Self {
        Manifest {
            name: k.name.into(),
            description: k.description.into(),
            bin: k.bin.into(),
            repo: k.repo.into(),
            min_terraforme: k.min_terraforme.map(Into::into),
            platforms: k.platforms.iter().map(|p| (*p).into()).collect(),
        }
    }
}

impl Manifest {
    /// O plugin conhecido com esse nome.
    pub fn known(name: &str) -> Option<Self> {
        KNOWN.iter().find(|k| k.name == name).map(Self::from)
    }

    /// Resolve `spec`: nome de um plugin conhecido ou instalado, ou `dono/repo`
    /// de um repositório com [`MANIFEST_FILE`].
    pub fn resolve(spec: &str) -> Result<Self> {
        if let Some(m) = Self::known(spec) {
            return Ok(m);
        }
        if let Some(m) = Installed::load()?.0.remove(spec) {
            return Ok(m);
        }
        let Some((owner, repo)) = spec.split_once('/') else {
            bail!(
                "plugin `{spec}` desconhecido — use um de `terraforme plugin list` ou `dono/repo` do GitHub"
            );
        };
        let manifest = Self::fetch(owner, repo)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Baixa o [`MANIFEST_FILE`] do branch padrão do repositório.
    fn fetch(owner: &str, repo: &str) -> Result<Self> {
        let url = format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{MANIFEST_FILE}");
        let text = ureq::get(&url)
            .header(
                "User-Agent",
                concat!("terraforme/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .with_context(|| format!("baixando {url}"))?
            .body_mut()
            .read_to_string()?;
        toml::from_str(&text).with_context(|| format!("lendo {MANIFEST_FILE} de {owner}/{repo}"))
    }

    /// Recusa nomes que escapariam da pasta de instalação.
    fn validate(&self) -> Result<()> {
        let ok = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        };
        if !ok(&self.name) || !ok(&self.bin) {
            bail!("{MANIFEST_FILE} inválido: `name` e `bin` só aceitam letras, números, - e _");
        }
        Ok(())
    }

    /// Roda neste sistema.
    pub fn supported(&self) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|p| p == env::consts::OS)
    }

    /// Erro quando o plugin não roda aqui ou pede um terraforme mais novo.
    fn check(&self) -> Result<()> {
        if !self.supported() {
            bail!(
                "o plugin `{}` não roda em {} (só em {})",
                self.name,
                env::consts::OS,
                self.platforms.join(", ")
            );
        }
        if let Some(min) = &self.min_terraforme
            && version(crate::VERSION) < version(min)
        {
            bail!(
                "o plugin `{}` pede o terraforme {min} ou mais novo (este é o {}) — rode `terraforme upgrade`",
                self.name,
                crate::VERSION
            );
        }
        Ok(())
    }

    /// Caminho do binário instalado, se houver.
    pub fn path(&self) -> Option<PathBuf> {
        locate(&self.bin)
    }
}

/// `X.Y.Z` em números, para comparar; partes inválidas valem 0.
fn version(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Plugins instalados (nome → manifest), em `<config dir>/plugins.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Installed(BTreeMap<String, Manifest>);

impl Installed {
    fn path() -> Result<PathBuf> {
        Ok(Config::dir()?.join("plugins.toml"))
    }

    fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lendo {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("lendo {}", path.display())),
        }
    }

    fn save(&self) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(path.parent().context("caminho inválido")?)?;
        fs::write(&path, toml::to_string(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }
}

/// Uma forma de instalar o binário de um plugin em `dir`.
pub trait Source {
    /// Como aparece para o usuário (ex.: "GitHub Release").
    fn label(&self) -> &'static str;
    fn install(&self, plugin: &Manifest, dir: &Path) -> Result<()>;
}

/// Fábrica: as formas de instalar disponíveis nesta máquina, da preferida
/// para a última tentativa.
pub fn sources() -> Vec<Box<dyn Source>> {
    let mut list: Vec<Box<dyn Source>> = Vec::new();
    // No Windows o download é pelo PowerShell, que sempre existe.
    if Release::target().is_some() && (cfg!(windows) || find_in_path("curl").is_some()) {
        list.push(Box::new(Release));
    }
    if find_in_path("cargo").is_some() {
        list.push(Box::new(Cargo));
    }
    list
}

/// Binário pronto do GitHub Release mais recente: o asset
/// `<bin>-<target>.tar.gz` (`.zip` no Windows), conferido contra o
/// `.sha256` publicado ao lado.
struct Release;

/// Baixa `$1/$2` e `$2.sha256`, confere e instala o binário `$3` em `$4`.
const RELEASE_SH: &str = r#"set -eu
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
curl -fsSLO "$1/$2"
curl -fsSLO "$1/$2.sha256"
if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$2.sha256"; else shasum -a 256 -c "$2.sha256"; fi
tar -xzf "$2" "$3"
mkdir -p "$4"
install -m 755 "$3" "$4/$3.new"
mv -f "$4/$3.new" "$4/$3"
"#;

/// O mesmo no Windows (PowerShell 5.1), com os valores nas variáveis
/// `TF_URL`, `TF_ASSET` (`.zip`), `TF_BIN` (com `.exe`) e `TF_DIR`. Um `.exe`
/// em uso não pode ser sobrescrito, mas pode ser renomeado: o atual vira
/// `.old` e é apagado quando possível. ASCII só, pelo console.
const RELEASE_PS1: &str = r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('terraforme-plugin-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $env:TF_ASSET
    Invoke-WebRequest -UseBasicParsing "$env:TF_URL/$env:TF_ASSET" -OutFile $zip
    Invoke-WebRequest -UseBasicParsing "$env:TF_URL/$env:TF_ASSET.sha256" -OutFile "$zip.sha256"
    $expected = ((Get-Content -Raw "$zip.sha256").Trim() -split '\s+')[0]
    $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash
    if ($actual -ne $expected) { throw "sha256 de $env:TF_ASSET nao confere" }
    Expand-Archive -Force $zip (Join-Path $tmp 'x')
    New-Item -ItemType Directory -Force -Path $env:TF_DIR | Out-Null
    $dest = Join-Path $env:TF_DIR $env:TF_BIN
    Copy-Item -Force (Join-Path $tmp "x\$env:TF_BIN") "$dest.new"
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
    if (Test-Path $dest) { Move-Item -Force $dest "$dest.old" }
    Move-Item -Force "$dest.new" $dest
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
}
"#;

impl Release {
    /// Alvo (target triple) dos assets publicados para esta máquina. Os
    /// plugins com janela usam glibc (o musl não abre X11/Wayland/GL).
    fn target() -> Option<String> {
        let arch = env::consts::ARCH;
        match env::consts::OS {
            "linux" => Some(format!("{arch}-unknown-linux-gnu")),
            "macos" => Some(format!("{arch}-apple-darwin")),
            "windows" => Some(format!("{arch}-pc-windows-msvc")),
            _ => None,
        }
    }

    /// Nome do asset do plugin para `target`.
    fn asset(bin: &str, target: &str) -> String {
        let ext = if target.contains("windows") {
            "zip"
        } else {
            "tar.gz"
        };
        format!("{bin}-{target}.{ext}")
    }
}

impl Source for Release {
    fn label(&self) -> &'static str {
        "GitHub Release"
    }

    fn install(&self, plugin: &Manifest, dir: &Path) -> Result<()> {
        let target = Self::target().context("sem binário publicado para este sistema")?;
        let url = format!(
            "https://github.com/{}/releases/latest/download",
            plugin.repo
        );
        let asset = Self::asset(&plugin.bin, &target);
        if cfg!(windows) {
            return run(Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                ])
                .args(["-Command", RELEASE_PS1])
                .env("TF_URL", &url)
                .env("TF_ASSET", &asset)
                .env("TF_BIN", exe(&plugin.bin))
                .env("TF_DIR", dir));
        }
        run(Command::new("sh")
            .args(["-c", RELEASE_SH, "sh", &url, &asset, &plugin.bin])
            .arg(dir))
    }
}

/// Compila do código-fonte com `cargo install --git` (precisa do Rust).
struct Cargo;

impl Source for Cargo {
    fn label(&self) -> &'static str {
        "cargo install"
    }

    fn install(&self, plugin: &Manifest, dir: &Path) -> Result<()> {
        // Numa pasta temporária, para não deixar os `.crates.toml` do cargo
        // ao lado dos binários do usuário.
        let root = env::temp_dir().join(format!("terraforme-plugin-{}", std::process::id()));
        let result = (|| {
            run(
                Command::new(find_in_path("cargo").context("`cargo` não encontrado")?)
                    .args(["install", "--locked", "--git"])
                    .arg(format!("https://github.com/{}", plugin.repo))
                    .arg("--root")
                    .arg(&root)
                    .arg(&plugin.bin),
            )?;
            let name = exe(&plugin.bin);
            fs::create_dir_all(dir)?;
            let tmp = dir.join(format!("{name}.new"));
            fs::copy(root.join("bin").join(&name), &tmp)
                .with_context(|| format!("copiando {name} para {}", dir.display()))?;
            replace(&tmp, &dir.join(&name))
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }
}

/// Resultado de uma instalação.
#[derive(Debug)]
pub struct Outcome {
    pub manifest: Manifest,
    pub path: PathBuf,
    /// [`Source::label`] da forma que funcionou.
    pub source: &'static str,
}

/// Instala (ou reinstala, para atualizar) o plugin `spec`, tentando cada
/// [`Source`] de [`sources`] até uma funcionar.
pub fn install(spec: &str) -> Result<Outcome> {
    let manifest = Manifest::resolve(spec)?;
    manifest.check()?;
    let dir = install_dir()?;
    let list = sources();
    if list.is_empty() {
        bail!(
            "sem como baixar o plugin `{}` — instale o `curl` (binário pronto) ou o Rust (`cargo`)",
            manifest.name
        );
    }
    let mut errors = Vec::new();
    for source in list {
        match source.install(&manifest, &dir) {
            Ok(()) => {
                let mut installed = Installed::load()?;
                installed.0.insert(manifest.name.clone(), manifest.clone());
                installed.save()?;
                return Ok(Outcome {
                    path: dir.join(exe(&manifest.bin)),
                    manifest,
                    source: source.label(),
                });
            }
            Err(e) => errors.push(format!("{}: {e:#}", source.label())),
        }
    }
    bail!(
        "falha ao instalar o plugin `{}`:\n  {}",
        manifest.name,
        errors.join("\n  ")
    )
}

/// Remove o binário do plugin (só o instalado pelo terraforme) e o tira de
/// `plugins.toml`. Devolve o caminho apagado.
pub fn remove(name: &str) -> Result<Option<PathBuf>> {
    let manifest = Manifest::resolve(name)?;
    let path = install_dir()?.join(exe(&manifest.bin));
    let removed = match fs::remove_file(&path) {
        Ok(()) => Some(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("apagando {}", path.display())),
    };
    let mut installed = Installed::load()?;
    if installed.0.remove(&manifest.name).is_some() {
        installed.save()?;
    }
    Ok(removed)
}

/// Plugins conhecidos e instalados (sem repetir), com o binário de cada um.
pub fn list() -> Result<Vec<(Manifest, Option<PathBuf>)>> {
    let mut all: BTreeMap<String, Manifest> = KNOWN
        .iter()
        .map(|k| (k.name.to_string(), Manifest::from(k)))
        .collect();
    all.extend(Installed::load()?.0);
    Ok(all
        .into_values()
        .map(|m| {
            let path = m.path();
            (m, path)
        })
        .collect())
}

/// Onde os plugins são instalados: `~/.local/bin` no Linux/macOS (como o
/// AI Jail) e a pasta do próprio `terraforme` no Windows.
pub fn install_dir() -> Result<PathBuf> {
    if cfg!(windows) {
        return env::current_exe()?
            .parent()
            .map(Path::to_path_buf)
            .context("pasta do terraforme não encontrada");
    }
    Ok(crate::agent::home()?.join(".local/bin"))
}

/// Binário de um plugin: o da [`install_dir`] (onde `plugin install` o põe,
/// mesmo fora do `PATH`) e, sem ele, o do `PATH` ou o ao lado do
/// `terraforme`. A pasta de instalação vem primeiro para um binário velho em
/// `target/debug` (de quando o plugin era deste repositório) não ganhar do
/// instalado.
pub fn locate(bin: &str) -> Option<PathBuf> {
    let installed = install_dir().ok().map(|dir| dir.join(exe(bin)));
    if let Some(path) = installed.filter(|p| p.is_file()) {
        return Some(path);
    }
    let other = crate::mcp::sibling_bin(bin);
    (other.is_absolute() && other.is_file()).then_some(other)
}

/// Põe `new` no lugar de `dest`. O Windows não deixa sobrescrever um `.exe`
/// em uso (ex.: a janela do mascote aberta), mas deixa renomeá-lo: o atual
/// vira `.old`, apagado quando possível (ou na próxima instalação).
fn replace(new: &Path, dest: &Path) -> Result<()> {
    let old = dest.with_extension("old");
    let _ = fs::remove_file(&old);
    if cfg!(windows) && dest.exists() {
        fs::rename(dest, &old).with_context(|| format!("renomeando {}", dest.display()))?;
    }
    fs::rename(new, dest).with_context(|| format!("instalando {}", dest.display()))?;
    let _ = fs::remove_file(&old);
    Ok(())
}

fn exe(bin: &str) -> String {
    format!("{bin}{}", env::consts::EXE_SUFFIX)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_is_known() {
        let pet = Manifest::known("pet").unwrap();
        assert_eq!(pet.bin, "terraforme-pet");
        assert_eq!(pet.repo, "Augustowtg/terraforme-pet");
        assert!(pet.validate().is_ok());
    }

    #[test]
    fn manifest_file_parses() {
        let m: Manifest = toml::from_str(
            r#"
name = "pet"
description = "Janela flutuante com o mascote do terraforme"
bin = "terraforme-pet"
repo = "Augustowtg/terraforme-pet"
min_terraforme = "0.24.0"
platforms = ["linux", "windows"]
"#,
        )
        .unwrap();
        assert_eq!(m, {
            let mut pet = Manifest::known("pet").unwrap();
            pet.min_terraforme = Some("0.24.0".into());
            pet
        });
    }

    #[test]
    fn validate_rejects_paths() {
        let mut m = Manifest::known("pet").unwrap();
        m.bin = "../evil".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn release_asset_is_zip_on_windows() {
        assert_eq!(
            Release::asset("terraforme-pet", "x86_64-pc-windows-msvc"),
            "terraforme-pet-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            Release::asset("terraforme-pet", "x86_64-unknown-linux-gnu"),
            "terraforme-pet-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn replace_swaps_the_binary() {
        let dir = env::temp_dir().join(format!("terraforme-replace-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (new, dest) = (dir.join("bin.new"), dir.join("bin.exe"));
        fs::write(&dest, "velho").unwrap();
        fs::write(&new, "novo").unwrap();
        replace(&new, &dest).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "novo");
        assert!(!new.exists() && !dir.join("bin.old").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_compares_numerically() {
        assert!(version("0.24.0") < version("0.100.0"));
        assert!(version("v1.2.3") == version("1.2.3"));
    }

    #[test]
    fn check_refuses_newer_terraforme() {
        let mut m = Manifest::known("pet").unwrap();
        m.platforms.clear();
        m.min_terraforme = Some("999.0.0".into());
        assert!(m.check().is_err());
        m.min_terraforme = Some("0.0.1".into());
        assert!(m.check().is_ok());
    }
}
