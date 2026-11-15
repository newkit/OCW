//! Rate limits (spec §12, §9.1): per client address on the token, device
//! and sign-in endpoints, and our own budget for Strava's API (200
//! requests per 15 minutes and 2,000 per day by default), kept in step
//! with what Strava reports in its `X-RateLimit-*` headers.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

/// A sliding one-minute window per (address, endpoint).
pub struct RateLimiter {
    per_minute: u32,
    hits: Mutex<HashMap<(IpAddr, &'static str), VecDeque<i64>>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Counts a request; `false`: over the limit (answer 429).
    pub fn allow(&self, ip: IpAddr, endpoint: &'static str, now: i64) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > 10_000 {
            // Forget idle addresses so the map cannot grow without bound.
            hits.retain(|_, q| q.back().is_some_and(|&t| now - t < 60));
        }
        let q = hits.entry((ip, endpoint)).or_default();
        while q.front().is_some_and(|&t| now - t >= 60) {
            q.pop_front();
        }
        if q.len() as u32 >= self.per_minute {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// Requests to Strava's API in the current windows.
pub struct StravaBudget {
    limit_15min: u32,
    limit_day: u32,
    sent: VecDeque<i64>,
    /// What Strava reported last: (usage 15 min, usage day, at).
    reported: Option<(u32, u32, i64)>,
}

impl StravaBudget {
    pub fn new(limit_15min: u32, limit_day: u32) -> Self {
        Self {
            limit_15min,
            limit_day,
            sent: VecDeque::new(),
            reported: None,
        }
    }

    /// Takes one request from the budget; `Err(seconds)`: wait that long.
    pub fn take(&mut self, now: i64) -> Result<(), i64> {
        while self.sent.front().is_some_and(|&t| now - t >= 86_400) {
            self.sent.pop_front();
        }
        let in_15 = self.sent.iter().filter(|&&t| now - t < 900).count() as u32;
        let day = self.sent.len() as u32;
        // Strava's own count (other instances, the app's other calls) in
        // the same 15-minute window.
        let (r15, rday) = match self.reported {
            Some((u15, ud, at)) if now / 900 == at / 900 => (u15, ud),
            Some((_, ud, at)) if now / 86_400 == at / 86_400 => (0, ud),
            _ => (0, 0),
        };
        if in_15.max(r15) >= self.limit_15min {
            return Err(900 - now.rem_euclid(900));
        }
        if day.max(rday) >= self.limit_day {
            return Err(86_400 - now.rem_euclid(86_400));
        }
        self.sent.push_back(now);
        Ok(())
    }

    /// `X-RateLimit-Limit: 200,2000` / `X-RateLimit-Usage: 12,140`.
    pub fn observe(&mut self, limit: Option<&str>, usage: Option<&str>, now: i64) {
        let pair = |s: &str| -> Option<(u32, u32)> {
            let (a, b) = s.split_once(',')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        };
        if let Some((l15, ld)) = limit.and_then(pair) {
            self.limit_15min = self.limit_15min.min(l15);
            self.limit_day = self.limit_day.min(ld);
        }
        if let Some((u15, ud)) = usage.and_then(pair) {
            self.reported = Some((u15, ud, now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_address_window() {
        let l = RateLimiter::new(3);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for _ in 0..3 {
            assert!(l.allow(a, "token", 100));
        }
        assert!(!l.allow(a, "token", 120));
        assert!(l.allow(a, "device", 120), "per endpoint");
        assert!(l.allow(b, "token", 120), "per address");
        assert!(l.allow(a, "token", 160), "the window slid on");
    }

    #[test]
    fn strava_budget_and_headers() {
        let mut s = StravaBudget::new(2, 3);
        assert_eq!(s.take(0), Ok(()));
        assert_eq!(s.take(1), Ok(()));
        assert_eq!(s.take(2), Err(898), "15-minute limit");
        assert_eq!(s.take(900), Ok(()));
        assert_eq!(s.take(1800), Err(86_400 - 1800), "daily limit");
        // Strava says it counted more (another instance).
        let mut s = StravaBudget::new(200, 2000);
        s.observe(Some("200,2000"), Some("200,500"), 10);
        assert!(s.take(20).is_err());
        assert!(s.take(900).is_ok(), "a new window");
        s.observe(Some("100,1000"), None, 900);
        assert_eq!((s.limit_15min, s.limit_day), (100, 1000));
    }
}
