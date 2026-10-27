//! Aba Consumo: quanto dos limites de uso da conta ainda resta, pelo route
//! (provedor) do chat.
//!
//! ```text
//!  Consumo
//!   Route           Claude · Pessoal · eu@gmail.com · Max
//!
//!   Sessão (5 h)
//!   ████████████████░░░░░░░░░░░░  58% restantes
//!   Reseta 27-10-2026 15:00 · zera em 2h 13m
//! ```
//!
//! Ao lado (ou embaixo, em telas estreitas), o gráfico **Consumo por dia**:
//! os tokens de cada um dos últimos [`DAYS`] dias, com o de hoje em destaque,
//! da altura da seção dos limites. Os botões **Sem cache** (o padrão, verde)
//! e **Com cache** (vermelho) escolhem se o cache entra; clique ou `c`.
//!
//! O consumo é carregado ao abrir a aba e atualizado a cada [`STALE`]
//! enquanto ela está aberta; `r` atualiza na hora.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Block, Padding, Paragraph};

use super::chat::compact_count;
use crate::agent::{ChatAdapter, DailyTokens, Usage, UsageWindow};
use crate::theme::{self, Variant};

pub const HELP: &str =
    "clique num dia: horas · c com/sem cache · r atualizar · Tab trocar aba · Esc ir para Sair";
const HELP_DAY: &str = "Esc voltar aos dias · c com/sem cache · r atualizar · Tab trocar aba";
/// Depois disso, o consumo mostrado é recarregado.
const STALE: Duration = Duration::from_secs(60);
/// Largura máxima da barra de progresso.
const BAR_MAX: u16 = 40;
/// Largura da coluna dos nomes (a mesma da Config).
const LABEL_WIDTH: usize = 16;
const DATE_FORMAT: &str = "%d-%m-%Y %H:%M";
/// Quantos dias o gráfico mostra.
const DAYS: usize = 14;
/// A partir desta largura, o gráfico fica ao lado das barras.
const SIDE_BY_SIDE: u16 = 100;
/// Largura mínima da coluna das barras, com o gráfico ao lado.
const BARS_MIN_WIDTH: u16 = 66;
/// Altura mínima do gráfico.
const CHART_MIN_HEIGHT: u16 = 10;
/// Altura do gráfico embaixo das barras, em telas estreitas.
const CHART_BELOW_HEIGHT: u16 = 14;
/// Os botões do gráfico: o texto e se somam o cache.
const BUTTONS: [(&str, bool); 2] = [(" Sem cache ", false), (" Com cache ", true)];
const BUTTON_GAP: &str = "  ";
/// Botão que volta das horas de um dia para os dias.
const BACK_BUTTON: &str = " ‹ Dias ";

#[derive(Default)]
pub struct UsageView {
    usage: Option<Usage>,
    error: Option<String>,
    loading: Option<Receiver<Result<Usage, String>>>,
    /// Quando o consumo mostrado foi pedido, e de qual conta.
    fetched: Option<(Instant, Option<String>)>,
    /// Hora da última atualização, para a tela.
    updated_at: Option<Timestamp>,
    /// Tokens por dia, o mais antigo primeiro.
    daily: Option<Vec<DailyTokens>>,
    daily_loading: Option<Receiver<Result<Vec<DailyTokens>, String>>>,
    /// O gráfico soma o cache (botão Com cache); o padrão é sem.
    with_cache: bool,
    /// Onde os botões Sem cache e Com cache ficaram no último desenho.
    buttons: Cell<[Rect; 2]>,
    /// O dia aberto (clicado): o gráfico mostra as horas dele.
    selected_day: Option<Date>,
    /// Onde o botão `‹ Dias` ficou, com um dia aberto.
    back_button: Cell<Option<Rect>>,
    /// Onde cada barra de dia ficou (com o rótulo), para o clique.
    day_bars: RefCell<Vec<(Rect, Date)>>,
}

impl UsageView {
    /// `r` atualiza, `c` alterna o gráfico entre sem e com cache e `Esc`,
    /// com um dia aberto, volta aos dias. Retorna `true` se a tecla foi usada.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        adapter: Option<&dyn ChatAdapter>,
        account: Option<&str>,
    ) -> bool {
        match key.code {
            KeyCode::Char('r') => self.refresh(adapter, account),
            KeyCode::Char('c') => self.with_cache = !self.with_cache,
            KeyCode::Esc if self.selected_day.is_some() => self.selected_day = None,
            _ => return false,
        }
        true
    }

    /// Recarrega se nunca carregou, se passou [`STALE`], se a conta mudou ou
    /// se uma janela já zerou.
    pub fn refresh_if_stale(&mut self, adapter: Option<&dyn ChatAdapter>, account: Option<&str>) {
        if self.loading.is_some() || self.daily_loading.is_some() {
            return;
        }
        let stale = match &self.fetched {
            None => true,
            Some((at, loaded)) => {
                at.elapsed() >= STALE || loaded.as_deref() != account || self.window_reset()
            }
        };
        if stale {
            self.refresh(adapter, account);
        }
    }

    /// O consumo mostrado não vale mais (ex.: depois de um login): recarrega
    /// na próxima vez que a aba estiver aberta.
    pub fn invalidate(&mut self) {
        self.fetched = None;
        self.error = None;
    }

    fn refresh(&mut self, adapter: Option<&dyn ChatAdapter>, account: Option<&str>) {
        let changed = self
            .fetched
            .as_ref()
            .is_some_and(|(_, loaded)| loaded.as_deref() != account);
        if changed {
            // O consumo era da outra conta.
            self.usage = None;
            self.daily = None;
        }
        self.fetched = Some((Instant::now(), account.map(str::to_string)));
        self.error = None;
        self.loading = adapter
            .filter(|a| a.is_available())
            .and_then(|a| a.load_usage(account));
        if self.loading.is_none() {
            self.usage = None;
        }
        self.daily_loading = adapter
            .filter(|a| a.is_available())
            .and_then(|a| a.load_daily(account, DAYS));
        if self.daily_loading.is_none() {
            self.daily = None;
        }
    }

    /// Uma janela passou do horário de zerar: o que está na tela já não vale.
    fn window_reset(&self) -> bool {
        let now = Timestamp::now().as_second();
        self.usage
            .iter()
            .flat_map(|u| &u.windows)
            .any(|w| w.resets_at.is_some_and(|at| at <= now))
    }

    /// Recebe o consumo e os tokens por dia, se já chegaram.
    pub fn tick(&mut self) {
        if let Some(loading) = &self.daily_loading {
            match loading.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Ok(Ok(daily)) => {
                    self.daily = Some(daily);
                    self.daily_loading = None;
                }
                // Sem os dados por dia, o gráfico some; as barras continuam.
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.daily = None;
                    self.daily_loading = None;
                }
            }
        }
        let Some(loading) = &self.loading else {
            return;
        };
        let result = match loading.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("o carregamento do consumo parou".to_string())
            }
            Ok(result) => result,
        };
        self.loading = None;
        match result {
            Ok(usage) => {
                self.usage = Some(usage);
                self.updated_at = Some(Timestamp::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// `route` é o provedor e a conta (ex.: `Claude · Default · eu@gmail.com`).
    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        adapter: Option<&dyn ChatAdapter>,
        route: &str,
    ) {
        let tz = TimeZone::system();
        let now = Timestamp::now();
        let today = now.to_zoned(tz.clone()).date();
        let side_by_side = area.width >= SIDE_BY_SIDE;
        let (bars_area, chart_area) = if side_by_side {
            let [left, right] =
                Layout::horizontal([Constraint::Min(BARS_MIN_WIDTH), Constraint::Fill(1)])
                    .areas(area);
            (left, Some(right))
        } else {
            (area, None)
        };
        let lines = self.lines(adapter, route, bars_area.width, &tz, now.as_second());
        // O gráfico tem a altura da seção dos limites (o padding de cima e as
        // linhas), com um mínimo para as barras caberem.
        let height = (lines.len() as u16 + 1).max(CHART_MIN_HEIGHT);
        let chart_area = match chart_area {
            Some(right) => Some(Rect {
                height: height.min(right.height),
                ..right
            }),
            // Embaixo, com a mesma altura, se couber.
            None => {
                let used = lines.len() as u16 + 2;
                let rest = area.height.saturating_sub(used);
                (rest >= CHART_MIN_HEIGHT).then(|| Rect {
                    y: area.y + used,
                    height: rest.min(CHART_BELOW_HEIGHT),
                    ..area
                })
            }
        };
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            bars_area,
        );
        if let Some(chart_area) = chart_area {
            let pad = if side_by_side { 1 } else { 0 };
            let chart_area = Block::new()
                .padding(Padding::new(1, 2, pad, 0))
                .inner(chart_area);
            self.draw_daily(frame, chart_area, today);
        }
    }

    /// O gráfico: **Consumo por dia** (os últimos dias) ou, com um dia
    /// escolhido, o consumo dele **por hora**. Título, botões, maior valor,
    /// as barras com o rótulo embaixo e o resumo.
    fn draw_daily(&self, frame: &mut Frame, area: Rect, today: Date) {
        self.day_bars.borrow_mut().clear();
        self.back_button.set(None);
        let Some(daily) = &self.daily else {
            let status = if self.daily_loading.is_some() {
                Span::styled("Somando os tokens por dia...", theme::activity())
            } else {
                Span::styled(
                    "Este provedor não informa o consumo por dia.",
                    theme::muted(),
                )
            };
            let title = Line::styled(" Consumo por dia", theme::section_title());
            frame.render_widget(Paragraph::new(vec![title, indent(status)]), area);
            return;
        };
        let [title_area, buttons, scale, chart, summary] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let day = self
            .selected_day
            .and_then(|date| daily.iter().find(|d| d.date == date));
        self.draw_buttons(frame, buttons, day.is_some());
        let chart = Rect {
            x: chart.x + 2,
            width: chart.width.saturating_sub(2),
            ..chart
        };

        let (title, bars, scale_text, summary_text) = match day {
            None => {
                // Cada dia precisa de 2 colunas para o rótulo; sem espaço, menos dias.
                let fit = (chart.width as usize + 1) / 3;
                let shown = &daily[daily.len().saturating_sub(fit)..];
                let bars: Vec<ChartBar> = shown
                    .iter()
                    .map(|d| ChartBar {
                        label: format!("{:02}", d.date.day()),
                        value: self.value(d.tokens, d.cache),
                        highlight: d.date == today,
                    })
                    .collect();
                let total: u64 = bars.iter().map(|b| b.value).sum();
                let today_value = shown
                    .iter()
                    .find(|d| d.date == today)
                    .map_or(0, |d| self.value(d.tokens, d.cache));
                let average = total / bars.len().max(1) as u64;
                (
                    " Consumo por dia".to_string(),
                    bars,
                    format!("últimos {} dias · clique num dia", shown.len()),
                    format!(
                        "Hoje {} · média {}/dia · total {}",
                        compact_count(today_value),
                        compact_count(average),
                        compact_count(total)
                    ),
                )
            }
            Some(day) => {
                let bars: Vec<ChartBar> = day
                    .hours
                    .iter()
                    .enumerate()
                    .map(|(hour, h)| ChartBar {
                        label: format!("{hour:02}"),
                        value: self.value(h.tokens, h.cache),
                        highlight: false,
                    })
                    .collect();
                let peak = bars
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, b)| b.value)
                    .filter(|(_, b)| b.value > 0);
                let mut summary = format!(
                    "{} · total {}",
                    day.date.strftime("%d-%m-%Y"),
                    compact_count(self.value(day.tokens, day.cache))
                );
                if let Some((hour, bar)) = peak {
                    summary.push_str(&format!(
                        " · pico às {hour:02}h ({})",
                        compact_count(bar.value)
                    ));
                }
                (
                    format!(" Consumo de {} por hora", day.date.strftime("%d-%m")),
                    bars,
                    "24 horas".to_string(),
                    summary,
                )
            }
        };
        frame.render_widget(
            Paragraph::new(Line::styled(title, theme::section_title())),
            title_area,
        );
        let max = bars.iter().map(|b| b.value).max().unwrap_or(0);
        frame.render_widget(
            Paragraph::new(indent(Span::styled(
                format!("máx. {} · {scale_text}", compact_count(max)),
                theme::muted(),
            ))),
            scale,
        );
        let slots = draw_bars(frame, chart, &bars);
        if day.is_none() {
            let shown = &daily[daily.len() - bars.len()..];
            *self.day_bars.borrow_mut() = slots
                .into_iter()
                .zip(shown.iter().map(|d| d.date))
                .collect();
        }
        frame.render_widget(
            Paragraph::new(indent(Span::styled(summary_text, theme::text()))),
            summary,
        );
    }

    /// O valor de uma barra: com ou sem o cache, conforme o botão.
    fn value(&self, tokens: u64, cache: u64) -> u64 {
        if self.with_cache {
            tokens + cache
        } else {
            tokens
        }
    }

    /// `[ ‹ Dias ]  [ Sem cache ]  [ Com cache ]`: o `‹ Dias` só com um dia
    /// aberto; o cache escolhido fica preenchido. Guarda onde cada botão
    /// ficou, para o clique do mouse.
    fn draw_buttons(&self, frame: &mut Frame, area: Rect, day_open: bool) {
        let mut x = area.x + 2;
        let mut spans = vec![Span::raw("  ")];
        let mut place = |label: &str, style: Style, spans: &mut Vec<Span<'static>>| {
            let width = label.chars().count() as u16;
            let rect = Rect {
                x,
                width,
                height: 1,
                ..area
            }
            .intersection(area);
            spans.push(Span::styled(label.to_string(), style));
            spans.push(Span::raw(BUTTON_GAP));
            x += width + BUTTON_GAP.len() as u16;
            rect
        };
        if day_open {
            let rect = place(BACK_BUTTON, theme::show_more(), &mut spans);
            self.back_button.set(Some(rect));
        }
        let mut rects = [Rect::default(); 2];
        for (i, (label, with_cache)) in BUTTONS.iter().enumerate() {
            let selected = *with_cache == self.with_cache;
            let style = if *with_cache {
                theme::with_cache(selected)
            } else {
                theme::without_cache(selected)
            };
            rects[i] = place(label, style, &mut spans);
        }
        self.buttons.set(rects);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Clique: nos botões de cache, no `‹ Dias` ou numa barra de dia (abre
    /// as horas dele).
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        if let Some(i) = self.buttons.get().iter().position(|r| r.contains(at)) {
            self.with_cache = BUTTONS[i].1;
        } else if self.back_button.get().is_some_and(|r| r.contains(at)) {
            self.selected_day = None;
        } else if let Some((_, date)) = self
            .day_bars
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains(at))
        {
            self.selected_day = Some(*date);
        }
    }

    /// A ajuda do rodapé: com um dia aberto, `Esc` volta aos dias.
    pub fn help(&self) -> &'static str {
        if self.selected_day.is_some() {
            HELP_DAY
        } else {
            HELP
        }
    }

    fn lines(
        &self,
        adapter: Option<&dyn ChatAdapter>,
        route: &str,
        width: u16,
        tz: &TimeZone,
        now: i64,
    ) -> Vec<Line<'static>> {
        let route_value = match adapter {
            Some(a) if a.is_available() => {
                let plan = self.usage.as_ref().and_then(|u| u.plan.as_deref());
                let text = match plan {
                    Some(plan) => format!("{route} · {plan}"),
                    None => route.to_string(),
                };
                Span::styled(text, theme::variant(Variant::Success))
            }
            Some(a) => Span::styled(format!("{} · não encontrado", a.name()), theme::error()),
            None => Span::styled("Nenhum", theme::muted()),
        };
        let mut lines = vec![
            Line::styled(" Consumo", theme::section_title()),
            info_row("Route", route_value),
            Line::default(),
        ];

        if let Some(error) = &self.error {
            lines.push(indent(Span::styled(error.clone(), theme::error())));
        }
        match &self.usage {
            Some(usage) if usage.windows.is_empty() => lines.push(indent(Span::styled(
                "O provedor não informou limites para esta conta.",
                theme::muted(),
            ))),
            Some(usage) => {
                // Padding da tela (1 + 2) e o recuo da linha.
                let bar = width.saturating_sub(5 + 6).min(BAR_MAX) as usize;
                for window in &usage.windows {
                    lines.extend(window_lines(window, bar, tz, now));
                    lines.push(Line::default());
                }
            }
            None if self.loading.is_some() => lines.push(indent(Span::styled(
                "Carregando o consumo...",
                theme::activity(),
            ))),
            None if self.error.is_none() => lines.push(indent(Span::styled(
                "Este provedor não informa consumo.",
                theme::muted(),
            ))),
            None => {}
        }
        if let Some(at) = self.updated_at.filter(|_| self.usage.is_some()) {
            let mut text = format!(
                "Atualizado às {}",
                at.to_zoned(tz.clone()).strftime("%H:%M")
            );
            if self.loading.is_some() {
                text.push_str(" · atualizando...");
            }
            lines.push(indent(Span::styled(text, theme::muted())));
        }
        lines
    }
}

/// Uma barra do gráfico (um dia ou uma hora).
pub(super) struct ChartBar {
    /// Embaixo da barra, com 2 colunas (ex.: `27`, `14`).
    pub label: String,
    pub value: u64,
    /// O dia de hoje, em destaque.
    pub highlight: bool,
}

/// Desenha as barras em `area` com os rótulos na última linha; sem espaço
/// para todos, só alguns (a cada 3 horas, por exemplo). Devolve onde cada
/// barra ficou, do topo ao rótulo.
pub(super) fn draw_bars(frame: &mut Frame, area: Rect, bars: &[ChartBar]) -> Vec<Rect> {
    if bars.is_empty() || area.height < 2 {
        return Vec::new();
    }
    // Cada barra ocupa `slot` colunas: a barra e um espaço (sem espaço se
    // só couber 1 coluna).
    let slot = ((area.width as usize + 1) / bars.len()).clamp(1, 7) as u16;
    let (bar_width, gap) = if slot >= 2 { (slot - 1, 1) } else { (1, 0) };
    let [chart, labels] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let max = bars.iter().map(|b| b.value).max().unwrap_or(0);
    let data: Vec<Bar> = bars
        .iter()
        .map(|bar| {
            let style = if bar.highlight {
                theme::variant(Variant::Active)
            } else {
                theme::variant(Variant::Accent)
            };
            Bar::default()
                .value(bar.value)
                .text_value(String::new())
                .style(style)
        })
        .collect();
    frame.render_widget(
        BarChart::default()
            .data(BarGroup::default().bars(&data))
            .bar_width(bar_width)
            .bar_gap(gap)
            .max(max.max(1)),
        chart,
    );
    // Rótulos de 2 colunas e 1 de folga: a cada `step` barras.
    let step = 3usize.div_ceil(slot as usize).max(1);
    let mut line = String::new();
    for (i, bar) in bars.iter().enumerate().step_by(step) {
        let x = i * slot as usize;
        if x + bar.label.len() > labels.width as usize {
            break;
        }
        line.push_str(&" ".repeat(x - line.chars().count()));
        line.push_str(&bar.label);
    }
    frame.render_widget(Paragraph::new(Span::styled(line, theme::muted())), labels);
    (0..bars.len())
        .map(|i| {
            Rect {
                x: area.x + i as u16 * slot,
                width: bar_width,
                ..area
            }
            .intersection(area)
        })
        .collect()
}

/// O consumo da sessão (as 5 horas) para a barra de consumo do chat. Carrega
/// só o consumo (sem os tokens por dia da aba), enquanto a barra está
/// visível, e recarrega como a aba: a cada [`STALE`], ao trocar de conta ou
/// depois de a janela zerar.
#[derive(Default)]
pub struct SessionUsage {
    window: Option<UsageWindow>,
    loading: Option<Receiver<Result<Usage, String>>>,
    /// Quando o consumo foi pedido, e de qual conta.
    fetched: Option<(Instant, Option<String>)>,
    /// A janela mudou desde o último [`SessionUsage::tick`].
    changed: bool,
}

impl SessionUsage {
    pub fn refresh_if_stale(&mut self, adapter: Option<&dyn ChatAdapter>, account: Option<&str>) {
        if self.loading.is_some() {
            return;
        }
        let now = Timestamp::now().as_second();
        let reset = self
            .window
            .as_ref()
            .and_then(|w| w.resets_at)
            .is_some_and(|at| at <= now);
        let stale = match &self.fetched {
            None => true,
            Some((at, loaded)) => at.elapsed() >= STALE || loaded.as_deref() != account || reset,
        };
        if !stale {
            return;
        }
        if self
            .fetched
            .as_ref()
            .is_some_and(|(_, loaded)| loaded.as_deref() != account)
        {
            // Era da outra conta.
            self.set_window(None);
        }
        self.fetched = Some((Instant::now(), account.map(str::to_string)));
        self.loading = adapter
            .filter(|a| a.is_available())
            .and_then(|a| a.load_usage(account));
        if self.loading.is_none() {
            self.set_window(None);
        }
    }

    fn set_window(&mut self, window: Option<UsageWindow>) {
        self.changed |= window != self.window;
        self.window = window;
    }

    /// O consumo não vale mais (ex.: depois de um login): recarrega na
    /// próxima vez.
    pub fn invalidate(&mut self) {
        self.fetched = None;
    }

    /// Recebe o consumo, se já chegou. Retorna `true` quando a janela mudou
    /// desde a última chamada. Com erro, fica a última janela carregada.
    pub fn tick(&mut self) -> bool {
        if let Some(loading) = &self.loading {
            match loading.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Ok(Ok(usage)) => {
                    self.loading = None;
                    self.set_window(usage.session().cloned());
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                }
            }
        }
        std::mem::take(&mut self.changed)
    }

    pub fn window(&self) -> Option<&UsageWindow> {
        self.window.as_ref()
    }
}

/// O texto da barra de consumo do chat: quanto resta da sessão e quando ela
/// zera (`58% · reseta 15:00`).
pub fn session_label(window: &UsageWindow, tz: &TimeZone) -> String {
    let left = (100.0 - window.percent).clamp(0.0, 100.0);
    match window
        .resets_at
        .and_then(|at| Timestamp::from_second(at).ok())
    {
        Some(at) => format!(
            "{left:.0}% · reseta {}",
            at.to_zoned(tz.clone()).strftime("%H:%M")
        ),
        None => format!("{left:.0}% restantes"),
    }
}

/// Nome, barra com a porcentagem que resta e quando zera.
fn window_lines(window: &UsageWindow, bar: usize, tz: &TimeZone, now: i64) -> Vec<Line<'static>> {
    let style = level_style(window.percent);
    let left = (100.0 - window.percent).clamp(0.0, 100.0);
    let filled = ((left / 100.0) * bar as f64).round() as usize;
    let filled = filled.min(bar);
    let reset = match (&window.detail, window.resets_at) {
        (Some(detail), _) => detail.clone(),
        (None, Some(at)) => format!(
            "Reseta {} · zera em {}",
            format_date(at, tz),
            format_remaining(at - now)
        ),
        (None, None) => "Sem horário para zerar".to_string(),
    };
    vec![
        indent(Span::styled(window.label.clone(), theme::text())),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("█".repeat(filled), style),
            Span::styled("░".repeat(bar - filled), theme::muted()),
            Span::styled(format!("  {left:.0}% restantes"), style),
        ]),
        indent(Span::styled(reset, theme::path())),
    ]
}

/// Pelo quanto foi usado: verde até 70% (restando mais de 30%), amarelo até
/// 90% e vermelho daí em diante (restando 10% ou menos).
fn level_style(percent: f64) -> Style {
    let variant = if percent >= 90.0 {
        Variant::Danger
    } else if percent >= 70.0 {
        Variant::Warning
    } else {
        Variant::Success
    };
    theme::variant(variant)
}

/// `DD-MM-YYYY HH:MM` no fuso `tz`.
fn format_date(seconds: i64, tz: &TimeZone) -> String {
    Timestamp::from_second(seconds)
        .map(|t| t.to_zoned(tz.clone()).strftime(DATE_FORMAT).to_string())
        .unwrap_or_default()
}

/// Quanto falta: `2d 5h`, `2h 13m`, `13m` ou `menos de 1m`.
fn format_remaining(seconds: i64) -> String {
    let minutes = seconds / 60;
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        _ if seconds < 60 => "menos de 1m".to_string(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

fn indent(span: Span<'static>) -> Line<'static> {
    Line::from(vec![Span::raw("  "), span])
}

fn info_row(label: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{label:<LABEL_WIDTH$}"), theme::muted()),
        value,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn remaining_time_is_readable() {
        assert_eq!(format_remaining(-5), "menos de 1m");
        assert_eq!(format_remaining(59), "menos de 1m");
        assert_eq!(format_remaining(13 * 60 + 30), "13m");
        assert_eq!(format_remaining(2 * 3600 + 13 * 60), "2h 13m");
        assert_eq!(format_remaining(2 * 86400 + 5 * 3600 + 59), "2d 5h");
    }

    #[test]
    fn date_uses_day_month_year() {
        let tz = TimeZone::fixed(jiff::tz::offset(-3));
        // 2026-10-27 18:00 UTC.
        assert_eq!(format_date(1_793_124_000, &tz), "27-10-2026 15:00");
    }

    #[test]
    fn window_shows_bar_percent_and_reset() {
        let tz = TimeZone::UTC;
        let window = UsageWindow {
            label: "Sessão (5 h)".into(),
            percent: 25.0,
            resets_at: Some(1_793_124_000),
            detail: None,
            session: true,
        };
        let lines = window_lines(&window, 20, &tz, 1_793_124_000 - (2 * 3600 + 13 * 60));
        assert_eq!(text(&lines[0]), "  Sessão (5 h)");
        assert_eq!(
            text(&lines[1]),
            format!("  {}{}  75% restantes", "█".repeat(15), "░".repeat(5))
        );
        assert_eq!(
            text(&lines[2]),
            "  Reseta 27-10-2026 18:00 · zera em 2h 13m"
        );
        // Na barra do chat: o que resta e a hora de zerar.
        assert_eq!(session_label(&window, &tz), "75% · reseta 18:00");
        let window = UsageWindow {
            resets_at: None,
            ..window
        };
        assert_eq!(session_label(&window, &tz), "75% restantes");
    }

    #[test]
    fn bar_color_follows_the_level() {
        assert_eq!(level_style(10.0), theme::variant(Variant::Success));
        assert_eq!(level_style(75.0), theme::variant(Variant::Warning));
        assert_eq!(level_style(95.0), theme::variant(Variant::Danger));
    }

    /// 25/10: 1M (+1M de cache); 26/10: nada; 27/10 (hoje): 3M (+5M).
    /// Por hora: 25/10 tudo às 10h; 27/10, 1M às 09h e 2M (+5M) às 14h.
    fn chart_view() -> UsageView {
        let day = |d: i8, hours: &[(usize, u64, u64)]| {
            let mut day = DailyTokens::empty(jiff::civil::date(2026, 10, d));
            for &(hour, tokens, cache) in hours {
                day.hours[hour].tokens = tokens;
                day.hours[hour].cache = cache;
                day.tokens += tokens;
                day.cache += cache;
            }
            day
        };
        UsageView {
            daily: Some(vec![
                day(25, &[(10, 1_000_000, 1_000_000)]),
                day(26, &[]),
                day(27, &[(9, 1_000_000, 0), (14, 2_000_000, 5_000_000)]),
            ]),
            ..UsageView::default()
        }
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    /// Desenha o gráfico num terminal 50x12 e devolve as linhas.
    fn draw_chart(view: &UsageView) -> (Vec<String>, ratatui::buffer::Buffer) {
        use ratatui::{Terminal, backend::TestBackend};

        let today = jiff::civil::date(2026, 10, 27);
        let mut terminal = Terminal::new(TestBackend::new(50, 12)).unwrap();
        terminal
            .draw(|frame| view.draw_daily(frame, frame.area(), today))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..12)
            .map(|y| {
                (0..50)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        (rows, buffer)
    }

    #[test]
    fn daily_chart_shows_buttons_days_today_and_summary() {
        let (rows, buffer) = draw_chart(&chart_view());
        assert_eq!(rows[0], " Consumo por dia");
        assert_eq!(rows[1], "   Sem cache    Com cache");
        assert_eq!(rows[2], "  máx. 3M · últimos 3 dias · clique num dia");
        assert!(rows[10].contains("25") && rows[10].contains("26") && rows[10].contains("27"));
        assert_eq!(rows[11], "  Hoje 3M · média 1.3M/dia · total 4M");
        // Sem cache é o padrão: verde preenchido; Com cache, só o texto vermelho.
        assert_eq!(buffer[(3, 1)].bg, theme::without_cache(true).bg.unwrap());
        assert_eq!(buffer[(16, 1)].fg, theme::with_cache(false).fg.unwrap());
        // A barra de hoje (a maior) chega ao topo do gráfico, em destaque.
        let top = (0..50).find(|&x| buffer[(x, 3)].symbol() == "█").unwrap();
        assert_eq!(
            buffer[(top, 3)].fg,
            theme::variant(Variant::Active).fg.unwrap()
        );
    }

    #[test]
    fn clicking_com_cache_sums_the_cache() {
        let mut view = chart_view();
        draw_chart(&view);
        let click = |column| click(column, 1);
        view.handle_mouse(click(17));
        assert!(view.with_cache);
        let (rows, buffer) = draw_chart(&view);
        assert_eq!(rows[2], "  máx. 8M · últimos 3 dias · clique num dia");
        assert_eq!(rows[11], "  Hoje 8M · média 3.3M/dia · total 10M");
        assert_eq!(buffer[(16, 1)].bg, theme::with_cache(true).bg.unwrap());

        // Clicar fora dos botões não muda nada; Sem cache volta.
        view.handle_mouse(click(30));
        assert!(view.with_cache);
        view.handle_mouse(click(4));
        assert!(!view.with_cache);
    }

    #[test]
    fn clicking_a_day_shows_its_hours_and_esc_goes_back() {
        let mut view = chart_view();
        draw_chart(&view);
        // As barras ocupam 7 colunas cada, a partir da coluna 2: o 27 é a 3ª.
        view.handle_mouse(click(17, 5));
        assert_eq!(view.selected_day, Some(jiff::civil::date(2026, 10, 27)));
        assert_eq!(view.help(), HELP_DAY);

        let (rows, _) = draw_chart(&view);
        assert_eq!(rows[0], " Consumo de 27-10 por hora");
        assert_eq!(rows[1], "   ‹ Dias    Sem cache    Com cache");
        assert_eq!(rows[2], "  máx. 2M · 24 horas");
        // 24 barras de 1 coluna e 1 de espaço: o rótulo a cada 2 horas.
        assert_eq!(rows[10], "  00  02  04  06  08  10  12  14  16  18  20  22");
        assert_eq!(rows[11], "  27-10-2026 · total 3M · pico às 14h (2M)");
        // A barra das 14h é a maior; a das 09h, metade.
        assert_eq!(rows[3].chars().nth(2 + 14 * 2), Some('█'));

        // Com cache, o pico soma o cache da hora.
        let key = |code| KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
        view.handle_key(key(KeyCode::Char('c')), None, None);
        let (rows, _) = draw_chart(&view);
        assert_eq!(rows[11], "  27-10-2026 · total 8M · pico às 14h (7M)");

        // Esc volta aos dias.
        assert!(view.handle_key(key(KeyCode::Esc), None, None));
        assert_eq!(view.selected_day, None);
        // Sem dia aberto, o Esc fica para a tela principal (ir para Sair).
        assert!(!view.handle_key(key(KeyCode::Esc), None, None));
    }

    #[test]
    fn back_button_returns_to_days() {
        let mut view = chart_view();
        draw_chart(&view);
        view.handle_mouse(click(3, 5)); // o dia 25
        assert_eq!(view.selected_day, Some(jiff::civil::date(2026, 10, 25)));
        let (rows, _) = draw_chart(&view);
        assert_eq!(rows[11], "  25-10-2026 · total 1M · pico às 10h (1M)");
        view.handle_mouse(click(4, 1)); // ‹ Dias
        assert_eq!(view.selected_day, None);
        let (rows, _) = draw_chart(&view);
        assert_eq!(rows[0], " Consumo por dia");
    }

    #[test]
    fn c_toggles_the_cache() {
        let mut view = chart_view();
        let key = KeyEvent::new(KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE);
        assert!(view.handle_key(key, None, None));
        assert!(view.with_cache);
        assert!(view.handle_key(key, None, None));
        assert!(!view.with_cache);
    }

    #[test]
    fn without_adapter_shows_no_route() {
        let view = UsageView::default();
        let lines = view.lines(None, "", 80, &TimeZone::UTC, 0);
        assert_eq!(text(&lines[1]).trim_end(), "  Route           Nenhum");
        assert_eq!(text(&lines[3]), "  Este provedor não informa consumo.");
    }
}
