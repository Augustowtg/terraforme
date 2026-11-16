//! Aba Terraforme: o agente que vai orquestrar o terra. Por enquanto, só o
//! logo no canto superior esquerdo (15% da altura da tela) e o mesmo input
//! do chat, embaixo.

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::input::{self, InputBox, Outcome};
use crate::theme;

pub const HELP: &str = "i escrever · Esc sair do modo de escrita · Tab trocar aba";

/// Quanto da altura da tela o logo ocupa, em porcentagem.
const LOGO_PERCENT: u16 = 15;
const WORD: &str = "TERRAFORME";
/// Altura e largura de cada letra do logo, em "pixels".
const GLYPH_HEIGHT: usize = 5;
const GLYPH_WIDTH: usize = 5;
/// Pixels vazios entre as letras.
const GLYPH_GAP: usize = 1;

/// As letras do logo, cinco linhas de cinco pixels (`#` aceso).
fn glyph(c: char) -> [&'static str; GLYPH_HEIGHT] {
    match c {
        'T' => ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
        'E' => ["#####", "#    ", "#### ", "#    ", "#####"],
        'R' => ["#### ", "#   #", "#### ", "#  # ", "#   #"],
        'A' => [" ### ", "#   #", "#####", "#   #", "#   #"],
        'F' => ["#####", "#    ", "#### ", "#    ", "#    "],
        'O' => [" ### ", "#   #", "#   #", "#   #", " ### "],
        'M' => ["#   #", "## ##", "# # #", "#   #", "#   #"],
        _ => ["     "; GLYPH_HEIGHT],
    }
}

/// O logo em pixels, sem escala: uma linha por linha de pixels.
fn pixels() -> Vec<Vec<bool>> {
    let letters: Vec<_> = WORD.chars().map(glyph).collect();
    (0..GLYPH_HEIGHT)
        .map(|row| {
            let mut line = Vec::new();
            for (i, letter) in letters.iter().enumerate() {
                if i > 0 {
                    line.extend(std::iter::repeat_n(false, GLYPH_GAP));
                }
                line.extend(letter[row].chars().map(|c| c == '#'));
            }
            line
        })
        .collect()
}

/// Largura do logo sem escala, em colunas.
fn base_width() -> usize {
    WORD.chars().count() * (GLYPH_WIDTH + GLYPH_GAP) - GLYPH_GAP
}

/// O logo na escala `scale`, com meio bloco (`▀▄█`): cada linha da tela
/// guarda duas linhas de pixels, e os pixels ficam quase quadrados.
fn logo_lines(scale: usize) -> Vec<String> {
    let scaled: Vec<Vec<bool>> = pixels()
        .into_iter()
        .flat_map(|row| {
            let wide: Vec<bool> = row
                .into_iter()
                .flat_map(|p| std::iter::repeat_n(p, scale))
                .collect();
            std::iter::repeat_n(wide, scale)
        })
        .collect();
    let empty = vec![false; scaled[0].len()];
    scaled
        .chunks(2)
        .map(|pair| {
            let top = &pair[0];
            let bottom = pair.get(1).unwrap_or(&empty);
            top.iter()
                .zip(bottom)
                .map(|(&t, &b)| match (t, b) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect()
        })
        .collect()
}

/// A maior escala que cabe em `area` (ao menos 1).
fn logo_scale(area: Rect) -> usize {
    (1..)
        .take_while(|&s| {
            (GLYPH_HEIGHT * s).div_ceil(2) <= area.height as usize
                && base_width() * s <= area.width as usize
        })
        .last()
        .unwrap_or(1)
}

#[derive(Default)]
pub struct TerraformeView {
    input: InputBox,
}

impl TerraformeView {
    /// Retorna `true` se a tecla foi usada pelo input.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.input.handle_key(key) {
            Outcome::Ignored => false,
            // O agente ainda não existe: o texto enviado só fica no
            // histórico do ↑.
            Outcome::Submit => {
                self.input.remember();
                self.input.clear();
                true
            }
            Outcome::Handled | Outcome::Command(_) => true,
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        self.input.paste(text);
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        self.input.handle_mouse(mouse);
    }

    pub fn set_caps_lock(&mut self, on: bool) {
        self.input.set_caps_lock(on);
    }

    pub fn tick(&mut self) {
        self.input.tick();
    }

    /// Desenha a aba em `area`; o logo usa 15% da altura de `screen`.
    pub fn draw(&self, frame: &mut Frame, area: Rect, screen: Rect) {
        let logo_height = (screen.height * LOGO_PERCENT / 100).max(3);
        let [logo, _, input] = Layout::vertical([
            Constraint::Length(logo_height),
            Constraint::Fill(1),
            Constraint::Length(input::HEIGHT),
        ])
        .areas(area);
        let [_, logo] =
            Layout::horizontal([Constraint::Length(1), Constraint::Fill(1)]).areas(logo);
        let [_, logo] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(logo);

        let lines: Vec<Line> = if logo.width as usize >= base_width() {
            logo_lines(logo_scale(logo))
                .into_iter()
                .map(|l| Line::from(Span::styled(l, theme::section_title())))
                .collect()
        } else {
            vec![Line::from(Span::styled(WORD, theme::section_title()))]
        };
        frame.render_widget(Paragraph::new(lines), logo);

        self.input.draw(
            frame,
            input,
            Line::from(Span::styled(" Terraforme ", theme::section_title())),
            Line::default(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_cresce_com_a_area() {
        assert_eq!(logo_scale(Rect::new(0, 0, 200, 7)), 2);
        assert_eq!(logo_scale(Rect::new(0, 0, 80, 7)), 1);
        assert_eq!(logo_scale(Rect::new(0, 0, 400, 13)), 5);
    }

    #[test]
    fn logo_em_meio_bloco_tem_metade_das_linhas() {
        let lines = logo_lines(2);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|l| l.chars().count() == base_width() * 2));
        assert!(lines[0].starts_with("██████████"));
    }
}
