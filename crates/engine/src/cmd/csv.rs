//! CSV import and export. Imports are all or nothing: a value that doesn't fit stops the
//! import and says which line and column, and nothing is added.

use std::path::Path;

use porchly_core::{Error, FieldKind, Ident, Result, limits};
use porchly_schema::{FieldDef, from_query};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

/// Rows shown in a preview.
const PREVIEW_ROWS: usize = 5;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "csv.preview",
            label: "Look at a CSV file",
            menu: "",
            shortcut: None,
            params: "path: file (.csv, .tsv or .txt) — returns its columns, the field type guessed for each, the first rows and the row count",
            enabled: always,
            run: preview,
        },
        Command {
            id: "csv.import",
            label: "Import CSV",
            menu: "Table/Import CSV…",
            shortcut: None,
            params: "path: file; table: text (a new table is made unless it exists and create is false); create?: bool (default: make the table if it doesn't exist); types?: {column: type} for a new table. Columns id, created_at and updated_at are skipped. All or nothing.",
            enabled: always,
            run: import,
        },
        Command {
            id: "csv.export",
            label: "Export CSV",
            menu: "Table/Export CSV…",
            shortcut: None,
            params: "table: text; path: file ending in .csv; overwrite?: bool (default false)",
            enabled: always,
            run: export,
        },
    ]
}

/// A column of the file: its header, the field name it becomes, and its type.
#[derive(Debug, Clone, Serialize)]
struct Column {
    header: String,
    /// The field name, or `None` for columns that are skipped (id, created_at, updated_at).
    name: Option<String>,
    #[serde(rename = "type")]
    kind: FieldKind,
}

struct Parsed {
    columns: Vec<Column>,
    /// Data rows (without the header), each with one string per column.
    rows: Vec<Vec<String>>,
}

fn preview(_: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let path = p.str("path")?;
    let parsed = read(Path::new(path))?;
    let sample: Vec<&Vec<String>> = parsed.rows.iter().take(PREVIEW_ROWS).collect();
    Ok(json!({
        "file": Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()),
        "suggested_table": Path::new(path).file_stem().and_then(|s| Ident::from_label(&s.to_string_lossy())).map(|i| i.as_str().to_owned()),
        "columns": to_json(&parsed.columns)?,
        "rows": sample,
        "total_rows": parsed.rows.len(),
    }))
}

fn import(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let path = p.str("path")?;
    let table = Ident::new(p.str("table")?)?;
    let mut parsed = read(Path::new(path))?;
    let store = cx.store();
    let exists = store.table(table.as_str()).is_ok();
    let create = p.bool_or("create", !exists)?;

    if create && exists {
        return Err(Error::Conflict(format!(
            "There's already a table called \"{table}\". Choose another name, or add the rows to it."
        )));
    }
    if create && let Some(types) = p.opt_object("types")? {
        for col in &mut parsed.columns {
            if let Some(t) = col.name.as_ref().and_then(|n| types.get(n)).and_then(Value::as_str) {
                col.kind = t.parse()?;
                if col.kind == FieldKind::Link {
                    return Err(Error::invalid(
                        "Link fields can't be made by an import. Import the numbers, then change the field.",
                    ));
                }
            }
        }
    }
    // The type of each column: from the table if it exists, from the file otherwise.
    let existing = if create { None } else { Some(store.table(table.as_str())?) };
    let mut kinds = Vec::with_capacity(parsed.columns.len());
    let mut unknown = Vec::new();
    for col in &parsed.columns {
        let kind = match (&col.name, &existing) {
            (None, _) => None,
            (Some(_), None) => Some(col.kind),
            (Some(name), Some(def)) => match def.field(name) {
                Some(f) => Some(f.kind),
                None => {
                    unknown.push(name.as_str());
                    None
                }
            },
        };
        kinds.push(kind);
    }
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "The file has columns that \"{table}\" doesn't: {}. Add those fields first, or import into a new table.",
            unknown.join(", ")
        )));
    }
    // Check every value before changing anything.
    let mut bodies = Vec::with_capacity(parsed.rows.len());
    for (i, row) in parsed.rows.iter().enumerate() {
        let mut body = Map::new();
        for ((col, kind), cell) in parsed.columns.iter().zip(&kinds).zip(row) {
            let (Some(name), Some(kind)) = (&col.name, kind) else { continue };
            let value = cell_value(*kind, cell).map_err(|e| {
                // Line numbers count the header as line 1.
                Error::invalid(format!("Line {}, column \"{}\": {e}", i + 2, col.header))
            })?;
            if !value.is_null() {
                body.insert(name.clone(), value);
            }
        }
        bodies.push(body);
    }
    if create {
        let fields = parsed
            .columns
            .iter()
            .filter_map(|c| c.name.as_ref().map(|n| Ok(FieldDef::new(Ident::field(n)?, c.kind, false))))
            .collect::<Result<Vec<_>>>()?;
        store.create_table(&table, fields)?;
    }
    let imported = match store.insert_rows(table.as_str(), &bodies) {
        Ok(n) => n,
        Err(e) => {
            // A new table that couldn't be filled is removed again, so a failed import leaves nothing behind.
            if create {
                let _ = store.drop_table(table.as_str());
            }
            return Err(e);
        }
    };
    let skipped: Vec<&str> = parsed.columns.iter().filter(|c| c.name.is_none()).map(|c| c.header.as_str()).collect();
    Ok(json!({ "table": table.as_str(), "imported": imported, "created": create, "skipped_columns": skipped }))
}

fn export(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let path = Path::new(p.str("path")?);
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
        return Err(Error::invalid("The file name must end in .csv."));
    }
    if path.exists() && !p.bool_or("overwrite", false)? {
        return Err(Error::Conflict(format!("{} already exists. Choose another name.", path.display())));
    }
    let store = cx.store();
    let def = store.table(p.str("table")?)?;
    let rows = store.all_rows(def.name.as_str())?;
    let mut headers = vec!["id".to_owned(), "created_at".to_owned(), "updated_at".to_owned()];
    headers.extend(def.fields.iter().map(|f| f.name.as_str().to_owned()));
    let mut out = csv::Writer::from_writer(Vec::new());
    out.write_record(&headers).map_err(csv_err)?;
    for row in &rows {
        let record: Vec<String> = headers.iter().map(|h| cell_text(row.get(h))).collect();
        out.write_record(&record).map_err(csv_err)?;
    }
    let bytes = out.into_inner().map_err(|e| Error::Internal(e.to_string()))?;
    std::fs::write(path, bytes).map_err(|e| Error::invalid(format!("Couldn't write {}: {e}", path.display())))?;
    Ok(json!({ "path": path.display().to_string(), "rows": rows.len() }))
}

/// Reads and checks a CSV file: size, extension, delimiter (comma, semicolon or tab), header.
fn read(path: &Path) -> Result<Parsed> {
    let ext_ok = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["csv", "tsv", "txt"].contains(&e.to_ascii_lowercase().as_str()));
    if !ext_ok {
        return Err(Error::invalid("Porchly imports .csv, .tsv and .txt files."));
    }
    let size =
        std::fs::metadata(path).map_err(|e| Error::invalid(format!("Couldn't read {}: {e}", path.display())))?.len();
    if size > limits::MAX_IMPORT_BYTES {
        return Err(Error::TooLarge(format!(
            "The file is too big to import (the limit is {} MB).",
            limits::MAX_IMPORT_BYTES / 1024 / 1024
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::invalid(format!("Couldn't read {}: {e}", path.display())))?;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Result<Parsed> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let first_line = text.lines().next().unwrap_or("");
    let delimiter =
        b",;\t".iter().copied().max_by_key(|d| first_line.bytes().filter(|b| b == d).count()).unwrap_or(b',');
    let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = reader.headers().map_err(csv_err)?.iter().map(|h| h.trim().to_owned()).collect();
    if headers.iter().all(String::is_empty) {
        return Err(Error::invalid("The file is empty, or its first line has no column names."));
    }
    if headers.len() > limits::MAX_FIELDS {
        return Err(Error::TooLarge(format!("The file has more than {} columns.", limits::MAX_FIELDS)));
    }
    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let record = record.map_err(csv_err)?;
        if rows.len() >= limits::MAX_IMPORT_ROWS {
            return Err(Error::TooLarge(format!("The file has more than {} rows.", limits::MAX_IMPORT_ROWS)));
        }
        if record.len() > headers.len() {
            return Err(Error::invalid(format!("Line {} has more values than there are column names.", i + 2)));
        }
        if record.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let mut cells: Vec<String> = record.iter().map(str::to_owned).collect();
        cells.resize(headers.len(), String::new());
        rows.push(cells);
    }
    let columns = name_columns(&headers, &rows);
    Ok(Parsed { columns, rows })
}

/// Turns headers into field names (unique, not reserved) and guesses each column's type.
fn name_columns(headers: &[String], rows: &[Vec<String>]) -> Vec<Column> {
    let mut used: Vec<String> = Vec::new();
    headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let base = Ident::from_label(header)
                .map(|id| id.as_str().to_owned())
                .unwrap_or_else(|| format!("column_{}", i + 1));
            if porchly_core::is_system_field(&base) {
                return Column { header: header.clone(), name: None, kind: FieldKind::Text };
            }
            let mut name = base.clone();
            let mut n = 2;
            while used.contains(&name) || Ident::field(&name).is_err() {
                name = format!("{base}_{n}");
                n += 1;
            }
            used.push(name.clone());
            let kind = guess(rows.iter().filter_map(|r| r.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty()));
            Column { header: header.clone(), name: Some(name), kind }
        })
        .collect()
}

/// The narrowest type every value fits: whole number, number, yes/no, date, else text.
fn guess<'a>(values: impl Iterator<Item = &'a str> + Clone) -> FieldKind {
    let all = |kind: FieldKind| {
        let mut any = false;
        let ok = values.clone().all(|v| {
            any = true;
            cell_value(kind, v).is_ok()
        });
        ok && any
    };
    let yes_no = values.clone().all(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no"));
    if all(FieldKind::Integer) {
        FieldKind::Integer
    } else if all(FieldKind::Number) {
        FieldKind::Number
    } else if yes_no && values.clone().next().is_some() {
        FieldKind::Boolean
    } else if all(FieldKind::Datetime) {
        FieldKind::Datetime
    } else {
        FieldKind::Text
    }
}

/// A cell as the JSON value for a field of `kind`. Empty cells are no value.
fn cell_value(kind: FieldKind, cell: &str) -> Result<Value> {
    let v = cell.trim();
    if v.is_empty() {
        return Ok(Value::Null);
    }
    let bad = |what: &str| Error::invalid(format!("\"{}\" isn't {what}.", v.chars().take(40).collect::<String>()));
    match kind {
        FieldKind::Text => Ok(Value::String(cell.to_owned())),
        FieldKind::Integer | FieldKind::Link => v.parse::<i64>().map(Value::from).map_err(|_| bad("a whole number")),
        FieldKind::Number => match v.parse::<f64>() {
            Ok(f) if f.is_finite() => serde_json::Number::from_f64(f).map(Value::Number).ok_or_else(|| bad("a number")),
            _ => Err(bad("a number")),
        },
        FieldKind::Boolean => match v.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Ok(Value::Bool(true)),
            "false" | "no" | "n" | "0" => Ok(Value::Bool(false)),
            _ => Err(bad("yes or no")),
        },
        FieldKind::Datetime => from_query(FieldKind::Datetime, v)
            .map(|_| Value::String(v.to_owned()))
            .map_err(|_| bad("a date like 2026-01-15")),
    }
}

/// A row value as CSV text.
fn cell_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn csv_err(e: csv::Error) -> Error {
    Error::invalid(format!("The file isn't valid CSV: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(csv: &str) -> Vec<(Option<String>, FieldKind)> {
        parse(csv.as_bytes()).unwrap().columns.into_iter().map(|c| (c.name, c.kind)).collect()
    }

    #[test]
    fn guesses_types_and_names() {
        let c = cols("Name,Age,Score,Member?,Joined,Notes\nAnn,31,4.5,yes,2026-01-02,\nBob,40,3,no,2025-12-30,hi\n");
        let expect = [
            ("name", FieldKind::Text),
            ("age", FieldKind::Integer),
            ("score", FieldKind::Number),
            ("member", FieldKind::Boolean),
            ("joined", FieldKind::Datetime),
            ("notes", FieldKind::Text),
        ];
        for (got, (name, kind)) in c.iter().zip(expect) {
            assert_eq!((got.0.as_deref(), got.1), (Some(name), kind));
        }
    }

    #[test]
    fn awkward_headers_get_usable_names() {
        let c = cols("id,Name,name,,sort,created_at\n1,a,b,c,d,x\n");
        let names: Vec<Option<&str>> = c.iter().map(|(n, _)| n.as_deref()).collect();
        assert_eq!(names, [None, Some("name"), Some("name_2"), Some("column_4"), Some("sort_2"), None]);
    }

    #[test]
    fn semicolons_tabs_and_bom() {
        assert_eq!(cols("\u{feff}a;b\n1;2\n").len(), 2);
        assert_eq!(cols("a\tb\tc\n1\t2\t3\n").len(), 3);
    }

    #[test]
    fn bad_files() {
        assert!(parse(b"").is_err());
        assert!(parse(b"a,b\n1,2,3\n").is_err(), "too many values");
        let p = parse(b"a,b\n1\n\n,\n").unwrap();
        assert_eq!(p.rows, [vec!["1".to_owned(), String::new()]], "short lines are padded, blank lines skipped");
    }
}
