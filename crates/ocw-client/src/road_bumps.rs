//! The own rider's bumps over road damage as the client presents them
//! (spec Phase 8 Q12 (c), v5.34): a short **resistance kick** on the
//! trainer in SIM mode ([`crate::trainer::TrainerControl`]), a small
//! **camera shake** (off with `camera.shake off`, reduced motion and the
//! kiosk's show mode) and a **sound** (`ocw_audio::cues::Cue::Pothole`,
//! `Cue::Crack`).
//!
//! The bumps come from the prediction: the same pure road data and the
//! same function (`ocw_sim::road_feel::crossed`) the game server uses for
//! the speed a bump costs, so the client feels them when it draws the
//! wheel over the hole, without waiting for the server. A feature is
//! presented once (by its id, for [`REPEAT_TICKS`]) even when a
//! reconciliation moves the prediction back over it.

use std::collections::VecDeque;
use std::sync::OnceLock;

use ocw_sim::RiderState;
use ocw_sim::road_feel::{BumpKind, RoadBump};
use ocw_world::Track;

/// The trainer's kick: its rolling resistance × (1 + this · strength) …
pub const KICK_CRR_PER_STRENGTH: f64 = 4.0;
/// … for this long (s; the trainer takes SIM updates every 0.5 s, so the
/// kick reaches it within that).
pub const KICK_S: f64 = 0.6;
/// The camera shake's strength per bump strength (a crash shakes with 1):
/// the largest pothole 0.28, an open crack 0.04.
pub const SHAKE_PER_STRENGTH: f64 = 0.35;
/// A feature met again within this many prediction ticks (3 s) is not
/// presented again.
pub const REPEAT_TICKS: u64 = 150;

/// A bump being presented.
#[derive(Debug, Clone)]
pub struct Seen {
    pub bump: RoadBump,
    /// Counts the bumps presented (1 for the first).
    pub seq: u64,
    /// The client time the camera first showed it (its shake's start).
    shown_at: OnceLock<f64>,
}

impl Seen {
    /// Seconds since the shake started at client time `now_s` (0 the
    /// first time it is asked).
    pub fn age_s(&self, now_s: f64) -> f64 {
        let at = *self.shown_at.get_or_init(|| now_s);
        (now_s - at).max(0.0)
    }

    /// The camera shake's strength.
    pub fn shake_strength(&self) -> f64 {
        self.bump.strength * SHAKE_PER_STRENGTH
    }

    /// The sound of the bump.
    pub fn cue(&self) -> ocw_audio::cues::Cue {
        match self.bump.kind {
            BumpKind::Pothole | BumpKind::EdgeBreak => ocw_audio::cues::Cue::Pothole,
            BumpKind::Crack | BumpKind::Alligator => ocw_audio::cues::Cue::Crack,
        }
    }
}

/// The trainer's rolling-resistance factor for a kick of `strength`.
pub fn kick_factor(strength: f64) -> f64 {
    1.0 + KICK_CRR_PER_STRENGTH * strength.clamp(0.0, 1.0)
}

/// The own rider's bumps.
#[derive(Debug, Default, Clone)]
pub struct RoadBumps {
    tick: u64,
    recent: VecDeque<(u64, u64)>,
    last: Option<Seen>,
    count: u64,
}

impl RoadBumps {
    /// One prediction tick from `before` to `after` on `track`: the bump
    /// to present, if a new one.
    pub fn observe(
        &mut self,
        track: &dyn Track,
        before: &RiderState,
        after: &RiderState,
    ) -> Option<RoadBump> {
        self.tick += 1;
        while self
            .recent
            .front()
            .is_some_and(|&(_, t)| t + REPEAT_TICKS < self.tick)
        {
            self.recent.pop_front();
        }
        let b = ocw_sim::road_feel::crossed(track, before, after)?;
        if self.recent.iter().any(|&(id, _)| id == b.id) {
            return None;
        }
        self.recent.push_back((b.id, self.tick));
        self.count += 1;
        self.last = Some(Seen {
            bump: b,
            seq: self.count,
            shown_at: OnceLock::new(),
        });
        Some(b)
    }

    /// The last bump presented.
    pub fn last(&self) -> Option<&Seen> {
        self.last.as_ref()
    }

    /// How many bumps were presented.
    pub fn count(&self) -> u64 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kicks_and_shakes_follow_the_strength() {
        assert_eq!(kick_factor(0.0), 1.0);
        assert!((kick_factor(0.8) - 4.2).abs() < 1e-12);
        assert!(kick_factor(0.12) < 1.5);
        assert_eq!(kick_factor(3.0), kick_factor(1.0));
        let seen = |kind, strength| Seen {
            bump: RoadBump {
                kind,
                strength,
                id: 1,
            },
            seq: 1,
            shown_at: OnceLock::new(),
        };
        let hole = seen(BumpKind::Pothole, 0.8);
        let crack = seen(BumpKind::Crack, 0.12);
        // A small shake: well under a crash's.
        assert!((hole.shake_strength() - 0.28).abs() < 1e-12);
        assert!(crack.shake_strength() < 0.05);
        assert_eq!(hole.cue(), ocw_audio::cues::Cue::Pothole);
        assert_eq!(
            seen(BumpKind::EdgeBreak, 0.5).cue(),
            ocw_audio::cues::Cue::Pothole
        );
        assert_eq!(crack.cue(), ocw_audio::cues::Cue::Crack);
        assert_eq!(
            seen(BumpKind::Alligator, 0.18).cue(),
            ocw_audio::cues::Cue::Crack
        );
        // The shake starts when first shown.
        assert_eq!(hole.age_s(10.0), 0.0);
        assert!((hole.age_s(10.25) - 0.25).abs() < 1e-12);
    }
}
