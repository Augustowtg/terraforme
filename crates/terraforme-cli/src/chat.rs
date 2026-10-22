//! Chat do modo Middleware (experimental, `[tui] middleware`): a instância
//! roda o agente sem a TUI dele ([`terraforme_core::chat`]) e a conversa é
//! desenhada aqui — as mensagens do agente em cima (com o Markdown e o código
//! coloridos, [`crate::markdown`]), a caixa de entrada embaixo
//! (`docs/tui/agente/agente.feature.md`). A caixa completa os comandos `/`
//! do agente e os arquivos da pasta com `@`, e os pedidos de permissão das
//! ferramentas aparecem no lugar dela.
//!
//! Agentes cujo protocolo abre uma sessão (o `opencode acp`) só aceitam
//! mensagem depois dela; o que o usuário mandar antes fica guardado.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use terraforme_core::agent::Adapter;
use terraforme_core::chat::{Answer, ChatCtx, ChatEvent, Permission, SlashCommand};
use unicode_width::UnicodeWidthChar;

use crate::markdown::{self, palette};

/// Linhas de texto visíveis na caixa de entrada; além disso ela rola.
const INPUT_LINES: usize = 6;
/// Linhas do erro (stderr) guardadas para mostrar se o agente sair.
const STDERR_LINES: usize = 20;
/// Itens visíveis na lista de sugestões (`/` e `@`).
const MENU_ITEMS: usize = 8;
/// Arquivos lidos da pasta para o `@` (o resto fica de fora).
const MAX_FILES: usize = 20_000;

/// Um item da conversa.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    User(String),
    Agent(String),
    Tool {
        /// Id da chamada quando o agente avisa mais de uma vez ([`ChatEvent::Tool`]).
        id: String,
        name: String,
        detail: String,
    },
    ToolResult {
        error: bool,
        text: String,
    },
    /// Permissão respondida: ferramenta, resumo e se foi liberada.
    Asked {
        tool: String,
        allowed: bool,
    },
    Note(String),
    Error(String),
}

/// O que a lista de sugestões completa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Command,
    File,
}

/// Sugestões para o texto antes do cursor: o que trocar (do caractere
/// `start` até o cursor) e os itens (texto, descrição).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Completion {
    kind: Kind,
    start: usize,
    items: Vec<(String, String)>,
}

/// Linhas desenhadas de um item: largura, tamanho do texto e as linhas.
type Cached = (usize, usize, Vec<Line<'static>>);

/// Conversa de uma instância no modo Middleware.
pub struct Chat {
    /// Nome do agente, no cabeçalho das respostas.
    name: String,
    adapter: &'static dyn Adapter,
    /// Pasta do agente, onde o `@` procura os arquivos.
    cwd: PathBuf,
    /// Modelo em uso, recebido no evento Init e mostrado no título da caixa.
    model: String,
    /// Nível de esforço (low/medium/high/xhigh/max/auto) ou vazio se não definido.
    effort: String,
    /// Modo do Claude (padrão, plan, auto…), mostrado no rodapé.
    mode: String,
    entries: Vec<Entry>,
    /// Linhas já desenhadas de cada item, pela largura e pelo tamanho do
    /// texto (o Markdown e as cores do código custam a cada desenho).
    cache: RefCell<Vec<Option<Cached>>>,
    /// Texto sendo digitado e a posição do cursor nele (em caracteres).
    input: String,
    cursor: usize,
    /// Linhas roladas para trás na conversa (0 = o fim).
    scroll: usize,
    /// Quanto dá para rolar, visto no último desenho.
    max_scroll: Cell<usize>,
    /// Esperando a resposta do agente.
    busy: bool,
    /// Saída do agente que ainda não fechou uma linha.
    partial: Vec<u8>,
    /// Últimas linhas do stderr.
    stderr: Vec<String>,
    /// Linhas a mandar ao agente ([`Chat::outbox`]).
    outbox: Vec<String>,
    /// Comandos `/` do agente.
    commands: Vec<SlashCommand>,
    /// Arquivos da pasta (relativos; pastas com `/` no fim), lidos no
    /// primeiro `@`.
    files: OnceCell<Vec<String>>,
    /// Item escolhido na lista de sugestões.
    pick: usize,
    /// `Esc` fechou a lista; volta a abrir quando o texto muda.
    dismissed: bool,
    /// Pedidos de permissão esperando o usuário (o primeiro aparece).
    permissions: VecDeque<Permission>,
    /// Opção escolhida no pedido de permissão.
    choice: usize,
    /// Sessão aberta pelo agente ([`ChatEvent::Session`]), usada nos pedidos
    /// seguintes quando o protocolo tem uma.
    session: Option<String>,
    /// Mensagens mandadas antes de a sessão abrir ([`Chat::message`]).
    waiting: Vec<String>,
    /// Bloco de texto fechado pelo fim de um turno: o texto do próximo abre
    /// outro bloco (os protocolos sem `TextStart` marcam o fim só aqui).
    closed: bool,
    /// Mensagens já enviadas, para navegar com ↑/↓.
    sent: Vec<String>,
    /// Posição no histórico (None = editando rascunho novo).
    hist_pos: Option<usize>,
    /// Rascunho salvo ao entrar no histórico.
    draft: String,
}

impl Chat {
    pub fn new(name: &str, adapter: &'static dyn Adapter, cwd: &Path, effort: &str) -> Chat {
        let ctx = ChatCtx {
            cwd: cwd.to_path_buf(),
            session: None,
        };
        Chat {
            name: name.to_string(),
            adapter,
            cwd: cwd.to_path_buf(),
            model: String::new(),
            effort: effort.to_string(),
            mode: "padrão".to_string(),
            entries: vec![Entry::Note(
                "Middleware (experimental): o agente roda sem a TUI dele e a conversa \
                 aparece aqui. / lista os comandos, @ os arquivos."
                    .into(),
            )],
            cache: RefCell::default(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            max_scroll: Cell::new(0),
            busy: false,
            partial: Vec::new(),
            stderr: Vec::new(),
            outbox: adapter.chat_start(&ctx),
            commands: Vec::new(),
            files: OnceCell::new(),
            pick: 0,
            dismissed: false,
            permissions: VecDeque::new(),
            choice: 0,
            session: None,
            waiting: Vec::new(),
            closed: false,
            sent: Vec::new(),
            hist_pos: None,
            draft: String::new(),
        }
    }

    /// Contexto que os [`Adapter::chat_*`] pedem.
    fn ctx(&self) -> ChatCtx {
        ChatCtx {
            cwd: self.cwd.clone(),
            session: self.session.clone(),
        }
    }

    /// Linhas a mandar ao agente (cada uma sem o `\n`), esvaziando a fila.
    pub fn outbox(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outbox)
    }

    /// Saída do agente: junta os pedaços em linhas e aplica os eventos.
    /// Devolve `true` quando o agente passa a esperar o usuário (a resposta
    /// terminou ou chegou um pedido de permissão).
    pub fn output(&mut self, bytes: &[u8]) -> bool {
        self.partial.extend_from_slice(bytes);
        let mut waiting = false;
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let ctx = self.ctx();
            for event in self.adapter.chat_events(line.trim_end(), &ctx) {
                waiting |= matches!(event, ChatEvent::Done { .. } | ChatEvent::Permission(_));
                self.apply(event);
            }
        }
        waiting
    }

    fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Init { model, commands } => {
                let note = format!("sessão iniciada · modelo {model}");
                if !model.is_empty() && !self.entries.contains(&Entry::Note(note.clone())) {
                    self.entries.push(Entry::Note(note));
                }
                if !model.is_empty() && self.model.is_empty() {
                    self.model = model.clone();
                }
                // Sem a resposta do `initialize`, ficam só os nomes.
                if self.commands.is_empty() {
                    self.commands = commands
                        .into_iter()
                        .map(|name| SlashCommand {
                            name,
                            description: String::new(),
                            hint: String::new(),
                        })
                        .collect();
                }
            }
            ChatEvent::Commands(commands) => self.commands = commands,
            ChatEvent::Session(id) => {
                self.session = Some(id);
                // O que o usuário mandou antes de a sessão abrir.
                for text in std::mem::take(&mut self.waiting) {
                    self.queue(&text);
                }
            }
            ChatEvent::Permission(permission) => {
                if self.permissions.is_empty() {
                    self.choice = 0;
                }
                self.permissions.push_back(permission);
                self.scroll = 0;
            }
            ChatEvent::Reply(line) => self.outbox.push(line),
            ChatEvent::TextStart => {
                self.closed = false;
                self.entries.push(Entry::Agent(String::new()));
            }
            ChatEvent::Text(text) => {
                // Sem o `TextStart`, o começo do bloco é o fim do anterior.
                if self.closed {
                    self.closed = false;
                    self.entries.push(Entry::Agent(String::new()));
                }
                match self.entries.last_mut() {
                    Some(Entry::Agent(current)) => current.push_str(&text),
                    _ => self.entries.push(Entry::Agent(text)),
                }
            }
            ChatEvent::Tool { id, name, detail } => {
                // Os avisos do meio da chamada repetem o começo sem o nome,
                // que fica como está.
                let at = if id.is_empty() {
                    None
                } else {
                    self.entries
                        .iter()
                        .rposition(|entry| matches!(entry, Entry::Tool { id: at, .. } if *at == id))
                };
                match at {
                    Some(index) => {
                        if let Entry::Tool {
                            name: kept,
                            detail: current,
                            ..
                        } = &mut self.entries[index]
                        {
                            if !name.is_empty() {
                                *kept = name;
                            }
                            if !detail.is_empty() {
                                *current = detail;
                            }
                        }
                    }
                    None => self.entries.push(Entry::Tool { id, name, detail }),
                }
            }
            ChatEvent::ToolResult { error, text } => {
                self.entries.push(Entry::ToolResult { error, text })
            }
            ChatEvent::Done { error, denied } => {
                self.busy = false;
                self.closed = true;
                // Bloco de texto aberto que não recebeu nada.
                if self.entries.last() == Some(&Entry::Agent(String::new())) {
                    self.entries.pop();
                }
                if let Some(error) = error {
                    // A mensagem sintética já trouxe o mesmo texto do erro.
                    if self.entries.last() == Some(&Entry::Agent(error.clone())) {
                        self.entries.pop();
                    }
                    self.entries.push(Entry::Error(error));
                }
                if !denied.is_empty() {
                    self.entries.push(Entry::Error(format!(
                        "permissão negada: {}",
                        denied.join(", ")
                    )));
                }
            }
        }
    }

    /// Uma linha do stderr do agente.
    pub fn stderr(&mut self, line: String) {
        if self.stderr.len() == STDERR_LINES {
            self.stderr.remove(0);
        }
        self.stderr.push(line);
    }

    /// O que o agente escreveu no stderr, para o aviso quando ele sai.
    pub fn errors(&self) -> String {
        self.stderr.join("\n")
    }

    /// Registra a mensagem do usuário na conversa e a põe na fila para o
    /// agente.
    pub fn message(&mut self, text: &str) {
        self.entries.push(Entry::User(text.to_string()));
        self.busy = true;
        self.scroll = 0;
        if let Some(level) = text
            .strip_prefix("/effort")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.effort = level.to_string();
        }
        // Agentes que abrem sessão só a aceitam depois de pronta.
        if self.session.is_none() && self.adapter.chat_waits_session() {
            self.waiting.push(text.to_string());
        } else {
            self.queue(text);
        }
    }

    /// Põe a mensagem na fila do agente, no formato do protocolo dele.
    fn queue(&mut self, text: &str) {
        let ctx = self.ctx();
        self.outbox.push(self.adapter.chat_input(text, &ctx));
    }

    /// Modo do Claude em uso (padrão, plan, auto…), para o rodapé da TUI.
    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Cola o texto na caixa de entrada.
    pub fn paste(&mut self, text: &str) {
        if !self.permissions.is_empty() {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.insert(&text);
    }

    fn insert(&mut self, text: &str) {
        let at = self.byte(self.cursor);
        self.input.insert_str(at, text);
        self.cursor += text.chars().count();
        self.edited();
    }

    /// O texto mudou: a lista de sugestões volta ao primeiro item.
    fn edited(&mut self) {
        self.pick = 0;
        self.dismissed = false;
    }

    /// Posição em bytes do caractere `index` da entrada.
    fn byte(&self, index: usize) -> usize {
        self.input
            .char_indices()
            .nth(index)
            .map_or(self.input.len(), |(i, _)| i)
    }

    /// Sugestões para o texto antes do cursor: os comandos quando a
    /// mensagem começa com `/`, os arquivos depois de um `@`.
    fn completion(&self) -> Option<Completion> {
        if self.dismissed || !self.permissions.is_empty() {
            return None;
        }
        let before: String = self.input.chars().take(self.cursor).collect();
        if let Some(query) = before.strip_prefix('/')
            && !query.contains(char::is_whitespace)
        {
            let items = rank_commands(&self.commands, query);
            return (!items.is_empty()).then_some(Completion {
                kind: Kind::Command,
                start: 0,
                items,
            });
        }
        let word = before.rfind(char::is_whitespace).map_or(0, |i| {
            i + before[i..].chars().next().map_or(1, char::len_utf8)
        });
        let query = before[word..].strip_prefix('@')?;
        let files = self.files.get_or_init(|| list_files(&self.cwd));
        let items: Vec<_> = rank_files(files, query)
            .into_iter()
            .map(|f| (f, String::new()))
            .collect();
        (!items.is_empty()).then_some(Completion {
            kind: Kind::File,
            start: before[..word].chars().count(),
            items,
        })
    }

    /// Troca o que foi digitado pela sugestão escolhida. Devolve o comando
    /// escolhido quando ele não leva argumentos (o `Enter` já o manda).
    fn accept(&mut self, completion: &Completion) -> bool {
        let Some((item, _)) = completion
            .items
            .get(self.pick.min(completion.items.len() - 1))
        else {
            return false;
        };
        let (text, ready) = match completion.kind {
            Kind::Command => {
                let hint = self
                    .commands
                    .iter()
                    .find(|c| &c.name == item)
                    .is_some_and(|c| !c.hint.is_empty());
                (format!("/{item} "), !hint)
            }
            // Pasta: segue sem espaço para continuar o caminho.
            Kind::File if item.ends_with('/') => (format!("@{item}"), false),
            Kind::File => (format!("@{item} "), false),
        };
        let from = self.byte(completion.start);
        let to = self.byte(self.cursor);
        self.input.replace_range(from..to, &text);
        self.cursor = completion.start + text.chars().count();
        self.edited();
        ready
    }

    /// Manda o que está na caixa de entrada.
    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();
        self.cursor = 0;
        self.hist_pos = None;
        self.draft.clear();
        self.edited();
        self.sent.push(text.clone());
        self.message(&text);
    }

    /// Navega ao histórico de mensagens enviadas (↑).
    fn history_up(&mut self) {
        if self.sent.is_empty() {
            return;
        }
        match self.hist_pos {
            None => {
                self.draft = self.input.clone();
                self.hist_pos = Some(self.sent.len() - 1);
            }
            Some(0) => return,
            Some(pos) => self.hist_pos = Some(pos - 1),
        }
        let msg = self.sent[self.hist_pos.unwrap()].clone();
        self.input = msg;
        self.cursor = self.input.chars().count();
        self.edited();
    }

    /// Avança no histórico (↓); depois da última mensagem, volta ao rascunho.
    fn history_down(&mut self) {
        let Some(pos) = self.hist_pos else {
            return;
        };
        if pos + 1 < self.sent.len() {
            self.hist_pos = Some(pos + 1);
            let msg = self.sent[pos + 1].clone();
            self.input = msg;
        } else {
            self.input = self.draft.clone();
            self.hist_pos = None;
            self.draft.clear();
        }
        self.cursor = self.input.chars().count();
        self.edited();
    }

    /// `Shift+Tab`: cicla entre os modos do agente (padrão → plan → auto).
    fn cycle_mode(&mut self) {
        const MODES: &[&str] = &["padrão", "plan", "auto"];
        const CMDS: &[&str] = &["", "/plan", "/auto"];
        let current = MODES.iter().position(|m| *m == self.mode).unwrap_or(0);
        let next = (current + 1) % MODES.len();
        // Ao voltar ao padrão reenvia o comando do modo atual para desativá-lo.
        let cmd = if CMDS[next].is_empty() {
            CMDS[current]
        } else {
            CMDS[next]
        };
        if !cmd.is_empty() {
            self.message(cmd);
        }
        self.mode = MODES[next].to_string();
    }

    /// Opções do pedido de permissão aberto.
    fn answers(permission: &Permission) -> Vec<(Answer, String)> {
        let mut answers = vec![(Answer::Once, "Sim".to_string())];
        if !permission.always.is_null() {
            answers.push((
                Answer::Always,
                format!("Sim, e não perguntar de novo ({})", permission.tool),
            ));
        }
        answers.push((Answer::Deny, "Não".to_string()));
        answers
    }

    /// Responde o primeiro pedido de permissão.
    fn answer(&mut self, answer: Answer) {
        let Some(permission) = self.permissions.pop_front() else {
            return;
        };
        self.choice = 0;
        let ctx = self.ctx();
        if let Some(line) = self.adapter.chat_permission(&permission, answer, &ctx) {
            self.outbox.push(line);
        }
        self.entries.push(Entry::Asked {
            tool: format!("{}  {}", permission.tool, permission.detail),
            allowed: answer != Answer::Deny,
        });
    }

    /// Tecla com um pedido de permissão aberto: `↑↓`/números escolhem,
    /// `Enter` confirma, `Esc` nega.
    fn permission_key(&mut self, key: KeyEvent) {
        let Some(permission) = self.permissions.front() else {
            return;
        };
        let answers = Chat::answers(permission);
        match key.code {
            KeyCode::Up => self.choice = self.choice.saturating_sub(1),
            KeyCode::Down => self.choice = (self.choice + 1).min(answers.len() - 1),
            KeyCode::Enter => self.answer(answers[self.choice.min(answers.len() - 1)].0),
            KeyCode::Esc => self.answer(Answer::Deny),
            KeyCode::Char(c) => match c.to_digit(10) {
                Some(n) if (1..=answers.len()).contains(&(n as usize)) => {
                    self.answer(answers[n as usize - 1].0)
                }
                _ => match c.to_ascii_lowercase() {
                    's' | 'y' => self.answer(Answer::Once),
                    'n' => self.answer(Answer::Deny),
                    _ => {}
                },
            },
            KeyCode::PageUp => self.scroll_by(10),
            KeyCode::PageDown => self.scroll_by(-10),
            _ => {}
        }
    }

    /// Edita a entrada; com `Enter` põe a mensagem na fila do agente
    /// ([`Chat::outbox`]). `Alt+Enter` (ou `Shift+Enter`, com o protocolo do
    /// kitty) quebra a linha. Com a lista de sugestões aberta, `↑↓` escolhem,
    /// `Tab`/`Enter` completam e `Esc` fecha. `↑↓` sem lista navegam o
    /// histórico de mensagens enviadas; rolar a conversa fica com `PgUp`/`PgDn`.
    /// `Shift+Tab` cicla o modo do agente.
    pub fn key(&mut self, key: KeyEvent) {
        if !self.permissions.is_empty() {
            self.permission_key(key);
            return;
        }
        let len = self.input.chars().count();
        let modified = key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
        if let Some(completion) = self.completion() {
            let last = completion.items.len() - 1;
            match key.code {
                KeyCode::Up => {
                    self.pick = self.pick.min(last).checked_sub(1).unwrap_or(last);
                    return;
                }
                KeyCode::Down => {
                    self.pick = if self.pick >= last { 0 } else { self.pick + 1 };
                    return;
                }
                KeyCode::Tab => {
                    self.accept(&completion);
                    return;
                }
                KeyCode::Enter if !modified => {
                    if self.accept(&completion) {
                        self.submit();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.dismissed = true;
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter if modified => self.insert("\n"),
            KeyCode::Enter => self.submit(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor = 0;
                self.hist_pos = None;
                self.draft.clear();
                self.edited();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert(&c.to_string())
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.input.remove(at);
                self.edited();
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte(self.cursor);
                self.input.remove(at);
                self.edited();
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::PageUp => self.scroll_by(10),
            KeyCode::PageDown => self.scroll_by(-10),
            KeyCode::Up => self.history_up(),
            KeyCode::Down => self.history_down(),
            KeyCode::BackTab => self.cycle_mode(),
            _ => {}
        }
    }

    /// Rola a conversa `delta` linhas para trás (negativo: para o fim).
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll.get());
    }

    /// A conversa em texto puro (`Ctrl+Alt+C`).
    pub fn transcript(&self) -> String {
        self.lines(usize::MAX)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Linhas da conversa já quebradas na largura `width`.
    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut cache = self.cache.borrow_mut();
        cache.resize(self.entries.len(), None);
        let mut lines = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            let starts_block = matches!(entry, Entry::User(_) | Entry::Agent(_));
            let after_block = matches!(
                self.entries.get(i.wrapping_sub(1)),
                Some(Entry::User(_) | Entry::Agent(_))
            );
            if i > 0 && (starts_block || after_block) {
                lines.push(Line::default());
            }
            // Só o que ainda muda: a resposta em andamento e a chamada de
            // ferramenta que ganha o resumo no meio do caminho.
            let size = match entry {
                Entry::Agent(text) | Entry::User(text) => text.len(),
                Entry::Tool { detail, .. } => detail.len(),
                _ => 0,
            };
            match &cache[i] {
                Some((w, s, cached)) if *w == width && *s == size => {
                    lines.extend(cached.iter().cloned())
                }
                _ => {
                    let rendered = self.entry_lines(entry, width);
                    lines.extend(rendered.iter().cloned());
                    cache[i] = Some((width, size, rendered));
                }
            }
        }
        lines
    }

    fn entry_lines(&self, entry: &Entry, width: usize) -> Vec<Line<'static>> {
        let body = width.saturating_sub(2).max(1);
        let indent = |lines: Vec<Line<'static>>| -> Vec<Line<'static>> {
            lines
                .into_iter()
                .map(|mut l| {
                    l.spans.insert(0, Span::raw("  "));
                    l
                })
                .collect()
        };
        let mut lines = Vec::new();
        match entry {
            Entry::User(text) => {
                lines.push(Line::raw("› você").bold().cyan());
                lines.extend(
                    wrap(text, body)
                        .into_iter()
                        .map(|l| Line::raw(format!("  {l}"))),
                );
            }
            Entry::Agent(text) => {
                lines.push(Line::raw(format!("◆ {}", self.name)).bold().green());
                lines.extend(indent(markdown::render(text, body)));
            }
            Entry::Tool { name, detail, .. } => {
                let text = format!("⚙ {name}  {detail}");
                lines.extend(
                    wrap(&text, body)
                        .into_iter()
                        .map(|l| Line::raw(format!("  {l}")).yellow()),
                );
            }
            Entry::ToolResult { error, text } => {
                let text = format!("↳ {text}");
                let style = if *error {
                    Style::new().red()
                } else {
                    Style::new().dark_gray()
                };
                lines.extend(
                    wrap(&text, body.saturating_sub(2).max(1))
                        .into_iter()
                        .map(|l| Line::styled(format!("    {l}"), style)),
                );
            }
            Entry::Asked { tool, allowed } => {
                let (mark, style) = if *allowed {
                    ("✔ permitido", Style::new().green())
                } else {
                    ("✘ negado", Style::new().red())
                };
                lines.extend(
                    wrap(&format!("{mark}: {tool}"), body)
                        .into_iter()
                        .map(|l| Line::styled(format!("  {l}"), style)),
                );
            }
            Entry::Note(text) => lines.extend(
                wrap(&format!("· {text}"), width.max(1))
                    .into_iter()
                    .map(|l| Line::raw(l).dark_gray().italic()),
            ),
            Entry::Error(text) => lines.extend(
                wrap(&format!("✘ {text}"), width.max(1))
                    .into_iter()
                    .map(|l| Line::raw(l).red()),
            ),
        }
        lines
    }

    /// Desenha a conversa e a caixa de entrada (ou o pedido de permissão);
    /// `cursor` põe o cursor do terminal na entrada (sem janela por cima).
    pub fn render(&self, frame: &mut Frame, area: Rect, cursor: bool) {
        let inner = area.width.saturating_sub(2) as usize;
        let (input_lines, (row, col)) = input_layout(&self.input, self.cursor, inner.max(1));
        let visible = input_lines.len().clamp(1, INPUT_LINES);
        let box_height = match self.permissions.front() {
            Some(permission) => Chat::answers(permission).len() as u16 + 3,
            None => visible as u16 + 2,
        };
        let [history, input] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(box_height)]).areas(area);

        let lines = self.lines(history.width as usize);
        let height = history.height as usize;
        let max_scroll = lines.len().saturating_sub(height);
        self.max_scroll.set(max_scroll);
        let scroll = self.scroll.min(max_scroll);
        let end = lines.len() - scroll;
        let start = end.saturating_sub(height);
        frame.render_widget(Paragraph::new(lines[start..end].to_vec()), history);
        if scroll > 0 {
            let hint = format!(" histórico −{scroll} · PgDn volta ");
            let width = hint.chars().count() as u16;
            let area = Rect::new(history.right().saturating_sub(width), history.y, width, 1)
                .intersection(history);
            frame.render_widget(Span::raw(hint).black().on_yellow(), area);
        }

        if let Some(permission) = self.permissions.front() {
            self.render_permission(frame, permission, input);
            return;
        }

        let first = (row + 1).saturating_sub(visible);
        let title_text = match (self.model.is_empty(), self.effort.is_empty()) {
            (true, _) => " mensagem ".to_string(),
            (false, true) => format!(" {} ", self.model),
            (false, false) => format!(" {} · esforço {} ", self.model, self.effort),
        };
        let status = if self.busy {
            Line::from(" ⏳ trabalhando… ").yellow()
        } else {
            Line::default()
        };
        let block = Block::bordered()
            .title(Line::from(title_text).bold().cyan())
            .title_bottom(status)
            .border_style(Style::new().cyan());
        let text: Vec<Line> = input_lines[first..]
            .iter()
            .take(visible)
            .map(|l| Line::raw(l.clone()))
            .collect();
        frame.render_widget(Paragraph::new(text).block(block), input);
        if cursor {
            frame.set_cursor_position(Position::new(
                input.x + 1 + col as u16,
                input.y + 1 + (row - first) as u16,
            ));
        }
        if let Some(completion) = self.completion() {
            self.render_menu(frame, &completion, history);
        }
    }

    /// Pedido de permissão no lugar da caixa de entrada.
    fn render_permission(&self, frame: &mut Frame, permission: &Permission, area: Rect) {
        let width = area.width.saturating_sub(4) as usize;
        let mut lines = vec![Line::from(vec![
            Span::raw(" "),
            Span::raw(truncate(&permission.detail, width)).fg(palette::TEXT),
        ])];
        for (i, (_, label)) in Chat::answers(permission).iter().enumerate() {
            let text = format!(
                " {} {}. {label}",
                if i == self.choice { "›" } else { " " },
                i + 1
            );
            lines.push(if i == self.choice {
                Line::raw(text).bold().fg(palette::FUNCTION)
            } else {
                Line::raw(text)
            });
        }
        let waiting = match self.permissions.len() {
            1 => String::new(),
            n => format!(" · +{} na fila", n - 1),
        };
        let block = Block::bordered()
            .title(
                Line::from(format!(" permitir {}? ", permission.tool))
                    .bold()
                    .fg(palette::FUNCTION),
            )
            .title_bottom(
                Line::from(format!(
                    " ↑↓/1-3 escolhe · Enter confirma · Esc nega{waiting} "
                ))
                .dark_gray(),
            )
            .border_style(Style::new().fg(palette::ACCENT));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Lista de sugestões, logo acima da caixa de entrada.
    fn render_menu(&self, frame: &mut Frame, completion: &Completion, history: Rect) {
        let count = completion.items.len();
        let shown = count.min(MENU_ITEMS).min(history.height as usize);
        if shown == 0 {
            return;
        }
        let pick = self.pick.min(count - 1);
        // Janela que acompanha o item escolhido.
        let first = (pick + 1).saturating_sub(shown);
        let area = Rect::new(
            history.x,
            history.bottom() - shown as u16,
            history.width,
            shown as u16,
        );
        frame.render_widget(Clear, area);
        let name_width = completion.items[first..first + shown]
            .iter()
            .map(|(name, _)| name.chars().count() + 3)
            .max()
            .unwrap_or(0)
            .min(area.width as usize / 2);
        let prefix = match completion.kind {
            Kind::Command => "/",
            Kind::File => "@",
        };
        let lines: Vec<Line> = completion.items[first..first + shown]
            .iter()
            .enumerate()
            .map(|(i, (name, description))| {
                let selected = first + i == pick;
                let style = if selected {
                    Style::new().fg(palette::TEXT).bg(palette::ACCENT).bold()
                } else {
                    Style::new().fg(palette::TEXT).bg(palette::PANEL)
                };
                let name = truncate(&format!(" {prefix}{name}"), name_width);
                let rest = (area.width as usize).saturating_sub(name.chars().count() + 1);
                Line::from(vec![
                    Span::styled(format!("{name:<name_width$}"), style),
                    Span::styled(
                        format!(" {:<rest$}", truncate(description, rest)),
                        if selected {
                            style.not_bold()
                        } else {
                            style.fg(palette::MUTED)
                        },
                    ),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// Comandos que começam com o texto (depois os que o contêm), em ordem.
fn rank_commands(commands: &[SlashCommand], query: &str) -> Vec<(String, String)> {
    let query = query.to_lowercase();
    let mut ranked: Vec<(usize, &SlashCommand)> = commands
        .iter()
        .filter_map(|c| {
            let name = c.name.to_lowercase();
            if name.starts_with(&query) {
                Some((0, c))
            } else {
                name.contains(&query).then_some((1, c))
            }
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
    ranked
        .into_iter()
        .map(|(_, c)| {
            let description = match c.hint.as_str() {
                "" => c.description.clone(),
                hint => format!("{hint} · {}", c.description),
            };
            (c.name.clone(), description)
        })
        .collect()
}

/// Arquivos e pastas que batem com o texto: primeiro o nome que começa com
/// ele, depois o nome que o contém, depois o caminho; os mais curtos antes.
/// Sem texto, os da raiz da pasta.
fn rank_files(files: &[String], query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    let mut ranked: Vec<(usize, &String)> = files
        .iter()
        .filter_map(|path| {
            let lower = path.to_lowercase();
            let name = lower.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            let rank = if query.is_empty() {
                lower.trim_end_matches('/').matches('/').count()
            } else if name.starts_with(&query) {
                0
            } else if name.contains(&query) {
                1
            } else if lower.contains(&query) {
                2
            } else {
                return None;
            };
            Some((rank, path))
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.len().cmp(&b.1.len()))
            .then(a.1.cmp(b.1))
    });
    ranked
        .into_iter()
        .take(50)
        .map(|(_, path)| path.clone())
        .collect()
}

/// Arquivos e pastas de `cwd` (respeitando o `.gitignore`, sem os ocultos),
/// relativos, com `/` no fim das pastas.
fn list_files(cwd: &Path) -> Vec<String> {
    ignore::WalkBuilder::new(cwd)
        .max_depth(Some(12))
        .build()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path().strip_prefix(cwd).ok()?;
            let text = path.to_str()?.replace('\\', "/");
            if text.is_empty() {
                return None;
            }
            Some(if entry.file_type().is_some_and(|t| t.is_dir()) {
                format!("{text}/")
            } else {
                text
            })
        })
        .take(MAX_FILES)
        .collect()
}

/// Corta o texto em `max` caracteres, com `…` no fim.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Largura do caractere na tela (as de controle contam zero).
fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Quebra o texto em linhas de até `width` colunas, nas palavras quando dá.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_inclusive(' ') {
            let fits: usize = word.trim_end().chars().map(char_width).sum();
            if used > 0 && used + fits > width {
                out.push(std::mem::take(&mut line).trim_end().to_string());
                used = 0;
            }
            if fits <= width {
                line.push_str(word);
                used += word.chars().map(char_width).sum::<usize>();
                continue;
            }
            // Palavra maior que a linha: corta nos caracteres.
            for c in word.chars() {
                let w = char_width(c);
                if used + w > width && used > 0 {
                    out.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += w;
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// Linhas da caixa de entrada (quebradas nos caracteres, para o cursor
/// bater) e a posição (linha, coluna) do cursor nelas.
fn input_layout(input: &str, cursor: usize, width: usize) -> (Vec<String>, (usize, usize)) {
    let mut lines = vec![String::new()];
    let mut col = 0;
    let mut at = (0, 0);
    for (i, c) in input.chars().enumerate() {
        if i == cursor {
            at = (lines.len() - 1, col);
        }
        if c == '\n' {
            lines.push(String::new());
            col = 0;
            continue;
        }
        let w = char_width(c);
        if col + w > width {
            lines.push(String::new());
            col = 0;
        }
        lines.last_mut().expect("sempre há uma linha").push(c);
        col += w;
    }
    if cursor >= input.chars().count() {
        if col >= width {
            lines.push(String::new());
            col = 0;
        }
        at = (lines.len() - 1, col);
    }
    (lines, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terraforme_core::agent::Provider;

    fn chat() -> Chat {
        Chat::new(
            "claude",
            Provider::Claude.adapter().unwrap(),
            Path::new("."),
            "",
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(chat: &mut Chat, text: &str) {
        for c in text.chars() {
            chat.key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn wraps_on_words_and_cuts_long_ones() {
        assert_eq!(wrap("oi tudo bem", 7), vec!["oi tudo", "bem"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn input_cursor_follows_wraps_and_newlines() {
        assert_eq!(input_layout("", 0, 5), (vec!["".into()], (0, 0)));
        let (lines, at) = input_layout("abcdefg", 7, 5);
        assert_eq!(lines, vec!["abcde", "fg"]);
        assert_eq!(at, (1, 2));
        let (lines, at) = input_layout("ab\ncd", 3, 5);
        assert_eq!(lines, vec!["ab", "cd"]);
        assert_eq!(at, (1, 0));
    }

    #[test]
    fn starts_by_asking_the_commands() {
        let mut chat = chat();
        let start = chat.outbox();
        assert!(start.len() == 1 && start[0].contains("initialize"));
        assert!(chat.outbox().is_empty());
    }

    #[test]
    fn enter_sends_and_alt_enter_breaks_the_line() {
        let mut chat = chat();
        chat.outbox();
        typed(&mut chat, "oi");
        chat.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        chat.key(key(KeyCode::Char('x')));
        chat.key(key(KeyCode::Left));
        chat.key(key(KeyCode::Backspace));
        assert_eq!(chat.input, "oix");
        chat.key(key(KeyCode::Enter));
        let sent = chat.outbox();
        assert!(sent.len() == 1 && sent[0].contains("\"oix\""));
        assert!(chat.busy && chat.input.is_empty());
        chat.key(key(KeyCode::Enter));
        assert!(chat.outbox().is_empty());
    }

    #[test]
    fn completes_slash_commands() {
        let mut chat = chat();
        chat.outbox();
        chat.apply(ChatEvent::Commands(vec![
            SlashCommand {
                name: "compact".into(),
                description: "resume".into(),
                hint: "[foco]".into(),
            },
            SlashCommand {
                name: "clear".into(),
                description: "limpa".into(),
                hint: String::new(),
            },
        ]));
        typed(&mut chat, "/c");
        let completion = chat.completion().unwrap();
        assert_eq!(completion.items.len(), 2);
        assert_eq!(completion.items[0].0, "clear");
        // Com argumentos: completa e espera.
        chat.key(key(KeyCode::Down));
        chat.key(key(KeyCode::Enter));
        assert_eq!(chat.input, "/compact ");
        assert!(chat.outbox().is_empty());
        // Sem argumentos: o Enter já manda.
        chat.key(key(KeyCode::Char('u')).tap_ctrl());
        typed(&mut chat, "/cle");
        chat.key(key(KeyCode::Enter));
        let sent = chat.outbox();
        assert!(sent.len() == 1 && sent[0].contains("\"/clear\""));
        // Esc fecha a lista até o texto mudar.
        typed(&mut chat, "/c");
        chat.key(key(KeyCode::Esc));
        assert!(chat.completion().is_none());
        typed(&mut chat, "o");
        assert!(chat.completion().is_some());
    }

    trait Ctrl {
        fn tap_ctrl(self) -> Self;
    }

    impl Ctrl for KeyEvent {
        fn tap_ctrl(mut self) -> Self {
            self.modifiers |= KeyModifiers::CONTROL;
            self
        }
    }

    #[test]
    fn completes_files_after_at() {
        let mut chat = chat();
        chat.files = OnceCell::from(vec![
            "src/".into(),
            "src/main.rs".into(),
            "docs/guia.md".into(),
            "Cargo.toml".into(),
        ]);
        typed(&mut chat, "veja @mai");
        let completion = chat.completion().unwrap();
        assert_eq!(completion.start, 5);
        assert_eq!(completion.items[0].0, "src/main.rs");
        chat.key(key(KeyCode::Tab));
        assert_eq!(chat.input, "veja @src/main.rs ");
        // Sem texto: a raiz primeiro; pasta segue sem espaço.
        assert_eq!(
            rank_files(chat.files.get().unwrap(), "")[..2],
            ["src/".to_string(), "Cargo.toml".to_string()]
        );
        typed(&mut chat, "@sr");
        chat.key(key(KeyCode::Enter));
        assert_eq!(chat.input, "veja @src/main.rs @src/");
        assert!(chat.outbox().len() == 1, "só o initialize, nada enviado");
    }

    #[test]
    fn asks_permission_in_place_of_the_input() {
        let mut chat = chat();
        chat.outbox();
        let line = r#"{"type":"control_request","request_id":"p1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch a"},"permission_suggestions":[{"type":"addRules","rules":[],"behavior":"allow","destination":"localSettings"}]}}"#;
        assert!(chat.output(format!("{line}\n").as_bytes()));
        assert_eq!(chat.permissions.len(), 1);
        // As teclas vão para o pedido, não para a caixa.
        chat.key(key(KeyCode::Char('x')));
        assert!(chat.input.is_empty());
        chat.key(key(KeyCode::Char('2')));
        let sent = chat.outbox();
        assert!(sent.len() == 1 && sent[0].contains("updatedPermissions"));
        assert!(chat.permissions.is_empty());
        assert!(chat.transcript().contains("✔ permitido: Bash  touch a"));
        assert!(chat.output(format!("{line}\n").as_bytes()));
        chat.key(key(KeyCode::Esc));
        assert!(chat.outbox()[0].contains("deny"));
    }

    #[test]
    fn streams_the_answer_and_finishes() {
        let mut chat = chat();
        chat.message("oi");
        let lines = [
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Olá"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":", **tudo** bem?"}},"parent_tool_use_id":null}"#,
        ]
        .join("\n");
        // A última linha chega em dois pedaços.
        let (a, b) = lines.split_at(lines.len() - 10);
        assert!(!chat.output(a.as_bytes()));
        assert!(!chat.output(format!("{b}\n").as_bytes()));
        assert_eq!(
            chat.entries.last(),
            Some(&Entry::Agent("Olá, **tudo** bem?".into()))
        );
        // O desenho em cache acompanha o texto que chega.
        assert!(chat.transcript().contains("◆ claude\n  Olá, tudo bem?"));
        assert!(chat.output(b"{\"type\":\"result\",\"is_error\":false}\n"));
        assert!(!chat.busy);
    }

    #[test]
    fn shows_local_commands_once() {
        let mut chat = chat();
        chat.message("/model");
        let reply = r#"{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Current model: Opus"}]},"parent_tool_use_id":null}"#;
        chat.output(format!("{reply}\n").as_bytes());
        assert!(chat.output(b"{\"type\":\"result\",\"is_error\":false}\n"));
        assert_eq!(
            chat.entries.last(),
            Some(&Entry::Agent("Current model: Opus".into()))
        );
        // Erro: o texto da mensagem sintética vira só o erro do `result`.
        chat.message("/x");
        let reply = r#"{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Unknown command"}]},"parent_tool_use_id":null}"#;
        chat.output(format!("{reply}\n").as_bytes());
        chat.output(b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Unknown command\"}\n");
        let n = chat.entries.len();
        assert_eq!(chat.entries[n - 2], Entry::User("/x".into()));
        assert_eq!(chat.entries[n - 1], Entry::Error("Unknown command".into()));
    }

    #[test]
    fn opencode_waits_the_session_and_opens_a_block_per_turn() {
        let mut chat = Chat::new(
            "opencode",
            Provider::OpenCode.adapter().unwrap(),
            Path::new("."),
            "",
        );
        // O handshake começa pelo `initialize`.
        let start = chat.outbox();
        assert!(start.len() == 1 && start[0].contains("\"initialize\""));
        // Manda-se mensagem depois: a sessão ainda não abriu.
        chat.message("oi");
        assert!(chat.outbox().is_empty() && chat.busy);
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#;
        chat.output(format!("{init}\n").as_bytes());
        let session_new = chat.outbox();
        assert!(session_new.len() == 1 && session_new[0].contains("\"session/new\""));
        assert!(session_new[0].contains("\"cwd\""));
        let opened = r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_1","configOptions":[{"id":"model","currentValue":"9router/mimo"}]}}"#;
        chat.output(format!("{opened}\n").as_bytes());
        assert_eq!(chat.session.as_deref(), Some("ses_1"));
        // A mensagem guardada sai com a sessão, e o modelo aparece na conversa.
        let sent = chat.outbox();
        assert!(sent.len() == 1 && sent[0].contains("session/prompt"));
        assert!(sent[0].contains("\"sessionId\":\"ses_1\"") && sent[0].contains("\"oi\""));
        assert!(
            chat.transcript()
                .contains("sessão iniciada · modelo 9router/mimo")
        );
        // Cada turno tem o seu bloco de texto.
        let chunk = |text: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{text}"}}}}}}}}"#
            )
        };
        chat.output(format!("{}\n", chunk("Olá")).as_bytes());
        assert_eq!(chat.entries.last(), Some(&Entry::Agent("Olá".into())));
        chat.message("tudo bem?");
        chat.outbox();
        assert!(
            chat.output(
                b"{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"stopReason\":\"end_turn\"}}\n"
            )
        );
        chat.output(format!("{}\n", chunk("Bem!")).as_bytes());
        assert_eq!(chat.entries.last(), Some(&Entry::Agent("Bem!".into())));
        assert!(!chat.busy);
        // As ferramentas repetem o começo no meio do caminho pelo mesmo id.
        let tool = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","name":"shell","title":"shell","status":"pending"}}}"#;
        let running = r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"in_progress","title":"ls -la"}}}"#;
        chat.output(format!("{tool}\n").as_bytes());
        chat.output(format!("{running}\n").as_bytes());
        let tools = chat
            .entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Tool { .. }))
            .count();
        assert_eq!(tools, 1);
        assert!(chat.transcript().contains("⚙ shell  ls -la"));
    }

    #[test]
    fn history_navigates_sent_messages() {
        let mut chat = chat();
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        // Sem histórico, ↑ não faz nada.
        chat.key(up);
        assert!(chat.input.is_empty() && chat.hist_pos.is_none());
        // Envia duas mensagens.
        typed(&mut chat, "primeira");
        chat.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        typed(&mut chat, "segunda");
        chat.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(chat.sent, vec!["primeira", "segunda"]);
        // ↑ mostra a mais recente e salva o rascunho vazio.
        chat.key(up);
        assert_eq!(chat.input, "segunda");
        assert_eq!(chat.hist_pos, Some(1));
        assert_eq!(chat.draft, "");
        // ↑ de novo mostra a mais antiga.
        chat.key(up);
        assert_eq!(chat.input, "primeira");
        assert_eq!(chat.hist_pos, Some(0));
        // ↑ no começo não sai do lugar.
        chat.key(up);
        assert_eq!(chat.input, "primeira");
        // ↓ volta para a mais recente.
        chat.key(down);
        assert_eq!(chat.input, "segunda");
        assert_eq!(chat.hist_pos, Some(1));
        // ↓ de novo restaura o rascunho.
        chat.key(down);
        assert_eq!(chat.input, "");
        assert!(chat.hist_pos.is_none());
    }

    #[test]
    fn history_saves_and_restores_draft() {
        let mut chat = chat();
        typed(&mut chat, "enviada");
        chat.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        // Rascunho em progresso.
        typed(&mut chat, "rascunho");
        // ↑ salva o rascunho.
        chat.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(chat.input, "enviada");
        assert_eq!(chat.draft, "rascunho");
        // ↓ restaura o rascunho.
        chat.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(chat.input, "rascunho");
        assert!(chat.hist_pos.is_none());
    }

    #[test]
    fn ctrl_u_resets_history_position() {
        let mut chat = chat();
        typed(&mut chat, "a");
        chat.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        chat.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert!(chat.hist_pos.is_some());
        chat.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(chat.hist_pos.is_none() && chat.input.is_empty());
    }

    #[test]
    fn effort_shown_in_title_and_updated_by_effort_command() {
        let init = r#"{"type":"system","subtype":"init","model":"claude-opus-5-5","parent_tool_use_id":null}"#;
        // Sem esforço inicial.
        let mut chat = Chat::new(
            "claude",
            Provider::Claude.adapter().unwrap(),
            Path::new("."),
            "",
        );
        chat.output(format!("{init}\n").as_bytes());
        assert_eq!(chat.model, "claude-opus-5-5");
        assert!(chat.effort.is_empty());
        // Com esforço passado no spawn.
        let mut chat = Chat::new(
            "claude",
            Provider::Claude.adapter().unwrap(),
            Path::new("."),
            "high",
        );
        chat.output(format!("{init}\n").as_bytes());
        assert_eq!(chat.effort, "high");
        // /effort atualiza o campo.
        chat.message("/effort medium");
        assert_eq!(chat.effort, "medium");
        chat.message("/effort xhigh");
        assert_eq!(chat.effort, "xhigh");
    }

    #[test]
    fn shift_tab_cycles_mode() {
        let mut chat = chat();
        let bt = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(chat.mode, "padrão");
        chat.key(bt);
        assert_eq!(chat.mode, "plan");
        let sent = chat.outbox();
        assert!(sent.iter().any(|s| s.contains("/plan")));
        chat.key(bt);
        assert_eq!(chat.mode, "auto");
        let sent = chat.outbox();
        assert!(sent.iter().any(|s| s.contains("/auto")));
        chat.key(bt);
        assert_eq!(chat.mode, "padrão");
        let sent = chat.outbox();
        assert!(sent.iter().any(|s| s.contains("/auto")));
    }
}
