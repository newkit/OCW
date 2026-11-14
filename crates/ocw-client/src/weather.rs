//! The session's weather on the client (spec §14 Phase 9, S4): the
//! server's weather from the welcome and the snapshots, approached
//! smoothly ([`ocw_weather::WeatherFollower`]), the lightning strikes in
//! client time (each once), the gusts on the server's session time, and
//! the [`WeatherView`] for the renderer and the sound. Shared by the
//! desktop and the browser app.

use std::collections::VecDeque;

use ocw_protocol::{WeatherInfo, v1};
use ocw_render::weather::{StrikeView, WeatherView};
use ocw_weather::{Ground, Strike, WeatherFollower, WeatherState};

use crate::client::ClientCore;

/// Strikes are kept this long after their flash (s of client time): the
/// thunder of a strike 15 km away arrives 44 s later.
pub const STRIKE_MEMORY_S: f64 = 60.0;

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct WeatherClient {
    follower: WeatherFollower,
    info: Option<WeatherInfo>,
    /// Client time minus session time (s), smoothed.
    offset: Option<f64>,
    strikes: Vec<StrikeView>,
    seen: VecDeque<u32>,
}

impl WeatherClient {
    /// A weather message received at client time `now_s`, `age_s` old.
    pub fn on_message(&mut self, w: &v1::SessionWeather, now_s: f64, age_s: f64) {
        let info = ocw_protocol::weather_info(w);
        self.follower
            .on_server(info.now, info.ground, info.revision, info.immediate, now_s);
        let measured = now_s - age_s - info.session_s;
        let offset = match self.offset {
            Some(o) if (measured - o).abs() < 1.0 => o + 0.1 * (measured - o),
            _ => measured,
        };
        self.offset = Some(offset);
        for s in &info.strikes {
            if self.seen.contains(&s.id) {
                continue;
            }
            self.seen.push_back(s.id);
            if self.seen.len() > 512 {
                self.seen.pop_front();
            }
            self.strikes.push(StrikeView {
                strike: *s,
                flash_at_s: s.at_s + offset,
            });
        }
        self.strikes
            .retain(|s| s.flash_at_s > now_s - STRIKE_MEMORY_S);
        self.info = Some(info);
    }

    /// The server's last weather message.
    pub fn info(&self) -> Option<&WeatherInfo> {
        self.info.as_ref()
    }

    /// The weather and the ground at client time `now_s`.
    pub fn now(&mut self, now_s: f64) -> Option<(WeatherState, Ground)> {
        self.follower.now(now_s)
    }

    /// The server's session time at client time `now_s`.
    pub fn session_time(&self, now_s: f64) -> Option<f64> {
        self.offset.map(|o| now_s - o)
    }

    /// Strikes of the last minute, with their flash in client time.
    pub fn strikes(&self) -> &[StrikeView] {
        &self.strikes
    }

    /// The renderer's view at client time `now_s`.
    pub fn view(&mut self, now_s: f64) -> Option<WeatherView> {
        let (state, ground) = self.now(now_s)?;
        let seed = self.info.as_ref().map_or(0, |i| i.seed);
        let session_s = self.session_time(now_s).unwrap_or(now_s);
        let gust = ocw_weather::wind::gust(seed, session_s);
        Some(WeatherView {
            wind_mps: state.wind_mps + (state.gust_mps - state.wind_mps).max(0.0) * gust,
            gust,
            summary: state.hud(),
            strikes: self.strikes.clone(),
            session_s,
            state,
            ground,
        })
    }
}

/// The weather for the renderer (`RideView::weather`) at client time
/// `now_s`; `None` until the server's weather is known.
pub fn weather_view(client: &mut ClientCore, now_s: f64) -> Option<WeatherView> {
    client.weather_mut().view(now_s)
}

/// The request a `weather …` command sends (`None`: nothing to ask the
/// server: showing, listing). Field changes apply to `base` – the target
/// of a running transition, else the weather now.
pub fn request_for(
    cmd: &ocw_weather::command::WeatherCommand,
    base: WeatherState,
) -> Option<ocw_protocol::WeatherChange> {
    use ocw_protocol::WeatherChange;
    use ocw_weather::command::WeatherCommand as W;
    Some(match cmd {
        W::Show | W::List => return None,
        W::Preset {
            state,
            transition_s,
            ..
        } => WeatherChange {
            target: Some(*state),
            transition_s: *transition_s,
            settle_ground: true,
            ..Default::default()
        },
        W::Set {
            fields,
            ground,
            transition_s,
        } => WeatherChange {
            target: (!fields.is_empty()).then(|| ocw_weather::command::apply(fields, base)),
            transition_s: *transition_s,
            ground: *ground,
            ..Default::default()
        },
        W::Simulate { seed } => WeatherChange {
            simulate: Some(seed.unwrap_or(0)),
            ..Default::default()
        },
        W::Strike {
            distance_m,
            bearing_deg,
        } => WeatherChange {
            strike: Some((*distance_m, bearing_deg.unwrap_or(25.0))),
            ..Default::default()
        },
    })
}

/// The base a field change applies to: a transition's target, else now.
pub fn change_base(client: &mut ClientCore, now_s: f64) -> WeatherState {
    let target = client.weather().info().and_then(|i| i.target.map(|t| t.0));
    target
        .or_else(|| client.weather_mut().now(now_s).map(|w| w.0))
        .unwrap_or_default()
}

/// `weather` without arguments: the weather now, the ground, the mode and
/// who sets it.
pub fn describe(client: &mut ClientCore, now_s: f64) -> Result<String, String> {
    let control = client.weather_control();
    let wc = client.weather_mut();
    let Some((w, g)) = wc.now(now_s) else {
        return Err("no weather yet (the server sends it when you join)".into());
    };
    let info = wc.info();
    let mode = info.map_or("fixed", |i| i.mode.name());
    let transition = info
        .and_then(|i| i.target)
        .map(|(t, left)| format!(" -> {} in {left:.0} s", t.condition().label()))
        .unwrap_or_default();
    Ok(format!(
        "{}{transition} ({mode}); road wet {:.0} %, puddles {:.0} %, snow cover {:.0} %; set by {control}",
        w.summary(),
        g.wetness() * 100.0,
        g.puddles() * 100.0,
        g.snow_cover() * 100.0,
    ))
}

/// A strike in client time (tests).
pub fn strike_view(strike: Strike, flash_at_s: f64) -> StrikeView {
    StrikeView { strike, flash_at_s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::RiderProfile;
    use ocw_server::local::SteppedServer;
    use ocw_weather::presets::preset;
    use ocw_world::Route;

    /// The weather end to end (spec §14 Phase 9, S4): the server's
    /// weather reaches the client; the client's `weather` request changes
    /// it for everyone; a strike arrives once, in client time.
    #[test]
    fn the_servers_weather_reaches_the_client_and_requests_change_it() {
        let mut client = ClientCore::new(Route::test_loop(), "Rain", RiderProfile::default());
        assert!(weather_view(&mut client, 0.0).is_none());
        let mut server = SteppedServer::new(Route::test_loop(), 0, 0.0);
        server.send(&client.hello());
        let mut t = 0.0;
        let pump = |client: &mut ClientCore, server: &mut SteppedServer, secs: f64, t: &mut f64| {
            let steps = (secs * 50.0) as usize;
            for _ in 0..steps.max(1) {
                server.tick();
                *t += 0.02;
                for bytes in server.drain() {
                    client.on_server_message(&bytes, *t);
                }
            }
        };
        pump(&mut client, &mut server, 0.2, &mut t);
        let v = weather_view(&mut client, t).expect("known after the welcome");
        assert_eq!(v.state, preset("clear").unwrap());
        assert!(client.may_change_weather(), "solo: the rider is the host");
        let change = ocw_protocol::WeatherChange {
            target: Some(preset("heavy-rain").unwrap()),
            settle_ground: true,
            ..Default::default()
        };
        server.send(&ClientCore::weather_message(&change));
        pump(&mut client, &mut server, 0.3, &mut t);
        let v = weather_view(&mut client, t).unwrap();
        assert_eq!(
            v.state,
            preset("heavy-rain").unwrap(),
            "set at once: no easing"
        );
        assert!(v.ground.puddles() > 0.9);
        assert!(v.wind_mps >= v.state.wind_mps && v.wind_mps <= v.state.gust_mps);
        // A strike: once, in client time, about now.
        let strike = ocw_protocol::WeatherChange {
            strike: Some((686.0, 0.0)),
            ..Default::default()
        };
        server.send(&ClientCore::weather_message(&strike));
        pump(&mut client, &mut server, 1.0, &mut t);
        let mine: Vec<&StrikeView> = client
            .weather_mut()
            .strikes()
            .iter()
            .filter(|s| s.strike.distance_m == 686.0)
            .collect();
        assert_eq!(mine.len(), 1, "once, though in many snapshots");
        let s = mine[0];
        assert!(
            (s.flash_at_s - (t - 1.0)).abs() < 0.15,
            "{} vs {t}",
            s.flash_at_s
        );
        assert!((s.strike.thunder_delay_s() - 2.0).abs() < 1e-3);
    }
}
