//! A lista do hub para pessoas: as instâncias separadas por workspace,
//! usada pela aba Orquestrador e pelo `terra status`.

use super::protocol::{Health, Instance, now};

/// As instâncias de um workspace.
pub struct Workspace<'a> {
    pub name: &'a str,
    pub instances: Vec<&'a Instance>,
}

impl Workspace<'_> {
    /// As instâncias vivas (`alive` ou `stale`).
    pub fn active(&self) -> usize {
        self.instances
            .iter()
            .filter(|i| i.health != Health::Gone)
            .count()
    }

    /// Os sub agentes das instâncias vivas.
    pub fn agents(&self) -> usize {
        self.instances
            .iter()
            .filter(|i| i.health != Health::Gone)
            .map(|i| i.status.agents)
            .sum()
    }

    /// Ex.: `3 instâncias ativas + 2 sub agentes`.
    pub fn summary(&self) -> String {
        let active = self.active();
        let agents = self.agents();
        let instances = plural(active, "instância ativa", "instâncias ativas");
        match agents {
            0 => instances,
            n => format!("{instances} + {}", plural(n, "sub agente", "sub agentes")),
        }
    }
}

/// Separa as instâncias por workspace, na ordem em que cada workspace
/// apareceu.
pub fn by_workspace(instances: &[Instance]) -> Vec<Workspace<'_>> {
    let mut groups: Vec<Workspace> = Vec::new();
    for instance in instances {
        match groups.iter_mut().find(|g| g.name == instance.workspace) {
            Some(group) => group.instances.push(instance),
            None => groups.push(Workspace {
                name: &instance.workspace,
                instances: vec![instance],
            }),
        }
    }
    groups
}

/// A linha de uma instância: id curto, pid, saúde, o que faz, chats,
/// tempo aberta e último sinal.
pub fn instance_line(instance: &Instance) -> String {
    let now = now();
    let doing = match (
        &instance.health,
        instance.status.running,
        &instance.status.task,
    ) {
        (Health::Gone, ..) => format!("saiu ({})", instance.reason.as_deref().unwrap_or("crash")),
        (_, true, Some(task)) => task.clone(),
        (_, true, None) => "rodando".into(),
        (_, false, _) => "parado".into(),
    };
    format!(
        "{}  pid {:<7} {:<6} {:<28} {}  aberto há {}  sinal há {}",
        instance.short_id(),
        instance.pid,
        instance.health.as_str(),
        truncate(&doing, 28),
        plural(instance.status.chats, "chat", "chats"),
        duration(now.saturating_sub(instance.started)),
        duration(now.saturating_sub(instance.last_seen)),
    )
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Ex.: `42s`, `5m`, `2h10m`.
pub fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::protocol::Status;

    fn instance(workspace: &str, health: Health, agents: usize) -> Instance {
        Instance {
            id: "0123456789".into(),
            pid: 1,
            app: String::new(),
            cwd: String::new(),
            workspace: workspace.into(),
            started: 0,
            last_seen: 0,
            health,
            reason: None,
            status: Status {
                agents,
                chats: agents + 1,
                ..Status::default()
            },
        }
    }

    #[test]
    fn instances_are_grouped_by_workspace_with_their_sub_agents() {
        let instances = vec![
            instance("a", Health::Alive, 1),
            instance("b", Health::Alive, 0),
            instance("a", Health::Stale, 0),
            instance("a", Health::Alive, 1),
            instance("a", Health::Gone, 4),
        ];
        let groups = by_workspace(&instances);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "a");
        assert_eq!(groups[0].summary(), "3 instâncias ativas + 2 sub agentes");
        assert_eq!(groups[1].summary(), "1 instância ativa");
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(300), "5m");
        assert_eq!(duration(7800), "2h10m");
    }
}
