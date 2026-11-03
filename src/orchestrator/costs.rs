//! Os custos para a aba Orquestrador: o uso de cada agente no dia, o custo
//! médio por nível e os alertas de orçamento (aviso a 80%, bloqueio a 100%).

use super::config::Config;
use super::run::{Run, RunState, local_date};

/// O uso de um agente da configuração num dia.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentDay {
    pub agent: String,
    /// As execuções que contam no orçamento (fora as recusadas).
    pub runs: u32,
    /// Entrada e saída (sem o cache, como no consumo do chat).
    pub tokens: u64,
    pub cost_usd: f64,
    pub budget: Option<u32>,
}

impl AgentDay {
    pub fn alert(&self) -> Option<Alert> {
        let limit = self.budget?;
        if self.runs >= limit {
            Some(Alert::Blocked)
        } else if self.runs * 5 >= limit * 4 {
            Some(Alert::Warning)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// 80% do orçamento do dia.
    Warning,
    /// O orçamento acabou: novas mensagens são recusadas.
    Blocked,
}

/// O dia de cada agente da configuração, na ordem do arquivo.
pub fn by_agent(runs: &[Run], config: &Config, day: jiff::civil::Date) -> Vec<AgentDay> {
    config
        .agents
        .iter()
        .map(|agent| {
            let today = runs.iter().filter(|run| {
                run.agent == agent.name
                    && run.state != RunState::Rejected
                    && local_date(run.created) == Some(day)
            });
            let mut usage = AgentDay {
                agent: agent.name.clone(),
                runs: 0,
                tokens: 0,
                cost_usd: 0.0,
                budget: agent.daily_budget,
            };
            for run in today {
                usage.runs += 1;
                if let Some(u) = &run.usage {
                    usage.tokens += u.input + u.output;
                    usage.cost_usd += u.cost_usd;
                }
            }
            usage
        })
        .collect()
}

/// O custo médio das execuções terminadas de cada nível (1, 2 e 3); `None`
/// sem nenhuma.
pub fn average_by_level(runs: &[Run]) -> [Option<f64>; 3] {
    let mut sums = [(0.0, 0u32); 3];
    for run in runs {
        let (Some(usage), 1..=3) = (&run.usage, run.level) else {
            continue;
        };
        let slot = &mut sums[usize::from(run.level) - 1];
        slot.0 += usage.cost_usd;
        slot.1 += 1;
    }
    sums.map(|(sum, n)| (n > 0).then(|| sum / f64::from(n)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::run::{Usage, today};
    use std::path::PathBuf;

    fn run(agent: &str, level: u8, state: RunState, cost: Option<f64>) -> Run {
        Run {
            id: "1".into(),
            agent: agent.into(),
            message: "m".into(),
            from: "usuario".into(),
            level,
            hops: 1,
            note: PathBuf::new(),
            state,
            created: jiff::Timestamp::now().as_second() as u64,
            started: None,
            finished: None,
            model: None,
            usage: cost.map(|cost_usd| Usage {
                input: 10,
                output: 5,
                cost_usd,
                ..Usage::default()
            }),
            error: None,
            summary: None,
        }
    }

    #[test]
    fn the_day_of_each_agent_and_its_budget_alerts() {
        let config = Config::parse(
            "[agents.api]\ncwd = \"/a\"\norcamento_diario = 5\n[agents.web]\ncwd = \"/w\"\n",
        )
        .unwrap();
        let mut runs: Vec<Run> = (0..4)
            .map(|_| run("api", 1, RunState::Done, Some(0.5)))
            .collect();
        runs.push(run("api", 1, RunState::Rejected, None));
        let days = by_agent(&runs, &config, today());
        assert_eq!(days[0].runs, 4);
        assert_eq!(days[0].tokens, 60);
        assert!((days[0].cost_usd - 2.0).abs() < 1e-9);
        assert_eq!(days[0].alert(), Some(Alert::Warning));
        assert_eq!(days[1].alert(), None, "sem orçamento, sem alerta");

        runs.push(run("api", 1, RunState::Running, None));
        assert_eq!(
            by_agent(&runs, &config, today())[0].alert(),
            Some(Alert::Blocked)
        );
    }

    #[test]
    fn the_average_cost_by_level() {
        let runs = [
            run("api", 1, RunState::Done, Some(0.1)),
            run("api", 1, RunState::Failed, Some(0.3)),
            run("api", 3, RunState::Done, Some(1.0)),
            run("api", 2, RunState::Queued, None),
        ];
        let [one, two, three] = average_by_level(&runs);
        assert!((one.unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(two, None);
        assert!((three.unwrap() - 1.0).abs() < 1e-9);
    }
}
