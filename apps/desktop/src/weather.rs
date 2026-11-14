//! The weather on the desktop client (spec §3.2.1 World → Weather, §14
//! Phase 9, S4): the session weather from the server (owned by the
//! `GameServer` like the world clock, eased by
//! [`ocw_client::weather::WeatherClient`]) into the renderer's view
//! ([`ocw_render::RideView::weather`]) and the menu ([`WeatherState`]), and
//! the console command `weather` (presets, fields, the simulation, a test
//! strike). The server decides who may change it (Q2: solo the rider, in a
//! shared ride the host, nobody in an organizer's event). The weather
//! machine's menu (presets, custom sliders, transitions) is S5, real
//! weather S7.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_weather::command::{self, WeatherCommand};

use crate::connection::{ClientState, Link};
use crate::settings::pages::Row;

/// The weather at the rider's place, for the menu.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct WeatherState {
    /// The weather now (`None` before the server's arrived).
    pub now: Option<ocw_weather::WeatherState>,
    pub ground: ocw_weather::Ground,
    pub mode: Option<ocw_weather::Mode>,
    /// A transition's target and the seconds left.
    pub target: Option<(ocw_weather::WeatherState, f64)>,
    /// Who sets it: "you", "the host", "the organizer".
    pub control: String,
}

impl WeatherState {
    /// "rain 11 °C" (the row on the World page).
    pub fn summary(&self) -> String {
        self.now.map_or_else(|| "clear".into(), |w| w.hud())
    }
}

/// The text of World → Weather.
pub const PENDING_NOTE: &str = "The session's weather. Change it with the console command weather; the weather machine's menu follows with Phase 9 S5.";

/// The rows of World → Weather (six rows and Back, ten-foot rule): the
/// weather now, the road, the wind, the mode, who sets it, and the weather
/// machine (disabled until S5).
pub fn rows(w: &WeatherState) -> Vec<Row> {
    let info = |label: &str, value: String| Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: None,
    };
    let Some(now) = w.now else {
        return vec![
            info("Now", "clear (no weather from the server)".into()),
            info("Weather machine", "disabled until Phase 9 S5".into()),
        ];
    };
    let road = if w.ground.snow_cover() > 0.05 {
        format!(
            "snow {:.0} %, road {}",
            w.ground.snow_cover() * 100.0,
            wet_label(&w.ground)
        )
    } else {
        wet_label(&w.ground)
    };
    let mode = match (w.mode, w.target) {
        (_, Some((t, left))) => format!("-> {} in {left:.0} s", t.condition().label()),
        (Some(m), None) => m.name().into(),
        (None, None) => "fixed".into(),
    };
    vec![
        info(
            "Now",
            format!("{} {:.0} °C", now.condition().label(), now.temperature_c),
        ),
        info(
            "Wind",
            format!(
                "{:.0} m/s from {}, gusts {:.0}",
                now.wind_mps,
                ocw_weather::state::compass(now.wind_from_deg),
                now.gust_mps
            ),
        ),
        info("Ground", road),
        info("Mode", mode),
        info("Who sets the weather", w.control.clone()),
        info("Weather machine", "disabled until Phase 9 S5".into()),
    ]
}

fn wet_label(g: &ocw_weather::Ground) -> String {
    match (g.wetness(), g.puddles()) {
        (w, _) if w < 0.05 => "dry".into(),
        (_, p) if p > 0.2 => "wet, puddles".into(),
        (w, _) if w < 0.6 => "damp".into(),
        _ => "wet".into(),
    }
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeatherState>().add_systems(
            Update,
            update_weather
                .after(crate::connection::update_view)
                .run_if(resource_exists::<ClientState>),
        );
        register_commands(app);
    }
}

/// The session weather into the view and the menu, every frame.
fn update_weather(
    time: Res<Time>,
    mut client: ResMut<ClientState>,
    mut view: ResMut<RideView>,
    mut state: ResMut<WeatherState>,
) {
    let now = time.elapsed_secs_f64();
    view.weather = ocw_client::weather::weather_view(&mut client.0, now);
    let control = client.0.weather_control().to_owned();
    let info = client.0.weather().info();
    let new = WeatherState {
        now: view.weather.as_ref().map(|w| w.state),
        ground: view
            .weather
            .as_ref()
            .map_or_else(Default::default, |w| w.ground),
        mode: info.map(|i| i.mode),
        target: info.and_then(|i| i.target),
        control,
    };
    state.set_if_neq(new);
}

/// Runs `weather ARGS`: shows the weather or asks the server for a change.
fn run_weather(world: &mut World, args: &[&str]) -> Result<String, String> {
    let cmd = command::parse(args)?;
    if cmd == WeatherCommand::List {
        return Ok(command::list());
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    let bytes = {
        let mut c = world.resource_mut::<ClientState>();
        if cmd == WeatherCommand::Show {
            return ocw_client::weather::describe(&mut c.0, now);
        }
        if c.0.weather().info().is_none() {
            return Err("no weather yet (the server sends it when you join)".into());
        }
        if !c.0.may_change_weather() {
            return Err(format!(
                "{} sets the weather of this ride",
                c.0.weather_control()
            ));
        }
        let base = ocw_client::weather::change_base(&mut c.0, now);
        let Some(change) = ocw_client::weather::request_for(&cmd, base) else {
            return Ok(String::new());
        };
        ocw_client::client::ClientCore::weather_message(&change)
    };
    world.resource::<Link>().send_bytes(bytes);
    // The server answers (a command result in the console).
    Ok(String::new())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        // A user command: the server decides who may change the weather
        // (spec §14 Phase 9, Q2; solo: you, a shared ride: the host).
        Permission::User,
        Command::new("weather")
            .about("The weather at your place in the world; presets, fields, transitions, the simulation")
            .long_about(command::USAGE)
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
            if let Err(e) = &out {
                world
                    .resource_mut::<ConsoleState>()
                    .print(format!("weather: {e}"));
                return Err(e.clone());
            }
            out
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::presets::preset;

    #[test]
    fn the_page_shows_the_servers_weather_and_the_machine_waits_for_s5() {
        let none = WeatherState::default();
        let r = rows(&none);
        assert_eq!(
            r[0].value.as_deref(),
            Some("clear (no weather from the server)")
        );
        let rain = preset("heavy-rain").unwrap();
        let w = WeatherState {
            now: Some(rain),
            ground: ocw_weather::Ground::settled(&rain),
            mode: Some(ocw_weather::Mode::Fixed),
            target: None,
            control: "you".into(),
        };
        let r = rows(&w);
        assert!(r.len() < crate::menu::model::MAX_ITEMS);
        assert_eq!(r[0].value.as_deref(), Some("heavy rain 11 °C"));
        assert_eq!(r[2].value.as_deref(), Some("wet, puddles"));
        assert_eq!(r[3].value.as_deref(), Some("fixed"));
        assert_eq!(r[4].value.as_deref(), Some("you"));
        assert_eq!(r[5].label, "Weather machine");
        // Nothing to select or change until S5.
        assert!(
            r.iter()
                .all(|r| r.confirm.is_none() && r.left.is_none() && r.right.is_none())
        );
        let snowing = WeatherState {
            now: Some(preset("heavy-snow").unwrap()),
            ground: ocw_weather::Ground::settled(&preset("heavy-snow").unwrap()),
            target: Some((preset("clear").unwrap(), 30.0)),
            ..w
        };
        let r = rows(&snowing);
        assert!(r[2].value.as_deref().unwrap().starts_with("snow 100 %"));
        assert_eq!(r[3].value.as_deref(), Some("-> clear in 30 s"));
        assert!(ocw_render::font::covers(&r[0].value.clone().unwrap()));
        assert!(PENDING_NOTE.contains("S5"));
    }

    #[test]
    fn console_lines_become_requests() {
        use ocw_client::weather::request_for;
        let base = preset("clear").unwrap();
        let c = request_for(
            &command::parse(&["thunderstorm", "in", "90"]).unwrap(),
            base,
        )
        .unwrap();
        assert_eq!(c.target, preset("thunderstorm"));
        assert_eq!((c.transition_s, c.settle_ground), (90.0, true));
        let c = request_for(&command::parse(&["rain=3", "wet=1"]).unwrap(), base).unwrap();
        let t = c.target.unwrap();
        assert_eq!((t.precip, t.precip_mm_h), (ocw_weather::Precip::Rain, 3.0));
        assert_eq!(
            t.temperature_c, base.temperature_c,
            "the rest from the base"
        );
        assert!(!c.settle_ground);
        assert_eq!(c.ground.wetness, Some(1.0));
        let c = request_for(&command::parse(&["wet=0.5"]).unwrap(), base).unwrap();
        assert!(c.target.is_none(), "only the ground");
        assert_eq!(
            request_for(&command::parse(&["auto"]).unwrap(), base)
                .unwrap()
                .simulate,
            Some(0)
        );
        assert!(
            request_for(&command::parse(&["strike", "800"]).unwrap(), base)
                .unwrap()
                .strike
                .is_some()
        );
        assert!(request_for(&WeatherCommand::Show, base).is_none());
    }
}
