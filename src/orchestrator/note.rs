//! A nota de handoff: o que um agente passa ao outro. Curta (até
//! [`MAX_LINES`] linhas) e aponta para arquivos em vez de copiar o
//! conteúdo deles.
//!
//! ```text
//! # Handoff <msg_id> · <origem> → <destino>
//! Objetivo:
//! Feito:
//! Arquivos:
//! Próximo passo:
//! Pronto quando:
//! ```

use std::path::PathBuf;

/// O máximo de linhas de uma nota.
pub const MAX_LINES: usize = 40;
/// As seções, na ordem.
pub const SECTIONS: [&str; 5] = [
    "Objetivo:",
    "Feito:",
    "Arquivos:",
    "Próximo passo:",
    "Pronto quando:",
];
const HEADER: &str = "# Handoff ";

fn header(id: &str, from: &str, to: &str) -> String {
    format!("{HEADER}{id} · {from} → {to}")
}

/// A nota em branco do `terra handoff new`, para o remetente preencher.
pub fn template(id: &str, from: &str, to: &str) -> String {
    let mut note = header(id, from, to);
    note.push('\n');
    for section in SECTIONS {
        note.push_str(section);
        note.push_str("\n\n");
    }
    note
}

/// A mini nota dos níveis 1 e 2: o texto é o objetivo e o próximo passo, e
/// as referências são os arquivos.
pub fn mini(id: &str, from: &str, to: &str, text: &str, refs: &[PathBuf]) -> String {
    let mut lines = vec![header(id, from, to), "Objetivo:".into()];
    lines.extend(text.lines().map(str::to_string));
    lines.push("Feito:".into());
    lines.push("-".into());
    lines.push("Arquivos:".into());
    if refs.is_empty() {
        lines.push("-".into());
    }
    lines.extend(refs.iter().map(|path| format!("- {}", path.display())));
    lines.push("Próximo passo:".into());
    lines.push("Fazer o objetivo acima.".into());
    lines.push("Pronto quando:".into());
    lines.push("O objetivo estiver feito.".into());
    lines.join("\n") + "\n"
}

/// O id da nota, do cabeçalho (`# Handoff <id> · …`).
pub fn id_of(note: &str) -> Option<&str> {
    let rest = note.lines().next()?.strip_prefix(HEADER)?;
    rest.split_whitespace()
        .next()
        .filter(|id| id.chars().all(|c| c.is_ascii_hexdigit()) && !id.is_empty())
}

/// Confere a nota do `--handoff`: o cabeçalho, as cinco seções e o tamanho.
pub fn check(note: &str) -> Result<(), String> {
    if !note.starts_with(HEADER) {
        return Err("a nota deve começar com \"# Handoff\" (crie com `terra handoff new`)".into());
    }
    let lines = note.lines().count();
    if lines > MAX_LINES {
        return Err(format!(
            "a nota tem {lines} linhas (máximo {MAX_LINES}): aponte para arquivos em vez de copiar"
        ));
    }
    let missing: Vec<&str> = SECTIONS
        .into_iter()
        .filter(|section| {
            !note
                .lines()
                .any(|line| line.trim_start().starts_with(section))
        })
        .collect();
    if !missing.is_empty() {
        return Err(format!("faltam seções na nota: {}", missing.join(" ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_passes_the_check_and_carries_its_id() {
        let note = template("ab12", "backend", "frontend");
        assert!(note.starts_with("# Handoff ab12 · backend → frontend\nObjetivo:"));
        check(&note).unwrap();
        assert_eq!(id_of(&note), Some("ab12"));
    }

    #[test]
    fn the_mini_note_points_to_the_files() {
        let refs = [PathBuf::from("/proj/a.rs"), PathBuf::from("/proj/b.rs")];
        let note = mini("ab12", "usuario", "backend", "Arrume o teste", &refs);
        check(&note).unwrap();
        assert!(note.contains("Objetivo:\nArrume o teste\n"));
        assert!(note.contains("Arquivos:\n- /proj/a.rs\n- /proj/b.rs\n"));
    }

    #[test]
    fn a_bad_note_is_refused_with_the_reason() {
        assert!(check("Objetivo: x").unwrap_err().contains("# Handoff"));
        let long = template("1", "a", "b") + &"linha\n".repeat(MAX_LINES);
        assert!(check(&long).unwrap_err().contains("máximo 40"));
        let partial = "# Handoff 1 · a → b\nObjetivo:\nFeito:\n";
        assert!(check(partial).unwrap_err().contains("Arquivos:"));
        assert_eq!(id_of("# Handoff nao-hex · a → b"), None);
    }
}
