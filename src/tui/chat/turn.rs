//! Um turno da conversa: envio da mensagem, pedidos de permissão, eventos do
//! agente e a auto compactação ao fim.

use super::permission::{self, Kind};
use super::{Chat, DEFAULT_CONTEXT_WINDOW};
use crate::agent::{AgentEvent, PermissionMode, ReportedMode, Request};
use crate::tui::history::Entry;
use crate::tui::input::commands;
use crate::tui::input::extensions::AgentStatus;

impl Chat {
    /// Envia o texto do input ao agente. Ignora texto vazio e, enquanto o
    /// agente ainda responde, mantém o texto para enviar depois. Um comando
    /// com `/` e argumentos (ex.: `/config model=opus`) vai como está: é o
    /// padrão do router; sem argumentos, um comando com interface no terra
    /// (ex.: `/clear`) roda a interface.
    pub(super) fn send(&mut self) {
        let prompt = self.input.text().trim().to_string();
        if self.read_only && !prompt.is_empty() {
            self.input.clear();
            self.history.push(Entry::Info(
                "Esta aba só mostra uma execução do orquestrador: ela roda sozinha.".into(),
            ));
            self.history.follow();
            return;
        }
        if prompt.is_empty() || self.run.is_some() {
            return;
        }
        // O `/agente <nome>` é o único comando do terra com argumento.
        if let Some(arg) = prompt.strip_prefix("/agente ") {
            self.remember_sent(&prompt);
            self.input.clear();
            self.agent_command(arg.trim());
            self.history.follow();
            return;
        }
        if let Some(name) = prompt.strip_prefix('/')
            && commands::has_interface(name)
        {
            self.remember_sent(&prompt);
            self.input.clear();
            self.run_command(name);
            return;
        }
        if prompt.starts_with('/') && self.route_to_provider(&prompt) {
            self.remember_sent(&prompt);
            self.input.clear();
            self.history.follow();
            return;
        }
        if self.start(&prompt, Entry::User(prompt.clone())) {
            self.remember_sent(&prompt);
            self.input.clear();
            // A auto compactação é para as mensagens, não para os comandos.
            self.auto_compact_armed = !prompt.starts_with('/');
        }
    }

    /// Guarda a mensagem enviada para o ↑, nesta execução e no workspace.
    fn remember_sent(&mut self, prompt: &str) {
        self.input.remember();
        if let Some(recorder) = &mut self.recorder
            && let Err(err) = recorder.remember_sent(prompt)
        {
            self.recording_failed(err);
        }
    }

    /// Manda `prompt` ao agente como uma mensagem do usuário (ex.: a
    /// primeira de um chat aberto pelo orquestrador).
    pub fn submit(&mut self, prompt: &str) {
        if self.run.is_none() && self.start(prompt, Entry::User(prompt.to_string())) {
            self.auto_compact_armed = true;
        }
    }

    /// Manda `prompt` ao agente na sessão atual e, se ele começou, põe
    /// `entry` no histórico. Retorna se a resposta começou.
    pub(super) fn start(&mut self, prompt: &str, entry: Entry) -> bool {
        let Some(adapter) = self.available_adapter() else {
            self.history.push(Entry::Error(self.unavailable_hint()));
            return false;
        };
        let extensions = self.agent_extensions();
        let started = adapter.start(Request {
            prompt,
            mode: self.permission_mode,
            session_id: self.session_id.as_deref(),
            model: self
                .session_model
                .as_deref()
                .or(self.chosen_model.as_deref()),
            account: self.account.as_deref(),
            effort: self
                .session_effort
                .as_deref()
                .or(self.default_effort.as_deref()),
            subagent_model: self.subagent_model.as_deref(),
            extensions: &extensions,
            allowed_tools: &self.allowed,
        });
        drop(extensions);
        self.allowed.clear();
        let ok = started.is_ok();
        match started {
            Ok(run) => {
                self.run = Some(run);
                self.requested_mode = Some(self.permission_mode);
                self.history.push(entry);
                self.agent_status = AgentStatus::Pensando;
                self.agent_activity = Some("Pensando...".to_string());
            }
            Err(err) => {
                self.history.push(Entry::Error(err));
                self.agent_status = AgentStatus::Erro;
            }
        }
        self.history.follow();
        ok
    }

    /// Manda a resposta do usuário a um pedido de permissão e a registra no
    /// histórico.
    pub(super) fn answer(&mut self, request: permission::Request, allow: bool) {
        match &request.kind {
            Kind::Ask { id } => {
                if let Some(run) = &mut self.run {
                    run.respond(id, allow);
                }
            }
            Kind::Blocked { allow: rule, .. } => {
                if allow {
                    self.allowed.push(rule.clone());
                }
            }
            // O chat não fala com o hub: a tela principal leva a resposta.
            Kind::Remote { run, id, .. } => {
                self.remote_answers.push((run.clone(), id.clone(), allow));
            }
        }
        let verdict = if allow { "Permitido" } else { "Negado" };
        let detail = if request.detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", request.detail)
        };
        self.history
            .push(Entry::Info(format!("{verdict}: {}{detail}.", request.name)));
        if self.run.is_some() {
            self.agent_activity = Some(match self.prompts.current() {
                Some(next) if matches!(next.kind, Kind::Ask { .. }) => {
                    format!("Aguardando permissão para {}...", next.name)
                }
                _ => "Pensando...".to_string(),
            });
        }
        self.history.follow();
        self.resume_allowed();
    }

    /// Com a resposta terminada e os pedidos respondidos, manda o agente
    /// fazer o que o modo bloqueou e o usuário permitiu, com essas
    /// ferramentas liberadas.
    fn resume_allowed(&mut self) {
        if self.run.is_some() || self.prompts.has_local() || self.allowed.is_empty() {
            return;
        }
        let prompt = format!(
            "O usuário permitiu no terra o que o modo de permissão tinha bloqueado, e agora está liberado: {}. Faça isso agora e continue a tarefa.",
            self.allowed.join(", ")
        );
        self.start(
            &prompt,
            Entry::Info("Fazendo o que foi permitido...".to_string()),
        );
    }

    /// Interrompe a resposta em andamento (soltar a sessão a encerra).
    pub(super) fn cancel(&mut self) {
        self.run = None;
        self.turn_spent = (0, 0);
        self.prompts.clear();
        self.allowed.clear();
        self.auto_compact_armed = false;
        self.history.push(Entry::Info("Interrompido.".to_string()));
        self.agent_status = AgentStatus::Ocioso;
        self.agent_activity = None;
    }

    /// Aplica um evento do agente ao histórico e ao status.
    pub(super) fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Init {
                session_id,
                model,
                permission_mode,
            } => {
                self.session_id = Some(session_id);
                self.model = Some(model).filter(|m| !m.is_empty());
                if self.read_only {
                    // A execução não segue o modo deste terminal: só mostra.
                    self.viewer_mode = match permission_mode {
                        ReportedMode::Known(mode) => Some(mode.label().to_string()),
                        ReportedMode::Unknown(name) => Some(name),
                        ReportedMode::Missing => None,
                    };
                } else {
                    self.sync_mode(permission_mode);
                }
            }
            AgentEvent::TextStart => {
                self.history.start_text();
                self.agent_activity = Some("Respondendo...".to_string());
            }
            AgentEvent::TextDelta(text) => self.history.append_text(&text),
            AgentEvent::Text(text) => self.history.push(Entry::Assistant(text)),
            AgentEvent::ToolUse { name, detail } => {
                self.agent_activity = Some(format!("Usando {name}..."));
                self.history.push(Entry::Tool { name, detail });
            }
            AgentEvent::PermissionRequest { id, name, detail } => {
                self.agent_activity = Some(format!("Aguardando permissão para {name}..."));
                self.prompts.push(permission::Request {
                    name,
                    detail,
                    kind: Kind::Ask { id },
                });
            }
            AgentEvent::Blocked {
                name,
                detail,
                reason,
                allow,
            } => {
                self.prompts.push(permission::Request {
                    name,
                    detail,
                    kind: Kind::Blocked { reason, allow },
                });
            }
            AgentEvent::ToolResult { is_error, summary } => {
                self.agent_activity = Some("Pensando...".to_string());
                self.history.push(Entry::ToolResult { is_error, summary });
            }
            AgentEvent::Context { tokens, window } => {
                self.context = Some(tokens);
                // O limite só vem no fim da resposta: fica o último informado.
                if window.is_some() {
                    self.context_window = window;
                }
            }
            AgentEvent::Compacted { tokens } => {
                // O tamanho depois da compactação mantém a barra; sem ele, o
                // novo vem na próxima chamada ao modelo. A mensagem já foi
                // compactada (pelo próprio agente): não compacta de novo.
                self.context = tokens;
                self.auto_compact_armed = false;
                self.history
                    .push(Entry::Info("Conversa compactada.".to_string()));
            }
            AgentEvent::Usage {
                tokens,
                cache_tokens,
            } => self.count_turn(tokens, cache_tokens),
            AgentEvent::Done {
                is_error,
                message,
                tokens,
                cache_tokens,
                denied,
            } => {
                // Sem o uso no fim, fica o que o turno informou até aqui.
                if tokens > 0 || cache_tokens > 0 {
                    self.count_turn(tokens, cache_tokens);
                }
                if denied > 0 {
                    self.history.push(Entry::Info(format!(
                        "{denied} ferramenta(s) negada(s) no modo de permissão {}.",
                        self.permission_mode.label()
                    )));
                }
                if let Some(message) = message {
                    self.history.push(Entry::Error(message.clone()));
                    self.hint_login(&message);
                }
                let armed = std::mem::take(&mut self.auto_compact_armed);
                self.finish(is_error);
                if !is_error && armed {
                    self.auto_compact();
                }
                self.resume_allowed();
            }
            AgentEvent::Failed(message) => {
                self.history.push(Entry::Error(message.clone()));
                self.hint_login(&message);
                self.finish(true);
            }
        }
    }

    /// Sincroniza com o modo em que o agente disse estar. Se ele não for o
    /// pedido (o provedor recusou ou trocou), o terminal passa a mostrar o
    /// modo real; uma troca feita com Shift+Tab durante a resposta é mantida
    /// para a próxima mensagem.
    /// O Yolo não é sincronizado: o agente roda pedindo permissão e o terra
    /// segue em Yolo, aprovando tudo.
    fn sync_mode(&mut self, reported: ReportedMode) {
        if self.requested_mode == Some(PermissionMode::Yolo) {
            self.agent_mode = Some(PermissionMode::Yolo);
            return;
        }
        let mode = match reported {
            ReportedMode::Known(mode) if mode != PermissionMode::Yolo => mode,
            ReportedMode::Unknown(name) => {
                self.history.push(Entry::Info(format!(
                    "O agente está no modo \"{name}\", que este terminal não conhece."
                )));
                self.agent_mode = None;
                return;
            }
            _ => {
                self.agent_mode = None;
                return;
            }
        };
        self.agent_mode = Some(mode);
        if self
            .requested_mode
            .is_some_and(|requested| requested != mode)
        {
            self.permission_mode = mode;
            self.history.push(Entry::Info(format!(
                "O agente está no modo {}.",
                mode.label()
            )));
        }
    }

    /// Troca o gasto do turno em andamento no consumo do chat.
    fn count_turn(&mut self, tokens: u64, cache_tokens: u64) {
        let (spent, cache_spent) = self.turn_spent;
        self.tokens_used = self.tokens_used + tokens - spent;
        self.cache_tokens_used = self.cache_tokens_used + cache_tokens - cache_spent;
        self.turn_spent = (tokens, cache_tokens);
    }

    fn finish(&mut self, is_error: bool) {
        self.run = None;
        self.turn_spent = (0, 0);
        self.prompts.clear_asks();
        self.auto_compact_armed = false;
        self.agent_activity = None;
        self.agent_status = if is_error {
            AgentStatus::Erro
        } else {
            AgentStatus::Ocioso
        };
    }

    /// Limite de contexto do modelo: o informado pelo agente ou o padrão.
    pub fn context_window(&self) -> u64 {
        self.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW)
    }

    /// Tokens em que a conversa deve ser compactada (o ponto da Config).
    pub(super) fn compact_tokens(&self) -> u64 {
        self.context_settings
            .compact_tokens(self.context_window())
            .max(1)
    }

    /// Com a auto compactação ligada, limpa a conversa quando o contexto chega
    /// no ponto de compactar, como o `/clear`, mas a resposta que acabou de
    /// chegar fica na tela. Só no fim da resposta a uma mensagem do usuário, e
    /// nunca com uma permissão pendente, que ainda precisa da sessão.
    fn auto_compact(&mut self) {
        let full = self.context.is_some_and(|t| t >= self.compact_tokens());
        let pending = self.prompts.has_local() || !self.allowed.is_empty();
        if self.context_settings.auto_compact && full && !pending {
            self.end_session();
            self.history.push(Entry::Info(format!(
                "O contexto chegou em {}% do limite: a próxima mensagem começa uma conversa nova.",
                self.context_settings.compact_at
            )));
        }
    }
}
