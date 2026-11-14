//! The session's weather on the wire (spec §14 Phase 9, S4): the state,
//! the ground, the lightning strikes, who may change it, and the weather
//! machine's request.

use ocw_core::RiderId;
use ocw_weather::command::GroundChange;
use ocw_weather::{CloudType, Ground, Mode, Precip, SessionWeather, Strike, WeatherState};

use crate::v1;
use crate::{ClockAccess, access_from_wire, access_to_wire, client_message};

fn cloud_to_wire(t: CloudType) -> i32 {
    (match t {
        CloudType::Cumulus => v1::CloudType::Cumulus,
        CloudType::Stratus => v1::CloudType::Stratus,
        CloudType::Cirrus => v1::CloudType::Cirrus,
        CloudType::Cumulonimbus => v1::CloudType::Cumulonimbus,
    }) as i32
}

fn cloud_from_wire(v: i32) -> CloudType {
    match v1::CloudType::try_from(v) {
        Ok(v1::CloudType::Stratus) => CloudType::Stratus,
        Ok(v1::CloudType::Cirrus) => CloudType::Cirrus,
        Ok(v1::CloudType::Cumulonimbus) => CloudType::Cumulonimbus,
        _ => CloudType::Cumulus,
    }
}

fn precip_to_wire(p: Precip) -> i32 {
    (match p {
        Precip::None => v1::Precipitation::None,
        Precip::Drizzle => v1::Precipitation::Drizzle,
        Precip::Rain => v1::Precipitation::Rain,
        Precip::Snow => v1::Precipitation::Snow,
        Precip::Sleet => v1::Precipitation::Sleet,
        Precip::Hail => v1::Precipitation::Hail,
    }) as i32
}

fn precip_from_wire(v: i32) -> Precip {
    match v1::Precipitation::try_from(v) {
        Ok(v1::Precipitation::Drizzle) => Precip::Drizzle,
        Ok(v1::Precipitation::Rain) => Precip::Rain,
        Ok(v1::Precipitation::Snow) => Precip::Snow,
        Ok(v1::Precipitation::Sleet) => Precip::Sleet,
        Ok(v1::Precipitation::Hail) => Precip::Hail,
        _ => Precip::None,
    }
}

/// A weather state on the wire.
pub fn weather_to_wire(w: &WeatherState) -> v1::Weather {
    v1::Weather {
        cloud_cover: w.cloud_cover,
        cloud_type: cloud_to_wire(w.cloud_type),
        cloud_base_m: w.cloud_base_m,
        precip: precip_to_wire(w.precip),
        precip_mm_h: w.precip_mm_h,
        fog_visibility_m: w.fog_visibility_m,
        wind_mps: w.wind_mps,
        gust_mps: w.gust_mps,
        wind_from_deg: w.wind_from_deg,
        temperature_c: w.temperature_c,
        humidity: w.humidity,
        pressure_hpa: w.pressure_hpa,
        lightning_per_min: w.lightning_per_min,
    }
}

/// A received weather state, clamped to the valid ranges.
pub fn weather_from_wire(w: &v1::Weather) -> WeatherState {
    WeatherState {
        cloud_cover: w.cloud_cover,
        cloud_type: cloud_from_wire(w.cloud_type),
        cloud_base_m: w.cloud_base_m,
        precip: precip_from_wire(w.precip),
        precip_mm_h: w.precip_mm_h,
        fog_visibility_m: w.fog_visibility_m,
        wind_mps: w.wind_mps,
        gust_mps: w.gust_mps,
        wind_from_deg: w.wind_from_deg,
        temperature_c: w.temperature_c,
        humidity: w.humidity,
        pressure_hpa: w.pressure_hpa,
        lightning_per_min: w.lightning_per_min,
    }
    .clamped()
}

fn strike_to_wire(s: &Strike) -> v1::LightningStrike {
    v1::LightningStrike {
        id: s.id,
        at_s: s.at_s,
        distance_m: s.distance_m,
        bearing_deg: s.bearing_deg,
        intensity: s.intensity,
        seed: s.seed,
    }
}

/// The session weather message at session time `t` (s).
pub fn session_weather(w: &SessionWeather, t: f64, access: ClockAccess) -> v1::SessionWeather {
    let g = w.ground();
    let left = w.transition_left_s(t);
    v1::SessionWeather {
        now: Some(weather_to_wire(&w.now(t))),
        transition_left_s: left as f32,
        wetness: g.wetness(),
        puddles: g.puddles(),
        snow_cover: g.snow_cover(),
        revision: w.revision(),
        immediate: w.immediate(),
        mode: match w.mode() {
            Mode::Fixed => v1::WeatherMode::Fixed,
            Mode::Simulated => v1::WeatherMode::Simulated,
        } as i32,
        access: access_to_wire(access),
        seed: w.seed(),
        session_s: t,
        strikes: w.strikes().iter().map(strike_to_wire).collect(),
        target: (left > 0.0).then(|| weather_to_wire(w.target())),
    }
}

/// The session weather as a client sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherInfo {
    pub now: WeatherState,
    /// A transition's target and the seconds left (`None`: none).
    pub target: Option<(WeatherState, f64)>,
    pub ground: Ground,
    pub revision: u32,
    pub immediate: bool,
    pub mode: Mode,
    pub access: ClockAccess,
    pub seed: u64,
    /// Session time of the message (s).
    pub session_s: f64,
    pub strikes: Vec<Strike>,
}

impl WeatherInfo {
    /// Whether rider `me` may change the weather (`host`: the session's
    /// host, from the clock).
    pub fn may_change(&self, me: Option<RiderId>, host: Option<RiderId>) -> bool {
        self.access.allows(me, host)
    }
}

pub fn weather_info(w: &v1::SessionWeather) -> WeatherInfo {
    let now = w
        .now
        .as_ref()
        .map_or_else(WeatherState::default, weather_from_wire);
    WeatherInfo {
        now,
        target: w.target.as_ref().map(|t| {
            (
                weather_from_wire(t),
                f64::from(w.transition_left_s.max(0.0)),
            )
        }),
        ground: Ground::with_factors(w.wetness, w.puddles, w.snow_cover),
        revision: w.revision,
        immediate: w.immediate,
        mode: match v1::WeatherMode::try_from(w.mode) {
            Ok(v1::WeatherMode::Simulated) => Mode::Simulated,
            _ => Mode::Fixed,
        },
        access: access_from_wire(w.access),
        seed: w.seed,
        session_s: w.session_s,
        strikes: w
            .strikes
            .iter()
            .map(|s| Strike {
                id: s.id,
                at_s: s.at_s,
                distance_m: s.distance_m.clamp(0.0, 1e6),
                bearing_deg: s.bearing_deg,
                intensity: s.intensity.clamp(0.0, 1.0),
                seed: s.seed,
            })
            .collect(),
    }
}

/// What the weather machine asks the server for.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeatherChange {
    /// Go to this weather.
    pub target: Option<WeatherState>,
    /// Over this many seconds (0: at once).
    pub transition_s: f64,
    /// A preset set at once also gets its ground.
    pub settle_ground: bool,
    /// The simulation from now on with this seed (0: the server picks).
    pub simulate: Option<u64>,
    pub ground: GroundChange,
    /// A test strike: distance (m) and bearing (°).
    pub strike: Option<(f32, f32)>,
}

/// A weather request (spec §14 Phase 9, S4).
pub fn weather_request(c: &WeatherChange) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Weather(v1::WeatherRequest {
        target: c.target.as_ref().map(weather_to_wire),
        transition_s: c.transition_s as f32,
        settle_ground: c.settle_ground,
        simulate: c.simulate.is_some(),
        seed: c.simulate.unwrap_or(0),
        wetness: c.ground.wetness,
        puddles: c.ground.puddles,
        snow_cover: c.ground.snow_cover,
        strike_distance_m: c.strike.map(|s| s.0),
        strike_bearing_deg: c.strike.map(|s| s.1),
    }))
}

/// The change a [`v1::WeatherRequest`] asks for (values clamped).
pub fn weather_change(r: &v1::WeatherRequest) -> WeatherChange {
    let share = |v: Option<f32>| v.filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0));
    WeatherChange {
        target: r.target.as_ref().map(weather_from_wire),
        transition_s: f64::from(r.transition_s).clamp(0.0, 24.0 * 3600.0).max(0.0),
        settle_ground: r.settle_ground,
        simulate: r.simulate.then_some(r.seed),
        ground: GroundChange {
            wetness: share(r.wetness),
            puddles: share(r.puddles),
            snow_cover: share(r.snow_cover),
        },
        strike: r.strike_distance_m.filter(|d| d.is_finite()).map(|d| {
            (
                d,
                r.strike_bearing_deg
                    .filter(|b| b.is_finite())
                    .unwrap_or(0.0),
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_client, decode_server, encode, server_message};
    use ocw_weather::presets::preset;

    /// The weather through the wire and back (spec §14 Phase 9, S4).
    #[test]
    fn session_weather_round_trip() {
        use v1::server_message::Msg as S;
        let mut w = SessionWeather::new(77);
        w.set(10.0, preset("thunderstorm").unwrap(), 60.0, true);
        w.tick(10.0);
        let s = w.strike(10.0, 2400.0, 135.0);
        let msg = session_weather(&w, 40.0, ClockAccess::Host);
        let snap = v1::Snapshot {
            server_tick: 2000,
            weather: Some(msg),
            ..Default::default()
        };
        let bytes = encode(&server_message(S::Snapshot(snap)));
        let S::Snapshot(back) = decode_server(&bytes).unwrap() else {
            panic!("a snapshot")
        };
        let info = weather_info(&back.weather.unwrap());
        assert_eq!(info.now, w.now(40.0));
        let (target, left) = info.target.unwrap();
        assert_eq!(target, preset("thunderstorm").unwrap());
        assert!((left - 30.0).abs() < 1e-3);
        assert_eq!(info.strikes, vec![s]);
        assert_eq!((info.seed, info.session_s), (77, 40.0));
        assert_eq!(info.mode, Mode::Fixed);
        assert!(!info.immediate);
        assert!(info.may_change(Some(RiderId(2)), Some(RiderId(2))));
        assert!(!info.may_change(Some(RiderId(3)), Some(RiderId(2))));
        // Every preset survives the trip.
        for (name, _) in ocw_weather::presets::PRESETS {
            let p = preset(name).unwrap();
            assert_eq!(weather_from_wire(&weather_to_wire(&p)), p, "{name}");
        }
        // Without a transition: no target.
        let calm = SessionWeather::new(1);
        assert!(
            session_weather(&calm, 0.0, ClockAccess::Anyone)
                .target
                .is_none()
        );
        assert!(
            weather_info(&v1::SessionWeather::default())
                .target
                .is_none()
        );
    }

    #[test]
    fn weather_requests_round_trip_and_are_clamped() {
        use v1::client_message::Msg;
        let c = WeatherChange {
            target: Some(preset("heavy-snow").unwrap()),
            transition_s: 90.0,
            settle_ground: true,
            simulate: None,
            ground: GroundChange {
                wetness: Some(0.5),
                puddles: None,
                snow_cover: Some(1.0),
            },
            strike: Some((800.0, 270.0)),
        };
        let Msg::Weather(r) = decode_client(&encode(&weather_request(&c))).unwrap() else {
            panic!("a weather request")
        };
        assert_eq!(weather_change(&r), c);
        let sim = WeatherChange {
            simulate: Some(9),
            ..Default::default()
        };
        let Msg::Weather(r) = decode_client(&encode(&weather_request(&sim))).unwrap() else {
            panic!()
        };
        assert_eq!(weather_change(&r), sim);
        // Nonsense from the wire is clamped.
        let bad = v1::WeatherRequest {
            target: Some(v1::Weather {
                cloud_cover: 9.0,
                precip_mm_h: f32::INFINITY,
                ..Default::default()
            }),
            transition_s: -5.0,
            wetness: Some(7.0),
            ..Default::default()
        };
        let c = weather_change(&bad);
        let t = c.target.unwrap();
        assert_eq!(t.cloud_cover, 1.0);
        assert_eq!(t.precip_mm_h, 0.0);
        assert_eq!(c.transition_s, 0.0);
        assert_eq!(c.ground.wetness, Some(1.0));
    }
}
