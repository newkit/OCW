//! Spatial voice (spec §7, §2.4): voices of riders close by on the road
//! are loud, riders far ahead or behind fade; the client subscribes only to
//! the nearest N riders (plus pinned ones: group and event rooms, where
//! everyone is a chosen partner) and pans by the lateral offset.

use ocw_core::RiderId;

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
    /// Faded down to `min_gain` at this distance; beyond it the voice is
    /// not heard at all.
    pub range_m: f64,
    /// Volume at the edge of the range.
    pub min_gain: f32,
    /// At most this many voices are subscribed (nearest first).
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
            full_m: 10.0,
            range_m: 300.0,
            min_gain: 0.15,
            max_voices: 10,
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
    /// Gain by distance: 1 up to `full_m`, then falling like 1/distance
    /// (normalised to `full_m`) but never below `min_gain` inside the range;
    /// 0 outside it.
    pub fn gain(&self, distance_m: f64) -> f32 {
        let d = distance_m.abs();
        if d <= self.full_m {
            1.0
        } else if d > self.range_m {
            0.0
        } else {
            ((self.full_m / d) as f32).max(self.min_gain)
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

/// The voices to hear: the nearest `max_voices` riders within range,
/// preferring those already subscribed (`current`), with their placement.
/// Sorted by rider id.
pub fn select(
    peers: &[Peer],
    current: &[RiderId],
    cfg: &SpatialConfig,
) -> Vec<(RiderId, Placement)> {
    let mut ranked: Vec<(f64, &Peer)> = peers
        .iter()
        .filter(|p| p.gap_m.is_finite() && p.gap_m.abs() <= cfg.range_m)
        .map(|p| {
            let bonus = if current.contains(&p.rider) {
                cfg.keep_bonus_m
            } else {
                0.0
            };
            (p.gap_m.abs() - bonus, p)
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.rider.cmp(&b.1.rider)));
    let mut out: Vec<(RiderId, Placement)> = ranked
        .into_iter()
        .take(cfg.max_voices)
        .map(|(_, p)| (p.rider, cfg.place(p)))
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

    #[test]
    fn gain_falls_with_distance_both_ways() {
        let c = SpatialConfig::default();
        assert_eq!(c.gain(0.0), 1.0);
        assert_eq!(c.gain(-10.0), 1.0);
        assert!((c.gain(20.0) - 0.5).abs() < 1e-6);
        // Riders behind sound quieter than adjacent ones (spec §2.4).
        assert!(c.gain(-40.0) < c.gain(-5.0));
        assert_eq!(c.gain(-40.0), c.gain(40.0));
        // Monotone, floored inside the range, silent outside.
        let mut last = 1.0;
        for d in (0..=300).step_by(5) {
            let g = c.gain(f64::from(d));
            assert!((c.min_gain..=last).contains(&g), "{d}: {g}");
            last = g;
        }
        assert_eq!(c.gain(300.1), 0.0);
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
        ];
        let ids: Vec<u32> = select(&peers, &[], &c).iter().map(|(r, _)| r.0).collect();
        assert_eq!(ids, vec![2, 3, 4]);
        // Fewer than N in range: all of them, nobody out of range.
        let c = SpatialConfig::default();
        let ids: Vec<u32> = select(&peers, &[], &c).iter().map(|(r, _)| r.0).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        assert!(select(&[], &[], &c).is_empty());
    }

    #[test]
    fn subscribed_voices_are_kept_near_the_boundary() {
        let c = SpatialConfig {
            max_voices: 1,
            ..Default::default()
        };
        // Rider 2 is a little nearer, but rider 1 is already heard.
        let peers = [peer(1, 30.0), peer(2, 25.0)];
        assert_eq!(select(&peers, &[RiderId(1)], &c)[0].0, RiderId(1));
        assert_eq!(select(&peers, &[], &c)[0].0, RiderId(2));
        // Much nearer wins anyway.
        let peers = [peer(1, 30.0), peer(2, 5.0)];
        assert_eq!(select(&peers, &[RiderId(1)], &c)[0].0, RiderId(2));
    }
}
