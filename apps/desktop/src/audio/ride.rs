//! The ride sounds (spec §3.3): wind rising with speed, the tyres on the
//! road surface, the freehub ticking while coasting and the drivetrain
//! whirring while pedalling. All procedurally synthesised here (our own
//! work, no samples).
//!
//! [`ride_params`] maps what the rider does (speed, cadence, power,
//! surface) onto sound parameters, [`RideParams::approach`] ramps between
//! them – both pure and unit-tested; [`RideSynth`] turns the parameters
//! into samples.

use ocw_world::Surface;

use super::dsp::{Burst, Noise, Pink, Svf, sine};

/// Wheel circumference of a 700×28c tyre in metres.
pub const WHEEL_CIRCUMFERENCE_M: f64 = 2.1;
/// Engagement points (POE) of the rear hub's ratchet: ticks per wheel
/// revolution while the wheel overruns the cassette. 24 is a typical road
/// freehub (common hubs have 18–54); at 30 km/h that is about 95 ticks/s.
pub const FREEWHEEL_POINTS: f64 = 24.0;
/// Chainring teeth (the chain's tooth rate is cadence × this).
pub const CHAINRING_TEETH: f64 = 50.0;
/// The top gear (50 × 11): the fastest the legs can drive the wheel per
/// crank revolution.
pub const TOP_GEAR_RATIO: f64 = CHAINRING_TEETH / 11.0;
/// Below this cadence the cranks do not drive the wheel (rpm).
pub const PEDALLING_RPM: f64 = 15.0;
/// Above this power (W) the rider drives the wheel, whatever the cadence
/// reads (no sensor, a dropout, a trainer that reports power only).
pub const PEDALLING_W: f64 = 10.0;
/// The speed (m/s, 54 km/h) the wind and tyre levels are scaled to.
const V_REF: f64 = 15.0;
/// Gains below this are silence (the synthesis skips the voice).
pub const SILENT: f32 = 1e-4;
/// Time constant of the parameter smoothing (no zipper noise).
pub const PARAM_TAU_S: f32 = 0.06;
/// The freehub fades in over about a quarter of a second when pedalling
/// stops (no burst of ticks) …
pub const FREEWHEEL_IN_TAU_S: f32 = 0.08;
/// … and is gone within a few tens of milliseconds when pedalling starts
/// again (the pawls stop ticking the moment the hub engages).
pub const FREEWHEEL_OUT_TAU_S: f32 = 0.008;

/// What the local rider does now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RideInput {
    pub speed_mps: f64,
    /// From the trainer or a cadence sensor, if one sends it.
    pub cadence_rpm: Option<f64>,
    pub power_w: f64,
    pub surface: Surface,
}

/// The cadence the drivetrain sound uses: the measured one, else (no
/// sensor, or it reads 0 while the rider puts out power) an estimate from
/// the power.
pub fn effective_cadence(input: &RideInput) -> f64 {
    match input.cadence_rpm {
        Some(c) if c >= PEDALLING_RPM => c,
        _ if input.power_w > PEDALLING_W => (75.0 + input.power_w / 20.0).clamp(75.0, 100.0),
        Some(c) => c.max(0.0),
        None => 0.0,
    }
}

/// Whether the rider's legs drive the rear wheel: then the freehub is
/// engaged and silent. Otherwise the wheel overruns the cassette and the
/// pawls tick. Power drives the wheel; without power, turning cranks drive
/// it only if the top gear at that cadence reaches the speed (spinning the
/// legs at 60 km/h downhill still lets the freehub tick).
pub fn drives_wheel(input: &RideInput) -> bool {
    if input.power_w > PEDALLING_W {
        return true;
    }
    match input.cadence_rpm {
        Some(c) if c >= PEDALLING_RPM => {
            let top_speed = c / 60.0 * TOP_GEAR_RATIO * WHEEL_CIRCUMFERENCE_M;
            top_speed + 0.5 >= input.speed_mps
        }
        _ => false,
    }
}

/// Parameters of the ride sounds: a gain (linear, before the effects bus)
/// and a frequency or rate for each voice. All zero = silence.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RideParams {
    /// Wind and air noise.
    pub wind_gain: f32,
    pub wind_cutoff_hz: f32,
    /// Rolling noise of the tyres (all surfaces, louder off asphalt).
    pub roll_gain: f32,
    pub roll_cutoff_hz: f32,
    /// Stones and grit (gravel, a little on grass): bursts per second.
    pub grit_rate_hz: f32,
    pub grit_gain: f32,
    /// Grass brushing the tyres.
    pub swish_gain: f32,
    /// Freehub pawls: ticks per second while the wheel overruns.
    pub freewheel_rate_hz: f32,
    /// Exactly 0 whenever the rider drives the wheel.
    pub freewheel_gain: f32,
    /// Chain over the chainring: teeth per second while pedalling (the
    /// pitch of the drivetrain's whir and hum).
    pub chain_rate_hz: f32,
    pub chain_gain: f32,
    /// Pedal strokes per second (two per crank revolution).
    pub stroke_hz: f32,
}

/// The ride sounds for `input`.
pub fn ride_params(input: &RideInput) -> RideParams {
    let v = input.speed_mps.max(0.0);
    let rel = v / V_REF;
    let cadence = effective_cadence(input);
    let mut p = RideParams {
        wind_cutoff_hz: (250.0 + 120.0 * v).min(4000.0) as f32,
        roll_cutoff_hz: (150.0 + 30.0 * v).min(1500.0) as f32,
        ..RideParams::default()
    };
    if v > 0.5 {
        // Air noise grows faster than the speed (turbulence at the ears).
        p.wind_gain = (0.45 * rel.powf(1.6)).min(0.9) as f32;
        let (roll, grit_per_m, grit, swish) = match input.surface {
            Surface::Asphalt => (0.10, 0.0, 0.0, 0.0),
            Surface::Gravel => (0.16, 6.0, 0.30, 0.0),
            Surface::Grass => (0.07, 1.0, 0.12, 0.22),
        };
        let level = rel.min(1.5);
        p.roll_gain = (roll * level.sqrt()) as f32;
        p.grit_rate_hz = (grit_per_m * v) as f32;
        p.grit_gain = (grit * level.sqrt()) as f32;
        p.swish_gain = (swish * level.min(1.0)) as f32;
    }
    if cadence >= PEDALLING_RPM {
        p.chain_rate_hz = (cadence / 60.0 * CHAINRING_TEETH) as f32;
        p.stroke_hz = (2.0 * cadence / 60.0) as f32;
        // A little louder under load.
        p.chain_gain = (0.05 + 0.07 * (input.power_w / 400.0).clamp(0.0, 1.0)) as f32;
    }
    if !drives_wheel(input) && v > 0.3 {
        let rate = v / WHEEL_CIRCUMFERENCE_M * FREEWHEEL_POINTS;
        p.freewheel_rate_hz = rate as f32;
        // Dense ticks add up: keep the buzz at high speed about as loud as
        // the ticking at walking pace.
        p.freewheel_gain = (0.32 / (1.0 + rate / 60.0).sqrt()) as f32;
    }
    p
}

/// The fraction a one-pole ramp with time constant `tau_s` moves in `dt_s`.
fn ramp(dt_s: f32, tau_s: f32) -> f32 {
    1.0 - (-dt_s.max(0.0) / tau_s).exp()
}

impl RideParams {
    /// Moves every parameter towards `t` for `dt_s` seconds: smooth
    /// changes (no zipper noise), the freehub fading in slowly when
    /// pedalling stops and out fast when it starts. A gain heading for 0
    /// reaches exactly 0.
    pub fn approach(&mut self, t: &RideParams, dt_s: f32) {
        let a = ramp(dt_s, PARAM_TAU_S);
        let f = |x: &mut f32, y: f32, a: f32| {
            *x += (y - *x) * a;
            if y == 0.0 && x.abs() < SILENT {
                *x = 0.0;
            }
        };
        f(&mut self.wind_gain, t.wind_gain, a);
        f(&mut self.wind_cutoff_hz, t.wind_cutoff_hz, a);
        f(&mut self.roll_gain, t.roll_gain, a);
        f(&mut self.roll_cutoff_hz, t.roll_cutoff_hz, a);
        f(&mut self.grit_rate_hz, t.grit_rate_hz, a);
        f(&mut self.grit_gain, t.grit_gain, a);
        f(&mut self.swish_gain, t.swish_gain, a);
        f(&mut self.chain_gain, t.chain_gain, a);
        let fw = if t.freewheel_gain > self.freewheel_gain {
            ramp(dt_s, FREEWHEEL_IN_TAU_S)
        } else {
            ramp(dt_s, FREEWHEEL_OUT_TAU_S)
        };
        f(&mut self.freewheel_gain, t.freewheel_gain, fw);
        // Rates jump (a rate gliding from 0 would tick slowly first);
        // only their gains fade.
        if t.freewheel_rate_hz > 0.0 {
            self.freewheel_rate_hz = t.freewheel_rate_hz;
        }
        if t.chain_rate_hz > 0.0 {
            self.chain_rate_hz = t.chain_rate_hz;
            self.stroke_hz = t.stroke_hz;
        }
    }
}

/// Synthesises the ride sounds sample by sample.
#[derive(Debug, Clone)]
pub struct RideSynth {
    sr: f32,
    pink: Pink,
    noise: Noise,
    wind_lp: Svf,
    wind_bp: Svf,
    /// Slow random gusts (0–1) and their target.
    gust: f32,
    gust_target: f32,
    roll_lp: Svf,
    grit: Burst,
    grit_bp: Svf,
    swish_hp: Svf,
    freewheel_phase: f32,
    /// The next tick's spacing jitter (0.9–1.1).
    freewheel_jitter: f32,
    click: Burst,
    click_bp: Svf,
    click_ring: f32,
    chain_phase: f32,
    stroke_phase: f32,
    chain_bp: Svf,
    chain_lp: Svf,
}

impl RideSynth {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        Self {
            sr,
            pink: Pink::new(0x5EED_0001),
            noise: Noise::new(0x5EED_0002),
            wind_lp: Svf::new(800.0, 0.6, sr),
            wind_bp: Svf::new(600.0, 1.2, sr),
            gust: 0.5,
            gust_target: 0.5,
            roll_lp: Svf::new(300.0, 0.7, sr),
            grit: Burst::default(),
            grit_bp: Svf::new(2600.0, 1.4, sr),
            swish_hp: Svf::new(2800.0, 0.6, sr),
            freewheel_phase: 0.0,
            freewheel_jitter: 1.0,
            click: Burst::default(),
            click_bp: Svf::new(4200.0, 4.0, sr),
            click_ring: 0.0,
            chain_phase: 0.0,
            stroke_phase: 0.0,
            chain_bp: Svf::new(700.0, 0.7, sr),
            chain_lp: Svf::new(1800.0, 0.6, sr),
        }
    }

    /// Follows the parameters' frequencies (call every few samples).
    pub fn set_filters(&mut self, p: &RideParams) {
        let sr = self.sr;
        let gust = 0.85 + 0.3 * self.gust;
        self.wind_lp.set(p.wind_cutoff_hz * gust, 0.6, sr);
        self.wind_bp.set(p.wind_cutoff_hz * 0.7 * gust, 1.6, sr);
        self.roll_lp.set(p.roll_cutoff_hz, 0.8, sr);
        // The whir rises a little with the cadence.
        self.chain_bp
            .set((350.0 + 5.0 * p.chain_rate_hz).min(1200.0), 0.7, sr);
        // Gusts: a slow random walk, a new target now and then.
        if self.noise.unit() < 0.01 {
            self.gust_target = self.noise.unit();
        }
        self.gust += (self.gust_target - self.gust) * 0.004;
    }

    /// The next sample (mono, roughly within ±1).
    pub fn sample(&mut self, p: &RideParams) -> f32 {
        let sr = self.sr;
        let mut out = 0.0;
        // Wind: pink noise, low-passed with a resonant band for the
        // "whoosh", louder in gusts.
        if p.wind_gain > 1e-4 {
            let n = self.pink.sample();
            let lp = self.wind_lp.process(n).low;
            let bp = self.wind_bp.process(n).band;
            out += p.wind_gain * (0.75 + 0.5 * self.gust) * (lp * 1.4 + bp * 0.8);
        }
        // Tyres: low rumble of the rolling, grit bursts, grass swish.
        if p.roll_gain > 1e-4 {
            let n = self.noise.white();
            out += p.roll_gain * self.roll_lp.process(n).low * 2.0;
        }
        if p.grit_gain > 1e-4 && p.grit_rate_hz > 0.0 {
            if self.noise.unit() < p.grit_rate_hz / sr {
                let size = self.noise.unit();
                self.grit.trigger(size * size, 0.0008 + 0.002 * size, sr);
            }
            let env = self.grit.step();
            if env > 0.0 {
                let n = self.noise.white();
                out += p.grit_gain * env * self.grit_bp.process(n).band * 3.0;
            }
        }
        if p.swish_gain > 1e-4 {
            let n = self.noise.white();
            out += p.swish_gain * (0.6 + 0.4 * self.gust) * self.swish_hp.process(n).high * 0.5;
        }
        // Freehub: one short metallic click per pawl engagement, only while
        // the wheel overruns (the gain is 0 while pedalling).
        if p.freewheel_gain > SILENT && p.freewheel_rate_hz > 0.0 {
            self.freewheel_phase += p.freewheel_rate_hz / sr / self.freewheel_jitter;
            if self.freewheel_phase >= 1.0 {
                self.freewheel_phase -= 1.0;
                self.freewheel_jitter = 0.92 + 0.16 * self.noise.unit();
                self.click
                    .trigger(0.7 + 0.3 * self.noise.unit(), 0.0011, sr);
            }
            let env = self.click.step();
            if env > 0.0 {
                let n = self.noise.white();
                let ring = sine(&mut self.click_ring, 5200.0, sr);
                out += p.freewheel_gain * env * (self.click_bp.process(n).band * 2.5 + ring * 0.35);
            }
        }
        // Drivetrain: a soft, continuous whir – broad band noise, gently
        // swelling with each pedal stroke – with a faint hum at the chain's
        // tooth rate. No pulses: the chain over the teeth is too fast and
        // even to be heard as single clicks (ticks are the freehub's).
        if p.chain_gain > SILENT && p.chain_rate_hz > 0.0 {
            let hum = sine(&mut self.chain_phase, p.chain_rate_hz, sr);
            let stroke = sine(&mut self.stroke_phase, p.stroke_hz, sr);
            let n = self.noise.white();
            let whir = self.chain_lp.process(self.chain_bp.process(n).band).low;
            out += p.chain_gain * (1.0 + 0.15 * stroke) * (whir * 2.2 + hum * 0.12);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kmh: f64, cadence: Option<f64>, power: f64, surface: Surface) -> RideInput {
        RideInput {
            speed_mps: kmh / 3.6,
            cadence_rpm: cadence,
            power_w: power,
            surface,
        }
    }

    fn asphalt(kmh: f64, cadence: f64) -> RideParams {
        ride_params(&input(kmh, Some(cadence), 200.0, Surface::Asphalt))
    }

    #[test]
    fn standing_still_is_silent() {
        let p = ride_params(&input(0.0, Some(0.0), 0.0, Surface::Gravel));
        assert_eq!(
            (
                p.wind_gain,
                p.roll_gain,
                p.grit_gain,
                p.freewheel_gain,
                p.chain_gain
            ),
            (0.0, 0.0, 0.0, 0.0, 0.0)
        );
        let mut s = RideSynth::new(48_000.0);
        assert!((0..4800).all(|_| s.sample(&p) == 0.0));
    }

    #[test]
    fn wind_rises_with_speed() {
        let mut last = (0.0, 0.0);
        for kmh in [5.0, 15.0, 25.0, 35.0, 45.0, 60.0] {
            let p = asphalt(kmh, 90.0);
            assert!(p.wind_gain > last.0, "{kmh} km/h");
            assert!(p.wind_cutoff_hz > last.1, "{kmh} km/h");
            last = (p.wind_gain, p.wind_cutoff_hz);
        }
        // Faster than linear: at twice the speed more than twice as loud.
        assert!(asphalt(40.0, 90.0).wind_gain > 2.0 * asphalt(20.0, 90.0).wind_gain);
        // Capped at very high speed (downhill).
        assert!(asphalt(120.0, 0.0).wind_gain <= 0.9);
    }

    #[test]
    fn tyre_noise_follows_the_surface() {
        let at = |s| ride_params(&input(25.0, Some(90.0), 200.0, s));
        let (a, g, m) = (
            at(Surface::Asphalt),
            at(Surface::Gravel),
            at(Surface::Grass),
        );
        assert!(a.roll_gain > 0.0 && a.grit_rate_hz == 0.0 && a.swish_gain == 0.0);
        assert!(g.roll_gain > a.roll_gain, "gravel rumbles more");
        assert!(g.grit_rate_hz > 0.0 && g.grit_gain > 0.0);
        assert!(m.swish_gain > 0.0 && m.grit_rate_hz < g.grit_rate_hz);
        // Stones per second grow with the speed.
        let slow = ride_params(&input(10.0, Some(90.0), 200.0, Surface::Gravel));
        assert!(slow.grit_rate_hz < g.grit_rate_hz);
    }

    /// Coasting: the legs still, no power.
    fn coasting(kmh: f64) -> RideParams {
        ride_params(&input(kmh, Some(0.0), 0.0, Surface::Asphalt))
    }

    #[test]
    fn freehub_ticks_while_coasting() {
        let c = coasting(18.0);
        let expected = 5.0 / WHEEL_CIRCUMFERENCE_M * FREEWHEEL_POINTS;
        assert!((f64::from(c.freewheel_rate_hz) - expected).abs() < 1e-3);
        assert!(c.freewheel_gain > 0.0 && c.chain_gain == 0.0);
        // Twice the speed, twice the ticks.
        let fast = coasting(36.0);
        assert!((fast.freewheel_rate_hz / c.freewheel_rate_hz - 2.0).abs() < 1e-4);
        // No cadence sensor and no power: coasting too.
        assert!(ride_params(&input(25.0, None, 0.0, Surface::Asphalt)).freewheel_gain > 0.0);
        // A slow crank turn (below the pedalling cadence) without power.
        assert!(ride_params(&input(18.0, Some(10.0), 0.0, Surface::Asphalt)).freewheel_gain > 0.0);
        // Not when stopped: the wheel does not turn.
        assert_eq!(coasting(0.0).freewheel_gain, 0.0);
        assert_eq!(coasting(0.5).freewheel_gain, 0.0);
    }

    #[test]
    fn freehub_is_silent_while_pedalling() {
        for kmh in [0.0, 5.0, 18.0, 30.0, 45.0, 70.0] {
            for cadence in [40.0, 60.0, 85.0, 110.0] {
                let p = asphalt(kmh, cadence);
                assert_eq!(p.freewheel_gain, 0.0, "{kmh} km/h, {cadence} rpm");
                assert!(p.chain_gain > 0.0, "{kmh} km/h, {cadence} rpm");
            }
            // No cadence sensor: power means pedalling.
            for power in [11.0, 80.0, 250.0, 900.0] {
                let p = ride_params(&input(kmh, None, power, Surface::Gravel));
                assert_eq!(p.freewheel_gain, 0.0, "{kmh} km/h, {power} W");
                assert!(p.chain_gain > 0.0);
            }
            // The cadence reads 0 (a dropout) while the power flows.
            let p = ride_params(&input(kmh, Some(0.0), 200.0, Surface::Asphalt));
            assert_eq!(p.freewheel_gain, 0.0, "{kmh} km/h");
            assert!(p.chain_gain > 0.0);
        }
        // Turning the cranks without measured power at a speed they can
        // drive: the hub is engaged.
        assert_eq!(
            ride_params(&input(30.0, Some(85.0), 0.0, Surface::Asphalt)).freewheel_gain,
            0.0
        );
    }

    /// Spinning the legs slower than the wheel (no power): the wheel
    /// overruns the cassette, the freehub ticks under the chain.
    #[test]
    fn freehub_ticks_when_the_wheel_overruns_the_legs() {
        let p = ride_params(&input(55.0, Some(30.0), 0.0, Surface::Asphalt));
        assert!(p.freewheel_gain > 0.0 && p.chain_gain > 0.0);
        assert!(!drives_wheel(&input(
            55.0,
            Some(30.0),
            0.0,
            Surface::Asphalt
        )));
        assert!(drives_wheel(&input(
            30.0,
            Some(90.0),
            0.0,
            Surface::Asphalt
        )));
        assert!(drives_wheel(&input(
            55.0,
            Some(30.0),
            150.0,
            Surface::Asphalt
        )));
    }

    #[test]
    fn chain_runs_at_the_cadence() {
        let p = asphalt(30.0, 90.0);
        assert!((p.chain_rate_hz - 75.0).abs() < 1e-4, "{}", p.chain_rate_hz);
        assert!((p.stroke_hz - 3.0).abs() < 1e-4);
        assert!(asphalt(30.0, 60.0).chain_rate_hz < p.chain_rate_hz);
        // Louder under load.
        let easy = ride_params(&input(30.0, Some(90.0), 80.0, Surface::Asphalt));
        let hard = ride_params(&input(30.0, Some(90.0), 450.0, Surface::Asphalt));
        assert!(hard.chain_gain > easy.chain_gain);
        // On the trainer at standstill (pedalling against the brake) the
        // chain still runs.
        assert!(asphalt(0.0, 90.0).chain_gain > 0.0);
    }

    #[test]
    fn cadence_is_estimated_without_a_sensor() {
        let at = |cadence, power| effective_cadence(&input(30.0, cadence, power, Surface::Asphalt));
        assert_eq!(at(None, 200.0), 85.0);
        assert_eq!(at(None, 2000.0), 100.0);
        // A sensor reading 0 under power: the estimate too.
        assert_eq!(at(Some(0.0), 200.0), 85.0);
        // A measured cadence wins.
        assert_eq!(at(Some(70.0), 200.0), 70.0);
        // No power, no cadence: coasting.
        assert_eq!(at(None, 0.0), 0.0);
        let p = ride_params(&input(30.0, None, 0.0, Surface::Asphalt));
        assert!(p.freewheel_gain > 0.0 && p.chain_gain == 0.0);
    }

    /// One mixer block (32 samples at 48 kHz).
    const DT: f32 = 32.0 / 48_000.0;

    /// Approaches `t` for `seconds`, yielding the freewheel gain each block.
    fn ramp_for(p: &mut RideParams, t: &RideParams, seconds: f32) -> Vec<f32> {
        (0..(seconds / DT).round() as usize)
            .map(|_| {
                p.approach(t, DT);
                p.freewheel_gain
            })
            .collect()
    }

    #[test]
    fn parameters_approach_the_target() {
        let target = asphalt(30.0, 90.0);
        let mut p = RideParams::default();
        // One time constant × ln 2: half way.
        p.approach(&target, PARAM_TAU_S * std::f32::consts::LN_2);
        assert!((p.wind_gain - target.wind_gain / 2.0).abs() < 1e-6);
        // Rates jump to the target, gains fade.
        assert_eq!(p.chain_rate_hz, target.chain_rate_hz);
        ramp_for(&mut p, &target, 1.0);
        assert!((p.wind_gain - target.wind_gain).abs() < 1e-6);
        // Coasting afterwards: the chain fades out at its last rate …
        let coast = coasting(30.0);
        p.approach(&coast, DT);
        assert_eq!(p.chain_rate_hz, target.chain_rate_hz);
        assert!(p.chain_gain < target.chain_gain && p.chain_gain > 0.0);
        // … and reaches exactly 0.
        ramp_for(&mut p, &coast, 1.0);
        assert_eq!(p.chain_gain, 0.0);
    }

    /// Pedalling stops: the freehub fades in smoothly (no burst), without
    /// overshooting its level.
    #[test]
    fn freehub_fades_in_when_pedalling_stops() {
        let mut p = asphalt(30.0, 90.0);
        let target = coasting(30.0);
        let g = ramp_for(&mut p, &target, 0.6);
        let at = |s: f32| g[(s / DT) as usize - 1];
        assert!(g.windows(2).all(|w| w[1] >= w[0]), "rising");
        assert!(g.iter().all(|&x| x <= target.freewheel_gain));
        assert!(at(0.02) < 0.3 * target.freewheel_gain, "{}", at(0.02));
        assert!(at(0.1) > 0.5 * target.freewheel_gain);
        assert!(at(0.5) > 0.95 * target.freewheel_gain);
    }

    /// Pedalling starts: the ticks are gone within tens of milliseconds,
    /// and the gain is exactly 0 after that.
    #[test]
    fn freehub_fades_out_fast_when_pedalling_starts() {
        let mut p = coasting(30.0);
        let start = p.freewheel_gain;
        // With a cadence sensor, and without one (power only).
        for target in [
            asphalt(30.0, 90.0),
            ride_params(&input(30.0, None, 200.0, Surface::Asphalt)),
        ] {
            let mut q = p;
            let g = ramp_for(&mut q, &target, 0.2);
            let at = |s: f32| g[(s / DT) as usize - 1];
            assert!(g.windows(2).all(|w| w[1] <= w[0]), "falling");
            assert!(at(0.03) < 0.05 * start, "{}", at(0.03));
            assert_eq!(at(0.1), 0.0);
        }
        // The synthesis plays no tick once the gain is 0.
        ramp_for(&mut p, &asphalt(30.0, 90.0), 0.2);
        let only_ticks = RideParams {
            freewheel_gain: p.freewheel_gain,
            freewheel_rate_hz: p.freewheel_rate_hz,
            ..RideParams::default()
        };
        assert!(p.freewheel_rate_hz > 0.0);
        assert_eq!(rms(&only_ticks, 0.2), 0.0);
    }

    fn render(p: &RideParams, seconds: f32) -> Vec<f32> {
        let mut s = RideSynth::new(48_000.0);
        let n = (48_000.0 * seconds) as usize;
        (0..n)
            .map(|i| {
                if i % 32 == 0 {
                    s.set_filters(p);
                }
                let v = s.sample(p);
                assert!(v.is_finite() && v.abs() < 2.0, "{v}");
                v
            })
            .collect()
    }

    fn rms(p: &RideParams, seconds: f32) -> f32 {
        let x = render(p, seconds);
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// How much the loudness varies from one 10 ms window to the next
    /// (coefficient of variation): small for a continuous sound, large for
    /// a click train.
    fn roughness(x: &[f32]) -> f32 {
        let w: Vec<f32> = x
            .chunks(480)
            .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
            .collect();
        let mean = w.iter().sum::<f32>() / w.len() as f32;
        let var = w.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / w.len() as f32;
        var.sqrt() / mean
    }

    /// The drivetrain is a continuous whir, not a train of clicks; the
    /// clicks are the freehub's.
    #[test]
    fn the_drivetrain_whirs_and_the_freehub_ticks() {
        for cadence in [60.0, 90.0, 110.0] {
            let p = asphalt(25.0, cadence);
            let chain = RideParams {
                chain_gain: p.chain_gain,
                chain_rate_hz: p.chain_rate_hz,
                stroke_hz: p.stroke_hz,
                ..RideParams::default()
            };
            let x = render(&chain, 1.1);
            let r = roughness(&x[4800..]);
            assert!(r < 0.35, "chain at {cadence} rpm: {r}");
        }
        let c = coasting(15.0);
        let ticks = RideParams {
            freewheel_gain: c.freewheel_gain,
            freewheel_rate_hz: c.freewheel_rate_hz,
            ..RideParams::default()
        };
        let r = roughness(&render(&ticks, 1.1)[4800..]);
        assert!(r > 0.8, "freehub: {r}");
    }

    /// The synthesised level follows the parameters: louder when faster,
    /// gravel louder than asphalt.
    #[test]
    fn synth_level_follows_the_parameters() {
        let slow = rms(&coasting(10.0), 0.5);
        let fast = rms(&asphalt(45.0, 90.0), 0.5);
        assert!(slow > 0.003, "{slow}");
        assert!(fast > 2.0 * slow, "{fast} vs {slow}");
        let gravel = rms(
            &ride_params(&input(20.0, Some(0.0), 0.0, Surface::Gravel)),
            0.5,
        );
        let road = rms(&coasting(20.0), 0.5);
        assert!(gravel > road, "{gravel} vs {road}");
    }
}
