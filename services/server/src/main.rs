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
    /// from `world-builder package`) instead of the test loop.
    #[arg(long, value_name = "PATH")]
    world: Option<PathBuf>,
    /// Seed of the free-ride start point.
    #[arg(long, default_value_t = 1)]
    seed: u64,
}

/// Free ride on a world package's road graph, from a seeded random start.
fn free_ride(path: &std::path::Path, seed: u64) -> Course {
    let fail = |e: String| -> ! {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    };
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(e.to_string()));
    let pkg = ocw_world::WorldPackage::from_bytes(&bytes).unwrap_or_else(|e| fail(e));
    let graph = pkg
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
        Some(path) => free_ride(path, args.seed),
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
