//! Markdown nas respostas do agente: títulos, listas, citações, negrito,
//! itálico e `código` ficam estilizados, sem os marcadores. Blocos cercados
//! por ``` viram uma caixa de borda arredondada, como a do input, com a
//! linguagem (`md`, `rust`...) no canto superior esquerdo; os de código
//! saem realçados (ver `code`).

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use super::{Glyph, Row, code, glyphs, wrap};
use crate::theme::{self, Variant};

/// Um trecho da resposta: texto corrido ou bloco cercado por ```.
#[derive(Debug, PartialEq, Eq)]
enum Segment<'a> {
    Text(Vec<&'a str>),
    Fence { lang: &'a str, lines: Vec<&'a str> },
}

/// Separa os blocos ```; um bloco ainda aberto (streaming) vai até o fim.
fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut prose: Vec<&str> = Vec::new();
    let mut fence: Option<(&str, Vec<&str>)> = None;
    for line in text.split('\n') {
        let fenced = line.trim_start().strip_prefix("```");
        match (&mut fence, fenced) {
            (Some(_), Some(rest)) if rest.trim().is_empty() => {
                let (lang, lines) = fence.take().unwrap();
                out.push(Segment::Fence { lang, lines });
            }
            (Some((_, lines)), _) => lines.push(line),
            (None, Some(lang)) => {
                if !prose.is_empty() {
                    out.push(Segment::Text(std::mem::take(&mut prose)));
                }
                fence = Some((lang.trim(), Vec::new()));
            }
            (None, None) => prose.push(line),
        }
    }
    if let Some((lang, lines)) = fence {
        out.push(Segment::Fence { lang, lines });
    }
    if !prose.is_empty() {
        out.push(Segment::Text(prose));
    }
    out
}

/// As linhas de uma resposta do agente em `width` colunas; a primeira leva
/// o prefixo `⏺ ` e as outras ficam alinhadas a ele.
pub(super) fn layout(text: &str, width: u16, entry: Option<usize>) -> Vec<Row> {
    let prefix = glyphs("⏺ ", theme::variant(Variant::Accent), false);
    let indent = glyphs("  ", Style::new(), false);
    let mut rows: Vec<Row> = Vec::new();
    for segment in segments(text) {
        match segment {
            Segment::Text(lines) => {
                for line in lines {
                    let head = if rows.is_empty() { &prefix } else { &indent };
                    rows.extend(wrap(head.clone(), line_glyphs(line), width, entry));
                }
            }
            Segment::Fence { lang, lines } => {
                let head = if rows.is_empty() { &prefix } else { &indent };
                rows.extend(boxed(head.clone(), &indent, lang, &lines, width, entry));
            }
        }
    }
    rows
}

/// Caixa de borda arredondada com a linguagem no canto superior esquerdo.
fn boxed(
    head: Vec<Glyph>,
    indent: &[Glyph],
    lang: &str,
    lines: &[&str],
    width: u16,
    entry: Option<usize>,
) -> Vec<Row> {
    let border = theme::border();
    let label = if lang.is_empty() { "código" } else { lang };
    // Borda + espaço de cada lado.
    let outer = (width as usize).saturating_sub(indent.len()).max(6);
    let inner = outer - 4;
    let markdown = matches!(lang, "md" | "markdown");

    let mut top = head;
    top.extend(glyphs("╭─ ", border, false));
    top.extend(glyphs(
        label,
        theme::variant(Variant::Accent).add_modifier(Modifier::BOLD),
        false,
    ));
    top.push(Glyph {
        ch: ' ',
        style: border,
        text: false,
    });
    let used = 4 + label.chars().count();
    top.extend(glyphs(
        &"─".repeat(outer.saturating_sub(used + 1)),
        border,
        false,
    ));
    top.extend(glyphs("╮", border, false));
    let mut rows = vec![Row {
        glyphs: top,
        wrapped: false,
        entry,
    }];

    let bodies: Vec<Vec<Glyph>> = if markdown {
        lines.iter().map(|line| line_glyphs(line)).collect()
    } else {
        code::highlight(lang, lines)
    };
    for body in bodies {
        for row in wrap(Vec::new(), body, inner as u16, entry) {
            let used: usize = row.glyphs.iter().map(|g| g.ch.width().unwrap_or(0)).sum();
            let mut glyphs_row = indent.to_vec();
            glyphs_row.extend(glyphs("│ ", border, false));
            glyphs_row.extend(row.glyphs);
            glyphs_row.extend(glyphs(
                &" ".repeat(inner.saturating_sub(used)),
                Style::new(),
                false,
            ));
            glyphs_row.extend(glyphs(" │", border, false));
            rows.push(Row {
                glyphs: glyphs_row,
                wrapped: row.wrapped,
                entry,
            });
        }
    }

    let mut bottom = indent.to_vec();
    bottom.extend(glyphs(
        &format!("╰{}╯", "─".repeat(outer - 2)),
        border,
        false,
    ));
    rows.push(Row {
        glyphs: bottom,
        wrapped: false,
        entry,
    });
    rows
}

/// Uma linha de markdown: título, item de lista ou citação, com os
/// marcadores trocados pelo estilo.
fn line_glyphs(line: &str) -> Vec<Glyph> {
    let trimmed = line.trim_start();
    let lead = &line[..line.len() - trimmed.len()];
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        let style = theme::variant(Variant::Accent).add_modifier(Modifier::BOLD);
        return inline(trimmed[hashes..].trim_start(), style);
    }
    if let Some(rest) = trimmed.strip_prefix("> ") {
        let mut out = glyphs("▎ ", theme::muted(), false);
        out.extend(inline(rest, theme::muted().add_modifier(Modifier::ITALIC)));
        return out;
    }
    if let Some(rest) = ["- ", "* ", "+ "]
        .iter()
        .find_map(|m| trimmed.strip_prefix(m))
    {
        let mut out = glyphs(lead, theme::text(), true);
        out.extend(glyphs("• ", theme::variant(Variant::Accent), true));
        out.extend(inline(rest, theme::text()));
        return out;
    }
    if matches!(trimmed, "---" | "***" | "___") {
        return glyphs(&"─".repeat(20), theme::border(), false);
    }
    inline(line, theme::text())
}

/// Negrito (`**`), itálico (`*`/`_`) e `código` dentro de uma linha.
fn inline(text: &str, base: Style) -> Vec<Glyph> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut bold, mut italic, mut code) = (false, false, false);
    let closes =
        |from: usize, marker: &[char]| (from..chars.len()).any(|j| chars[j..].starts_with(marker));
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' && (code || closes(i + 1, &['`'])) {
            code = !code;
            i += 1;
            continue;
        }
        if !code {
            if chars[i..].starts_with(&['*', '*']) && (bold || closes(i + 2, &['*', '*'])) {
                bold = !bold;
                i += 2;
                continue;
            }
            // `_` só marca itálico na borda da palavra (snake_case fica).
            let alnum = |k: Option<usize>| {
                k.and_then(|k| chars.get(k))
                    .is_some_and(|c| c.is_alphanumeric())
            };
            let edge = c == '*'
                || (c == '_'
                    && if italic {
                        !alnum(Some(i + 1))
                    } else {
                        !alnum(i.checked_sub(1))
                    });
            let opens = chars.get(i + 1).is_some_and(|n| !n.is_whitespace()) && closes(i + 1, &[c]);
            if (c == '*' || c == '_') && edge && (italic || opens) {
                italic = !italic;
                i += 1;
                continue;
            }
        }
        let style = if code {
            theme::mention()
        } else {
            let mut s = base;
            if bold {
                s = s.add_modifier(Modifier::BOLD);
            }
            if italic {
                s = s.add_modifier(Modifier::ITALIC);
            }
            s
        };
        out.push(Glyph {
            ch: c,
            style,
            text: true,
        });
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                r.glyphs
                    .iter()
                    .map(|g| g.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn splits_fences_even_when_still_open() {
        assert_eq!(
            segments("oi\n```md\n# t\n```\nfim\n```rust\nfn"),
            [
                Segment::Text(vec!["oi"]),
                Segment::Fence {
                    lang: "md",
                    lines: vec!["# t"]
                },
                Segment::Text(vec!["fim"]),
                Segment::Fence {
                    lang: "rust",
                    lines: vec!["fn"]
                },
            ]
        );
    }

    #[test]
    fn fence_becomes_box_with_language_label() {
        let rows = layout("veja:\n```md\n# Título\n- item\n```", 20, None);
        assert_eq!(
            plain(&rows),
            [
                "⏺ veja:",
                "  ╭─ md ───────────╮",
                "  │ Título         │",
                "  │ • item         │",
                "  ╰────────────────╯",
            ]
        );
    }

    #[test]
    fn inline_markers_become_styles() {
        let gs = inline("a **b** *c* `d` snake_case", theme::text());
        let text: String = gs.iter().map(|g| g.ch).collect();
        assert_eq!(text, "a b c d snake_case");
        let at = |ch| gs.iter().find(|g| g.ch == ch).unwrap().style;
        assert!(at('b').add_modifier.contains(Modifier::BOLD));
        assert!(at('c').add_modifier.contains(Modifier::ITALIC));
        assert_eq!(at('d'), theme::mention());
        assert!(!at('k').add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn border_is_left_out_of_copy() {
        let rows = layout("```\nx\n```", 20, None);
        let copied: String = rows
            .iter()
            .flat_map(|r| r.glyphs.iter())
            .filter(|g| g.text)
            .map(|g| g.ch)
            .collect();
        assert_eq!(copied, "x");
    }
}
