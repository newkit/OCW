//! VR in the desktop app (spec §14 Phase 11, X1): the headset as a
//! viewing device of the ride. `--vr` (or the setting `vr.enabled`, the
//! Graphics page's switch) starts an OpenXR session before the renderer
//! (the `vr` build: `cargo build -p ocw-desktop --features vr`); without a
//! runtime or headset – or in a build without VR – the app runs as before
//! and says why. `--vr-sim` renders both eyes from a simulated head on the
//! monitor (no headset, any build: screenshots, tests).
//!
//! Riding stays on the trainer, the keyboard and the gamepad. The console's
//! `vr status | recentre | mirror [eye|both|chase] | reconnect`; `C`
//! recentres.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::vr::{VrConfig, VrMirror, VrPlugin, VrState};

use crate::Args;

/// What [`prepare`] found.
pub enum Start {
    Off,
    /// `--vr-sim`.
    Sim,
    /// A headset, booted.
    #[cfg(feature = "vr")]
    Headset(Box<ocw_render::vr::openxr::Boot>),
    /// VR was asked for and cannot run: why.
    Unavailable(String),
}

/// Before the app is built: boots OpenXR when VR is wanted.
pub fn prepare(args: &Args) -> Start {
    if args.vr_sim {
        return Start::Sim;
    }
    if !args.vr {
        return Start::Off;
    }
    #[cfg(feature = "vr")]
    {
        let opts = ocw_render::vr::openxr::BootOptions::default();
        match ocw_render::vr::openxr::boot(&opts) {
            Ok(b) => {
                info!("VR: {}", ocw_render::vr::openxr::describe(&b.info));
                Start::Headset(Box::new(b))
            }
            Err(e) => Start::Unavailable(format!("VR is off: {e}")),
        }
    }
    #[cfg(not(feature = "vr"))]
    Start::Unavailable(format!("VR is off: {}", ocw_xr::session::XrError::NotBuilt))
}

impl Start {
    /// The renderer for a headset: Bevy draws with the device OpenXR made.
    pub fn render_plugin(&mut self) -> Option<bevy::render::RenderPlugin> {
        #[cfg(feature = "vr")]
        if let Start::Headset(b) = self {
            return Some(bevy::render::RenderPlugin {
                render_creation: std::mem::take(&mut b.creation),
                ..default()
            });
        }
        None
    }

    /// A headset paces the frames (`xrWaitFrame`), not the monitor.
    pub fn paced_by_headset(&self) -> bool {
        #[cfg(feature = "vr")]
        if matches!(self, Start::Headset(_)) {
            return true;
        }
        false
    }
}

/// Why VR does not run (the message on the monitor).
#[derive(Resource, Debug, Clone)]
pub struct VrUnavailable(pub String);

/// After the app's plugins: the VR plugins and the console command.
pub fn add(app: &mut App, start: Start) {
    register_console(app);
    match start {
        Start::Off => {}
        Start::Sim => {
            app.insert_resource(VrConfig::sim()).add_plugins(VrPlugin);
            // Both eyes on the monitor, unless the setting says otherwise.
            app.insert_resource(VrMirror::Both);
            vr_quality(app);
        }
        #[cfg(feature = "vr")]
        Start::Headset(b) => {
            let b = *b;
            app.insert_resource(VrConfig {
                mode: ocw_render::vr::VrMode::Headset,
                eye_px: UVec2::new(b.info.eye_px.0, b.info.eye_px.1),
                format: ocw_xr_openxr::vulkan::wgpu_format(b.info.format),
                ..VrConfig::sim()
            })
            .add_plugins((VrPlugin, b.plugin));
            vr_quality(app);
        }
        Start::Unavailable(why) => {
            eprintln!("{why}");
            warn!("{why}");
            app.insert_resource(VrUnavailable(why))
                .add_systems(Startup, show_unavailable)
                .add_systems(Update, fade_unavailable);
        }
    }
}

/// `display.quality auto` is the `vr` preset while VR runs.
fn vr_quality(app: &mut App) {
    app.insert_resource(ocw_ui::quality::QualityContext {
        vr: true,
        ..default()
    });
}

fn register_console(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("vr")
            .about("VR (spec §14 Phase 11): status, recentre (key C), mirror [eye|both|chase] (what the monitor shows), reconnect")
            .arg(
                Arg::new("action")
                    .value_parser(["status", "recentre", "mirror", "reconnect"])
                    .default_value("status"),
            )
            .arg(Arg::new("value").value_parser(VrMirror::CHOICES.to_vec())),
        |world, m| {
            let action = m.get_one::<String>("action").map_or("status", String::as_str);
            vr_command(world, action, m.get_one::<String>("value").map(String::as_str))
        },
    ));
}

fn vr_command(world: &mut World, action: &str, value: Option<&str>) -> Result<String, String> {
    let running = world.contains_resource::<VrConfig>();
    if !running {
        let why = world
            .get_resource::<VrUnavailable>()
            .map(|u| u.0.clone())
            .unwrap_or_else(|| {
                "VR is off: start with --vr (or set vr.enabled on and restart), or --vr-sim".into()
            });
        return match action {
            "reconnect" if world.contains_resource::<VrUnavailable>() => {
                world.resource_mut::<ConsoleState>().queue("reset");
                Ok("restarting to connect the headset".into())
            }
            // The setting is saved; it applies when VR runs.
            "mirror" if value.is_some() => Ok(format!(
                "mirror: {} (when VR runs)",
                value.unwrap_or_default()
            )),
            _ => Ok(why),
        };
    }
    match action {
        "status" => {
            let cfg = world.resource::<VrConfig>().clone();
            let mirror = *world.resource::<VrMirror>();
            Ok(world.resource::<VrState>().status(&cfg, mirror))
        }
        "recentre" => {
            world.resource_mut::<VrState>().recentre_now = true;
            Ok("recentred: the head is the rider's eye on the bike now".into())
        }
        "mirror" => {
            let Some(v) = value else {
                return Ok(format!("mirror: {}", world.resource::<VrMirror>().name()));
            };
            let m = VrMirror::parse(v).ok_or("eye, both or chase")?;
            world.insert_resource(m);
            Ok(format!("mirror: {}", m.name()))
        }
        "reconnect" => {
            let lost_for_good = world
                .resource::<VrState>()
                .session
                .as_ref()
                .is_some_and(|s| {
                    matches!(s.phase(), ocw_xr::session::Phase::Lost { restart: true })
                });
            if lost_for_good {
                world.resource_mut::<ConsoleState>().queue("reset");
                Ok("restarting to connect the headset again".into())
            } else {
                Ok("the session reconnects by itself (every 2 s while lost)".into())
            }
        }
        _ => Err("status, recentre, mirror or reconnect".into()),
    }
}

#[derive(Component)]
struct UnavailableNote(f32);

/// How long the message stays (s).
const NOTE_S: f32 = 20.0;

fn show_unavailable(mut commands: Commands, why: Res<VrUnavailable>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(48),
                left: percent(10),
                right: percent(10),
                padding: UiRect::axes(px(14), px(8)),
                border_radius: BorderRadius::all(px(6)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.35, 0.05, 0.05, 0.8)),
            UnavailableNote(NOTE_S),
        ))
        .with_child((
            Text::new(why.0.clone()),
            TextFont {
                font_size: 17.0.into(),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.95, 0.9)),
        ));
}

fn fade_unavailable(
    mut commands: Commands,
    time: Res<Time>,
    mut notes: Query<(Entity, &mut UnavailableNote)>,
) {
    for (e, mut n) in &mut notes {
        n.0 -= time.delta_secs();
        if n.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;

    fn app(start: Start) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin));
        add(&mut app, start);
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log().to_vec();
        let at = log.iter().rposition(|l| l == &format!("> {line}")).unwrap();
        log[at + 1..].join("\n")
    }

    #[test]
    fn vr_is_off_unless_asked_for() {
        let a = Args::parse_from(["ocw-desktop"]);
        assert!(matches!(prepare(&a), Start::Off));
        let a = Args::parse_from(["ocw-desktop", "--vr-sim"]);
        assert!(matches!(prepare(&a), Start::Sim));
        assert!(Args::try_parse_from(["ocw-desktop", "--vr", "--vr-sim"]).is_err());
    }

    /// No runtime on the build machines (and no VR in the plain build):
    /// `--vr` gives a message and the app runs on.
    #[test]
    fn without_a_runtime_the_app_runs_with_a_message() {
        let a = Args::parse_from(["ocw-desktop", "--vr"]);
        let mut start = prepare(&a);
        match &start {
            Start::Unavailable(why) => {
                assert!(why.starts_with("VR is off: "), "{why}");
                if cfg!(not(feature = "vr")) {
                    assert!(why.contains("--features vr"), "{why}");
                }
            }
            // A machine with a runtime and a headset (not the build machines).
            _ => return,
        }
        assert!(start.render_plugin().is_none());
        assert!(!start.paced_by_headset());
        let mut app = app(start);
        app.update();
        assert!(app.world().contains_resource::<VrUnavailable>());
        let s = run(&mut app, "vr status");
        assert!(s.starts_with("VR is off: "), "{s}");
        let s = run(&mut app, "vr mirror both");
        assert!(s.contains("when VR runs"), "{s}");
    }

    #[test]
    fn console_commands_without_vr() {
        let mut app = app(Start::Off);
        let s = run(&mut app, "vr");
        assert!(s.contains("start with --vr"), "{s}");
        let s = run(&mut app, "vr recentre");
        assert!(s.contains("--vr-sim"), "{s}");
    }

    #[test]
    fn console_commands_with_the_simulated_head() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin));
        register_console(&mut app);
        app.insert_resource(VrConfig::sim())
            .init_resource::<VrState>()
            .insert_resource(VrMirror::Both);
        let s = run(&mut app, "vr status");
        assert!(s.contains("simulated head"), "{s}");
        let s = run(&mut app, "vr mirror chase");
        assert_eq!(s, "mirror: chase");
        assert_eq!(*app.world().resource::<VrMirror>(), VrMirror::Chase);
        let s = run(&mut app, "vr mirror");
        assert_eq!(s, "mirror: chase");
        run(&mut app, "vr recentre");
        assert!(app.world().resource::<VrState>().recentre_now);
        let s = run(&mut app, "vr reconnect");
        assert!(s.contains("reconnects by itself"), "{s}");
    }
}
