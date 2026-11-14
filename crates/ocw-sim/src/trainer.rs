//! Smart trainer control (spec §5.3): what to send to the trainer, and when.
//!
//! Pure logic; time is passed in (seconds on any monotonic clock), so the same
//! code runs in tests, natively and in wasm. The device backend turns the
//! results into FTMS control point writes.
//!
//! - [`SimController`]: SIM mode. Road gradient → trainer gradient with trainer
//!   difficulty, separate downhill handling, look-ahead, a rate limit and a
//!   1–2 Hz update rate.
//! - [`ErgController`]: ERG mode. Target power with ramped steps and a
//!   "catch-up" when the cadence drops (avoids the ERG "spiral of death").

use ocw_core::{AIR_DENSITY, RiderProfile};

/// Settings of SIM mode. Defaults follow spec §5.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimSettings {
    /// Trainer difficulty, 0.0–1.0: scales the road gradient (default 0.5).
    pub difficulty: f64,
    /// Extra factor on negative gradients after the difficulty (default 0.5).
    pub downhill_scale: f64,
    /// Most negative gradient sent (default −4 %), so the trainer never
    /// spins out on long descents.
    pub downhill_limit: f64,
    /// Steepest gradient sent (default 20 %).
    pub uphill_limit: f64,
    /// Maximum change of the sent gradient per second (default 1 %/s).
    pub max_rate_per_s: f64,
    /// Minimum time between updates (default 0.5 s → at most 2 Hz).
    pub update_interval_s: f64,
    /// Resend an unchanged gradient after this long (default 5 s), in case a
    /// write was lost.
    pub keepalive_s: f64,
    /// Changes smaller than this are not sent (default 0.05 %).
    pub min_change: f64,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            difficulty: 0.5,
            downhill_scale: 0.5,
            downhill_limit: -0.04,
            uphill_limit: 0.20,
            max_rate_per_s: 0.01,
            update_interval_s: 0.5,
            keepalive_s: 5.0,
            min_change: 0.0005,
        }
    }
}

impl SimSettings {
    /// The gradient the trainer should simulate for a road gradient, before
    /// rate limiting.
    pub fn trainer_grade(&self, road_grade: f64) -> f64 {
        let g = road_grade * self.difficulty.clamp(0.0, 1.0);
        let g = if g < 0.0 { g * self.downhill_scale } else { g };
        g.clamp(self.downhill_limit, self.uphill_limit)
    }
}

/// SIM mode parameters for one FTMS *Set Indoor Bike Simulation Parameters* write.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimTarget {
    /// Gradient as rise over run.
    pub grade: f64,
    pub wind_speed_mps: f64,
    pub crr: f64,
    /// Wind resistance coefficient 0.5 · ρ · CdA, in kg/m.
    pub cw_kg_per_m: f64,
}

/// Turns the road gradient into rate-limited trainer updates.
#[derive(Debug, Clone)]
pub struct SimController {
    settings: SimSettings,
    crr: f64,
    cw_kg_per_m: f64,
    /// Rate-limited gradient (what the trainer is moving towards).
    current: Option<f64>,
    last_update_s: Option<f64>,
    last_sent: Option<(f64, f64, f64)>, // (time, grade, rolling factor)
    /// The coefficients changed since the last target ([`Self::set_profile`]).
    coefficients_changed: bool,
}

impl SimController {
    pub fn new(settings: SimSettings, profile: &RiderProfile) -> Self {
        Self {
            settings,
            crr: profile.crr,
            cw_kg_per_m: 0.5 * AIR_DENSITY * profile.cda_m2,
            current: None,
            last_update_s: None,
            last_sent: None,
            coefficients_changed: false,
        }
    }

    /// The rider's physics changed (another bike, parts, kit: the build in
    /// force, spec §2.5): the trainer gets the new rolling and wind
    /// resistance with the next update interval.
    pub fn set_profile(&mut self, profile: &RiderProfile) {
        let cw = 0.5 * AIR_DENSITY * profile.cda_m2;
        if profile.crr != self.crr || cw != self.cw_kg_per_m {
            self.crr = profile.crr;
            self.cw_kg_per_m = cw;
            self.coefficients_changed = true;
        }
    }

    pub fn settings(&self) -> &SimSettings {
        &self.settings
    }

    /// Changes the settings (e.g. the difficulty slider); takes effect at the
    /// next update, still rate limited.
    pub fn set_settings(&mut self, settings: SimSettings) {
        self.settings = settings;
    }

    /// Advances to time `now_s` with the road gradient under the rider and a
    /// short distance ahead (look-ahead, e.g. one second at the current
    /// speed). Returns a target when one should be sent now.
    pub fn update(&mut self, now_s: f64, grade_here: f64, grade_ahead: f64) -> Option<SimTarget> {
        self.update_on(now_s, grade_here, grade_ahead, 1.0)
    }

    /// [`SimController::update`] on a surface with this rolling resistance
    /// factor (gravel, grass: harder to pedal); a change is sent at once.
    pub fn update_on(
        &mut self,
        now_s: f64,
        grade_here: f64,
        grade_ahead: f64,
        rolling_factor: f64,
    ) -> Option<SimTarget> {
        // Look-ahead: aim between the gradient here and the one ahead, so the
        // trainer starts changing before the rider reaches a gradient change.
        let target = self
            .settings
            .trainer_grade(0.5 * (grade_here + grade_ahead));
        let current = match (self.current, self.last_update_s) {
            (Some(c), Some(t)) => {
                let max_step = self.settings.max_rate_per_s * (now_s - t).max(0.0);
                c + (target - c).clamp(-max_step, max_step)
            }
            // First update: start at the target.
            _ => target,
        };
        self.current = Some(current);
        self.last_update_s = Some(now_s);

        let send = match self.last_sent {
            None => true,
            Some((t, sent, factor)) => {
                let since = now_s - t;
                since >= self.settings.update_interval_s
                    && ((current - sent).abs() >= self.settings.min_change
                        || (rolling_factor - factor).abs() > 1e-9
                        || self.coefficients_changed
                        || since >= self.settings.keepalive_s)
            }
        };
        if !send {
            return None;
        }
        self.last_sent = Some((now_s, current, rolling_factor));
        self.coefficients_changed = false;
        Some(SimTarget {
            grade: current,
            wind_speed_mps: 0.0,
            crr: self.crr * rolling_factor,
            cw_kg_per_m: self.cw_kg_per_m,
        })
    }
}

/// Settings of ERG mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErgSettings {
    /// Maximum change of the sent target per second (default 50 W/s), so an
    /// interval step ramps over a few seconds instead of hitting at once.
    pub ramp_w_per_s: f64,
    /// Below this cadence the catch-up starts (default 55 rpm).
    pub catch_up_below_rpm: f64,
    /// Cadence must stay low this long before the catch-up starts (default 1 s).
    pub catch_up_delay_s: f64,
    /// Target factor during the catch-up (default 0.6).
    pub catch_up_factor: f64,
    /// The catch-up ends once the cadence is back above this (default 70 rpm).
    pub recover_above_rpm: f64,
    /// Minimum time between updates (default 1 s).
    pub update_interval_s: f64,
    /// Resend an unchanged target after this long (default 5 s).
    pub keepalive_s: f64,
}

impl Default for ErgSettings {
    fn default() -> Self {
        Self {
            ramp_w_per_s: 50.0,
            catch_up_below_rpm: 55.0,
            catch_up_delay_s: 1.0,
            catch_up_factor: 0.6,
            recover_above_rpm: 70.0,
            update_interval_s: 1.0,
            keepalive_s: 5.0,
        }
    }
}

/// Turns workout target power into ERG updates.
#[derive(Debug, Clone)]
pub struct ErgController {
    settings: ErgSettings,
    sent_target: Option<f64>,
    current: Option<f64>,
    last_update_s: Option<f64>,
    last_sent_s: Option<f64>,
    low_cadence_since: Option<f64>,
    catching_up: bool,
}

impl ErgController {
    pub fn new(settings: ErgSettings) -> Self {
        Self {
            settings,
            sent_target: None,
            current: None,
            last_update_s: None,
            last_sent_s: None,
            low_cadence_since: None,
            catching_up: false,
        }
    }

    /// Whether the cadence catch-up is active.
    pub fn catching_up(&self) -> bool {
        self.catching_up
    }

    /// Advances to `now_s` with the workout target and the current cadence.
    /// Returns a target power (W) when one should be sent now.
    pub fn update(&mut self, now_s: f64, target_w: f64, cadence_rpm: Option<f64>) -> Option<f64> {
        let s = self.settings;
        // Catch-up state. Unknown cadence counts as "no information": no change.
        if let Some(rpm) = cadence_rpm {
            if rpm < s.catch_up_below_rpm {
                let since = *self.low_cadence_since.get_or_insert(now_s);
                if now_s - since >= s.catch_up_delay_s {
                    self.catching_up = true;
                }
            } else {
                self.low_cadence_since = None;
                if rpm > s.recover_above_rpm {
                    self.catching_up = false;
                }
            }
        }
        let wanted = if self.catching_up {
            target_w * s.catch_up_factor
        } else {
            target_w
        }
        .max(0.0);

        let current = match (self.current, self.last_update_s) {
            (Some(c), Some(t)) => {
                // Dropping into the catch-up is immediate; everything else ramps.
                if self.catching_up && wanted < c {
                    wanted
                } else {
                    let max_step = s.ramp_w_per_s * (now_s - t).max(0.0);
                    c + (wanted - c).clamp(-max_step, max_step)
                }
            }
            _ => wanted,
        };
        self.current = Some(current);
        self.last_update_s = Some(now_s);

        let rounded = current.round();
        let send = match (self.last_sent_s, self.sent_target) {
            (Some(t), Some(sent)) => {
                let since = now_s - t;
                let urgent = self.catching_up && rounded < sent;
                (urgent || since >= s.update_interval_s)
                    && (rounded != sent || since >= s.keepalive_s)
            }
            _ => true,
        };
        if !send {
            return None;
        }
        self.sent_target = Some(rounded);
        self.last_sent_s = Some(now_s);
        Some(rounded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim() -> SimController {
        SimController::new(SimSettings::default(), &RiderProfile::default())
    }

    #[test]
    fn rougher_surfaces_raise_the_trainer_rolling_resistance() {
        let profile = RiderProfile::default();
        let mut c = SimController::new(SimSettings::default(), &profile);
        let first = c.update_on(0.0, 0.02, 0.02, 1.0).expect("first");
        assert_eq!(first.crr, profile.crr);
        // Onto gravel: sent at the next update interval, with 2.5 × Crr.
        let dt = c.settings().update_interval_s;
        let gravel = c.update_on(dt, 0.02, 0.02, 2.5).expect("surface changed");
        assert!((gravel.crr - 2.5 * profile.crr).abs() < 1e-12);
        // Unchanged afterwards: nothing new to send.
        assert!(c.update_on(2.0 * dt, 0.02, 0.02, 2.5).is_none());
    }

    /// Another build (an aero helmet, other tyres): the new coefficients go
    /// to the trainer at the next update interval, once.
    #[test]
    fn a_new_build_sends_its_coefficients() {
        let profile = RiderProfile::default();
        let mut c = SimController::new(SimSettings::default(), &profile);
        let dt = c.settings().update_interval_s;
        c.update(0.0, 0.02, 0.02).expect("first");
        // The same profile: nothing to send.
        c.set_profile(&profile);
        assert!(c.update(dt, 0.02, 0.02).is_none());
        let aero = RiderProfile {
            cda_m2: profile.cda_m2 * 0.9,
            crr: 0.003,
            ..profile
        };
        c.set_profile(&aero);
        let t = c.update(2.0 * dt, 0.02, 0.02).expect("new coefficients");
        assert_eq!(t.crr, 0.003);
        assert!((t.cw_kg_per_m - 0.5 * AIR_DENSITY * aero.cda_m2).abs() < 1e-12);
        assert!(c.update(3.0 * dt, 0.02, 0.02).is_none());
    }

    #[test]
    fn difficulty_and_downhill_scale_the_grade() {
        let s = SimSettings::default();
        assert!((s.trainer_grade(0.08) - 0.04).abs() < 1e-12);
        // Downhill: × difficulty × downhill scale = −6 % → −1.5 %.
        assert!((s.trainer_grade(-0.06) + 0.015).abs() < 1e-12);
        // Capped at −4 % and +20 %.
        assert_eq!(s.trainer_grade(-0.5), -0.04);
        assert_eq!(s.trainer_grade(1.0), 0.20);
        let full = SimSettings {
            difficulty: 1.0,
            ..s
        };
        assert!((full.trainer_grade(0.08) - 0.08).abs() < 1e-12);
        let flat = SimSettings {
            difficulty: 0.0,
            ..s
        };
        assert_eq!(flat.trainer_grade(0.12), 0.0);
        // Out-of-range difficulty is clamped.
        let silly = SimSettings {
            difficulty: 3.0,
            ..s
        };
        assert!((silly.trainer_grade(0.05) - 0.05).abs() < 1e-12);
    }

    #[test]
    fn first_update_is_sent_with_profile_coefficients() {
        let mut c = sim();
        let t = c.update(0.0, 0.04, 0.04).unwrap();
        assert!((t.grade - 0.02).abs() < 1e-12);
        assert_eq!(t.crr, RiderProfile::default().crr);
        assert!((t.cw_kg_per_m - 0.196).abs() < 1e-3);
        assert_eq!(t.wind_speed_mps, 0.0);
    }

    #[test]
    fn gradient_changes_are_rate_limited_to_one_percent_per_second() {
        let mut c = SimController::new(
            SimSettings {
                difficulty: 1.0,
                ..SimSettings::default()
            },
            &RiderProfile::default(),
        );
        c.update(0.0, 0.0, 0.0);
        // A wall: 10 % suddenly.
        let mut sent = Vec::new();
        let mut t = 0.0;
        while t < 12.0 {
            t += 0.05;
            if let Some(target) = c.update(t, 0.10, 0.10) {
                sent.push((t, target.grade));
            }
        }
        for w in sent.windows(2) {
            let rate = (w[1].1 - w[0].1) / (w[1].0 - w[0].0);
            assert!(rate <= 0.01 + 1e-9, "rate {rate}");
        }
        assert!((sent.last().unwrap().1 - 0.10).abs() < 1e-9);
        // It took about 10 s to get there.
        let reached = sent
            .iter()
            .find(|(_, g)| (*g - 0.10).abs() < 1e-9)
            .unwrap()
            .0;
        assert!((9.5..=10.6).contains(&reached), "reached at {reached}");
    }

    #[test]
    fn updates_are_at_most_two_per_second_and_skip_unchanged_values() {
        let mut c = sim();
        let mut times = Vec::new();
        let mut t = 0.0;
        // Changing gradient: updates every 0.5 s.
        while t < 5.0 {
            if c.update(t, t * 0.01, t * 0.01).is_some() {
                times.push(t);
            }
            t += 0.02;
        }
        for w in times.windows(2) {
            assert!(w[1] - w[0] >= 0.5 - 1e-9);
        }
        assert!(times.len() >= 9);
        // Constant gradient: only the keepalive every 5 s.
        let mut c = sim();
        let sends = (0..1000)
            .filter(|i| c.update(f64::from(*i) * 0.02, 0.03, 0.03).is_some())
            .count();
        assert_eq!(sends, 4); // t = 0, 5, 10, 15 s (20 s run)
    }

    #[test]
    fn look_ahead_starts_the_change_early() {
        let mut c = SimController::new(
            SimSettings {
                difficulty: 1.0,
                ..SimSettings::default()
            },
            &RiderProfile::default(),
        );
        c.update(0.0, 0.0, 0.0);
        // Still flat here, but a 6 % climb starts just ahead.
        let t = c.update(0.5, 0.0, 0.06).unwrap();
        assert!(t.grade > 0.0);
    }

    #[test]
    fn erg_steps_ramp_and_updates_are_limited() {
        let mut e = ErgController::new(ErgSettings::default());
        assert_eq!(e.update(0.0, 150.0, Some(90.0)), Some(150.0));
        // Step to 300 W: ramps at 50 W/s, sent at most once per second.
        let mut sent = Vec::new();
        for i in 1..=60 {
            let t = f64::from(i) / 10.0;
            if let Some(w) = e.update(t, 300.0, Some(90.0)) {
                sent.push((t, w));
            }
        }
        assert_eq!(sent.first().map(|s| s.1), Some(200.0));
        assert_eq!(sent.last().map(|s| s.1), Some(300.0));
        for w in sent.windows(2) {
            assert!(w[1].0 - w[0].0 >= 1.0 - 1e-9);
            assert!(w[1].1 - w[0].1 <= 50.0 * (w[1].0 - w[0].0) + 1e-6);
        }
    }

    #[test]
    fn erg_catch_up_on_cadence_drop_and_recovery() {
        let mut e = ErgController::new(ErgSettings::default());
        e.update(0.0, 250.0, Some(90.0));
        // Cadence collapses: catch-up after 1 s, sent immediately.
        assert_eq!(e.update(0.5, 250.0, Some(40.0)), None);
        assert!(!e.catching_up());
        assert_eq!(e.update(1.6, 250.0, Some(40.0)), Some(150.0));
        assert!(e.catching_up());
        // Cadence between the thresholds: stays in catch-up.
        e.update(2.0, 250.0, Some(60.0));
        assert!(e.catching_up());
        // Recovered: ramps back up to 250 W.
        let mut last = 150.0;
        let mut t = 2.0;
        while t < 8.0 {
            t += 0.25;
            if let Some(w) = e.update(t, 250.0, Some(85.0)) {
                assert!(w >= last);
                last = w;
            }
        }
        assert!(!e.catching_up());
        assert_eq!(last, 250.0);
    }

    #[test]
    fn erg_ignores_unknown_cadence_and_negative_targets() {
        let mut e = ErgController::new(ErgSettings::default());
        assert_eq!(e.update(0.0, -50.0, None), Some(0.0));
        for i in 1..50 {
            e.update(f64::from(i) * 0.1, 200.0, None);
        }
        assert!(!e.catching_up());
    }
}
