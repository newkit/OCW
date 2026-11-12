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
    /// A test tone on a bus (`audio.play test-music` …).
    Test(Bus),
}

impl Cue {
    /// The names `audio.play` takes.
    pub const NAMES: [(&'static str, Cue); 13] = [
        ("move", Cue::UiMove),
        ("value", Cue::UiValue),
        ("confirm", Cue::UiConfirm),
        ("back", Cue::UiBack),
        ("beep", Cue::CountdownBeep),
        ("go", Cue::CountdownGo),
        ("finish", Cue::Finish),
        ("step", Cue::WorkoutStep),
        ("shift", Cue::GearShift),
        ("test-music", Cue::Test(Bus::Music)),
        ("test-effects", Cue::Test(Bus::Effects)),
        ("test-voice", Cue::Test(Bus::Voice)),
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
            filter: Svf::new(2600.0, 2.0, sample_rate),
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
            Cue::Test(Bus::Music) => tone(t, 0.0, 0.8, 440.0, 0.3) + tone(t, 0.0, 0.8, 554.4, 0.2),
            Cue::Test(Bus::Effects) => tone(t, 0.0, 0.8, 660.0, 0.35),
            Cue::Test(Bus::Voice) => {
                tone(t, 0.0, 0.8, 330.0, 0.35) + tone(t, 0.0, 0.8, 990.0, 0.08)
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

/// The workout cue: the session's change counter (starts, step changes,
/// end, stop; `WorkoutSession::lap_count`) grew.
pub fn workout_cue(prev_count: u64, now_count: u64) -> Option<Cue> {
    (now_count > prev_count).then_some(Cue::WorkoutStep)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

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
