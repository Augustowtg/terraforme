//! Modelo do chat: o catálogo do provedor, o menu do `/model` e os rótulos.

use std::sync::mpsc::TryRecvError;

use super::Chat;
use super::model_menu::{self, Target, effort_label};
use crate::agent::{self, Catalog};
use crate::tui::history::Entry;
use crate::tui::input::commands;

impl Chat {
    /// Começa a carregar os comandos e modelos do agente (da conta atual).
    /// Sem provedor disponível, não há comandos.
    pub(super) fn load_catalog(&mut self) {
        self.catalog = None;
        self.catalog_loading = self
            .available_adapter()
            .and_then(|adapter| adapter.load_catalog(self.account.as_deref()));
        let loading = self.catalog_loading.is_some();
        self.input.set_commands((!loading).then(Vec::new));
    }

    /// Recebe o catálogo, quando ele chega.
    pub(super) fn poll_catalog(&mut self) {
        let Some(loading) = &self.catalog_loading else {
            return;
        };
        let catalog = match loading.try_recv() {
            Err(TryRecvError::Empty) => return,
            Ok(Ok(catalog)) => catalog,
            // Sem o catálogo, a lista fica vazia (o chat continua funcionando).
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => Catalog::default(),
        };
        self.catalog_loading = None;
        self.input
            .set_commands(Some(commands::with_terra(catalog.commands.clone())));
        self.catalog = Some(catalog);
        if let Some(menu) = &mut self.model_menu {
            menu.sync(self.catalog.as_ref());
        }
    }

    /// Aplica a escolha do menu do `/model`.
    pub(super) fn apply_model_menu(&mut self, outcome: model_menu::Outcome) {
        let (target, model, effort) = match outcome {
            model_menu::Outcome::Handled => return,
            // O Default só zera o modelo dos sub agentes, que é da Config.
            model_menu::Outcome::Close | model_menu::Outcome::Unset(_) => {
                self.model_menu = None;
                return;
            }
            model_menu::Outcome::Chosen {
                target,
                model,
                effort,
            } => (target, model, effort),
        };
        self.model_menu = None;
        let label = self.model_label_of(&model);
        let effort_text = effort
            .as_deref()
            .map_or(String::new(), |e| format!(" · esforço {}", effort_label(e)));
        match target {
            Target::Session => {
                self.history.push(Entry::Info(format!(
                    "Modelo desta conversa: {label}{effort_text}."
                )));
                // O título já mostra o escolhido; o agente confirma na resposta.
                self.model = self.resolved_model(&model).or(Some(model.clone()));
                self.session_model = Some(model);
                self.session_effort = effort;
            }
            Target::Default => {
                self.history.push(Entry::Info(format!(
                    "Modelo default: {label}{effort_text} (salvo nas Configurações)."
                )));
                // A conversa passa a usar o novo padrão.
                self.session_model = None;
                self.session_effort = None;
                self.model = self.resolved_model(&model);
                self.default_choice = Some((model, effort));
            }
            // Só a Config abre o menu dos sub agentes.
            Target::Subagent => {}
        }
        self.history.follow();
    }

    /// O novo modelo padrão escolhido no `/model`, uma vez, para a tela
    /// principal salvar nas configurações.
    pub fn take_default_choice(&mut self) -> Option<(String, Option<String>)> {
        self.default_choice.take()
    }

    #[cfg(test)]
    pub fn default_choice_for_test(&mut self, choice: (String, Option<String>)) {
        self.default_choice = Some(choice);
    }

    /// O nome real de um modelo do catálogo (ex.: `opus` → `claude-opus-5-5`).
    fn resolved_model(&self, id: &str) -> Option<String> {
        self.catalog
            .as_ref()?
            .models
            .iter()
            .find(|m| m.id == id)?
            .resolved
            .clone()
    }

    /// Nome de um modelo para a tela (ex.: `Opus - 5.5`).
    pub fn model_label_of(&self, id: &str) -> String {
        let info = self.catalog.as_ref().and_then(|c| {
            c.models
                .iter()
                .find(|m| m.id == id || m.resolved.as_deref() == Some(id))
        });
        match (info, &self.adapter) {
            (Some(info), Some(adapter)) => info
                .resolved
                .as_deref()
                .map_or_else(|| info.label.clone(), |r| adapter.model_label(r)),
            (Some(info), None) => info.label.clone(),
            (None, _) => id.to_string(),
        }
    }

    /// O catálogo do agente, quando já carregou (para o menu de modelos da
    /// Config).
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref()
    }

    /// Modelos para a Config: os do catálogo, ou os fixos do provedor
    /// enquanto o catálogo não chega.
    pub fn model_options(&self) -> Vec<agent::ModelOption> {
        match &self.catalog {
            Some(catalog) if !catalog.models.is_empty() => catalog
                .models
                .iter()
                .map(|m| agent::ModelOption {
                    id: m.id.clone(),
                    label: self.model_label_of(&m.id),
                })
                .collect(),
            _ => self
                .adapter
                .as_ref()
                .map(|a| a.models())
                .unwrap_or_default(),
        }
    }
}
