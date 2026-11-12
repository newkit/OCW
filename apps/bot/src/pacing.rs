//! How hard a bot pedals: its own steady power with a slow variation (like
//! the server's bots, spec §6.1), or following a named rider at a gap
//! (`--follow`), so remote riders stay around the kiosk's rider and show
//! up on its screen.

/// Fastest a follower closes a gap, on top of the leader's speed (m/s).
pub const MAX_CATCH_UP_MPS: f64 = 4.0;
/// Gap error (m) → wanted speed difference (m/s).
pub const GAP_GAIN: f64 = 0.25;
/// Speed error (m/s) → extra power (W).
pub const SPEED_GAIN_W: f64 = 80.0;
/// Most power a bot puts out (W).
pub const MAX_POWER_W: f64 = 450.0;

/// What the bot does with the pedals and the brake this moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effort {
    pub power_w: f64,
    pub brake: bool,
}

/// The rider a follower keeps its gap to, from the last snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leader {
    pub distance_m: f64,
    pub speed_mps: f64,
    pub power_w: f64,
}

/// A bot's pacing.
#[derive(Debug, Clone, PartialEq)]
pub struct Pacing {
    /// Steady power (W) when not following.
    pub base_w: f64,
    /// Phase of the slow ±10 % variation (so bots do not ride in lockstep).
    pub phase: f64,
    /// Follow this rider (display name) at `gap_m` ahead (negative: behind).
    pub follow: Option<(String, f64)>,
}

impl Pacing {
    /// The steady power with its variation at time `t_s`.
    pub fn own_power_w(&self, t_s: f64) -> f64 {
        (self.base_w * (1.0 + 0.1 * (t_s * 0.2 + self.phase).sin())).max(0.0)
    }

    /// The effort at time `t_s` for a bot at `distance_m` riding
    /// `speed_mps`; `leader` is the followed rider if it is in the session.
    pub fn effort(
        &self,
        t_s: f64,
        distance_m: f64,
        speed_mps: f64,
        leader: Option<Leader>,
    ) -> Effort {
        match (&self.follow, leader) {
            (Some((_, gap)), Some(l)) => follow(distance_m, speed_mps, l, *gap),
            _ => Effort {
                power_w: self.own_power_w(t_s),
                brake: false,
            },
        }
    }
}

/// Keeps `gap_m` ahead of the leader: a wanted speed from the gap error,
/// then the leader's power plus a correction for the speed error. Brakes
/// when much too fast (the leader stopped, or the bot overshot).
pub fn follow(distance_m: f64, speed_mps: f64, leader: Leader, gap_m: f64) -> Effort {
    let error = leader.distance_m + gap_m - distance_m;
    let wanted =
        (leader.speed_mps + (GAP_GAIN * error).clamp(-MAX_CATCH_UP_MPS, MAX_CATCH_UP_MPS)).max(0.0);
    let power_w = (leader.power_w + SPEED_GAIN_W * (wanted - speed_mps)).clamp(0.0, MAX_POWER_W);
    let brake = speed_mps - wanted > 1.5;
    Effort {
        power_w: if brake { 0.0 } else { power_w },
        brake,
    }
}

/// The gap of the `k`-th follower (0, 1, …): spread out ahead of the
/// leader, so a camera behind the leader sees them all.
pub fn follow_gap_m(k: u32) -> f64 {
    6.0 + 4.0 * f64::from(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leader(distance_m: f64, speed_mps: f64, power_w: f64) -> Leader {
        Leader {
            distance_m,
            speed_mps,
            power_w,
        }
    }

    #[test]
    fn own_power_varies_slowly_around_the_base() {
        let p = Pacing {
            base_w: 200.0,
            phase: 0.0,
            follow: None,
        };
        let samples: Vec<f64> = (0..400)
            .map(|i| p.own_power_w(f64::from(i) * 0.25))
            .collect();
        assert!(samples.iter().all(|w| (180.0..=220.0).contains(w)));
        assert!(samples.iter().any(|w| *w > 215.0) && samples.iter().any(|w| *w < 185.0));
        // Without a leader in the session a follower rides its own power.
        let f = Pacing {
            follow: Some(("Kiosk".into(), 6.0)),
            ..p.clone()
        };
        assert_eq!(f.effort(3.0, 0.0, 5.0, None).power_w, p.own_power_w(3.0));
    }

    #[test]
    fn a_follower_behind_pushes_harder_and_one_ahead_eases_off() {
        let l = leader(500.0, 8.0, 200.0);
        let behind = follow(400.0, 8.0, l, 6.0);
        let at_gap = follow(506.0, 8.0, l, 6.0);
        let ahead = follow(520.0, 8.0, l, 6.0);
        assert!(behind.power_w > 400.0, "{behind:?}");
        assert!((at_gap.power_w - 200.0).abs() < 1e-9, "{at_gap:?}");
        assert!(ahead.power_w < 50.0, "{ahead:?}");
        assert!(!behind.brake && !at_gap.brake);
        // Power stays within what a rider can do.
        assert!(follow(0.0, 0.0, l, 6.0).power_w <= MAX_POWER_W);
    }

    #[test]
    fn a_follower_stops_beside_a_waiting_leader() {
        // The leader waits at the start; the bot arrives at speed: brake.
        let l = leader(10.0, 0.0, 0.0);
        let arriving = follow(14.0, 6.0, l, 6.0);
        assert!(arriving.brake && arriving.power_w == 0.0);
        // At its place and standing: nothing to do.
        let parked = follow(16.0, 0.0, l, 6.0);
        assert_eq!((parked.power_w, parked.brake), (0.0, false));
        // Still short of the place: ride up to it.
        assert!(follow(0.0, 0.0, l, 6.0).power_w > 100.0);
    }

    #[test]
    fn a_closed_loop_holds_the_gap() {
        // Crude flat-road model: speed follows power (no physics crate
        // needed here); the follower ends up near its gap.
        let mut l = leader(50.0, 8.0, 200.0);
        let (mut d, mut v) = (0.0, 0.0);
        for _ in 0..3000 {
            let e = follow(d, v, l, 10.0);
            let target_v = 8.0 * e.power_w / 200.0;
            v += (target_v - v) * 0.02;
            if e.brake {
                v = (v - 0.2).max(0.0);
            }
            d += v * 0.02;
            l.distance_m += l.speed_mps * 0.02;
        }
        let gap = d - l.distance_m;
        assert!((gap - 10.0).abs() < 2.0, "gap {gap}");
    }

    #[test]
    fn gaps_spread_ahead() {
        assert_eq!(follow_gap_m(0), 6.0);
        assert!(follow_gap_m(3) > follow_gap_m(2));
    }
}
