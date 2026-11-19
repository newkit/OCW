//! The start choice of the desktop app (spec 3.2.4 Q3, Q5): at every start
//! that the command line does not decide, the menu opens on *Connect to a
//! server* / *Ride locally* with the last choice focused, so Enter takes
//! it. *Connect to a server* opens Ride → Join (the address field and the
//! recent servers of `ocw.toml [servers]`); *Ride locally* closes the menu
//! and rides on with the local rider in Training mode (every bike and all
//! gear, no hearts; Campaign mode is postponed, see
//! [`ocw_client::menu::start::LocalMode`]). The choice is kept in
//! `ocw.toml [start] choice`.
//!
//! Never asked when an option already decides ([`decided_by`]): a server,
//! a join link, a world or route, screenshots and captures, scripted
//! commands, the kiosk / show mode – so kiosks and tests never stop at the
//! question. `--start-choice ask|local|server` decides explicitly.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::menu::model::Action;
use ocw_client::menu::pages::Page as SubPage;
use ocw_client::menu::start::{Choice, LocalMode};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_ui::menu::MenuState;

use crate::Args;
use crate::config::{self, ConfigFiles};

/// The option that decides the start, if one does (`None`: ask).
pub fn decided_by(args: &Args) -> Option<&'static str> {
    let flags: [(bool, &'static str); 17] = [
        (args.server.is_some(), "--server"),
        (args.join.is_some(), "--join"),
        (args.world.is_some(), "--world"),
        (args.route.is_some(), "--route"),
        (args.route_file.is_some(), "--route-file"),
        (args.route_gpx.is_some(), "--route-gpx"),
        (args.free_ride, "--free-ride"),
        (args.start.is_some(), "--start"),
        (args.workout.is_some(), "--workout"),
        (args.screenshot.is_some(), "--screenshot"),
        (args.capture.is_some(), "--capture"),
        // The kiosk's show mode (scripts/kiosk-show.args).
        (args.no_hearts, "--no-hearts"),
        (args.fetch_world.is_some(), "--fetch-world"),
        (args.list_worlds, "--list-worlds"),
        // Scripted runs drive the app themselves.
        (!args.commands.is_empty(), "--cmd"),
        (!args.exec.is_empty(), "--exec"),
        (!args.timed.is_empty(), "--at"),
    ];
    flags.into_iter().find(|(on, _)| *on).map(|(_, name)| name)
}

/// What the start does: ask, or one choice right away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    Ask,
    Go(Choice),
}

/// `--start-choice`, else ask unless an option decides (then local, as
/// before).
pub fn start_mode(args: &Args) -> StartMode {
    match args.start_choice.as_deref() {
        Some("ask") => StartMode::Ask,
        Some(c) => StartMode::Go(Choice::parse(c).unwrap_or_default()),
        None if decided_by(args).is_some() => StartMode::Go(Choice::Local),
        None => StartMode::Ask,
    }
}

/// The last choice and where it is kept.
#[derive(Resource, Debug, Clone)]
pub struct StartChoice {
    pub last: Choice,
    pub files: Option<ConfigFiles>,
    /// The mode of *Ride locally* (Training mode; Campaign mode later).
    pub local_mode: LocalMode,
}

impl StartChoice {
    fn choose(&mut self, c: Choice) -> Result<(), String> {
        self.last = c;
        match &self.files {
            Some(f) => config::save_start_choice(f, c),
            None => Ok(()),
        }
    }
}

/// The start choice: the page's rows come from `ocw_client::menu::start`
/// (the desktop's menu hooks), this plugin opens it and runs its lines.
#[derive(Default)]
pub struct StartPlugin {
    /// Overrides [`start_mode`] (tests).
    pub mode: Option<StartMode>,
}

impl Plugin for StartPlugin {
    fn build(&self, app: &mut App) {
        let files = app.world().get_resource::<ConfigFiles>().cloned();
        let last = files
            .as_ref()
            .and_then(config::load_start_choice)
            .unwrap_or_default();
        let mode = self.mode.unwrap_or_else(|| {
            app.world()
                .get_resource::<Args>()
                .map_or(StartMode::Go(Choice::Local), start_mode)
        });
        app.insert_resource(StartChoice {
            last,
            files,
            local_mode: LocalMode::Training,
        })
        .insert_resource(OpenAtStart(mode))
        .add_systems(Startup, open_at_start);
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("start.choose")
                .about("The start choice: connect to a server (Ride -> Join) or ride locally (Training mode); kept for the next start")
                .arg(
                    Arg::new("choice")
                        .required(true)
                        .value_parser(["server", "local"]),
                ),
            |world, m| {
                let c = Choice::parse(m.get_one::<String>("choice").expect("required"))
                    .expect("checked");
                let mut s = world.resource_mut::<StartChoice>();
                let saved = s.choose(c);
                let mode = s.local_mode;
                let mut out = match c {
                    Choice::Server => {
                        "Connect to a server: type its address (Enter), paste a link, or pick a recent server".to_string()
                    }
                    Choice::Local => format!(
                        "riding locally as {} ({}: every bike and all gear, no hearts)",
                        crate::settings::local_rider(world).name,
                        mode.name()
                    ),
                };
                if let Err(e) = saved {
                    out.push_str(&format!(" (choice not saved: {e})"));
                }
                Ok(out)
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("start.ask")
                .about("Show the start choice (Connect to a server / Ride locally) again"),
            |world, _| {
                open(world);
                Ok(String::new())
            },
        ));
    }
}

#[derive(Resource)]
struct OpenAtStart(StartMode);

/// Opens the page with the last choice focused (Enter takes it).
fn open(world: &mut World) {
    let row = world.resource::<StartChoice>().last.row();
    let mut m = world.resource_mut::<MenuState>();
    m.queue(Action::OpenSub(SubPage::Start));
    m.queue(Action::Hover(row));
}

fn open_at_start(world: &mut World) {
    match world.resource::<OpenAtStart>().0 {
        StartMode::Ask => {
            info!("start choice: asking (connect to a server or ride locally)");
            open(world);
        }
        StartMode::Go(Choice::Server) => {
            world
                .resource_mut::<MenuState>()
                .queue(Action::OpenSub(SubPage::Join));
        }
        StartMode::Go(Choice::Local) => {}
    }
}

/// The rows of the page (for the desktop's menu hooks).
pub fn rows(world: &World) -> Vec<ocw_client::menu::pages::Row> {
    let last = world
        .get_resource::<StartChoice>()
        .map(|s| s.last)
        .unwrap_or_default();
    ocw_client::menu::start::rows(last, &crate::settings::local_rider(world).name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(v: &[&str]) -> Args {
        Args::parse_from(std::iter::once("ocw-desktop").chain(v.iter().copied()))
    }

    #[test]
    fn a_plain_start_asks_and_deciding_options_skip_the_question() {
        assert_eq!(start_mode(&args(&[])), StartMode::Ask);
        // Settings that do not decide where to ride still ask.
        assert_eq!(
            start_mode(&args(&[
                "--fullscreen",
                "--name",
                "Ada Rider",
                "--bots",
                "3"
            ])),
            StartMode::Ask
        );
        for deciding in [
            &["--server", "127.0.0.1:47800"][..],
            &["--world", "w.ocwpkg"],
            &["--route", "loop"],
            &["--screenshot", "s.jpg"],
            &["--capture", "frames"],
            // The kiosk's show mode (scripts/kiosk-show.args).
            &["--virtual", "--power", "500", "--no-hearts"],
            &["--cmd", "hud on"],
            &["--at", "2=hud on"],
            &["--join", "ocw://127.0.0.1:47800"],
        ] {
            let a = args(deciding);
            assert!(decided_by(&a).is_some(), "{deciding:?}");
            assert_eq!(start_mode(&a), StartMode::Go(Choice::Local), "{deciding:?}");
        }
        // Explicitly.
        assert_eq!(
            start_mode(&args(&["--start-choice", "ask", "--screenshot", "s.jpg"])),
            StartMode::Ask
        );
        assert_eq!(
            start_mode(&args(&["--start-choice", "server"])),
            StartMode::Go(Choice::Server)
        );
        assert!(Args::try_parse_from(["ocw-desktop", "--start-choice", "later"]).is_err());
    }

    /// The kiosk's default arguments never stop at the question.
    #[test]
    fn the_kiosk_show_arguments_decide() {
        let text = include_str!("../../../scripts/kiosk-show.args");
        let words = crate::argsfile::split(text).unwrap();
        let argv: Vec<String> = std::iter::once("ocw-desktop".to_string())
            .chain(words)
            .collect();
        let a = Args::parse_from(argv);
        assert_ne!(start_mode(&a), StartMode::Ask);
    }
}
