//! Undo for rows. Before rows are changed or deleted (one at a time or in bulk, from the app
//! or through the API), what they looked like is kept in `_porchly_undo_rows`, grouped into
//! one entry per change in `_porchly_undo_ops`. Undoing a change puts those rows back the way
//! they were, including links that a delete cleared.
//!
//! The last [`limits::MAX_UNDO_OPS`] changes are kept, for [`limits::UNDO_DAYS`] days. When a
//! table's structure changes, the changes recorded for it are forgotten (old values may no
//! longer fit); structure changes have their own safety net, a snapshot.

use porchly_core::{Error, FieldKind, Result, limits};
use porchly_schema::TableDef;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::Serialize;

use crate::{DbResult, NOW, Store};

/// Fields per `json_object` call (SQLite caps the arguments of one function call).
const FIELDS_PER_OBJECT: usize = 40;

/// A change that can be undone, as listed.
#[derive(Debug, Clone, Serialize)]
pub struct UndoInfo {
    pub id: i64,
    pub at: String,
    /// `at` in this computer's time (`2026-10-11 09:30`).
    pub at_local: String,
    /// What happened, in plain words ("Deleted 3 rows from orders").
    pub summary: String,
    pub rows: u64,
    pub undone: bool,
}

/// What undoing a change did.
#[derive(Debug, Clone, Serialize)]
pub struct Undone {
    pub id: i64,
    pub summary: String,
    /// Deleted rows that are back.
    pub restored: u64,
    /// Changed rows that have their old values again.
    pub reverted: u64,
}

/// Starts recording a change on `tx`.
pub(crate) fn begin(tx: &Connection, summary: &str) -> Result<i64> {
    tx.execute(&format!("INSERT INTO _porchly_undo_ops (at, summary) VALUES ({NOW}, ?1)"), [summary]).db()?;
    Ok(tx.last_insert_rowid())
}

/// Keeps how the rows of `def` matching `which` look right now. `which` is a condition on
/// the alias `u` (`u."id" = ?1`), with `params` numbered from 1. Returns how many rows.
pub(crate) fn record(
    tx: &Connection,
    op: i64,
    def: &TableDef,
    deleted: bool,
    which: &str,
    params: &[Value],
) -> Result<usize> {
    let mut all: Vec<Value> = params.to_vec();
    all.push(Value::Integer(op));
    let op_param = all.len();
    all.push(Value::Text(def.name.as_str().to_owned()));
    let name_param = all.len();
    tx.execute(
        &format!(
            "INSERT INTO _porchly_undo_rows (op, table_name, row_id, deleted, old) \
             SELECT ?{op_param}, ?{name_param}, u.\"id\", {}, {} FROM {} AS u WHERE {which}",
            i32::from(deleted),
            row_json(def),
            def.name.quoted(),
        ),
        params_from_iter(all.iter()),
    )
    .db()
}

/// A JSON object of the whole row `u`: system fields and every field of the table.
fn row_json(def: &TableDef) -> String {
    let mut pairs = vec![
        "'id', u.\"id\"".to_owned(),
        "'created_at', u.\"created_at\"".to_owned(),
        "'updated_at', u.\"updated_at\"".to_owned(),
    ];
    // Field names are validated identifiers, so they're safe as quoted keys.
    pairs.extend(def.fields.iter().map(|f| format!("'{}', u.{}", f.name.as_str(), f.name.quoted())));
    let mut objects = pairs.chunks(FIELDS_PER_OBJECT).map(|c| format!("json_object({})", c.join(", ")));
    let first = objects.next().unwrap_or_else(|| "json_object()".to_owned());
    // json_patch drops keys whose value is null; undo reads a missing key as "no value".
    objects.fold(first, |acc, o| format!("json_patch({acc}, {o})"))
}

/// Ends recording: a change that touched no rows isn't kept, and old changes are forgotten.
pub(crate) fn finish(tx: &Connection, op: i64, summary: &str) -> Result<()> {
    let rows: i64 = tx.query_row("SELECT count(*) FROM _porchly_undo_rows WHERE op = ?1", [op], |r| r.get(0)).db()?;
    if rows == 0 {
        tx.execute("DELETE FROM _porchly_undo_ops WHERE id = ?1", [op]).db()?;
        return Ok(());
    }
    tx.execute("UPDATE _porchly_undo_ops SET summary = ?2 WHERE id = ?1", rusqlite::params![op, summary]).db()?;
    tx.execute(
        &format!(
            "DELETE FROM _porchly_undo_ops WHERE id <= ?1 - {} \
             OR at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-{} days')",
            limits::MAX_UNDO_OPS,
            limits::UNDO_DAYS
        ),
        [op],
    )
    .db()?;
    Ok(())
}

/// Forgets the changes recorded for `table` (its structure is changing, or it is being
/// deleted). Changes left with no rows are removed. Run inside the change's transaction.
pub(crate) fn forget(tx: &Connection, table: &str) -> Result<()> {
    tx.execute("DELETE FROM _porchly_undo_rows WHERE table_name = ?1", [table]).db()?;
    tx.execute("DELETE FROM _porchly_undo_ops WHERE id NOT IN (SELECT DISTINCT op FROM _porchly_undo_rows)", [])
        .db()?;
    Ok(())
}

/// "1 row" / "3 rows".
pub(crate) fn rows_text(n: usize) -> String {
    if n == 1 { "1 row".to_owned() } else { format!("{n} rows") }
}

impl Store {
    /// Recent changes, newest first.
    pub fn undo_list(&self, limit: usize) -> Result<Vec<UndoInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT o.id, o.at, o.summary, o.undone_at IS NOT NULL, strftime('%Y-%m-%d %H:%M', o.at, 'localtime'), \
                 (SELECT count(*) FROM _porchly_undo_rows r WHERE r.op = o.id AND (r.deleted = 1 OR NOT EXISTS \
                   (SELECT 1 FROM _porchly_undo_rows d WHERE d.op = o.id AND d.deleted = 1))) \
                 FROM _porchly_undo_ops o ORDER BY o.id DESC LIMIT ?1",
            )
            .db()?;
        stmt.query_map([i64::try_from(limit.min(1000)).unwrap_or(1000)], |r| {
            Ok(UndoInfo {
                id: r.get(0)?,
                at: r.get(1)?,
                summary: r.get(2)?,
                undone: r.get(3)?,
                at_local: r.get(4)?,
                rows: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
            })
        })
        .db()?
        .collect::<rusqlite::Result<_>>()
        .db()
    }

    /// Undoes a change: `op`, or the newest one not undone yet. Deleted rows come back with
    /// their ids, changed rows get their old values, cleared links are restored. All or nothing.
    pub fn undo(&mut self, op: Option<i64>) -> Result<Undone> {
        let found: Option<(i64, String, bool)> = match op {
            Some(id) => self
                .conn
                .query_row(
                    "SELECT id, summary, undone_at IS NOT NULL FROM _porchly_undo_ops WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .db()?,
            None => self
                .conn
                .query_row(
                    "SELECT id, summary, 0 FROM _porchly_undo_ops WHERE undone_at IS NULL ORDER BY id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .db()?,
        };
        let Some((id, summary, undone)) = found else {
            return Err(Error::not_found(match op {
                Some(_) => "That change can't be undone any more (Porchly keeps recent changes for a week).",
                None => "There's nothing to undo.",
            }));
        };
        if undone {
            return Err(Error::invalid("That change was already undone."));
        }
        let tables: Vec<String> = {
            let mut stmt =
                self.conn.prepare("SELECT DISTINCT table_name FROM _porchly_undo_rows WHERE op = ?1").db()?;
            stmt.query_map([id], |r| r.get(0)).db()?.collect::<rusqlite::Result<_>>().db()?
        };
        let defs: Vec<TableDef> = tables.iter().map(|t| self.table(t)).collect::<Result<_>>()?;
        let all_tables = self.list_tables()?;

        let tx = self.conn.transaction().db()?;
        let has_deletes: bool = tx
            .query_row("SELECT count(*) > 0 FROM _porchly_undo_rows WHERE op = ?1 AND deleted = 1", [id], |r| r.get(0))
            .db()?;
        let (mut restored, mut reverted) = (0usize, 0usize);
        // Deleted rows first, so links restored below have something to point at.
        for def in &defs {
            let mut cols = vec!["\"id\"".to_owned(), "\"created_at\"".to_owned(), "\"updated_at\"".to_owned()];
            let mut vals = vec![
                "json_extract(r.old, '$.id')".to_owned(),
                "json_extract(r.old, '$.created_at')".to_owned(),
                "json_extract(r.old, '$.updated_at')".to_owned(),
            ];
            for f in &def.fields {
                cols.push(f.name.quoted());
                vals.push(format!("json_extract(r.old, '$.{}')", f.name.as_str()));
            }
            restored += tx
                .execute(
                    &format!(
                        "INSERT INTO {t} ({}) SELECT {} FROM _porchly_undo_rows AS r \
                         WHERE r.op = ?1 AND r.table_name = ?2 AND r.deleted = 1 \
                         AND NOT EXISTS (SELECT 1 FROM {t} AS x WHERE x.\"id\" = r.row_id)",
                        cols.join(", "),
                        vals.join(", "),
                        t = def.name.quoted(),
                    ),
                    rusqlite::params![id, def.name.as_str()],
                )
                .db()?;
        }
        for def in &defs {
            let mut sets = vec!["\"updated_at\" = json_extract(r.old, '$.updated_at')".to_owned()];
            sets.extend(
                def.fields
                    .iter()
                    .map(|f| format!("{} = json_extract(r.old, '$.{}')", f.name.quoted(), f.name.as_str())),
            );
            let n = tx
                .execute(
                    &format!(
                        "UPDATE {t} SET {} FROM _porchly_undo_rows AS r \
                         WHERE r.op = ?1 AND r.table_name = ?2 AND r.deleted = 0 AND {t}.\"id\" = r.row_id",
                        sets.join(", "),
                        t = def.name.quoted(),
                    ),
                    rusqlite::params![id, def.name.as_str()],
                )
                .db()?;
            // Rows whose links were cleared by a delete count with that delete, not on their own.
            if !has_deletes {
                reverted += n;
            }
        }
        // A link put back may point at a row deleted since: never leave it dangling.
        for def in &defs {
            for f in def.fields.iter().filter(|f| f.kind == FieldKind::Link) {
                let Some(target) = f.links_to.as_ref().filter(|t| all_tables.iter().any(|d| &d.name == *t)) else {
                    continue;
                };
                tx.execute(
                    &format!(
                        "UPDATE {t} SET {f} = NULL WHERE {f} IS NOT NULL AND {f} NOT IN (SELECT \"id\" FROM {})",
                        target.quoted(),
                        t = def.name.quoted(),
                        f = f.name.quoted(),
                    ),
                    [],
                )
                .db()?;
            }
        }
        tx.execute(&format!("UPDATE _porchly_undo_ops SET undone_at = {NOW} WHERE id = ?1"), [id]).db()?;
        tx.commit().db()?;
        Ok(Undone {
            id,
            summary,
            restored: u64::try_from(restored).unwrap_or(0),
            reverted: u64::try_from(reverted).unwrap_or(0),
        })
    }

    /// Forgets every recorded change (after restoring a backup, they no longer apply).
    pub(crate) fn forget_all_undo(&self) -> Result<()> {
        self.conn.execute("DELETE FROM _porchly_undo_ops", []).db()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ListQuery;
    use crate::test_util::temp_store;
    use porchly_core::Ident;
    use porchly_schema::FieldDef;
    use serde_json::json;

    fn body(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    /// customers ← orders.customer, with a few rows.
    fn shop() -> (tempfile::TempDir, Store) {
        let (dir, mut s) = temp_store();
        let f = |n: &str, k| FieldDef::new(Ident::new(n).unwrap(), k, false);
        s.create_table(
            &Ident::new("customers").unwrap(),
            vec![f("name", FieldKind::Text), f("vip", FieldKind::Boolean)],
        )
        .unwrap();
        s.create_table(
            &Ident::new("orders").unwrap(),
            vec![
                f("item", FieldKind::Text),
                f("price", FieldKind::Number),
                FieldDef::link(Ident::new("customer").unwrap(), Ident::new("customers").unwrap(), false),
            ],
        )
        .unwrap();
        for (name, vip) in [("Ann", true), ("Bob", false)] {
            s.insert_row("customers", &body(json!({"name": name, "vip": vip}))).unwrap();
        }
        for (item, price, c) in [("Tea", 2.5, 1), ("Jam", 4.0, 2), ("Bread", 3.25, 1)] {
            s.insert_row("orders", &body(json!({"item": item, "price": price, "customer": c}))).unwrap();
        }
        (dir, s)
    }

    fn q(s: &Store, table: &str, pairs: &[(&str, &str)]) -> ListQuery {
        s.list_query(table, pairs.iter().copied()).unwrap()
    }

    fn rows(s: &Store, table: &str) -> Vec<crate::Row> {
        s.list_rows(table, &ListQuery::default()).unwrap().items
    }

    #[test]
    fn a_changed_row_gets_its_old_values_back() {
        let (_dir, mut s) = shop();
        let before = s.get_row("orders", 1).unwrap();
        s.update_row("orders", 1, &body(json!({"item": "Coffee", "price": null, "customer": 2}))).unwrap();
        let list = s.undo_list(10).unwrap();
        assert_eq!((list.len(), list[0].summary.as_str(), list[0].rows), (1, "Changed row 1 in orders", 1));
        let done = s.undo(None).unwrap();
        assert_eq!((done.reverted, done.restored), (1, 0));
        assert_eq!(s.get_row("orders", 1).unwrap(), before, "every value, and the dates, are as before");
        assert!(s.undo_list(10).unwrap()[0].undone);
        assert!(s.undo(Some(done.id)).unwrap_err().to_string().contains("already undone"));
        assert!(s.undo(None).unwrap_err().to_string().contains("nothing to undo"));
    }

    #[test]
    fn a_deleted_row_comes_back_with_its_id_and_the_links_to_it() {
        let (_dir, mut s) = shop();
        let (customers, orders) = (rows(&s, "customers"), rows(&s, "orders"));
        s.delete_row("customers", 1).unwrap();
        assert_eq!(s.get_row("orders", 1).unwrap()["customer"], serde_json::Value::Null);
        let done = s.undo(None).unwrap();
        assert_eq!((done.summary.as_str(), done.restored), ("Deleted row 1 from customers", 1));
        assert_eq!(rows(&s, "customers"), customers, "Ann is back as row 1");
        assert_eq!(rows(&s, "orders"), orders, "and her orders link to her again");
    }

    #[test]
    fn bulk_changes_and_deletes_are_one_step_each() {
        let (_dir, mut s) = shop();
        let orders = rows(&s, "orders");
        let all = q(&s, "orders", &[("id", "gte.1")]);
        s.update_rows("orders", &all, &body(json!({"price": 0, "customer": null}))).unwrap();
        assert_eq!(s.undo_list(10).unwrap()[0].summary, "Changed 3 rows in orders");
        assert_eq!(s.undo(None).unwrap().reverted, 3);
        assert_eq!(rows(&s, "orders"), orders);

        let customers = rows(&s, "customers");
        let everyone = q(&s, "customers", &[("id", "gte.1")]);
        assert_eq!(s.delete_rows("customers", &everyone).unwrap(), 2);
        assert!(rows(&s, "customers").is_empty());
        let list = s.undo_list(10).unwrap();
        assert_eq!((list[0].summary.as_str(), list[0].rows), ("Deleted 2 rows from customers", 2));
        assert_eq!(s.undo(None).unwrap().restored, 2);
        assert_eq!(rows(&s, "customers"), customers);
        assert_eq!(rows(&s, "orders"), orders, "links are back too");

        // A bulk change that matched nothing leaves nothing to undo.
        let nobody = q(&s, "orders", &[("item", "Nothing")]);
        s.delete_rows("orders", &nobody).unwrap();
        assert_eq!(s.undo_list(10).unwrap().iter().filter(|o| !o.undone).count(), 0);
    }

    #[test]
    fn any_recent_change_can_be_undone_not_only_the_last() {
        let (_dir, mut s) = shop();
        s.delete_row("orders", 2).unwrap();
        s.update_row("orders", 1, &body(json!({"item": "Coffee"}))).unwrap();
        let delete_op = s.undo_list(10).unwrap()[1].id;
        s.undo(Some(delete_op)).unwrap();
        assert_eq!(s.get_row("orders", 2).unwrap()["item"], "Jam");
        assert_eq!(s.get_row("orders", 1).unwrap()["item"], "Coffee", "the later change is untouched");
        assert!(s.undo(Some(9999)).is_err());
    }

    #[test]
    fn a_restored_link_never_points_at_a_missing_row() {
        let (_dir, mut s) = shop();
        s.update_row("orders", 2, &body(json!({"customer": 1}))).unwrap();
        let change = s.undo_list(10).unwrap()[0].id;
        s.delete_row("customers", 2).unwrap();
        s.undo(Some(change)).unwrap();
        assert_eq!(s.get_row("orders", 2).unwrap()["customer"], serde_json::Value::Null, "Bob is gone");
    }

    #[test]
    fn structure_changes_forget_the_table_s_history() {
        let (_dir, mut s) = shop();
        s.update_row("orders", 1, &body(json!({"item": "Coffee"}))).unwrap();
        s.update_row("customers", 1, &body(json!({"name": "Anna"}))).unwrap();
        s.add_field("orders", FieldDef::new(Ident::new("note").unwrap(), FieldKind::Text, false)).unwrap();
        let left: Vec<String> = s.undo_list(10).unwrap().into_iter().map(|o| o.summary).collect();
        assert_eq!(left, ["Changed row 1 in customers"], "old values of orders may not fit any more");
        s.drop_table("orders").unwrap();
        s.undo(None).unwrap();
        assert_eq!(s.get_row("customers", 1).unwrap()["name"], "Ann");
    }

    #[test]
    fn wide_tables_and_odd_values_survive() {
        let (_dir, mut s) = temp_store();
        let fields: Vec<FieldDef> = (0..limits::MAX_FIELDS)
            .map(|i| FieldDef::new(Ident::new(&format!("f{i}")).unwrap(), FieldKind::Text, false))
            .collect();
        s.create_table(&Ident::new("wide").unwrap(), fields).unwrap();
        let odd = "quote ' \" back\\slash \n newline \u{1F3EE} {\"json\": [1]}";
        s.insert_row("wide", &body(json!({"f0": odd, "f199": "last", "f77": ""}))).unwrap();
        let before = s.get_row("wide", 1).unwrap();
        s.delete_row("wide", 1).unwrap();
        s.undo(None).unwrap();
        assert_eq!(s.get_row("wide", 1).unwrap(), before);
    }

    #[test]
    fn only_recent_changes_are_kept() {
        let (_dir, mut s) = shop();
        for i in 0..limits::MAX_UNDO_OPS + 5 {
            s.update_row("orders", 1, &body(json!({"item": format!("v{i}")}))).unwrap();
        }
        let kept = s.undo_list(1000).unwrap().len() as i64;
        assert_eq!(kept, limits::MAX_UNDO_OPS);
    }
}
