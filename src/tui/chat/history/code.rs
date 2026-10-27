//! Realce dos blocos de código cercados por ```, como o markdown dos blocos
//! `md`: palavras-chave, tipos, strings, números e comentários ganham cor
//! conforme a linguagem (`rust`, `py`, `ts`, `sh`...).

use ratatui::style::{Modifier, Style};

use super::Glyph;
use crate::theme::{self, Variant};

/// Como os comentários e as palavras-chave de uma linguagem se escrevem.
struct Lang {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    keywords: &'static [&'static str],
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try",
    "while", "with", "yield",
];
const JS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];
const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];
const C: &[&str] = &[
    "auto",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "for",
    "if",
    "import",
    "namespace",
    "new",
    "null",
    "nullptr",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "union",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];
const SHELL: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in",
    "local", "return", "then", "until", "while",
];
const SQL: &[&str] = &[
    "and", "as", "by", "create", "delete", "from", "group", "insert", "into", "join", "left",
    "limit", "not", "null", "on", "or", "order", "select", "set", "table", "update", "values",
    "where",
];
const DATA: &[&str] = &["true", "false", "null"];

fn lang(name: &str) -> Lang {
    let slashes: &[&str] = &["//"];
    let (line, block, keywords): (&[&str], _, &[&str]) = match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => (slashes, Some(("/*", "*/")), RUST),
        "python" | "py" => (&["#"], None, PYTHON),
        "js" | "javascript" | "ts" | "typescript" | "jsx" | "tsx" => {
            (slashes, Some(("/*", "*/")), JS)
        }
        "go" => (slashes, Some(("/*", "*/")), GO),
        "c" | "h" | "cpp" | "c++" | "java" | "kotlin" | "kt" | "cs" | "csharp" | "swift" => {
            (slashes, Some(("/*", "*/")), C)
        }
        "sh" | "bash" | "zsh" | "shell" | "console" => (&["#"], None, SHELL),
        "sql" => (&["--"], Some(("/*", "*/")), SQL),
        "toml" | "yaml" | "yml" | "ini" => (&["#"], None, DATA),
        "json" | "jsonc" => (slashes, None, DATA),
        _ => (&[], None, &[]),
    };
    Lang {
        line,
        block,
        keywords,
    }
}

/// Realça as linhas de um bloco; um comentário de bloco pode passar de uma
/// linha para a outra.
pub(super) fn highlight(name: &str, lines: &[&str]) -> Vec<Vec<Glyph>> {
    let lang = lang(name);
    // Sem linguagem conhecida, o bloco fica sem realce.
    if lang.keywords.is_empty() {
        return lines
            .iter()
            .map(|line| super::glyphs(line, theme::text(), true))
            .collect();
    }
    let sql = lang.keywords == SQL;
    let mut in_block = false;
    lines
        .iter()
        .map(|line| line_glyphs(&lang, sql, line, &mut in_block))
        .collect()
}

fn line_glyphs(lang: &Lang, sql: bool, line: &str, in_block: &mut bool) -> Vec<Glyph> {
    let chars: Vec<char> = line.chars().collect();
    let at = |i: usize, s: &str| {
        chars[i..]
            .iter()
            .copied()
            .take(s.chars().count())
            .eq(s.chars())
    };
    let comment = theme::muted().add_modifier(Modifier::ITALIC);
    let mut out = Vec::new();
    let push = |out: &mut Vec<Glyph>, from: usize, to: usize, style: Style| {
        out.extend(chars[from..to].iter().map(|&ch| Glyph {
            ch,
            style,
            text: true,
        }));
    };
    let mut i = 0;
    while i < chars.len() {
        if *in_block {
            let close = lang.block.map_or("", |b| b.1);
            let end = (i..chars.len())
                .find(|&j| at(j, close))
                .map_or(chars.len(), |j| {
                    *in_block = false;
                    j + close.chars().count()
                });
            push(&mut out, i, end, comment);
            i = end;
            continue;
        }
        if let Some((open, _)) = lang.block.filter(|b| at(i, b.0)) {
            *in_block = true;
            push(&mut out, i, i + open.chars().count(), comment);
            i += open.chars().count();
            continue;
        }
        // `#` só abre comentário no começo da linha ou depois de espaço
        // (no shell, `$#` e `${#x}` não são comentário).
        let starts = |m: &&str| at(i, m) && (*m != "#" || i == 0 || chars[i - 1].is_whitespace());
        if lang.line.iter().any(starts) {
            push(&mut out, i, chars.len(), comment);
            break;
        }
        let c = chars[i];
        if matches!(c, '"' | '\'' | '`') && !lang.keywords.is_empty() {
            // Em Rust, `'a` é um lifetime, não uma string.
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(chars.len());
            // Um char é `'x'` ou `'\n'`; o resto, como `'a` ou `'static`, é lifetime.
            let char_literal = j < chars.len() && (j == i + 2 || chars.get(i + 1) == Some(&'\\'));
            let lifetime = c == '\'' && lang.keywords == RUST && !char_literal;
            if !lifetime {
                push(&mut out, i, end, theme::variant(Variant::Success));
                i = end;
                continue;
            }
        }
        if c.is_ascii_digit() && !chars.get(i.wrapping_sub(1)).is_some_and(|p| is_word(*p)) {
            let end = (i..chars.len())
                .find(|&j| !(chars[j].is_ascii_alphanumeric() || matches!(chars[j], '.' | '_')))
                .unwrap_or(chars.len());
            push(&mut out, i, end, theme::variant(Variant::Warning));
            i = end;
            continue;
        }
        if is_word(c) {
            let end = (i..chars.len())
                .find(|&j| !is_word(chars[j]))
                .unwrap_or(chars.len());
            let word: String = chars[i..end].iter().collect();
            let keyword = if sql {
                lang.keywords.contains(&word.to_ascii_lowercase().as_str())
            } else {
                lang.keywords.contains(&word.as_str())
            };
            let style = if keyword {
                theme::variant(Variant::Accent).add_modifier(Modifier::BOLD)
            } else if !lang.keywords.is_empty() && word.starts_with(|c: char| c.is_uppercase()) {
                theme::variant(Variant::Active)
            } else if chars.get(end) == Some(&'(')
                || (lang.keywords == RUST && chars.get(end) == Some(&'!'))
            {
                theme::variant(Variant::Secondary).add_modifier(Modifier::BOLD)
            } else {
                theme::text()
            };
            push(&mut out, i, end, style);
            i = end;
            continue;
        }
        push(&mut out, i, i + 1, theme::text());
        i += 1;
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_of(gs: &[Glyph], text: &str, ch: char) -> Style {
        let s: String = gs.iter().map(|g| g.ch).collect();
        let at = s
            .chars()
            .zip(0..)
            .find_map(|(c, i)| (c == ch).then_some(i))
            .unwrap();
        assert_eq!(s, text);
        gs[at].style
    }

    #[test]
    fn keywords_strings_numbers_and_comments() {
        let text = "let x = \"s\" + 42; // c";
        let gs = &highlight("rust", &[text])[0];
        let keyword = theme::variant(Variant::Accent).add_modifier(Modifier::BOLD);
        assert_eq!(style_of(gs, text, 'l'), keyword);
        assert_eq!(style_of(gs, text, 's'), theme::variant(Variant::Success));
        assert_eq!(style_of(gs, text, '4'), theme::variant(Variant::Warning));
        assert_eq!(
            style_of(gs, text, 'c'),
            theme::muted().add_modifier(Modifier::ITALIC)
        );
        assert_eq!(style_of(gs, text, 'x'), theme::text());
    }

    #[test]
    fn block_comment_spans_lines() {
        let rows = highlight("ts", &["a /* b", "c */ d"]);
        let comment = theme::muted().add_modifier(Modifier::ITALIC);
        assert_eq!(rows[1][0].style, comment);
        assert_eq!(rows[1].last().unwrap().style, theme::text());
    }

    #[test]
    fn unknown_language_stays_plain() {
        let gs = &highlight("", &["if \"x\" 1"])[0];
        assert!(gs.iter().all(|g| g.style == theme::text()));
    }

    #[test]
    fn rust_char_literals_and_lifetimes() {
        let string = theme::variant(Variant::Success);
        let text = "'\\n' &'a str";
        let gs = &highlight("rust", &[text])[0];
        assert_eq!(gs[1].style, string);
        assert_ne!(style_of(gs, text, 'a'), string);
    }

    #[test]
    fn shell_hash_inside_word_is_not_comment() {
        let text = "echo $# ${#x} # c";
        let gs = &highlight("sh", &[text])[0];
        let comment = theme::muted().add_modifier(Modifier::ITALIC);
        assert_ne!(gs[6].style, comment);
        assert_eq!(gs.last().unwrap().style, comment);
    }
}
