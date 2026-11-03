use super::*;

#[test]
fn each_mode_has_its_own_color() {
    let variants: Vec<_> = PermissionMode::ALL.into_iter().map(mode_variant).collect();
    for (i, v) in variants.iter().enumerate() {
        assert!(!variants[i + 1..].contains(v), "cor repetida: {v:?}");
    }
}
