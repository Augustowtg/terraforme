//! Lista que abre logo acima de um elemento (ex.: a caixa de input), como as
//! do `@` (arquivos) e da `/` (comandos): borda arredondada, título com a
//! contagem e rolagem que mantém o item escolhido visível.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::theme;

/// Itens visíveis de cada vez.
pub const VISIBLE: usize = 8;

/// Desenha `items` acima de `anchor`, com o título `title`. Sem itens,
/// mostra `empty`. Sem espaço acima, não desenha.
pub fn draw_above(
    frame: &mut Frame,
    anchor: Rect,
    title: String,
    items: Vec<Line<'static>>,
    selected: usize,
    empty: &str,
) {
    let rows = items.len().clamp(1, VISIBLE) as u16;
    let height = (rows + 2).min(anchor.y);
    if height < 3 {
        return;
    }
    let area = Rect {
        y: anchor.y - height,
        height,
        ..anchor
    };
    let first = selected.saturating_sub(VISIBLE - 1);
    let lines: Vec<Line> = if items.is_empty() {
        vec![Line::styled(format!(" {empty}"), theme::muted())]
    } else {
        items.into_iter().skip(first).take(VISIBLE).collect()
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Span::styled(format!(" {title} "), theme::muted()))
        .style(theme::screen());
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// O começo de uma linha da lista: o marcador de foco ou o recuo.
pub fn marker(selected: bool) -> Span<'static> {
    if selected {
        Span::raw(format!("{} ", theme::FOCUS_MARKER))
    } else {
        Span::raw("  ")
    }
}
