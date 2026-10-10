//! Per-key token-bucket rate limiting, in memory.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// Requests per second each key may sustain, unless the owner chose otherwise.
pub const DEFAULT_PER_SECOND: u32 = 20;
/// A key may make this many seconds' worth of requests at once.
const BURST_SECONDS: f64 = 3.0;
/// Buckets kept before idle ones are dropped.
const MAX_BUCKETS: usize = 10_000;

/// The rate limit, changeable while the server runs (Settings in the app).
#[derive(Debug)]
pub struct Limits {
    per_second: AtomicU32,
}

impl Limits {
    pub fn new(per_second: u32) -> Self {
        Limits { per_second: AtomicU32::new(per_second.max(1)) }
    }

    pub fn per_second(&self) -> u32 {
        self.per_second.load(Ordering::Relaxed)
    }

    /// Takes effect on the next request. Never below 1: rate limiting is always on.
    pub fn set_per_second(&self, n: u32) {
        self.per_second.store(n.max(1), Ordering::Relaxed);
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new(DEFAULT_PER_SECOND)
    }
}

pub(crate) struct RateLimiter {
    buckets: Mutex<HashMap<i64, Bucket>>,
    rate: Rate,
}

enum Rate {
    Fixed { rate: f64, burst: f64 },
    Shared(Arc<Limits>),
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub(crate) fn new(rate: f64, burst: f64) -> Self {
        RateLimiter { buckets: Mutex::new(HashMap::new()), rate: Rate::Fixed { rate, burst } }
    }

    pub(crate) fn shared(limits: Arc<Limits>) -> Self {
        RateLimiter { buckets: Mutex::new(HashMap::new()), rate: Rate::Shared(limits) }
    }

    fn rate_and_burst(&self) -> (f64, f64) {
        match &self.rate {
            Rate::Fixed { rate, burst } => (*rate, *burst),
            Rate::Shared(l) => {
                let rate = f64::from(l.per_second());
                (rate, rate * BURST_SECONDS)
            }
        }
    }

    /// Takes one token for `key`; `false` means the request should be refused.
    pub(crate) fn allow(&self, key: i64) -> bool {
        let now = Instant::now();
        let (rate, burst) = self.rate_and_burst();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() >= MAX_BUCKETS && !buckets.contains_key(&key) {
            buckets.retain(|_, b| b.tokens + now.duration_since(b.last).as_secs_f64() * rate < burst);
        }
        let bucket = buckets.entry(key).or_insert(Bucket { tokens: burst, last: now });
        let refill = now.duration_since(bucket.last).as_secs_f64() * rate;
        bucket.tokens = (bucket.tokens + refill).min(burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refuse() {
        let l = RateLimiter::new(0.0001, 3.0);
        assert!(l.allow(1) && l.allow(1) && l.allow(1));
        assert!(!l.allow(1));
        assert!(l.allow(2), "keys have separate buckets");
    }

    #[test]
    fn shared_limits_change_live() {
        let limits = Arc::new(Limits::new(1));
        let l = RateLimiter::shared(limits.clone());
        let allowed = (0..10).filter(|_| l.allow(1)).count();
        assert_eq!(allowed, 3, "a burst of 3 seconds' worth");
        limits.set_per_second(100);
        assert!(l.allow(2), "a new key gets the new, bigger burst");
        assert_eq!((0..400).filter(|_| l.allow(3)).count(), 300);
        limits.set_per_second(0);
        assert_eq!(limits.per_second(), 1, "never off");
    }
}
