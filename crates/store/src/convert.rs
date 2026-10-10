//! Converting a field's existing values when its type changes. Every value either converts
//! cleanly or the change is refused with the rows that need attention; nothing is silently
//! dropped or truncated.

use porchly_core::{FieldKind, Result};
use porchly_schema::{SqlValue, from_query};
use rusqlite::types::ValueRef;

/// A value as text, the way a person would type it. `None` means no value.
pub(crate) fn as_text(old: FieldKind, new: FieldKind, v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null | ValueRef::Blob(_) => None,
        ValueRef::Integer(i) if old == FieldKind::Boolean => {
            let numeric = matches!(new, FieldKind::Integer | FieldKind::Number);
            Some(match (numeric, i != 0) {
                (true, true) => "1".into(),
                (true, false) => "0".into(),
                (false, true) => "true".into(),
                (false, false) => "false".into(),
            })
        }
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
    }
}

/// Converts one value to `new`. Empty text becomes no value, except for text fields.
pub(crate) fn convert(old: FieldKind, new: FieldKind, v: ValueRef<'_>) -> Result<SqlValue> {
    let Some(text) = as_text(old, new, v) else { return Ok(SqlValue::Null) };
    if new == FieldKind::Text {
        return Ok(SqlValue::Text(text));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(SqlValue::Null);
    }
    if new == FieldKind::Boolean {
        // People write yes/no; the API's query syntax only knows true/false.
        match trimmed.to_ascii_lowercase().as_str() {
            "yes" | "y" => return Ok(SqlValue::Integer(1)),
            "no" | "n" => return Ok(SqlValue::Integer(0)),
            _ => {}
        }
    }
    from_query(new, trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use FieldKind::*;

    fn text(s: &str) -> ValueRef<'_> {
        ValueRef::Text(s.as_bytes())
    }

    #[test]
    fn conversions_that_work() {
        assert_eq!(convert(Text, Integer, text(" 42 ")), Ok(SqlValue::Integer(42)));
        assert_eq!(convert(Text, Number, text("4.5")), Ok(SqlValue::Real(4.5)));
        assert_eq!(convert(Number, Integer, ValueRef::Real(3.0)), Ok(SqlValue::Integer(3)));
        assert_eq!(convert(Integer, Text, ValueRef::Integer(7)), Ok(SqlValue::Text("7".into())));
        assert_eq!(convert(Boolean, Integer, ValueRef::Integer(1)), Ok(SqlValue::Integer(1)));
        assert_eq!(convert(Boolean, Text, ValueRef::Integer(0)), Ok(SqlValue::Text("false".into())));
        assert_eq!(convert(Text, Boolean, text("Yes")), Ok(SqlValue::Integer(1)));
        assert_eq!(convert(Text, Boolean, text("no")), Ok(SqlValue::Integer(0)));
        assert_eq!(convert(Text, Boolean, text("true")), Ok(SqlValue::Integer(1)));
        assert_eq!(convert(Integer, Boolean, ValueRef::Integer(0)), Ok(SqlValue::Integer(0)));
        assert_eq!(convert(Text, Datetime, text("2026-01-15")), Ok(SqlValue::Text("2026-01-15".into())));
        assert_eq!(convert(Integer, Link, ValueRef::Integer(3)), Ok(SqlValue::Integer(3)));
        assert_eq!(convert(Text, Integer, text("  ")), Ok(SqlValue::Null));
        assert_eq!(convert(Integer, Text, ValueRef::Null), Ok(SqlValue::Null));
    }

    #[test]
    fn conversions_that_dont() {
        assert!(convert(Text, Integer, text("lots")).is_err());
        assert!(convert(Number, Integer, ValueRef::Real(2.5)).is_err());
        assert!(convert(Integer, Boolean, ValueRef::Integer(5)).is_err());
        assert!(convert(Text, Datetime, text("next week")).is_err());
        assert!(convert(Integer, Link, ValueRef::Integer(0)).is_err());
    }
}
