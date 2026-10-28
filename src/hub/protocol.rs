//! O protocolo do hub: NDJSON, um objeto JSON por linha, com o tipo em `t`.
//!
//! ```jsonc
//! // instância → hub
//! {"t":"hello","proto":1,"role":"instance","id":"7f3a…","pid":4812,"app":"0.9.0",
//!  "cwd":"/proj/x","workspace":"x","args":["cli"],"started":1730120000,
//!  "status":{"state":"idle","task":null,"chats":1,"agents":0}}
//! {"t":"heartbeat"}
//! {"t":"status","state":"running","task":"Pensando...","chats":2,"agents":1}
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

use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

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
}

impl Status {
    pub fn to_json(&self) -> Value {
        json!({
            "state": if self.running { "running" } else { "idle" },
            "task": self.task,
            "chats": self.chats,
            "agents": self.agents,
        })
    }

    pub fn from_json(value: &Value) -> Self {
        Self {
            running: value["state"] == "running",
            task: value["task"].as_str().map(str::to_string),
            chats: value["chats"].as_u64().unwrap_or(0) as usize,
            agents: value["agents"].as_u64().unwrap_or(0) as usize,
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

/// Escreve `msg` numa linha.
pub fn send(out: &mut impl Write, msg: &Value) -> io::Result<()> {
    let mut line = msg.to_string();
    line.push('\n');
    out.write_all(line.as_bytes())?;
    out.flush()
}

/// Lê a próxima linha como JSON. `Ok(None)` quando a conexão fechou; uma
/// linha que não é JSON vira `{}` (o `t` vazio é ignorado).
pub fn recv(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_str(&line).unwrap_or_else(|_| json!({})),
    ))
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
        };
        assert_eq!(Status::from_json(&status.to_json()), status);
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
