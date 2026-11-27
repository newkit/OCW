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
//! drawn the view carries the source's credit. The master switch
//! `traffic.enabled` off: nothing. `traffic.sound off`: drawn, not heard.

use std::sync::Arc;

use ocw_core::traffic::{AirTraffic, ViewChoice};
use ocw_render::aircraft::AirView;
use ocw_sim::air::live::LiveFrame;
use ocw_sim::air::upper::UpperAir;
use ocw_sim::air::{AirInputs, AirSky, AirSpace};
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

/// The client's sky.
#[derive(Debug, Default)]
pub struct AirClient {
    space: Option<(u64, Arc<AirSpace>)>,
    sky: AirSky,
}

impl AirClient {
    /// The world's airspace (built once per world; `key` names it).
    pub fn set_space(&mut self, key: u64, space: impl FnOnce() -> AirSpace) {
        if self.space.as_ref().is_none_or(|s| s.0 != key) {
            self.space = Some((key, Arc::new(space())));
            self.sky = AirSky::default();
        }
    }

    pub fn space(&self) -> Option<&Arc<AirSpace>> {
        self.space.as_ref().map(|s| &s.1)
    }

    /// The sky's flights (the `air` command).
    pub fn sky_mut(&mut self) -> &mut AirSky {
        &mut self.sky
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
        if Self::live_state(f) == Some(LiveState::Live)
            && let Some(live) = f.live
            && let Some(view) = Self::live_frame(&space, live, &inp, f)
        {
            return Some(view);
        }
        let aircraft = self.sky.now(&space, &inp);
        let heard = if f.choice.sound {
            self.sky.heard(&space, &inp, f.eye)
        } else {
            Vec::new()
        };
        Some(AirView {
            aircraft,
            heard,
            listener: f.eye,
            listener_forward: f.forward,
            attribution: None,
        })
    }

    /// The live aircraft at frame `f`: T8's drawing and sound rules (the
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
        assert_eq!(v.aircraft.len(), 2);
        assert!(v.aircraft.iter().all(|a| a.id & ID_BIT != 0));
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
