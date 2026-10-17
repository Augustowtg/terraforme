//! Terminal do terraforme (`terraforme cli`): o agente roda num pseudoterminal
//! dentro de uma TUI com abas no topo — o agente (com uma sub-aba por
//! instância), o consumo de tokens e as extensões do modo (os relatórios
//! delas). `Shift+Tab` alterna as abas e `Shift+F1`–`F9` pulam para uma aba.
//! As sub-abas trocam do mesmo jeito em todas as abas: `Alt+1`–`9` vai direto
//! e `Ctrl+←→` / `Ctrl+PgUp/PgDn` passam para a vizinha (`Shift+dígito` não
//! serve: na aba do agente chega ao Claude como `!@#$`…). `F12` abre outra
//! instância do agente e `Shift+F12` fecha a aberta. As instâncias continuam rodando enquanto outra aba ou
//! sub-aba está aberta.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::{Frame, crossterm};
use terraforme_core::agent::Adapter;
use terraforme_core::config::Config;
use terraforme_core::extension::{Extension, Report};
use terraforme_core::limits::{self, Limit, Source};
use terraforme_core::usage::{self, Summary, Tokens};
use tui_term::widget::PseudoTerminal;

/// Linhas de histórico guardadas da saída do agente.
const SCROLLBACK: usize = 5000;
/// Intervalo de atualização automática da aba de consumo.
const USAGE_REFRESH: Duration = Duration::from_secs(5);
/// Sub-abas da aba de consumo.
const USAGE_VIEWS: [&str; 3] = ["Limites", "Tokens", "Subagentes"];
/// Largura da barra de progresso dos limites.
const BAR_WIDTH: usize = 40;
/// Máximo de instâncias do agente abertas ao mesmo tempo.
pub const MAX_INSTANCES: usize = 9;

/// O que `run` precisa para montar o terminal.
pub struct Options {
    /// Nome da aba do agente (ex.: `claude`).
    pub agent: String,
    /// Texto à direita do menu (provider e modo).
    pub title: String,
    /// Integração com o agente (sessão e consumo).
    pub adapter: &'static dyn Adapter,
    /// Comando de cada instância do agente.
    pub command: Command,
    /// Instâncias abertas ao iniciar.
    pub instances: usize,
    /// Passa um id de sessão próprio a cada instância, para o consumo saber
    /// qual é a sessão dela; desligado quando os argumentos já escolhem a sessão.
    pub session_ids: bool,
    /// Aparecem num aviso inicial, já que a tela alternativa esconderia o
    /// que fosse impresso antes.
    pub notices: Vec<String>,
    /// Extensões do modo; os relatórios delas ficam na aba Extensões.
    pub extensions: Vec<Extension>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// O agente, com uma sub-aba por instância.
    Agent,
    Usage,
    /// Relatórios de todas as extensões do modo, um por sub-aba.
    Extensions,
}

impl Tab {
    fn label(self, agent: &str) -> String {
        match self {
            Tab::Agent => agent.to_string(),
            Tab::Usage => "Consumo".into(),
            Tab::Extensions => "Extensões".into(),
        }
    }

    /// Sub-abas como (grupo, rótulo): os relatórios, agrupados pela extensão,
    /// ou as visões do consumo (sem grupo). As instâncias do agente ficam à
    /// parte, porque não carregam conteúdo.
    fn views(self, extensions: &[Extension]) -> Vec<(&'static str, &'static str)> {
        match self {
            Tab::Agent => Vec::new(),
            Tab::Usage => USAGE_VIEWS.iter().map(|&v| ("", v)).collect(),
            Tab::Extensions => reports(extensions)
                .into_iter()
                .map(|(ext, report)| (ext.name(), report.label))
                .collect(),
        }
    }
}

/// Relatórios de todas as extensões, na ordem das sub-abas da aba Extensões.
fn reports(extensions: &[Extension]) -> Vec<(Extension, Report)> {
    extensions
        .iter()
        .flat_map(|&ext| ext.reports().iter().map(move |&report| (ext, report)))
        .collect()
}

enum Msg {
    Output(usize, Vec<u8>),
    Exited(usize),
    Loaded {
        tab: Tab,
        index: usize,
        result: Result<Text<'static>, String>,
    },
}

enum Content {
    Loading,
    Ready(Text<'static>),
    Failed(String),
}

/// Estado da aba aberta quando não é a do agente.
struct View {
    tab: Tab,
    /// Sub-aba escolhida (relatório da extensão ou visão do consumo).
    index: usize,
    content: Content,
    scroll: u16,
    loaded_at: Instant,
    loading: bool,
}

/// Uma instância do agente, no próprio pseudoterminal.
struct Agent {
    /// Número da sub-aba; não é reaproveitado quando a instância fecha.
    id: usize,
    parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Linhas roladas para trás no histórico (0 = ao vivo).
    scrollback: usize,
    launched: SystemTime,
    /// Id de sessão passado ao agente.
    session: Option<String>,
}

impl Agent {
    fn spawn(
        id: usize,
        adapter: &dyn Adapter,
        command: &Command,
        cwd: &Path,
        body: Rect,
        session_ids: bool,
        tx: &Sender<Msg>,
    ) -> Result<Agent> {
        let pty = native_pty_system()
            .openpty(pty_size(body))
            .map_err(|e| anyhow!("{e}"))
            .context("falha ao abrir o pseudoterminal")?;
        let mut builder = builder(command, cwd);
        let mut session = None;
        if session_ids {
            let id = uuid::Uuid::new_v4().to_string();
            if let Some(args) = adapter.session_args(&id) {
                builder.args(args);
                session = Some(id);
            }
        }
        let launched = SystemTime::now();
        let child = pty
            .slave
            .spawn_command(builder)
            .map_err(|e| anyhow!("{e}"))
            .context("falha ao iniciar o agente")?;
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
        let tx = tx.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Msg::Output(id, buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Exited(id));
        });

        Ok(Agent {
            id,
            parser: vt100::Parser::new(body.height, body.width, SCROLLBACK),
            writer: pty.master.take_writer().map_err(|e| anyhow!("{e}"))?,
            master: pty.master,
            child,
            scrollback: 0,
            launched,
            session,
        })
    }

    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// `isize::MIN` volta ao vivo.
    fn scroll_history(&mut self, delta: isize) {
        self.scrollback = if delta == isize::MIN {
            0
        } else {
            self.scrollback.saturating_add_signed(delta)
        };
        self.parser.screen_mut().set_scrollback(self.scrollback);
        // O vt100 limita ao histórico existente.
        self.scrollback = self.parser.screen().scrollback();
    }

    fn resize(&mut self, body: Rect) -> Result<()> {
        self.parser.screen_mut().set_size(body.height, body.width);
        self.master
            .resize(pty_size(body))
            .map_err(|e| anyhow!("{e}"))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct App {
    agent: String,
    title: String,
    extensions: Vec<Extension>,
    adapter: &'static dyn Adapter,
    command: Command,
    cwd: PathBuf,
    session_ids: bool,
    agents: Vec<Agent>,
    next_id: usize,
    active: Tab,
    /// Sub-aba (instância) aberta na aba do agente; é a sessão mostrada no
    /// consumo.
    focused: usize,
    view: Option<View>,
    notices: Option<(String, u16)>,
    /// Área do agente (a tela menos o menu e as sub-abas).
    body: Rect,
    tx: Sender<Msg>,
    quit: bool,
}

/// Abre a TUI com o agente dentro e só volta quando todas as instâncias
/// terminam (ou F10).
pub fn run(options: Options) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().context("tamanho do terminal")?;
    let cwd = match options.command.get_current_dir() {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let (tx, rx) = mpsc::channel();

    let mut app = App {
        agent: options.agent,
        title: options.title,
        extensions: options.extensions,
        adapter: options.adapter,
        command: options.command,
        cwd,
        session_ids: options.session_ids,
        agents: Vec::new(),
        next_id: 1,
        active: Tab::Agent,
        focused: 1,
        view: None,
        notices: (!options.notices.is_empty()).then(|| (options.notices.join("\n"), 0)),
        body: body_size(Rect::new(0, 0, cols, rows)),
        tx,
        quit: false,
    };
    for _ in 0..options.instances.clamp(1, MAX_INSTANCES) {
        app.spawn()?;
    }

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = app.event_loop(&mut terminal, &rx);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    // Encerra as instâncias que ainda estiverem rodando.
    drop(app);
    result
}

/// Abas na ordem do menu: o agente, o consumo e, se o modo usa alguma, as
/// extensões.
fn tabs(extensions: &[Extension]) -> Vec<Tab> {
    [Tab::Agent, Tab::Usage]
        .into_iter()
        .chain((!extensions.is_empty()).then_some(Tab::Extensions))
        .collect()
}

impl App {
    fn tabs(&self) -> Vec<Tab> {
        tabs(&self.extensions)
    }

    /// Abre mais uma instância do agente e devolve o número dela.
    fn spawn(&mut self) -> Result<usize> {
        let id = self.next_id;
        let agent = Agent::spawn(
            id,
            self.adapter,
            &self.command,
            &self.cwd,
            self.body,
            self.session_ids,
            &self.tx,
        )?;
        self.agents.push(agent);
        self.next_id += 1;
        Ok(id)
    }

    fn new_instance(&mut self) {
        if self.agents.len() >= MAX_INSTANCES {
            self.notices = Some((format!("⚠ no máximo {MAX_INSTANCES} instâncias"), 0));
            return;
        }
        match self.spawn() {
            Ok(id) => self.focus(id),
            Err(err) => self.notices = Some((format!("⚠ {err:#}"), 0)),
        }
    }

    fn agent(&self, id: usize) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    fn agent_mut(&mut self, id: usize) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Instância da sub-aba aberta, se a aba aberta for a do agente.
    fn active_agent(&mut self) -> Option<&mut Agent> {
        match self.active {
            Tab::Agent => self.agent_mut(self.focused),
            _ => None,
        }
    }

    /// Abre a aba do agente na sub-aba da instância.
    fn focus(&mut self, id: usize) {
        self.focused = id;
        self.select(Tab::Agent);
    }

    /// Sub-abas da aba aberta (instâncias do agente ou visões) e qual está
    /// aberta.
    fn sub_tabs(&self) -> (usize, usize) {
        match &self.view {
            Some(view) => (view.tab.views(&self.extensions).len(), view.index),
            None => (
                self.agents.len(),
                self.agents
                    .iter()
                    .position(|a| a.id == self.focused)
                    .unwrap_or(0),
            ),
        }
    }

    /// Abre a sub-aba `index` da aba aberta.
    fn pick(&mut self, index: usize) {
        let (count, current) = self.sub_tabs();
        if index >= count || index == current {
            return;
        }
        match &mut self.view {
            Some(view) => {
                view.index = index;
                self.load(true);
            }
            None => self.focus(self.agents[index].id),
        }
    }

    /// Passa `delta` sub-abas adiante (ou atrás), em volta.
    fn cycle(&mut self, delta: isize) {
        let (count, current) = self.sub_tabs();
        if count > 1 {
            self.pick((current as isize + delta).rem_euclid(count as isize) as usize);
        }
    }

    /// Tira da lista a instância que terminou; sem nenhuma, sai. Se era a
    /// sub-aba aberta, abre a vizinha.
    fn closed(&mut self, id: usize) {
        let Some(i) = self.agents.iter().position(|a| a.id == id) else {
            return;
        };
        self.agents.remove(i);
        if self.agents.is_empty() {
            self.quit = true;
            return;
        }
        if self.focused == id {
            self.focused = self.agents[i.min(self.agents.len() - 1)].id;
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal, rx: &Receiver<Msg>) -> Result<()> {
        let mut dirty = true;
        while !self.quit {
            while let Ok(msg) = rx.try_recv() {
                self.handle_msg(msg);
                dirty = true;
            }
            if let Some(view) = &self.view
                && view.tab == Tab::Usage
                && !view.loading
                && view.loaded_at.elapsed() >= USAGE_REFRESH
            {
                self.load(false);
            }
            if dirty {
                self.fit(terminal)?;
                terminal.draw(|f| self.draw(f))?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(16))? {
                // Várias teclas de uma vez (ex.: digitação rápida) num só redesenho.
                loop {
                    if self.handle_event(event::read()?)? {
                        return Ok(());
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
                dirty = true;
            }
        }
        Ok(())
    }

    /// Ajusta as instâncias ao tamanho real da tela. Conferido antes de cada
    /// desenho, e não só no `Event::Resize`, porque o tamanho lido antes de
    /// abrir a TUI pode não ser o da tela em que ela é desenhada.
    fn fit(&mut self, terminal: &DefaultTerminal) -> Result<()> {
        let size = terminal.size()?;
        let body = body_size(Rect::new(0, 0, size.width, size.height));
        if body != self.body {
            self.body = body;
            for agent in &mut self.agents {
                agent.resize(body)?;
            }
        }
        Ok(())
    }

    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Output(id, bytes) => {
                if let Some(agent) = self.agent_mut(id) {
                    agent.parser.process(&bytes);
                }
            }
            Msg::Exited(id) => self.closed(id),
            Msg::Loaded { tab, index, result } => {
                if let Some(view) = &mut self.view
                    && view.tab == tab
                    && view.index == index
                {
                    view.content = match result {
                        Ok(text) => Content::Ready(text),
                        Err(err) => Content::Failed(err),
                    };
                    view.loading = false;
                    view.loaded_at = Instant::now();
                }
            }
        }
    }

    /// Devolve `true` para sair.
    fn handle_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.notices.is_none() => {
                if let Some(agent) = self.active_agent() {
                    if agent.parser.screen().bracketed_paste() {
                        agent.send(format!("\x1b[200~{text}\x1b[201~").as_bytes())?;
                    } else {
                        agent.send(text.as_bytes())?;
                    }
                }
            }
            // O redesenho seguinte ajusta as instâncias ao novo tamanho (`fit`).
            _ => {}
        }
        Ok(false)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        let tabs = self.tabs();
        if let Some(n) = shift_function(key)
            && n <= tabs.len()
        {
            self.select(tabs[n - 1]);
            return Ok(false);
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::F(10) => return Ok(true),
            // Sub-abas da aba aberta, igual em todas: Alt+1–9 vai direto e
            // Ctrl+←→ / Ctrl+PgUp/PgDn passam para a vizinha.
            KeyCode::Char(c @ '1'..='9') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pick(c as usize - '1' as usize);
            }
            KeyCode::Left | KeyCode::PageUp if ctrl => self.cycle(-1),
            KeyCode::Right | KeyCode::PageDown if ctrl => self.cycle(1),
            // Alguns terminais mandam Shift+F12 como F24.
            KeyCode::F(12) if shift => self.close_active(),
            KeyCode::F(24) => self.close_active(),
            KeyCode::F(12) => self.new_instance(),
            // Tab e F1–F9 sem Shift seguem para o agente.
            KeyCode::BackTab => {
                let i = tabs.iter().position(|&t| t == self.active).unwrap_or(0);
                self.select(tabs[(i + 1) % tabs.len()]);
            }
            _ if self.view.is_some() => self.view_key(key),
            _ if self.notices.is_some() => self.notices_key(key),
            _ => {
                let Some(agent) = self.active_agent() else {
                    return Ok(false);
                };
                let half = agent.parser.screen().size().0 as isize / 2;
                match key.code {
                    KeyCode::PageUp if shift => agent.scroll_history(half),
                    KeyCode::PageDown if shift => agent.scroll_history(-half),
                    _ => {
                        let bytes = key_bytes(key, agent.parser.screen().application_cursor());
                        if !bytes.is_empty() {
                            agent.scroll_history(isize::MIN);
                            agent.send(&bytes)?;
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    /// Encerra a instância aberta; a aba some quando o processo termina.
    fn close_active(&mut self) {
        if let Some(agent) = self.active_agent() {
            let _ = agent.child.kill();
        }
    }

    fn notices_key(&mut self, key: KeyEvent) {
        let Some((_, scroll)) = &mut self.notices else {
            return;
        };
        match key.code {
            KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::Down => *scroll = scroll.saturating_add(1),
            _ => self.notices = None,
        }
    }

    fn view_key(&mut self, key: KeyEvent) {
        let Some(view) = &mut self.view else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.select(Tab::Agent),
            KeyCode::Up | KeyCode::Char('k') => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => view.scroll = view.scroll.saturating_add(1),
            KeyCode::PageUp => view.scroll = view.scroll.saturating_sub(10),
            KeyCode::PageDown => view.scroll = view.scroll.saturating_add(10),
            KeyCode::Home | KeyCode::Char('g') => view.scroll = 0,
            KeyCode::Char('r') => self.load(true),
            _ => {}
        }
    }

    /// Abre a aba; as que não são do agente recarregam o conteúdo.
    fn select(&mut self, tab: Tab) {
        self.active = tab;
        self.view = match tab {
            Tab::Agent => None,
            tab => Some(View {
                tab,
                index: 0,
                content: Content::Loading,
                scroll: 0,
                loaded_at: Instant::now(),
                loading: false,
            }),
        };
        if self.view.is_some() {
            self.load(true);
        }
    }

    /// Carrega o conteúdo da aba aberta em segundo plano. Sem `reset`, o
    /// conteúdo atual fica na tela até o novo chegar.
    fn load(&mut self, reset: bool) {
        let (launched, session) = self
            .agent(self.focused)
            .map(|a| (a.launched, a.session.clone()))
            .unwrap_or((SystemTime::now(), None));
        let Some(view) = &mut self.view else {
            return;
        };
        if reset {
            view.content = Content::Loading;
            view.scroll = 0;
        }
        view.loading = true;
        let (tab, index) = (view.tab, view.index);
        let report = reports(&self.extensions).get(index).copied();
        let tx = self.tx.clone();
        let cwd = self.cwd.clone();
        let adapter = self.adapter;
        thread::spawn(move || {
            let result = match tab {
                Tab::Agent => return,
                Tab::Usage => {
                    adapter
                        .usage(&cwd, launched, session.as_deref())
                        .map(|u| match index {
                            0 => format_limits(adapter, &u),
                            1 => format_usage(&u),
                            _ => format_agents(&u),
                        })
                }
                Tab::Extensions => match report {
                    Some((ext, report)) => ext.report(report).map(Text::from),
                    None => return,
                },
            };
            let result = result.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Loaded { tab, index, result });
        });
    }

    fn draw(&self, frame: &mut Frame) {
        let [menu, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());

        frame.render_widget(self.menu(), menu);

        if let Some(view) = &self.view {
            self.draw_view(frame, view, body);
            return;
        }

        if self.active == Tab::Agent
            && let Some(agent) = self.agent(self.focused)
        {
            let [instances, body] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
            frame.render_widget(self.instances(), instances);

            let mut term = PseudoTerminal::new(agent.parser.screen());
            if self.notices.is_some() || agent.scrollback > 0 {
                let mut cursor = tui_term::widget::Cursor::default();
                cursor.hide();
                term = term.cursor(cursor);
            }
            frame.render_widget(term, body);

            if agent.scrollback > 0 {
                let hint = format!(" histórico −{} · Shift+PgDn volta ", agent.scrollback);
                let width = hint.chars().count() as u16;
                let area = Rect::new(body.right().saturating_sub(width), body.y, width, 1)
                    .intersection(body);
                frame.render_widget(Span::raw(hint).black().on_yellow(), area);
            }
        }

        if let Some((text, scroll)) = &self.notices {
            let area = body.inner(Margin::new(
                (body.width / 12).max(1),
                (body.height / 4).max(1),
            ));
            frame.render_widget(Clear, area);
            let block = Block::bordered()
                .title(Line::from(" Avisos ").bold().cyan())
                .title_bottom(Line::from(" qualquer tecla fecha ").dark_gray())
                .border_style(Style::new().cyan());
            frame.render_widget(
                Paragraph::new(text.as_str())
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0))
                    .block(block),
                area,
            );
        }
    }

    fn menu(&self) -> Line<'_> {
        let mut spans = vec![
            Span::styled(" terraforme ", Style::new().black().on_green().bold()),
            Span::raw(" "),
        ];
        for (i, tab) in self.tabs().into_iter().enumerate() {
            let style = if tab == self.active {
                Style::new().black().on_cyan().bold()
            } else {
                Style::new().white().on_dark_gray()
            };
            spans.push(Span::styled(
                format!(" ⇧F{} ", i + 1),
                style.add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!("{} ", tab.label(&self.agent)), style));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(" F10 ", Style::new().white().on_red().bold()));
        spans.push(Span::styled("sair ", Style::new().white().on_red()));
        spans.push(Span::raw(format!("  ⇧Tab alterna · {}", self.title)).dark_gray());
        Line::from(spans).style(Style::new().bg(Color::Reset))
    }

    /// Sub-abas da aba do agente: uma por instância.
    fn instances(&self) -> Line<'_> {
        let labels: Vec<_> = self
            .agents
            .iter()
            .map(|a| ("", format!("{} {}", self.agent, a.id)))
            .collect();
        let (_, current) = self.sub_tabs();
        let mut spans = vec![Span::raw(" ")];
        spans.extend(sub_tab_spans(&labels, current));
        let hint = if self.agents.len() > 1 {
            " ⌥1-9/Ctrl+←→ sub-aba · F12 nova · ⇧F12 fecha"
        } else {
            " F12 nova instância · ⇧F12 fecha"
        };
        spans.push(Span::raw(hint).dark_gray());
        Line::from(spans)
    }

    fn draw_view(&self, frame: &mut Frame, view: &View, area: Rect) {
        let views = view.tab.views(&self.extensions);
        let title = match view.tab {
            Tab::Extensions => match views.get(view.index) {
                Some((ext, _)) => format!("Extensões · {ext}"),
                None => "Extensões".to_string(),
            },
            _ if self.agents.len() > 1 => {
                format!("Consumo de tokens · {} {}", self.agent, self.focused)
            }
            _ => "Consumo de tokens".to_string(),
        };
        let hint = if views.len() > 1 {
            " ⌥1-9/Ctrl+←→ sub-aba · ↑↓ rolar · r recarregar · ⇧Tab próxima aba · Esc agente "
        } else {
            " ↑↓ rolar · r recarregar · ⇧Tab próxima aba · Esc agente "
        };
        let block = Block::bordered()
            .title(Line::from(format!(" {title} ")).bold().cyan())
            .title_bottom(Line::from(hint).dark_gray())
            .border_style(Style::new().cyan());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let inner = if views.len() > 1 {
            let labels: Vec<_> = views
                .iter()
                .map(|&(group, name)| (group, name.to_string()))
                .collect();
            let tabs = sub_tab_spans(&labels, view.index);
            let [header, rest] =
                Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
            frame.render_widget(
                Paragraph::new(Line::from(tabs)).wrap(Wrap { trim: false }),
                header,
            );
            rest
        } else {
            inner
        };

        let paragraph = match &view.content {
            Content::Loading => Paragraph::new("carregando…").dark_gray(),
            Content::Ready(text) => Paragraph::new(text.clone()),
            Content::Failed(err) => Paragraph::new(format!("✘ {err}")).red(),
        };
        frame.render_widget(
            paragraph
                .wrap(Wrap { trim: false })
                .scroll((view.scroll, 0)),
            inner,
        );
    }
}

/// Barra de sub-abas, igual em todas as abas: `⌥N rótulo`, a aberta em
/// destaque e o nome do grupo (ex.: a extensão) antes do primeiro item dele.
fn sub_tab_spans(labels: &[(&str, String)], current: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (group, name)) in labels.iter().enumerate() {
        if !group.is_empty() && (i == 0 || labels[i - 1].0 != *group) {
            if i > 0 {
                spans.push(Span::raw("│ ").dark_gray());
            }
            spans.push(Span::raw(format!("{group} ")).bold());
        }
        let label = format!(" ⌥{} {name} ", i + 1);
        spans.push(if i == current {
            Span::styled(label, Style::new().black().on_cyan().bold())
        } else {
            Span::raw(label).cyan()
        });
        spans.push(Span::raw(" "));
    }
    spans
}

/// Uma barra por limite, com quanto já foi usado e quanto falta.
fn format_limits(adapter: &dyn Adapter, usage: &usage::Usage) -> Text<'static> {
    let budget = Config::load().map(|c| c.budget).unwrap_or_default();
    let limits = limits::collect(adapter, usage, &budget);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut lines: Vec<Line> = Vec::new();
    for limit in &limits.items {
        let mut title = vec![Span::raw(limit.label.clone()).bold()];
        if let Some(at) = limit.resets_at {
            title.push(
                Span::raw(format!("  · zera em {}", duration(at.saturating_sub(now)))).dark_gray(),
            );
        }
        if let Some((used, max)) = limit.tokens {
            title.push(
                Span::raw(format!(
                    "  · {} de {} tokens",
                    thousands(used),
                    thousands(max)
                ))
                .dark_gray(),
            );
        }
        lines.push(Line::from(title));
        lines.push(bar(limit));
        lines.push(Line::default());
    }

    if limits.items.is_empty() {
        lines.push(Line::raw("Nenhum limite disponível.").yellow());
        lines.push(Line::raw(
            "Sem login OAuth no Claude Code (assinatura Pro/Max/Team), defina um orçamento de tokens:",
        ));
        lines.push(
            Line::raw("  terraforme config --budget-5h 20000000 --budget-week 300000000").cyan(),
        );
        lines.push(Line::default());
    }
    if let Some(note) = limits.note {
        lines.push(Line::raw(format!("⚠ {note}")).yellow());
    }
    lines.push(
        Line::raw(match limits.source {
            Source::Plan => {
                "Fonte: limites do plano (api.anthropic.com, como o /usage) · atualiza a cada 60s"
            }
            Source::Budget => {
                "Fonte: orçamento [budget] do config.toml × transcripts do Claude Code"
            }
        })
        .dark_gray(),
    );
    Text::from(lines)
}

/// `██████░░░░  60% usado · 40% restante`, colorida pelo que falta.
fn bar(limit: &Limit) -> Line<'static> {
    let remaining = limit.remaining();
    let filled = ((limit.used.clamp(0.0, 100.0) / 100.0) * BAR_WIDTH as f64).round() as usize;
    let color = if remaining > 50.0 {
        Color::Green
    } else if remaining > 20.0 {
        Color::Yellow
    } else {
        Color::Red
    };
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::new().fg(color)),
        Span::raw("░".repeat(BAR_WIDTH - filled)).dark_gray(),
        Span::raw(format!("  {:.0}% usado · ", limit.used)),
        Span::styled(
            format!("{remaining:.0}% restante"),
            Style::new().fg(color).bold(),
        ),
    ])
}

/// `9000` → `2h30min`; `200000` → `2d 7h`.
fn duration(secs: u64) -> String {
    let (days, hours, mins) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("{mins}min"),
        (0, _) => format!("{hours}h{mins:02}min"),
        _ => format!("{days}d {hours}h"),
    }
}

const USAGE_HEADER: [&str; 6] = [
    "Entrada",
    "Saída",
    "Cache (grava)",
    "Cache (lê)",
    "Total",
    "Respostas",
];

fn usage_header() -> String {
    let [a, b, c, d, e, f] = USAGE_HEADER;
    format!("{:<22} {a:>12} {b:>12} {c:>14} {d:>14} {e:>14} {f:>9}", "")
}

fn usage_row(label: &str, tokens: &Tokens, responses: Option<usize>) -> String {
    format!(
        "{:<22} {:>12} {:>12} {:>14} {:>14} {:>14} {:>9}",
        truncate(label, 22),
        thousands(tokens.input),
        thousands(tokens.output),
        thousands(tokens.cache_write),
        thousands(tokens.cache_read),
        thousands(tokens.total()),
        responses.map(|r| thousands(r as u64)).unwrap_or_default()
    )
}

/// Tabela do consumo: uma linha por período e, embaixo, a sessão por modelo.
fn format_usage(usage: &usage::Usage) -> Text<'static> {
    let header = usage_header();
    let summary = |label: &str, s: &Summary| usage_row(label, &s.tokens, Some(s.responses));

    let empty = Summary::default();
    let mut lines = vec![
        header.clone(),
        summary("Sessão atual", usage.session.as_ref().unwrap_or(&empty)),
        summary("Projeto · 24h", &usage.project_day),
        summary("Todos · 5h", &usage.all_five_hours),
        summary("Todos · 24h", &usage.all_day),
        summary("Todos · 7 dias", &usage.all_week),
        String::new(),
    ];
    match &usage.session {
        Some(session) => {
            lines.push("Sessão atual por modelo".into());
            lines.push(header);
            for (model, tokens) in &session.by_model {
                lines.push(usage_row(model, tokens, None));
            }
        }
        None => lines.push("A sessão atual ainda não tem respostas do modelo.".into()),
    }
    lines.push(String::new());
    lines.push(format!(
        "Fonte: transcripts do Claude Code · atualiza a cada {}s",
        USAGE_REFRESH.as_secs()
    ));
    Text::from(lines.join("\n"))
}

/// A sessão atual dividida entre o agente principal e cada subagente.
fn format_agents(usage: &usage::Usage) -> Text<'static> {
    if usage.session_agents.is_empty() {
        return Text::from("A sessão atual ainda não tem respostas do modelo.");
    }
    let mut lines = vec![usage_header()];
    let mut descriptions = Vec::new();
    for agent in &usage.session_agents {
        let label = match (&agent.id, &agent.kind) {
            (None, _) => "Principal".to_string(),
            (Some(_), Some(kind)) => kind.clone(),
            (Some(id), None) => format!("subagente {}", truncate(id, 8)),
        };
        lines.push(usage_row(
            &label,
            &agent.summary.tokens,
            Some(agent.summary.responses),
        ));
        if let Some(description) = &agent.description {
            descriptions.push(format!("  {} — {description}", truncate(&label, 22)));
        }
    }
    let subagents = usage
        .session_agents
        .iter()
        .filter(|a| a.id.is_some())
        .count();
    lines.push(String::new());
    lines.push(format!("{subagents} subagente(s) nesta sessão"));
    lines.extend(descriptions);
    lines.push(String::new());
    lines.push(format!(
        "Fonte: transcripts do Claude Code (<sessão>/subagents) · atualiza a cada {}s",
        USAGE_REFRESH.as_secs()
    ));
    Text::from(lines.join("\n"))
}

/// Corta em `max` caracteres, terminando com `…`.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Shift+F1–F9 → 1–9. Alguns terminais mandam Shift+F1 como F13.
fn shift_function(key: KeyEvent) -> Option<usize> {
    match key.code {
        KeyCode::F(n @ 1..=9) if key.modifiers.contains(KeyModifiers::SHIFT) => Some(n as usize),
        KeyCode::F(n @ 13..=21) => Some(n as usize - 12),
        _ => None,
    }
}

/// `1234567` → `1.234.567`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Área do agente: a tela menos a linha do menu e a das sub-abas.
fn body_size(area: Rect) -> Rect {
    Rect::new(
        0,
        0,
        area.width.max(1),
        area.height.saturating_sub(2).max(1),
    )
}

fn pty_size(body: Rect) -> PtySize {
    PtySize {
        rows: body.height,
        cols: body.width,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Converte o `Command` montado pelo core para o formato do pseudoterminal.
fn builder(command: &Command, cwd: &std::path::Path) -> CommandBuilder {
    let mut builder = CommandBuilder::new(command.get_program());
    builder.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => builder.env(key, value),
            None => builder.env_remove(key),
        }
    }
    builder.cwd(cwd);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder
}

/// Bytes que um terminal xterm enviaria para a tecla.
fn key_bytes(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // Parâmetro xterm de modificadores: 1 + shift + 2·alt + 4·ctrl.
    let modifier = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    let cursor = |c: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let bytes = match key.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![c as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            ']' | '5' => vec![0x1d],
            '^' | '6' => vec![0x1e],
            '_' | '-' | '7' => vec![0x1f],
            _ => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift+Enter quebra a linha no Claude Code, como Alt+Enter.
        KeyCode::Enter if shift => return b"\x1b\r".to_vec(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return cursor('A'),
        KeyCode::Down => return cursor('B'),
        KeyCode::Right => return cursor('C'),
        KeyCode::Left => return cursor('D'),
        KeyCode::Home => return cursor('H'),
        KeyCode::End => return cursor('F'),
        KeyCode::Insert => return tilde(2),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let c = (b'P' + n - 1) as char;
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{c}").into_bytes()
            } else {
                format!("\x1bO{c}").into_bytes()
            };
        }
        KeyCode::F(n @ 5..=12) => return tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        _ => return Vec::new(),
    };

    if alt {
        [&[0x1b][..], &bytes].concat()
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        key_bytes(KeyEvent::new(code, modifiers), false)
    }

    #[test]
    fn plain_and_control_keys() {
        assert_eq!(key(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(key(KeyCode::Char('ç'), KeyModifiers::NONE), "ç".as_bytes());
        assert_eq!(key(KeyCode::Char('c'), KeyModifiers::CONTROL), [3]);
        assert_eq!(key(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        assert_eq!(key(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(key(KeyCode::Enter, KeyModifiers::SHIFT), b"\x1b\r");
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::NONE), [0x7f]);
        assert_eq!(key(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
    }

    #[test]
    fn cursor_and_function_keys() {
        assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(key(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D");
        assert_eq!(key(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(key(KeyCode::F(4), KeyModifiers::NONE), b"\x1bOS");
        assert_eq!(key(KeyCode::F(12), KeyModifiers::NONE), b"\x1b[24~");
    }

    #[test]
    fn tabs_list_agent_then_usage_and_extensions() {
        assert_eq!(tabs(&[]), [Tab::Agent, Tab::Usage]);
        assert_eq!(
            tabs(&[Extension::Rtk]),
            [Tab::Agent, Tab::Usage, Tab::Extensions]
        );
        assert_eq!(
            Tab::Extensions.views(&[Extension::Rtk, Extension::AiMemory])[0],
            ("RTK", "Economia")
        );
        assert_eq!(Tab::Agent.label("claude"), "claude");
    }

    #[test]
    fn shifted_keys_pick_tabs_and_views() {
        let f = |n, m| shift_function(KeyEvent::new(KeyCode::F(n), m));
        assert_eq!(f(2, KeyModifiers::SHIFT), Some(2));
        assert_eq!(f(2, KeyModifiers::NONE), None);
        assert_eq!(f(13, KeyModifiers::NONE), Some(1));
    }

    #[test]
    fn sub_tabs_share_one_style() {
        let text = |spans: Vec<Span>| {
            spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        let instances = [("", "claude 1".to_string()), ("", "claude 2".to_string())];
        assert_eq!(
            text(sub_tab_spans(&instances, 1)),
            " ⌥1 claude 1   ⌥2 claude 2  "
        );
        let reports = [
            ("RTK", "gain".to_string()),
            ("AI Memory", "Status".to_string()),
        ];
        let spans = sub_tab_spans(&reports, 0);
        assert_eq!(
            text(spans.clone()),
            "RTK  ⌥1 gain  │ AI Memory  ⌥2 Status  "
        );
        assert_eq!(spans[1].style.bg, Some(Color::Cyan));
    }

    #[test]
    fn durations_and_truncation() {
        assert_eq!(duration(59), "0min");
        assert_eq!(duration(9000), "2h30min");
        assert_eq!(duration(200_000), "2d 7h");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn thousands_uses_dots() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1.000");
        assert_eq!(thousands(1_234_567), "1.234.567");
    }

    #[test]
    fn body_leaves_room_for_the_menu_and_instances() {
        assert_eq!(body_size(Rect::new(0, 0, 80, 24)), Rect::new(0, 0, 80, 22));
        assert_eq!(body_size(Rect::new(0, 0, 0, 0)), Rect::new(0, 0, 1, 1));
    }
}
