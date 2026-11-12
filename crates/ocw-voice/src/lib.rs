//! Voice chat for Open Cycling World (spec §7, §2.4).
//!
//! Voice is **always on, like a call**: on a shared ride the client joins
//! the voice room automatically with an open microphone; the rider has two
//! mute switches (own microphone, speaker = all incoming voices). Voice
//! goes to a self-hosted LiveKit SFU, never over the game connection, and
//! is never recorded or stored.
//!
//! This crate is engine-independent and mostly pure:
//! - [`mute`]: the two mute switches and their console arguments,
//! - [`hotkeys`]: M / Shift+M, gamepad Select / R3 → the switches,
//! - [`join`]: when to join and leave the voice room (shared vs. solo
//!   rides, retries),
//! - [`spatial`]: attenuation by distance on the road and nearest-N
//!   selection,
//! - [`device`]: microphone / speaker selection with fallback,
//! - [`mix`]: the voice mixer (per-voice gain and pan), resampling and the
//!   confirmation sounds,
//! - [`controller`]: ties them together and drives a [`backend::VoiceBackend`].
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

pub mod backend;
pub mod controller;
pub mod device;
pub mod hotkeys;
pub mod join;
#[cfg(feature = "livekit")]
pub mod livekit;
pub mod mix;
pub mod mute;
pub mod spatial;

pub use backend::{BackendCommand, BackendEvent, NullBackend, VoiceBackend};
pub use controller::{Participant, Status, VoiceController, VoiceSettings};
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
