//! Open Cycling World's shared Bevy UI glue (spec §14 Phase 10, B6: UI
//! parity – one app, Q6 full parity). The desktop app (`apps/desktop`) and
//! the browser app (`apps/web`) add the same plugins:
//!
//! - [`menu`]: the in-game menu – keys, gamepad, mouse and touch → the
//!   Bevy-free model (`ocw_client::menu`) → `ocw_render::menu::MenuView`;
//!   menu functions are console lines,
//! - [`settings`]: the settings registry's values and their store (a file
//!   on the desktop, `localStorage` in the browser), `set` / `get`,
//! - [`timemachine`] and [`weather`]: the world clock and the session
//!   weather into the view and the menu, `time` / `weather …`;
//!   [`calendar`]: World → Time → Calendar and Season (spec §14 Phase 9
//!   S9),
//! - [`collisions`], [`events`]: the ride's rules and local events,
//! - [`riders_page`]: Navi → Riders in this ride,
//! - [`god`]: god mode – "Unicorn mode" (spec §14 Phase 13; shared since
//!   G5, v3.37): the flight's keys, gamepad, mouse and touch, the pointer
//!   capture, inspection and pins, the HUD's flight view, `god …`,
//! - [`controls`]: the Bevy input of the menu, [`bindings`]: the
//!   rebindable ride keys and buttons (M9), [`access`]: the accessibility
//!   options, [`history`]: the training history (spec §14 Phase 7),
//! - [`trust`]: the trust dialog and the trusted world keys (spec §4.7
//!   *User keys*), `world trust …`,
//! - [`touch`]: touch steering in a ride (spec §3.2.3: the screen's
//!   halves, the junction fan's arrows, two fingers for the camera),
//! - [`join`]: Ride → Join – the link field, the recent servers, `join`,
//!   `leave` (spec §3.2.1, §6.2 *Join links*).
//!
//! The apps differ in how they hold the ride: the desktop has
//! `ClientState` and a `Link` to its server, the browser a `WebSession`.
//! [`RideAccess`] is the one seam: how the shared code reaches the
//! [`ClientCore`] and sends messages to the server.

pub mod access;
pub mod air;
#[cfg(not(target_arch = "wasm32"))]
pub mod app_icon;
pub mod bindings;
pub mod calendar;
pub mod character;
pub mod collisions;
pub mod controls;
pub mod events;
pub mod god;
pub mod history;
pub mod join;
pub mod key_entry;
pub mod menu;
pub mod photographer;
pub mod profiles;
pub mod quality;
pub mod riders_page;
pub mod settings;
pub mod status;
pub mod timemachine;
pub mod touch;
pub mod traffic;
pub mod trust;
pub mod view_distance;
pub mod weather;
// The workshop scene and the garage (spec 14 Phase 4; both apps since
// 4O part 3).
pub mod workshop;
#[cfg(not(target_arch = "wasm32"))]
pub mod worlds;

use bevy::prelude::*;
use ocw_client::client::ClientCore;
pub use ocw_client::menu::model::Platform;

/// The app's client in its world, if a ride runs.
pub type ClientFn = for<'w> fn(&'w World) -> Option<&'w ClientCore>;
/// The same, mutable.
pub type ClientMutFn = for<'w> fn(&'w mut World) -> Option<Mut<'w, ClientCore>>;

/// How the shared plugins reach the app's ride client.
#[derive(Resource, Clone, Copy)]
pub struct RideAccess {
    /// The client, if a ride runs.
    pub client: ClientFn,
    pub client_mut: ClientMutFn,
    /// Sends an encoded client message to the server (local or remote).
    pub send: fn(&mut World, Vec<u8>),
}

/// The app's ride client, if a ride runs (and the app inserted
/// [`RideAccess`]).
pub fn client(world: &World) -> Option<&ClientCore> {
    let access = world.get_resource::<RideAccess>()?;
    (access.client)(world)
}

/// The app's ride client, mutable.
pub fn client_mut(world: &mut World) -> Option<Mut<'_, ClientCore>> {
    let access = *world.get_resource::<RideAccess>()?;
    (access.client_mut)(world)
}

/// Whether a ride runs (a client the shared code can reach).
pub fn riding(world: &World) -> bool {
    client(world).is_some()
}

/// Sends an encoded client message to the server; `false` without a ride.
pub fn send(world: &mut World, bytes: Vec<u8>) -> bool {
    let Some(access) = world.get_resource::<RideAccess>().copied() else {
        return false;
    };
    if (access.client)(world).is_none() {
        return false;
    }
    (access.send)(world, bytes);
    true
}

/// Runs `f` on the client and sends the message it returns.
pub fn with_client<R>(
    world: &mut World,
    f: impl FnOnce(&mut ClientCore) -> Result<(R, Option<Vec<u8>>), String>,
) -> Result<R, String> {
    let (out, msg) = {
        let mut c = client_mut(world).ok_or("not riding")?;
        f(&mut c)?
    };
    if let Some(m) = msg {
        send(world, m);
    }
    Ok(out)
}

/// Which app runs (the menu's items, the settings that apply).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiPlatform(pub Platform);

/// The app's system that writes the frame's [`ocw_render::RideView`] from
/// the client runs in this set; the shared systems that add to the view
/// (sky, weather) and read it (the menu) run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RideViewSet;

/// The shared systems that complete the view (after [`RideViewSet`]).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ViewExtrasSet;

/// Orders the sets (every shared plugin calls it; idempotent).
pub(crate) fn configure_sets(app: &mut App) {
    app.configure_sets(Update, ViewExtrasSet.after(RideViewSet));
}

/// The game time now (s).
pub(crate) fn now(world: &World) -> f64 {
    world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64())
}
