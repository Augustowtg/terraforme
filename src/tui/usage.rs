//! Aba Consumo: quanto dos limites de uso da conta já foi gasto, pelo route
//! (provedor) do chat.
//!
//! ```text
//!  Consumo
//!   Route           Claude · Pessoal · eu@gmail.com · Max
//!
//!   Sessão (5 h)
//!   ████████████░░░░░░░░░░░░░░░░  42%
//!   Reseta 27-10-2026 15:00 · zera em 2h 13m
//! ```
//!
//! O consumo é carregado ao abrir a aba e atualizado a cada [`STALE`]
//! enquanto ela está aberta; `r` atualiza na hora.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use crate::agent::{ChatAdapter, Usage, UsageWindow};
use crate::theme::{self, Variant};

pub const HELP: &str = "r atualizar · Tab trocar aba · Esc ir para Sair";
/// Depois disso, o consumo mostrado é recarregado.
const STALE: Duration = Duration::from_secs(60);
/// Largura máxima da barra de progresso.
const BAR_MAX: u16 = 40;
/// Largura da coluna dos nomes (a mesma da Config).
const LABEL_WIDTH: usize = 16;
const DATE_FORMAT: &str = "%d-%m-%Y %H:%M";

#[derive(Default)]
pub struct UsageView {
    usage: Option<Usage>,
    error: Option<String>,
    loading: Option<Receiver<Result<Usage, String>>>,
    /// Quando o consumo mostrado foi pedido, e de qual conta.
    fetched: Option<(Instant, Option<String>)>,
    /// Hora da última atualização, para a tela.
    updated_at: Option<Timestamp>,
}

impl UsageView {
    /// `r` atualiza. Retorna `true` se a tecla foi usada.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        adapter: Option<&dyn ChatAdapter>,
        account: Option<&str>,
    ) -> bool {
        if key.code != KeyCode::Char('r') {
            return false;
        }
        self.refresh(adapter, account);
        true
    }

    /// Recarrega se nunca carregou, se passou [`STALE`], se a conta mudou ou
    /// se uma janela já zerou.
    pub fn refresh_if_stale(&mut self, adapter: Option<&dyn ChatAdapter>, account: Option<&str>) {
        if self.loading.is_some() {
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

    fn refresh(&mut self, adapter: Option<&dyn ChatAdapter>, account: Option<&str>) {
        let changed = self
            .fetched
            .as_ref()
            .is_some_and(|(_, loaded)| loaded.as_deref() != account);
        if changed {
            // O consumo era da outra conta.
            self.usage = None;
        }
        self.fetched = Some((Instant::now(), account.map(str::to_string)));
        self.error = None;
        self.loading = adapter
            .filter(|a| a.is_available())
            .and_then(|a| a.load_usage(account));
        if self.loading.is_none() {
            self.usage = None;
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

    /// Recebe o consumo, se já chegou.
    pub fn tick(&mut self) {
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
        let now = Timestamp::now().as_second();
        let lines = self.lines(adapter, route, area.width, &tz, now);
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
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

/// Nome, barra com a porcentagem e quando zera.
fn window_lines(window: &UsageWindow, bar: usize, tz: &TimeZone, now: i64) -> Vec<Line<'static>> {
    let style = level_style(window.percent);
    let filled = ((window.percent / 100.0) * bar as f64).round() as usize;
    let filled = filled.min(bar);
    let reset = match window.resets_at {
        Some(at) => format!(
            "Reseta {} · zera em {}",
            format_date(at, tz),
            format_remaining(at - now)
        ),
        None => "Sem horário para zerar".to_string(),
    };
    vec![
        indent(Span::styled(window.label.clone(), theme::text())),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("█".repeat(filled), style),
            Span::styled("░".repeat(bar - filled), theme::muted()),
            Span::styled(format!("  {:.0}%", window.percent), style),
        ]),
        indent(Span::styled(reset, theme::path())),
    ]
}

/// Verde até 70%, amarelo até 90%, vermelho daí em diante.
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
        };
        let lines = window_lines(&window, 20, &tz, 1_793_124_000 - (2 * 3600 + 13 * 60));
        assert_eq!(text(&lines[0]), "  Sessão (5 h)");
        assert_eq!(
            text(&lines[1]),
            format!("  {}{}  25%", "█".repeat(5), "░".repeat(15))
        );
        assert_eq!(
            text(&lines[2]),
            "  Reseta 27-10-2026 18:00 · zera em 2h 13m"
        );
    }

    #[test]
    fn bar_color_follows_the_level() {
        assert_eq!(level_style(10.0), theme::variant(Variant::Success));
        assert_eq!(level_style(75.0), theme::variant(Variant::Warning));
        assert_eq!(level_style(95.0), theme::variant(Variant::Danger));
    }

    #[test]
    fn without_adapter_shows_no_route() {
        let view = UsageView::default();
        let lines = view.lines(None, "", 80, &TimeZone::UTC, 0);
        assert_eq!(text(&lines[1]).trim_end(), "  Route           Nenhum");
        assert_eq!(text(&lines[3]), "  Este provedor não informa consumo.");
    }
}
