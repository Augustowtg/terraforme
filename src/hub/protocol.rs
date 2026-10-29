//! O protocolo do hub: NDJSON, um objeto JSON por linha, com o tipo em `t`.
//!
//! ```jsonc
//! // instância → hub
//! {"t":"hello","proto":1,"role":"instance","id":"7f3a…","pid":4812,"app":"0.9.0",
//!  "cwd":"/proj/x","workspace":"x","args":["cli"],"started":1730120000,
//!  "status":{"state":"idle","task":null,"chats":1,"agents":0}}
//! {"t":"heartbeat"}
//! {"t":"status","state":"running","task":"Pensando...","chats":2,"agents":1,
//!  "tabs":[{"id":"9c1e…","title":"Principal","kind":"manual","task":"Pensando..."},
//!          {"id":"41d0…","title":"Chat 2","kind":"manual","task":null}]}
//! {"t":"bye","reason":"exit"}
//!
//! // hub → instância
//! {"t":"welcome","proto":1,"hub":"0.9.0"}
//! {"t":"reject","reason":"proto_mismatch","action":"restart_hub"}
//!
//! // monitor → hub
//! {"t":"hello","proto":1,"role":"monitor"}
//! {"t":"subscribe","topic":"instances"}
//!
//! // hub → monitor
//! {"t":"snapshot","instances":[ … ]}
//! {"t":"instance_added","instance":{ … }}
//! {"t":"instance_updated","id":"7f3a…","changes":{"status":{ … }}}
//! {"t":"instance_removed","id":"7f3a…","reason":"exit"}
//! ```

use std::io::{self, BufRead, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::MAX_LINE;

/// O estado de uma instância que ela mesma manda.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Algum chat com o agente trabalhando.
    pub running: bool,
    /// O que o agente está fazendo (o do primeiro chat que está rodando).
    pub task: Option<String>,
    /// Quantos chats estão abertos (o principal e os do `Alt+N`).
    pub chats: usize,
    /// Os sub agentes: os chats além do principal.
    pub agents: usize,
    /// As abas com agente desta janela: o chat principal e os do `Alt+N`.
    pub tabs: Vec<Tab>,
}

/// Uma aba com agente numa janela do terra (um chat).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tab {
    /// Um id aleatório, criado com o chat ([`new_id`]).
    pub id: String,
    /// O título na barra de chats (`Principal`, `Chat 2`…).
    pub title: String,
    /// O que o agente desta aba está fazendo; `None` quando parado.
    pub task: Option<String>,
}

impl Tab {
    fn to_json(&self) -> Value {
        // Toda aba de hoje é manual: o usuário a abriu e conversa nela.
        json!({"id": self.id, "title": self.title, "kind": "manual", "task": self.task})
    }

    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_str()?.to_string(),
            title: value["title"].as_str().unwrap_or_default().to_string(),
            task: value["task"].as_str().map(str::to_string),
        })
    }
}

impl Status {
    pub fn to_json(&self) -> Value {
        json!({
            "state": if self.running { "running" } else { "idle" },
            "task": self.task,
            "chats": self.chats,
            "agents": self.agents,
            "tabs": self.tabs.iter().map(Tab::to_json).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(value: &Value) -> Self {
        Self {
            running: value["state"] == "running",
            task: value["task"].as_str().map(str::to_string),
            chats: value["chats"].as_u64().unwrap_or(0) as usize,
            agents: value["agents"].as_u64().unwrap_or(0) as usize,
            tabs: value["tabs"]
                .as_array()
                .map(|tabs| tabs.iter().filter_map(Tab::from_json).collect())
                .unwrap_or_default(),
        }
    }
}

/// A saúde de uma instância, vista pelo hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Conectada e mandando sinais.
    Alive,
    /// Sem nenhum sinal há mais de [`super::STALE_AFTER`]: travou?
    Stale,
    /// A conexão fechou; fica na lista por [`super::GONE_KEEP`].
    Gone,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Alive => "alive",
            Health::Stale => "stale",
            Health::Gone => "gone",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "stale" => Health::Stale,
            "gone" => Health::Gone,
            _ => Health::Alive,
        }
    }
}

/// Uma instância como o monitor a vê (o objeto do `snapshot` e do
/// `instance_added`, com as `changes` aplicadas).
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub id: String,
    pub pid: u64,
    pub app: String,
    pub cwd: String,
    /// O nome do workspace (o salvo ou o da pasta).
    pub workspace: String,
    /// Quando a instância abriu (segundos Unix).
    pub started: u64,
    /// O último sinal que o hub recebeu (segundos Unix).
    pub last_seen: u64,
    pub health: Health,
    /// Por que saiu (`exit`, `crash`), quando `gone`.
    pub reason: Option<String>,
    pub status: Status,
}

impl Instance {
    pub fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
        Some(Self {
            id: value["id"].as_str()?.to_string(),
            pid: value["pid"].as_u64().unwrap_or(0),
            app: text("app"),
            cwd: text("cwd"),
            workspace: text("workspace"),
            started: value["started"].as_u64().unwrap_or(0),
            last_seen: value["last_seen"].as_u64().unwrap_or(0),
            health: Health::parse(value["health"].as_str().unwrap_or_default()),
            reason: value["reason"].as_str().map(str::to_string),
            status: Status::from_json(&value["status"]),
        })
    }

    /// O id curto, para a tela.
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

/// Aplica as `changes` de um `instance_updated` ao objeto da instância.
pub fn merge(instance: &mut Value, changes: &Value) {
    if let (Some(target), Some(changes)) = (instance.as_object_mut(), changes.as_object()) {
        for (key, value) in changes {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// O `hello` de uma instância.
pub fn hello_instance(id: &str, workspace: &str, cwd: &str, status: &Status) -> Value {
    json!({
        "t": "hello",
        "proto": super::PROTO,
        "role": "instance",
        "id": id,
        "pid": std::process::id(),
        "app": super::APP_VERSION,
        "cwd": cwd,
        "workspace": workspace,
        "args": std::env::args().skip(1).collect::<Vec<_>>(),
        "started": now(),
        "status": status.to_json(),
    })
}

/// O `status` de uma instância: o estado inteiro, não só o que mudou.
pub fn status_message(status: &Status) -> Value {
    let mut msg = Map::new();
    msg.insert("t".into(), "status".into());
    if let Value::Object(fields) = status.to_json() {
        msg.extend(fields);
    }
    Value::Object(msg)
}

/// Escreve `msg` numa linha. Uma mensagem acima de [`MAX_LINE`] não sai: o
/// outro lado a recusaria.
pub fn send(out: &mut impl Write, msg: &Value) -> io::Result<()> {
    let mut line = msg.to_string();
    if line.len() >= MAX_LINE {
        return Err(too_long());
    }
    line.push('\n');
    out.write_all(line.as_bytes())?;
    out.flush()
}

/// Lê a próxima linha como JSON. `Ok(None)` quando a conexão fechou; uma
/// linha que não é JSON vira `{}` (o `t` vazio é ignorado), e uma acima de
/// [`MAX_LINE`] é um erro (quem lê fecha a conexão).
pub fn recv(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut line = Vec::new();
    let limit = MAX_LINE as u64 + 1;
    if input.take(limit).read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE {
        return Err(too_long());
    }
    Ok(Some(
        serde_json::from_slice(&line).unwrap_or_else(|_| json!({})),
    ))
}

fn too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "linha acima de 1 MB")
}

/// Agora, em segundos Unix.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Um id novo, aleatório (o pid não serve: o SO reaproveita pids).
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Sem aleatoriedade do SO: o pid e a hora, que bastam numa máquina.
        let seed = (u128::from(std::process::id()) << 64)
            ^ SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
        bytes = seed.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_and_merges_into_the_instance() {
        let status = Status {
            running: true,
            task: Some("Pensando...".into()),
            chats: 3,
            agents: 2,
            tabs: vec![Tab {
                id: "9c1e".into(),
                title: "Principal".into(),
                task: Some("Pensando...".into()),
            }],
        };
        assert_eq!(Status::from_json(&status.to_json()), status);
        assert_eq!(status.to_json()["tabs"][0]["kind"], "manual");
        let msg = status_message(&status);
        assert_eq!(msg["t"], "status");
        assert_eq!(msg["state"], "running");

        let mut value = json!({"id": "abcdef0123", "health": "alive", "status": {}});
        merge(
            &mut value,
            &json!({"status": status.to_json(), "health": "stale"}),
        );
        let instance = Instance::from_json(&value).unwrap();
        assert_eq!(instance.status, status);
        assert_eq!(instance.health, Health::Stale);
        assert_eq!(instance.short_id(), "abcdef01");
    }

    #[test]
    fn ids_are_random_hex() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}

#[cfg(test)]
mod limits {
    use super::*;

    #[test]
    fn lines_above_one_megabyte_are_refused_both_ways() {
        let big = json!({"t": "status", "task": "x".repeat(MAX_LINE)});
        assert!(send(&mut Vec::new(), &big).is_err());

        let mut line = big.to_string().into_bytes();
        line.push(b'\n');
        let err = recv(&mut line.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let ok = b"{\"t\":\"heartbeat\"}\n";
        assert_eq!(recv(&mut &ok[..]).unwrap().unwrap()["t"], "heartbeat");
    }
}
