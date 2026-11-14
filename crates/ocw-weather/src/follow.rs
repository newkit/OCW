//! The session's weather on a client (spec §6.1: only the server's weather
//! counts): the state of the last snapshot, approached smoothly between
//! snapshots (the server's transitions are smooth already; this hides the
//! 10 Hz steps), and taken at once when the host changed it at once.

use crate::ground::Ground;
use crate::state::WeatherState;

/// Time constant of the approach (s).
pub const TAU_S: f32 = 0.5;

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct WeatherFollower {
    shown: Option<(WeatherState, Ground)>,
    target: Option<(WeatherState, Ground)>,
    last_t: f64,
    revision: Option<u32>,
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl WeatherFollower {
    /// The server's weather (`immediate`: its last change was made at
    /// once; a new `revision` of it is taken without easing).
    pub fn on_server(
        &mut self,
        w: WeatherState,
        ground: Ground,
        revision: u32,
        immediate: bool,
        now_s: f64,
    ) {
        let snap = self.shown.is_none() || (immediate && self.revision != Some(revision));
        self.target = Some((w, ground));
        if snap {
            self.shown = Some((w, ground));
            self.last_t = now_s;
        }
        self.revision = Some(revision);
    }

    /// Whether the server's weather arrived yet.
    pub fn known(&self) -> bool {
        self.target.is_some()
    }

    /// The weather at client time `now_s` (`None` before the first
    /// message, or from a server without weather).
    pub fn now(&mut self, now_s: f64) -> Option<(WeatherState, Ground)> {
        let (tw, tg) = self.target?;
        let (w, g) = self.shown.unwrap_or((tw, tg));
        let dt = (now_s - self.last_t).max(0.0) as f32;
        self.last_t = now_s;
        let k = 1.0 - (-dt / TAU_S).exp();
        let mut n = w.lerp(&tw, k);
        // The kinds follow the target at once (a small step never reaches
        // lerp's half-way switch); what stops falling fades out first.
        n.cloud_type = tw.cloud_type;
        if tw.precipitating() {
            n.precip = tw.precip;
        } else if n.precip_mm_h < 0.01 {
            n.precip = tw.precip;
            n.precip_mm_h = 0.0;
        }
        let ng = Ground {
            water_mm: mix(g.water_mm, tg.water_mm, k),
            puddle_mm: mix(g.puddle_mm, tg.puddle_mm, k),
            snow_mm: mix(g.snow_mm, tg.snow_mm, k),
        };
        self.shown = Some((n, ng));
        Some((n, ng))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset;
    use crate::session::SessionWeather;
    use crate::state::Precip;

    #[test]
    fn follows_the_server_smoothly_and_snaps_on_immediate_changes() {
        let mut f = WeatherFollower::default();
        assert!(f.now(0.0).is_none());
        let clear = preset("clear").unwrap();
        f.on_server(clear, Ground::default(), 1, true, 0.0);
        assert_eq!(f.now(0.0).unwrap().0, clear);
        // A transition on the server: snapshots every 0.1 s, the client
        // at 60 Hz stays close and moves smoothly.
        let mut server = SessionWeather::new(1);
        server.set(0.0, preset("rain").unwrap(), 20.0, true);
        let mut next = 0.0;
        let mut prev = clear.cloud_cover;
        for frame in 0..1800 {
            let t = frame as f64 / 60.0;
            while next <= t {
                server.tick(next);
                f.on_server(
                    server.now(next),
                    server.ground(),
                    server.revision(),
                    server.immediate(),
                    next,
                );
                next += 0.1;
            }
            let (w, _) = f.now(t).unwrap();
            assert!(w.cloud_cover >= prev - 1e-6, "monotonic at {t}");
            assert!(
                (w.cloud_cover - server.now(t).cloud_cover).abs() < 0.06,
                "close at {t}"
            );
            prev = w.cloud_cover;
        }
        let (w, _) = f.now(30.0).unwrap();
        assert_eq!(w.precip, Precip::Rain);
        assert!((w.precip_mm_h - 4.0).abs() < 0.05);
        // The host sets fog at once: taken at once.
        server.set(30.0, preset("fog").unwrap(), 0.0, true);
        f.on_server(
            server.now(30.0),
            server.ground(),
            server.revision(),
            server.immediate(),
            30.0,
        );
        assert_eq!(f.now(30.0).unwrap().0, preset("fog").unwrap());
        // The same revision again is no new jump: an eased change after
        // it is eased.
        f.on_server(
            preset("clear").unwrap(),
            Ground::default(),
            server.revision(),
            true,
            30.1,
        );
        let (w, _) = f.now(30.1).unwrap();
        assert!(w.fog_visibility_m < 1000.0, "eased, not snapped");
        // The rain stops: it fades out, then nothing falls.
        let mut g = WeatherFollower::default();
        g.on_server(preset("rain").unwrap(), Ground::default(), 1, true, 0.0);
        g.on_server(clear, Ground::default(), 1, false, 0.0);
        assert_eq!(g.now(0.05).unwrap().0.precip, Precip::Rain);
        assert_eq!(g.now(10.0).unwrap().0.precip, Precip::None);
    }
}
