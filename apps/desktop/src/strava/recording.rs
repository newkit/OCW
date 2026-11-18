//! The Strava recording on the desktop (spec §2.8.1, ST1): S (or its
//! pad button, Ride → Strava recording, `strava.record`) starts and
//! stops it; the part of the ride between S and S is saved as its own FIT
//! activity to `<data dir>/ocw/rides/strava/` and queued with the upload
//! (`strava-sync.toml`, the same thread as the rides). Start and stop each
//! set a lap mark in the ride and show a notice; the HUD shows the badge.
//! Leaving the ride, saving it or quitting stops and saves it. Every
//! recording is also written **always** into its own folder in the
//! activities folder (`~/Activities/OCW/2026-11-18 Puerto de la Cruz/`,
//! spec §2.8.1 *Where an S recording is written*, [`crate::activities`]):
//! the folder is made at the start (named by the date and the place it
//! starts at), the photographer's photos go there while it runs, the FIT
//! file at its stop; the stop notice names it. Without the
//! API set-up (`strava.toml`) S records the same and its stop points to
//! the simple upload in the browser (`ocw_client::strava::NOT_SET_UP`,
//! spec §2.8.1 *Simple upload*); `--strava-dry-run` (test runs) opens no
//! file manager or browser for that upload.

use std::path::PathBuf;

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::strava::{Outcome, StravaRecording};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::connection::ClientState;
use crate::recording::Recording;

/// The ride's Strava recording and, while it runs, its folder in the
/// activities folder (spec §2.8.1 *Where an S recording is written*).
#[derive(Resource)]
pub struct StravaRec(
    pub StravaRecording,
    pub Option<crate::activities::RecordingDir>,
);

/// `<rides>/strava`: the Strava recordings' FIT files.
pub fn strava_dir(rides: &std::path::Path) -> PathBuf {
    rides.join("strava")
}

pub(super) fn build(app: &mut App, api: bool) {
    let route = app
        .world()
        .get_resource::<ClientState>()
        .map(|c| c.0.course_name().to_string())
        .unwrap_or_default();
    app.insert_resource(StravaRec(StravaRecording::new(api, route), None))
        .add_systems(Update, (strava_key, show_in_hud))
        .add_systems(Last, stop_on_exit);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.record")
            .about("Start or stop the Strava recording (key S): the part between start and stop is its own Strava activity")
            .arg(Arg::new("what").default_value("toggle").help("start, stop, toggle or status")),
        |world, m| {
            let what = m.get_one::<String>("what").map_or("toggle", String::as_str);
            let now = crate::recording::game_now(world);
            match what {
                "status" => {
                    let s = &world.resource::<StravaRec>().0;
                    return Ok(match s.elapsed_s(now) {
                        Some(e) => format!("Strava recording: {}", ocw_client::history::hms(e)),
                        None if s.api => "Strava recording: off (S starts it; uploaded by your API app)".into(),
                        None => format!(
                            "Strava recording: off (S starts it). {}",
                            ocw_client::strava::NOT_SET_UP
                        ),
                    });
                }
                "start" | "stop" | "toggle" => {}
                other => return Err(format!("expected start, stop, toggle or status, not {other:?}")),
            }
            let out = {
                let mut s = world.resource_mut::<StravaRec>();
                match what {
                    "start" => s.0.start(now),
                    "stop" => s.0.stop(now),
                    _ => s.0.toggle(now),
                }
            };
            handle(world, out, now)
        },
    ));
}

/// What a start or stop does in the app: the ride's lap mark, the save
/// and the upload; the console's answer.
pub fn handle(world: &mut World, out: Outcome, now: f64) -> Result<String, String> {
    match out {
        Outcome::Started => {
            if let Some(mut rec) = world.get_resource_mut::<Recording>() {
                rec.recorder.mark_lap(now);
            }
            // Its own folder in the activities folder, named by the date
            // and the place it starts at.
            let dir = world
                .get_resource::<crate::activities::ActivitiesFolder>()
                .cloned()
                .map(|root| {
                    let unix_s = crate::recording::unix_now();
                    let (place, offset) = crate::activities::place_and_offset(world, unix_s);
                    crate::activities::RecordingDir::create(&root, unix_s, offset, &place)
                });
            let mut out = "Strava recording started".to_string();
            let dir = match dir {
                Some(Ok(d)) => {
                    out.push_str(&format!(", writing to {}", d.dir.display()));
                    Some(d)
                }
                Some(Err(e)) => {
                    warn!("Strava recording's folder: {e}");
                    None
                }
                None => None,
            };
            let mut s = world.resource_mut::<StravaRec>();
            s.0.set_folder(dir.as_ref().map(|d| d.label.clone()));
            s.1 = dir;
            Ok(out)
        }
        Outcome::Stopped(ride) => {
            if let Some(mut rec) = world.get_resource_mut::<Recording>() {
                rec.recorder.mark_lap(now);
            }
            let folder = world
                .get_resource_mut::<StravaRec>()
                .and_then(|mut s| s.1.take());
            let Some(mut ride) = ride else {
                if let Some(f) = &folder {
                    f.discard();
                }
                return Ok("Strava recording too short, not saved".into());
            };
            // Always into its folder in the activities folder (with the
            // photos taken while it ran) …
            let mut copy = None;
            if let Some(f) = &folder {
                match f.write_fit(&ocw_fit::encode(&ride.activity)) {
                    Ok(p) => {
                        info!(path = %p.display(), "Strava recording written to its folder");
                        ride.meta.folder = Some(p.to_string_lossy().into_owned());
                        copy = Some(p);
                    }
                    Err(e) => error!("Strava recording's folder: {e}"),
                }
            }
            // … and the history's own record (the API sync's file).
            let dir = world
                .get_resource::<Recording>()
                .and_then(|r| r.dir.clone())
                .ok_or("no data directory for rides")?;
            let path = crate::recording::save_ride(&strava_dir(&dir), &ride)?;
            info!(path = %path.display(), "Strava recording saved");
            let api = world
                .get_resource::<super::Strava>()
                .is_some_and(|s| s.api());
            if let Some(s) = world.get_resource::<super::Strava>() {
                s.ride_saved();
            }
            let mut out = format!(
                "Strava recording saved to {}",
                copy.as_deref().unwrap_or(&path).display()
            );
            if !api {
                out.push_str(&format!(". {}", ocw_client::strava::NOT_SET_UP));
            }
            Ok(out)
        }
        Outcome::Unchanged => Ok("unchanged".into()),
    }
}

/// Stops the recording (the ride ends: saved, switched, quit) and saves it.
pub fn stop(world: &mut World) {
    let now = crate::recording::game_now(world);
    let Some(mut s) = world.get_resource_mut::<StravaRec>() else {
        return;
    };
    if !s.0.is_recording() {
        return;
    }
    let out = s.0.stop(now);
    if let Err(e) = handle(world, out, now) {
        error!("Strava recording: {e}");
    }
}

fn strava_key(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    bindings: Option<Res<ocw_ui::bindings::ActiveBindings>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    mut commands: Commands,
) {
    let Some(keys) = keys else {
        return;
    };
    if !ocw_render::menu::ride_input(context) {
        return;
    }
    use ocw_client::bindings::Action;
    let key = ocw_ui::bindings::key_just_pressed(bindings.as_deref(), Action::Strava, &keys);
    let pad = ocw_ui::bindings::pad_just_pressed(bindings.as_deref(), Action::Strava, pads.iter());
    if key || pad {
        commands.queue(|world: &mut World| {
            let now = crate::recording::game_now(world);
            let out = world.resource_mut::<StravaRec>().0.toggle(now);
            match handle(world, out, now) {
                Ok(m) => info!("{m}"),
                Err(e) => info!("{e}"),
            }
        });
    }
}

fn show_in_hud(time: Res<Time>, s: Res<StravaRec>, view: Option<ResMut<RideView>>) {
    let Some(mut view) = view else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let rec = s.0.elapsed_s(now);
    let msg = s.0.notice(now).map(str::to_string);
    if view.strava_s != rec {
        view.strava_s = rec;
    }
    if view.strava_message != msg {
        view.strava_message = msg;
    }
}

fn stop_on_exit(mut exit: MessageReader<AppExit>, mut commands: Commands) {
    if exit.read().next().is_some() {
        commands.queue(stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activities::ActivitiesFolder;
    use ocw_client::recording::{RideRecorder, Sample};
    use ocw_world::Route;

    fn world(name: &str) -> (World, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("ocw-strava-folder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut w = World::new();
        w.insert_resource(Recording {
            recorder: RideRecorder::new("Loop"),
            dir: Some(base.join("data/rides")),
            enabled: true,
            hearts_start: None,
            workout: None,
        });
        w.insert_resource(ActivitiesFolder {
            dir: base.join("home/Activities/OCW"),
            label: "Activities/OCW".into(),
        });
        w.insert_resource(StravaRec(StravaRecording::new(false, "Loop"), None));
        (w, base)
    }

    fn feed(w: &mut World, from: i64, n: i64) {
        let route = Route::test_loop();
        let mut s = w.resource_mut::<StravaRec>();
        for i in 0..n {
            s.0.observe(
                &route,
                None,
                Sample {
                    game_s: i as f64,
                    distance: i as f64 * 8.0,
                    speed: 8.0,
                    power_w: 200.0,
                    ..Sample::default()
                },
                from + i,
            );
        }
    }

    /// S starts: the recording's folder is made in the activities folder;
    /// a photo while it runs goes there (once, its sidecar pointing
    /// there, the history finding it); S stops: the FIT file is written
    /// there too, the internal record keeps its path, the notice names the
    /// folder. The end of a ride stops and writes a running one; a second
    /// recording that day gets ` 2`; a too-short one leaves no folder.
    #[test]
    fn a_recording_is_written_to_its_folder_on_stop_and_at_ride_end() {
        let (mut w, base) = world("stop");
        let root = w.resource::<ActivitiesFolder>().dir.clone();
        let out = w.resource_mut::<StravaRec>().0.start(0.0);
        let said = handle(&mut w, out, 0.0).unwrap();
        assert!(said.contains("writing to"), "{said}");
        let rec = w.resource::<StravaRec>().1.clone().expect("its folder");
        assert!(rec.dir.is_dir() && rec.dir.starts_with(&root));
        // No route, world or places here: "Ride"; the world's local date.
        assert!(
            rec.target.folder.ends_with(" Ride"),
            "{}",
            rec.target.folder
        );
        assert!(rec.label.starts_with("Activities/OCW/20"), "{}", rec.label);
        // A photo while it runs.
        let (photo, went) = crate::photos::recording_target(&mut w).unwrap();
        assert_eq!(photo.parent(), Some(rec.dir.as_path()));
        assert!(photo.to_string_lossy().ends_with(" Ride photo 1.jpg"));
        assert_eq!(went, format!("Saved to {}", rec.label));
        let time = ocw_render::media::MediaTime {
            unix_s: 1_794_992_710,
            offset_s: 0,
        };
        let rides = base.join("data/rides");
        let side = crate::photos::photos_dir(&rides, "ride-1");
        let m = crate::photos::tests::meta(
            1,
            photo.file_name().unwrap().to_string_lossy().into_owned(),
            Some(photo.to_string_lossy().into_owned()),
            &time,
        );
        crate::photos::write_photo(
            &photo,
            Some(&side),
            &[90u8; 16 * 9 * 3],
            16,
            9,
            &m,
            &time,
            "OCW",
        )
        .unwrap();
        let found = crate::photos::ride_photo_files(&rides, "ride-1");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, photo, "the history and the sync find it there");
        assert_eq!(
            crate::photos::ride_photo_folder(&rides, "ride-1"),
            Some(rec.dir.clone())
        );
        // S stops.
        feed(&mut w, 1_794_992_700, 30);
        let out = w.resource_mut::<StravaRec>().0.stop(30.0);
        let said = handle(&mut w, out, 30.0).unwrap();
        let fit = rec.dir.join(rec.target.fit_name());
        assert!(said.contains(&fit.display().to_string()), "{said}");
        assert!(fit.is_file());
        let notice = w
            .resource::<StravaRec>()
            .0
            .notice(30.0)
            .unwrap()
            .to_string();
        assert!(
            notice.starts_with(&format!("Strava recording saved to {}: 0:30", rec.label)),
            "{notice}"
        );
        assert!(w.resource::<StravaRec>().1.is_none());
        // The internal record (history, API sync) points to it.
        let internal = std::fs::read_dir(strava_dir(&rides))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "fit"))
            .unwrap();
        assert_eq!(
            std::fs::read(&internal).unwrap(),
            std::fs::read(&fit).unwrap()
        );
        assert_eq!(
            crate::recording::load_meta(&internal).unwrap().folder,
            Some(fit.to_string_lossy().into_owned())
        );
        // In the folder: the FIT file and the photo, each once.
        let mut names: Vec<String> = std::fs::read_dir(&rec.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n.ends_with(" Ride.fit")), "{names:?}");
        assert!(
            names.iter().any(|n| n.ends_with(" Ride photo 1.jpg")),
            "{names:?}"
        );
        // A second recording that day, ended by the end of the ride.
        let out = w.resource_mut::<StravaRec>().0.start(40.0);
        handle(&mut w, out, 40.0).unwrap();
        let second = w.resource::<StravaRec>().1.clone().unwrap();
        assert_eq!(second.target.folder, format!("{} 2", rec.target.folder));
        feed(&mut w, 1_794_992_800, 10);
        crate::recording::save_now(&mut w, 1e9).unwrap();
        assert!(!w.resource::<StravaRec>().0.is_recording());
        assert!(
            second.dir.join(second.target.fit_name()).is_file(),
            "written at the ride's end"
        );
        // Too short: no folder left.
        let out = w.resource_mut::<StravaRec>().0.start(60.0);
        handle(&mut w, out, 60.0).unwrap();
        let short = w.resource::<StravaRec>().1.clone().unwrap();
        stop(&mut w);
        assert!(!short.dir.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
