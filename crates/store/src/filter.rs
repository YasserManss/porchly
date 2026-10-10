//! Conditions on rows, written as `field=op.value` in a query string (PostgREST style):
//! `minutes=lt.30`, `name=contains.soup`, `serves=in.2,4`, `rating=is.empty`. A value
//! without a known operator means "equals"; `eq.` forces equality for values that happen to
//! start with an operator name (`name=eq.in.the.box`).
//!
//! Values are always bound as SQL parameters; only the column (a validated [`Ident`]) and the
//! operator (from the fixed list below) shape the SQL text.

use std::collections::HashMap;

use porchly_core::{Error, FieldKind, Ident, Result, limits};
use porchly_schema::{SqlValue, TableDef, from_query};
use rusqlite::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    Starts,
    Ends,
    In,
    Empty,
    NotEmpty,
}

impl Op {
    /// Operator names as written before the dot.
    const NAMES: [(&'static str, Op); 11] = [
        ("eq", Op::Eq),
        ("ne", Op::Ne),
        ("lt", Op::Lt),
        ("lte", Op::Lte),
        ("gt", Op::Gt),
        ("gte", Op::Gte),
        ("contains", Op::Contains),
        ("starts", Op::Starts),
        ("ends", Op::Ends),
        ("in", Op::In),
        ("is", Op::Empty),
    ];

    fn name(self) -> &'static str {
        match self {
            Op::Eq => "eq",
            Op::Ne => "ne",
            Op::Lt => "lt",
            Op::Lte => "lte",
            Op::Gt => "gt",
            Op::Gte => "gte",
            Op::Contains => "contains",
            Op::Starts => "starts",
            Op::Ends => "ends",
            Op::In => "in",
            Op::Empty | Op::NotEmpty => "is",
        }
    }

    fn works_on(self, kind: FieldKind) -> bool {
        match self {
            Op::Eq | Op::Ne | Op::In | Op::Empty | Op::NotEmpty => true,
            Op::Lt | Op::Lte | Op::Gt | Op::Gte => kind != FieldKind::Boolean,
            Op::Contains | Op::Starts | Op::Ends => kind == FieldKind::Text,
        }
    }
}

/// The tables that a table's link fields point to, by link field name.
pub type Related = HashMap<String, TableDef>;

/// A column of the table, or of a table it links to (`customer.city`).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRef {
    /// The link field to follow first, if any.
    pub link: Option<Ident>,
    pub column: Ident,
    pub kind: FieldKind,
}

impl ColumnRef {
    /// Reads `field` or `link.field`. Only one link can be followed.
    pub fn resolve(table: &TableDef, related: &Related, key: &str) -> Result<Self> {
        let missing = || Error::invalid(format!("There's no field \"{}\".", short(key)));
        match key.split_once('.') {
            None => {
                let kind = table.column_kind(key).ok_or_else(missing)?;
                Ok(ColumnRef { link: None, column: Ident::column(key)?, kind })
            }
            Some((link, column)) => {
                let target = related
                    .get(link)
                    .ok_or_else(|| Error::invalid(format!("\"{}\" isn't a link to another table.", short(link))))?;
                let kind = target.column_kind(column).ok_or_else(missing)?;
                Ok(ColumnRef { link: Some(Ident::column(link)?), column: Ident::column(column)?, kind })
            }
        }
    }

    /// `alias."column"`.
    pub(crate) fn qualified(&self, alias: &str) -> String {
        format!("{alias}.{}", self.column.quoted())
    }
}

impl std::fmt::Display for ColumnRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.link {
            Some(link) => write!(f, "{link}.{}", self.column),
            None => write!(f, "{}", self.column),
        }
    }
}

/// One condition: `column op values`.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub column: ColumnRef,
    pub op: Op,
    pub values: Vec<SqlValue>,
}

impl Filter {
    /// Reads `key=raw` from a query string against `table` (and the tables it links to).
    pub fn parse(table: &TableDef, related: &Related, key: &str, raw: &str) -> Result<Self> {
        let column = ColumnRef::resolve(table, related, key)
            .map_err(|e| Error::invalid(format!("Can't filter by \"{}\": {e}", short(key))))?;
        let kind = column.kind;
        let (op, rest) = split_op(raw);
        let field_err = |e: Error| Error::invalid(format!("{}: {e}", column));
        if !op.works_on(kind) {
            return Err(Error::invalid(format!(
                "{column}: \"{}\" doesn't work on {} fields.",
                op.name(),
                kind.label().to_lowercase()
            )));
        }
        let values = match op {
            Op::Empty | Op::NotEmpty => Vec::new(),
            Op::In => {
                let items: Vec<&str> = rest.split(',').map(str::trim).collect();
                if items.len() > limits::MAX_IN_VALUES {
                    return Err(Error::TooLarge(format!(
                        "{column}: \"in\" takes at most {} values.",
                        limits::MAX_IN_VALUES
                    )));
                }
                if items.iter().all(|s| s.is_empty()) {
                    return Err(Error::invalid(format!("{column}: \"in\" needs values, like in.2,4")));
                }
                items.into_iter().map(|s| from_query(kind, s)).collect::<Result<Vec<_>>>().map_err(field_err)?
            }
            Op::Contains | Op::Starts | Op::Ends => vec![SqlValue::Text(like_pattern(op, rest))],
            _ => vec![from_query(kind, rest).map_err(field_err)?],
        };
        Ok(Filter { column, op, values })
    }

    /// The SQL condition on the table aliased `alias`, pushing its values onto `params`
    /// (placeholders are numbered from there).
    pub(crate) fn sql(&self, alias: &str, params: &mut Vec<Value>) -> String {
        let col = self.column.qualified(alias);
        let mut bind = |v: &SqlValue| {
            params.push(to_sql(v));
            format!("?{}", params.len())
        };
        match self.op {
            Op::Empty => format!("({col} IS NULL OR {col} = '')"),
            Op::NotEmpty => format!("({col} IS NOT NULL AND {col} <> '')"),
            Op::In => {
                let placeholders: Vec<String> = self.values.iter().map(&mut bind).collect();
                format!("{col} IN ({})", placeholders.join(", "))
            }
            Op::Contains | Op::Starts | Op::Ends => {
                let p = self.values.first().map(&mut bind).unwrap_or_else(|| "NULL".into());
                format!("{col} LIKE {p} ESCAPE '\\'")
            }
            op => {
                let sym = match op {
                    Op::Ne => "IS NOT",
                    Op::Lt => "<",
                    Op::Lte => "<=",
                    Op::Gt => ">",
                    Op::Gte => ">=",
                    _ => "=",
                };
                let p = self.values.first().map(&mut bind).unwrap_or_else(|| "NULL".into());
                format!("{col} {sym} {p}")
            }
        }
    }
}

/// Splits `op.rest`; a value without a known operator is an equality test on the whole value.
fn split_op(raw: &str) -> (Op, &str) {
    if let Some((name, rest)) = raw.split_once('.')
        && let Some((_, op)) = Op::NAMES.iter().find(|(n, _)| *n == name)
    {
        if *op == Op::Empty {
            return match rest {
                "empty" | "null" => (Op::Empty, ""),
                "notempty" | "notnull" => (Op::NotEmpty, ""),
                // "is.something" isn't a condition: compare the whole text.
                _ => (Op::Eq, raw),
            };
        }
        return (*op, rest);
    }
    (Op::Eq, raw)
}

/// A case-insensitive LIKE pattern matching `text` literally (`%`, `_` and `\` are escaped).
pub(crate) fn like_pattern(op: Op, text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    match op {
        Op::Starts => format!("{escaped}%"),
        Op::Ends => format!("%{escaped}"),
        _ => format!("%{escaped}%"),
    }
}

pub(crate) fn to_sql(v: &SqlValue) -> Value {
    match v {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(i) => Value::Integer(*i),
        SqlValue::Real(f) => Value::Real(*f),
        SqlValue::Text(s) => Value::Text(s.clone()),
    }
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use porchly_schema::FieldDef;

    fn table() -> TableDef {
        let f = |n: &str, kind| FieldDef { name: Ident::new(n).unwrap(), kind, required: false, links_to: None };
        TableDef {
            name: Ident::new("t").unwrap(),
            fields: vec![f("name", FieldKind::Text), f("minutes", FieldKind::Integer), f("veg", FieldKind::Boolean)],
        }
    }

    #[test]
    fn operators_are_read() {
        let t = table();
        let f = Filter::parse(&t, &Related::new(), "minutes", "lt.30").unwrap();
        assert_eq!((f.op, f.values), (Op::Lt, vec![SqlValue::Integer(30)]));
        assert_eq!(Filter::parse(&t, &Related::new(), "minutes", "30").unwrap().op, Op::Eq);
        assert_eq!(Filter::parse(&t, &Related::new(), "name", "is.empty").unwrap().op, Op::Empty);
        assert_eq!(Filter::parse(&t, &Related::new(), "name", "is.notempty").unwrap().op, Op::NotEmpty);
        let f = Filter::parse(&t, &Related::new(), "minutes", "in.2, 4,6").unwrap();
        assert_eq!(f.values.len(), 3);
    }

    #[test]
    fn plain_values_that_look_like_operators() {
        let t = table();
        // Not an operator name: the whole value is compared.
        let f = Filter::parse(&t, &Related::new(), "name", "Dr. Who").unwrap();
        assert_eq!((f.op, f.values), (Op::Eq, vec![SqlValue::Text("Dr. Who".into())]));
        let f = Filter::parse(&t, &Related::new(), "name", "is.this.it").unwrap();
        assert_eq!(f.values, vec![SqlValue::Text("is.this.it".into())]);
        // eq. forces equality.
        let f = Filter::parse(&t, &Related::new(), "name", "eq.in.the.box").unwrap();
        assert_eq!((f.op, f.values), (Op::Eq, vec![SqlValue::Text("in.the.box".into())]));
    }

    #[test]
    fn operators_must_suit_the_field() {
        let t = table();
        assert!(Filter::parse(&t, &Related::new(), "minutes", "contains.3").is_err());
        assert!(Filter::parse(&t, &Related::new(), "veg", "gt.true").is_err());
        assert!(Filter::parse(&t, &Related::new(), "minutes", "lt.soon").is_err());
        assert!(Filter::parse(&t, &Related::new(), "minutes", "in.").is_err());
        assert!(Filter::parse(&t, &Related::new(), "nope", "1").is_err());
    }

    #[test]
    fn like_wildcards_are_literal() {
        assert_eq!(like_pattern(Op::Contains, "50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(like_pattern(Op::Starts, "ab"), "ab%");
        assert_eq!(like_pattern(Op::Ends, "ab"), "%ab");
    }

    #[test]
    fn in_lists_are_capped() {
        let t = table();
        let many = vec!["1"; limits::MAX_IN_VALUES + 1].join(",");
        assert!(matches!(
            Filter::parse(&t, &Related::new(), "minutes", &format!("in.{many}")),
            Err(Error::TooLarge(_))
        ));
    }
}
