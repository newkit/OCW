//! The ride photographer's Bevy glue (spec §2.11), shared by the apps:
//! feeds the Bevy-free [`Photographer`] (`ocw_client::photographer`) from
//! the ride client and the loaded world each frame, puts the person, the
//! drone and the flash into the frame's view, asks the renderer for the
//! photo (`ocw_render::photographer::PhotoRequests`) and hands the
//! rendered image to the app ([`PhotographerHooks::save`]: the desktop
//! writes `photo-N.jpg` with its sidecar), and sends the shutter and the
//! drone's buzz to the app's audio ([`PhotographerHooks::sound`]).
//! Console: `photographer [on|off|status|now]` (setting
//! `photographer.enabled`); `now` plans the next photo at once, ignoring
//! the 20 minutes (tests).
//!
//! Its own feature: it runs in every ride unless switched off in Settings
//! (§2.11 Q1, decided); a Strava recording only uses the photos taken
//! within it (spec §2.8.1).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_audio::cues::Cue;
use ocw_audio::drone::DroneSound;
use ocw_audio::mixer::MixerMsg;
use ocw_audio::spatial::{Listener, place};
use ocw_client::photographer::{self as ph, Limits, Photographer, Taken};
use ocw_render::RideView;
use ocw_render::camera::CameraObstacles;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::package::PackageWorld;
use ocw_render::photographer::{PhotoCaptures, PhotoRequest, PhotoRequests, ViewFrustum};
use ocw_world::landuse::LandClass;
use ocw_world::package::WorldPackage;
use ocw_world::photo::{Conditions, PhotoScene};

/// What the app does with the photographer's output.
#[derive(Resource, Clone, Copy)]
pub struct PhotographerHooks {
    /// A photo was rendered: save it (the image and what the sidecar
    /// needs).
    pub save: fn(&mut World, &Taken, Image),
    /// Play a mixer message (the shutter, the drone's buzz).
    pub sound: fn(&World, MixerMsg),
}

/// The ride's photographer and its switch.
#[derive(Resource)]
pub struct RidePhotographer {
    pub photographer: Photographer,
    pub enabled: bool,
    /// The app lets it work (off in scripted screenshot and capture runs).
    pub allowed: bool,
    /// Time runs this many times faster for the limits (test runs).
    pub scale: f64,
    /// Photos asked from the renderer, by id.
    pending: BTreeMap<u64, Taken>,
    next_id: u64,
    /// The drone was heard last frame.
    drone_heard: bool,
}

impl RidePhotographer {
    pub fn new(scale: f64, seed: u64) -> Self {
        Self {
            photographer: Photographer::new(Limits::scaled(scale), seed),
            enabled: true,
            allowed: true,
            scale,
            pending: BTreeMap::new(),
            next_id: 1,
            drone_heard: false,
        }
    }

    /// A new ride: the count starts again.
    pub fn new_ride(&mut self) {
        self.photographer.reset();
    }
}

/// Adds the photographer; `scale` > 1 shortens its 20 minutes (tests).
pub struct PhotographerPlugin {
    pub scale: f64,
}

impl Plugin for PhotographerPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.insert_resource(RidePhotographer::new(self.scale, 0x0C3A_17E5))
            .add_systems(Update, (run, receive).chain().in_set(crate::ViewExtrasSet));
        register_commands(app);
    }
}

/// The loaded world as the photographer sees it: the drawn ground, the
/// solids the camera avoids (buildings, walls), the package's land use
/// (water, rock, forest) and forest cover.
pub struct WorldScene {
    ground: Option<ocw_render::Ground>,
    solids: CameraObstacles,
    pkg: Option<Arc<WorldPackage>>,
}

impl WorldScene {
    pub fn of(world: &World) -> Self {
        Self {
            ground: world.get_resource::<ocw_render::Ground>().cloned(),
            solids: world
                .get_resource::<CameraObstacles>()
                .cloned()
                .unwrap_or_default(),
            pkg: world
                .get_resource::<PackageWorld>()
                .map(|p| p.home().clone()),
        }
    }

    fn class(&self, x: f64, z: f64) -> Option<LandClass> {
        let p = self.pkg.as_ref()?;
        p.landuse.as_ref()?.class_at(f64::from(p.tile_size_m), x, z)
    }
}

impl PhotoScene for WorldScene {
    fn ground(&self, x: f64, z: f64) -> Option<f64> {
        self.ground.as_ref().and_then(|g| (g.0)(x, z))
    }
    fn water(&self, x: f64, z: f64) -> bool {
        self.class(x, z) == Some(LandClass::Water)
            || self
                .ground(x, z)
                .is_some_and(|g| g <= 0.2 && self.pkg.is_some())
    }
    fn rocky(&self, x: f64, z: f64) -> bool {
        matches!(
            self.class(x, z),
            Some(LandClass::Rock | LandClass::Scree | LandClass::Glacier | LandClass::Wetland)
        )
    }
    fn canopy_m(&self, x: f64, z: f64) -> f64 {
        let Some(p) = self.pkg.as_ref() else {
            return 0.0;
        };
        let measured = p
            .look
            .forest
            .as_ref()
            .and_then(|f| f.cell(f64::from(p.tile_size_m), x, z))
            .map(|c| f64::from(c.density) / 100.0);
        let density = match measured {
            Some(d) => d,
            None if self.class(x, z) == Some(LandClass::Forest) => 0.8,
            None => 0.0,
        };
        if density >= 0.4 { 22.0 } else { 0.0 }
    }
    fn solid_top(&self, x: f64, z: f64) -> Option<f64> {
        self.solids.top_at(x, z)
    }
}

/// The light and weather at the rider from the frame's view.
pub fn conditions(view: &RideView) -> Conditions {
    let mut c = Conditions::default();
    if let Some(s) = &view.sky {
        c.sun_elevation_deg = s.sky.sun.elevation_deg;
        c.sun_azimuth_deg = s.sky.sun.azimuth_deg;
    }
    if let Some(w) = &view.weather {
        c.visibility_m = f64::from(w.state.visibility_m());
        c.precip_mm_h = f64::from(w.state.precip_mm_h);
    }
    c
}

fn run(world: &mut World) {
    let Some(hooks) = world.get_resource::<PhotographerHooks>().copied() else {
        return;
    };
    let enabled = world
        .get_resource::<RidePhotographer>()
        .is_some_and(|p| p.enabled && p.allowed);
    let now = crate::now(world);
    let frustum = world
        .get_resource::<ViewFrustum>()
        .copied()
        .unwrap_or_default();
    let scene = WorldScene::of(world);
    let cond = world
        .get_resource::<RideView>()
        .map(conditions)
        .unwrap_or_default();
    let out = {
        let Some(client) = crate::client(world) else {
            return;
        };
        if client.rider_id().is_none() || client.god().active() {
            None
        } else {
            let s = client.state();
            let (d, lateral, speed) = (s.distance, s.lateral_offset, s.speed);
            let head = client.track().sample(d).offset_position(lateral);
            let forward = client.track().sample(d).direction;
            Some((d, lateral, speed, head, forward))
        }
    };
    let Some((d, lateral, speed, head, forward)) = out else {
        set_view(world, None);
        return;
    };
    if !enabled {
        set_view(world, None);
        stop_drone_sound(world, hooks);
        return;
    }
    // The client's track is borrowed from the world: the photographer runs
    // on a copy and is put back.
    let ((scene_now, events), p) = {
        let world_ref: &World = world;
        let client = crate::client(world_ref).expect("checked");
        let in_view =
            |p: [f64; 3]| frustum.sees(Vec3::new(p[0] as f32, p[1] as f32 + 1.0, p[2] as f32));
        let mut p = world_ref
            .resource::<RidePhotographer>()
            .photographer
            .clone();
        let r = p.frame(
            now,
            client.track(),
            d,
            lateral,
            speed,
            &scene,
            &cond,
            &in_view,
        );
        (r, p)
    };
    world.resource_mut::<RidePhotographer>().photographer = p;
    set_view(world, scene_now.as_ref());
    let listener = Listener {
        pos: [head[0], head[1] + 1.5, head[2]],
        forward,
        velocity: [forward[0] * speed, 0.0, forward[1] * speed],
    };
    // The drone's buzz while it flies.
    match scene_now.as_ref().and_then(|s| s.drone) {
        Some(dr) if dr.rotor > 0.0 => {
            let pl = place(&listener, dr.pos, dr.velocity);
            (hooks.sound)(
                world,
                MixerMsg::Drone(Some(DroneSound::placed(&pl, dr.rotor))),
            );
            world.resource_mut::<RidePhotographer>().drone_heard = true;
        }
        _ => stop_drone_sound(world, hooks),
    }
    if let Some(at) = events.shutter {
        let pl = place(&listener, at, [0.0; 3]);
        (hooks.sound)(
            world,
            MixerMsg::PlacedCue(Cue::Shutter, [pl.left, pl.right], web_time::Instant::now()),
        );
    }
    if let Some(t) = events.photo {
        let mut r = world.resource_mut::<RidePhotographer>();
        let id = r.next_id;
        r.next_id += 1;
        r.pending.insert(id, t.clone());
        world.resource_mut::<PhotoRequests>().0.push(PhotoRequest {
            id,
            eye: t.pose.eye,
            look: t.pose.look,
        });
        info!(
            n = t.n,
            score = t.score.total,
            kind = t.pose.kind.label(),
            "ride photo"
        );
    }
}

fn stop_drone_sound(world: &mut World, hooks: PhotographerHooks) {
    let heard = world
        .get_resource::<RidePhotographer>()
        .is_some_and(|r| r.drone_heard);
    if heard {
        (hooks.sound)(world, MixerMsg::Drone(None));
        world.resource_mut::<RidePhotographer>().drone_heard = false;
    }
}

fn set_view(world: &mut World, scene: Option<&ph::Scene>) {
    let want = scene.map(ph::view);
    if let Some(mut v) = world.get_resource_mut::<RideView>()
        && v.photographer != want
    {
        v.photographer = want;
    }
}

/// The rendered photos to the app.
fn receive(world: &mut World) {
    let Some(hooks) = world.get_resource::<PhotographerHooks>().copied() else {
        return;
    };
    let Some(mut caps) = world.get_resource_mut::<PhotoCaptures>() else {
        return;
    };
    let done = std::mem::take(&mut caps.0);
    for (id, image) in done {
        let taken = world
            .get_resource_mut::<RidePhotographer>()
            .and_then(|mut r| r.pending.remove(&id));
        if let Some(t) = taken {
            (hooks.save)(world, &t, image);
        }
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("photographer")
            .about("The ride photographer: on, off, status, now (plan the next photo at once) or drone (the next by a drone)")
            .arg(Arg::new("what").default_value("status")),
        |world, m| {
            let what = m.get_one::<String>("what").map_or("status", String::as_str);
            let mut r = world
                .get_resource_mut::<RidePhotographer>()
                .ok_or("no photographer")?;
            match what {
                "on" | "true" | "1" => r.enabled = true,
                "off" | "false" | "0" => r.enabled = false,
                "now" => {
                    r.enabled = true;
                    r.allowed = true;
                    r.photographer.force();
                }
                "drone" => {
                    r.enabled = true;
                    r.allowed = true;
                    r.photographer.force_drone();
                }
                "status" => {}
                other => {
                    return Err(format!(
                        "expected on, off, status, now or drone, not {other:?}"
                    ));
                }
            }
            let p = &r.photographer;
            let plan = p.plan().map_or("no photo planned".to_string(), |pl| {
                format!(
                    "next: {} ({}), score {:.2}, at {:.0} m",
                    pl.shot.pose.kind.label(),
                    if pl.shot.pose.is_drone() { "drone" } else { "person" },
                    pl.shot.score.total,
                    pl.at_m
                )
            });
            Ok(format!(
                "ride photographer {}: {} of {} photos in this ride, at least {:.0} min apart; {plan}",
                if r.enabled { "on" } else { "off" },
                p.photos(),
                p.limits.max_photos,
                p.limits.gap_s / 60.0
            ))
        },
    ));
}
