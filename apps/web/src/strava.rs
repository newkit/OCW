//! The Strava recording in the browser (spec §2.8.1 ST1 and *Where an S
//! recording is written*, v4.79 / v4.80): S (its pad button, Ride →
//! Strava recording, `strava.record`) starts and stops it as on the
//! desktop (`ocw_client::strava::StravaRecording`). Every recording is
//! written **always**: its FIT file and the photos the ride photographer
//! takes while it runs go into its own subfolder (`2026-11-18 Puerto de la
//! Cruz/`, named by `ocw_client::activities`) of the **activities folder**
//! the rider chose (Captures → *Activities folder: choose*,
//! `activities.folder choose`: the File System Access API, an `OCW`
//! folder made in the chosen one) – else they are downloads named by
//! their files (`OCW 2026-11-18 10-05 Puerto de la Cruz.fit`, `… photo
//! 1.jpg`). The stop notice and the photo preview say where they went.
//! A page cannot list its folder before it writes, so the ` 2` suffix
//! counts the recordings of this page. The page keeps its recordings'
//! FIT files ([`WebStrava::kept`]) for the simple upload: only they go
//! to Strava, never a whole ride (spec 2.8.1 Q6; [`recordings_by_ride`]).

use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::activities::{self, Target};
use ocw_client::strava::{Outcome, StravaRecording};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::plugin::Session;

/// What [`ActivitySaver`] calls: (path below the folder, MIME, bytes) →
/// `folder:NAME` or `download`.
pub type ActivitySaveFn = dyn Fn(&str, &str, Vec<u8>) -> String + Send + Sync;
/// The chosen folder's name while it may be written.
pub type ActivityFolderFn = dyn Fn() -> Option<String> + Send + Sync;

/// Where the recordings' files go: the activities folder the rider chose,
/// else downloads.
#[derive(Resource, Clone)]
pub struct ActivitySaver {
    pub save: Arc<ActivitySaveFn>,
    pub folder: Arc<ActivityFolderFn>,
}

impl ActivitySaver {
    /// Downloads only (no folder API, the tests): named by the file.
    pub fn downloads(d: crate::ui::Downloads) -> Self {
        Self {
            save: Arc::new(move |path, mime, bytes| {
                (d.0)(file_part(path), mime, bytes);
                "download".into()
            }),
            folder: Arc::new(|| None),
        }
    }

    /// A folder named `name` that keeps what is written in `log` (the
    /// tests: path, bytes).
    pub fn folder_log(name: &'static str, log: crate::ui::DownloadLog) -> Self {
        Self {
            save: Arc::new(move |path, mime, bytes| {
                log.lock()
                    .expect("not poisoned")
                    .push((path.into(), mime.into(), bytes));
                format!("folder:{name}")
            }),
            folder: Arc::new(move || Some(name.to_string())),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn page() -> Self {
        use crate::browser::{PageFolder, folder_name, folder_save};
        Self {
            save: Arc::new(|path, mime, bytes| {
                folder_save(PageFolder::Activities, path, mime, &bytes)
            }),
            folder: Arc::new(|| folder_name(PageFolder::Activities)),
        }
    }
}

/// The file's name of a path below the folder.
pub fn file_part(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Where a file went, for the notice and the preview: `Saved to
/// OCW/2026-11-18 Puerto de la Cruz` or `Downloaded OCW … .jpg`.
pub fn went(answer: &str, path: &str) -> String {
    match answer.strip_prefix("folder:") {
        Some(root) => match path.rsplit_once('/') {
            Some((dir, _)) => format!("Saved to {root}/{dir}"),
            None => format!("Saved to {root}"),
        },
        None => format!("Downloaded {}", file_part(path)),
    }
}

/// A Strava recording of this page, kept for the simple upload.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptRecording {
    /// Its start (UTC Unix s).
    pub start_unix_s: i64,
    /// Its file's name (`OCW 2026-11-18 10-05 Puerto de la Cruz.fit`).
    pub name: String,
    pub fit: Vec<u8>,
}

/// The browser's Strava recording and, while it runs, where it goes.
#[derive(Resource)]
pub struct WebStrava {
    pub rec: StravaRecording,
    pub target: Option<Target>,
    /// The folders this page's recordings used (for ` 2`, ` 3`).
    used: Vec<String>,
    /// This page's recordings (the simple upload sends them).
    pub kept: Vec<KeptRecording>,
}

/// The kept recordings within `ride`, oldest first.
pub fn recordings_of(world: &World, ride: &ocw_client::history::RideEntry) -> Vec<KeptRecording> {
    world
        .get_resource::<WebStrava>()
        .map(|s| {
            s.kept
                .iter()
                .filter(|k| {
                    ocw_client::strava::within_ride(k.start_unix_s, ride.start_unix_s, ride.elapsed_s)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// How many kept recordings each ride of the history has (by ride id;
/// the ride page's rows, spec 2.8.1 Q6).
pub fn recordings_by_ride(world: &World) -> std::collections::BTreeMap<String, usize> {
    let Some(h) = world.get_resource::<ocw_ui::history::TrainingHistory>() else {
        return Default::default();
    };
    h.history
        .rides
        .iter()
        .filter_map(|r| {
            let n = recordings_of(world, r).len();
            (n > 0).then(|| (r.id.clone(), n))
        })
        .collect()
}

impl Default for WebStrava {
    fn default() -> Self {
        // No API app in the browser: the stop points to the simple upload.
        Self {
            rec: StravaRecording::new(false, ""),
            target: None,
            used: Vec::new(),
            kept: Vec::new(),
        }
    }
}

/// The photo's path while a recording runs (counted): below the
/// activities folder, in the recording's subfolder.
pub fn photo_path(world: &mut World) -> Option<String> {
    let mut s = world.get_resource_mut::<WebStrava>()?;
    let t = s.target.as_mut()?;
    let name = t.next_photo();
    Some(t.rel(&name))
}

fn unix_now() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The place the recording starts at and the world's UTC offset.
fn place_and_offset(world: &World, unix_s: i64) -> (String, i32) {
    let client = ocw_ui::client(world);
    let at = client.filter(|c| c.rider_id().is_some()).map(|c| {
        let s = c.state();
        let p = c
            .track()
            .sample(s.distance)
            .offset_position(s.lateral_offset);
        [p[0], p[2]]
    });
    let route = client
        .map(|c| c.course_name().to_string())
        .unwrap_or_default();
    let world_name = world
        .get_resource::<crate::world::WorldStream>()
        .and_then(|s| s.identity.clone())
        .unwrap_or_default();
    let places = world
        .get_resource::<ocw_render::package::PackageWorld>()
        .and_then(|p| p.home().places.clone());
    let place = activities::place_name(places.as_ref(), at, &route, &world_name);
    let offset = world
        .get_resource::<ocw_ui::timemachine::WorldPlace>()
        .map(|p| p.zone.offset_s(unix_s * 1000))
        .unwrap_or(0);
    (place, offset)
}

/// Start, stop or toggle (S, the menu, `strava.record`); the answer.
pub fn record(world: &mut World, what: &str) -> Result<String, String> {
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    let route = ocw_ui::client(world)
        .map(|c| c.course_name().to_string())
        .unwrap_or_default();
    let out = {
        let mut s = world.resource_mut::<WebStrava>();
        s.rec.set_route(route);
        match what {
            "start" => s.rec.start(now),
            "stop" => s.rec.stop(now),
            "toggle" => s.rec.toggle(now),
            other => {
                return Err(format!(
                    "expected start, stop, toggle or status, not {other:?}"
                ));
            }
        }
    };
    let saver = world.resource::<ActivitySaver>().clone();
    match out {
        Outcome::Started => {
            let unix_s = unix_now();
            let (place, offset) = place_and_offset(world, unix_s);
            let mut s = world.resource_mut::<WebStrava>();
            let used = s.used.clone();
            let t = Target::new(unix_s, offset, &place, |n| used.contains(&n.to_string()));
            s.used.push(t.folder.clone());
            let label = match (saver.folder)() {
                Some(root) => t.label(&root),
                None => format!("your downloads as {}", t.fit_name()),
            };
            s.rec.set_folder(Some(label.clone()));
            s.target = Some(t);
            Ok(format!("Strava recording started: it goes to {label}"))
        }
        Outcome::Stopped(ride) => {
            let t = world.resource_mut::<WebStrava>().target.take();
            let (Some(ride), Some(t)) = (ride, t) else {
                return Ok("Strava recording too short, not saved".into());
            };
            let path = t.rel(&t.fit_name());
            let fit = ocw_fit::encode(&ride.activity);
            world.resource_mut::<WebStrava>().kept.push(KeptRecording {
                start_unix_s: ride.activity.start_unix_s,
                name: t.fit_name(),
                fit: fit.clone(),
            });
            let answer = (saver.save)(&path, "application/vnd.ant.fit", fit);
            let w = went(&answer, &path);
            info!(file = %path, went = %w, "Strava recording");
            Ok(format!("Strava recording: {w}"))
        }
        Outcome::Unchanged => Ok("unchanged".into()),
    }
}

/// Stops a running recording (the ride ends: `ride.save`).
pub fn stop(world: &mut World) {
    if world
        .get_resource::<WebStrava>()
        .is_some_and(|s| s.rec.is_recording())
        && let Err(e) = record(world, "stop")
    {
        warn!("Strava recording: {e}");
    }
}

/// Feeds the recording with the ride's samples (once a frame; the
/// recorder keeps one a second).
fn observe(time: Res<Time>, session: Option<Res<Session>>, mut s: ResMut<WebStrava>) {
    let Some(session) = session else { return };
    if !s.rec.is_recording() {
        return;
    }
    let c = &session.0.client;
    if c.rider_id().is_none() {
        return;
    }
    let now = time.elapsed_secs_f64();
    s.rec.observe(
        c.track(),
        c.geo_origin(),
        ocw_client::recording::sample(c, now),
        unix_now(),
    );
}

/// S or its pad button.
fn key(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    bindings: Option<Res<ocw_ui::bindings::ActiveBindings>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    mut commands: Commands,
) {
    use ocw_client::bindings::Action;
    let Some(keys) = keys else { return };
    if ocw_render::menu::ride_input(context)
        && (ocw_ui::bindings::key_just_pressed(bindings.as_deref(), Action::Strava, &keys)
            || ocw_ui::bindings::pad_just_pressed(bindings.as_deref(), Action::Strava, pads.iter()))
    {
        commands.queue(|world: &mut World| match record(world, "toggle") {
            Ok(m) => info!("{m}"),
            Err(e) => warn!("{e}"),
        });
    }
}

/// The HUD: the badge and the recording's notice, else the upload's.
fn hud(
    time: Res<Time>,
    s: Res<WebStrava>,
    upload: Res<crate::ui::StravaNotice>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    let rec = s.rec.elapsed_s(now);
    let msg = s.rec.notice(now).map(str::to_string).or_else(|| {
        upload
            .0
            .as_ref()
            .filter(|(_, until)| now < *until)
            .map(|(m, _)| m.clone())
    });
    if view.strava_s != rec {
        view.strava_s = rec;
    }
    if view.strava_message != msg {
        view.strava_message = msg;
    }
}

/// `activities.folder choose|forget|status`.
fn folder(world: &mut World, action: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use crate::browser::{PageFolder, folder_choose, folder_forget, folder_supported};
        if !folder_supported(PageFolder::Activities) {
            return Ok("recordings, photos and screenshots are downloads named OCW … (this browser cannot keep a folder)".into());
        }
        match action {
            "choose" => {
                folder_choose(PageFolder::Activities)?;
                return Ok(
                    "choose the activities folder (an OCW folder is made in it; each recording gets its own folder there)"
                        .into(),
                );
            }
            "forget" => {
                folder_forget(PageFolder::Activities)?;
                return Ok("recordings, photos and screenshots are downloads again".into());
            }
            _ => {}
        }
    }
    let _ = action;
    let saver = world.resource::<ActivitySaver>().clone();
    Ok(match (saver.folder)() {
        Some(f) => format!("everything goes to the folder {f}: each Strava recording in its own folder, photos and screenshots in a folder per day"),
        None => "recordings, photos and screenshots are downloads named OCW … (Captures -> Activities folder chooses the one folder for everything)".into(),
    })
}

pub fn build(app: &mut App) {
    if !app.world().contains_resource::<ActivitySaver>() {
        #[cfg(target_arch = "wasm32")]
        app.insert_resource(ActivitySaver::page());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let d = app.world().resource::<crate::ui::Downloads>().clone();
            app.insert_resource(ActivitySaver::downloads(d));
        }
    }
    app.init_resource::<WebStrava>()
        .add_systems(Update, (key, observe, hud).after(ocw_ui::RideViewSet));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.record")
            .about("Start or stop the Strava recording (key S): its FIT file and photos go to its own folder in the activities folder (or downloads)")
            .arg(Arg::new("what").default_value("toggle").help("start, stop, toggle or status")),
        |world, m| {
            let what = m.get_one::<String>("what").map_or("toggle", String::as_str);
            if what == "status" {
                let now = world.resource::<Time>().elapsed_secs_f64();
                let s = world.resource::<WebStrava>();
                return Ok(match (s.rec.elapsed_s(now), s.rec.folder()) {
                    (Some(e), Some(f)) => format!("Strava recording: {}, going to {f}", ocw_client::history::hms(e)),
                    (Some(e), None) => format!("Strava recording: {}", ocw_client::history::hms(e)),
                    _ => format!("Strava recording: off (S starts it). {}", ocw_client::strava::NOT_SET_UP),
                });
            }
            record(world, what)
        },
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("activities.folder")
            .about("The one folder for everything: the Strava recordings (each its own folder), photos and screenshots (a folder per day). choose (a folder; an OCW folder is made in it), forget (downloads), or nothing to show it")
            .arg(Arg::new("action").value_parser(["choose", "forget", "status"])),
        |world, m| folder(world, m.get_one::<String>("action").map_or("status", String::as_str)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_file_went() {
        assert_eq!(
            went(
                "folder:OCW",
                "2026-11-18 Puerto de la Cruz/OCW 2026-11-18 10-05 Puerto de la Cruz.fit"
            ),
            "Saved to OCW/2026-11-18 Puerto de la Cruz"
        );
        assert_eq!(
            went(
                "download",
                "2026-11-18 Puerto de la Cruz/OCW 2026-11-18 10-05 Puerto de la Cruz photo 1.jpg"
            ),
            "Downloaded OCW 2026-11-18 10-05 Puerto de la Cruz photo 1.jpg"
        );
        assert_eq!(file_part("a/b/c.fit"), "c.fit");
        assert_eq!(file_part("c.fit"), "c.fit");
    }
}
