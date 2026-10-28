//! A permissão no `agy`: dos chats delegados e do chat normal.
//!
//! O Claude tem o `--permission-prompt-tool`; o `agy` não tem nada parecido
//! (no `-p`, o que pediria permissão é negado sozinho). O que ele tem é o
//! gancho `PreToolUse` dos plugins: antes de cada ferramenta, ele roda um
//! comando, manda a chamada no stdin e lê a decisão (`allow` ou `deny`, com
//! o motivo) no stdout. Então, num chat delegado, o terra:
//!
//! 1. liga o `--dangerously-skip-permissions`, para o `agy` não negar sozinho;
//! 2. monta, numa pasta temporária passada em `--add-dir`, um plugin com um
//!    `hooks.json` que roda `terra agy-permission-hook` antes de cada
//!    ferramenta ([`Gate`]);
//! 3. o gancho ([`hook`]) deixa passar o que só lê e pergunta ao resto, pela
//!    ferramenta `permission` do MCP do terra ([`crate::mcp::ask_permission`]),
//!    a quem delegou, que responde com `decide`. Negado, ou sem resposta, a
//!    ferramenta não roda.
//!
//! No chat normal, nos modos Manual e Accept Edits, é o mesmo gancho, mas
//! quem decide é o usuário do terra, na tela do chat: o gancho leva o pedido
//! por um servidor local do processo do terra ([`channel`]), que o mostra
//! como um [`AgentEvent::PermissionRequest`](crate::agent::AgentEvent), igual
//! ao do Claude, e devolve a resposta do usuário.
//!
//! Sem o gancho (o MCP ou o chat fora do ar, um erro), a decisão é negar. O
//! `agy` mata o gancho depois de 30 s, a não ser que o `hooks.json` diga
//! outro `timeout` (em segundos): o plugin pede mais que os 10 minutos que
//! quem decide espera.
//!
//! No Yolo, o `agy` aprova tudo sem o gancho; no Plan, fica como está (o
//! `agy` nega o que mudaria algo); no Agente, o `agy` decide sozinho (nos
//! chats delegados, o gancho vale no Agente também).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::channel::{self, Channel};
use super::plugin::{make_dir, write_private};
use crate::agent::{AgentEvent, PermissionMode};
use crate::extension::Contribution;
use crate::mcp;

/// O nome do plugin do gancho no `agy`.
const PLUGIN: &str = "terraforme-permission";
/// Segundos que o `agy` espera o gancho: mais que o MCP espera a decisão.
const HOOK_TIMEOUT: u32 = 11 * 60;

/// As ferramentas do `agy` que só leem ou conversam: passam sem perguntar.
const READ_ONLY: &[&str] = &[
    "read_file",
    "list_dir",
    "grep_search",
    "find_by_name",
    "view_file",
    "view_file_outline",
    "view_code_item",
    "command_status",
    "read_terminal",
    "read_url_content",
    "search_web",
    "task_boundary",
    "notify_user",
    "list_resources",
    "read_resource",
];
/// As que editam arquivos: passam no Accept Edits.
const EDITS: &[&str] = &[
    "write_file",
    "write_to_file",
    "replace_file_content",
    "multi_replace_file_content",
    "edit_file",
];

/// O modo do chat delegado usa o gancho: nos outros (Yolo e Plan), o `agy`
/// decide sozinho.
pub fn gated(mode: PermissionMode) -> bool {
    !matches!(mode, PermissionMode::Yolo | PermissionMode::Plan)
}

/// O modo do chat normal em que o usuário decide pelo gancho: o Manual e o
/// Accept Edits. No Agente, o `agy` decide sozinho.
pub fn asks_user(mode: PermissionMode) -> bool {
    matches!(mode, PermissionMode::Manual | PermissionMode::AcceptEdits)
}

/// A quem o gancho leva o pedido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// A quem delegou o chat, pelo MCP do terra: o processo do terra e a
    /// delegação.
    Delegator { pid: u32, chat: u64 },
    /// Ao usuário, no chat do terra, pelo arquivo do [`Channel`].
    User(PathBuf),
}

/// O plugin temporário com o `hooks.json` do gancho; apagado ao soltar.
pub struct Gate {
    dir: PathBuf,
    /// O servidor que leva os pedidos ao chat, quando quem decide é o
    /// usuário. Cai depois de a pasta sumir.
    channel: Option<Channel>,
}

impl Gate {
    /// Monta o plugin para um chat delegado, com o terra e a delegação que a
    /// URL do servidor `terra` das extensões diz. `None` quando o chat não
    /// é delegado.
    pub fn create(
        extensions: &Contribution,
        server: &str,
        mode: PermissionMode,
    ) -> Result<Option<Gate>, String> {
        let url = extensions
            .mcp_servers
            .iter()
            .find(|s| s.name == server)
            .and_then(|s| s.url.as_deref());
        let Some((pid, chat)) = url.and_then(mcp::delegated_ids) else {
            return Ok(None);
        };
        let exe = std::env::current_exe()
            .map_err(|err| format!("o caminho do terra para o gancho do agy: {err}"))?;
        let accept_edits = mode == PermissionMode::AcceptEdits;
        let hooks = hooks_json(&exe, &Route::Delegator { pid, chat }, accept_edits);
        let dir = make_dir().map_err(|err| format!("pasta temporária do gancho do agy: {err}"))?;
        let gate = Gate { dir, channel: None };
        gate.write(&hooks)
            .map_err(|err| format!("gancho de permissão do agy: {err}"))?;
        Ok(Some(gate))
    }

    /// Monta o plugin para o chat normal, em que o usuário decide: abre o
    /// [`Channel`] e o gancho fala com ele.
    pub fn create_for_user(mode: PermissionMode) -> Result<Gate, String> {
        let exe = std::env::current_exe()
            .map_err(|err| format!("o caminho do terra para o gancho do agy: {err}"))?;
        let dir = make_dir().map_err(|err| format!("pasta temporária do gancho do agy: {err}"))?;
        // Já com a pasta no `Gate`: se algo falhar, ela é apagada ao soltar.
        let mut gate = Gate { dir, channel: None };
        gate.channel = Some(
            Channel::open(&gate.dir).map_err(|err| format!("canal de permissão do agy: {err}"))?,
        );
        let route = Route::User(gate.dir.join(channel::FILE));
        let hooks = hooks_json(&exe, &route, mode == PermissionMode::AcceptEdits);
        gate.write(&hooks)
            .map_err(|err| format!("gancho de permissão do agy: {err}"))?;
        Ok(gate)
    }

    /// Os pedidos de permissão do `agy` que o usuário ainda não viu.
    pub fn poll(&self) -> Vec<AgentEvent> {
        self.channel.as_ref().map(Channel::poll).unwrap_or_default()
    }

    /// A decisão do usuário a um pedido.
    pub fn respond(&self, id: &str, allow: bool) {
        if let Some(channel) = &self.channel {
            channel.respond(id, allow);
        }
    }

    /// A pasta do `--add-dir`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn write(&self, hooks: &Value) -> std::io::Result<()> {
        let plugin = self.dir.join(".agents").join("plugins").join(PLUGIN);
        fs::create_dir_all(&plugin)?;
        fs::write(
            plugin.join("plugin.json"),
            json!({ "name": PLUGIN }).to_string(),
        )?;
        write_private(&plugin.join("hooks.json"), &hooks.to_string())
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// O `hooks.json` do plugin: antes de qualquer ferramenta, o terra decide.
fn hooks_json(exe: &Path, route: &Route, accept_edits: bool) -> Value {
    let accept = if accept_edits { " --accept-edits" } else { "" };
    let target = match route {
        Route::Delegator { pid, chat } => format!("--from {pid} --chat {chat}"),
        Route::User(file) => format!("--channel {}", quote(file)),
    };
    let command = format!("{} agy-permission-hook {target}{accept}", quote(exe));
    json!({
        PLUGIN: {
            "PreToolUse": [{
                "matcher": "",
                "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT }],
            }],
        },
    })
}

/// O caminho do terra na linha de comando do gancho: entre aspas se tiver
/// espaço.
fn quote(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.contains(char::is_whitespace) {
        format!("\"{path}\"")
    } else {
        path.into_owned()
    }
}

/// O que o gancho faz com uma ferramenta.
#[derive(Debug, PartialEq, Eq)]
enum Policy {
    /// Passa sem perguntar.
    Allow,
    /// Pergunta a quem delegou.
    Ask,
}

fn policy(tool: &str, accept_edits: bool) -> Policy {
    if READ_ONLY.contains(&tool) || (accept_edits && EDITS.contains(&tool)) {
        Policy::Allow
    } else {
        Policy::Ask
    }
}

/// O comando `terra agy-permission-hook`: o gancho `PreToolUse` do `agy`.
/// Lê a chamada no stdin e escreve a decisão no stdout. Sempre sai com 0:
/// qualquer problema vira uma negativa com o motivo.
pub fn hook(route: &Route, accept_edits: bool) {
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let decision = decide(&stdin, accept_edits, |tool, input| match route {
        Route::Delegator { pid, chat } => {
            mcp::ask_permission(*pid, *chat, tool, input).map_err(|err| format!("{err:#}"))
        }
        Route::User(file) => channel::ask(file, tool, input),
    });
    println!("{decision}");
}

/// A decisão para a chamada `stdin` (o JSON do `agy`); `ask` pergunta a quem
/// delegou. Separada do [`hook`] para os testes.
fn decide(
    stdin: &str,
    accept_edits: bool,
    ask: impl FnOnce(&str, &Value) -> Result<Result<(), String>, String>,
) -> Value {
    let deny = |reason: String| json!({ "decision": "deny", "reason": reason });
    let Ok(call) = serde_json::from_str::<Value>(stdin) else {
        return deny("o terra não entendeu a chamada da ferramenta".into());
    };
    let Some(tool) = call["toolCall"]["name"].as_str() else {
        return deny("a chamada não diz a ferramenta".into());
    };
    if policy(tool, accept_edits) == Policy::Allow {
        return json!({ "decision": "allow" });
    }
    match ask(tool, &call["toolCall"]["args"]) {
        Ok(Ok(())) => json!({ "decision": "allow" }),
        Ok(Err(reason)) => deny(reason),
        Err(err) => deny(format!("o terra não conseguiu perguntar: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::McpServer;

    fn call(tool: &str) -> String {
        json!({ "toolCall": { "name": tool, "args": { "CommandLine": "ls" } } }).to_string()
    }

    fn never(_: &str, _: &Value) -> Result<Result<(), String>, String> {
        panic!("não devia perguntar");
    }

    #[test]
    fn reads_pass_and_the_rest_asks_the_delegator() {
        assert_eq!(
            decide(&call("read_file"), false, never)["decision"],
            "allow"
        );
        let asked = decide(&call("run_command"), false, |tool, input| {
            assert_eq!(tool, "run_command");
            assert_eq!(input["CommandLine"], "ls");
            Ok(Ok(()))
        });
        assert_eq!(asked["decision"], "allow");
        let denied = decide(&call("run_command"), false, |_, _| Ok(Err("não".into())));
        assert_eq!(denied, json!({ "decision": "deny", "reason": "não" }));
    }

    #[test]
    fn edits_pass_only_in_accept_edits() {
        assert_eq!(
            decide(&call("write_file"), true, never)["decision"],
            "allow"
        );
        let manual = decide(&call("write_file"), false, |_, _| Ok(Err("não".into())));
        assert_eq!(manual["decision"], "deny");
        // Rodar um comando continua perguntando.
        let command = decide(&call("run_command"), true, |_, _| Ok(Err("não".into())));
        assert_eq!(command["decision"], "deny");
    }

    #[test]
    fn a_problem_denies() {
        let broken = decide("isto não é json", false, never);
        assert_eq!(broken["decision"], "deny");
        let offline = decide(&call("run_command"), false, |_, _| Err("sem MCP".into()));
        assert_eq!(offline["decision"], "deny");
        assert!(offline["reason"].as_str().unwrap().contains("sem MCP"));
        assert_eq!(decide("{}", false, never)["decision"], "deny");
    }

    #[test]
    fn only_the_modes_that_ask_use_the_hook() {
        assert!(gated(PermissionMode::Manual));
        assert!(gated(PermissionMode::AcceptEdits));
        assert!(gated(PermissionMode::Agente));
        assert!(!gated(PermissionMode::Yolo));
        assert!(!gated(PermissionMode::Plan));
    }

    #[test]
    fn the_gate_writes_a_private_plugin_for_delegated_chats_only() {
        let delegated = Contribution {
            mcp_servers: vec![McpServer {
                name: "terra".into(),
                url: Some(mcp::delegated_url(7, 3)),
                ..Default::default()
            }],
            ..Default::default()
        };
        let gate = Gate::create(&delegated, "terra", PermissionMode::AcceptEdits)
            .unwrap()
            .unwrap();
        let dir = gate.dir().to_path_buf();
        let plugin = dir.join(".agents/plugins").join(PLUGIN);
        let hooks: Value =
            serde_json::from_str(&fs::read_to_string(plugin.join("hooks.json")).unwrap()).unwrap();
        let entry = &hooks[PLUGIN]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "");
        let command = entry["hooks"][0]["command"].as_str().unwrap();
        assert!(command.contains("agy-permission-hook --from 7 --chat 3 --accept-edits"));
        assert_eq!(entry["hooks"][0]["timeout"], HOOK_TIMEOUT);
        drop(gate);
        assert!(!dir.exists());

        // O agente do terra (sem `chat` na URL) não é delegado.
        let main = Contribution {
            mcp_servers: vec![McpServer {
                name: "terra".into(),
                url: Some(mcp::agent_url(7)),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            Gate::create(&main, "terra", PermissionMode::Manual)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn only_manual_and_accept_edits_ask_the_user() {
        assert!(asks_user(PermissionMode::Manual));
        assert!(asks_user(PermissionMode::AcceptEdits));
        assert!(!asks_user(PermissionMode::Agente));
        assert!(!asks_user(PermissionMode::Yolo));
        assert!(!asks_user(PermissionMode::Plan));
    }

    #[test]
    fn the_user_gate_points_the_hook_at_its_channel() {
        let gate = Gate::create_for_user(PermissionMode::Manual).unwrap();
        let dir = gate.dir().to_path_buf();
        let plugin = dir.join(".agents/plugins").join(PLUGIN);
        let hooks: Value =
            serde_json::from_str(&fs::read_to_string(plugin.join("hooks.json")).unwrap()).unwrap();
        let entry = &hooks[PLUGIN]["PreToolUse"][0];
        let command = entry["hooks"][0]["command"].as_str().unwrap();
        let file = dir.join(channel::FILE);
        assert!(command.contains(&format!("agy-permission-hook --channel {}", file.display())));
        assert!(!command.contains("--accept-edits") && !command.contains("--from"));
        assert_eq!(entry["hooks"][0]["timeout"], HOOK_TIMEOUT);
        assert!(file.exists());

        // O gancho (aqui, `channel::ask`) fala com o chat; o usuário decide.
        let hook = {
            let file = file.clone();
            std::thread::spawn(move || channel::ask(&file, "run_command", &json!({})))
        };
        let id = loop {
            if let Some(AgentEvent::PermissionRequest { id, name, .. }) = gate.poll().pop() {
                assert_eq!(name, "run_command");
                break id;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        gate.respond(&id, true);
        assert_eq!(hook.join().unwrap(), Ok(Ok(())));

        drop(gate);
        assert!(!dir.exists());
        assert!(channel::ask(&file, "run_command", &json!({})).is_err());
    }

    #[test]
    fn accept_edits_goes_to_the_user_hook_command() {
        let gate = Gate::create_for_user(PermissionMode::AcceptEdits).unwrap();
        let hooks = fs::read_to_string(
            gate.dir()
                .join(".agents/plugins")
                .join(PLUGIN)
                .join("hooks.json"),
        )
        .unwrap();
        assert!(hooks.contains("--accept-edits"));
    }

    #[test]
    fn delegated_ids_come_from_the_url() {
        assert_eq!(
            mcp::delegated_ids(&mcp::delegated_url(12, 5)),
            Some((12, 5))
        );
        assert_eq!(mcp::delegated_ids(&mcp::agent_url(12)), None);
    }
}
