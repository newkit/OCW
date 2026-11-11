//! Interpolation of other riders between server snapshots (spec §6.3).

use std::collections::{BTreeMap, VecDeque};

use ocw_core::RiderId;

use crate::physics::RiderState;

const MAX_SNAPSHOTS: usize = 32;

/// Buffers received snapshots and returns rider states at a render time that
/// lies slightly in the past (the interpolation delay).
#[derive(Debug, Clone, Default)]
pub struct SnapshotBuffer {
    /// (receive time in seconds, states)
    snapshots: VecDeque<(f64, BTreeMap<RiderId, RiderState>)>,
}

impl SnapshotBuffer {
    pub fn push(&mut self, time_s: f64, states: impl IntoIterator<Item = (RiderId, RiderState)>) {
        self.snapshots
            .push_back((time_s, states.into_iter().collect()));
        while self.snapshots.len() > MAX_SNAPSHOTS {
            self.snapshots.pop_front();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Rider states at `render_time_s`. Riders present in both surrounding
    /// snapshots are interpolated; otherwise the newest known state is used.
    pub fn sample(&self, render_time_s: f64) -> BTreeMap<RiderId, RiderState> {
        let Some((_, newest)) = self.snapshots.back() else {
            return BTreeMap::new();
        };
        let later = self.snapshots.iter().position(|(t, _)| *t >= render_time_s);
        let (a, b) = match later {
            Some(0) | None => return newest.clone(),
            Some(i) => (&self.snapshots[i - 1], &self.snapshots[i]),
        };
        let span = b.0 - a.0;
        let t = if span > 0.0 {
            ((render_time_s - a.0) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        b.1.iter()
            .map(|(id, sb)| {
                let s = match a.1.get(id) {
                    Some(sa) => RiderState {
                        distance: sa.distance + (sb.distance - sa.distance) * t,
                        speed: sa.speed + (sb.speed - sa.speed) * t,
                        lateral_offset: sa.lateral_offset
                            + (sb.lateral_offset - sa.lateral_offset) * t,
                    },
                    None => *sb,
                };
                (*id, s)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(distance: f64) -> RiderState {
        RiderState {
            distance,
            speed: 10.0,
            lateral_offset: 0.0,
        }
    }

    #[test]
    fn interpolates_between_snapshots() {
        let mut b = SnapshotBuffer::default();
        b.push(1.0, [(RiderId(1), st(10.0))]);
        b.push(1.1, [(RiderId(1), st(11.0))]);
        let s = b.sample(1.05);
        assert!((s[&RiderId(1)].distance - 10.5).abs() < 1e-9);
    }

    #[test]
    fn uses_newest_outside_range_and_for_new_riders() {
        let mut b = SnapshotBuffer::default();
        assert!(b.sample(0.0).is_empty());
        b.push(1.0, [(RiderId(1), st(10.0))]);
        b.push(1.1, [(RiderId(1), st(11.0)), (RiderId(2), st(3.0))]);
        assert_eq!(b.sample(5.0)[&RiderId(1)].distance, 11.0);
        assert_eq!(b.sample(1.05)[&RiderId(2)].distance, 3.0);
    }
}
