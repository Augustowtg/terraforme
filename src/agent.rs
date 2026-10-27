//! O agente por trás do chat.
//!
//! O chat conversa com qualquer provedor (route) pelo [`ChatAdapter`]: cada
//! provedor é um adapter em [`adapters`] (hoje, o Claude CLI). A interface só
//! conhece o contrato daqui: [`ChatAdapter`], [`Session`], [`Request`],
//! [`AgentEvent`] e [`PermissionMode`].

mod adapter;
pub mod adapters;
mod event;
mod mode;

pub use adapter::{
    AccountOption, Catalog, ChatAdapter, CommandInfo, ModelInfo, ModelOption, Request, Session,
    Usage, UsageWindow,
};
pub use event::{AgentEvent, ReportedMode};
pub use mode::PermissionMode;
