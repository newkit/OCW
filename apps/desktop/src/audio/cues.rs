//! Short one-shot sounds (spec §3.3): menu navigation, the event
//! countdown and finish, workout step changes, a gear-shift click and test
//! tones – all synthesised here (our own work, no samples). The functions
//! that decide *when* a cue plays (from the menu view, the event countdown,
//! the workout) are pure and unit-tested.

use std::f32::consts::PI;

use super::dsp::{Noise, Svf};
use super::mix::Bus;

/// A one-shot sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cue {
    /// Menu focus moved.
    UiMove,
    /// A value changed (← / → on a row).
    UiValue,
    /// A page opened (or the menu).
    UiConfirm,
    /// One page back (or the menu closed).
    UiBack,
    /// Countdown "3", "2", "1".
    CountdownBeep,
    /// Countdown "GO!".
    CountdownGo,
    /// Crossed the finish (race, time trial): a rising chime.
    Finish,
    /// A workout step began.
    WorkoutStep,
    /// A gear change: a mechanical double click.
    GearShift,
    /// God mode (spec §14 Phase 13C): the transformation's chime, into the
    /// unicorn and back.
    GodChime,
    /// A soft wing beat (a downstroke of the unicorn's wings).
    WingBeat,
    /// A muffled hoof beat on air (the unicorn's gallop while slow).
    Hoof,
    /// A soft bump (spec §14 Phase 12C): a body thud and a tyre squeak.
    Bump,
    /// A crash: the impact, the metal scrape of the sliding bike, a
    /// fading spin of the free wheel.
    Crash,
    /// Back on the bike: the cleats click in.
    Remount,
    /// The proximity cue (12D): a soft bell.
    Proximity,
    /// A test tone on a bus (`audio.play test-music` …).
    Test(Bus),
}

impl Cue {
    /// The names `audio.play` takes.
    pub const NAMES: [(&'static str, Cue); 21] = [
        ("move", Cue::UiMove),
        ("value", Cue::UiValue),
        ("confirm", Cue::UiConfirm),
        ("back", Cue::UiBack),
        ("beep", Cue::CountdownBeep),
        ("go", Cue::CountdownGo),
        ("finish", Cue::Finish),
        ("step", Cue::WorkoutStep),
        ("shift", Cue::GearShift),
        ("chime", Cue::GodChime),
        ("wing", Cue::WingBeat),
        ("hoof", Cue::Hoof),
        ("bump", Cue::Bump),
        ("crash", Cue::Crash),
        ("remount", Cue::Remount),
        ("proximity", Cue::Proximity),
        ("test-music", Cue::Test(Bus::Music)),
        ("test-effects", Cue::Test(Bus::Effects)),
        ("test-voice", Cue::Test(Bus::Voice)),
        ("test-others", Cue::Test(Bus::Others)),
        ("test", Cue::Test(Bus::Effects)),
    ];

    pub fn parse(s: &str) -> Option<Cue> {
        Self::NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(s))
            .map(|(_, c)| *c)
    }

    pub fn bus(self) -> Bus {
        match self {
            Cue::Test(b) => b,
            _ => Bus::Effects,
        }
    }

    /// Length in seconds.
    pub fn duration_s(self) -> f32 {
        match self {
            Cue::UiMove | Cue::UiValue => 0.04,
            Cue::UiConfirm | Cue::UiBack => 0.12,
            Cue::CountdownBeep => 0.2,
            Cue::CountdownGo => 0.6,
            Cue::Finish => 1.4,
            Cue::WorkoutStep => 0.32,
            Cue::GearShift => 0.09,
            Cue::GodChime => 1.3,
            Cue::WingBeat => 0.38,
            Cue::Hoof => 0.14,
            Cue::Bump => 0.35,
            Cue::Crash => 1.6,
            Cue::Remount => 0.3,
            Cue::Proximity => 0.5,
            Cue::Test(_) => 0.8,
        }
    }
}

/// A sine tone with a short attack and an exponential decay, starting at
/// `start` s and lasting `len` s; `t` is the cue's time.
fn tone(t: f32, start: f32, len: f32, freq: f32, amp: f32) -> f32 {
    let x = t - start;
    if !(0.0..len).contains(&x) {
        return 0.0;
    }
    let attack = (x / 0.004).min(1.0);
    let release = ((len - x) / 0.01).min(1.0);
    let decay = (-x * 3.0 / len).exp();
    amp * attack * release * decay * (2.0 * PI * freq * x).sin()
}

/// A bell-like note: a sine with a soft inharmonic partial.
fn bell(t: f32, start: f32, len: f32, freq: f32, amp: f32) -> f32 {
    tone(t, start, len, freq, amp) + tone(t, start, len * 0.5, freq * 2.76, amp * 0.25)
}

/// A playing cue.
#[derive(Debug, Clone)]
pub struct CueVoice {
    pub cue: Cue,
    /// Samples played so far.
    pos: usize,
    noise: Noise,
    filter: Svf,
}

impl CueVoice {
    pub fn new(cue: Cue, sample_rate: f32) -> Self {
        Self {
            cue,
            pos: 0,
            noise: Noise::new(0xC0E5 + cue.duration_s().to_bits()),
            filter: match cue {
                Cue::WingBeat => Svf::new(380.0, 0.8, sample_rate),
                Cue::Hoof | Cue::Bump => Svf::new(240.0, 0.9, sample_rate),
                Cue::Crash => Svf::new(3200.0, 1.5, sample_rate),
                _ => Svf::new(2600.0, 2.0, sample_rate),
            },
        }
    }

    pub fn done(&self, sample_rate: f32) -> bool {
        self.pos as f32 >= self.cue.duration_s() * sample_rate
    }

    /// The next sample (mono, within about ±0.6).
    pub fn sample(&mut self, sample_rate: f32) -> f32 {
        let t = self.pos as f32 / sample_rate;
        self.pos += 1;
        match self.cue {
            Cue::UiMove => tone(t, 0.0, 0.04, 1500.0, 0.18),
            Cue::UiValue => tone(t, 0.0, 0.04, 2100.0, 0.16),
            Cue::UiConfirm => tone(t, 0.0, 0.06, 880.0, 0.2) + tone(t, 0.05, 0.07, 1320.0, 0.2),
            Cue::UiBack => tone(t, 0.0, 0.06, 990.0, 0.18) + tone(t, 0.05, 0.07, 660.0, 0.18),
            Cue::CountdownBeep => tone(t, 0.0, 0.2, 880.0, 0.4) + tone(t, 0.0, 0.2, 1760.0, 0.08),
            Cue::CountdownGo => tone(t, 0.0, 0.6, 1760.0, 0.4) + tone(t, 0.0, 0.6, 880.0, 0.15),
            // C6 E6 G6 C7, overlapping.
            Cue::Finish => {
                bell(t, 0.0, 0.9, 1046.5, 0.25)
                    + bell(t, 0.13, 0.9, 1318.5, 0.25)
                    + bell(t, 0.26, 0.9, 1568.0, 0.25)
                    + bell(t, 0.39, 1.0, 2093.0, 0.3)
            }
            Cue::WorkoutStep => bell(t, 0.0, 0.16, 660.0, 0.3) + bell(t, 0.15, 0.17, 990.0, 0.3),
            Cue::GearShift => {
                // Two clicks of filtered noise and a low clunk.
                let n = self.noise.white();
                let band = self.filter.process(n).band;
                let click = |at: f32| {
                    let x = t - at;
                    if x >= 0.0 { (-x / 0.003).exp() } else { 0.0 }
                };
                band * 1.2 * (click(0.0) + 0.8 * click(0.035)) + tone(t, 0.035, 0.05, 180.0, 0.25)
            }
            // A sparkling run up (G6 C7 E7 G7 C8) over a soft C6.
            Cue::GodChime => {
                bell(t, 0.0, 1.2, 1046.5, 0.12)
                    + bell(t, 0.0, 0.5, 1568.0, 0.16)
                    + bell(t, 0.08, 0.5, 2093.0, 0.16)
                    + bell(t, 0.16, 0.6, 2637.0, 0.15)
                    + bell(t, 0.24, 0.7, 3136.0, 0.14)
                    + bell(t, 0.32, 0.9, 4186.0, 0.13)
            }
            // Air pushed down: low-passed noise swelling and fading.
            Cue::WingBeat => {
                let n = self.noise.white();
                let low = self.filter.process(n).low;
                let env = (t / 0.12).min(1.0) * (-(t - 0.12).max(0.0) / 0.08).exp();
                low * 0.5 * env
            }
            // A soft, low thud through cloth: a 75 Hz knock dying away and
            // a little low-passed noise (muffled: nothing above ~250 Hz).
            Cue::Hoof => {
                let n = self.noise.white();
                let low = self.filter.process(n).low;
                let env = (t / 0.004).min(1.0) * (-t / 0.03).exp();
                tone(t, 0.0, 0.14, 75.0, 0.32) + low * 0.6 * env
            }
            // A body thud (low knock and muffled noise) and a short tyre
            // squeak gliding down.
            Cue::Bump => {
                let n = self.noise.white();
                let low = self.filter.process(n).low;
                let env = (t / 0.003).min(1.0) * (-t / 0.05).exp();
                let squeak = if (0.04..0.2).contains(&t) {
                    let x = t - 0.04;
                    let f = 2400.0 - 2500.0 * x;
                    0.08 * (-x / 0.06).exp() * (2.0 * PI * f * x).sin()
                } else {
                    0.0
                };
                tone(t, 0.0, 0.25, 85.0, 0.45) + low * 0.7 * env + squeak
            }
            // The impact, then the scrape of metal on asphalt fading, and
            // the free wheel ticking slower and slower.
            Cue::Crash => {
                let n = self.noise.white();
                let band = self.filter.process(n).band;
                let impact = (t / 0.002).min(1.0) * (-t / 0.08).exp();
                let scrape = if t > 0.08 {
                    let x = t - 0.08;
                    (x / 0.05).min(1.0)
                        * (-x / 0.45).exp()
                        * (1.0 + 0.4 * (2.0 * PI * 23.0 * t).sin())
                } else {
                    0.0
                };
                let rate = 14.0 * (-t / 0.9).exp() + 2.0;
                let x = (t * rate).fract() / rate;
                let tick = if t > 0.3 {
                    (-x / 0.0015).exp() * 0.25
                } else {
                    0.0
                };
                tone(t, 0.0, 0.3, 60.0, 0.5) + n * 0.5 * impact + band * (0.35 * scrape + tick)
            }
            // Two cleats clicking in.
            Cue::Remount => {
                let n = self.noise.white();
                let band = self.filter.process(n).band;
                let click = |at: f32| {
                    let x = t - at;
                    if x >= 0.0 { (-x / 0.004).exp() } else { 0.0 }
                };
                band * 1.1 * (click(0.0) + click(0.16)) + tone(t, 0.0, 0.04, 1800.0, 0.06)
            }
            Cue::Proximity => bell(t, 0.0, 0.5, 1318.5, 0.12),
            Cue::Test(Bus::Music) => tone(t, 0.0, 0.8, 440.0, 0.3) + tone(t, 0.0, 0.8, 554.4, 0.2),
            Cue::Test(Bus::Effects) => tone(t, 0.0, 0.8, 660.0, 0.35),
            Cue::Test(Bus::Voice) => {
                tone(t, 0.0, 0.8, 330.0, 0.35) + tone(t, 0.0, 0.8, 990.0, 0.08)
            }
            Cue::Test(Bus::Others) => {
                // A freehub ticking 14 times a second (another rider
                // coasting), with a soft tone under it.
                let n = self.noise.white();
                let band = self.filter.process(n).band;
                let x = (t * 14.0).fract() / 14.0;
                band * 0.9 * (-x / 0.0015).exp() + tone(t, 0.0, 0.8, 880.0, 0.12)
            }
        }
    }
}

/// What the menu showed in a frame, for [`menu_cue`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuSnapshot {
    pub open: bool,
    pub title: String,
    /// The page stack depth is not in the view; the title tells pages
    /// apart well enough.
    pub focused: usize,
    /// The focused item's label (a value change shows in it).
    pub label: String,
}

/// The UI cue for the menu's change from `prev` to `now`, if any:
/// opening / a new page = confirm, closing = back, a moved focus = move,
/// a changed value on the same row = value.
pub fn menu_cue(prev: &MenuSnapshot, now: &MenuSnapshot) -> Option<Cue> {
    match (prev.open, now.open) {
        (false, false) => None,
        (false, true) => Some(Cue::UiConfirm),
        (true, false) => Some(Cue::UiBack),
        (true, true) if prev.title != now.title => Some(Cue::UiConfirm),
        (true, true) if prev.focused != now.focused => Some(Cue::UiMove),
        (true, true) if prev.label != now.label => Some(Cue::UiValue),
        _ => None,
    }
}

/// The cue for the event countdown's change from `prev` to `now` (the
/// HUD's big text: "Get ready", "3", "2", "1", "GO!").
pub fn countdown_cue(prev: Option<&str>, now: Option<&str>) -> Option<Cue> {
    if prev == now {
        return None;
    }
    match now? {
        "3" | "2" | "1" => Some(Cue::CountdownBeep),
        "GO!" => Some(Cue::CountdownGo),
        _ => None,
    }
}

/// The finish chime: the results panel came up.
pub fn finish_cue(prev_results: bool, now_results: bool) -> Option<Cue> {
    (!prev_results && now_results).then_some(Cue::Finish)
}

/// What the contact cues watch between frames (spec §14 Phase 12C).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContactWatch {
    wobbling: std::collections::BTreeSet<u32>,
    down: std::collections::BTreeMap<u32, bool>,
    near: bool,
}

/// A rider as the contact cues see it: id, the local rider, metres from
/// the local rider, the wobble (rad), the crash phase (remounting?).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactRider {
    pub id: u32,
    pub is_local: bool,
    pub distance_m: f64,
    pub wobble: f64,
    pub crash: Option<bool>,
}

/// Bumps within this distance of the local rider are heard (m) …
pub const BUMP_HEARD_M: f64 = 15.0;
/// … and crashes within this one.
pub const CRASH_HEARD_M: f64 = 40.0;

/// The contact cues for this frame (spec §14 Phase 12C): a bump when a
/// rider starts to wobble, a crash when one goes down (the own rider, or
/// others close by), the cleats when the own rider remounts, the bell
/// when someone comes close (the proximity cue, 12D).
pub fn contact_cues(
    watch: &mut ContactWatch,
    riders: &[ContactRider],
    proximity: Option<i32>,
) -> Vec<Cue> {
    let mut out = Vec::new();
    let mut wobbling = std::collections::BTreeSet::new();
    let mut down = std::collections::BTreeMap::new();
    for r in riders {
        let heard = |m: f64| r.is_local || r.distance_m.abs() < m;
        if r.wobble != 0.0 {
            wobbling.insert(r.id);
            if !watch.wobbling.contains(&r.id) && heard(BUMP_HEARD_M) {
                out.push(Cue::Bump);
            }
        }
        if let Some(remounting) = r.crash {
            down.insert(r.id, remounting);
            match watch.down.get(&r.id) {
                None if !remounting && heard(CRASH_HEARD_M) => out.push(Cue::Crash),
                Some(false) if remounting && r.is_local => out.push(Cue::Remount),
                _ => {}
            }
        }
    }
    let near = proximity.is_some();
    if near && !watch.near {
        out.push(Cue::Proximity);
    }
    out.dedup();
    *watch = ContactWatch {
        wobbling,
        down,
        near,
    };
    out
}

/// The workout cue: the session's change counter (starts, step changes,
/// end, stop; `WorkoutSession::lap_count`) grew.
pub fn workout_cue(prev_count: u64, now_count: u64) -> Option<Cue> {
    (now_count > prev_count).then_some(Cue::WorkoutStep)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn contact_cues_bump_crash_remount_and_proximity() {
        let mut w = ContactWatch::default();
        let r = |id, is_local, distance_m, wobble, crash| ContactRider {
            id,
            is_local,
            distance_m,
            wobble,
            crash,
        };
        assert!(contact_cues(&mut w, &[r(1, true, 0.0, 0.0, None)], None).is_empty());
        // A bump of the own rider; a far one is not heard.
        assert_eq!(
            contact_cues(
                &mut w,
                &[r(1, true, 0.0, 0.05, None), r(2, false, 90.0, 0.05, None)],
                None
            ),
            vec![Cue::Bump]
        );
        assert!(
            contact_cues(&mut w, &[r(1, true, 0.0, 0.03, None)], None).is_empty(),
            "once"
        );
        // A crash close by, then the own rider remounting.
        assert_eq!(
            contact_cues(
                &mut w,
                &[
                    r(1, true, 0.0, 0.0, Some(false)),
                    r(3, false, 10.0, 0.0, Some(false))
                ],
                None
            ),
            vec![Cue::Crash]
        );
        assert_eq!(
            contact_cues(&mut w, &[r(1, true, 0.0, 0.0, Some(true))], None),
            vec![Cue::Remount]
        );
        assert_eq!(contact_cues(&mut w, &[], Some(1)), vec![Cue::Proximity]);
        assert!(contact_cues(&mut w, &[], Some(-1)).is_empty());
    }

    fn snap(open: bool, title: &str, focused: usize, label: &str) -> MenuSnapshot {
        MenuSnapshot {
            open,
            title: title.into(),
            focused,
            label: label.into(),
        }
    }

    #[test]
    fn menu_changes_pick_the_ui_cue() {
        let closed = MenuSnapshot::default();
        let top = snap(true, "Open Cycling World", 0, "Resume");
        assert_eq!(menu_cue(&closed, &closed), None);
        assert_eq!(menu_cue(&closed, &top), Some(Cue::UiConfirm));
        assert_eq!(menu_cue(&top, &closed), Some(Cue::UiBack));
        assert_eq!(menu_cue(&top, &top), None, "nothing changed");
        let moved = snap(true, "Open Cycling World", 1, "Ride");
        assert_eq!(menu_cue(&top, &moved), Some(Cue::UiMove));
        let page = snap(true, "Ride", 0, "Race");
        assert_eq!(menu_cue(&moved, &page), Some(Cue::UiConfirm));
        let a = snap(true, "Audio", 0, "Master volume:  < 80 >");
        let b = snap(true, "Audio", 0, "Master volume:  < 85 >");
        assert_eq!(menu_cue(&a, &b), Some(Cue::UiValue));
    }

    #[test]
    fn countdown_beeps_once_per_number_and_go() {
        let seq = [
            None,
            Some("Get ready"),
            Some("Get ready"),
            Some("3"),
            Some("3"),
            Some("2"),
            Some("1"),
            Some("GO!"),
            Some("GO!"),
            None,
        ];
        let cues: Vec<Cue> = seq
            .windows(2)
            .filter_map(|w| countdown_cue(w[0], w[1]))
            .collect();
        assert_eq!(
            cues,
            [
                Cue::CountdownBeep,
                Cue::CountdownBeep,
                Cue::CountdownBeep,
                Cue::CountdownGo
            ]
        );
    }

    #[test]
    fn finish_and_workout_cues_fire_on_the_change() {
        assert_eq!(finish_cue(false, true), Some(Cue::Finish));
        assert_eq!(finish_cue(true, true), None);
        assert_eq!(finish_cue(true, false), None);
        assert_eq!(workout_cue(2, 3), Some(Cue::WorkoutStep));
        assert_eq!(workout_cue(3, 3), None);
        // A new session (counter reset) is not a step.
        assert_eq!(workout_cue(5, 0), None);
    }

    #[test]
    fn cues_parse_and_route_to_their_bus() {
        for (name, cue) in Cue::NAMES {
            assert_eq!(Cue::parse(name), Some(cue));
        }
        assert_eq!(Cue::parse("nope"), None);
        assert_eq!(Cue::parse("test-voice").unwrap().bus(), Bus::Voice);
        assert_eq!(Cue::parse("test-others").unwrap().bus(), Bus::Others);
        assert_eq!(Cue::Finish.bus(), Bus::Effects);
    }

    /// Every cue makes a sound, stays in range and ends.
    #[test]
    fn every_cue_sounds_and_ends() {
        for (name, cue) in Cue::NAMES {
            let mut v = CueVoice::new(cue, SR);
            let mut peak = 0.0f32;
            let mut n = 0;
            while !v.done(SR) {
                let s = v.sample(SR);
                assert!(s.is_finite());
                peak = peak.max(s.abs());
                n += 1;
            }
            assert!(peak > 0.05 && peak < 1.0, "{name}: peak {peak}");
            assert_eq!(n, (cue.duration_s() * SR).ceil() as usize, "{name}");
        }
    }
}
