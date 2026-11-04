//! O editor do `orchestrator.toml`, usado pela aba Orquestrador.
//!
//! Ele mexe no documento com o `toml_edit`, que mantém os comentários e a
//! ordem do arquivo, e só grava o que passa na validação de
//! [`Config::parse`]: uma mudança inválida não chega ao disco (e o hub nunca
//! a vê). A gravação é atômica (temporário + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use super::config::{self, Config};

pub struct Editor {
    path: PathBuf,
    doc: DocumentMut,
}

impl Editor {
    /// Abre o arquivo para editar; sem ele, começa vazio. Um TOML quebrado
    /// é um erro: a aba não mexe num arquivo que não entende.
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(format!("não foi possível ler {}: {err}", path.display())),
        };
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|err| format!("TOML inválido: {err}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    /// Valida e grava. Inválido, nada muda no disco e volta o erro.
    pub fn save(&self) -> Result<Config, String> {
        let text = self.text();
        let config = Config::parse(&text)?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, &text)
            .and_then(|()| fs::rename(&tmp, &self.path))
            .map_err(|err| format!("não foi possível gravar {}: {err}", self.path.display()))?;
        Ok(config)
    }

    /// A tabela `[name]`, criada se preciso. Num arquivo só com comentários,
    /// o `toml_edit` os guarda no fim do documento: a primeira tabela os leva
    /// para cima dela, onde estavam.
    fn table(&mut self, name: &str) -> &mut Table {
        if !self.doc.contains_key(name) {
            let mut table = Table::new();
            let trailing = self.doc.trailing().as_str().unwrap_or_default().to_string();
            if self.doc.is_empty() && !trailing.trim().is_empty() {
                table.decor_mut().set_prefix(trailing);
                self.doc.set_trailing("");
            }
            self.doc.insert(name, Item::Table(table));
        }
        let item = self.doc.entry(name).or_insert(Item::Table(Table::new()));
        if !item.is_table() {
            *item = Item::Table(Table::new());
        }
        item.as_table_mut().expect("é uma tabela")
    }

    /// `[global] key = value`.
    pub fn set_global(&mut self, key: &str, value: impl Into<Value>) {
        put(self.table("global"), key, value.into());
    }

    /// `[roteamento] key = value`; `None` tira a chave.
    pub fn set_routing(&mut self, key: &str, value: Option<Value>) {
        let table = self.table("roteamento");
        match value {
            Some(value) => put(table, key, value),
            None => {
                table.remove(key);
            }
        }
    }

    /// A tabela `[agents.<name>]`, se existe.
    fn agent_table(&mut self, name: &str) -> Option<&mut Table> {
        self.doc
            .get_mut("agents")?
            .as_table_mut()?
            .get_mut(name)?
            .as_table_mut()
    }

    /// Um agente novo, com a pasta. O nome tem que ser válido e livre.
    pub fn add_agent(&mut self, name: &str, cwd: &Path) -> Result<(), String> {
        if !config::valid_name(name) {
            return Err(format!(
                "\"{name}\" não é um nome de agente: letras minúsculas, dígitos, - ou _ (até 32)"
            ));
        }
        if self.agent_table(name).is_some() {
            return Err(format!("já existe o agente {name}"));
        }
        let agents = self.table("agents");
        // Só os `[agents.<nome>]` aparecem, sem um `[agents]` vazio (e o
        // comentário de cima dele passa para o primeiro agente).
        agents.set_implicit(true);
        let mut table = Table::new();
        if let Some(prefix) = agents
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .map(str::to_string)
            && !prefix.trim().is_empty()
        {
            table.decor_mut().set_prefix(prefix);
            agents.decor_mut().set_prefix("");
        }
        table["cwd"] = Item::Value(cwd.to_string_lossy().as_ref().into());
        agents[name] = Item::Table(table);
        Ok(())
    }

    pub fn remove_agent(&mut self, name: &str) {
        if let Some(agents) = self.doc.get_mut("agents").and_then(Item::as_table_mut) {
            agents.remove(name);
        }
    }

    /// `[agents.<name>] key = value`; `None` tira a chave.
    pub fn set_agent(&mut self, name: &str, key: &str, value: Option<Value>) {
        let Some(table) = self.agent_table(name) else {
            return;
        };
        match value {
            Some(value) => put(table, key, value),
            None => {
                table.remove(key);
            }
        }
    }

    /// `[modelos] alias = "id"`.
    pub fn set_model(&mut self, alias: &str, id: &str) {
        put(self.table("modelos"), alias, id.into());
    }

    pub fn remove_model(&mut self, alias: &str) {
        self.table("modelos").remove(alias);
    }

    fn rules(&mut self) -> &mut ArrayOfTables {
        let routing = self.table("roteamento");
        let item = routing
            .entry("regra")
            .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
        if !item.is_array_of_tables() {
            *item = Item::ArrayOfTables(ArrayOfTables::new());
        }
        item.as_array_of_tables_mut()
            .expect("é uma lista de tabelas")
    }

    /// Uma regra `[[roteamento.regra]]` nova no fim.
    pub fn add_rule(&mut self, tag: &str, model: &str) {
        let mut rule = Table::new();
        rule["tag"] = Item::Value(tag.into());
        rule["modelo"] = Item::Value(model.into());
        self.rules().push(rule);
    }

    /// Troca a regra `index` (na ordem do arquivo), no mesmo lugar.
    pub fn set_rule(&mut self, index: usize, tag: &str, model: &str) {
        if let Some(rule) = self.rules().get_mut(index) {
            put(rule, "tag", tag.into());
            put(rule, "modelo", model.into());
        }
    }

    /// Tira a regra `index` (na ordem do arquivo).
    pub fn remove_rule(&mut self, index: usize) {
        let rules = self.rules();
        if index < rules.len() {
            rules.remove(index);
        }
        if rules.is_empty() {
            self.table("roteamento").remove("regra");
        }
    }
}

/// `key = value` na tabela, mantendo o comentário que a linha já tinha (o
/// `toml_edit` o guarda no valor).
fn put(table: &mut Table, key: &str, mut value: Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(value);
}

/// Uma lista de textos para o TOML (as ferramentas).
pub fn texts(items: &[String]) -> Value {
    let mut array = toml_edit::Array::new();
    for item in items {
        array.push(item.as_str());
    }
    Value::Array(array)
}

/// Um número para o TOML.
pub fn number(n: u32) -> Value {
    Value::from(i64::from(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# meus agentes\n[global]\nmax_hops = 5 # cuidado com laços\n";

    fn editor(text: &str) -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orchestrator.toml");
        fs::write(&path, text).unwrap();
        let editor = Editor::open(&path).unwrap();
        (dir, editor)
    }

    #[test]
    fn edits_keep_the_comments_and_the_file_stays_valid() {
        let (dir, mut editor) = editor(FILE);
        editor.set_global("max_hops", number(7));
        editor.set_global("pausado", true);
        editor.add_agent("api", Path::new("/proj/api")).unwrap();
        editor.set_agent("api", "max_turnos", Some(number(30)));
        editor.set_agent(
            "api",
            "ferramentas",
            Some(texts(&["Read".into(), "Bash(npm test:*)".into()])),
        );
        editor.set_model("forte", "opus");
        editor.set_agent("api", "modelo", Some("forte".into()));
        editor.add_rule("testes", "forte");
        let config = editor.save().unwrap();
        assert_eq!(config.global.max_hops, 7);
        assert!(config.global.paused);
        let api = config.agent("api").unwrap();
        assert_eq!(
            (api.max_turns, api.model.as_deref()),
            (Some(30), Some("forte"))
        );
        assert_eq!(config.routing.rules.len(), 1);

        let text = fs::read_to_string(dir.path().join("orchestrator.toml")).unwrap();
        assert!(text.starts_with("# meus agentes\n"));
        assert!(text.contains("# cuidado com laços"));
        assert!(text.contains("[agents.api]"));
        assert!(!text.contains("[agents]\n"), "{text}");
    }

    #[test]
    fn an_invalid_change_never_reaches_the_disk() {
        let (dir, mut editor) = editor(FILE);
        // Uma regra para um modelo que não está em [modelos].
        editor.add_rule("testes", "sumiu");
        assert!(editor.save().unwrap_err().contains("[modelos]"));
        let text = fs::read_to_string(dir.path().join("orchestrator.toml")).unwrap();
        assert_eq!(text, FILE);

        assert!(editor.add_agent("Api", Path::new("/x")).is_err());
        let (_dir, mut editor) = super::tests::editor(FILE);
        editor.add_agent("api", Path::new("/x")).unwrap();
        assert!(editor.add_agent("api", Path::new("/y")).is_err());
    }

    #[test]
    fn a_comment_alone_stays_on_top() {
        let (_dir, mut editor) = editor("# meus agentes\n");
        editor.set_global("pausado", true);
        assert!(
            editor.text().starts_with("# meus agentes\n[global]"),
            "{}",
            editor.text()
        );
        let (_dir, mut editor) = super::tests::editor("# meus agentes\n");
        editor.add_agent("api", Path::new("/p")).unwrap();
        assert!(
            editor.text().starts_with("# meus agentes\n[agents.api]"),
            "{}",
            editor.text()
        );
    }

    #[test]
    fn removing_cleans_up() {
        let (_dir, mut editor) = editor(FILE);
        editor.add_agent("api", Path::new("/p")).unwrap();
        editor.set_model("forte", "opus");
        editor.add_rule("a", "forte");
        editor.add_rule("b", "forte");
        editor.remove_rule(0);
        let config = editor.save().unwrap();
        assert_eq!(config.routing.rules[0].tag, "b");
        editor.remove_rule(0);
        editor.remove_agent("api");
        editor.remove_model("forte");
        let config = editor.save().unwrap();
        assert!(config.agents.is_empty() && config.models.is_empty());
        assert!(!editor.text().contains("regra"));
    }

    #[test]
    fn a_broken_file_is_not_opened_and_a_missing_one_starts_empty() {
        let (_dir, _) = editor("");
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("a.toml");
        fs::write(&broken, "[global").unwrap();
        assert!(Editor::open(&broken).is_err());
        let missing = Editor::open(&dir.path().join("b.toml")).unwrap();
        assert_eq!(missing.text(), "");
    }
}
