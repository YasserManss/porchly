use porchly_core::{Error, FieldKind, Ident, Result, limits};
use porchly_schema::{TableDef, check_row};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{OptionalExtension, params_from_iter};
use serde::Serialize;

use crate::filter::{ColumnRef, Filter, Related, to_sql};
use crate::{DbResult, NOW, Store};

/// A row as JSON: `id`, `created_at`, `updated_at`, then the user fields.
pub type Row = serde_json::Map<String, serde_json::Value>;

/// A page of rows.
#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub items: Vec<Row>,
    pub limit: u32,
    pub offset: u64,
    pub total: u64,
}

/// What to list: paging, sort, filters (see [`Filter`]) and linked rows to embed.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub offset: Option<u64>,
    /// Column (possibly of a linked table) and whether it is descending.
    pub sort: Option<(ColumnRef, bool)>,
    pub filters: Vec<Filter>,
    /// Link fields whose rows are embedded under `"expand"`.
    pub expand: Vec<Ident>,
    /// Text to look for in any text field (case-insensitive), or a row id.
    pub search: Option<String>,
}

impl ListQuery {
    /// Builds a query from key/value pairs (an HTTP query string or command params).
    /// `limit`, `offset`, `sort` (`field`, `-field`, `link.field`) and `expand` (link fields,
    /// comma-separated) are reserved; every other key is a filter on a column of `table` or of
    /// a table it links to (`minutes=lt.30`, `customer.city=Paris`; see [`Filter`]).
    /// `related` comes from [`Store::related`].
    pub fn parse<'a>(
        table: &TableDef,
        related: &Related,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self> {
        let mut q = ListQuery::default();
        for (key, value) in pairs {
            match key {
                "limit" => {
                    let n: u32 = value.parse().map_err(|_| Error::invalid("limit must be a whole number."))?;
                    q.limit = Some(n);
                }
                "offset" => {
                    let n: u64 = value.parse().map_err(|_| Error::invalid("offset must be a whole number."))?;
                    q.offset = Some(n);
                }
                "sort" => {
                    let (name, desc) = match value.strip_prefix('-') {
                        Some(rest) => (rest, true),
                        None => (value, false),
                    };
                    let column = ColumnRef::resolve(table, related, name)
                        .map_err(|e| Error::invalid(format!("Can't sort by \"{}\": {e}", short(name))))?;
                    q.sort = Some((column, desc));
                }
                "expand" => q.expand.extend(parse_expand(table, value)?),
                "search" => {
                    if value.len() > limits::MAX_SEARCH_BYTES {
                        return Err(Error::TooLarge("The search text is too long.".into()));
                    }
                    let v = value.trim();
                    q.search = (!v.is_empty()).then(|| v.to_owned());
                }
                _ => {
                    if q.filters.len() >= limits::MAX_FILTERS {
                        return Err(Error::TooLarge(format!("At most {} filters are allowed.", limits::MAX_FILTERS)));
                    }
                    q.filters.push(Filter::parse(table, related, key, value)?);
                }
            }
        }
        Ok(q)
    }
}

/// Reads `expand=customer,product`: each name must be a link field of `table`.
pub fn parse_expand(table: &TableDef, value: &str) -> Result<Vec<Ident>> {
    let names: Vec<&str> = value.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    if names.len() > limits::MAX_EXPAND {
        return Err(Error::TooLarge(format!("At most {} links can be expanded at once.", limits::MAX_EXPAND)));
    }
    names
        .into_iter()
        .map(|name| match table.field(name) {
            Some(f) if f.kind == FieldKind::Link => Ok(f.name.clone()),
            _ => Err(Error::invalid(format!("Can't expand \"{}\": it isn't a link to another table.", short(name)))),
        })
        .collect()
}

impl Store {
    /// The tables `def`'s link fields point to, by link field name.
    pub fn related(&self, def: &TableDef) -> Result<Related> {
        let mut out = Related::new();
        for f in &def.fields {
            if let Some(target) = &f.links_to {
                let target_def = if target == &def.name { def.clone() } else { self.table(target.as_str())? };
                out.insert(f.name.as_str().to_owned(), target_def);
            }
        }
        Ok(out)
    }

    /// Parses query pairs for `table` (see [`ListQuery::parse`]).
    pub fn list_query<'a>(
        &self,
        table: &str,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<ListQuery> {
        let def = self.table(table)?;
        let related = self.related(&def)?;
        ListQuery::parse(&def, &related, pairs)
    }

    pub fn list_rows(&self, table: &str, q: &ListQuery) -> Result<Page> {
        let def = self.table(table)?;
        let limit = q.limit.unwrap_or(limits::DEFAULT_PAGE).min(limits::MAX_PAGE);
        let offset = q.offset.unwrap_or(0);
        if offset > limits::MAX_OFFSET {
            return Err(Error::invalid("offset is too large."));
        }
        let related = self.related(&def)?;

        // One LEFT JOIN per link followed by a filter or the sort.
        let mut joins: Vec<&Ident> = Vec::new();
        let used = q.filters.iter().map(|f| &f.column).chain(q.sort.as_ref().map(|(c, _)| c));
        for link in used.filter_map(|c| c.link.as_ref()) {
            if !joins.contains(&link) {
                joins.push(link);
            }
        }
        let alias_of = |c: &ColumnRef| match &c.link {
            Some(link) => format!("j{}", joins.iter().position(|j| *j == link).unwrap_or(0)),
            None => "t".to_owned(),
        };
        let mut from = format!("{} AS t", def.name.quoted());
        for (i, link) in joins.iter().enumerate() {
            let target = related
                .get(link.as_str())
                .ok_or_else(|| Error::invalid(format!("\"{link}\" isn't a link to another table.")))?;
            from.push_str(&format!(" LEFT JOIN {} AS j{i} ON j{i}.\"id\" = t.{}", target.name.quoted(), link.quoted()));
        }

        let mut params: Vec<Value> = Vec::new();
        let mut clauses: Vec<String> = q.filters.iter().map(|f| f.sql(&alias_of(&f.column), &mut params)).collect();
        if let Some(text) = &q.search {
            clauses.push(search_sql(&def, text, &mut params));
        }
        let where_sql = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };

        let total: i64 = self
            .conn
            .query_row(&format!("SELECT count(*) FROM {from}{where_sql}"), params_from_iter(params.iter()), |r| {
                r.get(0)
            })
            .db()?;

        let order = match &q.sort {
            Some((col, desc)) => {
                format!("{} {}, t.\"id\"", col.qualified(&alias_of(col)), if *desc { "DESC" } else { "ASC" })
            }
            None => "t.\"id\"".to_owned(),
        };
        params.push(Value::Integer(i64::from(limit)));
        let limit_param = params.len();
        params.push(Value::Integer(i64::try_from(offset).unwrap_or(i64::MAX)));
        let offset_param = params.len();
        let sql = format!(
            "SELECT {} FROM {from}{where_sql} ORDER BY {order} LIMIT ?{limit_param} OFFSET ?{offset_param}",
            select_list(&def, "t"),
        );
        let mut stmt = self.conn.prepare(&sql).db()?;
        let mut items = stmt
            .query_map(params_from_iter(params.iter()), |r| read_row(&def, r))
            .db()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .db()?;
        self.expand(&related, &mut items, &q.expand)?;
        Ok(Page { items, limit, offset, total: u64::try_from(total).unwrap_or(0) })
    }

    /// One row, with the rows its `expand` link fields point to embedded under `"expand"`.
    pub fn get_row_expanded(&self, table: &str, id: i64, expand: &str) -> Result<Row> {
        let def = self.table(table)?;
        let fields = parse_expand(&def, expand)?;
        let mut rows = vec![self.get_row(table, id)?];
        self.expand(&self.related(&def)?, &mut rows, &fields)?;
        rows.pop().ok_or_else(|| Error::Internal("row vanished while expanding".into()))
    }

    /// Embeds linked rows: for each link field in `fields`, `row.expand.<field>` becomes the
    /// row it points to (or null). One query per field, whatever the number of rows.
    fn expand(&self, related: &Related, rows: &mut [Row], fields: &[Ident]) -> Result<()> {
        for field in fields {
            let Some(target) = related.get(field.as_str()) else { continue };
            let mut ids: Vec<i64> =
                rows.iter().filter_map(|r| r.get(field.as_str()).and_then(|v| v.as_i64())).collect();
            ids.sort_unstable();
            ids.dedup();
            let mut found = std::collections::HashMap::new();
            if !ids.is_empty() {
                let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT {} FROM {} AS t WHERE t.\"id\" IN ({})",
                    select_list(target, "t"),
                    target.name.quoted(),
                    placeholders.join(", ")
                );
                let mut stmt = self.conn.prepare(&sql).db()?;
                let linked = stmt
                    .query_map(params_from_iter(ids.iter()), |r| read_row(target, r))
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                for row in linked {
                    if let Some(id) = row.get("id").and_then(|v| v.as_i64()) {
                        found.insert(id, row);
                    }
                }
            }
            for row in rows.iter_mut() {
                let linked = row
                    .get(field.as_str())
                    .and_then(|v| v.as_i64())
                    .and_then(|id| found.get(&id).cloned())
                    .map_or(serde_json::Value::Null, serde_json::Value::Object);
                let slot = row.entry("expand").or_insert_with(|| serde_json::Value::Object(Row::new()));
                if let Some(map) = slot.as_object_mut() {
                    map.insert(field.as_str().to_owned(), linked);
                }
            }
        }
        Ok(())
    }

    pub fn get_row(&self, table: &str, id: i64) -> Result<Row> {
        let def = self.table(table)?;
        self.conn
            .query_row(
                &format!("SELECT {} FROM {} AS t WHERE t.\"id\" = ?1", select_list(&def, "t"), def.name.quoted()),
                [id],
                |r| read_row(&def, r),
            )
            .optional()
            .db()?
            .ok_or_else(|| Error::not_found(format!("There's no row {id} in \"{}\".", def.name)))
    }

    pub fn insert_row(&mut self, table: &str, body: &serde_json::Map<String, serde_json::Value>) -> Result<Row> {
        let def = self.table(table)?;
        let tx = self.conn.transaction().db()?;
        let id = insert_checked(&tx, &def, body)?;
        tx.commit().db()?;
        self.get_row(table, id)
    }

    /// Inserts many rows, all or nothing: if any row is refused, none are added and the error
    /// says which one (counting from 1). Used by imports.
    pub fn insert_rows(&mut self, table: &str, rows: &[serde_json::Map<String, serde_json::Value>]) -> Result<usize> {
        if rows.len() > limits::MAX_IMPORT_ROWS {
            return Err(Error::TooLarge(format!("At most {} rows can be added at once.", limits::MAX_IMPORT_ROWS)));
        }
        let def = self.table(table)?;
        let tx = self.conn.transaction().db()?;
        for (i, body) in rows.iter().enumerate() {
            insert_checked(&tx, &def, body).map_err(|e| match e {
                Error::Invalid(m) => Error::Invalid(format!("Row {}: {m}", i + 1)),
                Error::TooLarge(m) => Error::TooLarge(format!("Row {}: {m}", i + 1)),
                other => other,
            })?;
        }
        tx.commit().db()?;
        Ok(rows.len())
    }

    /// Every row of a table, in `id` order (for exports).
    pub fn all_rows(&self, table: &str) -> Result<Vec<Row>> {
        let def = self.table(table)?;
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {} FROM {} AS t ORDER BY t.\"id\"", select_list(&def, "t"), def.name.quoted()))
            .db()?;
        stmt.query_map([], |r| read_row(&def, r)).db()?.collect::<rusqlite::Result<Vec<_>>>().db()
    }

    pub fn update_row(
        &mut self,
        table: &str,
        id: i64,
        body: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Row> {
        let def = self.table(table)?;
        let values = check_row(&def, body, false)?;
        if values.is_empty() {
            return Err(Error::invalid("Nothing to change: send at least one field."));
        }
        check_links(&self.conn, &def, &values)?;
        let mut sets = vec![format!("\"updated_at\" = {NOW}")];
        let mut params = Vec::new();
        for (name, v) in &values {
            params.push(to_sql(v));
            sets.push(format!("{} = ?{}", name.quoted(), params.len()));
        }
        params.push(Value::Integer(id));
        let tx = self.conn.transaction().db()?;
        let changed = tx
            .execute(
                &format!("UPDATE {} SET {} WHERE \"id\" = ?{}", def.name.quoted(), sets.join(", "), params.len()),
                params_from_iter(params.iter()),
            )
            .db()?;
        tx.commit().db()?;
        if changed == 0 {
            return Err(Error::not_found(format!("There's no row {id} in \"{}\".", def.name)));
        }
        self.get_row(table, id)
    }

    /// Deletes a row. Links to it from other rows are cleared (set to no value), never followed
    /// by more deletes, so deleting a customer never deletes their orders.
    pub fn delete_row(&mut self, table: &str, id: i64) -> Result<()> {
        let def = self.table(table)?;
        let incoming = self.links_into(def.name.as_str())?;
        let tx = self.conn.transaction().db()?;
        let changed = tx.execute(&format!("DELETE FROM {} WHERE \"id\" = ?1", def.name.quoted()), [id]).db()?;
        if changed == 0 {
            return Err(Error::not_found(format!("There's no row {id} in \"{}\".", def.name)));
        }
        for (t, f) in &incoming {
            tx.execute(
                &format!(
                    "UPDATE {} SET {} = NULL, \"updated_at\" = {NOW} WHERE {} = ?1",
                    t.quoted(),
                    f.quoted(),
                    f.quoted()
                ),
                [id],
            )
            .db()?;
        }
        tx.commit().db()
    }
}

/// Checks one row body and inserts it on `conn` (a transaction). Returns the new id.
fn insert_checked(
    conn: &rusqlite::Connection,
    def: &TableDef,
    body: &serde_json::Map<String, serde_json::Value>,
) -> Result<i64> {
    let values = check_row(def, body, true)?;
    check_links(conn, def, &values)?;
    let mut cols = vec!["\"created_at\"".to_owned(), "\"updated_at\"".to_owned()];
    let mut placeholders = vec![NOW.to_owned(), NOW.to_owned()];
    let mut params = Vec::new();
    for (name, v) in &values {
        params.push(to_sql(v));
        cols.push(name.quoted());
        placeholders.push(format!("?{}", params.len()));
    }
    conn.execute(
        &format!("INSERT INTO {} ({}) VALUES ({})", def.name.quoted(), cols.join(", "), placeholders.join(", ")),
        params_from_iter(params.iter()),
    )
    .db()?;
    Ok(conn.last_insert_rowid())
}

/// Every link value must point at a row that exists.
fn check_links(
    conn: &rusqlite::Connection,
    def: &TableDef,
    values: &[(Ident, porchly_schema::SqlValue)],
) -> Result<()> {
    for (name, value) in values {
        let (Some(field), porchly_schema::SqlValue::Integer(id)) = (def.field(name.as_str()), value) else {
            continue;
        };
        let Some(target) = &field.links_to else { continue };
        let exists: Option<i64> = conn
            .query_row(&format!("SELECT 1 FROM {} WHERE \"id\" = ?1", target.quoted()), [id], |r| r.get(0))
            .optional()
            .db()?;
        if exists.is_none() {
            return Err(Error::invalid(format!("{name}: there's no row {id} in \"{target}\".")));
        }
    }
    Ok(())
}

/// "Any text field contains `text`" (case-insensitive, `%` and `_` literal), or "the id is `text`".
fn search_sql(def: &TableDef, text: &str, params: &mut Vec<Value>) -> String {
    let mut any = Vec::new();
    let text_fields: Vec<&Ident> = def.fields.iter().filter(|f| f.kind == FieldKind::Text).map(|f| &f.name).collect();
    if !text_fields.is_empty() {
        params.push(Value::Text(crate::filter::like_pattern(crate::filter::Op::Contains, text)));
        let p = params.len();
        any.extend(text_fields.iter().map(|f| format!("t.{} LIKE ?{p} ESCAPE '\\'", f.quoted())));
    }
    if let Ok(id) = text.parse::<i64>() {
        params.push(Value::Integer(id));
        any.push(format!("t.\"id\" = ?{}", params.len()));
    }
    if any.is_empty() { "0".to_owned() } else { format!("({})", any.join(" OR ")) }
}

/// The row's columns, qualified with the table's alias in the query.
fn select_list(def: &TableDef, alias: &str) -> String {
    let mut cols =
        vec![format!("{alias}.\"id\""), format!("{alias}.\"created_at\""), format!("{alias}.\"updated_at\"")];
    cols.extend(def.fields.iter().map(|f| format!("{alias}.{}", f.name.quoted())));
    cols.join(", ")
}

fn read_row(def: &TableDef, r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let mut row = Row::new();
    let system = [("id", FieldKind::Integer), ("created_at", FieldKind::Datetime), ("updated_at", FieldKind::Datetime)];
    let columns = system.into_iter().chain(def.fields.iter().map(|f| (f.name.as_str(), f.kind)));
    for (i, (name, kind)) in columns.enumerate() {
        row.insert(name.to_owned(), to_json(kind, r.get_ref(i)?));
    }
    Ok(row)
}

fn to_json(kind: FieldKind, v: ValueRef<'_>) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        ValueRef::Null | ValueRef::Blob(_) => J::Null,
        ValueRef::Integer(i) if kind == FieldKind::Boolean => J::Bool(i != 0),
        ValueRef::Integer(i) => J::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(J::Null, J::Number),
        ValueRef::Text(t) => J::String(String::from_utf8_lossy(t).into_owned()),
    }
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::temp_store;
    use porchly_schema::FieldDef;
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, Store) {
        let (dir, mut s) = temp_store();
        let f = |n: &str, k, r| FieldDef { name: Ident::new(n).unwrap(), kind: k, required: r, links_to: None };
        s.create_table(
            &Ident::new("recipes").unwrap(),
            vec![
                f("name", FieldKind::Text, true),
                f("minutes", FieldKind::Integer, false),
                f("vegetarian", FieldKind::Boolean, false),
            ],
        )
        .unwrap();
        (dir, s)
    }

    fn body(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn crud() {
        let (_dir, mut s) = setup();
        let row = s.insert_row("recipes", &body(json!({"name":"Pancakes","minutes":20,"vegetarian":true}))).unwrap();
        assert_eq!(row["id"], 1);
        assert_eq!(row["vegetarian"], true);
        assert!(row["created_at"].as_str().unwrap().ends_with('Z'));

        let row = s.update_row("recipes", 1, &body(json!({"minutes":25}))).unwrap();
        assert_eq!(row["minutes"], 25);
        assert_eq!(row["name"], "Pancakes");

        assert!(matches!(s.update_row("recipes", 99, &body(json!({"minutes":1}))), Err(Error::NotFound(_))));
        s.delete_row("recipes", 1).unwrap();
        assert!(matches!(s.get_row("recipes", 1), Err(Error::NotFound(_))));
        assert!(matches!(s.delete_row("recipes", 1), Err(Error::NotFound(_))));
    }

    #[test]
    fn list_filter_sort_page() {
        let (_dir, mut s) = setup();
        for (name, minutes, veg) in [("Chili", 45, false), ("Pancakes", 20, true), ("Salad", 10, true)] {
            s.insert_row("recipes", &body(json!({"name":name,"minutes":minutes,"vegetarian":veg}))).unwrap();
        }
        let def = s.table("recipes").unwrap();

        let q = ListQuery::parse(&def, &Related::new(), [("vegetarian", "true"), ("sort", "-minutes")]).unwrap();
        let page = s.list_rows("recipes", &q).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0]["name"], "Pancakes");

        let q = ListQuery::parse(&def, &Related::new(), [("limit", "1"), ("offset", "1")]).unwrap();
        let page = s.list_rows("recipes", &q).unwrap();
        assert_eq!((page.total, page.items.len()), (3, 1));
        assert_eq!(page.items[0]["name"], "Pancakes");

        let q = ListQuery::parse(&def, &Related::new(), [("limit", "100000")]).unwrap();
        assert_eq!(s.list_rows("recipes", &q).unwrap().limit, limits::MAX_PAGE);
    }

    #[test]
    fn hostile_list_queries() {
        let (_dir, s) = setup();
        let def = s.table("recipes").unwrap();
        for (k, v) in [
            ("name\" OR 1=1 --", "x"),
            ("sort", "name; DROP TABLE recipes"),
            ("minutes", "1 OR 1=1"),
            ("limit", "-1"),
            ("nope", "1"),
            ("_porchly_keys", "1"),
        ] {
            assert!(ListQuery::parse(&def, &Related::new(), [(k, v)]).is_err(), "{k}={v}");
        }
        // A value that looks like SQL is just a value.
        let q = ListQuery::parse(&def, &Related::new(), [("name", "' OR '1'='1")]).unwrap();
        assert_eq!(s.list_rows("recipes", &q).unwrap().total, 0);
        assert!(s.table("recipes").is_ok());
    }

    /// Runs a list with `pairs` and returns the names of the rows found, in order.
    fn names(s: &Store, pairs: &[(&str, &str)]) -> Vec<String> {
        let def = s.table("recipes").unwrap();
        let q = ListQuery::parse(&def, &Related::new(), pairs.iter().copied()).unwrap();
        s.list_rows("recipes", &q).unwrap().items.iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect()
    }

    fn sample(s: &mut Store) {
        for v in [
            json!({"name":"Chili","minutes":45,"vegetarian":false}),
            json!({"name":"Pancakes","minutes":20,"vegetarian":true}),
            json!({"name":"Salad","minutes":10}),
            json!({"name":"Tomato soup","minutes":35,"vegetarian":true}),
            json!({"name":"50% off soup","minutes":null}),
        ] {
            s.insert_row("recipes", &body(v)).unwrap();
        }
    }

    #[test]
    fn comparison_operators() {
        let (_dir, mut s) = setup();
        sample(&mut s);
        assert_eq!(names(&s, &[("minutes", "lt.30")]), ["Pancakes", "Salad"]);
        assert_eq!(names(&s, &[("minutes", "lte.20")]), ["Pancakes", "Salad"]);
        assert_eq!(names(&s, &[("minutes", "gt.20"), ("sort", "minutes")]), ["Tomato soup", "Chili"]);
        assert_eq!(
            names(&s, &[("minutes", "gte.10"), ("minutes", "lt.21")]),
            ["Pancakes", "Salad"],
            "AND on one field"
        );
        assert_eq!(names(&s, &[("minutes", "in.10,45")]), ["Chili", "Salad"]);
    }

    #[test]
    fn not_equal_includes_empty_values() {
        let (_dir, mut s) = setup();
        sample(&mut s);
        // Salad and the soup have no "vegetarian" value; "not true" should include them.
        assert_eq!(names(&s, &[("vegetarian", "ne.true")]), ["Chili", "Salad", "50% off soup"]);
    }

    #[test]
    fn text_search_is_case_insensitive_and_literal() {
        let (_dir, mut s) = setup();
        sample(&mut s);
        assert_eq!(names(&s, &[("name", "contains.SOUP")]), ["Tomato soup", "50% off soup"]);
        assert_eq!(names(&s, &[("name", "starts.pan")]), ["Pancakes"]);
        assert_eq!(names(&s, &[("name", "ends.cakes")]), ["Pancakes"]);
        assert_eq!(names(&s, &[("name", "contains.%")]), ["50% off soup"], "% is a character, not a wildcard");
        assert!(names(&s, &[("name", "contains._")]).is_empty(), "_ is a character, not a wildcard");
    }

    #[test]
    fn empty_and_not_empty() {
        let (_dir, mut s) = setup();
        sample(&mut s);
        assert_eq!(names(&s, &[("minutes", "is.empty")]), ["50% off soup"]);
        assert_eq!(names(&s, &[("vegetarian", "is.empty")]), ["Salad", "50% off soup"]);
        assert_eq!(names(&s, &[("vegetarian", "is.notempty")]).len(), 3);
    }

    #[test]
    fn operator_values_are_never_sql() {
        let (_dir, mut s) = setup();
        sample(&mut s);
        assert!(names(&s, &[("name", "contains.' OR '1'='1")]).is_empty());
        assert!(names(&s, &[("name", "in.x') OR 1=1 --,y")]).is_empty());
        let def = s.table("recipes").unwrap();
        assert!(ListQuery::parse(&def, &Related::new(), [("minutes", "lt.1 OR 1=1")]).is_err());
        assert_eq!(s.list_rows("recipes", &ListQuery::default()).unwrap().total, 5, "table intact");
    }

    /// customers(name, city) ← orders(item, quantity, customer)
    fn shop() -> (tempfile::TempDir, Store) {
        let (dir, mut s) = temp_store();
        let f = |n: &str, k| FieldDef::new(Ident::new(n).unwrap(), k, false);
        s.create_table(&Ident::new("customers").unwrap(), vec![f("name", FieldKind::Text), f("city", FieldKind::Text)])
            .unwrap();
        s.create_table(
            &Ident::new("orders").unwrap(),
            vec![
                f("item", FieldKind::Text),
                f("quantity", FieldKind::Integer),
                FieldDef::link(Ident::new("customer").unwrap(), Ident::new("customers").unwrap(), false),
            ],
        )
        .unwrap();
        for (name, city) in [("Ann", "Paris"), ("Bob", "Oslo"), ("Cy", "Paris")] {
            s.insert_row("customers", &body(json!({"name": name, "city": city}))).unwrap();
        }
        for (item, q, c) in [("Tea", 2, 1), ("Jam", 1, 2), ("Bread", 3, 3), ("Milk", 1, 1)] {
            s.insert_row("orders", &body(json!({"item": item, "quantity": q, "customer": c}))).unwrap();
        }
        s.insert_row("orders", &body(json!({"item": "Gift"}))).unwrap();
        (dir, s)
    }

    fn items(s: &Store, pairs: &[(&str, &str)]) -> Vec<String> {
        let q = s.list_query("orders", pairs.iter().copied()).unwrap();
        s.list_rows("orders", &q).unwrap().items.iter().map(|r| r["item"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn links_must_point_at_existing_rows() {
        let (_dir, mut s) = shop();
        let err = s.insert_row("orders", &body(json!({"item": "X", "customer": 99}))).unwrap_err();
        assert!(err.to_string().contains("no row 99"), "{err}");
        assert!(s.update_row("orders", 1, &body(json!({"customer": 99}))).is_err());
        assert!(s.insert_row("orders", &body(json!({"item": "X", "customer": "Ann"}))).is_err());
        assert!(s.update_row("orders", 1, &body(json!({"customer": null}))).is_ok(), "a link can be cleared");
    }

    #[test]
    fn filter_and_sort_by_linked_fields() {
        let (_dir, s) = shop();
        assert_eq!(items(&s, &[("customer.city", "Paris")]), ["Tea", "Bread", "Milk"]);
        assert_eq!(items(&s, &[("customer.name", "starts.b")]), ["Jam"]);
        assert_eq!(items(&s, &[("customer.city", "Paris"), ("quantity", "gt.1")]), ["Tea", "Bread"]);
        assert_eq!(
            items(&s, &[("sort", "-customer.name"), ("customer", "is.notempty")]),
            ["Bread", "Jam", "Tea", "Milk"]
        );
        assert_eq!(items(&s, &[("customer.city", "is.empty")]), ["Gift"], "no link means no linked values");
        // The other direction: all orders of customer 1.
        assert_eq!(items(&s, &[("customer", "1")]), ["Tea", "Milk"]);
    }

    #[test]
    fn bad_link_paths_are_refused() {
        let (_dir, s) = shop();
        for (k, v) in [
            ("item.name", "x"),
            ("customer.nope", "x"),
            ("customer.city.name", "x"),
            ("sort", "customer.nope"),
            ("expand", "item"),
            ("expand", "nope"),
        ] {
            assert!(s.list_query("orders", [(k, v)]).is_err(), "{k}={v}");
        }
        let many = ["customer"; limits::MAX_EXPAND + 1].join(",");
        assert!(matches!(s.list_query("orders", [("expand", many.as_str())]), Err(Error::TooLarge(_))));
    }

    #[test]
    fn expand_embeds_linked_rows() {
        let (_dir, s) = shop();
        let q = s.list_query("orders", [("expand", "customer")]).unwrap();
        let page = s.list_rows("orders", &q).unwrap();
        assert_eq!(page.items[0]["customer"], 1, "the link itself is unchanged");
        assert_eq!(page.items[0]["expand"]["customer"]["name"], "Ann");
        assert_eq!(page.items[2]["expand"]["customer"]["city"], "Paris");
        assert!(page.items[4]["expand"]["customer"].is_null(), "no link, nothing to embed");
        let row = s.get_row_expanded("orders", 2, "customer").unwrap();
        assert_eq!(row["expand"]["customer"]["name"], "Bob");
        assert!(s.get_row("orders", 2).unwrap().get("expand").is_none(), "only when asked");
    }

    #[test]
    fn deleting_a_linked_row_clears_links_but_keeps_rows() {
        let (_dir, mut s) = shop();
        s.delete_row("customers", 1).unwrap();
        assert_eq!(items(&s, &[]).len(), 5, "orders are kept");
        assert_eq!(items(&s, &[("customer", "is.empty")]), ["Tea", "Milk", "Gift"]);
    }

    #[test]
    fn links_within_one_table() {
        let (_dir, mut s) = temp_store();
        s.create_table(
            &Ident::new("people").unwrap(),
            vec![
                FieldDef::new(Ident::new("name").unwrap(), FieldKind::Text, false),
                FieldDef::link(Ident::new("manager").unwrap(), Ident::new("people").unwrap(), false),
            ],
        )
        .unwrap();
        s.insert_row("people", &body(json!({"name": "Boss"}))).unwrap();
        s.insert_row("people", &body(json!({"name": "Dev", "manager": 1}))).unwrap();
        let q = s.list_query("people", [("manager.name", "Boss"), ("expand", "manager")]).unwrap();
        let page = s.list_rows("people", &q).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0]["expand"]["manager"]["name"], "Boss");
        s.delete_row("people", 1).unwrap();
        assert!(s.get_row("people", 2).unwrap()["manager"].is_null());
    }

    #[test]
    fn link_values_in_joins_are_never_sql() {
        let (_dir, s) = shop();
        assert!(items(&s, &[("customer.name", "' OR '1'='1")]).is_empty());
        assert!(items(&s, &[("customer.name", "contains.%")]).is_empty());
        assert!(s.list_query("orders", [("customer.name\" OR 1=1 --", "x")]).is_err());
    }

    #[test]
    fn search_looks_in_every_text_field() {
        let (_dir, mut s) = shop();
        s.insert_row("customers", &body(json!({"name": "Dana", "city": "Ann Arbor"}))).unwrap();
        let names = |s: &Store, q: &str| -> Vec<String> {
            let q = s.list_query("customers", [("search", q)]).unwrap();
            s.list_rows("customers", &q).unwrap().items.iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect()
        };
        assert_eq!(names(&s, "ANN"), ["Ann", "Dana"], "name or city, any case");
        assert_eq!(names(&s, "2"), ["Bob"], "a number also finds that row id");
        assert_eq!(names(&s, "%"), Vec::<String>::new(), "% is a character");
        assert_eq!(names(&s, "  "), ["Ann", "Bob", "Cy", "Dana"], "blank search finds everything");
        let q = s.list_query("orders", [("search", "milk"), ("quantity", "1")]).unwrap();
        assert_eq!(s.list_rows("orders", &q).unwrap().total, 1, "combines with conditions");
        assert!(s.list_query("customers", [("search", "x".repeat(2000).as_str())]).is_err());
    }

    #[test]
    fn bulk_insert_is_all_or_nothing() {
        let (_dir, mut s) = setup();
        let good = [body(json!({"name": "A", "minutes": 1})), body(json!({"name": "B"}))];
        assert_eq!(s.insert_rows("recipes", &good).unwrap(), 2);
        let mixed =
            [body(json!({"name": "C"})), body(json!({"name": "D", "minutes": "x"})), body(json!({"name": "E"}))];
        let err = s.insert_rows("recipes", &mixed).unwrap_err();
        assert!(err.to_string().starts_with("Row 2: minutes"), "{err}");
        assert_eq!(s.all_rows("recipes").unwrap().len(), 2, "nothing from the failed batch was added");
    }

    #[test]
    fn too_many_filters() {
        let (_dir, s) = setup();
        let def = s.table("recipes").unwrap();
        let pairs: Vec<(&str, &str)> = std::iter::repeat_n(("minutes", "1"), limits::MAX_FILTERS + 1).collect();
        assert!(matches!(ListQuery::parse(&def, &Related::new(), pairs), Err(Error::TooLarge(_))));
    }
}
