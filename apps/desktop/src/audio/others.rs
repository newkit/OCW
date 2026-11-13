//! The other riders' bikes (spec §3.3, decided 2026-11-13): every other
//! rider – bots and remote riders; the time-trial ghost stays silent –
//! makes the same synthesised sounds as the own bike (freehub ticks only
//! while coasting, drivetrain whir, tyres on the surface; no wind), placed
//! by [`super::spatial`] around the own rider's head.
//!
//! A group must sound like a group, not like copies of one bike:
//! - every rider has a [`Character`] derived from their id (the same on
//!   every run and every client): the hub's engagement points, the wheel,
//!   the click's ring and length, the drivetrain's and tyres' tuning, the
//!   cadence they spin at, a level, noise seeds and start phases;
//! - only the nearest [`FULL_VOICES`] riders get their own synthesis
//!   (with hysteresis, so two riders at about the same distance do not
//!   swap); the rest of the audible field is summed into one cheap
//!   [`GroupTarget`] layer: decorrelated rolling and whir noise left and
//!   right, and one random stream of ticks drawn from the coasting riders'
//!   rates, levels and positions;
//! - a crowd normaliser keeps a dense bunch from adding up past
//!   [`CROWD_MAX_ENERGY`], so the sum stays clear of the limiter.
//!
//! [`frame`] (game thread) and the parameter maths are pure and tested;
//! [`OthersSynth`] renders the frames in the mixer (stereo).

use ocw_core::RiderId;
use ocw_sim::RiderState;
use ocw_world::{Surface, Track};

use super::dsp::{Burst, Noise, Svf};
use super::ride::{
    CHAINRING_TEETH, PEDALLING_W, RideInput, RideParams, RideSynth, SILENT, Timbre, freewheel_gain,
    ride_params,
};
use super::spatial::{Listener, Placement, place};

/// Riders with their own synthesis (nearest first).
pub const FULL_VOICES: usize = 8;
/// At most this many further riders feed the group layer (nearest first).
pub const MAX_GROUP: usize = 48;
/// A rider already playing counts this much nearer when ranking.
pub const KEEP_FACTOR: f64 = 0.8;
/// Height of the listener's ears and of a bike's sounds above the road (m).
pub const HEAD_HEIGHT_M: f64 = 1.5;
pub const SOURCE_HEIGHT_M: f64 = 0.5;
/// The other riders together never get louder than this many riders at
/// the reference distance (energy, Σ gain²): a bunch of 30 must not clip.
pub const CROWD_MAX_ENERGY: f32 = 2.0;
/// Another bike at the reference distance (2 m) against the own one
/// (under the rider, about 1 m from the ears): half as loud.
pub const OTHERS_LEVEL: f32 = 0.5;
/// Hub engagement points to choose from (common road and gravel hubs).
const POE_CHOICES: [f64; 7] = [16.0, 18.0, 24.0, 24.0, 30.0, 36.0, 54.0];

/// splitmix64: a well-mixed hash of the rider id.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A rider's bike sound, deterministic by id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Character {
    /// Hub engagement points (ticks per wheel revolution).
    pub points: f64,
    /// Wheel circumference (m): tyre widths differ.
    pub wheel_m: f64,
    /// The cadence they ride at when the snapshot has none (rpm).
    pub cadence_rpm: f64,
    /// Overall level (some bikes are noisier).
    pub level: f32,
    pub timbre: Timbre,
}

impl Character {
    pub fn of(id: RiderId) -> Self {
        let mut h = mix64(u64::from(id.0) ^ 0x0C57_A0D1);
        let seed = (mix64(h ^ 0x5EED) as u32) | 1;
        // 16 bits at a time, as a fraction 0–1.
        let mut u = || {
            h = mix64(h);
            (h & 0xFFFF) as f64 / 65535.0
        };
        let points = POE_CHOICES[(u() * POE_CHOICES.len() as f64) as usize % POE_CHOICES.len()];
        let wheel_m = 2.08 + 0.10 * u();
        let cadence_rpm = 78.0 + 18.0 * u();
        let level = (0.8 + 0.4 * u()) as f32;
        let timbre = Timbre {
            seed,
            click_hz: (3000.0 + 2800.0 * u()) as f32,
            ring_hz: (4200.0 + 2400.0 * u()) as f32,
            click_tau_s: (0.0008 + 0.0007 * u()) as f32,
            chain_scale: (0.8 + 0.45 * u()) as f32,
            roll_scale: (0.85 + 0.3 * u()) as f32,
            tick_phase: u() as f32,
            stroke_phase: u() as f32,
        };
        Self {
            points,
            wheel_m,
            cadence_rpm,
            level,
            timbre,
        }
    }
}

/// Another rider as the audio sees them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtherRider {
    pub id: RiderId,
    /// Where the bike's sounds come from and how it moves.
    pub pos: [f64; 3],
    pub velocity: [f64; 3],
    pub speed_mps: f64,
    pub power_w: f64,
    /// From the snapshot if it carries one (it does not yet: the power
    /// decides, spec §3.3).
    pub cadence_rpm: Option<f64>,
    pub surface: Surface,
}

/// Position (at `height` above the road) and velocity of a rider in
/// `state` on `track`.
pub fn on_track(track: &dyn Track, state: &RiderState, height: f64) -> ([f64; 3], [f64; 3]) {
    let p = track.sample(state.distance);
    let mut pos = p.offset_position(state.lateral_offset);
    pos[1] += height;
    // Heading is relative to the road, positive = right.
    let (s, c) = state.heading.sin_cos();
    let [dx, dz] = p.direction;
    let [rx, rz] = p.right();
    let (fx, fz) = (dx * c + rx * s, dz * c + rz * s);
    let v = state.speed.max(0.0);
    let climb = p.grade / (1.0 + p.grade * p.grade).sqrt();
    (pos, [fx * v, climb * v, fz * v])
}

/// The listener at the own rider's head.
pub fn listener(track: &dyn Track, state: &RiderState) -> Listener {
    let (pos, velocity) = on_track(track, state, HEAD_HEIGHT_M);
    let h = (velocity[0] * velocity[0] + velocity[2] * velocity[2]).sqrt();
    let forward = if h > 1e-6 {
        [velocity[0] / h, velocity[2] / h]
    } else {
        let p = track.sample(state.distance);
        let (s, c) = state.heading.sin_cos();
        let r = p.right();
        [p.direction[0] * c + r[0] * s, p.direction[1] * c + r[1] * s]
    };
    Listener {
        pos,
        forward,
        velocity,
    }
}

impl OtherRider {
    /// What the ride-sound mapping sees: without a cadence in the
    /// snapshot, a rider putting out power pedals at their own cadence
    /// (so a group's whirs differ), one without power coasts.
    pub fn input(&self, c: &Character) -> RideInput {
        let cadence = self
            .cadence_rpm
            .or_else(|| (self.power_w > PEDALLING_W).then_some(c.cadence_rpm));
        RideInput {
            speed_mps: self.speed_mps,
            cadence_rpm: cadence,
            power_w: self.power_w,
            surface: self.surface,
        }
    }

    /// Whether the freehub ticks (the wheel overruns the legs).
    #[cfg(test)]
    pub fn coasting(&self, c: &Character) -> bool {
        self.speed_mps > 0.3 && !super::ride::drives_wheel(&self.input(c))
    }

    /// Whether the drivetrain whirs.
    #[cfg(test)]
    pub fn pedalling(&self, c: &Character) -> bool {
        super::ride::effective_cadence(&self.input(c)) >= super::ride::PEDALLING_RPM
    }
}

/// The rider's bike sounds before placement: the own mapping without the
/// wind (that is the listener's own air), with their hub, wheel and level.
pub fn other_params(r: &OtherRider, c: &Character) -> RideParams {
    let mut p = ride_params(&r.input(c));
    p.wind_gain = 0.0;
    if p.freewheel_gain > 0.0 {
        let rate = r.speed_mps.max(0.0) / c.wheel_m * c.points;
        p.freewheel_rate_hz = rate as f32;
        p.freewheel_gain = freewheel_gain(rate);
    }
    if p.chain_gain > 0.0 && r.cadence_rpm.is_none() {
        p.chain_rate_hz = (c.cadence_rpm / 60.0 * CHAINRING_TEETH) as f32;
        p.stroke_hz = (2.0 * c.cadence_rpm / 60.0) as f32;
    }
    for g in [
        &mut p.roll_gain,
        &mut p.grit_gain,
        &mut p.swish_gain,
        &mut p.freewheel_gain,
        &mut p.chain_gain,
    ] {
        *g *= c.level;
    }
    p
}

/// One rider with their own synthesis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceTarget {
    pub id: RiderId,
    pub character: Character,
    pub params: RideParams,
    pub left: f32,
    pub right: f32,
    pub air_cutoff_hz: f32,
    pub doppler: f32,
}

/// A coasting rider of the group layer: their ticks join one stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickSource {
    pub rate_hz: f32,
    pub gain: f32,
    pub left: f32,
    pub right: f32,
    pub click_hz: f32,
}

/// The rest of the audible field, summed (energies per channel).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupTarget {
    pub roll_left: f32,
    pub roll_right: f32,
    pub roll_cutoff_hz: f32,
    pub chain_left: f32,
    pub chain_right: f32,
    pub chain_rate_hz: f32,
    pub air_cutoff_hz: f32,
    pub ticks: Vec<TickSource>,
    /// Riders in the layer.
    pub riders: usize,
}

/// Everything the mixer needs for the other riders, once per frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OthersFrame {
    pub voices: Vec<VoiceTarget>,
    pub group: GroupTarget,
}

/// Riders with their placement, nearest first.
pub type Ranked<'a> = Vec<(&'a OtherRider, Placement)>;

/// The audible riders ranked: the nearest `n` (those in `playing`
/// counting [`KEEP_FACTOR`] of their distance) fully, the next
/// [`MAX_GROUP`] for the group layer. Each sorted nearest first.
pub fn select<'a>(
    listener: &Listener,
    riders: &'a [OtherRider],
    playing: &[RiderId],
    n: usize,
) -> (Ranked<'a>, Ranked<'a>) {
    let mut ranked: Vec<(f64, &OtherRider, Placement)> = riders
        .iter()
        .map(|r| (r, place(listener, r.pos, r.velocity)))
        .filter(|(_, p)| p.audible())
        .map(|(r, p)| {
            let keep = if playing.contains(&r.id) {
                KEEP_FACTOR
            } else {
                1.0
            };
            (p.distance_m * keep, r, p)
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    let mut it = ranked.into_iter().map(|(_, r, p)| (r, p));
    let full: Vec<_> = it.by_ref().take(n).collect();
    let group: Vec<_> = it.take(MAX_GROUP).collect();
    (full, group)
}

/// The frame for `riders` heard by `listener`; `playing` are the riders
/// that had their own voice in the last frame (hysteresis).
pub fn frame(listener: &Listener, riders: &[OtherRider], playing: &[RiderId]) -> OthersFrame {
    let (full, group) = select(listener, riders, playing, FULL_VOICES);
    let mut out = OthersFrame::default();
    let mut energy = 0.0f32;
    for (r, pl) in &full {
        let c = Character::of(r.id);
        let params = other_params(r, &c);
        energy += (pl.gain * c.level).powi(2);
        out.voices.push(VoiceTarget {
            id: r.id,
            character: c,
            params,
            left: pl.left,
            right: pl.right,
            air_cutoff_hz: pl.air_cutoff_hz,
            doppler: pl.doppler,
        });
    }
    let g = &mut out.group;
    let (mut roll_w, mut chain_w, mut air_w) = (0.0f32, 0.0f32, 0.0f32);
    let (mut roll_cut, mut chain_rate, mut air_cut) = (0.0f32, 0.0f32, 0.0f32);
    for (r, pl) in &group {
        let c = Character::of(r.id);
        let p = other_params(r, &c);
        energy += (pl.gain * c.level).powi(2);
        g.riders += 1;
        let roll = p.roll_gain + p.grit_gain * 0.5 + p.swish_gain * 0.5;
        g.roll_left += (roll * pl.left).powi(2);
        g.roll_right += (roll * pl.right).powi(2);
        g.chain_left += (p.chain_gain * pl.left).powi(2);
        g.chain_right += (p.chain_gain * pl.right).powi(2);
        let (wr, wc) = ((roll * pl.gain).powi(2), (p.chain_gain * pl.gain).powi(2));
        roll_cut += wr * p.roll_cutoff_hz;
        roll_w += wr;
        chain_rate += wc * p.chain_rate_hz;
        chain_w += wc;
        let wa = wr + wc + (p.freewheel_gain * pl.gain).powi(2);
        air_cut += wa * pl.air_cutoff_hz;
        air_w += wa;
        if p.freewheel_gain > 0.0 {
            g.ticks.push(TickSource {
                rate_hz: p.freewheel_rate_hz,
                gain: p.freewheel_gain * pl.gain,
                left: pl.left / pl.gain.max(1e-6),
                right: pl.right / pl.gain.max(1e-6),
                click_hz: c.timbre.click_hz,
            });
        }
    }
    for v in [
        &mut g.roll_left,
        &mut g.roll_right,
        &mut g.chain_left,
        &mut g.chain_right,
    ] {
        *v = v.sqrt();
    }
    g.roll_cutoff_hz = if roll_w > 0.0 {
        roll_cut / roll_w
    } else {
        300.0
    };
    g.chain_rate_hz = if chain_w > 0.0 {
        chain_rate / chain_w
    } else {
        0.0
    };
    g.air_cutoff_hz = if air_w > 0.0 { air_cut / air_w } else { 4000.0 };
    // Further away than the own bike; a dense bunch: scale everyone down
    // together.
    let crowd = if energy > CROWD_MAX_ENERGY {
        (CROWD_MAX_ENERGY / energy).sqrt()
    } else {
        1.0
    };
    {
        let k = OTHERS_LEVEL * crowd;
        for v in &mut out.voices {
            v.left *= k;
            v.right *= k;
        }
        let g = &mut out.group;
        for x in [
            &mut g.roll_left,
            &mut g.roll_right,
            &mut g.chain_left,
            &mut g.chain_right,
        ] {
            *x *= k;
        }
        for t in &mut g.ticks {
            t.gain *= k;
        }
    }
    out
}

/// The fraction a one-pole ramp with time constant `tau_s` moves in `dt_s`.
fn ramp(dt_s: f32, tau_s: f32) -> f32 {
    1.0 - (-dt_s.max(0.0) / tau_s).exp()
}

/// How fast the placement follows (s): a rider passing glides across.
const PLACE_TAU_S: f32 = 0.05;

/// One rider's own synthesis in the mixer.
struct Voice {
    id: RiderId,
    synth: RideSynth,
    now: RideParams,
    target: RideParams,
    left: f32,
    right: f32,
    target_lr: (f32, f32),
    air: Svf,
    air_cutoff_hz: f32,
    doppler: f32,
    /// Still in the frame; else fading out.
    active: bool,
}

impl Voice {
    fn new(t: &VoiceTarget, sr: f32) -> Self {
        Self {
            id: t.id,
            synth: RideSynth::with_timbre(sr, t.character.timbre),
            // From silence: the voice fades in.
            now: RideParams::default(),
            target: t.params,
            left: t.left,
            right: t.right,
            target_lr: (t.left, t.right),
            air: Svf::new(t.air_cutoff_hz, 0.7, sr),
            air_cutoff_hz: t.air_cutoff_hz,
            doppler: t.doppler,
            active: true,
        }
    }

    fn silent(&self) -> bool {
        let p = &self.now;
        [
            p.roll_gain,
            p.grit_gain,
            p.swish_gain,
            p.freewheel_gain,
            p.chain_gain,
        ]
        .iter()
        .all(|&g| g == 0.0)
    }
}

/// The group layer's synthesis: two decorrelated noises (left, right)
/// for the rolling and the whir, and a pool of tick slots.
struct Group {
    target: GroupTarget,
    /// Smoothed levels: roll l/r, chain l/r.
    levels: [f32; 4],
    noise: [Noise; 2],
    roll: [Svf; 2],
    chain_bp: [Svf; 2],
    air: [Svf; 2],
    picker: Noise,
    slots: [TickSlot; 4],
    next_slot: usize,
}

#[derive(Clone)]
struct TickSlot {
    env: Burst,
    bp: Svf,
    left: f32,
    right: f32,
}

impl Group {
    fn new(sr: f32) -> Self {
        let slot = TickSlot {
            env: Burst::default(),
            bp: Svf::new(4000.0, 4.0, sr),
            left: 0.0,
            right: 0.0,
        };
        Self {
            target: GroupTarget::default(),
            levels: [0.0; 4],
            noise: [Noise::new(0x6A0B_0001), Noise::new(0x6A0B_0002)],
            roll: [Svf::new(300.0, 0.8, sr), Svf::new(300.0, 0.8, sr)],
            chain_bp: [Svf::new(700.0, 0.7, sr), Svf::new(700.0, 0.7, sr)],
            air: [Svf::new(4000.0, 0.7, sr), Svf::new(4000.0, 0.7, sr)],
            picker: Noise::new(0x6A0B_0003),
            slots: [slot.clone(), slot.clone(), slot.clone(), slot],
            next_slot: 0,
        }
    }

    fn block(&mut self, dt_s: f32, sr: f32) {
        let t = &self.target;
        let a = ramp(dt_s, PLACE_TAU_S);
        let want = [t.roll_left, t.roll_right, t.chain_left, t.chain_right];
        for (l, w) in self.levels.iter_mut().zip(want) {
            *l += (w - *l) * a;
            if w == 0.0 && *l < SILENT {
                *l = 0.0;
            }
        }
        for f in &mut self.roll {
            f.set(t.roll_cutoff_hz.max(80.0), 0.8, sr);
        }
        let whir = (350.0 + 5.0 * t.chain_rate_hz).min(1200.0);
        for f in &mut self.chain_bp {
            f.set(whir, 0.7, sr);
        }
        for f in &mut self.air {
            f.set(t.air_cutoff_hz.max(500.0), 0.7, sr);
        }
    }

    fn sample(&mut self, sr: f32) -> (f32, f32) {
        let mut out = [0.0f32; 2];
        let [rl, rr, cl, cr] = self.levels;
        if rl + rr + cl + cr > 0.0 {
            for (ch, o) in out.iter_mut().enumerate() {
                let n = self.noise[ch].white();
                let (roll, chain) = if ch == 0 { (rl, cl) } else { (rr, cr) };
                *o += roll * self.roll[ch].process(n).low * 2.0;
                *o += chain * self.chain_bp[ch].process(n).band * 1.6;
            }
        }
        // Ticks: one random stream at the riders' summed rate; each tick
        // takes a rider (by their rate) for its level, pitch and side.
        let total: f32 = self.target.ticks.iter().map(|t| t.rate_hz).sum();
        if total > 0.0 && self.picker.unit() < (total / sr).min(1.0) {
            let mut x = self.picker.unit() * total;
            let mut pick = self.target.ticks[self.target.ticks.len() - 1];
            for t in &self.target.ticks {
                if x < t.rate_hz {
                    pick = *t;
                    break;
                }
                x -= t.rate_hz;
            }
            let n = self.slots.len();
            let s = &mut self.slots[self.next_slot];
            self.next_slot = (self.next_slot + 1) % n;
            let amp = pick.gain * (0.7 + 0.3 * self.picker.unit());
            s.env.trigger(amp, 0.001, sr);
            let k = 0.92 + 0.16 * self.picker.unit();
            s.bp.set(pick.click_hz * k, 4.0, sr);
            s.left = pick.left;
            s.right = pick.right;
        }
        for s in &mut self.slots {
            let e = s.env.step();
            if e > 0.0 {
                let v = e * s.bp.process(self.picker.white()).band * 2.5;
                out[0] += v * s.left;
                out[1] += v * s.right;
            }
        }
        let l = self.air[0].process(out[0]).low;
        let r = self.air[1].process(out[1]).low;
        (l, r)
    }
}

/// At most this many voices at once, fading ones included.
const MAX_LIVE_VOICES: usize = 2 * FULL_VOICES;

/// Renders the other riders (in the mixer, stereo).
pub struct OthersSynth {
    sr: f32,
    voices: Vec<Voice>,
    group: Group,
}

impl OthersSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            voices: Vec::new(),
            group: Group::new(sample_rate),
        }
    }

    /// Voices playing (fading ones included).
    #[cfg(test)]
    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    pub fn apply(&mut self, f: OthersFrame) {
        for v in &mut self.voices {
            v.active = false;
        }
        for t in &f.voices {
            match self.voices.iter_mut().find(|v| v.id == t.id) {
                Some(v) => {
                    v.target = t.params;
                    v.target_lr = (t.left, t.right);
                    v.air_cutoff_hz = t.air_cutoff_hz;
                    v.doppler = t.doppler;
                    v.active = true;
                }
                None => self.voices.push(Voice::new(t, self.sr)),
            }
        }
        for v in &mut self.voices {
            if !v.active {
                v.target = RideParams::default();
            }
        }
        // Too many fading voices (riders jumping around): drop the oldest
        // inactive first.
        while self.voices.len() > MAX_LIVE_VOICES {
            match self.voices.iter().position(|v| !v.active) {
                Some(i) => self.voices.remove(i),
                None => self.voices.remove(0),
            };
        }
        self.group.target = f.group;
    }

    /// Once per block of `block` samples.
    pub fn block(&mut self, block: usize) {
        let dt = block as f32 / self.sr;
        let a = ramp(dt, PLACE_TAU_S);
        let sr = self.sr;
        for v in &mut self.voices {
            v.now.approach(&v.target, dt);
            v.left += (v.target_lr.0 - v.left) * a;
            v.right += (v.target_lr.1 - v.right) * a;
            v.air.set(v.air_cutoff_hz, 0.7, sr);
            v.synth.set_pitch(v.doppler);
            v.synth.set_filters(&v.now);
        }
        self.voices.retain(|v| v.active || !v.silent());
        self.group.block(dt, sr);
    }

    /// The next stereo sample.
    pub fn sample(&mut self) -> (f32, f32) {
        let (mut l, mut r) = self.group.sample(self.sr);
        for v in &mut self.voices {
            let m = v.air.process(v.synth.sample(&v.now)).low;
            l += m * v.left;
            r += m * v.right;
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    fn listener() -> Listener {
        Listener {
            pos: [0.0, HEAD_HEIGHT_M, 0.0],
            forward: [1.0, 0.0],
            velocity: [8.0, 0.0, 0.0],
        }
    }

    /// A rider at (x, z) going +x at `kmh`, putting out `power`.
    fn rider(id: u32, x: f64, z: f64, kmh: f64, power: f64) -> OtherRider {
        let v = kmh / 3.6;
        OtherRider {
            id: RiderId(id),
            pos: [x, SOURCE_HEIGHT_M, z],
            velocity: [v, 0.0, 0.0],
            speed_mps: v,
            power_w: power,
            cadence_rpm: None,
            surface: Surface::Asphalt,
        }
    }

    #[test]
    fn character_is_deterministic_by_rider_id() {
        for id in [0, 1, 7, 12345, u32::MAX] {
            assert_eq!(Character::of(RiderId(id)), Character::of(RiderId(id)));
        }
        let cs: Vec<Character> = (1..=30).map(|i| Character::of(RiderId(i))).collect();
        for c in &cs {
            assert!(POE_CHOICES.contains(&c.points));
            assert!((2.08..=2.18).contains(&c.wheel_m));
            assert!((78.0..=96.0).contains(&c.cadence_rpm));
            assert!((0.8..=1.2).contains(&c.level));
            assert!((3000.0..=5800.0).contains(&c.timbre.click_hz));
            assert!((0.0..=1.0).contains(&c.timbre.tick_phase));
        }
        // A field of 30 is varied: no two alike, several hubs, a spread of
        // pitches and phases.
        for (i, a) in cs.iter().enumerate() {
            for b in &cs[i + 1..] {
                assert_ne!(a.timbre, b.timbre);
            }
        }
        let hubs: std::collections::BTreeSet<u64> = cs.iter().map(|c| c.points as u64).collect();
        assert!(hubs.len() >= 4, "{hubs:?}");
        let spread = |f: fn(&Character) -> f64| {
            let v: Vec<f64> = cs.iter().map(f).collect();
            v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(spread(|c| f64::from(c.timbre.click_hz)) > 1500.0);
        assert!(spread(|c| f64::from(c.timbre.tick_phase)) > 0.7);
        assert!(spread(|c| c.cadence_rpm) > 10.0);
        // So two riders coasting side by side tick at different rates.
        let (a, b) = (rider(1, 3.0, 0.0, 30.0, 0.0), rider(2, 3.0, 1.0, 30.0, 0.0));
        let (ca, cb) = (Character::of(a.id), Character::of(b.id));
        let ra = other_params(&a, &ca).freewheel_rate_hz;
        let rb = other_params(&b, &cb).freewheel_rate_hz;
        assert!(ra > 0.0 && rb > 0.0 && (ra - rb).abs() > 1.0, "{ra} {rb}");
    }

    #[test]
    fn coasting_and_pedalling_from_the_snapshot() {
        let c = Character::of(RiderId(3));
        // Power, no cadence in the snapshot: pedalling at their cadence.
        let p = rider(3, 5.0, 0.0, 32.0, 220.0);
        assert!(p.pedalling(&c) && !p.coasting(&c));
        let pp = other_params(&p, &c);
        assert_eq!(pp.freewheel_gain, 0.0);
        assert!(pp.chain_gain > 0.0);
        assert!(
            (f64::from(pp.chain_rate_hz) - c.cadence_rpm / 60.0 * CHAINRING_TEETH).abs() < 1e-3
        );
        // No power: coasting, the freehub ticks at their hub's rate.
        let q = OtherRider { power_w: 0.0, ..p };
        assert!(q.coasting(&c) && !q.pedalling(&c));
        let qp = other_params(&q, &c);
        assert_eq!(qp.chain_gain, 0.0);
        let rate = q.speed_mps / c.wheel_m * c.points;
        assert!((f64::from(qp.freewheel_rate_hz) - rate).abs() < 1e-3);
        assert!(qp.freewheel_gain > 0.0);
        // A trickle of power (≤ 10 W) is coasting too; stopped is silent.
        assert!(OtherRider { power_w: 8.0, ..p }.coasting(&c));
        let stopped = OtherRider {
            power_w: 0.0,
            speed_mps: 0.0,
            ..p
        };
        assert!(!stopped.coasting(&c));
        assert_eq!(other_params(&stopped, &c).freewheel_gain, 0.0);
        // A cadence in the snapshot (later protocols) wins: legs spinning
        // slower than the wheel without power let it tick.
        let spin = OtherRider {
            power_w: 0.0,
            cadence_rpm: Some(30.0),
            speed_mps: 15.0,
            ..p
        };
        assert!(spin.coasting(&c) && spin.pedalling(&c));
        // Never the wind: that is the listener's own air.
        assert_eq!(pp.wind_gain, 0.0);
        assert_eq!(qp.wind_gain, 0.0);
    }

    #[test]
    fn the_nearest_get_voices_the_rest_the_group() {
        let l = listener();
        // 20 riders 3, 6, 9 … 60 m ahead, two out of range.
        let mut riders: Vec<OtherRider> = (1..=20)
            .map(|i| rider(i, f64::from(i) * 3.0, 0.0, 30.0, 200.0))
            .collect();
        riders.push(rider(98, 90.0, 0.0, 30.0, 200.0));
        riders.push(rider(99, -200.0, 0.0, 30.0, 200.0));
        let (full, group) = select(&l, &riders, &[], FULL_VOICES);
        let ids: Vec<u32> = full.iter().map(|(r, _)| r.id.0).collect();
        assert_eq!(ids, (1..=8).collect::<Vec<_>>());
        let gids: Vec<u32> = group.iter().map(|(r, _)| r.id.0).collect();
        assert_eq!(gids, (9..=20).collect::<Vec<_>>(), "out of range: nowhere");
        // Hysteresis: rider 9 playing stays in against rider 8 a little
        // nearer …
        let (full, _) = select(&l, &riders, &[RiderId(9)], FULL_VOICES);
        assert!(full.iter().any(|(r, _)| r.id.0 == 9));
        assert!(!full.iter().any(|(r, _)| r.id.0 == 8));
        // … but not against a rider much nearer.
        let (full, _) = select(&l, &riders, &[RiderId(20)], FULL_VOICES);
        assert!(!full.iter().any(|(r, _)| r.id.0 == 20));
        // The frame: 8 voices, the group with 12 riders, all pedalling (no
        // ticks).
        let f = frame(&l, &riders, &[]);
        assert_eq!(f.voices.len(), FULL_VOICES);
        assert_eq!(f.group.riders, 12);
        assert!(f.group.ticks.is_empty());
        assert!(f.group.chain_left > 0.0 && f.group.roll_left > 0.0);
        // The group layer is capped too.
        let crowd: Vec<OtherRider> = (1..=80)
            .map(|i| rider(i, 2.0 + f64::from(i) * 0.5, 1.0, 30.0, 0.0))
            .collect();
        let f = frame(&l, &crowd, &[]);
        assert_eq!(f.group.riders, MAX_GROUP);
        assert_eq!(f.group.ticks.len(), MAX_GROUP, "all coasting");
        // Nobody else: empty.
        assert_eq!(frame(&l, &[], &[]), OthersFrame::default().with_defaults());
    }

    impl OthersFrame {
        /// An empty frame as [`frame`] makes it.
        fn with_defaults(mut self) -> Self {
            self.group.roll_cutoff_hz = 300.0;
            self.group.air_cutoff_hz = 4000.0;
            self
        }
    }

    #[test]
    fn group_layer_is_placed_by_its_riders() {
        let l = listener();
        // Beyond the 8 nearest, a bunch on the right, coasting.
        let mut riders: Vec<OtherRider> = (1..=8)
            .map(|i| rider(i, f64::from(i), 0.0, 30.0, 200.0))
            .collect();
        riders.extend((20..30).map(|i| rider(i, 3.0, 12.0 + f64::from(i) * 0.2, 30.0, 0.0)));
        let f = frame(&l, &riders, &[]);
        let g = &f.group;
        assert_eq!(g.riders, 10);
        assert!(
            g.roll_right > 2.0 * g.roll_left,
            "{} {}",
            g.roll_right,
            g.roll_left
        );
        assert_eq!(g.ticks.len(), 10);
        assert!(g.ticks.iter().all(|t| t.right > t.left && t.gain > 0.0));
    }

    #[test]
    fn rider_positions_come_from_the_track() {
        // A straight route along +x.
        let route = Route::from_points(
            "flat",
            "Flat",
            (0..=10).map(|i| [f64::from(i) * 100.0, 0.0, 0.0]).collect(),
            false,
            6.0,
        )
        .unwrap();
        let st = |d: f64, lat: f64, v: f64| RiderState {
            distance: d,
            speed: v,
            lateral_offset: lat,
            heading: 0.0,
            turn_curvature: 0.0,
        };
        let me = listener_on(&route, &st(100.0, 0.0, 10.0));
        assert!((me.pos[0] - 100.0).abs() < 1e-6 && (me.pos[1] - HEAD_HEIGHT_M).abs() < 1e-6);
        assert!((me.forward[0] - 1.0).abs() < 1e-6);
        // A rider 1.5 m to the right, 4 m behind and faster: right and
        // behind, coming closer (a little higher).
        let (pos, vel) = on_track(&route, &st(96.0, 1.5, 14.0), SOURCE_HEIGHT_M);
        assert!((pos[2] - 1.5).abs() < 1e-6, "right is +z here");
        let p = place(&me, pos, vel);
        assert!(p.pan > 0.3 && p.right > p.left);
        assert!(p.doppler > 1.0);
        // Steering left turns the velocity.
        let (_, vel) = on_track(
            &route,
            &RiderState {
                heading: -0.2,
                ..st(96.0, 1.5, 10.0)
            },
            SOURCE_HEIGHT_M,
        );
        assert!(vel[2] < 0.0 && vel[0] > 9.0);
    }

    fn listener_on(route: &Route, s: &RiderState) -> Listener {
        super::listener(route, s)
    }

    const SR: f32 = 48_000.0;
    const BLOCK: usize = 32;

    fn render(s: &mut OthersSynth, seconds: f32) -> Vec<(f32, f32)> {
        (0..(seconds * SR) as usize)
            .map(|i| {
                if i % BLOCK == 0 {
                    s.block(BLOCK);
                }
                s.sample()
            })
            .collect()
    }

    fn rms(x: &[(f32, f32)], ch: usize) -> f32 {
        let v = |p: &(f32, f32)| if ch == 0 { p.0 } else { p.1 };
        (x.iter().map(|p| v(p) * v(p)).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_rider_on_the_right_is_heard_on_the_right() {
        let mut s = OthersSynth::new(SR);
        s.apply(frame(&listener(), &[rider(5, 0.5, 2.5, 30.0, 0.0)], &[]));
        let x = render(&mut s, 0.6);
        let (l, r) = (rms(&x[9600..], 0), rms(&x[9600..], 1));
        assert!(r > 0.003, "{r}");
        assert!(r > 4.0 * l, "{l} {r}");
        // Gone: fades out and frees the voice.
        s.apply(frame(&listener(), &[], &[]));
        let x = render(&mut s, 0.6);
        assert_eq!(s.voices(), 0);
        assert!(
            x[x.len() - 4800..]
                .iter()
                .all(|&(l, r)| l == 0.0 && r == 0.0)
        );
    }

    #[test]
    fn far_riders_are_quieter_and_beyond_range_silent() {
        let level = |x: f64| {
            let mut s = OthersSynth::new(SR);
            s.apply(frame(&listener(), &[rider(5, x, 0.0, 30.0, 250.0)], &[]));
            let out = render(&mut s, 0.5);
            rms(&out[4800..], 0)
        };
        let (near, mid, far, gone) = (level(3.0), level(15.0), level(50.0), level(80.0));
        assert!(near > 3.0 * mid, "{near} {mid}");
        assert!(mid > 3.0 * far, "{mid} {far}");
        assert_eq!(gone, 0.0);
    }

    /// 30 riders in a bunch around the listener, half coasting: the sum is
    /// a group, louder than one rider but far from clipping (the crowd
    /// normaliser), with voices capped.
    #[test]
    fn thirty_riders_do_not_clip() {
        let l = listener();
        let bunch: Vec<OtherRider> = (1..=30)
            .map(|i| {
                let row = f64::from((i - 1) / 3);
                let col = f64::from((i - 1) % 3) - 1.0;
                rider(
                    i,
                    1.0 + row * 1.8 - 9.0,
                    col * 1.2,
                    34.0,
                    if i % 2 == 0 { 0.0 } else { 300.0 },
                )
            })
            .collect();
        let f = frame(&l, &bunch, &[]);
        assert_eq!(f.voices.len(), FULL_VOICES);
        assert_eq!(f.group.riders, 30 - FULL_VOICES);
        let mut s = OthersSynth::new(SR);
        s.apply(f);
        let x = render(&mut s, 1.0);
        let peak = x
            .iter()
            .map(|p| p.0.abs().max(p.1.abs()))
            .fold(0.0, f32::max);
        // At full volume and with the own bike's make-up gain (1.6) still
        // under the limiter's knee (0.8).
        let mut one = OthersSynth::new(SR);
        one.apply(frame(&l, &bunch[..1], &[]));
        let y = render(&mut one, 1.0);
        let one_peak = y
            .iter()
            .map(|p| p.0.abs().max(p.1.abs()))
            .fold(0.0, f32::max);
        assert!(peak * 1.6 < 0.8, "peak {peak}, one rider {one_peak}");
        assert!(rms(&x[9600..], 0) > 1.5 * rms(&y[9600..], 0));
        assert!(s.voices() <= MAX_LIVE_VOICES);
    }

    #[test]
    fn voices_follow_the_frames() {
        let l = listener();
        let field: Vec<OtherRider> = (1..=12)
            .map(|i| rider(i, f64::from(i) * 2.0, 0.0, 30.0, 200.0))
            .collect();
        let mut s = OthersSynth::new(SR);
        s.apply(frame(&l, &field, &[]));
        assert_eq!(s.voices(), FULL_VOICES);
        render(&mut s, 0.1);
        // The far half of the field only: the nearest four fade out while
        // new ones fade in, never more than the cap.
        s.apply(frame(&l, &field[4..], &[]));
        assert!(s.voices() <= MAX_LIVE_VOICES);
        render(&mut s, 1.0);
        assert_eq!(s.voices(), FULL_VOICES);
    }
}
