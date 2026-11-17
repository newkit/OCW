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
//! - [`audio`]: what the game sounds hear of the ride (the mixer's input,
//!   spec §3.3), [`voice`]: the voice call's per-frame logic and HUD (spec
//!   §2.4, §7), [`settings`]: the settings registry (spec §3.2.1) – shared
//!   with the browser app since Phase 10 B5.
//! - [`menu`]: the in-game menu's model, pages and rows, [`riders`]: the
//!   rider profiles, [`recording`]: the ride's FIT recording – shared with
//!   the browser app since Phase 10 B6.
//! - [`history`]: the training history (spec §14 Phase 7): the rides,
//!   weekly and monthly totals, personal bests and the power curve;
//!   [`bindings`]: the rebindable keys and buttons (M9).
//! - [`strava`]: the Strava recording with S (spec §2.8.1).
//! - [`photographer`]: the ride photographer (spec §2.11): when and
//!   where a photo is taken, the person or drone and the flash.
//! - [`world_trust`]: following the host to a world signed by a user key
//!   (spec §4.7 *User keys*): load, ask (the trust dialog) or leave.
//!
//! No threads, file system or sockets: everything here also builds for
//! `wasm32-unknown-unknown` (spec §3.5). The apps add the Bevy glue.

pub mod account;
pub mod aids;
pub mod audio;
pub mod bindings;
pub mod camera_fx;
pub mod camera_path;
pub mod client;
pub mod collisions;
pub mod damage;
pub mod event;
pub mod god_state;
pub mod gps;
pub mod history;
pub mod host;
pub mod input;
pub mod join;
pub mod menu;
/// The bike stand's part tiles (spec §14 Phase 4, 4G *Tiles*).
pub mod part_tiles;
pub mod photographer;
pub mod quality;
pub mod recording;
pub mod rider_list;
pub mod riders;
pub mod settings;
pub mod sky;
pub mod strava;
pub mod trainer;
pub mod view;
pub mod view_distance;
pub mod voice;
pub mod weather;
pub mod workout;
pub mod world_trust;
