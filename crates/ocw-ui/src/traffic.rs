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
use ocw_client::menu::model::Platform;
use ocw_client::menu::traffic::{AIR_LIVE_SOON, LIVE_ROAD_NONE, TrafficMenuInfo};
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

/// What the Settings → Traffic page shows of the session.
pub fn menu_info(world: &World, platform: Platform) -> TrafficMenuInfo {
    match crate::client(world) {
        Some(c) if c.rider_id().is_some() => TrafficMenuInfo {
            platform,
            session: c.traffic().info(),
            may_change: c.may_change_traffic(),
            decided_by: c.traffic().decided_by(c.rider_id(), c.host_rider()),
        },
        _ => TrafficMenuInfo::alone(platform),
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
        if air == AirTraffic::Live {
            format!(" ({AIR_LIVE_SOON}: simulated)")
        } else {
            String::new()
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
                AirTraffic::Live => format!("aircraft: live ({AIR_LIVE_SOON}: simulated)"),
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

pub fn register_commands(app: &mut App) {
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

/// The `traffic` command and the settings' resource.
pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        register_commands(app);
        // The simulated aircraft (T8).
        if !app.is_plugin_added::<crate::air::AirPlugin>() {
            app.add_plugins(crate::air::AirPlugin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
