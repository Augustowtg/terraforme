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
//!
//! - [`markdown`]: o texto do agente formatado (títulos, listas, ênfase);
//! - [`code`]: o realce dos blocos de código.

mod code;
mod markdown;

use std::cell::{Cell, RefCell};

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{self, Variant};

/// Ticks (de 100 ms) que a barra de rolagem fica visível depois de rolar.
const SCROLLBAR_TICKS: u64 = 20;
/// Dois cliques na mesma linha em até 4 ticks (~400 ms) são um duplo clique.
const DOUBLE_CLICK_TICKS: u64 = 4;
/// Colunas de uma parada de tabulação.
const TAB_WIDTH: usize = 4;
/// Junta emojis num só; cada terminal desenha o conjunto com uma largura.
const ZERO_WIDTH_JOINER: char = '\u{200d}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        detail: String,
    },
    ToolResult {
        is_error: bool,
        summary: String,
    },
    Error(String),
    /// Um aviso do terra (não do agente).
    Info(String),
    /// Um passo do agente (pensando, subagente, nova tentativa...). Os
    /// passos e as ferramentas de uma vez ficam numa árvore embaixo do
    /// pensamento.
    Step(String),
}

/// O pensamento sem o texto (o Claude manda antes do texto dele). Abre a
/// árvore dos passos que não vieram depois de um pensamento.
const THINKING: &str = "Pensando...";
/// O começo de um passo com o texto do pensamento.
const THOUGHT_PREFIX: &str = "Pensamento: ";

/// O passo é um pensamento: abre uma árvore.
fn is_thought(entry: &Entry) -> bool {
    matches!(entry, Entry::Step(text) if text == THINKING || text.starts_with(THOUGHT_PREFIX))
}

/// Entra na árvore de um pensamento: os passos e as ferramentas.
fn is_step(entry: &Entry) -> bool {
    matches!(
        entry,
        Entry::Step(_) | Entry::Tool { .. } | Entry::ToolResult { .. }
    )
}

/// Onde a entrada fica na árvore do pensamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Fora da árvore (ou o próprio pensamento).
    Root,
    /// Um galho (`├─`, ou `└─` no último).
    Branch { last: bool },
    /// O resultado do galho de cima, sem galho próprio.
    Under { last: bool },
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
    scroll: Cell<usize>,
    /// Quanto dá para rolar, calculado no último frame.
    max_scroll: Cell<usize>,
    area: Cell<Rect>,
    view: Cell<View>,
    /// Linhas do último frame, para o mouse e a cópia. Ficam guardadas entre
    /// os frames: só as entradas novas ou que mudaram são quebradas de novo.
    rows: RefCell<Vec<Row>>,
    /// Linha em que começa cada entrada já quebrada em `rows`.
    starts: RefCell<Vec<usize>>,
    /// Largura em que `rows` foi quebrado.
    laid_width: Cell<u16>,
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
        let placeholder =
            |e: Option<&Entry>| matches!(e, Some(Entry::Step(text)) if text == THINKING);
        if placeholder(self.entries.last()) && is_thought(&entry) {
            // O texto do pensamento chega depois do "Pensando...": fica no
            // lugar dele.
            let last = self.entries.len() - 1;
            self.entries[last] = entry;
            self.invalidate(last);
            return;
        }
        if let (Entry::Tool { name, .. }, Some(Entry::Step(text))) = (&entry, self.entries.last())
            && *text == format!("Preparando {name}...")
        {
            // O "Preparando Bash..." (enquanto a entrada da ferramenta chega)
            // vira a própria ferramenta: um galho só.
            let last = self.entries.len() - 1;
            self.entries[last] = entry;
            self.invalidate(last);
            return;
        }
        if is_step(&entry) && !is_thought(&entry) && !self.entries.last().is_some_and(is_step) {
            // Os passos sem pensamento antes (ex.: o Antigravity) ganham um.
            self.entries.push(Entry::Step(THINKING.to_string()));
        }
        self.entries.push(entry);
        // O galho de cima vira `├─` com este embaixo: a árvore é refeita.
        if let Some(head) = self.thought_of(self.entries.len() - 1) {
            self.invalidate(head + 1);
        }
    }

    /// O pensamento em cuja árvore fica a entrada `index`; `None` fora de
    /// uma árvore (e no próprio pensamento).
    fn thought_of(&self, index: usize) -> Option<usize> {
        if !is_step(&self.entries[index]) || is_thought(&self.entries[index]) {
            return None;
        }
        (0..index)
            .rev()
            .take_while(|&i| is_step(&self.entries[i]))
            .find(|&i| is_thought(&self.entries[i]))
    }

    fn place(&self, index: usize) -> Place {
        if self.thought_of(index).is_none() {
            return Place::Root;
        }
        // O último galho: depois dele (e dos resultados dele) a árvore acaba.
        let last = !self.entries[index + 1..]
            .iter()
            .find(|e| !matches!(e, Entry::ToolResult { .. }))
            .is_some_and(|e| is_step(e) && !is_thought(e));
        match self.entries[index] {
            Entry::ToolResult { .. } => Place::Under { last },
            _ => Place::Branch { last },
        }
    }

    /// Começa um novo bloco de texto do agente.
    pub fn start_text(&mut self) {
        self.entries.push(Entry::Assistant(String::new()));
    }

    /// Acrescenta um trecho ao texto do agente em andamento.
    pub fn append_text(&mut self, text: &str) {
        match self.entries.last_mut() {
            Some(Entry::Assistant(current)) => {
                current.push_str(text);
                self.invalidate(self.entries.len() - 1);
            }
            _ => self.entries.push(Entry::Assistant(text.to_string())),
        }
    }

    /// A entrada `index` mudou: as linhas dela são quebradas de novo.
    fn invalidate(&mut self, index: usize) {
        let starts = self.starts.get_mut();
        if let Some(&start) = starts.get(index) {
            self.rows.get_mut().truncate(start);
            starts.truncate(index);
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn tick(&mut self) {
        self.ticks += 1;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll
            .set((self.scroll.get() + lines).min(self.max_scroll.get()));
        self.last_scroll = Some(self.ticks);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll.set(self.scroll.get().saturating_sub(lines));
        self.last_scroll = Some(self.ticks);
    }

    /// Volta a acompanhar o fim da conversa.
    pub fn follow(&mut self) {
        self.scroll.set(0);
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

    /// A palavra sob a célula da tela (para o Ctrl+clique abrir o link ou o
    /// arquivo): o texto entre espaços e bordas, na linha da tela.
    // shortcut: um link quebrado em duas linhas vem cortado; juntar as linhas `wrapped` se incomodar.
    pub fn word_at(&self, column: u16, row: u16) -> Option<String> {
        let (r, col) = self.point_at(column, row);
        let rows = self.rows.borrow();
        let glyphs = &rows.get(r)?.glyphs;
        let mut x = 0;
        let at = glyphs.iter().position(|g| {
            x += g.ch.width().unwrap_or(0);
            x > col
        })?;
        let part = |g: &Glyph| g.text && !g.ch.is_whitespace() && !matches!(g.ch, '│' | '`');
        if !part(&glyphs[at]) {
            return None;
        }
        let start = glyphs[..at]
            .iter()
            .rposition(|g| !part(g))
            .map_or(0, |i| i + 1);
        let end = glyphs[at..]
            .iter()
            .position(|g| !part(g))
            .map_or(glyphs.len(), |i| at + i);
        Some(glyphs[start..end].iter().map(|g| g.ch).collect())
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

    /// Desfaz a seleção (ex.: quando outra começa no input).
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selecting = false;
    }

    /// O trecho selecionado; `None` sem seleção.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.ordered_selection()?;
        let rows = self.rows.borrow();
        let mut out = String::new();
        for (r, row) in rows.iter().enumerate().take(end.0 + 1).skip(start.0) {
            // As bordas das caixas (mensagem, código, tabela) não viram linha.
            let border = !row.glyphs.iter().any(|g| g.text)
                && row.glyphs.iter().any(|g| matches!(g.ch, '╭' | '╰' | '├'));
            if border {
                continue;
            }
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
            self.starts.borrow_mut().clear();
            self.max_scroll.set(0);
            frame.render_widget(Paragraph::new(empty), inner);
            return;
        }

        self.layout(inner.width);
        let rows = self.rows.borrow();
        let height = inner.height as usize;
        let max = rows.len().saturating_sub(height);
        // Rolado para cima, o trecho na tela fica onde está enquanto a
        // conversa cresce embaixo (ex.: a resposta chegando).
        let grown = max.saturating_sub(self.max_scroll.replace(max));
        if self.scroll.get() > 0 {
            self.scroll.set(self.scroll.get() + grown);
        }
        let top = max - self.scroll.get().min(max);
        self.view.set(View { inner, top });

        let selection = self.ordered_selection();
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(r, row)| {
                let mut col = 0;
                let mut spans: Vec<Span> = Vec::with_capacity(row.glyphs.len());
                for glyph in &row.glyphs {
                    let width = glyph.ch.width().unwrap_or(0);
                    if width == 0 {
                        // Um acento separado da letra (`e` + `´`) vai no
                        // mesmo span que ela; sozinho, o ratatui o descarta.
                        // O que mudaria a largura (ex.: o seletor de emoji)
                        // fica de fora, para as colunas baterem com o mouse.
                        if glyph.ch != ZERO_WIDTH_JOINER
                            && let Some(last) = spans.last_mut()
                        {
                            let joined = format!("{}{}", last.content, glyph.ch);
                            if joined.width() == last.content.width() {
                                last.content = joined.into();
                            }
                        }
                        continue;
                    }
                    let selected =
                        glyph.text && selection.is_some_and(|(s, e)| (s..=e).contains(&(r, col)));
                    col += width;
                    let style = if selected {
                        theme::selection()
                    } else {
                        glyph.style
                    };
                    spans.push(Span::styled(glyph.ch.to_string(), style));
                }
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        drop(rows);

        if self.scrollbar_visible() {
            let mut state = ScrollbarState::new(max + 1)
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

    /// Quebra em `width` colunas, em `self.rows`, as entradas que ainda não
    /// estão lá (as novas e a que mudou); em outra largura, todas.
    fn layout(&self, width: u16) {
        let mut rows = self.rows.borrow_mut();
        let mut starts = self.starts.borrow_mut();
        if self.laid_width.replace(width) != width {
            rows.clear();
            starts.clear();
        }
        for (i, entry) in self.entries.iter().enumerate().skip(starts.len()) {
            starts.push(rows.len());
            if matches!(entry, Entry::Assistant(text) if text.trim().is_empty()) {
                continue;
            }
            let place = self.place(i);
            // Linha em branco entre as entradas; o resultado fica colado na
            // ferramenta, e a árvore, no pensamento.
            if !rows.is_empty()
                && place == Place::Root
                && !matches!(entry, Entry::ToolResult { .. })
            {
                rows.push(Row::default());
            }
            if place != Place::Root {
                let (first, rest) = match place {
                    Place::Branch { last: false } => ("├─ ", "│  "),
                    Place::Branch { last: true } => ("└─ ", "   "),
                    Place::Under { last: false } => ("│  ", "│  "),
                    Place::Under { last: true } => ("   ", "   "),
                    Place::Root => unreachable!(),
                };
                let inner = width.saturating_sub(3).max(1);
                let body = logical_lines(entry)
                    .into_iter()
                    .flat_map(|(prefix, body)| wrap(prefix, body, inner, Some(i)));
                for (n, mut row) in body.enumerate() {
                    let branch = if n == 0 { first } else { rest };
                    let mut gs = glyphs(branch, theme::muted(), false);
                    gs.append(&mut row.glyphs);
                    row.glyphs = gs;
                    rows.push(row);
                }
                continue;
            }
            if let Entry::Assistant(text) = entry {
                let inner = (width as usize).max(5) - 4;
                let body = markdown::layout(text.trim(), inner as u16, Some(i));
                rows.extend(framed(body, width, theme::border(), Some(i)));
                continue;
            }
            if let Entry::User(text) = entry {
                rows.extend(user_box(text, width, Some(i)));
                continue;
            }
            for (prefix, body) in logical_lines(entry) {
                rows.extend(wrap(prefix, body, width, Some(i)));
            }
        }
    }
}

/// A mensagem do usuário numa caixa de borda arredondada (cor vinho), na
/// largura toda.
fn user_box(text: &str, width: u16, entry: Option<usize>) -> Vec<Row> {
    let inner = (width as usize).max(5) - 4;
    let body = text
        .split('\n')
        .flat_map(|line| {
            wrap(
                Vec::new(),
                glyphs(line, theme::text(), true),
                inner as u16,
                entry,
            )
        })
        .collect();
    framed(body, width, theme::user_border(), entry)
}

/// `body`, já quebrado em `width - 4` colunas, numa caixa de borda
/// arredondada na largura toda.
fn framed(body: Vec<Row>, width: u16, border: Style, entry: Option<usize>) -> Vec<Row> {
    // Borda + espaço de cada lado.
    let outer = (width as usize).max(5);
    let inner = outer - 4;
    let edge = |left: char, right: char| Row {
        glyphs: glyphs(
            &format!("{left}{}{right}", "─".repeat(outer - 2)),
            border,
            false,
        ),
        wrapped: false,
        entry,
    };
    let mut rows = vec![edge('╭', '╮')];
    for row in body {
        let used = measure(&row.glyphs);
        let mut gs = glyphs("│ ", border, false);
        gs.extend(row.glyphs);
        gs.extend(glyphs(
            &" ".repeat(inner.saturating_sub(used)),
            Style::new(),
            false,
        ));
        gs.extend(glyphs(" │", border, false));
        rows.push(Row {
            glyphs: gs,
            wrapped: row.wrapped,
            entry,
        });
    }
    rows.push(edge('╰', '╯'));
    rows
}

/// A largura de um trecho na tela.
fn measure(gs: &[Glyph]) -> usize {
    gs.iter().map(|g| g.ch.width().unwrap_or(0)).sum()
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
        Entry::Step(text) if is_thought(entry) => prefixed(
            text.strip_prefix(THOUGHT_PREFIX).unwrap_or(text),
            "💡 ",
            theme::variant(Variant::Accent),
            theme::muted().add_modifier(Modifier::ITALIC),
        ),
        Entry::Step(text) => prefixed(text, "· ", theme::muted(), theme::muted()),
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
    for glyph in expand_tabs(body) {
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

/// Troca cada Tab por espaços até a próxima parada de tabulação e tira os
/// outros caracteres de controle (ex.: o `\r` de uma saída do Windows): o
/// ratatui não desenha nenhum deles, e o Tab sumia com a indentação.
fn expand_tabs(body: Vec<Glyph>) -> Vec<Glyph> {
    if !body.iter().any(|g| g.ch.is_control()) {
        return body;
    }
    let mut out = Vec::with_capacity(body.len());
    let mut col = 0;
    for glyph in body {
        if glyph.ch == '\t' {
            let spaces = TAB_WIDTH - col % TAB_WIDTH;
            out.extend(std::iter::repeat_n(Glyph { ch: ' ', ..glyph }, spaces));
            col += spaces;
        } else if !glyph.ch.is_control() {
            col += glyph.ch.width().unwrap_or(0);
            out.push(glyph);
        }
    }
    out
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
    fn the_steps_of_a_thought_are_a_tree_under_it() {
        let mut history = History::default();
        history.push(Entry::Step("Pensamento: vou ler".into()));
        history.push(Entry::Tool {
            name: "Bash".into(),
            detail: "ls".into(),
        });
        history.push(Entry::ToolResult {
            is_error: false,
            summary: "a.txt".into(),
        });
        history.push(Entry::Tool {
            name: "Read".into(),
            detail: "a.txt".into(),
        });
        assert_eq!(
            screen(&history, 30, 6)[1..5],
            [
                "  💡  vou ler",
                "  ├─ ⏺ Bash(ls)",
                "  │    ⎿ a.txt",
                "  └─ ⏺ Read(a.txt)",
            ]
        );
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
        let lines = screen(&history, 30, 15);
        assert_eq!(
            &lines[1..15],
            [
                "  ╭────────────────────────╮",
                "  │ lista                  │",
                "  │ arquivos               │",
                "  ╰────────────────────────╯",
                "",
                "  ╭────────────────────────╮",
                "  │ ⏺ Vou listar.          │",
                "  ╰────────────────────────╯",
                "",
                "  💡  Pensando...",
                "  └─ ⏺ Bash(ls)",
                "       ⎿ a.txt",
                "",
                "  ✘ falhou",
            ]
        );
    }

    #[test]
    fn wraps_by_word_and_aligns_continuation() {
        let mut history = History::default();
        history.push(Entry::Assistant("um dois tres quatro".into()));
        // Área de texto: 18 - 4 (padding) = 14 colunas; 10 dentro da caixa.
        let lines = screen(&history, 18, 7);
        assert_eq!(
            &lines[2..5],
            ["  │ ⏺ um dois  │", "  │   tres     │", "  │   quatro   │"]
        );
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
        screen(&history, 18, 10);
        // Texto começa em x = 2, y = 1; a caixa do "oi" vai de y = 1 a 3 e a
        // da resposta começa em y = 5. "⏺ um dois" fica em y = 6, com o texto
        // a partir de x = 6, e as continuações "tres" e "quatro" em y = 7 e 8.
        assert_eq!(history.mouse_down(6, 6), None);
        history.mouse_drag(9, 7);
        assert_eq!(history.mouse_up(9, 7).as_deref(), Some("um dois tres"));
    }

    #[test]
    fn selection_across_entries_keeps_line_breaks() {
        let mut history = History::default();
        history.push(Entry::User("pergunta".into()));
        history.push(Entry::Assistant("resposta".into()));
        screen(&history, 30, 9);
        history.mouse_down(0, 1);
        assert_eq!(
            history.mouse_up(29, 6).as_deref(),
            Some("pergunta\n\nresposta")
        );
    }

    #[test]
    fn word_at_takes_the_word_under_the_mouse() {
        let mut history = History::default();
        history.push(Entry::Assistant("veja https://a.com/x agora".into()));
        screen(&history, 40, 6);
        // "⏺ veja https..." na linha 2, texto a partir de x = 6.
        assert_eq!(history.word_at(13, 2).as_deref(), Some("https://a.com/x"));
        assert_eq!(history.word_at(3, 2), None, "a borda não");
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
        screen(&history, 18, 12);
        history.mouse_down(8, 7);
        history.mouse_up(8, 7);
        history.tick();
        assert_eq!(
            history.mouse_down(8, 7).as_deref(),
            Some("um dois tres quatro\nfim")
        );
        // A seleção aparece destacada.
        let mut terminal = Terminal::new(TestBackend::new(18, 12)).unwrap();
        terminal
            .draw(|frame| history.draw(frame, frame.area(), Vec::new()))
            .unwrap();
        let bg = theme::selection().bg.unwrap();
        assert_eq!(terminal.backend().buffer()[(6, 6)].bg, bg);
        assert_ne!(terminal.backend().buffer()[(4, 6)].bg, bg, "o prefixo não");
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

    #[test]
    fn tabs_become_spaces_and_control_characters_go_away() {
        let mut history = History::default();
        history.push(Entry::User("a\tb\r".into()));
        history.push(Entry::Assistant("```go\n\tx := 1\n```".into()));
        let lines = screen(&history, 30, 11);
        assert!(lines[2].starts_with("  │ a   b "), "{:?}", lines[2]);
        assert!(lines[7].starts_with("  │   │     x := 1"), "{:?}", lines[7]);
        // Na cópia, o `\r` não vai.
        history.mouse_down(0, 2);
        assert_eq!(history.mouse_up(29, 2).as_deref(), Some("a   b"));
    }

    #[test]
    fn combining_accent_stays_with_its_letter() {
        let mut history = History::default();
        history.push(Entry::User("cafe\u{301} ok".into()));
        let lines = screen(&history, 30, 4);
        assert!(
            lines[2].starts_with("  │ cafe\u{301} ok "),
            "{:?}",
            lines[2]
        );
    }

    #[test]
    fn scrolled_up_view_stays_put_while_the_answer_arrives() {
        let mut history = info_history(10);
        screen(&history, 20, 6);
        history.scroll_up(4);
        let before = text_of(&screen(&history, 20, 6));
        history.start_text();
        history.append_text("um\n\ndois\n\ntres");
        assert_eq!(text_of(&screen(&history, 20, 6)), before);
        history.append_text("\n\nquatro");
        assert_eq!(text_of(&screen(&history, 20, 6)), before);
        // De volta ao fim, acompanha a resposta.
        history.follow();
        history.append_text("\n\ncinco");
        assert_eq!(text_of(&screen(&history, 20, 6))[4], "  │   cinco      │");
    }

    #[test]
    fn streamed_text_and_a_new_width_are_laid_out_again() {
        let mut history = History::default();
        history.push(Entry::User("oi".into()));
        history.start_text();
        history.append_text("um dois");
        assert_eq!(
            &screen(&history, 30, 8)[1..8],
            [
                "  ╭────────────────────────╮",
                "  │ oi                     │",
                "  ╰────────────────────────╯",
                "",
                "  ╭────────────────────────╮",
                "  │ ⏺ um dois              │",
                "  ╰────────────────────────╯",
            ]
        );
        history.append_text(" tres quatro");
        assert_eq!(screen(&history, 30, 8)[6], "  │ ⏺ um dois tres quatro  │");
        assert_eq!(
            &screen(&history, 18, 10)[6..9],
            ["  │ ⏺ um dois  │", "  │   tres     │", "  │   quatro   │"]
        );
        history.push(Entry::Info("fim".into()));
        assert_eq!(screen(&history, 30, 10)[9], "  · fim");
    }
}
