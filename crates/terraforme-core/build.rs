//! Embute as skills do repositório (`<workspace>/skills`) no binário.
//!
//! Gera `$OUT_DIR/skills.rs` com um `BUNDLED_FILES: &[(caminho, bytes)]`, para
//! o `terraforme setup` instalar as skills mesmo sem o repositório ao lado.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // crates/terraforme-core → raiz do workspace → skills/
    let skills = manifest.join("../../skills");
    println!("cargo:rerun-if-changed={}", skills.display());

    let mut files = Vec::new();
    collect(&skills, &skills, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("/// Arquivos das skills embutidos na compilação (caminho relativo → bytes).\n");
    out.push_str("pub static BUNDLED_FILES: &[(&str, &[u8])] = &[\n");
    for (relative, absolute) in &files {
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?})),",
            relative,
            absolute.to_string_lossy()
        )
        .expect("escrevendo a lista de skills");
    }
    out.push_str("];\n");

    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("skills.rs");
    fs::write(&dest, out).expect("gravando skills.rs");
}

/// Percorre `dir` recursivamente e acumula `(caminho relativo, absoluto)`,
/// ignorando `__pycache__` e arquivos gerados.
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            if let Ok(relative) = path.strip_prefix(root) {
                out.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
}
