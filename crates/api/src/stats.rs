//! Usage numbers for the owner: how many requests came in, from which keys, for which
//! tables. Counted in memory since Porchly started; nothing is sent anywhere, and no
//! addresses, query strings or row contents are kept.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use axum::http::Method;
use porchly_core::Ident;
use serde_json::{Value, json};

/// Minutes of history kept for the activity chart.
const MINUTES: usize = 60;
/// Most keys, tables or functions counted separately (names come from requests).
const MAX_NAMES: usize = 1_000;

pub struct Stats {
    started: Instant,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    total: u64,
    ok: u64,
    /// No key, a wrong key, or a key that may not do that (401, 403).
    refused: u64,
    /// Told to slow down (429).
    rate_limited: u64,
    /// Something wrong with the request (other 4xx).
    mistakes: u64,
    /// Porchly's own failures (5xx).
    failed: u64,
    reads: u64,
    writes: u64,
    /// Through the public link.
    public: u64,
    mcp: u64,
    /// Requests per minute; `minute` is the index of the last slot filled.
    per_minute: Vec<u32>,
    minute: u64,
    keys: HashMap<i64, KeyUse>,
    tables: HashMap<String, u64>,
    functions: HashMap<String, u64>,
}

struct KeyUse {
    name: String,
    requests: u64,
    last: Instant,
}

impl Default for Stats {
    fn default() -> Self {
        Stats { started: Instant::now(), inner: Mutex::new(Inner { per_minute: vec![0; MINUTES], ..Inner::default() }) }
    }
}

impl Stats {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the per-minute window forward to now.
    fn roll(&self, inner: &mut Inner) {
        let now = self.started.elapsed().as_secs() / 60;
        let gap = now.saturating_sub(inner.minute);
        if gap >= MINUTES as u64 {
            inner.per_minute = vec![0; MINUTES];
        } else {
            for step in 1..=gap {
                let slot = usize::try_from((inner.minute + step) % MINUTES as u64).unwrap_or(0);
                if let Some(v) = inner.per_minute.get_mut(slot) {
                    *v = 0;
                }
            }
        }
        inner.minute = now;
    }

    /// Counts one finished request.
    pub(crate) fn record(&self, method: &Method, path: &str, status: u16, public: bool) {
        // Health checks aren't use.
        if !(path.starts_with("/api") || path == "/mcp" || path == "/docs") {
            return;
        }
        let mut inner = self.lock();
        self.roll(&mut inner);
        let slot = usize::try_from(inner.minute % MINUTES as u64).unwrap_or(0);
        if let Some(v) = inner.per_minute.get_mut(slot) {
            *v = v.saturating_add(1);
        }
        inner.total += 1;
        match status {
            200..=399 => inner.ok += 1,
            401 | 403 => inner.refused += 1,
            429 => inner.rate_limited += 1,
            400..=499 => inner.mistakes += 1,
            _ => inner.failed += 1,
        }
        if method == Method::GET || method == Method::HEAD {
            inner.reads += 1;
        } else {
            inner.writes += 1;
        }
        if public {
            inner.public += 1;
        }
        if path == "/mcp" {
            inner.mcp += 1;
        }
        // Per table and per function, only for requests that worked: the names are then real.
        if status < 400 {
            let mut parts = path.trim_start_matches("/api/").split('/');
            match (path.starts_with("/api/"), parts.next(), parts.next()) {
                (true, Some("fn"), Some(name)) => count(&mut inner.functions, name),
                (true, Some(table), _) if table != "openapi.json" => count(&mut inner.tables, table),
                _ => {}
            }
        }
    }

    /// Notes that a key was used (after it was checked).
    pub(crate) fn key_used(&self, id: i64, name: &str) {
        let mut inner = self.lock();
        let now = Instant::now();
        if let Some(k) = inner.keys.get_mut(&id) {
            k.requests += 1;
            k.last = now;
            if k.name != name {
                k.name = name.to_owned();
            }
        } else if inner.keys.len() < MAX_NAMES {
            inner.keys.insert(id, KeyUse { name: name.to_owned(), requests: 1, last: now });
        }
    }

    /// The numbers, for showing.
    pub fn snapshot(&self) -> Value {
        let mut inner = self.lock();
        self.roll(&mut inner);
        // Oldest minute first, the current minute last.
        let newest = usize::try_from(inner.minute % MINUTES as u64).unwrap_or(0);
        let per_minute: Vec<u32> =
            (1..=MINUTES).map(|i| inner.per_minute.get((newest + i) % MINUTES).copied().unwrap_or(0)).collect();
        let mut keys: Vec<Value> = inner
            .keys
            .values()
            .map(|k| json!({ "name": k.name, "requests": k.requests, "last_used_seconds_ago": k.last.elapsed().as_secs() }))
            .collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.get("requests").and_then(Value::as_u64).unwrap_or(0)));
        json!({
            "seconds": self.started.elapsed().as_secs(),
            "requests": {
                "total": inner.total, "ok": inner.ok, "refused": inner.refused, "rate_limited": inner.rate_limited,
                "mistakes": inner.mistakes, "failed": inner.failed, "reads": inner.reads, "writes": inner.writes,
                "public": inner.public, "mcp": inner.mcp,
            },
            "per_minute": per_minute,
            "keys": keys,
            "tables": inner.tables,
            "functions": inner.functions,
        })
    }
}

fn count(map: &mut HashMap<String, u64>, name: &str) {
    if let Some(n) = map.get_mut(name) {
        *n += 1;
    } else if map.len() < MAX_NAMES && Ident::new(name).is_ok() {
        map.insert(name.to_owned(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_by_outcome_key_and_table() {
        let s = Stats::default();
        s.record(&Method::GET, "/api/recipes", 200, false);
        s.record(&Method::GET, "/api/recipes/3", 200, true);
        s.record(&Method::POST, "/api/recipes", 201, false);
        s.record(&Method::POST, "/api/fn/send_invoice/x", 200, false);
        s.record(&Method::POST, "/mcp", 200, false);
        s.record(&Method::GET, "/api/recipes", 401, false);
        s.record(&Method::DELETE, "/api/recipes/1", 403, false);
        s.record(&Method::GET, "/api/recipes", 429, false);
        s.record(&Method::GET, "/api/nope", 404, false);
        s.record(&Method::GET, "/api/recipes", 500, false);
        s.record(&Method::GET, "/health", 200, false);
        s.key_used(1, "website");
        s.key_used(1, "website");
        s.key_used(2, "phone");
        let v = s.snapshot();
        let r = &v["requests"];
        assert_eq!((r["total"].as_u64(), r["ok"].as_u64()), (Some(10), Some(5)), "health checks aren't counted");
        assert_eq!((r["refused"].as_u64(), r["rate_limited"].as_u64()), (Some(2), Some(1)));
        assert_eq!((r["mistakes"].as_u64(), r["failed"].as_u64()), (Some(1), Some(1)));
        assert_eq!((r["reads"].as_u64(), r["writes"].as_u64()), (Some(6), Some(4)));
        assert_eq!((r["public"].as_u64(), r["mcp"].as_u64()), (Some(1), Some(1)));
        assert_eq!(v["tables"], json!({"recipes": 3}), "only requests that worked, so only real tables");
        assert_eq!(v["functions"], json!({"send_invoice": 1}));
        assert_eq!((v["keys"][0]["name"].as_str(), v["keys"][0]["requests"].as_u64()), (Some("website"), Some(2)));
        let minutes = v["per_minute"].as_array().unwrap();
        assert_eq!(minutes.len(), 60);
        assert_eq!(minutes[59], 10, "everything happened in the current minute, shown last");
    }

    #[test]
    fn hostile_names_dont_pile_up() {
        let s = Stats::default();
        for i in 0..MAX_NAMES + 50 {
            s.record(&Method::GET, &format!("/api/t{i}"), 200, false);
        }
        s.record(&Method::GET, "/api/Not%20A%20Name", 200, false);
        s.record(&Method::GET, "/api/../../etc", 200, false);
        assert_eq!(s.snapshot()["tables"].as_object().unwrap().len(), MAX_NAMES);
    }
}
