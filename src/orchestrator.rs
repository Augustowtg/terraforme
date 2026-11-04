//! O orquestrador de agentes: as mensagens entre os agentes (`terra send`),
//! as notas de handoff (`terra handoff new`) e a configuração
//! (`~/.terraforme/orchestrator.toml`).
//!
//! Quem leva as mensagens é o hub ([`crate::hub`]). Um agente é uma aba com
//! agente de uma janela do terra (um **agente manual**, com o nome dado no
//! `/agente`) ou um agente da configuração (`[agents.<nome>]`, um **agente
//! despachado**, que roda numa execução nova para cada mensagem).
//!
//! Toda mensagem vira uma nota: nos níveis 1 (só o texto) e 2 (texto e
//! `--ref`), o terra escreve uma mini nota; no nível 3, a nota é a do
//! `--handoff`. Daí em diante, o caminho é um só. O documento completo está
//! em `docs/infra/orquestrador.md`.

pub mod config;
pub mod costs;
pub mod edit;
pub mod message;
pub mod note;
pub mod run;
pub mod send;

/// O nome do agente que está rodando: o terra o põe no ambiente do agente
/// de uma aba com nome, e o `terra send` o usa como remetente.
pub const AGENT_ENV: &str = "TERRA_AGENT";
/// Quantos saltos a mensagem que gerou esta execução já deu: o `terra send`
/// de dentro dela manda com um a mais.
pub const HOPS_ENV: &str = "TERRA_HOPS";
/// A execução despachada que está rodando: o `terra done` de dentro dela
/// conclui essa execução.
pub const RUN_ENV: &str = "TERRA_RUN";
/// O remetente quando o `terra send` não roda dentro de um agente.
pub const USER: &str = "usuario";

/// O remetente e os saltos de quem roda o `terra send`, do ambiente.
pub fn sender() -> (String, u32) {
    let from = std::env::var(AGENT_ENV)
        .ok()
        .filter(|name| config::valid_name(name))
        .unwrap_or_else(|| USER.to_string());
    let hops = std::env::var(HOPS_ENV)
        .ok()
        .and_then(|hops| hops.parse().ok())
        .unwrap_or(0);
    (from, hops)
}
