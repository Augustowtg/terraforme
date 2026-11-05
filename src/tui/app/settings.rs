//! Configurações do workspace vistas pela tela principal: extensões, valores da
//! aba Config e o modelo padrão escolhido no chat.

use super::App;
use crate::settings::{AgentSettings, ExtensionSettings};
use crate::tui::config;

impl App {
    /// A conta das extensões: o provedor e a conta do agente (ex.:
    /// `claude:.claude-trabalho`; a Default fica sem nada depois do `:`).
    pub(super) fn extension_account(&self) -> String {
        format!(
            "{}:{}",
            self.chat.provider_id().unwrap_or_default(),
            self.settings.account.as_deref().unwrap_or_default()
        )
    }

    /// Lê do banco as extensões da conta do agente e as aplica. Na primeira
    /// vez da conta, ela fica com as do `config.json` do workspace, de antes
    /// de as extensões irem para o banco.
    pub(super) fn load_extensions(&mut self) {
        if let Some(db) = &self.db {
            let legacy = self
                .config
                .as_deref()
                .map(ExtensionSettings::load_file)
                .unwrap_or_default();
            match db.extension_settings(&self.extension_account(), &legacy) {
                Ok(extensions) => self.extensions = extensions,
                Err(err) => self.chat.warn(format!("Extensões: {err:#}")),
            }
        }
        self.apply_extensions();
    }

    /// Grava no banco as extensões que mudaram de modo desde `previous`.
    pub(super) fn save_extensions(&mut self, previous: &ExtensionSettings) {
        let Some(db) = &self.db else {
            return;
        };
        let account = self.extension_account();
        let mut ids = previous.active();
        ids.extend(self.extensions.active());
        for id in ids {
            let mode = self.extensions.mode(&id);
            if mode != previous.mode(&id)
                && let Err(err) = db.set_extension(&account, &id, mode)
            {
                // Sem conseguir salvar, a mudança vale só nesta execução.
                self.chat.warn(format!("Extensões: {err:#}"));
                return;
            }
        }
    }

    /// Passa aos chats o que as extensões ativas acrescentam ao agente (vale
    /// a partir da próxima mensagem); as que falharem viram avisos.
    pub(super) fn apply_extensions(&mut self) {
        let (contribution, warnings) = self.registry.contribution(&self.extensions.active());
        for tab in &mut self.chats {
            tab.chat.set_extensions(contribution.clone());
        }
        self.chat.set_extensions(contribution);
        for warning in warnings {
            self.chat.warn(format!("Extensão {warning}"));
        }
    }

    /// Opções da Config: contas do provedor, modelos do catálogo do chat e
    /// as extensões do registro.
    pub(super) fn choices(&self) -> config::Choices {
        config::Choices {
            providers: self.providers.clone(),
            provider: self.chat.provider_id().map(str::to_string),
            accounts: self
                .chat
                .adapter()
                .map(|a| a.accounts())
                .unwrap_or_default(),
            models: self.chat.model_options(),
            catalog: self.chat.catalog().cloned(),
            context_window: self.chat.context_window(),
            extensions: self
                .registry
                .all()
                .map(|e| config::ExtensionOption {
                    id: e.id().to_string(),
                    name: e.name().to_string(),
                    description: e.description().to_string(),
                    homepage: e.homepage().to_string(),
                    status: e.status(),
                })
                .collect(),
            skills: self.skills(),
        }
    }

    /// As skills instaladas no agente, para a seção Skills da Config: as das
    /// pastas do provedor e, depois, as que só o catálogo do agente informa
    /// (ex.: de uma pasta que o terra não conhece).
    fn skills(&self) -> Vec<config::SkillInfo> {
        let Some(adapter) = self.chat.adapter() else {
            return Vec::new();
        };
        let mut skills: Vec<config::SkillInfo> = std::env::current_dir()
            .map(|cwd| adapter.skills(&cwd, self.settings.account.as_deref()))
            .unwrap_or_default()
            .into_iter()
            .map(|skill| config::SkillInfo {
                name: skill.name,
                origin: format!("{} · {}", adapter.name(), skill.scope),
                description: skill.description,
            })
            .collect();
        let commands = self.chat.catalog().map_or(&[][..], |c| &c.commands[..]);
        let mut builtin: Vec<config::SkillInfo> = commands
            .iter()
            .filter(|c| c.skill && !skills.iter().any(|s| s.name == c.name))
            .map(|c| config::SkillInfo {
                name: c.name.clone(),
                origin: format!("{} · catálogo", adapter.name()),
                description: c.description.clone(),
            })
            .collect();
        builtin.sort_by(|a, b| a.name.cmp(&b.name));
        skills.extend(builtin);
        skills
    }

    /// Salva as configurações e as aplica ao chat.
    pub(super) fn save_settings(&mut self, previous: &AgentSettings) {
        let provider = self.chat.provider_id().map(str::to_string);
        if let Some(config) = &self.config {
            // Sem conseguir salvar, a mudança vale só nesta execução.
            let _ = self.settings.save_file(config);
        }
        self.chat.apply_settings(&self.settings, Some(previous));
        for tab in &mut self.chats {
            tab.chat.apply_settings(&self.settings, Some(previous));
        }
        // Outra conta (ou provedor): as extensões passam a ser as dela.
        if self.settings.account != previous.account
            || self.chat.provider_id() != provider.as_deref()
        {
            self.load_extensions();
        }
    }

    /// O `/model` escolheu um novo modelo padrão: vai para as configurações.
    pub(super) fn take_default_model(&mut self) {
        if let Some((model, effort)) = self.chat.take_default_choice() {
            let previous = self.settings.clone();
            self.settings.model = Some(model);
            self.settings.effort = effort;
            self.save_settings(&previous);
        }
    }
}
