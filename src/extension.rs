//! Extensões: aplicações secundárias que melhoram o agente do chat (ex.: o
//! RTK, que enxuga a saída dos comandos, e o ai-memory, a memória de longo
//! prazo entre sessões).
//!
//! Cada extensão é uma estratégia (padrão Strategy) que implementa
//! [`Extension`]: diz se está instalada, o que acrescenta ao agente
//! ([`Contribution`]: hooks e servidores MCP) e o que mostrar na aba
//! Extensões ([`Report`]). O [`Registry`] conhece todas
//! ([`extensions::all`]) e junta o que as escolhidas no workspace
//! acrescentam; o adapter do agente traduz isso para o protocolo dele.

mod contract;
pub mod extensions;
mod registry;

pub use contract::{
    Contribution, Extension, Hook, Launcher, McpServer, Report, Stat, StatValue, Status,
};
pub use registry::Registry;
