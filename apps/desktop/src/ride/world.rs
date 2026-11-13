//! The World menu (spec §3.2.1): which world is ridden, what it is, and
//! its conditions. Bevy-free rows (like [`super::pages`]) and the console
//! command `world list | info | switch`.
//!
//! - *World* (top level): Change world, About this world, Routes, Time
//!   (the time machine, moved here from Navi), Weather.
//! - *Change world*: the worlds [`ocw_worldstore::discover`] finds (cache,
//!   the package given with `--world`, the packages next to it, the data
//!   directories), neighbouring packages folded into one world with parts,
//!   broken ones with their reason; then the built-in test routes.
//! - *A world*: ride its default (the first route of the package, else a
//!   free ride – as `--world` alone), a free ride, its routes, its parts.
//!   Every switch is the in-app ride switch (M3) after "Change the ride?".
//! - *About this world*: package, area and elevation, sun and time at the
//!   rider, data sources with their notices (the credits, spec §13), routes.
//! - *Weather*: clear until Phase 9 S4 ([`crate::weather`]).

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_world::manifest::Attribution;
use ocw_worldstore::discover::{Status, WorldEntry};

use super::choice::{RideChoice, WorldMode};
use super::pages::{Catalog, LoadedWorld};
use crate::settings::pages::{Activate, Page, Row};
use crate::timemachine::TimeMachineInfo;
use crate::weather::WeatherState;

/// What the World pages know about the world ridden now (built when the
/// package is loaded, `setup::build`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldDetails {
    /// Manifest id and version (`local/<name>`, 0 without a manifest).
    pub id: String,
    pub version: u64,
    /// The region's name for people.
    pub region: String,
    pub created_unix_s: Option<i64>,
    /// "verified", "unsigned" or "untrusted".
    pub trust: &'static str,
    /// The covered area (km²), the parts included.
    pub area_km2: Option<f64>,
    /// Lowest and highest terrain (m), the parts included.
    pub elevation_m: Option<(f32, f32)>,
    /// The package's own time zone (section `TZID`).
    pub time_zone: Option<String>,
    /// The credits of the package and its parts (spec §13), each once.
    pub attributions: Vec<Attribution>,
    /// The joined neighbouring packages (names).
    pub parts: Vec<String>,
    pub size_bytes: u64,
}

impl WorldDetails {
    /// The details of a loaded package and its joined neighbours.
    pub fn of(home: &ocw_worldstore::LoadedWorld, parts: &[ocw_worldstore::LoadedWorld]) -> Self {
        let mut attributions: Vec<Attribution> = Vec::new();
        let mut area = 0.0;
        let mut elevation: Option<(f32, f32)> = None;
        for w in std::iter::once(home).chain(parts) {
            let credits = match &w.manifest {
                Some(m) if !m.attribution.is_empty() => m.attribution.clone(),
                _ => vec![Attribution::openstreetmap()],
            };
            for a in credits {
                if !attributions.contains(&a) {
                    attributions.push(a);
                }
            }
            let p = &w.package;
            area += match &p.region {
                Some(r) => r.area_km2(),
                None => p.tiles.len() as f64 * f64::from(p.tile_size_m).powi(2) / 1e6,
            };
            if let Some((lo, hi)) = p.height_range() {
                elevation = Some(elevation.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        }
        let trust = match &home.trust {
            ocw_world::manifest::TrustStatus::Trusted(_) => "verified",
            ocw_world::manifest::TrustStatus::Untrusted(_) if home.manifest.is_none() => "unsigned",
            ocw_world::manifest::TrustStatus::Untrusted(why) if why.contains("not signed") => {
                "unsigned"
            }
            ocw_world::manifest::TrustStatus::Untrusted(_) => "untrusted",
        };
        Self {
            id: home.identity.id.clone(),
            version: home.identity.version,
            region: home
                .manifest
                .as_ref()
                .map_or_else(|| home.package.name.clone(), |m| m.name.clone()),
            created_unix_s: home.manifest.as_ref().map(|m| m.created_unix_s),
            trust,
            area_km2: (area > 0.0).then_some(area),
            elevation_m: elevation,
            time_zone: home.package.time_zone.clone(),
            attributions,
            parts: parts
                .iter()
                .map(|w| {
                    w.manifest
                        .as_ref()
                        .map_or_else(|| w.package.name.clone(), |m| m.name.clone())
                })
                .collect(),
            size_bytes: std::fs::metadata(&home.path).map_or(0, |m| m.len()),
        }
    }
}

/// A route of the loaded world.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteInfo {
    pub name: String,
    pub length_m: f64,
    /// Climbing of one lap (m), when the road graph has a profile.
    pub climb_m: Option<f64>,
}

fn info(label: &str, value: impl Into<String>) -> Row {
    Row {
        label: label.into(),
        value: Some(value.into()),
        left: None,
        right: None,
        confirm: None,
    }
}

fn open(label: &str, value: Option<String>, page: Page) -> Row {
    Row {
        label: label.into(),
        value,
        left: None,
        right: None,
        confirm: Some(Activate::Open(page)),
    }
}

fn ask(label: impl Into<String>, value: impl Into<String>, line: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value.into()),
        left: None,
        right: None,
        confirm: Some(Activate::Ask(line)),
    }
}

/// Shortens `s` to `max` characters (labels must fit the panel).
pub fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 3).collect();
        t.push_str("...");
        t
    }
}

pub fn km(m: f64) -> String {
    format!("{:.1} km", m / 1000.0)
}

/// "12.4 km, 830 m up".
pub fn route_value(r: &RouteInfo) -> String {
    match r.climb_m {
        Some(c) if c >= 1.0 => format!("{}, {:.0} m up", km(r.length_m), c),
        _ => km(r.length_m),
    }
}

/// "2026-11-13" (UTC) of a Unix time.
pub fn iso_date(unix_s: i64) -> String {
    let (y, m, d) = ocw_sky::time::utc_date(unix_s * 1000);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The names the list shows: a name that occurs twice (a test build next
/// to the release) shows its file name instead.
pub fn labels(worlds: &[WorldEntry]) -> Vec<String> {
    worlds
        .iter()
        .map(|w| {
            let twice = worlds.iter().filter(|o| o.name == w.name).count() > 1;
            match (&w.path, twice) {
                (Some(p), true) => p
                    .file_stem()
                    .map_or_else(|| w.name.clone(), |s| s.to_string_lossy().into()),
                _ => w.name.clone(),
            }
        })
        .collect()
}

/// A world's value in the list: "30 MB", "30 MB, unsigned, +1 part",
/// "broken" (version and signer: on its page).
fn list_value(w: &WorldEntry) -> String {
    if w.status.is_broken() {
        return "broken".into();
    }
    let mut v = ocw_worldstore::discover::megabytes(w.size_bytes);
    if !matches!(w.status, Status::Verified(_)) {
        v.push_str(", ");
        v.push_str(w.status.label());
    }
    match w.parts.len() {
        0 => {}
        1 => v.push_str(", +1 part"),
        n => v.push_str(&format!(", +{n} parts")),
    }
    v
}

/// The ride a world's "Ride" row starts: its first route, else a free ride
/// (as `--world` alone does, spec §3.2.1 M3).
pub fn default_choice(w: &WorldEntry) -> RideChoice {
    RideChoice::World {
        world: w.spec.clone(),
        mode: match w.first_route() {
            Some(r) if w.has_graph() => WorldMode::Route(r.into()),
            _ => WorldMode::FreeRide { start: None },
        },
    }
}

/// "route Vaduz - Malbun" / "free ride".
fn default_label(w: &WorldEntry) -> String {
    match default_choice(w) {
        RideChoice::World {
            mode: WorldMode::Route(r),
            ..
        } => format!("route {}", short(&r, 30)),
        _ => "free ride".into(),
    }
}

fn is_current(c: &Catalog, w: &WorldEntry) -> bool {
    c.world.as_ref().is_some_and(|l| {
        l.spec == w.spec
            || (!l.details.id.is_empty()
                && !l.details.id.starts_with("local/")
                && l.details.id == w.id
                && l.details.version == w.version)
    })
}

fn current_name(c: &Catalog) -> String {
    match (&c.world, &c.current) {
        (Some(w), _) if !w.details.region.is_empty() => w.details.region.clone(),
        (Some(w), _) => w.name.clone(),
        (None, Some(choice)) => choice.describe(),
        (None, None) => "-".into(),
    }
}

/// The rows of a World page (`page.is_world_page()`); empty for a world
/// whose index is no longer listed (the menu goes back).
pub fn rows(page: Page, c: &Catalog, tm: &TimeMachineInfo, weather: &WeatherState) -> Vec<Row> {
    match page {
        Page::WorldHome => {
            let routes = c.world.as_ref().map_or(0, |w| w.routes.len()) + c.user_routes.len();
            let time = match (&tm.time, tm.mode_label.as_str()) {
                (Some(t), "real") if tm.speed == 1.0 && !tm.paused => t.clone(),
                (Some(t), m) => format!(
                    "{t}, {}",
                    if tm.paused || tm.speed != 1.0 {
                        ocw_sky::clock::speed_label(tm.speed, tm.paused)
                    } else {
                        m.to_owned()
                    }
                ),
                (None, _) => "-".into(),
            };
            vec![
                open(
                    "Change world",
                    Some(short(&current_name(c), 28)),
                    Page::Worlds,
                ),
                open("About this world", None, Page::WorldInfo),
                open("Routes", Some(routes.to_string()), Page::Routes),
                open("Time", Some(time), Page::TimeMachine),
                open("Weather", Some(weather.summary()), Page::Weather),
                // God mode (spec §14 Phase 13A; the Ride page is full, the
                // ten-foot rule): fly over this world as a unicorn.
                Row {
                    label: crate::god::MENU_NAME.into(),
                    value: Some("fly over it (G)".into()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Run("god toggle".into())),
                },
            ]
        }
        Page::Worlds => {
            let names = labels(&c.worlds);
            let mut out: Vec<Row> = c
                .worlds
                .iter()
                .zip(&names)
                .enumerate()
                .map(|(i, (w, name))| {
                    let mut value = list_value(w);
                    if is_current(c, w) {
                        value.push_str(" - riding");
                    }
                    open(&short(name, 22), Some(value), Page::World(i))
                })
                .collect();
            if out.is_empty() {
                out.push(info("No world packages found", "see README"));
            }
            for (name, label) in [("hills", "Hills"), ("loop", "Loop")] {
                out.push(ask(
                    format!("Test route: {label}"),
                    if matches!(&c.current, Some(RideChoice::TestRoute(r)) if r == name) {
                        "built in - riding"
                    } else {
                        "built in"
                    },
                    RideChoice::TestRoute(name.into()).to_line(),
                ));
            }
            out
        }
        Page::World(i) => {
            let Some(w) = c.worlds.get(i) else {
                return Vec::new();
            };
            if let Some(why) = &w.blocked {
                return vec![
                    info("Cannot ride", short(why, 34)),
                    info("Package", short(&w.detail(), 34)),
                ];
            }
            let current = is_current(c, w);
            let free = |start| RideChoice::World {
                world: w.spec.clone(),
                mode: WorldMode::FreeRide { start },
            };
            let mut out = vec![ask("Ride", default_label(w), default_choice(w).to_line())];
            if w.has_graph() {
                out.push(ask("Free ride", "random start", free(None).to_line()));
            }
            if current
                && let Some(here) = c.here
                && c.world.as_ref().is_some_and(|l| l.has_graph)
            {
                out.push(ask(
                    "Free ride from here",
                    format!("{:.0}, {:.0}", here.0, here.1),
                    free(Some(here)).to_line(),
                ));
            }
            if current {
                let n = c.world.as_ref().map_or(0, |l| l.routes.len()) + c.user_routes.len();
                out.push(open("Routes", Some(n.to_string()), Page::Routes));
            } else if w.routes() > 0 && w.has_graph() {
                out.push(open(
                    "Routes",
                    Some(w.routes().to_string()),
                    Page::WorldRoutes(i),
                ));
            }
            if !w.parts.is_empty() {
                out.push(open(
                    "Parts",
                    Some(w.parts.len().to_string()),
                    Page::WorldParts(i),
                ));
            }
            out.push(info("Package", short(&w.detail(), 30)));
            out
        }
        Page::WorldRoutes(i) => {
            let Some(w) = c.worlds.get(i) else {
                return Vec::new();
            };
            let Some(s) = &w.summary else {
                return Vec::new();
            };
            s.routes
                .iter()
                .map(|r| {
                    ask(
                        short(&r.name, 24),
                        km(super::setup::graph_route_length_m(r)),
                        RideChoice::World {
                            world: w.spec.clone(),
                            mode: WorldMode::Route(r.name.clone()),
                        }
                        .to_line(),
                    )
                })
                .collect()
        }
        Page::WorldParts(i) => {
            let Some(w) = c.worlds.get(i) else {
                return Vec::new();
            };
            w.parts
                .iter()
                .map(|p| {
                    ask(
                        short(&p.name, 24),
                        "free ride here",
                        RideChoice::World {
                            world: p.spec.clone(),
                            mode: WorldMode::FreeRide { start: None },
                        }
                        .to_line(),
                    )
                })
                .collect()
        }
        Page::WorldInfo => info_rows(c, tm),
        Page::WorldSun => sun_rows(tm),
        Page::WorldSources => {
            let credits = credits(c);
            credits
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    open(
                        &short(&a.source, 24),
                        Some(short(&a.licence, 18)),
                        Page::WorldSource(i),
                    )
                })
                .collect()
        }
        Page::WorldSource(i) => {
            let credits = credits(c);
            let Some(a) = credits.get(i) else {
                return Vec::new();
            };
            let mut out = vec![
                info("Source", short(&a.source, 36)),
                info("Licence", short(&a.licence, 36)),
            ];
            if let Some(url) = &a.url {
                let bare = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_start_matches("www.");
                out.push(info("Web", short(bare, 40)));
            }
            out
        }
        Page::Weather => crate::weather::rows(weather),
        _ => Vec::new(),
    }
}

/// The credits of the world ridden now: the package's and its parts' (a
/// test route: OCW's own work, nothing to credit).
fn credits(c: &Catalog) -> Vec<Attribution> {
    c.world
        .as_ref()
        .map(|w| w.details.attributions.clone())
        .unwrap_or_default()
}

fn info_rows(c: &Catalog, tm: &TimeMachineInfo) -> Vec<Row> {
    let zone = tm
        .sun
        .as_ref()
        .map_or("-".to_string(), |s| short(&s.zone, 30));
    let Some(w) = &c.world else {
        return vec![
            info(
                "World",
                c.current
                    .as_ref()
                    .map_or("-".into(), |ch| short(&ch.describe(), 34)),
            ),
            info("Package", "none (a built-in test route)"),
            open("Sun and time", Some(zone), Page::WorldSun),
        ];
    };
    let d = &w.details;
    let parts = match d.parts.len() {
        0 => String::new(),
        1 => ", 1 part".into(),
        n => format!(", {n} parts"),
    };
    let version = if d.id.starts_with("local/") || d.id.is_empty() {
        "local build".to_string()
    } else {
        format!("{} v{}", d.id, d.version)
    };
    let package = match d.created_unix_s {
        Some(t) => format!("{}, {}", short(&version, 28), iso_date(t)),
        None => short(&version, 38),
    };
    let area = match (d.area_km2, d.elevation_m) {
        (Some(a), Some((lo, hi))) => format!("{a:.0} km2, {lo:.0} - {hi:.0} m"),
        (Some(a), None) => format!("{a:.0} km2"),
        (None, Some((lo, hi))) => format!("{lo:.0} - {hi:.0} m"),
        (None, None) => "-".into(),
    };
    let routes = w.routes.len() + c.user_routes.len();
    vec![
        info(
            "World",
            format!("{}, {}{parts}", short(&current_name(c), 22), d.trust),
        ),
        info("Package", package),
        info("Area", area),
        open("Sun and time", Some(zone), Page::WorldSun),
        open(
            "Data sources",
            Some(d.attributions.len().to_string()),
            Page::WorldSources,
        ),
        open("Routes", Some(routes.to_string()), Page::Routes),
    ]
}

fn sun_rows(tm: &TimeMachineInfo) -> Vec<Row> {
    let Some(s) = &tm.sun else {
        return vec![info("World time", "not known yet (joining)")];
    };
    let pair = |p: &(String, String)| format!("{} - {}", p.0, p.1);
    vec![
        info("Time zone", short(&s.zone, 34)),
        info(
            "World time",
            format!("{} {}", s.date, tm.time.clone().unwrap_or_default()),
        ),
        info("Sun", format!("{}, noon {}", pair(&s.sun), s.noon)),
        info("Civil twilight", pair(&s.civil)),
        info("Nautical twilight", pair(&s.nautical)),
        info("Astronomical", pair(&s.astronomical)),
    ]
}

/// Text paragraphs of a World page beyond its notes: a data source's full
/// notice.
pub fn text(page: Page, c: &Catalog) -> Vec<String> {
    match page {
        Page::WorldSource(i) => credits(c)
            .get(i)
            .map(|a| vec![a.notice.clone()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `world list`: every world found, numbered for `world switch N`.
pub fn list_text(c: &Catalog) -> String {
    if c.worlds.is_empty() {
        return "no world packages found (the world cache, the files next to --world, data/, data/pilot/ and next to the app); test routes: hills, loop".into();
    }
    let mut out = String::new();
    for (i, w) in c.worlds.iter().enumerate() {
        let name = &w.name;
        let riding = if is_current(c, w) { "  [riding]" } else { "" };
        out.push_str(&format!(
            "{:>2}. {name}  {}  {}  {} routes, {}{riding}\n    {}  ({})\n",
            i + 1,
            w.id,
            w.detail(),
            w.routes(),
            w.source.label(),
            w.spec,
            match &w.status {
                Status::Verified(by) => format!("signed by {by}"),
                Status::Unsigned => "no signature".into(),
                Status::Untrusted(why) | Status::Broken(why) => why.clone(),
            }
        ));
        if let Some(why) = &w.blocked
            && !w.status.is_broken()
        {
            out.push_str(&format!("    cannot ride: {why}\n"));
        }
        for p in &w.parts {
            out.push_str(&format!("    part: {}  {}  {}\n", p.name, p.id, p.spec));
        }
        for a in &w.also {
            out.push_str(&format!("    also: {a}\n"));
        }
    }
    out.push_str("test routes: hills, loop (world switch hills)");
    out
}

/// `world info`: the world ridden now (the About page as text).
pub fn info_text(c: &Catalog, tm: &TimeMachineInfo) -> String {
    let mut out = String::new();
    let line = |out: &mut String, l: String| {
        out.push_str(&l);
        out.push('\n');
    };
    match &c.world {
        None => line(
            &mut out,
            format!(
                "world: {} (no world package)",
                c.current.as_ref().map_or("-".into(), RideChoice::describe)
            ),
        ),
        Some(w) => {
            let d = &w.details;
            line(
                &mut out,
                format!("world: {} ({})", current_name(c), d.trust),
            );
            line(
                &mut out,
                format!("package: {} v{}, {}", d.id, d.version, w.spec),
            );
            if let Some(t) = d.created_unix_s {
                line(&mut out, format!("built: {}", iso_date(t)));
            }
            if let Some(a) = d.area_km2 {
                line(&mut out, format!("area: {a:.0} km2"));
            }
            if let Some((lo, hi)) = d.elevation_m {
                line(&mut out, format!("elevation: {lo:.0} - {hi:.0} m"));
            }
            if !d.parts.is_empty() {
                line(&mut out, format!("parts: {}", d.parts.join(", ")));
            }
            line(&mut out, "data sources:".into());
            for a in &d.attributions {
                line(
                    &mut out,
                    format!("  {} ({}): {}", a.source, a.licence, a.notice),
                );
            }
            line(&mut out, format!("routes: {}", w.routes.len()));
            for r in &w.routes {
                line(&mut out, format!("  {}: {}", r.name, route_value(r)));
            }
        }
    }
    if let Some(s) = &tm.sun {
        line(&mut out, format!("time zone: {}", s.zone));
        line(
            &mut out,
            format!(
                "world time: {} {} ({})",
                s.date,
                tm.time.clone().unwrap_or_default(),
                tm.mode_label
            ),
        );
        line(
            &mut out,
            format!("sun: rise {}, noon {}, set {}", s.sun.0, s.noon, s.sun.1),
        );
        line(
            &mut out,
            format!(
                "twilight: civil {} - {}, nautical {} - {}, astronomical {} - {}",
                s.civil.0,
                s.civil.1,
                s.nautical.0,
                s.nautical.1,
                s.astronomical.0,
                s.astronomical.1
            ),
        );
    }
    out.trim_end().to_owned()
}

/// What `world switch ARG` rides: a number of `world list`, an id,
/// `id@version`, a name or file name of the list, a package path, or a
/// test route; a world's default ride (first route, else free ride).
pub fn switch_choice(c: &Catalog, arg: &str) -> Result<RideChoice, String> {
    let arg = arg.trim();
    if super::choice::TEST_ROUTES.contains(&arg) {
        return Ok(RideChoice::TestRoute(arg.into()));
    }
    if let Ok(n) = arg.parse::<usize>() {
        let w = c
            .worlds
            .get(n.wrapping_sub(1))
            .ok_or_else(|| format!("no world {n} (world list has {})", c.worlds.len()))?;
        return entry_choice(w);
    }
    match ocw_server::worlds::resolve(&c.worlds, arg) {
        Ok(w) => entry_choice(w),
        Err(e) if std::path::Path::new(arg).is_file() => {
            let _ = e;
            Ok(RideChoice::World {
                world: arg.into(),
                mode: WorldMode::FreeRide { start: None },
            })
        }
        Err(e) => Err(e),
    }
}

fn entry_choice(w: &WorldEntry) -> Result<RideChoice, String> {
    match &w.blocked {
        Some(why) => Err(format!("{}: {why}", w.name)),
        None => Ok(default_choice(w)),
    }
}

/// The current world for the catalog: from the ride now.
pub fn loaded_world(cur: &super::CurrentWorld) -> LoadedWorld {
    LoadedWorld {
        spec: cur.spec.clone(),
        name: cur.name.clone(),
        routes: cur.routes.clone(),
        has_graph: cur.net.is_some(),
        details: cur.details.clone(),
    }
}

/// `world list | info | switch ARG` (user commands, like `ride.switch`).
pub fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("world")
            .about("The worlds you can ride: list them, show the current one, switch without restarting")
            .subcommand_required(true)
            .subcommand(Command::new("list").about("The world packages found (numbered for world switch)"))
            .subcommand(Command::new("info").about(
                "The world ridden now: package, area, data sources, time zone, sun times, routes",
            ))
            .subcommand(
                Command::new("switch")
                    .about("Ride another world (its first route, else a free ride); the ride so far is saved")
                    .arg(
                        Arg::new("world")
                            .required(true)
                            .value_name("N|ID|NAME|PATH")
                            .help("A number of world list, a package id, name or path, or a test route (hills, loop)"),
                    ),
            ),
        |world, m| {
            super::refresh_catalog(world);
            let c = world.resource::<super::RideCatalog>().0.clone();
            match m.subcommand() {
                Some(("list", _)) => Ok(list_text(&c)),
                Some(("info", _)) => {
                    let tm = world
                        .get_resource::<TimeMachineInfo>()
                        .cloned()
                        .unwrap_or_default();
                    Ok(info_text(&c, &tm))
                }
                Some(("switch", s)) => {
                    let arg = s.get_one::<String>("world").expect("required");
                    let choice = switch_choice(&c, arg)?;
                    super::begin_switch(world, choice)
                }
                _ => Err("world list | info | switch N|ID|NAME|PATH".into()),
            }
        },
    ));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, MAX_ITEMS, Menu, Page as MenuPage, row_label};
    use crate::ride::pages::MAX_ROW_CHARS;
    use crate::timemachine::SunToday;
    use ocw_world::summary::PackageSummary;
    use ocw_worldstore::discover::Source;

    fn entry(id: &str, name: &str, spec: &str, routes: &[&str]) -> WorldEntry {
        WorldEntry {
            spec: spec.into(),
            id: id.into(),
            version: 3,
            name: name.into(),
            path: Some(spec.into()),
            source: Source::Folder,
            size_bytes: 30_174_974,
            created_unix_s: Some(1_794_528_000),
            status: Status::Verified("OCW".into()),
            blocked: None,
            summary: Some(PackageSummary {
                name: name.into(),
                has_graph: true,
                routes: routes
                    .iter()
                    .map(|r| ocw_world::GraphRoute {
                        name: (*r).into(),
                        steps: vec![ocw_world::RouteStep::Edge {
                            id: ocw_world::graph::EdgeId(1),
                            from_m: 0.0,
                            to_m: 12_400.0,
                        }],
                        is_loop: false,
                    })
                    .collect(),
                ..PackageSummary::default()
            }),
            attributions: vec![Attribution::openstreetmap(), Attribution::srtm()],
            parts: Vec::new(),
            also: Vec::new(),
        }
    }

    /// Liechtenstein (ridden now, with a neighbour), the Black Forest, an
    /// unsigned test build and a broken file.
    pub(crate) fn catalog() -> Catalog {
        let mut li = entry(
            "europe/liechtenstein",
            "Liechtenstein",
            "/var/tmp/ocw/liechtenstein.ocwpkg",
            &["Vaduz - Malbun", "Rhine valley"],
        );
        li.parts.push(entry(
            "europe/graubuenden",
            "Graubuenden",
            "/var/tmp/ocw/gr.ocwpkg",
            &[],
        ));
        let bf = entry(
            "europe/blackforest",
            "Black Forest",
            "/var/tmp/ocw/blackforest.ocwpkg",
            &["Freiburg - Schauinsland"],
        );
        let mut free = entry(
            "local/liechtenstein-freeride",
            "Liechtenstein",
            "/var/tmp/ocw/liechtenstein-freeride.ocwpkg",
            &[],
        );
        free.status = Status::Unsigned;
        let mut zip = entry("local/zip", "zip", "/var/tmp/ocw/zip.ocwpkg", &[]);
        zip.status = Status::Broken("not an OCW world package (OCWPKG01/02)".into());
        zip.blocked = Some("not an OCW world package (OCWPKG01/02)".into());
        zip.summary = None;
        Catalog {
            worlds: vec![li, bf, free, zip],
            current: Some(RideChoice::World {
                world: "/var/tmp/ocw/liechtenstein.ocwpkg".into(),
                mode: WorldMode::Route("Vaduz - Malbun".into()),
            }),
            world: Some(LoadedWorld {
                spec: "/var/tmp/ocw/liechtenstein.ocwpkg".into(),
                name: "Liechtenstein".into(),
                routes: vec![RouteInfo {
                    name: "Vaduz - Malbun".into(),
                    length_m: 15_900.0,
                    climb_m: Some(1_180.0),
                }],
                has_graph: true,
                details: WorldDetails {
                    id: "europe/liechtenstein".into(),
                    version: 3,
                    region: "Liechtenstein".into(),
                    created_unix_s: Some(1_794_528_000),
                    trust: "verified",
                    area_km2: Some(160.4),
                    elevation_m: Some((430.0, 2599.0)),
                    time_zone: Some("Europe/Vaduz".into()),
                    attributions: vec![Attribution::openstreetmap(), Attribution::srtm()],
                    parts: vec!["Graubuenden".into()],
                    size_bytes: 30_174_974,
                },
            }),
            here: Some((120.4, -88.0)),
            ..Catalog::default()
        }
    }

    pub(crate) fn time_machine() -> TimeMachineInfo {
        let zone = ocw_sky::Zone::get("Europe/Vaduz").unwrap();
        let (ms, _) = zone.instant((2026, 11, 13), (14, 5, 0)).unwrap();
        TimeMachineInfo {
            time: Some("14:05 CET".into()),
            date: Some("Fri 13 Nov 2026".into()),
            mode_label: "real".into(),
            speed: 1.0,
            control: "you".into(),
            may_change: true,
            sun: Some(SunToday::at(ms, &zone, (47.141, 9.521))),
            ..TimeMachineInfo::default()
        }
    }

    fn rows_of(c: &Catalog) -> impl Fn(Page) -> Vec<Row> + '_ {
        move |p| {
            if p.is_world_page() {
                rows(p, c, &time_machine(), &WeatherState::default())
            } else {
                crate::ride::pages::rows(p, c)
            }
        }
    }

    /// The menu over `c`, opened at the top and clicked along `path`.
    fn open_at(c: &Catalog, path: &[usize]) -> Menu {
        let mut m = Menu::default();
        let r = rows_of(c);
        m.apply_with(Action::Open, &r);
        for &i in path {
            m.apply_with(Action::Click(i), &r);
        }
        m
    }

    #[test]
    fn the_world_page_shows_the_world_and_its_conditions() {
        let c = catalog();
        // Top → World.
        let m = open_at(&c, &[2]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WorldHome)));
        assert_eq!(
            m.labels(),
            [
                "Change world:  Liechtenstein",
                "About this world",
                "Routes:  1",
                "Time:  14:05 CET",
                "Weather:  clear",
                "God mode:  fly over it (G)",
                "Back"
            ]
        );
        // A time-lapse shows its speed; Time opens the time machine.
        let mut tm = time_machine();
        tm.speed = 600.0;
        tm.mode_label = "fixed".into();
        assert_eq!(
            row_label(&rows(Page::WorldHome, &c, &tm, &WeatherState::default())[3]),
            "Time:  14:05 CET, x600"
        );
        let m = open_at(&c, &[2, 3]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::TimeMachine)));
        let m = open_at(&c, &[2, 4]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Weather)));
        assert_eq!(m.labels()[0], "Now:  clear (no simulation yet)");
    }

    #[test]
    fn the_world_list_switches_after_asking() {
        let c = catalog();
        let r = rows_of(&c);
        // World → Change world.
        let mut m = open_at(&c, &[2, 0]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Worlds)));
        let l = m.labels();
        assert!(l.len() <= MAX_ITEMS, "{l:?}");
        assert_eq!(
            l,
            [
                "liechtenstein:  30 MB, +1 part - riding",
                "Black Forest:  30 MB",
                "liechtenstein-freeride:  30 MB, unsigned",
                "zip:  broken",
                "Test route: Hills:  built in",
                "Test route: Loop:  built in",
                "Back"
            ]
        );
        assert_eq!(m.apply_with(Action::Click(5), &r), Effect::None);
        assert_eq!(m.pending(), Some("ride.switch --route loop"));
        // The Black Forest: its default ride (the first route), after asking.
        let mut m = open_at(&c, &[2, 0, 1]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::World(1))));
        assert_eq!(
            m.labels(),
            [
                "Ride:  route Freiburg - Schauinsland",
                "Free ride:  random start",
                "Routes:  1",
                "Package:  v3, 30 MB, verified",
                "Back"
            ]
        );
        assert_eq!(m.apply_with(Action::Click(0), &r), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::ConfirmSwitch));
        assert_eq!(m.labels()[m.focus()], "Cancel");
        assert_eq!(
            m.pending(),
            Some(
                "ride.switch --world \"/var/tmp/ocw/blackforest.ocwpkg\" --route \"Freiburg - Schauinsland\""
            )
        );
        m.apply_with(Action::Up, &r);
        assert_eq!(
            m.apply_with(Action::Confirm, &r),
            Effect::Command(
                "ride.switch --world \"/var/tmp/ocw/blackforest.ocwpkg\" --route \"Freiburg - Schauinsland\"".into()
            )
        );
        assert!(!m.is_open());
        // Its routes, from the package summary.
        let mut m = open_at(&c, &[2, 0, 1, 2]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WorldRoutes(1))));
        assert_eq!(m.labels()[0], "Freiburg - Schauinsland:  12.4 km");
        m.apply_with(Action::Click(0), &r);
        assert_eq!(m.page(), Some(MenuPage::ConfirmSwitch));
        // The world ridden now: from here, its routes (Navi's), its parts.
        let m = open_at(&c, &[2, 0, 0]);
        assert_eq!(
            m.labels(),
            [
                "Ride:  route Vaduz - Malbun",
                "Free ride:  random start",
                "Free ride from here:  120, -88",
                "Routes:  1",
                "Parts:  1",
                "Package:  v3, 30 MB, verified",
                "Back"
            ]
        );
        let mut m = open_at(&c, &[2, 0, 0, 4]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WorldParts(0))));
        assert_eq!(m.labels()[0], "Graubuenden:  free ride here");
        m.apply_with(Action::Click(0), &r);
        assert_eq!(
            m.pending(),
            Some("ride.switch --world \"/var/tmp/ocw/gr.ocwpkg\" --free-ride")
        );
        // A world without routes: a free ride by default.
        let m = open_at(&c, &[2, 0, 2]);
        assert_eq!(m.labels()[0], "Ride:  free ride");
        // A broken package says why and offers nothing to ride.
        let m = open_at(&c, &[2, 0, 3]);
        let l = m.labels();
        assert!(
            l[0].starts_with("Cannot ride:  not an OCW world package"),
            "{l:?}"
        );
        assert_eq!(&l[1..], ["Package:  local, 30 MB, broken", "Back"]);
    }

    #[test]
    fn the_info_page_shows_package_sources_sun_and_routes() {
        let c = catalog();
        let tm = time_machine();
        let r = |p| rows(p, &c, &tm, &WeatherState::default());
        let l: Vec<String> = r(Page::WorldInfo).iter().map(row_label).collect();
        assert_eq!(
            l,
            [
                "World:  Liechtenstein, verified, 1 part",
                "Package:  europe/liechtenstein v3, 2026-11-13",
                "Area:  160 km2, 430 - 2599 m",
                "Sun and time:  Europe/Vaduz, UTC+01:00",
                "Data sources:  2",
                "Routes:  1",
            ]
        );
        // Sun times at the rider on the world's date (NOAA, ± 1 min).
        let l: Vec<String> = r(Page::WorldSun).iter().map(row_label).collect();
        assert_eq!(l[0], "Time zone:  Europe/Vaduz, UTC+01:00");
        assert_eq!(l[1], "World time:  Fri 13 Nov 2026 14:05 CET");
        assert!(l[2].starts_with("Sun:  07:"), "{}", l[2]);
        assert!(
            l[2].contains(" - 16:") && l[2].contains("noon 12:"),
            "{}",
            l[2]
        );
        assert!(l[3].starts_with("Civil twilight:  0"), "{}", l[3]);
        assert_eq!(l.len(), 6);
        // Data sources: one row each, the full notice as the page's text.
        let l: Vec<String> = r(Page::WorldSources).iter().map(row_label).collect();
        assert_eq!(
            l,
            [
                "OpenStreetMap:  ODbL-1.0",
                "SRTM elevation data:  public-domain"
            ]
        );
        let l: Vec<String> = r(Page::WorldSource(0)).iter().map(row_label).collect();
        assert_eq!(
            l,
            [
                "Source:  OpenStreetMap",
                "Licence:  ODbL-1.0",
                "Web:  openstreetmap.org/copyright"
            ]
        );
        let t = text(Page::WorldSource(0), &c);
        assert!(t[0].starts_with("© OpenStreetMap contributors."), "{t:?}");
        assert!(
            r(Page::WorldSource(5)).is_empty(),
            "gone: the menu goes back"
        );
        // Routes with length and climbing (Navi's list, from World too).
        let routes = crate::ride::pages::rows(Page::Routes, &c);
        assert_eq!(row_label(&routes[0]), "Vaduz - Malbun:  15.9 km, 1180 m up");
        // A test route: no package, still the sun.
        let test = Catalog {
            current: Some(RideChoice::TestRoute("hills".into())),
            ..Catalog::default()
        };
        let l: Vec<String> = rows(Page::WorldInfo, &test, &tm, &WeatherState::default())
            .iter()
            .map(row_label)
            .collect();
        assert_eq!(l[0], "World:  test route hills");
        assert!(l[1].contains("test route"), "{l:?}");
        assert!(rows(Page::WorldSources, &test, &tm, &WeatherState::default()).is_empty());
    }

    #[test]
    fn world_rows_fit_the_ten_foot_limits() {
        let mut c = catalog();
        c.worlds[1].name = "A very long world package name that goes on and on".into();
        let tm = time_machine();
        for p in [
            Page::WorldHome,
            Page::Worlds,
            Page::World(0),
            Page::World(1),
            Page::World(3),
            Page::WorldRoutes(1),
            Page::WorldParts(0),
            Page::WorldInfo,
            Page::WorldSun,
            Page::WorldSources,
            Page::WorldSource(0),
            Page::WorldSource(1),
            Page::Weather,
        ] {
            let rows = rows(p, &c, &tm, &WeatherState::default());
            assert!(!rows.is_empty(), "{p:?}");
            for row in rows {
                let l = row_label(&row);
                assert!(l.chars().count() <= MAX_ROW_CHARS, "{p:?}: {l}");
                let missing = ocw_render::font::missing(&l);
                assert!(missing.is_empty(), "{p:?}: {l}: {missing:?}");
            }
            for t in p.notes().iter().map(|s| s.to_string()).chain(text(p, &c)) {
                assert!(ocw_render::font::missing(&t).is_empty(), "{p:?}: {t}");
            }
        }
    }

    #[test]
    fn world_commands_list_inform_and_choose() {
        let c = catalog();
        let list = list_text(&c);
        assert!(
            list.contains(" 1. Liechtenstein  europe/liechtenstein"),
            "{list}"
        );
        assert!(
            list.contains("[riding]") && list.contains("part: Graubuenden"),
            "{list}"
        );
        assert!(
            list.contains(" 4. zip") && list.contains("not an OCW world package"),
            "{list}"
        );
        let info = info_text(&c, &time_machine());
        for want in [
            "world: Liechtenstein (verified)",
            "package: europe/liechtenstein v3",
            "built: 2026-11-13",
            "elevation: 430 - 2599 m",
            "parts: Graubuenden",
            "OpenStreetMap (ODbL-1.0): © OpenStreetMap contributors",
            "Elevation: SRTM, courtesy of NASA / USGS",
            "Vaduz - Malbun: 15.9 km, 1180 m up",
            "time zone: Europe/Vaduz, UTC+01:00",
            "sun: rise 07:",
        ] {
            assert!(info.contains(want), "{want} in\n{info}");
        }
        // world switch: by number, id, name, file name, test route.
        let line = |a: &str| switch_choice(&c, a).map(|c| c.to_line());
        let bf = "ride.switch --world \"/var/tmp/ocw/blackforest.ocwpkg\" --route \"Freiburg - Schauinsland\"";
        for a in [
            "2",
            "europe/blackforest",
            "black forest",
            "blackforest.ocwpkg",
            "europe/blackforest@3",
        ] {
            assert_eq!(line(a).as_deref(), Ok(bf), "{a}");
        }
        assert_eq!(line("hills").unwrap(), "ride.switch --route hills");
        assert_eq!(
            line("3").unwrap(),
            "ride.switch --world \"/var/tmp/ocw/liechtenstein-freeride.ocwpkg\" --free-ride"
        );
        assert!(line("4").unwrap_err().contains("not an OCW world package"));
        assert!(line("9").unwrap_err().contains("no world 9"));
        assert!(line("0").unwrap_err().contains("no world 0"));
        assert!(line("Atlantis").unwrap_err().contains("no world"));
    }
}
