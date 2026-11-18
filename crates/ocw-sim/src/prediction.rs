//! Client-side prediction of the local rider (spec §5.2, §6.1).

use ocw_core::{RiderProfile, SIM_DT};
use ocw_world::Track;

use crate::collide::{Body, Mode, Skid, WorldCache};
use crate::physics::{self, RiderState, Steering};
use crate::ride::{self, Controls};

/// Errors above this distance (m) are corrected immediately instead of smoothed.
pub const SNAP_THRESHOLD_M: f64 = 5.0;
/// Time over which smaller errors are blended away, in seconds.
pub const CORRECTION_TIME_S: f64 = 0.5;

/// Predicts the local rider so the bike reacts to pedaling immediately, and
/// converges smoothly to the authoritative server state.
#[derive(Debug, Clone)]
pub struct Predictor {
    profile: RiderProfile,
    state: RiderState,
    /// Remaining distance error still to be blended in.
    pending_correction: f64,
    /// Metres per second at which the pending correction is applied.
    correction_rate: f64,
    /// Distance error measured at the last reconciliation (for diagnostics).
    last_error: f64,
    /// Drafting factor on the drag, predicted by the client (1.0 = none).
    drag_factor: f64,
    /// Steering, wild mode and brake (the same rule as the server, so the
    /// bike reacts at once).
    controls: Controls,
    /// Scraped along the road edge in the last predicted tick.
    scraping: bool,
    /// Remaining lateral error to blend in, and its rate (m/s).
    pending_lateral: f64,
    lateral_rate: f64,
    /// Autodrive avoidance's virtual brake (spec §14 Phase 12D), as on
    /// the server.
    speed_cap: Option<f64>,
    /// The world's static obstacles (spec §14 Phase 12D, K3: the own
    /// rider's contacts with them are predicted exactly; crashes come only
    /// from the server) and the collision model in force.
    world: WorldCache,
    mode: Mode,
    ticks: u64,
    /// The session weather's physics while it is in force (spec §14 Phase
    /// 9, S6; the server's switch), as the server rides it.
    weather: Option<crate::weather::WeatherPhysics>,
    /// The damage's effects the server applies to the rider (spec §14
    /// Phase 12E, K4: from `RiderUpdate.damage`).
    damage: crate::damage::Effects,
    /// The skid of a lean beyond the wet road's grip (S6 → Phase 12: the
    /// speed it costs, as on the server; the fall is only the server's).
    skid: Skid,
}

impl Predictor {
    pub fn new(profile: RiderProfile, initial: RiderState) -> Self {
        Self {
            profile,
            state: initial,
            pending_correction: 0.0,
            correction_rate: 0.0,
            last_error: 0.0,
            drag_factor: 1.0,
            controls: Controls::default(),
            scraping: false,
            pending_lateral: 0.0,
            lateral_rate: 0.0,
            speed_cap: None,
            world: WorldCache::default(),
            mode: Mode::Off,
            ticks: 0,
            weather: None,
            damage: crate::damage::Effects::NONE,
            skid: Skid::default(),
        }
    }

    /// One tick of the skid after the physics step (as the server's step).
    fn skid_tick(
        &self,
        skid: &mut Skid,
        state: &mut RiderState,
        route: &dyn Track,
        controls: Controls,
    ) {
        let step = crate::collide::skid(
            skid,
            crate::collide::skid_lean(controls.steering, route, state.distance),
            crate::weather::grip(self.weather.as_ref(), route, state),
            state.speed,
            self.profile.rider_mass_kg + self.profile.bike_mass_kg,
            self.mode,
            SIM_DT,
        );
        state.speed -= step.speed_loss;
    }

    /// The world's static obstacles (spec §14 Phase 12 K3).
    pub fn set_world(
        &mut self,
        world: Option<std::sync::Arc<ocw_world::obstacles::WorldObstacles>>,
    ) {
        self.world = WorldCache::new(world);
    }

    /// The damage's effects in force (the server's, spec §14 Phase 12E).
    pub fn set_damage(&mut self, effects: crate::damage::Effects) {
        self.damage = effects;
    }

    pub fn damage(&self) -> crate::damage::Effects {
        self.damage
    }

    /// The profile, power and controls the rider rides with under the
    /// damage's effects (as the server's step).
    fn damaged(&self, power_w: f64) -> (RiderProfile, f64, Controls) {
        let fx = self.damage;
        let mut controls = self.controls;
        if let Steering::Lean(l) = controls.steering {
            controls.steering = Steering::Lean(l * fx.lean_factor);
        }
        controls.braking |= fx.held;
        (fx.profile(&self.profile), fx.power(power_w), controls)
    }

    fn damage_cap(&self, state: &mut RiderState) {
        if let Some(c) = self.damage.speed_cap
            && state.speed > c
        {
            state.speed = c;
        }
    }

    /// The slip of a lean beyond the wet road's grip (S6 → Phase 12: 0 =
    /// gripping, 1 = the fall), for the HUD's warning.
    pub fn slip(&self) -> f64 {
        self.skid.slip
    }

    /// The collision model the server says is in force.
    pub fn set_collision_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// In wild mode: pushed out of the world's solids, the speed into
    /// them lost (as the server does it; a crash is only the server's).
    fn world_tick(&mut self, state: &mut RiderState, route: &dyn Track) {
        if !self.controls.wild || !self.mode.on() || self.world.world.is_none() {
            return;
        }
        self.ticks += 1;
        let p = route.sample(state.distance);
        let at = p.offset_position(state.lateral_offset);
        let body = Body {
            id: ocw_core::RiderId(0),
            pos: [at[0], at[2]],
            road: p.direction,
            heading: state.heading,
            speed: state.speed,
            lean: 0.0,
            mass: self.profile.rider_mass_kg + self.profile.bike_mass_kg,
            fallen: false,
            manual: true,
            grip: crate::weather::grip(self.weather.as_ref(), route, state),
        };
        let mut scratch = Vec::new();
        let r = self.world.resolve(
            &body,
            at[1],
            self.ticks,
            self.mode,
            physics::WILD_MAX_HEADING,
            &mut scratch,
        );
        if r.touched() {
            state.distance += r.ds;
            state.lateral_offset =
                (state.lateral_offset + r.dd).clamp(-physics::WILD_LIMIT_M, physics::WILD_LIMIT_M);
            if let Some(v) = r.speed {
                state.speed = v;
            }
            if let Some(h) = r.heading {
                state.heading = h;
            }
        }
        if self.ticks.is_multiple_of(500) {
            self.world.sweep(self.ticks);
        }
    }

    /// A new physics profile (the workshop's bike change, spec §14 Phase 4,
    /// 4A): the server applies the same at once.
    pub fn set_profile(&mut self, profile: RiderProfile) {
        self.profile = profile;
    }

    pub fn profile(&self) -> &RiderProfile {
        &self.profile
    }

    /// Autodrive avoidance's speed cap for the next steps (m/s; `None`: none).
    pub fn set_speed_cap(&mut self, cap: Option<f64>) {
        self.speed_cap = cap;
    }

    /// Takes the server's state as it is, dropping pending corrections
    /// (the rider is held, e.g. at an event's start line).
    pub fn set_state(&mut self, state: RiderState) {
        self.last_error = state.distance - self.state.distance;
        self.state = state;
        self.pending_correction = 0.0;
        self.pending_lateral = 0.0;
    }

    pub fn state(&self) -> RiderState {
        self.state
    }

    /// Distance error (server − prediction) at the last reconciliation, in metres.
    pub fn last_error(&self) -> f64 {
        self.last_error
    }

    pub fn set_braking(&mut self, braking: bool) {
        self.controls.braking = braking;
    }

    /// Sets the steering target (already clamped to the road by the caller).
    pub fn set_lateral_target(&mut self, target_m: f64) {
        self.controls.steering = Steering::Auto(target_m);
    }

    /// Sets lean steering (rad, positive = right).
    pub fn set_lean(&mut self, lean_rad: f64) {
        self.controls.steering = Steering::Lean(lean_rad);
    }

    /// Wild mode: not bound to the road.
    pub fn set_wild(&mut self, wild: bool) {
        self.controls.wild = wild;
    }

    /// The weather's physics in force (spec §14 Phase 9, S6; `None`: the
    /// weather does not affect the physics), from the session weather the
    /// server sends.
    pub fn set_weather(&mut self, weather: Option<crate::weather::WeatherPhysics>) {
        self.weather = weather;
    }

    pub fn weather(&self) -> Option<&crate::weather::WeatherPhysics> {
        self.weather.as_ref()
    }

    /// Sets the predicted drafting factor (from the positions of others).
    pub fn set_drag_factor(&mut self, drag_factor: f64) {
        self.drag_factor = drag_factor;
    }

    /// Advances the prediction by one physics tick with the current power.
    pub fn step(&mut self, route: &dyn Track, power_w: f64) {
        let (profile, power_w, controls) = self.damaged(power_w);
        ride::tick_in(
            &mut self.state,
            &profile,
            power_w,
            controls,
            &mut self.scraping,
            route,
            self.drag_factor,
            self.weather.as_ref(),
            SIM_DT,
        );
        let (mut skid, mut state) = (self.skid, self.state);
        self.skid_tick(&mut skid, &mut state, route, controls);
        self.skid = skid;
        self.state = state;
        let mut capped = self.state;
        self.damage_cap(&mut capped);
        self.state = capped;
        let mut state = self.state;
        self.world_tick(&mut state, route);
        self.state = state;
        if let Some(c) = self.speed_cap
            && self.state.speed > c
        {
            self.state.speed = c;
        }
        if self.pending_lateral != 0.0 {
            let max_step = self.lateral_rate.abs() * SIM_DT;
            let applied = self.pending_lateral.clamp(-max_step, max_step);
            self.state.lateral_offset += applied;
            self.pending_lateral -= applied;
        }
        if self.pending_correction != 0.0 {
            // Apply the error linearly over CORRECTION_TIME_S.
            let max_step = self.correction_rate.abs() * SIM_DT;
            let applied = self.pending_correction.clamp(-max_step, max_step);
            self.state.distance += applied;
            self.pending_correction -= applied;
        }
        ride::stop_at_finish(&mut self.state, route);
    }

    /// Reconciles with an authoritative state that is `age_s` seconds old.
    ///
    /// The server state is extrapolated to "now" with the same physics and the
    /// current power, then compared with the prediction.
    pub fn reconcile(&mut self, route: &dyn Track, server: RiderState, age_s: f64, power_w: f64) {
        let mut extrapolated = server;
        let steps = (age_s.max(0.0) / SIM_DT).round() as usize;
        let mut scraping = self.scraping;
        let mut skid = self.skid;
        let (profile, power_w, controls) = self.damaged(power_w);
        for _ in 0..steps {
            ride::tick_in(
                &mut extrapolated,
                &profile,
                power_w,
                controls,
                &mut scraping,
                route,
                self.drag_factor,
                self.weather.as_ref(),
                SIM_DT,
            );
            self.skid_tick(&mut skid, &mut extrapolated, route, controls);
            self.damage_cap(&mut extrapolated);
            self.world_tick(&mut extrapolated, route);
        }
        let error = extrapolated.distance - self.state.distance;
        self.last_error = error;
        if error.abs() > SNAP_THRESHOLD_M {
            self.state = extrapolated;
            self.pending_correction = 0.0;
            self.pending_lateral = 0.0;
        } else {
            self.pending_correction = error;
            self.correction_rate = error / CORRECTION_TIME_S;
            self.state.speed = extrapolated.speed;
            // Lateral errors are blended too (the steering intent reaches
            // the server a little later than the prediction reacts).
            let lateral_error = extrapolated.lateral_offset - self.state.lateral_offset;
            self.pending_lateral = lateral_error;
            self.lateral_rate = lateral_error / CORRECTION_TIME_S;
            // The heading follows the server halfway per snapshot (10 Hz).
            self.state.heading += (extrapolated.heading - self.state.heading) * 0.5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{self, Environment};
    use ocw_world::Route;

    fn flat() -> Route {
        Route::from_points(
            "flat",
            "Flat",
            vec![[0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap()
    }

    #[test]
    fn set_state_drops_pending_corrections() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        p.reconcile(
            &route,
            RiderState {
                distance: 2.0,
                ..Default::default()
            },
            0.0,
            0.0,
        );
        let held = RiderState {
            distance: 3.0,
            lateral_offset: 1.2,
            ..Default::default()
        };
        p.set_state(held);
        assert_eq!(p.state(), held);
        assert_eq!(p.last_error(), 3.0);
        p.step(&route, 0.0);
        assert_eq!(p.state().distance, 3.0, "no correction left to blend in");
    }

    #[test]
    fn small_errors_are_blended() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut server = p.state();
        server.distance += 1.0;
        p.reconcile(&route, server, 0.0, 200.0);
        assert!((p.last_error() - 1.0).abs() < 1e-9);
        let before = p.state().distance;
        p.step(&route, 200.0);
        // Not snapped: only a fraction of the error is applied per tick.
        assert!(p.state().distance - before < 0.5);
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut reference = server;
        for _ in 0..101 {
            physics::step(
                &mut reference,
                &RiderProfile::default(),
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        assert!((p.state().distance - reference.distance).abs() < 0.05);
    }

    #[test]
    fn large_errors_snap() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let server = RiderState {
            distance: 50.0,
            speed: 8.0,
            lateral_offset: 0.0,
            ..Default::default()
        };
        p.reconcile(&route, server, 0.0, 200.0);
        assert_eq!(p.state(), server);
    }

    #[test]
    fn agreeing_server_causes_no_correction() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let mut server = RiderState::default();
        for tick in 1..=500 {
            p.step(&route, 220.0);
            physics::step(
                &mut server,
                &RiderProfile::default(),
                220.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
            if tick % 5 == 0 {
                p.reconcile(&route, server, 0.0, 220.0);
                assert!(p.last_error().abs() < 1e-9);
            }
        }
        assert_eq!(p.state(), server);
    }

    #[test]
    fn old_server_state_is_extrapolated_before_comparing() {
        let route = flat();
        let profile = RiderProfile::default();
        let mut server = RiderState {
            distance: 100.0,
            speed: 9.0,
            lateral_offset: 0.0,
            ..Default::default()
        };
        let old = server;
        // The server state is 0.1 s old; meanwhile we rode on.
        for _ in 0..5 {
            physics::step(
                &mut server,
                &profile,
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        let mut p = Predictor::new(profile, server);
        p.reconcile(&route, old, 0.1, 200.0);
        assert!(p.last_error().abs() < 1e-9, "error {}", p.last_error());
    }

    /// Spec §14 Phase 12D / K3: the own rider's contact with a wall in
    /// wild mode is predicted (pushed out, stopped) like the server's.
    #[test]
    fn wild_mode_contacts_with_the_world_are_predicted() {
        use ocw_world::obstacles::{LineObstacle, ObstacleKind, Obstacles, TileObstacles};
        let mut o = Obstacles::new(1000.0);
        o.tiles.insert(
            ocw_world::TileKey { x: 0, z: 0 },
            TileObstacles {
                lines: vec![LineObstacle {
                    kind: ObstacleKind::Wall,
                    flags: 0,
                    height_m: 1.5,
                    points: vec![[300.0, 10.0], [300.0, 30.0]],
                }],
                ..Default::default()
            },
        );
        let pkg = ocw_world::WorldPackage {
            name: "w".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: Some(o),
            places: None,
        };
        let world =
            ocw_world::obstacles::WorldObstacles::from_package(&pkg).map(std::sync::Arc::new);
        let route = flat();
        let start = RiderState {
            distance: 290.0,
            lateral_offset: 20.0,
            speed: 5.0,
            ..Default::default()
        };
        let run = |mode: Mode| {
            let mut p = Predictor::new(RiderProfile::default(), start);
            p.set_world(world.clone());
            p.set_collision_mode(mode);
            p.set_wild(true);
            p.set_lean(0.0);
            for _ in 0..150 {
                p.step(&route, 0.0);
            }
            p.state()
        };
        let stopped = run(Mode::Full);
        assert!(
            stopped.distance + 0.875 <= 299.81 && stopped.speed < 0.5,
            "{stopped:?}"
        );
        // Collisions off: through it.
        assert!(run(Mode::Off).distance > 300.0);
    }

    /// A lean beyond the wet road's grip (S6 → Phase 12 skid): the
    /// prediction loses the same speed as the server, tick for tick.
    #[test]
    fn the_skid_is_predicted_as_the_server_rides_it() {
        let w = crate::weather::WeatherPhysics {
            wind: [0.0, 0.0],
            temperature_c: 10.0,
            pressure_hpa: 1013.25,
            pressure_y_m: 0.0,
            humidity: 0.8,
            rolling_factor: 1.0,
            grip: 0.75,
        };
        let route = flat();
        let mut s = crate::session::Session::new(route.clone());
        s.set_collisions(true);
        s.set_weather(Some(w));
        s.set_weather_physics(true);
        let id = s.add_rider("a", RiderProfile::default());
        for _ in 0..=110 {
            s.step();
        }
        let start = RiderState {
            distance: 100.0,
            speed: 10.0,
            ..Default::default()
        };
        s.set_rider_state(id, start);
        s.set_wild(id, true);
        s.set_lean(id, 1, physics::MAX_STEER_LEAN);
        let mut p = Predictor::new(RiderProfile::default(), start);
        p.set_weather(Some(w));
        p.set_collision_mode(Mode::Full);
        p.set_wild(true);
        p.set_lean(physics::MAX_STEER_LEAN);
        let mut free = p.clone();
        free.set_collision_mode(Mode::Off);
        // Before the server's fall (about 0.75 s on a wet road).
        for _ in 0..25 {
            s.step();
            p.step(&route, 0.0);
            free.step(&route, 0.0);
        }
        assert!(s.crash(id).is_none());
        let server = s.rider_state(id).unwrap();
        assert_eq!(p.state(), server);
        assert!(server.speed < free.state().speed - 0.3, "{server:?}");
    }
}
