//! The weather on the desktop client (spec §3.2.1 World → Weather, §14
//! Phase 9, S4 / S5): the session weather from the server (owned by the
//! `GameServer` like the world clock, eased by
//! [`ocw_client::weather::WeatherClient`]) into the renderer's view
//! ([`ocw_render::RideView::weather`]) and the menu ([`WeatherState`]), the
//! **weather machine** – World → Weather: mode (fixed / simulated; real
//! weather is S7), the 17 presets as a grid with icons, the custom editor
//! (sky; wind and temperature), the transition time, a random weather from
//! a seed – and the console commands `weather`, `weather.list`,
//! `weather.status`, `weather.transition`, `weather.seed`.
//!
//! Every menu function is a console line (spec §3.2.1). The server
//! decides who may change the weather (Q2: solo the rider, in a shared ride
//! the host, nobody in an organizer's session or while an event's
//! organizer set it); the menu shows it and dims what the rider may not
//! change.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::{GridCell, MenuGrid};
use ocw_weather::machine::{
    self, DEFAULT_TRANSITION_S, Field, transition_label, transition_step, transition_suffix,
};
use ocw_weather::presets::{PRESETS, preset, title};

use crate::connection::{ClientState, Link};
use crate::settings::pages::{Activate, Page, Row};

/// Columns of the preset grid (17 presets and Back in four rows).
pub const GRID_COLUMNS: usize = 5;

/// The text of World → Weather.
pub const MACHINE_NOTE: &str = "A preset, your own weather or the simulation.";

/// The text of the custom editor's pages.
pub const CUSTOM_NOTE: &str = "Each change takes the transition time.";

/// The weather machine's own settings (not the server's): the transition
/// of the menu's changes and the seed of the random weather.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Machine {
    pub transition_s: f64,
    pub seed: u64,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            transition_s: DEFAULT_TRANSITION_S,
            seed: 1,
        }
    }
}

/// The weather at the rider's place, for the menu.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct WeatherState {
    /// The weather now (`None` before the server's arrived).
    pub now: Option<ocw_weather::WeatherState>,
    pub ground: ocw_weather::Ground,
    pub mode: Option<ocw_weather::Mode>,
    /// A transition's target and the seconds left.
    pub target: Option<(ocw_weather::WeatherState, f64)>,
    /// Who sets it: "you", "the host", "the organizer", "the event's
    /// organizer".
    pub control: String,
    /// Whether this rider may change it (the server checks again).
    pub may_change: bool,
    /// The organiser's weather of the running event (locked until it is
    /// over), for the page.
    pub event: Option<String>,
    /// The simulation's seed (while simulated).
    pub sim_seed: u64,
    pub machine: Machine,
}

impl WeatherState {
    /// "rain 11 °C" (the row on the World page).
    pub fn summary(&self) -> String {
        self.now.map_or_else(|| "clear".into(), |w| w.hud())
    }

    /// What the editor and the presets work from: a transition's target,
    /// else the weather now.
    pub fn base(&self) -> ocw_weather::WeatherState {
        self.target.map(|t| t.0).or(self.now).unwrap_or_default()
    }

    /// The preset the weather is (or is heading to), if it is one.
    pub fn current_preset(&self) -> Option<&'static str> {
        let base = self.base();
        PRESETS
            .iter()
            .map(|p| p.0)
            .find(|n| preset(n) == Some(base))
    }

    fn suffix(&self) -> String {
        transition_suffix(self.machine.transition_s)
    }
}

/// A row that opens a page.
fn open(label: &str, value: String, page: Page) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: Some(Activate::Open(page)),
    }
}

/// A row the rider may not use (dimmed).
fn disabled(label: &str, value: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: Some(Activate::Disabled),
    }
}

fn wet_label(g: &ocw_weather::Ground) -> String {
    match (g.wetness(), g.puddles()) {
        (w, _) if w < 0.05 => "dry".into(),
        (_, p) if p > 0.2 => "wet, puddles".into(),
        (w, _) if w < 0.6 => "damp".into(),
        _ => "wet".into(),
    }
}

/// The mode row's value: "fixed", "simulated (seed 7)", "-> rain in 40 s".
fn mode_value(w: &WeatherState) -> String {
    let simulated = w.mode == Some(ocw_weather::Mode::Simulated);
    match (w.target, simulated) {
        (Some((t, left)), sim) => format!(
            "-> {} in {left:.0} s{}",
            t.condition().label(),
            if sim { " (sim.)" } else { "" }
        ),
        (None, true) => format!("simulated (seed {})", w.sim_seed),
        (None, false) => "fixed".into(),
    }
}

/// The rows of the weather machine's pages (`page.is_weather_page()`;
/// six rows and Back at most, ten-foot rule; the presets are a grid).
pub fn rows(page: Page, w: &WeatherState) -> Vec<Row> {
    let may = w.may_change && w.now.is_some();
    let base = w.base();
    let suffix = w.suffix();
    match page {
        Page::Weather => {
            let simulated = w.mode == Some(ocw_weather::Mode::Simulated);
            let toggle = if simulated {
                "weather fixed".to_owned()
            } else {
                format!("weather auto {}", w.machine.seed)
            };
            let mode = if may {
                Row {
                    label: "Mode".into(),
                    value: Some(mode_value(w)),
                    left: Some(toggle.clone()),
                    right: Some(toggle.clone()),
                    confirm: Some(Activate::Command(toggle)),
                }
            } else {
                disabled("Mode", mode_value(w))
            };
            let random_line = format!("weather random {}{suffix}", w.machine.seed);
            let seed = |d: i64| {
                format!(
                    "weather.seed {}",
                    (w.machine.seed as i64 + d).clamp(1, 99_999)
                )
            };
            let random = if may {
                Row {
                    label: "Random weather".into(),
                    value: Some(format!("seed {}", w.machine.seed)),
                    left: Some(seed(-1)),
                    right: Some(seed(1)),
                    confirm: Some(Activate::Command(random_line)),
                }
            } else {
                disabled("Random weather", format!("seed {}", w.machine.seed))
            };
            let t = w.machine.transition_s;
            vec![
                mode,
                // Real weather from the providers: Phase 9 S7.
                disabled("Real weather", "coming (Phase 9 S7)".into()),
                open(
                    "Presets",
                    w.current_preset().map_or_else(|| "custom".into(), title),
                    Page::WeatherPresets,
                ),
                open("Custom", base.hud(), Page::WeatherCustom),
                Row {
                    label: "Transition".into(),
                    value: Some(transition_label(t)),
                    left: Some(format!("weather.transition {}", transition_step(t, -1))),
                    right: Some(format!("weather.transition {}", transition_step(t, 1))),
                    confirm: Some(Activate::Command(format!(
                        "weather.transition {}",
                        transition_step(t, 1)
                    ))),
                },
                random,
            ]
        }
        Page::WeatherPresets => PRESETS
            .iter()
            .map(|(name, _)| Row {
                label: title(name),
                value: None,
                left: None,
                right: None,
                confirm: Some(if may {
                    Activate::Command(format!("weather {name}{suffix}"))
                } else {
                    Activate::Disabled
                }),
            })
            .collect(),
        Page::WeatherCustom | Page::WeatherAir => {
            let fields: &[Field] = if page == Page::WeatherCustom {
                &Field::SKY
            } else {
                &Field::AIR
            };
            let mut out: Vec<Row> = fields
                .iter()
                .map(|f| {
                    if !may {
                        return disabled(f.label(), f.value(&base));
                    }
                    let line =
                        |dir| format!("weather {}{suffix}", f.assignment(&f.step(&base, dir)));
                    let (left, right) = (line(-1), line(1));
                    // A choice moves on with Enter; numbers only by ← / →.
                    let cycles = matches!(f, Field::CloudType | Field::Precip | Field::Direction);
                    Row {
                        label: f.label().into(),
                        value: Some(f.value(&base)),
                        confirm: cycles.then(|| Activate::Command(right.clone())),
                        left: Some(left),
                        right: Some(right),
                    }
                })
                .collect();
            if page == Page::WeatherCustom {
                let air = format!(
                    "{:.0} m/s {}, {:.0} °C",
                    base.wind_mps,
                    ocw_weather::state::compass(base.wind_from_deg),
                    base.temperature_c
                );
                out.push(open("Wind, temperature", air, Page::WeatherAir));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The status line under the title of a weather page (`focus`: the
/// focused item of the preset grid).
pub fn status(page: Page, w: &WeatherState, focus: usize) -> String {
    let Some(now) = w.now else {
        return "No weather from the server yet".into();
    };
    let by = match &w.event {
        Some(_) => "set by the event's organizer".to_owned(),
        None => format!("set by {}", w.control),
    };
    let t = transition_label(w.machine.transition_s);
    match page {
        Page::WeatherPresets => match PRESETS.get(focus) {
            Some((name, about)) => {
                // "thunderstorm: cloudburst, …" names itself once.
                let about = about.strip_prefix(&format!("{name}: ")).unwrap_or(about);
                let then = if w.may_change {
                    format!("Enter: {t}")
                } else {
                    by
                };
                format!("{}: {about} - {then}", title(name))
            }
            None => format!("Transition: {t} - {by}"),
        },
        Page::WeatherCustom | Page::WeatherAir if w.may_change => {
            format!("Now: {} - each change: {t}", now.hud())
        }
        _ => format!(
            "Now: {}, wind {:.0} m/s {}, road {} - {by}",
            now.hud(),
            now.wind_mps,
            ocw_weather::state::compass(now.wind_from_deg),
            if w.ground.snow_cover() > 0.05 {
                format!("snowy ({:.0} %)", w.ground.snow_cover() * 100.0)
            } else {
                wet_label(&w.ground)
            }
        ),
    }
}

/// The preset grid: 17 presets with their icons and Back.
pub fn grid(w: &WeatherState) -> MenuGrid {
    let current = w.current_preset();
    MenuGrid {
        columns: GRID_COLUMNS,
        cells: PRESETS
            .iter()
            .map(|(name, _)| GridCell {
                label: title(name),
                icon: preset(name).map(|p| ocw_weather::icon::Icon::of(&p)),
                marked: current == Some(*name),
            })
            .chain([GridCell {
                label: "Back".into(),
                icon: None,
                marked: false,
            }])
            .collect(),
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
    let may_change = client.0.may_change_weather();
    let event = client
        .0
        .event_info()
        .filter(|e| e.phase != ocw_sim::EventPhase::Finished)
        .and_then(|e| e.weather.clone());
    let info = client.0.weather().info();
    let new = WeatherState {
        now: view.weather.as_ref().map(|w| w.state),
        ground: view
            .weather
            .as_ref()
            .map_or_else(Default::default, |w| w.ground),
        mode: info.map(|i| i.mode),
        // Whole seconds: the menu's rows change once a second, not every frame.
        target: info.and_then(|i| i.target.map(|(t, left)| (t, left.ceil()))),
        sim_seed: info.map_or(0, |i| i.sim_seed),
        control,
        may_change,
        event,
        machine: state.machine,
    };
    state.set_if_neq(new);
}

/// Runs `weather ARGS`: shows the weather or asks the server for a change.
fn run_weather(world: &mut World, args: &[&str]) -> Result<String, String> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let (said, bytes) = {
        let mut c = world.resource_mut::<ClientState>();
        ocw_client::weather::run(&mut c.0, args, now)?
    };
    if let Some(bytes) = bytes {
        world.resource::<Link>().send_bytes(bytes);
    }
    // The server answers (a command result in the console).
    Ok(said)
}

/// `weather.transition [T]`: the menu's transition time.
fn run_transition(world: &mut World, arg: Option<&str>) -> Result<String, String> {
    let mut st = world.resource_mut::<WeatherState>();
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
    let mut st = world.resource_mut::<WeatherState>();
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
            let machine = world.resource::<WeatherState>().machine;
            let out = {
                let mut c = world.resource_mut::<ClientState>();
                ocw_client::weather::status(&mut c.0, now)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, MAX_ITEMS, Menu, Page as MenuPage, row_label};

    fn state(name: &str, may: bool) -> WeatherState {
        let w = preset(name).unwrap();
        WeatherState {
            now: Some(w),
            ground: ocw_weather::Ground::settled(&w),
            mode: Some(ocw_weather::Mode::Fixed),
            target: None,
            control: if may { "you" } else { "the host" }.into(),
            may_change: may,
            event: None,
            sim_seed: 0,
            machine: Machine::default(),
        }
    }

    fn labels(p: Page, w: &WeatherState) -> Vec<String> {
        rows(p, w).iter().map(row_label).collect()
    }

    #[test]
    fn the_weather_machine_page() {
        let w = state("heavy-rain", true);
        let r = rows(Page::Weather, &w);
        assert!(r.len() < MAX_ITEMS);
        assert_eq!(
            labels(Page::Weather, &w),
            [
                "Mode:  < fixed >",
                "Real weather:  coming (Phase 9 S7)",
                "Presets:  Heavy rain",
                "Custom:  heavy rain 11 °C",
                "Transition:  < 1 min >",
                "Random weather:  < seed 1 >",
            ]
        );
        // Real weather is dimmed (S7); the mode switches to the simulation.
        assert_eq!(r[1].confirm, Some(Activate::Disabled));
        assert_eq!(r[0].right.as_deref(), Some("weather auto 1"));
        assert_eq!(r[4].right.as_deref(), Some("weather.transition 300"));
        assert_eq!(r[4].left.as_deref(), Some("weather.transition 0"));
        assert_eq!(
            r[5].confirm,
            Some(Activate::Command("weather random 1 in 60".into()))
        );
        assert_eq!(r[5].left.as_deref(), Some("weather.seed 1"), "1 at least");
        assert_eq!(r[5].right.as_deref(), Some("weather.seed 2"));
        assert_eq!(r[2].confirm, Some(Activate::Open(Page::WeatherPresets)));
        assert!(status(Page::Weather, &w, 0).ends_with("road wet, puddles - set by you"));
        // Simulated, in a transition.
        let mut sim = w.clone();
        sim.mode = Some(ocw_weather::Mode::Simulated);
        sim.sim_seed = 7;
        sim.target = Some((preset("thunderstorm").unwrap(), 40.0));
        let r = rows(Page::Weather, &sim);
        assert_eq!(
            r[0].value.as_deref(),
            Some("-> thunderstorm in 40 s (sim.)")
        );
        sim.target = None;
        assert_eq!(
            rows(Page::Weather, &sim)[0].value.as_deref(),
            Some("simulated (seed 7)")
        );
        sim.target = Some((preset("thunderstorm").unwrap(), 40.0));
        assert_eq!(r[0].right.as_deref(), Some("weather fixed"));
        assert_eq!(r[2].value.as_deref(), Some("Thunderstorm"), "heading to");
        // Not the rider's: everything that changes it is dimmed.
        let host = state("rain", false);
        let r = rows(Page::Weather, &host);
        assert_eq!(r[0].confirm, Some(Activate::Disabled));
        assert!(r[0].left.is_none() && r[5].left.is_none());
        assert_eq!(r[5].confirm, Some(Activate::Disabled));
        assert!(status(Page::Weather, &host, 0).ends_with("set by the host"));
        // Before the server's weather.
        assert_eq!(
            status(Page::Weather, &WeatherState::default(), 0),
            "No weather from the server yet"
        );
        for p in [
            Page::Weather,
            Page::WeatherPresets,
            Page::WeatherCustom,
            Page::WeatherAir,
        ] {
            for l in labels(p, &sim) {
                assert!(ocw_render::font::covers(&l), "{l}");
                assert!(
                    l.chars().count() <= crate::ride::pages::MAX_ROW_CHARS,
                    "{l}"
                );
            }
            assert!(ocw_render::font::covers(&status(p, &sim, 3)));
        }
    }

    #[test]
    fn presets_apply_with_the_transition() {
        let mut w = state("clear", true);
        let r = rows(Page::WeatherPresets, &w);
        assert_eq!(r.len(), 17);
        assert_eq!(r[10].label, "Thunderstorm");
        assert_eq!(
            r[10].confirm,
            Some(Activate::Command("weather thunderstorm in 60".into()))
        );
        w.machine.transition_s = 0.0;
        assert_eq!(
            rows(Page::WeatherPresets, &w)[4].confirm,
            Some(Activate::Command("weather fog".into()))
        );
        // The grid: icons, the current one marked, Back last.
        let g = grid(&w);
        assert_eq!((g.columns, g.cells.len()), (GRID_COLUMNS, 18));
        assert!(g.cells[0].marked && g.cells.iter().filter(|c| c.marked).count() == 1);
        assert!(g.cells[..17].iter().all(|c| c.icon.is_some()));
        assert_eq!(g.cells[17].label, "Back");
        assert!(status(Page::WeatherPresets, &w, 10).starts_with("Thunderstorm: cloudburst"));
        assert!(status(Page::WeatherPresets, &w, 10).ends_with("Enter: instant"));
        // Not the rider's: dimmed.
        let host = state("clear", false);
        assert!(
            rows(Page::WeatherPresets, &host)
                .iter()
                .all(|r| r.confirm == Some(Activate::Disabled))
        );
        assert!(status(Page::WeatherPresets, &host, 0).ends_with("set by the host"));
    }

    /// The custom editor: ← / → are `weather FIELD=VALUE in T` lines that
    /// parse and step within the ranges.
    #[test]
    fn the_custom_editor_sends_field_lines() {
        let w = state("cloudy", true);
        let sky = rows(Page::WeatherCustom, &w);
        assert_eq!(
            sky.iter().map(row_label).collect::<Vec<_>>(),
            [
                "Cloud cover:  < 65 % >",
                "Cloud type:  < cumulus >",
                "Precipitation:  < none >",
                "Intensity:  < - >",
                "Fog:  < none >",
                "Wind, temperature:  4 m/s WSW, 16 °C",
            ]
        );
        assert_eq!(sky[0].right.as_deref(), Some("weather clouds=0.7 in 60"));
        assert_eq!(sky[0].left.as_deref(), Some("weather clouds=0.6 in 60"));
        assert!(sky[0].confirm.is_none(), "numbers by the arrows");
        assert_eq!(
            sky[2].right.as_deref(),
            Some("weather drizzle=0.3 clouds=0.8 in 60")
        );
        assert_eq!(sky[5].confirm, Some(Activate::Open(Page::WeatherAir)));
        let air = rows(Page::WeatherAir, &w);
        assert_eq!(air.len(), 4);
        assert_eq!(air[0].right.as_deref(), Some("weather wind=5 gust=9 in 60"));
        assert_eq!(air[3].left.as_deref(), Some("weather temp=15 in 60"));
        // Every line parses.
        for r in sky.iter().chain(&air) {
            for l in [&r.left, &r.right].into_iter().flatten() {
                let args: Vec<&str> = l.split_whitespace().skip(1).collect();
                ocw_weather::command::parse(&args).unwrap_or_else(|e| panic!("{l}: {e}"));
            }
        }
        // At the end of a range the arrow stays there.
        let mut hot = state("clear", true);
        hot.now.as_mut().unwrap().temperature_c = 50.0;
        assert_eq!(
            rows(Page::WeatherAir, &hot)[3].right.as_deref(),
            Some("weather temp=50 in 60")
        );
        // Not the rider's: shown, dimmed.
        let host = state("cloudy", false);
        let r = rows(Page::WeatherAir, &host);
        assert!(
            r.iter()
                .all(|r| r.confirm == Some(Activate::Disabled) && r.left.is_none())
        );
    }

    /// The menu: World → Weather → Presets (a grid: the arrows move in two
    /// directions) and Custom, with the rows above.
    #[test]
    fn the_menu_reaches_every_page_of_the_weather_machine() {
        let w = state("clear", true);
        let rows_of = |p: Page| rows(p, &w);
        let mut m = Menu::default();
        m.apply_with(Action::Open, rows_of);
        // Top → World → Weather.
        m.apply_with(Action::Click(2), rows_of);
        let world_rows = |p: Page| {
            if p.is_weather_page() {
                rows(p, &w)
            } else {
                crate::ride::world::rows(p, &Default::default(), &Default::default(), &w)
            }
        };
        m.apply_with(Action::Click(4), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Weather)));
        // Presets: a grid of 17 and Back.
        m.apply_with(Action::Click(2), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherPresets)));
        assert_eq!(m.item_count(), 18);
        assert_eq!(m.grid_columns(), Some(GRID_COLUMNS));
        m.apply_with(Action::Right, world_rows);
        m.apply_with(Action::Down, world_rows);
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 11, "two rows down, one right: hail");
        assert_eq!(
            m.apply_with(Action::Confirm, world_rows),
            Effect::Command("weather hail in 60".into())
        );
        // Down from the last row wraps to the top, Up from the top to the
        // last row, in the same column.
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 16);
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 1);
        m.apply_with(Action::Up, world_rows);
        assert_eq!(m.focus(), 16);
        m.apply_with(Action::Left, world_rows);
        m.apply_with(Action::Left, world_rows);
        assert_eq!(m.focus(), 14);
        // Back is the last cell.
        m.apply_with(Action::Click(17), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Weather)));
        // Custom → Wind and temperature.
        m.apply_with(Action::Click(3), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherCustom)));
        assert_eq!(
            m.apply_with(Action::Right, world_rows),
            Effect::Command("weather clouds=0.1 in 60".into())
        );
        // The ‹ arrow of row 4 (fog), clicked or tapped.
        assert_eq!(
            m.apply_with(Action::Step(4, -1), world_rows),
            Effect::Command("weather fog=20000 in 60".into())
        );
        assert_eq!(m.focus(), 4);
        m.apply_with(Action::Click(5), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherAir)));
        assert_eq!(m.labels().len(), 5);
        m.apply_with(Action::Back, world_rows);
        m.apply_with(Action::Back, world_rows);
        // The transition and the seed.
        m.apply_with(Action::Hover(4), world_rows);
        assert_eq!(
            m.apply_with(Action::Right, world_rows),
            Effect::Command("weather.transition 300".into())
        );
        m.apply_with(Action::Hover(5), world_rows);
        assert_eq!(
            m.apply_with(Action::Confirm, world_rows),
            Effect::Command("weather random 1 in 60".into())
        );
        // A dimmed row does nothing.
        m.apply_with(Action::Hover(1), world_rows);
        assert_eq!(m.apply_with(Action::Confirm, world_rows), Effect::None);
        assert!(m.dimmed()[1]);
    }
}
