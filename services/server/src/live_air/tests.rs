//! Tests of the live aircraft (spec 18E T10): recorded adsb.lol answers
//! (our own fetches, ODbL, identities replaced: `fixtures/manifest.toml`) →
//! aircraft; the request budget over simulated days; the feed down → the
//! simulated sky within 5 s; no identities in the messages or the logs;
//! the extrapolation on the recorded sequences. No network.

use std::sync::{Arc, Mutex};

use ocw_protocol::live_air::aircraft_from_wire;
use ocw_sim::air::AirKind;
use ocw_sim::air::live::{LiveAircraft, distance_m};

use super::*;

const BF: [&str; 3] = [
    include_str!("fixtures/blackforest-1.json"),
    include_str!("fixtures/blackforest-2.json"),
    include_str!("fixtures/blackforest-3.json"),
];
const TF: [&str; 3] = [
    include_str!("fixtures/tenerife-1.json"),
    include_str!("fixtures/tenerife-2.json"),
    include_str!("fixtures/tenerife-3.json"),
];

/// What the fixtures hold that must never leave the server: the made-up
/// addresses, callsigns, registrations and the type codes.
fn identities() -> Vec<String> {
    let mut out = Vec::new();
    for body in BF.iter().chain(TF.iter()) {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        for a in v["ac"].as_array().unwrap() {
            for k in ["hex", "flight", "r", "t"] {
                if let Some(s) = a[k].as_str() {
                    out.push(s.trim().to_owned());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn feed_now_ms(body: &str) -> i64 {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    v["now"].as_i64().unwrap()
}

fn ok(body: &str) -> Answer {
    Answer {
        result: Ok((200, body.to_owned())),
        remaining: Some(450),
        reset_s: Some(40_000),
        ..Default::default()
    }
}

/// 2026-11-27 17:17:23 UTC, the first fixture's time.
const T0: i64 = 1_795_799_843_781;

#[test]
fn every_fixture_has_its_source_and_licence() {
    let m = include_str!("fixtures/manifest.toml");
    for w in ["blackforest", "tenerife"] {
        for i in 1..=3 {
            assert!(m.contains(&format!("path = \"{w}-{i}.json\"")), "{w}-{i}");
        }
    }
    assert_eq!(m.matches("licence = \"ODbL 1.0\"").count(), 6);
}

#[test]
fn recorded_answers_become_aircraft_without_identities() {
    let src = AdsbLol::default();
    let bf = src.parse(BF[0], 1).unwrap();
    let raw: serde_json::Value = serde_json::from_str(BF[0]).unwrap();
    assert_eq!(
        bf.len(),
        raw["ac"].as_array().unwrap().len(),
        "all airborne"
    );
    assert!(bf.iter().filter(|s| s.kind == AirKind::Airliner).count() >= 14);
    // The EC45 (category A7) is a helicopter.
    assert!(bf.iter().any(|s| s.kind == AirKind::Helicopter));
    for s in &bf {
        assert!((47.0..49.0).contains(&s.lat) && (6.9..9.1).contains(&s.lon));
        assert!(s.alt_m > 0.0 && s.alt_m < 14_000.0);
        assert!(s.speed_mps >= 0.0 && s.speed_mps < 320.0);
        assert!((0.0..360.0).contains(&s.track_deg));
    }
    let tf = src.parse(TF[0], 1).unwrap();
    assert!(!tf.is_empty());
    assert!(tf.iter().all(|s| (27.5..29.2).contains(&s.lat)));
    // The same aircraft has the same key in each answer, another salt
    // another key.
    let bf2 = src.parse(BF[1], 1).unwrap();
    let same = bf
        .iter()
        .filter(|a| bf2.iter().any(|b| b.key == a.key))
        .count();
    assert!(same >= 15, "{same}");
    let other = src.parse(BF[0], 2).unwrap();
    assert!(bf.iter().zip(&other).all(|(a, b)| a.key != b.key));
    // On the ground, without a position, drones, too old: left out.
    let odd = r#"{"now": 1, "ac": [
        {"hex": "f0aaaa", "alt_baro": "ground", "lat": 47.9, "lon": 7.9, "gs": 10, "category": "A3"},
        {"hex": "f0aaab", "alt_baro": 3000, "gs": 100, "category": "A3"},
        {"hex": "f0aaac", "alt_baro": 300, "lat": 47.9, "lon": 7.9, "gs": 10, "category": "B6"},
        {"hex": "f0aaad", "alt_baro": 3000, "lat": 47.9, "lon": 7.9, "gs": 100, "seen_pos": 90},
        {"hex": "f0aaae", "alt_baro": 1500, "lat": 47.9, "lon": 7.9, "gs": 55, "category": "B2"}
    ]}"#;
    let s = src.parse(odd, 1).unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].kind, AirKind::Balloon);
    assert!(src.parse("<html>", 1).is_err());
    assert!(src.parse("{}", 1).is_err());
}

#[test]
fn the_request_is_an_area_query_with_the_key_only_in_a_header() {
    let r = AdsbLol::default().request(1, 47.92, 7.96, QUERY_RADIUS_NM);
    assert_eq!(r.url, "https://api.adsb.lol/v2/point/47.9200/7.9600/35");
    assert!(r.headers.is_empty());
    let k = AdsbLol {
        api_key: Some("k-test".into()),
    }
    .request(2, 28.3, -16.5, 400);
    assert!(k.url.ends_with("/250"), "at most 250 nm");
    assert_eq!(k.headers, [("x-api-key".to_owned(), "k-test".to_owned())]);
    assert!(!k.url.contains("k-test"));
    // The query follows the riders in steps.
    let (lat, lon) = query_centre(47.9213, 7.9788);
    assert!((lat - 47.9).abs() < 1e-9 && (lon - 8.0).abs() < 1e-9);
}

/// A feed answering every request with `answer`; the requests' times.
fn run_feed(
    feed: &mut LiveAirFeed,
    from_ms: i64,
    seconds: i64,
    wanted: impl Fn(i64) -> bool,
    mut answer: impl FnMut(i64) -> Option<Answer> + Send + 'static,
) -> (Vec<i64>, Vec<v1::LiveAirUpdate>) {
    let clock = Arc::new(Mutex::new(from_ms));
    let c2 = clock.clone();
    let mut f = FnAirFetcher::new(move |_r: &Request| answer(*c2.lock().unwrap()));
    let log = f.log.clone();
    let times = Arc::new(Mutex::new(Vec::new()));
    let mut updates = Vec::new();
    for s in 0..seconds {
        let now = from_ms + s * 1000;
        *clock.lock().unwrap() = now;
        let before = log.lock().unwrap().len();
        if let Some(u) = feed.step(now, wanted(now), (47.92, 7.96), Some(&mut f)) {
            updates.push(u);
        }
        if log.lock().unwrap().len() > before {
            times.lock().unwrap().push(now);
        }
    }
    let t = times.lock().unwrap().clone();
    (t, updates)
}

fn max_in_any_day(times: &[i64]) -> usize {
    let mut best = 0;
    let mut j = 0;
    for i in 0..times.len() {
        while times[i] - times[j] >= Budget::DAY_MS {
            j += 1;
        }
        best = best.max(i - j + 1);
    }
    best
}

/// The source's limit is never exceeded (18E T10): riding with live
/// aircraft for two whole simulated days, every answer good.
#[test]
fn the_request_budget_holds_over_simulated_days() {
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (times, updates) = run_feed(&mut feed, T0, 2 * 86_400, |_| true, |_| Some(ok(BF[0])));
    let day1 = times.iter().filter(|t| **t < T0 + Budget::DAY_MS).count();
    assert!(day1 <= Budget::DAY_MAX, "{day1}");
    assert!(max_in_any_day(&times) <= Budget::DAY_MAX);
    const {
        assert!(
            Budget::DAY_MAX < 500,
            "inside adsb.lol's 500 a day without a key"
        )
    };
    assert!(times.windows(2).all(|w| w[1] - w[0] >= Budget::MIN_GAP_MS));
    // The first 100 minutes a fetch a minute, then the refill's pace.
    assert!(
        times[99] - times[0] <= 100 * 61_000,
        "{}",
        times[99] - times[0]
    );
    let late: Vec<_> = times.windows(2).skip(150).map(|w| w[1] - w[0]).collect();
    assert!(
        late.iter().all(|g| *g >= 280_000 && *g <= 300_000),
        "{:?}",
        &late[..5]
    );
    // Data never stale while riding: each gap shorter than the clients'
    // extrapolation.
    assert!(late.iter().all(|g| *g < STALE_MS));
    assert!(
        updates
            .iter()
            .all(|u| u.status == v1::LiveAirStatus::Live as i32)
    );
    assert!(day1 > 350, "the budget is used: {day1}");
    // Nobody wants them: no requests at all.
    let mut idle = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (t, _) = run_feed(&mut idle, T0, 3600, |_| false, |_| Some(ok(BF[0])));
    assert!(t.is_empty());
}

/// The source's own word: few requests left in its window or a 429 pause
/// us; a hanging or failing source backs off.
#[test]
fn the_source_pauses_us_and_failures_back_off() {
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (t, updates) = run_feed(
        &mut feed,
        T0,
        7200,
        |_| true,
        |_| {
            Some(Answer {
                remaining: Some(10),
                reset_s: Some(3000),
                ..ok(BF[0])
            })
        },
    );
    // One request, then the pause until the reset (50 min), then one more.
    assert_eq!(t.len(), 3, "{t:?}");
    assert!(t[1] - t[0] >= 3_000_000);
    assert!(
        updates
            .iter()
            .any(|u| u.status == v1::LiveAirStatus::Down as i32 && u.note.contains("limit"))
    );
    let mut b = Budget::default();
    assert!(b.allows(T0));
    b.take(T0);
    b.note(
        T0,
        &Answer {
            result: Ok((429, String::new())),
            retry_after_s: Some(600),
            ..Default::default()
        },
    );
    assert!(!b.allows(T0 + 599_000) && b.allows(T0 + 600_000));
    // Failures: 1, 2, 4, 8 … minutes apart.
    let mut down = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (t, _) = run_feed(
        &mut down,
        T0,
        3 * 3600,
        |_| true,
        |_| {
            Some(Answer {
                result: Err("connection refused".into()),
                ..Default::default()
            })
        },
    );
    let gaps: Vec<i64> = t.windows(2).map(|w| (w[1] - w[0]) / 1000).collect();
    assert_eq!(&gaps[..5], &[60, 120, 240, 480, 960], "{gaps:?}");
    assert!(gaps.iter().all(|g| *g <= BACKOFF_MAX_MS / 1000));
}

/// The feed down → the riders are told within 5 s (they draw the
/// simulated sky).
#[test]
fn feed_down_is_told_within_five_seconds() {
    // Failing at once.
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (_, u) = run_feed(
        &mut feed,
        T0,
        3,
        |_| true,
        |_| {
            Some(Answer {
                result: Ok((503, "busy".into())),
                ..Default::default()
            })
        },
    );
    assert_eq!(u[0].status, v1::LiveAirStatus::Down as i32);
    assert!(u[0].note.contains("HTTP 503"), "{}", u[0].note);
    assert_eq!(feed.state(), FeedState::Down);
    // Hanging: no answer at all.
    let mut hang = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (t, u) = run_feed(&mut hang, T0, 10, |_| true, |_| None);
    assert_eq!(t, [T0]);
    let first = &u[0];
    assert_eq!(first.status, v1::LiveAirStatus::Down as i32);
    assert!(first.note.contains("no answer"));
    const { assert!(ANSWER_TIMEOUT_MS + 1000 < 5000) };
    // Live, then down: told at once, the last answer forgotten.
    let mut flaky = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let mut n = 0;
    let (_, u) = run_feed(
        &mut flaky,
        T0,
        65,
        |_| true,
        move |_| {
            n += 1;
            Some(if n == 1 {
                ok(BF[0])
            } else {
                Answer {
                    result: Err("timed out".into()),
                    ..Default::default()
                }
            })
        },
    );
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].status, v1::LiveAirStatus::Live as i32);
    assert_eq!(u[1].status, v1::LiveAirStatus::Down as i32);
    assert!(
        flaky
            .current(T0 + 65_000, true)
            .unwrap()
            .aircraft
            .is_empty()
    );
    // OpenSky: disabled, said so.
    let mut os = LiveAirFeed::new(Box::new(OpenSky), 7);
    let (t, u) = run_feed(&mut os, T0, 5, |_| true, |_| Some(ok(BF[0])));
    assert!(t.is_empty(), "never asked");
    assert_eq!(u[0].status, v1::LiveAirStatus::Unavailable as i32);
    assert!(u[0].note.contains("agreement"));
    // A server without a fetcher (the browser's): unavailable.
    let none = LiveAirFeed::default();
    assert_eq!(
        none.current(T0, false).unwrap().status,
        v1::LiveAirStatus::Unavailable as i32
    );
}

/// No identities in the messages (18D): the encoded updates hold none
/// of the fixtures' addresses, callsigns, registrations or type codes, and
/// the numbers carry on from fetch to fetch.
#[test]
fn no_identities_in_the_messages_and_numbers_carry_on() {
    let ids = identities();
    assert!(ids.iter().any(|s| s == "TEST01") && ids.iter().any(|s| s == "A320"));
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let mut i = 0;
    let (_, updates) = run_feed(
        &mut feed,
        T0,
        150,
        |_| true,
        move |_| {
            let b = BF[i.min(2)];
            i += 1;
            Some(ok(b))
        },
    );
    assert_eq!(updates.len(), 3);
    for u in &updates {
        let bytes = ocw_protocol::encode(&ocw_protocol::server_message(
            v1::server_message::Msg::LiveAir(u.clone()),
        ));
        let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
        for id in &ids {
            assert!(
                !text.contains(&id.to_ascii_lowercase()),
                "{id} in the message"
            );
        }
        assert_eq!(u.attribution, ADSB_LOL_ATTRIBUTION);
        // The compact states: about 25 bytes an aircraft.
        assert!(bytes.len() < 60 + 32 * u.aircraft.len(), "{}", bytes.len());
        assert!(ocw_protocol::decode_server(&bytes).is_ok());
    }
    let a: Vec<u32> = updates[0].aircraft.iter().map(|a| a.id).collect();
    let b: Vec<u32> = updates[1].aircraft.iter().map(|a| a.id).collect();
    assert!(a.iter().filter(|id| b.contains(id)).count() >= 15);
    let debug = format!("{feed:?}");
    assert!(ids.iter().all(|id| !debug.contains(id.as_str())));
}

/// … and none in the logs: the feed's log lines carry counts only.
#[test]
fn no_identities_in_the_logs() {
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let w = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || w.clone())
        .with_ansi(false)
        .finish();
    // The process's subscriber: a scoped one misses events whose callsites
    // another test's thread registered first (the interest cache); this
    // one also sees the other tests' lines, which must hold no identities
    // either.
    if tracing::subscriber::set_global_default(sub).is_err() {
        eprintln!("another subscriber is set: the log test is skipped");
        return;
    }
    {
        let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
        let mut n = 0;
        run_feed(
            &mut feed,
            T0,
            200,
            |_| true,
            move |_| {
                n += 1;
                Some(match n {
                    1 => ok(BF[0]),
                    2 => Answer {
                        result: Err("bad gateway".into()),
                        ..Default::default()
                    },
                    _ => ok(TF[0]),
                })
            },
        );
    }
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("live aircraft fetched"), "{log}");
    assert!(log.contains("unavailable"), "{log}");
    for id in identities() {
        assert!(!log.contains(&id), "{id} in the log");
    }
    assert!(!log.contains("api.adsb.lol/v2"), "no place in the logs");
}

/// The extrapolation between fetches on the recorded sequences: each
/// aircraft of the first answer carried 20 and 40 s forward along its
/// great circle lands where the next answers saw it.
#[test]
fn extrapolation_matches_the_recorded_sequences() {
    let src = AdsbLol::default();
    let mut report = Vec::new();
    for (name, seq) in [("blackforest", BF), ("tenerife", TF)] {
        let t0 = feed_now_ms(seq[0]);
        let first: Vec<(u64, LiveAircraft)> = src
            .parse(seq[0], 1)
            .unwrap()
            .into_iter()
            .map(|s| {
                (
                    s.key,
                    LiveAircraft {
                        id: 0,
                        kind: s.kind,
                        lat: s.lat,
                        lon: s.lon,
                        alt_m: s.alt_m,
                        speed_mps: s.speed_mps,
                        track_deg: s.track_deg,
                        climb_mps: s.climb_mps,
                        at_ms: t0 - (s.seen_s * 1000.0) as i64,
                    },
                )
            })
            .collect();
        for later in &seq[1..] {
            let t = feed_now_ms(later);
            let mut errs = Vec::new();
            let mut still = Vec::new();
            for s in src.parse(later, 1).unwrap() {
                let Some((_, a)) = first.iter().find(|(k, _)| *k == s.key) else {
                    continue;
                };
                let at = t - (s.seen_s * 1000.0) as i64;
                let (lat, lon, alt) = a.at(at).unwrap();
                let e = distance_m((lat, lon), (s.lat, s.lon));
                errs.push((e, (alt - s.alt_m).abs()));
                still.push(distance_m((a.lat, a.lon), (s.lat, s.lon)));
            }
            assert!(errs.len() >= 3, "{name}");
            let mut h: Vec<f64> = errs.iter().map(|e| e.0).collect();
            h.sort_by(f64::total_cmp);
            let median = h[h.len() / 2];
            let max = h[h.len() - 1];
            let mut moved = still.clone();
            moved.sort_by(f64::total_cmp);
            let dt = (t - t0) as f64 / 1000.0;
            report.push(format!(
                "{name} +{dt:.0} s: {} aircraft, median {median:.0} m, max {max:.0} m (moved: median {:.0} m)",
                h.len(),
                moved[moved.len() / 2]
            ));
            // Far better than standing still, and within a few seconds of
            // flight.
            assert!(median < 0.1 * moved[moved.len() / 2], "{report:?}");
            assert!(median < 400.0, "{report:?}");
            // A turning aircraft drifts off its straight line.
            assert!(max < 3000.0, "{report:?}");
            let alt_max = errs.iter().map(|e| e.1).fold(0.0, f64::max);
            // Climbs and descents change their rate.
            assert!(alt_max < 600.0, "{report:?} alt {alt_max}");
        }
    }
    println!("{}", report.join("\n"));
}

/// The wire's aircraft are where the feed saw them, as of their fix.
#[test]
fn updates_carry_the_positions_and_their_times() {
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (_, u) = run_feed(&mut feed, T0, 2, |_| true, |_| Some(ok(TF[0])));
    let u = &u[0];
    assert_eq!(u.fetched_ms, T0);
    assert_eq!(u.next_s, 60);
    let src = AdsbLol::default().parse(TF[0], 1).unwrap();
    assert_eq!(u.aircraft.len(), src.len());
    for (w, s) in u.aircraft.iter().zip(&src) {
        let a = aircraft_from_wire(w, u.fetched_ms);
        assert!((a.lat - s.lat).abs() < 1e-6 && (a.lon - s.lon).abs() < 1e-6);
        assert_eq!(a.at_ms, T0 - (s.seen_s * 1000.0).round() as i64);
        assert_eq!(a.kind, s.kind);
    }
    // The current answer for a rider who asks now; nothing after it grew
    // stale.
    assert_eq!(feed.current(T0 + 1000, true).as_ref(), Some(u));
    let mut f = FnAirFetcher::new(|_r: &Request| None);
    let stale = feed.step(T0 + STALE_MS, false, (28.29, -16.52), Some(&mut f));
    assert_eq!(stale.unwrap().status, v1::LiveAirStatus::Down as i32);
    assert!(
        feed.current(T0 + STALE_MS + 1, true)
            .is_some_and(|u| u.aircraft.is_empty())
    );
}

/// The game server (18E T10): one fetch for every rider who wants live
/// aircraft, sent to them only; a rider who starts wanting them gets the
/// last fetch at once; a server without a fetcher says so; the messages
/// carry no identities.
#[test]
fn the_game_server_shares_one_fetch_with_the_riders_who_want_it() {
    use crate::{ConnectionId, GameServer, ServerConfig};
    use ocw_protocol::v1::{client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
    use ocw_world::Route;

    fn hello(name: &str) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: name.into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            ..Default::default()
        })))
    }
    fn join(s: &mut GameServer, name: &str) -> ConnectionId {
        let c = s.connect();
        let r = s.handle(c, &hello(name));
        assert!(matches!(decode_server(&r[0]), Ok(S::Welcome(_))));
        c
    }
    fn live_to(s: &mut GameServer) -> Vec<(ConnectionId, v1::LiveAirUpdate)> {
        s.take_directed()
            .into_iter()
            .filter_map(|(c, b)| match decode_server(&b) {
                Ok(S::LiveAir(u)) => Some((c, u)),
                _ => None,
            })
            .collect()
    }
    let wish = |on| encode(&ocw_protocol::live_air::wish(on));
    let config = ServerConfig {
        real_epoch_ms: Some(T0),
        geo_origin: Some((47.92, 7.96)),
        ..Default::default()
    };
    let fetcher = FnAirFetcher::new(|_r: &Request| Some(ok(BF[0])));
    let log = fetcher.log.clone();
    let mut s = GameServer::new(Route::test_loop(), config.clone());
    s.set_air_fetcher(Box::new(fetcher));
    let ada = join(&mut s, "Ada Rider");
    let bea = join(&mut s, "Bea Climber");
    let cy = join(&mut s, "Cy Sprinter");
    // Nobody wants them: no requests.
    for _ in 0..200 {
        s.tick();
    }
    assert!(log.lock().unwrap().is_empty());
    s.handle(ada, &wish(true));
    s.handle(bea, &wish(true));
    let mut got = Vec::new();
    for _ in 0..100 {
        s.tick();
        got.extend(live_to(&mut s));
    }
    assert_eq!(log.lock().unwrap().len(), 1, "one fetch for both");
    let to: Vec<ConnectionId> = got.iter().map(|g| g.0).collect();
    assert!(to.contains(&ada) && to.contains(&bea) && !to.contains(&cy));
    let u = &got[0].1;
    assert_eq!(u.status, v1::LiveAirStatus::Live as i32);
    assert_eq!(u.aircraft.len(), 18);
    let ids = identities();
    let bytes = encode(&ocw_protocol::server_message(S::LiveAir(u.clone())));
    let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    assert!(ids.iter().all(|i| !text.contains(&i.to_ascii_lowercase())));
    // Cy starts wanting them: the last fetch at once, no new request.
    s.handle(cy, &wish(true));
    let now = live_to(&mut s);
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].0, cy);
    assert_eq!(&now[0].1, u);
    assert_eq!(log.lock().unwrap().len(), 1);
    // All stop: no more requests; one leaves.
    for c in [ada, bea, cy] {
        s.handle(c, &wish(false));
    }
    for _ in 0..(50 * 180) {
        s.tick();
    }
    assert_eq!(log.lock().unwrap().len(), 1);
    assert_eq!(s.live_air().stats.answers, 1);
    // A server without a fetcher (the browser's local server).
    let mut plain = GameServer::new(Route::test_loop(), config);
    let d = join(&mut plain, "Dee Descender");
    plain.handle(d, &wish(true));
    let told = live_to(&mut plain);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].1.status, v1::LiveAirStatus::Unavailable as i32);
}

// ---- Live and simulated together (spec 14 Phase 18 Q13 (c)) ------------

/// A made-up adsb.lol answer: an airliner overhead and, if given, an
/// aircraft of `category` `km` north of (47.90, 7.95) – the query's
/// centre for the place of `run_feed`.
fn made_up(extra: Option<(&str, f64)>) -> String {
    let mut list = vec![
        r#"{"hex":"f30001","flight":"TEST01","category":"A3","alt_geom":36000,"gs":450,"track":40,"lat":47.95,"lon":8.0,"seen_pos":0.0}"#.to_owned(),
    ];
    if let Some((category, km)) = extra {
        let (lat, lon) = ocw_sim::air::live::destination(47.90, 7.95, 0.0, km * 1000.0);
        list.push(format!(
            r#"{{"hex":"f30002","flight":"TEST02","category":"{category}","alt_geom":3500,"gs":0,"track":0,"lat":{lat:.6},"lon":{lon:.6},"seen_pos":0.0}}"#
        ));
    }
    format!(r#"{{"ac":[{}],"now":0}}"#, list.join(","))
}

/// The feed's ledger (Q13 (c)): a helicopter or small plane within 30 km
/// of the query's centre opens a time for its kind, at 31 km none; the
/// time ends with the fetch that no longer sees it, or when the feed is
/// lost; every update carries the ledger – kinds and times only.
#[test]
fn the_ledger_says_when_the_feed_showed_small_planes_and_helicopters_near() {
    use ocw_protocol::live_air::ledger_from_wire;
    let place = (47.92, 7.96);
    let centre = query_centre(place.0, place.1);
    assert!((centre.0 - 47.9).abs() < 1e-9 && (centre.1 - 7.95).abs() < 1e-9);
    let body = Arc::new(Mutex::new(String::new()));
    let b = body.clone();
    let mut f = FnAirFetcher::new(move |_r: &Request| Some(ok(&b.lock().unwrap())));
    let mut feed = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let mut fetch = |feed: &mut LiveAirFeed, t: i64, answer: String| {
        *body.lock().unwrap() = answer;
        feed.step(t, true, place, Some(&mut f)).expect("an update")
    };
    // 31 km: not near.
    let u = fetch(&mut feed, T0, made_up(Some(("A7", 31.0))));
    assert_eq!(u.aircraft.len(), 2);
    assert!(u.sim_busy.is_empty());
    // 29 km: a helicopter's time from this fetch on.
    let t1 = T0 + 60_000;
    let u = fetch(&mut feed, t1, made_up(Some(("A7", 29.0))));
    assert_eq!(u.sim_busy.len(), 1);
    assert_eq!(u.sim_busy[0].class, v1::LiveAirClass::Helicopter as i32);
    assert_eq!(u.sim_busy[0].from_ms, t1);
    assert!(
        u.sim_busy[0].to_ms > t1 + 60_000,
        "held until the next fetch"
    );
    let l = ledger_from_wire(&u.sim_busy);
    assert!(l.busy_at(AirKind::Helicopter, t1 + 30_000));
    assert!(!l.busy_at(AirKind::SmallPlane, t1 + 30_000));
    assert_eq!(&l, feed.ledger());
    // A small plane too (A1), the helicopter gone.
    let t2 = t1 + 60_000;
    let u = fetch(&mut feed, t2, made_up(Some(("A1", 12.0))));
    let l = ledger_from_wire(&u.sim_busy);
    assert!(l.busy_at(AirKind::Helicopter, t2 - 1) && !l.busy_at(AirKind::Helicopter, t2));
    assert!(l.busy_at(AirKind::SmallPlane, t2));
    // A glider near opens no time (nor an airliner or a balloon).
    let t3 = t2 + 60_000;
    let u = fetch(&mut feed, t3, made_up(Some(("B1", 2.0))));
    let l = ledger_from_wire(&u.sim_busy);
    assert!(!l.busy_at(AirKind::SmallPlane, t3) && !l.busy_at(AirKind::Helicopter, t3));
    assert_eq!(l.times().len(), 2, "what was stays for the flights of then");
    // The ledger is kinds and times: nothing of an aircraft, no place –
    // with it the update is a few bytes longer.
    let size = |u: &v1::LiveAirUpdate| {
        ocw_protocol::encode(&ocw_protocol::server_message(
            v1::server_message::Msg::LiveAir(u.clone()),
        ))
        .len()
    };
    let bare = v1::LiveAirUpdate {
        sim_busy: Vec::new(),
        ..u.clone()
    };
    assert!(size(&u) - size(&bare) <= 2 * (4 + 2 * 10));
    // The feed fails while a small plane is near: its time ends there
    // (the clients draw the whole simulated sky; what starts in it stays).
    let t4 = t3 + 60_000;
    fetch(&mut feed, t4, made_up(Some(("A1", 5.0))));
    assert!(feed.ledger().busy_at(AirKind::SmallPlane, t4 + 100_000));
    let t5 = t4 + 60_000;
    let down = fetch(&mut feed, t5, "not json".into());
    assert_eq!(down.status, v1::LiveAirStatus::Down as i32);
    assert!(down.sim_busy.is_empty());
    assert!(!feed.ledger().busy_at(AirKind::SmallPlane, t5));
    assert!(feed.ledger().busy_at(AirKind::SmallPlane, t5 - 1));
    // Another world: nothing of the old one.
    feed.world_changed();
    assert!(feed.ledger().times().is_empty());
    // The recorded Black Forest answer: only the two kinds ever.
    let mut bf = LiveAirFeed::new(Box::new(AdsbLol::default()), 7);
    let (_, ups) = run_feed(&mut bf, T0, 2, |_| true, |_| Some(ok(BF[0])));
    assert!(ups[0].sim_busy.iter().all(|b| {
        b.class == v1::LiveAirClass::Helicopter as i32
            || b.class == v1::LiveAirClass::SmallPlane as i32
    }));
}

/// A recorded answer (`OCW_LIVE_AIR_FILE`; pictures and tests without the
/// network): read again for every request, and taken as the sky of the
/// world clock – also after the time machine moved it, back or forward.
#[test]
fn a_recorded_answer_is_the_sky_of_the_world_clock() {
    use crate::{GameServer, ServerConfig};
    use ocw_protocol::v1::{client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
    use ocw_world::Route;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sky.json");
    std::fs::write(&path, made_up(Some(("A7", 10.0)))).unwrap();
    let fetcher = FileAirFetcher::new(&path);
    assert!(fetcher.recorded());
    assert!(!FnAirFetcher::new(|_r: &Request| None).recorded());
    let mut s = GameServer::new(
        Route::test_loop(),
        ServerConfig {
            real_epoch_ms: Some(T0),
            geo_origin: Some((47.92, 7.96)),
            ..Default::default()
        },
    );
    s.set_air_fetcher(Box::new(fetcher));
    let c = s.connect();
    s.handle(
        c,
        &encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: "Ada Rider".into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            ..Default::default()
        }))),
    );
    s.handle(c, &encode(&ocw_protocol::live_air::wish(true)));
    let run = |s: &mut GameServer, ticks: usize| -> Vec<v1::LiveAirUpdate> {
        let mut got = Vec::new();
        for _ in 0..ticks {
            s.tick();
            got.extend(s.take_directed().into_iter().filter_map(|(_, b)| {
                match decode_server(&b) {
                    Ok(S::LiveAir(u)) => Some(u),
                    _ => None,
                }
            }));
        }
        got
    };
    let live = |got: &[v1::LiveAirUpdate]| -> Vec<v1::LiveAirUpdate> {
        got.iter()
            .filter(|u| u.status == v1::LiveAirStatus::Live as i32)
            .cloned()
            .collect()
    };
    let first = run(&mut s, 100);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, v1::LiveAirStatus::Live as i32);
    assert_eq!(first[0].aircraft.len(), 2);
    assert_eq!(first[0].sim_busy.len(), 1, "the helicopter 10 km away");
    assert!((first[0].fetched_ms - T0).abs() < 5000);
    // The time machine: 135 days back, to a summer afternoon.
    let step = |s: &mut GameServer, ms: i64| {
        s.handle(
            c,
            &encode(&ocw_protocol::clock_request(ocw_sky::ClockChange::Step(ms))),
        );
    };
    let back = -135 * 86_400_000;
    step(&mut s, back);
    let then = live(&run(&mut s, 150));
    assert_eq!(then.len(), 1, "fetched again for the new time");
    assert!((then[0].fetched_ms - (T0 + back)).abs() < 10_000);
    assert_eq!(then[0].sim_busy.len(), 1);
    assert!(then[0].sim_busy[0].from_ms < T0 + back + 10_000);
    // … and forward again.
    step(&mut s, 3 * 3_600_000);
    let later = live(&run(&mut s, 150));
    assert_eq!(later.len(), 1);
    assert!((later[0].fetched_ms - (T0 + back + 3 * 3_600_000)).abs() < 15_000);
    // A file that is gone: the feed is down, the simulated sky.
    std::fs::remove_file(&path).unwrap();
    step(&mut s, 3_600_000);
    let gone = run(&mut s, 150);
    assert!(
        gone.iter()
            .any(|u| u.status == v1::LiveAirStatus::Down as i32
                && u.note.contains("recorded answer"))
    );
    assert!(
        gone.iter().all(|u| !u.note.contains("sky.json")),
        "no path in the note"
    );
}
