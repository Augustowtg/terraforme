//! Badge: rótulo curto com borda arredondada nas cores de uma variante do tema,
//! no mesmo formato do pill da navbar (" rótulo ").

use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::theme::{self, Variant};

/// Altura de um badge: borda, texto e borda.
pub const HEIGHT: u16 = 3;

pub fn badge(label: &str, variant: Variant) -> Paragraph<'static> {
    Paragraph::new(Span::styled(
        format!(" {label} "),
        theme::badge_text(variant),
    ))
    .block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::badge_border(variant)),
    )
}

/// Largura que o badge ocupa: rótulo, um espaço de cada lado e as bordas.
pub fn width(label: &str) -> u16 {
    Span::raw(label).width() as u16 + 4
}
