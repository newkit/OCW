//! The session's weather (spec §14 Phase 9, 9A / 9E): owned by the game
//! server like the world clock (§6.1), on **session time** (the server's
//! ticks), sent in `Welcome` and the snapshots.
//!
//! - A weather is either *fixed* (set by the host: at once or as a smooth
//!   transition over a chosen time) or *simulated*: a seeded stochastic
//!   model passes fronts and showers through (a Markov chain over the
//!   presets with varied intensities, durations and transitions), so a
//!   long ride is not static. Real weather (S7) will feed the same state.
//! - The ground's memory ([`Ground`]: road wetness, puddles, snow cover) is
//!   integrated every tick.
//! - Lightning strikes are drawn once per second of session time at the
//!   weather's rate ([`crate::lightning::slot_strike`]) and kept a few
//!   seconds, so every snapshot in that time carries them.

use crate::ground::Ground;
use crate::lightning::{self, Strike, unit};
use crate::presets::{nearest, preset};
use crate::state::{Precip, WeatherState, smooth};

/// How the weather is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Set by the host (or the default): stays until changed.
    Fixed,
    /// The simulation evolves it (seeded).
    Simulated,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Simulated => "simulated",
        }
    }
}

/// Strikes are kept this long after their flash (s): every snapshot in
/// that time carries them (unreliable datagrams may be lost).
pub const STRIKE_KEEP_S: f64 = 3.0;
/// A tick longer than this (a suspended machine) is cut (s).
const MAX_STEP_S: f64 = 10.0;

/// See the module docs.
#[derive(Debug, Clone)]
pub struct SessionWeather {
    from: WeatherState,
    to: WeatherState,
    start_t: f64,
    duration_s: f64,
    mode: Mode,
    sim_seed: u64,
    sim_step: u64,
    /// The simulation's base temperature (the weather when it started).
    sim_base_c: f32,
    next_change_t: f64,
    ground: Ground,
    last_t: Option<f64>,
    seed: u64,
    revision: u32,
    immediate: bool,
    strikes: Vec<Strike>,
    next_id: u32,
    next_slot: Option<i64>,
}

impl SessionWeather {
    /// A clear, fixed weather; `seed` drives the lightning, the gusts and
    /// (unless one is given) the simulation.
    pub fn new(seed: u64) -> Self {
        Self::with(WeatherState::default(), seed)
    }

    /// Starts with `w` (fixed), the ground settled for it.
    pub fn with(w: WeatherState, seed: u64) -> Self {
        let w = w.clamped();
        Self {
            from: w,
            to: w,
            start_t: 0.0,
            duration_s: 0.0,
            mode: Mode::Fixed,
            sim_seed: seed,
            sim_step: 0,
            sim_base_c: w.temperature_c,
            next_change_t: 0.0,
            ground: Ground::settled(&w),
            last_t: None,
            seed,
            revision: 0,
            immediate: true,
            strikes: Vec::new(),
            next_id: 1,
            next_slot: None,
        }
    }

    /// The weather at session time `t` (during a transition: eased between
    /// the old and the new state).
    pub fn now(&self, t: f64) -> WeatherState {
        self.from.lerp(&self.to, self.progress(t))
    }

    fn progress(&self, t: f64) -> f32 {
        if self.duration_s <= 0.0 {
            return 1.0;
        }
        smooth(0.0, 1.0, ((t - self.start_t) / self.duration_s) as f32)
    }

    /// Where a transition goes (the current state when there is none).
    pub fn target(&self) -> &WeatherState {
        &self.to
    }

    /// Seconds until the transition reaches its target (0: reached).
    pub fn transition_left_s(&self, t: f64) -> f64 {
        (self.start_t + self.duration_s - t).max(0.0)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn ground(&self) -> Ground {
        self.ground
    }

    /// The simulation's seed (spec §14 Phase 9, S5: `weather.status`).
    pub fn sim_seed(&self) -> u64 {
        self.sim_seed
    }

    /// The seed of the gusts and the lightning.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Counts the changes asked for (clients snap to a change made at
    /// once, [`SessionWeather::immediate`]).
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// Whether the last change was made at once (no transition).
    pub fn immediate(&self) -> bool {
        self.immediate
    }

    /// The strikes of the last [`STRIKE_KEEP_S`] seconds (and any whose
    /// flash is still to come within the current second).
    pub fn strikes(&self) -> &[Strike] {
        &self.strikes
    }

    /// Sets the weather to `target` (fixed from now on), at once
    /// (`transition_s` ≤ 0) or eased over `transition_s` seconds from the
    /// current state. `settle_ground`: a weather set at once also gets its
    /// ground ([`Ground::settled`]: a rain preset has wet roads at once).
    pub fn set(&mut self, t: f64, target: WeatherState, transition_s: f64, settle_ground: bool) {
        self.change(t, target.clamped(), transition_s);
        self.mode = Mode::Fixed;
        if self.immediate && settle_ground {
            self.ground = Ground::settled(&self.to);
        }
    }

    fn change(&mut self, t: f64, target: WeatherState, transition_s: f64) {
        self.from = self.now(t);
        self.to = target;
        self.start_t = t;
        self.duration_s = transition_s.max(0.0);
        self.immediate = self.duration_s == 0.0;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Overrides the ground (the console's `weather wet=…`, tests).
    pub fn set_ground(&mut self, ground: Ground) {
        self.ground = ground;
        self.immediate = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// The simulation evolves the weather from now on, seeded by `seed`.
    pub fn simulate(&mut self, t: f64, seed: u64) {
        self.mode = Mode::Simulated;
        self.sim_seed = seed;
        self.sim_step = 0;
        self.sim_base_c = self.now(t).temperature_c;
        self.next_change_t = t;
        self.revision = self.revision.wrapping_add(1);
    }

    /// A strike now at `distance_m` and `bearing_deg` (the console's test
    /// strike; real ones come from the rate).
    pub fn strike(&mut self, t: f64, distance_m: f32, bearing_deg: f32) -> Strike {
        let s = Strike {
            id: self.next_id,
            at_s: t,
            distance_m: distance_m.clamp(lightning::MIN_DISTANCE_M * 0.4, lightning::AUDIBLE_M),
            bearing_deg: bearing_deg.rem_euclid(360.0),
            intensity: 1.0,
            seed: (unit(self.seed, u64::from(self.next_id), 99) * f64::from(u32::MAX)) as u32,
        };
        self.next_id += 1;
        self.strikes.push(s);
        s
    }

    /// Advances to session time `t`: the ground, the lightning, the
    /// simulation.
    pub fn tick(&mut self, t: f64) {
        let dt = self
            .last_t
            .map_or(0.0, |last| (t - last).clamp(0.0, MAX_STEP_S));
        self.last_t = Some(t);
        let now = self.now(t);
        self.ground.step(&now, dt as f32);
        // One draw per whole second of session time.
        let slot = t.floor() as i64;
        let first = self.next_slot.unwrap_or(slot);
        for s in first..=slot {
            let rate = self.now(s as f64).lightning_per_min;
            if let Some((frac, d, bearing, intensity, seed)) =
                lightning::slot_strike(self.seed, s, rate)
            {
                self.strikes.push(Strike {
                    id: self.next_id,
                    at_s: s as f64 + frac,
                    distance_m: d,
                    bearing_deg: bearing,
                    intensity,
                    seed,
                });
                self.next_id += 1;
            }
        }
        self.next_slot = Some(slot + 1);
        self.strikes.retain(|s| s.at_s >= t - STRIKE_KEEP_S);
        if self.mode == Mode::Simulated && t >= self.next_change_t {
            self.simulation_step(t);
        }
    }

    /// The simulation's next weather: a step of the chain from the preset
    /// nearest to the current target, varied, for 8–25 minutes, reached
    /// over 2–6 minutes.
    fn simulation_step(&mut self, t: f64) {
        let k = self.sim_step;
        self.sim_step += 1;
        let sim_seed = self.sim_seed;
        let r = |n: u64| unit(sim_seed, k, n) as f32;
        let from = nearest(&self.to);
        let next = pick(next_of(from), r(1));
        let mut w = preset(next).unwrap_or_default();
        // Around the session's temperature, not the presets' own.
        let shift = self.sim_base_c - preset("clear").unwrap_or_default().temperature_c;
        w.temperature_c += shift + (r(2) - 0.5) * 4.0;
        if w.precipitating() && matches!(w.precip, Precip::Rain | Precip::Drizzle) {
            if w.temperature_c < 1.5 {
                w.precip = Precip::Snow;
                w.precip_mm_h = (w.precip_mm_h * 0.4).max(0.5);
                w.lightning_per_min = 0.0;
            } else if w.temperature_c < 3.0 {
                w.precip = Precip::Sleet;
            }
        }
        w.cloud_cover = (w.cloud_cover + (r(3) - 0.5) * 0.2).clamp(0.0, 1.0);
        w.wind_mps *= 0.7 + 0.6 * r(4);
        w.gust_mps = (w.gust_mps * (0.7 + 0.6 * r(4))).max(w.wind_mps);
        w.wind_from_deg = (w.wind_from_deg + (r(5) - 0.5) * 60.0).rem_euclid(360.0);
        w.precip_mm_h *= 0.6 + 0.9 * r(6);
        let transition = 120.0 + 240.0 * f64::from(r(7));
        self.change(t, w.clamped(), transition);
        self.next_change_t = t + 480.0 + 1020.0 * f64::from(r(8));
    }

    /// "rain 4.0 mm/h, … (fixed)", "… → thunderstorm in 40 s (simulated)".
    pub fn describe(&self, t: f64) -> String {
        let mut s = self.now(t).summary();
        let left = self.transition_left_s(t);
        if left > 0.0 {
            s += &format!(" -> {} in {left:.0} s", self.to.condition().label());
        }
        s + &format!(" ({})", self.mode.name())
    }
}

/// The chain: what may follow a preset, with weights.
fn next_of(p: &str) -> &'static [(&'static str, f32)] {
    match p {
        "clear" => &[
            ("fair", 0.5),
            ("cloudy", 0.2),
            ("windy", 0.1),
            ("mist", 0.1),
            ("clear", 0.1),
        ],
        "fair" => &[
            ("clear", 0.25),
            ("cloudy", 0.45),
            ("windy", 0.1),
            ("fair", 0.2),
        ],
        "cloudy" => &[
            ("fair", 0.25),
            ("overcast", 0.3),
            ("light-rain", 0.25),
            ("thunderstorm", 0.08),
            ("cloudy", 0.12),
        ],
        "overcast" => &[
            ("cloudy", 0.25),
            ("drizzle", 0.2),
            ("light-rain", 0.25),
            ("rain", 0.2),
            ("overcast", 0.1),
        ],
        "mist" => &[
            ("fair", 0.4),
            ("cloudy", 0.3),
            ("fog", 0.1),
            ("overcast", 0.2),
        ],
        "fog" => &[("mist", 0.6), ("overcast", 0.4)],
        "drizzle" => &[("overcast", 0.4), ("light-rain", 0.3), ("mist", 0.3)],
        "light-rain" => &[
            ("rain", 0.3),
            ("overcast", 0.35),
            ("cloudy", 0.25),
            ("drizzle", 0.1),
        ],
        "rain" => &[
            ("heavy-rain", 0.2),
            ("light-rain", 0.35),
            ("overcast", 0.3),
            ("thunderstorm", 0.1),
            ("storm", 0.05),
        ],
        "heavy-rain" => &[("rain", 0.5), ("thunderstorm", 0.2), ("overcast", 0.3)],
        "thunderstorm" => &[
            ("heavy-rain", 0.3),
            ("rain", 0.3),
            ("cloudy", 0.3),
            ("hail", 0.1),
        ],
        "hail" => &[("thunderstorm", 0.5), ("rain", 0.5)],
        "storm" => &[("rain", 0.5), ("windy", 0.5)],
        "windy" => &[
            ("fair", 0.4),
            ("cloudy", 0.3),
            ("storm", 0.1),
            ("windy", 0.2),
        ],
        // Snow and sleet: back through the grey.
        _ => &[("overcast", 0.5), ("cloudy", 0.3), ("light-rain", 0.2)],
    }
}

fn pick(options: &'static [(&'static str, f32)], u: f32) -> &'static str {
    let total: f32 = options.iter().map(|o| o.1).sum();
    let mut x = u * total;
    for (name, w) in options {
        if x < *w {
            return name;
        }
        x -= w;
    }
    options.last().map_or("clear", |o| o.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(w: &mut SessionWeather, from: f64, to: f64) {
        let mut t = from;
        while t <= to {
            w.tick(t);
            t += 0.02;
        }
    }

    #[test]
    fn a_transition_eases_from_the_current_state_to_the_target() {
        let mut w = SessionWeather::new(1);
        let clear = preset("clear").unwrap();
        assert_eq!(w.now(0.0), clear);
        let storm = preset("thunderstorm").unwrap();
        w.set(10.0, storm, 60.0, true);
        assert!(!w.immediate());
        assert_eq!(w.now(10.0), clear, "starts where it was");
        assert_eq!(w.now(70.0), storm);
        assert_eq!(w.transition_left_s(40.0), 30.0);
        let mid = w.now(40.0);
        assert!(mid.cloud_cover > clear.cloud_cover && mid.cloud_cover < storm.cloud_cover);
        // Eased: slow at the ends.
        let early = w.now(13.0).cloud_cover - clear.cloud_cover;
        let middle = w.now(43.0).cloud_cover - w.now(40.0).cloud_cover;
        assert!(early < middle);
        // A new change half way starts from where it is.
        w.set(40.0, clear, 0.0, true);
        assert_eq!(w.now(40.0), clear);
        assert!(w.immediate());
        assert!(w.describe(40.0).ends_with("(fixed)"));
    }

    #[test]
    fn the_ground_follows_the_weather_and_presets_set_at_once_settle_it() {
        let mut w = SessionWeather::new(1);
        w.set(0.0, preset("heavy-rain").unwrap(), 0.0, true);
        assert!(w.ground().puddles() > 0.9, "settled");
        // A field change at once keeps the ground.
        let g = w.ground();
        let warmer = WeatherState {
            temperature_c: 20.0,
            ..*w.target()
        };
        w.set(0.0, warmer, 0.0, false);
        assert_eq!(w.ground(), g);
        // A transition to fair: the road dries over time, ticked.
        w.set(0.0, preset("fair").unwrap(), 30.0, true);
        run(&mut w, 0.0, 600.0);
        assert!(w.ground().puddle_mm < g.puddle_mm);
        w.set_ground(Ground::with_factors(0.0, 0.0, 1.0));
        assert_eq!(w.ground().snow_cover(), 1.0);
    }

    /// Lightning: at the rate, deterministic for the seed, kept a few
    /// seconds; none in clear weather.
    #[test]
    fn lightning_strikes_at_the_rate_and_is_kept_briefly() {
        let mut calm = SessionWeather::new(5);
        run(&mut calm, 0.0, 120.0);
        assert!(calm.strikes().is_empty());
        let storm = WeatherState {
            lightning_per_min: 30.0,
            ..preset("thunderstorm").unwrap()
        };
        let mut a = SessionWeather::with(storm, 5);
        let mut b = SessionWeather::with(storm, 5);
        let mut seen = std::collections::BTreeMap::new();
        let mut t = 0.0;
        while t < 600.0 {
            a.tick(t);
            b.tick(t);
            assert_eq!(a.strikes(), b.strikes(), "the same seed, the same strikes");
            for s in a.strikes() {
                assert!(s.at_s >= t - STRIKE_KEEP_S && s.at_s < t + 1.0);
                seen.insert(s.id, *s);
            }
            t += 0.02;
        }
        // 600 s at 30 / min: 1 − e^−0.5 ≈ 39 % of the seconds.
        let n = seen.len() as f64;
        assert!((n / 600.0 - 0.393).abs() < 0.07, "{n}");
        // A test strike.
        let s = a.strike(600.0, 1200.0, 45.0);
        assert!(a.strikes().contains(&s));
        assert!((s.thunder_delay_s() - 3.5).abs() < 0.01);
    }

    /// The simulation evolves the weather plausibly and repeatably.
    #[test]
    fn the_simulation_passes_weathers_through_repeatably() {
        let run_sim = |seed: u64| {
            let mut w = SessionWeather::new(3);
            w.simulate(0.0, seed);
            let mut seen = Vec::new();
            let mut t = 0.0;
            while t < 6.0 * 3600.0 {
                w.tick(t);
                let c = w.now(t).condition();
                if seen.last() != Some(&c) {
                    seen.push(c);
                }
                t += 5.0;
            }
            (seen, w.now(t))
        };
        let (a, end) = run_sim(11);
        let (b, _) = run_sim(11);
        assert_eq!(a, b, "seeded");
        assert!(a.len() >= 6, "six hours bring changes: {a:?}");
        assert_ne!(run_sim(12).0, a);
        assert_eq!(end, end.clamped());
        // Cold sessions get snow instead of rain.
        let mut cold = SessionWeather::with(
            WeatherState {
                temperature_c: -6.0,
                ..preset("overcast").unwrap()
            },
            1,
        );
        cold.simulate(0.0, 4);
        let mut t = 0.0;
        while t < 24.0 * 3600.0 {
            cold.tick(t);
            let w = cold.now(t);
            assert!(
                !(w.precipitating() && w.precip == Precip::Rain),
                "rain at {}",
                w.temperature_c
            );
            t += 30.0;
        }
        assert_eq!(cold.mode(), Mode::Simulated);
        let mut w = cold.clone();
        w.set(t, preset("clear").unwrap(), 0.0, true);
        assert_eq!(w.mode(), Mode::Fixed, "a set stops the simulation");
    }

    #[test]
    fn revisions_count_the_changes() {
        let mut w = SessionWeather::new(1);
        let r = w.revision();
        w.set(0.0, preset("rain").unwrap(), 0.0, true);
        w.set_ground(Ground::default());
        w.simulate(0.0, 2);
        assert_eq!(w.revision(), r + 3);
        assert_eq!(pick(&[("a", 1.0), ("b", 1.0)], 0.75), "b");
        assert_eq!(pick(&[("a", 1.0), ("b", 1.0)], 0.25), "a");
    }
}
