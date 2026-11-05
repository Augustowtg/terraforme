//! A seção **Contexto** da aba Consumo: um resumo do `/context` do agente
//! pelas skills, que são muitas. Mostra [`PER_PAGE`] por página, cada uma com
//! o grupo (de onde vem) e os tokens, e a contagem por grupo.
//!
//! Os tokens são estimados (1 a cada [`CHARS_PER_TOKEN`] caracteres). **Sem
//! cache** conta só o que o agente carrega sempre (nome e descrição); **Com
//! cache** soma o `SKILL.md` inteiro, que entra no cache quando a skill é
//! usada. Os botões são os mesmos do gráfico.

use ratatui::text::{Line, Span};

use super::{LABEL_WIDTH, indent};
use crate::agent::ProviderSkill;
use crate::theme;
use crate::tui::chat::compact_count;

/// Skills por página.
pub(super) const PER_PAGE: usize = 3;
const CHARS_PER_TOKEN: usize = 4;
pub(super) const PREV: &str = " ‹ ";
pub(super) const NEXT: &str = " › ";

/// Uma skill e o que ela pesa no contexto.
pub(super) struct SkillCost {
    name: String,
    /// De onde vem (ex.: `usuário`, `plugin`).
    group: &'static str,
    /// Tokens de nome e descrição.
    listing: u64,
    /// Tokens do `SKILL.md` inteiro.
    file: u64,
}

impl SkillCost {
    pub(super) fn new(skill: &ProviderSkill) -> Self {
        let tokens = |chars: usize| chars.div_ceil(CHARS_PER_TOKEN) as u64;
        let listing = skill.name.chars().count() + skill.description.chars().count();
        let file = std::fs::read_to_string(&skill.path)
            .map(|text| text.chars().count())
            .unwrap_or(listing);
        Self {
            name: skill.name.clone(),
            group: skill.scope,
            listing: tokens(listing),
            file: tokens(file.max(listing)),
        }
    }

    fn tokens(&self, with_cache: bool) -> u64 {
        if with_cache { self.file } else { self.listing }
    }
}

/// Quantas páginas há para `total` skills (ao menos 1).
pub(super) fn pages(total: usize) -> usize {
    total.div_ceil(PER_PAGE).max(1)
}

/// As linhas da seção, com a página `page` (0 é a primeira). A segunda linha
/// é a do paginador, cujos botões ficam em [`pager_columns`].
pub(super) fn lines(skills: &[SkillCost], page: usize, with_cache: bool) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(" Contexto", theme::section_title())];
    if skills.is_empty() {
        lines.push(indent(Span::styled(
            "Este provedor não tem skills.",
            theme::muted(),
        )));
        return lines;
    }
    let total: u64 = skills.iter().map(|s| s.tokens(with_cache)).sum();
    let mut groups: Vec<(&str, usize)> = Vec::new();
    for skill in skills {
        match groups.iter_mut().find(|(g, _)| *g == skill.group) {
            Some((_, count)) => *count += 1,
            None => groups.push((skill.group, 1)),
        }
    }
    let groups: Vec<String> = groups
        .iter()
        .map(|(group, count)| format!("{group} {count}"))
        .collect();
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{:<LABEL_WIDTH$}", "Skills"), theme::muted()),
        Span::styled(
            format!("{} · {} tokens", skills.len(), compact_count(total)),
            theme::text(),
        ),
    ]));
    lines.push(indent(Span::styled(groups.join(" · "), theme::muted())));
    lines.push(Line::default());

    let page = page.min(pages(skills.len()) - 1);
    let shown = skills.iter().skip(page * PER_PAGE).take(PER_PAGE);
    let name_width = shown
        .clone()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    for skill in shown {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<LABEL_WIDTH$}", skill.group), theme::muted()),
            Span::styled(format!("{:<name_width$}", skill.name), theme::text()),
            Span::styled(
                format!("  {}", compact_count(skill.tokens(with_cache))),
                if with_cache {
                    theme::error()
                } else {
                    theme::variant(theme::Variant::Success)
                },
            ),
        ]));
    }
    // Linhas fixas por página, para a seção não pular ao paginar.
    for _ in skills.len().saturating_sub(page * PER_PAGE).min(PER_PAGE)..PER_PAGE {
        lines.push(Line::default());
    }
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(PREV, theme::show_more()),
        Span::styled(
            format!(" {}/{} ", page + 1, pages(skills.len())),
            theme::muted(),
        ),
        Span::styled(NEXT, theme::show_more()),
        Span::styled("  ←/→ muda de página", theme::muted()),
    ]));
    lines
}

/// Onde ficam os botões `‹` e `›` na linha do paginador, em colunas a partir
/// da borda da seção: `(início, largura)` de cada um.
pub(super) fn pager_columns(skills: usize, page: usize) -> [(u16, u16); 2] {
    let prev = PREV.chars().count() as u16;
    let label = format!(" {}/{} ", page + 1, pages(skills)).chars().count() as u16;
    [(2, prev), (2 + prev + label, NEXT.chars().count() as u16)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(name: &str, group: &'static str, listing: u64, file: u64) -> SkillCost {
        SkillCost {
            name: name.to_string(),
            group,
            listing,
            file,
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn pages_hold_three_skills() {
        assert_eq!(pages(0), 1);
        assert_eq!(pages(3), 1);
        assert_eq!(pages(4), 2);
        assert_eq!(pages(10), 4);
    }

    #[test]
    fn page_shows_three_skills_and_cache_changes_tokens() {
        let skills: Vec<SkillCost> = (0..5)
            .map(|i| cost(&format!("skill{i}"), "plugin", 100, 2_000))
            .collect();
        let rows =
            |page, cache| -> Vec<String> { lines(&skills, page, cache).iter().map(text).collect() };
        let first = rows(0, false);
        assert!(first.iter().any(|l| l.contains("skill2")));
        assert!(!first.iter().any(|l| l.contains("skill3")));
        assert!(
            first
                .iter()
                .any(|l| l.contains("plugin") && l.contains("100"))
        );
        assert!(
            rows(1, true)
                .iter()
                .any(|l| l.contains("skill3") && l.contains("2K"))
        );
        assert!(rows(1, true).last().unwrap().contains("2/2"));
        assert_eq!(rows(0, false).len(), rows(1, false).len());
    }
}
