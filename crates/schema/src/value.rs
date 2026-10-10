use porchly_core::{Error, FieldKind, Result, limits};

/// A checked value ready to bind into SQL.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

/// Converts a JSON value from a request body into a value of the field's kind.
pub fn from_json(kind: FieldKind, v: &serde_json::Value) -> Result<SqlValue> {
    use serde_json::Value as J;
    if v.is_null() {
        return Ok(SqlValue::Null);
    }
    match (kind, v) {
        (FieldKind::Text, J::String(s)) => text(s),
        (FieldKind::Integer, J::Number(n)) => {
            n.as_i64().map(SqlValue::Integer).ok_or_else(|| Error::invalid("expected a whole number."))
        }
        (FieldKind::Number, J::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() => Ok(SqlValue::Real(f)),
            _ => Err(Error::invalid("expected a number.")),
        },
        (FieldKind::Boolean, J::Bool(b)) => Ok(SqlValue::Integer(i64::from(*b))),
        (FieldKind::Link, J::Number(n)) => match n.as_i64() {
            Some(id) if id > 0 => Ok(SqlValue::Integer(id)),
            _ => Err(Error::invalid("expected the id of a row (a whole number from 1 up).")),
        },
        (FieldKind::Link, _) => Err(Error::invalid("expected the id of a row (a whole number from 1 up).")),
        (FieldKind::Datetime, J::String(s)) => datetime(s),
        (FieldKind::Text, _) => Err(Error::invalid("expected text.")),
        (FieldKind::Integer, _) => Err(Error::invalid("expected a whole number.")),
        (FieldKind::Number, _) => Err(Error::invalid("expected a number.")),
        (FieldKind::Boolean, _) => Err(Error::invalid("expected true or false.")),
        (FieldKind::Datetime, _) => {
            Err(Error::invalid("expected a date like \"2026-01-14\" or \"2026-01-14T14:30:00Z\"."))
        }
    }
}

/// Converts a query-string value (always text) into a value of the field's kind, for filters.
pub fn from_query(kind: FieldKind, s: &str) -> Result<SqlValue> {
    match kind {
        FieldKind::Text => text(s),
        FieldKind::Integer => {
            s.parse::<i64>().map(SqlValue::Integer).map_err(|_| Error::invalid("expected a whole number."))
        }
        FieldKind::Number => match s.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(SqlValue::Real(f)),
            _ => Err(Error::invalid("expected a number.")),
        },
        FieldKind::Boolean => match s {
            "true" | "1" => Ok(SqlValue::Integer(1)),
            "false" | "0" => Ok(SqlValue::Integer(0)),
            _ => Err(Error::invalid("expected true or false.")),
        },
        FieldKind::Datetime => datetime(s),
        FieldKind::Link => match s.parse::<i64>() {
            Ok(id) if id > 0 => Ok(SqlValue::Integer(id)),
            _ => Err(Error::invalid("expected the id of a row (a whole number from 1 up).")),
        },
    }
}

fn text(s: &str) -> Result<SqlValue> {
    if s.len() > limits::MAX_TEXT_BYTES {
        return Err(Error::TooLarge(format!("text can be at most {} KB.", limits::MAX_TEXT_BYTES / 1024)));
    }
    Ok(SqlValue::Text(s.to_owned()))
}

fn datetime(s: &str) -> Result<SqlValue> {
    if is_datetime(s) {
        Ok(SqlValue::Text(s.to_owned()))
    } else {
        Err(Error::invalid("expected a date like \"2026-01-14\" or \"2026-01-14T14:30:00Z\"."))
    }
}

/// Accepts `YYYY-MM-DD`, optionally followed by `THH:MM`, `:SS`, `.fraction` and a zone
/// (`Z` or `±HH:MM`). Checks ranges loosely (month 1–12, day 1–31, hour < 24, minute and second < 60).
fn is_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() > 40 {
        return false;
    }
    let mut p = Parser { b, i: 0 };
    let ok_date = p.num(4).is_some()
        && p.eat(b'-')
        && p.num(2).is_some_and(|m| (1..=12).contains(&m))
        && p.eat(b'-')
        && p.num(2).is_some_and(|d| (1..=31).contains(&d));
    if !ok_date {
        return false;
    }
    if p.done() {
        return true;
    }
    if !(p.eat(b'T') || p.eat(b' ')) {
        return false;
    }
    if !(p.num(2).is_some_and(|h| h < 24) && p.eat(b':') && p.num(2).is_some_and(|m| m < 60)) {
        return false;
    }
    if p.eat(b':') {
        if !p.num(2).is_some_and(|sec| sec < 60) {
            return false;
        }
        if p.eat(b'.') && p.digits() == 0 {
            return false;
        }
    }
    if p.done() || (p.eat(b'Z') && p.done()) {
        return true;
    }
    (p.eat(b'+') || p.eat(b'-'))
        && p.num(2).is_some_and(|h| h < 24)
        && p.eat(b':')
        && p.num(2).is_some_and(|m| m < 60)
        && p.done()
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn eat(&mut self, c: u8) -> bool {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn num(&mut self, len: usize) -> Option<u32> {
        let mut n: u32 = 0;
        for _ in 0..len {
            let c = *self.b.get(self.i)?;
            if !c.is_ascii_digit() {
                return None;
            }
            n = n.checked_mul(10)?.checked_add(u32::from(c - b'0'))?;
            self.i += 1;
        }
        Some(n)
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        self.i - start
    }

    fn done(&self) -> bool {
        self.i == self.b.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn datetimes() {
        for ok in ["2026-01-14", "2026-01-14T14:30", "2026-01-14T14:30:00Z", "2026-01-14 14:30:00.123+02:00"] {
            assert!(is_datetime(ok), "{ok}");
        }
        for bad in
            ["", "2026-13-01", "2026-01-14T25:00", "14/01/2026", "2026-01-14T14:30:00Zjunk", "2026-01-14T14:30:00."]
        {
            assert!(!is_datetime(bad), "{bad}");
        }
    }

    #[test]
    fn json_values() {
        assert_eq!(from_json(FieldKind::Integer, &json!(5)), Ok(SqlValue::Integer(5)));
        assert!(from_json(FieldKind::Integer, &json!(5.5)).is_err());
        assert_eq!(from_json(FieldKind::Number, &json!(5)), Ok(SqlValue::Real(5.0)));
        assert_eq!(from_json(FieldKind::Boolean, &json!(false)), Ok(SqlValue::Integer(0)));
        assert!(from_json(FieldKind::Boolean, &json!("yes")).is_err());
        assert_eq!(from_json(FieldKind::Link, &json!(3)), Ok(SqlValue::Integer(3)));
        assert!(from_json(FieldKind::Link, &json!(0)).is_err());
        assert!(from_json(FieldKind::Link, &json!("3")).is_err());
        assert_eq!(from_json(FieldKind::Text, &json!(null)), Ok(SqlValue::Null));
    }

    #[test]
    fn query_values() {
        assert_eq!(from_query(FieldKind::Boolean, "true"), Ok(SqlValue::Integer(1)));
        assert!(from_query(FieldKind::Integer, "1; DROP TABLE x").is_err());
        assert!(from_query(FieldKind::Number, "NaN").is_err());
    }

    #[test]
    fn text_limit() {
        let big = "x".repeat(limits::MAX_TEXT_BYTES + 1);
        assert!(matches!(from_json(FieldKind::Text, &json!(big)), Err(Error::TooLarge(_))));
    }
}
