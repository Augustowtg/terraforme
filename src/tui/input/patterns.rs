//! Padrões de prompt com `&` no input.
//!
//! Digitar `&` abre uma lista de padrões que melhoram o prompt (ex.:
//! `&ASD-STE100`), cada um com uma descrição do que ele faz. O escolhido vira
//! um badge, como o `@`, e vai para o agente como a instrução do padrão (ex.:
//! `use ASD-STE100 com 75% de precisão`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::theme::{self, Variant};
use crate::tui::widgets::popup;

/// Um padrão: o nome (o que se escreve depois do `&`), a descrição da lista e
/// a instrução que vai para o agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    pub name: &'static str,
    pub description: &'static str,
    pub instruction: &'static str,
}

/// Os padrões da lista, na ordem em que aparecem.
pub const PATTERNS: &[Pattern] = &[
    Pattern {
        name: "ASD-STE100",
        description: "Diminui o retorno: frases curtas, diretas e sem ambiguidade (Simplified Technical English)",
        instruction: "use ASD-STE100 com 75% de precisão",
    },
    Pattern {
        name: "BLUF",
        description: "A resposta vem primeiro, os detalhes depois (Bottom Line Up Front)",
        instruction: "use BLUF: a conclusão na primeira frase e os detalhes depois",
    },
    Pattern {
        name: "KISS",
        description: "A solução mais simples que resolve, sem complexidade a mais",
        instruction: "siga o KISS: a solução mais simples que resolve o pedido",
    },
    Pattern {
        name: "YAGNI",
        description: "Só o que foi pedido, sem funções para um futuro que talvez não venha",
        instruction: "siga o YAGNI: não implemente nada além do que foi pedido",
    },
    Pattern {
        name: "DRY",
        description: "Reaproveita o código que já existe em vez de duplicar",
        instruction: "siga o DRY: reaproveite o código existente em vez de duplicar",
    },
    Pattern {
        name: "SOLID",
        description: "Classes e funções com uma responsabilidade e dependências por abstração",
        instruction: "siga os princípios SOLID no código",
    },
    Pattern {
        name: "TDD",
        description: "Escreve o teste que falha antes do código que o faz passar",
        instruction: "use TDD: escreva o teste que falha antes do código",
    },
    Pattern {
        name: "Passo-a-passo",
        description: "Pensa em etapas antes de responder, mostrando o raciocínio",
        instruction: "pense passo a passo antes de responder",
    },
];
/// Largura da coluna do nome (`&nome`).
const NAME_WIDTH: usize = 16;

/// O padrão de nome `name` (sem diferenciar maiúsculas).
pub fn find(name: &str) -> Option<&'static Pattern> {
    PATTERNS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Padrões que combinam com `query`: começo do nome, parte do nome e parte
/// da descrição, nessa ordem; no empate, a ordem da lista.
pub fn matches(query: &str) -> Vec<&'static Pattern> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, &Pattern)> = PATTERNS
        .iter()
        .filter_map(|pattern| {
            let name = pattern.name.to_lowercase();
            let rank = if name.starts_with(&query) {
                0
            } else if name.contains(&query) {
                1
            } else if pattern.description.to_lowercase().contains(&query) {
                2
            } else {
                return None;
            };
            Some((rank, pattern))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, pattern)| pattern).collect()
}

/// Desenha a lista acima de `input`.
pub fn draw(frame: &mut Frame, input: Rect, found: &[&Pattern], selected: usize) {
    let items = found
        .iter()
        .enumerate()
        .map(|(i, pattern)| item(pattern, i == selected))
        .collect();
    popup::draw_above(
        frame,
        input,
        format!("Padrões ({})", found.len()),
        items,
        selected,
        "Nenhum padrão",
    );
}

fn item(pattern: &Pattern, selected: bool) -> Line<'static> {
    let mut name = format!("&{:<width$}", pattern.name, width = NAME_WIDTH - 1);
    if pattern.name.chars().count() >= NAME_WIDTH - 1 {
        name.push(' ');
    }
    let description = format!("- {}", pattern.description);
    if selected {
        return Line::from(vec![
            popup::marker(true),
            Span::raw(name),
            Span::raw(description),
        ])
        .style(theme::focused());
    }
    Line::from(vec![
        popup::marker(false),
        Span::styled(name, theme::variant(Variant::Active)),
        Span::styled(description, theme::path()),
    ])
}

#[cfg(test)]
#[path = "tests/patterns.rs"]
mod tests;
