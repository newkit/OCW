//! Headless game server (Phase 1 skeleton).
//!
//! Runs the authoritative session with test riders at the fixed tick rate and
//! logs their progress. The QUIC transport for remote clients follows in
//! Phase 5 (spec §6.2).

use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use ocw_core::{SIM_DT, SIM_TICK_HZ, mps_to_kmh};
use ocw_server::{GameServer, ServerConfig};
use ocw_world::Route;

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
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let route = Route::test_loop();
    tracing::info!(
        route = route.name(),
        length_km = route.length() / 1000.0,
        bots = args.bots,
        "starting session"
    );
    let mut server = GameServer::new(
        route,
        ServerConfig {
            allow_dev_commands: false,
            bots: args.bots,
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
                tracing::info!(
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
