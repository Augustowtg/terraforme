//! Desenho do chat: histórico, painel de entrada, badges e barras de contexto e
//! de consumo.

use std::borrow::Cow;

use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Gauge, Paragraph};

use super::{Chat, HEIGHT, SPINNER, compact_count};
use crate::agent::{self, PermissionMode};
use crate::theme::{self, Variant};
use crate::tui::input::extensions::{self, mode_variant};
use crate::tui::input::{self, InputMode};
use crate::tui::usage;
use crate::tui::widgets::badge;

pub(super) const HELP_NORMAL: &str =
    "i escrever · Shift+Tab modo · PgUp/PgDn rolar · Tab trocar aba · Esc ir para Sair";
const HELP_RUNNING: &str = "Esc interromper · i escrever · Shift+Tab modo · PgUp/PgDn rolar";
const HELP_INSERT: &str = "Enter enviar · Esc parar · \\+Enter nova linha · @ arquivo · / comando";
const HELP_PICKER: &str = "↑/↓ escolher · Tab/Enter inserir · Esc fechar · digite para filtrar";
pub(super) const HELP_MODEL: &str =
    "↑/↓ escolher · ←/→ esforço · Enter selecionar · Esc voltar/fechar";
pub(super) const HELP_PERMISSION: &str =
    "←/→ escolher · Enter confirmar · s permitir · n/Esc negar";
pub(super) const HELP_QUESTION: &str =
    "↑/↓ escolher · Espaço marcar · Enter confirmar · ← voltar · Esc não responder";
const HELP_COMMANDS: &str = "↑/↓ escolher · Tab/Enter executar · Esc fechar · digite para filtrar";
/// Largura da barra de contexto, com as bordas.
const CONTEXT_WIDTH: u16 = 26;
/// Texto da barra de contexto quando ele chega no ponto de compactar.
/// Largura da barra de consumo, ao lado da de contexto.
const USAGE_WIDTH: u16 = 24;
const COMPACT_HINT: &str = "Compacte a conversa";

impl Chat {
    /// Texto do rodapé: o aviso do momento ou as teclas do modo atual.
    pub fn help(&self) -> Cow<'static, str> {
        if let Some((message, _)) = &self.notice {
            return Cow::Owned(message.clone());
        }
        Cow::Borrowed(match self.input.mode() {
            _ if self.model_menu.is_some() => HELP_MODEL,
            _ if self.prompts.asks_question() => HELP_QUESTION,
            _ if self.prompts.is_open() => HELP_PERMISSION,
            _ if self.input.command_picker_open() => HELP_COMMANDS,
            _ if self.input.picker_open() => HELP_PICKER,
            InputMode::Normal if self.run.is_some() => HELP_RUNNING,
            InputMode::Normal => HELP_NORMAL,
            InputMode::Insert => HELP_INSERT,
        })
    }

    /// Histórico da conversa (acima do painel).
    pub fn draw_history(&self, frame: &mut Frame, area: Rect) {
        let hint = match self.available_adapter() {
            Some(adapter) => format!(
                "Converse com o {}: i para escrever, Enter para enviar.",
                adapter.name()
            ),
            None => self.unavailable_hint(),
        };
        let empty = vec![
            Line::styled("Chat", theme::section_title()),
            Line::styled(hint, theme::muted()),
        ];
        self.history.draw(frame, area, empty);
    }

    /// Altura do painel: sem badges para mostrar, a linha deles some e o
    /// espaço fica para a conversa.
    pub fn height(&self) -> u16 {
        if self.badges().is_empty() && self.context_bar().is_none() && self.usage_bar().is_none() {
            HEIGHT - badge::HEIGHT
        } else {
            HEIGHT
        }
    }

    /// Badges visíveis: o modo fora do padrão (Agente) ou com o agente ativo
    /// (respondendo ou com sessão iniciada), e o consumo depois do primeiro
    /// token.
    pub(super) fn badges(&self) -> Vec<(String, Variant)> {
        let mode = self.permission_mode;
        let active = self.run.is_some() || self.agent_mode.is_some();
        let mut badges = Vec::new();
        if let Some(name) = &self.viewer_mode {
            badges.push((format!("Modo: {name}"), Variant::Secondary));
        } else if mode != PermissionMode::default() || active {
            // Trocado com Shift+Tab durante a resposta: o agente segue no modo
            // dele até a próxima mensagem.
            let label = match self.agent_mode {
                Some(current) if self.run.is_some() && current != mode => {
                    format!("Modo: {} → {}", current.label(), mode.label())
                }
                _ => format!("Modo: {}", mode.label()),
            };
            badges.push((label, mode_variant(mode)));
        }
        if self.tokens_used > 0 {
            badges.push((
                format!("Consumo: {} Tokens", compact_count(self.tokens_used)),
                Variant::Secondary,
            ));
        }
        badges
    }

    /// A barra de contexto, quando ela está visível e o agente já informou o
    /// tamanho dele: o texto e quanto do ponto de compactar está ocupado (de
    /// 0 a 1).
    pub(super) fn context_bar(&self) -> Option<(String, f64)> {
        let tokens = self.context.filter(|_| self.context_settings.visible)?;
        let limit = self.compact_tokens();
        let ratio = tokens as f64 / limit as f64;
        let label = if ratio >= 1.0 {
            COMPACT_HINT.to_string()
        } else {
            format!(
                "{} / {} ({:.0}%)",
                compact_count(tokens),
                compact_count(limit),
                (ratio * 100.0).floor()
            )
        };
        Some((label, ratio.min(1.0)))
    }

    /// A barra de consumo, quando ela está visível e o provedor já informou o
    /// consumo da sessão: o texto e quanto dela resta (de 0 a 100).
    pub(super) fn usage_bar(&self) -> Option<(String, f64)> {
        let window = self
            .session_usage
            .as_ref()
            .filter(|_| self.usage_settings.visible)?;
        let left = (100.0 - window.percent).clamp(0.0, 100.0);
        Some((usage::session_label(window, &TimeZone::system()), left))
    }

    /// Painel: atividade do agente, caixa de input e badges.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        // Os badges ficam com o que sobra: sem eles, o painel encolhe (veja
        // `height`) e o input mantém a altura inteira.
        let [activity, input, badges] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(input::HEIGHT),
            Constraint::Fill(1),
        ])
        .areas(area);

        if let Some(text) = &self.agent_activity {
            let line = format!(" {} {text}", SPINNER[self.spinner_frame]);
            frame.render_widget(
                Paragraph::new(Span::styled(line, theme::activity())),
                activity,
            );
        }
        let status = extensions::status_line(self.agent_status);
        self.input.draw(frame, input, self.model_title(), status);
        if let Some(menu) = &self.model_menu {
            let label = |m: &agent::ModelInfo| self.model_label_of(&m.id);
            menu.draw(frame, input, self.catalog.as_ref(), &label);
        }
        self.prompts.draw(frame, input);
        self.draw_badges(frame, badges);
    }

    /// O título do input com o provedor e o modelo desta conversa.
    pub(super) fn model_title(&self) -> Line<'static> {
        // O provedor é trocado na Config (Provedor).
        let router = match (&self.adapter, self.viewer_provider) {
            (Some(adapter), _) => adapter.name(),
            (None, Some((name, _))) => name,
            (None, None) => "Sem agente",
        };
        let model = match (&self.model, &self.adapter, self.viewer_provider) {
            (Some(id), Some(adapter), _) => adapter.model_label(id),
            (Some(id), None, Some((_, label))) => label(id),
            _ => "Modelo padrão".to_string(),
        };
        extensions::model_title(router, &model)
    }

    /// Os badges à esquerda e, no canto direito, embaixo do status, a barra
    /// de contexto, com a de consumo à esquerda dela.
    fn draw_badges(&self, frame: &mut Frame, area: Rect) {
        let mut area = area;
        if let Some((label, ratio)) = self.context_bar() {
            let [badges, context] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Length(CONTEXT_WIDTH)])
                    .spacing(1)
                    .areas(area);
            area = badges;
            let gauge = Gauge::default()
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(theme::context_border(ratio)),
                )
                .gauge_style(theme::context_bar(ratio))
                .ratio(ratio)
                .label(label);
            frame.render_widget(gauge, context);
        }
        if let Some((label, left)) = self.usage_bar() {
            let [badges, usage] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Length(USAGE_WIDTH)])
                    .spacing(1)
                    .areas(area);
            area = badges;
            let gauge = Gauge::default()
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(theme::usage_border(left)),
                )
                .gauge_style(theme::usage_bar(left))
                .ratio(left / 100.0)
                .label(label);
            frame.render_widget(gauge, usage);
        }
        let badges = self.badges();
        let areas = Layout::horizontal(
            badges
                .iter()
                .map(|(label, _)| Constraint::Length(badge::width(label))),
        )
        .spacing(1)
        .flex(Flex::Start)
        .split(area);
        for ((label, variant), area) in badges.iter().zip(areas.iter()) {
            frame.render_widget(badge::badge(label, *variant), *area);
        }
    }
}
