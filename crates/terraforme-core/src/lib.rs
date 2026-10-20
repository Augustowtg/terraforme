//! Núcleo do terraforme: toda a lógica fica aqui para ser reutilizada
//! pela CLI.

pub mod accounts;
pub mod agent;
pub mod config;
pub mod extension;
pub mod jail;
pub mod limits;
pub mod mcp;
pub mod skill;
pub mod space;
pub mod status;
pub mod usage;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn greeting(name: &str) -> String {
    format!("Olá, {name}! terraforme v{VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_includes_name_and_version() {
        let msg = greeting("mundo");
        assert!(msg.contains("mundo"));
        assert!(msg.contains(VERSION));
    }
}
