//! The mix (spec §3.3): master × music / effects / voice, and the other
//! riders' bikes as a part of the effects (master × effects × others).
//! Volumes are percent (0–100) as in the settings; the gain of a bus is
//! the product of the master's and the bus's gain, both on a squared
//! (perceptual) curve, and 0 when muted. Pure and unit-tested.

/// An audio bus. Every sound plays on exactly one; the master scales them
/// all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Background music (none yet; the bus and its volume exist).
    Music,
    /// Game sounds: bike, wind, tyres, UI, event and workout cues.
    Effects,
    /// Voice chat (Phase 6, spec §7) and the voice test tone.
    Voice,
    /// The other riders' bikes (bots, remote riders): a part of the
    /// effects, so its gain is master × effects × others.
    Others,
}

impl Bus {
    pub const ALL: [Bus; 4] = [Bus::Music, Bus::Effects, Bus::Voice, Bus::Others];

    pub fn name(self) -> &'static str {
        match self {
            Bus::Music => "music",
            Bus::Effects => "effects",
            Bus::Voice => "voice",
            Bus::Others => "others",
        }
    }

    pub fn parse(s: &str) -> Option<Bus> {
        Bus::ALL
            .into_iter()
            .find(|b| b.name().eq_ignore_ascii_case(s))
    }

    fn index(self) -> usize {
        match self {
            Bus::Music => 0,
            Bus::Effects => 1,
            Bus::Voice => 2,
            Bus::Others => 3,
        }
    }
}

/// The user's mix: volumes in percent and the mute switch
/// (`audio.master`, `audio.music`, `audio.effects`, `audio.voice`,
/// `audio.others`, `audio.mute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mix {
    pub master: u8,
    pub music: u8,
    pub effects: u8,
    pub voice: u8,
    pub others: u8,
    pub mute: bool,
}

impl Default for Mix {
    /// The registry defaults (`settings/registry.rs`; a test checks they
    /// match).
    fn default() -> Self {
        Self {
            master: 80,
            music: 60,
            effects: 80,
            voice: 100,
            others: 80,
            mute: false,
        }
    }
}

/// Linear gain of a volume in percent: (v / 100)², so 50 % is about
/// -12 dB (with a linear fader the top half seems to do nothing). Values
/// above 100 count as 100.
pub fn volume_gain(percent: u8) -> f32 {
    let v = f32::from(percent.min(100)) / 100.0;
    v * v
}

impl Mix {
    /// The volume of `bus` in percent.
    pub fn volume(&self, bus: Bus) -> u8 {
        match bus {
            Bus::Music => self.music,
            Bus::Effects => self.effects,
            Bus::Voice => self.voice,
            Bus::Others => self.others,
        }
    }

    pub fn set_volume(&mut self, bus: Bus, percent: u8) {
        let p = percent.min(100);
        match bus {
            Bus::Music => self.music = p,
            Bus::Effects => self.effects = p,
            Bus::Voice => self.voice = p,
            Bus::Others => self.others = p,
        }
    }

    /// The gain of each bus: master × bus (× effects for the others), 0
    /// when muted.
    pub fn gains(&self) -> BusGains {
        let mut g = [0.0; 4];
        if !self.mute {
            let master = volume_gain(self.master);
            for b in Bus::ALL {
                g[b.index()] = master * volume_gain(self.volume(b));
            }
            g[Bus::Others.index()] *= volume_gain(self.effects);
        }
        BusGains(g)
    }
}

/// Linear gain per bus, as the mixer applies it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BusGains([f32; 4]);

impl BusGains {
    pub fn get(&self, bus: Bus) -> f32 {
        self.0[bus.index()]
    }

    /// Moves each gain the fraction `a` (0–1) towards `target` (the
    /// mixer's smoothing, so a volume change does not click).
    pub fn approach(&mut self, target: &BusGains, a: f32) {
        for (g, t) in self.0.iter_mut().zip(target.0) {
            *g += (t - *g) * a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve_is_squared_and_clamped() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(100), 1.0);
        assert_eq!(volume_gain(50), 0.25);
        assert_eq!(volume_gain(255), 1.0);
        let mut last = -1.0;
        for p in 0..=100 {
            let g = volume_gain(p);
            assert!(g > last, "rises strictly at {p}");
            last = g;
        }
    }

    #[test]
    fn bus_gain_is_master_times_bus() {
        let mix = Mix {
            master: 50,
            music: 100,
            effects: 50,
            voice: 0,
            others: 50,
            mute: false,
        };
        let g = mix.gains();
        assert_eq!(g.get(Bus::Music), 0.25);
        assert_eq!(g.get(Bus::Effects), 0.25 * 0.25);
        assert_eq!(g.get(Bus::Voice), 0.0);
        // The other riders are a part of the effects.
        assert_eq!(g.get(Bus::Others), 0.25 * 0.25 * 0.25);
        assert_eq!(Mix { effects: 0, ..mix }.gains().get(Bus::Others), 0.0);
        assert_eq!(
            Mix { others: 0, ..mix }.gains().get(Bus::Effects),
            0.25 * 0.25
        );
        // Commutes: master 100 × bus 50 is bus 100 × master 50.
        let swapped = Mix {
            master: 100,
            music: 50,
            ..mix
        };
        assert_eq!(swapped.gains().get(Bus::Music), g.get(Bus::Music));
        // Full volume is unity gain; master 0 silences every bus.
        let full = Mix {
            master: 100,
            music: 100,
            effects: 100,
            voice: 100,
            others: 100,
            mute: false,
        };
        assert!(Bus::ALL.iter().all(|&b| full.gains().get(b) == 1.0));
        let silent = Mix { master: 0, ..full };
        assert!(Bus::ALL.iter().all(|&b| silent.gains().get(b) == 0.0));
    }

    #[test]
    fn mute_silences_all_buses_and_keeps_the_volumes() {
        let mut mix = Mix {
            mute: true,
            ..Mix::default()
        };
        assert_eq!(mix.gains(), BusGains::default());
        assert_eq!(mix.volume(Bus::Effects), 80);
        mix.mute = false;
        assert!(mix.gains().get(Bus::Effects) > 0.0);
    }

    #[test]
    fn buses_parse_and_set() {
        for b in Bus::ALL {
            assert_eq!(Bus::parse(b.name()), Some(b));
        }
        assert_eq!(Bus::parse("EFFECTS"), Some(Bus::Effects));
        assert_eq!(Bus::parse("master"), None);
        let mut mix = Mix::default();
        mix.set_volume(Bus::Voice, 140);
        assert_eq!(mix.voice, 100);
        mix.set_volume(Bus::Music, 30);
        assert_eq!(mix.volume(Bus::Music), 30);
        assert_eq!(Bus::parse("Others"), Some(Bus::Others));
        mix.set_volume(Bus::Others, 45);
        assert_eq!(mix.others, 45);
    }

    #[test]
    fn gains_approach_the_target_smoothly() {
        let target = Mix {
            master: 100,
            effects: 100,
            ..Mix::default()
        }
        .gains();
        let mut g = BusGains::default();
        g.approach(&target, 0.5);
        assert_eq!(g.get(Bus::Effects), 0.5);
        for _ in 0..40 {
            g.approach(&target, 0.5);
        }
        assert!((g.get(Bus::Effects) - 1.0).abs() < 1e-6);
    }
}
