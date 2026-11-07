//! Desenho da tela principal: barra de abas, barra de chats, conteúdo da aba e
//! as caixas de confirmação.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{App, Focus, MAIN_CHAT_TITLE, SaveChoice, Tab};
use crate::hub::client::{Monitor, Reporter};
use crate::theme;
use crate::tui::extensions;
use crate::tui::orchestrator as orchestrator_tab;
use crate::tui::update;
use crate::tui::usage::TabSpend;

/// O retângulo de um item de `width` colunas que começa em `x` na linha de
/// `area`, cortado pela borda direita dela.
fn clip(x: u16, width: usize, area: Rect) -> Rect {
    let width = (width as u16).min(area.right().saturating_sub(x));
    Rect::new(x, area.y, width, area.height.min(1))
}

const QUIT_LABEL: &str = " Sair ";
const TAB_GAP: &str = " ";
const HELP: &str = "Tab ou clique trocar aba · Esc ir para Sair";
const HELP_QUIT: &str = "Enter sair · Tab/Esc voltar às abas";
const HELP_PROVIDER: &str = "Enter entrar no terminal · Tab trocar aba · Esc ir para Sair";
const HELP_CHATS: &str = "Alt+N novo chat · Alt+←/→ trocar chat · Alt+W fechar chat";
const HELP_NEW_CHAT: &str = "Alt+N novo chat";
const HELP_TERM: &str = "Ctrl+Q voltar às abas";
const HELP_CONFIRM: &str = "←/→ escolher · Enter confirmar · Esc cancelar";
const CONFIRM_TITLE: &str = " Sair ";
pub(super) const CONFIRM_TEXT: &str = "Deseja sair do terraforme?";
const YES_LABEL: &str = "  Sim  ";
const NO_LABEL: &str = "  Não  ";
const SAVE_TITLE: &str = " Salvar workspace ";
pub(super) const SAVE_TEXT: &str = "Este workspace não está salvo. Deseja salvar?";
/// As opções da pergunta de salvar, na ordem de [`SaveChoice`].
const SAVE_LABELS: [&str; 3] = ["  Sim  ", "  Não  ", "  Não (não perguntar novamente)  "];

impl App {
    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(theme::screen()), area);
        let [nav, content, footer] = layout(area);

        self.draw_nav(frame, nav);
        self.draw_content(frame, content);
        let help: Cow<'static, str> = match self.focus {
            Focus::Tabs if self.captures_keys() => HELP_TERM.into(),
            Focus::Tabs if self.tab == Tab::Chat && self.uses_provider() => HELP_PROVIDER.into(),
            Focus::Tabs if self.tab == Tab::Chat && !self.chats.is_empty() => {
                format!("{} · {HELP_CHATS}", self.current_chat().help()).into()
            }
            Focus::Tabs if self.tab == Tab::Chat => {
                format!("{} · {HELP_NEW_CHAT}", self.current_chat().help()).into()
            }
            Focus::Tabs if self.tab == Tab::Config => self.config_view.help().into(),
            Focus::Tabs if self.tab == Tab::Extensoes => extensions::HELP.into(),
            Focus::Tabs if self.tab == Tab::Orquestrador => self.orchestrator_view.help().into(),
            Focus::Tabs if self.tab == Tab::Consumo => self.usage.help().into(),
            Focus::Tabs if self.tab == Tab::Atualizacao => update::HELP.into(),
            Focus::Tabs => HELP.into(),
            Focus::Quit => HELP_QUIT.into(),
            Focus::Confirm(_) | Focus::SaveAsk(_) => HELP_CONFIRM.into(),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {help}"), theme::muted())),
            footer,
        );
        match self.focus {
            Focus::Confirm(yes) => draw_confirm(frame, area, yes),
            Focus::SaveAsk(choice) => draw_save_ask(frame, area, choice),
            _ => {}
        }
    }

    fn draw_nav(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::border())
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let total = self.total_label();
        let [tabs_area, total_area, quit_area] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(total.as_ref().map_or(0, |t| t.chars().count() as u16)),
            Constraint::Length(QUIT_LABEL.len() as u16),
        ])
        .areas(inner);
        if let Some(total) = total {
            frame.render_widget(
                Paragraph::new(Span::styled(total, theme::muted())),
                total_area,
            );
        }

        let mut spans = Vec::new();
        let mut rects = Vec::new();
        let mut x = tabs_area.x;
        let tabs = Tab::ALL
            .into_iter()
            .filter(|tab| tab.visible(self.update_view.available()));
        for (i, tab) in tabs.enumerate() {
            if i > 0 {
                spans.push(Span::raw(TAB_GAP));
                x = x.saturating_add(TAB_GAP.width() as u16);
            }
            let style = if tab == self.tab && self.focus == Focus::Tabs {
                theme::tab_active()
            } else {
                theme::tab_inactive()
            };
            let title = match self.unread() {
                n if n > 0 && tab == Tab::Chat => format!(" {} ✉{n} ", tab.title()),
                _ => format!(" {} ", tab.title()),
            };
            rects.push((clip(x, title.width(), tabs_area), tab));
            x = x.saturating_add(title.width() as u16);
            spans.push(Span::styled(title, style));
        }
        *self.tab_rects.borrow_mut() = rects;
        frame.render_widget(Paragraph::new(Line::from(spans)), tabs_area);

        let quit_style = if self.focus == Focus::Tabs {
            theme::quit()
        } else {
            theme::quit_focused()
        };

        frame.render_widget(
            Paragraph::new(Span::styled(QUIT_LABEL, quit_style)).alignment(Alignment::Right),
            quit_area,
        );
        // O rótulo fica à direita da área, que pode ser mais larga que ele.
        let width = (QUIT_LABEL.width() as u16).min(quit_area.width);
        self.quit_rect.set(Rect::new(
            quit_area.right() - width,
            quit_area.y,
            width,
            quit_area.height.min(1),
        ));
    }

    /// O consumo geral, ao lado do "Sair": os tokens de todos os chats deste
    /// terra (entrada e saída, sem os de cache, como no consumo do chat).
    fn total_label(&self) -> Option<String> {
        let tokens = self.closed_tokens
            + self.chat.tokens_used()
            + self.chats.iter().map(|c| c.chat.tokens_used()).sum::<u64>();
        (self.settings.usage.total_visible && tokens > 0).then(|| {
            format!(
                "Consumo geral: {} Tokens  ",
                crate::tui::chat::compact_count(tokens)
            )
        })
    }

    /// A barra de chats, embaixo da barra de abas e no mesmo tema: o chat
    /// do terra e os abertos com `Alt+N`.
    fn draw_chats(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::border())
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let titles = std::iter::once((MAIN_CHAT_TITLE, &self.chat))
            .chain(self.chats.iter().map(|c| (c.title.as_str(), &c.chat)))
            .map(|(title, chat)| match chat.unread() {
                0 => title.to_string(),
                n => format!("{title} ✉{n}"),
            });
        let mut spans = Vec::new();
        let mut rects = Vec::new();
        let mut x = inner.x;
        for (i, title) in titles.enumerate() {
            if i > 0 {
                spans.push(Span::raw(TAB_GAP));
                x = x.saturating_add(TAB_GAP.width() as u16);
            }
            let style = if i == self.active_chat && self.focus == Focus::Tabs {
                theme::tab_active()
            } else {
                theme::tab_inactive()
            };
            let title = format!(" {title} ");
            rects.push((clip(x, title.width(), inner), i));
            x = x.saturating_add(title.width() as u16);
            spans.push(Span::styled(title, style));
        }
        *self.chat_rects.borrow_mut() = rects;
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
    }

    fn draw_content(&self, frame: &mut Frame, area: Rect) {
        // A barra de chats só reaparece se for desenhada abaixo.
        self.chat_rects.borrow_mut().clear();
        if self.tab == Tab::Chat && self.uses_provider() {
            match &self.term {
                Some(term) => term.draw(frame, area, self.captures_keys()),
                None => self.draw_provider(frame, area),
            }
            return;
        }
        if self.tab == Tab::Chat {
            let bar = if self.chats.is_empty() { 0 } else { 3 };
            let chat = self.current_chat();
            let [chats, history, panel] = Layout::vertical([
                Constraint::Length(bar),
                Constraint::Fill(1),
                Constraint::Length(chat.height()),
            ])
            .areas(area);
            if bar > 0 {
                self.draw_chats(frame, chats);
            }
            chat.draw_history(frame, history);
            chat.draw(frame, panel);
            return;
        }
        if self.tab == Tab::Config {
            self.config_view.draw(
                frame,
                area,
                &self.settings,
                &self.extensions,
                &self.choices(),
            );
            return;
        }
        if self.tab == Tab::Orquestrador {
            let instances = self.hub_monitor.as_ref().map(Monitor::instances);
            let hub = orchestrator_tab::HubView {
                instances: instances.as_ref(),
                own: self.hub.as_ref().map(Reporter::id),
            };
            self.orchestrator_view.draw(frame, area, &hub);
            return;
        }
        if self.tab == Tab::Atualizacao {
            self.update_view.draw(frame, area);
            return;
        }
        if self.tab == Tab::Extensoes {
            self.extensions_view
                .draw(frame, area, &self.registry, &self.extensions);
            return;
        }
        let route = match self.chat.adapter() {
            Some(adapter) => format!("{} · {}", adapter.name(), self.chat.account_label()),
            None => String::new(),
        };
        self.usage.draw(
            frame,
            area,
            self.chat.adapter(),
            &route,
            &self.tab_spends(),
            self.active_chat,
        );
    }

    /// O que cada chat aberto gastou, para a sessão da aba Consumo.
    fn tab_spends(&self) -> Vec<TabSpend> {
        let main = TabSpend {
            title: MAIN_CHAT_TITLE.to_string(),
            spend: self.chat.spend(),
            running: self.chat.turn_spend().to_vec(),
            note: self.uses_provider().then_some(
                "A interface original do provedor não passa pelo terra: o consumo dela não conta aqui.",
            ),
        };
        let others = self.chats.iter().map(|tab| TabSpend {
            title: tab.title.clone(),
            spend: tab.chat.spend(),
            running: if tab.run.is_some() {
                Vec::new()
            } else {
                tab.chat.turn_spend().to_vec()
            },
            note: if tab.subagent.is_some() {
                Some("O consumo do subagente entra na aba que o lançou.")
            } else if tab.run.is_some() {
                Some("Uma execução despachada não entra na sessão da pasta.")
            } else {
                None
            },
        });
        std::iter::once(main).chain(others).collect()
    }

    /// A aba Chat na Interface "Original do provedor" sem a interface
    /// rodando: como abrir e como a última terminou.
    fn draw_provider(&self, frame: &mut Frame, area: Rect) {
        let name = self.chat.adapter().map_or("provedor", |a| a.name());
        let mut lines = vec![
            Line::styled(" Interface original", theme::section_title()),
            Line::from(vec![
                Span::styled(format!("  {name}"), theme::text()),
                Span::styled(format!(" · {}", self.chat.account_label()), theme::muted()),
            ]),
            Line::default(),
            Line::styled(
                format!("  Enter abre o chat do {name} neste workspace, aqui na aba Chat."),
                theme::muted(),
            ),
            Line::styled(
                "  Para usar o chat do terra, troque a Interface na aba Configurações.",
                theme::muted(),
            ),
        ];
        match &self.provider_result {
            Some(Ok(())) => {
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("  {name} encerrado."),
                    theme::variant(theme::Variant::Success),
                ));
            }
            Some(Err(message)) => {
                lines.push(Line::default());
                lines.push(Line::styled(format!("  {message}"), theme::error()));
            }
            None => {}
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
    }
}

/// Barra de abas, conteúdo da aba e rodapé.
fn layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// A área de conteúdo numa tela `screen`.
pub(super) fn content_area(screen: Rect) -> Rect {
    layout(screen)[1]
}

/// A pergunta de salvar o workspace, centralizada sobre a tela.
fn draw_save_ask(frame: &mut Frame, area: Rect, choice: SaveChoice) {
    let [area] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(62)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Span::styled(SAVE_TITLE, theme::section_title()))
        .style(theme::screen())
        .padding(Padding::vertical(1));
    let mut buttons = Vec::new();
    for (option, label) in SaveChoice::ALL.into_iter().zip(SAVE_LABELS) {
        if !buttons.is_empty() {
            buttons.push(Span::raw("   "));
        }
        let selected = option == choice;
        let style = match option {
            SaveChoice::Yes => theme::confirm_yes(selected),
            SaveChoice::No | SaveChoice::NeverAsk => theme::confirm_no(selected),
        };
        buttons.push(Span::styled(label, style));
    }
    let lines = vec![Line::from(SAVE_TEXT), Line::default(), Line::from(buttons)];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        area,
    );
}

/// Caixa de confirmação de saída, centralizada sobre a tela.
fn draw_confirm(frame: &mut Frame, area: Rect, yes: bool) {
    let [area] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(36)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::quit())
        .title(Span::styled(CONFIRM_TITLE, theme::quit()))
        .style(theme::screen())
        .padding(Padding::vertical(1));
    let lines = vec![
        Line::from(CONFIRM_TEXT),
        Line::default(),
        Line::from(vec![
            Span::styled(YES_LABEL, theme::confirm_yes(yes)),
            Span::raw("   "),
            Span::styled(NO_LABEL, theme::confirm_no(!yes)),
        ]),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        area,
    );
}
