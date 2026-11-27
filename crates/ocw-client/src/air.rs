//! The client's sky traffic (spec 14 Phase 18, 18A, 18C, T8): the
//! simulated aircraft of `ocw_sim::air` for the rider's own screen and
//! ears – from the session's air seed, the world clock and the weather the
//! server sends, so riders of a group ride who have aircraft on see the
//! same planes in the same places; nothing goes to the server (Q2).
//!
//! `traffic.air`: *off* draws nothing, *simulated* the simulated sky,
//! *live* the simulated sky too until the live feed is built (T10; the
//! menu says "live (simulated)"). The master switch `traffic.enabled`
//! off: nothing. `traffic.sound off`: drawn, not heard.

use std::sync::Arc;

use ocw_core::traffic::{AirTraffic, ViewChoice};
use ocw_render::aircraft::AirView;
use ocw_sim::air::{AirInputs, AirSky, AirSpace};
use ocw_weather::state::WeatherState;

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
        // Live is the simulated sky until T10.
        matches!(choice.air_shown(), AirTraffic::Simulated | AirTraffic::Live)
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
