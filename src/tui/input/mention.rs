//! Menções de arquivo no input: digitar `@` abre uma lista dos arquivos e
//! pastas do diretório, filtrada pelo que vem depois do `@`; o escolhido vira
//! um badge cinza (`@/src/main.rs`) que se comporta como um caractere só.
//!
//! No texto do input, cada menção é um caractere da área de uso privado do
//! Unicode ([`token`]), que aponta para o caminho guardado pela caixa. Assim o
//! cursor, a seleção e o Backspace tratam o badge inteiro como uma unidade.
//!
//! Uma skill escolhida na lista da `/` vira um badge do mesmo jeito
//! (`/docs`), em qualquer ponto do texto. Um texto colado com mais de
//! [`PASTE_LIMIT`] caracteres também, como no Claude Code
//! (`[Pasted text #1 +12 lines]`); ele vai inteiro na mensagem.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::files::FileEntry;
use crate::theme;
use crate::tui::widgets::popup;

/// Primeiro caractere usado para menções (área de uso privado suplementar A).
const TOKEN_BASE: u32 = 0xF0000;
/// Máximo de resultados considerados no filtro.
const MAX_RESULTS: usize = 200;
/// Acima disso (em caracteres), um texto colado vira um badge.
pub const PASTE_LIMIT: usize = 100;

/// Caractere que representa a menção de índice `index`.
pub fn token(index: usize) -> char {
    char::from_u32(TOKEN_BASE + index as u32).expect("índice de menção válido")
}

/// Índice da menção, se `c` for um token.
pub fn token_index(c: char) -> Option<usize> {
    (c as u32)
        .checked_sub(TOKEN_BASE)
        .filter(|i| *i < 0xFFFE)
        .map(|i| i as usize)
}

/// O que um badge do input aponta: um arquivo do `@` ou uma skill da `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mention {
    /// Caminho relativo ao diretório (ex.: `src/main.rs`).
    File(String),
    /// Nome da skill, sem a `/` (ex.: `docs`).
    Skill(String),
    /// Texto colado grande: `number` é o `#` do badge (1, 2...).
    Paste { number: usize, text: String },
}

/// Texto do badge (sem os espaços do pill): `@/src/main.rs`, `/docs` ou
/// `[Pasted text #1 +12 lines]` (sem quebras de linha, só `[Pasted text #1]`).
pub fn badge_label(mention: &Mention) -> String {
    match mention {
        Mention::File(path) => format!("@/{path}"),
        Mention::Skill(name) => format!("/{name}"),
        Mention::Paste { number, text } => match text.matches('\n').count() {
            0 => format!("[Pasted text #{number}]"),
            lines => format!("[Pasted text #{number} +{lines} lines]"),
        },
    }
}

/// Colunas que o badge ocupa: o rótulo e um espaço de cada lado.
pub fn badge_width(mention: &Mention) -> usize {
    badge_label(mention).chars().count() + 2
}

/// O badge desenhado no input.
pub fn badge_span(mention: &Mention, selected: bool) -> Span<'static> {
    let style = if selected {
        theme::selection()
    } else {
        theme::mention()
    };
    Span::styled(format!(" {} ", badge_label(mention)), style)
}

/// Como a menção vai no texto enviado ao agente: `@src/main.rs`, `/docs` ou
/// o texto colado inteiro.
pub fn prompt_text(mention: &Mention) -> String {
    match mention {
        Mention::File(path) => format!("@{path}"),
        Mention::Skill(name) => format!("/{name}"),
        Mention::Paste { text, .. } => text.clone(),
    }
}

/// A lista aberta: onde está o `@` e qual item está selecionado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picker {
    /// Posição (em caracteres) do `@` no texto.
    pub start: usize,
    pub selected: usize,
}

impl Picker {
    pub fn new(start: usize) -> Self {
        Self { start, selected: 0 }
    }

    pub fn up(&mut self, total: usize) {
        if total > 0 {
            self.selected = (self.selected + total - 1) % total;
        }
    }

    pub fn down(&mut self, total: usize) {
        if total > 0 {
            self.selected = (self.selected + 1) % total;
        }
    }
}

/// Entradas que combinam com `query`, das melhores para as piores. Sem
/// consulta, as mais rasas primeiro.
pub fn matches<'a>(entries: &'a [FileEntry], query: &str) -> Vec<&'a FileEntry> {
    let query = query.to_lowercase();
    let mut scored: Vec<((u8, usize), &FileEntry)> = entries
        .iter()
        .filter_map(|entry| score(&entry.path, &query).map(|s| (s, entry)))
        .collect();
    scored.sort_by(|(a, ea), (b, eb)| a.cmp(b).then_with(|| ea.path.cmp(&eb.path)));
    scored
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, entry)| entry)
        .collect()
}

/// Quanto menor, melhor: (tipo de acerto, tamanho). Começo do nome do
/// arquivo > parte do nome > parte do caminho > letras na ordem.
fn score(path: &str, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((path.matches('/').count() as u8, path.len()));
    }
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let rank = if name.starts_with(query) {
        0
    } else if name.contains(query) {
        1
    } else if lower.contains(query) {
        2
    } else if is_subsequence(query, &lower) {
        3
    } else {
        return None;
    };
    Some((rank, path.len()))
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

/// Desenha a lista logo acima de `input` (a caixa de texto).
pub fn draw(frame: &mut Frame, input: Rect, found: &[&FileEntry], selected: usize) {
    let items = found
        .iter()
        .enumerate()
        .map(|(i, entry)| item(entry, i == selected))
        .collect();
    popup::draw_above(
        frame,
        input,
        format!("Arquivos ({})", found.len()),
        items,
        selected,
        "Nenhum arquivo encontrado",
    );
}

fn item(entry: &FileEntry, selected: bool) -> Line<'static> {
    let style = if selected {
        theme::focused()
    } else if entry.is_dir {
        theme::path()
    } else {
        theme::text()
    };
    let name = if entry.is_dir {
        format!("{}/", entry.path)
    } else {
        entry.path.clone()
    };
    let line = Line::from(vec![popup::marker(selected), Span::styled(name, style)]);
    if selected {
        line.style(theme::focused())
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[&str]) -> Vec<FileEntry> {
        paths
            .iter()
            .map(|p| FileEntry {
                path: p.trim_end_matches('/').to_string(),
                is_dir: p.ends_with('/'),
            })
            .collect()
    }

    fn found(paths: &[&str], query: &str) -> Vec<String> {
        let entries = entries(paths);
        matches(&entries, query)
            .into_iter()
            .map(|e| e.path.clone())
            .collect()
    }

    #[test]
    fn tokens_round_trip() {
        for i in [0, 1, 500] {
            assert_eq!(token_index(token(i)), Some(i));
        }
        assert_eq!(token_index('a'), None);
        assert_eq!(token_index('@'), None);
    }

    #[test]
    fn empty_query_lists_shallow_first() {
        assert_eq!(
            found(&["src/", "src/main.rs", "README.md", "src/tui/app.rs"], ""),
            ["src", "README.md", "src/main.rs", "src/tui/app.rs"]
        );
    }

    #[test]
    fn query_prefers_file_name_then_path_then_letters() {
        let paths = [
            "src/tui/app.rs",
            "src/apple.rs",
            "docs/maps.md",
            "a/p/p.txt",
        ];
        assert_eq!(
            found(&paths, "app"),
            ["src/apple.rs", "src/tui/app.rs", "a/p/p.txt"]
        );
        // Sem diferenciar maiúsculas.
        assert_eq!(found(&["README.md"], "read"), ["README.md"]);
        assert!(found(&paths, "xyz").is_empty());
    }

    #[test]
    fn picker_wraps_around() {
        let mut picker = Picker::new(0);
        picker.up(3);
        assert_eq!(picker.selected, 2);
        picker.down(3);
        assert_eq!(picker.selected, 0);
        picker.down(0);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn badge_label_and_prompt_text() {
        let file = Mention::File("src/test.js".into());
        assert_eq!(badge_label(&file), "@/src/test.js");
        assert_eq!(badge_width(&file), 15);
        assert_eq!(prompt_text(&file), "@src/test.js");
        let skill = Mention::Skill("docs".into());
        assert_eq!(badge_label(&skill), "/docs");
        assert_eq!(prompt_text(&skill), "/docs");
        let paste = Mention::Paste {
            number: 2,
            text: "a\nb\nc".into(),
        };
        assert_eq!(badge_label(&paste), "[Pasted text #2 +2 lines]");
        assert_eq!(prompt_text(&paste), "a\nb\nc");
        let line = Mention::Paste {
            number: 1,
            text: "x".repeat(150),
        };
        assert_eq!(badge_label(&line), "[Pasted text #1]");
    }
}
