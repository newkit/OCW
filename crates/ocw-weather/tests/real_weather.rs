//! Real weather (spec §14 Phase 9, S7) against recorded provider answers
//! (`tests/data/`, see `manifest.toml` there for their sources and
//! licences): the parsers, the mapping, the feed's choice of providers,
//! cache, fallback and offline behaviour. No network.

use ocw_weather::real::feed::{Kind, RealFeed, Status};
use ocw_weather::real::time::{HOUR_MS, days_from_civil, parse_iso, utc_ms};
use ocw_weather::real::{
    Cell, Choice, Place, Provider, dwd, meteoswiss, metno, nearest_station, normalize, openmeteo,
    stations, to_state,
};
use ocw_weather::{CloudType, Precip};

const OM_FORECAST: &str = include_str!("data/openmeteo-forecast-vaduz.json");
const OM_LOTHAR: &str = include_str!("data/openmeteo-archive-lothar-feldberg.json");
const METNO: &str = include_str!("data/metno-complete-vaduz.json");
const SMN_VAD: &str = include_str!("data/meteoswiss-vad-t-now.csv");
const DWD_FELDBERG: &str = include_str!("data/dwd-10908-beob.csv");

/// When the samples were recorded: 14 Nov 2026 11:40 UTC.
fn recorded() -> i64 {
    utc_ms(2026, 11, 14, 11, 40, 0).unwrap()
}

fn vaduz() -> Place {
    Place {
        lat: 47.141,
        lon: 9.521,
        alt_m: 455.0,
    }
}

fn feldberg() -> Place {
    Place {
        lat: 47.875,
        lon: 8.004,
        alt_m: 1400.0,
    }
}

#[test]
fn open_meteo_forecast_parses() {
    let s = openmeteo::parse(OM_FORECAST).unwrap();
    assert!(!s.measured);
    assert_eq!(s.elevation_m, Some(400.0));
    // Seven days back, four ahead, hourly.
    assert_eq!(s.obs.len(), 11 * 24);
    let (first, last) = s.span().unwrap();
    assert_eq!(last - first, (11 * 24 - 1) * HOUR_MS);
    let o = s.at(parse_iso("2026-11-14T11:00:00Z").unwrap()).unwrap();
    assert_eq!(o.temperature_c, Some(5.8));
    assert_eq!(o.cloud_cover, Some(0.0));
    assert_eq!(o.visibility_m, Some(34_420.0));
    assert_eq!(o.pressure_hpa, Some(971.8));
    assert!(o.wind_mps.unwrap() < 1.0);
    let w = to_state(&normalize(&o, s.elevation_m, 455.0));
    assert_eq!(w.precip, Precip::None);
    assert!(w.cloud_cover < 0.05);
    assert!((w.temperature_c - (5.8 - 0.0065 * 55.0)).abs() < 0.01);
    // Every hour maps to a valid state.
    for o in &s.obs {
        let w = to_state(o);
        assert_eq!(w, w.clamped());
    }
}

/// Storm Lothar, 26 Dec 1999, at the Feldberg (ERA5 via Open-Meteo): the
/// famous stormy day of "ride a day".
#[test]
fn era5_history_of_storm_lothar() {
    let s = openmeteo::parse(OM_LOTHAR).unwrap();
    assert_eq!(s.obs.len(), 48);
    assert_eq!(s.elevation_m, Some(1400.0));
    let at = utc_ms(1999, 12, 26, 11, 0, 0).unwrap();
    let w = to_state(&normalize(&s.at(at).unwrap(), s.elevation_m, 1400.0));
    assert!(w.gust_mps > 35.0, "hurricane-force gusts: {}", w.gust_mps);
    assert!(w.wind_mps > 12.0);
    assert_eq!(w.precip, Precip::Rain);
    assert!(w.precip_mm_h > 2.0);
    assert!(w.cloud_cover > 0.6);
    // The air pressure on the summit, from the surface pressure.
    assert!((w.pressure_hpa - 833.4).abs() < 1.0);
    // Half an hour later: between the hours.
    let half = s.at(at + HOUR_MS / 2).unwrap();
    let g = half.gust_mps.unwrap();
    assert!((40.4..=40.6).contains(&g), "{g}");
}

#[test]
fn met_norway_complete_parses() {
    let s = metno::parse(METNO).unwrap();
    assert_eq!(s.obs.len(), 33);
    assert_eq!(s.elevation_m, Some(400.0));
    let o = s.obs[0];
    assert_eq!(o.at_ms, parse_iso("2026-11-14T11:00:00Z").unwrap());
    assert!(o.temperature_c.is_some() && o.pressure_msl_hpa.is_some());
    assert!(o.cloud_low.is_some() && o.fog.is_some());
    assert_eq!(o.precip_mm_h, Some(0.0));
    assert!(o.sky.is_some());
    // The six-hourly steps at the end: precipitation per hour.
    let last = s.obs.last().unwrap();
    assert!(last.at_ms - s.obs[0].at_ms > 5 * 24 * HOUR_MS);
    let w = to_state(&normalize(&o, s.elevation_m, 455.0));
    assert_eq!(w, w.clamped());
    assert!(
        w.pressure_hpa < 980.0,
        "reduced to the height: {}",
        w.pressure_hpa
    );
}

#[test]
fn a_met_norway_thunderstorm_and_snow() {
    let body = r#"{"geometry":{"type":"Point","coordinates":[9.55,47.15,400]},
      "properties":{"timeseries":[
       {"time":"2026-07-01T15:00:00Z","data":{"instant":{"details":{"air_pressure_at_sea_level":1008.0,"air_temperature":24.0,"cloud_area_fraction":95.0,"cloud_area_fraction_low":60.0,"dew_point_temperature":17.0,"relative_humidity":65.0,"wind_from_direction":250.0,"wind_speed":6.0,"wind_speed_of_gust":19.0}},
        "next_1_hours":{"summary":{"symbol_code":"heavyrainshowersandthunder_day"},"details":{"precipitation_amount":14.2,"probability_of_thunder":80.0}}}},
       {"time":"2026-07-01T16:00:00Z","data":{"instant":{"details":{"air_temperature":-2.0,"cloud_area_fraction":100.0}},
        "next_6_hours":{"summary":{"symbol_code":"snow"},"details":{"precipitation_amount":6.0}}}}]}}"#;
    let s = metno::parse(body).unwrap();
    let storm = to_state(&normalize(&s.obs[0], s.elevation_m, 400.0));
    assert_eq!(storm.cloud_type, CloudType::Cumulonimbus);
    assert_eq!(storm.precip, Precip::Rain);
    assert!(storm.lightning_per_min > 0.0 && storm.gust_mps == 19.0);
    let snow = to_state(&s.obs[1]);
    assert_eq!(snow.precip, Precip::Snow);
    assert!((snow.precip_mm_h - 1.0).abs() < 1e-4, "6 mm in 6 h");
}

#[test]
fn meteoswiss_vaduz_measurements_parse() {
    let st = nearest_station(stations::SWISS, (47.15, 9.55), 400.0).unwrap();
    assert_eq!(st.id, "VAD");
    assert_eq!(
        meteoswiss::url(&st),
        "https://data.geo.admin.ch/ch.meteoschweiz.ogd-smn/vad/ogd-smn_vad_t_now.csv"
    );
    let s = meteoswiss::parse(SMN_VAD, &st).unwrap();
    assert!(s.measured);
    assert_eq!(s.station.as_deref(), Some("Vaduz (VAD)"));
    assert_eq!(s.elevation_m, Some(457.0));
    let last = s.obs.last().unwrap();
    assert_eq!(last.at_ms, utc_ms(2026, 11, 14, 11, 40, 0).unwrap());
    assert_eq!(last.temperature_c, Some(7.3));
    assert_eq!(last.pressure_hpa, Some(964.4));
    assert_eq!(last.wind_from_deg, Some(295.0));
    assert!((last.humidity.unwrap() - 0.528).abs() < 1e-4);
    assert_eq!(last.precip_mm_h, Some(0.0));
    // Ten-minute steps since midnight UTC.
    assert!(s.obs.len() > 60);
    assert_eq!(s.obs[1].at_ms - s.obs[0].at_ms, 10 * 60_000);
    assert!(meteoswiss::parse("station_abbr;x\n", &st).is_err());
}

#[test]
fn dwd_feldberg_observations_parse() {
    let st = nearest_station(stations::GERMAN, (47.85, 8.05), 1400.0).unwrap();
    assert_eq!((st.id, st.height_m), ("10908", 1486.0));
    assert_eq!(
        dwd::url(&st),
        "https://opendata.dwd.de/weather/weather_reports/poi/10908-BEOB.csv"
    );
    let s = dwd::parse(DWD_FELDBERG, &st).unwrap();
    assert!(s.measured);
    let o = s.obs.last().unwrap();
    assert_eq!(o.at_ms % HOUR_MS, 0, "hourly");
    assert!(o.at_ms <= recorded() && recorded() - o.at_ms < 3 * HOUR_MS);
    // 11:00 UTC: -0.1 °C, mean wind 33 km/h, gusts 49 km/h; a summit station
    // reports no sea-level pressure ("---").
    assert_eq!(o.temperature_c, Some(-0.1));
    assert!(o.pressure_msl_hpa.is_none());
    assert!((o.gust_mps.unwrap() - 49.0 / 3.6).abs() < 1e-4);
    assert!(o.visibility_m.unwrap() > 10_000.0, "km → m");
    assert!(o.wind_mps.unwrap() < 30.0, "km/h → m/s");
    assert!(o.cloud_cover.is_some() && o.sky.is_some());
    // Sorted oldest first.
    assert!(s.obs.windows(2).all(|w| w[0].at_ms < w[1].at_ms));
    let w = to_state(&normalize(o, s.elevation_m, 1400.0));
    assert_eq!(w, w.clamped());
    assert!(dwd::parse("a;b\n1;2\n", &st).is_err());
}

/// Answers every pending request of `feed` from the recorded samples.
fn answer(feed: &mut RealFeed, at: i64, now: i64) -> Vec<(Provider, Kind)> {
    let mut asked = Vec::new();
    for f in feed.wants(at, now) {
        asked.push((f.provider, f.kind));
        let body = match (f.provider, f.kind) {
            (Provider::OpenMeteo, Kind::Historic) => OM_LOTHAR,
            (Provider::OpenMeteo, _) => OM_FORECAST,
            (Provider::MetNorway, _) => METNO,
            (Provider::MeteoSwiss, _) => SMN_VAD,
            (Provider::Dwd, _) => DWD_FELDBERG,
        };
        feed.on_result(&f.url, Ok(body.into()), now);
    }
    asked
}

/// Automatic in Vaduz: MeteoSwiss's station now plus MET Norway's
/// forecast; asked once per cell until the answers are old.
#[test]
fn automatic_in_vaduz_merges_meteoswiss_and_met_norway_and_caches() {
    let now = recorded();
    let mut feed = RealFeed::new(Choice::Automatic);
    assert!(feed.wants(now, now).is_empty(), "no place yet");
    assert!(feed.resolve(now, now).is_none());
    feed.set_place(vaduz());
    assert_eq!(feed.status(now, now), Status::Waiting);
    let asked = answer(&mut feed, now, now);
    assert_eq!(
        asked,
        [
            (Provider::MeteoSwiss, Kind::Measured),
            (Provider::MetNorway, Kind::Forecast)
        ]
    );
    let r = feed.resolve(now, now).unwrap();
    assert_eq!(r.providers, [Provider::MeteoSwiss, Provider::MetNorway]);
    assert_eq!(r.attribution, "Weather: MeteoSwiss, MET Norway, CC BY 4.0");
    assert_eq!(r.station.as_deref(), Some("Vaduz (VAD)"));
    assert_eq!(r.status, Status::Live);
    assert!(r.note.is_none(), "{:?}", r.note);
    // The station's temperature (7.3 °C at 457 m) at the rider's 455 m.
    assert!(
        (r.state.temperature_c - 7.3).abs() < 0.05,
        "{}",
        r.state.temperature_c
    );
    assert_eq!(r.state.wind_from_deg, 295.0);
    // The same cell a minute later: nothing to ask (cached).
    assert!(feed.wants(now + 60_000, now + 60_000).is_empty());
    // A rider 300 m away: the same cell, still nothing.
    feed.set_place(Place {
        lat: 47.143,
        ..vaduz()
    });
    assert!(feed.wants(now + 120_000, now + 120_000).is_empty());
    // After 10 minutes the station again, after 30 the forecast.
    let later = now + 11 * 60_000;
    assert_eq!(
        answer(&mut feed, later, later),
        [(Provider::MeteoSwiss, Kind::Measured)]
    );
    let later = now + 31 * 60_000;
    let asked = answer(&mut feed, later, later);
    assert!(asked.contains(&(Provider::MetNorway, Kind::Forecast)));
    // Tomorrow afternoon: the forecast only (no station for the future).
    let tomorrow = now + 27 * HOUR_MS;
    let r = feed.resolve(tomorrow, later).unwrap();
    assert_eq!(r.providers, [Provider::MetNorway]);
    assert!(r.station.is_none());
}

#[test]
fn a_fixed_provider_and_one_that_does_not_cover_the_place() {
    let now = recorded();
    // Open-Meteo in Vaduz.
    let mut feed = RealFeed::new(Choice::Only(Provider::OpenMeteo));
    feed.set_place(vaduz());
    assert_eq!(
        answer(&mut feed, now, now),
        [(Provider::OpenMeteo, Kind::Forecast)]
    );
    let r = feed.resolve(now, now).unwrap();
    assert_eq!(r.providers, [Provider::OpenMeteo]);
    assert_eq!(r.attribution, "Weather: Open-Meteo, CC BY 4.0");
    // DWD in Liechtenstein: not covered, Automatic with a note.
    let mut feed = RealFeed::new(Choice::Only(Provider::Dwd));
    feed.set_place(vaduz());
    let asked = answer(&mut feed, now, now);
    assert_eq!(asked[0], (Provider::MeteoSwiss, Kind::Measured));
    let r = feed.resolve(now, now).unwrap();
    assert!(
        r.note
            .as_deref()
            .unwrap()
            .contains("DWD does not cover this place: Automatic"),
        "{:?}",
        r.note
    );
    // DWD in the Black Forest: its station.
    let mut feed = RealFeed::new(Choice::Only(Provider::Dwd));
    feed.set_place(feldberg());
    let asked = answer(&mut feed, now, now);
    assert_eq!(asked[0], (Provider::Dwd, Kind::Measured));
    let r = feed.resolve(now, now).unwrap();
    assert_eq!(r.station.as_deref(), Some("Feldberg/S. (10908)"));
    assert_eq!(r.providers[0], Provider::Dwd);
    // Tenerife, Automatic: no regional network, MET Norway alone.
    let mut feed = RealFeed::new(Choice::Automatic);
    feed.set_place(Place {
        lat: 28.27,
        lon: -16.64,
        alt_m: 2100.0,
    });
    assert_eq!(
        answer(&mut feed, now, now),
        [(Provider::MetNorway, Kind::Forecast)]
    );
}

/// MET Norway down: Open-Meteo takes over (with a note); retries wait.
#[test]
fn a_failing_provider_falls_back_and_waits_before_asking_again() {
    let now = recorded();
    let mut feed = RealFeed::new(Choice::Only(Provider::MetNorway));
    feed.set_place(feldberg());
    let f = feed.wants(now, now);
    assert_eq!(f.len(), 1);
    feed.on_result(&f[0].url, Err("MET Norway: HTTP 503".into()), now);
    assert!(feed.resolve(now, now).is_none());
    assert_eq!(
        feed.last_error(now, now).as_deref(),
        Some("MET Norway: HTTP 503")
    );
    // The fallback is asked at once; MET Norway not again for a minute.
    let f = feed.wants(now + 1000, now + 1000);
    assert_eq!(
        f.iter().map(|f| f.provider).collect::<Vec<_>>(),
        [Provider::OpenMeteo]
    );
    feed.on_result(&f[0].url, Ok(OM_FORECAST.into()), now + 1000);
    let r = feed.resolve(now, now + 1000).unwrap();
    assert_eq!(r.providers, [Provider::OpenMeteo]);
    assert!(
        r.note
            .unwrap()
            .contains("MET Norway unavailable: Open-Meteo")
    );
    // A minute later MET Norway is tried again; a body that does not parse
    // counts as a failure, the wait doubles.
    let f = feed.wants(now + 61_000, now + 61_000);
    assert_eq!(f[0].provider, Provider::MetNorway);
    feed.on_result(
        &f[0].url,
        Ok("<html>maintenance</html>".into()),
        now + 61_000,
    );
    assert!(
        feed.wants(now + 122_000, now + 122_000).is_empty(),
        "2 min now"
    );
    assert_eq!(feed.wants(now + 182_000, now + 182_000).len(), 1);
    // An answer nobody asked for is ignored.
    feed.on_result("https://example.org/", Ok(METNO.into()), now);
}

/// Offline: the last data stays (stale, with a note); without any, the
/// status says offline (the server simulates meanwhile).
#[test]
fn offline_keeps_the_last_weather_or_says_so() {
    let now = recorded();
    let mut feed = RealFeed::new(Choice::Only(Provider::OpenMeteo));
    feed.set_place(vaduz());
    answer(&mut feed, now, now);
    // Two hours later the network is gone.
    let later = now + 2 * HOUR_MS;
    for f in feed.wants(later, later) {
        feed.on_result(&f.url, Err("offline".into()), later);
    }
    let r = feed.resolve(later, later).unwrap();
    assert_eq!(r.status, Status::Stale);
    assert!(
        r.note
            .unwrap()
            .contains("no new data: the weather of 11:40 UTC")
    );
    // Never any data.
    let mut feed = RealFeed::new(Choice::Automatic);
    feed.set_place(vaduz());
    for f in feed.wants(now, now) {
        feed.on_result(&f.url, Err("offline".into()), now);
    }
    // The fallback is asked next, and fails too.
    for f in feed.wants(now + 1, now + 1) {
        feed.on_result(&f.url, Err("offline".into()), now + 1);
    }
    assert!(feed.resolve(now, now + 1).is_none());
    assert_eq!(feed.status(now, now + 1), Status::Offline);
}

/// Ride a day: a moment long ago comes from ERA5 (one request per day,
/// cached), whatever the choice; far ahead the forecast's last hour.
#[test]
fn history_and_the_end_of_the_forecast() {
    let now = recorded();
    let lothar = utc_ms(1999, 12, 26, 11, 0, 0).unwrap();
    let mut feed = RealFeed::new(Choice::Only(Provider::MeteoSwiss));
    feed.set_place(feldberg());
    let plan = feed.plan(lothar, now).unwrap();
    assert!(plan.historic && plan.regional.is_none());
    assert!(plan.globals[0].url.contains("start_date=1999-12-26"));
    assert_eq!(
        Cell::of(&feldberg()).centre(),
        (47.85, 8.05),
        "the cell of the recorded sample"
    );
    assert_eq!(
        answer(&mut feed, lothar, now),
        [(Provider::OpenMeteo, Kind::Historic)]
    );
    let r = feed.resolve(lothar, now).unwrap();
    assert!(r.historic);
    assert_eq!(
        r.attribution,
        "Weather: Open-Meteo (ERA5, Copernicus), CC BY 4.0"
    );
    assert!(r.state.gust_mps > 35.0);
    assert!(r.note.unwrap().contains("ERA5"));
    // The next hour of the same day: no new request.
    assert!(feed.wants(lothar + HOUR_MS, now).is_empty());
    // Another day: another request.
    let next_week = lothar + 7 * 24 * HOUR_MS;
    assert_eq!(feed.wants(next_week, now).len(), 1);
    assert_eq!(
        days_from_civil(1999, 12, 26),
        ocw_weather::real::time::day_of(lothar)
    );
    // Ten days ahead: beyond the forecast, its last hour, with a note.
    let mut feed = RealFeed::new(Choice::Only(Provider::OpenMeteo));
    feed.set_place(vaduz());
    answer(&mut feed, now, now);
    let far = now + 10 * 24 * HOUR_MS;
    let r = feed.resolve(far, now).unwrap();
    assert!(r.note.unwrap().contains("beyond the forecast"));
    // Three days back: the forecast's past days (no history request).
    let plan = feed.plan(now - 3 * 24 * HOUR_MS, now).unwrap();
    assert!(!plan.historic);
}
