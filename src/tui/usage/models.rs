//! As seções **Tokens por modelo** (a conta do route) e **Tokens na
//! workspace** (a pasta aberta, com todos os provedores e contas) da aba Consumo: o
//! total do período do gráfico, o maior primeiro, com o cache conforme o
//! botão.

use ratatui::text::{Line, Span};

use super::indent;
use crate::theme;
use crate::tui::chat::compact_count;

/// Uma linha: o nome (ex.: `Opus - 5.5`, `Claude · Opus - 5.5`) e o gasto.
pub(super) struct Row {
    pub label: String,
    pub tokens: u64,
    pub cache: u64,
}

/// Uma seção: o título, as linhas com gasto e o total.
pub(super) fn lines(
    title: &str,
    rows: &[Row],
    days: usize,
    with_cache: bool,
) -> Vec<Line<'static>> {
    let value = |row: &Row| row.tokens + if with_cache { row.cache } else { 0 };
    // O mesmo nome em mais de uma linha (o modelo em duas contas) soma.
    let mut merged: Vec<(&str, u64)> = Vec::new();
    for row in rows {
        match merged.iter_mut().find(|(label, _)| *label == row.label) {
            Some((_, total)) => *total += value(row),
            None => merged.push((row.label.as_str(), value(row))),
        }
    }
    let mut rows = merged;
    rows.retain(|(_, value)| *value > 0);
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let mut lines = vec![Line::from(vec![
        Span::styled(format!(" {title}"), theme::section_title()),
        Span::styled(format!("  últimos {days} dias"), theme::muted()),
    ])];
    if rows.is_empty() {
        lines.push(indent(Span::styled(
            "Sem consumo no período.",
            theme::muted(),
        )));
        return lines;
    }
    let total: u64 = rows.iter().map(|(_, value)| value).sum();
    rows.push(("Total", total));
    let width = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    let last = rows.len() - 1;
    for (i, (label, value)) in rows.iter().enumerate() {
        let style = if i == last {
            theme::muted()
        } else {
            theme::text()
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{label:<width$}"), style),
            Span::styled(format!("  {:>6}", compact_count(*value)), theme::text()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_sorted_with_total_and_cache() {
        let rows = [
            Row {
                label: "Haiku".into(),
                tokens: 1_000,
                cache: 0,
            },
            Row {
                label: "Fable - 5.2".into(),
                tokens: 2_000,
                cache: 8_000,
            },
            Row {
                label: "Haiku".into(),
                tokens: 0,
                cache: 0,
            },
            Row {
                label: "Fable - 5.2".into(),
                tokens: 500,
                cache: 0,
            },
            Row {
                label: "Parado".into(),
                tokens: 0,
                cache: 0,
            },
        ];
        let text = |cache| -> Vec<String> {
            lines("Tokens por modelo", &rows, 14, cache)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        assert_eq!(
            text(false),
            [
                " Tokens por modelo  últimos 14 dias",
                "  Fable - 5.2    2.5K",
                "  Haiku            1K",
                "  Total          3.5K",
            ]
        );
        assert_eq!(text(true)[1], "  Fable - 5.2   10.5K");
        assert_eq!(text(true)[3], "  Total         11.5K");
        assert_eq!(lines("T", &[], 14, false).len(), 2);
    }
}
