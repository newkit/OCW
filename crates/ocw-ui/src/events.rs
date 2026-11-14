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
    // The organiser's damage rules (spec §14 Phase 12E, 12G).
    let damage_rules = |c: Command| {
        c.arg(
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
    // The organiser's damage rules, if any is given (spec §14 Phase 12E).
    let flag = |id: &str| {
        args.try_get_one::<bool>(id)
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false)
    };
    let damage_on = args
        .try_get_one::<String>("damage")
        .ok()
        .flatten()
        .map(|v| v == "on");
    let damage =
        (damage_on.is_some() || flag("dnf-injured") || flag("neutral-service")).then(|| {
            ocw_sim::damage::Rules {
                on: damage_on,
                dnf_badly_injured: flag("dnf-injured"),
                neutral_service: flag("neutral-service"),
            }
        });
    let start = |spec: ocw_sim::EventSpec, bots| {
        ClientCore::event_start_message_rules(
            &spec,
            bots,
            physics(),
            weather.as_ref(),
            damage,
            weather_physics,
        )
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
