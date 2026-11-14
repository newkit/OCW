//! Headless game server.
//!
//! Runs the authoritative session with test riders at the fixed tick rate and
//! logs their progress. With `--listen` it also accepts desktop clients over
//! QUIC (spec §6.2; `ocw-desktop --server ADDR`).

use std::thread;
use std::time::{Duration, Instant};

use std::path::PathBuf;

use clap::Parser;
use ocw_core::{SIM_DT, SIM_TICK_HZ, mps_to_kmh};
use ocw_server::net::{DEFAULT_PORT, NetServer, ServerIdentity};
use ocw_server::stats::{ServerReport, TickStats};
use ocw_server::{Course, GameServer, ServerConfig};
use ocw_world::Route;

fn parse_time_control(s: &str) -> Result<ocw_server::ClockAccess, String> {
    match s {
        "host" => Ok(ocw_server::ClockAccess::Host),
        "anyone" => Ok(ocw_server::ClockAccess::Anyone),
        "nobody" => Ok(ocw_server::ClockAccess::Nobody),
        _ => Err("host, anyone or nobody".into()),
    }
}

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
    /// Put an Echo rider (the voice echo test, spec §7) at the start of
    /// every session and let riders move it (`voice.echo here`).
    #[arg(long)]
    echo_bot: bool,
    /// Who may change the world clock with the time machine (spec §14
    /// Phase 9, 9D): the host (the rider connected longest), anyone, or
    /// nobody (the clock stays real time).
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    time_control: ocw_server::ClockAccess,
    /// Who may switch the session to another world package from the World
    /// menu (spec §3.2.1): the host, anyone, or nobody. The server rides
    /// only packages it finds itself (its cache and the files next to
    /// --world); everyone joins again on the new world.
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    world_control: ocw_server::ClockAccess,
    /// Who may enter god mode and fly over the world as a unicorn (spec
    /// §14 Phase 13A): the host, anyone, or nobody (an organizer's
    /// session). Never during a running event.
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    god_control: ocw_server::ClockAccess,
    /// Rider collisions (spec §14 Phase 12, on by default): soft bumps and
    /// crashes; group rides soft bumps only.
    #[arg(long, default_value = "on", value_parser = ["on", "off"])]
    collisions: String,
    /// Chaotic bots that sometimes make mistakes (Phase 12 Q3).
    #[arg(long)]
    chaotic_bots: bool,
    /// Who may change the collision rules (host, anyone, nobody).
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    collision_control: ocw_server::ClockAccess,
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
    /// Ride a route file from `world-builder route` (`ocw-route 1`)
    /// instead of a test route. Clients ride the same file
    /// (`ocw-desktop --route-file`, `ocw-bot --route-file`).
    #[arg(long, value_name = "PATH", conflicts_with_all = ["world", "test_route"])]
    route_file: Option<PathBuf>,
    /// Write the session's statistics (tick timing, who joined and left)
    /// as JSON to this file at the end (`ocw_server::stats::ServerReport`).
    #[arg(long, value_name = "PATH")]
    stats_file: Option<PathBuf>,
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
    /// LiveKit server for voice chat (spec §7), e.g. ws://127.0.0.1:7880;
    /// default: $OCW_LIVEKIT_URL. Voice is off without it.
    #[arg(long, value_name = "URL")]
    livekit_url: Option<String>,
    /// LiveKit API key; default: $OCW_LIVEKIT_API_KEY. The secret is read
    /// only from $OCW_LIVEKIT_API_SECRET.
    #[arg(long, value_name = "KEY")]
    livekit_api_key: Option<String>,
}

const DEFAULT_LISTEN: &str = "0.0.0.0:47800";

/// How world packages are loaded and verified (spec §4.7).
fn load_options(args: &Args) -> Result<ocw_worldstore::LoadOptions, String> {
    let trust = ocw_worldstore::trust::TrustConfig {
        root_files: args.trust_root.clone(),
        ..ocw_worldstore::trust::TrustConfig::for_build()
    }
    .trust_store()?;
    Ok(ocw_worldstore::LoadOptions {
        allow_unsigned: args.allow_unsigned,
        ..ocw_worldstore::LoadOptions::new(
            trust,
            ocw_worldstore::WorldCache::new(ocw_worldstore::WorldCache::default_root()),
        )
    })
}

/// Free ride on a world package's road graph (a path or a cached id, its
/// signature checked), from a seeded random start, or along one of its routes.
fn free_ride(args: &Args, spec: &str) -> Course {
    let fail = |e: String| -> ! {
        eprintln!("{spec}: {e}");
        std::process::exit(1);
    };
    let opts = load_options(args).unwrap_or_else(|e| fail(e));
    ocw_server::worlds::load_course(
        spec,
        args.route.as_deref(),
        &opts,
        !args.no_neighbours,
        args.seed,
    )
    .unwrap_or_else(|e| fail(e))
    .0
}

/// The worlds this server may switch to: the cache and the packages next
/// to --world (spec §3.2.1).
fn catalog(
    args: &Args,
    opts: &ocw_worldstore::LoadOptions,
) -> Vec<ocw_worldstore::discover::WorldEntry> {
    let d = ocw_worldstore::discover::Discovery {
        cache: opts.cache.clone(),
        given: args.world.clone(),
        dirs: Vec::new(),
        trust: opts.trust.clone(),
        now_unix_s: opts.now_unix_s,
        allow_unsigned: opts.allow_unsigned,
        require_signed: opts.require_signed,
    };
    ocw_worldstore::discover::discover(&d, &mut Default::default())
}

/// The result of loading a world for a switch.
type Loaded = Result<(Course, ocw_protocol::v1::WorldChange), String>;

/// A world switch being loaded on its own thread (the session goes on).
type WorldLoad = (ocw_server::WorldSwitch, std::sync::mpsc::Receiver<Loaded>);

/// Takes the host's world switch (checked by the game server) and starts
/// loading it; applies a finished load. Failures go to the asker.
fn pump_world_switch(
    args: &Args,
    server: &mut GameServer,
    mut net: Option<&mut NetServer>,
    loading: &mut Option<WorldLoad>,
) {
    let mut tell = |conns: &[ocw_server::ConnectionId], bytes: &[u8]| {
        if let Some(n) = net.as_deref_mut() {
            n.send_to(conns, bytes);
        }
    };
    if loading.is_none()
        && let Some(req) = server.take_world_switch()
    {
        let started = load_options(args).and_then(|opts| {
            let entries = catalog(args, &opts);
            let spec = ocw_server::worlds::resolve(&entries, &req.world)?
                .spec
                .clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let route = req.route.clone();
            let (neighbours, seed) = (!args.no_neighbours, args.seed);
            tracing::info!(%spec, ?route, by = %req.by, "loading world for the switch");
            thread::Builder::new()
                .name("ocw-world-load".into())
                .spawn(move || {
                    let r = ocw_server::worlds::load_course(
                        &spec,
                        route.as_deref(),
                        &opts,
                        neighbours,
                        seed,
                    );
                    let _ = tx.send(r);
                })
                .map_err(|e| e.to_string())?;
            Ok(rx)
        });
        match started {
            Ok(rx) => *loading = Some((req, rx)),
            Err(e) => {
                tracing::warn!(world = %req.world, %e, "world switch refused");
                tell(&[req.conn], &ocw_server::result_message(false, e));
            }
        }
    }
    let Some((req, rx)) = loading.as_ref() else {
        return;
    };
    let result = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the loading thread ended".into()),
    };
    match result {
        Ok((course, mut change)) => {
            change.by.clone_from(&req.by);
            let (conns, bytes) = server.switch_world(course, change);
            tell(&conns, &bytes);
        }
        Err(e) => {
            tracing::warn!(world = %req.world, %e, "world switch failed");
            tell(
                &[req.conn],
                &ocw_server::result_message(false, format!("{}: {e}", req.world)),
            );
        }
    }
    *loading = None;
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let mut route_sha256 = String::new();
    let course = match (&args.world, &args.route_file) {
        (Some(spec), _) => free_ride(&args, spec),
        (None, Some(path)) => {
            let (route, sha) = load_route_file(path).unwrap_or_else(|e| {
                eprintln!("{}: {e}", path.display());
                std::process::exit(1);
            });
            tracing::info!(route = route.id(), sha256 = %sha, "route file");
            route_sha256 = sha;
            Course::Route(route)
        }
        (None, None) if args.test_route == "hills" => Course::Route(Route::test_hills()),
        (None, None) => Course::Route(Route::test_loop()),
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
    // Voice chat (spec §7): LiveKit from the options or the environment;
    // the API secret only from the environment (not visible in `ps`).
    let voice = ocw_server::voice::VoiceConfig::from_parts(
        args.livekit_url.clone(),
        args.livekit_api_key.clone(),
        None,
        |k| std::env::var(k).ok(),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    match &voice {
        Some(v) => tracing::info!(url = %v.url, room = %v.session_room(), "voice chat on"),
        None => tracing::info!(
            "voice chat off (set OCW_LIVEKIT_URL, OCW_LIVEKIT_API_KEY, OCW_LIVEKIT_API_SECRET)"
        ),
    }
    let mut server = GameServer::new(
        course,
        ServerConfig {
            allow_dev_commands: false,
            bots: args.bots,
            start_m: 0.0,
            allow_local_events: false,
            voice,
            clock_access: args.time_control,
            world_access: args.world_control,
            god_access: args.god_control,
            real_epoch_ms: None,
            allow_echo_bot: args.echo_bot,
            echo_bot_at_start: args.echo_bot,
            collisions: args.collisions == "on",
            chaotic_bots: args.chaotic_bots,
            collision_access: args.collision_control,
        },
    );
    if let Some(echo) = server.session().echo_rider() {
        tracing::info!(
            echo = %echo,
            identity = %ocw_protocol::voice_identity(echo),
            "echo rider at the start (voice echo test; the echo_participant example joins as it)"
        );
    }
    let mut net = args.listen.map(|addr| listen(&args, addr));
    let stop = net.as_ref().map(NetServer::shutdown_on_signal);
    let tick = Duration::from_secs_f64(SIM_DT);
    let mut next = Instant::now();
    // Time spent in the server tick (physics, bots, snapshot encoding) and
    // how late each tick starts against the fixed schedule.
    let mut stats = TickStats::default();
    let mut world_load: Option<WorldLoad> = None;
    loop {
        let started = Instant::now();
        let late = if args.fast {
            Duration::ZERO
        } else {
            started.saturating_duration_since(next)
        };
        if let Some(net) = &mut net {
            net.pump(&mut server);
        }
        pump_world_switch(&args, &mut server, net.as_mut(), &mut world_load);
        let broadcast = server.tick();
        if let Some(net) = &mut net {
            net.broadcast(&server, &broadcast);
        }
        let took = started.elapsed();
        stats.record(took, late, server.session().rider_count());
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
        ticks = stats.ticks(),
        mean_tick_us = format!("{:.1}", stats.mean_tick_us()),
        max_tick_us = format!("{:.1}", stats.max_tick_us()),
        budget_us = format!("{:.0}", SIM_DT * 1e6),
        over_budget = stats.over_budget(),
        late_ticks = stats.late_ticks(),
        max_late_ms = format!("{:.1}", stats.max_late_ms()),
        "session finished"
    );
    if let Some(path) = &args.stats_file {
        // Before the shutdown: whoever is connected now stayed to the end.
        let session = server.session();
        let report = ServerReport {
            sim_time_s: session.time_s(),
            bots: args.bots,
            joined: server.joined().to_vec(),
            left: server.left().to_vec(),
            riders_at_end: session.rider_count(),
            route_id: if session.is_free_ride() {
                String::new()
            } else {
                session.route().id().to_owned()
            },
            route_sha256,
            ..Default::default()
        }
        .with_timing(&stats, !args.fast);
        if let Err(e) = std::fs::write(path, report.to_json()) {
            eprintln!("{}: {e}", path.display());
        }
    }
    if let Some(net) = net {
        net.shutdown();
    }
}

/// Loads an `ocw-route` file: the route and the SHA-256 of the file (so
/// clients can check they ride the same one).
fn load_route_file(path: &std::path::Path) -> Result<(Route, String), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // The same hex SHA-256 as certificate fingerprints.
    let sha = ocw_server::net::tls::fingerprint(&bytes);
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let file = ocw_world::RouteFile::parse(&text)?;
    let route = file.to_route().map_err(|e| e.to_string())?;
    Ok((route, sha))
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
