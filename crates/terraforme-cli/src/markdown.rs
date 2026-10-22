//! Markdown das respostas do modo Middleware ([`crate::chat`]): títulos,
//! listas, citações, ênfase, código em linha e blocos de código com a cor da
//! linguagem ([`syntect`], sintaxes do `bat` via [`two_face`]). As cores são
//! as do tema VS-Claude (<https://github.com/EstarinAzx/VS-Claude>).

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use unicode_width::UnicodeWidthChar;

/// Paleta do VS-Claude.
pub mod palette {
    use ratatui::style::Color;

    pub const TEXT: Color = Color::Rgb(0xF6, 0xF6, 0xF4);
    pub const MUTED: Color = Color::Rgb(0x8F, 0x8D, 0x83);
    pub const ACCENT: Color = Color::Rgb(0xA6, 0x5F, 0x47);
    pub const PANEL: Color = Color::Rgb(0x26, 0x26, 0x26);
    pub const STRING: Color = Color::Rgb(0x86, 0xE8, 0x9A);
    pub const FUNCTION: Color = Color::Rgb(0xFF, 0xA6, 0x5D);
    pub const TYPE: Color = Color::Rgb(0x7A, 0xB8, 0xFF);
    pub const QUOTE: Color = Color::Rgb(0x9C, 0x98, 0x93);
}

use palette::*;

/// Escopos do tema do VS-Claude (`themes/vs-claude-color-theme.json`):
/// seletores, cor e estilo (`b` negrito, `i` itálico, `u` sublinhado).
const THEME: &[(&str, u32, &str)] = &[
    ("comment, punctuation.definition.comment", 0x969696, ""),
    (
        "string, punctuation.definition.string, string.template",
        0x86E89A,
        "",
    ),
    (
        "constant.character.escape, constant.other.placeholder",
        0xFF85C8,
        "",
    ),
    (
        "constant.numeric, constant.language, variable.other.constant, entity.name.constant, \
         support.constant, support.variable",
        0xFFDF61,
        "",
    ),
    (
        "keyword, keyword.control, keyword.operator.word, keyword.operator.new, storage.type, \
         storage.modifier, variable.language, punctuation.definition.keyword",
        0xC79BFF,
        "b",
    ),
    (
        "keyword.operator, punctuation.accessor, punctuation.separator, \
         punctuation.terminator, storage.type.function.arrow, variable.other.enummember",
        0x5DE4FA,
        "",
    ),
    (
        "entity.name.function, meta.function-call, support.function, variable.function, \
         entity.other.attribute-name",
        0xFFA65D,
        "",
    ),
    (
        "entity.name.class, entity.name.type, entity.other.inherited-class, support.class, \
         support.type, entity.name.namespace, entity.name.module, entity.name.interface, \
         entity.name.trait, entity.name.enum",
        0x7AB8FF,
        "",
    ),
    (
        "variable.parameter, entity.name.tag, markup.deleted",
        0xA65F47,
        "",
    ),
    (
        "variable.other.property, meta.property.object, support.type.property-name, \
         meta.object-literal.key, markup.quote",
        0x9C9893,
        "",
    ),
    (
        "meta.decorator, meta.annotation, punctuation.decorator, storage.type.annotation, \
         string.regexp",
        0xFF85C8,
        "",
    ),
    ("punctuation, meta.brace", 0x7A7A7A, ""),
    (
        "markup.heading, punctuation.definition.heading",
        0x7AB8FF,
        "b",
    ),
    ("markup.bold", 0xFFA65D, "b"),
    ("markup.italic", 0xFFA65D, "i"),
    (
        "markup.inline.raw, markup.raw, markup.inserted",
        0x86E89A,
        "",
    ),
    ("markup.underline.link, string.other.link", 0x7AB8FF, "u"),
    ("markup.list, punctuation.definition.list", 0xA65F47, ""),
    ("markup.changed", 0xFFDF61, ""),
    ("invalid, invalid.illegal", 0xFFFFFF, "bu"),
];

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn theme() -> &'static Theme {
    static THEME_: OnceLock<Theme> = OnceLock::new();
    THEME_.get_or_init(|| {
        let rgb = |c: u32| syntect::highlighting::Color {
            r: (c >> 16) as u8,
            g: (c >> 8) as u8,
            b: c as u8,
            a: 0xFF,
        };
        let scopes = THEME
            .iter()
            .filter_map(|(selectors, color, style)| {
                let mut font = FontStyle::empty();
                for (flag, bit) in [
                    ('b', FontStyle::BOLD),
                    ('i', FontStyle::ITALIC),
                    ('u', FontStyle::UNDERLINE),
                ] {
                    if style.contains(flag) {
                        font |= bit;
                    }
                }
                Some(ThemeItem {
                    scope: selectors.parse::<ScopeSelectors>().ok()?,
                    style: StyleModifier {
                        foreground: Some(rgb(*color)),
                        background: None,
                        font_style: Some(font),
                    },
                })
            })
            .collect();
        Theme {
            name: Some("VS-Claude".into()),
            author: None,
            settings: ThemeSettings {
                foreground: Some(rgb(0xF6F6F4)),
                ..ThemeSettings::default()
            },
            scopes,
        }
    })
}

/// Sintaxe da linguagem do bloco (nome ou extensão: `rust`, `rs`, `ts`…).
fn syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let lang = lang.trim();
    if lang.is_empty() {
        return None;
    }
    let alias = match lang.to_ascii_lowercase().as_str() {
        "shell" | "console" | "zsh" => "bash",
        "typescript" => "ts",
        "javascript" | "node" => "js",
        "python" | "py3" => "py",
        "golang" => "go",
        "c++" => "cpp",
        "c#" | "csharp" => "cs",
        "yml" => "yaml",
        "jsonc" => "json",
        _ => lang,
    }
    .to_string();
    syntaxes().find_syntax_by_token(&alias)
}

/// Uma linha de código com a cor da linguagem (texto puro sem ela).
fn code_spans(highlighter: Option<&mut HighlightLines>, line: &str) -> Vec<Span<'static>> {
    let base = Style::new().fg(TEXT).bg(PANEL);
    let Some(highlighter) = highlighter else {
        return vec![Span::styled(line.to_string(), base)];
    };
    let with_newline = format!("{line}\n");
    match highlighter.highlight_line(&with_newline, syntaxes()) {
        Ok(ranges) => ranges
            .into_iter()
            .filter_map(|(style, text)| {
                let text = text.trim_end_matches('\n');
                if text.is_empty() {
                    return None;
                }
                let fg = style.foreground;
                let mut out = base.fg(Color::Rgb(fg.r, fg.g, fg.b));
                if style.font_style.contains(FontStyle::BOLD) {
                    out = out.add_modifier(Modifier::BOLD);
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    out = out.add_modifier(Modifier::ITALIC);
                }
                if style.font_style.contains(FontStyle::UNDERLINE) {
                    out = out.add_modifier(Modifier::UNDERLINED);
                }
                Some(Span::styled(text.to_string(), out))
            })
            .collect(),
        Err(_) => vec![Span::styled(line.to_string(), base)],
    }
}

/// Texto em linha: `código`, **negrito**, *itálico* e [links](url).
pub fn inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>| {
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(plain), base));
        }
    };
    // Fim do delimitador `close` a partir de `from`, na mesma linha.
    let find = |from: usize, close: &[char]| -> Option<usize> {
        (from..chars.len().saturating_sub(close.len() - 1))
            .find(|&j| chars[j..].starts_with(close) && j > from)
    };
    while i < chars.len() {
        let c = chars[i];
        let rest = &chars[i..];
        if c == '`'
            && let Some(end) = find(i + 1, &['`'])
        {
            flush(&mut plain, &mut spans);
            let code: String = chars[i + 1..end].iter().collect();
            spans.push(Span::styled(code, base.fg(STRING)));
            i = end + 1;
        } else if (rest.starts_with(&['*', '*']) || rest.starts_with(&['_', '_']))
            && let Some(end) = find(i + 2, &[c, c])
        {
            flush(&mut plain, &mut spans);
            let inner: String = chars[i + 2..end].iter().collect();
            spans.extend(inline(
                &inner,
                base.fg(FUNCTION).add_modifier(Modifier::BOLD),
            ));
            i = end + 2;
        } else if c == '*'
            && rest.get(1).is_some_and(|n| !n.is_whitespace())
            && let Some(end) = find(i + 1, &['*'])
        {
            flush(&mut plain, &mut spans);
            let inner: String = chars[i + 1..end].iter().collect();
            spans.extend(inline(
                &inner,
                base.fg(FUNCTION).add_modifier(Modifier::ITALIC),
            ));
            i = end + 1;
        } else if c == '['
            && let Some(close) = find(i + 1, &[']', '('])
            && let Some(end) = find(close + 2, &[')'])
        {
            flush(&mut plain, &mut spans);
            let label: String = chars[i + 1..close].iter().collect();
            spans.push(Span::styled(
                label,
                base.fg(TYPE).add_modifier(Modifier::UNDERLINED),
            ));
            i = end + 1;
        } else {
            plain.push(c);
            i += 1;
        }
    }
    flush(&mut plain, &mut spans);
    spans
}

fn width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Quebra os trechos coloridos em linhas de até `max` colunas, nos espaços
/// quando dá. Cada linha ganha `first` na frente (a primeira) ou `indent`.
pub fn wrap_spans(
    spans: Vec<Span<'static>>,
    max: usize,
    first: Span<'static>,
    indent: Span<'static>,
) -> Vec<Line<'static>> {
    let room = max.saturating_sub(first.width()).max(1);
    // Palavras: pedaços com estilo até um espaço (inclusive).
    let mut words: Vec<Vec<(String, Style)>> = vec![Vec::new()];
    for span in &spans {
        for piece in span.content.split_inclusive(' ') {
            words
                .last_mut()
                .expect("sempre há uma palavra")
                .push((piece.to_string(), span.style));
            if piece.ends_with(' ') {
                words.push(Vec::new());
            }
        }
    }
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0;
    let push = |lines: &mut Vec<Vec<Span<'static>>>, text: String, style: Style| {
        lines
            .last_mut()
            .expect("sempre há uma linha")
            .push(Span::styled(text, style));
    };
    for word in words.into_iter().filter(|w| !w.is_empty()) {
        let fits: usize = word.iter().map(|(t, _)| width(t.trim_end())).sum();
        if used > 0 && used + fits > room {
            lines.push(Vec::new());
            used = 0;
        }
        for (text, style) in word {
            if used + width(&text) <= room || fits <= room {
                used += width(&text);
                push(&mut lines, text, style);
                continue;
            }
            // Palavra maior que a linha: corta nos caracteres.
            let mut chunk = String::new();
            for c in text.chars() {
                let w = c.width().unwrap_or(0);
                if used + w > room && used > 0 {
                    push(&mut lines, std::mem::take(&mut chunk), style);
                    lines.push(Vec::new());
                    used = 0;
                }
                chunk.push(c);
                used += w;
            }
            push(&mut lines, chunk, style);
        }
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(i, mut spans)| {
            // Sem o espaço do fim da linha.
            if let Some(last) = spans.last_mut() {
                last.content = last.content.trim_end().to_string().into();
            }
            spans.insert(
                0,
                if i == 0 {
                    first.clone()
                } else {
                    indent.clone()
                },
            );
            Line::from(spans)
        })
        .collect()
}

/// Item de lista: recuo, marcador (`-`, `*`, `+` ou `1.`) e o texto.
fn list_item(line: &str) -> Option<(usize, String, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(bullet) {
            let mark = if let Some(task) = text.strip_prefix("[ ] ") {
                return Some((indent, "☐ ".into(), task));
            } else if let Some(task) = text
                .strip_prefix("[x] ")
                .or_else(|| text.strip_prefix("[X] "))
            {
                return Some((indent, "☑ ".into(), task));
            } else {
                "• ".to_string()
            };
            return Some((indent, mark, text));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits < 4 {
        let after = &rest[digits..];
        if let Some(text) = after.strip_prefix(". ").or(after.strip_prefix(") ")) {
            return Some((indent, format!("{}. ", &rest[..digits]), text));
        }
    }
    None
}

/// Cerca de bloco de código (` ``` ` ou `~~~`) e a linguagem dela.
fn fence(line: &str) -> Option<&str> {
    let line = line.trim_start();
    line.strip_prefix("```")
        .or_else(|| line.strip_prefix("~~~"))
        .map(str::trim)
}

/// O Markdown em linhas de até `max` colunas.
pub fn render(text: &str, max: usize) -> Vec<Line<'static>> {
    let max = max.max(1);
    let base = Style::new().fg(TEXT);
    let mut out = Vec::new();
    // Bloco de código aberto: o colorizador da linguagem (se conhecida).
    let mut code: Option<Option<HighlightLines>> = None;
    for line in text.split('\n') {
        if let Some(lang) = fence(line) {
            match code {
                Some(_) => code = None,
                None => {
                    let syntax = syntax(lang);
                    let label = syntax.map_or(lang, |s| s.name.as_str());
                    if !label.is_empty() {
                        out.push(Line::styled(
                            format!(" {label} "),
                            Style::new()
                                .fg(MUTED)
                                .bg(PANEL)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    }
                    code = Some(syntax.map(|s| HighlightLines::new(s, theme())));
                }
            }
            continue;
        }
        if let Some(highlighter) = &mut code {
            let spans = code_spans(highlighter.as_mut(), &line.replace('\t', "    "));
            out.extend(code_lines(spans, max));
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push(Line::default());
        } else if let Some((level, title)) = heading(trimmed) {
            let mut style = base.fg(TYPE).add_modifier(Modifier::BOLD);
            if level == 1 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            out.extend(wrap_spans(
                inline(title, style),
                max,
                Span::raw(""),
                Span::raw(""),
            ));
        } else if trimmed.len() >= 3
            && ["-", "*", "_"]
                .iter()
                .any(|c| trimmed.chars().all(|x| x.to_string() == *c || x == ' '))
        {
            out.push(Line::styled(
                "─".repeat(max.min(60)),
                Style::new().fg(MUTED),
            ));
        } else if let Some(quote) = trimmed.strip_prefix('>') {
            out.extend(wrap_spans(
                inline(
                    quote.trim_start(),
                    base.fg(QUOTE).add_modifier(Modifier::ITALIC),
                ),
                max,
                Span::styled("▎ ", Style::new().fg(ACCENT)),
                Span::styled("▎ ", Style::new().fg(ACCENT)),
            ));
        } else if let Some((indent, mark, item)) = list_item(line) {
            let pad = " ".repeat(indent.min(12));
            let first = Span::styled(format!("{pad}{mark}"), Style::new().fg(ACCENT));
            let rest = Span::raw(" ".repeat(first.width()));
            out.extend(wrap_spans(inline(item, base), max, first, rest));
        } else if trimmed.starts_with('|') {
            out.push(table_row(trimmed, base));
        } else {
            out.extend(wrap_spans(
                inline(line, base),
                max,
                Span::raw(""),
                Span::raw(""),
            ));
        }
    }
    out
}

/// Título (`#` a `######`): o nível e o texto.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let title = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, title.trim()))
}

/// Linha de tabela: as barras em cinza; a de separação inteira.
fn table_row(line: &str, base: Style) -> Line<'static> {
    let border = Style::new().fg(MUTED);
    if line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
        return Line::styled(line.to_string(), border);
    }
    let mut spans = Vec::new();
    for (i, cell) in line.split('|').enumerate() {
        if i > 0 {
            spans.push(Span::styled("|", border));
        }
        spans.extend(inline(cell, base));
    }
    Line::from(spans)
}

/// Linha de código cortada na largura (sem quebrar em palavras), com o
/// fundo do painel até a borda.
fn code_lines(spans: Vec<Span<'static>>, max: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(" ", Style::new().bg(PANEL))]];
    let mut used = 1;
    for span in spans {
        let mut chunk = String::new();
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > max.saturating_sub(1) && used > 1 {
                lines
                    .last_mut()
                    .expect("sempre há uma linha")
                    .push(Span::styled(std::mem::take(&mut chunk), span.style));
                lines.push(vec![Span::styled(" ", Style::new().bg(PANEL))]);
                used = 1;
            }
            chunk.push(c);
            used += w;
        }
        lines
            .last_mut()
            .expect("sempre há uma linha")
            .push(Span::styled(chunk, span.style));
    }
    lines
        .into_iter()
        .map(|mut spans| {
            let used: usize = spans.iter().map(Span::width).sum();
            spans.push(Span::styled(
                " ".repeat(max.saturating_sub(used)),
                Style::new().bg(PANEL),
            ));
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn inline_marks_code_bold_and_links() {
        let spans = inline("use `cargo` **já** e [docs](https://x)", Style::new());
        let parts: Vec<_> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(parts, vec!["use ", "cargo", " ", "já", " e ", "docs"]);
        assert_eq!(spans[1].style.fg, Some(STRING));
        assert!(spans[3].style.add_modifier.contains(Modifier::BOLD));
        assert!(spans[5].style.add_modifier.contains(Modifier::UNDERLINED));
        // Asterisco solto fica como está.
        let spans = inline("2 * 3", Style::new());
        assert_eq!(spans[0].content, "2 * 3");
    }

    #[test]
    fn renders_blocks() {
        let md = "# Título\n- um\n  1. dois\n> nota\n---\n| a | b |\n|---|---|";
        assert_eq!(
            text(&render(md, 40)),
            vec![
                "Título",
                "• um",
                "  1. dois",
                "▎ nota",
                &"─".repeat(40),
                "| a | b |",
                "|---|---|"
            ]
        );
    }

    #[test]
    fn colors_code_blocks_by_language() {
        let lines = render("```rust\nfn main() {}\n```\ndepois", 30);
        assert_eq!(text(&lines), vec![" Rust", " fn main() {}", "depois"]);
        // `fn` em negrito com a cor das palavras-chave, fundo até a borda.
        let code = &lines[1];
        let keyword = code.spans.iter().find(|s| s.content == "fn").unwrap();
        assert_eq!(keyword.style.fg, Some(Color::Rgb(0xC7, 0x9B, 0xFF)));
        assert_eq!(code.width(), 30);
        // Linguagem desconhecida ou bloco ainda aberto: texto puro.
        let lines = render("```\nx = 1", 10);
        assert_eq!(text(&lines), vec![" x = 1"]);
    }

    #[test]
    fn wraps_styled_words() {
        let lines = wrap_spans(
            inline("aaa **bbb** ccc", Style::new()),
            8,
            Span::raw("• "),
            Span::raw("  "),
        );
        assert_eq!(text(&lines), vec!["• aaa", "  bbb", "  ccc"]);
    }
}
