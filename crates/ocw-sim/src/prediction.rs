//! Client-side prediction of the local rider (spec §5.2, §6.1).

use ocw_core::{RiderProfile, SIM_DT};
use ocw_world::Route;

use crate::physics::{self, Environment, RiderState};

/// Errors above this distance (m) are corrected immediately instead of smoothed.
pub const SNAP_THRESHOLD_M: f64 = 5.0;
/// Time over which smaller errors are blended away, in seconds.
pub const CORRECTION_TIME_S: f64 = 0.5;

/// Predicts the local rider so the bike reacts to pedaling immediately, and
/// converges smoothly to the authoritative server state.
#[derive(Debug, Clone)]
pub struct Predictor {
    profile: RiderProfile,
    state: RiderState,
    /// Remaining distance error still to be blended in.
    pending_correction: f64,
    /// Metres per second at which the pending correction is applied.
    correction_rate: f64,
    /// Distance error measured at the last reconciliation (for diagnostics).
    last_error: f64,
    /// Drafting factor on the drag, predicted by the client (1.0 = none).
    drag_factor: f64,
    /// Steering target (same rule as the server, so steering reacts at once).
    lateral_target: Option<f64>,
    /// Remaining lateral error to blend in, and its rate (m/s).
    pending_lateral: f64,
    lateral_rate: f64,
}

impl Predictor {
    pub fn new(profile: RiderProfile, initial: RiderState) -> Self {
        Self {
            profile,
            state: initial,
            pending_correction: 0.0,
            correction_rate: 0.0,
            last_error: 0.0,
            drag_factor: 1.0,
            lateral_target: None,
            pending_lateral: 0.0,
            lateral_rate: 0.0,
        }
    }

    pub fn state(&self) -> RiderState {
        self.state
    }

    /// Distance error (server − prediction) at the last reconciliation, in metres.
    pub fn last_error(&self) -> f64 {
        self.last_error
    }

    /// Sets the steering target (already clamped to the road by the caller).
    pub fn set_lateral_target(&mut self, target_m: f64) {
        self.lateral_target = Some(target_m);
    }

    /// Sets the predicted drafting factor (from the positions of others).
    pub fn set_drag_factor(&mut self, drag_factor: f64) {
        self.drag_factor = drag_factor;
    }

    /// Advances the prediction by one physics tick with the current power.
    pub fn step(&mut self, route: &Route, power_w: f64) {
        let env = Environment::with_grade(route.sample(self.state.distance).grade)
            .with_drag_factor(self.drag_factor);
        physics::step(&mut self.state, &self.profile, power_w, &env, SIM_DT);
        if let Some(target) = self.lateral_target {
            self.state.lateral_offset = physics::steer(self.state.lateral_offset, target, SIM_DT);
        }
        if self.pending_lateral != 0.0 {
            let max_step = self.lateral_rate.abs() * SIM_DT;
            let applied = self.pending_lateral.clamp(-max_step, max_step);
            self.state.lateral_offset += applied;
            self.pending_lateral -= applied;
        }
        if self.pending_correction != 0.0 {
            // Apply the error linearly over CORRECTION_TIME_S.
            let max_step = self.correction_rate.abs() * SIM_DT;
            let applied = self.pending_correction.clamp(-max_step, max_step);
            self.state.distance += applied;
            self.pending_correction -= applied;
        }
    }

    /// Reconciles with an authoritative state that is `age_s` seconds old.
    ///
    /// The server state is extrapolated to "now" with the same physics and the
    /// current power, then compared with the prediction.
    pub fn reconcile(&mut self, route: &Route, server: RiderState, age_s: f64, power_w: f64) {
        let mut extrapolated = server;
        let steps = (age_s.max(0.0) / SIM_DT).round() as usize;
        for _ in 0..steps {
            let env = Environment::with_grade(route.sample(extrapolated.distance).grade)
                .with_drag_factor(self.drag_factor);
            physics::step(&mut extrapolated, &self.profile, power_w, &env, SIM_DT);
            if let Some(target) = self.lateral_target {
                extrapolated.lateral_offset =
                    physics::steer(extrapolated.lateral_offset, target, SIM_DT);
            }
        }
        let error = extrapolated.distance - self.state.distance;
        self.last_error = error;
        if error.abs() > SNAP_THRESHOLD_M {
            self.state = extrapolated;
            self.pending_correction = 0.0;
            self.pending_lateral = 0.0;
        } else {
            self.pending_correction = error;
            self.correction_rate = error / CORRECTION_TIME_S;
            self.state.speed = extrapolated.speed;
            // Lateral errors are blended too (the steering intent reaches
            // the server a little later than the prediction reacts).
            let lateral_error = extrapolated.lateral_offset - self.state.lateral_offset;
            self.pending_lateral = lateral_error;
            self.lateral_rate = lateral_error / CORRECTION_TIME_S;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Route {
        Route::from_points(
            "flat",
            "Flat",
            vec![[0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap()
    }

    #[test]
    fn small_errors_are_blended() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut server = p.state();
        server.distance += 1.0;
        p.reconcile(&route, server, 0.0, 200.0);
        assert!((p.last_error() - 1.0).abs() < 1e-9);
        let before = p.state().distance;
        p.step(&route, 200.0);
        // Not snapped: only a fraction of the error is applied per tick.
        assert!(p.state().distance - before < 0.5);
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut reference = server;
        for _ in 0..101 {
            physics::step(
                &mut reference,
                &RiderProfile::default(),
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        assert!((p.state().distance - reference.distance).abs() < 0.05);
    }

    #[test]
    fn large_errors_snap() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let server = RiderState {
            distance: 50.0,
            speed: 8.0,
            lateral_offset: 0.0,
        };
        p.reconcile(&route, server, 0.0, 200.0);
        assert_eq!(p.state(), server);
    }

    #[test]
    fn agreeing_server_causes_no_correction() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let mut server = RiderState::default();
        for tick in 1..=500 {
            p.step(&route, 220.0);
            physics::step(
                &mut server,
                &RiderProfile::default(),
                220.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
            if tick % 5 == 0 {
                p.reconcile(&route, server, 0.0, 220.0);
                assert!(p.last_error().abs() < 1e-9);
            }
        }
        assert_eq!(p.state(), server);
    }

    #[test]
    fn old_server_state_is_extrapolated_before_comparing() {
        let route = flat();
        let profile = RiderProfile::default();
        let mut server = RiderState {
            distance: 100.0,
            speed: 9.0,
            lateral_offset: 0.0,
        };
        let old = server;
        // The server state is 0.1 s old; meanwhile we rode on.
        for _ in 0..5 {
            physics::step(
                &mut server,
                &profile,
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        let mut p = Predictor::new(profile, server);
        p.reconcile(&route, old, 0.1, 200.0);
        assert!(p.last_error().abs() < 1e-9, "error {}", p.last_error());
    }
}
