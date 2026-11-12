//! When to be in the voice room (spec §2.4, always-on behaviour): join
//! automatically on a shared ride – a session with other human riders, or
//! a voice group the rider chose – and not on solo rides (local server,
//! bots only). Leaving waits a grace period, so a rider who reconnects or
//! a short gap in the snapshots does not drop the call. Failed attempts
//! are retried with a growing back-off.

/// Seconds the ride must stay solo before the client leaves the room.
pub const LEAVE_GRACE_S: f64 = 20.0;
/// First and longest wait before retrying a failed join.
pub const RETRY_FIRST_S: f64 = 2.0;
pub const RETRY_MAX_S: f64 = 60.0;
/// A token request without an answer counts as failed after this long.
pub const REQUEST_TIMEOUT_S: f64 = 10.0;

/// What the client knows about the ride.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RideFacts {
    /// The server issues voice tokens (`Welcome.voice`).
    pub server_offers_voice: bool,
    /// Riders of connected clients, the own one included (snapshots).
    pub human_riders: u32,
    /// A voice group chosen by the rider (`voice.group NAME`).
    pub group: Option<String>,
}

impl RideFacts {
    /// A shared ride: someone else to talk to, or a group to wait in.
    pub fn shared(&self) -> bool {
        self.server_offers_voice && (self.human_riders >= 2 || self.group.is_some())
    }
}

/// Decides whether the client should be in the room, with the leave grace.
#[derive(Debug, Clone, Default)]
pub struct JoinPolicy {
    /// Since when the ride is solo while in the room.
    solo_since: Option<f64>,
    wanted: bool,
}

impl JoinPolicy {
    /// Whether to be in the room at `now_s`. `enabled`: voice chat is on
    /// (setting `voice.enabled`); turning it off leaves at once, as does a
    /// server without voice.
    pub fn wanted(&mut self, facts: &RideFacts, enabled: bool, now_s: f64) -> bool {
        if !enabled || !facts.server_offers_voice {
            self.wanted = false;
            self.solo_since = None;
            return false;
        }
        if facts.shared() {
            self.solo_since = None;
            self.wanted = true;
        } else if self.wanted {
            let since = *self.solo_since.get_or_insert(now_s);
            if now_s - since >= LEAVE_GRACE_S {
                self.wanted = false;
                self.solo_since = None;
            }
        }
        self.wanted
    }
}

/// Back-off between failed joins: 2, 4, 8 … 60 s.
#[derive(Debug, Clone, Default)]
pub struct Retry {
    failures: u32,
}

impl Retry {
    /// A failure: the wait before the next attempt.
    pub fn failed(&mut self) -> f64 {
        let wait = (RETRY_FIRST_S * 2f64.powi(self.failures.min(10) as i32)).min(RETRY_MAX_S);
        self.failures += 1;
        wait
    }

    pub fn succeeded(&mut self) {
        self.failures = 0;
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(offers: bool, humans: u32) -> RideFacts {
        RideFacts {
            server_offers_voice: offers,
            human_riders: humans,
            group: None,
        }
    }

    #[test]
    fn joins_shared_rides_only() {
        let mut p = JoinPolicy::default();
        // Solo ride on the local server (no voice) or a remote one alone.
        assert!(!p.wanted(&facts(false, 3), true, 0.0));
        assert!(!p.wanted(&facts(true, 1), true, 0.0));
        assert!(!p.wanted(&facts(true, 0), true, 0.0));
        // Another human rider joins: in at once.
        assert!(p.wanted(&facts(true, 2), true, 1.0));
        // Voice turned off in the settings: out at once.
        assert!(!p.wanted(&facts(true, 2), false, 2.0));
        assert!(p.wanted(&facts(true, 2), true, 3.0));
    }

    #[test]
    fn a_group_is_shared_even_alone() {
        let mut p = JoinPolicy::default();
        let mut f = facts(true, 1);
        f.group = Some("team-1".into());
        assert!(p.wanted(&f, true, 0.0));
        // But not on a server without voice.
        f.server_offers_voice = false;
        assert!(!p.wanted(&f, true, 0.0));
    }

    #[test]
    fn leaves_after_the_grace_period() {
        let mut p = JoinPolicy::default();
        assert!(p.wanted(&facts(true, 2), true, 0.0));
        // The other rider leaves: stay for the grace period.
        assert!(p.wanted(&facts(true, 1), true, 10.0));
        assert!(p.wanted(&facts(true, 1), true, 10.0 + LEAVE_GRACE_S - 0.1));
        // They come back in time: the grace starts again next time.
        assert!(p.wanted(&facts(true, 2), true, 35.0));
        assert!(p.wanted(&facts(true, 1), true, 40.0));
        assert!(p.wanted(&facts(true, 1), true, 40.0 + LEAVE_GRACE_S - 0.1));
        assert!(!p.wanted(&facts(true, 1), true, 40.0 + LEAVE_GRACE_S));
        assert!(!p.wanted(&facts(true, 1), true, 100.0));
        // The server stops offering voice (a new session): out at once.
        assert!(p.wanted(&facts(true, 2), true, 200.0));
        assert!(!p.wanted(&facts(false, 2), true, 201.0));
    }

    #[test]
    fn retry_backs_off_and_resets() {
        let mut r = Retry::default();
        let waits: Vec<f64> = (0..8).map(|_| r.failed()).collect();
        assert_eq!(waits, vec![2.0, 4.0, 8.0, 16.0, 32.0, 60.0, 60.0, 60.0]);
        assert_eq!(r.failures(), 8);
        r.succeeded();
        assert_eq!(r.failed(), 2.0);
        // The first retry is not faster than the server's rate limit (2 s).
        const { assert!(RETRY_FIRST_S >= 2.0) };
    }
}
