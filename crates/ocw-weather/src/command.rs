//! The console command `weather` (spec §14 Phase 9, S4; the weather
//! machine's menu comes with S5): parsing and applying field changes, pure
//! so the desktop and the browser app share it and the tests run it.

use crate::ground::Ground;
use crate::presets::{PRESETS, preset};
use crate::state::{CloudType, Precip, WeatherState};

/// Usage of `weather`.
pub const USAGE: &str = "\
weather                      the weather now
weather list                 the presets
weather PRESET [in T]        a preset at once, or as a transition over T (90, 30s, 5m)
weather auto [SEED]          the weather simulation (fronts, showers) from now
weather FIELD=VALUE... [in T] change fields: clouds=0-1 type=cumulus|stratus|cirrus|cb
                             base=M rain|drizzle|snow|sleet|hail=MM_H precip=none
                             fog=VISIBILITY_M wind=M_S gust=M_S dir=DEG|SW temp=C
                             humidity=0-1 pressure=HPA lightning=PER_MIN
                             ground: wet=0-1 puddles=0-1 snowcover=0-1
weather strike [DIST_M [BEARING]] a lightning strike now (test)
Who may change it: you (solo), the host (shared ride), nobody (an organizer's event).";

/// One field change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Clouds(f32),
    Type(CloudType),
    Base(f32),
    Precip(Precip, f32),
    Fog(f32),
    Wind(f32),
    Gust(f32),
    Dir(f32),
    Temp(f32),
    Humidity(f32),
    Pressure(f32),
    Lightning(f32),
}

/// Ground overrides (factors 0–1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GroundChange {
    pub wetness: Option<f32>,
    pub puddles: Option<f32>,
    pub snow_cover: Option<f32>,
}

impl GroundChange {
    pub fn is_empty(&self) -> bool {
        self.wetness.is_none() && self.puddles.is_none() && self.snow_cover.is_none()
    }

    /// `g` with the overrides.
    pub fn apply(&self, g: Ground) -> Ground {
        Ground::with_factors(
            self.wetness.unwrap_or(g.wetness()),
            self.puddles.unwrap_or(g.puddles()),
            self.snow_cover.unwrap_or(g.snow_cover()),
        )
    }
}

/// What `weather …` asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum WeatherCommand {
    Show,
    List,
    Preset {
        name: String,
        state: WeatherState,
        transition_s: f64,
    },
    Set {
        fields: Vec<Field>,
        ground: GroundChange,
        transition_s: f64,
    },
    Simulate {
        seed: Option<u64>,
    },
    Strike {
        distance_m: f32,
        bearing_deg: Option<f32>,
    },
}

/// "90", "30s", "5m", "1h" → seconds.
pub fn parse_duration(s: &str) -> Result<f64, String> {
    let (num, unit) = match s.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => (&s[..i], &s[i..]),
        None => (s, "s"),
    };
    let v: f64 = num.parse().map_err(|_| format!("not a duration: {s}"))?;
    let k = match unit {
        "s" => 1.0,
        "m" | "min" => 60.0,
        "h" => 3600.0,
        _ => return Err(format!("not a duration: {s} (90, 30s, 5m, 1h)")),
    };
    if !(v >= 0.0 && v.is_finite()) {
        return Err(format!("not a duration: {s}"));
    }
    Ok(v * k)
}

fn number(key: &str, v: &str) -> Result<f32, String> {
    let v = v.trim_end_matches('%');
    v.parse::<f32>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("{key}: not a number: {v}"))
}

/// A share: 0–1, or a percentage (`80%`, or anything above 1).
fn share(key: &str, v: &str) -> Result<f32, String> {
    let x = number(key, v)?;
    let x = if v.ends_with('%') || x > 1.0 {
        x / 100.0
    } else {
        x
    };
    if (0.0..=1.0).contains(&x) {
        Ok(x)
    } else {
        Err(format!("{key}: 0-1 or 0-100 %"))
    }
}

fn direction(v: &str) -> Result<f32, String> {
    const POINTS: [&str; 16] = [
        "n", "nne", "ne", "ene", "e", "ese", "se", "sse", "s", "ssw", "sw", "wsw", "w", "wnw",
        "nw", "nnw",
    ];
    if let Some(i) = POINTS.iter().position(|p| *p == v.to_ascii_lowercase()) {
        return Ok(i as f32 * 22.5);
    }
    number("dir", v).map(|d| d.rem_euclid(360.0))
}

/// Parses the arguments of `weather`.
pub fn parse(args: &[&str]) -> Result<WeatherCommand, String> {
    let (args, transition_s) = match args {
        [rest @ .., "in", t] => (rest, parse_duration(t)?),
        _ => (args, 0.0),
    };
    match args {
        [] if transition_s == 0.0 => Ok(WeatherCommand::Show),
        [] => Err("weather in T: which weather?".into()),
        ["list"] => Ok(WeatherCommand::List),
        ["auto" | "simulate"] => Ok(WeatherCommand::Simulate { seed: None }),
        ["auto" | "simulate", seed] => Ok(WeatherCommand::Simulate {
            seed: Some(seed.parse().map_err(|_| format!("not a seed: {seed}"))?),
        }),
        ["strike", rest @ ..] => {
            let distance_m = match rest.first() {
                Some(d) => number("distance", d)?,
                None => 1500.0,
            };
            let bearing_deg = rest.get(1).map(|b| direction(b)).transpose()?;
            Ok(WeatherCommand::Strike {
                distance_m,
                bearing_deg,
            })
        }
        [name] if !name.contains('=') => {
            let state = preset(name).ok_or_else(|| {
                let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
                format!("unknown weather {name:?}; presets: {}", names.join(", "))
            })?;
            Ok(WeatherCommand::Preset {
                name: (*name).to_owned(),
                state,
                transition_s,
            })
        }
        fields => {
            let mut out = Vec::new();
            let mut ground = GroundChange::default();
            for f in fields {
                let Some((k, v)) = f.split_once('=') else {
                    return Err(format!(
                        "{f}: FIELD=VALUE expected (weather list: the presets)"
                    ));
                };
                let k = k.to_ascii_lowercase();
                let field = match k.as_str() {
                    "clouds" | "cover" => Field::Clouds(share(&k, v)?),
                    "type" => {
                        Field::Type(CloudType::parse(v).ok_or_else(|| {
                            format!("type: cumulus, stratus, cirrus or cb, not {v}")
                        })?)
                    }
                    "base" => Field::Base(number(&k, v)?),
                    "precip" if v == "none" => Field::Precip(Precip::None, 0.0),
                    "rain" | "drizzle" | "snow" | "sleet" | "hail" => {
                        Field::Precip(Precip::parse(&k).expect("listed"), number(&k, v)?.max(0.0))
                    }
                    "fog" | "visibility" | "vis" => Field::Fog(number(&k, v)?),
                    "wind" => Field::Wind(number(&k, v)?.max(0.0)),
                    "gust" | "gusts" => Field::Gust(number(&k, v)?.max(0.0)),
                    "dir" | "from" => Field::Dir(direction(v)?),
                    "temp" | "temperature" => Field::Temp(number(&k, v)?),
                    "humidity" => Field::Humidity(share(&k, v)?),
                    "pressure" => Field::Pressure(number(&k, v)?),
                    "lightning" => Field::Lightning(number(&k, v)?.max(0.0)),
                    "wet" | "wetness" => {
                        ground.wetness = Some(share(&k, v)?);
                        continue;
                    }
                    "puddles" => {
                        ground.puddles = Some(share(&k, v)?);
                        continue;
                    }
                    "snowcover" | "snow_cover" => {
                        ground.snow_cover = Some(share(&k, v)?);
                        continue;
                    }
                    _ => return Err(format!("unknown field {k:?}\n{USAGE}")),
                };
                out.push(field);
            }
            Ok(WeatherCommand::Set {
                fields: out,
                ground,
                transition_s,
            })
        }
    }
}

/// `base` with the field changes.
pub fn apply(fields: &[Field], base: WeatherState) -> WeatherState {
    let mut w = base;
    for f in fields {
        match *f {
            Field::Clouds(v) => w.cloud_cover = v,
            Field::Type(t) => w.cloud_type = t,
            Field::Base(m) => w.cloud_base_m = m,
            Field::Precip(p, mm) => {
                w.precip = if mm > 0.0 { p } else { Precip::None };
                w.precip_mm_h = mm;
            }
            Field::Fog(v) => w.fog_visibility_m = v,
            Field::Wind(v) => {
                w.wind_mps = v;
                w.gust_mps = w.gust_mps.max(v);
            }
            Field::Gust(v) => w.gust_mps = v.max(w.wind_mps),
            Field::Dir(d) => w.wind_from_deg = d,
            Field::Temp(c) => w.temperature_c = c,
            Field::Humidity(h) => w.humidity = h,
            Field::Pressure(p) => w.pressure_hpa = p,
            Field::Lightning(l) => w.lightning_per_min = l,
        }
    }
    w.clamped()
}

/// The presets, one per line, for `weather list`.
pub fn list() -> String {
    PRESETS
        .iter()
        .map(|(n, about)| format!("{n:<13} {about}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_transitions_and_simulation() {
        assert_eq!(parse(&[]).unwrap(), WeatherCommand::Show);
        assert_eq!(parse(&["list"]).unwrap(), WeatherCommand::List);
        let WeatherCommand::Preset {
            name,
            state,
            transition_s,
        } = parse(&["thunderstorm", "in", "2m"]).unwrap()
        else {
            panic!()
        };
        assert_eq!((name.as_str(), transition_s), ("thunderstorm", 120.0));
        assert_eq!(state, preset("thunderstorm").unwrap());
        assert!(matches!(
            parse(&["fog"]).unwrap(),
            WeatherCommand::Preset {
                transition_s: 0.0,
                ..
            }
        ));
        assert!(parse(&["tornado"]).unwrap_err().contains("presets: clear"));
        assert_eq!(
            parse(&["auto", "7"]).unwrap(),
            WeatherCommand::Simulate { seed: Some(7) }
        );
        assert_eq!(
            parse(&["auto"]).unwrap(),
            WeatherCommand::Simulate { seed: None }
        );
        assert_eq!(
            parse(&["strike", "800", "SW"]).unwrap(),
            WeatherCommand::Strike {
                distance_m: 800.0,
                bearing_deg: Some(225.0)
            }
        );
        assert!(parse(&["in", "5"]).is_err());
        assert_eq!(parse_duration("90").unwrap(), 90.0);
        assert_eq!(parse_duration("1h").unwrap(), 3600.0);
        assert!(parse_duration("5 parsecs").is_err());
        assert!(list().lines().count() == PRESETS.len());
    }

    #[test]
    fn fields_change_the_state_and_the_ground() {
        let WeatherCommand::Set {
            fields,
            ground,
            transition_s,
        } = parse(&[
            "rain=6",
            "wind=12",
            "dir=sw",
            "clouds=90%",
            "wet=1",
            "in",
            "30s",
        ])
        .unwrap()
        else {
            panic!()
        };
        assert_eq!(transition_s, 30.0);
        assert_eq!(ground.wetness, Some(1.0));
        let w = apply(&fields, WeatherState::default());
        assert_eq!((w.precip, w.precip_mm_h), (Precip::Rain, 6.0));
        assert_eq!((w.wind_mps, w.wind_from_deg), (12.0, 225.0));
        assert!(w.gust_mps >= 12.0, "gusts at least the wind");
        assert!((w.cloud_cover - 0.9).abs() < 1e-6);
        // Stopping the precipitation.
        let WeatherCommand::Set { fields, .. } = parse(&["rain=0"]).unwrap() else {
            panic!()
        };
        assert_eq!(apply(&fields, w).precip, Precip::None);
        let WeatherCommand::Set { fields, .. } = parse(&["precip=none", "type=cb"]).unwrap() else {
            panic!()
        };
        let w2 = apply(&fields, w);
        assert!(!w2.precipitating() && w2.cloud_type == CloudType::Cumulonimbus);
        assert!(parse(&["clouds=170"]).unwrap_err().contains("0-1"));
        assert!(
            parse(&["colour=blue"])
                .unwrap_err()
                .contains("unknown field")
        );
        assert!(parse(&["wind=fast"]).is_err());
        let g = GroundChange {
            snow_cover: Some(1.0),
            ..Default::default()
        };
        assert!(!g.is_empty());
        assert_eq!(g.apply(Ground::default()).snow_cover(), 1.0);
        assert!(GroundChange::default().is_empty());
    }
}
