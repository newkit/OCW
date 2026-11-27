//! Traffic in both apps (spec 14 Phase 18, 18A, T1): the `[traffic]`
//! settings as a resource, the `traffic` console command (show, and set
//! within the access rules), the road-traffic request a new session's
//! local server gets, and the Settings → Traffic page's session info.
//!
//! Road traffic is the session's (the server decides: the single player on
//! the local servers, the host of a shared ride, the organiser in events,
//! Q2) and in the desktop app only (Q9 (b)); aircraft, UFOs and the sounds
//! are the rider's own. Nothing draws vehicles yet: these are the switches
//! and the state the next milestones read.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::client::ClientCore;
use ocw_client::live_air::LiveState;
use ocw_client::menu::model::Platform;
use ocw_client::menu::traffic::{LIVE_ROAD_NONE, TrafficMenuInfo};
use ocw_core::traffic::{AirTraffic, DriverManners, RoadRules, RoadTraffic, Ufos, ViewChoice};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::UiPlatform;

/// The rider's traffic settings (the `[traffic]` rows; their apply
/// commands keep this up to date).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrafficSettings {
    /// `traffic.road`, `traffic.manners`: what the rider asks of the
    /// sessions they decide.
    pub road: RoadRules,
    /// `traffic.enabled`, `traffic.air`, `traffic.ufo`, `traffic.sound`.
    pub view: ViewChoice,
    /// `traffic.live_road` (Q5: no world has a source yet).
    pub live_road: bool,
}

impl TrafficSettings {
    /// The road traffic the rider asks for: none with the master switch
    /// off.
    pub fn road_asked(&self) -> RoadRules {
        if self.view.enabled {
            self.road
        } else {
            RoadRules {
                level: RoadTraffic::Off,
                ..self.road
            }
        }
    }

    /// The request for a new session's local server, if the rider asks
    /// for anything but the servers' default (no road traffic, polite).
    pub fn session_message(&self) -> Option<Vec<u8>> {
        let r = self.road_asked();
        (r != RoadRules::default())
            .then(|| ClientCore::traffic_message(Some(r.level), Some(r.manners)))
    }
}

/// With *Aircraft: live*: whether the live aircraft are drawn now, or
/// why not (spec 18E T10; `None`: live is not chosen).
pub fn live_air_state(world: &World) -> Option<LiveState> {
    let s = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    if !ocw_client::air::AirClient::wants_live(&s.view) {
        return None;
    }
    let clock = world
        .get_resource::<ocw_render::RideView>()
        .and_then(|v| v.sky.as_ref().map(|s| s.sky.instant));
    Some(match (crate::client(world), clock) {
        (Some(c), Some(ms)) if c.rider_id().is_some() => c.live_air().state(ms),
        _ => LiveState::Waiting,
    })
}

/// What the Settings → Traffic page shows of the session.
pub fn menu_info(world: &World, platform: Platform) -> TrafficMenuInfo {
    let live_air = live_air_state(world);
    match crate::client(world) {
        Some(c) if c.rider_id().is_some() => TrafficMenuInfo {
            platform,
            session: c.traffic().info(),
            may_change: c.may_change_traffic(),
            decided_by: c.traffic().decided_by(c.rider_id(), c.host_rider()),
            live_air,
        },
        _ => TrafficMenuInfo {
            live_air,
            ..TrafficMenuInfo::alone(platform)
        },
    }
}

fn platform(world: &World) -> Platform {
    world
        .get_resource::<UiPlatform>()
        .map_or(Platform::Desktop, |p| p.0)
}

const WHAT: [&str; 7] = [
    "road",
    "manners",
    "air",
    "ufo",
    "sound",
    "enabled",
    "live_road",
];

/// `traffic`: everything the rider and the session have.
pub fn summary(world: &World) -> String {
    let s = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    let mut out = Vec::new();
    out.push(format!(
        "traffic: {}",
        if s.view.enabled { "on" } else { "off" }
    ));
    if platform(world) == Platform::Desktop {
        match crate::client(world).filter(|c| c.rider_id().is_some()) {
            Some(c) => {
                let t = c.traffic();
                out.push(format!(
                    "road traffic: {} (decided by {})",
                    ocw_server_describe(t.road()),
                    t.decided_by(c.rider_id(), c.host_rider())
                ));
            }
            None => out.push(format!(
                "road traffic: {} (for the next ride)",
                ocw_server_describe(s.road_asked())
            )),
        }
    } else {
        out.push("road traffic: in the desktop app only".into());
    }
    let air = s.view.air_shown();
    out.push(format!(
        "aircraft: {}{}",
        air.key(),
        match live_air_state(world) {
            Some(LiveState::Live) => {
                let credit = crate::client(world)
                    .and_then(|c| c.live_air().attribution().map(str::to_owned))
                    .unwrap_or_default();
                format!(" ({credit})")
            }
            Some(st) => format!(" ({})", st.label()),
            None => String::new(),
        }
    ));
    let in_event = crate::client(world).is_some_and(|c| c.traffic().by_event());
    out.push(format!(
        "UFOs: {}{}",
        s.view.ufo.key(),
        if in_event && s.view.ufo != Ufos::Off {
            " (none in events)"
        } else {
            ""
        }
    ));
    out.push(format!(
        "sound: {}",
        if s.view.sound { "on" } else { "off" }
    ));
    if let Some(c) = crate::client(world).filter(|c| c.rider_id().is_some()) {
        out.push(format!("air seed: {:016x}", c.traffic().air_seed()));
    }
    if platform(world) == Platform::Desktop {
        out.push(format!(
            "live traffic data: {} ({LIVE_ROAD_NONE})",
            if s.live_road { "on" } else { "off" }
        ));
    }
    out.join("\n")
}

/// "busy, drivers polite" (the server's wording).
fn ocw_server_describe(r: RoadRules) -> String {
    match r.level {
        RoadTraffic::Off => "off".into(),
        RoadTraffic::Lunatic => "lunatic".into(),
        l => format!("{}, drivers {}", l.key(), r.manners.label()),
    }
}

/// Sends the rider's road traffic to the session's server, if a ride runs
/// and the rider may change it (the server answers and checks again).
fn send_road(world: &mut World, rules: RoadRules) -> Result<String, String> {
    if platform(world) != Platform::Desktop {
        return Err("road traffic is in the desktop app only".into());
    }
    let Some(c) = crate::client(world) else {
        return Ok("applies to the next ride".into());
    };
    if c.rider_id().is_some() && !c.may_change_traffic() {
        let by = c.traffic().decided_by(c.rider_id(), c.host_rider());
        return Err(if c.traffic().by_event() {
            "the event's organiser set the road traffic; it can be changed when the event is over"
                .into()
        } else {
            format!("the road traffic of this ride is decided by {by}")
        });
    }
    crate::send(
        world,
        ClientCore::traffic_message(Some(rules.level), Some(rules.manners)),
    );
    // The server confirms.
    Ok(String::new())
}

fn on_off(v: &str) -> Result<bool, String> {
    match v {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err("on or off".into()),
    }
}

/// `traffic [WHAT VALUE]`.
pub fn run(world: &mut World, what: Option<&str>, value: Option<&str>) -> Result<String, String> {
    let Some(what) = what else {
        return Ok(summary(world));
    };
    let Some(value) = value else {
        return Err(format!("traffic {what} needs a value"));
    };
    let mut s = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    let said = match what {
        "road" => {
            s.road.level = RoadTraffic::parse(value)
                .ok_or("road traffic: off, light, normal, busy or lunatic")?;
            None
        }
        "manners" => {
            s.road.manners = DriverManners::parse(value)
                .ok_or("driver manners: polite, mostly_polite or some_realism")?;
            None
        }
        "enabled" => {
            s.view.enabled = on_off(value)?;
            Some(format!(
                "traffic {}",
                if s.view.enabled { "on" } else { "off" }
            ))
        }
        "air" => {
            s.view.air = AirTraffic::parse(value).ok_or("aircraft: off, simulated or live")?;
            Some(match s.view.air {
                AirTraffic::Live => format!(
                    "aircraft: live (from {}, fetched by the game server; the simulated sky while it cannot be reached)",
                    ocw_client::menu::traffic::AIR_LIVE_SOURCE
                ),
                a => format!("aircraft: {}", a.key()),
            })
        }
        "ufo" => {
            s.view.ufo = Ufos::parse(value).ok_or("UFOs: off, rare or often")?;
            Some(format!("UFOs: {}", s.view.ufo.key()))
        }
        "sound" => {
            s.view.sound = on_off(value)?;
            Some(format!(
                "traffic sound {}",
                if s.view.sound { "on" } else { "off" }
            ))
        }
        "live_road" => {
            s.live_road = on_off(value)?;
            Some(format!(
                "live traffic data {} ({LIVE_ROAD_NONE})",
                if s.live_road { "on" } else { "off" }
            ))
        }
        _ => return Err(format!("traffic: one of {}", WHAT.join(", "))),
    };
    let before = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    world.insert_resource(s);
    // The road traffic the rider asks for changed: tell the server (the
    // desktop; the browser has no road traffic, Q9).
    if s.road_asked() != before.road_asked() || matches!(what, "road" | "manners") {
        if platform(world) != Platform::Desktop {
            return match said {
                Some(m) => Ok(m),
                None => Err("road traffic is in the desktop app only".into()),
            };
        }
        // The master switch of a guest in a shared ride: the host's cars
        // stay (they can touch the rider).
        if what == "enabled"
            && crate::client(world)
                .is_some_and(|c| c.rider_id().is_some() && !c.may_change_traffic())
        {
            return Ok(format!(
                "{}; the road traffic of this ride stays (decided by the host)",
                said.unwrap_or_default()
            ));
        }
        let sent = send_road(world, s.road_asked())?;
        return Ok(match said {
            Some(m) if sent.is_empty() => m,
            Some(m) => format!("{m}; road traffic {sent}"),
            None => sent,
        });
    }
    Ok(said.unwrap_or_default())
}

/// Where a camera stands to see a bus at (or coming to) a stop: beside
/// the kerb side, `ahead` metres ahead of `at` (x, z) along `heading`,
/// looking back at `at`; (x, z, compass yaw in degrees: 0 north, 90 east).
pub fn bus_camera(at: [f64; 2], heading: [f64; 2], keep_left: bool, ahead: f64) -> [f64; 3] {
    let side = if keep_left { -1.0 } else { 1.0 };
    let right = [-heading[1] * side, heading[0] * side];
    let cam = [
        at[0] + right[0] * 7.0 + heading[0] * ahead,
        at[1] + right[1] * 7.0 + heading[1] * ahead,
    ];
    let (dx, dz) = (at[0] - cam[0], at[1] - cam[1]);
    let yaw = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
    [cam[0], cam[1], yaw]
}

/// Whether the line of sight from `a` to `b` (x, z) crosses no wall,
/// fence or building of the world's obstacles.
fn clear_view(cache: &mut ocw_sim::collide::WorldCache, a: [f64; 2], b: [f64; 2]) -> bool {
    use ocw_world::obstacles::{ObstacleKind, Shape};
    let lo = [a[0].min(b[0]) - 1.0, a[1].min(b[1]) - 1.0];
    let hi = [a[0].max(b[0]) + 1.0, a[1].max(b[1]) + 1.0];
    let mut solids = Vec::new();
    cache.solids_near(lo, hi, 0, &mut solids);
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    !solids.iter().any(|s| match s.shape {
        Shape::Segment { a: c, b: d, .. }
            if matches!(
                s.kind,
                ObstacleKind::Building | ObstacleKind::Wall | ObstacleKind::Hedge
            ) =>
        {
            cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0
        }
        _ => false,
    })
}

/// `traffic.bus`: the buses on their lines near the rider, or (`watch`)
/// the unicorn's camera beside one at a stop.
fn bus_command(world: &mut World, mode: Option<&str>) -> Result<String, String> {
    if mode == Some("off") {
        let mut view = world.resource_mut::<ocw_render::RideView>();
        view.guided_camera = None;
        return Ok("camera: chase".into());
    }
    let watch = mode.is_some();
    let coming = mode == Some("coming");
    use ocw_sim::traffic::{VehicleClass, bus, world::WorldRules};
    let client = crate::client(world).ok_or("not riding")?;
    let net = client
        .free_network()
        .cloned()
        .ok_or("buses run in free rides on worlds with bus lines")?;
    let view = world
        .get_resource::<ocw_render::RideView>()
        .ok_or("no view")?;
    let me = view
        .local()
        .map(|l| {
            let p = client.track().sample(l.state.distance).position;
            [p[0], p[2]]
        })
        .unwrap_or([0.0, 0.0]);
    let mut buses: Vec<_> = view
        .vehicles
        .iter()
        .filter(|v| v.class == VehicleClass::Bus && v.line != 0)
        .copied()
        .collect();
    if buses.is_empty() {
        return Err("no bus on its line near the rider".into());
    }
    let d = |v: &ocw_render::traffic::VehicleView| (v.pos[0] - me[0]).hypot(v.pos[2] - me[1]);
    buses.sort_by(|a, b| d(a).total_cmp(&d(b)));
    let rules = WorldRules::new(&net);
    let core: Vec<_> = (0..net.graph().edges.len())
        .map(|e| ocw_sim::traffic::EdgeRules::edge_rule(&rules, &net, e))
        .collect();
    let (lines, _) = bus::lines_of(&net, &core, 0);
    let line_of = |id: u32| lines.iter().find(|l| l.id == id);
    if !watch {
        let rows: Vec<String> = buses
            .iter()
            .take(8)
            .map(|v| {
                let sign = line_of(v.line).map_or_else(|| "?".to_owned(), |l| l.sign());
                format!(
                    "bus {} \"{sign}\" {:.0} m away, {:.0} km/h{}",
                    v.id,
                    d(v),
                    v.speed * 3.6,
                    if v.doors { ", doors open" } else { "" }
                )
            })
            .collect();
        return Ok(rows.join("; "));
    }
    // A bus at its stop with the doors open, else the nearest coming to one.
    let dwelling = if coming {
        None
    } else {
        buses.iter().find(|v| v.doors)
    };
    let (at, heading, what) = if let Some(v) = dwelling {
        let sign = line_of(v.line).map_or_else(String::new, |l| l.sign());
        (
            [v.pos[0], v.pos[2]],
            v.heading,
            format!("bus {} \"{sign}\" at its stop", v.id),
        )
    } else {
        let (v, line, k) = buses
            .iter()
            .find_map(|v| {
                let line = line_of(v.line)?;
                let (k, ahead) = bus::next_stop(&net, line, [v.pos[0], v.pos[2]])?;
                // Coming: far enough to be seen arriving.
                if coming && !(60.0..400.0).contains(&ahead) {
                    return None;
                }
                Some((v, line, k))
            })
            .ok_or("no bus coming to a stop")?;
        // The bus will stand with its middle 6 m behind the stop's front.
        let (p, h) = bus::place(&net, line, line.stops[k].at - 6.0);
        // In its lane at the kerb, not on the centreline.
        let side = if net.keep_left { -2.5 } else { 2.5 };
        let p = [p[0] - h[1] * side, p[1] + h[0] * side];
        (
            p,
            h,
            format!("stop {k} of \"{}\", bus {} coming", line.sign(), v.id),
        )
    };
    // The first place with a clear view of the bus (no wall or building
    // between), the farther ones first.
    let aheads: &[f64] = if dwelling.is_some() {
        &[14.0, 11.0, -12.0, 8.0]
    } else {
        &[22.0, 17.0, -18.0, 12.0]
    };
    let mut cache = ocw_sim::collide::WorldCache::new(net.obstacles().cloned());
    let [x, z, _] = aheads
        .iter()
        .map(|&a| bus_camera(at, heading, net.keep_left, a))
        .find(|c| clear_view(&mut cache, [c[0], c[1]], at))
        .unwrap_or_else(|| bus_camera(at, heading, net.keep_left, aheads[0]));
    let ground = |x: f64, z: f64| {
        world
            .get_resource::<ocw_render::Ground>()
            .and_then(|g| (g.0)(x, z))
    };
    let base = ground(at[0], at[1]).unwrap_or(0.0);
    let eye = [x, ground(x, z).unwrap_or(base).max(base) + 2.6, z];
    // At the bus's middle, a little above its floor.
    let to = [at[0] - eye[0], base + 1.2 - eye[1], at[1] - eye[2]];
    let n = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2])
        .sqrt()
        .max(1e-6);
    let mut view = world.resource_mut::<ocw_render::RideView>();
    view.guided_camera = Some(ocw_render::GuidedCamera {
        eye,
        look: [to[0] / n, to[1] / n, to[2] / n],
        velocity: [0.0, 0.0],
    });
    Ok(format!("camera by {what} (x {x:.0} z {z:.0})"))
}

pub fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("traffic.bus")
            .about("Road traffic (spec 18B): the buses on their lines near the rider (line, destination, doors); 'traffic.bus watch' puts the camera beside the nearest bus at its stop with the doors open, else at the next stop of the nearest bus coming to one, 'traffic.bus coming' at the stop of a bus 60 – 400 m before it, 'traffic.bus off' back to the chase camera")
            .arg(Arg::new("watch").value_parser(["watch", "coming", "off"])),
        |world, m| bus_command(world, m.get_one::<String>("watch").map(String::as_str)),
    ));
    app.init_resource::<TrafficSettings>()
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("traffic")
                .about("Traffic (spec 14 Phase 18): without arguments what the ride has; road LEVEL (off, light, normal, busy, lunatic; the host decides in shared rides, desktop only), manners (polite, mostly_polite, some_realism), air (off, simulated, live), ufo (off, rare, often), sound, enabled, live_road (on / off)")
                .arg(Arg::new("what").value_parser(WHAT))
                .arg(Arg::new("value")),
            |world, m| {
                let what = m.get_one::<String>("what").cloned();
                let value = m.get_one::<String>("value").cloned();
                run(world, what.as_deref(), value.as_deref())
            },
        ));
}

/// Whether the road vehicles are heard (spec 18E T7): the master switch
/// and `traffic.sound`.
fn vehicle_sound(
    settings: Option<Res<TrafficSettings>>,
    view: Option<ResMut<ocw_render::RideView>>,
) {
    let Some(mut view) = view else { return };
    let s = settings.map(|s| s.view).unwrap_or_default();
    let on = s.enabled && s.sound;
    if view.vehicle_sound != on {
        view.vehicle_sound = on;
    }
}

/// The `traffic` command and the settings' resource.
pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        register_commands(app);
        crate::configure_sets(app);
        // Road vehicles heard or not (T7: `traffic.enabled`, `traffic.sound`).
        app.add_systems(Update, vehicle_sound.in_set(crate::ViewExtrasSet));
        // The simulated aircraft (T8).
        if !app.is_plugin_added::<crate::air::AirPlugin>() {
            app.add_plugins(crate::air::AirPlugin);
        }
        // The UFOs (T9).
        if !app.is_plugin_added::<crate::ufo::UfoPlugin>() {
            app.add_plugins(crate::ufo::UfoPlugin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_camera_stands_on_the_kerb_side_looking_back() {
        // A bus northwards at the origin: the camera 7 m east (its right,
        // the doors) and 9 m ahead, looking south-west at it.
        let [x, z, yaw] = bus_camera([0.0, 0.0], [0.0, -1.0], false, 9.0);
        assert!((x - 7.0).abs() < 1e-9 && (z + 9.0).abs() < 1e-9);
        assert!((yaw - 217.87).abs() < 0.05, "{yaw}");
        // Where traffic keeps left the doors are on the left.
        let [x, _, _] = bus_camera([0.0, 0.0], [0.0, -1.0], true, 9.0);
        assert!((x + 7.0).abs() < 1e-9);
    }

    fn app(platform: Platform) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ocw_render::console::ConsoleCorePlugin)
            .insert_resource(UiPlatform(platform))
            .add_plugins(TrafficPlugin);
        app
    }

    fn world(app: &mut App) -> &mut World {
        app.world_mut()
    }

    #[test]
    fn the_first_start_asks_nothing_of_the_server() {
        let s = TrafficSettings::default();
        assert_eq!(s.road_asked(), RoadRules::default());
        assert!(s.session_message().is_none(), "Q1: no road traffic");
        assert_eq!(s.view.air_shown(), AirTraffic::Simulated, "Q1: aircraft on");
        let busy = TrafficSettings {
            road: RoadRules {
                level: RoadTraffic::Busy,
                manners: DriverManners::Polite,
            },
            ..Default::default()
        };
        assert!(busy.session_message().is_some());
        let off = TrafficSettings {
            view: ViewChoice {
                enabled: false,
                ..Default::default()
            },
            ..busy
        };
        assert_eq!(
            off.road_asked().level,
            RoadTraffic::Off,
            "the master switch"
        );
        assert!(off.session_message().is_none());
    }

    #[test]
    fn the_command_sets_and_shows_without_a_ride() {
        let mut a = app(Platform::Desktop);
        let w = world(&mut a);
        assert_eq!(
            run(w, Some("road"), Some("busy")),
            Ok("applies to the next ride".into())
        );
        assert_eq!(
            run(w, Some("manners"), Some("mostly_polite")),
            Ok("applies to the next ride".into())
        );
        assert_eq!(run(w, Some("ufo"), Some("often")), Ok("UFOs: often".into()));
        assert!(
            run(w, Some("air"), Some("live"))
                .unwrap()
                .contains("simulated"),
            "live falls back to simulated"
        );
        assert!(run(w, Some("road"), Some("jam")).is_err());
        let s = *w.resource::<TrafficSettings>();
        assert_eq!(s.road.level, RoadTraffic::Busy);
        assert_eq!(s.road.manners, DriverManners::MostlyPolite);
        let shown = run(w, None, None).unwrap();
        assert!(
            shown.contains("road traffic: busy, drivers mostly polite (for the next ride)"),
            "{shown}"
        );
        assert!(shown.contains("UFOs: often"), "{shown}");
        assert!(shown.contains(LIVE_ROAD_NONE), "{shown}");
    }

    /// Q9: no road traffic in the browser; the sky works.
    #[test]
    fn the_browser_has_no_road_traffic() {
        let mut a = app(Platform::Browser);
        let w = world(&mut a);
        assert!(run(w, Some("road"), Some("busy")).is_err());
        assert_eq!(run(w, Some("ufo"), Some("rare")), Ok("UFOs: rare".into()));
        assert_eq!(
            run(w, Some("enabled"), Some("off")),
            Ok("traffic off".into())
        );
        let shown = run(w, None, None).unwrap();
        assert!(shown.contains("in the desktop app only"), "{shown}");
        assert!(!shown.contains("live traffic data"), "{shown}");
    }
}
