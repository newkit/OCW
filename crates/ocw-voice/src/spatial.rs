//! Spatial voice (spec §7, §2.4, §7.1.7 Q6 / Q10): voices of riders close
//! by on the road are at full volume up to 10 m, then fade to **zero** at
//! 300 m (no floor: beyond that a rider is not heard at all); the client
//! subscribes to at most N voices (24), riders who talk now or did a moment
//! ago first, then the nearest; it pans by the lateral offset. Who may be
//! heard at all is the game server's (`VoiceHearing`: the hearing range
//! with a margin, the audiences and the blocks); this module only ranks
//! and places.

use std::collections::BTreeSet;

use ocw_core::RiderId;

/// The hearing range (spec §7.1.7 Q10): full volume up to [`FULL_M`], then
/// fading to zero at [`RANGE_M`].
pub const FULL_M: f64 = 10.0;
pub const RANGE_M: f64 = 300.0;
/// The default cap on simultaneous voices (spec §7.1.2).
pub const MAX_VOICES: usize = 24;
/// The gain of a rider the game server lets the listener hear whose place
/// on the road is not known yet (joining, a test without a view).
pub const UNPLACED_GAIN: f32 = 0.15;

/// A remote rider as the listener sees them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peer {
    pub rider: RiderId,
    /// Distance from the listener on the road (m; positive = ahead). On a
    /// route it is the gap along the route; in free ride, where riders
    /// ride their own paths, the straight-line distance (an approximation
    /// documented in spec §7).
    pub gap_m: f64,
    /// Lateral offset relative to the listener (m; positive = right).
    pub lateral_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialConfig {
    /// Full volume up to this distance.
    pub full_m: f64,
    /// Faded down to zero at this distance; beyond it the voice is not
    /// heard at all.
    pub range_m: f64,
    /// At most this many voices are subscribed (talking first, then the
    /// nearest).
    pub max_voices: usize,
    /// A voice already subscribed counts this much nearer when ranking,
    /// so two riders at almost the same distance do not swap all the time.
    pub keep_bonus_m: f64,
    /// Pan by the lateral offset (at most ±`max_pan`, full at
    /// `pan_width_m`).
    pub max_pan: f32,
    pub pan_width_m: f64,
}

impl Default for SpatialConfig {
    fn default() -> Self {
        Self {
            full_m: FULL_M,
            range_m: RANGE_M,
            max_voices: MAX_VOICES,
            keep_bonus_m: 15.0,
            max_pan: 0.5,
            pan_width_m: 4.0,
        }
    }
}

/// How one voice is played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub gain: f32,
    /// −1 (left) … 1 (right).
    pub pan: f32,
}

impl SpatialConfig {
    /// Gain by distance (spec §7.1.7 Q10): 1 up to `full_m`, then falling
    /// like 1/distance (normalised to `full_m`) times a smooth taper that
    /// reaches zero at `range_m` (smoothstep: no step at either end); 0 at
    /// and beyond it.
    pub fn gain(&self, distance_m: f64) -> f32 {
        let d = distance_m.abs();
        if !d.is_finite() || d >= self.range_m {
            0.0
        } else if d <= self.full_m {
            1.0
        } else {
            let t = ((d - self.full_m) / (self.range_m - self.full_m)).clamp(0.0, 1.0);
            let taper = 1.0 - t * t * (3.0 - 2.0 * t);
            ((self.full_m / d) * taper) as f32
        }
    }

    pub fn pan(&self, lateral_m: f64, distance_m: f64) -> f32 {
        // Pan only riders beside the listener; far ahead or behind they are
        // in front or at the back, not left or right.
        let beside = (1.0 - distance_m.abs() / (2.0 * self.full_m)).clamp(0.0, 1.0);
        ((lateral_m / self.pan_width_m).clamp(-1.0, 1.0) * beside) as f32 * self.max_pan
    }

    pub fn place(&self, p: &Peer) -> Placement {
        Placement {
            gain: self.gain(p.gap_m),
            pan: self.pan(p.lateral_m, p.gap_m),
        }
    }
}

/// The voices to hear: at most `max_voices` riders within range – those
/// who talk (now or a moment ago, `talking`) first, then the nearest –
/// preferring those already subscribed (`current`), with their
/// placement. Sorted by rider id.
pub fn select(
    peers: &[Peer],
    current: &[RiderId],
    talking: &BTreeSet<RiderId>,
    cfg: &SpatialConfig,
) -> Vec<(RiderId, Placement)> {
    let mut ranked: Vec<(bool, f64, &Peer)> = peers
        .iter()
        .filter(|p| p.gap_m.is_finite() && p.gap_m.abs() < cfg.range_m)
        .map(|p| {
            let bonus = if current.contains(&p.rider) {
                cfg.keep_bonus_m
            } else {
                0.0
            };
            (!talking.contains(&p.rider), p.gap_m.abs() - bonus, p)
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.rider.cmp(&b.2.rider))
    });
    let mut out: Vec<(RiderId, Placement)> = ranked
        .into_iter()
        .take(cfg.max_voices)
        .map(|(_, _, p)| (p.rider, cfg.place(p)))
        .collect();
    out.sort_by_key(|(r, _)| *r);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: u32, gap: f64) -> Peer {
        Peer {
            rider: RiderId(id),
            gap_m: gap,
            lateral_m: 0.0,
        }
    }

    fn ids(v: &[(RiderId, Placement)]) -> Vec<u32> {
        v.iter().map(|(r, _)| r.0).collect()
    }

    /// Spec §7.1.7 Q6 / Q10: full volume up to 10 m, then fading smoothly
    /// to zero at 300 m – no floor, silent beyond.
    #[test]
    fn gain_is_full_to_10_m_then_fades_to_zero_at_300_m() {
        let c = SpatialConfig::default();
        assert_eq!(c.gain(0.0), 1.0);
        assert_eq!(c.gain(-10.0), 1.0);
        assert_eq!(c.gain(10.0), 1.0);
        // About 10 m / distance near the rider.
        assert!((c.gain(20.0) - 0.5).abs() < 0.01, "{}", c.gain(20.0));
        // Riders behind sound quieter than adjacent ones (spec §2.4).
        assert!(c.gain(-40.0) < c.gain(-5.0));
        assert_eq!(c.gain(-40.0), c.gain(40.0));
        // Monotone, no floor: down to zero at the end of the range.
        let mut last = 1.0;
        for d in 0..=700 {
            let g = c.gain(f64::from(d) * 0.5);
            assert!((0.0..=last).contains(&g), "{d}: {g}");
            last = g;
        }
        assert!(c.gain(200.0) < 0.02, "{}", c.gain(200.0));
        assert!(c.gain(299.0) < 1e-4, "smooth at the end: {}", c.gain(299.0));
        assert_eq!(c.gain(300.0), 0.0);
        assert_eq!(c.gain(300.1), 0.0);
        assert_eq!(c.gain(5000.0), 0.0);
        assert_eq!(c.gain(f64::NAN), 0.0);
        // Continuous at 10 m.
        assert!((c.gain(10.001) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn pan_only_beside_the_listener() {
        let c = SpatialConfig::default();
        assert_eq!(c.pan(0.0, 0.0), 0.0);
        assert!((c.pan(4.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((c.pan(-10.0, 0.0) + 0.5).abs() < 1e-6, "clamped");
        assert!(c.pan(2.0, 5.0) > 0.0 && c.pan(2.0, 5.0) < c.pan(2.0, 0.0));
        assert_eq!(c.pan(2.0, 50.0), 0.0);
    }

    #[test]
    fn nearest_n_within_range() {
        let none = BTreeSet::new();
        let c = SpatialConfig {
            max_voices: 3,
            ..Default::default()
        };
        let peers = [
            peer(1, 200.0),
            peer(2, -5.0),
            peer(3, 50.0),
            peer(4, -80.0),
            peer(5, 400.0),
            peer(6, f64::NAN),
            peer(7, 300.0),
        ];
        assert_eq!(ids(&select(&peers, &[], &none, &c)), vec![2, 3, 4]);
        // Fewer than N in range: all of them, nobody out of range.
        let c = SpatialConfig::default();
        assert_eq!(ids(&select(&peers, &[], &none, &c)), vec![1, 2, 3, 4]);
        assert!(select(&[], &[], &none, &c).is_empty());
    }

    /// The cap of 24 voices (spec §7.1.2): with more riders in range the
    /// ones who talk come first, then the nearest.
    #[test]
    fn the_cap_prefers_talking_riders_then_the_nearest() {
        let c = SpatialConfig::default();
        assert_eq!(c.max_voices, 24);
        let peers: Vec<Peer> = (1..=40).map(|i| peer(i, f64::from(i) * 5.0)).collect();
        let none = BTreeSet::new();
        assert_eq!(
            ids(&select(&peers, &[], &none, &c)),
            (1..=24).collect::<Vec<_>>(),
            "the nearest 24"
        );
        // Riders 30 and 35 talk: they are heard, the two farthest of the
        // nearest 24 make room.
        let talking: BTreeSet<RiderId> = [RiderId(30), RiderId(35)].into();
        let mut want: Vec<u32> = (1..=22).collect();
        want.extend([30, 35]);
        assert_eq!(ids(&select(&peers, &[], &talking, &c)), want);
        // A talking rider out of range is still not heard.
        let talking: BTreeSet<RiderId> = [RiderId(99)].into();
        let far = [peer(99, 450.0), peer(1, 20.0)];
        assert_eq!(ids(&select(&far, &[], &talking, &c)), vec![1]);
    }

    #[test]
    fn subscribed_voices_are_kept_near_the_boundary() {
        let none = BTreeSet::new();
        let c = SpatialConfig {
            max_voices: 1,
            ..Default::default()
        };
        // Rider 2 is a little nearer, but rider 1 is already heard.
        let peers = [peer(1, 30.0), peer(2, 25.0)];
        assert_eq!(select(&peers, &[RiderId(1)], &none, &c)[0].0, RiderId(1));
        assert_eq!(select(&peers, &[], &none, &c)[0].0, RiderId(2));
        // Much nearer wins anyway.
        let peers = [peer(1, 30.0), peer(2, 5.0)];
        assert_eq!(select(&peers, &[RiderId(1)], &none, &c)[0].0, RiderId(2));
    }
}
