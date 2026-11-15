//! Local events in both apps (spec §2.3; shared since Phase 10 B6): the
//! console command `event race | group | tt | cancel | status | dismiss`
//! (the Ride page's items run it). The server runs the event; this asks.

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::client::ClientCore;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// Registers `event`.
pub struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.console_command(ConsoleCommand::new(
            Permission::User,
            event_command(),
            run_event,
        ));
    }
}

/// The console command `event` (spec §2.3): start a local event, cancel
/// it, or show its state.
pub fn event_command() -> Command {
    let finish = |c: Command| {
        c.arg(
            Arg::new("laps")
                .long("laps")
                .value_parser(value_parser!(u32).range(1..=100))
                .help("Laps of a loop"),
        )
        .arg(
            Arg::new("km")
                .long("km")
                .value_parser(value_parser!(f64))
                .conflicts_with("laps")
                .help("Distance to the finish (km)"),
        )
    };
    let bots = Arg::new("bots")
        .long("bots")
        .value_parser(value_parser!(u32).range(0..=50))
        .help("Bots in the field (default: the current ones)");
    let physics = Arg::new("physics")
        .long("physics")
        .value_parser(["open", "profile", "equal"])
        .help("Race mode: open (parts count, capped), profile (only the frame's profile; default for races and group rides), equal (one bike for all)");
    // The organiser's weather (spec §14 Phase 9, S5): riders cannot change
    // it until the event is over.
    let weather = Arg::new("weather")
        .long("weather")
        .num_args(1..)
        .allow_hyphen_values(true)
        .value_name("WEATHER")
        .help("The event's weather: PRESET [FIELD=VALUE...], custom FIELD=VALUE..., random SEED or auto SEED (the simulation); fixed for the event");
    // The organiser's weather-physics rule (spec §14 Phase 9, S6, Q4).
    let weather_physics = Arg::new("weather-physics")
        .long("weather-physics")
        .value_parser(["on", "off"])
        .help("Whether the weather affects the physics in the event: wind, air density, wet roads (default off)");
    // The organiser's rules (spec §14 Phase 12E, 12G, K6); without an
    // option the Organiser rules page's settings (`event.*`) apply.
    let damage_rules = |c: Command| {
        c.arg(
            Arg::new("collisions")
                .long("collisions")
                .value_parser(["off", "soft", "full"])
                .help("Collisions in the event: off, soft (bumps only), full (default: group rides soft, races full)"),
        )
        .arg(
            Arg::new("aids")
                .long("aids")
                .value_parser(["on", "off"])
                .help("The riding aids (last-moment warning, proximity arrow and bell) allowed (default: yes, races too)"),
        )
        .arg(
            Arg::new("chaotic-bots")
                .long("chaotic-bots")
                .value_parser(["on", "off"])
                .help("Chaotic bots in the event (default: your ride.chaotic_bots)"),
        )
        .arg(
            Arg::new("time-limit")
                .long("time-limit")
                .value_parser(value_parser!(f64))
                .help("Time limit after the winner (minutes, 0.5 to 120; default: 2 min or a quarter of the winner's time)"),
        )
        .arg(
            Arg::new("service-s")
                .long("service-s")
                .value_parser(value_parser!(u32).range(0..=600))
                .help("The neutral service's wait for a spare wheel or bike (s; with --neutral-service)"),
        )
        .arg(
            Arg::new("damage")
                .long("damage")
                .value_parser(["on", "off"])
                .help("Damage in the event (default: your ride.damage)"),
        )
        .arg(
            Arg::new("dnf-injured")
                .long("dnf-injured")
                .action(clap::ArgAction::SetTrue)
                .help("A badly injured rider is DNF"),
        )
        .arg(
            Arg::new("neutral-service")
                .long("neutral-service")
                .action(clap::ArgAction::SetTrue)
                .help(
                    "Neutral service: a spare bike for a broken one, a 30 s stop restores the bike",
                ),
        )
    };
    Command::new("event")
        .about("Local events: a race, group ride or time trial against your ghost (spec 2.3)")
        .subcommand_required(true)
        .subcommand(
            damage_rules(finish(Command::new("race").about(
                "Race from the start formation to the finish (default: one lap of a loop, 5 km of a route that ends)",
            ))
            .arg(bots.clone())
            .arg(physics.clone())
            .arg(weather.clone())
            .arg(weather_physics.clone())),
        )
        .subcommand(
            damage_rules(Command::new("group")
                .about("Group ride: the bots hold a pace and regroup; no ranking")
                .arg(
                    Arg::new("watts")
                        .long("watts")
                        .value_parser(value_parser!(f64))
                        .help("The group's pace (W; default 150)"),
                )
                .arg(bots)
                .arg(physics.clone())
                .arg(weather.clone())
                .arg(weather_physics.clone())),
        )
        .subcommand(damage_rules(
            finish(
                Command::new("tt").about("Time trial alone against your best effort (the ghost)"),
            )
            .arg(physics)
            .arg(weather)
            .arg(weather_physics),
        ))
        .subcommand(Command::new("cancel").about("End the current event"))
        .subcommand(Command::new("status").about("The current event and its standings"))
        .subcommand(Command::new("dismiss").about("Close the results panel"))
}

/// Starts, cancels or shows the event (console `event ...`).
pub fn run_event(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let (sub, args) = m
        .subcommand()
        .ok_or("event race | group | tt | cancel | status")?;
    let laps = || args.get_one::<u32>("laps").copied();
    let km = || args.get_one::<f64>("km").map(|k| k * 1000.0);
    let bots = || args.get_one::<u32>("bots").copied();
    let physics = || {
        args.try_get_one::<String>("physics")
            .ok()
            .flatten()
            .and_then(|s| ocw_sim::PhysicsMode::parse(s))
    };
    let weather = match args.try_get_many::<String>("weather").ok().flatten() {
        Some(v) => {
            let v: Vec<&str> = v.map(String::as_str).collect();
            Some(ocw_weather::machine::parse_event_weather(&v)?)
        }
        None => None,
    };
    let weather_physics = args
        .try_get_one::<String>("weather-physics")
        .ok()
        .flatten()
        .map(|s| s == "on");
    // The organiser's rules (spec §14 Phase 12E, K6): the options, else
    // the Organiser rules page's settings.
    let defaults = OrganiserDefaults::of(world);
    let mut rules = organiser_rules(args, &defaults)?;
    rules.physics = physics();
    rules.weather_physics = weather_physics;
    let start = |spec: ocw_sim::EventSpec, bots| {
        ClientCore::event_start_message_organiser(&spec, bots, weather.as_ref(), &rules)
    };
    let msg = match sub {
        "race" => start(
            ocw_sim::EventSpec::Race {
                laps: laps(),
                length_m: km(),
            },
            bots(),
        ),
        "tt" => start(
            ocw_sim::EventSpec::TimeTrial {
                laps: laps(),
                length_m: km(),
            },
            None,
        ),
        "group" => start(
            ocw_sim::EventSpec::GroupRide {
                target_w: args.get_one::<f64>("watts").copied(),
            },
            bots(),
        ),
        "cancel" => ClientCore::event_cancel_message(),
        "dismiss" => {
            let now = crate::now(world);
            let mut c = crate::client_mut(world).ok_or("not riding")?;
            return if c.dismiss_results(now) {
                Ok("results closed".into())
            } else {
                Err("no results panel open".into())
            };
        }
        _ => {
            let now = crate::now(world);
            let c = crate::client(world).ok_or("not riding")?;
            return Ok(event_status(c, now));
        }
    };
    // The server answers (printed when it arrives).
    if !crate::send(world, msg) {
        return Err("not riding".into());
    }
    Ok(String::new())
}

/// The Organiser rules page's settings (spec §14 Phase 12 K6): the
/// defaults of `event …` when an option is not given.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrganiserDefaults {
    /// `auto`, `off`, `soft`, `full`.
    pub collisions: String,
    /// `auto`, `on`, `off` (also the next two).
    pub damage: String,
    pub aids: String,
    pub chaotic_bots: String,
    pub neutral_service: bool,
    pub dnf_injured: bool,
}

impl OrganiserDefaults {
    /// From the settings (the registry's defaults without them).
    pub fn of(world: &World) -> Self {
        let text = |k: &str| match world.get_resource::<crate::settings::UserSettings>() {
            Some(us) => us.value(k).to_string(),
            None => ocw_client::settings::def(k)
                .map(|d| d.default.to_string())
                .unwrap_or_default(),
        };
        Self {
            collisions: text("event.collisions"),
            damage: text("event.damage"),
            aids: text("event.aids"),
            chaotic_bots: text("event.chaotic_bots"),
            neutral_service: crate::settings::setting_on(world, "event.neutral_service"),
            dnf_injured: crate::settings::setting_on(world, "event.dnf_injured"),
        }
    }
}

/// The organiser's rules of an `event …` line (spec §14 Phase 12 K6): its
/// options, else the defaults of the Organiser rules page; `auto` and
/// absent leave a rule to the event's kind or the session's switch.
pub fn organiser_rules(
    args: &clap::ArgMatches,
    defaults: &OrganiserDefaults,
) -> Result<ocw_sim::event::Rules, String> {
    let opt = |id: &str| args.try_get_one::<String>(id).ok().flatten().cloned();
    let on_off = |v: Option<String>, d: &str| match v.as_deref().unwrap_or(d) {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    let flag = |id: &str| {
        args.try_get_one::<bool>(id)
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false)
    };
    let collisions = match opt("collisions").as_deref().unwrap_or(&defaults.collisions) {
        "off" => Some(ocw_sim::collide::Mode::Off),
        "soft" => Some(ocw_sim::collide::Mode::Soft),
        "full" => Some(ocw_sim::collide::Mode::Full),
        _ => None,
    };
    let damage_on = on_off(opt("damage"), &defaults.damage);
    let dnf = flag("dnf-injured") || defaults.dnf_injured;
    let service = flag("neutral-service") || defaults.neutral_service;
    let service_s = args.try_get_one::<u32>("service-s").ok().flatten().copied();
    if service_s.is_some() && !service {
        return Err("--service-s needs the neutral service (--neutral-service)".into());
    }
    let time_limit_s = match args.try_get_one::<f64>("time-limit").ok().flatten() {
        Some(m) if !(0.5..=120.0).contains(m) => {
            return Err("--time-limit: 0.5 to 120 minutes".into());
        }
        Some(m) => Some(m * 60.0),
        None => None,
    };
    Ok(ocw_sim::event::Rules {
        collisions,
        damage: (damage_on.is_some() || dnf || service).then_some(ocw_sim::damage::Rules {
            on: damage_on,
            dnf_badly_injured: dnf,
            neutral_service: service,
            service_s,
        }),
        aids: on_off(opt("aids"), &defaults.aids),
        chaotic_bots: on_off(opt("chaotic-bots"), &defaults.chaotic_bots),
        time_limit_s,
        ..Default::default()
    })
}

/// `event status`: the event's title, the HUD lines and the standings.
pub fn event_status(client: &ClientCore, now_s: f64) -> String {
    let (Some(info), Some(view)) = (client.event_info(), client.event_view(now_s)) else {
        return "no event (event race | group | tt)".into();
    };
    let mut s = format!("{} ({:?})", view.title, info.phase);
    for l in &view.lines {
        s.push_str("\n  ");
        s.push_str(l);
    }
    for r in ocw_client::event::result_rows(&info.kind, &info.standings, client.rider_id()) {
        s.push_str(&format!(
            "\n{:>4} {:<16} {:>10} {}{}",
            r.position,
            r.name,
            r.time,
            r.gap,
            if r.is_local { "  <- you" } else { "" }
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(line: &str, d: &OrganiserDefaults) -> Result<ocw_sim::event::Rules, String> {
        let m = event_command()
            .try_get_matches_from(std::iter::once("event").chain(line.split_whitespace()))
            .map_err(|e| e.to_string())?;
        let (_, args) = m.subcommand().unwrap();
        organiser_rules(args, d)
    }

    fn auto() -> OrganiserDefaults {
        OrganiserDefaults {
            collisions: "auto".into(),
            damage: "auto".into(),
            aids: "auto".into(),
            chaotic_bots: "auto".into(),
            neutral_service: false,
            dnf_injured: false,
        }
    }

    /// K6 (spec §14 Phase 12): the console's organiser options, the page's
    /// settings as their defaults, and the checks.
    #[test]
    fn event_options_set_the_organisers_rules() {
        assert_eq!(rules("race", &auto()).unwrap(), Default::default());
        let r = rules(
            "race --collisions soft --aids on --chaotic-bots off --time-limit 5 --neutral-service --service-s 20 --dnf-injured --damage on",
            &auto(),
        )
        .unwrap();
        assert_eq!(r.collisions, Some(ocw_sim::collide::Mode::Soft));
        assert_eq!((r.aids, r.chaotic_bots), (Some(true), Some(false)));
        assert_eq!(r.time_limit_s, Some(300.0));
        let d = r.damage.unwrap();
        assert!(d.neutral_service && d.dnf_badly_injured && d.on == Some(true));
        assert_eq!(d.service_s, Some(20));
        // The page's settings are the defaults; an option wins.
        let page = OrganiserDefaults {
            collisions: "full".into(),
            aids: "on".into(),
            neutral_service: true,
            ..auto()
        };
        let r = rules("group --aids off", &page).unwrap();
        assert_eq!(r.collisions, Some(ocw_sim::collide::Mode::Full));
        assert_eq!(r.aids, Some(false));
        assert!(r.damage.unwrap().neutral_service);
        // Checks.
        assert!(rules("race --service-s 20", &auto()).is_err());
        assert!(rules("race --time-limit 500", &auto()).is_err());
        assert!(rules("race --collisions hard", &auto()).is_err());
        assert_eq!(rules("tt --aids on", &auto()).unwrap().aids, Some(true));
    }
}
