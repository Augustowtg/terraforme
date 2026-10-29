//! A lista do hub para pessoas: as instâncias separadas por workspace,
//! usada pela aba Orquestrador e pelo `terra status`.

use super::protocol::{Health, Instance, Tab, now};

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

/// O que a instância está fazendo: a tarefa, `rodando`, `parado` ou,
/// quando saiu, `saiu (exit|crash)`.
pub fn doing(instance: &Instance) -> String {
    match (
        &instance.health,
        instance.status.running,
        &instance.status.task,
    ) {
        (Health::Gone, ..) => format!("saiu ({})", instance.reason.as_deref().unwrap_or("crash")),
        (_, true, Some(task)) => task.clone(),
        (_, true, None) => "rodando".into(),
        (_, false, _) => "parado".into(),
    }
}

/// A linha de uma instância: id curto, pid, saúde, o que faz, chats,
/// tempo aberta e último sinal.
pub fn instance_line(instance: &Instance) -> String {
    let now = now();
    format!(
        "{}  pid {:<7} {:<6} {:<28} {}  aberto há {}  sinal há {}",
        instance.short_id(),
        instance.pid,
        instance.health.as_str(),
        truncate(&doing(instance), 28),
        plural(instance.status.chats, "chat", "chats"),
        duration(now.saturating_sub(instance.started)),
        duration(now.saturating_sub(instance.last_seen)),
    )
}

/// A linha de uma aba de uma janela no `terra status`: o título e o que o
/// agente dela está fazendo (ex.: `Chat 2       parado`).
pub fn tab_line(tab: &Tab) -> String {
    let doing = tab.task.as_deref().unwrap_or("parado");
    format!("{:<12} {}", truncate(&tab.title, 12), truncate(doing, 40))
}

/// As colunas de uma instância na aba Orquestrador: o que faz, as sessões
/// (os chats) e há quanto tempo está aberta (ex.: `parado`, `1 sessão`,
/// `10 min`).
pub fn instance_columns(instance: &Instance) -> [String; 3] {
    [
        truncate(&doing(instance), 28),
        plural(instance.status.chats, "sessão", "sessões"),
        elapsed(now().saturating_sub(instance.started)),
    ]
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

/// Ex.: `42 s`, `10 min`, `2h10`.
pub fn elapsed(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{}h{:02}", secs / 3600, secs % 3600 / 60),
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
    fn tab_lines_show_what_the_agent_does() {
        let tab = Tab {
            id: "1".into(),
            title: "Principal".into(),
            task: None,
        };
        assert_eq!(tab_line(&tab), "Principal    parado");
        let busy = Tab {
            task: Some("Usando Bash...".into()),
            ..tab
        };
        assert_eq!(tab_line(&busy), "Principal    Usando Bash...");
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(300), "5m");
        assert_eq!(duration(7800), "2h10m");
        assert_eq!(elapsed(42), "42 s");
        assert_eq!(elapsed(600), "10 min");
        assert_eq!(elapsed(7800), "2h10");
    }

    #[test]
    fn instance_columns_hide_the_pid_and_count_sessions() {
        let mut one = instance("a", Health::Alive, 0);
        one.started = now() - 600;
        assert_eq!(instance_columns(&one), ["parado", "1 sessão", "10 min"]);
        let gone = instance("a", Health::Gone, 2);
        assert_eq!(instance_columns(&gone)[..2], ["saiu (crash)", "3 sessões"]);
    }
}
