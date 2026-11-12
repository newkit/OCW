//! What is ridden (spec §3.2.1, M3), Bevy-free: a test route, a route
//! file, or a world package in free ride / on one of its routes. A choice
//! comes from the command line ([`RideChoice::from_args`]), from the
//! console command `ride.switch` ([`RideChoice::from_switch`]) or a menu
//! row ([`RideChoice::to_line`]), and turns back into the ride options of
//! [`Args`] ([`RideChoice::apply_to`]) for the setup at the start and the
//! in-app switch alike.

use std::path::PathBuf;

use crate::Args;

/// The built-in test routes.
pub const TEST_ROUTES: [&str; 2] = ["hills", "loop"];

/// What to ride.
#[derive(Debug, Clone, PartialEq)]
pub enum RideChoice {
    /// A built-in test route (`--route hills | loop`).
    TestRoute(String),
    /// A route file from `world-builder route` (`--route-file`).
    RouteFile(PathBuf),
    /// A world package (`--world PATH | ID`).
    World { world: String, mode: WorldMode },
}

/// How a world package is ridden.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldMode {
    /// Free ride on the road graph (`--free-ride`), from the road nearest
    /// local X,Z (`--start`) or a random one.
    FreeRide { start: Option<(f64, f64)> },
    /// A route of the package by name, or an `ocw-graph-route` file
    /// (`--route NAME | FILE`), on the road graph.
    Route(String),
    /// A GPX file matched onto the road graph at the start (`--route-gpx`).
    Gpx(PathBuf),
    /// A route file with the package's terrain (`--route-file`).
    RouteFile(PathBuf),
    /// A test route with the package loaded (`--world` without a mode).
    TestRoute(String),
}

impl RideChoice {
    /// The ride the command line asks for.
    pub fn from_args(args: &Args) -> Self {
        let Some(world) = args.world.clone() else {
            return match &args.route_file {
                Some(f) => Self::RouteFile(f.clone()),
                None => Self::TestRoute(args.route.clone()),
            };
        };
        let mode = if let Some(gpx) = &args.route_gpx {
            WorldMode::Gpx(gpx.clone())
        } else if args.graph_route() {
            WorldMode::Route(args.route.clone())
        } else if args.free_ride {
            WorldMode::FreeRide { start: args.start }
        } else if let Some(f) = &args.route_file {
            WorldMode::RouteFile(f.clone())
        } else {
            WorldMode::TestRoute(args.route.clone())
        };
        Self::World { world, mode }
    }

    /// Sets the ride options of `args` to this choice (all others stay:
    /// bots, trust flags, recording, …).
    pub fn apply_to(&self, args: &mut Args) {
        args.route = "hills".into();
        args.route_gpx = None;
        args.route_file = None;
        args.world = None;
        args.free_ride = false;
        args.start = None;
        match self {
            Self::TestRoute(name) => args.route.clone_from(name),
            Self::RouteFile(f) => args.route_file = Some(f.clone()),
            Self::World { world, mode } => {
                args.world = Some(world.clone());
                match mode {
                    WorldMode::FreeRide { start } => {
                        args.free_ride = true;
                        args.start = *start;
                    }
                    WorldMode::Route(r) => args.route.clone_from(r),
                    WorldMode::Gpx(g) => args.route_gpx = Some(g.clone()),
                    WorldMode::RouteFile(f) => args.route_file = Some(f.clone()),
                    WorldMode::TestRoute(r) => args.route.clone_from(r),
                }
            }
        }
    }

    /// The choice of a `ride.switch` command: `--route NAME | FILE`,
    /// `--route-file FILE`, `--world PKG [--free-ride] [--start X,Z]`,
    /// `--world PKG --route NAME | FILE`, `--world PKG --route-gpx FILE`.
    pub fn from_switch(s: &SwitchArgs) -> Result<Self, String> {
        let Some(world) = s.world.clone() else {
            if s.free_ride || s.start.is_some() || s.route_gpx.is_some() {
                return Err("--free-ride, --start and --route-gpx need --world".into());
            }
            return match (&s.route_file, &s.route) {
                (Some(_), Some(_)) => Err("give --route or --route-file, not both".into()),
                (Some(f), None) => Ok(Self::RouteFile(f.clone())),
                (None, Some(r)) if TEST_ROUTES.contains(&r.as_str()) => {
                    Ok(Self::TestRoute(r.clone()))
                }
                (None, Some(r)) => Err(format!(
                    "--route {r}: not a test route ({}); package routes need --world",
                    TEST_ROUTES.join(", ")
                )),
                (None, None) => Err("what to ride: --route, --route-file or --world".into()),
            };
        };
        let given = [
            s.free_ride || s.start.is_some(),
            s.route.is_some(),
            s.route_gpx.is_some(),
            s.route_file.is_some(),
        ];
        if given.iter().filter(|g| **g).count() > 1 {
            return Err(
                "with --world give one of --free-ride / --start, --route, --route-gpx, --route-file"
                    .into(),
            );
        }
        let mode = if let Some(r) = &s.route {
            if TEST_ROUTES.contains(&r.as_str()) {
                WorldMode::TestRoute(r.clone())
            } else {
                WorldMode::Route(r.clone())
            }
        } else if let Some(g) = &s.route_gpx {
            WorldMode::Gpx(g.clone())
        } else if let Some(f) = &s.route_file {
            WorldMode::RouteFile(f.clone())
        } else {
            // A world alone is a free ride.
            WorldMode::FreeRide { start: s.start }
        };
        Ok(Self::World { world, mode })
    }

    /// The `ride.switch` command line of this choice (menu rows run it).
    pub fn to_line(&self) -> String {
        let q = |s: &str| format!("\"{s}\"");
        let p = |p: &PathBuf| q(&p.display().to_string());
        match self {
            Self::TestRoute(r) => format!("ride.switch --route {r}"),
            Self::RouteFile(f) => format!("ride.switch --route-file {}", p(f)),
            Self::World { world, mode } => {
                let w = q(world);
                match mode {
                    WorldMode::FreeRide { start: None } => {
                        format!("ride.switch --world {w} --free-ride")
                    }
                    WorldMode::FreeRide {
                        start: Some((x, z)),
                    } => format!("ride.switch --world {w} --free-ride --start {x:.0},{z:.0}"),
                    WorldMode::Route(r) => format!("ride.switch --world {w} --route {}", q(r)),
                    WorldMode::Gpx(g) => format!("ride.switch --world {w} --route-gpx {}", p(g)),
                    WorldMode::RouteFile(f) => {
                        format!("ride.switch --world {w} --route-file {}", p(f))
                    }
                    WorldMode::TestRoute(r) => format!("ride.switch --world {w} --route {r}"),
                }
            }
        }
    }

    /// A short description for notices: "Loading <describe> ...".
    pub fn describe(&self) -> String {
        let file = |p: &PathBuf| {
            p.file_stem()
                .map_or_else(|| p.display().to_string(), |s| s.to_string_lossy().into())
        };
        match self {
            Self::TestRoute(r) => format!("test route {r}"),
            Self::RouteFile(f) => format!("route {}", file(f)),
            Self::World { world, mode } => {
                let w = world_name(world);
                match mode {
                    WorldMode::FreeRide { .. } => format!("{w}, free ride"),
                    WorldMode::Route(r) => format!("{w}, route {}", file(&PathBuf::from(r))),
                    WorldMode::Gpx(g) => format!("{w}, GPX {}", file(g)),
                    WorldMode::RouteFile(f) => format!("{w}, route {}", file(f)),
                    WorldMode::TestRoute(r) => format!("{w}, test route {r}"),
                }
            }
        }
    }
}

/// A world spec for people: the file stem of a path, the id of a cache id
/// (`europe/liechtenstein@3` → `europe/liechtenstein`).
pub fn world_name(spec: &str) -> String {
    let p = std::path::Path::new(spec);
    if p.extension().is_some_and(|e| e == "ocwpkg") {
        return p
            .file_stem()
            .map_or(spec.into(), |s| s.to_string_lossy().into());
    }
    spec.split('@').next().unwrap_or(spec).to_string()
}

/// The options of `ride.switch` (parsed by clap in the console command).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SwitchArgs {
    pub route: Option<String>,
    pub route_file: Option<PathBuf>,
    pub route_gpx: Option<PathBuf>,
    pub world: Option<String>,
    pub free_ride: bool,
    pub start: Option<(f64, f64)>,
}

impl SwitchArgs {
    /// Parses the words of a `ride.switch` line (without the command name).
    #[cfg(test)]
    pub fn parse(words: &[&str]) -> Result<Self, String> {
        let m = command()
            .try_get_matches_from(std::iter::once("ride.switch").chain(words.iter().copied()))
            .map_err(|e| e.to_string())?;
        Ok(Self::from_matches(&m))
    }

    pub fn from_matches(m: &clap::ArgMatches) -> Self {
        Self {
            route: m.get_one::<String>("route").cloned(),
            route_file: m.get_one::<PathBuf>("route-file").cloned(),
            route_gpx: m.get_one::<PathBuf>("route-gpx").cloned(),
            world: m.get_one::<String>("world").cloned(),
            free_ride: m.get_flag("free-ride"),
            start: m.get_one::<(f64, f64)>("start").copied(),
        }
    }
}

/// The clap definition of `ride.switch`.
pub fn command() -> clap::Command {
    use clap::{Arg, ArgAction, Command, value_parser};
    Command::new("ride.switch")
        .about("Ride something else without restarting: a test route, a route file or a world package (free ride or one of its routes); the ride so far is saved")
        .arg(
            Arg::new("route")
                .long("route")
                .value_name("NAME|FILE")
                .help("Test route (hills, loop); with --world a route of the package or an ocw-graph-route file"),
        )
        .arg(
            Arg::new("route-file")
                .long("route-file")
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .help("A route file from world-builder route"),
        )
        .arg(
            Arg::new("route-gpx")
                .long("route-gpx")
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .help("A GPX file, matched onto the road graph of --world"),
        )
        .arg(
            Arg::new("world")
                .long("world")
                .value_name("PATH|ID")
                .help("World package: a file or a cache id"),
        )
        .arg(
            Arg::new("free-ride")
                .long("free-ride")
                .action(ArgAction::SetTrue)
                .help("Free ride on the package's road graph (the default with --world alone)"),
        )
        .arg(
            Arg::new("start")
                .long("start")
                .value_name("X,Z")
                .allow_hyphen_values(true)
                .value_parser(crate::parse_xz)
                .help("Free ride start: the road nearest local X,Z metres"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(v: &[&str]) -> Args {
        Args::parse_from(std::iter::once("ocw-desktop").chain(v.iter().copied()))
    }

    fn switch(v: &[&str]) -> Result<RideChoice, String> {
        RideChoice::from_switch(&SwitchArgs::parse(v)?)
    }

    #[test]
    fn the_command_line_is_a_choice() {
        assert_eq!(
            RideChoice::from_args(&args(&[])),
            RideChoice::TestRoute("hills".into())
        );
        assert_eq!(
            RideChoice::from_args(&args(&["--route-file", "r.route"])),
            RideChoice::RouteFile("r.route".into())
        );
        assert_eq!(
            RideChoice::from_args(&args(&[
                "--world",
                "w.ocwpkg",
                "--free-ride",
                "--start",
                "-5,7"
            ])),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::FreeRide {
                    start: Some((-5.0, 7.0))
                }
            }
        );
        assert_eq!(
            RideChoice::from_args(&args(&["--world", "w.ocwpkg", "--route", "Valley"])),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::Route("Valley".into())
            }
        );
        assert_eq!(
            RideChoice::from_args(&args(&["--world", "w.ocwpkg", "--route-gpx", "a.gpx"])),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::Gpx("a.gpx".into())
            }
        );
    }

    #[test]
    fn a_choice_round_trips_through_args_and_its_command_line() {
        let choices = [
            RideChoice::TestRoute("loop".into()),
            RideChoice::RouteFile("/data/a b.route".into()),
            RideChoice::World {
                world: "europe/example@2".into(),
                mode: WorldMode::FreeRide { start: None },
            },
            RideChoice::World {
                world: "/var/tmp/w.ocwpkg".into(),
                mode: WorldMode::FreeRide {
                    start: Some((-1200.0, 530.0)),
                },
            },
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::Route("Valley Loop".into()),
            },
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::Gpx("t.gpx".into()),
            },
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::RouteFile("r.route".into()),
            },
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::TestRoute("hills".into()),
            },
        ];
        for c in choices {
            // Into the options of a running app (other options stay).
            let mut a = args(&["--bots", "3", "--world", "old.ocwpkg", "--free-ride"]);
            c.apply_to(&mut a);
            assert_eq!(a.bots, 3);
            assert_eq!(RideChoice::from_args(&a), c, "{c:?}");
            // Through the console line of a menu row.
            let line = c.to_line();
            let words = ocw_render::console::split_line(&line);
            assert_eq!(words[0], "ride.switch");
            let w: Vec<&str> = words[1..].iter().map(String::as_str).collect();
            assert_eq!(switch(&w).unwrap(), c, "{line}");
        }
    }

    #[test]
    fn switch_options_are_checked() {
        assert_eq!(
            switch(&["--world", "w.ocwpkg"]).unwrap(),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::FreeRide { start: None }
            },
            "a world alone is a free ride"
        );
        assert_eq!(
            switch(&["--world", "w.ocwpkg", "--start", "3,4"]).unwrap(),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::FreeRide {
                    start: Some((3.0, 4.0))
                }
            }
        );
        for bad in [
            &[][..],
            &["--route", "Valley"],
            &["--free-ride"],
            &["--start", "1,2"],
            &["--route", "hills", "--route-file", "r.route"],
            &["--world", "w", "--free-ride", "--route", "Valley"],
            &["--world", "w", "--route-gpx", "a.gpx", "--route-file", "r"],
            &["--world", "w", "--start", "1;2"],
            &["--bots", "3"],
        ] {
            assert!(switch(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn descriptions_name_the_world_and_mode() {
        assert_eq!(world_name("/var/tmp/ocw/example.ocwpkg"), "example");
        assert_eq!(world_name("europe/example@3"), "europe/example");
        assert_eq!(
            switch(&[
                "--world",
                "/x/example.ocwpkg",
                "--route",
                "/r/Up Hill.ocw-graph-route"
            ])
            .unwrap()
            .describe(),
            "example, route Up Hill"
        );
        assert_eq!(
            RideChoice::TestRoute("loop".into()).describe(),
            "test route loop"
        );
    }
}
