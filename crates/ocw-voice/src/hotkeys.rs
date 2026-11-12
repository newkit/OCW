//! The mute hotkeys (spec §2.4), independent of any input library: the
//! app translates its key and button events into [`Input`] and runs the
//! console command of the resulting [`Toggle`].

/// The two mute switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Mic,
    Speaker,
}

impl Toggle {
    /// The console command that flips it (menu and scripts use the same).
    pub fn command(self) -> &'static str {
        match self {
            Self::Mic => "voice.mute-mic",
            Self::Speaker => "voice.mute-speaker",
        }
    }
}

/// An input event the hotkeys care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The `M` key went down, with Shift held or not.
    KeyM { shift: bool },
    /// The gamepad's Select button (Back / View / Share).
    PadSelect,
    /// The right-stick click (R3).
    PadRightStick,
}

/// Keyboard `M` = microphone, `Shift+M` = speaker; gamepad Select =
/// microphone, R3 = speaker.
pub fn toggle(input: Input) -> Toggle {
    match input {
        Input::KeyM { shift: false } | Input::PadSelect => Toggle::Mic,
        Input::KeyM { shift: true } | Input::PadRightStick => Toggle::Speaker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_and_gamepad_hotkeys() {
        assert_eq!(toggle(Input::KeyM { shift: false }), Toggle::Mic);
        assert_eq!(toggle(Input::KeyM { shift: true }), Toggle::Speaker);
        assert_eq!(toggle(Input::PadSelect), Toggle::Mic);
        assert_eq!(toggle(Input::PadRightStick), Toggle::Speaker);
        assert_eq!(Toggle::Mic.command(), "voice.mute-mic");
        assert_eq!(Toggle::Speaker.command(), "voice.mute-speaker");
    }
}
