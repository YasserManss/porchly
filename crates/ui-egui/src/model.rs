//! The engine's JSON, read into types the panels can use.

use porchly_core::FieldKind;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    pub required: bool,
    /// For link fields: the table they point to.
    #[serde(default)]
    pub links_to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyInfo {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub can_write: bool,
    pub revoked_at: Option<String>,
}

/// A function as listed in the sidebar.
#[derive(Debug, Clone, Deserialize)]
pub struct FnItem {
    pub name: String,
    /// "key" (needs a key) or "public" (anyone).
    pub access: String,
}

#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub running: bool,
    pub url: String,
    pub error: Option<String>,
}

impl From<&Value> for ServerStatus {
    fn from(v: &Value) -> Self {
        ServerStatus {
            running: v.get("running").and_then(Value::as_bool).unwrap_or(false),
            url: v.get("url").and_then(Value::as_str).unwrap_or_default().to_owned(),
            error: v.get("error").and_then(Value::as_str).filter(|e| *e != "stopped").map(str::to_owned),
        }
    }
}

/// A cell value as people see and type it.
pub fn display(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Reads what someone typed into a cell of `kind`. Empty means "no value".
pub fn parse(kind: FieldKind, text: &str) -> Result<Value, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Value::Null);
    }
    match kind {
        FieldKind::Text | FieldKind::Datetime => Ok(Value::String(text.to_owned())),
        FieldKind::Integer => text
            .replace(',', "")
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| format!("\"{text}\" isn't a whole number.")),
        FieldKind::Number => match text.parse::<f64>() {
            Ok(f) => {
                serde_json::Number::from_f64(f).map(Value::Number).ok_or_else(|| format!("\"{text}\" isn't a number."))
            }
            Err(_) => Err(format!("\"{text}\" isn't a number.")),
        },
        FieldKind::Link => match text.parse::<i64>() {
            Ok(id) if id > 0 => Ok(Value::from(id)),
            _ => Err(format!("\"{text}\" isn't a row number.")),
        },
        FieldKind::Boolean => match text.to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" => Ok(Value::Bool(true)),
            "no" | "false" | "0" => Ok(Value::Bool(false)),
            _ => Err(format!("\"{text}\" isn't yes or no.")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parsing_typed_values() {
        assert_eq!(parse(FieldKind::Integer, " 1,200 "), Ok(json!(1200)));
        assert!(parse(FieldKind::Integer, "1.5").is_err());
        assert_eq!(parse(FieldKind::Number, "4.5"), Ok(json!(4.5)));
        assert!(parse(FieldKind::Number, "inf").is_err());
        assert_eq!(parse(FieldKind::Text, ""), Ok(Value::Null));
        assert_eq!(parse(FieldKind::Boolean, "Yes"), Ok(json!(true)));
    }

    #[test]
    fn displaying_values() {
        assert_eq!(display(Some(&json!("x"))), "x");
        assert_eq!(display(Some(&json!(4.5))), "4.5");
        assert_eq!(display(Some(&Value::Null)), "");
    }
}
