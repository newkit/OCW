//! OCW Studio (`ocw-studio`, spec §14 Phase 16): the editor for models,
//! worlds and routes, its own desktop app (Q1) on the game's crates. This
//! is the shell of stage E1: a world package drawn by the game's renderer
//! (`ocw-render`, exactly as in a ride) in a 3D view flown with the
//! unicorn's controls, a 2D map of the package, the workspaces (World,
//! Routes; Models to come) and the document core `ocw-edit` behind the
//! Edit menu (undo / redo, autosave).
//!
//! - [`world`]: opening a package (verified like the game), the renderer's
//!   world, the camera's start;
//! - [`flight`]: the 3D view's flight (Phase 13's input and flight model);
//! - [`map`]: the 2D map (land use, buildings, roads from the package);
//! - [`info`]: the World workspace's package facts;
//! - [`edit`]: the documents, the commands the UI sends, autosave,
//!   `studio …` console commands;
//! - [`patch`]: the world patch's file and its preview on the package's
//!   road graph (E5 step 2), [`pick`]: clicks to roads, [`overlay`]: the
//!   patch drawn over the 3D view, [`build`]: *Build package* with
//!   `world-builder`;
//! - [`routes`]: the Routes workspace (16D, E1; external planners E2):
//!   the route document, imports with map matching, the built-in planner
//!   and BRouter, exports, *Test ride*;
//! - [`ui`]: the egui panels.

mod build;
mod edit;
mod flight;
mod icon;
mod info;
mod map;
mod overlay;
mod patch;
mod pick;
mod routes;
mod routes_ui;
mod ui;
mod world;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use clap::Parser;
use ocw_render::console::{ConsoleState, Permission};
use ocw_render::hud::VersionLabel;
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_sim::god::FlightModel;

#[derive(Parser, Debug, Clone, Resource)]
#[command(
    version,
    about = "OCW Studio: the editor for models, worlds and routes"
)]
pub struct Args {
    /// The world package to open (a .ocwpkg path or an installed id).
    #[arg(long, value_name = "PATH|ID")]
    pub world: Option<String>,
    /// The workspace to start in.
    #[arg(long, value_enum, default_value = "world")]
    pub workspace: ui::Workspace,
    /// Hide the 2D map.
    #[arg(long)]
    pub no_map: bool,
    /// Fly like the unicorn's plane (default: hover, the camera stays put
    /// without input).
    #[arg(long)]
    pub plane: bool,
    /// Accept unsigned / untrusted packages from the cache too.
    #[arg(long)]
    pub allow_unsigned: bool,
    /// Trust the development root (release builds; debug builds do).
    #[arg(long)]
    pub trust_dev_root: bool,
    /// Extra trusted root key files.
    #[arg(long, value_name = "FILE")]
    pub trust_root: Vec<PathBuf>,
    /// Do not join the installed neighbouring packages.
    #[arg(long)]
    pub no_neighbours: bool,
    /// Window size.
    #[arg(long, default_value = "1600x900", value_parser = parse_size)]
    pub window: (u32, u32),
    /// Console commands run at start, in order (`studio view add Pass`).
    #[arg(long = "cmd", value_name = "COMMAND")]
    pub commands: Vec<String>,
    /// Save a screenshot after `--screenshot-delay` seconds and quit
    /// (automated runs, `drive.sh`).
    #[arg(long, value_name = "PATH")]
    pub screenshot: Option<PathBuf>,
    #[arg(long, default_value_t = 5.0)]
    pub screenshot_delay: f64,
    /// Log the render rate every this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub log_fps: Option<f64>,
    #[arg(long)]
    pub no_vsync: bool,
    /// The BRouter server for planning (`route.planner.brouter_url`; there
    /// is no default, 16H Q3). Saved with the planner settings.
    #[arg(long, value_name = "URL")]
    pub brouter_url: Option<String>,
    /// Where autosaves go (default `<data dir>/ocw/studio/autosave`).
    #[arg(long, value_name = "DIR")]
    pub autosave_dir: Option<PathBuf>,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((
        w.parse().map_err(|_| "bad width")?,
        h.parse().map_err(|_| "bad height")?,
    ))
}

fn main() -> AppExit {
    let args = Args::parse();
    let settings = world::LoadSettings {
        allow_unsigned: args.allow_unsigned,
        trust_dev_root: args.trust_dev_root,
        trust_root: args.trust_root.clone(),
        no_neighbours: args.no_neighbours,
    };
    // The package is loaded before the window opens (a bad path ends here,
    // like the game's `--world`).
    let loaded = args.world.as_deref().map(|spec| {
        world::load(spec, &settings).unwrap_or_else(|e| {
            eprintln!("{spec}: {e}");
            std::process::exit(1);
        })
    });
    let store_dir = args.autosave_dir.clone().or_else(edit::autosave_dir);
    let store = store_dir
        .clone()
        .map(|d| Box::new(ocw_edit::DirStore::new(d)) as Box<dyn ocw_edit::AutosaveStore>);
    // Desktop entry and icons for menus, docks and file managers (not for
    // the headless screenshot runs).
    #[cfg(target_os = "linux")]
    if args.screenshot.is_none() {
        match icon::ICON.install_desktop_integration() {
            Ok(0) => {}
            Ok(n) => info!("desktop integration: {n} files written"),
            Err(e) => warn!("desktop integration: {e}"),
        }
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "OCW Studio".into(),
                    // App id (Wayland) / WM_CLASS (X11): the desktop entry
                    // `ocw-studio.desktop` and its icon.
                    name: Some(icon::APP_ID.into()),
                    resolution: WindowResolution::new(args.window.0, args.window.1),
                    present_mode: if args.no_vsync {
                        PresentMode::AutoNoVsync
                    } else {
                        PresentMode::AutoVsync
                    },
                    ..default()
                }),
                ..default()
            })
            .disable::<bevy::audio::AudioPlugin>(),
    )
    .insert_resource(RideView {
        free_ride: true,
        ..RideView::new(ocw_world::Route::test_hills())
    })
    .insert_resource(VersionLabel(format!(
        "OCW Studio {}",
        env!("CARGO_PKG_VERSION")
    )))
    .insert_resource(ocw_render::frame_rate::LogRenderRate(
        args.log_fps.filter(|s| *s > 0.0),
    ))
    .insert_resource(settings)
    .insert_resource(edit::Studio::new(store, store_dir))
    .insert_resource(ui::UiState {
        workspace: args.workspace,
        show_map: !args.no_map,
        ..Default::default()
    })
    .insert_resource(args.clone())
    .add_plugins(OcwRenderPlugins)
    .add_plugins((
        ocw_render::platform::PlatformPlugin,
        ocw_ui::app_icon::AppIconPlugin(&icon::ICON),
        flight::FlightPlugin,
        edit::EditPlugin,
        overlay::OverlayPlugin,
        ui::UiPlugin,
    ))
    .add_systems(Startup, startup_commands)
    .add_systems(Update, routes::update_routes);
    if let Some(url) = &args.brouter_url {
        app.world_mut()
            .resource_mut::<edit::Studio>()
            .routes
            .settings
            .brouter_url = url.trim().to_string();
    }
    if args.plane {
        app.world_mut()
            .resource_mut::<flight::StudioFlight>()
            .flight
            .model = FlightModel::Plane;
    }
    if let Some(path) = args.screenshot.clone() {
        app.insert_resource(ocw_render::autoshot::AutoScreenshot {
            path,
            delay_s: args.screenshot_delay,
        })
        .add_plugins(ocw_render::autoshot::AutoScreenshotPlugin);
    }
    if let Some(l) = loaded {
        world::show(app.world_mut(), l, 0.0, false);
    }
    app.run()
}

/// The studio is a local tool: every console command is allowed; then the
/// `--cmd` lines.
fn startup_commands(args: Res<Args>, mut console: ResMut<ConsoleState>) {
    console.permission = Permission::Dev;
    for c in &args.commands {
        console.queue(c.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line() {
        let a = Args::parse_from(["ocw-studio"]);
        assert_eq!(a.world, None);
        assert_eq!(a.workspace, ui::Workspace::World);
        assert_eq!(a.window, (1600, 900));
        let a = Args::parse_from([
            "ocw-studio",
            "--world",
            "data/testland.ocwpkg",
            "--workspace",
            "routes",
            "--no-map",
            "--cmd",
            "studio view add Pass",
            "--screenshot",
            "shot.jpg",
        ]);
        assert_eq!(a.world.as_deref(), Some("data/testland.ocwpkg"));
        assert_eq!(a.workspace, ui::Workspace::Routes);
        assert!(a.no_map);
        assert_eq!(a.commands, vec!["studio view add Pass"]);
        assert!(Args::try_parse_from(["ocw-studio", "--window", "big"]).is_err());
    }
}
