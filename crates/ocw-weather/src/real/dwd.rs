//! DWD open data (spec 9F, Q6: regional, Germany): the hourly observations
//! of the station nearest to the place ("POI" reports,
//! `opendata.dwd.de/weather/weather_reports/poi/<id>-BEOB.csv`): cloud
//! cover and base, visibility, the present weather, temperature, dew
//! point, humidity, pressure, wind, gusts and the last hour's
//! precipitation. CC BY 4.0 (the DWD's legal notice: its open spatial
//! data may be re-used with the source acknowledged), "Source: Deutscher
//! Wetterdienst".
//!
//! The ICON-D2 model (GRIB2) and the MOSMIX point forecasts are open data
//! too; reading them is left for later (the global forecast covers the next
//! hours).

use super::{Obs, Precip, Series, Sky, StationPick};

/// The request for `station`.
pub fn url(station: &StationPick) -> String {
    format!(
        "https://opendata.dwd.de/weather/weather_reports/poi/{}-BEOB.csv",
        station.id
    )
}

/// The DWD's present-weather condition (1–31, as in the POI reports).
pub fn condition_sky(code: i64) -> Sky {
    let p = |precip, strength, showers| Sky {
        precip: Some(precip),
        strength,
        showers,
        ..Default::default()
    };
    match code {
        1 => Sky {
            cover: Some(0.0),
            ..Default::default()
        },
        2 => Sky {
            cover: Some(0.3),
            ..Default::default()
        },
        3 => Sky {
            cover: Some(0.7),
            ..Default::default()
        },
        4 => Sky {
            cover: Some(1.0),
            ..Default::default()
        },
        5 | 6 => Sky {
            fog: true,
            cover: Some(1.0),
            ..Default::default()
        },
        7 => p(Precip::Rain, 0, false),
        8 | 10 => p(Precip::Rain, 1, false),
        9 | 11 => p(Precip::Rain, 2, false),
        12 => p(Precip::Sleet, 1, false),
        13 => p(Precip::Sleet, 2, false),
        14 => p(Precip::Snow, 0, false),
        15 => p(Precip::Snow, 1, false),
        16 => p(Precip::Snow, 2, false),
        17 | 24 => p(Precip::Hail, 1, true),
        25 => p(Precip::Hail, 2, true),
        18 => p(Precip::Rain, 1, true),
        19 => p(Precip::Rain, 2, true),
        20 => p(Precip::Sleet, 1, true),
        21 => p(Precip::Sleet, 2, true),
        22 => p(Precip::Snow, 1, true),
        23 => p(Precip::Snow, 2, true),
        26 => Sky {
            thunder: true,
            cover: Some(0.8),
            ..Default::default()
        },
        27 => Sky {
            thunder: true,
            ..p(Precip::Rain, 1, true)
        },
        28 => Sky {
            thunder: true,
            ..p(Precip::Rain, 2, true)
        },
        29 => Sky {
            thunder: true,
            ..p(Precip::Hail, 1, true)
        },
        30 => Sky {
            thunder: true,
            ..p(Precip::Hail, 2, true)
        },
        _ => Sky::default(),
    }
}

/// A number with a decimal comma; "---" is missing.
fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() || s.starts_with("--") {
        return None;
    }
    s.replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Parses a POI report: a header of parameter names, two rows of units and
/// German names, then one row per hour (newest first, "dd.mm.yy;HH:MM"
/// UTC).
pub fn parse(body: &str, station: &StationPick) -> Result<Series, String> {
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    let head: Vec<&str> = lines
        .next()
        .ok_or("DWD: empty file")?
        .split(';')
        .map(str::trim)
        .collect();
    let col = |name: &str| head.iter().position(|h| *h == name);
    let c = (
        col("cloud_cover_total"),
        col("dry_bulb_temperature_at_2_meter_above_ground"),
        col("dew_point_temperature_at_2_meter_above_ground"),
        col("relative_humidity"),
        col("pressure_reduced_to_mean_sea_level"),
        col("height_of_base_of_lowest_cloud_above_station"),
        col("horizontal_visibility"),
        col("present_weather"),
    );
    let w = (
        col("mean_wind_speed_during last_10_min_at_10_meters_above_ground"),
        col("maximum_wind_speed_last_hour"),
        col("mean_wind_direction_during_last_10 min_at_10_meters_above_ground"),
        col("precipitation_amount_last_hour"),
    );
    if c.1.is_none() {
        return Err(format!("DWD {}: not a POI report", station.id));
    }
    let kmh = |v: f32| v / 3.6;
    let mut obs = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(';').collect();
        let (Some(d), Some(t)) = (f.first(), f.get(1)) else {
            continue;
        };
        let Some(at_ms) = super::time::parse_dotted(d, t) else {
            // The unit and German header rows.
            continue;
        };
        let get = |i: Option<usize>| number(f.get(i?)?);
        obs.push(Obs {
            at_ms,
            cloud_cover: get(c.0).map(|p| (p / 100.0).clamp(0.0, 1.0)),
            temperature_c: get(c.1),
            dew_point_c: get(c.2),
            humidity: get(c.3).map(|h| (h / 100.0).clamp(0.0, 1.0)),
            pressure_msl_hpa: get(c.4),
            cloud_base_m: get(c.5),
            visibility_m: get(c.6).map(|km| km * 1000.0),
            sky: get(c.7).map(|v| condition_sky(v as i64)),
            wind_mps: get(w.0).map(kmh),
            gust_mps: get(w.1).map(kmh),
            wind_from_deg: get(w.2),
            precip_mm_h: get(w.3).map(|p| p.max(0.0)),
            ..Default::default()
        });
    }
    if obs.is_empty() {
        return Err(format!("DWD {}: no observations", station.id));
    }
    obs.sort_by_key(|o| o.at_ms);
    Ok(Series {
        elevation_m: Some(station.height_m),
        station: Some(format!("{} ({})", station.name, station.id)),
        measured: true,
        obs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_and_numbers() {
        assert_eq!(condition_sky(1).cover, Some(0.0));
        assert!(condition_sky(5).fog);
        assert_eq!(condition_sky(16).precip, Some(Precip::Snow));
        assert!(condition_sky(28).thunder && condition_sky(28).strength == 2);
        assert_eq!(condition_sky(30).precip, Some(Precip::Hail));
        assert_eq!(number("-10,4"), Some(-10.4));
        assert_eq!(number("---"), None);
        assert_eq!(number(""), None);
    }
}
