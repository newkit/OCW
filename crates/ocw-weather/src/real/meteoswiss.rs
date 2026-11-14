//! MeteoSwiss open data (spec 9F, Q6: regional, Switzerland and
//! Liechtenstein): the 10-minute measurements of the SwissMetNet station
//! nearest to the place (`ogd-smn_<abbr>_t_now.csv` on data.geo.admin.ch:
//! today's values, UTC). CC BY 4.0, "Source: MeteoSwiss". The stations
//! measure temperature, humidity, dew point, pressure, wind, gusts and
//! precipitation; clouds and visibility come from the global forecast.
//!
//! The numerical forecasts (ICON-CH1/2, GRIB2) and the local point
//! forecasts are open data too; reading them is left for later (the
//! global forecast covers the next hours).

use super::{Obs, Series, StationPick};

/// The request for `station` (today's 10-minute values).
pub fn url(station: &StationPick) -> String {
    let a = station.id.to_ascii_lowercase();
    format!("https://data.geo.admin.ch/ch.meteoschweiz.ogd-smn/{a}/ogd-smn_{a}_t_now.csv")
}

/// Parses `ogd-smn_<abbr>_t_now.csv` (`;`-separated, a header of
/// parameter codes, "dd.mm.yyyy HH:MM" in UTC).
pub fn parse(body: &str, station: &StationPick) -> Result<Series, String> {
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    let head: Vec<&str> = lines
        .next()
        .ok_or("MeteoSwiss: empty file")?
        .split(';')
        .map(str::trim)
        .collect();
    let col = |name: &str| head.iter().position(|h| *h == name);
    let time = col("reference_timestamp").ok_or("MeteoSwiss: no reference_timestamp")?;
    let (t, rh, td, qfe) = (
        col("tre200s0"),
        col("ure200s0"),
        col("tde200s0"),
        col("prestas0"),
    );
    let (ff, fx, dd, rr) = (
        col("fkl010z0"),
        col("fkl010z1"),
        col("dkl010z0"),
        col("rre150z0"),
    );
    let mut obs = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(';').map(str::trim).collect();
        let get = |c: Option<usize>| -> Option<f32> {
            let v = *f.get(c?)?;
            v.parse::<f32>().ok().filter(|x| x.is_finite())
        };
        let Some((date, hm)) = f.get(time).and_then(|s| s.split_once(' ')) else {
            continue;
        };
        let Some(at_ms) = super::time::parse_dotted(date, hm) else {
            continue;
        };
        obs.push(Obs {
            at_ms,
            temperature_c: get(t),
            dew_point_c: get(td),
            humidity: get(rh).map(|h| (h / 100.0).clamp(0.0, 1.0)),
            pressure_hpa: get(qfe),
            // Ten minutes' sum → per hour.
            precip_mm_h: get(rr).map(|p| p.max(0.0) * 6.0),
            wind_mps: get(ff),
            gust_mps: get(fx),
            wind_from_deg: get(dd),
            ..Default::default()
        });
    }
    if obs.is_empty() {
        return Err(format!("MeteoSwiss {}: no measurements", station.id));
    }
    obs.sort_by_key(|o| o.at_ms);
    // The precipitation of the last half hour (three values): a single
    // ten-minute shower is not the weather of the hour.
    let n = obs.len();
    let recent: Vec<f32> = obs[n.saturating_sub(3)..]
        .iter()
        .filter_map(|o| o.precip_mm_h)
        .collect();
    if !recent.is_empty() {
        let mean = recent.iter().sum::<f32>() / recent.len() as f32;
        obs[n - 1].precip_mm_h = Some(mean);
    }
    Ok(Series {
        elevation_m: Some(station.height_m),
        station: Some(format!("{} ({})", station.name, station.id)),
        measured: true,
        obs,
    })
}
