//! Voice chat for Open Cycling World (spec §7, §2.4).
//!
//! Voice is **always on, like a call**: on a shared ride the client joins
//! the voice room automatically, with the microphone muted (spec §7.1.7
//! Q7: M unmutes) and heard only by whom *Who hears me* allows; the rider
//! has two mute switches (own microphone, speaker = all incoming voices). Voice
//! goes to a self-hosted LiveKit SFU, never over the game connection, and
//! is never recorded or stored.
//!
//! This crate is engine-independent and mostly pure:
//! - [`mute`]: the two mute switches and their console arguments,
//! - [`hotkeys`]: M / Shift+M, gamepad Select / R3 → the switches,
//! - [`join`]: when to join and leave the voice room (shared vs. solo
//!   rides, retries),
//! - [`spatial`]: attenuation by distance on the road (to zero at 300 m)
//!   and the cap with talking riders first,
//! - [`device`]: microphone / speaker selection with fallback,
//! - [`mix`]: the voice mixer (per-voice gain and pan), resampling and the
//!   confirmation sounds,
//! - [`echo`]: the echo test (an Echo rider plays the own microphone
//!   back, delayed, from its place on the road),
//! - [`audience`]: *Who hears me* and *Voice room* (spec §7.1.3, §7.1.7
//!   Q8) and the game server's answer, whose voice may reach whom,
//! - [`riders`]: the rider's volume, Block and selection for single other riders,
//!   remembered by account or by name per server (spec §7.1.7 Q9),
//! - [`controller`]: ties them together and drives a [`backend::VoiceBackend`];
//! - [`web`]: the browser's backend (spec §14 Phase 10 B5): the same
//!   controller over the page's livekit-client (JS) shim.
//!
//! The real backend ([`livekit`], cargo feature `livekit`) connects with the
//! LiveKit Rust SDK and plays / captures with cpal. Without the feature,
//! [`backend::NullBackend`] reports voice as not built in, so the app builds
//! and runs everywhere.
//!
//! Platforms (spec §7, platform plan): everything outside [`livekit`] uses
//! no threads, clock, file system or audio device and builds for
//! `wasm32-unknown-unknown`; times are passed in. The backend trait only
//! queues commands and returns events already received, so a browser
//! backend (livekit-client JS, async) or a mobile one fits behind it; device
//! lists are whatever the backend last learnt (a browser enumerates them
//! asynchronously).

pub mod audience;
pub mod backend;
pub mod controller;
pub mod device;
pub mod echo;
pub mod hotkeys;
pub mod join;
#[cfg(feature = "livekit")]
pub mod livekit;
pub mod mix;
pub mod mute;
pub mod riders;
pub mod spatial;
pub mod web;

pub use backend::{BackendCommand, BackendEvent, Levels, NullBackend, VoiceBackend};
pub use controller::{EchoStatus, Participant, Status, VoiceController, VoiceSettings};
pub use mute::{MuteState, Switch};

/// The backend this build has: LiveKit with the `livekit` feature,
/// otherwise the null backend.
pub fn default_backend() -> Box<dyn VoiceBackend> {
    #[cfg(feature = "livekit")]
    {
        Box::new(livekit::LiveKitBackend::new())
    }
    #[cfg(not(feature = "livekit"))]
    {
        Box::new(NullBackend)
    }
}

/// Whether this build includes a real voice backend.
pub const BUILT_IN: bool = cfg!(feature = "livekit");
