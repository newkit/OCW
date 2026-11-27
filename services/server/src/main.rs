//! Headless game server.
//!
//! Runs the authoritative session with test riders at the fixed tick rate and
//! logs their progress. With `--listen` it also accepts desktop clients over
//! QUIC (spec §6.2; `ocw-desktop --server ADDR`), with `--webtransport`
//! also the browser app over WebTransport (spec §14 Phase 10 B3).

use std::thread;
use std::time::{Duration, Instant};

use std::path::PathBuf;

use clap::Parser;
use ocw_core::{SIM_DT, SIM_TICK_HZ, mps_to_kmh};
use ocw_server::net::{DEFAULT_PORT, NetServer, ServerIdentity, Transport, WebIdentity};
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

fn parse_weather(s: &str) -> Result<String, String> {
    if s == "real" {
        return Ok(s.to_owned());
    }
    ocw_weather::presets::preset(s)
        .map(|_| s.to_owned())
        .ok_or_else(|| {
            let names: Vec<&str> = ocw_weather::presets::PRESETS.iter().map(|p| p.0).collect();
            format!("real or one of {}", names.join(", "))
        })
}

fn parse_provider(s: &str) -> Result<ocw_weather::real::Choice, String> {
    ocw_weather::real::Choice::parse(s).ok_or_else(|| {
        let ids: Vec<&str> = ocw_weather::real::Choice::ALL
            .iter()
            .map(|c| c.id())
            .collect();
        format!("one of {}", ids.join(", "))
    })
}

/// A voice URL as LiveKit clients take it: `ws://` or `wss://` and a host.
fn parse_voice_url(s: &str) -> Result<String, String> {
    let rest = s
        .strip_prefix("wss://")
        .or_else(|| s.strip_prefix("ws://"))
        .ok_or("a ws:// or wss:// URL")?;
    if rest.is_empty() || rest.starts_with('/') || rest.contains(char::is_whitespace) {
        return Err("a ws:// or wss:// URL with a host".into());
    }
    Ok(s.trim_end_matches('/').to_owned())
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
    /// Test runs (spec §2.11): the ride photographer's 20 minutes between
    /// photos pass this many times faster (as `ocw-desktop
    /// --photo-time-scale`), so the server relays the riders' faster shots.
    #[arg(long, value_name = "FACTOR", default_value_t = 1.0, hide = true)]
    photo_time_scale: f64,
    /// Put an Echo rider (the voice echo test, spec §7) at the start of
    /// every session and let riders move it (`voice.echo here`).
    #[arg(long)]
    echo_bot: bool,
    /// Tests only: accept the clients' dev commands (`teleport` …), so a
    /// test can place riders near and far (the voice range tests, spec
    /// §7.1.7 Q6). Never on a public server.
    #[arg(long, hide = true)]
    dev_commands: bool,
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
    /// Damage (spec §14 Phase 12E, off by default, Q1): hits take off the
    /// riders' condition and damage their bikes; only with collisions on.
    #[arg(long, default_value = "off", value_parser = ["on", "off"])]
    damage: String,
    /// Who may change the collision rules (host, anyone, nobody).
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    collision_control: ocw_server::ClockAccess,
    /// Who may change the weather (spec §14 Phase 9, S4, Q2): the host,
    /// anyone, or nobody (the weather stays as started).
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    weather_control: ocw_server::ClockAccess,
    /// The session's road traffic (spec 14 Phase 18, 18A; nothing drives
    /// yet): off, light, normal, busy or lunatic.
    #[arg(long, default_value = "off", value_parser = ocw_server::traffic::parse_level)]
    traffic: ocw_core::traffic::RoadTraffic,
    /// How drivers behave around riders (Q8): polite, mostly_polite,
    /// some_realism.
    #[arg(long, default_value = "polite", value_parser = ocw_server::traffic::parse_manners)]
    driver_manners: ocw_core::traffic::DriverManners,
    /// Who may change the road traffic (Q2): the host, anyone, or nobody
    /// (it stays as started).
    #[arg(long, default_value = "host", value_parser = parse_time_control)]
    traffic_control: ocw_server::ClockAccess,
    /// Whether the weather affects the physics (spec §14 Phase 9, S6, Q4:
    /// wind, air density, wet roads; on by default, events follow their
    /// organiser's rule). The host changes it like the weather.
    #[arg(long, default_value = "on", value_parser = ["on", "off"])]
    weather_physics: String,
    /// The weather the session starts with: a preset (`clear`, `rain`,
    /// `thunderstorm`, `heavy-snow`, `fog` …; ocw_weather::presets) or
    /// `real` (spec §14 Phase 9, S7: from the providers for the place of
    /// the session and the world clock's moment).
    #[arg(long, value_name = "PRESET|real", value_parser = parse_weather)]
    weather: Option<String>,
    /// The real weather's data source: automatic (the regional network –
    /// MeteoSwiss in Switzerland and Liechtenstein, DWD in Germany –, then
    /// MET Norway, then Open-Meteo), meteoswiss, dwd, metno or open-meteo.
    /// OCW_WEATHER_USER_AGENT names the requests' User-Agent (default: the
    /// product, no personal data); OCW_WEATHER_OFFLINE=1 makes none.
    #[arg(long, value_name = "SOURCE", default_value = "automatic", value_parser = parse_provider)]
    weather_provider: ocw_weather::real::Choice,
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
    /// Trust this user key (spec §4.7 *User keys*: a rider's own key that
    /// signs their unofficial worlds) – its 64 hex digits, its .pub.json
    /// file, or a package signed with it (repeatable). The keys in this
    /// account's <data dir>/ocw/trusted-keys.json are trusted too. Riders
    /// never make the server trust a key.
    #[arg(long, value_name = "KEY|FILE")]
    trust_key: Vec<String>,
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
    /// Also accept the browser app over WebTransport (HTTP/3) on this UDP
    /// address (default 0.0.0.0:47801 when given without a value), in the
    /// same session (spec §14 Phase 10 B3). Its certificate is valid for
    /// 13 days (browsers take it by hash, `cert-hash`), kept in --tls-dir
    /// and renewed two days before it ends.
    #[arg(long, value_name = "ADDR", num_args = 0..=1,
          default_missing_value = DEFAULT_WEBTRANSPORT, requires = "listen")]
    webtransport: Option<std::net::SocketAddr>,
    /// Write the WebTransport endpoint's URL and certificate hash as JSON
    /// to this file (and again after a renewal), for scripts and pages.
    #[arg(long, value_name = "PATH", requires = "webtransport")]
    web_info_file: Option<PathBuf>,
    /// Answer `GET /join.json` on this TCP address (spec 11.5 *Join
    /// info*): the public WebTransport URL with the current certificate
    /// hashes (and the next one a day before a renewal), the voice URL and
    /// the world, for pages that learn them at runtime (the public preview).
    /// Other requests are passed on to --join-upstream (one TLS port of a
    /// hosting's proxy serves both, Fly.io's 443). A browser's `GET /`
    /// gets the server's status page and `/stats.json` its public
    /// statistics, counts only (spec 11.5 *Status page*).
    #[arg(long, value_name = "ADDR", requires = "webtransport")]
    join_http: Option<std::net::SocketAddr>,
    /// Keep the status page's totals (rides, km, since when) in this small
    /// JSON file across restarts (spec 11.5 *Status page*); without it
    /// they start again at every start.
    #[arg(long, value_name = "PATH", requires = "join_http")]
    public_stats_file: Option<PathBuf>,
    /// Where --join-http passes other requests (default: the bundled voice
    /// server's signalling port on 127.0.0.1); `none`: answer 404.
    #[arg(long, value_name = "ADDR|none", requires = "join_http")]
    join_upstream: Option<String>,
    /// LiveKit server for voice chat (spec §7), e.g. ws://127.0.0.1:7880;
    /// default: $OCW_LIVEKIT_URL. Voice is off without it.
    #[arg(long, value_name = "URL")]
    livekit_url: Option<String>,
    /// LiveKit API key; default: $OCW_LIVEKIT_API_KEY. The secret is read
    /// only from $OCW_LIVEKIT_API_SECRET.
    #[arg(long, value_name = "KEY")]
    livekit_api_key: Option<String>,
    /// Voice chat (spec §7.1): `auto` starts the bundled LiveKit server
    /// (`livekit-server`) beside a server that others join (--listen),
    /// unless --livekit-url / $OCW_LIVEKIT_URL name one; `off`: no voice.
    #[arg(long, default_value = "auto", value_parser = ["auto", "off"])]
    voice: String,
    /// The voice server binary to start (default: livekit-server next to
    /// this executable, then on PATH).
    #[arg(long, value_name = "PATH")]
    voice_server: Option<PathBuf>,
    /// The bundled voice server's ports: signalling (TCP), ICE over TCP,
    /// media (one UDP port).
    #[arg(long, value_name = "SIGNAL,TCP,UDP", default_value = "7880,7881,7882",
          value_parser = ocw_server::voice::launcher::VoicePorts::parse)]
    voice_ports: ocw_server::voice::launcher::VoicePorts,
    /// The host riders from the internet reach this server on (the join
    /// link's host, a name or an address): the bundled voice server
    /// announces its address for media, and their grants name it. Without
    /// it the voice server asks a STUN server for the public address.
    #[arg(long, value_name = "HOST")]
    voice_public_host: Option<String>,
    /// The voice URL riders from the internet get, whole, instead of
    /// ws://HOST:SIGNAL (spec 11.5): `wss://NAME` when TLS in front of the
    /// signalling port forwards to it (a hosting's proxy, Fly.io's on 443),
    /// so browser pages served over https:// can connect. Riders on this
    /// machine and the LAN keep ws://.
    #[arg(long, value_name = "URL", requires = "listen", value_parser = parse_voice_url)]
    voice_public_url: Option<String>,
    /// Open the router's ports automatically while running (UPnP-IGD,
    /// spec §7.1.7 Q4): the game's UDP port (and WebTransport's) and the
    /// bundled voice server's TCP / UDP ports, removed again at the end.
    /// The router's public address then names the join link and the voice
    /// server's media address (unless --voice-public-host is given). Off
    /// by default for the headless server.
    #[arg(long, requires = "listen")]
    upnp: bool,
    /// Send QUIC packets one by one, without UDP segmentation offload
    /// (GSO): for hosts whose UDP path drops batched packets (Fly.io,
    /// spec 11.5, where most handshakes got no answer with it).
    #[arg(long, requires = "listen")]
    no_gso: bool,
    /// Keep QUIC packets at 1200 bytes (no path MTU discovery): for hosts
    /// whose UDP path loses larger packets (Fly.io, spec 11.5, where the
    /// replies on the reliable stream were lost once the size was raised).
    #[arg(long, requires = "listen")]
    no_mtu_discovery: bool,
    /// Run the accounts backend (sign-in with Google and Strava, spec
    /// §9.1) in this process from this configuration (a copy of
    /// services/accounts/accounts.example.toml, git-ignored: it holds the client
    /// secrets). The hello's access tokens are checked against its keys.
    #[arg(long, value_name = "FILE", conflicts_with = "auth_dev")]
    auth_config: Option<PathBuf>,
    /// Development sign-in: the accounts backend with in-process FAKE
    /// Google and Strava servers (every sign-in succeeds) on this address
    /// (default 127.0.0.1:8480). Never for real riders.
    #[arg(long, value_name = "ADDR", num_args = 0..=1, default_missing_value = "127.0.0.1:8480")]
    auth_dev: Option<String>,
    /// The browser app's page for --auth-dev: allowed as the browser's
    /// redirect URI, its origin may call the token endpoint.
    #[arg(long, value_name = "URL", requires = "auth_dev")]
    auth_dev_web: Option<String>,
    /// Write the --auth-dev URLs (backend, fake Google, fake Strava) as
    /// JSON to this file, for scripts.
    #[arg(long, value_name = "PATH", requires = "auth_dev")]
    auth_info_file: Option<PathBuf>,
    /// Check the access tokens of a separate accounts backend: its issuer
    /// URL. Its keys come from its JWKS (fetched every five minutes and
    /// when a token names an unknown key).
    #[arg(long, value_name = "URL", conflicts_with_all = ["auth_config", "auth_dev"])]
    auth_issuer: Option<String>,
    /// The audience the access tokens must name.
    #[arg(long, value_name = "AUD", default_value = ocw_auth::DEFAULT_AUDIENCE)]
    auth_audience: String,
    /// Refuse riders without an account (default: anonymous riders are
    /// allowed, Phase 10 Q5).
    #[arg(long)]
    auth_required: bool,
    /// Credit the hearts of signed-in riders (spec 14 Phase 4, 4O Q19):
    /// this server's signing key file (`world-builder keys new-signing`;
    /// the accounts backend trusts it when the project's root key certified
    /// it). When a rider leaves, a signed ride credit goes to the accounts
    /// backend; the apps then upload no summary of that ride.
    #[arg(long, value_name = "FILE")]
    credit_key: Option<PathBuf>,
    /// The accounts backend the credits go to (default: the sign-in's
    /// issuer).
    #[arg(long, value_name = "URL", requires = "credit_key")]
    credit_backend: Option<String>,
    /// Check the parts of signed-in riders against their account (spec 14
    /// Phase 4, 4O W5d): only parts the account owns count, anonymous
    /// riders ride base parts (needs a sign-in; the garage comes from
    /// `--credit-backend` or the sign-in's issuer).
    #[arg(long)]
    check_parts: bool,
}

/// The parts check of `--check-parts` (spec 14 Phase 4, 4O W5d).
fn start_ownership(args: &Args, auth: Option<&Auth>) -> Option<ocw_server::Ownership> {
    if !args.check_parts {
        return None;
    }
    let Some(backend) = args
        .credit_backend
        .clone()
        .or_else(|| auth.map(|a| a.verifier.issuer().to_owned()))
    else {
        eprintln!("--check-parts: no accounts backend (a sign-in or --credit-backend)");
        std::process::exit(2);
    };
    tracing::info!(%backend, "parts checked against the riders' accounts");
    match ocw_server::ownership::BackendOwnership::start(backend) {
        Ok(o) => Some(ocw_server::Ownership(std::sync::Arc::new(o))),
        Err(e) => {
            eprintln!("--check-parts: {e}");
            std::process::exit(2);
        }
    }
}

/// The hearts credits of `--credit-key` (spec 14 Phase 4, 4O Q19).
fn start_credits(args: &Args, auth: Option<&Auth>) -> Option<ocw_server::Credits> {
    let path = args.credit_key.as_ref()?;
    let fail = |e: String| -> ! {
        eprintln!("--credit-key: {e}");
        std::process::exit(2);
    };
    let json =
        std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let key = ocw_world::manifest::SigningKeyPair::from_json(&json).unwrap_or_else(|e| fail(e));
    let backend = args
        .credit_backend
        .clone()
        .or_else(|| auth.map(|a| a.verifier.issuer().to_owned()))
        .unwrap_or_else(|| fail("no accounts backend: --credit-backend URL or a sign-in".into()));
    let claims = key.claims();
    tracing::info!(key = %claims.key_id, name = %claims.name, %backend, "hearts credited by this server");
    let sender = ocw_server::credits::BackendCredits::start(key, backend)
        .unwrap_or_else(|e| fail(e.to_string()));
    Some(ocw_server::Credits(std::sync::Arc::new(sender)))
}

/// Sign-in (spec §9.1): the verifier of the hello's tokens, and what keeps
/// the backend (or the key refresher) running.
struct Auth {
    verifier: std::sync::Arc<ocw_auth::Verifier>,
    _runtime: Option<tokio::runtime::Runtime>,
}

/// Fetches a backend's JWKS (through its discovery document).
fn fetch_jwks(issuer: &str) -> Result<ocw_auth::Jwks, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let get = |url: &str| -> Result<String, String> {
        agent
            .get(url)
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(|e| format!("{url}: {e}"))
    };
    let disc: ocw_auth::oidc::Discovery =
        serde_json::from_str(&get(&format!("{issuer}/.well-known/openid-configuration"))?)
            .map_err(|e| format!("{issuer}: {e}"))?;
    if disc.issuer != issuer {
        return Err(format!(
            "{issuer}: the discovery names another issuer ({})",
            disc.issuer
        ));
    }
    serde_json::from_str(&get(&disc.jwks_uri)?).map_err(|e| format!("{}: {e}", disc.jwks_uri))
}

/// Keeps a remote backend's keys fresh: every five minutes, and soon after
/// a token named an unknown key (at most every 30 s).
fn refresh_keys(issuer: String, verifier: std::sync::Arc<ocw_auth::Verifier>) {
    let _ = thread::Builder::new()
        .name("ocw-jwks".into())
        .spawn(move || {
            let mut last: Option<Instant> = None;
            loop {
                let due = last.is_none_or(|t| {
                    t.elapsed() >= Duration::from_secs(300)
                        || (t.elapsed() >= Duration::from_secs(30) && verifier.take_stale())
                });
                if due {
                    last = Some(Instant::now());
                    match fetch_jwks(&issuer) {
                        Ok(keys) => {
                            tracing::debug!(keys = keys.keys.len(), "JWKS refreshed");
                            verifier.set_keys(keys);
                        }
                        Err(e) => tracing::warn!(%e, "JWKS fetch failed"),
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
}

#[cfg(feature = "backend")]
fn start_backend(args: &Args) -> Option<Auth> {
    use ocw_accounts::{AuthService, Config, fake::DevBackend};
    let fail = |e: String| -> ! {
        eprintln!("accounts backend: {e}");
        std::process::exit(2);
    };
    if args.auth_config.is_none() && args.auth_dev.is_none() {
        return None;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ocw-accounts")
        .enable_all()
        .build()
        .unwrap_or_else(|e| fail(e.to_string()));
    let service = if let Some(path) = &args.auth_config {
        let cfg = Config::load(path).unwrap_or_else(|e| fail(e));
        let listen = cfg.listen.clone();
        let svc = std::sync::Arc::new(AuthService::new(cfg).unwrap_or_else(|e| fail(e)));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(&listen))
            .unwrap_or_else(|e| fail(format!("{listen}: {e}")));
        let s = svc.clone();
        runtime.spawn(async move { ocw_accounts::serve(s, listener).await });
        tracing::info!(issuer = %svc.issuer(), %listen, providers = ?svc.providers(), "accounts backend on");
        svc
    } else {
        let listen = args.auth_dev.clone().unwrap_or_default();
        let web = args.auth_dev_web.clone().map(|page| {
            let after = page.find("://").map_or(0, |i| i + 3);
            let origin = page[after..]
                .find('/')
                .map_or(page.clone(), |i| page[..after + i].to_owned());
            (page, origin)
        });
        let dev = runtime
            .block_on(DevBackend::start(&listen, web))
            .unwrap_or_else(|e| fail(e));
        tracing::warn!(
            issuer = %dev.issuer,
            google = %dev.google.base,
            strava = %dev.strava.base,
            "DEVELOPMENT sign-in with FAKE Google and Strava (--auth-dev): not for real riders"
        );
        if let Some(path) = &args.auth_info_file {
            let info = serde_json::json!({
                "issuer": dev.issuer,
                "fake_google": dev.google.base,
                "fake_strava": dev.strava.base,
            });
            if let Err(e) = std::fs::write(path, info.to_string()) {
                fail(format!("{}: {e}", path.display()));
            }
        }
        // The fakes' servers run on the runtime; the handles may go.
        dev.service.clone()
    };
    let verifier = std::sync::Arc::new(service.verifier());
    // Key rotation, and the game server's copy of the keys.
    let v = verifier.clone();
    runtime.spawn(async move {
        loop {
            match service.maybe_rotate_keys() {
                Ok(true) => tracing::info!("signing key rotated"),
                Ok(false) => {}
                Err(e) => tracing::warn!(%e, "key rotation failed"),
            }
            v.set_keys(service.jwks());
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    Some(Auth {
        verifier,
        _runtime: Some(runtime),
    })
}

#[cfg(not(feature = "backend"))]
fn start_backend(args: &Args) -> Option<Auth> {
    if args.auth_config.is_some() || args.auth_dev.is_some() {
        eprintln!("this ocw-server was built without the accounts backend (feature `backend`)");
        std::process::exit(2);
    }
    None
}

/// The sign-in of this server: its own backend, a remote one, or none.
fn start_auth(args: &Args) -> Option<Auth> {
    if let Some(a) = start_backend(args) {
        return Some(a);
    }
    let issuer = args
        .auth_issuer
        .as_deref()?
        .trim_end_matches('/')
        .to_owned();
    let keys = fetch_jwks(&issuer).unwrap_or_else(|e| {
        tracing::warn!(%e, "JWKS not available yet: signed-in riders are refused until it is");
        ocw_auth::Jwks::default()
    });
    let verifier = std::sync::Arc::new(ocw_auth::Verifier::new(&issuer, &args.auth_audience, keys));
    refresh_keys(issuer.clone(), verifier.clone());
    tracing::info!(%issuer, audience = %args.auth_audience, "sign-in: access tokens checked against the backend's JWKS");
    Some(Auth {
        verifier,
        _runtime: None,
    })
}

const DEFAULT_LISTEN: &str = "0.0.0.0:47800";
const DEFAULT_WEBTRANSPORT: &str = "0.0.0.0:47801";

/// How world packages are loaded and verified (spec §4.7).
fn load_options(args: &Args) -> Result<ocw_worldstore::LoadOptions, String> {
    let user_keys = args
        .trust_key
        .iter()
        .map(|k| ocw_worldstore::userkeys::read_key(k).map_err(|e| format!("--trust-key {k}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let trust = ocw_worldstore::trust::TrustConfig {
        root_files: args.trust_root.clone(),
        user_keys,
        // Load once is a rider's answer in an app, never the server's.
        session_keys: false,
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
fn free_ride(args: &Args, spec: &str) -> (Course, (f64, f64), ocw_protocol::v1::WorldChange) {
    let fail = |e: String| -> ! {
        eprintln!("{spec}: {e}");
        std::process::exit(1);
    };
    let opts = load_options(args).unwrap_or_else(|e| fail(e));
    let (course, change, origin) = ocw_server::worlds::load_course_at(
        spec,
        args.route.as_deref(),
        &opts,
        !args.no_neighbours,
        args.seed,
    )
    .unwrap_or_else(|e| fail(e));
    (course, origin, change)
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
type Loaded = Result<(Course, ocw_protocol::v1::WorldChange, (f64, f64)), String>;

/// A world switch being loaded on its own thread (the session goes on).
type WorldLoad = (ocw_server::WorldSwitch, std::sync::mpsc::Receiver<Loaded>);

/// Takes the host's world switch (checked by the game server) and starts
/// loading it; applies a finished load. Failures go to the asker.
fn pump_world_switch(
    args: &Args,
    server: &mut GameServer,
    mut net: Option<&mut NetServer>,
    loading: &mut Option<WorldLoad>,
) -> Option<ocw_protocol::v1::WorldChange> {
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
                    let r = ocw_server::worlds::load_course_at(
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
    let (req, rx) = loading.as_ref()?;
    let result = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the loading thread ended".into()),
    };
    let mut switched = None;
    match result {
        Ok((course, mut change, origin)) => {
            change.by.clone_from(&req.by);
            switched = Some(change.clone());
            let (conns, bytes) = server.switch_world(course, change);
            server.set_geo_origin(Some(origin));
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
    switched
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let mut route_sha256 = String::new();
    let mut geo_origin = None;
    // What the join link names (spec §6.2 *Join links*): the package and
    // the route.
    let mut link_world: (Option<String>, Option<String>) = (None, None);
    // What the join info names (--join-http, spec 11.5).
    let mut join_world = JoinWorld::default();
    let course = match (&args.world, &args.route_file) {
        (Some(spec), _) => {
            let (course, origin, change) = free_ride(&args, spec);
            geo_origin = Some(origin);
            join_world = JoinWorld::of(&change);
            link_world = (
                Some(if change.package_id.starts_with("local/") {
                    change.package_id.clone()
                } else {
                    format!("{}@{}", change.package_id, change.version)
                }),
                (!change.route.is_empty()).then(|| change.route.clone()),
            );
            course
        }
        (None, Some(path)) => {
            let (route, sha) = load_route_file(path).unwrap_or_else(|e| {
                eprintln!("{}: {e}", path.display());
                std::process::exit(1);
            });
            tracing::info!(route = route.id(), sha256 = %sha, "route file");
            route_sha256 = sha;
            Course::Route(route)
        }
        (None, None) if args.test_route == "hills" => {
            link_world.1 = Some("hills".into());
            join_world.route = Some("hills".into());
            Course::Route(Route::test_hills())
        }
        (None, None) => {
            link_world.1 = Some("loop".into());
            join_world.route = Some("loop".into());
            Course::Route(Route::test_loop())
        }
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
    // Without one, a server that others join starts the bundled voice
    // server (spec §7.1.1) before it opens its port, and with --upnp the
    // router's ports are opened (§7.1.7 Q4) – both on the hosting
    // services' own thread (`ocw_server::hosting`, shared with the app's
    // Host a ride).
    let (voice, services) = voice_setup(&args);
    let public_ip = services.as_ref().and_then(|s| s.status().public_ip);
    // Sign-in (spec §9.1): the backend in this process, or a remote one.
    let auth = start_auth(&args);
    if args.auth_required && auth.is_none() {
        eprintln!("--auth-required needs --auth-config, --auth-dev or --auth-issuer");
        std::process::exit(2);
    }
    let mut server = GameServer::new(
        course,
        ServerConfig {
            allow_dev_commands: args.dev_commands,
            photo_gap_s: Some(ocw_world::photo_share::GAP_S / args.photo_time_scale.max(1.0)),
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
            damage: args.damage == "on",
            collision_access: args.collision_control,
            weather_access: args.weather_control,
            road_traffic: args.traffic,
            driver_manners: args.driver_manners,
            traffic_access: args.traffic_control,
            air_seed: None,
            weather: args
                .weather
                .as_deref()
                .filter(|w| *w != "real")
                .map(|w| ocw_weather::presets::preset(w).expect("checked by the parser")),
            weather_seed: None,
            geo_origin,
            real_weather: (args.weather.as_deref() == Some("real"))
                .then_some(args.weather_provider),
            weather_physics: args.weather_physics == "on",
            auth: auth.as_ref().map(|a| a.verifier.clone()),
            auth_required: args.auth_required,
            hearts_credits: start_credits(&args, auth.as_ref()),
            ownership: start_ownership(&args, auth.as_ref()),
        },
    );
    // Real weather (spec §14 Phase 9, S7): the server asks the providers,
    // never the clients.
    server.set_weather_fetcher(Box::new(
        ocw_server::realweather::http::HttpFetcher::default(),
    ));
    if let Some(echo) = server.session().echo_rider() {
        tracing::info!(
            echo = %echo,
            identity = %ocw_protocol::voice_identity(echo),
            "echo rider at the start (voice echo test; the echo_participant example joins as it)"
        );
    }
    let mut net = args
        .listen
        .map(|addr| listen(&args, addr, &link_world, public_ip));
    let stop = net.as_ref().map(NetServer::shutdown_on_signal);
    let join = start_join(&args);
    if let (Some((board, status)), Some(n)) = (&join, &net) {
        board.set(&join_info(&args, n, &join_world));
        status.set_page(page_info(&args, n, &join_world));
    }
    // The status page's counts (spec 11.5 *Status page*).
    let mut public = join.as_ref().map(|_| start_public_stats(&args));
    let mut public_saved = Instant::now();
    // When the WebTransport certificate was last checked for renewal.
    let mut cert_checked = Instant::now();
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
        let switched = pump_world_switch(&args, &mut server, net.as_mut(), &mut world_load);
        if let Some(change) = &switched {
            join_world = JoinWorld::of(change);
        }
        let mut join_changed = switched.is_some();
        if let Some(n) = &mut net
            && cert_checked.elapsed() >= Duration::from_secs(60)
        {
            cert_checked = Instant::now();
            join_changed |= rotate_web_cert(&args, n);
        }
        if join_changed && let (Some((board, status)), Some(n)) = (&join, &net) {
            board.set(&join_info(&args, n, &join_world));
            status.set_page(page_info(&args, n, &join_world));
        }
        let broadcast = server.tick();
        let directed = server.take_directed();
        if let Some(net) = &mut net {
            net.broadcast(&server, &broadcast);
            // A voice block's VoiceApart (spec §7.1.7 Q9): to one rider each.
            net.send_directed(directed, None);
        }
        let took = started.elapsed();
        stats.record(took, late, server.session().rider_count());
        if let (Some(p), Some((_, status))) = (&mut public, &join) {
            let now_s = ocw_server::unix_now_ms() / 1000;
            p.record_tick(took);
            p.observe(now_s, SIM_DT, &server.public_observation());
            if server
                .session()
                .tick()
                .is_multiple_of(u64::from(SIM_TICK_HZ))
            {
                status.set_stats(&p.report(
                    now_s,
                    env!("CARGO_PKG_VERSION"),
                    &world_note(&join_world),
                ));
            }
            if public_saved.elapsed() >= Duration::from_secs(60) {
                public_saved = Instant::now();
                save_public_stats(&args, p);
            }
        }
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
    let web = net
        .as_ref()
        .map(|n| n.counters(Transport::WebTransport))
        .unwrap_or_default();
    if net.as_ref().is_some_and(|n| n.web_addr().is_some()) {
        tracing::info!(
            sessions = web.connections,
            messages_in = web.messages_in,
            datagrams_in = web.datagrams_in,
            "WebTransport endpoint"
        );
    }
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
            web_sessions: web.connections,
            web_messages_in: web.messages_in,
            web_datagrams_in: web.datagrams_in,
            ..Default::default()
        }
        .with_timing(&stats, !args.fast);
        if let Err(e) = std::fs::write(path, report.to_json()) {
            eprintln!("{}: {e}", path.display());
        }
    }
    if let Some(p) = &public {
        save_public_stats(&args, p);
    }
    if let Some(net) = net {
        net.shutdown();
    }
    // The voice server and the router's ports go with the game server.
    drop(services);
}

/// The voice of this server (spec §7, §7.1.1): an external LiveKit from
/// the options or the environment (the API secret only from the
/// environment, not visible in `ps`), else the bundled one started now for
/// a server that others join, else none – the log says why. With --upnp
/// the router's ports are opened too (§7.1.7 Q4). Both run on the hosting
/// services' thread (`ocw_server::hosting`), stopped when it is dropped.
fn voice_setup(
    args: &Args,
) -> (
    Option<ocw_server::voice::VoiceConfig>,
    Option<ocw_server::hosting::HostServices>,
) {
    use ocw_server::hosting::{HostServices, HostingOptions, VoicePlan};
    use ocw_server::voice::launcher::{self, LaunchOptions};
    let external = ocw_server::voice::VoiceConfig::from_parts(
        args.livekit_url.clone(),
        args.livekit_api_key.clone(),
        None,
        |k| std::env::var(k).ok(),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let Some(listen) = args.listen else {
        if args.voice == "off" {
            tracing::info!("voice chat off (--voice off)");
            return (None, None);
        }
        if let Some(v) = &external {
            tracing::info!(url = %v.url, room = %v.session_room(), "voice chat on (external LiveKit)");
        } else {
            tracing::info!("voice chat off (no --listen: nobody to talk to)");
        }
        return (external, None);
    };
    let mut opts = HostingOptions {
        web_udp: args.webtransport.map(|a| a.port()),
        upnp: args.upnp.then(ocw_server::portmap::upnp::discover),
        ..HostingOptions::new(listen)
    };
    let mut voice = None;
    if args.voice == "off" {
        tracing::info!("voice chat off (--voice off)");
        opts.voice_off = "--voice off".into();
    } else if let Some(v) = external {
        tracing::info!(url = %v.url, room = %v.session_room(), "voice chat on (external LiveKit)");
        opts.voice_off = "an external LiveKit".into();
        voice = Some(v);
    } else {
        let exe = std::env::current_exe().ok();
        match launcher::find_binary(
            args.voice_server.as_deref(),
            exe.as_deref().and_then(std::path::Path::parent),
            std::env::var_os("PATH").as_deref(),
        ) {
            Ok(binary) => {
                opts.voice = Some(VoicePlan {
                    launch: LaunchOptions {
                        ports: args.voice_ports,
                        ..LaunchOptions::new(binary, launcher::default_state_dir())
                    },
                    public_host: args.voice_public_host.clone(),
                });
            }
            Err(e) => {
                tracing::warn!("voice chat off: {e}");
                opts.voice_off = e;
            }
        }
    }
    if opts.voice.is_none() && opts.upnp.is_none() {
        return (voice, None);
    }
    let services = HostServices::spawn(opts).unwrap_or_else(|e| {
        eprintln!("cannot start the hosting services: {e}");
        std::process::exit(1);
    });
    // The voice server before the port opens (the first riders get voice):
    // the router search, then at most the voice server's ready time.
    let within =
        ocw_server::portmap::SEARCH_TIMEOUT + launcher::READY_TIMEOUT + Duration::from_secs(5);
    if !services.wait_ready(within) {
        tracing::warn!(
            "the voice server and the router's ports are not ready yet; the game server starts anyway"
        );
    }
    let mut voice = voice.or_else(|| services.voice_config());
    if let (Some(url), Some(reach)) = (
        &args.voice_public_url,
        voice.as_mut().and_then(|v| v.reach.as_mut()),
    ) {
        tracing::info!(%url, "voice URL for riders from the internet");
        reach.public_url = Some(url.clone());
    }
    (voice, Some(services))
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
fn listen(
    args: &Args,
    addr: std::net::SocketAddr,
    (world, route): &(Option<String>, Option<String>),
    public_ip: Option<std::net::IpAddr>,
) -> NetServer {
    let identity = match &args.tls_dir {
        Some(dir) => ServerIdentity::load_or_create(dir),
        None => ServerIdentity::self_signed(),
    }
    .unwrap_or_else(|e| {
        eprintln!("TLS certificate: {e}");
        std::process::exit(1);
    });
    if args.no_gso {
        ocw_server::net::tls::set_segmentation_offload(false);
        tracing::info!("UDP segmentation offload off (--no-gso)");
    }
    if args.no_mtu_discovery {
        ocw_server::net::tls::set_mtu_discovery(false);
        tracing::info!("QUIC path MTU discovery off (--no-mtu-discovery): 1200-byte packets");
    }
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
    // The join link first (spec §6.2 *Join links*): the kiosk reads both
    // once the "Clients connect with" lines are there.
    println!(
        "{}",
        ocw_server::net::join_lines(
            local,
            net.fingerprint(),
            world.as_deref(),
            route.as_deref(),
            public_ip
        )
    );
    println!(
        "Clients connect with:\n  ocw-desktop --server {host}{port} --server-cert {}",
        net.fingerprint()
    );
    if local.ip().is_loopback() || local.ip().is_unspecified() {
        println!("  (on this machine also: ocw-desktop --server 127.0.0.1{port} --insecure-dev)");
    }
    let mut net = net;
    if let Some(addr) = args.webtransport {
        let now = ocw_server::unix_now_ms() / 1000;
        let identity = match &args.tls_dir {
            Some(dir) => WebIdentity::load_or_create(dir, now).map(|(id, _)| id),
            None => WebIdentity::generate(now),
        }
        .unwrap_or_else(|e| {
            eprintln!("WebTransport certificate: {e}");
            std::process::exit(1);
        });
        let web = net
            .listen_webtransport(addr, identity, args.tls_dir.clone())
            .unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
        announce_web(args, &net, web);
    }
    net
}

/// Logs and prints how browsers join (and writes --web-info-file).
fn announce_web(args: &Args, net: &NetServer, addr: std::net::SocketAddr) {
    use ocw_server::net::webtransport;
    let hash = net.web_cert_hash().unwrap_or_default();
    let hashes = net.web_cert_hashes();
    let not_after = net.web_cert_not_after().unwrap_or_default();
    let url = webtransport::url_for(addr);
    tracing::info!(addr = %addr, cert_hash = %hash, not_after_unix_s = not_after, "listening (WebTransport, UDP)");
    println!(
        "Browsers join with the page options:
  ?{}
  (the certificate is valid until {} UTC; it is renewed two days before, then the page needs the new cert-hash)",
        webtransport::page_query(&url, &hash),
        utc(not_after)
    );
    if let Some(path) = &args.web_info_file {
        let mut info = serde_json::json!({
            "url": url,
            "port": addr.port(),
            "cert_hash": hash,
            "cert_hashes": hashes,
            "not_after_unix_s": not_after,
            "query": webtransport::page_query(&url, &hash),
        });
        // Behind a hosting's address translation (Fly.io, spec 11.5) the
        // bound address is an inner one: pages from the internet use the
        // public host.
        if let Some(public) = public_web_url(args.voice_public_host.as_deref(), addr.port()) {
            info["public_query"] = webtransport::page_query(&public, &hash).into();
            info["public_url"] = public.into();
        }
        if let Err(e) = std::fs::write(
            path,
            format!(
                "{info:#}
"
            ),
        ) {
            eprintln!("{}: {e}", path.display());
        }
    }
}

/// The WebTransport URL riders from the internet use: `https://HOST:PORT`
/// with the public host (`--voice-public-host`), when one is given.
fn public_web_url(public_host: Option<&str>, port: u16) -> Option<String> {
    let host = public_host.filter(|h| !h.is_empty())?;
    Some(format!(
        "https://{}:{port}",
        ocw_server::voice::url_host(host)
    ))
}

/// Renews the WebTransport certificate when it nears its end (and
/// announces the next one a day before). Returns whether the hashes
/// changed.
fn rotate_web_cert(args: &Args, net: &mut NetServer) -> bool {
    use ocw_server::net::WebCertChange;
    let now = ocw_server::unix_now_ms() / 1000;
    match net.rotate_web_cert(now) {
        Ok(Some(change)) => {
            match &change {
                WebCertChange::Announced { next } => tracing::info!(
                    next_cert_hash = %next,
                    "next WebTransport certificate announced (join info: both hashes)"
                ),
                WebCertChange::Renewed { hash } => tracing::warn!(
                    cert_hash = %hash,
                    "WebTransport certificate renewed: pages need the new cert-hash"
                ),
            }
            if let Some(addr) = net.web_addr() {
                announce_web(args, net, addr);
            }
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::error!(%e, "WebTransport certificate renewal failed");
            false
        }
    }
}

/// The world the join info names (spec 11.5 *Join info*).
#[derive(Default)]
struct JoinWorld {
    id: Option<String>,
    /// The region's name for people (the status page).
    name: Option<String>,
    version: Option<u64>,
    sha256: Option<String>,
    route: Option<String>,
}

impl JoinWorld {
    fn of(change: &ocw_protocol::v1::WorldChange) -> Self {
        Self {
            id: Some(change.package_id.clone()),
            name: (!change.name.is_empty()).then(|| change.name.clone()),
            version: (change.version > 0).then_some(change.version),
            sha256: (!change.content_sha256.is_empty()).then(|| {
                change
                    .content_sha256
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            }),
            route: (!change.route.is_empty()).then(|| change.route.clone()),
        }
    }
}

/// Starts --join-http (spec 11.5 *Join info*) with the status page
/// (*Status page*).
fn start_join(
    args: &Args,
) -> Option<(
    ocw_server::net::JoinBoard,
    ocw_server::net::join_http::StatusBoard,
)> {
    let addr = args.join_http?;
    let upstream = match args.join_upstream.as_deref() {
        Some("none") => None,
        Some(a) => Some(a.parse().unwrap_or_else(|e| {
            eprintln!("--join-upstream {a}: {e}");
            std::process::exit(2);
        })),
        None => Some(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            args.voice_ports.signal,
        ))),
    };
    let board = ocw_server::net::JoinBoard::default();
    let status = ocw_server::net::join_http::StatusBoard::default();
    match ocw_server::net::JoinFront::spawn_with(
        addr,
        upstream,
        board.clone(),
        Some(status.clone()),
    ) {
        Ok(front) => {
            tracing::info!(addr = %front.local_addr(), upstream = ?upstream, path = ocw_server::net::join_http::PATH, stats = ocw_server::net::join_http::STATS_PATH, "join info and status page for pages (HTTP)");
            Some((board, status))
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// The status page's data now (spec 11.5 *Status page*): the desktop
/// join link with the public host and the world.
fn page_info(
    args: &Args,
    net: &NetServer,
    world: &JoinWorld,
) -> ocw_server::net::join_http::PageInfo {
    let local = net.local_addr();
    let link_world = world.id.as_ref().map(|id| match world.version {
        Some(v) if !id.starts_with("local/") => format!("{id}@{v}"),
        _ => id.clone(),
    });
    let desktop_link = match args.voice_public_host.as_deref().filter(|h| !h.is_empty()) {
        Some(host) => {
            let mut l = ocw_protocol::join::JoinLink::new(host.to_owned(), local.port());
            l.cert = Some(net.fingerprint().to_string());
            l.world = link_world;
            l.route = world.route.clone();
            l.to_link()
        }
        None => ocw_server::net::join_links(
            local,
            net.fingerprint(),
            link_world.as_deref(),
            world.route.as_deref(),
        )
        .into_iter()
        .next()
        .unwrap_or_default(),
    };
    ocw_server::net::join_http::PageInfo {
        desktop_link,
        world_name: world
            .name
            .clone()
            .or_else(|| world.id.clone())
            .unwrap_or_default(),
        world_version: world.version,
        route: world.route.clone(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// The status page's world note.
fn world_note(world: &JoinWorld) -> ocw_server::public_stats::WorldNote {
    ocw_server::public_stats::WorldNote {
        id: world.id.clone(),
        name: world.name.clone(),
        version: world.version,
        route: world.route.clone(),
    }
}

/// The public statistics to start from: the totals kept in
/// --public-stats-file, if any.
fn start_public_stats(args: &Args) -> ocw_server::public_stats::PublicStats {
    use ocw_server::public_stats::{PublicStats, Saved};
    let now = ocw_server::unix_now_ms() / 1000;
    let saved = args.public_stats_file.as_ref().and_then(|p| {
        let text = std::fs::read_to_string(p).ok()?;
        match serde_json::from_str::<Saved>(&text) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(path = %p.display(), %e, "public statistics file unreadable: starting again");
                None
            }
        }
    });
    match saved {
        Some(s) => PublicStats::resume(now, s),
        None => PublicStats::new(now),
    }
}

/// Writes the totals to --public-stats-file (a new file renamed over the
/// old one, so a crash never leaves half a file).
fn save_public_stats(args: &Args, stats: &ocw_server::public_stats::PublicStats) {
    let Some(path) = &args.public_stats_file else {
        return;
    };
    let tmp = path.with_extension("json.new");
    let text = serde_json::to_string_pretty(stats.saved()).unwrap_or_default() + "\n";
    if let Err(e) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path)) {
        tracing::warn!(path = %path.display(), %e, "cannot save the public statistics");
    }
}

/// The join info of this server now: the public WebTransport URL, the
/// hashes, the voice URL, the world.
fn join_info(args: &Args, net: &NetServer, world: &JoinWorld) -> ocw_server::net::JoinInfo {
    use ocw_server::net::webtransport;
    let addr = net.web_addr().unwrap_or_else(|| net.local_addr());
    ocw_server::net::JoinInfo {
        webtransport: public_web_url(args.voice_public_host.as_deref(), addr.port())
            .unwrap_or_else(|| webtransport::url_for(addr)),
        cert_hashes: net.web_cert_hashes(),
        not_after_unix_s: net.web_cert_not_after().unwrap_or_default(),
        voice: args.voice_public_url.clone(),
        world: world.id.clone(),
        world_version: world.version,
        world_sha256: world.sha256.clone(),
        route: world.route.clone(),
        query: String::new(),
        join: public_join_link(
            args.voice_public_host.as_deref(),
            net.local_addr().port(),
            net.fingerprint(),
            world,
        ),
    }
    .with_query()
}

/// The desktop's QUIC join link for riders from the internet (the join
/// info's `join`, spec 11.5): the public host, the QUIC port and
/// fingerprint, the world as `ID@VERSION` (a local package by its id) and
/// the route. None without a public host.
fn public_join_link(
    public_host: Option<&str>,
    port: u16,
    fingerprint: &str,
    world: &JoinWorld,
) -> Option<String> {
    let host = public_host.filter(|h| !h.is_empty())?;
    let mut link = ocw_protocol::join::JoinLink::new(host, port);
    link.cert = Some(fingerprint.to_owned());
    link.world = world.id.as_ref().map(|id| match world.version {
        Some(v) if !id.starts_with("local/") => format!("{id}@{v}"),
        _ => id.clone(),
    });
    link.route = world.route.clone();
    Some(link.to_link())
}

/// A Unix time as `YYYY-MM-DD HH:MM` (UTC).
fn utc(unix_s: i64) -> String {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UPnP is off for the headless server unless asked (spec §7.1.7 Q4),
    /// and needs --listen (nothing to open otherwise).
    #[test]
    fn upnp_is_off_by_default_for_the_headless_server() {
        let a = Args::try_parse_from(["ocw-server", "--listen"]).unwrap();
        assert!(!a.upnp);
        let a = Args::try_parse_from(["ocw-server", "--listen", "--upnp"]).unwrap();
        assert!(a.upnp);
        assert!(Args::try_parse_from(["ocw-server", "--upnp"]).is_err());
    }

    /// The public WebTransport URL of web-info.json (spec 11.5): the
    /// public host with the bound port, IPv6 in brackets.
    #[test]
    fn public_web_url_names_the_public_host() {
        assert_eq!(
            public_web_url(Some("203.0.113.7"), 47801).as_deref(),
            Some("https://203.0.113.7:47801")
        );
        assert_eq!(
            public_web_url(Some("2001:db8::7"), 47801).as_deref(),
            Some("https://[2001:db8::7]:47801")
        );
        assert_eq!(public_web_url(Some(""), 47801), None);
        assert_eq!(public_web_url(None, 47801), None);
    }

    /// The join info's QUIC link (spec 11.5): public host, fingerprint,
    /// world with its version; none without a public host.
    #[test]
    fn public_join_link_names_host_fingerprint_and_world() {
        let fp = "ab".repeat(32);
        let w = JoinWorld {
            id: Some("liechtenstein".into()),
            version: Some(10),
            ..JoinWorld::default()
        };
        assert_eq!(
            public_join_link(Some("203.0.113.7"), 47800, &fp, &w),
            Some(format!(
                "ocw://203.0.113.7?cert={fp}&world=liechtenstein@10"
            ))
        );
        let hills = JoinWorld {
            route: Some("hills".into()),
            ..JoinWorld::default()
        };
        assert_eq!(
            public_join_link(Some("203.0.113.7"), 47900, &fp, &hills),
            Some(format!("ocw://203.0.113.7:47900?cert={fp}&route=hills"))
        );
        assert_eq!(public_join_link(None, 47800, &fp, &w), None);
        assert_eq!(public_join_link(Some(""), 47800, &fp, &w), None);
    }

    /// --join-http (spec 11.5 *Join info*) needs WebTransport; its
    /// upstream is an address or `none`.
    #[test]
    fn join_http_needs_webtransport() {
        let a = Args::try_parse_from([
            "ocw-server",
            "--listen",
            "--webtransport",
            "--join-http",
            "0.0.0.0:8080",
        ])
        .unwrap();
        assert_eq!(a.join_http, Some("0.0.0.0:8080".parse().unwrap()));
        assert!(a.join_upstream.is_none());
        assert!(
            Args::try_parse_from(["ocw-server", "--listen", "--join-http", "0.0.0.0:8080"])
                .is_err()
        );
        assert!(Args::try_parse_from(["ocw-server", "--join-upstream", "none"]).is_err());
    }

    /// --no-gso (spec 11.5) only beside --listen.
    #[test]
    fn no_gso_needs_listen() {
        assert!(
            Args::try_parse_from(["ocw-server", "--listen", "--no-gso"])
                .unwrap()
                .no_gso
        );
        assert!(
            !Args::try_parse_from(["ocw-server", "--listen"])
                .unwrap()
                .no_gso
        );
        assert!(Args::try_parse_from(["ocw-server", "--no-gso"]).is_err());
        assert!(
            Args::try_parse_from(["ocw-server", "--listen", "--no-mtu-discovery"])
                .unwrap()
                .no_mtu_discovery
        );
        assert!(Args::try_parse_from(["ocw-server", "--no-mtu-discovery"]).is_err());
    }

    /// --voice-public-url (spec 11.5): a ws:// or wss:// URL with a host,
    /// only beside --listen; a trailing slash is dropped.
    #[test]
    fn voice_public_url_is_a_websocket_url() {
        let a = Args::try_parse_from([
            "ocw-server",
            "--listen",
            "--voice-public-url",
            "wss://ride.example.org/",
        ])
        .unwrap();
        assert_eq!(
            a.voice_public_url.as_deref(),
            Some("wss://ride.example.org")
        );
        assert_eq!(
            parse_voice_url("ws://203.0.113.7:7880").as_deref(),
            Ok("ws://203.0.113.7:7880")
        );
        for bad in [
            "https://ride.example.org",
            "wss://",
            "wss:///x",
            "ride.example.org",
        ] {
            assert!(parse_voice_url(bad).is_err(), "{bad}");
        }
        assert!(
            Args::try_parse_from(["ocw-server", "--voice-public-url", "wss://x.example"]).is_err()
        );
    }
}
