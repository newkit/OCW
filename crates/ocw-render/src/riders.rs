//! Rider avatars (placeholder geometry until the asset pipeline exists, Phase 4).

use std::collections::BTreeMap;

use bevy::prelude::*;
use ocw_core::RiderId;

use crate::{RideView, to_vec3};

pub struct RidersPlugin;

impl Plugin for RidersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderEntities>()
            .add_systems(PostUpdate, sync_riders.before(TransformSystems::Propagate));
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
        .spawn((transform, Visibility::default(), Avatar { id, is_local }))
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
            // Legs.
            for x in [-0.1, 0.1] {
                a.spawn((
                    Mesh3d(meshes.add(Capsule3d::new(0.07, 0.45))),
                    MeshMaterial3d(dark.clone()),
                    Transform::from_xyz(x, 0.75, 0.2),
                ));
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
