//! MET Norway Locationforecast 2.0 (spec 9F, Q6: the global fallback):
//! hourly point forecasts for the whole globe, `complete` (with the cloud
//! layers and the fog fraction). Licence CC BY 4.0 and NLOD 2.0 ("Data
//! from MET Norway"). The terms ask for an identifying User-Agent (the
//! server sends a generic product name, never personal data), at most four
//! decimals in the coordinates, caching and respect of `Expires`; the
//! server asks once per grid cell every 30 minutes at most.

use serde_json::Value;

use super::{Cell, Obs, Precip, Series, Sky};

/// The request for `cell` (the altitude band as `altitude`).
pub fn url(cell: &Cell) -> String {
    let (lat, lon) = cell.centre();
    format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/complete?lat={lat:.2}&lon={lon:.2}&altitude={:.0}",
        cell.alt_m()
    )
}

/// What a symbol code says ("lightrainshowers_day", "heavysnowandthunder",
/// "fog", "partlycloudy_night").
pub fn symbol_sky(code: &str) -> Sky {
    let base = code.split('_').next().unwrap_or(code);
    let strength = if base.starts_with("light") {
        0
    } else if base.starts_with("heavy") {
        2
    } else {
        1
    };
    let precip = if base.contains("sleet") {
        Some(Precip::Sleet)
    } else if base.contains("snow") {
        Some(Precip::Snow)
    } else if base.contains("rain") {
        Some(Precip::Rain)
    } else {
        None
    };
    let cover = match base {
        "clearsky" => Some(0.0),
        "fair" => Some(0.2),
        "partlycloudy" => Some(0.5),
        "cloudy" | "fog" => Some(1.0),
        _ => None,
    };
    Sky {
        cover,
        precip,
        strength,
        showers: base.contains("showers"),
        thunder: base.contains("thunder"),
        fog: base == "fog",
    }
}

fn num(details: &Value, key: &str) -> Option<f32> {
    details.get(key)?.as_f64().map(|v| v as f32)
}

/// Parses a `complete` (or `compact`) response. The precipitation of an
/// entry is the next hour's (or the next six hours' when the hourly steps
/// end), per hour.
pub fn parse(body: &str) -> Result<Series, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("MET Norway: not JSON: {e}"))?;
    let ts = v
        .pointer("/properties/timeseries")
        .and_then(Value::as_array)
        .ok_or("MET Norway: no timeseries")?;
    let mut obs = Vec::with_capacity(ts.len());
    for e in ts {
        let Some(at_ms) = e
            .get("time")
            .and_then(Value::as_str)
            .and_then(super::time::parse_iso)
        else {
            continue;
        };
        let Some(d) = e.pointer("/data/instant/details") else {
            continue;
        };
        let pct = |k| num(d, k).map(|p| (p / 100.0).clamp(0.0, 1.0));
        let (next, hours) = match e.pointer("/data/next_1_hours") {
            Some(n) => (Some(n), 1.0),
            None => (e.pointer("/data/next_6_hours"), 6.0),
        };
        let precip = next
            .and_then(|n| n.pointer("/details/precipitation_amount"))
            .and_then(Value::as_f64)
            .map(|p| (p as f32 / hours).max(0.0));
        let mut sky = next
            .and_then(|n| n.pointer("/summary/symbol_code"))
            .and_then(Value::as_str)
            .map(symbol_sky);
        if let (Some(s), Some(p)) = (
            sky.as_mut(),
            next.and_then(|n| n.pointer("/details/probability_of_thunder"))
                .and_then(Value::as_f64),
        ) {
            s.thunder |= p >= 50.0;
        }
        obs.push(Obs {
            at_ms,
            temperature_c: num(d, "air_temperature"),
            dew_point_c: num(d, "dew_point_temperature"),
            humidity: pct("relative_humidity"),
            pressure_msl_hpa: num(d, "air_pressure_at_sea_level"),
            cloud_cover: pct("cloud_area_fraction"),
            cloud_low: pct("cloud_area_fraction_low"),
            cloud_mid: pct("cloud_area_fraction_medium"),
            cloud_high: pct("cloud_area_fraction_high"),
            fog: pct("fog_area_fraction"),
            precip_mm_h: precip,
            wind_mps: num(d, "wind_speed"),
            gust_mps: num(d, "wind_speed_of_gust"),
            wind_from_deg: num(d, "wind_from_direction"),
            sky,
            ..Default::default()
        });
    }
    if obs.is_empty() {
        return Err("MET Norway: an empty timeseries".into());
    }
    obs.sort_by_key(|o| o.at_ms);
    Ok(Series {
        // The forecast is for the altitude asked for.
        elevation_m: v
            .pointer("/geometry/coordinates/2")
            .and_then(Value::as_f64)
            .map(|e| e as f32),
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
    fn the_request_has_four_decimals_at_most() {
        let u = url(&Cell::of(&Place {
            lat: 47.14123,
            lon: 9.52156,
            alt_m: 455.0,
        }));
        assert_eq!(
            u,
            "https://api.met.no/weatherapi/locationforecast/2.0/complete?lat=47.15&lon=9.55&altitude=400"
        );
    }

    #[test]
    fn symbols_say_what_falls() {
        let s = symbol_sky("heavyrainshowersandthunder_day");
        assert_eq!(s.precip, Some(Precip::Rain));
        assert!(s.thunder && s.showers && s.strength == 2);
        assert_eq!(symbol_sky("lightsnow").precip, Some(Precip::Snow));
        assert_eq!(symbol_sky("lightsnow").strength, 0);
        assert_eq!(symbol_sky("sleetshowers_night").precip, Some(Precip::Sleet));
        assert!(symbol_sky("fog").fog);
        assert_eq!(symbol_sky("clearsky_day").cover, Some(0.0));
        assert_eq!(symbol_sky("cloudy").precip, None);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"properties":{"timeseries":[]}}"#).is_err());
    }
}
