//! `ocw-bot`: headless remote riders for a game server (spec §11.4).
//!
//! ```text
//! ocw-bot --server HOST[:PORT] --server-cert SHA256 --riders 2 --seconds 120 \
//!         [--route loop | --route-file F.route] [--follow NAME] [--label mac]
//! ocw-bot verdict --server-stats stats.json --processes 3 --riders-per-process 2 \
//!         [--kiosk NAME] bot-a.log bot-b.log ...
//! ```
//!
//! The first form connects `--riders` riders (one QUIC connection each),
//! rides until `--seconds` or the server's shutdown, prints progress, and
//! ends with one `OCW-BOT-REPORT {json}` line. Exit code 0 if every rider
//! joined and none lost its connection. The second form checks a whole
//! multiplayer test run (server statistics + every bot process's output).

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use clap::{Args as ClapArgs, Parser, Subcommand};
use ocw_bot::report::NORMAL_CLOSE;
use ocw_bot::{BotReport, BotRider, Expect, Pacing, RiderConfig, connect_hint, pacing, verdict};
use ocw_core::SIM_DT;
use ocw_server::net::{ClientOptions, LinkStatus, NetClient, ServerTrust};
use ocw_server::stats::ServerReport;
use ocw_world::Route;

#[derive(Parser)]
#[command(
    version,
    about = "Open Cycling World headless bot client (multiplayer test)",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    #[command(flatten)]
    ride: RideArgs,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check a multiplayer test run: the server's --stats-file and the
    /// output of every bot process.
    Verdict(VerdictArgs),
}

#[derive(ClapArgs)]
struct RideArgs {
    /// The game server (`ocw-server --listen`): HOST[:PORT], default port 47800.
    #[arg(long, value_name = "ADDR")]
    server: Option<String>,
    /// SHA-256 fingerprint of the server's certificate (ocw-server prints it).
    #[arg(long, value_name = "SHA256")]
    server_cert: Option<String>,
    /// Development only: accept any certificate of a server on this machine.
    #[arg(long, conflicts_with = "server_cert")]
    insecure_dev: bool,
    /// Riders in this process (one connection each).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=50))]
    riders: u32,
    /// Ride this long (s); 0 = until the server shuts down.
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,
    /// Label of this process in the report (e.g. the OS; never a host name).
    #[arg(long, default_value = std::env::consts::OS)]
    label: String,
    /// Rider names are PREFIX-1, PREFIX-2, … (default: the label).
    #[arg(long)]
    name_prefix: Option<String>,
    /// Index of the first rider across all bot processes (gaps, looks).
    #[arg(long, default_value_t = 0)]
    index: u32,
    /// Test route the server rides (`ocw-server --test-route`).
    #[arg(long, default_value = "loop", value_parser = ["loop", "hills"])]
    route: String,
    /// Route file the server rides (`ocw-server --route-file`).
    #[arg(long, value_name = "PATH")]
    route_file: Option<PathBuf>,
    /// Base power in watts.
    #[arg(long, default_value_t = 200.0)]
    power: f64,
    /// Rider weight in kg.
    #[arg(long, default_value_t = 75.0)]
    weight: f64,
    /// Ride a few metres ahead of this rider (display name), e.g. the
    /// kiosk's, so the bots stay on its screen; without it in the session
    /// the bots ride their own power.
    #[arg(long, value_name = "NAME")]
    follow: Option<String>,
    /// Give up on a rider without a Welcome after this many seconds.
    #[arg(long, default_value_t = 25.0)]
    join_timeout: f64,
    /// Also write the report (JSON) to this file.
    #[arg(long, value_name = "PATH")]
    report: Option<PathBuf>,
}

#[derive(ClapArgs)]
struct VerdictArgs {
    /// The server's statistics (`ocw-server --stats-file`).
    #[arg(long, value_name = "PATH")]
    server_stats: PathBuf,
    /// Bot processes started (each output file is one).
    #[arg(long)]
    processes: usize,
    #[arg(long, default_value_t = 1)]
    riders_per_process: u32,
    /// Display name of a GUI client that takes part (the kiosk, a camera
    /// client); repeatable.
    #[arg(long = "client", value_name = "NAME")]
    clients: Vec<String>,
    /// Minimum metres every bot rider must ride.
    #[arg(long, default_value_t = 20.0)]
    min_ridden_m: f64,
    /// Output of each bot process (with its OCW-BOT-REPORT line).
    outputs: Vec<PathBuf>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // The transport logs addresses at info: keep them out.
                .unwrap_or_else(|_| "info,quinn=warn,ocw_server::net=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let code = match cli.cmd {
        Some(Cmd::Verdict(v)) => run_verdict(&v),
        None => ride(&cli.ride),
    };
    std::process::exit(code);
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("ocw-bot: {msg}");
    std::process::exit(2);
}

/// The route and the SHA-256 of its file (empty for a test route).
fn load_route(args: &RideArgs) -> (Route, String) {
    let Some(path) = &args.route_file else {
        let route = if args.route == "hills" {
            Route::test_hills()
        } else {
            Route::test_loop()
        };
        return (route, String::new());
    };
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let sha = ocw_server::net::tls::fingerprint(&bytes);
    let text =
        String::from_utf8(bytes).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let route = ocw_world::RouteFile::parse(&text)
        .and_then(|f| f.to_route().map_err(|e| e.to_string()))
        .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    (route, sha)
}

struct Link {
    rider: BotRider,
    net: Option<NetClient>,
}

fn ride(args: &RideArgs) -> i32 {
    let Some(spec) = &args.server else {
        fail("--server HOST[:PORT] is required (or use `ocw-bot verdict`)");
    };
    let (addr, server_name) = ocw_server::net::resolve(spec).unwrap_or_else(|e| fail(e));
    let trust = ServerTrust::for_server(addr, args.server_cert.as_deref(), args.insecure_dev)
        .unwrap_or_else(|e| fail(e));
    let (route, route_sha256) = load_route(args);
    let prefix = args
        .name_prefix
        .clone()
        .unwrap_or_else(|| args.label.clone());
    tracing::info!(
        route = route.id(),
        length_km = format!("{:.2}", route.length() / 1000.0),
        riders = args.riders,
        label = %args.label,
        "starting bots"
    );
    let start = Instant::now();
    // The wall clock goes on while a machine sleeps, the monotonic one
    // does not (macOS): the difference is the time suspended.
    let wall_start = SystemTime::now();
    let mut links: Vec<Link> = (0..args.riders)
        .map(|i| {
            let k = args.index + i;
            let (bike, look) = ocw_server::bot_bike(k);
            let config = RiderConfig {
                name: ocw_bot::rider_name(&prefix, i),
                weight_kg: args.weight,
                bike,
                look,
                pacing: Pacing {
                    base_w: ocw_bot::rider_power_w(args.power, k),
                    phase: f64::from(k) * 1.7,
                    follow: args.follow.clone().map(|n| (n, pacing::follow_gap_m(k))),
                },
            };
            let rider = BotRider::new(config, route.clone());
            let net = NetClient::connect(addr, &server_name, &trust, ClientOptions::default())
                .unwrap_or_else(|e| fail(e));
            net.send(rider.hello());
            Link {
                rider,
                net: Some(net),
            }
        })
        .collect();
    let tick = Duration::from_secs_f64(SIM_DT);
    let mut next = Instant::now();
    let mut next_log = 10.0;
    loop {
        let now = start.elapsed().as_secs_f64();
        for l in &mut links {
            let Some(net) = &l.net else { continue };
            for m in net.receive() {
                l.rider.on_message(&m, now);
            }
            if let Some(rtt) = net.rtt() {
                l.rider.set_snapshot_age(rtt.as_secs_f64() / 2.0);
            }
            if let LinkStatus::Closed(reason) = net.status() {
                l.rider.close(&reason, now);
            }
            if !l.rider.riding() && !l.rider.closed() && now > args.join_timeout {
                l.rider.close(
                    &format!("no Welcome within {:.0} s", args.join_timeout),
                    now,
                );
            }
            for m in l.rider.tick(now) {
                net.send(m);
            }
            if l.rider.closed()
                && let Some(net) = l.net.take()
            {
                net.close();
            }
        }
        if now >= next_log {
            next_log += 10.0;
            for l in &links {
                let r = l.rider.report();
                let s = l.rider.state();
                tracing::info!(
                    rider = l.rider.name(),
                    t = format!("{now:.0}s"),
                    distance_m = format!("{:.0}", s.distance),
                    speed_kmh = format!("{:.1}", s.speed * 3.6),
                    snapshots = r.snapshots,
                    riders_seen = r.max_riders_seen,
                    following = r.following,
                    "progress"
                );
            }
        }
        let all_closed = links.iter().all(|l| l.rider.closed());
        if all_closed || (args.seconds > 0.0 && now >= args.seconds) {
            break;
        }
        next += tick;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => thread::sleep(wait),
            // Fell behind (a stalled machine): do not try to catch up.
            None => next = Instant::now(),
        }
    }
    let end = start.elapsed().as_secs_f64();
    let wall = wall_start.elapsed().map_or(end, |d| d.as_secs_f64());
    let suspended_s = (wall - end).max(0.0);
    for l in &mut links {
        l.rider.close("bot finished", end);
        if let Some(net) = l.net.take() {
            net.close();
        }
    }
    let report = BotReport {
        label: args.label.clone(),
        os: std::env::consts::OS.to_owned(),
        route_id: route.id().to_owned(),
        route_sha256,
        seconds: end,
        suspended_s,
        riders: links.iter().map(|l| l.rider.report()).collect(),
    };
    let mut ok = true;
    let port = addr.port();
    if let Some(n) = ocw_bot::report::suspend_note(suspended_s) {
        println!("note: {n}");
    }
    for r in &report.riders {
        let closed = r.closed.clone().unwrap_or_default();
        let normal = NORMAL_CLOSE.iter().any(|n| closed.contains(n));
        let good = r.joined_at_s.is_some() && r.rejected.is_none() && normal;
        ok &= good;
        println!(
            "{} {}: {} snapshots, saw {} riders ({} others), rode {:.0} m, prediction mean {:.3} m / max {:.2} m, end: {closed}",
            if good { "ok  " } else { "FAIL" },
            r.name,
            r.snapshots,
            r.max_riders_seen,
            r.others_seen.len(),
            r.ridden_m(),
            r.prediction_error_mean_m(),
            r.prediction_error_max_m,
        );
        if let Some(why) = &r.rejected {
            println!("     rejected: {why}");
        }
        if let Some(hint) = connect_hint(&closed, port) {
            println!("     {hint}");
        }
    }
    println!("{}", report.to_line());
    if let Some(path) = &args.report
        && let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&report).expect("data"))
    {
        eprintln!("{}: {e}", path.display());
    }
    if ok { 0 } else { 1 }
}

fn run_verdict(args: &VerdictArgs) -> i32 {
    let text = std::fs::read_to_string(&args.server_stats)
        .unwrap_or_else(|e| fail(format!("{}: {e}", args.server_stats.display())));
    let server = ServerReport::from_json(&text)
        .unwrap_or_else(|e| fail(format!("{}: {e}", args.server_stats.display())));
    let mut bots = Vec::new();
    for path in &args.outputs {
        let out = std::fs::read_to_string(path).unwrap_or_default();
        match BotReport::from_output(&out).into_iter().next() {
            Some(Ok(r)) => bots.push(r),
            Some(Err(e)) => println!("FAIL {}: unreadable report: {e}", path.display()),
            None => println!(
                "FAIL {}: no report (the bot did not finish)",
                path.display()
            ),
        }
    }
    let expect = Expect {
        bot_processes: args.processes,
        riders_per_process: args.riders_per_process,
        clients: args.clients.clone(),
        min_ridden_m: args.min_ridden_m,
        ..Default::default()
    };
    let v = verdict(&server, &bots, &expect);
    for line in v.lines() {
        println!("{line}");
    }
    if v.passed() { 0 } else { 1 }
}
