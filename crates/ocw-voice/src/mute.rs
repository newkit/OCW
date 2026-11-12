//! The two mute switches of always-on voice (spec §2.4): the own
//! microphone and the speaker (all incoming voices; game sounds have their
//! own mute). Both are remembered in `settings.toml`; on the first start
//! both are on (not muted).

/// What a toggle command asks for: `on` / `off` / toggle. For the mute
/// commands, `on` means *muted* (`voice.mute-mic on` mutes the microphone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
    Toggle,
}

impl Switch {
    /// Parses a console argument; none = toggle.
    pub fn parse(arg: Option<&str>) -> Result<Self, String> {
        match arg.map(|a| a.trim().to_ascii_lowercase()) {
            None => Ok(Self::Toggle),
            Some(a) => match a.as_str() {
                "on" | "true" | "yes" | "1" => Ok(Self::On),
                "off" | "false" | "no" | "0" => Ok(Self::Off),
                "toggle" | "" => Ok(Self::Toggle),
                _ => Err(format!("expected on, off or toggle, not {a:?}")),
            },
        }
    }

    /// The new value of a switch that is `current` now.
    pub fn apply(self, current: bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Toggle => !current,
        }
    }
}

/// The rider's mute switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MuteState {
    /// Nobody hears the rider: the microphone track is muted (not
    /// unpublished), so unmuting is instant.
    pub mic_muted: bool,
    /// The rider hears no voices; the microphone keeps its state.
    pub speaker_muted: bool,
}

/// Which switch changed, for the confirmation sound and the HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteChange {
    Mic { muted: bool },
    Speaker { muted: bool },
}

impl MuteState {
    /// Sets the microphone switch; the change, if there was one.
    pub fn set_mic(&mut self, s: Switch) -> Option<MuteChange> {
        let new = s.apply(self.mic_muted);
        (new != self.mic_muted).then(|| {
            self.mic_muted = new;
            MuteChange::Mic { muted: new }
        })
    }

    pub fn set_speaker(&mut self, s: Switch) -> Option<MuteChange> {
        let new = s.apply(self.speaker_muted);
        (new != self.speaker_muted).then(|| {
            self.speaker_muted = new;
            MuteChange::Speaker { muted: new }
        })
    }

    /// Whether the microphone is sent: connected and not muted.
    pub fn mic_live(&self, connected: bool) -> bool {
        connected && !self.mic_muted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_arguments() {
        assert_eq!(Switch::parse(None), Ok(Switch::Toggle));
        assert_eq!(Switch::parse(Some("ON")), Ok(Switch::On));
        assert_eq!(Switch::parse(Some("off")), Ok(Switch::Off));
        assert_eq!(Switch::parse(Some("toggle")), Ok(Switch::Toggle));
        assert!(Switch::parse(Some("maybe")).is_err());
        assert!(Switch::On.apply(false) && Switch::On.apply(true));
        assert!(!Switch::Off.apply(true));
        assert!(Switch::Toggle.apply(false) && !Switch::Toggle.apply(true));
    }

    #[test]
    fn first_start_is_unmuted_and_switches_are_independent() {
        let mut m = MuteState::default();
        assert!(!m.mic_muted && !m.speaker_muted);
        assert!(m.mic_live(true));
        assert!(!m.mic_live(false), "not sent while not connected");
        assert_eq!(
            m.set_mic(Switch::Toggle),
            Some(MuteChange::Mic { muted: true })
        );
        assert!(!m.mic_live(true));
        // Muting the speaker leaves the microphone as it is.
        assert_eq!(
            m.set_speaker(Switch::On),
            Some(MuteChange::Speaker { muted: true })
        );
        assert!(m.mic_muted);
        assert_eq!(
            m.set_speaker(Switch::Toggle),
            Some(MuteChange::Speaker { muted: false })
        );
        assert!(m.mic_muted);
        // Setting the current value is no change (no sound).
        assert_eq!(m.set_mic(Switch::On), None);
        assert_eq!(m.set_speaker(Switch::Off), None);
        assert_eq!(
            m.set_mic(Switch::Off),
            Some(MuteChange::Mic { muted: false })
        );
    }
}
