//! Open Cycling World client logic, shared by the desktop app
//! (`apps/desktop`) and the browser app (`apps/web`, spec §14 Phase 10).
//!
//! - [`client::ClientCore`]: the client loop (device samples → server,
//!   server messages → prediction and interpolation → render view),
//! - [`collisions`]: crashes, bumps and the collision rules (spec §14 Phase 12),
//! - [`event`]: events, standings and ghosts on the client,
//! - [`god_state`]: god mode's flight state (spec §14 Phase 13),
//! - [`input`]: the pure key and gamepad logic of the ride controls,
//! - [`gps`] and [`sky`]: the rider's position and the sky for the HUD,
//! - [`weather`]: the session's weather, strikes and gusts (spec §14 Phase 9, S4).
//!
//! No threads, file system or sockets: everything here also builds for
//! `wasm32-unknown-unknown` (spec §3.5). The apps add the Bevy glue.

pub mod client;
pub mod collisions;
pub mod event;
pub mod god_state;
pub mod gps;
pub mod input;
pub mod sky;
pub mod weather;
