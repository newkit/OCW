//! The rider profiles and the rider of the running ride, shared by both
//! apps since the workshop is (spec 14 Phase 4, 4O part 3): the desktop
//! keeps them in `ocw.toml [riders]`, the browser only the riding rider
//! (the anonymous default, or the signed-in rider; W5b brings the
//! account's).

use std::sync::Arc;

use bevy::prelude::*;
use ocw_client::riders::{Rider, Riders};

/// Where the profiles are kept (the desktop: a section of `ocw.toml`).
pub trait ProfilesStore: Send + Sync {
    /// Writes the profiles' TOML.
    fn write(&self, text: &str) -> Result<(), String>;
    /// Where, for the rider ("~/.config/ocw/ocw.toml [riders]").
    fn describe(&self) -> String;
}

/// The rider profiles, and the rider of the running ride.
#[derive(Resource, Default)]
pub struct RiderProfiles {
    pub riders: Riders,
    /// Where they are kept (`ocw.toml [riders]`); `None`: not kept.
    pub path: Option<Arc<dyn ProfilesStore>>,
    /// Who rides now: the active profile at the start, or `--rider`.
    /// Changes to the profiles apply from the next start.
    pub riding: Rider,
}

impl RiderProfiles {
    /// Writes the profiles where they are kept.
    pub fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => p.write(&self.riders.to_toml()),
            None => Err("no configuration directory".into()),
        }
    }
}

/// The physics profile and display name of the local rider: from
/// [`RiderProfiles::riding`], or the default rider.
pub fn local_rider(world: &World) -> Rider {
    world
        .get_resource::<RiderProfiles>()
        .map(|r| r.riding.clone())
        .unwrap_or_default()
}
