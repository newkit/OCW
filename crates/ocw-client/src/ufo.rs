//! The client's UFOs (spec 14 Phase 18, 18C *UFOs*, 18E T9; Q7): the
//! sightings and scenes of `ocw_sim::ufo` for the rider's own screen and
//! ears, from the session's air seed, the world clock and the weather – so
//! riders of a group ride who have UFOs on see the same saucers, crop
//! circles and cows; nothing goes to the server (Q2). In god mode the
//! unicorn's visitor comes besides ([`ocw_sim::ufo::curiosity`]).
//!
//! `traffic.ufo` *off* (the default, Q1), the master switch off or a race
//! or event: nothing ([`ViewChoice::ufos_shown`]). `traffic.sound off`:
//! seen, not heard.

use std::sync::Arc;

use ocw_core::traffic::{Ufos, ViewChoice};
use ocw_render::ufo::UfoView;
use ocw_sim::ufo::curiosity::{Curiosity, Unicorn};
use ocw_sim::ufo::{Land, UfoGround, UfoInputs, UfoSky};
use ocw_weather::state::WeatherState;
use ocw_world::landuse::LandClass;
use ocw_world::package::WorldPackage;

/// What one frame of the UFOs needs.
#[derive(Debug, Clone, Copy)]
pub struct UfoFrame<'a> {
    /// The rider's `[traffic]` view settings.
    pub choice: ViewChoice,
    /// A race or event is on (none then, 18C).
    pub in_event: bool,
    /// The session's air seed.
    pub seed: u64,
    /// The world clock (ms since the Unix epoch); `None` before it is known.
    pub clock_ms: Option<i64>,
    /// The session's weather (`None`: the default fair day).
    pub weather: Option<&'a WeatherState>,
    /// The world's origin (lat, lon): the sun of the slots.
    pub origin: (f64, f64),
    /// The camera and which way it looks (x, z).
    pub eye: [f64; 3],
    pub forward: [f64; 2],
    /// The local rider's unicorn in god mode.
    pub unicorn: Option<Unicorn>,
}

/// The ground the UFOs ask (shared by the frames of one world).
pub type SharedGround = Arc<dyn UfoGround + Send + Sync>;

/// The client's UFOs.
#[derive(Default)]
pub struct UfoClient {
    ground: Option<(u64, SharedGround)>,
    sky: UfoSky,
    curiosity: Curiosity,
    /// `ufo.visit`: start the unicorn's visitor at the next frame.
    visit_now: bool,
}

impl std::fmt::Debug for UfoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UfoClient")
            .field("world", &self.ground.as_ref().map(|g| g.0))
            .field("visits", &self.curiosity.visits)
            .finish()
    }
}

impl UfoClient {
    /// The world's ground (built once per world; `key` names it).
    pub fn set_ground(&mut self, key: u64, ground: impl FnOnce() -> SharedGround) {
        if self.ground.as_ref().is_none_or(|g| g.0 != key) {
            self.ground = Some((key, ground()));
            self.sky = UfoSky::default();
        }
    }

    pub fn ground(&self) -> Option<&SharedGround> {
        self.ground.as_ref().map(|g| &g.1)
    }

    pub fn sky_mut(&mut self) -> &mut UfoSky {
        &mut self.sky
    }

    pub fn curiosity(&self) -> &Curiosity {
        &self.curiosity
    }

    /// Start the unicorn's visitor now (the `ufo.visit` command).
    pub fn visit_now(&mut self) {
        self.visit_now = true;
    }

    /// The inputs of the sky at frame `f` (`None`: no clock yet, or off).
    pub fn inputs<'a>(f: &UfoFrame<'a>, fallback: &'a WeatherState) -> Option<UfoInputs<'a>> {
        let level = f.choice.ufos_shown(f.in_event);
        if level == Ufos::Off {
            return None;
        }
        Some(UfoInputs {
            seed: f.seed,
            now_s: f.clock_ms? as f64 / 1000.0,
            level,
            weather: f.weather.unwrap_or(fallback),
            origin: f.origin,
            viewer: f.eye,
        })
    }

    /// The frame's UFOs (`None`: none drawn).
    pub fn frame(&mut self, f: &UfoFrame) -> Option<UfoView> {
        let fallback = WeatherState::default();
        let Some(inp) = Self::inputs(f, &fallback) else {
            // Off: the visitor leaves too.
            self.curiosity.step(f.seed, Ufos::Off, 0.0, None);
            return None;
        };
        let ground = self.ground.as_ref()?.1.clone();
        let mut scene = self.sky.now(ground.as_ref(), &inp);
        if std::mem::take(&mut self.visit_now)
            && let Some(u) = &f.unicorn
            && self.curiosity.phase().is_none()
        {
            self.curiosity.start_now(f.seed, inp.now_s, u);
        }
        if let Some(v) = self
            .curiosity
            .step(f.seed, inp.level, inp.now_s, f.unicorn.as_ref())
        {
            scene.ufos.push(v);
        }
        Some(UfoView {
            ufos: scene.ufos,
            cows: scene.cows,
            circles: scene.circles,
            now_s: inp.now_s,
            heard: f.choice.sound,
            listener: f.eye,
            listener_forward: f.forward,
        })
    }
}

/// The UFOs' ground from a world package: its terrain, land use (farmland
/// and meadows are fields) and road strips (searched in the tiles around
/// a point when a sighting is built – no index kept: the iPhone's memory).
pub struct PackageGround(pub Arc<WorldPackage>);

impl UfoGround for PackageGround {
    fn height(&self, x: f64, z: f64) -> Option<f64> {
        self.0.height(x, z)
    }

    fn land(&self, x: f64, z: f64) -> Land {
        let Some(lu) = &self.0.landuse else {
            return Land::Unknown;
        };
        match lu.class_at(f64::from(self.0.tile_size_m), x, z) {
            None => Land::Unknown,
            Some(LandClass::Farmland | LandClass::Meadow | LandClass::Default) => Land::Field,
            Some(LandClass::Water) => Land::Water,
            Some(_) => Land::Other,
        }
    }

    fn road_m(&self, x: f64, z: f64, within: f64) -> f64 {
        let size = f64::from(self.0.tile_size_m);
        let mut best = within;
        let (i0, i1) = (
            ((x - within) / size).floor() as i32,
            ((x + within) / size).floor() as i32,
        );
        let (k0, k1) = (
            ((z - within) / size).floor() as i32,
            ((z + within) / size).floor() as i32,
        );
        for ix in i0..=i1 {
            for kz in k0..=k1 {
                let Some(t) = self
                    .0
                    .tiles
                    .get(&ocw_world::package::TileKey { x: ix, z: kz })
                else {
                    continue;
                };
                for r in &t.roads {
                    let half = f64::from(r.width_m) / 2.0;
                    for w in r.points.windows(2) {
                        let a = [f64::from(w[0][0]), f64::from(w[0][2])];
                        let b = [f64::from(w[1][0]), f64::from(w[1][2])];
                        let d = segment_m([x, z], a, b) - half;
                        best = best.min(d.max(0.0));
                    }
                }
            }
        }
        best
    }
}

/// The distance from `p` to the segment `a`–`b`.
pub fn segment_m(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - ab[0] * t).hypot(p[1] - a[1] - ab[1] * t)
}

/// The UFOs' ground of a route without a package (the test routes): the
/// terrain from `height`, open land everywhere, the route the only road.
pub struct RouteGround {
    pub height: Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>,
    pub points: Vec<[f64; 2]>,
}

impl UfoGround for RouteGround {
    fn height(&self, x: f64, z: f64) -> Option<f64> {
        (self.height)(x, z)
    }
    fn land(&self, _: f64, _: f64) -> Land {
        Land::Unknown
    }
    fn road_m(&self, x: f64, z: f64, within: f64) -> f64 {
        self.points
            .windows(2)
            .map(|w| segment_m([x, z], w[0], w[1]) - 4.0)
            .fold(within, f64::min)
            .max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::traffic::AirTraffic;

    /// A flat world at 300 m with one straight road along z = 0.
    fn client() -> UfoClient {
        let mut c = UfoClient::default();
        c.set_ground(1, || {
            Arc::new(RouteGround {
                height: Arc::new(|x: f64, z: f64| {
                    (x.abs() < 40_000.0 && z.abs() < 40_000.0).then_some(300.0)
                }),
                points: vec![[-40_000.0, 0.0], [40_000.0, 0.0]],
            })
        });
        c
    }

    const NIGHT_MS: i64 = 1_784_156_400_000; // 2026-07-15 23:00 UTC

    fn frame(choice: ViewChoice, t_ms: i64) -> UfoFrame<'static> {
        UfoFrame {
            choice,
            in_event: false,
            seed: 7,
            clock_ms: Some(t_ms),
            weather: None,
            origin: (47.14, 9.52),
            eye: [0.0, 320.0, 0.0],
            forward: [0.0, -1.0],
            unicorn: None,
        }
    }

    fn often() -> ViewChoice {
        ViewChoice {
            ufo: Ufos::Often,
            ..ViewChoice::default()
        }
    }

    /// The ids seen over `hours` of night at 5 s steps.
    fn seen(c: &mut UfoClient, f: impl Fn(i64) -> UfoFrame<'static>, hours: i64) -> Vec<u64> {
        let mut ids = Vec::new();
        for i in 0..hours * 720 {
            if let Some(v) = c.frame(&f(NIGHT_MS + i * 5000)) {
                ids.extend(v.ufos.iter().map(|u| u.id));
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    #[test]
    fn off_by_default_and_none_in_events_or_with_the_master_switch() {
        let mut c = client();
        // Q1: off by default.
        assert!(c.frame(&frame(ViewChoice::default(), NIGHT_MS)).is_none());
        assert!(!seen(&mut c, |t| frame(often(), t), 6).is_empty());
        for f in [
            |t| UfoFrame {
                in_event: true,
                ..frame(often(), t)
            },
            |t| UfoFrame {
                choice: ViewChoice {
                    enabled: false,
                    ..often()
                },
                ..frame(often(), t)
            },
        ] {
            let mut c = client();
            assert!(seen(&mut c, f, 6).is_empty());
        }
        // Aircraft off does not matter.
        let mut c = client();
        let ids = seen(
            &mut c,
            |t| {
                frame(
                    ViewChoice {
                        air: AirTraffic::Off,
                        ..often()
                    },
                    t,
                )
            },
            6,
        );
        assert!(!ids.is_empty());
    }

    #[test]
    fn no_visitor_in_an_event_even_in_god_mode() {
        let mut c = client();
        let u = Unicorn {
            pos: [0.0, 800.0, 0.0],
            vel: [12.0, 0.0, 0.0],
        };
        let mut f = frame(often(), NIGHT_MS);
        f.unicorn = Some(u);
        f.in_event = true;
        c.visit_now();
        for i in 0..2000 {
            f.clock_ms = Some(NIGHT_MS + i * 50);
            assert!(c.frame(&f).is_none());
        }
        // Out of the event: the visitor comes when asked.
        f.in_event = false;
        c.visit_now();
        let mut met = false;
        for i in 0..200 {
            f.clock_ms = Some(NIGHT_MS + 100_000 + i * 50);
            let v = c.frame(&f).expect("on");
            met |= v
                .ufos
                .iter()
                .any(|u| u.kind == ocw_sim::ufo::SightingKind::Visitor);
        }
        assert!(met);
    }

    #[test]
    fn two_riders_see_the_same_ufos_and_sound_off_is_seen_not_heard() {
        let (mut a, mut b) = (client(), client());
        let mut shared = 0;
        for i in 0..6 * 720 {
            let t = NIGHT_MS + i * 5000;
            let fa = a.frame(&frame(often(), t)).expect("on");
            let fb = b
                .frame(&UfoFrame {
                    choice: ViewChoice {
                        sound: false,
                        ..often()
                    },
                    eye: [1500.0, 320.0, 900.0],
                    ..frame(often(), t)
                })
                .expect("on");
            assert!(fa.heard && !fb.heard);
            for u in &fa.ufos {
                if let Some(v) = fb.ufos.iter().find(|v| v.id == u.id) {
                    assert_eq!(u, v);
                    shared += 1;
                }
            }
        }
        assert!(shared > 0);
    }

    #[test]
    fn the_distance_to_a_road_segment() {
        assert_eq!(segment_m([0.0, 5.0], [-10.0, 0.0], [10.0, 0.0]), 5.0);
        assert_eq!(segment_m([20.0, 0.0], [-10.0, 0.0], [10.0, 0.0]), 10.0);
        assert_eq!(segment_m([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]), 5.0);
    }
}
