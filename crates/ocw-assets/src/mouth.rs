//! The mouth while a rider is heard speaking (spec 14 Phase 4, 4N *Face*,
//! Q24 (a)): the lips open a little in short pulses, a few times a second
//! with a little randomness so it reads as speech, easing in and out –
//! only while the voice call says the rider is speaking, never while the
//! rider's microphone is muted (the X over the mouth, spec 2.6). The
//! renderer moves the near model's lower lip ([`crate::rider::LIP_LOWER`])
//! by [`crate::rider::lower_lip_offset`] of [`MouthAnim::open`]; the far
//! model (`lod1`) has no lips and never moves. Pure and Bevy-free; looks
//! only.
//!
//! The call gives only *speaking or not* per participant (LiveKit's
//! active speakers), no level of the others' voices, so the pulses are
//! made up: syllables of [`SYLLABLE_S`] with a random height, now and then
//! a short pause, seeded per rider so a group does not move in step.

/// A syllable lasts between these (s): about 4–7 pulses a second.
pub const SYLLABLE_S: (f32, f32) = (0.14, 0.26);
/// Its height (share of the full opening).
pub const SYLLABLE_OPEN: (f32, f32) = (0.6, 1.0);
/// Share of syllables that are a short pause (the mouth nearly closed).
pub const PAUSE_SHARE: f32 = 0.15;
/// The lips follow the syllables with these time constants (s): opening
/// and closing, so a pulse eases instead of jumping.
pub const OPEN_TAU_S: f32 = 0.025;
pub const CLOSE_TAU_S: f32 = 0.05;
/// Speaking fades in and out over this time (s, a smoothstep) …
pub const FADE_S: f32 = 0.12;
/// … and stops this much faster when the X comes on (muted mid-word).
pub const MUTE_S: f32 = 0.04;
/// The share of a syllable at its end with the lips closed (between the
/// pulses).
pub const GAP_SHARE: f32 = 0.3;

/// One rider's mouth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouthAnim {
    /// How far the lips are open: 0 closed … 1 fully open.
    open: f32,
    /// The speech's ramp: 0 silent … 1 speaking (eased by a smoothstep).
    gain: f32,
    /// Time in the current syllable and its length and height.
    t: f32,
    len: f32,
    height: f32,
    rng: u32,
}

impl MouthAnim {
    /// A closed mouth whose pulses are seeded by `seed` (the rider id).
    pub fn new(seed: u32) -> Self {
        let mut m = Self {
            open: 0.0,
            gain: 0.0,
            t: 0.0,
            len: SYLLABLE_S.0,
            height: 0.0,
            rng: (seed.wrapping_mul(0x9E37_79B9) ^ 0x5bd1_e995) | 1,
        };
        m.next_syllable();
        m
    }

    /// How far the lips are open now (0 … 1).
    pub fn open(&self) -> f32 {
        self.open
    }

    /// The lips are (nearly) closed and still.
    pub fn closed(&self) -> bool {
        self.open < 1e-3 && self.gain < 1e-3
    }

    fn random(&mut self) -> f32 {
        // xorshift32: deterministic, the same on every client.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn next_syllable(&mut self) {
        self.t = 0.0;
        self.len = SYLLABLE_S.0 + (SYLLABLE_S.1 - SYLLABLE_S.0) * self.random();
        self.height = if self.random() < PAUSE_SHARE {
            0.0
        } else {
            SYLLABLE_OPEN.0 + (SYLLABLE_OPEN.1 - SYLLABLE_OPEN.0) * self.random()
        };
    }

    /// One frame of `dt` seconds: `speaking` from the voice call's speaking
    /// cue, `muted` the X over the mouth (as this viewer sees it). Returns
    /// the opening (0 … 1).
    pub fn update(&mut self, speaking: bool, muted: bool, dt: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.25);
        if muted {
            // Never while muted: closes at once-ish, and stays closed.
            self.gain = (self.gain - dt / MUTE_S).max(0.0);
            let k = 1.0 - (-dt / (0.5 * MUTE_S)).exp();
            self.open = if self.gain == 0.0 {
                0.0
            } else {
                (self.open - self.open * k).min(self.gain)
            };
            return self.open;
        }
        self.gain = if speaking {
            (self.gain + dt / FADE_S).min(1.0)
        } else {
            (self.gain - dt / FADE_S).max(0.0)
        };
        // The syllables run only while there is speech to show.
        let mut target = 0.0;
        if self.gain > 0.0 {
            self.t += dt;
            while self.t >= self.len {
                self.t -= self.len;
                let rest = self.t;
                self.next_syllable();
                self.t = rest.min(self.len);
            }
            let x = self.t / (self.len * (1.0 - GAP_SHARE));
            let pulse = if x < 1.0 {
                (std::f32::consts::PI * x).sin()
            } else {
                0.0
            };
            let g = self.gain * self.gain * (3.0 - 2.0 * self.gain);
            target = g * self.height * pulse;
        }
        let tau = if target > self.open {
            OPEN_TAU_S
        } else {
            CLOSE_TAU_S
        };
        self.open += (target - self.open) * (1.0 - (-dt / tau).exp());
        if self.gain == 0.0 && self.open < 2e-3 {
            self.open = 0.0;
        }
        self.open = self.open.clamp(0.0, 1.0);
        self.open
    }
}

/// Whether a rider's mouth moves (spec 14 Phase 4 Q24 (a)): heard speaking
/// in the call (`speaking`: the speaking cue, for the own rider only while
/// the own voice is on), not muted (`muted`: the X over the mouth), and
/// on the near model (`near`).
pub fn moves(speaking: bool, muted: bool, near: bool) -> bool {
    speaking && !muted && near
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn run(m: &mut MouthAnim, speaking: bool, muted: bool, s: f32) -> Vec<f32> {
        (0..(s / DT).round() as usize)
            .map(|_| m.update(speaking, muted, DT))
            .collect()
    }

    /// Peaks of the opening (local maxima above `min`).
    fn peaks(v: &[f32], min: f32) -> usize {
        v.windows(3)
            .filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > min)
            .count()
    }

    #[test]
    fn the_lips_pulse_only_while_speaking() {
        let mut m = MouthAnim::new(7);
        assert!(run(&mut m, false, false, 2.0).iter().all(|&o| o == 0.0));
        assert!(m.closed());
        let talk = run(&mut m, true, false, 3.0);
        let max = talk.iter().copied().fold(0.0, f32::max);
        assert!(max > 0.4 && max <= 1.0, "{max}");
        // A few pulses a second: 3 s of speech give 8 … 22 of them.
        let n = peaks(&talk, 0.15);
        assert!((8..=22).contains(&n), "{n} pulses");
        // Between the pulses the lips come nearly closed again.
        assert!(talk[30..].iter().filter(|&&o| o < 0.15).count() > 15);
        // Silence: the mouth closes and stays closed.
        let after = run(&mut m, false, false, 1.0);
        assert!(after[after.len() - 1] == 0.0 && m.closed());
    }

    #[test]
    fn the_lips_never_move_while_muted() {
        let mut m = MouthAnim::new(3);
        assert!(run(&mut m, true, true, 3.0).iter().all(|&o| o == 0.0));
        // Muted mid-word: closed within a tenth of a second (the X takes a
        // quarter of a second to scale in).
        run(&mut m, true, false, 1.0);
        let v = run(&mut m, true, true, 0.5);
        assert!(v[6..].iter().all(|&o| o < 0.05), "{v:?}");
        assert!(v[v.len() - 1] == 0.0);
    }

    #[test]
    fn the_lips_ease_in_and_out() {
        let mut m = MouthAnim::new(11);
        let v = run(&mut m, true, false, 1.0);
        // No jump from one frame to the next (the full opening takes a few
        // frames), and the first frame barely opens.
        assert!(v[0] < 0.1, "{}", v[0]);
        for w in v.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.35, "{w:?}");
        }
        // Speech fades out over a few frames, not at once.
        let open_before = m.open();
        let mut m2 = m;
        let out = run(&mut m2, false, false, 0.5);
        for w in out.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.35, "closing: {w:?}");
        }
        assert!(open_before == 0.0 || out[0] > 0.0, "eases out");
        // Closed within the fade and the lips' lag (well under 0.5 s).
        assert!(out[24..].iter().all(|&o| o == 0.0), "{out:?}");
        assert_eq!(out[out.len() - 1], 0.0);
    }

    #[test]
    fn riders_do_not_speak_in_step() {
        let a = run(&mut MouthAnim::new(1), true, false, 2.0);
        let b = run(&mut MouthAnim::new(2), true, false, 2.0);
        assert_ne!(a, b);
        // Deterministic per seed (the same on every client).
        assert_eq!(a, run(&mut MouthAnim::new(1), true, false, 2.0));
    }

    #[test]
    fn only_the_near_model_of_a_speaking_unmuted_rider_moves() {
        assert!(moves(true, false, true));
        assert!(!moves(true, true, true), "the X mouth");
        assert!(!moves(true, false, false), "lod1");
        assert!(!moves(false, false, true));
    }
}
