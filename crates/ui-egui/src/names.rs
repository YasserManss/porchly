//! People type names like "Guest List"; links and the API use `guest_list`. These turn one
//! into the other.

/// Turns a friendly name into an identifier: lowercase, spaces and punctuation become `_`.
/// Returns `None` when nothing usable is left (for example only symbols).
pub fn to_ident(label: &str) -> Option<String> {
    porchly_core::Ident::from_label(label).map(|i| i.as_str().to_owned())
}

/// Turns an identifier back into a friendly name: `guest_list` → "Guest list".
pub fn pretty(ident: &str) -> String {
    let spaced = ident.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_names_become_identifiers() {
        assert_eq!(to_ident("Guest List").as_deref(), Some("guest_list"));
        assert_eq!(to_ident("  Mom's Recipes!! ").as_deref(), Some("mom_s_recipes"));
        assert_eq!(to_ident("2026 Budget").as_deref(), Some("t_2026_budget"));
        assert_eq!(to_ident("Café").as_deref(), Some("caf"));
        assert_eq!(to_ident("!!!"), None);
        assert_eq!(to_ident(""), None);
        assert!(
            to_ident(&"word ".repeat(40))
                .is_some_and(|s| s.len() <= porchly_core::limits::MAX_IDENT_LEN && !s.ends_with('_'))
        );
    }

    #[test]
    fn identifiers_become_friendly() {
        assert_eq!(pretty("guest_list"), "Guest list");
        assert_eq!(pretty(""), "");
    }
}
