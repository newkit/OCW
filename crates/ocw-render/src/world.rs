//! Static world: ground, road, scenery, light (Phase 1 test route only).

use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use crate::{RideView, to_vec3};

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.62, 0.78, 0.95)))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 600.0,
                ..default()
            })
            .add_systems(Startup, spawn_world);
    }
}

/// Sky color, also used for the distance fog.
pub const SKY: Color = Color::srgb(0.62, 0.78, 0.95);

fn spawn_world(
    mut commands: Commands,
    view: Res<RideView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let route = &view.route;

    // Sun.
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(300.0, 600.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            maximum_distance: 400.0,
            first_cascade_far_bound: 30.0,
            ..default()
        }
        .build(),
    ));

    // Ground, slightly below the lowest road point.
    let min_y = route
        .points()
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min) as f32;
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(6000.0, 6000.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.33, 0.52, 0.25),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, min_y - 0.6, 0.0),
    ));

    // Road and its embankment (a wider, lower ribbon, so the road never floats).
    let width = route.road_width_m() as f32;
    commands.spawn((
        Mesh3d(meshes.add(ribbon_mesh(&view, width + 6.0, -0.05, -0.65))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.38, 0.5, 0.28),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));
    commands.spawn((
        Mesh3d(meshes.add(ribbon_mesh(&view, width, 0.0, 0.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.22, 0.22, 0.24),
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));
    commands.spawn((
        Mesh3d(meshes.add(center_line_mesh(&view))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.95, 0.9),
            perceptual_roughness: 0.6,
            ..default()
        })),
    ));

    spawn_trees(&mut commands, &view, &mut meshes, &mut materials, min_y);
    spawn_start_arch(&mut commands, &view, &mut meshes, &mut materials);
}

/// A strip along the route, `width` wide, raised by `lift` at the centre and
/// `edge_drop` at the edges.
fn ribbon_mesh(view: &RideView, width: f32, lift: f32, edge_drop: f32) -> Mesh {
    let route = &view.route;
    let step = 2.0;
    let n = (route.length() / step).ceil() as usize;
    let mut positions = Vec::with_capacity((n + 1) * 2);
    let mut normals = Vec::with_capacity((n + 1) * 2);
    let mut uvs = Vec::with_capacity((n + 1) * 2);
    for i in 0..=n {
        let d = (i as f64 * step).min(route.length());
        let p = route.sample(d);
        let half = f64::from(width / 2.0);
        let mut left = to_vec3(p.offset_position(-half));
        let mut right = to_vec3(p.offset_position(half));
        left.y += lift + edge_drop;
        right.y += lift + edge_drop;
        positions.push(left.to_array());
        positions.push(right.to_array());
        normals.extend([[0.0, 1.0, 0.0]; 2]);
        let v = d as f32 / 10.0;
        uvs.push([0.0, v]);
        uvs.push([1.0, v]);
    }
    let mut indices = Vec::with_capacity(n * 6);
    for i in 0..n as u32 {
        let (l0, r0, l1, r1) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        // Counter-clockwise seen from above, so the face points up.
        indices.extend([l0, r0, l1, r0, r1, l1]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Dashed centre line: 3 m dash, 6 m gap.
fn center_line_mesh(view: &RideView) -> Mesh {
    let route = &view.route;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut d = 0.0;
    while d + 3.0 < route.length() {
        let base = positions.len() as u32;
        for (dd, side) in [(0.0, -0.08), (0.0, 0.08), (3.0, -0.08), (3.0, 0.08)] {
            let mut v = to_vec3(route.sample(d + dd).offset_position(side));
            v.y += 0.02;
            positions.push(v.to_array());
        }
        indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        d += 9.0;
    }
    let normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Small deterministic pseudo-random generator (no extra dependency needed).
struct Lcg(u64);

impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Trees along the roadside. All trees share one mesh and material each, so
/// Bevy batches them (instancing, spec §4.5).
fn spawn_trees(
    commands: &mut Commands,
    view: &RideView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    ground_y: f32,
) {
    let crown = meshes.add(Cone {
        radius: 2.2,
        height: 7.0,
    });
    let trunk = meshes.add(Cylinder::new(0.25, 2.0));
    let crown_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.13, 0.33, 0.15),
        perceptual_roughness: 1.0,
        ..default()
    });
    let trunk_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.24, 0.15),
        perceptual_roughness: 1.0,
        ..default()
    });
    let route = &view.route;
    let mut rng = Lcg(42);
    let mut d = 0.0;
    while d < route.length() {
        let p = route.sample(d);
        let side = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
        let dist = route.road_width_m() / 2.0 + 6.0 + f64::from(rng.next_f32()) * 40.0;
        let mut pos = to_vec3(p.offset_position(side * dist));
        pos.y = pos.y.min(ground_y + 30.0) - 0.5;
        let scale = 0.7 + rng.next_f32() * 0.8;
        commands
            .spawn((
                Transform::from_translation(pos).with_scale(Vec3::splat(scale)),
                Visibility::default(),
            ))
            .with_children(|tree| {
                tree.spawn((
                    Mesh3d(trunk.clone()),
                    MeshMaterial3d(trunk_mat.clone()),
                    Transform::from_xyz(0.0, 1.0, 0.0),
                ));
                tree.spawn((
                    Mesh3d(crown.clone()),
                    MeshMaterial3d(crown_mat.clone()),
                    Transform::from_xyz(0.0, 5.0, 0.0),
                ));
            });
        d += 6.0 + f64::from(rng.next_f32()) * 14.0;
    }
}

/// Start/finish arch at distance 0.
fn spawn_start_arch(
    commands: &mut Commands,
    view: &RideView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let p = view.route.sample(0.0);
    let half = view.route.road_width_m() / 2.0 + 0.8;
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.15, 0.1),
        ..default()
    });
    let post = meshes.add(Cuboid::new(0.4, 5.0, 0.4));
    for side in [-half, half] {
        let mut pos = to_vec3(p.offset_position(side));
        pos.y += 2.5;
        commands.spawn((
            Mesh3d(post.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(pos),
        ));
    }
    let mut top = to_vec3(p.position);
    top.y += 5.0;
    let yaw = (p.direction[0] as f32).atan2(p.direction[1] as f32);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(half as f32 * 2.0 + 0.4, 0.8, 0.4))),
        MeshMaterial3d(mat),
        Transform::from_translation(top)
            .with_rotation(Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_2)),
    ));
}

/// Distance fog component to attach to the camera.
pub fn fog() -> DistanceFog {
    DistanceFog {
        color: SKY,
        falloff: FogFalloff::Linear {
            start: 250.0,
            end: 1800.0,
        },
        ..default()
    }
}
