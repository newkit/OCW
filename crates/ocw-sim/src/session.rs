//! Authoritative session logic: all riders of one ride, stepped at a fixed tick (spec §6.1).
//!
//! Used by the remote game server and by the in-process local server alike.

use std::collections::BTreeMap;

use ocw_core::{RiderId, RiderProfile, SIM_DT};
use ocw_world::Route;

use crate::drafting;
use crate::physics::{self, Environment, RiderState};

/// Latest sensor input of a rider.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RiderInput {
    pub power_w: f64,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
}

/// How a rider's power is produced.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Control {
    /// Power comes from a connected client.
    Client,
    /// Server-side test rider with a gently varying power around `base_power_w`.
    Bot { base_power_w: f64 },
}

#[derive(Debug, Clone)]
struct Rider {
    name: String,
    profile: RiderProfile,
    state: RiderState,
    input: RiderInput,
    last_input_seq: u32,
    control: Control,
    /// Lateral offset the rider steers to (client riders only).
    lateral_target: Option<f64>,
    last_steer_seq: u32,
}

/// State of one rider as broadcast to clients.
#[derive(Debug, Clone, PartialEq)]
pub struct RiderSnapshot {
    pub id: RiderId,
    pub name: String,
    pub state: RiderState,
    pub power_w: f64,
    /// Sequence number of the last input from this rider's client the server applied.
    pub last_input_seq: u32,
}

/// One ride: a route plus the riders on it.
#[derive(Debug, Clone)]
pub struct Session {
    route: Route,
    riders: BTreeMap<RiderId, Rider>,
    tick: u64,
    next_id: u32,
}

impl Session {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            riders: BTreeMap::new(),
            tick: 0,
            next_id: 1,
        }
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// Number of physics ticks since the session started.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Session time in seconds.
    pub fn time_s(&self) -> f64 {
        self.tick as f64 * SIM_DT
    }

    pub fn rider_count(&self) -> usize {
        self.riders.len()
    }

    /// Adds a rider controlled by a client.
    pub fn add_rider(&mut self, name: impl Into<String>, profile: RiderProfile) -> RiderId {
        self.insert(name.into(), profile, Control::Client)
    }

    /// Adds a server-side test rider ("bot") riding around `base_power_w`.
    pub fn add_bot(&mut self, name: impl Into<String>, base_power_w: f64) -> RiderId {
        self.insert(
            name.into(),
            RiderProfile::default(),
            Control::Bot { base_power_w },
        )
    }

    fn insert(&mut self, name: String, profile: RiderProfile, control: Control) -> RiderId {
        let id = RiderId(self.next_id);
        self.next_id += 1;
        // Spread riders across the lane: 0, +1.2, -1.2, +2.4, -2.4 m ...
        let slot = self.riders.len() as f64;
        let side = if self.riders.len().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let lateral_offset = side * ((slot + 1.0) / 2.0).floor() * 1.2;
        self.riders.insert(
            id,
            Rider {
                name,
                profile,
                state: RiderState {
                    lateral_offset,
                    ..Default::default()
                },
                input: RiderInput::default(),
                last_input_seq: 0,
                control,
                lateral_target: None,
                last_steer_seq: 0,
            },
        );
        id
    }

    pub fn remove_rider(&mut self, id: RiderId) -> bool {
        self.riders.remove(&id).is_some()
    }

    /// Applies a client input. Inputs with a sequence number not newer than the
    /// last applied one are ignored (reordered or replayed packets, spec §6.4).
    pub fn apply_input(&mut self, id: RiderId, seq: u32, input: RiderInput) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client && seq > r.last_input_seq => {
                r.last_input_seq = seq;
                r.input = RiderInput {
                    power_w: input.power_w.max(0.0),
                    ..input
                };
                true
            }
            _ => false,
        }
    }

    /// Steering intent of a client rider: lateral offset to move to, clamped
    /// to the road (spec §6.1: clients send intents, the server moves the
    /// rider). Stale sequence numbers and bots are ignored.
    pub fn set_lateral_target(&mut self, id: RiderId, seq: u32, target_m: f64) -> bool {
        let limit = physics::lateral_limit(self.route.road_width_m());
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client && seq > r.last_steer_seq => {
                r.last_steer_seq = seq;
                r.lateral_target = Some(target_m.clamp(-limit, limit));
                true
            }
            _ => false,
        }
    }

    /// Moves a rider to `distance` metres (dev command, local server only).
    pub fn teleport(&mut self, id: RiderId, distance: f64) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) => {
                r.state.distance = distance.max(0.0);
                true
            }
            None => false,
        }
    }

    /// Advances the session by one physics tick.
    pub fn step(&mut self) {
        self.tick += 1;
        let t = self.time_s();
        // Drafting uses everyone's position at the start of the tick.
        let positions: Vec<(RiderId, f64, f64)> = self
            .riders
            .iter()
            .map(|(id, r)| (*id, r.state.distance, r.state.lateral_offset))
            .collect();
        for (id, rider) in &mut self.riders {
            let drag = drafting::drag_factor(
                (rider.state.distance, rider.state.lateral_offset),
                positions
                    .iter()
                    .filter(|(other, _, _)| other != id)
                    .map(|(_, d, l)| (*d, *l)),
            );
            if let Control::Bot { base_power_w } = rider.control {
                // Deterministic variation so bots don't ride in perfect lockstep.
                let phase = id.0 as f64 * 1.7;
                rider.input.power_w = base_power_w * (1.0 + 0.1 * (t * 0.2 + phase).sin());
            }
            let env = Environment::with_grade(self.route.sample(rider.state.distance).grade)
                .with_drag_factor(drag);
            physics::step(
                &mut rider.state,
                &rider.profile,
                rider.input.power_w,
                &env,
                SIM_DT,
            );
            if let Some(target) = rider.lateral_target {
                rider.state.lateral_offset =
                    physics::steer(rider.state.lateral_offset, target, SIM_DT);
            }
        }
    }

    /// Current state of all riders.
    pub fn snapshot(&self) -> Vec<RiderSnapshot> {
        self.riders
            .iter()
            .map(|(id, r)| RiderSnapshot {
                id: *id,
                name: r.name.clone(),
                state: r.state,
                power_w: r.input.power_w,
                last_input_seq: r.last_input_seq,
            })
            .collect()
    }

    pub fn rider_state(&self, id: RiderId) -> Option<RiderState> {
        self.riders.get(&id).map(|r| r.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_inputs_are_ignored() {
        let mut s = Session::new(Route::test_loop());
        let id = s.add_rider("a", RiderProfile::default());
        let input = |p| RiderInput {
            power_w: p,
            ..Default::default()
        };
        assert!(s.apply_input(id, 2, input(200.0)));
        assert!(!s.apply_input(id, 1, input(900.0)));
        assert!(!s.apply_input(id, 2, input(900.0)));
        assert_eq!(s.snapshot()[0].power_w, 200.0);
    }

    #[test]
    fn bots_ride_and_clients_cannot_drive_bots() {
        let mut s = Session::new(Route::test_loop());
        let bot = s.add_bot("bot", 200.0);
        assert!(!s.apply_input(bot, 1, RiderInput::default()));
        for _ in 0..1500 {
            s.step();
        }
        // 30 s at ~200 W from standstill.
        assert!(s.rider_state(bot).unwrap().distance > 150.0);
        assert_eq!(s.tick(), 1500);
    }

    #[test]
    fn riders_get_separate_lanes() {
        let mut s = Session::new(Route::test_loop());
        let offsets: Vec<f64> = (0..3)
            .map(|i| {
                let id = s.add_bot(format!("b{i}"), 100.0);
                s.rider_state(id).unwrap().lateral_offset
            })
            .collect();
        assert_eq!(offsets, vec![0.0, -1.2, 1.2]);
    }

    #[test]
    fn rider_behind_catches_up_by_drafting() {
        let mut s = Session::new(Route::test_loop());
        let leader = s.add_rider("Leader", RiderProfile::default());
        let follower = s.add_rider("Follower", RiderProfile::default());
        s.teleport(leader, 1005.0);
        s.teleport(follower, 1000.0);
        for (seq, id) in [(1, leader), (1, follower)] {
            s.apply_input(
                id,
                seq,
                RiderInput {
                    power_w: 250.0,
                    cadence_rpm: None,
                    heart_rate_bpm: None,
                },
            );
        }
        for _ in 0..(60 * 50) {
            s.step();
        }
        let gap =
            s.rider_state(leader).unwrap().distance - s.rider_state(follower).unwrap().distance;
        // Same power: without drafting the gap would stay at 5 m.
        assert!(gap < 4.5, "gap {gap}");
    }

    #[test]
    fn client_riders_steer_across_the_road() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("Me", RiderProfile::default());
        let bot = s.add_bot("Bot", 200.0);
        let start = s.rider_state(me).unwrap().lateral_offset;
        assert!(s.set_lateral_target(me, 1, 2.0));
        for _ in 0..25 {
            s.step(); // 0.5 s
        }
        let moved = s.rider_state(me).unwrap().lateral_offset - start;
        assert!((moved - 0.75).abs() < 1e-6, "{moved}");
        // Clamped to the road (7 m wide → ±2.9 m).
        assert!(s.set_lateral_target(me, 2, 10.0));
        for _ in 0..500 {
            s.step();
        }
        assert!((s.rider_state(me).unwrap().lateral_offset - 2.9).abs() < 1e-9);
        // Stale intents and bots are ignored.
        assert!(!s.set_lateral_target(me, 2, -2.0));
        assert!(!s.set_lateral_target(bot, 3, 0.0));
    }
}
