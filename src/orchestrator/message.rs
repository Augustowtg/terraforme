//! Uma mensagem entre agentes: quem manda, para quem, o nível, os saltos
//! e a nota. Cada uma fica em `~/.terraforme/messages/<msg_id>/`, com a
//! nota (`note.md`) e o estado (`message.json`, gravado pelo hub).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::config::Global;

/// Onde as mensagens ficam, na pasta do terra.
pub const DIR: &str = "messages";

/// O estado de uma mensagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// O hub a mandou à janela da aba manual, que ainda não confirmou.
    Sent,
    /// A janela confirmou (`ack`): está na aba, esperando o usuário.
    Delivered,
    /// Na fila do agente despachado.
    Queued,
    /// A execução do agente despachado está rodando.
    Running,
    /// A execução terminou bem.
    Done,
    /// A execução falhou (ou estourou o tempo).
    Failed,
    Rejected,
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            State::Sent => "enviada",
            State::Delivered => "entregue",
            State::Queued => "na_fila",
            State::Running => "em_execucao",
            State::Done => "concluida",
            State::Failed => "falhou",
            State::Rejected => "recusada",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        [
            State::Sent,
            State::Delivered,
            State::Queued,
            State::Running,
            State::Done,
            State::Failed,
            State::Rejected,
        ]
        .into_iter()
        .find(|state| state.key() == key)
    }
}

/// Por que uma mensagem foi recusada: um código curto e o texto para quem
/// mandou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    pub code: &'static str,
    pub text: String,
}

impl Reject {
    pub fn new(code: &'static str, text: impl Into<String>) -> Self {
        Self {
            code,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    /// 1: só o texto; 2: texto e referências; 3: handoff completo.
    pub level: u8,
    /// Quantas vezes a conversa já passou de um agente a outro (a primeira
    /// mensagem tem 1).
    pub hops: u32,
    pub tag: Option<String>,
    pub text: Option<String>,
    /// Os arquivos do `--ref`, absolutos.
    pub refs: Vec<PathBuf>,
    /// A nota da mensagem (a mini nota ou a cópia do `--handoff`).
    pub note: PathBuf,
    /// Quando foi criada (segundos Unix).
    pub created: u64,
}

impl Message {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "from": self.from,
            "to": self.to,
            "level": self.level,
            "hops": self.hops,
            "tag": self.tag,
            "text": self.text,
            "refs": self.refs,
            "note": self.note,
            "created": self.created,
        })
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value[key].as_str().map(str::to_string);
        Some(Self {
            id: text("id")?,
            from: text("from")?,
            to: text("to")?,
            level: u8::try_from(value["level"].as_u64()?).ok()?,
            hops: u32::try_from(value["hops"].as_u64()?).ok()?,
            tag: text("tag"),
            text: text("text"),
            refs: value["refs"]
                .as_array()
                .map(|refs| {
                    refs.iter()
                        .filter_map(Value::as_str)
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            note: PathBuf::from(text("note")?),
            created: value["created"].as_u64().unwrap_or(0),
        })
    }

    /// As regras que o `terra send` confere antes de mandar e o hub de novo
    /// ao receber: os saltos e o tamanho do texto.
    pub fn check(&self, global: &Global) -> Result<(), Reject> {
        if self.hops > global.max_hops {
            return Err(Reject::new(
                "hops",
                format!(
                    "a mensagem já deu {} saltos (máximo {}): possível laço entre agentes",
                    self.hops, global.max_hops
                ),
            ));
        }
        let words = self.text.as_deref().map_or(0, words);
        if words > global.word_limit as usize {
            return Err(Reject::new(
                "palavras",
                format!(
                    "o texto tem {words} palavras (máximo {}): escreva uma nota com \
                     `terra handoff new` e mande com --handoff",
                    global.word_limit
                ),
            ));
        }
        if !self.note.is_file() {
            return Err(Reject::new(
                "arquivo",
                format!("a nota não existe: {}", self.note.display()),
            ));
        }
        Ok(())
    }
}

pub fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// A pasta de uma mensagem.
pub fn dir(home: &Path, id: &str) -> PathBuf {
    home.join(DIR).join(id)
}

/// Grava o estado da mensagem no `message.json` dela (temporário + rename).
pub fn save(
    message: &Message,
    state: State,
    reason: Option<&Reject>,
    home: &Path,
) -> io::Result<()> {
    let dir = dir(home, &message.id);
    fs::create_dir_all(&dir)?;
    let mut value = message.to_json();
    value["state"] = state.key().into();
    value["reason"] = reason.map_or(Value::Null, |r| json!({"code": r.code, "text": r.text}));
    let tmp = dir.join("message.json.tmp");
    fs::write(&tmp, format!("{value:#}\n"))?;
    fs::rename(&tmp, dir.join("message.json"))
}

/// A mensagem gravada (o `message.json`).
pub fn load(home: &Path, id: &str) -> Option<Message> {
    let text = fs::read_to_string(dir(home, id).join("message.json")).ok()?;
    Message::from_json(&serde_json::from_str(&text).ok()?)
}

/// O estado gravado de uma mensagem.
#[cfg(test)]
pub fn state(home: &Path, id: &str) -> Option<State> {
    let text = fs::read_to_string(dir(home, id).join("message.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    State::parse(value["state"].as_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn message(note: PathBuf) -> Message {
        Message {
            id: "ab12".into(),
            from: "usuario".into(),
            to: "backend".into(),
            level: 1,
            hops: 1,
            tag: Some("testes".into()),
            text: Some("Arrume o teste".into()),
            refs: vec![PathBuf::from("/proj/a.rs")],
            note,
            created: 1,
        }
    }

    #[test]
    fn round_trips_and_saves_its_state() {
        let home = tempfile::tempdir().unwrap();
        let msg = message(home.path().join("note.md"));
        assert_eq!(Message::from_json(&msg.to_json()), Some(msg.clone()));
        save(&msg, State::Sent, None, home.path()).unwrap();
        assert_eq!(state(home.path(), "ab12"), Some(State::Sent));
        save(&msg, State::Delivered, None, home.path()).unwrap();
        assert_eq!(state(home.path(), "ab12"), Some(State::Delivered));
    }

    #[test]
    fn too_many_hops_or_words_or_a_missing_note_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let note = home.path().join("note.md");
        fs::write(&note, "# Handoff").unwrap();
        let global = Global::default();
        let ok = message(note.clone());
        ok.check(&global).unwrap();

        let looping = Message {
            hops: 6,
            ..ok.clone()
        };
        assert_eq!(looping.check(&global).unwrap_err().code, "hops");

        let long = Message {
            text: Some("palavra ".repeat(501)),
            ..ok.clone()
        };
        let err = long.check(&global).unwrap_err();
        assert_eq!(err.code, "palavras");
        assert!(err.text.contains("terra handoff new"));

        let lost = Message {
            note: home.path().join("sumiu.md"),
            ..ok
        };
        assert_eq!(lost.check(&global).unwrap_err().code, "arquivo");
    }
}
