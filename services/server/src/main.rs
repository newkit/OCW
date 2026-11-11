//! Headless game server (Phase 1 skeleton).
//!
//! Runs the authoritative session with test riders at the fixed tick rate and
//! logs their progress. The QUIC transport for remote clients follows in
//! Phase 5 (spec §6.2).

use std::thread;
use std::time::{Duration, Instant};

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use ocw_core::{SIM_DT, SIM_TICK_HZ, mps_to_kmh};
use ocw_server::{Course, GameServer, ServerConfig};
use ocw_world::{RoadNetwork, Route};

#[derive(Parser)]
#[command(version, about = "Open Cycling World headless game server")]
struct Args {
    /// Number of server-side test riders.
    #[arg(long, default_value_t = 3)]
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
}

/// Free ride on a world package's road graph, from a seeded random start.
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
    let seed = args.seed;
    let graph = world
        .package
        .graph
        .unwrap_or_else(|| fail("no road graph (rebuild it with world-builder package)".into()));
    let net = Arc::new(RoadNetwork::new(graph));
    let (edge, offset_m, forward) = net
        .random_start(seed)
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
    }
    let mut server = GameServer::new(
        course,
        ServerConfig {
            allow_dev_commands: false,
            bots: args.bots,
            start_m: 0.0,
        },
    );
    let tick = Duration::from_secs_f64(SIM_DT);
    let mut next = Instant::now();
    loop {
        // No network clients yet; snapshots would be sent to them here.
        let _snapshot = server.tick();
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
        if !args.fast {
            next += tick;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
        }
    }
    tracing::info!("session finished");
}
