//! Reading command parameters. Parameters come from people, scripts and agents, so every
//! accessor checks presence and type and returns a plain-language error.

use porchly_core::{Error, Result};
use serde_json::{Map, Value};

static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);

pub(crate) struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    /// `null` counts as no parameters; anything other than an object is an error.
    pub fn new(v: &'a Value) -> Result<Self> {
        match v {
            Value::Object(m) => Ok(Params(m)),
            Value::Null => Ok(Params(&EMPTY)),
            _ => Err(Error::invalid("Parameters must be a JSON object, like {\"table\": \"recipes\"}.")),
        }
    }

    pub fn str(&self, key: &str) -> Result<&'a str> {
        match self.0.get(key) {
            Some(Value::String(s)) => Ok(s),
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be text."))),
            None => Err(missing(key)),
        }
    }

    pub fn opt_str(&self, key: &str) -> Result<Option<&'a str>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => self.str(key).map(Some),
        }
    }

    pub fn bool_or(&self, key: &str, default: bool) -> Result<bool> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be true or false."))),
        }
    }

    pub fn opt_bool(&self, key: &str) -> Result<Option<bool>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be true or false."))),
        }
    }

    pub fn i64(&self, key: &str) -> Result<i64> {
        match self.0.get(key) {
            Some(Value::Number(n)) => {
                n.as_i64().ok_or_else(|| Error::invalid(format!("\"{key}\" must be a whole number.")))
            }
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be a whole number."))),
            None => Err(missing(key)),
        }
    }

    pub fn object(&self, key: &str) -> Result<&'a Map<String, Value>> {
        match self.0.get(key) {
            Some(Value::Object(m)) => Ok(m),
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be an object, like {{\"name\": \"value\"}}."))),
            None => Err(missing(key)),
        }
    }

    pub fn opt_object(&self, key: &str) -> Result<Option<&'a Map<String, Value>>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => self.object(key).map(Some),
        }
    }

    pub fn opt_array(&self, key: &str) -> Result<Option<&'a Vec<Value>>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(a)) => Ok(Some(a)),
            Some(_) => Err(Error::invalid(format!("\"{key}\" must be a list."))),
        }
    }

    /// A scalar parameter rendered as text (for paging and filters), if present.
    pub fn opt_scalar_text(&self, key: &str) -> Result<Option<String>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => scalar_text(key, v).map(Some),
        }
    }
}

/// Renders a JSON scalar as the text a query string would carry.
pub(crate) fn scalar_text(key: &str, v: &Value) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(Error::invalid(format!("\"{key}\" must be text, a number, or true/false."))),
    }
}

fn missing(key: &str) -> Error {
    Error::invalid(format!("\"{key}\" is missing."))
}
