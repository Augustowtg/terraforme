//! Markdown nas respostas do agente: títulos, listas, citações, negrito,
//! itálico e `código` ficam estilizados, sem os marcadores. Blocos cercados
//! por ``` viram uma caixa de borda arredondada, como a do input, com a
//! linguagem (`md`, `rust`...) no canto superior esquerdo; os de código
//! saem realçados (ver `code`). Tabelas (`| a | b |` com a linha `|---|`)
//! viram uma caixa com as colunas alinhadas e o cabeçalho em destaque.

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use super::{Glyph, Row, code, glyphs, measure, wrap};
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
                let mut i = 0;
                while i < lines.len() {
                    let head = if rows.is_empty() { &prefix } else { &indent };
                    let table = table_len(&lines[i..]);
                    if table > 0 {
                        let cells: Vec<Vec<&str>> = lines[i..i + table]
                            .iter()
                            .enumerate()
                            .filter(|&(k, _)| k != 1)
                            .map(|(_, line)| cells(line))
                            .collect();
                        rows.extend(table_box(head.clone(), &indent, &cells, width, entry));
                        i += table;
                    } else {
                        rows.extend(wrap(head.clone(), line_glyphs(lines[i]), width, entry));
                        i += 1;
                    }
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

/// As células de uma linha `| a | b |`. shortcut: `\|` dentro da célula
/// também separa; tratar se aparecer em resposta real.
fn cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    line.split('|').map(str::trim).collect()
}

/// Quantas linhas do começo de `lines` formam uma tabela: o cabeçalho, a
/// linha `|---|---|` e as linhas seguintes que começam com `|`. 0 se não
/// houver tabela ali.
fn table_len(lines: &[&str]) -> usize {
    let row = |l: &&str| l.trim_start().starts_with('|');
    let separator = |l: &str| {
        row(&l)
            && cells(l).iter().all(|c| {
                let c = c.trim_matches(':');
                !c.is_empty() && c.chars().all(|ch| ch == '-')
            })
    };
    match lines {
        [head, sep, ..] if row(head) && separator(sep) => {
            2 + lines[2..].iter().take_while(|l| row(l)).count()
        }
        _ => 0,
    }
}

/// A tabela numa caixa de borda arredondada. Se as colunas cabem sem partir
/// palavras, ficam lado a lado (as largas quebram a linha); se não, cada
/// linha da tabela vira um cartão com `campo │ valor`, um por linha.
fn table_box(
    head: Vec<Glyph>,
    indent: &[Glyph],
    table: &[Vec<&str>],
    width: u16,
    entry: Option<usize>,
) -> Vec<Row> {
    let header = theme::variant(Variant::Accent).add_modifier(Modifier::BOLD);
    let columns = table.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let cell = |r: usize, c: usize, style| inline(table[r].get(c).copied().unwrap_or(""), style);
    // Cada coluna leva " x │": 3 colunas além do texto, mais a borda esquerda.
    let room = |columns: usize| (width as usize).saturating_sub(indent.len() + 1 + 3 * columns);

    let grid: Vec<Vec<Vec<Glyph>>> = (0..table.len())
        .map(|r| {
            let style = if r == 0 { header } else { theme::text() };
            (0..columns).map(|c| cell(r, c, style)).collect()
        })
        .collect();
    let natural: Vec<usize> = (0..columns)
        .map(|c| {
            grid.iter()
                .map(|row| measure(&row[c]))
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    let longest_word: Vec<usize> = (0..columns)
        .map(|c| {
            grid.iter()
                .flat_map(|row| row[c].split(|g| g.ch == ' ').map(measure))
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    if let Some(widths) = fit(&natural, &longest_word, room(columns)) {
        let separators: Vec<usize> = if grid.len() > 1 { vec![0] } else { Vec::new() };
        return draw_grid(head, indent, grid, &widths, &separators, entry);
    }

    // Cartões: o nome da coluna à esquerda e o valor à direita.
    let room = room(2).max(2);
    let label = (0..columns)
        .map(|c| measure(&cell(0, c, header)))
        .max()
        .unwrap_or(1)
        .clamp(1, (room / 3).max(1));
    let mut rows = Vec::new();
    let mut separators = Vec::new();
    for r in 1..table.len() {
        if r > 1 {
            separators.push(rows.len() - 1);
        }
        for c in 0..columns {
            rows.push(vec![cell(0, c, header), cell(r, c, theme::text())]);
        }
    }
    draw_grid(
        head,
        indent,
        rows,
        &[label, room - label],
        &separators,
        entry,
    )
}

/// As larguras das colunas, somando `room` (a tabela ocupa a largura toda):
/// as naturais mais a sobra, proporcional a elas, se couberem; se não, cada
/// uma com pelo menos a sua maior palavra e o que sobra vai, coluna a
/// coluna, para a mais estreita que ainda não chegou na natural. `None` se nem as maiores palavras couberem.
fn fit(natural: &[usize], minimum: &[usize], room: usize) -> Option<Vec<usize>> {
    let total: usize = natural.iter().sum();
    if total <= room {
        let extra = room - total;
        let mut widths: Vec<usize> = natural.iter().map(|n| n + extra * n / total).collect();
        if let Some(last) = widths.last_mut() {
            *last += room - (total + natural.iter().map(|n| extra * n / total).sum::<usize>());
        }
        return Some(widths);
    }
    let mut widths: Vec<usize> = minimum
        .iter()
        .zip(natural)
        .map(|(m, n)| *m.min(n))
        .collect();
    let mut extra = room.checked_sub(widths.iter().sum())?;
    while extra > 0 {
        let Some(c) = (0..widths.len())
            .filter(|&c| widths[c] < natural[c])
            .min_by_key(|&c| widths[c])
        else {
            break;
        };
        widths[c] += 1;
        extra -= 1;
    }
    Some(widths)
}

/// Desenha as linhas `grid` em colunas de `widths`, com um traço depois de
/// cada linha em `separators`. As células quebram por palavra.
fn draw_grid(
    head: Vec<Glyph>,
    indent: &[Glyph],
    grid: Vec<Vec<Vec<Glyph>>>,
    widths: &[usize],
    separators: &[usize],
    entry: Option<usize>,
) -> Vec<Row> {
    let border = theme::border();
    let line = |left: &str, mid: &str, right: &str, first: Option<Vec<Glyph>>| {
        let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        let mut gs = first.unwrap_or_else(|| indent.to_vec());
        gs.extend(glyphs(
            &format!("{left}{}{right}", parts.join(mid)),
            border,
            false,
        ));
        Row {
            glyphs: gs,
            wrapped: false,
            entry,
        }
    };
    let mut rows = vec![line("╭", "┬", "╮", Some(head))];
    for (r, row) in grid.into_iter().enumerate() {
        let wrapped: Vec<Vec<Vec<Glyph>>> = row
            .into_iter()
            .zip(widths)
            .map(|(cell, &w)| cell_lines(&cell, w))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for k in 0..height {
            let mut gs = indent.to_vec();
            gs.extend(glyphs("│", border, false));
            for (c, cell) in wrapped.iter().enumerate() {
                let body = cell.get(k).map_or(&[][..], |line| &line[..]);
                gs.extend(glyphs(" ", Style::new(), false));
                gs.extend(body.iter().copied());
                gs.extend(glyphs(
                    &" ".repeat(widths[c].saturating_sub(measure(body))),
                    Style::new(),
                    false,
                ));
                // Na cópia, as células da linha saem separadas por espaço.
                gs.push(Glyph {
                    ch: ' ',
                    style: Style::new(),
                    text: c + 1 < wrapped.len(),
                });
                gs.extend(glyphs("│", border, false));
            }
            rows.push(Row {
                glyphs: gs,
                wrapped: false,
                entry,
            });
        }
        if separators.contains(&r) {
            rows.push(line("├", "┼", "┤", None));
        }
    }
    rows.push(line("╰", "┴", "╯", None));
    rows
}

/// As linhas de uma célula em `width` colunas, quebrando por palavra (a
/// palavra maior que a coluna é partida).
fn cell_lines(cell: &[Glyph], width: usize) -> Vec<Vec<Glyph>> {
    let mut lines = Vec::new();
    let mut line: Vec<Glyph> = Vec::new();
    for word in cell.split(|g| g.ch == ' ').filter(|w| !w.is_empty()) {
        if !line.is_empty() && measure(&line) + 1 + measure(word) > width {
            lines.push(std::mem::take(&mut line));
        }
        if let Some(&last) = line.last() {
            line.push(Glyph { ch: ' ', ..last });
        }
        for &glyph in word {
            if !line.is_empty() && measure(&line) + glyph.ch.width().unwrap_or(0) > width {
                lines.push(std::mem::take(&mut line));
            }
            line.push(glyph);
        }
    }
    lines.push(line);
    lines
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
    fn table_fills_the_width_with_aligned_columns() {
        let rows = layout(
            "veja:\n| Nome | Qtd |\n|---|:--:|\n| **a** | 10 |\n| bb | 2 |\nfim",
            30,
            None,
        );
        assert_eq!(
            plain(&rows),
            [
                "⏺ veja:",
                "  ╭──────────────┬───────────╮",
                "  │ Nome         │ Qtd       │",
                "  ├──────────────┼───────────┤",
                "  │ a            │ 10        │",
                "  │ bb           │ 2         │",
                "  ╰──────────────┴───────────╯",
                "  fim",
            ]
        );
    }

    #[test]
    fn wide_table_shrinks_and_wraps_cells() {
        let rows = layout("| a | bbbb cccc dddd |\n|---|---|\n| x | y |", 16, None);
        assert!(
            rows.iter().all(|r| r.glyphs.len() <= 16),
            "{:?}",
            plain(&rows)
        );
        assert_eq!(plain(&rows)[0], "⏺ ╭───┬────────╮");
        assert_eq!(
            &plain(&rows)[1..4],
            ["  │ a │ bbbb   │", "  │   │ cccc   │", "  │   │ dddd   │"]
        );
    }

    #[test]
    fn table_too_narrow_for_its_words_becomes_cards() {
        let rows = layout(
            "| Campo | O que é |\n|---|---|\n| appointment_at | data do exame |\n| x | y |",
            24,
            None,
        );
        assert_eq!(
            plain(&rows),
            [
                "⏺ ╭───────┬────────────╮",
                "  │ Campo │ appointmen │",
                "  │       │ t_at       │",
                "  │ O que │ data do    │",
                "  │ é     │ exame      │",
                "  ├───────┼────────────┤",
                "  │ Campo │ x          │",
                "  │ O que │ y          │",
                "  │ é     │            │",
                "  ╰───────┴────────────╯",
            ]
        );
    }

    #[test]
    fn cells_wrap_by_word_without_edge_spaces() {
        let rows = layout("| a | um dois tres |\n|---|---|\n| b | c |", 17, None);
        assert_eq!(
            &plain(&rows)[1..4],
            [
                "  │ a │ um dois │",
                "  │   │ tres    │",
                "  ├───┼─────────┤"
            ]
        );
    }

    #[test]
    fn pipe_line_without_separator_stays_text() {
        assert_eq!(plain(&layout("| a | b |", 20, None)), ["⏺ | a | b |"]);
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
