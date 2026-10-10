//! Table and field definitions, and conversion of incoming values (JSON bodies, query
//! strings) into checked, typed values ready to bind into SQL.

mod value;

pub use value::{SqlValue, from_json, from_query};

use porchly_core::{Error, FieldKind, Ident, Result, limits};
use serde::Serialize;

/// A user field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldDef {
    pub name: Ident,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    pub required: bool,
    /// For `link` fields: the table whose rows this field points to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links_to: Option<Ident>,
}

impl FieldDef {
    /// A field that isn't a link.
    pub fn new(name: Ident, kind: FieldKind, required: bool) -> Self {
        FieldDef { name, kind, required, links_to: None }
    }

    /// A field holding the `id` of a row in `table`.
    pub fn link(name: Ident, table: Ident, required: bool) -> Self {
        FieldDef { name, kind: FieldKind::Link, required, links_to: Some(table) }
    }
}

/// A user table and its fields, in display order. Every table also has the system
/// fields `id`, `created_at` and `updated_at`, which are not listed here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableDef {
    pub name: Ident,
    pub fields: Vec<FieldDef>,
}

impl TableDef {
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name.as_str() == name)
    }

    /// Kind of any column, including the system fields.
    pub fn column_kind(&self, name: &str) -> Option<FieldKind> {
        match name {
            "id" => Some(FieldKind::Integer),
            "created_at" | "updated_at" => Some(FieldKind::Datetime),
            _ => self.field(name).map(|f| f.kind),
        }
    }
}

/// Checks a new table's field list: count, duplicates, names.
pub fn check_fields(fields: &[FieldDef]) -> Result<()> {
    if fields.len() > limits::MAX_FIELDS {
        return Err(Error::TooLarge(format!("A table can have at most {} fields.", limits::MAX_FIELDS)));
    }
    let mut seen = std::collections::HashSet::new();
    for f in fields {
        if porchly_core::Ident::field(f.name.as_str()).is_err() {
            return Err(Error::invalid(format!(
                "\"{}\" is added to every table automatically. Please choose another name.",
                f.name
            )));
        }
        match (f.kind, &f.links_to) {
            (FieldKind::Link, None) => {
                return Err(Error::invalid(format!("\"{}\" links to another table: say which one.", f.name)));
            }
            (FieldKind::Link, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return Err(Error::invalid(format!("Only link fields can point to another table (\"{}\").", f.name)));
            }
        }
        if !seen.insert(f.name.as_str()) {
            return Err(Error::invalid(format!("The field \"{}\" appears twice.", f.name)));
        }
    }
    Ok(())
}

/// Checks a row body for insert or update against the table and returns the typed values
/// in table field order. Unknown fields and system fields are rejected; on insert, required
/// fields must be present and not null.
pub fn check_row(
    table: &TableDef,
    body: &serde_json::Map<String, serde_json::Value>,
    is_insert: bool,
) -> Result<Vec<(Ident, SqlValue)>> {
    for key in body.keys() {
        if table.field(key).is_none() {
            if porchly_core::is_system_field(key) {
                return Err(Error::invalid(format!("\"{key}\" is set by Porchly and can't be changed.")));
            }
            return Err(Error::invalid(format!("The table \"{}\" has no field \"{}\".", table.name, short(key))));
        }
    }
    let mut out = Vec::new();
    for f in &table.fields {
        match body.get(f.name.as_str()) {
            Some(v) => {
                let value = from_json(f.kind, v).map_err(|e| prefix_field(&f.name, e))?;
                if f.required && value == SqlValue::Null {
                    return Err(Error::invalid(format!("\"{}\" is required and can't be empty.", f.name)));
                }
                out.push((f.name.clone(), value));
            }
            None if is_insert && f.required => {
                return Err(Error::invalid(format!("\"{}\" is required.", f.name)));
            }
            None => {}
        }
    }
    Ok(out)
}

fn prefix_field(name: &Ident, e: Error) -> Error {
    match e {
        Error::Invalid(m) => Error::Invalid(format!("{name}: {m}")),
        Error::TooLarge(m) => Error::TooLarge(format!("{name}: {m}")),
        other => other,
    }
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table() -> TableDef {
        TableDef {
            name: Ident::new("recipes").unwrap(),
            fields: vec![
                FieldDef { name: Ident::new("name").unwrap(), kind: FieldKind::Text, required: true, links_to: None },
                FieldDef {
                    name: Ident::new("minutes").unwrap(),
                    kind: FieldKind::Integer,
                    required: false,
                    links_to: None,
                },
                FieldDef {
                    name: Ident::new("vegetarian").unwrap(),
                    kind: FieldKind::Boolean,
                    required: false,
                    links_to: None,
                },
            ],
        }
    }

    fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn insert_ok() {
        let row = check_row(&table(), &obj(json!({"name":"Pancakes","minutes":20,"vegetarian":true})), true).unwrap();
        assert_eq!(row.len(), 3);
        assert_eq!(row[2].1, SqlValue::Integer(1));
    }

    #[test]
    fn insert_missing_required() {
        assert!(check_row(&table(), &obj(json!({"minutes":20})), true).is_err());
        assert!(check_row(&table(), &obj(json!({"name":null})), true).is_err());
        assert!(check_row(&table(), &obj(json!({"minutes":20})), false).is_ok());
    }

    #[test]
    fn rejects_unknown_and_system_fields() {
        assert!(check_row(&table(), &obj(json!({"name":"x","colour":"red"})), true).is_err());
        assert!(check_row(&table(), &obj(json!({"name":"x","id":5})), true).is_err());
    }

    #[test]
    fn rejects_wrong_types() {
        assert!(check_row(&table(), &obj(json!({"name":"x","minutes":"twenty"})), true).is_err());
        assert!(check_row(&table(), &obj(json!({"name":5})), true).is_err());
    }

    #[test]
    fn duplicate_fields() {
        let mut t = table();
        t.fields.push(t.fields[0].clone());
        assert!(check_fields(&t.fields).is_err());
    }
}
