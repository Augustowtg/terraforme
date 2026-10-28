//! Aba Extensões: o que cada extensão do registro informa (ex.: os tokens
//! que o RTK economizou e a memória guardada pelo ai-memory).
//!
//! ```text
//!  Extensões                 RTK 0.45.0 · ativa neste workspace
//!  ▌ RTK        ativa          Enxuga a saída dos comandos do agente...
//!    ai-memory  inativa        https://github.com/rtk-ai/rtk
//!
//!                              Tokens economizados   1.5M
//!                              ...
//!                            Tokens economizados por dia
//!                              (gráfico dos últimos dias)
//! ```
//!
//! `↑`/`↓` escolhem a extensão; o relatório dela é carregado ao abrir a aba
//! e atualizado a cada [`STALE`] enquanto ela está aberta; `r` atualiza na
//! hora. Ativar e desativar fica na Config.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

use super::chat::compact_count;
use super::usage::{ChartBar, draw_bars};
use crate::extension::{Extension, Registry, Report, StatValue, Status};
use crate::settings::{ExtensionMode, ExtensionSettings};
use crate::theme::{self, Variant};

pub const HELP: &str =
    "↑/↓ escolher · r atualizar · ativar nas Configurações · Tab trocar aba · Esc ir para Sair";
/// Depois disso, o relatório mostrado é recarregado.
const STALE: Duration = Duration::from_secs(60);
/// Largura da lista das extensões.
const LIST_WIDTH: u16 = 30;
/// Largura da coluna dos nomes das linhas do relatório.
const LABEL_WIDTH: usize = 22;
/// Altura máxima do gráfico por dia (título, escala, barras e resumo).
const CHART_MAX_HEIGHT: u16 = 14;

/// O relatório de uma extensão.
#[derive(Default)]
struct Loaded {
    report: Option<Report>,
    error: Option<String>,
    loading: Option<Receiver<Result<Report, String>>>,
    /// Quando foi pedido.
    fetched: Option<Instant>,
}

#[derive(Default)]
pub struct ExtensionsView {
    /// Posição na lista do registro.
    selected: usize,
    /// Os relatórios, pelo id da extensão.
    reports: HashMap<&'static str, Loaded>,
}

impl ExtensionsView {
    /// `↑`/`↓` escolhem a extensão e `r` atualiza o relatório dela. Retorna
    /// `true` se a tecla foi usada.
    pub fn handle_key(&mut self, key: KeyEvent, registry: &Registry) -> bool {
        let last = registry.all().count().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('r') => {
                if let Some(extension) = self.extension(registry) {
                    self.refresh(extension);
                }
            }
            _ => return false,
        }
        true
    }

    fn extension<'a>(&self, registry: &'a Registry) -> Option<&'a dyn Extension> {
        registry.all().nth(self.selected)
    }

    /// Carrega o relatório da extensão escolhida se ele nunca foi carregado
    /// ou se passou [`STALE`].
    pub fn refresh_if_stale(&mut self, registry: &Registry) {
        let Some(extension) = self.extension(registry) else {
            return;
        };
        let loaded = self.reports.get(extension.id());
        let stale = loaded.is_none_or(|l| {
            l.loading.is_none() && l.fetched.is_none_or(|at| at.elapsed() >= STALE)
        });
        if stale {
            self.refresh(extension);
        }
    }

    fn refresh(&mut self, extension: &dyn Extension) {
        let loaded = self.reports.entry(extension.id()).or_default();
        loaded.fetched = Some(Instant::now());
        loaded.error = None;
        loaded.loading = extension.load_report();
    }

    /// Recebe os relatórios que já chegaram.
    pub fn tick(&mut self) {
        for loaded in self.reports.values_mut() {
            let Some(loading) = &loaded.loading else {
                continue;
            };
            let result = match loading.try_recv() {
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => {
                    Err("o carregamento do relatório parou".to_string())
                }
                Ok(result) => result,
            };
            loaded.loading = None;
            match result {
                Ok(report) => loaded.report = Some(report),
                Err(error) => loaded.error = Some(error),
            }
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        registry: &Registry,
        settings: &ExtensionSettings,
    ) {
        let [list, detail] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Fill(1)]).areas(area);
        let mut lines = vec![Line::styled(" Extensões", theme::section_title())];
        for (i, extension) in registry.all().enumerate() {
            let (state, style) = state(extension, settings);
            let name = format!("{:<12}", extension.name());
            lines.push(if i == self.selected {
                Line::from(vec![
                    Span::raw(format!("{} ", theme::FOCUS_MARKER)),
                    Span::raw(name),
                    Span::raw(state),
                ])
                .style(theme::focused())
            } else {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(name, theme::text()),
                    Span::styled(state, style),
                ])
            });
        }
        if registry.all().next().is_none() {
            lines.push(Line::styled(
                "  Nenhuma extensão disponível.",
                theme::muted(),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            list,
        );
        if let Some(extension) = self.extension(registry) {
            self.draw_detail(frame, detail, extension, settings);
        }
    }

    /// O nome, a versão, a descrição e o relatório da extensão escolhida,
    /// com o gráfico por dia embaixo quando ela tem.
    fn draw_detail(
        &self,
        frame: &mut Frame,
        area: Rect,
        extension: &dyn Extension,
        settings: &ExtensionSettings,
    ) {
        let area = Block::new().padding(Padding::new(0, 2, 1, 0)).inner(area);
        let status = extension.status();
        let title = match &status {
            Status::Installed { version: Some(v) } => format!(" {} {v}", extension.name()),
            _ => format!(" {}", extension.name()),
        };
        let (state, style) = state(extension, settings);
        let state = match settings.mode(extension.id()) {
            ExtensionMode::Account if status.is_installed() => "ativa nesta conta".to_string(),
            ExtensionMode::Always if status.is_installed() => {
                "usar sempre (em todas as contas)".to_string()
            }
            _ => state,
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled(title, theme::section_title()),
                Span::styled(" · ", theme::muted()),
                Span::styled(state, style),
            ]),
            indent(Span::styled(extension.description(), theme::muted())),
            indent(Span::styled(extension.homepage(), theme::path())),
            Line::default(),
        ];
        let loaded = self.reports.get(extension.id());
        let report = loaded.and_then(|l| l.report.as_ref());
        if !status.is_installed() {
            lines.push(indent(Span::styled(
                format!("Não instalada. Instale em {}", extension.homepage()),
                theme::error(),
            )));
        } else if let Some(error) = loaded.and_then(|l| l.error.as_ref()) {
            lines.push(indent(Span::styled(error.clone(), theme::error())));
        } else if report.is_none() && loaded.is_some_and(|l| l.loading.is_some()) {
            lines.push(indent(Span::styled("Carregando...", theme::muted())));
        } else if report.is_none() {
            lines.push(indent(Span::styled(
                "Esta extensão não tem relatório.",
                theme::muted(),
            )));
        }
        for stat in report.iter().flat_map(|r| &r.stats) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<LABEL_WIDTH$}", stat.label), theme::muted()),
                Span::styled(format_value(&stat.value), theme::text()),
            ]));
        }

        let height = lines.len() as u16;
        let [text, chart] =
            Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area);
        frame.render_widget(Paragraph::new(lines), text);
        if let Some(report) = report.filter(|r| !r.daily.is_empty()) {
            let chart = Rect {
                y: chart.y + 1,
                height: chart.height.saturating_sub(1).min(CHART_MAX_HEIGHT),
                ..chart
            };
            draw_daily(frame, chart, report, jiff::Zoned::now().date());
        }
    }
}

/// O estado da extensão na lista: não instalada, ativa ou inativa.
fn state(
    extension: &dyn Extension,
    settings: &ExtensionSettings,
) -> (String, ratatui::style::Style) {
    if !extension.status().is_installed() {
        ("não instalada".to_string(), theme::error())
    } else if settings.is_enabled(extension.id()) {
        ("ativa".to_string(), theme::variant(Variant::Success))
    } else {
        ("inativa".to_string(), theme::muted())
    }
}

/// O gráfico por dia do relatório: título, escala, as barras (hoje em
/// destaque) e o resumo.
fn draw_daily(frame: &mut Frame, area: Rect, report: &Report, today: jiff::civil::Date) {
    if area.height < 5 {
        return;
    }
    let [title, scale, bars_area, summary] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let bars_area = Rect {
        x: bars_area.x + 2,
        width: bars_area.width.saturating_sub(2),
        ..bars_area
    };
    // Cada dia precisa de 2 colunas para o rótulo; sem espaço, menos dias.
    let fit = (bars_area.width as usize + 1) / 3;
    let shown = &report.daily[report.daily.len().saturating_sub(fit)..];
    let bars: Vec<ChartBar> = shown
        .iter()
        .map(|(date, value)| ChartBar {
            label: format!("{:02}", date.day()),
            value: *value,
            highlight: *date == today,
        })
        .collect();
    let total: u64 = bars.iter().map(|b| b.value).sum();
    let max = bars.iter().map(|b| b.value).max().unwrap_or(0);
    let today_value = shown
        .iter()
        .find(|(date, _)| *date == today)
        .map_or(0, |(_, v)| *v);
    let average = total / bars.len().max(1) as u64;

    let name = report.daily_title.as_deref().unwrap_or("Por dia");
    frame.render_widget(
        Paragraph::new(Line::styled(format!(" {name}"), theme::section_title())),
        title,
    );
    frame.render_widget(
        Paragraph::new(indent(Span::styled(
            format!("máx. {} · últimos {} dias", compact_count(max), shown.len()),
            theme::muted(),
        ))),
        scale,
    );
    draw_bars(frame, bars_area, &bars);
    frame.render_widget(
        Paragraph::new(indent(Span::styled(
            format!(
                "Hoje {} · média {}/dia · total {}",
                compact_count(today_value),
                compact_count(average),
                compact_count(total)
            ),
            theme::text(),
        ))),
        summary,
    );
}

fn indent(span: Span<'static>) -> Line<'static> {
    Line::from(vec![Span::raw("  "), span])
}

/// Um valor do relatório para a tela.
fn format_value(value: &StatValue) -> String {
    match value {
        StatValue::Count(n) => n.to_string(),
        StatValue::Tokens(n) => compact_count(*n),
        StatValue::Percent(p) => format!("{p:.1}%"),
        StatValue::Bytes(n) => format_bytes(*n),
        StatValue::Text(text) => text.clone(),
    }
}

/// `24911872` → `23.8 MB`.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{Contribution, Stat};
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::mpsc;

    struct Fake {
        id: &'static str,
        installed: bool,
        report: Option<Result<Report, String>>,
    }

    impl Extension for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn description(&self) -> &'static str {
            "faz coisas"
        }
        fn homepage(&self) -> &'static str {
            "https://exemplo"
        }
        fn status(&self) -> Status {
            if self.installed {
                Status::Installed {
                    version: Some("1.2".into()),
                }
            } else {
                Status::Missing
            }
        }
        fn contribution(&self) -> Result<Contribution, String> {
            Ok(Contribution::default())
        }
        fn load_report(&self) -> Option<Receiver<Result<Report, String>>> {
            let report = self.report.clone()?;
            let (tx, rx) = mpsc::channel();
            tx.send(report).unwrap();
            Some(rx)
        }
    }

    fn registry() -> Registry {
        let today = jiff::Zoned::now().date();
        let report = Report {
            stats: vec![
                Stat::new("Tokens economizados", StatValue::Tokens(1_500_000)),
                Stat::new("Economia média", StatValue::Percent(42.25)),
            ],
            daily_title: Some("Tokens economizados por dia".into()),
            daily: vec![(today.yesterday().unwrap(), 1_000), (today, 3_000)],
        };
        Registry::new(vec![
            Box::new(Fake {
                id: "rtk",
                installed: true,
                report: Some(Ok(report)),
            }),
            Box::new(Fake {
                id: "memoria",
                installed: true,
                report: Some(Err("servidor fora".into())),
            }),
            Box::new(Fake {
                id: "faltando",
                installed: false,
                report: None,
            }),
        ])
    }

    fn rows(
        view: &ExtensionsView,
        registry: &Registry,
        settings: &ExtensionSettings,
    ) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), registry, settings))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..24)
            .map(|y| {
                (0..90)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn key(view: &mut ExtensionsView, registry: &Registry, code: KeyCode) -> bool {
        view.handle_key(KeyEvent::new(code, KeyModifiers::NONE), registry)
    }

    #[test]
    fn shows_the_list_and_the_report_with_the_daily_chart() {
        let registry = registry();
        let settings = ExtensionSettings {
            enabled: vec!["rtk".into()],
            ..ExtensionSettings::default()
        };
        let mut view = ExtensionsView::default();
        view.refresh_if_stale(&registry);
        let rows_before = rows(&view, &registry, &settings);
        assert!(rows_before[5].contains("Carregando..."), "{rows_before:?}");
        view.tick();

        let rows = rows(&view, &registry, &settings);
        assert_eq!(
            rows[1],
            "  Extensões                    rtk 1.2 · ativa nesta conta"
        );
        assert!(rows[2].starts_with(" ▌ rtk         ativa"), "{}", rows[2]);
        assert!(rows[3].starts_with("   memoria     inativa"), "{}", rows[3]);
        assert!(
            rows[4].starts_with("   faltando    não instalada"),
            "{}",
            rows[4]
        );
        assert!(rows[2].ends_with("faz coisas"));
        assert!(rows[3].ends_with("https://exemplo"));
        assert!(
            rows[5].ends_with("Tokens economizados   1.5M"),
            "{}",
            rows[5]
        );
        assert!(
            rows[6].ends_with("Economia média        42.2%"),
            "{}",
            rows[6]
        );
        assert!(
            rows[8].ends_with("Tokens economizados por dia"),
            "{}",
            rows[8]
        );
        assert!(rows[9].ends_with("máx. 3K · últimos 2 dias"), "{}", rows[9]);
        assert!(
            rows.iter()
                .any(|r| r.ends_with("Hoje 3K · média 2K/dia · total 4K")),
            "{rows:?}"
        );
    }

    #[test]
    fn arrows_choose_the_extension_and_errors_or_missing_are_shown() {
        let registry = registry();
        let settings = ExtensionSettings::default();
        let mut view = ExtensionsView::default();
        assert!(key(&mut view, &registry, KeyCode::Down));
        view.refresh_if_stale(&registry);
        view.tick();
        let rows1 = rows(&view, &registry, &settings);
        assert!(rows1[5].ends_with("servidor fora"), "{}", rows1[5]);

        assert!(key(&mut view, &registry, KeyCode::Down));
        assert!(key(&mut view, &registry, KeyCode::Down));
        assert_eq!(view.selected, 2, "a última fica");
        view.refresh_if_stale(&registry);
        let rows2 = rows(&view, &registry, &settings);
        assert!(
            rows2[5].ends_with("Não instalada. Instale em https://exemplo"),
            "{}",
            rows2[5]
        );
        assert!(key(&mut view, &registry, KeyCode::Char('r')));
        assert!(!key(&mut view, &registry, KeyCode::Tab));
    }

    #[test]
    fn formats_values() {
        assert_eq!(format_value(&StatValue::Count(8047)), "8047");
        assert_eq!(format_value(&StatValue::Tokens(1_476_310)), "1.5M");
        assert_eq!(format_value(&StatValue::Percent(42.236)), "42.2%");
        assert_eq!(format_value(&StatValue::Bytes(24_911_872)), "23.8 MB");
        assert_eq!(format_value(&StatValue::Bytes(512)), "512 B");
        assert_eq!(format_value(&StatValue::Text("ok".into())), "ok");
    }
}
