//! The server's cache of provider answers (spec 9F: "cached per region,
//! so many riders cause few requests"): keyed by the request (a URL names
//! the provider, the grid cell, the altitude band or station, and the day
//! for history), each entry fresh for its kind's time to live. Instants are
//! passed in; the oldest entries go when it is full.

use std::collections::BTreeMap;

/// How long answers stay fresh (ms).
pub mod ttl {
    /// Station measurements (10-minute or hourly values).
    pub const MEASURED_MS: i64 = 10 * 60_000;
    /// Forecasts (MET Norway updates hourly and asks for `Expires`, about
    /// 30 minutes; Open-Meteo every hour or so).
    pub const FORECAST_MS: i64 = 30 * 60_000;
    /// History (ERA5 does not change).
    pub const HISTORIC_MS: i64 = 7 * 86_400_000;
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct Cache<V> {
    entries: BTreeMap<String, Entry<V>>,
    max: usize,
}

#[derive(Debug, Clone)]
struct Entry<V> {
    value: V,
    stored_ms: i64,
    ttl_ms: i64,
}

impl<V: Clone> Cache<V> {
    /// A cache of at most `max` entries.
    pub fn new(max: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            max: max.max(1),
        }
    }

    /// The answer for `key` if it is still fresh at `now_ms`.
    pub fn fresh(&self, key: &str, now_ms: i64) -> Option<V> {
        let e = self.entries.get(key)?;
        (now_ms - e.stored_ms < e.ttl_ms && now_ms >= e.stored_ms).then(|| e.value.clone())
    }

    /// The answer for `key` however old, with when it was stored (offline:
    /// the last weather is better than none).
    pub fn last(&self, key: &str) -> Option<(V, i64)> {
        self.entries
            .get(key)
            .map(|e| (e.value.clone(), e.stored_ms))
    }

    /// Stores `value` for `key` at `now_ms`, fresh for `ttl_ms`.
    pub fn insert(&mut self, key: &str, value: V, now_ms: i64, ttl_ms: i64) {
        if !self.entries.contains_key(key) && self.entries.len() >= self.max {
            // The oldest goes.
            if let Some(k) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.stored_ms)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&k);
            }
        }
        self.entries.insert(
            key.to_owned(),
            Entry {
                value,
                stored_ms: now_ms,
                ttl_ms,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_fresh_for_their_time_and_the_oldest_goes() {
        let mut c = Cache::new(2);
        c.insert("a", 1, 0, 1000);
        assert_eq!(c.fresh("a", 999), Some(1));
        assert_eq!(c.fresh("a", 1000), None, "stale");
        assert_eq!(c.last("a"), Some((1, 0)), "but kept");
        assert_eq!(c.fresh("a", -5), None, "a clock going back is not fresh");
        c.insert("b", 2, 10, 1000);
        c.insert("c", 3, 20, 1000);
        assert_eq!(c.len(), 2);
        assert!(c.last("a").is_none(), "the oldest went");
        // Replacing keeps the size.
        c.insert("b", 4, 30, 1000);
        assert_eq!((c.len(), c.fresh("b", 40)), (2, Some(4)));
        assert!(!c.is_empty());
    }
}
