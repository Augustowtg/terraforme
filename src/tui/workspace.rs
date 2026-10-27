//! Navegador de workspaces (`terra workspace`): Recentes e Salvos, com
//! "...Mostrar mais" em cada seção.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use super::{is_ctrl_c, with_terminal};
use crate::db::{Db, default_name};
use crate::theme;

/// Quantos itens cada seção carrega por vez.
const PAGE: usize = 5;

const HELP: &str = "↑/k ↓/j navegar · g/G início/fim · Enter selecionar · q sair";

/// Navegador de workspaces. Retorna o path escolhido, ou `None` se o usuário
/// saiu sem selecionar.
pub fn run_workspace(db: &Db) -> Result<Option<PathBuf>> {
    let mut app = WorkspaceApp::new(db)?;
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| app.draw(frame))?;
            let Some(key) = next_key()? else { continue };
            if is_quit(&key) {
                return Ok(None);
            }
            if let Some(path) = app.handle_key(key)? {
                return Ok(Some(path));
            }
        }
    })
}

/// Pinta o fundo e devolve as áreas de conteúdo e da barra inferior.
fn screen_areas(frame: &mut Frame) -> [Rect; 2] {
    let area = frame.area();
    frame.render_widget(Block::new().style(theme::screen()), area);
    let [_, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    [body, footer]
}

fn next_key() -> std::io::Result<Option<KeyEvent>> {
    match event::read()? {
        // No Windows também chegam eventos de Release; só interessa o Press.
        Event::Key(key) if key.kind == KeyEventKind::Press => Ok(Some(key)),
        _ => Ok(None),
    }
}

fn is_quit(key: &KeyEvent) -> bool {
    is_ctrl_c(key) || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Recents,
    Saved,
}

struct Entry {
    name: String,
    path: PathBuf,
    exists: bool,
}

impl Entry {
    fn new(name: String, path: PathBuf) -> Self {
        let exists = path.is_dir();
        Self { name, path, exists }
    }
}

struct Section {
    title: &'static str,
    entries: Vec<Entry>,
    has_more: bool,
}

/// Uma linha da lista. Só `Entry` e `ShowMore` recebem foco.
enum Row {
    Title(&'static str),
    Empty,
    Entry(SectionKind, usize),
    ShowMore(SectionKind),
    Spacer,
}

impl Row {
    fn selectable(&self) -> bool {
        matches!(self, Row::Entry(..) | Row::ShowMore(_))
    }
}

struct WorkspaceApp<'a> {
    db: &'a Db,
    recents: Section,
    saved: Section,
    rows: Vec<Row>,
    /// Índices de `rows` que podem receber foco.
    selectable: Vec<usize>,
    /// Posição em `selectable`.
    focus: Option<usize>,
    status: Option<&'static str>,
    list_state: ListState,
}

impl<'a> WorkspaceApp<'a> {
    fn new(db: &'a Db) -> Result<Self> {
        let mut app = Self {
            db,
            recents: Section {
                title: "Recentes",
                entries: Vec::new(),
                has_more: false,
            },
            saved: Section {
                title: "Salvos",
                entries: Vec::new(),
                has_more: false,
            },
            rows: Vec::new(),
            selectable: Vec::new(),
            focus: None,
            status: None,
            list_state: ListState::default(),
        };
        app.load_more(SectionKind::Recents)?;
        app.load_more(SectionKind::Saved)?;
        app.rebuild_rows();
        app.focus = (!app.selectable.is_empty()).then_some(0);
        Ok(app)
    }

    fn section(&self, kind: SectionKind) -> &Section {
        match kind {
            SectionKind::Recents => &self.recents,
            SectionKind::Saved => &self.saved,
        }
    }

    /// Carrega a próxima página da seção (busca um item a mais para saber se
    /// ainda há o que mostrar).
    fn load_more(&mut self, kind: SectionKind) -> Result<()> {
        let db = self.db;
        let (section, mut page) = match kind {
            SectionKind::Recents => {
                let page = db
                    .recents(self.recents.entries.len(), PAGE + 1)?
                    .into_iter()
                    .map(|r| Entry::new(default_name(&r.path), r.path))
                    .collect::<Vec<_>>();
                (&mut self.recents, page)
            }
            SectionKind::Saved => {
                let page = db
                    .workspaces(self.saved.entries.len(), PAGE + 1)?
                    .into_iter()
                    .map(|w| Entry::new(w.name, w.path))
                    .collect::<Vec<_>>();
                (&mut self.saved, page)
            }
        };
        section.has_more = page.len() > PAGE;
        page.truncate(PAGE);
        section.entries.extend(page);
        Ok(())
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for kind in [SectionKind::Recents, SectionKind::Saved] {
            let section = self.section(kind);
            if !rows.is_empty() {
                rows.push(Row::Spacer);
            }
            rows.push(Row::Title(section.title));
            if section.entries.is_empty() {
                rows.push(Row::Empty);
            }
            rows.extend((0..section.entries.len()).map(|i| Row::Entry(kind, i)));
            if section.has_more {
                rows.push(Row::ShowMore(kind));
            }
        }
        self.selectable = (0..rows.len()).filter(|&i| rows[i].selectable()).collect();
        self.rows = rows;
    }

    /// Trata uma tecla; retorna o path quando um workspace é aberto.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<PathBuf>> {
        self.status = None;
        let Some(focus) = self.focus else {
            return Ok(None);
        };
        let last = self.selectable.len() - 1;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.focus = Some(focus.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.focus = Some((focus + 1).min(last)),
            KeyCode::Home | KeyCode::Char('g') => self.focus = Some(0),
            KeyCode::End | KeyCode::Char('G') => self.focus = Some(last),
            KeyCode::Enter => return self.activate(self.selectable[focus]),
            _ => {}
        }
        Ok(None)
    }

    fn activate(&mut self, row: usize) -> Result<Option<PathBuf>> {
        match self.rows[row] {
            Row::Entry(kind, i) => {
                let entry = &self.section(kind).entries[i];
                if entry.path.is_dir() {
                    return Ok(Some(entry.path.clone()));
                }
                self.status = Some("Path não encontrado");
            }
            Row::ShowMore(kind) => {
                let first_new = self.section(kind).entries.len();
                self.load_more(kind)?;
                self.rebuild_rows();
                self.focus = self.selectable.iter().position(
                    |&r| matches!(self.rows[r], Row::Entry(k, i) if k == kind && i == first_new),
                );
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [body, footer] = screen_areas(frame);
        let focused_row = self.focus.map(|f| self.selectable[f]);

        let items: Vec<ListItem> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| self.render_row(row, focused_row == Some(i)))
            .collect();
        self.list_state.select(focused_row);
        frame.render_stateful_widget(List::new(items), body, &mut self.list_state);

        let footer_line = match self.status {
            Some(msg) => Line::from(Span::styled(format!(" {msg}"), theme::error())),
            None => Line::from(Span::styled(format!(" {HELP}"), theme::muted())),
        };
        frame.render_widget(Paragraph::new(footer_line), footer);
    }

    fn render_row(&self, row: &Row, focused: bool) -> ListItem<'static> {
        let marker = if focused {
            Span::raw(format!("{} ", theme::FOCUS_MARKER))
        } else {
            Span::raw("  ")
        };
        let line = match row {
            Row::Title(title) => {
                Line::from(Span::styled(format!(" {title}"), theme::section_title()))
            }
            Row::Spacer => Line::default(),
            Row::Empty => Line::from(vec![marker, Span::styled("Nenhum item", theme::muted())]),
            Row::ShowMore(_) => {
                let style = if focused {
                    Style::new()
                } else {
                    theme::show_more()
                };
                Line::from(vec![marker, Span::styled("...Mostrar mais", style)])
            }
            Row::Entry(kind, i) => {
                let entry = &self.section(*kind).entries[*i];
                let (name, sep, path) = match (entry.exists, focused) {
                    (false, _) => (theme::muted(), theme::muted(), theme::muted()),
                    (true, true) => (Style::new(), Style::new(), Style::new()),
                    (true, false) => (theme::text(), theme::text(), theme::path()),
                };
                Line::from(vec![
                    marker,
                    Span::styled(entry.name.clone(), name),
                    Span::styled(" - ", sep),
                    Span::styled(entry.path.display().to_string(), path),
                ])
            }
        };
        let item = ListItem::new(line);
        if focused {
            item.style(theme::focused())
        } else {
            item
        }
    }
}
