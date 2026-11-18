//! The Strava recording on the desktop (spec §2.8.1, ST1): S (or its
//! pad button, Ride → Strava recording, `strava.record`) starts and
//! stops it; the part of the ride between S and S is saved as its own FIT
//! activity to `<data dir>/ocw/rides/strava/` and queued with the upload
//! (`strava-sync.toml`, the same thread as the rides). Start and stop each
//! set a lap mark in the ride and show a notice; the HUD shows the badge.
//! Leaving the ride, saving it or quitting stops and saves it. Without the
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

/// The ride's Strava recording.
#[derive(Resource)]
pub struct StravaRec(pub StravaRecording);

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
    app.insert_resource(StravaRec(StravaRecording::new(api, route)))
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
            Ok("Strava recording started".into())
        }
        Outcome::Stopped(ride) => {
            if let Some(mut rec) = world.get_resource_mut::<Recording>() {
                rec.recorder.mark_lap(now);
            }
            let Some(ride) = ride else {
                return Ok("Strava recording too short, not saved".into());
            };
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
            let mut out = format!("Strava recording saved to {}", path.display());
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
