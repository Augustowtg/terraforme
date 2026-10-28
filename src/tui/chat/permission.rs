//! Pedidos de permissão do agente: a caixa acima do input com a ferramenta e
//! os botões Permitir e Negar. Os pedidos ficam numa fila e aparecem um de
//! cada vez. Há dois tipos: o agente esperando a resposta (ela vai para a
//! sessão) e uma ferramenta que o modo já bloqueou (permitir libera ela numa
//! mensagem nova).

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use crate::theme::{self, Variant};

const ALLOW_LABEL: &str = " Permitir ";
const DENY_LABEL: &str = " Negar ";

/// Um pedido do agente esperando a resposta do usuário.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub name: String,
    pub detail: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// O agente espera a resposta: o id do pedido na sessão.
    Ask { id: String },
    /// O modo bloqueou a ferramenta sem perguntar, pelo motivo `reason`; o
    /// agente seguiu sem ela. `allow` libera ela numa mensagem nova.
    Blocked { reason: String, allow: String },
}

/// A fila de pedidos e o botão escolhido no da frente.
#[derive(Debug, Default)]
pub struct Prompts {
    queue: VecDeque<Request>,
    allow: bool,
}

impl Prompts {
    pub fn push(&mut self, request: Request) {
        if self.queue.is_empty() {
            self.allow = true;
        }
        self.queue.push_back(request);
    }

    pub fn current(&self) -> Option<&Request> {
        self.queue.front()
    }

    pub fn is_open(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Esquece os pedidos (a resposta foi interrompida).
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Esquece os pedidos que esperavam a sessão (a resposta acabou); os
    /// bloqueados ficam, porque não dependem dela.
    pub fn clear_asks(&mut self) {
        self.queue
            .retain(|request| matches!(request.kind, Kind::Blocked { .. }));
    }

    /// Trata uma tecla com um pedido aberto: `←`/`→` trocam o botão, `Enter`
    /// confirma, `s`/`y` permitem e `n`/`Esc` negam. Retorna o pedido
    /// respondido e se foi permitido; as outras teclas não fazem nada.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<(Request, bool)> {
        let allow = match key.code {
            KeyCode::Left | KeyCode::Right => {
                self.allow = !self.allow;
                return None;
            }
            KeyCode::Enter => self.allow,
            KeyCode::Char('s' | 'S' | 'y' | 'Y') => true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => false,
            _ => return None,
        };
        let request = self.queue.pop_front()?;
        self.allow = true;
        Some((request, allow))
    }

    /// A caixa do pedido da frente, logo acima do input.
    pub fn draw(&self, frame: &mut Frame, input: Rect) {
        let Some(request) = self.current() else {
            return;
        };
        let (title, verb) = match request.kind {
            Kind::Ask { .. } => ("Permissão", "O agente quer usar "),
            Kind::Blocked { .. } => ("Bloqueado pelo modo Agente", "O agente tentou usar "),
        };
        let mut lines = vec![
            Line::styled(
                title,
                theme::variant(Variant::Warning).add_modifier(Modifier::BOLD),
            ),
            Line::from(vec![
                Span::styled(verb, theme::text()),
                Span::styled(
                    request.name.clone(),
                    theme::text().add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        if !request.detail.is_empty() {
            lines.push(Line::styled(request.detail.clone(), theme::path()));
        }
        if let Kind::Blocked { reason, .. } = &request.kind {
            if !reason.is_empty() {
                lines.push(Line::styled(format!("Motivo: {reason}"), theme::muted()));
            }
            lines.push(Line::styled(
                "Permitir manda o agente fazer numa mensagem nova, com ela liberada.",
                theme::muted(),
            ));
        }
        let waiting = self.queue.len() - 1;
        if waiting > 0 {
            lines.push(Line::styled(
                format!("mais {waiting} pedido(s) na fila"),
                theme::muted(),
            ));
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(ALLOW_LABEL, theme::confirm_yes(self.allow)),
            Span::raw("   "),
            Span::styled(DENY_LABEL, theme::confirm_no(!self.allow)),
        ]));

        let height = (lines.len() as u16 + 2).min(input.y);
        if height < 3 {
            return;
        }
        let area = Rect {
            y: input.y - height,
            height,
            ..input
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::variant(Variant::Warning))
            .padding(Padding::horizontal(1))
            .style(theme::screen());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn request(id: &str) -> Request {
        Request {
            name: "Bash".to_string(),
            detail: "ls".to_string(),
            kind: Kind::Ask { id: id.to_string() },
        }
    }

    #[test]
    fn finishing_keeps_only_blocked_requests() {
        let mut prompts = Prompts::default();
        prompts.push(request("r1"));
        let blocked = Request {
            kind: Kind::Blocked {
                reason: "perigoso".to_string(),
                allow: "Bash(ls)".to_string(),
            },
            ..request("r2")
        };
        prompts.push(blocked.clone());
        prompts.clear_asks();
        assert_eq!(prompts.current(), Some(&blocked));
    }

    #[test]
    fn requests_are_answered_in_order() {
        let mut prompts = Prompts::default();
        prompts.push(request("r1"));
        prompts.push(request("r2"));

        assert_eq!(prompts.handle_key(key(KeyCode::Char('x'))), None);
        assert_eq!(
            prompts.handle_key(key(KeyCode::Enter)),
            Some((request("r1"), true))
        );
        assert_eq!(prompts.handle_key(key(KeyCode::Right)), None);
        assert_eq!(
            prompts.handle_key(key(KeyCode::Enter)),
            Some((request("r2"), false))
        );
        assert!(!prompts.is_open());
        assert_eq!(prompts.handle_key(key(KeyCode::Enter)), None);
    }

    #[test]
    fn shortcuts_allow_and_deny() {
        let mut prompts = Prompts::default();
        for id in ["r1", "r2", "r3"] {
            prompts.push(request(id));
        }
        assert_eq!(
            prompts.handle_key(key(KeyCode::Char('s'))).map(|r| r.1),
            Some(true)
        );
        assert_eq!(
            prompts.handle_key(key(KeyCode::Char('n'))).map(|r| r.1),
            Some(false)
        );
        assert_eq!(
            prompts.handle_key(key(KeyCode::Esc)).map(|r| r.1),
            Some(false)
        );
    }
}
