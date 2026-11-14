//! Named weather states (spec §14 Phase 9, 9E: the weather machine's
//! presets come with S5; S4 has them for the console and the tests),
//! extremes included (decision Q9: storms, hail, heavy snow, thunderstorms,
//! dense fog, gale-force wind).

use crate::state::{CloudType, Precip, WeatherState};

/// Name and one-line description of every preset, in menu order.
pub const PRESETS: [(&str, &str); 17] = [
    ("clear", "clear sky, light wind"),
    ("fair", "fair-weather cumulus, sun between the clouds"),
    ("cloudy", "broken cumulus, sun now and then"),
    ("overcast", "a closed grey layer, flat light, no shadows"),
    ("fog", "dense fog (visibility 80 m), still air"),
    ("mist", "mist and low cloud (visibility 600 m)"),
    ("drizzle", "drizzle under a low grey layer"),
    ("light-rain", "light rain (1 mm/h)"),
    ("rain", "rain (4 mm/h)"),
    ("heavy-rain", "heavy rain (15 mm/h), puddles"),
    ("thunderstorm", "thunderstorm: cloudburst, gusts, lightning"),
    ("hail", "hailstorm under a thunder cloud"),
    ("snow", "snowfall (1.5 mm/h), below freezing"),
    (
        "heavy-snow",
        "heavy snow (5 mm/h), visibility a few hundred metres",
    ),
    ("sleet", "sleet around freezing"),
    ("windy", "dry and windy, gusts to 18 m/s"),
    ("storm", "gale with rain, gusts to 32 m/s"),
];

/// The preset `name` (see [`PRESETS`]).
pub fn preset(name: &str) -> Option<WeatherState> {
    let d = WeatherState::default();
    let rain = |mm: f32| WeatherState {
        cloud_cover: 0.97,
        cloud_type: CloudType::Stratus,
        cloud_base_m: 700.0,
        precip: Precip::Rain,
        precip_mm_h: mm,
        fog_visibility_m: 12_000.0,
        wind_mps: 4.0,
        gust_mps: 8.0,
        temperature_c: 11.0,
        humidity: 0.93,
        pressure_hpa: 1004.0,
        ..d
    };
    let snow = |mm: f32, vis: f32, t: f32| WeatherState {
        cloud_cover: 1.0,
        cloud_type: CloudType::Stratus,
        cloud_base_m: 500.0,
        precip: Precip::Snow,
        precip_mm_h: mm,
        fog_visibility_m: vis,
        wind_mps: 3.0,
        gust_mps: 7.0,
        wind_from_deg: 300.0,
        temperature_c: t,
        humidity: 0.92,
        pressure_hpa: 1008.0,
        ..d
    };
    Some(match name {
        "clear" => d,
        "fair" => WeatherState {
            cloud_cover: 0.3,
            cloud_base_m: 1400.0,
            wind_mps: 3.0,
            gust_mps: 6.0,
            temperature_c: 20.0,
            humidity: 0.5,
            ..d
        },
        "cloudy" => WeatherState {
            cloud_cover: 0.65,
            cloud_base_m: 1200.0,
            wind_mps: 4.0,
            gust_mps: 8.0,
            temperature_c: 16.0,
            humidity: 0.65,
            pressure_hpa: 1012.0,
            ..d
        },
        "overcast" => WeatherState {
            cloud_cover: 1.0,
            cloud_type: CloudType::Stratus,
            cloud_base_m: 900.0,
            fog_visibility_m: 20_000.0,
            wind_mps: 3.0,
            gust_mps: 6.0,
            temperature_c: 12.0,
            humidity: 0.8,
            pressure_hpa: 1010.0,
            ..d
        },
        "fog" => WeatherState {
            cloud_cover: 1.0,
            cloud_type: CloudType::Stratus,
            cloud_base_m: 100.0,
            fog_visibility_m: 80.0,
            wind_mps: 0.5,
            gust_mps: 1.0,
            temperature_c: 4.0,
            humidity: 1.0,
            pressure_hpa: 1025.0,
            ..d
        },
        "mist" => WeatherState {
            cloud_cover: 0.95,
            cloud_type: CloudType::Stratus,
            cloud_base_m: 250.0,
            fog_visibility_m: 600.0,
            wind_mps: 1.0,
            gust_mps: 2.0,
            temperature_c: 7.0,
            humidity: 0.98,
            pressure_hpa: 1022.0,
            ..d
        },
        "drizzle" => WeatherState {
            precip: Precip::Drizzle,
            precip_mm_h: 0.3,
            cloud_base_m: 300.0,
            fog_visibility_m: 4000.0,
            wind_mps: 2.0,
            gust_mps: 4.0,
            temperature_c: 9.0,
            humidity: 0.98,
            ..rain(0.0)
        },
        "light-rain" => WeatherState {
            cloud_cover: 0.92,
            cloud_base_m: 900.0,
            ..rain(1.0)
        },
        "rain" => rain(4.0),
        "heavy-rain" => WeatherState {
            cloud_base_m: 500.0,
            fog_visibility_m: 8000.0,
            wind_mps: 6.0,
            gust_mps: 12.0,
            ..rain(15.0)
        },
        "thunderstorm" => WeatherState {
            cloud_cover: 1.0,
            cloud_type: CloudType::Cumulonimbus,
            cloud_base_m: 800.0,
            precip: Precip::Rain,
            precip_mm_h: 30.0,
            fog_visibility_m: 6000.0,
            wind_mps: 9.0,
            gust_mps: 22.0,
            wind_from_deg: 240.0,
            temperature_c: 19.0,
            humidity: 0.95,
            pressure_hpa: 1002.0,
            lightning_per_min: 6.0,
        },
        "hail" => WeatherState {
            precip: Precip::Hail,
            precip_mm_h: 25.0,
            temperature_c: 14.0,
            lightning_per_min: 4.0,
            ..preset("thunderstorm")?
        },
        "snow" => snow(1.5, 3000.0, -2.0),
        "heavy-snow" => WeatherState {
            wind_mps: 6.0,
            gust_mps: 12.0,
            ..snow(5.0, 1500.0, -5.0)
        },
        "sleet" => WeatherState {
            precip: Precip::Sleet,
            temperature_c: 1.0,
            ..snow(2.0, 4000.0, 1.0)
        },
        "windy" => WeatherState {
            cloud_cover: 0.4,
            cloud_base_m: 1600.0,
            wind_mps: 11.0,
            gust_mps: 18.0,
            wind_from_deg: 260.0,
            temperature_c: 15.0,
            humidity: 0.5,
            pressure_hpa: 1006.0,
            ..d
        },
        "storm" => WeatherState {
            cloud_cover: 1.0,
            cloud_type: CloudType::Stratus,
            cloud_base_m: 500.0,
            precip: Precip::Rain,
            precip_mm_h: 8.0,
            fog_visibility_m: 7000.0,
            wind_mps: 21.0,
            gust_mps: 32.0,
            wind_from_deg: 255.0,
            temperature_c: 10.0,
            humidity: 0.9,
            pressure_hpa: 975.0,
            lightning_per_min: 0.0,
        },
        _ => return None,
    })
}

/// The preset's name for a menu: "Heavy rain" for `heavy-rain`.
pub fn title(name: &str) -> String {
    let s = name.replace('-', " ");
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The preset closest to `w` (for the menu's "preset" row; by condition
/// and intensity).
pub fn nearest(w: &WeatherState) -> &'static str {
    PRESETS
        .iter()
        .map(|(n, _)| (*n, distance(w, &preset(n).expect("listed"))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or("clear", |(n, _)| n)
}

fn distance(a: &WeatherState, b: &WeatherState) -> f32 {
    let mut d = (a.cloud_cover - b.cloud_cover).abs() * 2.0
        + (a.precip_mm_h.ln_1p() - b.precip_mm_h.ln_1p()).abs()
        + (a.fog_visibility_m.ln() - b.fog_visibility_m.ln()).abs() * 0.5
        + (a.wind_mps - b.wind_mps).abs() * 0.1
        + (a.lightning_per_min - b.lightning_per_min).abs() * 0.3
        + (a.temperature_c - b.temperature_c).abs() * 0.02;
    if a.precip != b.precip {
        d += 2.0;
    }
    if a.cloud_type != b.cloud_type {
        d += 0.3;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid_and_named() {
        for (name, about) in PRESETS {
            let w = preset(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(w.clamped(), w, "{name} is within the ranges");
            assert!(!about.is_empty());
            assert_eq!(nearest(&w), name, "{name} is its own nearest");
        }
        assert!(preset("tornado").is_none());
        assert_eq!(title("heavy-rain"), "Heavy rain");
        assert_eq!(title("clear"), "Clear");
    }

    /// Decision Q9: the extremes are there, uncapped.
    #[test]
    fn extreme_weather_is_included() {
        let storm = preset("storm").unwrap();
        assert!(storm.gust_mps >= 28.0, "gale-force gusts");
        let ts = preset("thunderstorm").unwrap();
        assert!(ts.lightning_per_min > 0.0 && ts.precip_mm_h >= 25.0);
        assert!(preset("fog").unwrap().visibility_m() < 100.0);
        assert!(preset("heavy-snow").unwrap().visibility_m() < 500.0);
        assert_eq!(preset("hail").unwrap().precip, Precip::Hail);
    }
}
