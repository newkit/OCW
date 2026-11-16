//! Ride → Host a ride (spec §3.2.1 *Host a ride*, §6.2): the wizard's
//! plan and its pages, Bevy-free and shared by both apps (the browser
//! shows the entry dimmed: hosting needs the desktop app).
//!
//! The steps: World (the worlds the World menu finds, and the test
//! routes) → Route (the package's routes or free ride) → Date and time
//! (real time, or a time machine preset and a day offset) → Weather
//! (simulated, real, or a preset of the weather machine) → Options (bots,
//! who may change time and weather, the port, sign-in) → Summary with
//! *Start hosting*. Each row is a console line (`host.plan …`), so the
//! menu, the console and scripts share one path; [`HostPlan::apply`]
//! runs those lines on the plan.

use crate::menu::pages::{Activate, Page, Row};

/// Bots on a hosted ride, at most.
pub const MAX_BOTS: u32 = 20;
/// The time machine's presets (`time preset NAME`, `ocw_sky::machine`).
pub const TIME_PRESETS: [&str; 10] = [
    "sunrise",
    "golden-hour",
    "noon",
    "sunset",
    "blue-hour",
    "midnight",
    "summer-solstice",
    "winter-solstice",
    "full-moon",
    "new-moon",
];

/// A world to host on (from the app's world list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWorld {
    /// For people.
    pub label: String,
    /// What `ride.switch --world` takes; `None`: the test routes.
    pub spec: Option<String>,
    /// The package's routes (or the test routes' names).
    pub routes: Vec<String>,
    /// The package has a road graph (free ride possible).
    pub free_ride: bool,
}

impl HostWorld {
    /// The built-in test routes.
    pub fn test_routes() -> Self {
        Self {
            label: "Test routes".into(),
            spec: None,
            routes: vec!["hills".into(), "loop".into()],
            free_ride: false,
        }
    }
}

/// The weather of a hosted ride.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HostWeather {
    /// The seeded weather simulation (the server's default).
    #[default]
    Simulated,
    /// Real weather at the place (`weather.real on`).
    Real,
    /// A preset of the weather machine.
    Preset(String),
}

/// What will be hosted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlan {
    /// Index into the world list (0: the first entry).
    pub world: usize,
    /// The route's name; `None`: a free ride.
    pub route: Option<String>,
    /// `None`: real time; else a time machine preset.
    pub time_preset: Option<String>,
    /// Days from today (the time machine's day steps).
    pub day_offset: i32,
    pub weather: HostWeather,
    pub bots: u32,
    /// Everyone may change the time and weather (else the host only).
    pub open_conditions: bool,
    pub port: u16,
    pub sign_in: bool,
}

impl Default for HostPlan {
    fn default() -> Self {
        Self {
            world: 0,
            route: None,
            time_preset: None,
            day_offset: 0,
            weather: HostWeather::Simulated,
            bots: 5,
            open_conditions: false,
            port: ocw_protocol::join::DEFAULT_PORT,
            sign_in: false,
        }
    }
}

fn step_in<T: PartialEq + Clone>(list: &[T], now: &T, dir: i32) -> T {
    let i = list.iter().position(|x| x == now).unwrap_or(0) as i32;
    list[(i + dir).rem_euclid(list.len() as i32) as usize].clone()
}

impl HostPlan {
    /// The world chosen (clamped to the list).
    pub fn world_of<'a>(&self, worlds: &'a [HostWorld]) -> Option<&'a HostWorld> {
        worlds.get(self.world.min(worlds.len().saturating_sub(1)))
    }

    /// The route (or free ride) for people.
    pub fn route_label(&self) -> String {
        self.route.clone().unwrap_or_else(|| "free ride".to_owned())
    }

    pub fn time_label(&self) -> String {
        let t = self
            .time_preset
            .clone()
            .unwrap_or_else(|| "real time".into());
        match self.day_offset {
            0 => t,
            d => format!("{t}, {d:+} days"),
        }
    }

    pub fn weather_label(&self) -> String {
        match &self.weather {
            HostWeather::Simulated => "simulated".into(),
            HostWeather::Real => "real weather".into(),
            HostWeather::Preset(p) => p.clone(),
        }
    }

    /// The console lines the host runs once riding on its server: the
    /// time machine and the weather machine.
    pub fn condition_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(p) = &self.time_preset {
            out.push(format!("time preset {p}"));
        }
        if self.day_offset != 0 {
            out.push(format!("time {:+}d", self.day_offset));
        }
        match &self.weather {
            HostWeather::Simulated => {}
            HostWeather::Real => out.push("weather.real on".into()),
            HostWeather::Preset(p) => out.push(format!("weather {p}")),
        }
        out
    }

    /// Applies one `host.plan` line's words (`world N`, `route NAME|free`,
    /// `time real|PRESET|next|prev`, `day +1|-1|0`, `weather …`, `bots N`,
    /// `open on|off`, `port N`, `sign-in on|off`).
    pub fn apply(&mut self, words: &[&str], worlds: &[HostWorld]) -> Result<String, String> {
        let usage = || {
            "host.plan world N | route NAME|free | time real|PRESET|next|prev | day +1|-1|0 | weather simulated|real|PRESET|next|prev | bots N|+1|-1 | open on|off | port N|+1|-1 | sign-in on|off".to_string()
        };
        let (key, rest) = words.split_first().ok_or_else(usage)?;
        let value = rest.join(" ");
        match *key {
            "world" => {
                let n: usize = value.parse().map_err(|_| usage())?;
                let w = worlds.get(n).ok_or_else(|| {
                    format!("no world {n} (0–{})", worlds.len().saturating_sub(1))
                })?;
                self.world = n;
                // The route of the new world: its first, else free ride.
                self.route = if w.free_ride && w.spec.is_some() {
                    None
                } else {
                    w.routes.first().cloned()
                };
                Ok(format!("host world: {}", w.label))
            }
            "route" => {
                let w = self.world_of(worlds).ok_or("no world to host")?;
                if value == "free" {
                    if !w.free_ride {
                        return Err(format!("{} has no road graph for a free ride", w.label));
                    }
                    self.route = None;
                } else {
                    let r = w
                        .routes
                        .iter()
                        .find(|r| r.eq_ignore_ascii_case(&value))
                        .ok_or_else(|| format!("{} has no route {value:?}", w.label))?;
                    self.route = Some(r.clone());
                }
                Ok(format!("host route: {}", self.route_label()))
            }
            "time" => {
                let mut all: Vec<Option<String>> = vec![None];
                all.extend(TIME_PRESETS.iter().map(|p| Some((*p).to_owned())));
                self.time_preset = match value.as_str() {
                    "real" => None,
                    "next" => step_in(&all, &self.time_preset, 1),
                    "prev" => step_in(&all, &self.time_preset, -1),
                    p if TIME_PRESETS.contains(&p) => Some(p.to_owned()),
                    _ => return Err(usage()),
                };
                Ok(format!("host time: {}", self.time_label()))
            }
            "day" => {
                self.day_offset = match value.as_str() {
                    "0" => 0,
                    v => {
                        (self.day_offset + v.parse::<i32>().map_err(|_| usage())?).clamp(-366, 366)
                    }
                };
                Ok(format!("host time: {}", self.time_label()))
            }
            "weather" => {
                let mut all = vec![HostWeather::Simulated, HostWeather::Real];
                all.extend(
                    ocw_weather::presets::PRESETS
                        .iter()
                        .map(|(n, _)| HostWeather::Preset((*n).to_owned())),
                );
                self.weather = match value.as_str() {
                    "simulated" => HostWeather::Simulated,
                    "real" => HostWeather::Real,
                    "next" => step_in(&all, &self.weather, 1),
                    "prev" => step_in(&all, &self.weather, -1),
                    p if ocw_weather::presets::preset(p).is_some() => HostWeather::Preset(p.into()),
                    _ => return Err(usage()),
                };
                Ok(format!("host weather: {}", self.weather_label()))
            }
            "bots" => {
                self.bots = match value.as_str() {
                    "+1" => (self.bots + 1).min(MAX_BOTS),
                    "-1" => self.bots.saturating_sub(1),
                    v => v.parse::<u32>().map_err(|_| usage())?.min(MAX_BOTS),
                };
                Ok(format!("host bots: {}", self.bots))
            }
            "open" => {
                self.open_conditions = parse_on(&value).ok_or_else(usage)?;
                Ok(format!("host: {}", self.access_label()))
            }
            "sign-in" => {
                self.sign_in = parse_on(&value).ok_or_else(usage)?;
                Ok(format!("host sign-in required: {}", on_off(self.sign_in)))
            }
            "port" => {
                self.port = match value.as_str() {
                    "+1" => self.port.saturating_add(1).max(1024),
                    "-1" => self.port.saturating_sub(1).max(1024),
                    v => v
                        .parse::<u16>()
                        .ok()
                        .filter(|p| *p >= 1024)
                        .ok_or("a port from 1024 to 65535")?,
                };
                Ok(format!("host port: {}", self.port))
            }
            _ => Err(usage()),
        }
    }

    pub fn access_label(&self) -> &'static str {
        if self.open_conditions {
            "everyone changes time and weather"
        } else {
            "the host changes time and weather"
        }
    }
}

fn parse_on(v: &str) -> Option<bool> {
    match v {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn value_row(label: &str, value: String, left: String, right: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: Some(left),
        right: Some(right),
        confirm: None,
    }
}

fn next_row(label: &str, page: Page) -> Row {
    Row {
        label: label.into(),
        value: None,
        left: None,
        right: None,
        confirm: Some(Activate::Open(page)),
    }
}

fn info_row(label: &str, value: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: None,
    }
}

/// The rows of a wizard page; `sign_in`: an accounts server is
/// configured (the sign-in row is shown).
pub fn rows(page: Page, plan: &HostPlan, worlds: &[HostWorld], sign_in: bool) -> Vec<Row> {
    let world = plan.world_of(worlds);
    match page {
        Page::HostWorld => worlds
            .iter()
            .enumerate()
            .map(|(i, w)| Row {
                label: w.label.clone(),
                value: (i == plan.world).then(|| "chosen".to_owned()),
                left: None,
                right: None,
                confirm: Some(Activate::RunOpen(
                    format!("host.plan world {i}"),
                    Page::HostRoute,
                )),
            })
            .collect(),
        Page::HostRoute => {
            let Some(w) = world else {
                return Vec::new();
            };
            let mut out: Vec<Row> = Vec::new();
            if w.free_ride && w.spec.is_some() {
                out.push(Row {
                    label: "Free ride".into(),
                    value: plan.route.is_none().then(|| "chosen".to_owned()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::RunOpen(
                        "host.plan route free".into(),
                        Page::HostTime,
                    )),
                });
            }
            out.extend(w.routes.iter().map(|r| Row {
                label: r.clone(),
                value: (plan.route.as_deref() == Some(r)).then(|| "chosen".to_owned()),
                left: None,
                right: None,
                confirm: Some(Activate::RunOpen(
                    format!("host.plan route \"{r}\""),
                    Page::HostTime,
                )),
            }));
            out
        }
        Page::HostTime => vec![
            value_row(
                "Time",
                plan.time_preset
                    .clone()
                    .unwrap_or_else(|| "real time".into()),
                "host.plan time prev".into(),
                "host.plan time next".into(),
            ),
            value_row(
                "Day",
                match plan.day_offset {
                    0 => "today".into(),
                    d => format!("{d:+} days"),
                },
                "host.plan day -1".into(),
                "host.plan day +1".into(),
            ),
            Row::action("Real time, today", "host.plan time real"),
            next_row("Next: weather", Page::HostWeather),
        ],
        Page::HostWeather => vec![
            value_row(
                "Weather",
                plan.weather_label(),
                "host.plan weather prev".into(),
                "host.plan weather next".into(),
            ),
            Row::action("Simulated weather", "host.plan weather simulated"),
            next_row("Next: options", Page::HostOptions),
        ],
        Page::HostOptions => {
            let mut out = vec![
                value_row(
                    "Bots",
                    plan.bots.to_string(),
                    "host.plan bots -1".into(),
                    "host.plan bots +1".into(),
                ),
                Row {
                    label: "Time and weather".into(),
                    value: Some(
                        if plan.open_conditions {
                            "everyone"
                        } else {
                            "host only"
                        }
                        .into(),
                    ),
                    left: Some(format!("host.plan open {}", on_off(!plan.open_conditions))),
                    right: Some(format!("host.plan open {}", on_off(!plan.open_conditions))),
                    confirm: Some(Activate::Command(format!(
                        "host.plan open {}",
                        on_off(!plan.open_conditions)
                    ))),
                },
                value_row(
                    "UDP port",
                    plan.port.to_string(),
                    "host.plan port -1".into(),
                    "host.plan port +1".into(),
                ),
            ];
            if sign_in {
                let toggle = format!("host.plan sign-in {}", on_off(!plan.sign_in));
                out.push(Row {
                    label: "Sign-in required".into(),
                    value: Some(on_off(plan.sign_in).into()),
                    left: Some(toggle.clone()),
                    right: Some(toggle.clone()),
                    confirm: Some(Activate::Command(toggle)),
                });
            }
            out.push(next_row("Next: summary", Page::HostSummary));
            out
        }
        Page::HostSummary => vec![
            info_row(
                "World",
                world.map_or_else(|| "none".into(), |w| short(&w.label, 26)),
            ),
            info_row("Route", short(&plan.route_label(), 26)),
            info_row("Time", plan.time_label()),
            info_row("Weather", plan.weather_label()),
            info_row(
                "Bots, port",
                format!(
                    "{}, UDP {}{}",
                    plan.bots,
                    plan.port,
                    if plan.sign_in { ", sign-in" } else { "" }
                ),
            ),
            Row {
                label: "Start hosting".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::Run("host start".into())),
            },
        ],
        _ => Vec::new(),
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worlds() -> Vec<HostWorld> {
        vec![
            HostWorld::test_routes(),
            HostWorld {
                label: "Fictional Valley".into(),
                spec: Some("local/valley.ocwpkg".into()),
                routes: vec!["Up the Hill".into(), "Lake Loop".into()],
                free_ride: true,
            },
        ]
    }

    fn run(p: &mut HostPlan, line: &str) -> Result<String, String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        p.apply(&words, &worlds())
    }

    #[test]
    fn defaults_are_real_time_simulated_weather_five_bots_port_47800() {
        let p = HostPlan::default();
        assert_eq!((p.bots, p.port, p.open_conditions), (5, 47800, false));
        assert_eq!(p.time_label(), "real time");
        assert_eq!(p.weather_label(), "simulated");
        assert!(p.condition_lines().is_empty());
    }

    #[test]
    fn the_steps_go_back_and_forth_and_end_at_the_summary() {
        let mut p = HostPlan::default();
        let w = worlds();
        // World: each world opens the route page.
        let r = rows(Page::HostWorld, &p, &w, false);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[1].confirm,
            Some(Activate::RunOpen(
                "host.plan world 1".into(),
                Page::HostRoute
            ))
        );
        run(&mut p, "world 1").unwrap();
        assert_eq!(p.route, None, "a package with a graph: free ride first");
        let r = rows(Page::HostRoute, &p, &w, false);
        assert_eq!(
            r.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            ["Free ride", "Up the Hill", "Lake Loop"]
        );
        run(&mut p, "route lake loop").unwrap();
        assert_eq!(p.route.as_deref(), Some("Lake Loop"));
        assert!(
            run(&mut p, "route Nowhere")
                .unwrap_err()
                .contains("no route")
        );
        // Test routes: no free ride.
        run(&mut p, "world 0").unwrap();
        assert_eq!(p.route.as_deref(), Some("hills"));
        assert!(run(&mut p, "route free").is_err());
        // Time: real → presets → back; day steps.
        run(&mut p, "time next").unwrap();
        assert_eq!(p.time_preset.as_deref(), Some("sunrise"));
        run(&mut p, "time prev").unwrap();
        run(&mut p, "time prev").unwrap();
        assert_eq!(p.time_preset.as_deref(), Some("new-moon"));
        run(&mut p, "day +1").unwrap();
        run(&mut p, "day +1").unwrap();
        assert_eq!(p.time_label(), "new-moon, +2 days");
        // Weather: simulated → real → presets.
        run(&mut p, "weather next").unwrap();
        assert_eq!(p.weather, HostWeather::Real);
        run(&mut p, "weather fog").unwrap();
        assert_eq!(
            p.condition_lines(),
            ["time preset new-moon", "time +2d", "weather fog"]
        );
        // Options: bots capped, port from 1024.
        run(&mut p, "bots 50").unwrap();
        assert_eq!(p.bots, MAX_BOTS);
        run(&mut p, "bots -1").unwrap();
        assert_eq!(p.bots, 19);
        assert!(run(&mut p, "port 80").is_err());
        run(&mut p, "port +1").unwrap();
        assert_eq!(p.port, 47801);
        run(&mut p, "open on").unwrap();
        // The sign-in row only with an accounts server.
        assert!(
            rows(Page::HostOptions, &p, &w, false)
                .iter()
                .all(|r| r.label != "Sign-in required")
        );
        assert!(
            rows(Page::HostOptions, &p, &w, true)
                .iter()
                .any(|r| r.label == "Sign-in required")
        );
        // Summary: the choices, then Start hosting.
        let s = rows(Page::HostSummary, &p, &w, false);
        assert_eq!(s[0].value.as_deref(), Some("Test routes"));
        assert_eq!(s[1].value.as_deref(), Some("hills"));
        assert_eq!(s[3].value.as_deref(), Some("fog"));
        assert_eq!(s[4].value.as_deref(), Some("19, UDP 47801"));
        assert_eq!(
            s.last().unwrap().confirm,
            Some(Activate::Run("host start".into()))
        );
        assert!(
            run(&mut p, "nonsense")
                .unwrap_err()
                .starts_with("host.plan world N")
        );
    }

    #[test]
    fn every_wizard_page_fits_six_rows() {
        let p = HostPlan::default();
        for page in [
            Page::HostTime,
            Page::HostWeather,
            Page::HostOptions,
            Page::HostSummary,
        ] {
            assert!(rows(page, &p, &worlds(), true).len() <= 6, "{page:?}");
        }
    }
}
