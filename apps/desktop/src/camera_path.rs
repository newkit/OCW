//! The camera's extras in the desktop app:
//!
//! - **`camera.shake`** (setting `display.camera_shake`, spec §14 Phase
//!   12C): the chase camera's short shake after a crash or a hard bump; off
//!   for motion comfort (the kiosk's show mode turns it off).
//! - **Guided camera paths** (spec §14 Phase 13 G5, flyover videos for the
//!   art project): `camera.path add` takes the camera's pose now as a
//!   keyframe (`camera.path key X Y Z HEADING PITCH` gives one), `list`,
//!   `remove N`, `clear`, `save FILE` / `load FILE` (the text format of
//!   `ocw_client::camera_path`; a bare name lives in
//!   `<data dir>/ocw/camera-paths/`), `play [--seconds S] [--ease linear|
//!   smooth|in-out]`, `stop`. While a path plays, the camera follows it
//!   ([`ocw_render::RideView::guided_camera`]) and the tiles stream around
//!   it; the pose is a function of the game time since `play`, so a capture
//!   (`ocw-desktop --capture`, a fixed game-time step per frame) renders
//!   the flyover smoothly. The spline itself is pure and shared
//!   (`ocw_client::camera_path`).

use std::path::PathBuf;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::camera_path::{CameraPath, Ease, Playback, Pose};
use ocw_render::RideView;
use ocw_render::camera::FollowCamera;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// `display.camera_shake` (spec §14 Phase 12C): shared with the browser
/// app (`ocw_ui::collisions`, with its command `camera.shake`).
pub use ocw_ui::collisions::CameraShake;

/// The keyframes being edited and the path playing.
#[derive(Resource, Debug, Clone, Default)]
pub struct GuidedPath {
    pub path: CameraPath,
    pub playing: Option<Playback>,
    /// The last pose played and its game time (the streaming's velocity).
    last: Option<(f64, [f64; 3])>,
}

pub struct CameraPathPlugin;

impl Plugin for CameraPathPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<CameraShake>() {
            ocw_ui::collisions::register_camera_shake(app);
        }
        app.init_resource::<GuidedPath>()
            .add_systems(Update, play)
            .console_command(ConsoleCommand::new(
                Permission::User,
                path_command(),
                path_action,
            ))
            // Tab completion (spec §2.9): `load` takes a saved path's name.
            .console_values("camera.path", "args", |_, q| {
                if q.before.is_empty() && q.words.get(1).is_some_and(|a| a == "load") {
                    saved_paths()
                } else {
                    Vec::new()
                }
            });
    }
}

fn path_command() -> Command {
    Command::new("camera.path")
        .about("Guided camera path for flyover videos: add (the camera now), key X Y Z HEADING PITCH, list, remove N, clear, save FILE, load FILE, play [--seconds S] [--ease linear|smooth|in-out], stop")
        .arg(
            Arg::new("action")
                .required(true)
                .value_parser([
                    "add", "key", "list", "remove", "clear", "save", "load", "play", "stop",
                ]),
        )
        .arg(
            Arg::new("args")
                .num_args(0..)
                .allow_hyphen_values(true)
                .help("key: X Y Z HEADING PITCH (m, degrees); remove: N; save / load: FILE"),
        )
        .arg(
            Arg::new("seconds")
                .long("seconds")
                .value_parser(value_parser!(f64))
                .help("play: how long the flight takes (default: the file's, else 20 s)"),
        )
        .arg(
            Arg::new("ease")
                .long("ease")
                .value_parser(Ease::NAMES)
                .help("play: linear, smooth (default) or in-out"),
        )
}

/// The file of `camera.path save / load NAME`: a bare name (no directory)
/// in `<data dir>/ocw/camera-paths/` with `.campath` added when it has no
/// extension; a path as given.
pub fn path_file(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(name);
    if p.components().count() > 1 || p.is_absolute() {
        return Some(p);
    }
    let mut file = crate::ride::data_dir()?.join("camera-paths").join(&p);
    if file.extension().is_none() {
        file.set_extension("campath");
    }
    Some(file)
}

/// The names of the saved camera paths (`camera.path load NAME`).
fn saved_paths() -> Vec<String> {
    let Some(dir) = crate::ride::data_dir().map(|d| d.join("camera-paths")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "campath" {
                return None;
            }
            Some(p.file_stem()?.to_string_lossy().into_owned())
        })
        .collect()
}

/// The follow camera's pose now.
fn camera_pose(world: &mut World) -> Option<Pose> {
    let mut q = world.query_filtered::<&Transform, With<FollowCamera>>();
    let t = q.iter(world).next()?;
    let f = t.forward();
    Some(Pose {
        eye: [
            f64::from(t.translation.x),
            f64::from(t.translation.y),
            f64::from(t.translation.z),
        ],
        look: [f64::from(f.x), f64::from(f.y), f64::from(f.z)],
    })
}

fn path_action(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let action = m.get_one::<String>("action").expect("required").as_str();
    let args: Vec<String> = m
        .get_many::<String>("args")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let now = world.resource::<Time>().elapsed_secs_f64();
    let pose = if action == "add" {
        Some(camera_pose(world).ok_or("no camera")?)
    } else {
        None
    };
    let mut g = world.resource_mut::<GuidedPath>();
    match action {
        "add" => {
            g.path.keys.push(pose.expect("taken above"));
            Ok(format!("key {} added", g.path.keys.len()))
        }
        "key" => {
            let v: Vec<f64> = args
                .iter()
                .map(|a| {
                    a.parse::<f64>()
                        .map_err(|_| format!("'{a}' is not a number"))
                })
                .collect::<Result<_, _>>()?;
            let [x, y, z, heading, pitch] = v[..] else {
                return Err("key X Y Z HEADING PITCH (metres, degrees)".into());
            };
            g.path.keys.push(Pose {
                eye: [x, y, z],
                look: Pose::look_of(heading.to_radians(), pitch.to_radians()),
            });
            Ok(format!("key {} added", g.path.keys.len()))
        }
        "list" if g.path.keys.is_empty() => Ok("no keys (camera.path add)".into()),
        "list" => Ok(g.path.list().join("\n")),
        "remove" => {
            let n: usize = args
                .first()
                .and_then(|a| a.parse().ok())
                .ok_or("remove N (1 = the first key)")?;
            if n == 0 || n > g.path.keys.len() {
                return Err(format!("no key {n} ({} keys)", g.path.keys.len()));
            }
            g.path.keys.remove(n - 1);
            Ok(format!("key {n} removed, {} left", g.path.keys.len()))
        }
        "clear" => {
            g.path = CameraPath::default();
            g.playing = None;
            Ok("camera path cleared".into())
        }
        "save" => {
            let name = args.first().ok_or("save FILE")?;
            let file = path_file(name).ok_or("no data folder")?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&file, g.path.to_text())
                .map_err(|e| format!("{}: {e}", file.display()))?;
            Ok(format!(
                "{} keys saved to {}",
                g.path.keys.len(),
                file.display()
            ))
        }
        "load" => {
            let name = args.first().ok_or("load FILE")?;
            let file = path_file(name).ok_or("no data folder")?;
            let text =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            g.path = CameraPath::parse(&text)?;
            Ok(format!(
                "{} keys loaded from {}",
                g.path.keys.len(),
                file.display()
            ))
        }
        "play" => {
            let seconds = m.get_one::<f64>("seconds").copied();
            if seconds.is_some_and(|s| s <= 0.0) {
                return Err("--seconds must be positive".into());
            }
            let ease = m.get_one::<String>("ease").and_then(|e| Ease::parse(e));
            let play = g
                .path
                .play(now, seconds, ease)
                .ok_or("no keys to play (camera.path add)")?;
            let line = format!(
                "playing {} keys in {:.1} s ({})",
                g.path.keys.len(),
                play.seconds,
                play.ease.name()
            );
            g.playing = Some(play);
            g.last = None;
            Ok(line)
        }
        "stop" => {
            let was = g.playing.take().is_some();
            Ok(if was { "stopped" } else { "not playing" }.into())
        }
        _ => unreachable!("clap checks the actions"),
    }
}

/// Puts the camera on the path while it plays (the game time since
/// `play`); afterwards back to the chase camera.
fn play(time: Res<Time>, mut g: ResMut<GuidedPath>, mut view: ResMut<RideView>) {
    let now = time.elapsed_secs_f64();
    let pose = g.playing.as_ref().and_then(|p| p.pose(now));
    if pose.is_none() && g.playing.is_some() {
        info!("camera path played");
        g.playing = None;
    }
    let guided = pose.map(|p| {
        let velocity = match g.last {
            Some((t, e)) if now > t => {
                [(p.eye[0] - e[0]) / (now - t), (p.eye[2] - e[2]) / (now - t)]
            }
            _ => [0.0, 0.0],
        };
        ocw_render::GuidedCamera {
            eye: p.eye,
            look: p.look,
            velocity,
        }
    });
    g.last = pose.map(|p| (now, p.eye));
    if view.guided_camera != guided {
        view.guided_camera = guided;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
    use std::time::Duration;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )))
            .add_plugins(CameraPathPlugin);
        app.world_mut().spawn((
            Transform::from_xyz(10.0, 20.0, 30.0).looking_to(Vec3::NEG_Z, Vec3::Y),
            FollowCamera,
        ));
        app.update();
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

    #[test]
    fn keys_are_added_listed_saved_loaded_and_played() {
        let mut app = app();
        assert!(run(&mut app, "camera.path play").contains("no keys"));
        assert!(run(&mut app, "camera.path add").contains("key 1 added"));
        assert!(run(&mut app, "camera.path key 110 60 -70 90 -10").contains("key 2 added"));
        assert!(run(&mut app, "camera.path key 1 2").starts_with("error"));
        let list = {
            run(&mut app, "camera.path list");
            app.world()
                .resource::<ConsoleState>()
                .log()
                .to_vec()
                .join("\n")
        };
        assert!(
            list.contains("key 1: x 10.0 y 20.0 z 30.0, heading 0°"),
            "{list}"
        );
        assert!(
            list.contains("key 2: x 110.0 y 60.0 z -70.0, heading 90°"),
            "{list}"
        );
        // Save and load a file (a path as given).
        let dir = std::env::temp_dir().join(format!("ocw-campath-{}", std::process::id()));
        let file = dir.join("flyover.campath");
        let line = format!("camera.path save {}", file.display());
        assert!(run(&mut app, &line).contains("2 keys saved"), "{line}");
        run(&mut app, "camera.path clear");
        assert!(app.world().resource::<GuidedPath>().path.keys.is_empty());
        let line = format!("camera.path load {}", file.display());
        assert!(run(&mut app, &line).contains("2 keys loaded"));
        let _ = std::fs::remove_dir_all(&dir);
        // Play over 1 s (ten frames of 0.1 s of game time).
        assert!(
            run(&mut app, "camera.path play --seconds 1 --ease linear")
                .contains("in 1.0 s (linear)")
        );
        // The camera follows from the next frame on.
        app.update();
        let first = app.world().resource::<RideView>().guided_camera.unwrap();
        assert!(first.eye[0] >= 10.0 && first.eye[0] < 40.0, "{first:?}");
        let mut xs = vec![first.eye[0]];
        for _ in 0..7 {
            app.update();
            xs.push(
                app.world()
                    .resource::<RideView>()
                    .guided_camera
                    .unwrap()
                    .eye[0],
            );
        }
        // Evenly on its way (linear), the streaming knows the speed.
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
        let g = app.world().resource::<RideView>().guided_camera.unwrap();
        assert!(g.velocity[0] > 50.0, "{g:?}");
        // Over: back to the chase camera.
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().resource::<RideView>().guided_camera.is_none());
        assert!(app.world().resource::<GuidedPath>().playing.is_none());
        assert!(run(&mut app, "camera.path remove 3").starts_with("error"));
        assert!(run(&mut app, "camera.path remove 1").contains("1 left"));
    }

    #[test]
    fn the_shake_switch_and_the_files() {
        let mut app = app();
        assert!(app.world().resource::<CameraShake>().0);
        assert_eq!(run(&mut app, "camera.shake off"), "camera shake off");
        assert!(!app.world().resource::<CameraShake>().0);
        assert_eq!(run(&mut app, "camera.shake"), "camera shake on");
        // A bare name goes to the data folder with the extension added.
        if let Some(f) = path_file("alps") {
            assert!(f.ends_with("camera-paths/alps.campath"), "{}", f.display());
        }
        assert_eq!(path_file("/tmp/x.txt"), Some(PathBuf::from("/tmp/x.txt")));
    }
}
