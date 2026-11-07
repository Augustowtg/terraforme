//! A seção **Sessão** da aba Consumo: o que os chats do terra gastaram na pasta
//! aberta, que fica no SQLite do workspace e continua de uma execução à outra,
//! e o que a aba de chat atual gastou desde que abriu.
//!
//! ```text
//!  Sessão · terraforme-cli
//!    Simplificado    Detalhado
//!                  Input     Output    Cache
//!   Aba Principal  1,2K      3,4K      120K
//!   Pasta          45K       80K       2,1M
//! ```
//!
//! O modo **Detalhado** ocupa a aba e soma os custos e a análise: por tipo de
//! token, por modelo, por aba aberta e por dia. O custo é o que o provedor
//! informa, por modelo; ele não informa quanto foi de cada tipo de token.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use super::{BUTTON_GAP, format_date, indent};
use crate::agent::{ModelSpend, merge_spend, total_spend};
use crate::db::history::{ChatHistory, FolderUsage};
use crate::theme;
use crate::tui::chat::compact_count;
use crate::tui::orchestrator::usd;

/// De quanto em quanto tempo a sessão é relida com a aba aberta.
const RELOAD: Duration = Duration::from_secs(2);
const SIMPLE_BUTTON: &str = " Simplificado ";
const DETAIL_BUTTON: &str = " Detalhado ";
/// Largura da coluna de nomes, onde cabe o nome de um modelo.
const NAME_WIDTH: usize = 20;
/// Largura de uma coluna de números, com a folga para a próxima.
const COLUMN: usize = 14;
/// O que a página detalhada anda com `PageUp` e `PageDown`.
const PAGE: u16 = 10;
/// O que a página detalhada anda com a roda do mouse.
const WHEEL: u16 = 3;
const COST_NOTE: &str = "Custo estimado pelo provedor, a preço de lista: num plano de assinatura, não é o valor cobrado.";
const KIND_NOTE: &str = "O provedor informa o custo por modelo, não por tipo de token.";
const UNKNOWN_COST: &str = "não informado";

/// O que um chat aberto gastou.
pub struct TabSpend {
    pub title: String,
    pub spend: Vec<ModelSpend>,
    /// O turno em andamento, que ainda não está na sessão da pasta.
    pub running: Vec<ModelSpend>,
    /// Por que o consumo da aba pode não ser o esperado.
    pub note: Option<&'static str>,
}

#[derive(Default)]
pub(super) struct SessionView {
    /// O SQLite do workspace e a pasta da sessão (`None` nos testes).
    source: Option<(PathBuf, String)>,
    store: Option<ChatHistory>,
    usage: FolderUsage,
    loaded_at: Option<Instant>,
    error: Option<String>,
    detailed: bool,
    scroll: Cell<u16>,
    /// Onde os botões Simplificado e Detalhado ficaram no último desenho.
    buttons: Cell<[Rect; 2]>,
}

impl SessionView {
    pub(super) fn set_source(&mut self, file: PathBuf, folder: String) {
        self.source = Some((file, folder));
    }

    pub(super) fn detailed(&self) -> bool {
        self.detailed
    }

    pub(super) fn reload_if_stale(&mut self) {
        if self.loaded_at.is_none_or(|at| at.elapsed() >= RELOAD) {
            self.reload();
        }
    }

    pub(super) fn reload(&mut self) {
        self.loaded_at = Some(Instant::now());
        let Some((file, folder)) = &self.source else {
            return;
        };
        // Sem o arquivo ainda não há turno, e ele continua sem ser criado.
        if self.store.is_none() && !file.is_file() {
            return;
        }
        let result = match &self.store {
            Some(store) => store.folder_usage(folder),
            None => ChatHistory::open(file).and_then(|store| {
                let usage = store.folder_usage(folder);
                self.store = Some(store);
                usage
            }),
        };
        match result {
            Ok(usage) => {
                self.usage = usage;
                self.error = None;
            }
            Err(err) => self.error = Some(format!("O consumo da sessão não foi lido: {err:#}")),
        }
    }

    fn set_detailed(&mut self, detailed: bool) {
        self.detailed = detailed;
        self.scroll.set(0);
    }

    /// `d` troca o modo; no detalhado, as setas rolam e `Esc` volta.
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> bool {
        let scroll = self.scroll.get();
        match key.code {
            KeyCode::Char('d') => self.set_detailed(!self.detailed),
            KeyCode::Esc if self.detailed => self.set_detailed(false),
            KeyCode::Up if self.detailed => self.scroll.set(scroll.saturating_sub(1)),
            KeyCode::Down if self.detailed => self.scroll.set(scroll.saturating_add(1)),
            KeyCode::PageUp if self.detailed => self.scroll.set(scroll.saturating_sub(PAGE)),
            KeyCode::PageDown if self.detailed => self.scroll.set(scroll.saturating_add(PAGE)),
            _ => return false,
        }
        true
    }

    /// Clique nos botões e, no detalhado, a roda. No detalhado, o que estava
    /// embaixo da página não recebe o clique.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let scroll = self.scroll.get();
        match mouse.kind {
            MouseEventKind::ScrollUp if self.detailed => {
                self.scroll.set(scroll.saturating_sub(WHEEL));
            }
            MouseEventKind::ScrollDown if self.detailed => {
                self.scroll.set(scroll.saturating_add(WHEEL));
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                if let Some(i) = self.buttons.get().iter().position(|r| r.contains(at)) {
                    self.set_detailed(i == 1);
                    return true;
                }
            }
            _ => {}
        }
        self.detailed
    }

    /// Guarda onde os botões ficaram: `inner` é a área das linhas da seção.
    pub(super) fn place_buttons(&self, inner: Rect) {
        let simple = SIMPLE_BUTTON.chars().count() as u16;
        let detail = DETAIL_BUTTON.chars().count() as u16;
        let y = inner.y + 1;
        let rect = |x, width| {
            Rect {
                x,
                y,
                width,
                height: 1,
            }
            .intersection(inner)
        };
        let second = inner.x + 2 + simple + BUTTON_GAP.len() as u16;
        self.buttons
            .set([rect(inner.x + 2, simple), rect(second, detail)]);
    }

    fn title(&self) -> Line<'static> {
        let name = self.source.as_ref().map(|(_, folder)| {
            std::path::Path::new(folder)
                .file_name()
                .map_or_else(|| folder.clone(), |n| n.to_string_lossy().into_owned())
        });
        match name {
            Some(name) => Line::styled(format!(" Sessão · {name}"), theme::section_title()),
            None => Line::styled(" Sessão", theme::section_title()),
        }
    }

    fn button_line(&self) -> Line<'static> {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(SIMPLE_BUTTON, theme::without_cache(!self.detailed)),
            Span::raw(BUTTON_GAP),
            Span::styled(DETAIL_BUTTON, theme::without_cache(self.detailed)),
        ])
    }

    /// O total da pasta: o que está no banco e os turnos em andamento aqui.
    fn folder_spend(&self, tabs: &[TabSpend]) -> ModelSpend {
        let mut spend = self.usage.models.clone();
        for tab in tabs {
            merge_spend(&mut spend, &tab.running);
        }
        total_spend(&spend)
    }

    /// As linhas do modo simplificado; a segunda é a dos botões.
    pub(super) fn simple_lines(&self, tabs: &[TabSpend], current: usize) -> Vec<Line<'static>> {
        let mut lines = vec![self.title(), self.button_line()];
        lines.push(table_row(
            "",
            &["Input", "Output", "Cache"].map(String::from),
            theme::muted(),
        ));
        let row = |label: &str, spend: &ModelSpend| {
            let cells = [spend.input, spend.output, spend.cache()].map(compact_count);
            table_row(label, &cells, theme::text())
        };
        let tab = tabs.get(current);
        let label = tab.map_or("Aba".to_string(), |tab| format!("Aba {}", tab.title));
        let spend = tab.map_or_else(ModelSpend::default, |tab| total_spend(&tab.spend));
        lines.push(row(&label, &spend));
        lines.push(row("Pasta", &self.folder_spend(tabs)));
        if let Some(note) = tab.and_then(|tab| tab.note) {
            lines.push(indent(Span::styled(note, theme::muted())));
        }
        if let Some(error) = &self.error {
            lines.push(indent(Span::styled(error.clone(), theme::error())));
        }
        lines.push(Line::default());
        lines
    }

    /// O modo detalhado em toda a `area`: o título e os botões ficam parados
    /// e o resto rola.
    pub(super) fn draw_detail(
        &self,
        frame: &mut Frame,
        area: Rect,
        tabs: &[TabSpend],
        current: usize,
    ) {
        let inner = Block::new().padding(Padding::new(1, 2, 1, 0)).inner(area);
        let [head, body] =
            Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(inner);
        frame.render_widget(Paragraph::new(vec![self.title(), self.button_line()]), head);
        self.place_buttons(head);
        let folder = self.source.as_ref().map(|(_, folder)| folder.as_str());
        let lines = detail_lines(
            &self.usage,
            self.folder_spend(tabs),
            tabs,
            current,
            folder,
            self.error.as_deref(),
            &TimeZone::system(),
        );
        let last = (lines.len() as u16).saturating_sub(body.height);
        self.scroll.set(self.scroll.get().min(last));
        frame.render_widget(Paragraph::new(lines).scroll((self.scroll.get(), 0)), body);
    }
}

/// O título da coluna de nomes: cortado, para não empurrar os números.
fn clip(label: &str) -> String {
    let width = NAME_WIDTH - 1;
    if label.chars().count() <= width {
        return label.to_string();
    }
    let mut clipped: String = label.chars().take(width - 1).collect();
    clipped.push('…');
    clipped
}

/// Uma linha com o nome e as colunas de números.
fn table_row(label: &str, cells: &[String], style: ratatui::style::Style) -> Line<'static> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(format!("{:<NAME_WIDTH$}", clip(label)), theme::muted()),
    ];
    spans.extend(
        cells
            .iter()
            .map(|cell| Span::styled(format!("{cell:<COLUMN$}"), style)),
    );
    Line::from(spans)
}

/// O nome e um valor, na página detalhada.
fn fact(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{label:<26}"), theme::muted()),
        Span::styled(value, theme::text()),
    ])
}

fn section(title: &str) -> Line<'static> {
    Line::styled(format!(" {title}"), theme::section_title())
}

fn cost(micros: Option<u64>) -> String {
    micros.map_or_else(
        || UNKNOWN_COST.to_string(),
        |micros| usd(micros as f64 / 1e6),
    )
}

/// Quanto `part` é de `whole`, como `12%`.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "-".to_string();
    }
    format!("{}%", (part as f64 / whole as f64 * 100.0).round() as u64)
}

/// Todas as entradas do modelo: a nova e as de cache.
fn all_input(spend: &ModelSpend) -> u64 {
    spend.input + spend.cache()
}

fn model_name(model: &str) -> &str {
    if model.is_empty() {
        "desconhecido"
    } else {
        model
    }
}

/// A linha de um gasto nas tabelas por modelo e por aba.
fn spend_row(label: &str, spend: &ModelSpend, total_cost: u64) -> Line<'static> {
    let mut cells = [spend.input, spend.output, spend.cache()]
        .map(compact_count)
        .to_vec();
    cells.push(cost(spend.cost_micros));
    if let Some(micros) = spend.cost_micros {
        cells.push(percent(micros, total_cost));
    }
    table_row(label, &cells, theme::text())
}

fn spend_header() -> Line<'static> {
    table_row(
        "",
        &["Input", "Output", "Cache", "Custo"].map(String::from),
        theme::muted(),
    )
}

/// As linhas da página detalhada. `folder_spend` é o total da pasta com os
/// turnos em andamento.
fn detail_lines(
    usage: &FolderUsage,
    folder_spend: ModelSpend,
    tabs: &[TabSpend],
    current: usize,
    folder: Option<&str>,
    error: Option<&str>,
    tz: &TimeZone,
) -> Vec<Line<'static>> {
    let tab = tabs.get(current);
    let tab_spend = tab.map_or_else(ModelSpend::default, |tab| total_spend(&tab.spend));
    let mut lines = Vec::new();
    if let Some(error) = error {
        lines.push(indent(Span::styled(error.to_string(), theme::error())));
    }
    if let Some(folder) = folder {
        lines.push(fact("Pasta", folder.to_string()));
    }
    lines.push(fact(
        "Aba atual",
        tab.map_or_else(String::new, |tab| tab.title.clone()),
    ));
    if let Some(note) = tab.and_then(|tab| tab.note) {
        lines.push(indent(Span::styled(note, theme::muted())));
    }
    lines.push(Line::default());

    lines.push(section("Tokens"));
    let tokens = folder_spend.input + folder_spend.output + folder_spend.cache();
    lines.push(table_row(
        "",
        &["Esta aba", "Pasta", "% da pasta"].map(String::from),
        theme::muted(),
    ));
    let kinds = [
        ("Input", tab_spend.input, folder_spend.input),
        ("Output", tab_spend.output, folder_spend.output),
        (
            "Escrita de cache",
            tab_spend.cache_creation,
            folder_spend.cache_creation,
        ),
        (
            "Leitura de cache",
            tab_spend.cache_read,
            folder_spend.cache_read,
        ),
    ];
    for (label, of_tab, of_folder) in kinds {
        let cells = [
            compact_count(of_tab),
            compact_count(of_folder),
            percent(of_folder, tokens),
        ];
        lines.push(table_row(label, &cells, theme::text()));
    }
    let tab_tokens = tab_spend.input + tab_spend.output + tab_spend.cache();
    lines.push(table_row(
        "Total",
        &[compact_count(tab_tokens), compact_count(tokens)],
        theme::text(),
    ));
    lines.push(table_row(
        "Custo",
        &[cost(tab_spend.cost_micros), cost(folder_spend.cost_micros)],
        theme::text(),
    ));
    lines.push(Line::default());

    lines.push(section("Análise da pasta"));
    if usage.turns == 0 {
        lines.push(indent(Span::styled(
            "Nenhum turno terminou nesta pasta ainda.",
            theme::muted(),
        )));
    } else {
        if let (Some(first), Some(last)) = (usage.first_at, usage.last_at) {
            lines.push(fact(
                "Período",
                format!(
                    "{} a {}",
                    format_date(first / 1000, tz),
                    format_date(last / 1000, tz)
                ),
            ));
        }
        lines.push(fact(
            "Turnos e conversas",
            format!("{} em {}", usage.turns, usage.conversations),
        ));
        lines.push(fact("Cache reaproveitado", {
            format!(
                "{} da entrada veio da leitura de cache",
                percent(folder_spend.cache_read, all_input(&folder_spend))
            )
        }));
        lines.push(fact(
            "Output por entrada",
            percent(folder_spend.output, all_input(&folder_spend)),
        ));
        lines.push(fact(
            "Tokens por turno",
            compact_count(tokens / usage.turns),
        ));
        if folder_spend.thinking > 0 {
            lines.push(fact(
                "Pensamento",
                format!(
                    "{} ({} do output)",
                    compact_count(folder_spend.thinking),
                    percent(folder_spend.thinking, folder_spend.output)
                ),
            ));
        }
        if folder_spend.web_searches > 0 {
            lines.push(fact("Buscas na web", folder_spend.web_searches.to_string()));
        }
        if let Some(micros) = total_spend(&usage.models).cost_micros {
            let priced = usage.turns.saturating_sub(usage.turns_without_cost).max(1);
            lines.push(fact("Custo por turno", cost(Some(micros / priced))));
            lines.push(fact("Turno mais caro", cost(usage.priciest_turn_micros)));
            let days = usage
                .days
                .iter()
                .filter(|d| d.cost_micros.is_some())
                .count() as u64;
            let last_days: u64 = usage.days.iter().filter_map(|d| d.cost_micros).sum();
            lines.push(fact(
                "Custo por dia (com uso)",
                cost(Some(last_days / days.max(1))),
            ));
        }
        if usage.turns_without_cost > 0 {
            lines.push(fact(
                "Turnos sem custo",
                format!("{} (o provedor não informa)", usage.turns_without_cost),
            ));
        }
    }
    lines.push(Line::default());

    let total_cost = folder_spend.cost_micros.unwrap_or(0);
    if !usage.models.is_empty() {
        lines.push(section("Custo por modelo"));
        lines.push(spend_header());
        for model in &usage.models {
            lines.push(spend_row(model_name(&model.model), model, total_cost));
        }
        lines.push(Line::default());
    }

    lines.push(section("Abas abertas"));
    lines.push(spend_header());
    for (i, tab) in tabs.iter().enumerate() {
        let title = if i == current {
            format!("› {}", tab.title)
        } else {
            tab.title.clone()
        };
        lines.push(spend_row(&title, &total_spend(&tab.spend), total_cost));
    }
    lines.push(Line::default());

    if !usage.days.is_empty() {
        lines.push(section("Últimos dias com uso"));
        lines.push(table_row(
            "",
            &["Turnos", "Tokens", "Cache", "Custo"].map(String::from),
            theme::muted(),
        ));
        for day in &usage.days {
            let cells = [
                day.turns.to_string(),
                compact_count(day.tokens),
                compact_count(day.cache),
                cost(day.cost_micros),
            ];
            lines.push(table_row(&day_label(&day.day), &cells, theme::text()));
        }
        lines.push(Line::default());
    }
    lines.push(indent(Span::styled(COST_NOTE, theme::muted())));
    lines.push(indent(Span::styled(KIND_NOTE, theme::muted())));
    lines
}

/// `2026-11-06` como `06-11-2026`.
fn day_label(day: &str) -> String {
    let parts: Vec<&str> = day.split('-').collect();
    match parts[..] {
        [year, month, day] => format!("{day}-{month}-{year}"),
        _ => day.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history::DayUsage;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn spend(model: &str, input: u64, output: u64, cost_micros: Option<u64>) -> ModelSpend {
        ModelSpend {
            model: model.to_string(),
            input,
            output,
            cache_creation: 100,
            cache_read: 900,
            cost_micros,
            ..ModelSpend::default()
        }
    }

    fn tab(title: &str, spend: Vec<ModelSpend>, running: Vec<ModelSpend>) -> TabSpend {
        TabSpend {
            title: title.to_string(),
            spend,
            running,
            note: None,
        }
    }

    #[test]
    fn simple_shows_input_output_and_cache_of_the_tab_and_the_folder() {
        let view = SessionView {
            usage: FolderUsage {
                models: vec![spend("opus", 40, 60, Some(1_000))],
                ..FolderUsage::default()
            },
            ..SessionView::default()
        };
        let running = spend("opus", 5, 7, None);
        let tabs = [
            tab("Principal", vec![spend("opus", 1, 2, None)], vec![]),
            tab("Chat 2", vec![running.clone()], vec![running]),
        ];
        let lines: Vec<String> = view.simple_lines(&tabs, 1).iter().map(text).collect();
        assert!(lines[2].contains("Input") && lines[2].contains("Cache"));
        let row: Vec<&str> = lines[3].split_whitespace().collect();
        assert_eq!(row, ["Aba", "Chat", "2", "5", "7", "1K"]);
        // A pasta soma o que está no banco e o turno em andamento.
        let row: Vec<&str> = lines[4].split_whitespace().collect();
        assert_eq!(row, ["Pasta", "45", "67", "2K"]);
        assert!(!lines.iter().any(|l| l.contains("US$")), "sem custo aqui");
    }

    #[test]
    fn detail_has_the_costs_and_the_analysis() {
        let models = vec![
            spend("opus", 100, 200, Some(3_000_000)),
            spend("haiku", 100, 200, Some(1_000_000)),
        ];
        let usage = FolderUsage {
            first_at: Some(1_793_124_000_000),
            last_at: Some(1_793_124_000_000),
            turns: 4,
            conversations: 2,
            priciest_turn_micros: Some(2_500_000),
            days: vec![DayUsage {
                day: "2026-11-06".to_string(),
                turns: 4,
                tokens: 600,
                cache: 2_000,
                cost_micros: Some(4_000_000),
            }],
            models: models.clone(),
            ..FolderUsage::default()
        };
        let tabs = [tab("Principal", vec![models[1].clone()], vec![])];
        let lines = detail_lines(
            &usage,
            total_spend(&models),
            &tabs,
            0,
            Some("/x/y"),
            None,
            &TimeZone::UTC,
        );
        let page: Vec<String> = lines.iter().map(text).collect();
        let has = |needles: &[&str]| page.iter().any(|l| needles.iter().all(|n| l.contains(n)));
        assert!(has(&["Pasta", "/x/y"]));
        assert!(has(&["Escrita de cache", "100", "200"]));
        assert!(has(&["Leitura de cache", "900"]));
        assert!(has(&["Custo", "US$ 1,0000", "US$ 4,0000"]));
        assert!(has(&["Turnos e conversas", "4 em 2"]));
        assert!(
            has(&["Cache reaproveitado", "82%"]),
            "1,8K de 2,2K: {page:?}"
        );
        assert!(has(&["Custo por turno", "US$ 1,0000"]));
        assert!(has(&["Turno mais caro", "US$ 2,5000"]));
        assert!(has(&["opus", "US$ 3,0000", "75%"]));
        assert!(has(&["› Principal", "US$ 1,0000", "25%"]));
        assert!(has(&["06-11-2026", "4", "US$ 4,0000"]));
        assert!(has(&["por modelo, não por tipo de token"]));
    }

    #[test]
    fn a_provider_without_cost_says_so_instead_of_zero() {
        let models = vec![spend("gemini", 10, 20, None)];
        let usage = FolderUsage {
            turns: 2,
            turns_without_cost: 2,
            models: models.clone(),
            ..FolderUsage::default()
        };
        let tabs = [tab("Principal", models.clone(), vec![])];
        let page: Vec<String> = detail_lines(
            &usage,
            total_spend(&models),
            &tabs,
            0,
            None,
            None,
            &TimeZone::UTC,
        )
        .iter()
        .map(text)
        .collect();
        assert!(
            page.iter()
                .any(|l| l.contains("gemini") && l.contains(UNKNOWN_COST))
        );
        assert!(
            page.iter()
                .any(|l| l.contains("Turnos sem custo") && l.contains('2'))
        );
        assert!(!page.iter().any(|l| l.contains("US$ 0,0000")));
    }

    #[test]
    fn the_keys_change_the_mode_and_only_the_detail_scrolls() {
        let key = |code| KeyEvent::from(code);
        let mut view = SessionView::default();
        assert!(
            !view.handle_key(key(KeyCode::Down)),
            "o simplificado não rola"
        );
        assert!(
            !view.handle_key(key(KeyCode::Esc)),
            "o Esc segue para a tela"
        );
        assert!(view.handle_key(key(KeyCode::Char('d'))));
        assert!(view.detailed());
        view.handle_key(key(KeyCode::PageDown));
        assert_eq!(view.scroll.get(), PAGE);
        view.handle_key(key(KeyCode::Up));
        assert_eq!(view.scroll.get(), PAGE - 1);
        assert!(view.handle_key(key(KeyCode::Esc)));
        assert!(!view.detailed());
        assert_eq!(view.scroll.get(), 0, "volta ao topo");
    }

    #[test]
    fn a_folder_without_the_file_does_not_create_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        let mut view = SessionView::default();
        view.set_source(file.clone(), "/x".to_string());
        view.reload();
        assert!(!file.exists());
        assert!(view.error.is_none());
    }
}
