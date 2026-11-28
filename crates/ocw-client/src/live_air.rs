//! The client's live aircraft (spec 14 Phase 18, 18A, 18D, 18E T10; Q6
//! (b)): the rider's *Aircraft: live* goes to the server
//! (`LiveAirWish`), the server's `LiveAirUpdate`s come back – the aircraft
//! of its last fetch, with their session numbers, classes, positions and
//! motion, no identities – and [`crate::air::AirClient`] draws them
//! together with the simulated flights the update's ledger admits (Q13
//! (c), `ocw_sim::air::mixed`) while they are usable: the update says
//! *live*, the world clock is within the fixes' extrapolation
//! (`ocw_sim::air::live::MAX_AGE_S`; the time machine elsewhere: the
//! simulated sky), and otherwise the simulated sky with a note why
//! ([`LiveState`]). After each fetch every aircraft that was already
//! there glides from where the client extrapolated it to its new place
//! over [`BLEND_MS`].

use std::collections::HashMap;

use ocw_protocol::live_air::{aircraft_from_wire, ledger_from_wire, status, wish};
use ocw_protocol::{encode, v1};
use ocw_sim::air::live::{LiveAircraft, MAX_AGE_S};
use ocw_sim::air::mixed::Ledger;
use ocw_world::geo::EARTH_RADIUS_M;

/// A correction after a new fetch is blended in over this (ms).
pub const BLEND_MS: i64 = 3000;

/// Why live aircraft are or are not drawn now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    /// Drawn: the feed's aircraft.
    Live,
    /// Asked, nothing come yet.
    Waiting,
    /// The feed failed or is paused.
    Down,
    /// The server fetches none (the browser's own server).
    Unavailable,
    /// The world clock is not at the real time (the time machine).
    OtherTime,
    /// The last fetch is too old to extrapolate.
    Stale,
}

impl LiveState {
    /// For Settings and the `air` / `traffic` commands.
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Waiting => "simulated until the first fetch",
            Self::Down => "simulated: the live feed is down",
            Self::Unavailable => "simulated: not on this server",
            Self::OtherTime => "simulated: live only at the real time",
            Self::Stale => "simulated: the last fetch is too old",
        }
    }
}

/// The last update as received.
#[derive(Debug, Clone, PartialEq)]
struct Received {
    status: v1::LiveAirStatus,
    note: String,
    attribution: String,
    fetched_ms: i64,
    aircraft: Vec<LiveAircraft>,
    /// When the feed showed small planes and helicopters near (Q13 (c)).
    ledger: Ledger,
}

/// A blended correction: (Δlat, Δlon, Δalt) at `from_ms`, fading out.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Blend {
    d: [f64; 3],
    from_ms: i64,
}

/// The live aircraft of the client.
#[derive(Debug, Clone, Default)]
pub struct LiveAirClient {
    last: Option<Received>,
    /// What the server was told (`None`: nothing in this session yet).
    told: Option<bool>,
    blend: HashMap<u32, Blend>,
}

impl LiveAirClient {
    /// A new session: tell the server again, forget the old sky.
    pub fn on_welcome(&mut self) {
        self.told = None;
        self.clear();
    }

    /// Another world: its sky comes with the next fetch.
    pub fn clear(&mut self) {
        self.last = None;
        self.blend.clear();
    }

    /// The `LiveAirWish` to send when the rider's wish changed or the
    /// session is new (`None`: nothing to say).
    pub fn wish_message(&mut self, wanted: bool) -> Option<Vec<u8>> {
        if self.told == Some(wanted) {
            return None;
        }
        // Nothing to take back in a new session.
        let first = self.told.is_none();
        self.told = Some(wanted);
        if !wanted {
            self.clear();
            if first {
                return None;
            }
        }
        Some(encode(&wish(wanted)))
    }

    /// A server's update.
    pub fn on_update(&mut self, u: &v1::LiveAirUpdate) {
        let status = status(u);
        let aircraft: Vec<LiveAircraft> = u
            .aircraft
            .iter()
            .map(|a| aircraft_from_wire(a, u.fetched_ms))
            .collect();
        // Where the client had each aircraft at the new fix's time, against
        // where the fix is: the correction to blend out.
        let mut blend = HashMap::new();
        if let Some(old) = self
            .last
            .as_ref()
            .filter(|o| o.status == v1::LiveAirStatus::Live)
        {
            for a in &aircraft {
                let Some(prev) = old.aircraft.iter().find(|p| p.id == a.id) else {
                    continue;
                };
                let t = u.fetched_ms;
                let (Some(was), Some(is)) = (self.extrapolated(prev, t), a.at(t)) else {
                    continue;
                };
                let d = [was.0 - is.0, was.1 - is.1, was.2 - is.2];
                // A far jump (a new fix after a long gap) is not blended.
                let metres = d[0].abs().max(d[1].abs()) * EARTH_RADIUS_M.to_radians();
                if metres < 5000.0 {
                    blend.insert(a.id, Blend { d, from_ms: t });
                }
            }
        }
        self.blend = blend;
        self.last = Some(Received {
            status,
            note: u.note.clone(),
            attribution: u.attribution.clone(),
            fetched_ms: u.fetched_ms,
            aircraft,
            ledger: ledger_from_wire(&u.sim_busy),
        });
    }

    /// `prev` at `t_ms` with its own blend.
    fn extrapolated(&self, prev: &LiveAircraft, t_ms: i64) -> Option<(f64, f64, f64)> {
        let (lat, lon, alt) = prev.at(t_ms)?;
        let o = self.offset_deg(prev.id, t_ms);
        Some((lat + o[0], lon + o[1], alt + o[2]))
    }

    fn offset_deg(&self, id: u32, t_ms: i64) -> [f64; 3] {
        match self.blend.get(&id) {
            Some(b) => {
                let w = 1.0 - ((t_ms - b.from_ms) as f64 / BLEND_MS as f64).clamp(0.0, 1.0);
                [b.d[0] * w, b.d[1] * w, b.d[2] * w]
            }
            None => [0.0; 3],
        }
    }

    /// The blend of aircraft `id` at `t_ms` in the world's metres (x east,
    /// y up, z south) near latitude `lat`.
    pub fn offset_m(&self, id: u32, t_ms: i64, lat: f64) -> [f64; 3] {
        let o = self.offset_deg(id, t_ms);
        let k = EARTH_RADIUS_M.to_radians();
        [o[1] * k * lat.to_radians().cos(), o[2], -o[0] * k]
    }

    /// The state at world time `t_ms` while the rider wants live aircraft.
    pub fn state(&self, t_ms: i64) -> LiveState {
        let Some(r) = &self.last else {
            return LiveState::Waiting;
        };
        match r.status {
            v1::LiveAirStatus::Down => LiveState::Down,
            v1::LiveAirStatus::Unavailable => LiveState::Unavailable,
            v1::LiveAirStatus::Live => {
                let age = (t_ms - r.fetched_ms) as f64 / 1000.0;
                // The world clock follows the real one within a few
                // seconds; far before the fetch or hours after it is the
                // time machine.
                if !(-120.0..=3600.0).contains(&age) {
                    LiveState::OtherTime
                } else if age > MAX_AGE_S {
                    LiveState::Stale
                } else {
                    LiveState::Live
                }
            }
        }
    }

    /// The aircraft to draw at `t_ms`, if live aircraft are usable.
    pub fn aircraft(&self, t_ms: i64) -> Option<&[LiveAircraft]> {
        (self.state(t_ms) == LiveState::Live)
            .then(|| self.last.as_ref().map(|r| r.aircraft.as_slice()))
            .flatten()
    }

    /// The server's ledger of the update drawn at `t_ms` (spec 14 Phase 18
    /// Q13 (c)): which simulated small planes and helicopters fly beside
    /// the live aircraft (`None`: live aircraft are not usable).
    pub fn ledger(&self, t_ms: i64) -> Option<&Ledger> {
        (self.state(t_ms) == LiveState::Live)
            .then(|| self.last.as_ref().map(|r| &r.ledger))
            .flatten()
    }

    /// The source's credit (shown while live aircraft are drawn).
    pub fn attribution(&self) -> Option<&str> {
        self.last
            .as_ref()
            .map(|r| r.attribution.as_str())
            .filter(|a| !a.is_empty())
    }

    /// The server's word on why there are none (`""`: none given).
    pub fn note(&self) -> &str {
        self.last.as_ref().map_or("", |r| r.note.as_str())
    }

    /// When the aircraft drawn were fetched (Unix ms).
    pub fn fetched_ms(&self) -> Option<i64> {
        self.last.as_ref().map(|r| r.fetched_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::live_air::aircraft_to_wire;
    use ocw_sim::air::AirKind;

    fn plane(id: u32, lon: f64, at_ms: i64) -> LiveAircraft {
        LiveAircraft {
            id,
            kind: AirKind::Airliner,
            lat: 47.9,
            lon,
            alt_m: 10_000.0,
            speed_mps: 200.0,
            track_deg: 90.0,
            climb_mps: 0.0,
            at_ms,
        }
    }

    fn update(t: i64, planes: &[LiveAircraft]) -> v1::LiveAirUpdate {
        v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Live as i32,
            attribution: "Aircraft data: adsb.lol (ODbL)".into(),
            fetched_ms: t,
            aircraft: planes.iter().map(|p| aircraft_to_wire(p, t)).collect(),
            next_s: 60,
            ..Default::default()
        }
    }

    #[test]
    fn the_wish_is_told_once_per_change_and_session() {
        let mut c = LiveAirClient::default();
        assert!(c.wish_message(false).is_none(), "nothing to take back");
        let on = c.wish_message(true).expect("told");
        let msg = ocw_protocol::decode_client(&on).unwrap();
        assert!(matches!(msg, v1::client_message::Msg::LiveAir(w) if w.on));
        assert!(c.wish_message(true).is_none());
        assert!(c.wish_message(false).is_some());
        c.wish_message(true);
        c.on_welcome();
        assert!(
            c.wish_message(true).is_some(),
            "a new session hears it again"
        );
    }

    #[test]
    fn states_follow_the_update_and_the_clock() {
        let t = 1_800_000_000_000;
        let mut c = LiveAirClient::default();
        assert_eq!(c.state(t), LiveState::Waiting);
        c.on_update(&update(t, &[plane(1, 7.9, t)]));
        assert_eq!(c.state(t + 10_000), LiveState::Live);
        assert_eq!(c.aircraft(t + 10_000).unwrap().len(), 1);
        assert_eq!(c.attribution(), Some("Aircraft data: adsb.lol (ODbL)"));
        assert_eq!(c.state(t + 500_000), LiveState::Stale);
        assert_eq!(c.state(t + 86_400_000), LiveState::OtherTime);
        assert_eq!(c.state(t - 3_600_000), LiveState::OtherTime);
        assert!(c.aircraft(t + 500_000).is_none());
        c.on_update(&v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Down as i32,
            note: "adsb.lol: no answer".into(),
            ..Default::default()
        });
        assert_eq!(c.state(t), LiveState::Down);
        assert!(c.aircraft(t).is_none());
        assert_eq!(c.note(), "adsb.lol: no answer");
        c.on_update(&v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Unavailable as i32,
            ..Default::default()
        });
        assert_eq!(c.state(t), LiveState::Unavailable);
        for s in [
            LiveState::Waiting,
            LiveState::Down,
            LiveState::Unavailable,
            LiveState::OtherTime,
            LiveState::Stale,
        ] {
            assert!(s.label().starts_with("simulated"));
        }
    }

    /// A new fix moves an aircraft smoothly: no jump at the fetch, gone
    /// after [`BLEND_MS`].
    #[test]
    fn corrections_are_blended_out() {
        let t = 1_800_000_000_000;
        let mut c = LiveAirClient::default();
        c.on_update(&update(t, &[plane(1, 7.9, t)]));
        // 60 s later the aircraft is 300 m further east than extrapolated.
        let t2 = t + 60_000;
        let ahead = plane(1, 7.9, t).at(t2).unwrap();
        let k = EARTH_RADIUS_M.to_radians() * 47.9_f64.to_radians().cos();
        let fix = plane(1, ahead.1 + 300.0 / k, t2);
        c.on_update(&update(t2, &[fix]));
        let o = c.offset_m(1, t2, 47.9);
        assert!(
            (o[0] + 300.0).abs() < 2.0,
            "starts where it was drawn: {o:?}"
        );
        let half = c.offset_m(1, t2 + BLEND_MS / 2, 47.9);
        assert!((half[0] + 150.0).abs() < 2.0, "{half:?}");
        assert_eq!(c.offset_m(1, t2 + BLEND_MS, 47.9), [0.0, 0.0, 0.0]);
        assert_eq!(c.offset_m(2, t2, 47.9), [0.0; 3], "a new one: no blend");
    }
}
