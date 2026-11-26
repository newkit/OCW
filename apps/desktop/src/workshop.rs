//! The workshop (spec 14 Phase 4): the scene, the garage and the account's
//! workshop are shared with the browser app (`ocw_ui::workshop`, 4O part
//! 3); this is the desktop's side of it – the ride's flags from the
//! command line, the wallet's folder, the session's start and the
//! account's `ocw-account` thread.

pub use ocw_ui::workshop::*;

use bevy::prelude::*;

use crate::Args;
use crate::account::Account;
use crate::connection::SessionStart;

/// The shared workshop with the desktop's [`WorkshopHost`].
pub struct WorkshopPlugin;

impl Plugin for WorkshopPlugin {
    fn build(&self, app: &mut App) {
        let h = host_of(app.world().get_resource::<Args>());
        app.insert_resource(h)
            .add_plugins(ocw_ui::workshop::WorkshopPlugin);
    }
}

fn wallet_dir() -> Option<std::path::PathBuf> {
    // Tests keep the wallet in memory.
    if cfg!(test) {
        None
    } else {
        wallet::default_dir()
    }
}

/// The desktop's host: `--server` (a remote server credits itself),
/// `--capture`, `--no-hearts`, the data folder and the account thread.
pub fn host_of(args: Option<&Args>) -> WorkshopHost {
    WorkshopHost {
        local_server: args.is_none_or(|a| a.server.is_none()),
        capture: args.is_some_and(|a| a.capture.is_some()),
        no_hearts: args.is_some_and(|a| a.no_hearts),
        requires_account: false,
        wallet_dir,
        session_start: |w| w.get_resource::<SessionStart>().map_or(0.0, |s| s.0),
        signed_in: |w| w.get_resource::<Account>().is_some_and(Account::signed_in),
        workshop_update: |w, seen| {
            w.get_resource::<Account>()
                .map(|a| a.workshop_update(seen))
                .unwrap_or_default()
        },
        send_action: |w, id, action| {
            w.get_resource::<Account>()
                .is_some_and(|a| a.send_action(id, action))
        },
        sync_workshop: |w| {
            if let Some(a) = w.get_resource::<Account>() {
                a.sync_workshop();
            }
        },
    }
}

#[cfg(test)]
#[path = "workshop_tests.rs"]
mod tests;
