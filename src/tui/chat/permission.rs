//! Pedidos de permissão do agente: a caixa acima do input com a ferramenta e
//! os botões Permitir e Negar. Os pedidos ficam numa fila e aparecem um de
//! cada vez. Há três tipos: o agente esperando a resposta (ela vai para a
//! sessão), uma ferramenta que o modo já bloqueou (permitir libera ela numa
//! mensagem nova) e o agente de uma execução despachada esperando a resposta
//! (ela vai ao hub; só no chat principal).

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use crate::agent::Question;
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
    /// O agente pergunta ao usuário: as respostas saem em
    /// [`Prompts::take_answers`] (Esc recusa).
    Question {
        id: String,
        questions: Vec<Question>,
    },
    /// O agente `agent` da execução despachada `run` espera a resposta: o id
    /// do pedido no hub. Não depende da resposta deste chat: fica quando
    /// ela acaba ou é interrompida, e sai quando a execução termina.
    Remote {
        run: String,
        id: String,
        agent: String,
    },
    /// O `sudo` do agente pede a senha: ela é digitada oculta e sai em
    /// [`Prompts::take_secret`] (Esc nega).
    Password { id: String },
}

/// A fila de pedidos e o botão escolhido no da frente.
#[derive(Debug, Default)]
pub struct Prompts {
    queue: VecDeque<Request>,
    allow: bool,
    /// Na pergunta da frente: a opção escolhida (a última é "Outro"), o texto
    /// do "Outro" sendo digitado e as respostas já dadas.
    choice: usize,
    typing: Option<String>,
    answers: Vec<(String, String)>,
    /// A senha sendo digitada no pedido do `sudo`.
    secret: String,
}

const OTHER_LABEL: &str = "Outro…";
/// O tamanho máximo do [`Prompts::summary`].
const SUMMARY_CHARS: usize = 160;

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

    /// O pedido da frente numa linha, para as outras janelas verem que ele
    /// espera (ex.: `Pergunta: Qual cor?`, `Permissão: Bash ls`).
    pub fn summary(&self) -> Option<String> {
        let request = self.current()?;
        let what = format!("{} {}", request.name, request.detail);
        let text = match &request.kind {
            Kind::Question { questions, .. } => format!(
                "Pergunta: {}",
                questions.first().map_or("", |q| q.text.as_str())
            ),
            Kind::Ask { .. } | Kind::Remote { .. } => format!("Permissão: {what}"),
            Kind::Blocked { .. } => format!("Bloqueado: {what}"),
            Kind::Password { .. } => format!("Senha do sudo: {}", request.detail),
        };
        // Curto: vai ao hub em cada status.
        Some(text.trim().chars().take(SUMMARY_CHARS).collect())
    }

    pub fn is_open(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Esquece os pedidos deste chat (a resposta foi interrompida); os das
    /// execuções ficam.
    pub fn clear(&mut self) {
        self.queue
            .retain(|request| matches!(request.kind, Kind::Remote { .. }));
        self.secret.clear();
    }

    /// Esquece os pedidos que esperavam a sessão (a resposta acabou); os
    /// bloqueados e os das execuções ficam, porque não dependem dela.
    pub fn clear_asks(&mut self) {
        self.queue.retain(|request| {
            !matches!(
                request.kind,
                Kind::Ask { .. } | Kind::Question { .. } | Kind::Password { .. }
            )
        });
        self.secret.clear();
        self.reset_question();
        self.answers.clear();
    }

    /// Esquece os pedidos da execução `run` (ela terminou).
    pub fn clear_run(&mut self, run: &str) {
        let front = self.queue.front().cloned();
        self.queue
            .retain(|request| !matches!(&request.kind, Kind::Remote { run: r, .. } if r == run));
        // O botão escolhido era o do pedido que saiu.
        if self.queue.front() != front.as_ref() {
            self.allow = true;
        }
    }

    /// Já está na fila o pedido `id` da execução `run` (o hub manda de novo
    /// quando a janela reconecta).
    pub fn has_remote(&self, run: &str, id: &str) -> bool {
        self.queue.iter().any(|request| {
            matches!(&request.kind, Kind::Remote { run: r, id: i, .. } if r == run && i == id)
        })
    }

    /// Os pedidos das execuções esperando resposta: o id da execução e o
    /// texto do pedido (a ferramenta e o detalhe).
    pub fn remote(&self) -> Vec<(String, String)> {
        self.queue
            .iter()
            .filter_map(|request| match &request.kind {
                Kind::Remote { run, .. } => Some((
                    run.clone(),
                    format!("{} {}", request.name, request.detail)
                        .trim()
                        .to_string(),
                )),
                _ => None,
            })
            .collect()
    }

    /// Há pedido deste chat (os das execuções não contam): a sessão ainda
    /// precisa dele.
    pub fn has_local(&self) -> bool {
        self.queue
            .iter()
            .any(|request| !matches!(request.kind, Kind::Remote { .. }))
    }

    /// A pergunta da frente com a vez de responder.
    fn question(&self) -> Option<&Question> {
        match &self.current()?.kind {
            Kind::Question { questions, .. } => questions.get(self.answers.len()),
            _ => None,
        }
    }

    pub fn is_question(&self) -> bool {
        self.question().is_some()
    }

    /// As respostas da pergunta que acabou de ser respondida.
    pub fn take_answers(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.answers)
    }

    /// Teclas de uma pergunta: `↑`/`↓` escolhem, `Enter` confirma (no
    /// "Outro", abre o texto) e `Esc` recusa.
    fn question_key(&mut self, key: KeyEvent) -> Option<(Request, bool)> {
        let question = self.question()?;
        let count = question.options.len() + 1;
        let text = question.text.clone();
        let picked = question.options.get(self.choice).map(|o| o.0.clone());
        let answer = match (&mut self.typing, key.code) {
            (Some(_), KeyCode::Esc) => {
                self.typing = None;
                return None;
            }
            (Some(typed), KeyCode::Char(c)) => {
                typed.push(c);
                return None;
            }
            (Some(typed), KeyCode::Backspace) => {
                typed.pop();
                return None;
            }
            (Some(typed), KeyCode::Enter) if !typed.trim().is_empty() => typed.trim().to_string(),
            (Some(_), _) => return None,
            (None, KeyCode::Up) => {
                self.choice = (self.choice + count - 1) % count;
                return None;
            }
            (None, KeyCode::Down) => {
                self.choice = (self.choice + 1) % count;
                return None;
            }
            (None, KeyCode::Enter) => match picked {
                Some(label) => label,
                None => {
                    self.typing = Some(String::new());
                    return None;
                }
            },
            (None, KeyCode::Esc) => {
                let request = self.queue.pop_front()?;
                self.reset_question();
                self.answers.clear();
                return Some((request, false));
            }
            (None, _) => return None,
        };
        self.answers.push((text, answer));
        self.reset_question();
        if self.question().is_some() {
            return None;
        }
        Some((self.queue.pop_front()?, true))
    }

    fn reset_question(&mut self) {
        self.choice = 0;
        self.typing = None;
    }

    pub fn is_password(&self) -> bool {
        matches!(
            self.current(),
            Some(Request {
                kind: Kind::Password { .. },
                ..
            })
        )
    }

    /// A senha digitada no pedido do `sudo` que acabou de ser respondido.
    pub fn take_secret(&mut self) -> String {
        std::mem::take(&mut self.secret)
    }

    /// Teclas da senha: o texto vai oculto, `Enter` manda e `Esc` nega.
    fn password_key(&mut self, key: KeyEvent) -> Option<(Request, bool)> {
        let allow = match key.code {
            KeyCode::Char(c) => {
                self.secret.push(c);
                return None;
            }
            KeyCode::Backspace => {
                self.secret.pop();
                return None;
            }
            KeyCode::Enter if !self.secret.is_empty() => true,
            KeyCode::Esc => {
                self.secret.clear();
                false
            }
            _ => return None,
        };
        Some((self.queue.pop_front()?, allow))
    }

    /// Trata uma tecla com um pedido aberto: `←`/`→` trocam o botão, `Enter`
    /// confirma, `s`/`y` permitem e `n`/`Esc` negam. Retorna o pedido
    /// respondido e se foi permitido; as outras teclas não fazem nada.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<(Request, bool)> {
        if self.is_question() {
            return self.question_key(key);
        }
        if self.is_password() {
            return self.password_key(key);
        }
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
        let (title, verb) = match &request.kind {
            Kind::Ask { .. } => ("Permissão".to_string(), "O agente quer usar "),
            Kind::Blocked { .. } => (
                "Bloqueado pelo modo Agente".to_string(),
                "O agente tentou usar ",
            ),
            Kind::Remote { agent, .. } => (format!("Permissão · ▶ {agent}"), "O agente quer usar "),
            Kind::Question { .. } => ("Pergunta do agente".to_string(), ""),
            Kind::Password { .. } => (
                "Senha do sudo".to_string(),
                "O agente quer rodar como root com ",
            ),
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
        if let Some(question) = self.question() {
            lines.truncate(1);
            lines.push(Line::styled(
                question.text.clone(),
                theme::text().add_modifier(Modifier::BOLD),
            ));
            let labels = question
                .options
                .iter()
                .map(|(label, description)| (label.as_str(), description.as_str()))
                .chain([(OTHER_LABEL, "")]);
            for (i, (label, description)) in labels.enumerate() {
                let on = i == self.choice;
                let mut text = format!("{} {label}", if on { "›" } else { " " });
                if let (true, Some(typed)) = (on, &self.typing) {
                    text = format!("› Outro: {typed}▌");
                } else if !description.is_empty() {
                    text.push_str(&format!(" — {description}"));
                }
                lines.push(Line::styled(
                    text,
                    if on { theme::text() } else { theme::muted() },
                ));
            }
        } else if self.is_password() {
            lines.push(Line::styled(
                format!("Senha: {}▌", "•".repeat(self.secret.chars().count())),
                theme::text(),
            ));
        } else {
            lines.push(Line::from(vec![
                Span::styled(ALLOW_LABEL, theme::confirm_yes(self.allow)),
                Span::raw("   "),
                Span::styled(DENY_LABEL, theme::confirm_no(!self.allow)),
            ]));
        }

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

    fn remote(run: &str, id: &str) -> Request {
        Request {
            kind: Kind::Remote {
                run: run.to_string(),
                id: id.to_string(),
                agent: "api".to_string(),
            },
            ..request(id)
        }
    }

    #[test]
    fn requests_of_runs_survive_the_chat_and_leave_with_their_run() {
        let mut prompts = Prompts::default();
        prompts.push(request("r1"));
        prompts.push(remote("run1", "a"));
        prompts.push(remote("run2", "b"));
        assert!(prompts.has_local());
        prompts.clear_asks();
        assert_eq!(prompts.current(), Some(&remote("run1", "a")));
        assert!(!prompts.has_local());
        prompts.clear();
        assert!(prompts.has_remote("run1", "a") && prompts.has_remote("run2", "b"));
        assert!(!prompts.has_remote("run1", "b"));

        // Com Negar escolhido, o pedido sai e o próximo volta para Permitir.
        assert_eq!(prompts.handle_key(key(KeyCode::Right)), None);
        prompts.clear_run("run1");
        assert_eq!(
            prompts.handle_key(key(KeyCode::Enter)),
            Some((remote("run2", "b"), true))
        );
        assert!(!prompts.is_open());
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
    fn a_question_is_answered_with_an_option_or_typed_text() {
        let ask = |text: &str| Question {
            text: text.to_string(),
            options: vec![("A".to_string(), String::new())],
        };
        let mut prompts = Prompts::default();
        prompts.push(Request {
            name: "AskUserQuestion".to_string(),
            detail: String::new(),
            kind: Kind::Question {
                id: "q".to_string(),
                questions: vec![ask("um?"), ask("dois?")],
            },
        });
        assert_eq!(prompts.handle_key(key(KeyCode::Enter)), None);
        // "Outro" abre o texto, e a resposta é o que foi digitado.
        assert_eq!(prompts.handle_key(key(KeyCode::Down)), None);
        assert_eq!(prompts.handle_key(key(KeyCode::Enter)), None);
        prompts.handle_key(key(KeyCode::Char('x')));
        let (_, allow) = prompts.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(allow);
        assert_eq!(
            prompts.take_answers(),
            vec![
                ("um?".to_string(), "A".to_string()),
                ("dois?".to_string(), "x".to_string())
            ]
        );
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
