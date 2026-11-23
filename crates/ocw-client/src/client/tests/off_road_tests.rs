//! Riding off the road on the client (spec §14 Phase 13A): the landing
//! ring's kinds, the prediction off the road agreeing with the server,
//! the view and the trainer on the terrain's gradient, the FIT file's
//! distance, elevation and GPS track (Q9 (a)), joining a road again.

use super::*;
use ocw_sim::offroad::test_world::{height, world};

const NORTH: f64 = 0.0;
const EAST: f64 = -std::f64::consts::FRAC_PI_2;

impl Harness {
    /// A free ride on the off-road test world (the road along z = 500).
    fn off_road() -> Self {
        let net = world();
        let server = GameServer::new(
            ocw_server::Course::FreeRide {
                net: net.clone(),
                start: ocw_sim::FreeRideStart {
                    edge: 0,
                    offset_m: 100.0,
                    forward: true,
                },
            },
            ServerConfig {
                allow_dev_commands: true,
                bots: 0,
                start_m: 0.0,
                voice: None,
                ..Default::default()
            },
        );
        Self::with(
            server,
            ClientCore::free_ride(net, "Ada Rider", RiderProfile::default())
                .with_geo_origin((47.14, 9.52)),
        )
    }

    /// One tick of everything, the flight stepped too.
    fn tick_all(&mut self) -> Option<crate::god_state::GodEvent> {
        let t = self.time_s;
        let flat = |_: f64, _: f64| Some(0.0);
        let e = self.client.god_mut().step(
            &ocw_sim::god::FlightInput::default(),
            (0.0, 0.0),
            0.0,
            ocw_core::SIM_DT,
            t,
            &flat,
            None,
        );
        if let Some(m) = self.client.god_mut().pose_message(t) {
            self.send(&m);
        }
        for msg in self.server.tick() {
            self.receive(&msg);
        }
        self.client.predict_step();
        self.time_s += ocw_core::SIM_DT;
        e
    }

    /// Flies to over (x, z) facing `yaw` and lands (G); the landing the
    /// client saw.
    fn land_at(&mut self, x: f64, z: f64, yaw: f64) -> ocw_sim::offroad::Landing {
        let msg = self.client.god_mut().request_enter(self.time_s).unwrap();
        self.send(&msg);
        assert!(self.client.god().flying());
        self.client
            .god_mut()
            .teleport([x, 80.0, z], Some(yaw), None);
        for _ in 0..10 {
            self.tick_all();
        }
        let spot = self.client.landing(x, z, yaw).unwrap();
        self.client.god_mut().set_landing_at(Some(spot));
        if spot.allowed() {
            let msg = self
                .client
                .god_mut()
                .request_leave(false, self.time_s)
                .unwrap();
            self.send(&msg);
            let mut landed = false;
            for _ in 0..200 {
                landed |= self.tick_all() == Some(crate::god_state::GodEvent::Landed);
            }
            assert!(landed, "landed");
        }
        spot
    }

    fn me(&self) -> RiderId {
        self.client.rider_id().unwrap()
    }

    /// The own rider where the client and the server have it.
    fn places(&self) -> ([f64; 3], [f64; 3]) {
        let st = self.client.state();
        let mine = self
            .client
            .track()
            .sample(st.distance)
            .offset_position(st.lateral_offset);
        let s = self.server.session();
        let sv = s.rider_state(self.me()).unwrap();
        let theirs = s
            .track(self.me())
            .sample(sv.distance)
            .offset_position(sv.lateral_offset);
        (mine, theirs)
    }
}

#[test]
fn the_landing_ring_is_green_on_a_meadow_and_red_on_water_and_houses() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    use ocw_sim::offroad::Landing;
    let c = &h.client;
    assert!(matches!(
        c.landing(900.0, 800.0, NORTH),
        Some(Landing::Ground { .. })
    ));
    assert!(matches!(
        c.landing(500.0, 502.0, NORTH),
        Some(Landing::Road { .. })
    ));
    for (x, z) in [(1300.0, 700.0), (306.0, 705.0), (2500.0, 700.0)] {
        assert!(
            matches!(c.landing(x, z, NORTH), Some(Landing::Refused { .. })),
            "{x},{z}"
        );
    }
    // The ring's colour follows (gold road, green ground, red refused).
    let mut g = crate::god_state::GodCore::default();
    g.set_landing_at(c.landing(900.0, 800.0, NORTH));
    assert_eq!(g.landing_kind(), ocw_render::LandingKind::Ground);
    g.set_landing_at(c.landing(1300.0, 700.0, NORTH));
    assert_eq!(g.landing_kind(), ocw_render::LandingKind::Refused);
    assert_eq!(g.landing_refused(), Some("not on water"));
    g.set_landing_at(c.landing(500.0, 502.0, NORTH));
    assert_eq!(g.landing_kind(), ocw_render::LandingKind::Road);
    assert_eq!(g.landing_refused(), None);
    // The server refuses it too: the unicorn flies on.
    let msg = h.client.god_mut().request_enter(h.time_s).unwrap();
    h.send(&msg);
    h.client
        .god_mut()
        .teleport([1300.0, 80.0, 700.0], Some(NORTH), None);
    for _ in 0..10 {
        h.tick_all();
    }
    let msg = h.client.god_mut().request_leave(false, h.time_s).unwrap();
    h.send(&msg);
    for _ in 0..20 {
        h.tick_all();
    }
    assert!(h.client.god().flying(), "still flying");
    assert!(h.server.session().is_god(h.me()));
}

#[test]
fn off_the_road_the_prediction_agrees_with_the_server() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    let spot = h.land_at(900.0, 800.0, NORTH);
    assert!(matches!(spot, ocw_sim::offroad::Landing::Ground { .. }));
    let me = h.me();
    assert!(h.server.session().off_road(me).is_some());
    assert!(h.client.wild(), "wild mode on after landing off the road");
    assert!(!h.client.autodrive());
    // Ride, lean a little now and then: the client stays with the server.
    let mut worst: f64 = 0.0;
    for i in 0..20 {
        let lean = [0.0, 0.12, 0.0, -0.1][i % 4];
        h.client.set_lean(lean);
        if let Some(m) = h.client.steer_message(h.time_s) {
            h.send(&m);
        }
        h.ride(220.0, 1.0);
        let (mine, theirs) = h.places();
        worst = worst.max((mine[0] - theirs[0]).hypot(mine[2] - theirs[2]));
        assert!((mine[1] - theirs[1]).abs() < 0.5);
    }
    assert!(worst < 1.0, "worst {worst} m");
    // Still off the road, on the meadow, and moved.
    let p = h.server.session().off_road(me).unwrap();
    assert!((p.x - 900.0).hypot(p.z - 800.0) > 50.0, "{p:?}");
    let st = h.client.state();
    assert!(st.speed > 3.0, "{st:?}");
    // No junction or road notice off the road.
    assert!(h.client.junction_view().is_none() && h.client.notice().is_none());
    // The view draws the own rider on the terrain's track.
    let tracks = h.client.off_road_tracks(h.time_s);
    assert!(tracks.contains_key(&me));
    // Hearts by the work as always (Q9 (a)).
    h.ride(400.0, 40.0);
    assert!(h.client.server_hearts() > 0);
}

#[test]
fn a_forty_percent_slope_reaches_the_hud_and_the_trainer() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    // At the foot of the ramp (z < 400 rises 40 % to the north).
    h.land_at(500.0, 395.0, NORTH);
    h.ride(300.0, 8.0);
    let st = h.client.state();
    let grade = h.client.track().sample(st.distance).grade;
    assert!((grade - 0.4).abs() < 0.02, "{grade}");
    // The trainer gets the terrain's gradient, within what the trainer
    // settings let it simulate (up to 20 % by default, as on roads; Q7 (b):
    // the physics takes the full gradient).
    let p = trainer_sim(&h);
    let limit = ocw_sim::SimSettings::default().uphill_limit;
    assert!((p.grade - limit).abs() < 1e-3, "{p:?}");
    // Downhill the other way: no speed cap.
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    h.land_at(500.0, 40.0, std::f64::consts::PI);
    h.ride(0.0, 12.0);
    let st = h.client.state();
    assert!(st.speed > 20.0, "{st:?}");
    assert!(h.client.track().sample(st.distance).grade < -0.3);
}

#[test]
fn the_fit_file_counts_distance_elevation_and_the_real_gps_track_off_the_road() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    h.land_at(500.0, 395.0, NORTH);
    let mut rec = crate::recording::RideRecorder::new("Off-road test");
    let start = h.client.state().distance;
    for _ in 0..60 {
        h.ride(320.0, 1.0);
        crate::recording::observe_client(&mut rec, &h.client, h.time_s, 1_800_000_000);
    }
    let ridden = h.client.state().distance - start;
    assert!(ridden > 30.0, "{ridden}");
    // The distance counted is the distance ridden.
    assert!(
        (rec.distance_m() - ridden).abs() < 3.0,
        "{} vs {ridden}",
        rec.distance_m()
    );
    let rs = rec.records().to_vec();
    // Elevation: up the ramp with the terrain.
    let (first, last) = (rs.first().unwrap(), rs.last().unwrap());
    let climbed = last.altitude_m.unwrap() - first.altitude_m.unwrap();
    assert!(climbed > 8.0, "{climbed}");
    // The GPS track is where the rider is off the road.
    let proj = ocw_world::geo::LocalProjection::new(47.14, 9.52);
    let p = h.server.session().off_road(h.me()).unwrap();
    let (lat, lon) = last.position.unwrap();
    let back = proj.project(lat, lon);
    assert!(
        (back[0] - p.x).hypot(back[1] - p.z) < 3.0,
        "{back:?} vs {p:?}"
    );
    let alt = last.altitude_m.unwrap();
    assert!((alt - height(p.x, p.z)).abs() < 1.0, "{alt}");
    // And it is written to the FIT file.
    let ride = rec.finish().unwrap();
    let fit = ocw_fit::decode(&ocw_fit::encode(&ride.activity)).unwrap();
    let last = fit.records.last().unwrap();
    assert!(
        (last.distance_m - ridden).abs() < 3.0,
        "{}",
        last.distance_m
    );
    assert!(last.position.is_some() && last.altitude_m.unwrap() > first.altitude_m.unwrap() + 8.0);
}

#[test]
fn riding_onto_the_road_the_client_rides_its_path_again() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    // North of the road, heading east-south-east onto it at 30°.
    h.land_at(400.0, 480.0, EAST - 0.5);
    assert!(h.client.predictor.off_road().is_some());
    h.ride(250.0, 20.0);
    let me = h.me();
    assert!(h.server.session().off_road(me).is_none(), "joined the road");
    assert!(h.client.predictor.off_road().is_none(), "the client too");
    let (mine, theirs) = h.places();
    assert!(close(mine, theirs, 1.0), "{mine:?} vs {theirs:?}");
    assert!((theirs[2] - 500.0).abs() < 60.0, "{theirs:?}");
    // On the path now (the track is the road's).
    let p = h.client.track().sample(h.client.state().distance);
    assert!((p.position[2] - 500.0).abs() < 1e-6, "{p:?}");
}

#[test]
fn others_see_the_rider_on_the_terrain() {
    let mut h = Harness::off_road();
    h.ride(0.0, 0.5);
    let conn = h.server.connect();
    let mut guest = ClientCore::free_ride(world(), "Bea Climber", RiderProfile::default());
    for reply in h.server.handle(conn, &guest.hello()) {
        guest.on_server_message(&reply, h.time_s);
    }
    h.land_at(900.0, 800.0, NORTH);
    for _ in 0..(10.0 / ocw_core::SIM_DT) as usize {
        if (h.time_s * 4.0).fract() < 1e-9
            && let Some(sample) = h.client.on_device_event(power(200.0, h.time_s))
        {
            h.send(&sample);
        }
        for msg in h.server.tick() {
            h.receive(&msg);
            guest.on_server_message(&msg, h.time_s);
        }
        h.client.predict_step();
        guest.predict_step();
        h.time_s = ((h.time_s + ocw_core::SIM_DT) * 1000.0).round() / 1000.0;
    }
    let me = h.me();
    let tracks = guest.off_road_tracks(h.time_s);
    let t = tracks.get(&me).expect("the host's terrain track");
    let seen = guest
        .view(h.time_s, 0.0)
        .into_iter()
        .find(|r| r.id == me)
        .unwrap();
    let at = t
        .sample(seen.state.distance)
        .offset_position(seen.state.lateral_offset);
    let p = h.server.session().off_road(me).unwrap();
    // Drawn slightly in the past (the interpolation delay).
    assert!((at[0] - p.x).hypot(at[2] - p.z) < 5.0, "{at:?} vs {p:?}");
}
