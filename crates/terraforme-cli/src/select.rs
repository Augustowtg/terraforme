//! Seleção múltipla no terminal: `↑`/`↓` (ou `k`/`j`) movem, `Espaço` marca,
//! `a` marca/desmarca todos, `Enter` confirma e `Esc`/`q` cancela.

use std::io::{self, Write};

use anyhow::Result;
use ratatui::crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{self, Clear, ClearType};
use ratatui::crossterm::{execute, queue};

/// Item da lista; `disabled` aparece apagado, com o motivo, e não marca.
pub struct Item {
    pub label: String,
    pub disabled: Option<String>,
}

/// Estado da lista, separado do terminal para ser testável.
struct Checklist<'a> {
    items: &'a [Item],
    checked: Vec<bool>,
    cursor: usize,
}

impl<'a> Checklist<'a> {
    fn new(items: &'a [Item], checked: &[bool]) -> Self {
        let checked = items
            .iter()
            .enumerate()
            .map(|(i, item)| item.disabled.is_none() && checked.get(i).copied().unwrap_or(false))
            .collect();
        Self {
            items,
            checked,
            cursor: 0,
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.cursor = (self.cursor as isize + delta).rem_euclid(len) as usize;
    }

    fn toggle(&mut self) {
        if self.items[self.cursor].disabled.is_none() {
            self.checked[self.cursor] = !self.checked[self.cursor];
        }
    }

    /// Marca todos os habilitados; se já estavam todos marcados, desmarca.
    fn toggle_all(&mut self) {
        let enabled = |i: &usize| self.items[*i].disabled.is_none();
        let all = (0..self.items.len())
            .filter(enabled)
            .all(|i| self.checked[i]);
        for i in (0..self.items.len()).filter(enabled).collect::<Vec<_>>() {
            self.checked[i] = !all;
        }
    }

    fn selected(&self) -> Vec<usize> {
        (0..self.items.len()).filter(|&i| self.checked[i]).collect()
    }

    fn lines(&self) -> Vec<String> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let pointer = if i == self.cursor { "❯" } else { " " };
                match &item.disabled {
                    Some(why) => format!("{pointer} [-] {} ({why})", item.label),
                    None => {
                        let mark = if self.checked[i] { "x" } else { " " };
                        format!("{pointer} [{mark}] {}", item.label)
                    }
                }
            })
            .collect()
    }
}

/// Mostra a lista no stderr e devolve os índices marcados; `None` se o
/// usuário cancelou.
pub fn checkboxes(title: &str, items: &[Item], checked: &[bool]) -> Result<Option<Vec<usize>>> {
    if items.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut list = Checklist::new(items, checked);
    let mut err = io::stderr();
    eprintln!("{title}");
    eprintln!("  ↑/↓ move · Espaço marca · a marca todos · Enter confirma · Esc cancela");

    terminal::enable_raw_mode()?;
    let _ = execute!(err, Hide);
    let result = (|| -> Result<Option<Vec<usize>>> {
        let mut drawn = 0;
        loop {
            if drawn > 0 {
                queue!(err, MoveUp(drawn as u16))?;
            }
            for line in list.lines() {
                queue!(err, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
                write!(err, "  {line}\r\n")?;
            }
            err.flush()?;
            drawn = items.len();

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => list.step(-1),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => list.step(1),
                KeyCode::Char(' ') => list.toggle(),
                KeyCode::Char('a') => list.toggle_all(),
                KeyCode::Enter => return Ok(Some(list.selected())),
                KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                _ => {}
            }
        }
    })();
    let _ = execute!(err, Show);
    terminal::disable_raw_mode()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![
            Item {
                label: "Claude Code".into(),
                disabled: None,
            },
            Item {
                label: "Cursor".into(),
                disabled: Some("sem instalador".into()),
            },
            Item {
                label: "OpenCode".into(),
                disabled: None,
            },
        ]
    }

    #[test]
    fn space_toggles_only_enabled_items() {
        let items = items();
        let mut list = Checklist::new(&items, &[true, true, false]);
        // Desabilitado não começa marcado.
        assert_eq!(list.selected(), vec![0]);
        list.step(1);
        list.toggle();
        assert_eq!(list.selected(), vec![0]);
        list.step(1);
        list.toggle();
        assert_eq!(list.selected(), vec![0, 2]);
        // Volta do primeiro para o último.
        list.step(1);
        assert_eq!(list.cursor, 0);
        list.step(-1);
        assert_eq!(list.cursor, 2);
    }

    #[test]
    fn toggle_all_marks_then_clears() {
        let items = items();
        let mut list = Checklist::new(&items, &[]);
        list.toggle_all();
        assert_eq!(list.selected(), vec![0, 2]);
        list.toggle_all();
        assert!(list.selected().is_empty());
    }

    #[test]
    fn lines_show_cursor_marks_and_reason() {
        let items = items();
        let list = Checklist::new(&items, &[true]);
        assert_eq!(
            list.lines(),
            vec![
                "❯ [x] Claude Code",
                "  [-] Cursor (sem instalador)",
                "  [ ] OpenCode",
            ]
        );
    }
}
