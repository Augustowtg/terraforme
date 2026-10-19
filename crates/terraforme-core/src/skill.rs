//! Skills de agente embutidas no binário (pasta `skills/` do repositório).
//!
//! O `terraforme setup` copia cada skill para a pasta global de skills de cada
//! agente (ver [`crate::agent::Adapter::skills_dir`]). Os arquivos são embutidos
//! na compilação pelo `build.rs`, então a instalação funciona mesmo sem o
//! repositório por perto.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

include!(concat!(env!("OUT_DIR"), "/skills.rs"));

/// Nomes das skills embutidas (a pasta de primeiro nível em `skills/`).
pub fn names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = BUNDLED_FILES
        .iter()
        .filter_map(|&(path, _)| path.split('/').next())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Instala a skill `name` em `dest_root/<name>`, gravando (e sobrescrevendo)
/// todos os arquivos dela. Devolve o diretório da skill instalada.
pub fn install(name: &str, dest_root: &Path) -> Result<PathBuf> {
    if !names().contains(&name) {
        bail!("skill {name} não está embutida no terraforme");
    }
    let prefix = format!("{name}/");
    let dir = dest_root.join(name);
    let mut written = 0;
    for &(path, bytes) in BUNDLED_FILES {
        let Some(relative) = path.strip_prefix(&prefix) else {
            continue;
        };
        let target = dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("criando {}", parent.display()))?;
        }
        fs::write(&target, bytes).with_context(|| format!("gravando {}", target.display()))?;
        written += 1;
    }
    if written == 0 {
        bail!("a skill {name} não tem arquivos embutidos");
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_the_terraform_requisitos_skill() {
        let names = names();
        assert!(names.contains(&"terraform-requisitos"), "{names:?}");
        assert!(names.contains(&"terraforme-doc"), "{names:?}");
    }

    #[test]
    fn install_writes_the_skill_files() {
        let dest = std::env::temp_dir().join(format!("terraforme-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);

        let dir = install("terraform-requisitos", &dest).unwrap();
        assert_eq!(dir, dest.join("terraform-requisitos"));
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("scripts/memoria.py").is_file());
        // A parte da di2win foi removida da skill.
        assert!(!dir.join("scripts/gerar_pdf_di2win.py").exists());
        assert!(!dir.join("references/icons").exists());

        assert!(install("inexistente", &dest).is_err());
        fs::remove_dir_all(&dest).unwrap();
    }
}
