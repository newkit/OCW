//! Headless game server.
//!
//! Runs the authoritative session with test riders at the fixed tick rate and
//! logs their progress. With `--listen` it also accepts desktop clients over
//! QUIC (spec §6.2; `ocw-desktop --server ADDR`).

use std::thread;
use std::time::{Duration, Instant};

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use ocw_core::{SIM_DT, SIM_TICK_HZ, mps_to_kmh};
use ocw_server::net::{DEFAULT_PORT, NetServer, ServerIdentity};
use ocw_server::{Course, GameServer, ServerConfig};
use ocw_world::{RoadNetwork, Route};

#[derive(Parser)]
#[command(version, about = "Open Cycling World headless game server")]
struct Args {
    /// Number of server-side test riders.
    #[arg(long, default_value_t = 10)]
    bots: u32,
    /// Stop after this many seconds of simulated time (0 = run until killed).
    #[arg(long, default_value_t = 0.0)]
    seconds: f64,
    /// Simulate as fast as possible instead of in real time.
    #[arg(long)]
    fast: bool,
    /// Free ride on the road graph of this world package (version 2,
    /// from `world-builder package`; a path or a cache id) instead of the
    /// test loop. Verified on load (spec §4.7).
    #[arg(long, value_name = "PATH|ID")]
    world: Option<String>,
    /// Accept cached world packages that are not signed by a trusted key.
    #[arg(long)]
    allow_unsigned: bool,
    /// Also trust this root public key file (repeatable).
    #[arg(long, value_name = "FILE")]
    trust_root: Vec<PathBuf>,
    /// Seed of the free-ride start point.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Ride this route of the world package (`world-builder gpx` / `route
    /// --world`) instead of a free ride.
    #[arg(long, value_name = "NAME", requires = "world")]
    route: Option<String>,
    /// Ride the world package alone, without the installed neighbouring
    /// packages its roads continue into (spec §2.2.2).
    #[arg(long, requires = "world")]
    no_neighbours: bool,
    /// Test route without --world: loop (gentle) or hills (steep). Clients
    /// must ride the same one (`ocw-desktop --route loop|hills`).
    #[arg(long, default_value = "loop", value_parser = ["loop", "hills"], conflicts_with = "world")]
    test_route: String,
    /// Accept desktop clients over QUIC on this UDP address (default
    /// 0.0.0.0:47800 when given without a value). Runs until Ctrl-C unless
    /// --seconds is set.
    #[arg(long, value_name = "ADDR", num_args = 0..=1,
          default_missing_value = DEFAULT_LISTEN)]
    listen: Option<std::net::SocketAddr>,
    /// Keep the self-signed TLS certificate in this directory (cert.der,
    /// key.der), so its fingerprint stays the same across restarts.
    /// Default: a new certificate at every start.
    #[arg(long, value_name = "DIR", requires = "listen")]
    tls_dir: Option<PathBuf>,
}

const DEFAULT_LISTEN: &str = "0.0.0.0:47800";

/// Free ride on a world package's road graph (a path or a cached id, its
/// signature checked), from a seeded random start, or along one of its routes.
fn free_ride(args: &Args, spec: &str) -> Course {
    let fail = |e: String| -> ! {
        eprintln!("{spec}: {e}");
        std::process::exit(1);
    };
    let trust = ocw_worldstore::trust::TrustConfig {
        root_files: args.trust_root.clone(),
        ..ocw_worldstore::trust::TrustConfig::for_build()
    }
    .trust_store()
    .unwrap_or_else(|e| fail(e));
    let opts = ocw_worldstore::LoadOptions {
        allow_unsigned: args.allow_unsigned,
        ..ocw_worldstore::LoadOptions::new(
            trust,
            ocw_worldstore::WorldCache::new(ocw_worldstore::WorldCache::default_root()),
        )
    };
    let world = ocw_worldstore::load_world(spec, &opts).unwrap_or_else(|e| fail(e.to_string()));
    tracing::info!(world = %world.describe(), "world package");
    // The installed neighbours the roads continue into: one joined graph.
    let hood = ocw_worldstore::neighbourhood(&world, spec, &opts, !args.no_neighbours);
    for line in &hood.log {
        tracing::info!("{line}");
    }
    let pkg = world.package;
    let graph =
        hood.set.graph().cloned().unwrap_or_else(|| {
            fail("no road graph (rebuild it with world-builder package)".into())
        });
    let net = Arc::new(RoadNetwork::new(graph).with_package_name(hood.set.name()));
    if let Some(name) = args.route.as_deref() {
        let stored = pkg.route(name).unwrap_or_else(|| {
            let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
            fail(format!(
                "no route {name:?} (the package has: {})",
                names.join(", ")
            ))
        });
        let plan = stored
            .resolve(&net)
            .unwrap_or_else(|e| fail(format!("route {name:?} does not fit the road graph: {e}")));
        return Course::RouteRide {
            net,
            plan: Arc::new(plan),
        };
    }
    let (edge, offset_m, forward) = net
        .random_start(args.seed)
        .unwrap_or_else(|| fail("empty road graph".into()));
    Course::FreeRide {
        net,
        start: ocw_sim::FreeRideStart {
            edge,
            offset_m,
            forward,
        },
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let course = match &args.world {
        Some(spec) => free_ride(&args, spec),
        None if args.test_route == "hills" => Course::Route(Route::test_hills()),
        None => Course::Route(Route::test_loop()),
    };
    match &course {
        Course::Route(route) => tracing::info!(
            route = route.name(),
            length_km = route.length() / 1000.0,
            bots = args.bots,
            "starting session"
        ),
        Course::FreeRide { net, .. } => tracing::info!(
            edges = net.graph().edges.len(),
            bots = args.bots,
            "starting free ride"
        ),
        Course::RouteRide { plan, .. } => tracing::info!(
            route = %plan.name,
            length_km = plan.length_m() / 1000.0,
            bots = args.bots,
            "starting route ride"
        ),
    }
    let mut server = GameServer::new(
        course,
        ServerConfig {
            allow_dev_commands: false,
            bots: args.bots,
            start_m: 0.0,
        },
    );
    let mut net = args.listen.map(|addr| listen(&args, addr));
    let stop = net.as_ref().map(NetServer::shutdown_on_signal);
    let tick = Duration::from_secs_f64(SIM_DT);
    let mut next = Instant::now();
    // Time spent in the server tick (physics, bots, snapshot encoding).
    let (mut busy, mut worst, mut ticks) = (Duration::ZERO, Duration::ZERO, 0u32);
    loop {
        let started = Instant::now();
        if let Some(net) = &mut net {
            net.pump(&mut server);
        }
        let broadcast = server.tick();
        if let Some(net) = &mut net {
            net.broadcast(&server, &broadcast);
        }
        let took = started.elapsed();
        busy += took;
        worst = worst.max(took);
        ticks += 1;
        let session = server.session();
        if session.tick().is_multiple_of(u64::from(SIM_TICK_HZ)) {
            for r in session.snapshot() {
                let track = session.track(r.id);
                let p = track.sample(r.state.distance).position;
                tracing::info!(
                    road = %track.name_at(r.state.distance),
                    x = format!("{:.0}", p[0]),
                    z = format!("{:.0}", p[2]),
                    t = format!("{:.0}s", session.time_s()),
                    rider = %r.id,
                    name = %r.name,
                    power_w = r.power_w.round(),
                    speed_kmh = format!("{:.1}", mps_to_kmh(r.state.speed)),
                    distance_km = format!("{:.3}", r.state.distance / 1000.0),
                );
            }
        }
        if args.seconds > 0.0 && session.time_s() >= args.seconds {
            break;
        }
        if stop
            .as_ref()
            .is_some_and(|s| s.load(std::sync::atomic::Ordering::SeqCst))
        {
            tracing::info!("shutting down");
            break;
        }
        if !args.fast {
            next += tick;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
        }
    }
    tracing::info!(
        ticks,
        mean_tick_us = format!("{:.1}", busy.as_secs_f64() * 1e6 / f64::from(ticks.max(1))),
        max_tick_us = format!("{:.1}", worst.as_secs_f64() * 1e6),
        budget_us = format!("{:.0}", SIM_DT * 1e6),
        "session finished"
    );
    if let Some(net) = net {
        net.shutdown();
    }
}

/// Opens the QUIC endpoint and tells the user how to connect.
fn listen(args: &Args, addr: std::net::SocketAddr) -> NetServer {
    let identity = match &args.tls_dir {
        Some(dir) => ServerIdentity::load_or_create(dir),
        None => ServerIdentity::self_signed(),
    }
    .unwrap_or_else(|e| {
        eprintln!("TLS certificate: {e}");
        std::process::exit(1);
    });
    let net = NetServer::bind(addr, &identity).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let local = net.local_addr();
    tracing::info!(addr = %local, cert_sha256 = net.fingerprint(), "listening (QUIC, UDP)");
    let host = if local.ip().is_unspecified() {
        "HOST".to_owned()
    } else if local.is_ipv6() {
        format!("[{}]", local.ip())
    } else {
        local.ip().to_string()
    };
    let port = if local.port() == DEFAULT_PORT {
        String::new()
    } else {
        format!(":{}", local.port())
    };
    println!(
        "Clients connect with:\n  ocw-desktop --server {host}{port} --server-cert {}",
        net.fingerprint()
    );
    if local.ip().is_loopback() || local.ip().is_unspecified() {
        println!("  (on this machine also: ocw-desktop --server 127.0.0.1{port} --insecure-dev)");
    }
    net
}
