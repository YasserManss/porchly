use std::fmt;
use std::str::FromStr;

use crate::{Error, Result};

/// The type of a field. Kept small and friendly; the UI shows the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Integer,
    Number,
    Boolean,
    Datetime,
    /// The `id` of a row in another table (which table is part of the field's definition).
    Link,
}

impl FieldKind {
    pub const ALL: [FieldKind; 6] = [
        FieldKind::Text,
        FieldKind::Integer,
        FieldKind::Number,
        FieldKind::Boolean,
        FieldKind::Datetime,
        FieldKind::Link,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Integer => "integer",
            FieldKind::Number => "number",
            FieldKind::Boolean => "boolean",
            FieldKind::Datetime => "datetime",
            FieldKind::Link => "link",
        }
    }

    /// The label shown to people in the UI.
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Text => "Text",
            FieldKind::Integer => "Whole number",
            FieldKind::Number => "Number",
            FieldKind::Boolean => "Yes / No",
            FieldKind::Datetime => "Date and time",
            FieldKind::Link => "Link to another table",
        }
    }

    /// The SQLite column type.
    pub fn sql_type(self) -> &'static str {
        match self {
            FieldKind::Text | FieldKind::Datetime => "TEXT",
            FieldKind::Integer | FieldKind::Boolean | FieldKind::Link => "INTEGER",
            FieldKind::Number => "REAL",
        }
    }
}

impl FromStr for FieldKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        FieldKind::ALL.into_iter().find(|k| k.as_str() == s).ok_or_else(|| {
            let names: Vec<&str> = FieldKind::ALL.iter().map(|k| k.as_str()).collect();
            Error::invalid(format!("\"{s}\" isn't a field type. Use one of: {}.", names.join(", ")))
        })
    }
}

impl fmt::Display for FieldKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for k in FieldKind::ALL {
            assert_eq!(k.as_str().parse::<FieldKind>(), Ok(k));
        }
        assert!("blob".parse::<FieldKind>().is_err());
    }
}
