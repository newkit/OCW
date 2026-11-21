//! Ride → Host a ride (spec §3.2.1 *Host a ride*, §6.2): the wizard's
//! plan and its pages, Bevy-free and shared by both apps (the browser
//! shows the entry dimmed: hosting needs the desktop app).
//!
//! The steps: World (the worlds the World menu finds, and the test
//! routes) → Route (the package's routes or free ride) → Date and time
//! (real time, or a time machine preset and a day offset) → Weather
//! (simulated, real, or a preset of the weather machine, and who may
//! change time and weather) → Options (bots, voice, the port, the
//! router, sign-in) → Summary with
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
    /// Open the router's ports automatically (UPnP; setting `host.upnp`,
    /// spec §7.1.7 Q4: on by default for Host a ride).
    pub upnp: bool,
    /// Voice chat on the hosted ride: the bundled voice server starts
    /// with it (setting `host.voice`, spec §7.1.7 Q15: on by default);
    /// off, the ride has no voice, like `ocw-server --voice off`.
    pub voice: bool,
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
            upnp: true,
            voice: true,
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
    /// `open on|off`, `port N`, `sign-in on|off`, `upnp on|off`,
    /// `voice on|off`).
    pub fn apply(&mut self, words: &[&str], worlds: &[HostWorld]) -> Result<String, String> {
        let usage = || {
            "host.plan world N | route NAME|free | time real|PRESET|next|prev | day +1|-1|0 | weather simulated|real|PRESET|next|prev | bots N|+1|-1 | open on|off | port N|+1|-1 | sign-in on|off | upnp on|off | voice on|off".to_string()
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
            "upnp" => {
                self.upnp = parse_on(&value).ok_or_else(usage)?;
                Ok(format!("host: {}", self.upnp_label()))
            }
            "voice" => {
                self.voice = parse_on(&value).ok_or_else(usage)?;
                Ok(format!("host: {}", self.voice_label()))
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

    /// The router's ports, for people.
    pub fn upnp_label(&self) -> &'static str {
        if self.upnp {
            "the router's ports are opened automatically (UPnP)"
        } else {
            "the router's ports are opened by hand"
        }
    }

    /// Voice chat, for people.
    pub fn voice_label(&self) -> &'static str {
        if self.voice {
            "voice chat on (the voice server starts with the ride)"
        } else {
            "no voice chat on this ride"
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

/// An on / off row: Left, Right and Enter switch it (`LINE on|off`).
fn toggle_row(label: &str, on: bool, line: &str) -> Row {
    let toggle = format!("{line} {}", on_off(!on));
    Row {
        label: label.into(),
        value: Some(on_off(on).into()),
        left: Some(toggle.clone()),
        right: Some(toggle.clone()),
        confirm: Some(Activate::Command(toggle)),
    }
}

/// *Time and weather*: host only / everyone (`host.plan open on|off`).
fn access_row(plan: &HostPlan) -> Row {
    let toggle = format!("host.plan open {}", on_off(!plan.open_conditions));
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
        left: Some(toggle.clone()),
        right: Some(toggle.clone()),
        confirm: Some(Activate::Command(toggle)),
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
            // Who may change them (moved from the Options page for its
            // Voice row, spec §7.1.7 Q15: six rows).
            access_row(plan),
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
                toggle_row("Voice", plan.voice, "host.plan voice"),
                value_row(
                    "UDP port",
                    plan.port.to_string(),
                    "host.plan port -1".into(),
                    "host.plan port +1".into(),
                ),
                toggle_row("Open the router's ports", plan.upnp, "host.plan upnp"),
            ];
            if sign_in {
                out.push(toggle_row(
                    "Sign-in required",
                    plan.sign_in,
                    "host.plan sign-in",
                ));
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
                    "{}, UDP {}{}{}",
                    plan.bots,
                    plan.port,
                    if plan.sign_in { ", sign-in" } else { "" },
                    if plan.voice { "" } else { ", no voice" }
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

/// The voice server of a hosting, for its pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostVoice {
    /// Starting (the hosting waits for it).
    Starting,
    /// Running on TCP `signal`, TCP `tcp` and UDP `udp`.
    On { signal: u16, tcp: u16, udp: u16 },
    /// No voice server of its own: why (turned off in the plan, it did
    /// not start, no binary).
    Off(String),
}

/// The router's ports of a hosting (spec §7.1.7 Q4), for its pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostRouter {
    /// *Open the router's ports* is off.
    Off,
    Searching,
    /// Mapped: the router's public address (if it is one), how many of
    /// the ports, and what to know (some refused, not public).
    Opened {
        public: Option<String>,
        ports: usize,
        of: usize,
        note: Option<String>,
    },
    /// No UPnP router (or UPnP off on it): why.
    NotAvailable(String),
    /// The router refused: why.
    Failed(String),
}

/// What a hosting's pages show about its network: the ports others
/// reach, the voice server and the router (spec §7.1.1 *Network*,
/// §7.1.6 V2). Filled by the desktop app from its hosting services; the
/// browser cannot host ("Hosting needs the desktop app").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNet {
    /// The game's UDP port (QUIC).
    pub game_udp: u16,
    /// WebTransport's UDP port, if open (browsers).
    pub web_udp: Option<u16>,
    pub voice: HostVoice,
    pub router: HostRouter,
}

/// Why a hosting has no voice when its plan turned it off (spec §7.1.7
/// Q15).
pub const VOICE_TURNED_OFF: &str = "turned off on the Options page of Host a ride";

/// The voice server's default ports (spec §7.1.1: TCP 7880, TCP 7881,
/// UDP 7882).
pub const VOICE_PORTS: (u16, u16, u16) = (7880, 7881, 7882);

impl HostNet {
    /// Before the start (the summary): the plan's port, the bundled voice
    /// server's default ports (none with voice off), the router as
    /// planned.
    pub fn planned(plan: &HostPlan) -> Self {
        let (signal, tcp, udp) = VOICE_PORTS;
        Self {
            game_udp: plan.port,
            web_udp: None,
            voice: if plan.voice {
                HostVoice::On { signal, tcp, udp }
            } else {
                HostVoice::Off(VOICE_TURNED_OFF.into())
            },
            router: if plan.upnp {
                HostRouter::Searching
            } else {
                HostRouter::Off
            },
        }
    }

    /// The ports, short: `UDP 47800 · voice TCP 7880, 7881, UDP 7882`.
    pub fn ports_label(&self) -> String {
        let mut s = format!("UDP {}", self.game_udp);
        if let Some(w) = self.web_udp {
            s.push_str(&format!(", {w}"));
        }
        match &self.voice {
            HostVoice::On { signal, tcp, udp } => {
                s.push_str(&format!(" · voice TCP {signal}, {tcp}, UDP {udp}"));
            }
            HostVoice::Starting => s.push_str(" · voice starting"),
            HostVoice::Off(_) => s.push_str(" · no voice"),
        }
        s
    }

    /// The ports, one per entry, with what they are for.
    pub fn port_list(&self) -> Vec<String> {
        let mut out = vec![format!("UDP {} (game)", self.game_udp)];
        if let Some(w) = self.web_udp {
            out.push(format!("UDP {w} (game, browsers)"));
        }
        if let HostVoice::On { signal, tcp, udp } = &self.voice {
            out.push(format!("TCP {signal} (voice signalling)"));
            out.push(format!("TCP {tcp} (voice over TCP)"));
            out.push(format!("UDP {udp} (voice)"));
        }
        out
    }

    /// The router row's value.
    pub fn router_label(&self) -> String {
        match &self.router {
            HostRouter::Off => "open them by hand".into(),
            HostRouter::Searching => "opening (UPnP) …".into(),
            HostRouter::Opened { ports, of, .. } if ports == of => "opened (UPnP)".into(),
            HostRouter::Opened { ports, of, .. } => format!("{ports} of {of} opened (UPnP)"),
            HostRouter::NotAvailable(_) => "no UPnP router".into(),
            HostRouter::Failed(_) => "failed".into(),
        }
    }

    /// The ports, the firewall / port-forwarding hint and what the router
    /// did, for the page's text.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("Ports: {}.", self.port_list().join(", "))];
        let by_hand = "Open them in this computer's firewall; for riders from the internet also forward them on the router (README: Riding on a server on the internet).";
        match &self.router {
            HostRouter::Off => out.push(by_hand.into()),
            HostRouter::Searching => out.push(
                "The app asks the router to open them (UPnP); open them in this computer's firewall."
                    .into(),
            ),
            HostRouter::Opened { public, note, .. } => {
                out.push(match public {
                    Some(ip) => format!(
                        "The router opened them (UPnP) while you host; riders from the internet reach {ip}. Open them in this computer's firewall too."
                    ),
                    None => "The router opened them (UPnP) while you host; open them in this computer's firewall too.".into(),
                });
                out.extend(note.clone());
            }
            HostRouter::NotAvailable(why) | HostRouter::Failed(why) => {
                out.push(format!("The router did not open them: {why}."));
                out.push(by_hand.into());
            }
        }
        if let HostVoice::Off(why) = &self.voice {
            out.push(format!("No voice on this ride: {why}."));
        }
        out
    }

    /// The rows of the join link page: the ports and the router.
    pub fn rows(&self) -> Vec<Row> {
        vec![
            info_row("Ports", self.ports_label()),
            info_row("Router", self.router_label()),
        ]
    }
}

/// The summary page's text: where the server runs, the ports, the router
/// and the hint (spec §7.1.1 *Network*).
pub fn summary_lines(plan: &HostPlan) -> Vec<String> {
    let mut out = vec![if plan.voice {
        "The server and its voice server run in this app on all addresses; you ride on it as the host."
    } else {
        "The server runs in this app on all addresses; you ride on it as the host."
    }
    .to_owned()];
    out.extend(HostNet::planned(plan).lines());
    out
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

    /// *Open the router's ports* (spec §7.1.7 Q4): on by default for Host
    /// a ride, a row of the options, `host.plan upnp on|off`.
    #[test]
    fn the_router_switch_is_on_by_default_and_switchable() {
        let mut p = HostPlan::default();
        assert!(p.upnp);
        let w = worlds();
        let row = rows(Page::HostOptions, &p, &w, false)
            .into_iter()
            .find(|r| r.label == "Open the router's ports")
            .expect("an options row");
        assert_eq!(row.value.as_deref(), Some("on"));
        assert_eq!(
            row.confirm,
            Some(Activate::Command("host.plan upnp off".into()))
        );
        assert_eq!(row.left.as_deref(), Some("host.plan upnp off"));
        assert_eq!(
            run(&mut p, "upnp off").unwrap(),
            "host: the router's ports are opened by hand"
        );
        assert!(!p.upnp);
        let row = rows(Page::HostOptions, &p, &w, false)
            .into_iter()
            .find(|r| r.label == "Open the router's ports")
            .unwrap();
        assert_eq!(row.value.as_deref(), Some("off"));
        assert!(run(&mut p, "upnp maybe").is_err());
    }

    /// *Voice* (spec §7.1.7 Q15): on by default, a row of the options,
    /// `host.plan voice on|off`; off, the summary and its ports have no
    /// voice server and say why.
    #[test]
    fn the_voice_switch_is_on_by_default_and_switchable() {
        let mut p = HostPlan::default();
        assert!(p.voice);
        let w = worlds();
        let voice_row = |p: &HostPlan| {
            rows(Page::HostOptions, p, &w, true)
                .into_iter()
                .find(|r| r.label == "Voice")
                .expect("an options row")
        };
        let row = voice_row(&p);
        assert_eq!(row.value.as_deref(), Some("on"));
        assert_eq!(
            row.confirm,
            Some(Activate::Command("host.plan voice off".into()))
        );
        assert_eq!(row.right.as_deref(), Some("host.plan voice off"));
        assert_eq!(
            run(&mut p, "voice off").unwrap(),
            "host: no voice chat on this ride"
        );
        assert!(!p.voice);
        assert_eq!(voice_row(&p).value.as_deref(), Some("off"));
        assert!(run(&mut p, "voice maybe").is_err());
        // The summary: no voice server, no voice ports, why.
        let net = HostNet::planned(&p);
        assert_eq!(net.voice, HostVoice::Off(VOICE_TURNED_OFF.into()));
        assert_eq!(net.port_list(), ["UDP 47800 (game)"]);
        let text = summary_lines(&p).join("\n");
        assert!(text.starts_with("The server runs in this app"), "{text}");
        assert!(!text.contains("7880"), "{text}");
        assert!(
            text.contains("No voice on this ride: turned off on the Options page"),
            "{text}"
        );
        let s = rows(Page::HostSummary, &p, &w, false);
        assert_eq!(s[4].value.as_deref(), Some("5, UDP 47800, no voice"));
        run(&mut p, "voice on").unwrap();
        assert!(summary_lines(&p).join("\n").contains("TCP 7880"));
        // Who changes time and weather moved to the weather page.
        assert!(
            rows(Page::HostWeather, &p, &w, true)
                .iter()
                .any(|r| r.label == "Time and weather")
        );
        assert!(
            rows(Page::HostOptions, &p, &w, true)
                .iter()
                .all(|r| r.label != "Time and weather")
        );
    }

    /// The summary's text lists the game and voice ports with the hint,
    /// and what the router will do.
    #[test]
    fn the_summary_lists_the_ports_and_the_router() {
        let mut p = HostPlan::default();
        let text = summary_lines(&p).join("\n");
        assert!(
            text.contains(
                "Ports: UDP 47800 (game), TCP 7880 (voice signalling), TCP 7881 (voice over TCP), UDP 7882 (voice)."
            ),
            "{text}"
        );
        assert!(
            text.contains("asks the router to open them (UPnP)"),
            "{text}"
        );
        run(&mut p, "upnp off").unwrap();
        run(&mut p, "port 47900").unwrap();
        let text = summary_lines(&p).join("\n");
        assert!(text.contains("UDP 47900 (game)"), "{text}");
        assert!(
            text.contains("also forward them on the router (README"),
            "{text}"
        );
        // The summary's rows still fit (the ports are in its text).
        assert!(rows(Page::HostSummary, &p, &worlds(), true).len() <= 6);
    }

    /// The join link page's rows and text while hosting: the ports, the
    /// router's result (mapped / not available / failed and why), voice.
    #[test]
    fn the_hosting_shows_ports_router_and_voice() {
        let mut net = HostNet {
            game_udp: 47800,
            web_udp: None,
            voice: HostVoice::On {
                signal: 7880,
                tcp: 7881,
                udp: 7882,
            },
            router: HostRouter::Opened {
                public: Some("81.2.69.160".into()),
                ports: 4,
                of: 4,
                note: None,
            },
        };
        let r = net.rows();
        assert_eq!(r[0].label, "Ports");
        assert_eq!(
            r[0].value.as_deref(),
            Some("UDP 47800 · voice TCP 7880, 7881, UDP 7882")
        );
        assert_eq!(r[1].value.as_deref(), Some("opened (UPnP)"));
        assert!(r.iter().all(|r| r.confirm.is_none()), "information only");
        assert!(net.lines()[1].contains("riders from the internet reach 81.2.69.160"));
        net.router = HostRouter::Opened {
            public: None,
            ports: 3,
            of: 4,
            note: Some("TCP 7881 (voice over TCP) not opened: in use".into()),
        };
        assert_eq!(net.router_label(), "3 of 4 opened (UPnP)");
        assert!(net.lines().iter().any(|l| l.contains("7881")));
        net.router = HostRouter::NotAvailable("no UPnP router answered within 3 s".into());
        assert_eq!(net.router_label(), "no UPnP router");
        let l = net.lines();
        assert_eq!(
            l[1],
            "The router did not open them: no UPnP router answered within 3 s."
        );
        assert!(l[2].starts_with("Open them in this computer's firewall"));
        net.router = HostRouter::Failed("the router refused".into());
        assert_eq!(net.router_label(), "failed");
        net.router = HostRouter::Off;
        assert_eq!(net.router_label(), "open them by hand");
        // Without voice: the game's port only, and why.
        net.voice = HostVoice::Off("no livekit-server next to the app".into());
        assert_eq!(net.ports_label(), "UDP 47800 · no voice");
        assert_eq!(net.port_list(), ["UDP 47800 (game)"]);
        assert!(
            net.lines()
                .last()
                .unwrap()
                .contains("No voice on this ride: no livekit-server")
        );
        net.web_udp = Some(47801);
        assert_eq!(net.port_list()[1], "UDP 47801 (game, browsers)");
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
