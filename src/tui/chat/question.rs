//! As perguntas do agente: o formulário da caixa acima do input, com uma
//! pergunta de cada vez, as opções dela e uma resposta escrita pelo usuário.

use std::collections::BTreeSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::agent::Question;
use crate::theme::{self, Variant};

const TITLE: &str = "Pergunta do agente";
const OTHER_LABEL: &str = "Outra resposta: ";
const OTHER_HINT: &str = "digite aqui";
const POINTER: &str = "› ";
const NO_POINTER: &str = "  ";

/// O que uma tecla fez no formulário.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    /// Todas as perguntas têm resposta.
    Answered,
    /// O usuário não quis responder.
    Refused,
}

/// As perguntas de um pedido e o que o usuário já escolheu em cada uma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    questions: Vec<Question>,
    choices: Vec<Choice>,
    /// A pergunta na tela.
    step: usize,
}

/// A escolha numa pergunta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Choice {
    /// A linha sob o cursor; depois das opções, a da resposta escrita.
    cursor: usize,
    /// As opções marcadas, nas perguntas de várias respostas.
    marked: BTreeSet<usize>,
    /// A resposta escrita.
    other: String,
}

impl Form {
    pub fn new(questions: Vec<Question>) -> Self {
        let choices = vec![Choice::default(); questions.len()];
        Self {
            questions,
            choices,
            step: 0,
        }
    }

    /// `↑`/`↓` trocam de linha, `Espaço` marca a opção, `Enter` confirma a
    /// pergunta, `←` volta à anterior e `Esc` recusa. Na linha da resposta
    /// escrita, os caracteres vão para o texto.
    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        let question = &self.questions[self.step];
        let other = question.options.len();
        let choice = &mut self.choices[self.step];
        let writing = choice.cursor == other;
        match key.code {
            KeyCode::Esc => return Outcome::Refused,
            KeyCode::Up => choice.cursor = choice.cursor.checked_sub(1).unwrap_or(other),
            KeyCode::Down => choice.cursor = (choice.cursor + 1) % (other + 1),
            KeyCode::Left => self.step = self.step.saturating_sub(1),
            KeyCode::Backspace if writing => {
                choice.other.pop();
            }
            KeyCode::Char(' ') if !writing => {
                if question.multi_select && !choice.marked.remove(&choice.cursor) {
                    choice.marked.insert(choice.cursor);
                }
            }
            KeyCode::Char(c)
                if writing
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                choice.other.push(c);
            }
            KeyCode::Enter => return self.confirm(),
            _ => {}
        }
        Outcome::Pending
    }

    /// Cola o texto na resposta escrita da pergunta na tela, numa linha só.
    pub fn paste(&mut self, text: &str) {
        let choice = &mut self.choices[self.step];
        choice.cursor = self.questions[self.step].options.len();
        choice
            .other
            .push_str(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    /// As respostas, na ordem das perguntas.
    pub fn answers(&self) -> Vec<String> {
        (0..self.questions.len())
            .map(|step| self.answer(step).unwrap_or_default())
            .collect()
    }

    /// As linhas do histórico sobre o que foi respondido.
    pub fn summary(&self, answered: bool) -> Vec<String> {
        self.questions
            .iter()
            .zip(self.answers())
            .map(|(question, answer)| {
                if answered {
                    format!("Pergunta: {} Resposta: {answer}", question.text)
                } else {
                    format!("Pergunta sem resposta: {}", question.text)
                }
            })
            .collect()
    }

    /// As linhas da caixa, quebradas em `width` colunas, e a linha do cursor.
    pub fn lines(&self, width: u16) -> (Vec<Line<'static>>, u16) {
        let width = usize::from(width).max(1);
        let question = &self.questions[self.step];
        let choice = &self.choices[self.step];

        let mut title = TITLE.to_string();
        if !question.header.is_empty() {
            title = format!("{title} · {}", question.header);
        }
        if self.questions.len() > 1 {
            title = format!("{title} ({}/{})", self.step + 1, self.questions.len());
        }
        let mut lines = vec![Line::styled(
            title,
            theme::variant(Variant::Warning).add_modifier(Modifier::BOLD),
        )];
        lines.extend(
            wrap(&question.text, width)
                .into_iter()
                .map(|row| Line::styled(row, theme::text().add_modifier(Modifier::BOLD))),
        );
        if question.multi_select {
            lines.push(Line::styled("Pode marcar mais de uma.", theme::muted()));
        }
        lines.push(Line::default());

        let mut focus = 0;
        for (index, option) in question.options.iter().enumerate() {
            let selected = choice.cursor == index;
            if selected {
                focus = lines.len();
            }
            let mark = match (question.multi_select, choice.marked.contains(&index)) {
                (false, _) => "",
                (true, true) => "[x] ",
                (true, false) => "[ ] ",
            };
            let label = format!("{mark}{}", option.label);
            lines.extend(rows(&label, selected, row_style(selected), width));
            lines.extend(
                wrap(&option.description, width.saturating_sub(POINTER.width()))
                    .into_iter()
                    .map(|row| Line::styled(format!("{NO_POINTER}{row}"), theme::muted())),
            );
        }

        let writing = choice.cursor == question.options.len();
        let text = if writing {
            format!("{OTHER_LABEL}{}▏", choice.other)
        } else if choice.other.is_empty() {
            format!("{OTHER_LABEL}{OTHER_HINT}")
        } else {
            format!("{OTHER_LABEL}{}", choice.other)
        };
        lines.extend(rows(&text, writing, row_style(writing), width));
        if writing {
            focus = lines.len() - 1;
        }
        (lines, focus as u16)
    }

    /// Confirma a pergunta na tela: passa à próxima ou, na última, termina.
    /// Sem resposta, fica nela; nas de várias respostas, `Enter` sem nenhuma
    /// marcada marca a opção sob o cursor.
    fn confirm(&mut self) -> Outcome {
        let (question, choice) = (&self.questions[self.step], &mut self.choices[self.step]);
        if question.multi_select
            && choice.marked.is_empty()
            && choice.other.trim().is_empty()
            && choice.cursor < question.options.len()
        {
            choice.marked.insert(choice.cursor);
        }
        if self.answer(self.step).is_none() {
            return Outcome::Pending;
        }
        if self.step + 1 == self.questions.len() {
            return Outcome::Answered;
        }
        self.step += 1;
        Outcome::Pending
    }

    /// A resposta de uma pergunta: a opção sob o cursor ou, nas de várias
    /// respostas, as marcadas (separadas por vírgula, como o `claude` espera),
    /// mais a escrita. `None` sem nenhuma.
    fn answer(&self, step: usize) -> Option<String> {
        let (question, choice) = (&self.questions[step], &self.choices[step]);
        let other = choice.other.trim();
        if !question.multi_select {
            return match question.options.get(choice.cursor) {
                Some(option) => Some(option.label.clone()),
                None => (!other.is_empty()).then(|| other.to_string()),
            };
        }
        let mut answers: Vec<&str> = choice
            .marked
            .iter()
            .filter_map(|&index| question.options.get(index))
            .map(|option| option.label.as_str())
            .collect();
        if !other.is_empty() {
            answers.push(other);
        }
        (!answers.is_empty()).then(|| answers.join(", "))
    }
}

/// O estilo de uma linha de resposta.
fn row_style(selected: bool) -> Style {
    if selected {
        theme::text().add_modifier(Modifier::BOLD)
    } else {
        theme::text()
    }
}

/// Uma linha de resposta quebrada, com o ponteiro na primeira se `selected`.
fn rows(text: &str, selected: bool, style: Style, width: usize) -> Vec<Line<'static>> {
    wrap(text, width.saturating_sub(POINTER.width()))
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let prefix = if selected && index == 0 {
                POINTER
            } else {
                NO_POINTER
            };
            Line::from(vec![
                Span::styled(prefix, theme::variant(Variant::Warning)),
                Span::styled(row, style),
            ])
        })
        .collect()
}

/// Quebra o texto em linhas de até `width` colunas, por palavra e em cada
/// `\n`; palavras maiores que a linha são cortadas. Sem texto, nenhuma linha.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for paragraph in text.lines() {
        let mut row = String::new();
        let mut used = 0;
        for word in paragraph.split_whitespace() {
            let size = word.width();
            if used > 0 && used + 1 + size > width {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            if used > 0 {
                row.push(' ');
                used += 1;
            }
            for c in word.chars() {
                let size = c.width().unwrap_or(0);
                if used + size > width && used > 0 {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                row.push(c);
                used += size;
            }
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::QuestionOption;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn question(text: &str, multi_select: bool, options: &[&str]) -> Question {
        Question {
            header: "Tema".to_string(),
            text: text.to_string(),
            multi_select,
            options: options
                .iter()
                .map(|label| QuestionOption {
                    label: label.to_string(),
                    description: format!("sobre {label}"),
                })
                .collect(),
        }
    }

    fn screen(form: &Form, width: u16) -> Vec<String> {
        form.lines(width)
            .0
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn enter_goes_through_the_questions_and_left_goes_back() {
        let mut form = Form::new(vec![
            question("Cor?", false, &["Azul", "Verde"]),
            question("Tamanho?", false, &["P", "G"]),
        ]);
        form.handle_key(key(KeyCode::Down));
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Pending);
        assert!(screen(&form, 40)[0].ends_with("(2/2)"));
        assert!(screen(&form, 40).contains(&"Tamanho?".to_string()));

        // Voltar mantém a escolha.
        form.handle_key(key(KeyCode::Left));
        assert!(screen(&form, 40).contains(&"› Verde".to_string()));
        form.handle_key(key(KeyCode::Enter));

        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Answered);
        assert_eq!(form.answers(), ["Verde", "P"]);
        assert_eq!(
            form.summary(true),
            [
                "Pergunta: Cor? Resposta: Verde",
                "Pergunta: Tamanho? Resposta: P"
            ]
        );
        assert_eq!(form.summary(false)[0], "Pergunta sem resposta: Cor?");
    }

    #[test]
    fn several_answers_are_marked_and_joined() {
        let mut form = Form::new(vec![question("Quais?", true, &["A", "B", "C"])]);
        form.handle_key(key(KeyCode::Char(' ')));
        form.handle_key(key(KeyCode::Down));
        form.handle_key(key(KeyCode::Down));
        form.handle_key(key(KeyCode::Char(' ')));
        form.handle_key(key(KeyCode::Up));
        form.handle_key(key(KeyCode::Char(' ')));
        form.handle_key(key(KeyCode::Char(' ')));
        assert!(screen(&form, 40).contains(&"  [x] C".to_string()));
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Answered);
        assert_eq!(form.answers(), ["A, C"]);

        // Enter sem marcar nada marca a opção sob o cursor.
        let mut form = Form::new(vec![question("Quais?", true, &["A", "B"])]);
        form.handle_key(key(KeyCode::Down));
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Answered);
        assert_eq!(form.answers(), ["B"]);
    }

    #[test]
    fn the_written_answer_needs_text() {
        let mut form = Form::new(vec![question("Cor?", false, &["Azul"])]);
        form.handle_key(key(KeyCode::Up));
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Pending);

        for c in "roxz".chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
        form.handle_key(key(KeyCode::Backspace));
        form.handle_key(key(KeyCode::Char('o')));
        form.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(screen(&form, 40).contains(&"› Outra resposta: roxo▏".to_string()));
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Answered);
        assert_eq!(form.answers(), ["roxo"]);

        // Colar vai para a resposta escrita, numa linha só.
        let mut form = Form::new(vec![question("Quais?", true, &["A"])]);
        form.handle_key(key(KeyCode::Char(' ')));
        form.paste("duas\nlinhas");
        assert_eq!(form.handle_key(key(KeyCode::Enter)), Outcome::Answered);
        assert_eq!(form.answers(), ["A, duas linhas"]);
    }

    #[test]
    fn letters_only_write_on_the_written_answer_and_esc_refuses() {
        let mut form = Form::new(vec![question("Cor?", false, &["Azul"])]);
        assert_eq!(form.handle_key(key(KeyCode::Char('n'))), Outcome::Pending);
        assert_eq!(form.choices[0].other, "");
        assert_eq!(form.handle_key(key(KeyCode::Esc)), Outcome::Refused);
    }

    #[test]
    fn long_text_wraps_and_the_cursor_row_follows() {
        assert_eq!(wrap("um dois tres", 7), ["um dois", "tres"]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
        assert!(wrap("", 10).is_empty());

        let mut form = Form::new(vec![question("Cor?", false, &["Azul", "Verde"])]);
        // Título, pergunta, espaço, Azul e a descrição dele: Verde é a linha 5.
        form.handle_key(key(KeyCode::Down));
        assert_eq!(form.lines(40).1, 5);
        form.handle_key(key(KeyCode::Down));
        assert_eq!(form.lines(40).1, 7);
    }
}
