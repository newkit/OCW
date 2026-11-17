//! The weather machine's menu (spec §3.2.1 World → Weather, §14 Phase 9,
//! S5 / S7), Bevy-free and shared by the desktop and browser apps: what
//! the menu knows of the session weather ([`WeatherState`], filled every
//! frame from the client by `ocw_ui::weather`), the rows of World →
//! Weather (mode, real weather, the 17 presets as a grid with icons, the
//! custom editor, the transition, random weather), the status line and
//! the preset grid. Every row is a console line (`weather …`,
//! `weather.transition`, `weather.real`, …); the server decides who may
//! change the weather and the rows of a rider who may not are dimmed.

use ocw_render::menu::{GridCell, MenuGrid};
use ocw_weather::machine::{
    DEFAULT_TRANSITION_S, Field, transition_label, transition_step, transition_suffix,
};
use ocw_weather::presets::{PRESETS, preset, title};
use ocw_weather::real::Choice;
use ocw_weather::real::days::DAYS;
use ocw_weather::real::feed::RealInfo;

use super::pages::{Activate, Page, Row};

/// Columns of the preset grid (17 presets and Back in four rows).
pub const GRID_COLUMNS: usize = 5;

/// The text of World → Weather.
pub const MACHINE_NOTE: &str = "A preset, your own weather or the simulation.";

/// The text of the custom editor's pages.
pub const CUSTOM_NOTE: &str = "Each change takes the transition time.";

/// The text of the real weather's page.
pub const REAL_NOTE: &str = "The weather of your place in the world now, or of a day in the past.";

/// The weather machine's own settings (not the server's): the transition
/// of the menu's changes, the seed of the random weather, the real
/// weather's data source (setting `weather.provider`) and the day the
/// "Ride a day" row shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Machine {
    pub transition_s: f64,
    pub seed: u64,
    pub provider: Choice,
    pub day: usize,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            transition_s: DEFAULT_TRANSITION_S,
            seed: 1,
            provider: Choice::Automatic,
            // The first day of the past (index 0 is today).
            day: 1,
        }
    }
}

/// The weather at the rider's place, for the menu.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// Real weather (S7; `None`: off).
    pub real: Option<RealInfo>,
    /// Whether the weather affects the physics now (S6, the server's rule;
    /// `None` before its weather arrived).
    pub physics: Option<bool>,
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

/// The mode row's value: "fixed", "simulated (seed 7)", "-> rain in 40 s",
/// "real (Automatic)".
fn mode_value(w: &WeatherState) -> String {
    let simulated = w.mode == Some(ocw_weather::Mode::Simulated);
    if let Some(r) = &w.real {
        return format!("real ({})", r.choice.name());
    }
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
            let toggle = if simulated || w.real.is_some() {
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
                // Real weather from the providers (S7).
                open("Real weather", real_value(w), Page::WeatherReal),
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
        Page::WeatherReal => real_rows(w, may),
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
            if page == Page::WeatherAir {
                // Weather physics (S6): the wind, the air and the road in
                // the physics; the host's (the organiser's in an event).
                let on = w.physics.unwrap_or(false);
                let toggle = format!("weather.physics {}", if on { "off" } else { "on" });
                let value = if on { "on" } else { "off" }.to_owned();
                out.push(if may && w.event.is_none() {
                    Row {
                        label: "Weather physics".into(),
                        value: Some(value),
                        left: Some(toggle.clone()),
                        right: Some(toggle.clone()),
                        confirm: Some(Activate::Command(toggle)),
                    }
                } else {
                    disabled("Weather physics", value)
                });
            }
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

/// The Real weather row's value: "off", "on: MeteoSwiss, MET Norway",
/// "on: waiting for the data".
fn real_value(w: &WeatherState) -> String {
    match &w.real {
        Some(r) => format!("on: {}", r.source_label()),
        None => "off".into(),
    }
}

/// "Storm Lothar, 1999" / "Today".
fn day_label(i: usize) -> String {
    match DAYS.get(i) {
        Some(d) if d.date.is_empty() => d.title.into(),
        Some(d) => format!("{}, {}", d.title, &d.date[..4]),
        None => "-".into(),
    }
}

/// World → Weather → Real weather (S7): on / off (`weather.real`, saved),
/// the data source (`weather.provider`, saved; in a shared ride the
/// host's applies), ride a day (← / → the day, Enter rides it: the time
/// machine and real weather), the data's station and time.
fn real_rows(w: &WeatherState, may: bool) -> Vec<Row> {
    let on = w.real.is_some();
    let choice = w.real.as_ref().map_or(w.machine.provider, |r| r.choice);
    let toggle = format!("weather.real {}", if on { "off" } else { "on" });
    let n = DAYS.len() as i64;
    let day = |d: i64| format!("weather.day {}", (w.machine.day as i64 + d).rem_euclid(n));
    let mut rows = if may {
        vec![
            Row {
                label: "Real weather".into(),
                value: Some(if on { "on" } else { "off" }.into()),
                left: Some(toggle.clone()),
                right: Some(toggle.clone()),
                confirm: Some(Activate::Command(toggle)),
            },
            Row {
                label: "Data source".into(),
                value: Some(choice.name().into()),
                left: Some(format!("weather.provider {}", choice.step(-1).id())),
                right: Some(format!("weather.provider {}", choice.step(1).id())),
                confirm: Some(Activate::Command(format!(
                    "weather.provider {}",
                    choice.step(1).id()
                ))),
            },
            Row {
                label: "Ride a day".into(),
                value: Some(day_label(w.machine.day)),
                left: Some(day(-1)),
                right: Some(day(1)),
                confirm: DAYS
                    .get(w.machine.day)
                    .map(|d| Activate::Command(format!("weather day {}", d.id))),
            },
        ]
    } else {
        vec![
            disabled("Real weather", if on { "on" } else { "off" }.into()),
            disabled("Data source", choice.name().into()),
            disabled("Ride a day", day_label(w.machine.day)),
        ]
    };
    if let Some(r) = &w.real {
        let mut data = Vec::new();
        if let Some(s) = &r.station {
            data.push(s.clone());
        }
        if r.data_ms != 0 {
            data.push(if r.historic {
                ocw_weather::real::time::utc_label(r.data_ms)
            } else {
                format!("{} UTC", ocw_weather::real::time::hh_mm_utc(r.data_ms))
            });
        }
        if data.is_empty() {
            data.push(r.status.name().into());
        }
        let text = data.join(", ");
        let text: String = text.chars().take(36).collect();
        rows.push(Row {
            label: "Data".into(),
            value: Some(text),
            left: None,
            right: None,
            confirm: None,
        });
    }
    rows
}

/// The status line under the title of a weather page (`focus`: the
/// focused item of the preset grid).
pub fn status(page: Page, w: &WeatherState, focus: usize) -> String {
    let Some(now) = w.now else {
        return "No weather from the server yet".into();
    };
    if page == Page::WeatherReal {
        return real_status(w, focus);
    }
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
            "Now: {}, wind {:.0} m/s {}, road {} - {by}{}",
            now.hud(),
            now.wind_mps,
            ocw_weather::state::compass(now.wind_from_deg),
            if w.ground.snow_cover() > 0.05 {
                format!("snowy ({:.0} %)", w.ground.snow_cover() * 100.0)
            } else {
                wet_label(&w.ground)
            },
            w.real
                .as_ref()
                .filter(|r| !r.attribution.is_empty())
                .map(|r| format!(" - {}", r.attribution))
                .unwrap_or_default()
        ),
    }
}

/// The real weather page's status: the attribution (or why there is no
/// data), a note, who sets it; on the day row the day's story.
fn real_status(w: &WeatherState, focus: usize) -> String {
    let by = match &w.event {
        Some(_) => "set by the event's organizer".to_owned(),
        None => format!("set by {}", w.control),
    };
    if focus == 2
        && let Some(d) = DAYS.get(w.machine.day)
    {
        let when = if d.date.is_empty() {
            "now".to_owned()
        } else {
            format!("{} {}", d.date, d.time)
        };
        return format!("{} ({when}): {} - Enter rides it", d.title, d.about);
    }
    match &w.real {
        None => format!("Off. Data source: {} - {by}", w.machine.provider.name()),
        Some(r) => {
            let mut s = if r.attribution.is_empty() {
                format!("Real weather: {}", r.source_label())
            } else {
                r.attribution.clone()
            };
            if let Some(n) = &r.note {
                s += &format!(" - {n}");
            }
            s + &format!(" - {by}")
        }
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
                swatch: None,
                marked: current == Some(*name),
                thumb: None,
            })
            .chain([GridCell {
                label: "Back".into(),
                icon: None,
                swatch: None,
                marked: false,
                thumb: None,
            }])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{MAX_ITEMS, row_label};
    use super::*;

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
            real: None,
            physics: Some(true),
            machine: Machine::default(),
        }
    }

    fn real_info(providers: &[ocw_weather::real::Provider]) -> RealInfo {
        RealInfo {
            choice: Choice::Automatic,
            providers: providers.to_vec(),
            attribution: ocw_weather::real::attribution(providers, false),
            note: None,
            status: ocw_weather::real::feed::Status::Live,
            data_ms: 1_794_656_400_000,
            historic: false,
            station: Some("Vaduz (VAD)".into()),
            ..Default::default()
        }
    }

    /// World → Weather → Real weather (S7): on / off, the data source and
    /// ride a day as console lines; the attribution under the title.
    #[test]
    fn the_real_weather_page() {
        use ocw_weather::real::Provider;
        let mut w = state("clear", true);
        let r = rows(Page::Weather, &w);
        assert_eq!(row_label(&r[1]), "Real weather:  off");
        assert_eq!(r[1].confirm, Some(Activate::Open(Page::WeatherReal)));
        let real = rows(Page::WeatherReal, &w);
        assert_eq!(
            real.iter().map(row_label).collect::<Vec<_>>(),
            [
                "Real weather:  < off >",
                "Data source:  < Automatic >",
                "Ride a day:  < Storm Lothar, 1999 >",
            ]
        );
        assert_eq!(
            real[0].confirm,
            Some(Activate::Command("weather.real on".into()))
        );
        assert_eq!(
            real[1].right.as_deref(),
            Some("weather.provider meteoswiss")
        );
        assert_eq!(real[1].left.as_deref(), Some("weather.provider open-meteo"));
        assert_eq!(real[2].right.as_deref(), Some("weather.day 2"));
        assert_eq!(
            real[2].confirm,
            Some(Activate::Command("weather day lothar".into()))
        );
        assert!(status(Page::WeatherReal, &w, 0).starts_with("Off. Data source: Automatic"));
        assert!(status(Page::WeatherReal, &w, 2).starts_with("Storm Lothar (1999-12-26 12:00)"));
        // On: the providers on the machine's page, the data row, the
        // attribution under the title.
        w.real = Some(real_info(&[Provider::MeteoSwiss, Provider::MetNorway]));
        w.mode = Some(ocw_weather::Mode::Real);
        let r = rows(Page::Weather, &w);
        assert_eq!(row_label(&r[0]), "Mode:  < real (Automatic) >");
        assert_eq!(r[0].right.as_deref(), Some("weather fixed"));
        assert_eq!(
            row_label(&r[1]),
            "Real weather:  on: MeteoSwiss, MET Norway"
        );
        let real = rows(Page::WeatherReal, &w);
        assert_eq!(row_label(&real[0]), "Real weather:  < on >");
        assert_eq!(
            real[0].confirm,
            Some(Activate::Command("weather.real off".into()))
        );
        assert_eq!(row_label(&real[3]), "Data:  Vaduz (VAD), 11:40 UTC");
        assert!(real[3].confirm.is_none());
        let s = status(Page::WeatherReal, &w, 0);
        assert_eq!(s, "Weather: MeteoSwiss, MET Norway, CC BY 4.0 - set by you");
        assert!(
            status(Page::Weather, &w, 0).ends_with("Weather: MeteoSwiss, MET Norway, CC BY 4.0")
        );
        // A note and waiting for the data.
        let mut waiting = real_info(&[]);
        waiting.attribution.clear();
        waiting.station = None;
        waiting.data_ms = 0;
        waiting.status = ocw_weather::real::feed::Status::Waiting;
        waiting.note = Some("fetching the weather ...".into());
        w.real = Some(waiting);
        assert_eq!(
            row_label(&rows(Page::Weather, &w)[1]),
            "Real weather:  on: waiting for the data"
        );
        assert_eq!(
            status(Page::WeatherReal, &w, 0),
            "Real weather: waiting for the data - fetching the weather ... - set by you"
        );
        assert_eq!(row_label(&rows(Page::WeatherReal, &w)[3]), "Data:  waiting");
        // Not the rider's: dimmed.
        let mut host = state("clear", false);
        host.real = Some(real_info(&[Provider::MetNorway]));
        let r = rows(Page::WeatherReal, &host);
        assert!(r[..3].iter().all(|r| r.confirm == Some(Activate::Disabled)));
        assert!(status(Page::WeatherReal, &host, 0).ends_with("set by the host"));
        // Every label fits and every line parses.
        for d in 0..ocw_weather::real::days::DAYS.len() {
            let mut w = state("clear", true);
            w.machine.day = d;
            w.machine.provider = Choice::Only(Provider::OpenMeteo);
            for r in rows(Page::WeatherReal, &w) {
                let l = row_label(&r);
                assert!(
                    l.chars().count() <= super::super::pages::MAX_ROW_CHARS,
                    "{l}"
                );
                assert!(ocw_render::font::covers(&l), "{l}");
            }
            assert!(ocw_render::font::covers(&status(Page::WeatherReal, &w, 2)));
            if let Some(Activate::Command(c)) = &rows(Page::WeatherReal, &w)[2].confirm {
                let args: Vec<&str> = c.split_whitespace().skip(1).collect();
                ocw_weather::command::parse(&args).unwrap();
            }
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
                "Real weather:  off",
                "Presets:  Heavy rain",
                "Custom:  heavy rain 11 °C",
                "Transition:  < 1 min >",
                "Random weather:  < seed 1 >",
            ]
        );
        // Real weather opens its page (S7); the mode switches to the simulation.
        assert_eq!(r[1].confirm, Some(Activate::Open(Page::WeatherReal)));
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
                    l.chars().count() <= super::super::pages::MAX_ROW_CHARS,
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
        assert_eq!(air.len(), 5);
        // Weather physics (S6): the switch, on the page of the wind.
        assert_eq!(row_label(&air[4]), "Weather physics:  < on >");
        assert_eq!(
            air[4].confirm,
            Some(Activate::Command("weather.physics off".into()))
        );
        assert_eq!(air[0].right.as_deref(), Some("weather wind=5 gust=9 in 60"));
        assert_eq!(air[3].left.as_deref(), Some("weather temp=15 in 60"));
        // Every line parses.
        for r in sky.iter().chain(&air) {
            for l in [&r.left, &r.right]
                .into_iter()
                .flatten()
                .filter(|l| l.starts_with("weather "))
            {
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
}
