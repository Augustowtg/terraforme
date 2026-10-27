//! Histórico da conversa na aba Chat: mensagens, texto do agente chegando em
//! streaming, ferramentas usadas e erros.
//!
//! Acompanha o fim da conversa, a não ser que o usuário role para cima (roda
//! do mouse ou PageUp/PageDown); ao rolar, aparece uma barra de rolagem que
//! some depois de 2 s. O texto pode ser selecionado com o mouse: arrastar
//! seleciona um trecho e um duplo clique seleciona a mensagem inteira.
//!
//! As linhas são quebradas aqui (e não pelo `Paragraph`) para o mouse saber
//! exatamente qual caractere está em cada célula.

use std::cell::{Cell, RefCell};

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use unicode_width::UnicodeWidthChar;

use crate::theme::{self, Variant};

/// Ticks (de 100 ms) que a barra de rolagem fica visível depois de rolar.
const SCROLLBAR_TICKS: u64 = 20;
/// Dois cliques na mesma linha em até 4 ticks (~400 ms) são um duplo clique.
const DOUBLE_CLICK_TICKS: u64 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    Assistant(String),
    Tool { name: String, detail: String },
    ToolResult { is_error: bool, summary: String },
    Error(String),
    Info(String),
}

/// Um caractere já posicionado.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    ch: char,
    style: Style,
    /// Faz parte do texto da mensagem; prefixos (`⏺ `) e recuos não entram na
    /// cópia.
    text: bool,
}

/// Uma linha da tela.
#[derive(Debug, Default)]
struct Row {
    glyphs: Vec<Glyph>,
    /// Continuação da linha anterior (quebra automática): na cópia, as duas
    /// se juntam sem `\n`.
    wrapped: bool,
    /// Entrada do histórico a que a linha pertence (`None` nos espaços).
    entry: Option<usize>,
}

/// Linha e coluna no histórico inteiro (não só na parte visível).
type Point = (usize, usize);

#[derive(Debug, Default, Clone, Copy)]
struct View {
    /// Área do texto.
    inner: Rect,
    /// Primeira linha visível.
    top: usize,
}

#[derive(Default)]
pub struct History {
    entries: Vec<Entry>,
    /// Linhas roladas para cima a partir do fim (0 = acompanhando o fim).
    scroll: usize,
    /// Quanto dá para rolar, calculado no último frame.
    max_scroll: Cell<usize>,
    area: Cell<Rect>,
    view: Cell<View>,
    /// Linhas do último frame, para o mouse e a cópia.
    rows: RefCell<Vec<Row>>,
    /// Seleção: ponta fixa e ponta que segue o mouse.
    selection: Option<(Point, Point)>,
    /// Botão esquerdo pressionado sobre o histórico (arrastando).
    selecting: bool,
    ticks: u64,
    /// Tick da última rolagem, para a barra sumir depois.
    last_scroll: Option<u64>,
    /// Tick e linha do último clique, para o duplo clique.
    last_click: Option<(u64, usize)>,
}

impl History {
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Começa um novo bloco de texto do agente.
    pub fn start_text(&mut self) {
        self.entries.push(Entry::Assistant(String::new()));
    }

    /// Acrescenta um trecho ao texto do agente em andamento.
    pub fn append_text(&mut self, text: &str) {
        match self.entries.last_mut() {
            Some(Entry::Assistant(current)) => current.push_str(text),
            _ => self.entries.push(Entry::Assistant(text.to_string())),
        }
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn tick(&mut self) {
        self.ticks += 1;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll = (self.scroll + lines).min(self.max_scroll.get());
        self.last_scroll = Some(self.ticks);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        self.last_scroll = Some(self.ticks);
    }

    /// Volta a acompanhar o fim da conversa.
    pub fn follow(&mut self) {
        self.scroll = 0;
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        self.area.get().contains(Position::new(column, row))
    }

    /// A barra só aparece logo depois de rolar, e só se a conversa não cabe.
    fn scrollbar_visible(&self) -> bool {
        self.max_scroll.get() > 0
            && self
                .last_scroll
                .is_some_and(|tick| self.ticks - tick < SCROLLBAR_TICKS)
    }

    pub fn is_selecting(&self) -> bool {
        self.selecting
    }

    /// Clique: começa uma seleção. Num duplo clique, seleciona a mensagem
    /// inteira e devolve o texto para copiar.
    pub fn mouse_down(&mut self, column: u16, row: u16) -> Option<String> {
        let point = self.point_at(column, row);
        let double = self
            .last_click
            .is_some_and(|(tick, r)| r == point.0 && self.ticks - tick <= DOUBLE_CLICK_TICKS);
        let entry = self.rows.borrow().get(point.0).and_then(|r| r.entry);
        if double && let Some(entry) = entry {
            self.last_click = None;
            self.selecting = false;
            let rows = self.rows.borrow();
            let first = rows.iter().position(|r| r.entry == Some(entry))?;
            let last = rows.iter().rposition(|r| r.entry == Some(entry))?;
            drop(rows);
            self.selection = Some(((first, 0), (last, usize::MAX)));
            return self.selected_text();
        }
        self.last_click = Some((self.ticks, point.0));
        self.selection = Some((point, point));
        self.selecting = true;
        None
    }

    /// Arrastar estende a seleção; acima ou abaixo do histórico, rola.
    pub fn mouse_drag(&mut self, column: u16, row: u16) {
        if !self.selecting {
            return;
        }
        let inner = self.view.get().inner;
        if row < inner.y {
            self.scroll_up(1);
        } else if row >= inner.bottom() {
            self.scroll_down(1);
        }
        let point = self.point_at(column, row);
        if let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, point));
        }
    }

    /// Soltar o botão termina a seleção e devolve o texto para copiar. Um
    /// clique sem arrastar desfaz a seleção.
    pub fn mouse_up(&mut self, column: u16, row: u16) -> Option<String> {
        if !self.selecting {
            return None;
        }
        self.mouse_drag(column, row);
        self.selecting = false;
        match self.selection {
            Some((anchor, head)) if anchor != head => self.selected_text(),
            _ => {
                self.selection = None;
                None
            }
        }
    }

    /// Ponto do histórico sob a célula da tela (limitado à parte visível).
    fn point_at(&self, column: u16, row: u16) -> Point {
        let view = self.view.get();
        let dy = row
            .saturating_sub(view.inner.y)
            .min(view.inner.height.saturating_sub(1));
        let last = self.rows.borrow().len().saturating_sub(1);
        (
            (view.top + dy as usize).min(last),
            column.saturating_sub(view.inner.x) as usize,
        )
    }

    /// Seleção em ordem de leitura (início, fim), com o fim incluído.
    fn ordered_selection(&self) -> Option<(Point, Point)> {
        let (anchor, head) = self.selection?;
        Some((anchor.min(head), anchor.max(head)))
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.ordered_selection()?;
        let rows = self.rows.borrow();
        let mut out = String::new();
        for (r, row) in rows.iter().enumerate().take(end.0 + 1).skip(start.0) {
            if r > start.0 && !row.wrapped {
                out.push('\n');
            }
            let mut col = 0;
            for glyph in &row.glyphs {
                if glyph.text && (start..=end).contains(&(r, col)) {
                    out.push(glyph.ch);
                }
                col += glyph.ch.width().unwrap_or(0);
            }
        }
        let text = out
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        let text = text.trim_matches('\n').to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Desenha o histórico; sem mensagens, mostra `empty`.
    pub fn draw(&self, frame: &mut Frame, area: Rect, empty: Vec<Line<'static>>) {
        self.area.set(area);
        let block = Block::new().padding(Padding::new(2, 2, 1, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.entries.is_empty() {
            self.rows.borrow_mut().clear();
            self.max_scroll.set(0);
            frame.render_widget(Paragraph::new(empty), inner);
            return;
        }

        let rows = self.layout(inner.width);
        let height = inner.height as usize;
        let max = rows.len().saturating_sub(height);
        self.max_scroll.set(max);
        let top = max - self.scroll.min(max);
        self.view.set(View { inner, top });

        let selection = self.ordered_selection();
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(r, row)| {
                let mut col = 0;
                let spans = row
                    .glyphs
                    .iter()
                    .map(|glyph| {
                        let selected = glyph.text
                            && selection.is_some_and(|(s, e)| (s..=e).contains(&(r, col)));
                        col += glyph.ch.width().unwrap_or(0);
                        let style = if selected {
                            theme::selection()
                        } else {
                            glyph.style
                        };
                        Span::styled(glyph.ch.to_string(), style)
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        *self.rows.borrow_mut() = rows;

        if self.scrollbar_visible() {
            let mut state = ScrollbarState::new(max)
                .position(top)
                .viewport_content_length(height);
            let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(theme::border())
                .thumb_symbol("┃")
                .thumb_style(theme::variant(Variant::Secondary));
            let bar_area = Rect {
                y: inner.y,
                height: inner.height,
                ..area
            };
            frame.render_stateful_widget(bar, bar_area, &mut state);
        }
    }

    /// Todas as linhas do histórico, quebradas em `width` colunas.
    fn layout(&self, width: u16) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if matches!(entry, Entry::Assistant(text) if text.trim().is_empty()) {
                continue;
            }
            // Linha em branco entre as entradas; o resultado fica colado na
            // ferramenta.
            if !rows.is_empty() && !matches!(entry, Entry::ToolResult { .. }) {
                rows.push(Row::default());
            }
            for (prefix, body) in logical_lines(entry) {
                rows.extend(wrap(prefix, body, width, Some(i)));
            }
        }
        rows
    }
}

/// As linhas de uma entrada (uma por `\n`), como (prefixo, texto).
fn logical_lines(entry: &Entry) -> Vec<(Vec<Glyph>, Vec<Glyph>)> {
    match entry {
        Entry::User(text) => {
            let prefix = theme::variant(Variant::Active).add_modifier(Modifier::BOLD);
            prefixed(text, "› ", prefix, theme::text())
        }
        Entry::Assistant(text) => prefixed(
            text.trim(),
            "⏺ ",
            theme::variant(Variant::Accent),
            theme::text(),
        ),
        Entry::Tool { name, detail } => {
            let mut body = glyphs(name, theme::text().add_modifier(Modifier::BOLD), true);
            if !detail.is_empty() {
                body.extend(glyphs(&format!("({detail})"), theme::path(), true));
            }
            vec![(glyphs("⏺ ", theme::variant(Variant::Success), false), body)]
        }
        Entry::ToolResult { is_error, summary } => {
            let style = if *is_error {
                theme::error()
            } else {
                theme::muted()
            };
            let summary = if summary.is_empty() {
                "(sem saída)"
            } else {
                summary
            };
            vec![(
                glyphs("  ⎿ ", theme::muted(), false),
                glyphs(summary, style, true),
            )]
        }
        Entry::Error(text) => prefixed(text, "✘ ", theme::error(), theme::error()),
        Entry::Info(text) => prefixed(text, "· ", theme::muted(), theme::muted()),
    }
}

/// Uma linha por `\n`: a primeira com `prefix`, as outras alinhadas a ela.
fn prefixed(
    text: &str,
    prefix: &str,
    prefix_style: Style,
    style: Style,
) -> Vec<(Vec<Glyph>, Vec<Glyph>)> {
    let indent = " ".repeat(prefix.chars().count());
    text.split('\n')
        .enumerate()
        .map(|(i, line)| {
            let head = if i == 0 { prefix } else { &indent };
            (glyphs(head, prefix_style, false), glyphs(line, style, true))
        })
        .collect()
}

fn glyphs(text: &str, style: Style, is_text: bool) -> Vec<Glyph> {
    text.chars()
        .map(|ch| Glyph {
            ch,
            style,
            text: is_text,
        })
        .collect()
}

/// Quebra uma linha em `width` colunas, por palavra (ou no meio dela, se não
/// couber). As continuações ficam alinhadas ao texto, depois do prefixo.
fn wrap(prefix: Vec<Glyph>, body: Vec<Glyph>, width: u16, entry: Option<usize>) -> Vec<Row> {
    let width = width.max(1) as usize;
    let measure = |gs: &[Glyph]| gs.iter().map(|g| g.ch.width().unwrap_or(0)).sum::<usize>();
    let indent_width = measure(&prefix);
    let indent = if indent_width < width {
        glyphs(&" ".repeat(indent_width), Style::new(), false)
    } else {
        Vec::new()
    };

    let mut rows = Vec::new();
    let mut row = prefix;
    let mut body_start = row.len();
    let mut col = indent_width;
    let mut last_space: Option<usize> = None;
    for glyph in body {
        let w = glyph.ch.width().unwrap_or(0);
        if col + w > width && row.len() > body_start {
            // Leva a palavra incompleta para a próxima linha.
            let rest = match last_space {
                Some(space) => row.split_off(space + 1),
                None => Vec::new(),
            };
            rows.push(Row {
                glyphs: row,
                wrapped: !rows.is_empty(),
                entry,
            });
            row = indent.clone();
            body_start = row.len();
            col = measure(&row) + measure(&rest);
            row.extend(rest);
            last_space = None;
        }
        if glyph.ch == ' ' {
            last_space = Some(row.len());
        }
        row.push(glyph);
        col += w;
    }
    rows.push(Row {
        glyphs: row,
        wrapped: !rows.is_empty(),
        entry,
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    /// Desenha e devolve as linhas da tela (sem espaços à direita).
    fn screen(history: &History, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| history.draw(frame, frame.area(), vec![Line::from("vazio")]))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Só o texto das linhas, sem a coluna da barra de rolagem.
    fn text_of(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.trim_end_matches(['│', '┃']).trim_end().to_string())
            .collect()
    }

    fn info_history(n: usize) -> History {
        let mut history = History::default();
        for i in 0..n {
            history.push(Entry::Info(format!("linha {i}")));
        }
        history
    }

    #[test]
    fn streams_text_into_one_entry() {
        let mut history = History::default();
        history.start_text();
        history.append_text("o");
        history.append_text("i");
        assert_eq!(history.entries(), [Entry::Assistant("oi".into())]);
    }

    #[test]
    fn renders_entries_with_prefixes() {
        let mut history = History::default();
        history.push(Entry::User("lista\narquivos".into()));
        history.start_text();
        history.append_text("Vou listar.");
        history.push(Entry::Tool {
            name: "Bash".into(),
            detail: "ls".into(),
        });
        history.push(Entry::ToolResult {
            is_error: false,
            summary: "a.txt".into(),
        });
        history.push(Entry::Error("falhou".into()));
        let lines = screen(&history, 30, 12);
        assert_eq!(
            &lines[1..10],
            [
                "  › lista",
                "    arquivos",
                "",
                "  ⏺ Vou listar.",
                "",
                "  ⏺ Bash(ls)",
                "    ⎿ a.txt",
                "",
                "  ✘ falhou",
            ]
        );
    }

    #[test]
    fn wraps_by_word_and_aligns_continuation() {
        let mut history = History::default();
        history.push(Entry::Assistant("um dois tres quatro".into()));
        // Área de texto: 14 - 4 (padding) = 10 colunas.
        let lines = screen(&history, 14, 6);
        assert_eq!(&lines[1..4], ["  ⏺ um dois", "    tres", "    quatro"]);
    }

    #[test]
    fn follows_the_end_and_scrolls_back() {
        // 10 entradas + 9 linhas em branco = 19 linhas; 5 visíveis (6 - padding).
        let mut history = info_history(10);
        assert_eq!(screen(&history, 20, 6)[5], "  · linha 9");
        // Duas linhas acima: "linha 8" vira a última visível.
        history.scroll_up(2);
        assert_eq!(text_of(&screen(&history, 20, 6))[5], "  · linha 8");
        history.scroll_up(1000);
        assert_eq!(text_of(&screen(&history, 20, 6))[1], "  · linha 0");
        history.follow();
        assert_eq!(text_of(&screen(&history, 20, 6))[5], "  · linha 9");
    }

    #[test]
    fn scrollbar_shows_after_scrolling_and_hides_after_two_seconds() {
        let has_bar = |history: &History| {
            screen(history, 20, 6)[1..]
                .iter()
                .any(|l| l.ends_with('│') || l.ends_with('┃'))
        };
        let mut history = info_history(10);
        assert!(!has_bar(&history), "sem rolar, não aparece");
        history.scroll_up(3);
        assert!(has_bar(&history));
        for _ in 0..SCROLLBAR_TICKS - 1 {
            history.tick();
        }
        assert!(has_bar(&history));
        history.tick();
        assert!(!has_bar(&history), "some depois de 2 s");
    }

    #[test]
    fn no_scrollbar_when_conversation_fits() {
        let mut history = info_history(2);
        screen(&history, 20, 10);
        history.scroll_up(3);
        assert!(
            !screen(&history, 20, 10)
                .iter()
                .any(|l| l.ends_with('│') || l.ends_with('┃'))
        );
    }

    #[test]
    fn drag_selects_text_without_prefixes() {
        let mut history = History::default();
        history.push(Entry::User("oi".into()));
        history.push(Entry::Assistant("um dois tres quatro".into()));
        screen(&history, 14, 8);
        // Texto começa em x = 2, y = 1. "⏺ um dois" fica em y = 3, e as
        // continuações "tres" e "quatro" em y = 4 e 5.
        assert_eq!(history.mouse_down(4, 3), None);
        history.mouse_drag(7, 4);
        assert_eq!(history.mouse_up(7, 4).as_deref(), Some("um dois tres"));
    }

    #[test]
    fn selection_across_entries_keeps_line_breaks() {
        let mut history = History::default();
        history.push(Entry::User("pergunta".into()));
        history.push(Entry::Assistant("resposta".into()));
        screen(&history, 30, 8);
        history.mouse_down(0, 1);
        assert_eq!(
            history.mouse_up(29, 3).as_deref(),
            Some("pergunta\n\nresposta")
        );
    }

    #[test]
    fn click_without_drag_clears_selection() {
        let mut history = History::default();
        history.push(Entry::Assistant("resposta".into()));
        screen(&history, 30, 6);
        history.mouse_down(5, 1);
        assert_eq!(history.mouse_up(5, 1), None);
        assert_eq!(history.selection, None);
    }

    #[test]
    fn double_click_selects_whole_message() {
        let mut history = History::default();
        history.push(Entry::User("oi".into()));
        history.push(Entry::Assistant("um dois tres quatro\nfim".into()));
        screen(&history, 14, 10);
        history.mouse_down(6, 4);
        history.mouse_up(6, 4);
        history.tick();
        assert_eq!(
            history.mouse_down(6, 4).as_deref(),
            Some("um dois tres quatro\nfim")
        );
        // A seleção aparece destacada.
        let mut terminal = Terminal::new(TestBackend::new(14, 10)).unwrap();
        terminal
            .draw(|frame| history.draw(frame, frame.area(), Vec::new()))
            .unwrap();
        let bg = theme::selection().bg.unwrap();
        assert_eq!(terminal.backend().buffer()[(4, 3)].bg, bg);
        assert_ne!(terminal.backend().buffer()[(2, 3)].bg, bg, "o prefixo não");
    }

    #[test]
    fn slow_second_click_is_not_double_click() {
        let mut history = History::default();
        history.push(Entry::Assistant("resposta".into()));
        screen(&history, 30, 6);
        history.mouse_down(5, 1);
        history.mouse_up(5, 1);
        for _ in 0..=DOUBLE_CLICK_TICKS {
            history.tick();
        }
        assert_eq!(history.mouse_down(5, 1), None);
    }

    #[test]
    fn empty_history_shows_hint() {
        let history = History::default();
        assert_eq!(screen(&history, 20, 4)[1], "  vazio");
    }
}
