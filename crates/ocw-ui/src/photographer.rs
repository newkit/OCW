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
//!
//! Other riders see it (spec §2.11 Q2, v4.38): the own shot goes to the
//! server whenever it changes (`Photographer::shared`), the other riders'
//! shots come back through the ride client (`ClientCore::remote_shots`)
//! and are drawn (`RideView::photographers`) and heard (the shutter placed
//! at their camera, the loudest drone) here, in both apps. The setting
//! only stops the own photographer: the others' are drawn even when it is
//! off, while flying as the unicorn and in scripted runs.
//!
//! A rider may hide the others' photographers (spec 2.11 Q5, decided
//! 2026-11-19): setting `photographer.others` (default on = shown),
//! console `photographer others on|off`. Off, their person, drone and
//! flash are not drawn and their shutter and drone are not heard – for
//! this rider only: the own photographer still runs and is shared, the
//! server and the other riders see nothing change.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_audio::cues::Cue;
use ocw_audio::mixer::MixerMsg;
use ocw_audio::spatial::Listener;
use ocw_client::photographer::{self as ph, Limits, Photographer, Taken};
use ocw_render::RideView;
use ocw_render::camera::CameraObstacles;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::package::PackageWorld;
use ocw_render::photo_preview::{Caption, PhotoPreview};
use ocw_render::photographer::{PhotoCaptures, PhotoRequest, PhotoRequests, ViewFrustum};
use ocw_world::landuse::LandClass;
use ocw_world::package::WorldPackage;
use ocw_world::photo::{Conditions, Landmark, PhotoScene};
use ocw_world::photo_share::SharedShot;

/// The own photographer is off on this platform or by the page's switch
/// (spec 14 Phase 10, 10G): it takes no photos whatever the setting says;
/// the other riders' photographers are still drawn. The browser app
/// inserts it.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotosOff {
    /// The default on iOS / iPadOS WebKit with WebGL 2: the first photo
    /// closed the page on an iPhone (iOS 18.7, 2026-11-27).
    IosWebKit,
    /// The address's `photos=0`.
    Switch,
}

/// The reason of the iOS default.
pub const IOS_PHOTOS_WHY: &str =
    "iOS WebKit with WebGL 2: taking the first photo closed the page on an iPhone";

impl PhotosOff {
    /// For the log, the status and the page text.
    pub fn long(self) -> &'static str {
        match self {
            Self::IosWebKit => IOS_PHOTOS_WHY,
            Self::Switch => "photos=0",
        }
    }

    /// For the Settings rows.
    pub fn short(self) -> &'static str {
        match self {
            Self::IosWebKit => "off on iOS (WebGL 2)",
            Self::Switch => "off (photos=0)",
        }
    }

    /// The page log's line.
    pub fn line(self) -> String {
        match self {
            Self::IosWebKit => format!(
                "ride photos off on {IOS_PHOTOS_WHY}; photos=1 turns them on (photofix=… to test a fix)"
            ),
            Self::Switch => "ride photos off (photos=0)".into(),
        }
    }
}

/// What the app does with the photographer's output.
#[derive(Resource, Clone, Copy)]
pub struct PhotographerHooks {
    /// A photo was rendered: save it (the image and what the sidecar
    /// needs); returns where it went for the HUD's photo preview ("Saved
    /// to Pictures/OCW, added to your ride"), `None` when it was not kept.
    pub save: fn(&mut World, &Taken, Image) -> Option<String>,
    /// Play a mixer message (the shutter, the drone's buzz).
    pub sound: fn(&mut World, MixerMsg),
}

/// The ride's photographer and its switch.
#[derive(Resource)]
pub struct RidePhotographer {
    pub photographer: Photographer,
    pub enabled: bool,
    /// The other riders' photographers are drawn and heard (setting
    /// `photographer.others`, spec 2.11 Q5; purely local).
    pub others: bool,
    /// The app lets it work (off in scripted screenshot and capture runs).
    pub allowed: bool,
    /// Time runs this many times faster for the limits (test runs).
    pub scale: f64,
    /// Photos asked from the renderer, by id.
    pending: BTreeMap<u64, Taken>,
    next_id: u64,
    /// The drone was heard last frame.
    drone_heard: bool,
    /// The game time the own photographer last ran at (never goes back).
    last_t: Option<f64>,
    /// The own shot as last sent to the server.
    sent: Option<SharedShot>,
    /// The other riders' shutters already played (rider, shot).
    shutters_heard: BTreeSet<(u32, u32)>,
    /// The other riders' shots drawn so far (rider, shot; logged once).
    seen: BTreeSet<(u32, u32)>,
    /// The own drone's phase last frame (each change is logged: scripted
    /// drone videos, `camera.drone`).
    drone_phase: Option<ocw_world::photo_drone::DronePhase>,
    /// The loaded packages' peaks and viewpoints (section `LMRK`, spec
    /// 2.11 Q4) and the world set they were taken from (pointer,
    /// generation, tiles version).
    landmarks: Arc<Vec<Landmark>>,
    landmarks_key: Option<(usize, u64, u64)>,
}

impl RidePhotographer {
    pub fn new(scale: f64, seed: u64) -> Self {
        Self {
            photographer: Photographer::new(Limits::scaled(scale), seed),
            enabled: true,
            others: true,
            allowed: true,
            scale,
            pending: BTreeMap::new(),
            next_id: 1,
            drone_heard: false,
            last_t: None,
            sent: None,
            shutters_heard: BTreeSet::new(),
            seen: BTreeSet::new(),
            drone_phase: None,
            landmarks: Arc::new(Vec::new()),
            landmarks_key: None,
        }
    }

    /// The peaks and viewpoints the view score counts (spec 2.11 Q4).
    pub fn landmarks(&self) -> &[Landmark] {
        &self.landmarks
    }

    /// A new ride: the count starts again.
    pub fn new_ride(&mut self) {
        self.photographer.reset();
        self.last_t = None;
    }

    /// The own shot as last sent to the server (spec §2.11).
    pub fn sent(&self) -> Option<&SharedShot> {
        self.sent.as_ref()
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
    landmarks: Arc<Vec<Landmark>>,
}

/// Takes the loaded packages' peaks and viewpoints into the photographer
/// whenever the world set changes.
fn refresh_landmarks(world: &mut World) {
    let key = world.get_resource::<PackageWorld>().map(|w| {
        let s = &w.set;
        (Arc::as_ptr(s) as usize, s.generation(), s.tiles_version())
    });
    let Some(p) = world.get_resource::<RidePhotographer>() else {
        return;
    };
    if p.landmarks_key == key {
        return;
    }
    let list = world
        .get_resource::<PackageWorld>()
        .map(|w| Landmark::of_set(&w.set))
        .unwrap_or_default();
    let mut p = world.resource_mut::<RidePhotographer>();
    p.landmarks_key = key;
    p.landmarks = Arc::new(list);
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
            landmarks: world
                .get_resource::<RidePhotographer>()
                .map(|p| p.landmarks.clone())
                .unwrap_or_default(),
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
    fn landmarks(&self) -> &[Landmark] {
        &self.landmarks
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
        .is_some_and(|p| p.enabled && p.allowed)
        && !world.contains_resource::<PhotosOff>();
    let show_others = world
        .get_resource::<RidePhotographer>()
        .is_none_or(|p| p.others);
    let now = crate::now(world);
    refresh_landmarks(world);
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
        let times = client.server_time_s(now).zip(client.photo_time_s(now));
        if let (Some(_), Some((server_t, remote_t))) = (client.rider_id(), times) {
            let s = client.state();
            let (d, lateral, speed) = (s.distance, s.lateral_offset, s.speed);
            let head = client.track().sample(d).offset_position(lateral);
            let forward = client.track().sample(d).direction;
            // The other riders' photographers, at the time the other
            // riders are drawn at (spec 2.11, v4.38) – unless this rider
            // hides them (Q5).
            let remote = if show_others {
                client.remote_shots().scenes(remote_t, &scene, head)
            } else {
                Vec::new()
            };
            Some(Frame {
                d,
                lateral,
                speed,
                head,
                forward,
                server_t,
                remote_t,
                remote,
                flying: client.god().active(),
            })
        } else {
            None
        }
    };
    let Some(f) = out else {
        set_view(world, None, &[]);
        stop_drone_sound(world, hooks);
        return;
    };
    // The game time the own photographer runs on: the server's, never
    // going back by its jitter (a new session starts again at 0).
    let t = {
        let mut r = world.resource_mut::<RidePhotographer>();
        let t = match r.last_t {
            Some(l) if f.server_t < l && f.server_t > l - 2.0 => l,
            _ => f.server_t,
        };
        r.last_t = Some(t);
        t
    };
    let mut own_scene = None;
    let mut events = ph::Events::default();
    if enabled && !f.flying {
        // The client's track is borrowed from the world: the photographer
        // runs on a copy and is put back.
        let ((scene_now, ev), p) = {
            let world_ref: &World = world;
            let client = crate::client(world_ref).expect("checked");
            let in_view =
                |p: [f64; 3]| frustum.sees(Vec3::new(p[0] as f32, p[1] as f32 + 1.0, p[2] as f32));
            let mut p = world_ref
                .resource::<RidePhotographer>()
                .photographer
                .clone();
            let r = p.frame(
                t,
                client.track(),
                f.d,
                f.lateral,
                f.speed,
                &scene,
                &cond,
                &in_view,
            );
            (r, p)
        };
        world.resource_mut::<RidePhotographer>().photographer = p;
        own_scene = scene_now;
        events = ev;
    } else {
        // Switched off (or flying): the own photographer leaves; the
        // others see it go.
        world
            .resource_mut::<RidePhotographer>()
            .photographer
            .stop(t);
    }
    let phase = own_scene.as_ref().and_then(|s| s.drone).map(|d| d.phase);
    if world.resource::<RidePhotographer>().drone_phase != phase {
        world.resource_mut::<RidePhotographer>().drone_phase = phase;
        info!(
            t = format!("{t:.1}"),
            app_s = format!("{now:.1}"),
            phase = ?phase,
            "photographer's drone"
        );
    }
    share(world);
    let remote: Vec<ph::Scene> = f.remote.iter().map(|r| r.scene.clone()).collect();
    set_view(world, own_scene.as_ref(), &remote);
    let listener = Listener {
        pos: [f.head[0], f.head[1] + 1.5, f.head[2]],
        forward: f.forward,
        velocity: [f.forward[0] * f.speed, 0.0, f.forward[1] * f.speed],
    };
    // The drone heard: the loudest of the own and the others' (one voice).
    let drones: Vec<_> = own_scene
        .iter()
        .chain(&remote)
        .filter_map(|s| s.drone)
        .collect();
    match ph::loudest_drone(&listener, &drones) {
        Some(sound) => {
            (hooks.sound)(world, MixerMsg::Drone(Some(sound)));
            world.resource_mut::<RidePhotographer>().drone_heard = true;
        }
        None => stop_drone_sound(world, hooks),
    }
    if let Some(at) = events.shutter {
        let g = ph::shutter_gains(&listener, at);
        (hooks.sound)(
            world,
            MixerMsg::PlacedCue(Cue::Shutter, g, web_time::Instant::now()),
        );
    }
    for r in &f.remote {
        let key = (r.rider, r.shot);
        if world.resource_mut::<RidePhotographer>().seen.insert(key) {
            info!(
                rider = r.rider,
                shot = r.shot,
                drone = r.scene.drone.is_some(),
                "another rider's photographer"
            );
        }
    }
    // The others' shutters, once each, placed where their camera is.
    for r in &f.remote {
        let Some((at, eye)) = r.flash else {
            continue;
        };
        let key = (r.rider, r.shot);
        let due = f.remote_t >= at && f.remote_t - at < 1.0;
        let heard = world
            .resource::<RidePhotographer>()
            .shutters_heard
            .contains(&key);
        if due && !heard {
            info!(
                rider = r.rider,
                shot = r.shot,
                "another rider's photo (flash)"
            );
            world
                .resource_mut::<RidePhotographer>()
                .shutters_heard
                .insert(key);
            let g = ph::shutter_gains(&listener, eye);
            (hooks.sound)(
                world,
                MixerMsg::PlacedCue(Cue::Shutter, g, web_time::Instant::now()),
            );
        }
    }
    if let Some(mut c) = crate::client_mut(world) {
        c.prune_shots(now);
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

/// What a frame needs of the ride client.
struct Frame {
    d: f64,
    lateral: f64,
    speed: f64,
    head: [f64; 3],
    forward: [f64; 2],
    server_t: f64,
    remote_t: f64,
    remote: Vec<ph::RemoteScene>,
    flying: bool,
}

/// Sends the own photographer's shot to the server whenever it changed
/// (spec §2.11, v4.38): the others draw it.
fn share(world: &mut World) {
    let Some(r) = world.get_resource::<RidePhotographer>() else {
        return;
    };
    let Some(shot) = r.photographer.shared() else {
        return;
    };
    if r.sent.as_ref() == Some(shot) {
        return;
    }
    let shot = shot.clone();
    let bytes = ocw_client::client::ClientCore::photo_message(&shot);
    if crate::send(world, bytes) {
        world.resource_mut::<RidePhotographer>().sent = Some(shot);
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

fn set_view(world: &mut World, own: Option<&ph::Scene>, others: &[ph::Scene]) {
    let want = own.map(ph::view);
    let others: Vec<_> = others.iter().map(ph::view).collect();
    if let Some(mut v) = world.get_resource_mut::<RideView>() {
        if v.photographer != want {
            v.photographer = want;
        }
        if v.photographers != others {
            v.photographers = others;
        }
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
            // The HUD's preview (spec §3.2): the thumbnail from the image
            // in memory, before it goes to be saved.
            let thumb = world
                .get_resource::<PhotoPreview>()
                .is_some_and(|p| p.enabled)
                .then(|| ocw_render::photo_preview::thumbnail(&image))
                .flatten();
            // `photofix=nosave` (spec 10G): no JPEG, no download.
            let nosave = world
                .get_resource::<ocw_render::switches::RenderSwitches>()
                .is_some_and(|s| s.photo_fix.nosave);
            let went = if nosave {
                info!(
                    "ride photo {id}: not saved (photofix=nosave), {} × {}",
                    image.width(),
                    image.height()
                );
                Some("Not saved (photofix=nosave)".to_string())
            } else {
                info!("ride photo {id}: saving");
                (hooks.save)(world, &t, image)
            };
            if let (Some(thumb), Some(went)) = (thumb, went)
                && let Some(mut p) = world.get_resource_mut::<PhotoPreview>()
            {
                p.show_thumb(thumb, Caption::ride_photo(t.pose.is_drone(), went));
            }
        }
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("photographer")
            .about("The ride photographer: on, off, status, now (plan the next photo at once), drone (the next by a drone) or others on|off (show or hide the other riders' photographers, for you only)")
            .arg(Arg::new("what").default_value("status"))
            .arg(Arg::new("value")),
        |world, m| {
            let what = m.get_one::<String>("what").map_or("status", String::as_str);
            let value = m.get_one::<String>("value").map(String::as_str);
            let off = world.get_resource::<PhotosOff>().copied();
            if let Some(off) = off
                && matches!(what, "now" | "drone")
            {
                return Err(format!(
                    "ride photos are off ({}); photos=1 in the address turns them on",
                    off.long()
                ));
            }
            let mut r = world
                .get_resource_mut::<RidePhotographer>()
                .ok_or("no photographer")?;
            if what == "others" {
                // Spec 2.11 Q5: purely local – nothing goes to the server.
                match value.unwrap_or("status") {
                    "on" | "true" | "1" => r.others = true,
                    "off" | "false" | "0" => r.others = false,
                    "status" => {}
                    other => return Err(format!("expected on or off, not {other:?}")),
                }
                return Ok(format!(
                    "other riders' photographers {}",
                    if r.others {
                        "shown"
                    } else {
                        "hidden (for you only)"
                    }
                ));
            }
            if let Some(v) = value {
                return Err(format!("unexpected {v:?} after {what:?}"));
            }
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
                        "expected on, off, status, now, drone or others, not {other:?}"
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
            let state = match off {
                Some(o) => format!("off ({})", o.long()),
                None if r.enabled => "on".into(),
                None => "off".into(),
            };
            Ok(format!(
                "ride photographer {state}: {} of {} photos in this ride, at least {:.0} min apart; {plan}; other riders' photographers {}",
                p.photos(),
                p.limits.max_photos,
                p.limits.gap_s / 60.0,
                if r.others { "shown" } else { "hidden" }
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use ocw_client::client::ClientCore;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
    use ocw_server::local::FrameServer;
    use ocw_world::Route;
    use ocw_world::photo::{CameraPose, Carrier, PoseKind};

    use super::*;

    /// A ride on the in-memory local server, reached through
    /// [`crate::RideAccess`] as the apps do; the trainer at 300 W.
    #[derive(Resource)]
    struct Ride {
        client: ClientCore,
        server: FrameServer,
        next_sample: f64,
    }

    /// What the photographer sent to the app's audio.
    #[derive(Resource, Default)]
    struct Heard(Vec<MixerMsg>);

    fn step_ride(time: Res<Time>, mut ride: ResMut<Ride>) {
        let now = time.elapsed_secs_f64();
        let Ride {
            client,
            server,
            next_sample,
        } = &mut *ride;
        if now >= *next_sample {
            *next_sample = now + 0.25;
            let ev = ocw_devices::DeviceEvent::Power {
                watts: 300.0,
                cadence_rpm: Some(90.0),
                timestamp: Duration::from_secs_f64(now),
            };
            if let Some(bytes) = client.on_device_event(ev) {
                server.send(&bytes);
            }
        }
        server.advance(time.delta_secs_f64());
        for m in server.drain() {
            let _ = client.on_server_message(&m, now);
        }
        client.predict_step();
    }

    fn app() -> App {
        let route = Route::test_loop();
        let client = ClientCore::new(route.clone(), "Ada Rider", Default::default());
        let mut server = FrameServer::new(route.clone(), 0, 0.0);
        server.send(&client.hello());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .insert_resource(RideView::new(route))
            .insert_resource(ocw_render::Ground(Arc::new(|_, _| Some(0.0))))
            .insert_resource(Ride {
                client,
                server,
                next_sample: 0.0,
            })
            .init_resource::<Heard>()
            .init_resource::<PhotoRequests>()
            .insert_resource(crate::RideAccess {
                client: |w| w.get_resource::<Ride>().map(|r| &r.client),
                client_mut: |w| {
                    w.get_resource_mut::<Ride>()
                        .map(|r| r.map_unchanged(|r| &mut r.client))
                },
                send: |w, bytes| {
                    if let Some(mut r) = w.get_resource_mut::<Ride>() {
                        r.server.send(&bytes);
                    }
                },
            })
            .insert_resource(PhotographerHooks {
                save: |_, _, _| None,
                sound: |w, m| w.resource_mut::<Heard>().0.push(m),
            })
            .add_systems(Update, step_ride.in_set(crate::RideViewSet))
            .add_plugins(PhotographerPlugin { scale: 1.0 });
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world()
            .resource::<ConsoleState>()
            .log()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    fn client(app: &App) -> &ClientCore {
        &app.world().resource::<Ride>().client
    }

    fn now(app: &App) -> f64 {
        app.world().resource::<Time>().elapsed_secs_f64()
    }

    /// Rides until the client has the game time; returns the time the
    /// other riders are drawn at.
    fn joined(app: &mut App) -> f64 {
        for _ in 0..500 {
            app.update();
            if let Some(t) = client(app).photo_time_s(now(app)) {
                return t;
            }
        }
        panic!("no game time");
    }

    /// Another rider's (id 9999) person standing 10 m from the own rider,
    /// showing now, the photo 1 s later – as the server relays it.
    fn others_shot(app: &mut App, t: f64) {
        let c = client(app);
        let s = c.state();
        let head = c
            .track()
            .sample(s.distance)
            .offset_position(s.lateral_offset);
        let feet = [head[0] + 10.0, 0.0, head[2]];
        let shot = SharedShot {
            rider: 9999,
            shot: 1,
            pose: CameraPose {
                kind: PoseKind::Side,
                carrier: Carrier::Person { crouch: false },
                feet,
                eye: [feet[0], 1.6, feet[2]],
                look: [-1.0, 0.0, 0.0],
            },
            seed: 7,
            appear_at: t,
            planned_at: t + 1.0,
            taken_at: Some(t + 1.0),
            gone_at: None,
            aborted_at: None,
            flight: None,
        };
        let wire = ocw_protocol::v1::PhotoShot {
            rider_id: 9999,
            ..ocw_protocol::photo_to_wire(&shot)
        };
        let bytes = ocw_protocol::encode(&ocw_protocol::server_message(
            ocw_protocol::v1::server_message::Msg::Photo(wire),
        ));
        let at = now(app);
        let mut ride = app.world_mut().resource_mut::<Ride>();
        let _ = ride.client.on_server_message(&bytes, at);
        assert_eq!(ride.client.remote_shots().len(), 1);
    }

    fn shutters(app: &App) -> usize {
        app.world()
            .resource::<Heard>()
            .0
            .iter()
            .filter(|m| matches!(m, MixerMsg::PlacedCue(Cue::Shutter, _, _)))
            .count()
    }

    /// Runs 3 s; returns the most other riders' photographers drawn in a
    /// frame and whether a flash was drawn.
    fn watch(app: &mut App) -> (usize, bool) {
        let (mut most, mut flash) = (0, false);
        for _ in 0..150 {
            app.update();
            let v = app.world().resource::<RideView>();
            most = most.max(v.photographers.len());
            flash |= v.photographers.iter().any(|p| p.flash.is_some());
        }
        (most, flash)
    }

    /// Shown by default (spec 2.11 Q2): another rider's photographer is
    /// drawn, flashes and its shutter is heard.
    #[test]
    fn others_photographers_are_drawn_and_heard_by_default() {
        let mut app = app();
        assert!(app.world().resource::<RidePhotographer>().others);
        run(&mut app, "photographer off");
        let t = joined(&mut app);
        others_shot(&mut app, t);
        let (drawn, flash) = watch(&mut app);
        assert_eq!(drawn, 1);
        assert!(flash, "the flash drawn");
        assert_eq!(shutters(&app), 1, "the shutter heard once");
    }

    /// Hidden (spec 2.11 Q5): `photographer others off` – the other
    /// rider's person and flash are not drawn and the shutter is not
    /// heard; shown again, the person is back (its shutter, long past, is
    /// not played late). Only this client: nothing goes to the server.
    #[test]
    fn others_photographers_can_be_hidden_for_this_rider_only() {
        let mut app = app();
        run(&mut app, "photographer off");
        let out = run(&mut app, "photographer others off");
        assert!(
            out.ends_with("other riders' photographers hidden (for you only)"),
            "{out}"
        );
        assert!(!app.world().resource::<RidePhotographer>().others);
        let status = run(&mut app, "photographer");
        assert!(
            status.ends_with("other riders' photographers hidden"),
            "{status}"
        );
        let bad = run(&mut app, "photographer others maybe");
        assert!(bad.contains("expected on or off"), "{bad}");
        let t = joined(&mut app);
        let sent_before = app.world().resource::<RidePhotographer>().sent().cloned();
        others_shot(&mut app, t);
        let (drawn, flash) = watch(&mut app);
        assert_eq!((drawn, flash), (0, false), "nothing drawn");
        assert_eq!(shutters(&app), 0, "nothing heard");
        assert!(
            !app.world()
                .resource::<Heard>()
                .0
                .iter()
                .any(|m| matches!(m, MixerMsg::Drone(Some(_)))),
            "no drone heard"
        );
        // Still known to the client (only the drawing is hidden), nothing
        // sent for it.
        assert_eq!(client(&app).remote_shots().len(), 1);
        assert_eq!(
            app.world().resource::<RidePhotographer>().sent().cloned(),
            sent_before
        );
        let shown = run(&mut app, "photographer others on");
        assert!(
            shown.ends_with("other riders' photographers shown"),
            "{shown}"
        );
        let (drawn, _) = watch(&mut app);
        assert_eq!(drawn, 1, "shown again");
        assert_eq!(shutters(&app), 0, "the old shutter is not played late");
    }

    /// The own photographer is not affected (spec 2.11 Q5): with the
    /// others hidden it still comes, is drawn and is shared.
    #[test]
    fn the_own_photographer_is_drawn_with_the_others_hidden() {
        let mut app = app();
        run(&mut app, "photographer others off");
        joined(&mut app);
        run(&mut app, "photographer now");
        let mut drawn = false;
        for _ in 0..9000 {
            app.update();
            if app.world().resource::<RideView>().photographer.is_some() {
                drawn = true;
                break;
            }
        }
        assert!(drawn, "the own photographer drawn");
        let r = app.world().resource::<RidePhotographer>();
        assert!(r.enabled && !r.others);
        assert!(r.sent().is_some(), "the own shot shared with the others");
    }

    /// Rides up to 9000 frames (3 min): whether the own photographer
    /// showed up, and the photos asked from the renderer meanwhile.
    fn own_photographer(app: &mut App) -> (bool, usize) {
        let (mut shown, mut asked) = (false, 0);
        for _ in 0..9000 {
            app.update();
            asked += std::mem::take(&mut app.world_mut().resource_mut::<PhotoRequests>().0).len();
            shown |= app.world().resource::<RideView>().photographer.is_some();
            if shown && asked > 0 {
                break;
            }
        }
        (shown, asked)
    }

    /// Off on iOS WebKit with WebGL 2 (spec 14 Phase 10, 10G: the first
    /// photo closed the page on an iPhone): no photo is asked for, even
    /// with `photographer now`, the status and the error say why; the
    /// other riders' photographers are still drawn. Without it the same
    /// ride's photographer shows up.
    #[test]
    fn photos_off_takes_no_photo_and_says_why() {
        let mut app = app();
        app.insert_resource(PhotosOff::IosWebKit);
        joined(&mut app);
        let answer = run(&mut app, "photographer now");
        assert!(
            answer.contains("ride photos are off (iOS WebKit with WebGL 2: taking the first photo closed the page on an iPhone)")
                && answer.contains("photos=1"),
            "{answer}"
        );
        let status = run(&mut app, "photographer");
        assert!(
            status.starts_with("ride photographer off (iOS WebKit"),
            "{status}"
        );
        assert_eq!(own_photographer(&mut app), (false, 0));
        assert!(app.world().resource::<RidePhotographer>().sent().is_none());
        let t = client(&app).photo_time_s(now(&app)).unwrap();
        others_shot(&mut app, t);
        assert_eq!(watch(&mut app).0, 1, "the others' photographers drawn");
        assert!(PhotosOff::IosWebKit.line().starts_with(
            "ride photos off on iOS WebKit with WebGL 2: taking the first photo closed the page on an iPhone"
        ));
        assert_eq!(PhotosOff::Switch.short(), "off (photos=0)");
        // The same ride without the platform's default.
        let mut app = self::app();
        joined(&mut app);
        run(&mut app, "photographer now");
        assert!(
            own_photographer(&mut app).0,
            "the own photographer shows up"
        );
    }
}
