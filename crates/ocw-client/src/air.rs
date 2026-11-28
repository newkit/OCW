//! The client's sky traffic (spec 14 Phase 18, 18A, 18C, T8): the
//! simulated aircraft of `ocw_sim::air` for the rider's own screen and
//! ears – from the session's air seed, the world clock and the weather the
//! server sends, so riders of a group ride who have aircraft on see the
//! same planes in the same places; nothing goes to the server (Q2).
//!
//! `traffic.air`: *off* draws nothing, *simulated* the simulated sky,
//! *live* the live aircraft the server shares ([`crate::live_air`], T10)
//! while they are usable, else the simulated sky (the menu and the `air`
//! command say why: [`crate::live_air::LiveState`]); with live aircraft
//! drawn the view carries the source's credit.
//!
//! **Live and simulated together** (Q13 (c), `ocw_sim::air::mixed`):
//! beside the live aircraft fly the simulated gliders, balloons and small
//! planes in circuits, and the simulated small planes and helicopters
//! that started while the feed showed none of their kind within 30 km
//! (the server's ledger in the update); the simulated airliners stay out.
//! No simulated flight starts within 5 km and 600 m of a live aircraft.
//! All share the quality's budget, the nearest first. The master switch
//! `traffic.enabled` off: nothing. `traffic.sound off`: drawn, not heard.

use std::sync::Arc;

use ocw_core::traffic::{AirTraffic, ViewChoice};
use ocw_render::aircraft::AirView;
use ocw_sim::air::live::{LiveAircraft, LiveFrame};
use ocw_sim::air::mixed::{self, Counts, Ledger, QUIET_KINDS, Separation};
use ocw_sim::air::upper::UpperAir;
use ocw_sim::air::{AirInputs, AirKind, AirSky, AirSpace, Flight};
use ocw_weather::state::WeatherState;
use ocw_world::geo::LocalProjection;

use crate::live_air::{LiveAirClient, LiveState};

/// What one frame of the sky needs.
#[derive(Debug, Clone, Copy)]
pub struct AirFrame<'a> {
    /// The rider's `[traffic]` view settings.
    pub choice: ViewChoice,
    /// The session's air seed.
    pub seed: u64,
    /// The world clock (ms since the Unix epoch) and the place's UTC
    /// offset (s); `None` before the server's clock is known.
    pub clock: Option<(i64, i32)>,
    /// The session's weather (`None`: the default fair day).
    pub weather: Option<&'a WeatherState>,
    /// The eye (camera) and which way it looks (x, z).
    pub eye: [f64; 3],
    pub forward: [f64; 2],
    /// The ground under the rider (m): where the weather's temperature is.
    pub ground_m: f64,
    /// The quality level's budget and whether contrails are drawn.
    pub max_aircraft: usize,
    pub contrails: bool,
    /// The live aircraft the server sent (`None`: no ride).
    pub live: Option<&'a LiveAirClient>,
}

/// The view radius of the sky (m).
pub const SKY_RADIUS_M: f64 = 110_000.0;

/// What flies in a sky of live and simulated aircraft (the `air`
/// command; spec 14 Phase 18 Q13 (c)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixStatus {
    /// The live aircraft within the sky's radius, by kind.
    pub live: Counts,
    /// The simulated flights beside them, by kind.
    pub simulated: Counts,
    /// Whether the feed shows a small plane / a helicopter within 30 km
    /// now (no new simulated one of the kind starts), in the order of
    /// [`QUIET_KINDS`].
    pub busy: [bool; QUIET_KINDS.len()],
}

impl MixStatus {
    /// The lines of the `air` command.
    pub fn lines(&self) -> Vec<String> {
        let quiet = |i: usize| {
            let kind = match QUIET_KINDS[i] {
                AirKind::Helicopter => "helicopters",
                _ => "small planes away from circuits",
            };
            if self.busy[i] {
                format!("{kind}: the feed shows one within 30 km – no new simulated ones")
            } else {
                format!("{kind}: none live within 30 km – simulated ones fly")
            }
        };
        vec![
            format!("live aircraft: {}", mixed::describe(&self.live)),
            format!("simulated with them: {}", mixed::describe(&self.simulated)),
            format!(
                "  gliders, balloons and circuits always; {}; {}; airliners: live only",
                quiet(0),
                quiet(1)
            ),
        ]
    }
}

/// The client's sky.
#[derive(Debug, Default)]
pub struct AirClient {
    space: Option<(u64, Arc<AirSpace>)>,
    sky: AirSky,
    /// Which simulated flights started clear of the live aircraft.
    clear: Separation,
}

impl AirClient {
    /// The world's airspace (built once per world; `key` names it).
    pub fn set_space(&mut self, key: u64, space: impl FnOnce() -> AirSpace) {
        if self.space.as_ref().is_none_or(|s| s.0 != key) {
            self.space = Some((key, Arc::new(space())));
            self.sky = AirSky::default();
            self.clear.clear();
        }
    }

    pub fn space(&self) -> Option<&Arc<AirSpace>> {
        self.space.as_ref().map(|s| &s.1)
    }

    /// The sky's flights (the `air` command).
    pub fn sky_mut(&mut self) -> &mut AirSky {
        &mut self.sky
    }

    /// The live aircraft and the ledger at frame `f`, while live aircraft
    /// are drawn.
    fn live_now<'a>(f: &AirFrame<'a>) -> Option<(&'a [LiveAircraft], &'a Ledger)> {
        if Self::live_state(f) != Some(LiveState::Live) {
            return None;
        }
        let (ms, _) = f.clock?;
        let live = f.live?;
        Some((live.aircraft(ms)?, live.ledger(ms)?))
    }

    /// The simulated flights drawn at frame `f` (not limited by the
    /// budget): all of the simulated sky – or, while live aircraft are
    /// drawn, those that fly beside them (Q13 (c): the rule of their kind
    /// by the server's ledger, and started clear of the live aircraft).
    /// `None`: no airspace or clock yet.
    pub fn flights(&mut self, f: &AirFrame, fallback: &WeatherState) -> Option<Vec<Arc<Flight>>> {
        let space = self.space.as_ref()?.1.clone();
        let inp = Self::inputs(f, fallback)?;
        Some(self.flights_in(&space, &inp, f))
    }

    fn flights_in(&mut self, space: &AirSpace, inp: &AirInputs, f: &AirFrame) -> Vec<Arc<Flight>> {
        let proj = LocalProjection::new(space.origin.0, space.origin.1);
        let (set, ledger) = match Self::live_now(f) {
            Some((set, ledger)) => (set, Some(ledger)),
            None => (&[][..], None),
        };
        let clear = &mut self.clear;
        self.sky.flights_where(space, inp, |fl| {
            // Every flight drawn is remembered, so that none is taken
            // away when live aircraft arrive.
            ledger.is_none_or(|l| l.admits(fl))
                && clear.keeps(fl, set, &proj, space.relief(), inp.now_s)
        })
    }

    /// What is live and what is simulated at frame `f` (`None`: live
    /// aircraft are not drawn).
    pub fn mix_status(&mut self, f: &AirFrame, fallback: &WeatherState) -> Option<MixStatus> {
        let (set, ledger) = Self::live_now(f)?;
        let (ms, _) = f.clock?;
        let space = self.space.as_ref()?.1.clone();
        let inp = Self::inputs(f, fallback)?;
        let proj = LocalProjection::new(space.origin.0, space.origin.1);
        let live = mixed::count(set.iter().filter_map(|a| {
            let p = ocw_sim::air::live::position(a, &proj, space.relief(), ms)?;
            ((p[0] - f.eye[0]).hypot(p[2] - f.eye[2]) <= SKY_RADIUS_M).then_some(a.kind)
        }));
        let now = inp.now_s;
        let simulated = mixed::count(
            self.flights_in(&space, &inp, f)
                .iter()
                .filter(|fl| {
                    let p = fl.state(now).pos;
                    (p[0] - f.eye[0]).hypot(p[2] - f.eye[2]) <= SKY_RADIUS_M
                })
                .map(|fl| fl.kind),
        );
        Some(MixStatus {
            live,
            simulated,
            busy: QUIET_KINDS.map(|k| ledger.busy_at(k, ms)),
        })
    }

    /// Whether aircraft are drawn with these settings.
    pub fn shown(choice: &ViewChoice) -> bool {
        matches!(choice.air_shown(), AirTraffic::Simulated | AirTraffic::Live)
    }

    /// Whether the rider wants live aircraft (the wish the server gets).
    pub fn wants_live(choice: &ViewChoice) -> bool {
        choice.air_shown() == AirTraffic::Live
    }

    /// With *live*: whether the live aircraft are drawn at frame `f`, or
    /// why not (`None`: live is not chosen).
    pub fn live_state(f: &AirFrame) -> Option<LiveState> {
        if !Self::wants_live(&f.choice) {
            return None;
        }
        let Some((ms, _)) = f.clock else {
            return Some(LiveState::Waiting);
        };
        Some(f.live.map_or(LiveState::Waiting, |l| l.state(ms)))
    }

    /// The inputs of the sky at frame `f` (`None`: no clock yet).
    pub fn inputs<'a>(f: &AirFrame<'a>, fallback: &'a WeatherState) -> Option<AirInputs<'a>> {
        let (ms, offset) = f.clock?;
        Some(AirInputs {
            seed: f.seed,
            now_s: ms as f64 / 1000.0,
            utc_offset_s: offset,
            weather: f.weather.unwrap_or(fallback),
            viewer: f.eye,
            radius_m: SKY_RADIUS_M,
            max_aircraft: f.max_aircraft,
            contrails: f.contrails,
            ground_m: f.ground_m,
        })
    }

    /// The frame's aircraft (`None`: none drawn).
    pub fn frame(&mut self, f: &AirFrame) -> Option<AirView> {
        if !Self::shown(&f.choice) {
            return None;
        }
        let space = self.space.as_ref()?.1.clone();
        let fallback = WeatherState::default();
        let inp = Self::inputs(f, &fallback)?;
        // The simulated flights: all of them, or those that fly beside
        // the live aircraft.
        let flights = self.flights_in(&space, &inp, f);
        let (aircraft, heard, attribution) = match Self::live_now(f)
            .and(f.live)
            .and_then(|live| Self::live_frame(&space, live, &inp, f))
        {
            Some(live) => (live.aircraft, live.heard, live.attribution),
            None => (Vec::new(), Vec::new(), None),
        };
        let aircraft = mixed::now(aircraft, &flights, &inp);
        let heard = if f.choice.sound {
            mixed::heard(heard, &flights, inp.now_s, f.eye)
        } else {
            Vec::new()
        };
        Some(AirView {
            aircraft,
            heard,
            listener: f.eye,
            listener_forward: f.forward,
            attribution,
        })
    }

    /// The live aircraft's part of frame `f`: T8's drawing and sound rules (the
    /// budget, the contrails from the upper air, the delay of sound), the
    /// server's aircraft extrapolated to the world clock and the
    /// corrections blended; with the source's credit.
    fn live_frame(
        space: &AirSpace,
        live: &LiveAirClient,
        inp: &AirInputs,
        f: &AirFrame,
    ) -> Option<AirView> {
        let (ms, _) = f.clock?;
        let set = live.aircraft(ms)?;
        let proj = LocalProjection::new(space.origin.0, space.origin.1);
        let upper = UpperAir::from_weather(inp.weather, inp.ground_m);
        let frame = LiveFrame {
            t_ms: ms,
            viewer: f.eye,
            radius_m: SKY_RADIUS_M,
            max_aircraft: f.max_aircraft,
            contrails: f.contrails,
            upper: &upper,
            ground_m: f.ground_m,
        };
        let lat = space.origin.0;
        let aircraft = ocw_sim::air::live::now(set, &proj, space.relief(), &frame, |id| {
            live.offset_m(id, ms, lat)
        });
        let heard = if f.choice.sound {
            ocw_sim::air::live::heard(set, &proj, space.relief(), ms, f.eye)
        } else {
            Vec::new()
        };
        Some(AirView {
            aircraft,
            heard,
            listener: f.eye,
            listener_forward: f.forward,
            attribution: live.attribution().map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::traffic::Ufos;
    use ocw_world::aero::NEARBY;

    fn client() -> AirClient {
        let mut c = AirClient::default();
        // Puerto de la Cruz: TFN and TFS from the table.
        c.set_space(1, || {
            AirSpace::new(
                (28.41, -16.55),
                None,
                ocw_sim::air::relief::Relief::default(),
                |_, _| None,
            )
        });
        assert!(NEARBY.iter().any(|n| n.iata == "TFN"));
        c
    }

    fn frame(choice: ViewChoice) -> AirFrame<'static> {
        AirFrame {
            choice,
            seed: 99,
            // 2026-07-15 10:00 UTC, 11:00 in Tenerife.
            clock: Some((1_784_109_600_000, 3600)),
            weather: None,
            eye: [0.0, 300.0, 0.0],
            forward: [0.0, -1.0],
            ground_m: 300.0,
            max_aircraft: 30,
            contrails: true,
            live: None,
        }
    }

    #[test]
    fn off_draws_nothing_and_live_is_simulated() {
        let mut c = client();
        let on = ViewChoice::default();
        let sim = c.frame(&frame(on)).expect("aircraft by default (Q1)");
        assert!(!sim.aircraft.is_empty());
        assert!(
            c.frame(&frame(ViewChoice {
                air: AirTraffic::Off,
                ..on
            }))
            .is_none()
        );
        assert!(
            c.frame(&frame(ViewChoice {
                enabled: false,
                ..on
            }))
            .is_none()
        );
        let live = c
            .frame(&frame(ViewChoice {
                air: AirTraffic::Live,
                ufo: Ufos::Rare,
                ..on
            }))
            .expect("live falls back to simulated");
        assert_eq!(live.aircraft, sim.aircraft);
        // No clock yet: nothing.
        assert!(
            c.frame(&AirFrame {
                clock: None,
                ..frame(on)
            })
            .is_none()
        );
    }

    /// Live (18E T10): the server's aircraft with the credit while they
    /// are usable; the simulated sky (no credit) while the feed is down,
    /// nothing came yet, or the clock is elsewhere.
    #[test]
    fn live_draws_the_servers_aircraft_and_falls_back_to_simulated() {
        use ocw_protocol::live_air::aircraft_to_wire;
        use ocw_protocol::v1;
        use ocw_sim::air::AirKind;
        use ocw_sim::air::live::{ID_BIT, LiveAircraft};
        let mut c = client();
        let on = ViewChoice {
            air: AirTraffic::Live,
            ..ViewChoice::default()
        };
        let base = frame(on);
        let t = base.clock.unwrap().0;
        let sim = c.frame(&frame(ViewChoice::default())).unwrap();
        // Nothing from the server yet: simulated.
        let mut live = LiveAirClient::default();
        let f = AirFrame {
            live: Some(&live),
            ..base
        };
        assert_eq!(AirClient::live_state(&f), Some(LiveState::Waiting));
        let v = c.frame(&f).unwrap();
        assert_eq!(v.aircraft, sim.aircraft);
        assert!(v.attribution.is_none());
        // Two aircraft over Puerto de la Cruz.
        let planes = [
            LiveAircraft {
                id: 1,
                kind: AirKind::Airliner,
                lat: 28.45,
                lon: -16.5,
                alt_m: 1200.0,
                speed_mps: 80.0,
                track_deg: 300.0,
                climb_mps: -4.0,
                at_ms: t - 2000,
            },
            LiveAircraft {
                id: 2,
                kind: AirKind::Helicopter,
                lat: 28.40,
                lon: -16.56,
                alt_m: 600.0,
                speed_mps: 50.0,
                track_deg: 10.0,
                climb_mps: 0.0,
                at_ms: t - 2000,
            },
        ];
        live.on_update(&v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Live as i32,
            attribution: "Aircraft data: adsb.lol (ODbL)".into(),
            fetched_ms: t - 1000,
            aircraft: planes
                .iter()
                .map(|p| aircraft_to_wire(p, t - 1000))
                .collect(),
            next_s: 60,
            ..Default::default()
        });
        let f = AirFrame {
            live: Some(&live),
            ..base
        };
        assert_eq!(AirClient::live_state(&f), Some(LiveState::Live));
        let v = c.frame(&f).unwrap();
        let drawn: Vec<_> = v.aircraft.iter().filter(|a| mixed::is_live(a.id)).collect();
        assert_eq!(drawn.len(), 2);
        assert!(drawn.iter().all(|a| a.id & ID_BIT != 0));
        // Beside them only what the mix admits (Q13 (c)): no simulated
        // airliner.
        assert!(
            v.aircraft
                .iter()
                .all(|a| mixed::is_live(a.id) || a.kind != AirKind::Airliner)
        );
        assert!(
            v.aircraft
                .iter()
                .any(|a| a.kind == AirKind::Airliner && a.gear_down),
            "descending low: on approach"
        );
        assert_eq!(
            v.attribution.as_deref(),
            Some("Aircraft data: adsb.lol (ODbL)")
        );
        assert!(!v.heard.is_empty());
        // The time machine elsewhere: simulated, no credit.
        let night = AirFrame {
            clock: Some((t + 6 * 3_600_000, 0)),
            ..f
        };
        assert_eq!(AirClient::live_state(&night), Some(LiveState::OtherTime));
        assert!(c.frame(&night).unwrap().attribution.is_none());
        // The feed down: the simulated sky at once (18E T10: < 5 s).
        live.on_update(&v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Down as i32,
            note: "adsb.lol: no answer".into(),
            ..Default::default()
        });
        let f = AirFrame {
            live: Some(&live),
            ..base
        };
        assert_eq!(AirClient::live_state(&f), Some(LiveState::Down));
        let v = c.frame(&f).unwrap();
        assert_eq!(v.aircraft, sim.aircraft);
        assert!(v.attribution.is_none());
        // Simulated chosen: no live state at all.
        assert_eq!(AirClient::live_state(&frame(ViewChoice::default())), None);
    }

    #[test]
    fn sound_off_is_seen_not_heard() {
        let mut c = client();
        let on = ViewChoice::default();
        let quiet = c
            .frame(&frame(ViewChoice { sound: false, ..on }))
            .expect("drawn");
        assert!(quiet.heard.is_empty());
        assert!(!quiet.aircraft.is_empty());
    }

    #[test]
    fn two_clients_see_the_same_sky() {
        let (mut a, mut b) = (client(), client());
        let f = frame(ViewChoice::default());
        assert_eq!(a.frame(&f), b.frame(&f));
    }
}

/// Live and simulated aircraft together (spec 14 Phase 18 Q13 (c)): a
/// headless sky over the Black Forest with a fake feed – the server's
/// `LiveAirFeed` answering from made-up adsb.lol JSON, no network.
#[cfg(test)]
mod mixed_tests {
    use super::*;
    use ocw_core::traffic::Ufos;
    use ocw_protocol::v1;
    use ocw_server::live_air::{AdsbLol, Answer, FnAirFetcher, LiveAirFeed};
    use ocw_sim::air::Mission;
    use ocw_sim::air::live::destination;
    use ocw_sim::air::mixed::{Share, is_live, share};
    use ocw_weather::state::CloudType;
    use ocw_world::aero::{Aero, Aerodrome, AerodromeKind, Helipad};

    /// The Black Forest world's origin.
    const PLACE: (f64, f64) = (47.92, 7.96);
    /// 2026-07-15 13:00 UTC, 15:00 in the Black Forest: a soaring
    /// afternoon.
    const T0: i64 = 1_784_120_400_000;
    const SEED: u64 = 2026;

    fn aero() -> Aero {
        let ad = |name: &str, kind, pos: [f32; 2]| Aerodrome {
            osm_id: 1,
            name: name.into(),
            icao: String::new(),
            iata: String::new(),
            kind,
            lat_lon: (0.0, 0.0),
            pos,
            ele_m: Some(600.0),
            outside: false,
        };
        let pad = |name: &str, pos, hospital| Helipad {
            osm_id: 2,
            name: name.into(),
            pos,
            ele_m: Some(450.0),
            hospital,
            heliport: !hospital,
        };
        Aero {
            aerodromes: vec![
                ad(
                    "Hill Gliding Club",
                    AerodromeKind::Gliding,
                    [-6000.0, -9000.0],
                ),
                ad(
                    "Ridge Gliding Club",
                    AerodromeKind::Gliding,
                    [9000.0, 7000.0],
                ),
                ad("Meadow Field", AerodromeKind::Airfield, [-11_000.0, 4000.0]),
                ad("Valley Field", AerodromeKind::Airfield, [7000.0, -13_000.0]),
            ],
            runways: Vec::new(),
            helipads: vec![
                pad("Valley Hospital", [3000.0, -2000.0], true),
                pad("Town Hospital", [-8000.0, 12_000.0], true),
                pad("Heliport", [-4000.0, 8000.0], false),
                pad("Lake Heliport", [12_000.0, -3000.0], false),
            ],
        }
    }

    fn client() -> AirClient {
        let mut c = AirClient::default();
        c.set_space(7, || {
            AirSpace::new(
                PLACE,
                Some(&aero()),
                ocw_sim::air::relief::Relief::default(),
                |_, _| None,
            )
        });
        c
    }

    fn summer() -> WeatherState {
        WeatherState {
            cloud_cover: 0.3,
            cloud_type: CloudType::Cumulus,
            cloud_base_m: 1600.0,
            wind_mps: 2.0,
            wind_from_deg: 40.0,
            temperature_c: 24.0,
            humidity: 0.5,
            ..Default::default()
        }
    }

    fn frame<'a>(
        t_ms: i64,
        air: AirTraffic,
        weather: &'a WeatherState,
        live: Option<&'a LiveAirClient>,
    ) -> AirFrame<'a> {
        AirFrame {
            choice: ViewChoice {
                air,
                ufo: Ufos::Off,
                ..ViewChoice::default()
            },
            seed: SEED,
            clock: Some((t_ms, 7200)),
            weather: Some(weather),
            eye: [0.0, 700.0, 0.0],
            forward: [0.0, -1.0],
            ground_m: 600.0,
            max_aircraft: 60,
            contrails: true,
            live,
        }
    }

    /// One aircraft of a made-up adsb.lol answer, `km` from the place on
    /// bearing `bearing`.
    fn ac(n: u32, category: &str, km: f64, bearing: f64, alt_ft: f64, kt: f64) -> String {
        let (lat, lon) = destination(PLACE.0, PLACE.1, bearing, km * 1000.0);
        format!(
            r#"{{"hex":"f2{n:04x}","flight":"TEST{n:02}","category":"{category}","alt_geom":{alt_ft},"gs":{kt},"track":{},"lat":{lat:.6},"lon":{lon:.6},"seen_pos":0.0}}"#,
            (bearing + 90.0) % 360.0
        )
    }

    /// Three airliners high up and one helicopter `heli_km` away.
    fn answer(heli_km: Option<f64>) -> String {
        let mut list = vec![
            ac(1, "A3", 18.0, 250.0, 36_000.0, 450.0),
            ac(2, "A3", 34.0, 20.0, 38_000.0, 460.0),
            ac(3, "A5", 52.0, 140.0, 34_000.0, 470.0),
        ];
        if let Some(km) = heli_km {
            list.push(ac(4, "A7", km, 300.0, 3600.0, 0.0));
        }
        format!(r#"{{"ac":[{}],"now":0}}"#, list.join(","))
    }

    /// The fake feed: the server's feed with a fetcher that answers from
    /// `body` (changed between fetches by the test).
    struct Feed {
        feed: LiveAirFeed,
        body: std::sync::Arc<std::sync::Mutex<String>>,
        fetcher: Box<dyn ocw_server::live_air::AirFetcher>,
    }

    impl Feed {
        fn new() -> Self {
            let body = std::sync::Arc::new(std::sync::Mutex::new(answer(None)));
            let b = body.clone();
            Self {
                feed: LiveAirFeed::new(Box::new(AdsbLol::default()), 7),
                fetcher: Box::new(FnAirFetcher::new(move |_| {
                    Some(Answer {
                        result: Ok((200, b.lock().unwrap().clone())),
                        ..Default::default()
                    })
                })),
                body,
            }
        }

        /// A fetch at `t_ms` answering `body`.
        fn fetch(&mut self, t_ms: i64, body: String) -> v1::LiveAirUpdate {
            *self.body.lock().unwrap() = body;
            self.feed
                .step(t_ms, true, PLACE, Some(self.fetcher.as_mut()))
                .expect("an update")
        }
    }

    fn kinds(v: &AirView, live: bool) -> Vec<AirKind> {
        v.aircraft
            .iter()
            .filter(|a| is_live(a.id) == live)
            .map(|a| a.kind)
            .collect()
    }

    /// The scene of Q13 (c): a fake feed with airliners and one
    /// helicopter far away – the sky shows the live airliners and the
    /// helicopter plus the simulated gliders; no simulated airliner; the
    /// status counts both.
    #[test]
    fn live_airliners_fly_with_simulated_gliders() {
        let w = summer();
        let mut c = client();
        let sim = c
            .frame(&frame(T0, AirTraffic::Simulated, &w, None))
            .unwrap();
        let gliders = |v: &AirView| {
            v.aircraft
                .iter()
                .filter(|a| a.kind == AirKind::Glider)
                .map(|a| a.id)
                .collect::<Vec<_>>()
        };
        assert!(!gliders(&sim).is_empty(), "a soaring afternoon");
        assert!(sim.aircraft.iter().any(|a| a.kind == AirKind::Airliner));
        let mut feed = Feed::new();
        let mut live = LiveAirClient::default();
        live.on_update(&feed.fetch(T0 - 5000, answer(Some(45.0))));
        let f = frame(T0, AirTraffic::Live, &w, Some(&live));
        assert_eq!(AirClient::live_state(&f), Some(LiveState::Live));
        let v = c.frame(&f).unwrap();
        let mut seen = kinds(&v, true);
        seen.sort();
        assert_eq!(
            seen,
            [
                AirKind::Airliner,
                AirKind::Airliner,
                AirKind::Airliner,
                AirKind::Helicopter
            ],
            "the feed's aircraft"
        );
        assert_eq!(gliders(&v), gliders(&sim), "the simulated gliders stay");
        assert!(
            !kinds(&v, false).contains(&AirKind::Airliner),
            "no simulated airliner beside the live ones"
        );
        assert_eq!(
            v.attribution.as_deref(),
            Some("Aircraft data: adsb.lol (ODbL)")
        );
        // Every simulated aircraft drawn is one of the simulated sky, in
        // the same place.
        for a in v.aircraft.iter().filter(|a| !is_live(a.id)) {
            assert!(sim.aircraft.contains(a), "{:?}", a.kind);
        }
        // Heard: both kinds of sources, the nearest first.
        assert!(
            v.heard
                .windows(2)
                .all(|p| p[0].distance_m <= p[1].distance_m)
        );
        // The status.
        let st = c.mix_status(&f, &w).expect("live");
        assert_eq!(st.live, [3, 0, 1, 0, 0]);
        assert_eq!(st.simulated[0], 0);
        assert_eq!(st.simulated[3], gliders(&sim).len());
        assert_eq!(st.busy, [false, false], "the helicopter is 45 km away");
        let lines = st.lines().join("\n");
        assert!(
            lines.contains("live aircraft: 3 airliners, 1 helicopter"),
            "{lines}"
        );
        assert!(lines.contains("simulated with them: "), "{lines}");
        assert!(lines.contains("glider"), "{lines}");
        assert!(
            lines.contains("helicopters: none live within 30 km"),
            "{lines}"
        );
        assert!(lines.contains("airliners: live only"), "{lines}");
        // Simulated chosen: no status.
        assert!(
            c.mix_status(&frame(T0, AirTraffic::Simulated, &w, None), &w)
                .is_none()
        );
    }

    /// Every simulated flight between `from` and `to` whose kind waits
    /// for a quiet feed.
    fn quiet_flights(c: &mut AirClient, w: &WeatherState, from: i64, to: i64) -> Vec<Arc<Flight>> {
        let mut all: Vec<Arc<Flight>> = Vec::new();
        let mut t = from;
        while t <= to {
            for f in c
                .flights(&frame(t, AirTraffic::Simulated, w, None), w)
                .unwrap()
            {
                if share(f.kind, f.mission) == Share::Quiet && !all.iter().any(|a| a.id == f.id) {
                    all.push(f);
                }
            }
            t += 30_000;
        }
        all
    }

    /// The helicopter at 31 km: simulated helicopters start; at 29 km:
    /// none starts, the one already flying finishes; after it left the
    /// one that did not start does not appear – and two clients (one of
    /// them joining late) see the same sky all along.
    #[test]
    fn simulated_helicopters_wait_for_a_quiet_feed_on_every_client() {
        let w = summer();
        // A helicopter flight of the simulated afternoon and when it starts.
        let mut scout = client();
        let all = quiet_flights(&mut scout, &w, T0 - 3 * 3_600_000, T0 + 2 * 3_600_000);
        let heli = all
            .iter()
            .filter(|f| f.kind == AirKind::Helicopter)
            .min_by(|a, b| a.start_s.total_cmp(&b.start_s))
            .expect("a simulated helicopter in five hours")
            .clone();
        assert!(all.iter().any(|f| f.mission == Mission::CrossCountry));
        let start = (heli.start_s * 1000.0) as i64;
        let mid = start + 60_000;
        let flies = |c: &mut AirClient, live: &LiveAirClient, t: i64| {
            c.flights(&frame(t, AirTraffic::Live, &w, Some(live)), &w)
                .unwrap()
                .iter()
                .any(|f| f.id == heli.id)
        };
        // 31 km away when it starts: it flies.
        let mut feed = Feed::new();
        let mut live = LiveAirClient::default();
        live.on_update(&feed.fetch(start - 30_000, answer(Some(31.0))));
        assert!(
            flies(&mut client(), &live, mid),
            "31 km: simulated ones fly"
        );
        // 29 km away when it starts: it does not – on a client that was
        // there (a) and on one that joins a minute later (b).
        let mut feed = Feed::new();
        let (mut la, mut lb) = (LiveAirClient::default(), LiveAirClient::default());
        let (mut a, mut b) = (client(), client());
        let u = feed.fetch(start - 30_000, answer(Some(29.0)));
        assert_eq!(u.sim_busy.len(), 1);
        la.on_update(&u);
        assert!(!flies(&mut a, &la, start + 10_000), "29 km: none starts");
        // It leaves; the next fetch says so.
        let u = feed.fetch(start + 40_000, answer(None));
        la.on_update(&u);
        lb.on_update(&feed.feed.current(mid, true).expect("the last fetch"));
        let mut compared = 0;
        for t in (mid..mid + 360_000).step_by(45_000) {
            let fa = frame(t, AirTraffic::Live, &w, Some(&la));
            let fb = frame(t, AirTraffic::Live, &w, Some(&lb));
            assert_eq!(AirClient::live_state(&fa), Some(LiveState::Live));
            assert!(!flies(&mut a, &la, t), "no pop-in after the live one left");
            assert_eq!(a.frame(&fa), b.frame(&fb), "the same sky at {t}");
            compared += 1;
        }
        assert!(compared >= 6);
        // Already flying when the live one turns up: it finishes.
        let mut feed = Feed::new();
        let mut live = LiveAirClient::default();
        let mut c = client();
        live.on_update(&feed.fetch(start - 30_000, answer(None)));
        assert!(flies(&mut c, &live, start + 20_000));
        live.on_update(&feed.fetch(start + 30_000, answer(Some(12.0))));
        let end = (heli.end_s() * 1000.0) as i64;
        let late = (start + 300_000).min(end - 1000);
        assert!(flies(&mut c, &live, late), "it is not taken out of the sky");
        // … also for a rider who joins now.
        let mut joined = LiveAirClient::default();
        joined.on_update(&feed.feed.current(late, true).expect("the last fetch"));
        assert!(flies(&mut client(), &joined, late));
        // … and one that starts now does not fly.
        let ledger = live.ledger(start + 31_000).unwrap();
        assert!(ledger.busy_at(AirKind::Helicopter, start + 31_000));
        assert!(!ledger.busy_at(AirKind::SmallPlane, start + 31_000));
    }

    /// The fallback is the simulated sky as before: the feed down, too
    /// old, or the clock elsewhere – every simulated flight, airliners
    /// too, no credit.
    #[test]
    fn a_stale_or_failed_feed_gives_the_whole_simulated_sky() {
        let w = summer();
        let mut feed = Feed::new();
        let mut live = LiveAirClient::default();
        live.on_update(&feed.fetch(T0 - 5000, answer(Some(20.0))));
        let mut c = client();
        let mixed = c
            .frame(&frame(T0, AirTraffic::Live, &w, Some(&live)))
            .unwrap();
        assert!(mixed.attribution.is_some());
        // Eight minutes later without a fetch: too old.
        let t = T0 + 480_000;
        let sim = client()
            .frame(&frame(t, AirTraffic::Simulated, &w, None))
            .unwrap();
        let f = frame(t, AirTraffic::Live, &w, Some(&live));
        assert_eq!(AirClient::live_state(&f), Some(LiveState::Stale));
        let v = c.frame(&f).unwrap();
        assert_eq!(v, sim, "the simulated sky, unchanged");
        assert!(v.aircraft.iter().any(|a| a.kind == AirKind::Airliner));
        // The feed down.
        live.on_update(&v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Down as i32,
            ..Default::default()
        });
        let f = frame(t, AirTraffic::Live, &w, Some(&live));
        assert_eq!(c.frame(&f).unwrap(), sim);
        assert!(c.mix_status(&f, &w).is_none());
    }

    /// No simulated flight starts next to a live aircraft: a live small
    /// plane low over the gliding club when a glider would start.
    #[test]
    fn no_simulated_flight_starts_beside_a_live_aircraft() {
        let w = summer();
        let mut scout = client();
        let from = T0 - 2 * 3_600_000;
        let mut gliders: Vec<Arc<Flight>> = Vec::new();
        let mut t = from;
        while t < T0 + 3_600_000 {
            for f in scout
                .flights(&frame(t, AirTraffic::Simulated, &w, None), &w)
                .unwrap()
            {
                if f.kind == AirKind::Glider && !gliders.iter().any(|g| g.id == f.id) {
                    gliders.push(f);
                }
            }
            t += 30_000;
        }
        let g = gliders
            .iter()
            .find(|g| g.start_s * 1000.0 > from as f64 + 60_000.0)
            .expect("a glider starting in three hours")
            .clone();
        let start = (g.start_s * 1000.0) as i64;
        let p = g.state(g.start_s).pos;
        let proj = LocalProjection::new(PLACE.0, PLACE.1);
        // A live small plane 2 km east of where it starts, 300 m above.
        let (lat, lon) = proj.unproject([p[0] + 2000.0, p[2]]);
        let near = LiveAircraft {
            id: 9,
            kind: AirKind::SmallPlane,
            lat,
            lon,
            alt_m: p[1] + 300.0,
            speed_mps: 0.0,
            track_deg: 0.0,
            climb_mps: 0.0,
            at_ms: start - 10_000,
        };
        let update = |a: &LiveAircraft| v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Live as i32,
            fetched_ms: start - 10_000,
            aircraft: vec![ocw_protocol::live_air::aircraft_to_wire(a, start - 10_000)],
            ..Default::default()
        };
        let mut live = LiveAirClient::default();
        live.on_update(&update(&near));
        let flies = |c: &mut AirClient, live: &LiveAirClient, t: i64| {
            c.flights(&frame(t, AirTraffic::Live, &w, Some(live)), &w)
                .unwrap()
                .iter()
                .any(|f| f.id == g.id)
        };
        let (mut a, mut b) = (client(), client());
        assert!(!flies(&mut a, &live, start + 2000), "2 km and 300 m away");
        assert!(
            !flies(&mut b, &live, start + 50_000),
            "another client alike"
        );
        assert!(
            !flies(&mut a, &live, start + 200_000),
            "for its whole flight"
        );
        // 8 km away: it flies.
        let (lat, lon) = proj.unproject([p[0] + 8000.0, p[2]]);
        let mut far = LiveAirClient::default();
        far.on_update(&update(&LiveAircraft { lat, lon, ..near }));
        assert!(flies(&mut client(), &far, start + 2000));
        // High above (an airliner at 11 km): it flies.
        let mut high = LiveAirClient::default();
        high.on_update(&update(&LiveAircraft {
            alt_m: 11_000.0,
            ..near
        }));
        assert!(flies(&mut client(), &high, start + 2000));
    }

    /// All share the budget: the nearest, live or simulated.
    #[test]
    fn the_budget_holds_for_the_mixed_sky() {
        let w = summer();
        let mut feed = Feed::new();
        let mut live = LiveAirClient::default();
        live.on_update(&feed.fetch(T0 - 5000, answer(Some(45.0))));
        let mut c = client();
        let all = c
            .frame(&frame(T0, AirTraffic::Live, &w, Some(&live)))
            .unwrap();
        assert!(all.aircraft.len() > 3);
        let few = c
            .frame(&AirFrame {
                max_aircraft: 3,
                ..frame(T0, AirTraffic::Live, &w, Some(&live))
            })
            .unwrap();
        assert_eq!(few.aircraft, all.aircraft[..3]);
        let d = |a: &ocw_sim::air::AircraftNow| a.pos[0].hypot(a.pos[2]);
        assert!(all.aircraft.windows(2).all(|p| d(&p[0]) <= d(&p[1])));
    }
}
