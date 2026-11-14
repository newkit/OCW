//! The client's side of collisions (spec §14 Phase 12): the rules the
//! server sent (Welcome, Snapshot), every rider's crash and last bump from
//! the snapshots (a crash is only shown when the server says so, 12D), the
//! wobble after a bump, the crash line of the HUD and the feed lines
//! ("R. Rider crashed at km 12.4"), and the proximity cue (an optional aid,
//! off by default, 12D).

use std::collections::{BTreeMap, VecDeque};

use ocw_core::{RiderId, SIM_DT};
use ocw_protocol::v1;
use ocw_sim::collide::{self, Bump, CrashPhase, CrashView, Mode};

/// How long a feed line stays (s).
pub const FEED_S: f64 = 8.0;
/// At most this many feed lines.
pub const FEED_LINES: usize = 3;
/// The proximity cue (12D): someone within this far beside (m, between
/// the footprints' centres sideways) …
pub const PROXIMITY_BESIDE_M: f64 = 1.0;
/// … or this far in front (m, between the bodies).
pub const PROXIMITY_AHEAD_M: f64 = 2.0;

/// A rider's crash as last received, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seen {
    crash: CrashView,
    at_s: f64,
}

/// Collision state of the client.
#[derive(Debug, Clone, Default)]
pub struct CollisionClient {
    mode: Mode,
    access: Option<ocw_protocol::ClockAccess>,
    chaotic_bots: bool,
    crashes: BTreeMap<RiderId, Seen>,
    bumps: BTreeMap<RiderId, Bump>,
    /// Crash starts already reported (rider, approximate start time).
    reported: BTreeMap<RiderId, f64>,
    feed: VecDeque<(f64, String)>,
}

impl CollisionClient {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn chaotic_bots(&self) -> bool {
        self.chaotic_bots
    }

    /// Who may change the rules (`None`: not told yet).
    pub fn access(&self) -> Option<ocw_protocol::ClockAccess> {
        self.access
    }

    pub fn on_welcome(&mut self, w: &v1::Welcome) {
        self.mode = ocw_protocol::collision_mode_from_wire(w.collisions);
        self.access = Some(ocw_protocol::access_from_wire(w.collision_access));
        self.chaotic_bots = w.chaotic_bots;
        self.crashes.clear();
        self.bumps.clear();
        self.reported.clear();
    }

    /// A snapshot: the rules, every rider's crash and bump; `own` is the
    /// local rider (its feed line says "You").
    pub fn on_snapshot(&mut self, s: &v1::Snapshot, now_s: f64, own: Option<RiderId>) {
        self.mode = ocw_protocol::collision_mode_from_wire(s.collisions);
        self.chaotic_bots = s.chaotic_bots;
        let mut crashes = BTreeMap::new();
        for u in &s.riders {
            let id = RiderId(u.rider_id);
            if let Some(b) = ocw_protocol::rider_bump(u) {
                self.bumps.insert(id, b);
            }
            let Some(c) = ocw_protocol::rider_crash(u) else {
                continue;
            };
            crashes.insert(
                id,
                Seen {
                    crash: c,
                    at_s: now_s,
                },
            );
            let start = now_s - c.elapsed_s;
            let new = self
                .reported
                .get(&id)
                .is_none_or(|t| (start - t).abs() > 1.0);
            if new {
                self.reported.insert(id, start);
                let km = f64::from(u.distance_cm) / 100_000.0;
                let line = if Some(id) == own {
                    format!("You crashed ({})", c.kind.label())
                } else {
                    format!("{} crashed at km {km:.1}", u.display_name)
                };
                self.feed.push_back((now_s, line));
                while self.feed.len() > FEED_LINES {
                    self.feed.pop_front();
                }
            }
        }
        self.crashes = crashes;
        self.bumps
            .retain(|id, _| s.riders.iter().any(|u| u.rider_id == id.0));
    }

    /// A rider's crash at client time `now_s` (advanced since the
    /// snapshot); `None` while riding.
    pub fn crash(&self, id: RiderId, now_s: f64) -> Option<CrashView> {
        let seen = self.crashes.get(&id)?;
        let elapsed_s = seen.crash.elapsed_s + (now_s - seen.at_s).max(0.0);
        // Never further than the next snapshot would say (0.2 s late at most).
        let elapsed_s = elapsed_s.min(seen.crash.elapsed_s + 0.3);
        let phase = collide::crash_phase(elapsed_s, seen.crash.down_s)?;
        Some(CrashView {
            phase,
            elapsed_s,
            ..seen.crash
        })
    }

    /// Whether a rider was down in the last snapshot.
    pub fn crashed(&self, id: RiderId) -> bool {
        self.crashes.contains_key(&id)
    }

    /// The wobble of a rider at server tick `tick` (fractional, estimated).
    pub fn wobble(&self, id: RiderId, tick: f64) -> f64 {
        self.bumps.get(&id).map_or(0.0, |b| {
            collide::wobble((tick - b.tick as f64) * SIM_DT, b.strength, b.side)
        })
    }

    /// The last bump of a rider (its tick: the sound plays once).
    pub fn bump(&self, id: RiderId) -> Option<Bump> {
        self.bumps.get(&id).copied()
    }

    /// The feed lines of the last [`FEED_S`].
    pub fn feed(&self, now_s: f64) -> Vec<String> {
        self.feed
            .iter()
            .filter(|(t, _)| now_s - t < FEED_S)
            .map(|(_, l)| l.clone())
            .collect()
    }

    /// The HUD's crash line for the local rider (12C): "Crash!" with the
    /// time down counting.
    pub fn notice(&self, own: Option<RiderId>, now_s: f64) -> Option<String> {
        let c = self.crash(own?, now_s)?;
        Some(match c.phase {
            CrashPhase::Falling | CrashPhase::Down => format!(
                "Crash! ({}) - back on the bike in {:.1} s",
                c.kind.label(),
                c.remaining_s()
            ),
            CrashPhase::Remount => "Getting back on the bike ...".into(),
        })
    }
}

/// The proximity cue (12D, an optional aid): the side someone is on
/// (-1 left, +1 right, 0 in front) when within [`PROXIMITY_BESIDE_M`]
/// beside or [`PROXIMITY_AHEAD_M`] in front; `others` are (metres ahead,
/// metres to the right) relative to the rider.
pub fn proximity(others: impl IntoIterator<Item = (f64, f64)>) -> Option<i32> {
    let len = collide::BODY_LENGTH_M;
    let width = collide::BODY_WIDTH_M;
    others
        .into_iter()
        .filter_map(|(ahead, right)| {
            let beside = ahead.abs() < len && right.abs() < width + PROXIMITY_BESIDE_M;
            let front = ahead > 0.0 && ahead < len + PROXIMITY_AHEAD_M && right.abs() < width;
            if front {
                Some((ahead, 0))
            } else if beside {
                Some((right.abs(), if right < 0.0 { -1 } else { 1 }))
            } else {
                None
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, side)| side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proximity_cue_beside_and_in_front() {
        assert_eq!(proximity([]), None);
        assert_eq!(proximity([(0.2, 1.1)]), Some(1));
        assert_eq!(proximity([(-0.5, -1.2)]), Some(-1));
        assert_eq!(proximity([(0.0, 1.8)]), None, "too far beside");
        assert_eq!(proximity([(3.0, 0.1)]), Some(0), "in front");
        assert_eq!(proximity([(4.0, 0.1)]), None);
    }
}
