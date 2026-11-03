use super::*;

fn entries(paths: &[&str]) -> Vec<FileEntry> {
    paths
        .iter()
        .map(|p| FileEntry {
            path: p.trim_end_matches('/').to_string(),
            is_dir: p.ends_with('/'),
        })
        .collect()
}

fn found(paths: &[&str], query: &str) -> Vec<String> {
    let entries = entries(paths);
    matches(&entries, query)
        .into_iter()
        .map(|e| e.path.clone())
        .collect()
}

#[test]
fn tokens_round_trip() {
    for i in [0, 1, 500] {
        assert_eq!(token_index(token(i)), Some(i));
    }
    assert_eq!(token_index('a'), None);
    assert_eq!(token_index('@'), None);
}

#[test]
fn empty_query_lists_shallow_first() {
    assert_eq!(
        found(&["src/", "src/main.rs", "README.md", "src/tui/app.rs"], ""),
        ["src", "README.md", "src/main.rs", "src/tui/app.rs"]
    );
}

#[test]
fn query_prefers_file_name_then_path_then_letters() {
    let paths = [
        "src/tui/app.rs",
        "src/apple.rs",
        "docs/maps.md",
        "a/p/p.txt",
    ];
    assert_eq!(
        found(&paths, "app"),
        ["src/apple.rs", "src/tui/app.rs", "a/p/p.txt"]
    );
    // Sem diferenciar maiúsculas.
    assert_eq!(found(&["README.md"], "read"), ["README.md"]);
    assert!(found(&paths, "xyz").is_empty());
}

#[test]
fn picker_wraps_around() {
    let mut picker = Picker::new(0);
    picker.up(3);
    assert_eq!(picker.selected, 2);
    picker.down(3);
    assert_eq!(picker.selected, 0);
    picker.down(0);
    assert_eq!(picker.selected, 0);
}

#[test]
fn badge_label_and_prompt_text() {
    let file = Mention::File("src/test.js".into());
    assert_eq!(badge_label(&file), "@/src/test.js");
    assert_eq!(badge_width(&file), 15);
    assert_eq!(prompt_text(&file), "@src/test.js");
    let skill = Mention::Skill("docs".into());
    assert_eq!(badge_label(&skill), "/docs");
    assert_eq!(prompt_text(&skill), "/docs");
    let paste = Mention::Paste {
        number: 2,
        text: "a\nb\nc".into(),
    };
    assert_eq!(badge_label(&paste), "[Pasted text #2 +2 lines]");
    assert_eq!(prompt_text(&paste), "a\nb\nc");
    let line = Mention::Paste {
        number: 1,
        text: "x".repeat(150),
    };
    assert_eq!(badge_label(&line), "[Pasted text #1]");
}
