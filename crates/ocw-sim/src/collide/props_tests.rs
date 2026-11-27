//! The point obstacles' and street lamps' solids from their model files'
//! collision proxies (spec 15H / 15N, Phase 12), and riders hitting a
//! street lamp's pole.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use ocw_assets::model::{Model, Node, Primitive};
use ocw_world::graph::RoadClass;
use ocw_world::obstacles::{ObstacleKind, PointShapes, ShapePart, WorldObstacles};
use ocw_world::{RoadDress, RoadLit, RoadStrip, RoadSurface, Tile, TileKey, WorldPackage};

use super::*;

/// The sizes riders hit before the proxies were read
/// ([`ObstacleKind::radius_m`]) against the files' proxies: bollards,
/// posts, gates and the other barriers with posts stay posts at their
/// point, within 0.08 m of the old radius (posts 0.13 m for 0.15, a
/// gate's posts 0.08 m for 0.15, the generic barrier post 0.25 m for
/// 0.3); a concrete block is its 1.2 × 0.6 m box (the old 0.5 m circle
/// within 0.2 m), a cycle barrier its two staggered rails, a street lamp
/// its 0.14 m pole.
#[test]
fn point_shapes_come_from_the_files_proxies_at_the_old_sizes() {
    let s = props_shapes();
    for (kind, want, tol) in [
        (ObstacleKind::Bollard, 0.10, 0.005),
        (ObstacleKind::Post, 0.13, 0.005),
        (ObstacleKind::Gate, 0.08, 0.005),
        (ObstacleKind::LiftGate, 0.15, 0.005),
        (ObstacleKind::Barrier, 0.25, 0.005),
        (ObstacleKind::Pylon, 2.5, 0.005),
    ] {
        let p = s.parts(kind);
        assert_eq!(p.len(), 1, "{kind:?}: {p:?}");
        assert!(p[0].round && p[0].centre == [0.0, 0.0], "{kind:?}: {p:?}");
        let r = p[0].half[0];
        assert!((r - want).abs() < tol, "{kind:?}: {r}");
        assert!(
            (r - kind.radius_m()).abs() <= 0.08,
            "{kind:?}: proxy {r} m, before {} m",
            kind.radius_m()
        );
    }
    let block = s.parts(ObstacleKind::Block);
    assert_eq!(block.len(), 1);
    assert!(!block[0].round);
    assert!((block[0].half[0] - 0.6).abs() < 1e-3 && (block[0].half[1] - 0.3).abs() < 1e-3);
    assert!((block[0].half[0] - ObstacleKind::Block.radius_m()).abs() <= 0.2);
    let cycle = s.parts(ObstacleKind::CycleBarrier);
    assert_eq!(cycle.len(), 2, "{cycle:?}");
    assert!(
        cycle
            .iter()
            .all(|p| !p.round && p.half[0] > 0.7 && p.half[1] < 0.05)
    );
    assert!((s.reach_m(ObstacleKind::CycleBarrier) - 1.4).abs() < 0.1);
    let pole = s.parts(ObstacleKind::StreetLamp);
    assert_eq!(pole.len(), 1);
    assert!(
        (pole[0].half[0] - ocw_world::lamps::POLE_R_M).abs() < 1e-4,
        "{pole:?}"
    );
    // A square pole is a circle of its half width.
    assert_eq!(
        pole[0].shape([1.0, 2.0], 0.3),
        Shape::Circle {
            c: [1.0, 2.0],
            r: pole[0].half[0]
        }
    );
    // Every reach within the search margin round a body.
    for k in ObstacleKind::ALL.iter().filter(|k| !k.is_line()) {
        assert!(s.reach_m(*k) <= MAX_POINT_RADIUS_M, "{k:?}");
    }
    // Trees have no file: their trunk as before.
    assert_eq!(
        s.parts(ObstacleKind::PlantedTree),
        vec![ShapePart::circle(ObstacleKind::PlantedTree.radius_m())]
    );
    assert_eq!(
        PointShapes::radii().parts(ObstacleKind::Post),
        vec![ShapePart::circle(0.15)]
    );
}

fn box_mesh(lo: Vec3, hi: Vec3) -> Primitive {
    let mut p = Primitive::default();
    for c in 0..8u32 {
        let pick = |b: u32, a: f32, z: f32| if c & b == 0 { a } else { z };
        p.positions.push([
            pick(1, lo.x, hi.x),
            pick(2, lo.y, hi.y),
            pick(4, lo.z, hi.z),
        ]);
        p.normals.push([0.0, 1.0, 0.0]);
    }
    p.indices = vec![0, 1, 2];
    p
}

/// Both proxy forms (15H Q9 is open): a modelled mesh and a scaled empty
/// give the same solid – a post, and a box off the origin.
#[test]
fn both_proxy_forms_give_the_same_solid() {
    let empties = {
        let mut m = Model::default();
        let root = m.add(Node::new("thing", None, Vec3::ZERO));
        let mut c = Node::new("col_box", Some(root), Vec3::new(0.4, 0.5, -0.2));
        c.scale = Vec3::new(0.6, 0.5, 0.1);
        m.add(c);
        m
    };
    let meshes = {
        let mut m = Model::default();
        let root = m.add(Node::new("thing", None, Vec3::ZERO));
        let mut c = Node::new("col_box", Some(root), Vec3::ZERO);
        c.primitives.push(box_mesh(
            Vec3::new(-0.2, 0.0, -0.3),
            Vec3::new(1.0, 1.0, -0.1),
        ));
        m.add(c);
        m
    };
    let (a, b) = (shape_of_model(&empties), shape_of_model(&meshes));
    assert_eq!(a.len(), 1);
    for (x, y) in a.iter().zip(&b) {
        assert!(!x.round && !y.round);
        for k in 0..2 {
            assert!((x.centre[k] - y.centre[k]).abs() < 1e-5, "{a:?} {b:?}");
            assert!((x.half[k] - y.half[k]).abs() < 1e-5, "{a:?} {b:?}");
        }
    }
    // A box turned with its way: a wall along its longer side.
    match a[0].shape([10.0, 10.0], std::f64::consts::FRAC_PI_2) {
        Shape::Segment { a: p, b: q, half } => {
            assert!((half - 0.1).abs() < 1e-6);
            // Model x turns to world −z by a quarter turn.
            assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() > 0.9);
        }
        s => panic!("{s:?}"),
    }
    // Posts: a cylinder as an empty or a mesh.
    let post = |mesh: bool| {
        let mut m = Model::default();
        let root = m.add(Node::new("post", None, Vec3::ZERO));
        let mut c = Node::new("col_cylinder", Some(root), Vec3::new(0.0, 1.0, 0.0));
        if mesh {
            c.primitives.push(box_mesh(
                Vec3::new(-0.12, -1.0, -0.12),
                Vec3::new(0.12, 1.0, 0.12),
            ));
        } else {
            c.scale = Vec3::new(0.12, 1.0, 0.12);
        }
        m.add(c);
        shape_of_model(&m)
    };
    assert_eq!(post(true), post(false));
    assert!((post(true)[0].half[0] - 0.12).abs() < 1e-6);
}

/// A lit residential road (6 m) along x at z = 0 through tile (0, 0): its
/// lamps stand right of it (+z) every 32 m.
pub(crate) fn lit_road_package() -> WorldPackage {
    let class = RoadClass::Residential;
    let strip = RoadStrip {
        class,
        width_m: 6.0,
        points: (0..=100).map(|i| [i as f32 * 10.0, 0.0, 0.0]).collect(),
        surface: RoadSurface::of_class(class),
        dress: RoadDress::of_class(class, 6.0),
        lit: RoadLit::Yes,
        condition: Default::default(),
    };
    let key = TileKey { x: 0, z: 0 };
    WorldPackage {
        name: "lit road".into(),
        origin: (47.0, 9.0),
        tile_size_m: 1000.0,
        samples: 2,
        tiles: BTreeMap::from([(
            key,
            Tile {
                key,
                heights: vec![0.0; 4],
                roads: vec![strip],
            },
        )]),
        graph: None,
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

/// The poles in the obstacles are the lamps the apps draw, both kinds: a
/// package's lamps ([`ocw_world::lamps::package_lamps`]) – also after the
/// package went through its file (what a remote client reads).
#[test]
fn the_obstacles_have_the_drawn_lamps() {
    let mut pkg = lit_road_package();
    let mut o = ocw_world::obstacles::Obstacles::new(1000.0);
    o.tiles.insert(
        TileKey { x: 0, z: 0 },
        ocw_world::obstacles::TileObstacles {
            points: vec![ocw_world::obstacles::PointObstacle {
                kind: ObstacleKind::StreetLamp,
                flags: 0,
                pos: [205.0, 9.0],
                angle: 0.0,
                width_m: 0.0,
                height_m: 0.0,
            }],
            ..Default::default()
        },
    );
    pkg.obstacles = Some(o);
    let w = WorldObstacles::from_package(&pkg).unwrap();
    let lamps = ocw_world::lamps::package_lamps(&pkg);
    let feet: Vec<[f32; 2]> = lamps
        .values()
        .flatten()
        .map(|l| [l.foot[0], l.foot[2]])
        .collect();
    assert_eq!(w.lamp_count(), feet.len());
    let got: Vec<[f32; 2]> = w
        .tile_lamp_feet(TileKey { x: 0, z: 0 })
        .iter()
        .map(|f| f.0)
        .collect();
    assert_eq!(got, feet);
    assert!(got.contains(&[205.0, 9.0]), "the mapped lamp");
    let read = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
    assert_eq!(WorldObstacles::from_package(&read).unwrap(), w);
    // Each pole is in the tile's index, solid.
    let index = w
        .tile_index_with(TileKey { x: 0, z: 0 }, props_shapes())
        .unwrap();
    let poles = index
        .solids()
        .iter()
        .filter(|s| s.kind == ObstacleKind::StreetLamp)
        .count();
    assert_eq!(poles, feet.len());
}

/// The session's rider rides into a lamp pole beside the road (wild
/// mode, collisions on): a crash at speed, a bump that stops a slow
/// rider without passing through – and the client's prediction stops
/// where the server does; collisions off: through it.
#[test]
fn a_rider_hits_a_lamp_pole() {
    let pkg = lit_road_package();
    let world = WorldObstacles::from_package(&pkg).map(Arc::new);
    let foot = world
        .as_ref()
        .unwrap()
        .tile_lamp_feet(TileKey { x: 0, z: 0 })[3]
        .0;
    let (x, z) = (f64::from(foot[0]), f64::from(foot[1]));
    assert!(z > 3.3, "beside the road: {foot:?}");
    let route = ocw_world::Route::from_points(
        "straight",
        "Straight",
        vec![[0.0, 0.0, 0.0], [2000.0, 0.0, 0.0]],
        false,
        6.0,
    )
    .unwrap();
    let ride = |speed: f64, collisions: bool| {
        let mut s = crate::session::Session::new(route.clone());
        s.set_collisions(collisions);
        s.set_obstacles(world.clone());
        let id = s.add_rider("ada", ocw_core::RiderProfile::default());
        for _ in 0..=110 {
            s.step();
        }
        let start = crate::physics::RiderState {
            distance: x - 5.0,
            lateral_offset: z,
            speed,
            ..Default::default()
        };
        s.set_rider_state(id, start);
        s.set_wild(id, true);
        s.set_lean(id, 1, 0.0);
        let mut p = crate::prediction::Predictor::new(ocw_core::RiderProfile::default(), start);
        p.set_world(world.clone());
        p.set_collision_mode(if collisions { Mode::Full } else { Mode::Off });
        p.set_wild(true);
        p.set_lean(0.0);
        for _ in 0..150 {
            s.step();
            p.step(&route, 0.0);
        }
        (
            s.crash(id).map(|c| c.kind),
            s.rider_state(id).unwrap(),
            p.state(),
        )
    };
    // At 8 m/s: a crash into the street lamp.
    let (crash, _, _) = ride(8.0, true);
    assert_eq!(crash, Some(ContactKind::World(ObstacleKind::StreetLamp)));
    // Slowly: stopped at the pole, not through it; the prediction agrees.
    let (crash, server, client) = ride(2.0, true);
    assert_eq!(crash, None);
    assert!(
        server.distance + BODY_LENGTH_M / 2.0 <= x + 0.01,
        "{server:?}"
    );
    assert!(
        (client.distance - server.distance).abs() < 0.05,
        "{client:?} {server:?}"
    );
    // Collisions off: no pole.
    let (crash, server, _) = ride(8.0, false);
    assert_eq!(crash, None);
    assert!(server.distance > x + 2.0, "{server:?}");
}

/// `dev.lamp`: a rider in wild mode is put before the nearest lamp pole
/// beside the road and bumps into it slowly (no crash; stopped at it).
#[test]
fn dev_lamp_stages_a_bump_into_a_lamp_pole() {
    use crate::session::Stage;
    let pkg = lit_road_package();
    let world = WorldObstacles::from_package(&pkg).map(Arc::new);
    let route = ocw_world::Route::from_points(
        "straight",
        "Straight",
        vec![[0.0, 0.0, 0.0], [2000.0, 0.0, 0.0]],
        false,
        6.0,
    )
    .unwrap();
    let mut s = crate::session::Session::new(route);
    s.set_collisions(true);
    s.set_obstacles(world);
    let id = s.add_rider("bea", ocw_core::RiderProfile::default());
    for _ in 0..=110 {
        s.step();
    }
    s.set_rider_state(
        id,
        crate::physics::RiderState {
            distance: 100.0,
            speed: 5.0,
            ..Default::default()
        },
    );
    assert!(s.stage(id, Stage::Lamp).is_err(), "wild mode first");
    s.set_wild(id, true);
    let msg = s.stage(id, Stage::Lamp).unwrap();
    assert!(msg.contains("street lamp"), "{msg}");
    let start = s.rider_state(id).unwrap();
    for _ in 0..200 {
        s.step();
    }
    assert!(s.crash(id).is_none());
    let end = s.rider_state(id).unwrap();
    // Stopped at the pole, 6 m ahead (less half a bike).
    assert!(end.distance < start.distance + 6.0, "{start:?} {end:?}");
    assert!(end.distance > start.distance + 4.0, "{start:?} {end:?}");
    assert!(end.speed < 0.5, "{end:?}");
}
