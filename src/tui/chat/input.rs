//! Caixa de input do chat: edição estilo Vim (NORMAL/INSERT), cursor em barra
//! piscando, sobrescrever com a tecla Insert, seleção com o mouse, várias
//! linhas (Shift+Enter, Alt+Enter, Ctrl+J ou `\` + Enter), menções de
//! arquivo com `@` ([`super::mention`]), colar texto (`Ctrl+V` ou o colar do
//! terminal) e as mensagens já enviadas com ↑/↓.

use std::cell::Cell;
use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use unicode_width::UnicodeWidthChar;

use super::commands;
use super::mention::{self, Mention, Picker};
use crate::agent::CommandInfo;
use crate::files::{FileEntry, FileIndex};
use crate::platform::clipboard;
use crate::theme::{self, Variant};

/// Altura da caixa: bordas e três linhas de texto.
pub const HEIGHT: u16 = 5;
/// Ticks entre cada troca do cursor piscante (com tick de 100 ms, ~500 ms).
const BLINK_TICKS: u32 = 5;
const PLACEHOLDER: &str = "Pressione i para escrever";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// O campo não recebe texto.
    #[default]
    Normal,
    /// Digitando (entra com `i` ou `a`, sai com `Esc`).
    Insert,
}

/// O que a caixa fez com uma tecla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Não é com ela (ex.: Tab ou Esc no modo NORMAL).
    Ignored,
    Handled,
    /// `Enter`: enviar o texto.
    Submit,
    /// Um comando escolhido na lista da `/` (sem a `/`), para o chat executar.
    Command(String),
}

/// Onde a caixa foi desenhada no último frame; o mouse usa isso para achar o
/// caractere clicado.
#[derive(Debug, Default, Clone, Copy)]
struct View {
    /// A caixa inteira, com bordas: um clique nela começa a escrever.
    frame: Rect,
    /// Só a área do texto.
    text: Rect,
    /// Primeira linha do texto visível.
    scroll: usize,
}

pub struct InputBox {
    mode: InputMode,
    buffer: String,
    /// Tecla Insert: o texto digitado substitui o que está à frente do cursor.
    overwrite: bool,
    /// Caps Lock ligado (lido do sistema a cada tick).
    caps_lock: bool,
    /// Posição do cursor em caracteres (não em bytes). Com seleção, é a ponta
    /// que se move.
    cursor: usize,
    /// Outra ponta da seleção feita com o mouse.
    anchor: Option<usize>,
    /// Botão esquerdo pressionado sobre a caixa (arrastando).
    selecting: bool,
    view: Cell<View>,
    cursor_visible: bool,
    /// Ticks desde a última troca do cursor piscante.
    blink_ticks: u32,
    /// As menções (arquivos e skills); no texto, cada uma é um
    /// [`mention::token`].
    mentions: Vec<Mention>,
    /// Lista de arquivos aberta pelo `@`.
    picker: Option<Picker>,
    /// Arquivos do diretório, observados a partir do primeiro `@`.
    files: Option<FileIndex>,
    /// Lista de comandos aberta pela `/` no começo do texto.
    command_picker: Option<Picker>,
    /// Comandos do agente; `None` enquanto carregam.
    commands: Option<Vec<CommandInfo>>,
    /// Mensagens enviadas nesta execução, a mais antiga primeiro.
    sent: Vec<Draft>,
    /// A mensagem enviada mostrada com ↑/↓ e o texto que estava sendo
    /// escrito antes, para voltar a ele.
    browsing: Option<(usize, Draft)>,
}

/// O texto da caixa com as menções dele.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Draft {
    buffer: String,
    mentions: Vec<Mention>,
}

impl Default for InputBox {
    fn default() -> Self {
        Self {
            mode: InputMode::default(),
            buffer: String::new(),
            overwrite: false,
            caps_lock: false,
            cursor: 0,
            anchor: None,
            selecting: false,
            view: Cell::default(),
            cursor_visible: true,
            blink_ticks: 0,
            mentions: Vec::new(),
            picker: None,
            files: None,
            command_picker: None,
            commands: None,
            sent: Vec::new(),
            browsing: None,
        }
    }
}

impl InputBox {
    pub fn mode(&self) -> InputMode {
        self.mode
    }

    /// O texto para enviar: cada menção vira `@caminho` (ou `/skill`).
    pub fn text(&self) -> String {
        self.buffer
            .chars()
            .map(|c| match self.mention(c) {
                Some(mention) => mention::prompt_text(mention),
                None => c.to_string(),
            })
            .collect()
    }

    /// Guarda o texto da caixa entre as mensagens enviadas, para voltar a
    /// ele com ↑ (sem repetir a última).
    pub fn remember(&mut self) {
        let draft = self.draft();
        if !draft.buffer.trim().is_empty() && self.sent.last() != Some(&draft) {
            self.sent.push(draft);
        }
    }

    /// Esvazia a caixa (depois de enviar), sem sair do modo atual.
    pub fn clear(&mut self) {
        self.browsing = None;
        self.buffer.clear();
        self.cursor = 0;
        self.anchor = None;
        self.mentions.clear();
        self.picker = None;
        self.command_picker = None;
    }

    /// Os comandos do agente para a lista da `/` (`None`: ainda carregando).
    pub fn set_commands(&mut self, commands: Option<Vec<CommandInfo>>) {
        self.commands = commands;
    }

    /// A lista de arquivos do `@` está aberta.
    pub fn picker_open(&self) -> bool {
        self.picker.is_some() || self.command_picker.is_some()
    }

    /// A lista aberta é a de comandos (da `/`).
    pub fn command_picker_open(&self) -> bool {
        self.command_picker.is_some()
    }

    #[cfg(test)]
    pub fn set_files(&mut self, files: FileIndex) {
        self.files = Some(files);
    }

    /// A menção, se `c` for o token de uma.
    fn mention(&self, c: char) -> Option<&Mention> {
        mention::token_index(c).and_then(|i| self.mentions.get(i))
    }

    /// Colunas que `c` ocupa na tela (um badge ocupa o caminho inteiro).
    fn width_of(&self, c: char) -> usize {
        match self.mention(c) {
            Some(mention) => mention::badge_width(mention),
            None => c.width().unwrap_or(0),
        }
    }

    pub fn set_caps_lock(&mut self, on: bool) {
        self.caps_lock = on;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        match self.mode {
            InputMode::Normal => self.handle_normal(key),
            InputMode::Insert => self.handle_insert(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            // Com Caps Lock ou Shift, o `i` chega maiúsculo.
            KeyCode::Char('i' | 'I') => {}
            KeyCode::Char('a') => self.move_to(self.cursor + 1),
            _ => return Outcome::Ignored,
        }
        self.mode = InputMode::Insert;
        self.show_cursor();
        Outcome::Handled
    }

    fn handle_insert(&mut self, key: KeyEvent) -> Outcome {
        if let Some(outcome) = self.handle_command_picker(key) {
            self.show_cursor();
            return outcome;
        }
        if let Some(outcome) = self.handle_picker(key) {
            self.show_cursor();
            return outcome;
        }
        let typed = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let edited = matches!(
            key.code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
        );
        let mut outcome = Outcome::Handled;
        match key.code {
            KeyCode::Esc => {
                self.anchor = None;
                self.mode = InputMode::Normal;
            }
            KeyCode::Insert => self.overwrite = !self.overwrite,
            // Shift+Enter pula linha. Alt+Enter, Ctrl+J e `\` + Enter fazem o
            // mesmo nos terminais que não diferenciam o Shift+Enter do Enter
            // (ex.: GNOME Terminal).
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.insert_char('\n')
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => self.insert_char('\n'),
            // Nos terminais em que o Ctrl+V chega como tecla (ex.: GNOME
            // Terminal), cola a área de transferência do sistema.
            KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(text) = clipboard::paste() {
                    self.insert_paste(&text);
                }
            }
            KeyCode::Char(c) if typed => {
                // `@` abre a lista de arquivos, menos no meio de uma palavra
                // (ex.: um e-mail).
                let opens =
                    c == '@' && !self.char_before_cursor().is_some_and(char::is_alphanumeric);
                // `/` abre a lista de comandos, só no começo do texto.
                let command = c == '/' && self.cursor == 0;
                self.insert_char(c);
                if opens {
                    self.open_picker(self.cursor - 1);
                }
                if command {
                    self.command_picker = Some(Picker::new(0));
                }
            }
            KeyCode::Backspace => {
                if !self.delete_selection() && self.cursor > 0 {
                    self.cursor -= 1;
                    let at = self.byte_at(self.cursor);
                    self.buffer.remove(at);
                }
            }
            KeyCode::Delete => {
                if !self.delete_selection() && self.cursor < self.len() {
                    let at = self.byte_at(self.cursor);
                    self.buffer.remove(at);
                }
            }
            KeyCode::Up => self.up(),
            KeyCode::Down => self.down(),
            KeyCode::Left => self.move_to(self.cursor.saturating_sub(1)),
            KeyCode::Right => self.move_to(self.cursor + 1),
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(self.len()),
            KeyCode::Enter
                if self.selection().is_none() && self.char_before_cursor() == Some('\\') =>
            {
                // Troca a `\` por uma quebra de linha.
                self.cursor -= 1;
                let at = self.byte_at(self.cursor);
                self.buffer.remove(at);
                self.insert_char('\n');
            }
            KeyCode::Enter => outcome = Outcome::Submit,
            _ => {}
        }
        if edited {
            // O texto mudou: ele passa a ser o que está sendo escrito.
            self.browsing = None;
        }
        self.refresh_picker(edited);
        self.show_cursor();
        outcome
    }

    /// Cola `text` no cursor (entrando no modo INSERT). Com mais de
    /// [`mention::PASTE_LIMIT`] caracteres, vira um badge
    /// `[Pasted text #1 +12 lines]`, como no Claude Code; o texto inteiro vai
    /// na mensagem. As quebras de linha não enviam.
    pub fn paste(&mut self, text: &str) {
        self.mode = InputMode::Insert;
        self.insert_paste(text);
        self.browsing = None;
        self.refresh_picker(true);
        self.show_cursor();
    }

    fn insert_paste(&mut self, text: &str) {
        let text = clean_paste(text);
        if text.is_empty() {
            return;
        }
        if text.chars().count() <= mention::PASTE_LIMIT {
            for c in text.chars() {
                self.insert_char(c);
            }
            return;
        }
        let number = 1 + self
            .mentions
            .iter()
            .filter(|m| matches!(m, Mention::Paste { .. }))
            .count();
        let token = mention::token(self.mentions.len());
        self.mentions.push(Mention::Paste { number, text });
        self.insert_char(token);
    }

    /// ↑: sobe uma linha do texto; na primeira, mostra a mensagem enviada
    /// anterior.
    fn up(&mut self) {
        let (line, column) = self.line_and_column();
        if line > 0 {
            self.move_to_line(line - 1, column);
            return;
        }
        let index = match &self.browsing {
            Some((0, _)) => return,
            Some((index, _)) => index - 1,
            None if self.sent.is_empty() => return,
            None => self.sent.len() - 1,
        };
        let draft = match self.browsing.take() {
            Some((_, draft)) => draft,
            None => self.draft(),
        };
        self.load(self.sent[index].clone());
        self.browsing = Some((index, draft));
    }

    /// ↓: desce uma linha do texto; na última, mostra a mensagem enviada
    /// seguinte e, depois da mais nova, o texto que estava sendo escrito.
    fn down(&mut self) {
        let (line, column) = self.line_and_column();
        if line + 1 < self.buffer.split('\n').count() {
            self.move_to_line(line + 1, column);
            return;
        }
        let Some((index, draft)) = self.browsing.take() else {
            return;
        };
        match self.sent.get(index + 1) {
            Some(next) => {
                self.load(next.clone());
                self.browsing = Some((index + 1, draft));
            }
            None => self.load(draft),
        }
    }

    fn draft(&self) -> Draft {
        Draft {
            buffer: self.buffer.clone(),
            mentions: self.mentions.clone(),
        }
    }

    /// Põe `draft` na caixa, com o cursor no fim.
    fn load(&mut self, draft: Draft) {
        self.buffer = draft.buffer;
        self.mentions = draft.mentions;
        self.picker = None;
        self.command_picker = None;
        self.move_to(self.len());
    }

    /// Linha do cursor e a coluna dele nela, em caracteres.
    fn line_and_column(&self) -> (usize, usize) {
        let before: Vec<char> = self.buffer.chars().take(self.cursor).collect();
        let line = before.iter().filter(|&&c| c == '\n').count();
        let column = before.iter().rev().take_while(|&&c| c != '\n').count();
        (line, column)
    }

    /// Leva o cursor à `column` da linha `line` (ou ao fim dela, se for mais
    /// curta).
    fn move_to_line(&mut self, line: usize, column: usize) {
        let mut start = 0;
        for (i, text) in self.buffer.split('\n').enumerate() {
            let len = text.chars().count();
            if i == line {
                self.move_to(start + column.min(len));
                return;
            }
            start += len + 1;
        }
    }

    /// Teclas da lista de arquivos, enquanto ela está aberta: ↑/↓ (ou
    /// Ctrl+P/Ctrl+N) escolhem, Tab/Enter inserem e Esc fecha. As outras
    /// seguem para a edição e filtram a lista.
    fn handle_picker(&mut self, key: KeyEvent) -> Option<Outcome> {
        let mut picker = self.picker?;
        let found = self.found();
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Up => picker.up(found.len()),
            KeyCode::Down => picker.down(found.len()),
            KeyCode::Char('p') if ctrl => picker.up(found.len()),
            KeyCode::Char('n') if ctrl => picker.down(found.len()),
            KeyCode::Tab => self.accept(found.get(picker.selected)),
            KeyCode::Enter if key.modifiers.is_empty() => self.accept(found.get(picker.selected)),
            KeyCode::Esc => self.picker = None,
            _ => return None,
        }
        if let Some(open) = &mut self.picker {
            open.selected = picker.selected;
        }
        Some(Outcome::Handled)
    }

    /// Teclas da lista de comandos, como as da lista de arquivos; Tab/Enter
    /// tiram a `/consulta` do texto e devolvem o comando para o chat. Num
    /// comando do padrão do router, o Tab completa o texto (`/config `) para
    /// receber argumentos, e o Enter roda ele sem argumentos.
    fn handle_command_picker(&mut self, key: KeyEvent) -> Option<Outcome> {
        let mut picker = self.command_picker?;
        let found = self.found_commands();
        let total = found.as_ref().map_or(0, Vec::len);
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Up => picker.up(total),
            KeyCode::Down => picker.down(total),
            KeyCode::Char('p') if ctrl => picker.up(total),
            KeyCode::Char('n') if ctrl => picker.down(total),
            KeyCode::Tab | KeyCode::Enter
                if key.modifiers.is_empty() || key.code == KeyCode::Tab =>
            {
                let chosen = found.and_then(|f| f.into_iter().nth(picker.selected));
                let complete = key.code == KeyCode::Tab
                    && chosen
                        .as_ref()
                        .is_some_and(|c| !c.skill && !commands::has_interface(&c.name));
                if complete {
                    return Some(self.complete_command(chosen));
                }
                return Some(self.accept_command(chosen));
            }
            KeyCode::Esc => self.command_picker = None,
            _ => return None,
        }
        if let Some(open) = &mut self.command_picker {
            open.selected = picker.selected;
        }
        Some(Outcome::Handled)
    }

    /// Comandos que combinam com a consulta (`None`: ainda carregando).
    fn found_commands(&self) -> Option<Vec<CommandInfo>> {
        let picker = self.command_picker?;
        let commands = self.commands.as_ref()?;
        let query = self.query_of(picker);
        Some(
            commands::matches(commands, &query)
                .into_iter()
                .cloned()
                .collect(),
        )
    }

    /// Tira a `/consulta` do texto e devolve o comando; uma skill vira um
    /// badge (e um espaço), como o `@`, para escrever o pedido depois dela.
    /// Sem item, só fecha.
    fn accept_command(&mut self, command: Option<CommandInfo>) -> Outcome {
        let Some(picker) = self.command_picker.take() else {
            return Outcome::Handled;
        };
        let Some(command) = command else {
            return Outcome::Handled;
        };
        let bytes = self.byte_at(picker.start)..self.byte_at(self.cursor);
        if command.skill {
            let token = mention::token(self.mentions.len());
            self.mentions.push(Mention::Skill(command.name));
            self.buffer.replace_range(bytes, &format!("{token} "));
            self.cursor = picker.start + 2;
            self.anchor = None;
            return Outcome::Handled;
        }
        self.buffer.replace_range(bytes, "");
        self.cursor = picker.start;
        self.anchor = None;
        Outcome::Command(command.name)
    }

    /// Troca a `/consulta` pelo nome do comando e um espaço, para escrever os
    /// argumentos (ex.: `/config model=opus`). Sem item, só fecha.
    fn complete_command(&mut self, command: Option<CommandInfo>) -> Outcome {
        let Some(picker) = self.command_picker.take() else {
            return Outcome::Handled;
        };
        let Some(command) = command else {
            return Outcome::Handled;
        };
        let bytes = self.byte_at(picker.start)..self.byte_at(self.cursor);
        let text = format!("/{} ", command.name);
        self.buffer.replace_range(bytes, &text);
        self.cursor = picker.start + text.chars().count();
        self.anchor = None;
        Outcome::Handled
    }

    /// Abre a lista com o `@` na posição `start`. Os arquivos começam a ser
    /// observados no primeiro `@`.
    fn open_picker(&mut self, start: usize) {
        self.picker = Some(Picker::new(start));
        if self.files.is_none() {
            self.files = std::env::current_dir().ok().map(FileIndex::watch);
        }
    }

    /// Fecha a lista se o `@` sumiu, o cursor saiu da consulta ou ela ganhou
    /// um espaço. Uma edição volta a seleção para o primeiro item.
    /// Vale para as duas listas (`@` e `/`).
    fn refresh_picker(&mut self, edited: bool) {
        for (slot, trigger) in [(self.picker, '@'), (self.command_picker, '/')] {
            let Some(mut picker) = slot else {
                continue;
            };
            let refreshed = if self.picker_valid(picker, trigger) {
                if edited {
                    picker.selected = 0;
                }
                Some(picker)
            } else {
                None
            };
            match trigger {
                '@' => self.picker = refreshed,
                _ => self.command_picker = refreshed,
            }
        }
    }

    /// O `trigger` (`@` ou `/`) ainda está lá, o cursor está depois dele e a
    /// consulta não tem espaço.
    fn picker_valid(&self, picker: Picker, trigger: char) -> bool {
        let start = picker.start;
        self.cursor > start
            && self.buffer.chars().nth(start) == Some(trigger)
            && !self
                .buffer
                .chars()
                .skip(start + 1)
                .take(self.cursor - start - 1)
                .any(char::is_whitespace)
    }

    /// O que vem depois do `@` até o cursor.
    fn query(&self) -> Option<String> {
        self.picker.map(|picker| self.query_of(picker))
    }

    /// O que vem depois do caractere que abriu a lista até o cursor.
    fn query_of(&self, picker: Picker) -> String {
        self.buffer
            .chars()
            .skip(picker.start + 1)
            .take(self.cursor.saturating_sub(picker.start + 1))
            .collect()
    }

    /// Arquivos e pastas que combinam com a consulta, já com a lista atual.
    fn found(&self) -> Vec<FileEntry> {
        let (Some(files), Some(query)) = (&self.files, self.query()) else {
            return Vec::new();
        };
        let entries = files.snapshot();
        mention::matches(&entries, &query)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Troca `@consulta` pelo badge de `entry` (e um espaço). Sem item, só
    /// fecha a lista.
    fn accept(&mut self, entry: Option<&FileEntry>) {
        let picker = self.picker.take();
        let (Some(picker), Some(entry)) = (picker, entry) else {
            return;
        };
        let bytes = self.byte_at(picker.start)..self.byte_at(self.cursor);
        let token = mention::token(self.mentions.len());
        self.mentions.push(Mention::File(entry.path.clone()));
        self.buffer.replace_range(bytes, &format!("{token} "));
        self.cursor = picker.start + 2;
        self.anchor = None;
    }

    /// Clique na caixa posiciona o cursor (e começa a escrever); arrastar
    /// seleciona o texto. Retorna `false` se o evento não era com ela.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                if !self.view.get().frame.contains(at) {
                    return false;
                }
                let pos = self.position_at(mouse.column, mouse.row);
                self.mode = InputMode::Insert;
                self.cursor = pos;
                self.anchor = Some(pos);
                self.selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selecting => {
                self.cursor = self.position_at(mouse.column, mouse.row);
            }
            MouseEventKind::Up(MouseButton::Left) if self.selecting => {
                self.selecting = false;
                if self.selection().is_none() {
                    self.anchor = None;
                }
            }
            _ => return false,
        }
        self.show_cursor();
        true
    }

    /// Faz o cursor piscar; chamado a cada tick do loop.
    pub fn tick(&mut self) {
        self.blink_ticks += 1;
        if self.blink_ticks >= BLINK_TICKS {
            self.blink_ticks = 0;
            self.cursor_visible = !self.cursor_visible;
        }
    }

    /// Trecho selecionado, em caracteres; `None` sem seleção ou se vazia.
    fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    /// Insere `c` no cursor. Com seleção, substitui o trecho selecionado; no
    /// modo sobrescrever, substitui o caractere à frente (menos a quebra de
    /// linha, para não juntar linhas).
    fn insert_char(&mut self, c: char) {
        if !self.delete_selection()
            && self.overwrite
            && c != '\n'
            && self
                .buffer
                .chars()
                .nth(self.cursor)
                .is_some_and(|next| next != '\n')
        {
            let at = self.byte_at(self.cursor);
            self.buffer.remove(at);
        }
        let at = self.byte_at(self.cursor);
        self.buffer.insert(at, c);
        self.cursor += 1;
    }

    fn char_before_cursor(&self) -> Option<char> {
        let pos = self.cursor.checked_sub(1)?;
        self.buffer.chars().nth(pos)
    }

    /// Apaga o trecho selecionado. Retorna `false` se não havia seleção.
    fn delete_selection(&mut self) -> bool {
        let selection = self.selection();
        self.anchor = None;
        let Some(range) = selection else {
            return false;
        };
        let bytes = self.byte_at(range.start)..self.byte_at(range.end);
        self.buffer.replace_range(bytes, "");
        self.cursor = range.start;
        true
    }

    /// Move o cursor (limitado ao texto) e desfaz a seleção.
    fn move_to(&mut self, pos: usize) {
        self.anchor = None;
        self.cursor = pos.min(self.len());
    }

    /// Posição no texto do caractere na coluna/linha da tela. Fora do texto,
    /// fica no começo ou no fim da linha mais próxima.
    fn position_at(&self, column: u16, row: u16) -> usize {
        let view = self.view.get();
        let rows = wrap_rows(&self.buffer, view.text.width, |c| self.width_of(c));
        let y = (row.saturating_sub(view.text.y) as usize + view.scroll).min(rows.len() - 1);
        let x = column.saturating_sub(view.text.x) as usize;
        let range = rows[y].clone();
        let mut col = 0;
        for (i, c) in self
            .buffer
            .chars()
            .enumerate()
            .take(range.end)
            .skip(range.start)
        {
            col += self.width_of(c);
            if x < col {
                return i;
            }
        }
        range.end
    }

    /// Mantém o cursor aceso enquanto o usuário digita.
    fn show_cursor(&mut self) {
        self.cursor_visible = true;
        self.blink_ticks = 0;
    }

    fn len(&self) -> usize {
        self.buffer.chars().count()
    }

    fn byte_at(&self, pos: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(pos)
            .map_or(self.buffer.len(), |(i, _)| i)
    }

    /// Desenha a caixa com os títulos de cima (esquerda e direita) e, na
    /// borda de baixo, os indicadores de Caps e Insert.
    pub fn draw(&self, frame: &mut Frame, area: Rect, left: Line<'static>, right: Line<'static>) {
        let inserting = self.mode == InputMode::Insert;
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::input_border(inserting))
            .title_top(left.left_aligned())
            .title_top(right.right_aligned())
            .title_bottom(self.lock_indicators().right_aligned())
            .padding(Padding::horizontal(1));
        let text_area = block.inner(area);
        frame.render_widget(block, area);

        // As linhas são quebradas aqui (e não pelo Paragraph) para o mouse
        // saber exatamente em que coluna cada caractere ficou.
        let rows = wrap_rows(&self.buffer, text_area.width, |c| self.width_of(c));
        let (cursor_row, cursor_col) = self.locate(&rows, text_area.width, self.cursor);
        let height = text_area.height as usize;
        let scroll = cursor_row.saturating_sub(height.saturating_sub(1));
        self.view.set(View {
            frame: area,
            text: text_area,
            scroll,
        });

        let lines = if !inserting && self.buffer.is_empty() {
            vec![Line::styled(PLACEHOLDER, theme::muted())]
        } else {
            rows.iter()
                .skip(scroll)
                .take(height)
                .map(|row| self.row_line(row.clone()))
                .collect()
        };
        frame.render_widget(Paragraph::new(lines), text_area);

        // O cursor é o do próprio terminal, em barra (`|`): fica entre dois
        // caracteres sem ocupar uma coluna. Piscar é mostrar ou esconder.
        if inserting && self.cursor_visible && cursor_row - scroll < height {
            frame.set_cursor_position((
                text_area.x + cursor_col as u16,
                text_area.y + (cursor_row - scroll) as u16,
            ));
        }

        if let Some(picker) = self.command_picker {
            let found = self.found_commands();
            let refs: Option<Vec<&CommandInfo>> = found.as_ref().map(|f| f.iter().collect());
            let selected = picker
                .selected
                .min(found.as_ref().map_or(0, |f| f.len().saturating_sub(1)));
            commands::draw(frame, area, refs.as_deref(), selected);
        }

        if let Some(picker) = self.picker {
            let found = self.found();
            let selected = picker.selected.min(found.len().saturating_sub(1));
            let refs: Vec<&FileEntry> = found.iter().collect();
            mention::draw(frame, area, &refs, selected);
        }
    }

    /// Linha e coluna (na tela) da posição `pos` do texto.
    fn locate(&self, rows: &[Range<usize>], width: u16, pos: usize) -> (usize, usize) {
        let row = rows.iter().rposition(|r| r.start <= pos).unwrap_or(0);
        let col: usize = self
            .buffer
            .chars()
            .take(pos)
            .skip(rows[row].start)
            .map(|c| self.width_of(c))
            .sum();
        // No fim de uma linha cheia, o cursor desce para a próxima.
        if col >= width as usize && width > 0 {
            (row + 1, 0)
        } else {
            (row, col)
        }
    }

    /// Uma linha do texto, com o trecho selecionado destacado e as menções
    /// como badges.
    fn row_line(&self, row: Range<usize>) -> Line<'static> {
        let selection = self.selection().unwrap_or(0..0);
        let mut spans = Vec::new();
        let mut run = String::new();
        let mut run_selected = false;
        for (i, c) in self
            .buffer
            .chars()
            .enumerate()
            .take(row.end)
            .skip(row.start)
        {
            let selected = selection.contains(&i);
            if let Some(mention) = self.mention(c) {
                push_run(&mut spans, &mut run, run_selected);
                spans.push(mention::badge_span(mention, selected));
                continue;
            }
            if selected != run_selected {
                push_run(&mut spans, &mut run, run_selected);
                run_selected = selected;
            }
            run.push(c);
        }
        push_run(&mut spans, &mut run, run_selected);
        Line::from(spans)
    }

    /// "● Caps ● Insert" na borda de baixo, só com o que estiver ligado.
    fn lock_indicators(&self) -> Line<'static> {
        let active: Vec<&str> = [("Caps", self.caps_lock), ("Insert", self.overwrite)]
            .into_iter()
            .filter_map(|(label, on)| on.then_some(label))
            .collect();
        if active.is_empty() {
            return Line::default();
        }
        let mut spans = vec![Span::raw(" ")];
        for label in active {
            spans.push(Span::styled("● ", theme::variant(Variant::Success)));
            spans.push(Span::styled(format!("{label} "), theme::text()));
        }
        Line::from(spans)
    }
}

/// O texto colado como vai para a caixa: quebras de linha do Windows e do
/// Mac antigo viram `\n`, o Tab vira quatro espaços (ele não tem largura na
/// tela) e os outros caracteres de controle somem.
fn clean_paste(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
        .chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .filter(|&c| mention::token_index(c).is_none())
        .collect()
}

/// Fecha um trecho de texto corrido como um span (destacado ou não).
fn push_run(spans: &mut Vec<Span<'static>>, run: &mut String, selected: bool) {
    if !run.is_empty() {
        let style = if selected {
            theme::selection()
        } else {
            theme::text()
        };
        spans.push(Span::styled(std::mem::take(run), style));
    }
}

/// Quebra o texto em linhas de até `width` colunas, por caractere, e em cada
/// `\n`. Devolve o trecho (em caracteres) de cada linha, sem o `\n`; sempre há
/// pelo menos uma. `width_of` diz quantas colunas cada caractere ocupa.
fn wrap_rows(text: &str, width: u16, width_of: impl Fn(char) -> usize) -> Vec<Range<usize>> {
    let width = width.max(1) as usize;
    let mut rows = Vec::new();
    let (mut start, mut col, mut count) = (0, 0, 0);
    for (i, c) in text.chars().enumerate() {
        count = i + 1;
        if c == '\n' {
            rows.push(start..i);
            start = i + 1;
            col = 0;
            continue;
        }
        let w = width_of(c);
        if col + w > width && col > 0 {
            rows.push(start..i);
            start = i;
            col = 0;
        }
        col += w;
    }
    rows.push(start..count);
    rows
}

#[cfg(test)]
mod tests;
