//! [Graphify](https://github.com/Graphify-Labs/graphify): transforma o projeto
//! num grafo de conhecimento que o agente consulta em vez de reler os arquivos.
//!
//! No agente, ela é o par de hooks `PreToolUse` que o `graphify claude install`
//! grava: antes de uma busca ou de uma leitura, o `graphify hook-guard` lembra
//! o agente de consultar o grafo, se o projeto tiver um. O relatório vem do
//! `graphify-out` do workspace.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

use super::{find_program, version};
use crate::extension::{Contribution, Extension, Hook, Report, Stat, StatValue, Status};

/// A pasta que o graphify grava no workspace (a pasta do terra).
const OUTPUT: &str = "graphify-out";

pub struct Graphify {
    /// Executável; `None` quando não está instalado.
    bin: Option<PathBuf>,
    version: Option<String>,
}

impl Graphify {
    /// Procura o `graphify` no `PATH`.
    pub fn detect() -> Self {
        let bin = find_program("graphify");
        let version = bin.as_deref().and_then(version);
        Self { bin, version }
    }
}

impl Extension for Graphify {
    fn id(&self) -> &'static str {
        "graphify"
    }

    fn name(&self) -> &'static str {
        "Graphify"
    }

    fn description(&self) -> &'static str {
        "Grafo de conhecimento do projeto, que o agente consulta antes de ler os arquivos"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/Graphify-Labs/graphify"
    }

    fn status(&self) -> Status {
        match self.bin {
            Some(_) => Status::Installed {
                version: self.version.clone(),
            },
            None => Status::Missing,
        }
    }

    /// Os hooks rodam pelo `PATH` do agente, que é o do terra. Sem um grafo no
    /// projeto, eles não dizem nada.
    fn contribution(&self) -> Result<Contribution, String> {
        let hook = |matcher: &str, kind: &str| Hook {
            event: "PreToolUse".to_string(),
            matcher: matcher.to_string(),
            command: format!("graphify hook-guard {kind}"),
        };
        Ok(Contribution {
            hooks: vec![hook("Bash|Grep", "search"), hook("Read|Glob", "read")],
            ..Contribution::default()
        })
    }

    fn load_report(&self) -> Option<Receiver<Result<Report, String>>> {
        self.bin.as_ref()?;
        let version = self.version.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(read_report(Path::new(OUTPUT), version.as_deref()));
        });
        Some(rx)
    }
}

/// O relatório a partir da pasta `dir` do graphify.
fn read_report(dir: &Path, version: Option<&str>) -> Result<Report, String> {
    let mut stats = match fs::read_to_string(dir.join("graph.json")) {
        Ok(graph) => {
            let cost = fs::read_to_string(dir.join("cost.json")).ok();
            parse_graph(&graph, cost.as_deref())?
        }
        Err(_) => vec![Stat::new(
            "Grafo",
            StatValue::Text("não gerado (rode /graphify . no agente)".to_string()),
        )],
    };
    if let Some(version) = version {
        stats.push(Stat::new("Versão", StatValue::Text(version.to_string())));
    }
    Ok(Report {
        stats,
        ..Report::default()
    })
}

/// O tamanho do grafo (`graph.json`) e o que a extração gastou (`cost.json`,
/// quando existe e é válido).
fn parse_graph(graph: &str, cost: Option<&str>) -> Result<Vec<Stat>, String> {
    let graph: Value =
        serde_json::from_str(graph).map_err(|err| format!("grafo inválido: {err}"))?;
    let length = |key: &str| graph[key].as_array().map_or(0, Vec::len) as u64;
    let communities: BTreeSet<u64> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| node["community"].as_u64())
        .collect();
    let mut stats = vec![
        Stat::new("Nós", StatValue::Count(length("nodes"))),
        Stat::new("Arestas", StatValue::Count(length("links"))),
        Stat::new("Comunidades", StatValue::Count(communities.len() as u64)),
    ];
    let cost = cost.and_then(|cost| serde_json::from_str::<Value>(cost).ok());
    if let Some(cost) = cost {
        let tokens = |key: &str| cost[key].as_u64().unwrap_or(0);
        stats.push(Stat::new(
            "Tokens na extração",
            StatValue::Tokens(tokens("total_input_tokens") + tokens("total_output_tokens")),
        ));
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contribution_is_the_search_and_read_hooks() {
        let graphify = Graphify {
            bin: Some(PathBuf::from("graphify")),
            version: Some("0.9.94".into()),
        };
        assert_eq!(
            graphify.status(),
            Status::Installed {
                version: Some("0.9.94".into())
            }
        );
        let contribution = graphify.contribution().unwrap();
        let hook = |matcher: &str, command: &str| Hook {
            event: "PreToolUse".into(),
            matcher: matcher.into(),
            command: command.into(),
        };
        assert_eq!(
            contribution.hooks,
            [
                hook("Bash|Grep", "graphify hook-guard search"),
                hook("Read|Glob", "graphify hook-guard read"),
            ]
        );
        assert!(contribution.mcp_servers.is_empty());
        assert!(contribution.launchers.is_empty());

        let missing = Graphify {
            bin: None,
            version: None,
        };
        assert_eq!(missing.status(), Status::Missing);
        assert!(missing.load_report().is_none());
    }

    #[test]
    fn report_has_the_graph_size_and_the_extraction_cost() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("graph.json"),
            r#"{"nodes": [{"community": 0}, {"community": 0}, {"community": 3}, {}],
                "links": [{}, {}]}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("cost.json"),
            r#"{"total_input_tokens": 1000, "total_output_tokens": 500}"#,
        )
        .unwrap();
        let report = read_report(dir.path(), Some("0.9.94")).unwrap();
        assert_eq!(
            report.stats,
            [
                Stat::new("Nós", StatValue::Count(4)),
                Stat::new("Arestas", StatValue::Count(2)),
                Stat::new("Comunidades", StatValue::Count(2)),
                Stat::new("Tokens na extração", StatValue::Tokens(1500)),
                Stat::new("Versão", StatValue::Text("0.9.94".into())),
            ]
        );
        assert!(report.daily.is_empty());
    }

    #[test]
    fn report_without_a_graph_says_how_to_generate_it() {
        let dir = tempfile::tempdir().unwrap();
        let report = read_report(dir.path(), None).unwrap();
        assert_eq!(report.stats.len(), 1);
        assert_eq!(report.stats[0].label, "Grafo");
    }

    #[test]
    fn report_ignores_a_broken_cost_but_not_a_broken_graph() {
        let stats = parse_graph(r#"{"nodes": [], "links": []}"#, Some("não é json")).unwrap();
        assert_eq!(stats.len(), 3);
        assert!(parse_graph("não é json", None).is_err());
    }
}
