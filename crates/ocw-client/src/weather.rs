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
        W::Random { seed, transition_s } => WeatherChange {
            target: Some(ocw_weather::machine::random(seed.unwrap_or(1)).1),
            transition_s: *transition_s,
            settle_ground: true,
            ..Default::default()
        },
        // The weather it is heading to, from now on fixed (at once here;
        // [`command_request`] lets a running transition finish).
        W::Fixed => WeatherChange {
            target: Some(base),
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

/// The request of a `weather …` command on this client at client time
/// `now_s` (`None`: nothing to send); field changes apply to
/// [`change_base`], `weather fixed` lets a running transition finish, a
/// random weather without a seed gets one from the time.
pub fn command_request(
    client: &mut ClientCore,
    cmd: &ocw_weather::command::WeatherCommand,
    now_s: f64,
) -> Option<ocw_protocol::WeatherChange> {
    use ocw_weather::command::WeatherCommand as W;
    let base = change_base(client, now_s);
    let left = client
        .weather()
        .info()
        .and_then(|i| i.target.map(|t| t.1))
        .unwrap_or(0.0);
    let cmd = match cmd {
        W::Random {
            seed: None,
            transition_s,
        } => W::Random {
            seed: Some(random_seed(now_s)),
            transition_s: *transition_s,
        },
        c => c.clone(),
    };
    let mut change = request_for(&cmd, base)?;
    if cmd == W::Fixed {
        change.transition_s = left;
    }
    Some(change)
}

/// A seed from the client's clock (a random weather without a seed): the
/// same number of milliseconds, the same weather.
pub fn random_seed(now_s: f64) -> u64 {
    ((now_s * 1000.0) as u64 % 100_000).max(1)
}

/// Runs `weather ARGS` on this client (desktop and browser app): the text
/// to print, and the request to send (if any). The server decides; the
/// client checks the permission too, only for a quick message.
pub fn run(
    client: &mut ClientCore,
    args: &[&str],
    now_s: f64,
) -> Result<(String, Option<Vec<u8>>), String> {
    use ocw_weather::command::{self, WeatherCommand as W};
    let cmd = command::parse(args)?;
    match &cmd {
        W::List => return Ok((command::list(), None)),
        W::Show => return describe(client, now_s).map(|s| (s, None)),
        _ => {}
    }
    if client.weather().info().is_none() {
        return Err("no weather yet (the server sends it when you join)".into());
    }
    if !client.may_change_weather() {
        return Err(format!(
            "{} sets the weather of this ride",
            client.weather_control()
        ));
    }
    let Some(change) = command_request(client, &cmd, now_s) else {
        return Ok((String::new(), None));
    };
    // Say which weather a random seed gave (the server answers the rest).
    let said = match (&cmd, change.target) {
        (W::Random { seed, .. }, Some(t)) => {
            let seed = seed.unwrap_or_else(|| random_seed(now_s));
            format!(
                "random weather (seed {seed}): {}",
                ocw_weather::machine::random(seed).0
            ) + &if t.precipitating() {
                format!(", {}", t.hud())
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    Ok((said, Some(ClientCore::weather_message(&change))))
}

/// `weather.list`: the presets with their descriptions, the custom fields
/// and the transitions (spec §14 Phase 9, S5).
pub fn list_long() -> String {
    use ocw_weather::machine::{Field, TRANSITIONS_S, transition_label};
    let mut s = String::from("Presets (weather PRESET [in T]):\n");
    s += &ocw_weather::command::list();
    s += "\nCustom fields (weather FIELD=VALUE... [in T]): ";
    s += &Field::ALL
        .iter()
        .map(|f| f.label().to_lowercase())
        .collect::<Vec<_>>()
        .join(", ");
    s += " (clouds type rain|drizzle|snow|sleet|hail precip=none fog wind gust dir temp)";
    s += "\nTransitions of the menu: ";
    s += &TRANSITIONS_S
        .iter()
        .map(|t| transition_label(*t))
        .collect::<Vec<_>>()
        .join(", ");
    s += "; random weather: weather random SEED; simulation: weather auto SEED";
    s
}

/// `weather.status`: everything about the session's weather – now and
/// where it is going, the mode and seed, visibility, lightning, the
/// ground, who sets it (and an event's lock).
pub fn status(client: &mut ClientCore, now_s: f64) -> Result<String, String> {
    use ocw_weather::machine::{distance_label, transition_label};
    let control = client.weather_control();
    let event = client.event_info().and_then(|e| e.weather.clone());
    let wc = client.weather_mut();
    let Some((w, g)) = wc.now(now_s) else {
        return Err("no weather yet (the server sends it when you join)".into());
    };
    let info = wc.info().cloned();
    let mut lines = vec![format!("now: {}", w.summary())];
    if let Some((t, left)) = info.as_ref().and_then(|i| i.target) {
        lines.push(format!(
            "-> {} in {} ({})",
            t.condition().label(),
            transition_label(left.round()),
            t.summary()
        ));
    }
    let mode = info.as_ref().map_or(ocw_weather::Mode::Fixed, |i| i.mode);
    lines.push(match (mode, info.as_ref().map_or(0, |i| i.sim_seed)) {
        (ocw_weather::Mode::Simulated, seed) => format!("mode: simulated (seed {seed})"),
        _ => "mode: fixed".into(),
    });
    lines.push(format!(
        "clouds {:.0} % {} (base {:.0} m), visibility {}, humidity {:.0} %, {:.0} hPa, lightning {:.1} / min",
        w.cloud_cover * 100.0,
        w.cloud_type.name(),
        w.cloud_base_m,
        distance_label(w.visibility_m()),
        w.humidity * 100.0,
        w.pressure_hpa,
        w.lightning_per_min
    ));
    lines.push(format!(
        "ground: road wet {:.0} %, puddles {:.0} %, snow cover {:.0} %",
        g.wetness() * 100.0,
        g.puddles() * 100.0,
        g.snow_cover() * 100.0
    ));
    lines.push(match event {
        Some(e) => format!("set by {control}: {e} for the event"),
        None => format!("set by {control}"),
    });
    Ok(lines.join("\n"))
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

    fn pump(client: &mut ClientCore, server: &mut SteppedServer, secs: f64, t: &mut f64) {
        let steps = (secs * 50.0) as usize;
        for _ in 0..steps.max(1) {
            server.tick();
            *t += 0.02;
            for bytes in server.drain() {
                client.on_server_message(&bytes, *t);
            }
        }
    }

    /// The weather machine's console lines end to end (spec §14 Phase 9,
    /// S5): a random weather from a seed over a transition, `weather
    /// fixed` after the simulation, `weather.status`, and an event's
    /// weather that nobody can change while it runs.
    #[test]
    fn the_weather_machine_from_the_console_end_to_end() {
        let mut client = ClientCore::new(Route::test_loop(), "Rain", RiderProfile::default());
        let mut server = SteppedServer::new(Route::test_loop(), 0, 0.0);
        server.send(&client.hello());
        let mut t = 0.0;
        assert!(run(&mut client, &["fog"], t).is_err(), "no weather yet");
        pump(&mut client, &mut server, 0.2, &mut t);
        // A random weather: the seed's, over a minute.
        let (said, bytes) = run(&mut client, &["random", "42", "in", "1m"], t).unwrap();
        let (name, w) = ocw_weather::machine::random(42);
        assert!(said.contains("seed 42") && said.contains(name), "{said}");
        server.send(&bytes.unwrap());
        pump(&mut client, &mut server, 0.5, &mut t);
        let info = client.weather().info().unwrap().clone();
        let (target, left) = info.target.expect("a transition");
        assert_eq!(target, w);
        assert!(left > 55.0 && left < 60.0, "{left}");
        // The status names it all.
        let s = status(&mut client, t).unwrap();
        assert!(s.contains("mode: fixed") && s.contains("set by you"), "{s}");
        assert!(s.contains(&format!("-> {}", w.condition().label())), "{s}");
        // The simulation, then fixed: the weather it heads to stays.
        let (_, b) = run(&mut client, &["auto", "9"], t).unwrap();
        server.send(&b.unwrap());
        pump(&mut client, &mut server, 0.5, &mut t);
        let s = status(&mut client, t).unwrap();
        assert!(s.contains("simulated (seed 9)"), "{s}");
        let (_, b) = run(&mut client, &["fixed"], t).unwrap();
        server.send(&b.unwrap());
        pump(&mut client, &mut server, 0.5, &mut t);
        assert_eq!(
            client.weather().info().unwrap().mode,
            ocw_weather::Mode::Fixed
        );
        // Listing and showing need no server.
        assert!(run(&mut client, &["list"], t).unwrap().1.is_none());
        assert!(list_long().contains("Heavy") || list_long().contains("heavy-rain"));
        assert!(list_long().contains("15 min"));
        // An event with the organiser's weather: locked until it is over.
        let ts = ocw_weather::machine::parse_event_weather(&["thunderstorm"]).unwrap();
        server.send(&ClientCore::event_start_message_weather(
            &ocw_sim::EventSpec::Race {
                laps: None,
                length_m: Some(1000.0),
            },
            Some(0),
            None,
            Some(&ts),
        ));
        pump(&mut client, &mut server, 0.5, &mut t);
        assert!(!client.may_change_weather());
        assert_eq!(client.weather_control(), "the event's organizer");
        let err = run(&mut client, &["clear"], t).unwrap_err();
        assert!(err.contains("the event's organizer"), "{err}");
        let s = status(&mut client, t).unwrap();
        assert!(s.contains("thunderstorm for the event"), "{s}");
        assert_eq!(
            client.weather().info().unwrap().now,
            preset("thunderstorm").unwrap()
        );
        // Cancelled: the rider's again.
        server.send(&ClientCore::event_cancel_message());
        pump(&mut client, &mut server, 0.5, &mut t);
        assert!(client.may_change_weather());
        assert!(run(&mut client, &["clear"], t).unwrap().1.is_some());
    }
}
