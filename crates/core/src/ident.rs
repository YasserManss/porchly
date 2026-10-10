use std::fmt;

use crate::{Error, Result, limits};

/// A validated table or field name: `^[a-z][a-z0-9_]{0,62}$`, not reserved for Porchly
/// (`_porchly…`) or SQLite (`sqlite_…`). Because of these rules an `Ident` is always safe
/// to quote into SQL; values never are, and are always bound as parameters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct Ident(String);

/// Field names every table has; users can't create fields with these names.
pub const SYSTEM_FIELDS: [&str; 3] = ["id", "created_at", "updated_at"];

/// Names that would clash with the API's own query parameters and row keys.
pub const RESERVED_NAMES: [&str; 5] = ["expand", "limit", "offset", "search", "sort"];

/// Table names the API uses for itself (`/api/fn/…` runs functions).
pub const RESERVED_TABLES: [&str; 1] = ["fn"];

/// Whether `name` is one of the [`SYSTEM_FIELDS`].
pub fn is_system_field(name: &str) -> bool {
    SYSTEM_FIELDS.contains(&name)
}

impl Ident {
    /// Validates a table or field name.
    pub fn new(name: &str) -> Result<Self> {
        if name.is_empty() {
            return Err(Error::invalid("A name can't be empty."));
        }
        if name.len() > limits::MAX_IDENT_LEN {
            return Err(Error::invalid(format!(
                "\"{}…\" is too long: names can have at most {} characters.",
                truncate(name, 20),
                limits::MAX_IDENT_LEN
            )));
        }
        let mut chars = name.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        if !first_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(Error::invalid(format!(
                "\"{}\" can't be used as a name. Use lowercase letters, digits and underscores, starting with a letter (for example \"recipes\" or \"guest_list\").",
                truncate(name, 40)
            )));
        }
        if name.starts_with("sqlite_") {
            return Err(Error::invalid(format!("\"{name}\" is reserved. Please choose another name.")));
        }
        Ok(Ident(name.to_owned()))
    }

    /// Validates a name for a user field: also rejects the system fields.
    pub fn field(name: &str) -> Result<Self> {
        let ident = Self::new(name)?;
        if is_system_field(name) {
            return Err(Error::invalid(format!(
                "\"{name}\" is added to every table automatically. Please choose another name."
            )));
        }
        if RESERVED_NAMES.contains(&name) {
            return Err(Error::invalid(format!("\"{name}\" is reserved by the API. Please choose another name.")));
        }
        Ok(ident)
    }

    /// Accepts a system field name too (for sorting and reading `id`, `created_at`, `updated_at`).
    pub fn column(name: &str) -> Result<Self> {
        Self::new(name)
    }

    /// Turns a friendly name ("Guest List", "Phone #") into an identifier ("guest_list",
    /// "phone"): lowercase, other characters become `_`, a leading digit gets `t_`. Returns
    /// `None` when nothing usable is left. The result may still be reserved (see [`Ident::field`]).
    pub fn from_label(label: &str) -> Option<Ident> {
        let mut out = String::new();
        for c in label.trim().chars() {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                out.push(c);
            } else if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
        }
        let out = out.trim_end_matches('_');
        if out.is_empty() {
            return None;
        }
        let mut out = if out.starts_with(|c: char| c.is_ascii_digit()) { format!("t_{out}") } else { out.to_owned() };
        out.truncate(limits::MAX_IDENT_LEN);
        Ident::new(out.trim_end_matches('_')).ok()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The identifier double-quoted for SQL. Safe because of the validation in [`Ident::new`];
    /// quotes are still escaped as defence in depth.
    pub fn quoted(&self) -> String {
        format!("\"{}\"", self.0.replace('"', "\"\""))
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_good_names() {
        for name in ["recipes", "guest_list", "a", "t2", "x_1_y"] {
            assert!(Ident::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_bad_names() {
        let long = "a".repeat(64);
        for name in
            ["", "Recipes", "1abc", "_porchly_keys", "_x", "sqlite_master", "a-b", "a b", "a\"b", "a;drop", "é", &long]
        {
            assert!(Ident::new(name).is_err(), "{name}");
        }
    }

    #[test]
    fn system_fields_are_not_user_fields() {
        assert!(Ident::field("id").is_err());
        assert!(Ident::field("created_at").is_err());
        assert!(Ident::field("expand").is_err());
        assert!(Ident::field("sort").is_err());
        assert!(Ident::column("id").is_ok());
    }

    #[test]
    fn from_friendly_labels() {
        let id = |s: &str| Ident::from_label(s).map(|i| i.as_str().to_owned());
        assert_eq!(id("Guest List").as_deref(), Some("guest_list"));
        assert_eq!(id("  Mom's Recipes!! ").as_deref(), Some("mom_s_recipes"));
        assert_eq!(id("2026 Budget").as_deref(), Some("t_2026_budget"));
        assert_eq!(id("sqlite_x").as_deref(), None, "reserved prefixes stay invalid");
        assert_eq!(id("!!!"), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(Ident::new("recipes").map(|i| i.quoted()), Ok("\"recipes\"".to_owned()));
    }
}
