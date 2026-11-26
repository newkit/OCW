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

/// The account's rider (spec 14 Phase 4, 4O W5b): how the app reads and
/// writes it – the desktop's account thread, the browser's `fetch()`.
/// Signed in, the account's rider is the riding rider (name, weight,
/// height, FTP, bike profile, character and kit look); every change of it
/// goes back to the account; signed out, the local rider again.
#[derive(Resource, Clone, Copy)]
pub struct RiderAccount {
    /// The account's rider as last fetched, with a generation that changes
    /// with every answer; `None`: signed out (or not fetched yet), `Some((g,
    /// None))`: the account has no rider yet.
    pub fetched: fn(&World) -> FetchedRider,
    /// Sends the rider to the account (answered by a new generation).
    pub put: fn(&World, RiderDoc),
}

use ocw_account_api::rider::RiderDoc;

/// The account's rider as fetched, with its generation ([`RiderAccount`]).
pub type FetchedRider = Option<(u64, Option<RiderDoc>)>;

/// What [`follow_rider`] remembers.
#[derive(Resource, Default)]
pub struct RiderSync {
    seen: u64,
    /// The local riding rider, set aside while the account's rides.
    local: Option<Rider>,
    /// The rider as the account last had it (JSON).
    kept: Option<serde_json::Value>,
    /// A change waits this long (s) before it is sent (typing a name, a
    /// slider).
    due_in: Option<f64>,
}

/// How long a change of the rider waits before it goes to the account.
pub const RIDER_PUT_DELAY_S: f64 = 2.0;

fn doc_of(r: &Rider) -> Option<RiderDoc> {
    Some(RiderDoc {
        name: r.name.clone(),
        rider: serde_json::to_value(r).ok()?,
        updated_at: 0,
    })
}

/// Follows the account's rider (spec 14 Phase 4, 4O W5b): takes it as the
/// riding rider when it arrives, sends the riding rider when the account
/// has none yet or when it changed, and brings the local rider back when
/// signed out.
pub fn follow_rider(world: &mut World) {
    let Some(acc) = world.get_resource::<RiderAccount>().copied() else {
        return;
    };
    let dt = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.delta_secs_f64());
    let fetched = (acc.fetched)(world);
    let riding = local_rider(world);
    let mut put = None;
    let mut take = None;
    {
        let mut sync = world.get_resource_or_insert_with(RiderSync::default);
        match fetched {
            None => {
                // Signed out: the local rider again.
                if let Some(local) = sync.local.take() {
                    take = Some(local);
                }
                sync.seen = 0;
                sync.kept = None;
                sync.due_in = None;
            }
            Some((g, doc)) if g != sync.seen => {
                sync.seen = g;
                match doc.and_then(|d| serde_json::from_value::<Rider>(d.rider).ok()) {
                    Some(r) => {
                        if sync.local.is_none() {
                            sync.local = Some(riding.clone());
                        }
                        // As this app writes it (keys of a newer app are
                        // not ours to compare).
                        sync.kept = serde_json::to_value(&r).ok();
                        sync.due_in = None;
                        if r != riding {
                            take = Some(r);
                        }
                    }
                    // None yet: the rider ridden now becomes the account's
                    // – once (a refused one is not offered again until it
                    // changes), and not the default "Rider" (a reserved
                    // name: the rider names themself first, 4N).
                    None => {
                        if sync.local.is_none() {
                            sync.local = Some(riding.clone());
                        }
                        let now = serde_json::to_value(&riding).ok();
                        if sync.kept != now {
                            sync.kept = now;
                            if ocw_core::name::check(&riding.name).is_ok() {
                                put = doc_of(&riding);
                            }
                        }
                    }
                }
            }
            Some(_) => {
                // Signed in: a change of the riding rider goes up after a
                // moment (a valid name only).
                let now = serde_json::to_value(&riding).ok();
                if sync.kept.is_some()
                    && now != sync.kept
                    && ocw_core::name::check(&riding.name).is_ok()
                {
                    let left = sync.due_in.get_or_insert(RIDER_PUT_DELAY_S);
                    *left -= dt;
                    if *left <= 0.0 {
                        sync.due_in = None;
                        sync.kept = now;
                        put = doc_of(&riding);
                    }
                }
            }
        }
    }
    if let Some(r) = take
        && let Some(mut p) = world.get_resource_mut::<RiderProfiles>()
    {
        info!(rider = %r.name, "the riding rider: the account's");
        p.riding = r;
        // The ride shows the look (and the bike) at once.
        if let Some(mut c) = world.get_resource_mut::<ocw_render::console::ConsoleState>() {
            c.queue("workshop.apply");
        }
    }
    if let Some(d) = put {
        (acc.put)(world, d);
    }
}

/// Adds [`follow_rider`] (an app with a [`RiderAccount`]).
pub struct RiderAccountPlugin;

impl Plugin for RiderAccountPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderSync>()
            .add_systems(PreUpdate, follow_rider);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;

    static FETCHED: Mutex<FetchedRider> = Mutex::new(None);
    static PUT: Mutex<Vec<RiderDoc>> = Mutex::new(Vec::new());

    fn rider(name: &str, kg: f64) -> Rider {
        Rider {
            name: name.into(),
            weight_kg: kg,
            ..Rider::default()
        }
    }

    /// The account's rider (spec 14 Phase 4, 4O W5b): an account without
    /// one gets the local rider; the account's arrives as the riding
    /// rider; a change goes back after a moment; signed out, the local
    /// rider again.
    #[test]
    fn the_account_rider_rides_when_signed_in() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                500,
            )))
            .insert_resource(RiderProfiles {
                riding: rider("Kai Local", 80.0),
                ..RiderProfiles::default()
            })
            .insert_resource(RiderAccount {
                fetched: |_| FETCHED.lock().unwrap().clone(),
                put: |_, d| PUT.lock().unwrap().push(d),
            })
            .add_plugins(RiderAccountPlugin);
        let riding = |app: &App| app.world().resource::<RiderProfiles>().riding.clone();
        app.update();
        // The reserved default name is never offered.
        app.world_mut().resource_mut::<RiderProfiles>().riding = rider("Rider", 80.0);
        *FETCHED.lock().unwrap() = Some((1, None));
        app.update();
        assert!(PUT.lock().unwrap().is_empty(), "\"Rider\" is not offered");
        *FETCHED.lock().unwrap() = None;
        app.update();
        app.world_mut().resource_mut::<RiderProfiles>().riding = rider("Kai Local", 80.0);
        app.update();
        assert!(PUT.lock().unwrap().is_empty(), "signed out: nothing sent");
        // Signed in, the account has no rider yet: the local one goes up.
        *FETCHED.lock().unwrap() = Some((1, None));
        app.update();
        let sent = PUT.lock().unwrap().pop().expect("the local rider sent");
        assert_eq!(sent.name, "Kai Local");
        assert_eq!(sent.rider["weight_kg"], 80.0);
        // The account's rider arrives (another computer saved it).
        let lea = rider("Lea Account", 58.0);
        *FETCHED.lock().unwrap() = Some((
            2,
            Some(RiderDoc {
                name: lea.name.clone(),
                rider: serde_json::to_value(&lea).unwrap(),
                updated_at: 1,
            }),
        ));
        app.update();
        assert_eq!(riding(&app), lea);
        // A change goes back after a moment, once.
        app.world_mut()
            .resource_mut::<RiderProfiles>()
            .riding
            .weight_kg = 59.0;
        app.update();
        assert!(PUT.lock().unwrap().is_empty(), "not at once");
        for _ in 0..8 {
            app.update();
        }
        let sent = PUT.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].rider["weight_kg"], 59.0);
        // Signed out: the local rider again.
        *FETCHED.lock().unwrap() = None;
        app.update();
        assert_eq!(riding(&app).name, "Kai Local");
    }
}
