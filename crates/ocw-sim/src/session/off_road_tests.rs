//! Riding off the road in the session (spec §14 Phase 13A, Q7 – Q9): the
//! unicorn lands on the ground in a free ride (wild mode on), refused on
//! water and buildings, on the route in route rides; the rider rides the
//! terrain, earns hearts by work, joins a road by riding onto it, leaves
//! wild mode's band into the terrain, J puts them back on the road.

use super::*;
use crate::god::GodPose;
use crate::offroad::tests::world;

fn free() -> Session {
    Session::free_ride(
        world(),
        FreeRideStart {
            edge: 0,
            offset_m: 100.0,
            forward: true,
        },
    )
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

fn ride(s: &mut Session, seconds: f64) {
    for _ in 0..(seconds / SIM_DT).round() as usize {
        s.step();
    }
}

/// Flies rider `id` to over (x, z) facing `yaw` and lands.
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

const NORTH: f64 = 0.0;
const EAST: f64 = -std::f64::consts::FRAC_PI_2;

#[test]
fn landing_on_a_meadow_rides_the_terrain_in_wild_mode() {
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    pedal(&mut s, me, 1, 200.0);
    ride(&mut s, 5.0);
    let d0 = s.rider_state(me).unwrap().distance;
    s.take_path_changes();
    let at = land(&mut s, me, 900.0, 800.0, NORTH).unwrap();
    assert!((at[0] - 900.0).abs() < 1e-6 && (at[2] - 800.0).abs() < 1e-6, "{at:?}");
    let pose = s.off_road(me).expect("off the road");
    assert_eq!((pose.x, pose.z, pose.yaw), (900.0, 800.0, NORTH));
    let st = s.rider_state(me).unwrap();
    assert_eq!(st.speed, 0.0);
    assert_eq!(st.distance, d0, "the flight credits nothing");
    assert!(s.riders[&me].wild, "wild mode on");
    assert_eq!(s.riders[&me].steering, Steering::Lean(0.0));
    // The snapshot says so.
    let snap = s.snapshot();
    assert_eq!(snap.iter().find(|r| r.id == me).unwrap().off_road, Some(pose));
    // Riding north across the meadow: the distance counts (Q9 (a)),
    // hearts by the work as always.
    let hearts = s.hearts(me);
    // 400 W for 30 s: 12 kJ, a heart.
    pedal(&mut s, me, 2, 400.0);
    ride(&mut s, 30.0);
    let st = s.rider_state(me).unwrap();
    let p = s.off_road(me).unwrap();
    assert!(st.speed > 4.0, "{st:?}");
    assert!((p.z - (800.0 - (st.distance - d0))).abs() < 0.5, "{p:?} {st:?}");
    assert!((p.x - 900.0).abs() < 0.01);
    assert!(s.hearts(me) > hearts);
    // The track is the terrain's: the gradient there, a meadow.
    let t = s.track(me);
    assert_eq!(t.sample(st.distance).grade, 0.0);
    assert_eq!(t.surface_at(st.distance), ocw_world::Surface::Grass);
    // No path changes while off the road.
    assert!(s.take_path_changes().is_empty());
    // Wild mode cannot be switched off off the road.
    s.set_wild(me, false);
    assert!(s.riders[&me].wild);
}

#[test]
fn landing_is_refused_on_water_and_buildings_and_the_unicorn_flies_on() {
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    for (x, z) in [(1300.0, 700.0), (306.0, 705.0), (3000.0, 700.0)] {
        let e = land(&mut s, me, x, z, NORTH).unwrap_err();
        assert!(e.contains("cannot land"), "{e}");
        assert!(s.is_god(me), "still flying");
        assert!(s.off_road(me).is_none());
    }
    // A meadow then.
    land(&mut s, me, 1100.0, 700.0, NORTH).unwrap();
    assert!(!s.is_god(me) && s.off_road(me).is_some());
    // Near the road: on the road, not off it.
    land(&mut s, me, 700.0, 503.0, NORTH).unwrap();
    assert!(s.off_road(me).is_none());
    let st = s.rider_state(me).unwrap();
    let p = s.track(me).sample(st.distance).position;
    assert!((p[2] - 500.0).abs() < 0.5, "{p:?}");
}

#[test]
fn route_rides_land_on_the_route() {
    let net = world();
    let plan = ocw_world::RoutePlan::new(
        &net,
        "row",
        vec![
            Segment::whole(&net, 0, true),
            Segment::whole(&net, 1, true),
        ],
        false,
    )
    .unwrap();
    let mut s = Session::route_ride(net, Arc::new(plan));
    let me = s.add_rider("Bea Climber", RiderProfile::default());
    assert!(!s.off_road_allowed());
    land(&mut s, me, 900.0, 800.0, NORTH).unwrap();
    assert!(s.off_road(me).is_none(), "on the route");
    let st = s.rider_state(me).unwrap();
    assert!((st.distance - 900.0).abs() < 1.0, "{st:?}");
    // A fixed route too (and its water does not matter there).
    let mut s = Session::new(Route::test_loop());
    let me = s.add_rider("Bea Climber", RiderProfile::default());
    s.enter_god(me).unwrap();
    assert!(s.leave_god(me, GodLeave::Land { x: 1300.0, z: 700.0 }).is_ok());
    assert!(s.off_road(me).is_none());
}

#[test]
fn riding_onto_the_road_joins_it_in_the_direction_of_travel() {
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    // 20 m north of the road, heading east-south-east (30° to the road).
    land(&mut s, me, 400.0, 480.0, EAST - 0.5).unwrap();
    s.take_path_changes();
    pedal(&mut s, me, 1, 250.0);
    let mut joined = None;
    for i in 0..(30.0 / SIM_DT) as usize {
        s.step();
        if s.off_road(me).is_none() {
            joined = Some(i);
            break;
        }
    }
    assert!(joined.is_some(), "joined the road");
    let st = s.rider_state(me).unwrap();
    // On the carriageway, riding east along it at the angle it came in.
    assert!(st.lateral_offset.abs() <= 3.0, "{st:?}");
    assert!((st.heading.abs() - 0.5).abs() < 0.05, "{st:?}");
    let p = s.track(me).sample(st.distance);
    assert!(p.direction[0] > 0.99, "{p:?}");
    let changes = s.take_path_changes();
    assert!(changes.iter().any(|c| c.rider == me && c.reset));
    // Wild mode stays on (spec 13A *Back to the road*).
    assert!(s.riders[&me].wild);
}

#[test]
fn out_of_wild_modes_band_into_the_terrain_and_j_back_to_the_road() {
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    pedal(&mut s, me, 1, 250.0);
    ride(&mut s, 5.0);
    s.set_wild(me, true);
    s.set_lean(me, 1, -0.3);
    let mut out = false;
    for _ in 0..(60.0 / SIM_DT) as usize {
        s.step();
        let st = s.rider_state(me).unwrap();
        if s.off_road(me).is_some() {
            out = true;
            break;
        }
        assert!(st.lateral_offset.abs() <= physics::WILD_LIMIT_M + 1e-9);
    }
    assert!(out, "left the band");
    let p = s.off_road(me).unwrap();
    assert!((p.z - (500.0 - physics::WILD_LIMIT_M)).abs() < 2.0, "{p:?}");
    // J: back on the nearest road, standing.
    s.rejoin_route(me, None).unwrap();
    assert!(s.off_road(me).is_none());
    let st = s.rider_state(me).unwrap();
    assert_eq!(st.speed, 0.0);
    let q = s.track(me).sample(st.distance).position;
    assert!((q[2] - 500.0).abs() < 0.5, "{q:?}");
}

#[test]
fn off_the_road_on_a_fixed_route_the_band_holds() {
    // Wild mode keeps its 60 m on a route: no terrain riding there.
    let mut s = Session::new(Route::test_loop());
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    pedal(&mut s, me, 1, 250.0);
    ride(&mut s, 3.0);
    s.set_wild(me, true);
    s.set_lean(me, 1, 0.3);
    ride(&mut s, 40.0);
    assert!(s.off_road(me).is_none());
}

#[test]
fn flying_back_from_the_terrain_returns_there() {
    let mut s = free();
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    land(&mut s, me, 900.0, 800.0, NORTH).unwrap();
    pedal(&mut s, me, 1, 200.0);
    ride(&mut s, 5.0);
    let before = s.off_road(me).unwrap();
    s.enter_god(me).unwrap();
    ride(&mut s, 2.0);
    s.leave_god(me, GodLeave::FlyBack).unwrap();
    let back = s.off_road(me).unwrap();
    assert!((back.x - before.x).abs() < 1e-9 && (back.z - before.z).abs() < 1e-9);
}
