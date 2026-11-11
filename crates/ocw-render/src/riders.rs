//! Rider avatars (placeholder geometry until the asset pipeline exists, Phase 4).

use std::collections::BTreeMap;

use bevy::prelude::*;
use ocw_core::RiderId;

use crate::{RideView, to_vec3};

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

fn sync_riders(
    mut commands: Commands,
    view: Res<RideView>,
    mut entities: ResMut<RiderEntities>,
    mut transforms: Query<&mut Transform, With<Avatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
        let t = rider_transform(&view, r.state.distance, r.state.lateral_offset);
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
    // Jersey colour: orange for the local rider, varied blues/greens for others.
    let jersey = if is_local {
        Color::srgb(1.0, 0.45, 0.05)
    } else {
        Color::hsl((190.0 + id.0 as f32 * 47.0) % 360.0, 0.6, 0.45)
    };
    let jersey = materials.add(StandardMaterial {
        base_color: jersey,
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
            // Head with helmet.
            a.spawn((
                Mesh3d(meshes.add(Sphere::new(0.12))),
                MeshMaterial3d(skin.clone()),
                Transform::from_xyz(0.0, 1.45, -0.32),
            ));
            a.spawn((
                Mesh3d(meshes.add(Sphere::new(0.135))),
                MeshMaterial3d(jersey.clone()),
                Transform::from_xyz(0.0, 1.5, -0.3).with_scale(Vec3::new(1.0, 0.7, 1.2)),
            ));
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
