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

use ocw_core::{AIR_DENSITY, GRAVITY, RiderProfile};

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
    /// Headwind along the rider's travel (m/s, negative = tailwind; spec
    /// §14 Phase 9, S6), for trainers that simulate wind (FTMS, Wahoo).
    pub wind_speed_mps: f64,
    pub crr: f64,
    /// Wind resistance coefficient 0.5 · ρ · CdA, in kg/m.
    pub cw_kg_per_m: f64,
    /// The gradient with the wind folded in, for trainers that cannot
    /// simulate wind (spec §14 Phase 9, Q5): [`grade`](Self::grade) plus
    /// the wind's extra drag as a gradient. The same as `grade` in calm air.
    pub wind_grade: f64,
}

/// The weather's load on the trainer in SIM mode (spec §14 Phase 9, S6,
/// Q5), while the weather affects the physics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainerWind {
    /// Headwind along the rider's travel (m/s, negative = tailwind).
    pub head_mps: f64,
    /// Air density at the rider (kg/m³).
    pub air_density: f64,
}

/// Steps the trainer's wind is sent in (m/s): FTMS carries 0.001 m/s, but
/// a gusty wind would write at every interval.
pub const WIND_STEP_MPS: f64 = 0.5;
/// Steps of the air density in the trainer's wind coefficient (kg/m³;
/// FTMS carries 0.01 kg/m).
pub const DENSITY_STEP: f64 = 0.01;

/// Time constant of the wind sent to the trainer (s). The weather's gusts
/// change the headwind every few seconds; sent as they come, each 0.5 m/s
/// step was a jump in the resistance on a road of constant gradient (the
/// user's report, 2026-11-16: "while the road was constantly at 10 %
/// inclination, the resistance varied a bit"). The trainer gets the wind
/// smoothed over this long instead: gusts are damped, a lasting change (a
/// stronger wind, a turn into it) arrives within two or three of these.
/// The physics of the ride keeps every gust.
pub const TRAINER_WIND_TAU_S: f64 = 15.0;

/// Hysteresis of the wind steps, as a share of [`WIND_STEP_MPS`]: a new
/// step is sent only once the smoothed wind is this far from the one sent,
/// so a wind near a step boundary does not flip between two steps.
pub const WIND_HYSTERESIS: f64 = 0.75;

/// The trainer's wind coefficient is sent again for a change of the air's
/// density only from this share on (2 %). Climbing, the air thins by about
/// 1 % per 100 m; every density step re-sent a Wahoo KICKR's whole SIM
/// mode (mass, Crr, cw) for a change worth well under 0.1 % of the load.
pub const CW_RESEND_SHARE: f64 = 0.02;

/// The wind's extra drag over still air on a rider at `speed` (m/s) with
/// wind coefficient `cw_kg_per_m` and total mass `mass_kg`, as a gradient
/// (rise over run): what a trainer without wind support gets on top of
/// the road's gradient (Q5). Positive in a headwind, negative in a
/// tailwind.
pub fn wind_as_grade(head_mps: f64, speed: f64, cw_kg_per_m: f64, mass_kg: f64) -> f64 {
    let v = speed.max(0.0);
    let air = v + head_mps;
    cw_kg_per_m * (air * air.abs() - v * v) / (mass_kg.max(1.0) * GRAVITY)
}

/// Turns the road gradient into rate-limited trainer updates.
#[derive(Debug, Clone)]
pub struct SimController {
    settings: SimSettings,
    crr: f64,
    cw_kg_per_m: f64,
    /// The profile's CdA and total mass (the wind coefficient at the
    /// weather's density, the wind's folded gradient).
    cda_m2: f64,
    mass_kg: f64,
    /// The wind last sent (m/s, in [`WIND_STEP_MPS`]).
    wind_sent: f64,
    /// The headwind smoothed over [`TRAINER_WIND_TAU_S`] (`None`: no
    /// weather wind).
    wind_smooth: Option<f64>,
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
            cda_m2: profile.cda_m2,
            mass_kg: profile.rider_mass_kg + profile.bike_mass_kg,
            wind_sent: 0.0,
            wind_smooth: None,
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
        self.mass_kg = profile.rider_mass_kg + profile.bike_mass_kg;
        if profile.crr != self.crr || profile.cda_m2 != self.cda_m2 {
            self.crr = profile.crr;
            // At the density in use (the standard one, or the weather's).
            self.cw_kg_per_m *= profile.cda_m2 / self.cda_m2.max(1e-9);
            self.cda_m2 = profile.cda_m2;
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
        self.update_in(now_s, grade_here, grade_ahead, rolling_factor, None, 0.0)
    }

    /// [`SimController::update_on`] in the weather (spec §14 Phase 9, S6,
    /// Q5; `None`: still air at the standard density, as before) for a
    /// rider at `speed` (m/s): the trainer gets the headwind (FTMS and
    /// Wahoo simulate it) and the wind coefficient at the air's density,
    /// both in steps ([`WIND_STEP_MPS`], [`DENSITY_STEP`]). The headwind is
    /// smoothed over [`TRAINER_WIND_TAU_S`] with a hysteresis of
    /// [`WIND_HYSTERESIS`] steps, and a density change is sent only from
    /// [`CW_RESEND_SHARE`] on, so a steady road gives a steady load. The
    /// target also carries the
    /// gradient with the wind folded in for trainers without wind
    /// ([`SimTarget::wind_grade`]). The gradient itself stays the road's.
    pub fn update_in(
        &mut self,
        now_s: f64,
        grade_here: f64,
        grade_ahead: f64,
        rolling_factor: f64,
        wind: Option<TrainerWind>,
        speed: f64,
    ) -> Option<SimTarget> {
        let density = wind.map_or(AIR_DENSITY, |w| {
            (w.air_density / DENSITY_STEP).round() * DENSITY_STEP
        });
        let cw = 0.5 * density * self.cda_m2;
        if (cw - self.cw_kg_per_m).abs() >= CW_RESEND_SHARE * self.cw_kg_per_m {
            self.cw_kg_per_m = cw;
            self.coefficients_changed = true;
        }
        self.wind_smooth = match (wind, self.wind_smooth, self.last_update_s) {
            (None, _, _) => None,
            (Some(w), Some(smooth), Some(t)) => {
                let share = 1.0 - (-(now_s - t).max(0.0) / TRAINER_WIND_TAU_S).exp();
                Some(smooth + share * (w.head_mps - smooth))
            }
            // The weather's first wind: as it is.
            (Some(w), _, _) => Some(w.head_mps),
        };
        let head = match self.wind_smooth {
            None => 0.0,
            Some(smooth) if (smooth - self.wind_sent).abs() >= WIND_HYSTERESIS * WIND_STEP_MPS => {
                (smooth / WIND_STEP_MPS).round() * WIND_STEP_MPS
            }
            Some(_) => self.wind_sent,
        };
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

        let wind_changed = head != self.wind_sent;
        let send = match self.last_sent {
            None => true,
            Some((t, sent, factor)) => {
                let since = now_s - t;
                since >= self.settings.update_interval_s
                    && ((current - sent).abs() >= self.settings.min_change
                        || (rolling_factor - factor).abs() > 1e-9
                        || self.coefficients_changed
                        || wind_changed
                        || since >= self.settings.keepalive_s)
            }
        };
        if !send {
            return None;
        }
        self.last_sent = Some((now_s, current, rolling_factor));
        self.coefficients_changed = false;
        self.wind_sent = head;
        let wind_grade = if head == 0.0 {
            current
        } else {
            let fold = wind_as_grade(head, speed, self.cw_kg_per_m, self.mass_kg);
            (current + fold).clamp(self.settings.downhill_limit, self.settings.uphill_limit)
        };
        Some(SimTarget {
            grade: current,
            wind_speed_mps: head,
            crr: self.crr * rolling_factor,
            cw_kg_per_m: self.cw_kg_per_m,
            wind_grade,
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

    /// Spec §14 Phase 9, S6 / Q5: the trainer gets the headwind and the
    /// air's wind coefficient; the folded gradient for trainers without
    /// wind; the road's gradient unchanged.
    #[test]
    fn the_wind_goes_to_the_trainer() {
        let profile = RiderProfile::default();
        let mut c = SimController::new(SimSettings::default(), &profile);
        let dt = c.settings().update_interval_s;
        let head = TrainerWind {
            head_mps: 5.2,
            air_density: 1.1,
        };
        let t = c.update_in(0.0, 0.04, 0.04, 1.0, Some(head), 10.0).unwrap();
        assert!((t.grade - 0.02).abs() < 1e-12, "the road's gradient stays");
        assert_eq!(t.wind_speed_mps, 5.0, "in steps of 0.5 m/s");
        assert!((t.cw_kg_per_m - 0.5 * 1.1 * profile.cda_m2).abs() < 1e-12);
        let mass = profile.rider_mass_kg + profile.bike_mass_kg;
        let fold = wind_as_grade(5.0, 10.0, t.cw_kg_per_m, mass);
        assert!((t.wind_grade - (0.02 + fold)).abs() < 1e-12);
        // A 5 m/s headwind at 36 km/h is worth about 2 %.
        assert!((0.015..0.03).contains(&fold), "{fold}");
        // The same wind: nothing new to send.
        assert!(c.update_in(dt, 0.04, 0.04, 1.0, Some(head), 10.0).is_none());
        // Every interval for `seconds` from `from_s` in `wind`: the targets
        // sent (the keepalive re-sends one every 5 s).
        let ride = |c: &mut SimController, from_s: f64, seconds: f64, wind: TrainerWind| {
            let mut sent = Vec::new();
            let mut t = from_s;
            while t < from_s + seconds {
                t += dt;
                sent.extend(c.update_in(t, 0.04, 0.04, 1.0, Some(wind), 10.0));
            }
            sent
        };
        let winds = |sent: &[SimTarget]| sent.iter().map(|t| t.wind_speed_mps).collect::<Vec<_>>();
        // A gust of two seconds, then the wind as before: the trainer keeps
        // its wind (the user's report, 2026-11-16: the resistance varied on
        // a steady 10 % road).
        let gust = TrainerWind {
            head_mps: 6.0,
            ..head
        };
        let sent = [ride(&mut c, dt, 2.0, gust), ride(&mut c, 2.5, 30.0, head)].concat();
        assert!(!sent.is_empty(), "the keepalive");
        assert!(winds(&sent).iter().all(|w| *w == 5.0), "{:?}", winds(&sent));
        // The same wind lasting: it arrives within half a minute.
        let t = *ride(&mut c, 32.5, 30.0, gust)
            .last()
            .expect("the stronger wind");
        assert_eq!(t.wind_speed_mps, 6.0);
        // A tailwind (the road turned): negative wind, a lower folded
        // gradient, once the smoothed wind has turned (a turn from +6 to
        // −4 m/s settles on its step after about 66 s).
        let tail = TrainerWind {
            head_mps: -4.0,
            ..head
        };
        let t = *ride(&mut c, 62.5, 90.0, tail).last().expect("the tailwind");
        assert_eq!(t.wind_speed_mps, -4.0);
        assert!(t.wind_grade < t.grade);
        // The folded gradient stays within the trainer's limits.
        let gale = TrainerWind {
            head_mps: -30.0,
            ..head
        };
        let t = *ride(&mut c, 152.5, 90.0, gale).last().expect("the gale");
        assert_eq!(t.wind_grade, SimSettings::default().downhill_limit);
        // The weather off again: still air at the standard density, at once.
        let t = c.update_in(243.0, 0.04, 0.04, 1.0, None, 10.0).unwrap();
        assert_eq!(t.wind_speed_mps, 0.0);
        assert_eq!(t.wind_grade, t.grade);
        assert!((t.cw_kg_per_m - 0.5 * AIR_DENSITY * profile.cda_m2).abs() < 1e-12);
        assert!(
            c.update_in(243.0 + dt, 0.04, 0.04, 1.0, None, 10.0)
                .is_none()
        );
        // Calm air folds nothing.
        assert_eq!(wind_as_grade(0.0, 10.0, 0.2, 80.0), 0.0);
    }

    /// Climbing, the air thins (about 1 % per 100 m): a small density
    /// change does not re-send the coefficients (on a Wahoo KICKR the
    /// whole SIM mode), a change of [`CW_RESEND_SHARE`] does.
    #[test]
    fn a_small_density_change_is_not_sent() {
        let profile = RiderProfile::default();
        let mut c = SimController::new(SimSettings::default(), &profile);
        let still = |air_density| {
            Some(TrainerWind {
                head_mps: 0.0,
                air_density,
            })
        };
        let first = c.update_in(0.0, 0.1, 0.1, 1.0, still(1.10), 3.5).unwrap();
        // 1 % thinner (about 100 m higher): only the keepalive, unchanged.
        let mut t = 0.0;
        while t < 4.0 {
            t += 0.5;
            assert!(c.update_in(t, 0.1, 0.1, 1.0, still(1.09), 3.5).is_none());
        }
        let keepalive = c.update_in(5.0, 0.1, 0.1, 1.0, still(1.09), 3.5).unwrap();
        assert_eq!(keepalive.cw_kg_per_m, first.cw_kg_per_m);
        // 3 % thinner: the new coefficient at the next interval.
        let t = c.update_in(5.5, 0.1, 0.1, 1.0, still(1.067), 3.5).unwrap();
        assert!((t.cw_kg_per_m - 0.5 * 1.07 * profile.cda_m2).abs() < 1e-12);
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
