//! Rider avatars (placeholder geometry until the asset pipeline exists, Phase 4).

use std::collections::BTreeMap;

use bevy::prelude::*;
use ocw_core::RiderId;

use crate::{Ground, RideView, to_vec3};

pub struct RidersPlugin;

impl Plugin for RidersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderEntities>().add_systems(
            PostUpdate,
            (sync_riders, pedal)
                .chain()
                .before(TransformSystems::Propagate),
        );
    }
}

/// Marks a rider avatar entity.
#[derive(Component)]
pub struct Avatar {
    pub id: RiderId,
    pub is_local: bool,
}

#[derive(Resource, Default)]
struct RiderEntities(BTreeMap<RiderId, Entity>);

/// Crank angle of an avatar in radians.
#[derive(Component, Default)]
struct Crank(f32);

/// A leg, rotating about the hip with the crank; `phase` is π for the right leg.
#[derive(Component)]
struct PedalLeg {
    phase: f32,
}

/// Jersey colour of the local rider: #FF1472.
pub const LOCAL_JERSEY: Color = Color::srgb(1.0, 20.0 / 255.0, 114.0 / 255.0);

/// Jersey colour: the local rider's own colour, greens to violets for others.
pub fn jersey_color(id: RiderId, is_local: bool) -> Color {
    if is_local {
        LOCAL_JERSEY
    } else {
        // Green to violet only (150°–290°), never the local rider's pink.
        Color::hsl(150.0 + (id.0 as f32 * 47.0) % 140.0, 0.6, 0.45)
    }
}

/// Cadence assumed for riders without cadence data while they move.
pub const DEFAULT_CADENCE_RPM: f64 = 85.0;
/// Maximum leg swing about the hip, in radians.
const LEG_SWING: f32 = 0.6;

/// Crank speed in rad/s: the measured cadence, or a default cadence while
/// the rider moves (others' cadence is not sent over the network).
pub fn crank_rate(cadence_rpm: Option<f64>, speed_mps: f64) -> f32 {
    let rpm = cadence_rpm.unwrap_or(if speed_mps > 1.0 {
        DEFAULT_CADENCE_RPM
    } else {
        0.0
    });
    (rpm / 60.0 * std::f64::consts::TAU) as f32
}

/// Leg rotation about the hip for a crank angle.
pub fn leg_rotation(crank: f32, phase: f32) -> Quat {
    Quat::from_rotation_x((crank + phase).sin() * LEG_SWING)
}

/// Turns the cranks and swings the legs.
fn pedal(
    time: Res<Time>,
    view: Res<RideView>,
    mut avatars: Query<(&Avatar, &mut Crank)>,
    mut legs: Query<(&PedalLeg, &ChildOf, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (avatar, mut crank) in &mut avatars {
        if let Some(r) = view.riders.iter().find(|r| r.id == avatar.id) {
            let rate = crank_rate(r.cadence_rpm, r.state.speed);
            crank.0 = (crank.0 + rate * dt) % std::f32::consts::TAU;
        }
    }
    for (leg, parent, mut t) in &mut legs {
        if let Ok((_, crank)) = avatars.get(parent.parent()) {
            t.rotation = leg_rotation(crank.0, leg.phase);
        }
    }
}

/// Position and orientation of a rider on the route.
pub fn rider_transform(view: &RideView, distance: f64, lateral_offset: f64) -> Transform {
    let p = view.route.sample(distance);
    let pos = to_vec3(p.offset_position(lateral_offset));
    // Bevy models face -Z; the route direction is (x, z).
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let pitch = (p.grade as f32).atan();
    let yaw = Transform::default().looking_to(forward, Vec3::Y).rotation;
    Transform::from_translation(pos).with_rotation(yaw * Quat::from_rotation_x(pitch))
}

/// Distance beside the road over which the avatar goes from the road's
/// height onto the ground (m).
pub const OFF_ROAD_BLEND_M: f64 = 2.0;

/// Largest lean angle (radians, ~35°).
const MAX_LEAN: f64 = 0.61;

/// Lean into a curve (radians, positive = to the right): a bike in a
/// steady turn leans by atan(v² κ / g).
pub fn lean_angle(curvature: f64, speed_mps: f64) -> f64 {
    (speed_mps * speed_mps * curvature / 9.81)
        .atan()
        .clamp(-MAX_LEAN, MAX_LEAN)
}

/// Rotation (about the travel direction, through the tyre contact) that
/// leans the avatar; Bevy models face -Z, so a lean to the right is a
/// negative rotation about Z.
pub fn lean_rotation(lean: f64) -> Quat {
    Quat::from_rotation_z(-lean as f32)
}

/// Half the wheelbase of the avatar (wheel contacts at ±this along the bike).
pub const HALF_WHEELBASE_M: f64 = 0.55;

/// Height of the surface under a point `along` metres ahead of the rider
/// (along its heading): the road there, blended onto the ground beside
/// the road over [`OFF_ROAD_BLEND_M`] (no jump at the edge, no wheels in
/// an embankment).
fn contact_height(
    view: &RideView,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
    along: f64,
) -> (Vec3, f32) {
    let d = state.distance + along * state.heading.cos();
    let lateral = state.lateral_offset + along * state.heading.sin();
    let p = to_vec3(view.route.sample(d).offset_position(lateral));
    let beyond = lateral.abs() - view.route.road_width_m() / 2.0;
    let mut y = p.y;
    if beyond > 0.0
        && let Some(h) = ground.and_then(|g| (g.0)(f64::from(p.x), f64::from(p.z)))
    {
        let w = (beyond / OFF_ROAD_BLEND_M).min(1.0) as f32;
        y += (h as f32 - y) * w;
    }
    (p, y)
}

/// The avatar: turned by the rider's heading relative to the road, leaning
/// for the curve it rides, and standing on the surface under both wheels
/// (road, or the ground beside it): over a bump the front wheel rises
/// first, then the rear.
pub fn avatar_transform(
    view: &RideView,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
) -> Transform {
    let mut t = rider_transform(view, state.distance, state.lateral_offset);
    let (front_p, front) = contact_height(view, state, ground, HALF_WHEELBASE_M);
    let (rear_p, rear) = contact_height(view, state, ground, -HALF_WHEELBASE_M);
    t.translation.y = 0.5 * (front + rear);
    let run = Vec2::new(front_p.x - rear_p.x, front_p.z - rear_p.z)
        .length()
        .max(1e-3);
    let pitch = (front - rear).atan2(run);
    let p = view.route.sample(state.distance);
    let road = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let yaw = Transform::default().looking_to(road, Vec3::Y).rotation;
    // Positive heading = right; Bevy turns left for a positive Y rotation.
    t.rotation = Quat::from_rotation_y(-state.heading as f32)
        * yaw
        * Quat::from_rotation_x(pitch)
        * lean_rotation(lean_angle(state.turn_curvature, state.speed));
    t
}

fn sync_riders(
    mut commands: Commands,
    view: Res<RideView>,
    mut entities: ResMut<RiderEntities>,
    mut transforms: Query<&mut Transform, With<Avatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    ground: Option<Res<Ground>>,
) {
    // Remove riders that left.
    let present: Vec<RiderId> = view.riders.iter().map(|r| r.id).collect();
    entities.0.retain(|id, e| {
        let keep = present.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for r in &view.riders {
        let t = avatar_transform(&view, &r.state, ground.as_deref());
        match entities.0.get(&r.id) {
            Some(e) => {
                if let Ok(mut tr) = transforms.get_mut(*e) {
                    *tr = t;
                }
            }
            None => {
                let e = spawn_avatar(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    r.id,
                    r.is_local,
                    t,
                );
                entities.0.insert(r.id, e);
            }
        }
    }
}

fn spawn_avatar(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    id: RiderId,
    is_local: bool,
    transform: Transform,
) -> Entity {
    let jersey = materials.add(StandardMaterial {
        base_color: jersey_color(id, is_local),
        ..default()
    });
    let dark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.5,
        ..default()
    });
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.68, 0.55),
        ..default()
    });
    let wheel = meshes.add(Torus::new(0.3, 0.35));
    commands
        .spawn((
            transform,
            Visibility::default(),
            Avatar { id, is_local },
            Crank::default(),
        ))
        .with_children(|a| {
            // Wheels (front at -Z).
            for z in [-0.55, 0.55] {
                a.spawn((
                    Mesh3d(wheel.clone()),
                    MeshMaterial3d(dark.clone()),
                    Transform::from_xyz(0.0, 0.35, z)
                        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ));
            }
            // Frame.
            a.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.06, 0.08, 1.05))),
                MeshMaterial3d(dark.clone()),
                Transform::from_xyz(0.0, 0.62, 0.0),
            ));
            // Torso, leaning forward.
            a.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.17, 0.5))),
                MeshMaterial3d(jersey.clone()),
                Transform::from_xyz(0.0, 1.15, 0.05).with_rotation(Quat::from_rotation_x(-1.0)),
            ));
            // Legs: pivot at the hip, swinging with the crank.
            let leg = meshes.add(Capsule3d::new(0.07, 0.5));
            for (x, phase) in [(-0.1, 0.0), (0.1, std::f32::consts::PI)] {
                a.spawn((
                    Transform::from_xyz(x, 1.0, 0.2),
                    Visibility::default(),
                    PedalLeg { phase },
                ))
                .with_children(|hip| {
                    hip.spawn((
                        Mesh3d(leg.clone()),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_xyz(0.0, -0.32, -0.05),
                    ));
                });
            }
            // Head.
            a.spawn((
                Mesh3d(meshes.add(Sphere::new(0.12))),
                MeshMaterial3d(skin.clone()),
                Transform::from_xyz(0.0, 1.45, -0.32),
            ));
            if is_local {
                // Classic cotton cycling cap: white crown, band in the
                // jersey colour, short brim flipped up.
                let white = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.95, 0.93),
                    perceptual_roughness: 0.9,
                    ..default()
                });
                a.spawn((
                    Mesh3d(meshes.add(Sphere::new(0.126))),
                    MeshMaterial3d(white),
                    Transform::from_xyz(0.0, 1.5, -0.32).with_scale(Vec3::new(1.0, 0.62, 1.05)),
                ));
                a.spawn((
                    Mesh3d(meshes.add(Cylinder::new(0.127, 0.035))),
                    MeshMaterial3d(jersey.clone()),
                    Transform::from_xyz(0.0, 1.49, -0.32),
                ));
                a.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.17, 0.012, 0.075))),
                    MeshMaterial3d(jersey.clone()),
                    Transform::from_xyz(0.0, 1.5, -0.47).with_rotation(Quat::from_rotation_x(0.45)),
                ));
            } else {
                // Helmet.
                a.spawn((
                    Mesh3d(meshes.add(Sphere::new(0.135))),
                    MeshMaterial3d(jersey.clone()),
                    Transform::from_xyz(0.0, 1.5, -0.3).with_scale(Vec3::new(1.0, 0.7, 1.2)),
                ));
            }
        })
        .id()
}

#[cfg(test)]
mod tests {
    use ocw_world::Route;

    use super::*;

    fn view(points: Vec<[f64; 3]>) -> RideView {
        RideView::new(Route::from_points("t", "T", points, false, 6.0).unwrap())
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn local_rider_wears_ff1472() {
        let c = jersey_color(RiderId(7), true).to_srgba();
        let hex = (
            (c.red * 255.0).round() as u8,
            (c.green * 255.0).round() as u8,
            (c.blue * 255.0).round() as u8,
        );
        assert_eq!(hex, (0xFF, 0x14, 0x72));
        // Other riders are never pink/red (hue 150°–290°).
        for id in 0..200 {
            let hue = Hsla::from(jersey_color(RiderId(id), false)).hue;
            assert!((150.0..=290.0).contains(&hue), "rider {id}: hue {hue}");
        }
    }

    #[test]
    fn cranks_turn_with_cadence_or_movement() {
        // 90 rpm = 1.5 turns per second.
        assert!((crank_rate(Some(90.0), 0.0) - 3.0 * std::f32::consts::PI).abs() < 1e-5);
        // Measured 0 rpm: legs stand still even when coasting.
        assert_eq!(crank_rate(Some(0.0), 10.0), 0.0);
        // Others (no cadence): default cadence while moving, still when stopped.
        assert!(crank_rate(None, 8.0) > 0.0);
        assert_eq!(crank_rate(None, 0.5), 0.0);
        // Left and right legs swing in opposite directions.
        let (l, r) = (
            leg_rotation(1.0, 0.0),
            leg_rotation(1.0, std::f32::consts::PI),
        );
        assert!((l.to_euler(EulerRot::XYZ).0 + r.to_euler(EulerRot::XYZ).0).abs() < 1e-5);
        assert!(l.to_euler(EulerRot::XYZ).0.abs() <= LEG_SWING + 1e-6);
    }

    #[test]
    fn faces_along_the_road_and_offsets_to_the_right() {
        // Road heading east (+x); right of east is south (+z).
        let v = view(vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        let t = rider_transform(&v, 10.0, 0.0);
        assert!(close(t.translation, Vec3::new(10.0, 0.0, 0.0)));
        assert!(close(t.forward().as_vec3(), Vec3::X));
        let right = rider_transform(&v, 10.0, 1.5);
        assert!(close(right.translation, Vec3::new(10.0, 0.0, 1.5)));
        assert!(close(right.right().as_vec3(), Vec3::Z));
    }

    #[test]
    fn leans_into_curves() {
        // A circle of radius 50 m, clockwise seen from above (x east, z
        // south): heading east, then turning towards south = right.
        let r = 50.0;
        let pts: Vec<[f64; 3]> = (0..120)
            .map(|i| {
                let a = f64::from(i) / 120.0 * std::f64::consts::TAU;
                [r * a.sin(), 0.0, r - r * a.cos()]
            })
            .collect();
        let v = RideView::new(Route::from_points("c", "C", pts, true, 6.0).unwrap());
        let k = v.route.curvature(80.0);
        assert!((k - 1.0 / r).abs() < 0.002, "curvature {k}");
        // 10 m/s on R 50: atan(100 / (50 * 9.81)) = 11.5° to the right.
        let lean = lean_angle(k, 10.0);
        assert!(
            (lean.to_degrees() - 11.5).abs() < 0.6,
            "{}",
            lean.to_degrees()
        );
        let mut t = rider_transform(&v, 80.0, 0.0);
        let up_before = t.up().as_vec3();
        t.rotation *= lean_rotation(lean);
        // The top of the rider moves to the right (towards the centre).
        let shift = (t.up().as_vec3() - up_before).dot(t.right().as_vec3());
        assert!(shift > 0.15, "shift {shift}");
        // Straight road and standing still: upright; capped when too fast.
        let straight = view(vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        assert!(straight.route.curvature(50.0).abs() < 1e-9);
        assert_eq!(lean_angle(k, 0.0), 0.0);
        assert_eq!(lean_angle(k, 40.0), MAX_LEAN);
        assert_eq!(lean_angle(-k, 40.0), -MAX_LEAN);
    }

    #[test]
    fn lean_steering_turns_and_tilts_the_avatar() {
        let v = view(vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0]]);
        let state = ocw_sim::RiderState {
            distance: 100.0,
            speed: 10.0,
            heading: 0.2,
            turn_curvature: 0.02,
            ..Default::default()
        };
        let t = avatar_transform(&v, &state, None);
        // Heading east, turned 0.2 rad to the right (towards +z, south).
        let f = t.forward().as_vec3();
        assert!((f.z - 0.2f32.sin()).abs() < 1e-3, "forward {f:?}");
        // Leaning right: the top tilts towards the rider's right.
        let upright = avatar_transform(
            &v,
            &ocw_sim::RiderState {
                turn_curvature: 0.0,
                ..state
            },
            None,
        );
        let shift = (t.up().as_vec3() - upright.up().as_vec3()).dot(upright.right().as_vec3());
        assert!(shift > 0.15, "shift {shift}");
        // Off the road (wild mode), on the ground instead of the road plane.
        let ground = Ground(std::sync::Arc::new(|_, _| Some(12.0)));
        let off = ocw_sim::RiderState {
            lateral_offset: 20.0,
            ..state
        };
        assert_eq!(
            avatar_transform(&v, &off, Some(&ground)).translation.y,
            12.0
        );
        assert_eq!(
            avatar_transform(&v, &state, Some(&ground)).translation.y,
            0.0
        );
        // Just beside the road (1 m of the 2 m blend): halfway onto the ground.
        let edge = ocw_sim::RiderState {
            lateral_offset: v.route.road_width_m() / 2.0 + 1.0,
            ..state
        };
        let y = avatar_transform(&v, &edge, Some(&ground)).translation.y;
        assert!((y - 6.0).abs() < 1e-4, "{y}");
    }

    #[test]
    fn front_wheel_climbs_a_bump_first() {
        // Flat road at 0 m with a 0.3 m step up at x = 50 (5 cm ramp).
        let pts: Vec<[f64; 3]> = (0..=400)
            .map(|i| {
                let x = f64::from(i) * 0.25;
                [x, if x >= 50.0 { 0.3 } else { 0.0 }, 0.0]
            })
            .collect();
        let v = view(pts);
        let at = |d: f64| {
            let s = ocw_sim::RiderState {
                distance: d,
                speed: 5.0,
                ..Default::default()
            };
            let t = avatar_transform(&v, &s, None);
            (t.translation.y, t.forward().as_vec3().y)
        };
        // Before the step: level. Front wheel on it: nose up, half lifted.
        let (y0, f0) = at(48.0);
        assert!(y0.abs() < 1e-4 && f0.abs() < 1e-4, "{y0} {f0}");
        let (y1, f1) = at(50.0);
        assert!(f1 > 0.2 && (y1 - 0.15).abs() < 0.03, "{y1} {f1}");
        // Both wheels up: level again, 0.3 m higher.
        let (y2, f2) = at(52.0);
        assert!((y2 - 0.3).abs() < 1e-4 && f2.abs() < 1e-4, "{y2} {f2}");
    }

    #[test]
    fn pitches_up_on_climbs() {
        // 10 % climb heading north (-z).
        let v = view(vec![[0.0, 0.0, 0.0], [0.0, 10.0, -100.0]]);
        let t = rider_transform(&v, 50.0, 0.0);
        let f = t.forward().as_vec3();
        assert!(f.y > 0.09 && f.y < 0.11, "forward {f:?}");
        assert!(f.z < -0.99);
        assert!((t.translation.y - 5.0).abs() < 0.1);
    }
}
