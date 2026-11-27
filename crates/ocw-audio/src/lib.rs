//! Game audio of Open Cycling World (spec §3.3, menu increment M5 in
//! §3.2.1), shared by the desktop and the browser app (spec §14 Phase 10
//! B5: one client, full parity).
//!
//! - [`mix`]: the buses (master × music / effects / voice / others) and the
//!   volume maths;
//! - [`ride`]: speed / cadence / power / surface → ride sound parameters
//!   and their synthesis (wind, tyres, freewheel, chain);
//! - [`spatial`]: where a sound is heard (distance, direction, air,
//!   Doppler);
//! - [`others`]: the other riders' bikes – character per rider, nearest
//!   voices and the group layer, and their synthesis;
//! - [`weather`]: rain, hail, drizzle, gusts, wet tyres and thunder (spec
//!   §14 Phase 9, S4);
//! - [`aircraft`]: the simulated aircraft – jets, propellers, rotors, a
//!   glider's whistle, a balloon's burner (spec 14 Phase 18, T8);
//! - [`cues`]: one-shot sounds (UI, countdown, finish, workout steps, gear
//!   shift, crashes and bumps, the buckled wheel's rub, test tones) and when
//!   they play;
//! - [`mixer`]: renders all of it into an interleaved buffer of any
//!   channel count;
//! - [`schedule`] and [`unlock`]: the browser's output (spec §14 Phase 10
//!   B5) – how far ahead the mixer renders into Web Audio buffers, and the
//!   autoplay rule (audio starts on the first user gesture).
//!
//! Every sound is synthesised in code – our own work, no sample files and
//! no third-party recordings (spec §13). Pure: no threads, devices, files
//! or clocks (the cue timestamps use `web_time::Instant`, std's on native),
//! so it builds for `wasm32-unknown-unknown`. The outputs are the apps':
//! cpal on its own thread on the desktop, Web Audio buffers scheduled from
//! the frame loop in the browser.

pub mod aircraft;
pub mod cues;
#[cfg(test)]
mod demo;
pub mod drone;
pub mod dsp;
pub mod mix;
pub mod mixer;
pub mod others;
pub mod ride;
pub mod schedule;
pub mod spatial;
pub mod unlock;
pub mod weather;
