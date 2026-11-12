//! Building a ride from its options (spec §3.2.1, M3): the route, the
//! terrain around a route file, the world package with its installed
//! neighbours and road network, the free-ride start or the route on the
//! road graph. The same code runs at program start (main thread, errors
//! end the program) and for an in-app switch (`ride.switch`, on a
//! background thread; errors keep the current ride). No Bevy world is
//! touched here: the result is plain data that `ride::apply` inserts.

use std::sync::Arc;

use ocw_world::{ElevationGrid, RoadNetwork, Route};

use super::choice::RideChoice;
use crate::Args;
use crate::connection::FreeRideSetup;
use crate::worlds::{self, WorldInfo};

/// Everything a new ride session needs, loaded.
pub struct RideSetup {
    pub choice: RideChoice,
    /// The route (in free ride: a placeholder, riders ride their paths).
    pub route: Route,
    /// Real terrain around a route file.
    pub terrain_base: Option<ElevationGrid>,
    /// Free ride or a route on the road graph.
    pub free: Option<FreeRideSetup>,
    pub world: Option<WorldSetup>,
}

/// The world package of a ride.
pub struct WorldSetup {
    pub info: WorldInfo,
    pub render: ocw_render::package::PackageWorld,
    /// The road network of the package and its neighbours (packages with a
    /// road graph only): free ride, routes, GPX import.
    pub net: Option<Arc<RoadNetwork>>,
    /// The package's routes (section `ROUT`): name and length (m).
    pub routes: Vec<(String, f64)>,
    /// (lat, lon) of the package's local metres.
    pub origin: (f64, f64),
}

/// A built-in test route by name.
pub fn test_route(name: &str) -> Option<Route> {
    match name {
        "hills" => Some(Route::test_hills()),
        "loop" => Some(Route::test_loop()),
        _ => None,
    }
}

/// Loads the ride `args` describes.
pub fn build(args: &Args) -> Result<RideSetup, String> {
    let choice = RideChoice::from_args(args);
    let (route, terrain_base) = match &args.route_file {
        Some(path) => load_route_file(path)?,
        None => match test_route(args.route_name()) {
            Some(r) => (r, None),
            None if args.world.is_some() => (Route::test_hills(), None),
            None => {
                return Err(format!(
                    "--route {}: not a test route (hills, loop); package routes need --world",
                    args.route_name()
                ));
            }
        },
    };
    let mut setup = RideSetup {
        choice,
        route,
        terrain_base,
        free: None,
        world: None,
    };
    let Some(spec) = &args.world else {
        return Ok(setup);
    };
    let loaded = worlds::try_load(args, spec)?;
    let hood = worlds::try_neighbourhood(args, spec, &loaded)?;
    let info = WorldInfo::of(spec, &loaded, &hood);
    let pkg = &loaded.package;
    let net = hood
        .set
        .graph()
        .cloned()
        .map(|graph| Arc::new(RoadNetwork::new(graph).with_package_name(hood.set.name())));
    // `--world` alone: the package's first route, else a free ride.
    let world_alone = args.world_alone();
    let alone;
    let args = if world_alone {
        // On a remote server: a free ride, as `ocw-server --world` alone.
        let first = match &args.server {
            Some(_) => None,
            None => pkg.routes.first().map(|r| r.name.as_str()),
        };
        let (resolved, note) = world_alone_ride(args, first, net.is_some());
        println!("{note}");
        alone = resolved;
        &alone
    } else {
        args
    };
    setup.choice = RideChoice::from_args(args);
    if args.free_ride || args.graph_route() {
        let Some(net) = &net else {
            return Err(format!(
                "{spec}: no road graph in this world package; rebuild it with `world-builder package` (format OCWPKG02)"
            ));
        };
        let free = match free_ride_setup(args, pkg, net.clone()) {
            // The package's own first route does not fit its graph: a free
            // ride rather than no ride.
            Err(e) if world_alone && args.route.is_some() => {
                eprintln!("{e}; free ride instead");
                let mut fallback = args.clone();
                fallback.route = None;
                fallback.free_ride = true;
                setup.choice = RideChoice::from_args(&fallback);
                free_ride_setup(&fallback, pkg, net.clone())?
            }
            other => other?,
        };
        setup.free = Some(free);
    }
    let routes = pkg
        .routes
        .iter()
        .map(|r| (r.name.clone(), graph_route_length_m(r)))
        .collect();
    let origin = pkg.origin;
    setup.world = Some(WorldSetup {
        info,
        render: worlds::package_world(args, hood),
        net,
        routes,
        origin,
    });
    Ok(setup)
}

/// What `--world` without a route option rides ([`Args::world_alone`]):
/// the package's first route (section `ROUT`), else a free ride; a
/// package without a road graph keeps the default test route. The ride
/// options for it and a line for the log.
pub fn world_alone_ride(args: &Args, first_route: Option<&str>, has_graph: bool) -> (Args, String) {
    let mut a = args.clone();
    let note = match (has_graph, first_route) {
        (false, _) => format!(
            "--world without a route option: the package has no road graph, riding the test route {}",
            args.route_name()
        ),
        (true, Some(name)) => {
            a.route = Some(name.to_string());
            format!(
                "--world without a route option: riding the package's first route {name:?} (--free-ride for a free ride)"
            )
        }
        (true, None) => {
            a.free_ride = true;
            "--world without a route option: the package has no routes, free ride".to_string()
        }
    };
    (a, note)
}

/// The length of a stored route (sum of its edge steps).
pub fn graph_route_length_m(r: &ocw_world::GraphRoute) -> f64 {
    r.steps
        .iter()
        .map(|s| match s {
            ocw_world::RouteStep::Edge { from_m, to_m, .. } => (to_m - from_m).abs(),
            ocw_world::RouteStep::Turnaround => 0.0,
        })
        .sum()
}

/// Free ride: where to start on the road network (joined with the
/// neighbours, spec §2.2.2), or the route to follow on it.
fn free_ride_setup(
    args: &Args,
    pkg: &ocw_world::WorldPackage,
    net: Arc<RoadNetwork>,
) -> Result<FreeRideSetup, String> {
    if args.graph_route() {
        let plan = graph_route_plan(args, pkg, &net)?;
        println!(
            "route {}: {:.1} km, {} steps{}",
            plan.name,
            plan.length_m() / 1000.0,
            plan.steps.len(),
            if plan.is_loop { ", a loop" } else { "" }
        );
        let (edge, offset_m, forward) = plan.start();
        return Ok(FreeRideSetup {
            origin: pkg.origin,
            net,
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
            plan: Some(Arc::new(plan)),
        });
    }
    let start = match args.start {
        Some((x, z)) => net
            .nearest([x, z])
            .map(|(edge, offset)| (edge, offset, net.edge(edge).oneway.allows(true))),
        None => {
            let random = args
                .random_start
                .unwrap_or(args.screenshot.is_none() && args.capture.is_none());
            let seed = if random {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64)
            } else {
                1
            };
            net.random_start(seed)
        }
    };
    let (edge, offset_m, forward) = start.ok_or("the road graph is empty")?;
    let e = net.edge(edge);
    println!(
        "free ride from {} ({:?}), {:.0} m along edge {}",
        e.name.as_deref().unwrap_or("unnamed road"),
        e.class,
        offset_m,
        e.id.0
    );
    Ok(FreeRideSetup {
        origin: pkg.origin,
        net,
        start: ocw_sim::FreeRideStart {
            edge,
            offset_m,
            forward,
        },
        plan: None,
    })
}

/// Matches a GPX file onto `net` (spec §2.7): the plan and the report.
pub fn match_gpx_file(
    path: &std::path::Path,
    net: &RoadNetwork,
    origin: (f64, f64),
) -> Result<ocw_world::matching::MatchReport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let gpx = ocw_world::gpx::Gpx::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = gpx.name.clone().unwrap_or_else(|| {
        path.file_stem()
            .map_or("GPX route".into(), |s| s.to_string_lossy().into_owned())
    });
    ocw_world::matching::match_gpx(
        net,
        &gpx,
        origin,
        &name,
        &ocw_world::matching::MatchParams::default(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// The route to ride on the road graph: a GPX file matched now, an
/// `ocw-graph-route` file, or a route of the package by name.
fn graph_route_plan(
    args: &Args,
    pkg: &ocw_world::WorldPackage,
    net: &RoadNetwork,
) -> Result<ocw_world::RoutePlan, String> {
    if let Some(path) = &args.route_gpx {
        let report = match_gpx_file(path, net, pkg.origin)?;
        println!("{}", report.summary());
        return Ok(report.plan);
    }
    let file = std::path::Path::new(args.route_name());
    if file.is_file() {
        return read_graph_route(file, net, &pkg.name);
    }
    let route = pkg.route(args.route_name()).ok_or_else(|| {
        let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
        format!(
            "no route {:?} in {} (it has: {}); add one with `world-builder gpx`",
            args.route_name(),
            pkg.name,
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        )
    })?;
    route
        .resolve(net)
        .map_err(|e| format!("route {:?} does not fit the road graph: {e}", route.name))
}

/// An `ocw-graph-route` file resolved on `net` (package `pkg_name`).
pub fn read_graph_route(
    file: &std::path::Path,
    net: &RoadNetwork,
    pkg_name: &str,
) -> Result<ocw_world::RoutePlan, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let (route, package) =
        ocw_world::GraphRoute::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some((name, hash)) = package
        && hash != net.fingerprint()
    {
        eprintln!(
            "{}: made for {name} ({hash:016x}), this package is {pkg_name} ({:016x}); trying the stable edge IDs",
            file.display(),
            net.fingerprint()
        );
    }
    route.resolve(net).map_err(|e| {
        format!(
            "{}: the route does not fit this package's road graph: {e} (re-match it with world-builder gpx)",
            file.display()
        )
    })
}

/// Loads an ocw-route file: the route and the terrain around it.
pub fn load_route_file(path: &std::path::Path) -> Result<(Route, Option<ElevationGrid>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file =
        ocw_world::RouteFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let route = Route::from_points(
        file.name.clone(),
        file.name.clone(),
        file.points.clone(),
        false,
        file.width_m,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?
    .with_surfaces(file.surfaces.clone())
    .with_geo_origin(file.origin);
    Ok((route, file.grid))
}

#[cfg(test)]
mod tests {
    use super::super::choice::WorldMode;
    use super::*;
    use clap::Parser;

    fn args(v: &[&str]) -> Args {
        Args::parse_from(std::iter::once("ocw-desktop").chain(v.iter().copied()))
    }

    #[test]
    fn test_routes_need_no_files() {
        let s = build(&args(&["--route", "loop"])).unwrap();
        assert_eq!(s.choice, RideChoice::TestRoute("loop".into()));
        assert_eq!(s.route.name(), Route::test_loop().name());
        assert!(s.free.is_none() && s.world.is_none() && s.terrain_base.is_none());
    }

    #[test]
    fn bad_rides_are_errors_not_exits() {
        assert!(
            build(&args(&["--route", "Valley"]))
                .err()
                .unwrap()
                .contains("not a test route")
        );
        let missing = std::env::temp_dir().join("ocw-no-such-file.route");
        assert!(build(&args(&["--route-file", missing.to_str().unwrap()])).is_err());
        let missing = std::env::temp_dir().join("ocw-no-such-world.ocwpkg");
        assert!(
            build(&args(&[
                "--world",
                missing.to_str().unwrap(),
                "--free-ride"
            ]))
            .is_err()
        );
    }

    /// Regression: `--world PKG` without a route option rode the built-in
    /// hills test route through the package's terrain (the camera inside a
    /// hillside). Now it rides the package's first route, else a free
    /// ride; any route option keeps winning (the kiosk adds its own
    /// --route-file to a --world from the defaults file).
    #[test]
    fn a_world_alone_rides_its_first_route_or_free() {
        let alone = args(&["--world", "w.ocwpkg"]);
        assert!(alone.world_alone());
        assert_eq!(
            RideChoice::from_args(&alone),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::TestRoute("hills".into())
            },
            "before the package is loaded"
        );
        let (a, note) = world_alone_ride(&alone, Some("Vaduz - Malbun"), true);
        assert!(a.graph_route() && !a.free_ride);
        assert_eq!(a.route.as_deref(), Some("Vaduz - Malbun"));
        assert!(note.contains("first route \"Vaduz - Malbun\""), "{note}");
        assert_eq!(
            RideChoice::from_args(&a),
            RideChoice::World {
                world: "w.ocwpkg".into(),
                mode: WorldMode::Route("Vaduz - Malbun".into())
            }
        );
        let (a, note) = world_alone_ride(&alone, None, true);
        assert!(a.free_ride && !a.graph_route(), "{note}");
        assert!(note.contains("free ride"), "{note}");
        // Without a road graph: the test route as before.
        let (a, note) = world_alone_ride(&alone, Some("R"), false);
        assert!(
            !a.free_ride && !a.graph_route() && a.route.is_none(),
            "{note}"
        );
        // Any ride option is not "alone".
        for more in [
            &["--route-file", "r.route"][..],
            &["--route", "hills"],
            &["--route", "Valley"],
            &["--route-gpx", "t.gpx"],
            &["--free-ride"],
            &["--start", "1,2"],
        ] {
            let v: Vec<&str> = ["--world", "w.ocwpkg"]
                .iter()
                .chain(more)
                .copied()
                .collect();
            assert!(!args(&v).world_alone(), "{more:?}");
        }
        // Without --world the default test route.
        assert_eq!(
            RideChoice::from_args(&args(&[])),
            RideChoice::TestRoute("hills".into())
        );
    }

    #[test]
    fn stored_route_length_sums_the_edges() {
        let r = ocw_world::GraphRoute {
            name: "r".into(),
            steps: vec![
                ocw_world::RouteStep::Edge {
                    id: ocw_world::graph::EdgeId(1),
                    from_m: 0.0,
                    to_m: 120.0,
                },
                ocw_world::RouteStep::Turnaround,
                ocw_world::RouteStep::Edge {
                    id: ocw_world::graph::EdgeId(1),
                    from_m: 120.0,
                    to_m: 20.0,
                },
            ],
            is_loop: false,
        };
        assert_eq!(graph_route_length_m(&r), 220.0);
    }
}
