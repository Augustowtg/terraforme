use super::*;

#[test]
fn terra_commands_are_added_once() {
    let login = |description: &str| CommandInfo {
        name: "login".into(),
        description: description.into(),
        skill: false,
    };
    let added = with_terra(Vec::new());
    assert_eq!(added.iter().filter(|c| c.name == "login").count(), 1);
    // Se o agente um dia informar o /login, fica o dele.
    let mine = with_terra(vec![login("do agente")]);
    assert_eq!(mine[0], login("do agente"));
    assert_eq!(mine.iter().filter(|c| c.name == "login").count(), 1);
    // Os do orquestrador também entram.
    assert!(mine.iter().any(|c| c.name == "agente"));
    assert!(mine.iter().any(|c| c.name == "mensagens"));
}

#[test]
fn long_names_do_not_stick_to_the_description() {
    let text = |name: &str| {
        let command = CommandInfo {
            name: name.into(),
            description: "faz algo".into(),
            skill: false,
        };
        item(&command, false)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
    };
    assert!(text("usage").contains("/usage            faz algo"));
    assert!(text("ai-memory-durable-pages").contains("/ai-memory-durable-pages faz algo"));
    assert!(text("install-github-app").contains("/install-github-app faz algo"));
}

fn commands(names: &[(&str, &str)]) -> Vec<CommandInfo> {
    names
        .iter()
        .map(|(name, description)| CommandInfo {
            name: name.to_string(),
            description: description.to_string(),
            skill: false,
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
    // Sem consulta, todos; os com interface (/clear, /compact e /model)
    // primeiro.
    assert_eq!(found(&list, ""), ["clear", "compact", "model", "context"]);
    assert_eq!(found(&list, "co"), ["compact", "context"]);
    // "session" só aparece na descrição do /clear.
    assert_eq!(found(&list, "session"), ["clear"]);
    assert!(found(&list, "xyz").is_empty());
}

#[test]
fn knows_which_commands_have_an_interface() {
    assert!(has_interface("clear"));
    assert!(has_interface("compact"));
    assert!(!has_interface("context"));
}
