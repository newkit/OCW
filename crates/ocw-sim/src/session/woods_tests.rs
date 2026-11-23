//! Off the road every drawn tree is an obstacle (spec 14 Phase 13 Q10
//! (a)): deep in a wood a rider at speed crashes into a trunk, at walking
//! pace stops at it; with collisions off the trunks are not made; the
//! cells come and go with the rider; the prediction stops there too.

use super::*;
use crate::collide::{ContactKind, Mode};
use crate::god::{GodPose, forward};
use crate::offroad::test_world::wooded_world;
use ocw_world::obstacles::ObstacleKind;
use ocw_world::woods::{Stored, Trunk, WoodCell};

fn free() -> Session {
    let mut s = Session::free_ride(
        wooded_world(),
        FreeRideStart {
            edge: 0,
            offset_m: 100.0,
            forward: true,
        },
    );
    s.set_collisions(true);
    s
}

fn pedal(s: &mut Session, id: RiderId, seq: u32, watts: f64) {
    s.apply_input(
        id,
        seq,
        RiderInput {
            power_w: watts,
            ..Default::default()
        },
    );
}

fn land(s: &mut Session, id: RiderId, x: f64, z: f64, yaw: f64) -> Result<[f64; 3], String> {
    if !s.is_god(id) {
        s.enter_god(id).unwrap();
    }
    s.set_god_pose(
        id,
        GodPose {
            pos: [x, 200.0, z],
            yaw,
            pitch: 0.0,
            speed: 0.0,
        },
        true,
    )
    .unwrap();
    s.leave_god(id, GodLeave::Land { x, z })
}

/// The trunks of the test world's wood around (700, 800).
fn wood_trunks() -> Vec<Trunk> {
    let net = wooded_world();
    let woods = net.woods().unwrap().clone();
    WoodCell::covering([620.0, 700.0], [780.0, 900.0], 1000.0)
        .into_iter()
        .flat_map(|c| woods.grid_trunks(c, &Stored::default()).unwrap())
        .collect()
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// A trunk deep in the wood with a clear run of `before` m at it (no other
/// trunk within a rider's reach of the line): where to stand, facing it.
fn run_at_trunk(before: f64) -> ([f64; 2], f64, Trunk) {
    let trunks = wood_trunks();
    assert!(trunks.len() > 300, "a dense wood: {}", trunks.len());
    for t in &trunks {
        for k in 0..16 {
            let yaw = f64::from(k) * std::f64::consts::TAU / 16.0;
            let d = forward(yaw);
            let p = [t.c[0] - d[0] * before, t.c[1] - d[1] * before];
            let clear = trunks.iter().filter(|u| u.id != t.id).all(|u| {
                let rel = [u.c[0] - p[0], u.c[1] - p[1]];
                let along = dot(rel, d);
                let beside = (rel[0] * d[1] - rel[1] * d[0]).abs();
                along < -2.0 || along > before + 3.0 || beside > u.r + 1.2
            });
            if clear {
                return (p, yaw, *t);
            }
        }
    }
    panic!("no clear run at a trunk in the wood");
}

/// Where the rider is along the run from `p` in direction `yaw`.
fn along(s: &Session, id: RiderId, p: [f64; 2], yaw: f64) -> f64 {
    let pose = s.off_road(id).unwrap();
    dot([pose.x - p[0], pose.z - p[1]], forward(yaw))
}

fn rider_at_speed(s: &mut Session, p: [f64; 2], yaw: f64, speed: f64, watts: f64) -> RiderId {
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    // Past the new rider's ghost time (12A): solid from then on.
    for _ in 0..((JOIN_GHOST_S + 0.5) / SIM_DT) as usize {
        s.step();
    }
    land(s, me, p[0], p[1], yaw).expect("lands in the wood");
    let st = s.rider_state(me).unwrap();
    s.set_rider_state(me, RiderState { speed, ..st });
    pedal(s, me, 1, watts);
    me
}

#[test]
fn a_crash_at_speed_into_a_trunk_deep_in_the_wood() {
    // Far beyond the 60 m of the road (z 500): no OBST trees here.
    let (p, yaw, t) = run_at_trunk(4.0);
    assert!(t.c[1] > 600.0 + 60.0);
    let mut s = free();
    let me = rider_at_speed(&mut s, p, yaw, 8.0, 250.0);
    let mut crash = None;
    for _ in 0..100 {
        s.step();
        if let Some(c) = s.crash(me) {
            crash = Some(c);
            break;
        }
    }
    let c = crash.expect("crashed into the trunk");
    assert_eq!(c.kind, ContactKind::World(ObstacleKind::Tree));
    assert!(along(&s, me, p, yaw) < 4.0 - t.r, "not through it");
    assert!(s.world.wood_cells() > 0, "the trunks were made");
}

#[test]
fn a_stop_at_walking_pace_at_a_trunk() {
    let (p, yaw, t) = run_at_trunk(2.5);
    let mut s = free();
    let me = rider_at_speed(&mut s, p, yaw, 1.4, 40.0);
    let mut stopped = false;
    for _ in 0..(5.0 / SIM_DT) as usize {
        s.step();
        assert!(s.crash(me).is_none(), "no crash at walking pace");
        let a = along(&s, me, p, yaw);
        assert!(a < 2.5 - t.r, "through the trunk: {a}");
        stopped |= s.rider_state(me).unwrap().speed < 0.05;
    }
    assert!(stopped, "stopped at the trunk");
    // Pressed against it, still there.
    let a = along(&s, me, p, yaw);
    assert!(a > 2.5 - t.r - 1.6, "{a}");
    assert!(s.rider_state(me).unwrap().speed < 0.5);
}

#[test]
fn with_collisions_off_no_trunks_are_made() {
    let (p, yaw, _) = run_at_trunk(3.0);
    let mut s = free();
    s.set_collisions(false);
    let me = rider_at_speed(&mut s, p, yaw, 4.0, 200.0);
    // The landing checks the ground (never inside a trunk), riding makes
    // none.
    let landed = s.world.wood_cells_made();
    for _ in 0..(3.0 / SIM_DT) as usize {
        s.step();
    }
    assert!(s.crash(me).is_none());
    assert!(along(&s, me, p, yaw) > 8.0, "rode through");
    assert_eq!(s.world.wood_cells_made(), landed);
}

#[test]
fn group_rides_only_bump_at_a_trunk() {
    // The soft model (group rides, 12G): no crash, the run is stopped.
    let (p, yaw, t) = run_at_trunk(4.0);
    let mut s = free();
    let me = rider_at_speed(&mut s, p, yaw, 8.0, 250.0);
    let tick = s.tick;
    let pose = s.off_road(me).unwrap();
    let st = s.rider_state(me).unwrap();
    let body = crate::collide::Body {
        id: me,
        pos: [pose.x, pose.z],
        road: forward(pose.yaw),
        heading: 0.0,
        speed: st.speed,
        lean: 0.0,
        mass: 85.0,
        fallen: false,
        manual: true,
        grip: 1.0,
    };
    // Up against the trunk at 8 m/s.
    let at = [
        t.c[0] - forward(yaw)[0] * 1.2,
        t.c[1] - forward(yaw)[1] * 1.2,
    ];
    let body = crate::collide::Body { pos: at, ..body };
    let mut scratch = Vec::new();
    let soft = s
        .world
        .resolve(&body, 0.0, tick, Mode::Soft, 1.0, true, &mut scratch);
    assert!(soft.crash.is_none() && soft.touched(), "{soft:?}");
    let full = s
        .world
        .resolve(&body, 0.0, tick, Mode::Full, 1.0, true, &mut scratch);
    assert!(full.crash.is_some());
    // Not deep (on the road band's rules): the trunk is not there.
    let road = s
        .world
        .resolve(&body, 0.0, tick, Mode::Full, 1.0, false, &mut scratch);
    assert!(!road.touched());
}

#[test]
fn the_cells_follow_the_rider_and_go_after_it() {
    let (p, yaw, _) = run_at_trunk(3.0);
    let mut s = free();
    let me = rider_at_speed(&mut s, p, yaw, 0.0, 0.0);
    s.step();
    let near = s.world.wood_cells();
    assert!((1..=4).contains(&near), "{near} cells around one rider");
    // Back on the road: after 10 s the cells are dropped.
    s.rejoin_route(me, None).unwrap();
    pedal(&mut s, me, 2, 150.0);
    for _ in 0..(16.0 / SIM_DT) as usize {
        s.step();
    }
    assert_eq!(s.world.wood_cells(), 0);
}

#[test]
fn landing_on_a_trunk_is_refused() {
    let (_, _, t) = run_at_trunk(3.0);
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    let e = land(&mut s, me, t.c[0], t.c[1], 0.0).unwrap_err();
    assert!(e.contains("obstacle"), "{e}");
}

#[test]
fn the_prediction_stops_at_the_trunk_like_the_server() {
    let (p, yaw, t) = run_at_trunk(2.5);
    let net = wooded_world();
    let mut s = free();
    let me = rider_at_speed(&mut s, p, yaw, 1.4, 40.0);
    let mut pred = crate::prediction::Predictor::new(
        RiderProfile::default(),
        RiderState {
            speed: 1.4,
            ..s.rider_state(me).unwrap()
        },
    );
    pred.set_world(net.obstacles().cloned());
    pred.set_woods(net.woods().cloned());
    pred.set_collision_mode(Mode::Full);
    let pose = crate::offroad::OffRoadPose {
        x: p[0],
        z: p[1],
        yaw,
    };
    pred.set_off_road(Some(pose.track(
        net.ground().unwrap().clone(),
        s.rider_state(me).unwrap().distance,
    )));
    for _ in 0..(4.0 / SIM_DT) as usize {
        s.step();
        let tr = pred.off_road().unwrap().clone();
        pred.step(&tr, 40.0);
    }
    let a = along(&s, me, p, yaw);
    let at = pred.off_road().unwrap().at();
    let b = dot([at[0] - p[0], at[1] - p[1]], forward(yaw));
    assert!(b < 2.5 - t.r, "the prediction went through: {b}");
    assert!((a - b).abs() < 0.3, "server {a}, prediction {b}");
}
