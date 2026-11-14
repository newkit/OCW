//! Open-Meteo (spec 9F, Q6, Q8): the forecast API (global, hourly, the
//! past week and the next days) and the historical API (ERA5 reanalysis,
//! hourly since 1940). Data CC BY 4.0 ("Weather data by Open-Meteo.com");
//! the free API is for non-commercial use (< 10 000 calls a day), which
//! fits OCW (no commercial release, Phase 8 Q2). ERA5: "Contains modified
//! Copernicus Climate Change Service information".
//!
//! Requests ask for SI units and Unix times (`wind_speed_unit=ms`,
//! `timeformat=unixtime`), at the cell's centre and the altitude band
//! (`elevation=`: Open-Meteo moves the temperature to that height).

use serde_json::Value;

use super::{Cell, Obs, Series, wmo_sky};

/// The hourly fields of the forecast.
pub const FORECAST_FIELDS: &str = "temperature_2m,relative_humidity_2m,dew_point_2m,surface_pressure,cloud_cover,cloud_cover_low,cloud_cover_mid,cloud_cover_high,precipitation,rain,showers,snowfall,weather_code,visibility,wind_speed_10m,wind_gusts_10m,wind_direction_10m";
/// The hourly fields ERA5 has (no visibility, no showers).
pub const ARCHIVE_FIELDS: &str = "temperature_2m,relative_humidity_2m,dew_point_2m,surface_pressure,cloud_cover,cloud_cover_low,cloud_cover_mid,cloud_cover_high,precipitation,rain,snowfall,weather_code,wind_speed_10m,wind_gusts_10m,wind_direction_10m";

/// Days of the past the forecast request covers (older days come from
/// the historical API, which is about five days behind).
pub const PAST_DAYS: u32 = 7;
/// Days ahead.
pub const FORECAST_DAYS: u32 = 4;

/// The forecast request for `cell`.
pub fn forecast_url(cell: &Cell) -> String {
    let (lat, lon) = cell.centre();
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.2}&longitude={lon:.2}&elevation={:.0}&hourly={FORECAST_FIELDS}&wind_speed_unit=ms&timeformat=unixtime&past_days={PAST_DAYS}&forecast_days={FORECAST_DAYS}",
        cell.alt_m()
    )
}

/// The historical (ERA5) request for `cell` on UTC day `day` (days since
/// 1970) and the next (so the last hours interpolate across midnight).
pub fn archive_url(cell: &Cell, day: i64) -> String {
    let (lat, lon) = cell.centre();
    format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.2}&longitude={lon:.2}&elevation={:.0}&start_date={}&end_date={}&hourly={ARCHIVE_FIELDS}&wind_speed_unit=ms&timeformat=unixtime",
        cell.alt_m(),
        super::time::date_label(day),
        super::time::date_label(day + 1),
    )
}

fn column<'a>(hourly: &'a Value, name: &str) -> Option<&'a Vec<Value>> {
    hourly.get(name)?.as_array()
}

fn val(col: Option<&Vec<Value>>, i: usize) -> Option<f32> {
    col?.get(i)?.as_f64().map(|v| v as f32)
}

/// Parses a forecast or historical response (hourly, Unix times, SI
/// units). Precipitation is the hour's sum (mm), snowfall in cm (7 cm of
/// snow ≈ 1 mm of water... Open-Meteo counts 0.7 cm per mm).
pub fn parse(body: &str) -> Result<Series, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("Open-Meteo: not JSON: {e}"))?;
    if v.get("error").and_then(Value::as_bool) == Some(true) {
        return Err(format!(
            "Open-Meteo: {}",
            v.get("reason").and_then(Value::as_str).unwrap_or("error")
        ));
    }
    let hourly = v.get("hourly").ok_or("Open-Meteo: no hourly data")?;
    let times = column(hourly, "time").ok_or("Open-Meteo: no times")?;
    let c = |n| column(hourly, n);
    let (t2, rh, td, sp) = (
        c("temperature_2m"),
        c("relative_humidity_2m"),
        c("dew_point_2m"),
        c("surface_pressure"),
    );
    let (cc, cl, cm, ch) = (
        c("cloud_cover"),
        c("cloud_cover_low"),
        c("cloud_cover_mid"),
        c("cloud_cover_high"),
    );
    let (pr, sf, wc, vis) = (
        c("precipitation"),
        c("snowfall"),
        c("weather_code"),
        c("visibility"),
    );
    let (ws, wg, wd) = (
        c("wind_speed_10m"),
        c("wind_gusts_10m"),
        c("wind_direction_10m"),
    );
    let pct = |col, i| val(col, i).map(|p| (p / 100.0).clamp(0.0, 1.0));
    let mut obs = Vec::with_capacity(times.len());
    for (i, t) in times.iter().enumerate() {
        let Some(secs) = t.as_i64() else { continue };
        let precip = val(pr, i).map(|p| p.max(0.0));
        let snow_water = val(sf, i).map(|s| s.max(0.0) / 0.7);
        obs.push(Obs {
            at_ms: secs * 1000,
            temperature_c: val(t2, i),
            dew_point_c: val(td, i),
            humidity: pct(rh, i),
            pressure_hpa: val(sp, i),
            cloud_cover: pct(cc, i),
            cloud_low: pct(cl, i),
            cloud_mid: pct(cm, i),
            cloud_high: pct(ch, i),
            precip_mm_h: precip,
            snow_share: match (precip, snow_water) {
                (Some(p), Some(s)) if p > 0.0 => Some((s / p).clamp(0.0, 1.0)),
                _ => None,
            },
            visibility_m: val(vis, i),
            wind_mps: val(ws, i),
            gust_mps: val(wg, i),
            wind_from_deg: val(wd, i),
            sky: wc
                .and_then(|w| w.get(i))
                .and_then(Value::as_i64)
                .map(wmo_sky),
            ..Default::default()
        });
    }
    if obs.is_empty() {
        return Err("Open-Meteo: an empty series".into());
    }
    obs.sort_by_key(|o| o.at_ms);
    Ok(Series {
        elevation_m: v.get("elevation").and_then(Value::as_f64).map(|e| e as f32),
        station: None,
        measured: false,
        obs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::real::Place;

    #[test]
    fn requests_name_the_cell_and_the_band() {
        let cell = Cell::of(&Place {
            lat: 47.88,
            lon: 8.0,
            alt_m: 1050.0,
        });
        let f = forecast_url(&cell);
        assert!(f.starts_with("https://api.open-meteo.com/v1/forecast?latitude=47.85&longitude=8.05&elevation=1000&"), "{f}");
        assert!(f.contains("wind_speed_unit=ms") && f.contains("timeformat=unixtime"));
        let day = crate::real::time::days_from_civil(1999, 12, 26);
        let a = archive_url(&cell, day);
        assert!(
            a.contains("start_date=1999-12-26&end_date=1999-12-27"),
            "{a}"
        );
        assert!(!a.contains("visibility"));
    }

    #[test]
    fn errors_and_garbage_are_errors() {
        assert!(parse("<html>").is_err());
        let e = parse(r#"{"error":true,"reason":"Latitude must be in range"}"#).unwrap_err();
        assert!(e.contains("Latitude"), "{e}");
        assert!(parse(r#"{"hourly":{"time":[]}}"#).is_err());
    }
}
