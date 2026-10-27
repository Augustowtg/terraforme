//! Aba MCP: o orquestrador. Aparece com o MCP online e mostra os spaces (os
//! workspaces salvos e os abertos em algum terra) e, em cada um, se há uma
//! sessão ativa, só o terra cli aberto ou nada.
//!
//! ```text
//!  MCP · http://127.0.0.1:30303/mcp
//!    2 terra cli abertos · 1 com sessão ativa
//!
//!  Spaces
//!  ▌ ● terraforme-cli  sessão ativa      /home/eu/terraforme-cli
//!    ● site            terra cli aberto  /home/eu/site
//!    ○ notas           fechado           /home/eu/notas
//! ```
//!
//! `↑`/`↓` escolhem o space; a lista é atualizada a cada [`STALE`] enquanto
//! a aba está aberta.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::db;
use crate::mcp::{self, Instance};
use crate::theme::{self, Variant};

pub const HELP: &str = "↑/↓ escolher · Tab trocar aba · Esc ir para Sair";
/// Depois disso, a lista é recarregada.
pub const STALE: Duration = Duration::from_secs(1);
/// A maior largura da coluna dos nomes.
const NAME_MAX: usize = 28;
/// Largura da coluna do estado.
const STATUS_WIDTH: usize = 18;
/// As linhas antes da lista (título, resumo, linha vazia e o título Spaces)
/// e o recuo de cima.
const HEADER: u16 = 5;

/// O que está aberto num space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpaceStatus {
    /// Um terra aberto com uma conversa com o agente.
    Session,
    /// Um terra aberto, sem conversa.
    Open,
    /// Nenhum terra aberto (um workspace salvo).
    Closed,
}

impl SpaceStatus {
    fn label(self) -> &'static str {
        match self {
            SpaceStatus::Session => "sessão ativa",
            SpaceStatus::Open => "terra cli aberto",
            SpaceStatus::Closed => "fechado",
        }
    }

    fn dot(self) -> Span<'static> {
        match self {
            SpaceStatus::Session => Span::styled("●", theme::variant(Variant::Success)),
            SpaceStatus::Open => Span::styled("●", theme::variant(Variant::Warning)),
            SpaceStatus::Closed => Span::styled("○", theme::muted()),
        }
    }
}

/// Um space da lista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub name: String,
    pub dir: PathBuf,
    pub status: SpaceStatus,
    /// Quantos terra estão abertos nele.
    pub open: usize,
}

/// Junta os workspaces salvos com os terra abertos: um space por pasta, com
/// as sessões ativas primeiro, depois os abertos e os fechados, cada grupo
/// por nome.
pub fn spaces(saved: &[db::Workspace], instances: &[Instance]) -> Vec<Space> {
    let mut spaces: Vec<Space> = saved
        .iter()
        .map(|w| Space {
            name: w.name.clone(),
            dir: w.path.clone(),
            status: SpaceStatus::Closed,
            open: 0,
        })
        .collect();
    for instance in instances {
        let status = if instance.session {
            SpaceStatus::Session
        } else {
            SpaceStatus::Open
        };
        match spaces.iter_mut().find(|s| s.dir == instance.dir) {
            Some(space) => {
                space.status = space.status.min(status);
                space.open += 1;
            }
            None => spaces.push(Space {
                name: instance.name.clone(),
                dir: instance.dir.clone(),
                status,
                open: 1,
            }),
        }
    }
    spaces.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    spaces
}

#[derive(Default)]
pub struct McpView {
    selected: usize,
    spaces: Vec<Space>,
    /// Quando a lista foi carregada.
    refreshed: Option<Instant>,
}

impl McpView {
    /// Se a lista deve ser recarregada.
    pub fn is_stale(&self) -> bool {
        self.refreshed.is_none_or(|at| at.elapsed() >= STALE)
    }

    pub fn refresh(&mut self, saved: &[db::Workspace], instances: &[Instance]) {
        self.spaces = spaces(saved, instances);
        self.selected = self.selected.min(self.spaces.len().saturating_sub(1));
        self.refreshed = Some(Instant::now());
    }

    /// `↑`/`↓` escolhem o space. Retorna `true` se a tecla foi usada.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let last = self.spaces.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            _ => return false,
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let open: usize = self.spaces.iter().map(|s| s.open).sum();
        let sessions = self
            .spaces
            .iter()
            .filter(|s| s.status == SpaceStatus::Session)
            .count();
        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Orquestrador", theme::section_title()),
                Span::styled(format!(" · {}", mcp::url()), theme::path()),
            ]),
            Line::styled(
                format!(
                    "   {} · {sessions} com sessão ativa",
                    if open == 1 {
                        "1 terra cli aberto".to_string()
                    } else {
                        format!("{open} terra cli abertos")
                    },
                ),
                theme::muted(),
            ),
            Line::default(),
            Line::styled(" Spaces", theme::section_title()),
        ];
        if self.spaces.is_empty() {
            lines.push(Line::styled(
                "   Nenhum space: salve um com terra workspace --save.",
                theme::muted(),
            ));
        }
        let name_width = self
            .spaces
            .iter()
            .map(|s| s.name.width())
            .max()
            .unwrap_or(0)
            .min(NAME_MAX);
        // A lista rola para o space escolhido ficar na tela.
        let rows = usize::from(area.height.saturating_sub(HEADER)).max(1);
        let offset = self.selected.saturating_sub(rows - 1);
        for (i, space) in self.spaces.iter().enumerate().skip(offset).take(rows) {
            lines.push(space_line(space, name_width, i == self.selected));
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
    }
}

fn space_line(space: &Space, name_width: usize, selected: bool) -> Line<'static> {
    let name: String = truncate(&space.name, name_width);
    let pad = name_width - name.width();
    let mut status = space.status.label().to_string();
    if space.open > 1 {
        status = format!("{status} ({})", space.open);
    }
    let text = format!(" {name}{}  {status:<STATUS_WIDTH$}", " ".repeat(pad));
    let dir = space.dir.display().to_string();
    if selected {
        Line::from(vec![
            Span::raw(format!("{} ", theme::FOCUS_MARKER)),
            space.status.dot(),
            Span::raw(text),
            Span::raw(dir),
        ])
        .style(theme::focused().add_modifier(Modifier::BOLD))
    } else {
        let style = if space.status == SpaceStatus::Closed {
            theme::muted()
        } else {
            theme::text()
        };
        Line::from(vec![
            Span::raw("  "),
            space.status.dot(),
            Span::styled(text, style),
            Span::styled(dir, theme::path()),
        ])
    }
}

/// `text` com no máximo `width` colunas, com `…` quando corta.
fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn saved(name: &str) -> db::Workspace {
        db::Workspace {
            name: name.into(),
            path: PathBuf::from(format!("/w/{name}")),
        }
    }

    fn open(pid: u32, name: &str, session: bool) -> Instance {
        Instance {
            pid,
            name: name.into(),
            dir: PathBuf::from(format!("/w/{name}")),
            session,
        }
    }

    #[test]
    fn spaces_join_saved_workspaces_with_open_terra() {
        let list = spaces(
            &[saved("notas"), saved("terra"), saved("Site")],
            &[
                open(1, "Site", false),
                open(2, "terra", false),
                open(3, "terra", true),
                open(4, "novo", false),
            ],
        );
        let rows: Vec<_> = list
            .iter()
            .map(|s| (s.name.as_str(), s.status, s.open))
            .collect();
        assert_eq!(
            rows,
            [
                ("terra", SpaceStatus::Session, 2),
                ("novo", SpaceStatus::Open, 1),
                ("Site", SpaceStatus::Open, 1),
                ("notas", SpaceStatus::Closed, 0),
            ]
        );
    }

    #[test]
    fn draws_the_spaces_with_their_status() {
        let mut view = McpView::default();
        view.refresh(
            &[saved("notas")],
            &[open(1, "terra", true), open(2, "site", false)],
        );
        assert!(view.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert!(!view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let mut terminal = Terminal::new(TestBackend::new(70, 10)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..70)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(1), "  Orquestrador · http://127.0.0.1:30303/mcp");
        assert_eq!(row(2), "    2 terra cli abertos · 1 com sessão ativa");
        assert_eq!(row(4), "  Spaces");
        assert_eq!(row(5), "   ● terra  sessão ativa      /w/terra");
        assert_eq!(row(6), " ▌ ● site   terra cli aberto  /w/site");
        assert_eq!(row(7), "   ○ notas  fechado           /w/notas");
    }

    #[test]
    fn long_names_are_cut() {
        assert_eq!(truncate("terraforme", 6), "terra…");
        assert_eq!(truncate("terra", 6), "terra");
    }
}
