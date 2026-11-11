//! Authoritative session logic: all riders of one ride, stepped at a fixed tick (spec §6.1).
//!
//! Used by the remote game server and by the in-process local server alike.

use std::collections::BTreeMap;

use ocw_core::{RiderId, RiderProfile, SIM_DT};
use ocw_world::Route;

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
        for (id, rider) in &mut self.riders {
            if let Control::Bot { base_power_w } = rider.control {
                // Deterministic variation so bots don't ride in perfect lockstep.
                let phase = id.0 as f64 * 1.7;
                rider.input.power_w = base_power_w * (1.0 + 0.1 * (t * 0.2 + phase).sin());
            }
            let env = Environment::with_grade(self.route.sample(rider.state.distance).grade);
            physics::step(
                &mut rider.state,
                &rider.profile,
                rider.input.power_w,
                &env,
                SIM_DT,
            );
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
}
