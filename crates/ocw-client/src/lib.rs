//! Open Cycling World client logic, shared by the desktop app
//! (`apps/desktop`) and the browser app (`apps/web`, spec §14 Phase 10).
//!
//! - [`client::ClientCore`]: the client loop (device samples → server,
//!   server messages → prediction and interpolation → render view),
//! - [`collisions`]: crashes, bumps and the collision rules (spec §14 Phase 12),
//!   [`aids`]: the "last moment" warning and the proximity arrow (12D),
//!   [`camera_fx`]: the chase camera's shake and wider orbit after a crash (12C),
//! - [`camera_path`]: guided camera paths for flyover videos (Phase 13 G5),
//! - [`rider_list`]: the session's riders for the menu (who flies, damage dots),
//! - [`damage`]: the riders' damage, the HUD widget and the carried condition (12E, 12F),
//! - [`event`]: events, standings and ghosts on the client,
//! - [`god_state`]: god mode's flight state (spec §14 Phase 13),
//! - [`input`]: the pure key and gamepad logic of the ride controls,
//! - [`gps`] and [`sky`]: the rider's position and the sky for the HUD,
//! - [`trainer`]: SIM / ERG trainer control (spec §5.3) and [`workout`]:
//!   structured workouts driving ERG (spec §2.8).
//! - [`weather`]: the session's weather, strikes and gusts (spec §14 Phase 9, S4).
//!
//! No threads, file system or sockets: everything here also builds for
//! `wasm32-unknown-unknown` (spec §3.5). The apps add the Bevy glue.

pub mod aids;
pub mod camera_fx;
pub mod camera_path;
pub mod client;
pub mod collisions;
pub mod damage;
pub mod event;
pub mod god_state;
pub mod gps;
pub mod input;
pub mod rider_list;
pub mod sky;
pub mod trainer;
pub mod weather;
pub mod workout;
