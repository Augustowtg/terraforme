//! Comandos com `/` no input.
//!
//! A lista vem do agente (o catálogo do adapter: [`CommandInfo`]); a função
//! de cada comando é uma interface do terra, implementada no chat. Comandos
//! que o terra ainda não implementa aparecem esmaecidos e, se escolhidos,
//! avisam no histórico.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::agent::CommandInfo;
use crate::theme::{self, Variant};
use crate::tui::widgets::popup;

/// Comandos que já têm interface no terra.
const INTERFACES: &[&str] = &["clear", "model", "login"];
/// Comandos do terra que o agente não informa (o `/login` do Claude Code só
/// existe no modo interativo dele): nome e descrição.
const TERRA_COMMANDS: &[(&str, &str)] = &[(
    "login",
    "Entra de novo na conta do agente (ex.: token expirado ou revogado)",
)];
/// Largura da coluna do nome (`/nome`).
const NAME_WIDTH: usize = 18;

pub fn has_interface(name: &str) -> bool {
    INTERFACES.contains(&name)
}

/// Os comandos do agente mais os do terra que ele não informou.
pub fn with_terra(mut commands: Vec<CommandInfo>) -> Vec<CommandInfo> {
    for (name, description) in TERRA_COMMANDS {
        if !commands.iter().any(|c| c.name == *name) {
            commands.push(CommandInfo {
                name: name.to_string(),
                description: description.to_string(),
            });
        }
    }
    commands
}

/// Comandos que combinam com `query`: começo do nome, parte do nome e parte
/// da descrição, nessa ordem; os que têm interface vêm antes.
pub fn matches<'a>(commands: &'a [CommandInfo], query: &str) -> Vec<&'a CommandInfo> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, bool, &CommandInfo)> = commands
        .iter()
        .filter_map(|command| {
            let name = command.name.to_lowercase();
            let rank = if name.starts_with(&query) {
                0
            } else if name.contains(&query) {
                1
            } else if command.description.to_lowercase().contains(&query) {
                2
            } else {
                return None;
            };
            Some((rank, !has_interface(&command.name), command))
        })
        .collect();
    found.sort_by(|a, b| (a.0, a.1, &a.2.name).cmp(&(b.0, b.1, &b.2.name)));
    found.into_iter().map(|(_, _, command)| command).collect()
}

#[cfg(test)]
#[test]
fn terra_commands_are_added_once() {
    let login = |description: &str| CommandInfo {
        name: "login".into(),
        description: description.into(),
    };
    let added = with_terra(Vec::new());
    assert_eq!(added.iter().filter(|c| c.name == "login").count(), 1);
    // Se o agente um dia informar o /login, fica o dele.
    assert_eq!(with_terra(vec![login("do agente")]), [login("do agente")]);
}

/// Desenha a lista acima de `input`. `None` em `found` é o catálogo ainda
/// carregando.
pub fn draw(frame: &mut Frame, input: Rect, found: Option<&[&CommandInfo]>, selected: usize) {
    let Some(found) = found else {
        popup::draw_above(
            frame,
            input,
            "Comandos".to_string(),
            Vec::new(),
            0,
            "Carregando os comandos do agente...",
        );
        return;
    };
    let items = found
        .iter()
        .enumerate()
        .map(|(i, command)| item(command, i == selected))
        .collect();
    popup::draw_above(
        frame,
        input,
        format!("Comandos ({})", found.len()),
        items,
        selected,
        "Nenhum comando",
    );
}

fn item(command: &CommandInfo, selected: bool) -> Line<'static> {
    let ready = has_interface(&command.name);
    let name = format!("/{:<width$}", command.name, width = NAME_WIDTH - 1);
    let mut description = command.description.clone();
    if !ready {
        description = format!("{description} · sem interface no terra");
    }
    if selected {
        return Line::from(vec![
            popup::marker(true),
            Span::raw(name),
            Span::raw(description),
        ])
        .style(theme::focused());
    }
    let (name_style, description_style) = if ready {
        (theme::variant(Variant::Active), theme::path())
    } else {
        (theme::muted(), theme::muted())
    };
    Line::from(vec![
        popup::marker(false),
        Span::styled(name, name_style),
        Span::styled(description, description_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(names: &[(&str, &str)]) -> Vec<CommandInfo> {
        names
            .iter()
            .map(|(name, description)| CommandInfo {
                name: name.to_string(),
                description: description.to_string(),
            })
            .collect()
    }

    fn found(list: &[CommandInfo], query: &str) -> Vec<String> {
        matches(list, query)
            .into_iter()
            .map(|c| c.name.clone())
            .collect()
    }

    #[test]
    fn filters_by_name_then_description_with_interfaces_first() {
        let list = commands(&[
            ("compact", "Free up context by summarizing"),
            ("context", "Show current context usage"),
            ("clear", "Start a new session"),
            ("model", "Set the AI model"),
        ]);
        // Sem consulta, todos; os com interface (/clear e /model) primeiro.
        assert_eq!(found(&list, ""), ["clear", "model", "compact", "context"]);
        assert_eq!(found(&list, "co"), ["compact", "context"]);
        // "session" só aparece na descrição do /clear.
        assert_eq!(found(&list, "session"), ["clear"]);
        assert!(found(&list, "xyz").is_empty());
    }

    #[test]
    fn knows_which_commands_have_an_interface() {
        assert!(has_interface("clear"));
        assert!(!has_interface("compact"));
    }
}
