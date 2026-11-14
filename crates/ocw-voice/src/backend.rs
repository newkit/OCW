//! The voice backend interface (spec §7: "isolated … behind a trait"): the
//! controller sends commands and polls events; it never blocks. The real
//! backend is [`crate::livekit::LiveKitBackend`]; [`NullBackend`] stands in
//! when voice is not built in, [`MockBackend`] in tests.

use crate::echo::EchoParams;
use crate::mix::Cue;

#[derive(Debug, Clone, PartialEq)]
pub enum BackendCommand {
    /// Join the room the token is for.
    Connect {
        url: String,
        token: String,
    },
    Disconnect,
    /// Mute / unmute the published microphone track (not unpublished).
    SetMicMuted(bool),
    /// Silence all incoming voices (they stay subscribed).
    SetSpeakerMuted(bool),
    /// Open this microphone / speaker (`None`: the system default).
    SetInputDevice(Option<String>),
    SetOutputDevice(Option<String>),
    /// Receive a participant's voice or stop receiving it.
    Subscribe {
        identity: String,
        on: bool,
    },
    /// Play a participant's voice at this gain (0 …) and pan (−1 … 1).
    SetPlacement {
        identity: String,
        gain: f32,
        pan: f32,
    },
    /// A confirmation sound.
    PlayCue(Cue),
    /// The echo test's local loopback (spec §7 *Echo bot*): the processed
    /// microphone, delayed, played as the voice [`crate::echo::ECHO_IDENTITY`]
    /// (placed with `SetPlacement` like any voice). Active, it opens the
    /// audio devices even without a call.
    SetEcho(EchoParams),
}

/// Audio levels for the HUD (0 … 1).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    /// The microphone, while the echo test listens.
    pub input: f32,
    /// The echo as played.
    pub echo: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    Connected {
        room: String,
    },
    /// The connection dropped and the SDK tries again by itself.
    Reconnecting,
    Reconnected,
    /// Left the room (`reason` empty after a requested disconnect).
    Disconnected {
        reason: String,
    },
    ConnectFailed(String),
    ParticipantJoined {
        identity: String,
        name: String,
    },
    ParticipantLeft {
        identity: String,
    },
    /// A participant muted / unmuted their microphone.
    ParticipantMuted {
        identity: String,
        muted: bool,
    },
    /// Who is talking now (identities; the own one included).
    Speaking(Vec<String>),
    /// An audio device could not be opened or failed.
    DeviceError(String),
}

pub trait VoiceBackend: Send {
    /// `Err(why)` if this backend cannot connect at all.
    fn available(&self) -> Result<(), String>;
    /// The present microphones and speakers by name.
    fn input_devices(&mut self) -> Vec<String>;
    fn output_devices(&mut self) -> Vec<String>;
    fn send(&mut self, cmd: BackendCommand);
    fn poll(&mut self) -> Vec<BackendEvent>;
    /// The levels the backend last measured.
    fn levels(&self) -> Levels {
        Levels::default()
    }
}

/// Voice is not built into this app (cargo feature `voice` off).
#[derive(Debug, Default)]
pub struct NullBackend;

pub const NOT_BUILT_IN: &str = "voice chat is not built into this app (cargo feature \"voice\")";

impl VoiceBackend for NullBackend {
    fn available(&self) -> Result<(), String> {
        Err(NOT_BUILT_IN.into())
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, _cmd: BackendCommand) {}
    fn poll(&mut self) -> Vec<BackendEvent> {
        Vec::new()
    }
}

/// A scripted backend for tests: records commands, returns queued events.
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    pub sent: Vec<BackendCommand>,
    pub events: Vec<BackendEvent>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

impl VoiceBackend for MockBackend {
    fn available(&self) -> Result<(), String> {
        Ok(())
    }
    fn input_devices(&mut self) -> Vec<String> {
        self.inputs.clone()
    }
    fn output_devices(&mut self) -> Vec<String> {
        self.outputs.clone()
    }
    fn send(&mut self, cmd: BackendCommand) {
        self.sent.push(cmd);
    }
    fn poll(&mut self) -> Vec<BackendEvent> {
        std::mem::take(&mut self.events)
    }
}
