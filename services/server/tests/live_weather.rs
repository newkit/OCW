//! Live smoke test of the real-weather providers (spec §14 Phase 9, S7):
//! one real request per provider through the server's HTTP fetcher, parsed
//! and mapped. Off by default (tests never touch the network); run it with
//!
//! ```sh
//! OCW_LIVE_WEATHER=1 cargo test -p ocw-server --test live_weather -- --nocapture
//! ```

use std::time::Duration;

use ocw_server::realweather::http::fetch_blocking;
use ocw_weather::real::feed::{Fetch, Kind, RealFeed};
use ocw_weather::real::{Cell, Choice, Place, Provider, normalize, to_state};

fn live() -> bool {
    std::env::var("OCW_LIVE_WEATHER").is_ok_and(|v| v == "1")
}

fn now_ms() -> i64 {
    ocw_server::unix_now_ms()
}

const VADUZ: Place = Place {
    lat: 47.141,
    lon: 9.521,
    alt_m: 455.0,
};

const FELDBERG: Place = Place {
    lat: 47.875,
    lon: 8.004,
    alt_m: 1400.0,
};

/// The feed's requests for `place` with `choice` at moment `at`.
fn plan(choice: Choice, place: Place, at: i64) -> Vec<Fetch> {
    let mut feed = RealFeed::new(choice);
    feed.set_place(place);
    let p = feed.plan(at, now_ms()).expect("a plan");
    p.regional.into_iter().chain(p.globals).collect()
}

fn check(f: &Fetch, place: Place) -> String {
    let body = fetch_blocking(f, Duration::from_secs(40))
        .unwrap_or_else(|e| panic!("{} {}: {e}", f.provider.name(), f.url));
    let series = f.parse(&body).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n{}",
            f.provider.name(),
            &body[..body.len().min(400)]
        )
    });
    assert!(!series.obs.is_empty());
    let at = if f.kind == Kind::Historic {
        series.obs[series.obs.len() / 4].at_ms
    } else {
        now_ms()
    };
    let (o, _) = series
        .at_or_edge(at)
        .or_else(|| series.obs.last().map(|o| (*o, true)))
        .expect("data");
    let w = to_state(&normalize(&o, series.elevation_m, place.alt_m));
    assert!((-45.0..45.0).contains(&w.temperature_c), "{w:?}");
    assert!((500.0..1080.0).contains(&w.pressure_hpa), "{w:?}");
    let line = format!(
        "{:<11} {:<10} {} obs, {} – {}{}",
        f.provider.name(),
        format!("{:?}", f.kind),
        series.obs.len(),
        ocw_weather::real::time::utc_label(o.at_ms),
        w.summary(),
        series
            .station
            .map(|s| format!(" [{s}]"))
            .unwrap_or_default()
    );
    println!("{line}");
    line
}

#[test]
fn every_provider_answers_and_parses() {
    if !live() {
        eprintln!("skipped: set OCW_LIVE_WEATHER=1 for the live weather smoke test");
        return;
    }
    let now = now_ms();
    let mut lines = Vec::new();
    // MeteoSwiss (Vaduz), MET Norway and Open-Meteo (its forecast).
    for f in plan(Choice::Automatic, VADUZ, now) {
        assert!(matches!(
            f.provider,
            Provider::MeteoSwiss | Provider::MetNorway | Provider::OpenMeteo
        ));
        lines.push(check(&f, VADUZ));
    }
    // DWD (the Feldberg station).
    let dwd = plan(Choice::Only(Provider::Dwd), FELDBERG, now);
    assert_eq!(dwd[0].provider, Provider::Dwd);
    lines.push(check(&dwd[0], FELDBERG));
    // Open-Meteo's history (ERA5): Storm Lothar on the Feldberg.
    let lothar = ocw_weather::real::time::utc_ms(1999, 12, 26, 11, 0, 0).unwrap();
    let hist = plan(Choice::Automatic, FELDBERG, lothar);
    assert_eq!(hist[0].kind, Kind::Historic);
    lines.push(check(&hist[0], FELDBERG));
    // The cache: the same request again comes without a request.
    let again = std::time::Instant::now();
    check(&hist[0], FELDBERG);
    assert!(again.elapsed() < Duration::from_millis(500), "cached");
    let cell = Cell::of(&VADUZ);
    println!("cell of Vaduz: {:?} -> {:?}", cell, cell.centre());
    assert_eq!(lines.len(), 5);
}
