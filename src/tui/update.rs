//! A aba Atualização: aparece ao lado do Chat quando o CHANGELOG do GitHub
//! tem uma versão mais nova que a do terra. Mostra as notas das versões
//! novas (`↑/↓` e `PgUp/PgDn` rolam) e o `Enter` instala, pelo mesmo
//! caminho do `terra upgrade`, fora da tela.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};

use crate::theme;
use crate::upgrade::{self, Update};

pub const HELP: &str = "Enter instalar · ↑/↓ rolar · Tab trocar aba · Esc ir para Sair";

#[derive(Default)]
pub struct UpdateView {
    /// A verificação em andamento, numa thread.
    checking: Option<Receiver<Option<Update>>>,
    update: Option<Update>,
    scroll: u16,
    /// Pedido de instalar, para o loop rodar fora da tela.
    install: bool,
    /// Como terminou a última instalação.
    result: Option<Result<(), String>>,
}

impl UpdateView {
    /// Procura uma versão nova numa thread; sem rede, a aba não aparece.
    pub fn check(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(upgrade::check().ok().flatten());
        });
        self.checking = Some(rx);
    }

    /// Recebe o resultado da verificação.
    pub fn tick(&mut self) {
        let Some(rx) = &self.checking else {
            return;
        };
        match rx.try_recv() {
            Ok(update) => {
                self.update = update;
                self.checking = None;
            }
            Err(TryRecvError::Disconnected) => self.checking = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Há versão nova: a aba aparece.
    pub fn available(&self) -> bool {
        self.update.is_some()
    }

    #[cfg(test)]
    pub fn set_update(&mut self, update: Option<Update>) {
        self.update = update;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => self.install = true,
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::Home => self.scroll = 0,
            _ => return false,
        }
        true
    }

    pub fn take_install(&mut self) -> bool {
        std::mem::take(&mut self.install)
    }

    pub fn finish_install(&mut self, result: Result<(), String>) {
        self.result = Some(result);
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let Some(update) = &self.update else {
            return;
        };
        let mut lines = vec![
            Line::styled(
                format!(
                    " Nova versão do terra: {} (a sua é a {})",
                    update.version,
                    upgrade::current()
                ),
                theme::section_title(),
            ),
            match &self.result {
                Some(Ok(())) => Line::styled(
                    format!(
                        " Instalada a {}. Feche e abra o terra para usar a versão nova.",
                        update.version
                    ),
                    theme::muted(),
                ),
                Some(Err(err)) => {
                    Line::styled(format!(" A instalação falhou: {err}"), theme::error())
                }
                None => Line::styled(" Enter instala a versão nova.", theme::muted()),
            },
            Line::raw(""),
        ];
        lines.extend(
            update
                .notes
                .lines()
                .map(|line| Line::styled(line.to_string(), theme::text())),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(Block::new().padding(Padding::new(1, 2, 1, 0))),
            area,
        );
    }
}
