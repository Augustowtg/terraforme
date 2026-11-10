//! O editor do `orchestrator.toml`, usado pela aba Orquestrador.
//!
//! Ele mexe no documento com o `toml_edit`, que mantém os comentários e a
//! ordem do arquivo, e só grava o que passa na validação de
//! [`Config::parse`]: uma mudança inválida não chega ao disco (e o hub nunca
//! a vê). A gravação é atômica (temporário + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, Value};

use super::config::Config;

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
}

/// `key = value` na tabela, mantendo o comentário que a linha já tinha (o
/// `toml_edit` o guarda no valor).
fn put(table: &mut Table, key: &str, mut value: Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(value);
}

/// Um número para o TOML.
pub fn number(n: u32) -> Value {
    Value::from(i64::from(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::config::portable;

    const FILE: &str = "# meus agentes\n[global]\nmax_hops = 5 # cuidado com laços\n\n[agents.api]\ncwd = \"/proj/api\" # a pasta\n";

    fn editor(text: &str) -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orchestrator.toml");
        fs::write(&path, text).unwrap();
        let editor = Editor::open(&path).unwrap();
        (dir, editor)
    }

    #[test]
    fn edits_keep_the_comments_and_the_file_stays_valid() {
        let (dir, mut editor) = editor(&portable(FILE));
        editor.set_global("max_hops", number(7));
        editor.set_global("pausado", true);
        let config = editor.save().unwrap();
        assert_eq!(config.global.max_hops, 7);
        assert!(config.global.paused);
        assert!(config.agent("api").is_some());

        let text = fs::read_to_string(dir.path().join("orchestrator.toml")).unwrap();
        assert!(text.starts_with("# meus agentes\n"));
        assert!(text.contains("# cuidado com laços"));
        assert!(text.contains(&portable(
            "[agents.api]\ncwd = \"/proj/api\" # a pasta\n"
        )));
    }

    #[test]
    fn an_invalid_change_never_reaches_the_disk() {
        let (dir, mut editor) = editor(FILE);
        editor.set_global("modo_execucao", "janela");
        assert!(editor.save().unwrap_err().contains("janela"));
        let text = fs::read_to_string(dir.path().join("orchestrator.toml")).unwrap();
        assert_eq!(text, FILE);
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
