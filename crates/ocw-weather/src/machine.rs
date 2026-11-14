//! The weather machine (spec §14 Phase 9, milestone S5): what the menu
//! *World → Weather* and the console offer on top of the session weather –
//! the transition times, the custom editor's fields with their ranges and
//! steps (validated and clamped here; the server clamps again), a random
//! weather from a seed, and the organiser's weather of an event.
//!
//! Pure, like the rest of the crate, so the desktop and the browser app
//! share it and the tests run it without Bevy.

use crate::lightning::unit;
use crate::presets::{PRESETS, preset};
use crate::state::{CLEAR_VISIBILITY_M, CloudType, Precip, WeatherState, compass};

/// The transition times the menu offers (s): at once, 1, 5 and 15 minutes.
pub const TRANSITIONS_S: [f64; 4] = [0.0, 60.0, 300.0, 900.0];

/// The menu's transition by default (s).
pub const DEFAULT_TRANSITION_S: f64 = 60.0;

/// "instant", "1 min", "15 min", "90 s".
pub fn transition_label(s: f64) -> String {
    if s <= 0.0 {
        "instant".into()
    } else if s >= 60.0 && (s / 60.0).fract().abs() < 1e-9 {
        format!("{:.0} min", s / 60.0)
    } else {
        format!("{s:.0} s")
    }
}

/// The next / previous transition of [`TRANSITIONS_S`] from `s` (`dir`
/// +1 / −1), wrapping around.
pub fn transition_step(s: f64, dir: i32) -> f64 {
    let n = TRANSITIONS_S.len() as i32;
    let at = TRANSITIONS_S
        .iter()
        .position(|&t| (t - s).abs() < 0.5)
        .map(|i| i as i32);
    let i = match at {
        Some(i) => (i + dir).rem_euclid(n),
        // Not one of the menu's: the nearest in that direction.
        None if dir > 0 => TRANSITIONS_S.iter().position(|&t| t > s).unwrap_or(0) as i32,
        None => TRANSITIONS_S
            .iter()
            .rposition(|&t| t < s)
            .unwrap_or(TRANSITIONS_S.len() - 1) as i32,
    };
    TRANSITIONS_S[i as usize]
}

/// " in 60" for a console line (empty at once).
pub fn transition_suffix(s: f64) -> String {
    if s > 0.0 {
        format!(" in {s:.0}")
    } else {
        String::new()
    }
}

/// The editor's ranges (spec Q9: the extremes are in; the console reaches
/// beyond, up to [`WeatherState::clamped`]).
pub mod range {
    /// Wind (m/s): calm to a hurricane's 60.
    pub const WIND_MPS: (f32, f32) = (0.0, 60.0);
    /// Gusts (m/s): at least the wind.
    pub const GUST_MPS: (f32, f32) = (0.0, 90.0);
    pub const TEMPERATURE_C: (f32, f32) = (-40.0, 50.0);
    /// Precipitation (mm/h, water equivalent).
    pub const PRECIP_MM_H: (f32, f32) = (0.1, 400.0);
    /// Fog visibility (m); the top is clear air.
    pub const FOG_M: (f32, f32) = (10.0, super::CLEAR_VISIBILITY_M);
}

/// The precipitation steps of the editor (mm/h).
const INTENSITY_MM_H: [f32; 20] = [
    0.1, 0.2, 0.3, 0.5, 1.0, 2.0, 4.0, 6.0, 8.0, 12.0, 15.0, 20.0, 30.0, 50.0, 75.0, 100.0, 150.0,
    200.0, 300.0, 400.0,
];
/// The fog steps of the editor (m); the last is clear air.
const FOG_M: [f32; 16] = [
    10.0,
    20.0,
    30.0,
    50.0,
    80.0,
    120.0,
    200.0,
    300.0,
    600.0,
    1000.0,
    2000.0,
    4000.0,
    8000.0,
    12_000.0,
    20_000.0,
    CLEAR_VISIBILITY_M,
];

/// What falls when the editor turns precipitation on (mm/h): a typical
/// amount per type.
pub fn typical_mm_h(p: Precip) -> f32 {
    match p {
        Precip::None => 0.0,
        Precip::Drizzle => 0.3,
        Precip::Rain => 4.0,
        Precip::Snow => 1.5,
        Precip::Sleet => 2.0,
        Precip::Hail => 25.0,
    }
}

/// A cumulonimbus brings lightning (strikes / min) when the editor makes one.
const CB_LIGHTNING_PER_MIN: f32 = 3.0;

/// One field of the custom editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Clouds,
    CloudType,
    Precip,
    Intensity,
    Fog,
    Wind,
    Gust,
    Direction,
    Temperature,
}

impl Field {
    /// All fields in the editor's order.
    pub const ALL: [Field; 9] = [
        Field::Clouds,
        Field::CloudType,
        Field::Precip,
        Field::Intensity,
        Field::Fog,
        Field::Wind,
        Field::Gust,
        Field::Direction,
        Field::Temperature,
    ];
    /// The first page of the editor: the sky.
    pub const SKY: [Field; 5] = [
        Field::Clouds,
        Field::CloudType,
        Field::Precip,
        Field::Intensity,
        Field::Fog,
    ];
    /// The second page: wind and temperature.
    pub const AIR: [Field; 4] = [
        Field::Wind,
        Field::Gust,
        Field::Direction,
        Field::Temperature,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Clouds => "Cloud cover",
            Self::CloudType => "Cloud type",
            Self::Precip => "Precipitation",
            Self::Intensity => "Intensity",
            Self::Fog => "Fog",
            Self::Wind => "Wind",
            Self::Gust => "Gusts",
            Self::Direction => "Wind from",
            Self::Temperature => "Temperature",
        }
    }

    /// The field's value in `w`, as the editor shows it.
    pub fn value(self, w: &WeatherState) -> String {
        match self {
            Self::Clouds => format!("{:.0} %", w.cloud_cover * 100.0),
            Self::CloudType => w.cloud_type.name().into(),
            Self::Precip => {
                if w.precipitating() {
                    w.precip.name().into()
                } else {
                    "none".into()
                }
            }
            Self::Intensity => {
                if w.precipitating() {
                    mm_label(w.precip_mm_h)
                } else {
                    "-".into()
                }
            }
            Self::Fog => {
                if w.fog_visibility_m >= CLEAR_VISIBILITY_M - 1.0 {
                    "none".into()
                } else {
                    format!("{} visibility", distance_label(w.fog_visibility_m))
                }
            }
            Self::Wind => format!("{:.0} m/s ({:.0} km/h)", w.wind_mps, w.wind_mps * 3.6),
            Self::Gust => format!("{:.0} m/s", w.gust_mps),
            Self::Direction => format!(
                "{} ({:.0}°)",
                compass(w.wind_from_deg),
                w.wind_from_deg.round().rem_euclid(360.0)
            ),
            Self::Temperature => format!("{:.0} °C", w.temperature_c),
        }
    }

    /// `w` with this field one step up (`dir` > 0) or down, within the
    /// editor's ranges and kept consistent ([`validate`]).
    pub fn step(self, w: &WeatherState, dir: i32) -> WeatherState {
        let mut n = *w;
        let up = dir > 0;
        match self {
            Self::Clouds => n.cloud_cover = grid_step(w.cloud_cover, 0.1, up).clamp(0.0, 1.0),
            Self::CloudType => {
                let all = CloudType::ALL;
                let i = all.iter().position(|t| *t == w.cloud_type).unwrap_or(0) as i32;
                n.cloud_type =
                    all[(i + if up { 1 } else { -1 }).rem_euclid(all.len() as i32) as usize];
                if n.cloud_type == CloudType::Cumulonimbus {
                    n.lightning_per_min = n.lightning_per_min.max(CB_LIGHTNING_PER_MIN);
                    n.cloud_cover = n.cloud_cover.max(0.8);
                }
            }
            Self::Precip => {
                let all = Precip::ALL;
                let cur = if w.precipitating() {
                    w.precip
                } else {
                    Precip::None
                };
                let i = all.iter().position(|p| *p == cur).unwrap_or(0) as i32;
                let p = all[(i + if up { 1 } else { -1 }).rem_euclid(all.len() as i32) as usize];
                n.precip = p;
                n.precip_mm_h = typical_mm_h(p);
                if p != Precip::None {
                    // Something falls from a grey sky.
                    n.cloud_cover = n.cloud_cover.max(0.8);
                }
            }
            Self::Intensity => {
                if w.precipitating() {
                    n.precip_mm_h = ladder(&INTENSITY_MM_H, w.precip_mm_h, up);
                }
            }
            Self::Fog => n.fog_visibility_m = ladder(&FOG_M, w.fog_visibility_m, up),
            Self::Wind => {
                n.wind_mps =
                    grid_step(w.wind_mps, 1.0, up).clamp(range::WIND_MPS.0, range::WIND_MPS.1);
                // The gusts keep their margin above the wind.
                n.gust_mps = w.gust_mps + (n.wind_mps - w.wind_mps);
            }
            Self::Gust => {
                n.gust_mps =
                    grid_step(w.gust_mps, 1.0, up).clamp(range::GUST_MPS.0, range::GUST_MPS.1);
            }
            Self::Direction => {
                n.wind_from_deg = grid_step(w.wind_from_deg, 22.5, up).rem_euclid(360.0);
            }
            Self::Temperature => {
                n.temperature_c = grid_step(w.temperature_c, 1.0, up)
                    .clamp(range::TEMPERATURE_C.0, range::TEMPERATURE_C.1);
            }
        }
        validate(n)
    }

    /// The `weather` console fields that set this field to its value in
    /// `w` ("clouds=0.7", "rain=4", "precip=none", "dir=225").
    pub fn assignment(self, w: &WeatherState) -> String {
        match self {
            Self::Clouds => format!("clouds={}", num(w.cloud_cover)),
            Self::CloudType => format!(
                "type={} lightning={}",
                w.cloud_type.name(),
                num(w.lightning_per_min)
            ),
            Self::Precip | Self::Intensity => {
                if w.precipitating() {
                    format!(
                        "{}={} clouds={}",
                        w.precip.name(),
                        num(w.precip_mm_h),
                        num(w.cloud_cover)
                    )
                } else {
                    "precip=none".into()
                }
            }
            Self::Fog => format!("fog={}", num(w.fog_visibility_m)),
            Self::Wind => format!("wind={} gust={}", num(w.wind_mps), num(w.gust_mps)),
            Self::Gust => format!("gust={}", num(w.gust_mps)),
            Self::Direction => format!("dir={}", num(w.wind_from_deg)),
            Self::Temperature => format!("temp={}", num(w.temperature_c)),
        }
    }
}

/// A number for a console line: no needless decimals ("0.7", "4", "12.5").
fn num(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    let s = format!("{r:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// "0.3 mm/h", "15 mm/h".
pub fn mm_label(mm: f32) -> String {
    if mm < 1.0 {
        format!("{mm:.1} mm/h")
    } else {
        format!("{mm:.0} mm/h")
    }
}

/// "80 m", "1.2 km", "12 km".
pub fn distance_label(m: f32) -> String {
    if m < 1000.0 {
        format!("{m:.0} m")
    } else if m < 10_000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{:.0} km", m / 1000.0)
    }
}

/// The next multiple of `size` above `v` (`up`) or below it: 0.05 → 0.1
/// or 0, 0.3 → 0.4 or 0.2.
fn grid_step(v: f32, size: f32, up: bool) -> f32 {
    let x = v / size;
    let k = if up {
        (x + 1e-3).floor() + 1.0
    } else {
        (x - 1e-3).ceil() - 1.0
    };
    k * size
}

/// The next step of `steps` above (or below) `v`; at the ends it stays.
fn ladder(steps: &[f32], v: f32, up: bool) -> f32 {
    if up {
        steps
            .iter()
            .copied()
            .find(|&s| s > v * 1.001)
            .unwrap_or(steps[steps.len() - 1])
    } else {
        steps
            .iter()
            .rev()
            .copied()
            .find(|&s| s < v * 0.999)
            .unwrap_or(steps[0])
    }
}

/// `w` within the editor's ranges and consistent: gusts at least the wind,
/// no intensity without precipitation (and none as precipitation without an
/// intensity), lightning only from a cumulonimbus; then
/// [`WeatherState::clamped`] (the server clamps again).
pub fn validate(w: WeatherState) -> WeatherState {
    let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
    let d = WeatherState::default();
    let mut n = w;
    n.wind_mps = fin(w.wind_mps, d.wind_mps).clamp(range::WIND_MPS.0, range::WIND_MPS.1);
    n.gust_mps = fin(w.gust_mps, n.wind_mps).clamp(n.wind_mps, range::GUST_MPS.1);
    n.temperature_c =
        fin(w.temperature_c, d.temperature_c).clamp(range::TEMPERATURE_C.0, range::TEMPERATURE_C.1);
    n.fog_visibility_m =
        fin(w.fog_visibility_m, d.fog_visibility_m).clamp(range::FOG_M.0, range::FOG_M.1);
    let mm = fin(w.precip_mm_h, 0.0);
    if w.precip == Precip::None || mm <= 0.0 {
        n.precip = Precip::None;
        n.precip_mm_h = 0.0;
    } else {
        n.precip_mm_h = mm.clamp(range::PRECIP_MM_H.0, range::PRECIP_MM_H.1);
    }
    if n.cloud_type != CloudType::Cumulonimbus {
        n.lightning_per_min = 0.0;
    }
    n.clamped()
}

/// A random weather from `seed`: one of the presets, varied like a step of
/// the simulation (cover ±0.1, wind ×0.7–1.3, direction ±30°, intensity
/// ×0.6–1.5, temperature ±3 °C). The same seed gives the same weather.
pub fn random(seed: u64) -> (&'static str, WeatherState) {
    let r = |n: u64| unit(seed, 0x5EED, n) as f32;
    let i = ((r(0) * PRESETS.len() as f32) as usize).min(PRESETS.len() - 1);
    let name = PRESETS[i].0;
    let mut w = preset(name).unwrap_or_default();
    w.cloud_cover = (w.cloud_cover + (r(1) - 0.5) * 0.2).clamp(0.0, 1.0);
    let k = 0.7 + 0.6 * r(2);
    w.wind_mps *= k;
    w.gust_mps = (w.gust_mps * k).max(w.wind_mps);
    w.wind_from_deg = (w.wind_from_deg + (r(3) - 0.5) * 60.0).rem_euclid(360.0);
    if w.precipitating() {
        w.precip_mm_h *= 0.6 + 0.9 * r(4);
    }
    w.temperature_c += (r(5) - 0.5) * 6.0;
    // Snow stays frozen, rain stays liquid.
    match w.precip {
        Precip::Snow => w.temperature_c = w.temperature_c.min(0.5),
        Precip::Rain | Precip::Drizzle => w.temperature_c = w.temperature_c.max(3.0),
        _ => {}
    }
    (name, w.clamped())
}

/// The organiser's weather of an event (spec §14 Phase 9, S5; Q4: the
/// organiser decides, riders cannot change it while the event runs).
#[derive(Debug, Clone, PartialEq)]
pub enum EventWeather {
    /// A preset ("thunderstorm"), a varied one or a custom state
    /// ("custom"), fixed for the event.
    Fixed { name: String, state: WeatherState },
    /// The simulation with this seed (the same fronts for every run).
    Simulated { seed: u64 },
}

impl EventWeather {
    /// "thunderstorm", "custom: rain 4.0 mm/h, …", "simulated (seed 7)".
    pub fn label(&self) -> String {
        match self {
            Self::Fixed { name, state } if preset(name) == Some(*state) => name.replace('-', " "),
            Self::Fixed { name, state } => format!("{}: {}", name.replace('-', " "), state.hud()),
            Self::Simulated { seed } => format!("simulated (seed {seed})"),
        }
    }
}

/// Usage of the event's `--weather`.
pub const EVENT_USAGE: &str =
    "--weather PRESET [FIELD=VALUE...] | custom FIELD=VALUE... | random SEED | auto SEED";

/// Parses an event's `--weather` values: a preset (optionally changed by
/// `FIELD=VALUE`s), `custom FIELD=VALUE…` (on the clear default), `random
/// SEED` or `auto SEED` / `simulated SEED` (the simulation; without a seed
/// 1, so every run of the event is the same).
pub fn parse_event_weather(args: &[&str]) -> Result<EventWeather, String> {
    use crate::command::{WeatherCommand, parse};
    let seed = |s: Option<&&str>| -> Result<u64, String> {
        s.map_or(Ok(1), |s| s.parse().map_err(|_| format!("not a seed: {s}")))
    };
    match args {
        [] => Err(format!("which weather? {EVENT_USAGE}")),
        ["auto" | "simulate" | "simulated", rest @ ..] if rest.len() <= 1 => {
            Ok(EventWeather::Simulated {
                seed: seed(rest.first())?,
            })
        }
        ["random", rest @ ..] if rest.len() <= 1 => {
            let s = seed(rest.first())?;
            let (name, state) = random(s);
            Ok(EventWeather::Fixed {
                name: format!("random {s} ({name})"),
                state,
            })
        }
        ["custom", fields @ ..] => match parse(fields)? {
            WeatherCommand::Set {
                fields,
                ground,
                transition_s,
            } if ground.is_empty() && transition_s == 0.0 => Ok(EventWeather::Fixed {
                name: "custom".into(),
                state: validate(crate::command::apply(&fields, WeatherState::default())),
            }),
            _ => Err(format!("custom FIELD=VALUE... ({EVENT_USAGE})")),
        },
        _ => match parse(args)? {
            WeatherCommand::Preset {
                name,
                state,
                transition_s: 0.0,
            } => Ok(EventWeather::Fixed {
                name: if preset(&name) == Some(state) {
                    name
                } else {
                    format!("{name}, changed")
                },
                state,
            }),
            _ => Err(format!("an event's weather: {EVENT_USAGE}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_step_through_the_menus_times() {
        assert_eq!(transition_label(0.0), "instant");
        assert_eq!(transition_label(60.0), "1 min");
        assert_eq!(transition_label(900.0), "15 min");
        assert_eq!(transition_label(90.0), "90 s");
        assert_eq!(transition_step(0.0, 1), 60.0);
        assert_eq!(transition_step(60.0, 1), 300.0);
        assert_eq!(transition_step(900.0, 1), 0.0, "wraps");
        assert_eq!(transition_step(0.0, -1), 900.0, "wraps back");
        assert_eq!(transition_step(90.0, 1), 300.0, "from a console value");
        assert_eq!(transition_step(90.0, -1), 60.0);
        assert_eq!(transition_suffix(0.0), "");
        assert_eq!(transition_suffix(300.0), " in 300");
    }

    /// The custom editor (spec §14 Phase 9, S5): every step stays within
    /// the editor's ranges and consistent, at both ends.
    #[test]
    fn the_editor_steps_within_its_ranges_and_stays_consistent() {
        let clear = preset("clear").unwrap();
        // Many steps up and down never leave the ranges.
        for f in Field::ALL {
            for start in PRESETS.map(|p| preset(p.0).unwrap()) {
                let mut w = start;
                for dir in [1, -1] {
                    for _ in 0..120 {
                        w = f.step(&w, dir);
                        assert_eq!(validate(w), w, "{f:?}: {w:?}");
                        assert!(w.gust_mps >= w.wind_mps);
                        assert!(w.precipitating() || w.precip_mm_h == 0.0);
                        assert!(
                            (range::WIND_MPS.0..=range::WIND_MPS.1).contains(&w.wind_mps),
                            "{w:?}"
                        );
                        assert!(
                            (range::TEMPERATURE_C.0..=range::TEMPERATURE_C.1)
                                .contains(&w.temperature_c)
                        );
                    }
                }
            }
        }
        // The ends.
        let mut w = clear;
        for _ in 0..200 {
            w = Field::Temperature.step(&w, 1);
        }
        assert_eq!(w.temperature_c, 50.0);
        for _ in 0..30 {
            w = Field::Fog.step(&w, -1);
        }
        assert_eq!(w.fog_visibility_m, 10.0);
        assert_eq!(Field::Fog.value(&w), "10 m visibility");
        assert_eq!(Field::Fog.value(&clear), "none");
        // Clouds in tenths.
        let w = Field::Clouds.step(&clear, 1);
        assert!((w.cloud_cover - 0.1).abs() < 1e-6, "{}", w.cloud_cover);
        assert_eq!(Field::Clouds.value(&w), "10 %");
        // Turning precipitation on: rain at its typical amount, from a grey sky.
        let w = Field::Precip.step(&Field::Precip.step(&clear, 1), 1);
        assert_eq!((w.precip, w.precip_mm_h), (Precip::Rain, 4.0));
        assert!(w.cloud_cover >= 0.8);
        assert_eq!(Field::Intensity.value(&w), "4 mm/h");
        let heavier = Field::Intensity.step(&w, 1);
        assert_eq!(heavier.precip_mm_h, 6.0);
        // No intensity without precipitation.
        assert_eq!(Field::Intensity.step(&clear, 1), validate(clear));
        assert_eq!(Field::Intensity.value(&clear), "-");
        // Back to none.
        let none = Field::Precip.step(&Field::Precip.step(&w, -1), -1);
        assert!(!none.precipitating() && none.precip_mm_h == 0.0);
        // Wind: the gusts keep their margin; the gusts never below the wind.
        let w = Field::Wind.step(&clear, 1);
        assert_eq!((w.wind_mps, w.gust_mps), (3.0, 5.0));
        let mut g = w;
        for _ in 0..10 {
            g = Field::Gust.step(&g, -1);
        }
        assert_eq!(g.gust_mps, g.wind_mps);
        // The direction in compass points, wrapping.
        let mut d = clear;
        d.wind_from_deg = 0.0;
        assert_eq!(Field::Direction.step(&d, -1).wind_from_deg, 337.5);
        assert_eq!(Field::Direction.value(&d), "N (0°)");
        // A cumulonimbus brings lightning; another type takes it away.
        let cb = Field::CloudType.step(&clear, -1);
        assert_eq!(cb.cloud_type, CloudType::Cumulonimbus);
        assert!(cb.lightning_per_min > 0.0);
        assert_eq!(Field::CloudType.step(&cb, 1).lightning_per_min, 0.0);
    }

    #[test]
    fn invalid_values_are_clamped_and_made_consistent() {
        let mut w = preset("rain").unwrap();
        w.wind_mps = 500.0;
        w.gust_mps = 1.0;
        w.temperature_c = f32::NAN;
        w.fog_visibility_m = -3.0;
        w.precip_mm_h = 9999.0;
        w.lightning_per_min = 9.0;
        let v = validate(w);
        assert_eq!(v.wind_mps, 60.0);
        assert_eq!(v.gust_mps, 60.0);
        assert_eq!(v.temperature_c, WeatherState::default().temperature_c);
        assert_eq!(v.fog_visibility_m, 10.0);
        assert_eq!(v.precip_mm_h, 400.0);
        assert_eq!(v.lightning_per_min, 0.0, "no lightning from stratus");
        let mut none = preset("clear").unwrap();
        none.precip_mm_h = 5.0;
        assert_eq!(validate(none).precip_mm_h, 0.0);
        let mut zero = preset("rain").unwrap();
        zero.precip_mm_h = 0.0;
        assert_eq!(validate(zero).precip, Precip::None);
    }

    /// The editor's console lines parse and give the edited state.
    #[test]
    fn editor_steps_become_console_lines() {
        use crate::command::{WeatherCommand, apply, parse};
        let base = preset("cloudy").unwrap();
        for f in Field::ALL {
            for dir in [1, -1] {
                let w = f.step(&base, dir);
                let line = f.assignment(&w);
                let args: Vec<&str> = line.split_whitespace().collect();
                let WeatherCommand::Set { fields, .. } =
                    parse(&args).unwrap_or_else(|e| panic!("{line}: {e}"))
                else {
                    panic!("{line}")
                };
                let got = apply(&fields, base);
                let v = f.value(&w);
                assert_eq!(f.value(&got), v, "{f:?} {line}");
            }
        }
        assert_eq!(num(0.7000001), "0.7");
        assert_eq!(num(4.0), "4");
        assert_eq!(num(337.5), "337.5");
    }

    #[test]
    fn random_weather_is_seeded_and_varied() {
        let a = random(7);
        assert_eq!(a, random(7), "the same seed, the same weather");
        let names: std::collections::BTreeSet<&str> = (0..200).map(|s| random(s).0).collect();
        assert!(names.len() >= 12, "{names:?}");
        for s in 0..200 {
            let (name, w) = random(s);
            assert_eq!(w.clamped(), w);
            if name.contains("snow") {
                assert!(w.temperature_c <= 0.5);
            }
            assert!(w.gust_mps >= w.wind_mps);
        }
    }

    #[test]
    fn event_weather_from_the_organisers_line() {
        let ts = parse_event_weather(&["thunderstorm"]).unwrap();
        assert_eq!(
            ts,
            EventWeather::Fixed {
                name: "thunderstorm".into(),
                state: preset("thunderstorm").unwrap()
            }
        );
        assert_eq!(ts.label(), "thunderstorm");
        let EventWeather::Fixed { name, state } =
            parse_event_weather(&["rain", "temp=5", "wind=12"]).unwrap()
        else {
            panic!()
        };
        assert_eq!(name, "rain, changed");
        assert_eq!((state.temperature_c, state.wind_mps), (5.0, 12.0));
        let EventWeather::Fixed { name, state } =
            parse_event_weather(&["custom", "snow=2", "temp=-3", "fog=400"]).unwrap()
        else {
            panic!()
        };
        assert_eq!(name, "custom");
        assert_eq!((state.precip, state.temperature_c), (Precip::Snow, -3.0));
        assert!(
            parse_event_weather(&["custom", "snow=2"])
                .unwrap()
                .label()
                .starts_with("custom: snow")
        );
        assert_eq!(
            parse_event_weather(&["auto", "7"]).unwrap(),
            EventWeather::Simulated { seed: 7 }
        );
        assert_eq!(
            parse_event_weather(&["simulated"]).unwrap().label(),
            "simulated (seed 1)"
        );
        assert!(
            parse_event_weather(&["random", "3"])
                .unwrap()
                .label()
                .starts_with("random 3")
        );
        assert!(parse_event_weather(&[]).is_err());
        assert!(parse_event_weather(&["thunderstorm", "in", "60"]).is_err());
        assert!(parse_event_weather(&["custom", "wet=1"]).is_err());
        assert!(parse_event_weather(&["tornado"]).is_err());
    }
}
