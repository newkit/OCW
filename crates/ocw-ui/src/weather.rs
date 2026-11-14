//! The weather in both apps (spec §3.2.1 World → Weather, §14 Phase 9,
//! S4 / S5 / S7; shared since Phase 10 B6): the session weather from the
//! server (owned by the `GameServer` like the world clock, eased by
//! `ocw_client::weather::WeatherClient`) into the renderer's view
//! ([`ocw_render::RideView::weather`]) and the menu ([`WeatherMenu`]; its
//! rows: `ocw_client::menu::weather`), and the console commands
//! `weather`, `weather.list`, `weather.status`, `weather.transition`,
//! `weather.seed`, `weather.real`, `weather.provider`, `weather.physics`,
//! `weather.day`, `weather.days`.
//!
//! Every menu function is a console line (spec §3.2.1). The server
//! decides who may change the weather (Q2: solo the rider, in a shared ride
//! the host, nobody in an organizer's session or while an event's
//! organizer set it); the menu shows it and dims what the rider may not
//! change.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::client::ClientCore;
pub use ocw_client::menu::weather::*;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_weather::machine::{self, transition_label};
use ocw_weather::real::Choice;
use ocw_weather::real::days::DAYS;

use crate::settings::save_setting;

/// The menu's weather state as a resource (refreshed every frame).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct WeatherMenu(pub WeatherState);

/// Whether a new ride starts in real weather (spec §14 Phase 9, 9A, S7):
/// `given` (the desktop's `--real-weather BOOL`), else the setting
/// `weather.real` (on by default) – except scripted runs (screenshots and
/// captures: reproducible).
pub fn real_at_start(world: &World, given: Option<bool>, scripted: bool) -> bool {
    given.unwrap_or(crate::settings::setting_on(world, "weather.real") && !scripted)
}

/// The local ride's weather-physics switch at the start (spec §14 Phase 9,
/// S6, Q4): the local servers start with it on; the setting
/// `weather.physics` off sends the switch.
pub fn physics_message(world: &World) -> Option<Vec<u8>> {
    let on = crate::settings::setting_on(world, "weather.physics");
    (!on).then(|| {
        ClientCore::weather_message(&ocw_protocol::WeatherChange {
            physics: Some(false),
            ..Default::default()
        })
    })
}

/// `weather.physics [on|off]`: whether the weather affects the physics in
/// this ride (asks the server: the host decides) and the next local rides
/// (setting `weather.physics`, saved).
fn run_physics(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let on = match arg {
        None => {
            return Ok(match crate::client(world) {
                Some(c) => ocw_client::weather::physics_line(c, now),
                None => "weather physics: not riding".into(),
            });
        }
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        Some(a) => return Err(format!("on or off, not {a}")),
    };
    save_setting(world, "weather.physics", if on { "on" } else { "off" });
    if !crate::riding(world) {
        return Ok(format!(
            "weather physics {} for the next ride",
            if on { "on" } else { "off" }
        ));
    }
    run_weather(world, &["physics", if on { "on" } else { "off" }])
}

/// The request for real weather from the machine's data source.
pub fn real_message(world: &World) -> Vec<u8> {
    let provider = world
        .get_resource::<WeatherMenu>()
        .map_or(Choice::Automatic, |w| w.machine.provider);
    ClientCore::weather_message(&ocw_protocol::WeatherChange {
        real: Some(provider),
        ..Default::default()
    })
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<WeatherMenu>()
            .add_systems(Update, update_weather.in_set(crate::ViewExtrasSet));
        register_commands(app);
    }
}

/// The session weather into the view and the menu, every frame.
fn update_weather(world: &mut World) {
    let now = crate::now(world);
    let (weather, state) = {
        let Some(mut client) = crate::client_mut(world) else {
            return;
        };
        let weather = ocw_client::weather::weather_view(&mut client, now);
        let state = weather_state(&client, weather.as_ref());
        (weather, state)
    };
    if let Some(mut view) = world.get_resource_mut::<RideView>() {
        view.weather = weather;
    }
    let mut menu = world.resource_mut::<WeatherMenu>();
    let new = WeatherState {
        machine: menu.machine,
        ..state
    };
    if menu.0 != new {
        menu.0 = new;
    }
}

/// The menu's weather from the client and the frame's view.
pub fn weather_state(
    client: &ClientCore,
    view: Option<&ocw_render::weather::WeatherView>,
) -> WeatherState {
    let control = client.weather_control().to_owned();
    let may_change = client.may_change_weather();
    let event = client
        .event_info()
        .filter(|e| e.phase != ocw_sim::EventPhase::Finished)
        .and_then(|e| e.weather.clone());
    let info = client.weather().info();
    WeatherState {
        now: view.map(|w| w.state),
        ground: view.map_or_else(Default::default, |w| w.ground),
        mode: info.map(|i| i.mode),
        real: info.and_then(|i| i.real.clone()),
        // Whole seconds: the menu's rows change once a second, not every frame.
        target: info.and_then(|i| i.target.map(|(t, left)| (t, left.ceil()))),
        sim_seed: info.map_or(0, |i| i.sim_seed),
        physics: info.map(|i| i.physics.on),
        control,
        may_change,
        event,
        machine: Machine::default(),
    }
}

/// Runs `weather ARGS`: shows the weather or asks the server for a change.
/// `weather day …` runs its lines (the time machine, then real weather);
/// `weather real` without a source takes the machine's (`weather.provider`).
fn run_weather(world: &mut World, args: &[&str]) -> Result<String, String> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    if let Ok(ocw_weather::command::WeatherCommand::Day { title, lines }) =
        ocw_weather::command::parse(args)
    {
        let c = crate::client(world).ok_or("not riding")?;
        if !c.may_change_weather() {
            let who = c.weather_control();
            return Err(format!("{who} sets the weather of this ride"));
        }
        let mut console = world.resource_mut::<ConsoleState>();
        for l in &lines {
            console.queue(l.clone());
        }
        return Ok(format!("ride a day: {title} ({})", lines.join(", ")));
    }
    let provider = world.resource::<WeatherMenu>().machine.provider;
    let source;
    let args: Vec<&str> = match args {
        ["real"] | ["real", "in", _] => {
            source = provider.id().to_owned();
            let mut a = vec!["real", source.as_str()];
            a.extend_from_slice(&args[1..]);
            a
        }
        a => a.to_vec(),
    };
    let (said, bytes) = {
        let mut c = crate::client_mut(world).ok_or("not riding")?;
        ocw_client::weather::run(&mut c, &args, now)?
    };
    if let Some(bytes) = bytes {
        crate::send(world, bytes);
    }
    // The server answers (a command result in the console).
    Ok(said)
}

/// `weather.transition [T]`: the menu's transition time.
fn run_transition(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let mut st = world.resource_mut::<WeatherMenu>();
    if let Some(a) = arg {
        let s = match a {
            "instant" | "off" | "now" => 0.0,
            a => ocw_weather::command::parse_duration(a)?,
        };
        st.machine.transition_s = s.clamp(0.0, 24.0 * 3600.0);
    }
    Ok(format!(
        "transition of the weather machine: {}",
        transition_label(st.machine.transition_s)
    ))
}

/// `weather.seed [N]`: the random weather's seed.
fn run_seed(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let mut st = world.resource_mut::<WeatherMenu>();
    if let Some(a) = arg {
        st.machine.seed = a
            .parse::<u64>()
            .ok()
            .filter(|s| (1..=99_999).contains(s))
            .ok_or_else(|| format!("a seed 1-99999, not {a}"))?;
    }
    let seed = st.machine.seed;
    Ok(format!(
        "random weather seed {seed}: {}",
        machine::random(seed).0
    ))
}

/// `weather.real on|off`: real weather in this ride (asks the server) and
/// for the next rides (setting `weather.real`, saved).
fn run_real(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let on = match arg {
        None => {
            let st = world.resource::<WeatherMenu>();
            return Ok(match &st.real {
                Some(r) => ocw_client::weather::real_line(r),
                None => format!(
                    "real weather off (data source {})",
                    st.machine.provider.name()
                ),
            });
        }
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        Some(a) => return Err(format!("on or off, not {a}")),
    };
    save_setting(world, "weather.real", if on { "on" } else { "off" });
    if !crate::riding(world) {
        return Ok(format!(
            "real weather {} for the next ride",
            if on { "on" } else { "off" }
        ));
    }
    let real_now = world.resource::<WeatherMenu>().real.is_some();
    match (on, real_now) {
        (true, _) => run_weather(world, &["real"]),
        (false, true) => run_weather(world, &["fixed"]),
        (false, false) => Ok("real weather off".into()),
    }
}

/// `weather.provider [SOURCE|next|prev]`: the real weather's data source
/// (setting `weather.provider`, saved); while the weather is real the
/// server switches to it.
fn run_provider(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let current = world.resource::<WeatherMenu>().machine.provider;
    let Some(a) = arg else {
        return Ok(format!(
            "data source of real weather: {} (automatic, meteoswiss, dwd, metno, open-meteo)",
            current.name()
        ));
    };
    let choice = match a {
        "next" => current.step(1),
        "prev" | "previous" => current.step(-1),
        a => Choice::parse(a).ok_or_else(|| {
            format!("one of automatic, meteoswiss, dwd, metno, open-meteo; not {a}")
        })?,
    };
    world.resource_mut::<WeatherMenu>().machine.provider = choice;
    save_setting(world, "weather.provider", choice.id());
    let real_now = world.resource::<WeatherMenu>().real.is_some();
    if real_now && crate::riding(world) {
        run_weather(world, &["real", choice.id()])
    } else {
        Ok(format!("data source of real weather: {}", choice.name()))
    }
}

/// `weather.day [N|ID]`: the day of the "Ride a day" row.
fn run_day(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let mut st = world.resource_mut::<WeatherMenu>();
    if let Some(a) = arg {
        st.machine.day = match a.parse::<usize>() {
            Ok(i) if i < DAYS.len() => i,
            _ => DAYS
                .iter()
                .position(|d| d.id == a)
                .ok_or_else(|| format!("a day 0-{} or its name (weather.days)", DAYS.len() - 1))?,
        };
    }
    let d = &DAYS[st.machine.day];
    Ok(format!("ride a day: {} ({})", d.title, d.about))
}

fn print_err(world: &mut World, cmd: &str, out: Result<String, String>) -> Result<String, String> {
    if let Err(e) = &out {
        world
            .resource_mut::<ConsoleState>()
            .print(format!("{cmd}: {e}"));
    }
    out
}

fn register_commands(app: &mut App) {
    let opt_arg = |name: &'static str, help: &'static str| {
        Arg::new(name).help(help).allow_hyphen_values(true)
    };
    app.console_command(ConsoleCommand::new(
        // A user command: the server decides who may change the weather
        // (spec §14 Phase 9, Q2; solo: you, a shared ride: the host).
        Permission::User,
        Command::new("weather")
            .about("The weather at your place in the world; presets, fields, transitions, random, the simulation")
            .long_about(ocw_weather::command::USAGE)
            .arg(
                Arg::new("args")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .action(ArgAction::Append),
            ),
        |world, m| {
            let args: Vec<&str> = m
                .get_many::<String>("args")
                .map(|v| v.map(String::as_str).collect())
                .unwrap_or_default();
            let out = run_weather(world, &args);
            print_err(world, "weather", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.list")
            .about("The weather presets, the custom fields and the transitions (spec 9 S5)"),
        |_, _| Ok(ocw_client::weather::list_long()),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.status").about(
            "Everything about the session's weather: now, where it is going, mode and seed, ground, who sets it",
        ),
        |world, _| {
            let now = world.resource::<Time>().elapsed_secs_f64();
            let machine = world.resource::<WeatherMenu>().machine;
            let out = match crate::client_mut(world) {
                Some(mut c) => ocw_client::weather::status(&mut c, now),
                None => Err("not riding".into()),
            }
            .map(|s| {
                format!(
                    "{s}\nweather machine: transition {}, random seed {}",
                    transition_label(machine.transition_s),
                    machine.seed
                )
            });
            print_err(world, "weather.status", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.transition")
            .about("The weather machine's transition: instant, 1m, 5m, 15m or any time")
            .arg(opt_arg("time", "instant, 60, 5m, 15m (none: show it)")),
        |world, m| {
            let a = m.get_one::<String>("time").map(String::as_str);
            let out = run_transition(world, a);
            print_err(world, "weather.transition", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.real")
            .about("Real weather from the providers in this ride and the next ones (saved): on or off")
            .arg(opt_arg("on", "on or off (none: show it)")),
        |world, m| {
            let a = m.get_one::<String>("on").map(String::as_str);
            let out = run_real(world, a);
            print_err(world, "weather.real", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.physics")
            .about("Whether the weather affects the physics (wind, air density, wet roads; the trainer gets the wind), saved; the host decides in shared rides, the organizer in events")
            .arg(opt_arg("on", "on or off (none: show it and what it does at the rider)")),
        |world, m| {
            let a = m.get_one::<String>("on").map(String::as_str);
            let out = run_physics(world, a);
            print_err(world, "weather.physics", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.provider")
            .about("The data source of real weather (saved): automatic, meteoswiss, dwd, metno, open-meteo")
            .arg(opt_arg("source", "a source, next or prev (none: show it)")),
        |world, m| {
            let a = m.get_one::<String>("source").map(String::as_str);
            let out = run_provider(world, a);
            print_err(world, "weather.provider", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.day")
            .about("The day of the weather machine's Ride a day row (weather day NAME rides it)")
            .arg(opt_arg("day", "its number or name (none: show it)")),
        |world, m| {
            let a = m.get_one::<String>("day").map(String::as_str);
            let out = run_day(world, a);
            print_err(world, "weather.day", out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.days")
            .about("The days to ride (weather day NAME): famous storms, heat and snow"),
        |_, _| Ok(ocw_weather::command::days_list()),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.seed")
            .about("The seed of the weather machine's random weather and simulation")
            .arg(opt_arg("seed", "1-99999 (none: show it)")),
        |world, m| {
            let a = m.get_one::<String>("seed").map(String::as_str);
            let out = run_seed(world, a);
            print_err(world, "weather.seed", out)
        },
    ));
}
