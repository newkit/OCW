//! What the workshop needs from its app (spec 14 Phase 4, 4O part 3: the
//! same workshop in both apps, B6): how the ride runs (a local server, a
//! capture, the kiosk's show), when the ride session started, and the
//! account – the desktop's `ocw-account` thread or the browser's
//! `fetch()` calls – that keeps the signed-in rider's workshop. The ride's
//! client and server link come through [`crate::RideAccess`].

use std::path::PathBuf;

use bevy::prelude::*;
use ocw_account_api::offline::Synced;
use ocw_account_api::workshop::{Action, ActionAnswer};

/// An action's answer for the frame loop.
#[derive(Debug, Clone, PartialEq)]
pub struct ActAnswer {
    pub id: u64,
    pub result: Result<ActionAnswer, String>,
}

/// What changed in the account's workshop since the app last looked.
#[derive(Debug, Default)]
pub struct WorkshopUpdate {
    /// Signed in, with the account's workshop known (from the backend or
    /// the cache).
    pub signed_in: bool,
    /// The workshop, when it changed since `generation`.
    pub synced: Option<Synced>,
    pub online: bool,
    /// Ride summaries waiting for the backend.
    pub queued: usize,
    pub generation: u64,
    /// The actions' answers (taken).
    pub answers: Vec<ActAnswer>,
}

/// The app's side of the workshop (a resource the app inserts before
/// [`super::WorkshopPlugin`]; without it: a local ride without an account).
#[derive(Resource, Clone, Copy)]
pub struct WorkshopHost {
    /// The ride runs on a local server (hearts are booked by the app; on
    /// a remote one the server's crediting counts, 4E).
    pub local_server: bool,
    /// A scripted capture: nothing is booked or kept.
    pub capture: bool,
    /// `--no-hearts` (the kiosk's show).
    pub no_hearts: bool,
    /// Customising needs an account (the browser, Phase 10 Q5): signed
    /// out, the workshop, the character and the garage are locked.
    pub requires_account: bool,
    /// Where the local wallet lives (`<data dir>/ocw`), `None`: in memory.
    pub wallet_dir: fn() -> Option<PathBuf>,
    /// When the ride session started (game time, s); a new session counts
    /// the server's hearts from zero.
    pub session_start: fn(&World) -> f64,
    /// Signed in to an account (whatever its workshop).
    pub signed_in: fn(&World) -> bool,
    /// The account's workshop since a generation.
    pub workshop_update: fn(&World, u64) -> WorkshopUpdate,
    /// Sends an action to the account (answered later); `false`: no
    /// account.
    pub send_action: fn(&World, u64, Action) -> bool,
    /// Fetches the account's workshop again.
    pub sync_workshop: fn(&World),
}

fn no_dir() -> Option<PathBuf> {
    None
}

impl Default for WorkshopHost {
    fn default() -> Self {
        Self {
            local_server: true,
            capture: false,
            no_hearts: false,
            requires_account: false,
            wallet_dir: no_dir,
            session_start: |_| 0.0,
            signed_in: |_| false,
            workshop_update: |_, _| WorkshopUpdate::default(),
            send_action: |_, _, _| false,
            sync_workshop: |_| {},
        }
    }
}

/// The app's host, or the default.
pub fn host(world: &World) -> WorkshopHost {
    world
        .get_resource::<WorkshopHost>()
        .copied()
        .unwrap_or_default()
}
