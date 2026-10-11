use porchly_core::{Error, FieldKind, Ident, Result, limits};
use porchly_schema::{FieldDef, SqlValue, TableDef, check_fields};
use rusqlite::OptionalExtension;

use crate::convert::{as_text, convert};
use crate::filter::to_sql;
use crate::{DbResult, NOW, Store};

/// What to change about a field. `None` keeps the current setting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldChange {
    pub name: Option<Ident>,
    pub kind: Option<FieldKind>,
    pub required: Option<bool>,
    /// The table a link field points to (needed when changing a field into a link).
    pub links_to: Option<Ident>,
}

/// How many problem rows an error message names before saying "and N more".
const EXAMPLES: usize = 3;

impl Store {
    /// All user tables, by name.
    pub fn list_tables(&self) -> Result<Vec<TableDef>> {
        let mut stmt = self.conn.prepare("SELECT name FROM _porchly_tables ORDER BY name").db()?;
        let names: Vec<String> = stmt.query_map([], |r| r.get(0)).db()?.collect::<rusqlite::Result<_>>().db()?;
        names.iter().map(|n| self.table(n)).collect()
    }

    /// One table. Names that aren't valid identifiers are reported as not found, so
    /// Porchly's own tables can never be reached this way.
    pub fn table(&self, name: &str) -> Result<TableDef> {
        let not_found = || Error::not_found(format!("There's no table called \"{}\".", short(name)));
        let ident = Ident::new(name).map_err(|_| not_found())?;
        let exists: Option<String> = self
            .conn
            .query_row("SELECT name FROM _porchly_tables WHERE name = ?1", [ident.as_str()], |r| r.get(0))
            .optional()
            .db()?;
        if exists.is_none() {
            return Err(not_found());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, kind, required, links_to FROM _porchly_fields WHERE table_name = ?1 ORDER BY position",
            )
            .db()?;
        let raw: Vec<(String, String, bool, Option<String>)> = stmt
            .query_map([ident.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()?;
        let fields = raw
            .into_iter()
            .map(|(name, kind, required, links_to)| {
                let links_to = links_to.as_deref().map(Ident::new).transpose()?;
                Ok(FieldDef { name: Ident::new(&name)?, kind: kind.parse::<FieldKind>()?, required, links_to })
            })
            .collect::<Result<Vec<_>>>()
            .map_err(|_| Error::Storage(format!("field metadata for \"{ident}\" is damaged")))?;
        Ok(TableDef { name: ident, fields })
    }

    pub fn create_table(&mut self, name: &Ident, fields: Vec<FieldDef>) -> Result<TableDef> {
        check_fields(&fields)?;
        if porchly_core::RESERVED_TABLES.contains(&name.as_str()) {
            return Err(Error::invalid(format!(
                "\"{name}\" is reserved: /api/{name} is where functions live. Please choose another name."
            )));
        }
        let count: i64 = self.conn.query_row("SELECT count(*) FROM _porchly_tables", [], |r| r.get(0)).db()?;
        if usize::try_from(count).unwrap_or(usize::MAX) >= limits::MAX_TABLES {
            return Err(Error::TooLarge(format!("A project can have at most {} tables.", limits::MAX_TABLES)));
        }
        if self.table_exists_in_sqlite(name)? {
            return Err(Error::Conflict(format!("There's already a table called \"{name}\".")));
        }
        for f in &fields {
            self.check_link_target(f, name)?;
        }
        let mut columns = vec![
            "\"id\" INTEGER PRIMARY KEY AUTOINCREMENT".to_owned(),
            "\"created_at\" TEXT NOT NULL".to_owned(),
            "\"updated_at\" TEXT NOT NULL".to_owned(),
        ];
        columns.extend(fields.iter().map(|f| format!("{} {}", f.name.quoted(), f.kind.sql_type())));

        let tx = self.conn.transaction().db()?;
        tx.execute(&format!("CREATE TABLE {} ({})", name.quoted(), columns.join(", ")), []).db()?;
        tx.execute(&format!("INSERT INTO _porchly_tables (name, created_at) VALUES (?1, {NOW})"), [name.as_str()])
            .db()?;
        for (position, f) in fields.iter().enumerate() {
            tx.execute(
                "INSERT INTO _porchly_fields (table_name, name, kind, required, position, links_to)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    name.as_str(),
                    f.name.as_str(),
                    f.kind.as_str(),
                    f.required,
                    position as i64,
                    f.links_to.as_ref().map(Ident::as_str)
                ],
            )
            .db()?;
            if f.kind == FieldKind::Link {
                tx.execute(&create_link_index(name, &f.name), []).db()?;
            }
        }
        tx.commit().db()?;
        self.table(name.as_str())
    }

    /// Deletes a table and all its rows. A snapshot is taken first.
    pub fn drop_table(&mut self, name: &str) -> Result<()> {
        let table = self.table(name)?;
        let incoming: Vec<String> = self
            .links_into(table.name.as_str())?
            .into_iter()
            .filter(|(t, _)| *t != table.name)
            .map(|(t, f)| format!("{t}.{f}"))
            .collect();
        if !incoming.is_empty() {
            return Err(Error::Conflict(format!(
                "Other tables link to \"{}\" ({}). Delete those link fields first.",
                table.name,
                incoming.join(", ")
            )));
        }
        self.snapshot(&format!("before-drop-table-{}", table.name))?;
        let tx = self.conn.transaction().db()?;
        crate::undo::forget(&tx, table.name.as_str())?;
        tx.execute(&format!("DROP TABLE {}", table.name.quoted()), []).db()?;
        tx.execute("DELETE FROM _porchly_fields WHERE table_name = ?1", [table.name.as_str()]).db()?;
        tx.execute("DELETE FROM _porchly_tables WHERE name = ?1", [table.name.as_str()]).db()?;
        tx.commit().db()
    }

    pub fn add_field(&mut self, table: &str, field: FieldDef) -> Result<TableDef> {
        let def = self.table(table)?;
        Ident::field(field.name.as_str())?;
        if def.field(field.name.as_str()).is_some() {
            return Err(Error::Conflict(format!("\"{}\" already has a field called \"{}\".", def.name, field.name)));
        }
        if def.fields.len() >= limits::MAX_FIELDS {
            return Err(Error::TooLarge(format!("A table can have at most {} fields.", limits::MAX_FIELDS)));
        }
        check_fields(std::slice::from_ref(&field))?;
        self.check_link_target(&field, &def.name)?;
        if field.required && self.count_rows(&def)? > 0 {
            return Err(Error::invalid(format!(
                "\"{}\" already has rows, so a new field can't be required yet. Add it as optional, fill it in, then make it required.",
                def.name
            )));
        }
        let tx = self.conn.transaction().db()?;
        crate::undo::forget(&tx, def.name.as_str())?;
        tx.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", def.name.quoted(), field.name.quoted(), field.kind.sql_type()),
            [],
        )
        .db()?;
        tx.execute(
            "INSERT INTO _porchly_fields (table_name, name, kind, required, links_to, position)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT coalesce(max(position), -1) + 1 FROM _porchly_fields WHERE table_name = ?1))",
            rusqlite::params![
                def.name.as_str(),
                field.name.as_str(),
                field.kind.as_str(),
                field.required,
                field.links_to.as_ref().map(Ident::as_str)
            ],
        )
        .db()?;
        if field.kind == FieldKind::Link {
            tx.execute(&create_link_index(&def.name, &field.name), []).db()?;
        }
        tx.commit().db()?;
        self.table(table)
    }

    /// Deletes a field and its values. A snapshot is taken first.
    pub fn drop_field(&mut self, table: &str, field: &str) -> Result<TableDef> {
        let def = self.table(table)?;
        let Some(f) = def.field(field) else {
            return Err(Error::not_found(format!("\"{}\" has no field called \"{}\".", def.name, short(field))));
        };
        let fname = f.name.clone();
        let is_link = f.kind == FieldKind::Link;
        self.snapshot(&format!("before-drop-field-{}-{}", def.name, fname))?;
        let tx = self.conn.transaction().db()?;
        crate::undo::forget(&tx, def.name.as_str())?;
        if is_link {
            tx.execute(&format!("DROP INDEX IF EXISTS {}", link_index_name(&def.name, &fname)), []).db()?;
        }
        tx.execute(&format!("ALTER TABLE {} DROP COLUMN {}", def.name.quoted(), fname.quoted()), []).db()?;
        tx.execute(
            "DELETE FROM _porchly_fields WHERE table_name = ?1 AND name = ?2",
            [def.name.as_str(), fname.as_str()],
        )
        .db()?;
        tx.commit().db()?;
        self.table(table)
    }

    /// Every `(table, field)` whose link field points at `table`.
    pub(crate) fn links_into(&self, table: &str) -> Result<Vec<(Ident, Ident)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name, name FROM _porchly_fields WHERE kind = 'link' AND links_to = ?1 ORDER BY table_name, name")
            .db()?;
        let raw: Vec<(String, String)> =
            stmt.query_map([table], |r| Ok((r.get(0)?, r.get(1)?))).db()?.collect::<rusqlite::Result<_>>().db()?;
        raw.into_iter().map(|(t, f)| Ok((Ident::new(&t)?, Ident::new(&f)?))).collect()
    }

    /// A link field must point at a table that exists (or at the table being created, for
    /// links within one table, like `people.manager`).
    fn check_link_target(&self, field: &FieldDef, own_table: &Ident) -> Result<()> {
        let Some(target) = &field.links_to else { return Ok(()) };
        if target == own_table {
            return Ok(());
        }
        self.table(target.as_str()).map(|_| ()).map_err(|_| {
            Error::invalid(format!("\"{}\" links to \"{target}\", but there's no table with that name.", field.name))
        })
    }

    /// Renames a field, changes its type, makes it required or optional, or points a link at
    /// another table. Existing values are converted to the new type; if any can't be, nothing
    /// changes and the error names the rows to fix. A snapshot is taken before values are rewritten.
    pub fn update_field(&mut self, table: &str, field: &str, change: FieldChange) -> Result<TableDef> {
        let def = self.table(table)?;
        let Some(old) = def.field(field).cloned() else {
            return Err(Error::not_found(format!("\"{}\" has no field called \"{}\".", def.name, short(field))));
        };
        let kind = change.kind.unwrap_or(old.kind);
        let links_to = match kind {
            FieldKind::Link => change.links_to.or_else(|| old.links_to.clone()),
            _ => None,
        };
        let new = FieldDef {
            name: change.name.unwrap_or_else(|| old.name.clone()),
            kind,
            required: change.required.unwrap_or(old.required),
            links_to,
        };
        if new == old {
            return Ok(def);
        }
        Ident::field(new.name.as_str())?;
        check_fields(std::slice::from_ref(&new))?;
        self.check_link_target(&new, &def.name)?;
        if new.name != old.name && def.field(new.name.as_str()).is_some() {
            return Err(Error::Conflict(format!("\"{}\" already has a field called \"{}\".", def.name, new.name)));
        }

        // Read every value, converting when the type (or link target) changes.
        let retyped = new.kind != old.kind || new.links_to != old.links_to;
        let values = self.converted_values(&def, &old, &new)?;
        if new.required {
            let empty: Vec<i64> = values
                .iter()
                .filter(|(_, v)| matches!(v, SqlValue::Null) || matches!(v, SqlValue::Text(t) if t.trim().is_empty()))
                .map(|(id, _)| *id)
                .collect();
            if !empty.is_empty() {
                return Err(Error::invalid(format!(
                    "\"{}\" can't be required yet: {} empty ({}). Fill those in first.",
                    new.name,
                    rows_phrase(empty.len()),
                    examples(empty.iter().map(|id| format!("row {id}")), empty.len())
                )));
            }
        }

        if retyped {
            self.snapshot(&format!("before-change-field-{}-{}", def.name, old.name))?;
        }
        let t = def.name.quoted();
        let tx = self.conn.transaction().db()?;
        crate::undo::forget(&tx, def.name.as_str())?;
        if old.kind == FieldKind::Link {
            tx.execute(&format!("DROP INDEX IF EXISTS {}", link_index_name(&def.name, &old.name)), []).db()?;
        }
        if retyped {
            // SQLite can't change a column's type: build the new column, then swap it in.
            let tmp = "\"_porchly_new_column\"";
            tx.execute(&format!("ALTER TABLE {t} ADD COLUMN {tmp} {}", new.kind.sql_type()), []).db()?;
            for (id, v) in &values {
                if *v != SqlValue::Null {
                    tx.execute(
                        &format!("UPDATE {t} SET {tmp} = ?1 WHERE \"id\" = ?2"),
                        rusqlite::params![to_sql(v), id],
                    )
                    .db()?;
                }
            }
            tx.execute(&format!("ALTER TABLE {t} DROP COLUMN {}", old.name.quoted()), []).db()?;
            tx.execute(&format!("ALTER TABLE {t} RENAME COLUMN {tmp} TO {}", new.name.quoted()), []).db()?;
        } else if new.name != old.name {
            tx.execute(&format!("ALTER TABLE {t} RENAME COLUMN {} TO {}", old.name.quoted(), new.name.quoted()), [])
                .db()?;
        }
        if new.kind == FieldKind::Link {
            tx.execute(&create_link_index(&def.name, &new.name), []).db()?;
        }
        tx.execute(
            "UPDATE _porchly_fields SET name = ?1, kind = ?2, required = ?3, links_to = ?4
             WHERE table_name = ?5 AND name = ?6",
            rusqlite::params![
                new.name.as_str(),
                new.kind.as_str(),
                new.required,
                new.links_to.as_ref().map(Ident::as_str),
                def.name.as_str(),
                old.name.as_str()
            ],
        )
        .db()?;
        tx.commit().db()?;
        self.table(table)
    }

    /// Every row's value of `old`, converted for `new`. Fails, naming rows, if any value
    /// doesn't fit the new type or points at a row that doesn't exist.
    fn converted_values(&self, def: &TableDef, old: &FieldDef, new: &FieldDef) -> Result<Vec<(i64, SqlValue)>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT \"id\", {} FROM {} ORDER BY \"id\"", old.name.quoted(), def.name.quoted()))
            .db()?;
        let mut rows = stmt.query([]).db()?;
        let mut out = Vec::new();
        let mut bad: Vec<String> = Vec::new();
        let mut bad_count = 0usize;
        while let Some(row) = rows.next().db()? {
            let id: i64 = row.get(0).db()?;
            let raw = row.get_ref(1).db()?;
            match convert(old.kind, new.kind, raw) {
                Ok(v) => out.push((id, v)),
                Err(_) => {
                    bad_count += 1;
                    if bad.len() < EXAMPLES {
                        let shown: String =
                            as_text(old.kind, FieldKind::Text, raw).unwrap_or_default().chars().take(30).collect();
                        bad.push(format!("row {id}: \"{shown}\""));
                    }
                }
            }
        }
        if bad_count > 0 {
            return Err(Error::invalid(format!(
                "Can't change \"{}\" to {}: {} values that don't fit ({}). Change or clear them first, or choose another type.",
                old.name,
                new.kind.label().to_lowercase(),
                rows_phrase(bad_count),
                examples(bad.into_iter(), bad_count)
            )));
        }
        if let Some(target) = &new.links_to {
            let mut missing = Vec::new();
            for (id, v) in &out {
                if let SqlValue::Integer(link) = v {
                    let exists: Option<i64> = self
                        .conn
                        .query_row(&format!("SELECT 1 FROM {} WHERE \"id\" = ?1", target.quoted()), [link], |r| {
                            r.get(0)
                        })
                        .optional()
                        .db()?;
                    if exists.is_none() {
                        missing.push(format!("row {id}: {link}"));
                    }
                }
            }
            if !missing.is_empty() {
                let n = missing.len();
                return Err(Error::invalid(format!(
                    "Can't link \"{}\" to \"{target}\": {} point to rows that don't exist there ({}). Change or clear them first.",
                    old.name,
                    rows_phrase(n),
                    examples(missing.into_iter(), n)
                )));
            }
        }
        Ok(out)
    }

    fn table_exists_in_sqlite(&self, name: &Ident) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE lower(name) = lower(?1)", [name.as_str()], |r| {
                r.get(0)
            })
            .db()?;
        Ok(n > 0)
    }

    pub fn count_rows(&self, table: &TableDef) -> Result<u64> {
        let n: i64 =
            self.conn.query_row(&format!("SELECT count(*) FROM {}", table.name.quoted()), [], |r| r.get(0)).db()?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

/// Link fields are indexed so joins and "all orders of this customer" stay fast.
fn link_index_name(table: &Ident, field: &Ident) -> String {
    format!("\"_porchly_link_{table}_{field}\"")
}

fn create_link_index(table: &Ident, field: &Ident) -> String {
    format!("CREATE INDEX {} ON {} ({})", link_index_name(table, field), table.quoted(), field.quoted())
}

/// "1 row has" / "3 rows have".
fn rows_phrase(n: usize) -> String {
    if n == 1 { "1 row has".to_owned() } else { format!("{n} rows have") }
}

/// The first few examples, then "and N more".
fn examples(items: impl Iterator<Item = String>, total: usize) -> String {
    let shown: Vec<String> = items.take(EXAMPLES).collect();
    let more = total.saturating_sub(shown.len());
    if more > 0 { format!("{}, and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::temp_store;

    fn field(name: &str, kind: FieldKind, required: bool) -> FieldDef {
        FieldDef::new(Ident::new(name).unwrap(), kind, required)
    }

    #[test]
    fn create_list_drop() {
        let (_dir, mut s) = temp_store();
        let name = Ident::new("recipes").unwrap();
        let t = s.create_table(&name, vec![field("title", FieldKind::Text, true)]).unwrap();
        assert_eq!(t.fields.len(), 1);
        assert!(matches!(s.create_table(&name, vec![]), Err(Error::Conflict(_))));
        assert_eq!(s.list_tables().unwrap().len(), 1);
        s.drop_table("recipes").unwrap();
        assert!(s.list_tables().unwrap().is_empty());
        assert_eq!(s.list_snapshots().unwrap().len(), 1);
        assert!(s.create_table(&Ident::new("fn").unwrap(), vec![]).is_err(), "/api/fn is for functions");
    }

    fn link(name: &str, to: &str) -> FieldDef {
        FieldDef::link(Ident::new(name).unwrap(), Ident::new(to).unwrap(), false)
    }

    #[test]
    fn link_fields() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("customers").unwrap(), vec![field("name", FieldKind::Text, false)]).unwrap();
        let orders = s.create_table(&Ident::new("orders").unwrap(), vec![link("customer", "customers")]).unwrap();
        assert_eq!(orders.fields[0].links_to.as_ref().map(Ident::as_str), Some("customers"));
        // Links within one table, and links added later.
        s.create_table(&Ident::new("people").unwrap(), vec![link("manager", "people")]).unwrap();
        s.add_field("orders", link("sold_by", "people")).unwrap();
        // Targets must exist; only link fields have one.
        assert!(s.create_table(&Ident::new("bad").unwrap(), vec![link("x", "nowhere")]).is_err());
        assert!(s.add_field("orders", link("y", "nowhere")).is_err());
        let mut not_link = field("z", FieldKind::Text, false);
        not_link.links_to = Some(Ident::new("customers").unwrap());
        assert!(s.add_field("orders", not_link).is_err());
        let mut no_target = field("z", FieldKind::Link, false);
        no_target.links_to = None;
        assert!(s.add_field("orders", no_target).is_err());
        // A link field can be dropped (its index goes too).
        s.drop_field("orders", "sold_by").unwrap();
    }

    #[test]
    fn linked_tables_cannot_be_dropped() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("customers").unwrap(), vec![]).unwrap();
        s.create_table(&Ident::new("orders").unwrap(), vec![link("customer", "customers")]).unwrap();
        let err = s.drop_table("customers").unwrap_err();
        assert!(err.to_string().contains("orders.customer"), "{err}");
        s.drop_field("orders", "customer").unwrap();
        s.drop_table("customers").unwrap();
        // A table that only links to itself can be dropped.
        s.create_table(&Ident::new("people").unwrap(), vec![link("manager", "people")]).unwrap();
        s.drop_table("people").unwrap();
    }

    fn rows(s: &mut Store, table: &str, values: &[serde_json::Value]) {
        for v in values {
            s.insert_row(table, v.as_object().unwrap()).unwrap();
        }
    }

    fn values(s: &Store, table: &str, field: &str) -> Vec<serde_json::Value> {
        s.list_rows(table, &Default::default()).unwrap().items.iter().map(|r| r[field].clone()).collect()
    }

    fn change() -> FieldChange {
        FieldChange::default()
    }

    #[test]
    fn rename_a_field_keeps_its_values() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![field("minutes", FieldKind::Integer, false)]).unwrap();
        rows(&mut s, "t", &[serde_json::json!({"minutes": 5})]);
        let def = s
            .update_field("t", "minutes", FieldChange { name: Some(Ident::new("time").unwrap()), ..change() })
            .unwrap();
        assert_eq!(def.fields[0].name.as_str(), "time");
        assert_eq!(values(&s, "t", "time"), [serde_json::json!(5)]);
        // Names follow the usual rules.
        s.create_table(
            &Ident::new("u").unwrap(),
            vec![field("a", FieldKind::Text, false), field("b", FieldKind::Text, false)],
        )
        .unwrap();
        assert!(matches!(
            s.update_field("u", "a", FieldChange { name: Some(Ident::new("b").unwrap()), ..change() }),
            Err(Error::Conflict(_))
        ));
        assert!(s.update_field("u", "a", FieldChange { name: Some(Ident::new("id").unwrap()), ..change() }).is_err());
        assert!(s.update_field("u", "nope", change()).is_err());
    }

    #[test]
    fn change_type_converts_values() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![field("amount", FieldKind::Text, false)]).unwrap();
        rows(
            &mut s,
            "t",
            &[
                serde_json::json!({"amount": "12"}),
                serde_json::json!({"amount": " 7 "}),
                serde_json::json!({"amount": ""}),
                serde_json::json!({}),
            ],
        );
        s.update_field("t", "amount", FieldChange { kind: Some(FieldKind::Integer), ..change() }).unwrap();
        assert_eq!(
            values(&s, "t", "amount"),
            [serde_json::json!(12), serde_json::json!(7), serde_json::Value::Null, serde_json::Value::Null]
        );
        // Numbers are stored as numbers afterwards, so filters compare numerically.
        let q = s.list_query("t", [("amount", "gt.9")]).unwrap();
        assert_eq!(s.list_rows("t", &q).unwrap().total, 1);
        // And back to text.
        s.update_field("t", "amount", FieldChange { kind: Some(FieldKind::Text), ..change() }).unwrap();
        assert_eq!(values(&s, "t", "amount")[0], serde_json::json!("12"));
        assert_eq!(s.list_snapshots().unwrap().len(), 2, "a snapshot before each conversion");
    }

    #[test]
    fn change_type_refuses_values_that_dont_fit() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![field("amount", FieldKind::Text, false)]).unwrap();
        let vals: Vec<serde_json::Value> =
            ["1", "lots", "2", "a few", "x", "y"].iter().map(|v| serde_json::json!({"amount": v})).collect();
        rows(&mut s, "t", &vals);
        let err =
            s.update_field("t", "amount", FieldChange { kind: Some(FieldKind::Integer), ..change() }).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("4 rows have") && msg.contains("row 2: \"lots\"") && msg.contains("and 1 more"), "{msg}");
        assert_eq!(values(&s, "t", "amount")[1], serde_json::json!("lots"), "nothing changed");
        assert_eq!(s.table("t").unwrap().fields[0].kind, FieldKind::Text);
    }

    #[test]
    fn required_needs_every_row_filled() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![field("name", FieldKind::Text, false)]).unwrap();
        rows(&mut s, "t", &[serde_json::json!({"name": "a"}), serde_json::json!({"name": " "})]);
        let err = s.update_field("t", "name", FieldChange { required: Some(true), ..change() }).unwrap_err();
        assert!(err.to_string().contains("row 2"), "{err}");
        s.update_row("t", 2, serde_json::json!({"name": "b"}).as_object().unwrap()).unwrap();
        assert!(
            s.update_field("t", "name", FieldChange { required: Some(true), ..change() }).unwrap().fields[0].required
        );
        assert!(
            !s.update_field("t", "name", FieldChange { required: Some(false), ..change() }).unwrap().fields[0].required
        );
    }

    #[test]
    fn numbers_become_links_only_if_the_rows_exist() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("customers").unwrap(), vec![field("name", FieldKind::Text, false)]).unwrap();
        rows(&mut s, "customers", &[serde_json::json!({"name": "Ann"})]);
        s.create_table(&Ident::new("orders").unwrap(), vec![field("customer", FieldKind::Integer, false)]).unwrap();
        rows(&mut s, "orders", &[serde_json::json!({"customer": 1}), serde_json::json!({"customer": 9})]);
        let to_link = || FieldChange {
            kind: Some(FieldKind::Link),
            links_to: Some(Ident::new("customers").unwrap()),
            ..change()
        };
        let err = s.update_field("orders", "customer", to_link()).unwrap_err();
        assert!(err.to_string().contains("row 2: 9"), "{err}");
        assert!(
            s.update_field("orders", "customer", FieldChange { kind: Some(FieldKind::Link), ..change() }).is_err(),
            "needs a target"
        );
        s.update_row("orders", 2, serde_json::json!({"customer": null}).as_object().unwrap()).unwrap();
        let def = s.update_field("orders", "customer", to_link()).unwrap();
        assert_eq!(def.fields[0].links_to.as_ref().map(Ident::as_str), Some("customers"));
        // Renaming a link keeps it working, and it can still be dropped afterwards (index follows).
        s.update_field("orders", "customer", FieldChange { name: Some(Ident::new("buyer").unwrap()), ..change() })
            .unwrap();
        let q = s.list_query("orders", [("buyer.name", "Ann"), ("expand", "buyer")]).unwrap();
        assert_eq!(s.list_rows("orders", &q).unwrap().items[0]["expand"]["buyer"]["name"], "Ann");
        s.update_field("orders", "buyer", FieldChange { kind: Some(FieldKind::Integer), ..change() }).unwrap();
        assert!(s.drop_table("customers").is_ok(), "no link left");
    }

    #[test]
    fn porchly_tables_are_invisible() {
        let (_dir, s) = temp_store();
        assert!(matches!(s.table("_porchly_keys"), Err(Error::NotFound(_))));
        assert!(matches!(s.table("sqlite_master"), Err(Error::NotFound(_))));
    }

    #[test]
    fn add_and_drop_fields() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![]).unwrap();
        let t = s.add_field("t", field("a", FieldKind::Integer, false)).unwrap();
        assert_eq!(t.fields.len(), 1);
        assert!(matches!(s.add_field("t", field("a", FieldKind::Text, false)), Err(Error::Conflict(_))));
        let t = s.drop_field("t", "a").unwrap();
        assert!(t.fields.is_empty());
        assert!(s.drop_field("t", "a").is_err());
    }

    #[test]
    fn required_field_needs_empty_table() {
        let (_dir, mut s) = temp_store();
        s.create_table(&Ident::new("t").unwrap(), vec![field("a", FieldKind::Text, false)]).unwrap();
        let body = serde_json::json!({"a": "x"});
        s.insert_row("t", body.as_object().unwrap()).unwrap();
        assert!(s.add_field("t", field("b", FieldKind::Text, true)).is_err());
        assert!(s.add_field("t", field("b", FieldKind::Text, false)).is_ok());
    }
}
