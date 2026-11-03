use super::*;

fn names(query: &str) -> Vec<&'static str> {
    matches(query).into_iter().map(|p| p.name).collect()
}

#[test]
fn empty_query_lists_every_pattern_in_order() {
    assert_eq!(
        names(""),
        PATTERNS.iter().map(|p| p.name).collect::<Vec<_>>()
    );
}

#[test]
fn query_matches_name_then_description() {
    assert_eq!(names("asd"), ["ASD-STE100"]);
    assert_eq!(names("ste")[0], "ASD-STE100");
    // Pela descrição: "Diminui o retorno".
    assert_eq!(names("retorno"), ["ASD-STE100"]);
    assert!(names("xyz").is_empty());
}

#[test]
fn find_ignores_case() {
    assert_eq!(find("asd-ste100").map(|p| p.name), Some("ASD-STE100"));
    assert_eq!(find("ASD"), None);
}

#[test]
fn item_shows_name_and_description() {
    let pattern = find("ASD-STE100").unwrap();
    let text: String = item(pattern, false)
        .spans
        .iter()
        .map(|s| s.content.to_string())
        .collect();
    assert!(text.contains("&ASD-STE100     - Diminui o retorno"));
}
