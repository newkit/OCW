//! Static world: ground, road, scenery, light (Phase 1 test route only).

use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight, NotShadowCaster};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use ocw_world::{Route, Terrain, TerrainParams};

use crate::camera::CameraInteraction;
use crate::see_through::{SeeThroughMaterial, see_through};
use crate::terrain::{far_ring_mesh, terrain_grids};
use crate::{RideView, route_bounds, to_vec3};

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.62, 0.78, 0.95)))
            // 4096² per cascade (default 2048²): sharper, less shimmering shadows.
            .insert_resource(bevy::light::DirectionalLightShadowMap { size: 4096 })
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 600.0,
                ..default()
            })
            .add_systems(Startup, spawn_world);
    }
}

/// Marks what [`spawn_world`] spawned (light, terrain, road, trees, arch),
/// so a ride switch can despawn it ([`crate::rebuild_scene`]).
#[derive(Component)]
pub struct SceneEntity;

/// Sky color, also used for the distance fog.
pub const SKY: Color = Color::srgb(0.62, 0.78, 0.95);

pub(crate) fn spawn_world(
    mut commands: Commands,
    view: Res<RideView>,
    package: Option<Res<crate::package::PackageWorld>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tree_materials: ResMut<Assets<SeeThroughMaterial>>,
) {
    let route = &view.route;

    // Sun.
    commands.spawn((
        SceneEntity,
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            // A bit more bias than the default: the coarse terrain triangles
            // otherwise shadow themselves in flickering stripes.
            shadow_depth_bias: 0.04,
            shadow_normal_bias: 2.5,
            ..default()
        },
        Transform::from_xyz(300.0, 600.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y),
        // Four cascades over 250 m with generous overlap: more shadow-map
        // texels per metre near the rider, softer cascade transitions.
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.1,
            maximum_distance: 250.0,
            first_cascade_far_bound: 20.0,
            overlap_proportion: 0.3,
        }
        .build(),
    ));

    // With a world package, terrain, roads and trees are streamed from it
    // (package.rs); the route only adds its centre line and the start arch.
    if let Some(pkg) = &package {
        // The surface exactly as meshed (triangles), not bilinear: on steep
        // slopes they differ by metres within a cell. Across all packages
        // of the world set (updated when neighbours load).
        commands.insert_resource(pkg.ground());
        if view.free_ride {
            // No route: every rider rides their own path.
            return;
        }
        commands.spawn((
            SceneEntity,
            Mesh3d(meshes.add(center_line_mesh(&view, CENTRE_LINE_LIFT_PACKAGE_M))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.95, 0.95, 0.9),
                perceptual_roughness: 0.6,
                ..default()
            })),
        ));
        spawn_start_arch(&mut commands, &view, &mut meshes, &mut materials);
        return;
    }

    // Terrain conformed to the road (spec §4.3), in three levels of detail.
    let (route_min, route_max) = route_bounds(route);
    let coarse =
        crate::terrain::near_spacing(route_min, route_max) > crate::terrain::NEAR_SPACING_M;
    let params = TerrainParams {
        // A coarser near grid interpolates more across the road: more
        // clearance, covered by deeper road edge faces.
        road_clearance_m: if coarse {
            0.3
        } else {
            TerrainParams::default().road_clearance_m
        },
        ..TerrainParams::default()
    };
    let terrain = match &view.terrain_base {
        Some(grid) => Terrain::with_base(route, params, grid.origin, grid.max(), |x, z| {
            grid.sample(x, z)
        }),
        None => Terrain::from_route(route, params),
    };
    {
        let t = terrain.clone();
        commands.insert_resource(crate::Ground(std::sync::Arc::new(move |x, z| {
            Some(t.height(x, z))
        })));
    }
    let edge_depth = if coarse {
        COARSE_ROAD_EDGE_DEPTH_M
    } else {
        ROAD_EDGE_DEPTH_M
    };
    let (near, middle) = terrain_grids(&terrain, route_min, route_max);
    let ground = materials.add(StandardMaterial {
        base_color: Color::WHITE, // vertex colours
        perceptual_roughness: 1.0,
        ..default()
    });
    for mesh in [
        near.mesh(None),
        middle.mesh(Some((near.origin, near.max()))),
        far_ring_mesh(middle.origin, middle.max(), terrain.mean_height()),
    ] {
        // Receives shadows but casts none: gentle hills barely cast visible
        // shadows, and the shadow pass would draw the whole terrain again.
        commands.spawn((
            SceneEntity,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(ground.clone()),
            NotShadowCaster,
            CameraInteraction::Solid,
        ));
    }

    // Road surface.
    let width = route.road_width_m() as f32;
    commands.spawn((
        SceneEntity,
        Mesh3d(meshes.add(ribbon_mesh(&view, width))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colours: the surface
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));
    // Road edges: side faces down into the terrain, so the gap between road
    // and terrain (road clearance) is never visible from the side.
    commands.spawn((
        SceneEntity,
        Mesh3d(meshes.add(road_edge_mesh(&view, width, edge_depth))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.45, 0.43, 0.38),
            perceptual_roughness: 1.0,
            cull_mode: None,
            double_sided: true,
            ..default()
        })),
        NotShadowCaster,
    ));
    commands.spawn((
        SceneEntity,
        Mesh3d(meshes.add(center_line_mesh(&view, 0.02))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.95, 0.9),
            perceptual_roughness: 0.6,
            ..default()
        })),
    ));

    let trees = tree_positions(route, &terrain);
    spawn_trees(&mut commands, &trees, &mut meshes, &mut tree_materials);
    spawn_start_arch(&mut commands, &view, &mut meshes, &mut materials);
}

/// The road surface: a flat strip along the route, `width` wide.
/// Colour of a road surface.
pub fn surface_color(surface: ocw_world::Surface) -> Color {
    match surface {
        ocw_world::Surface::Asphalt => Color::srgb(0.22, 0.22, 0.24),
        ocw_world::Surface::Gravel => Color::srgb(0.58, 0.52, 0.42),
        ocw_world::Surface::Grass => Color::srgb(0.30, 0.45, 0.22),
    }
}

fn ribbon_mesh(view: &RideView, width: f32) -> Mesh {
    let route = &view.route;
    let step = 2.0;
    let n = (route.length() / step).ceil() as usize;
    let mut positions = Vec::with_capacity((n + 1) * 2);
    let mut normals = Vec::with_capacity((n + 1) * 2);
    let mut uvs = Vec::with_capacity((n + 1) * 2);
    let mut colors = Vec::with_capacity((n + 1) * 2);
    for i in 0..=n {
        let d = (i as f64 * step).min(route.length());
        let p = route.sample(d);
        let c = surface_color(route.surface_at(d))
            .to_linear()
            .to_f32_array();
        colors.extend([c, c]);
        let half = f64::from(width / 2.0);
        let left = to_vec3(p.offset_position(-half));
        let right = to_vec3(p.offset_position(half));
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// How far the road edge faces reach below the road surface. Must exceed the
/// terrain's road clearance plus the grid interpolation error.
const ROAD_EDGE_DEPTH_M: f32 = 0.5;
/// Road edge depth with a coarse near grid (long real routes).
const COARSE_ROAD_EDGE_DEPTH_M: f32 = 2.0;

/// Vertical strips along both road edges, from the surface down `depth`.
fn road_edge_mesh(view: &RideView, width: f32, depth: f32) -> Mesh {
    let route = &view.route;
    let step = 2.0;
    let n = (route.length() / step).ceil() as usize;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for side in [-1.0f64, 1.0] {
        let first = positions.len() as u32;
        for i in 0..=n {
            let p = route.sample((i as f64 * step).min(route.length()));
            let top = to_vec3(p.offset_position(side * f64::from(width / 2.0)));
            positions.push(top.to_array());
            positions.push((top - Vec3::Y * depth).to_array());
            let r = p.right();
            let out = [(side * r[0]) as f32, 0.0, (side * r[1]) as f32];
            normals.extend([out, out]);
        }
        for i in 0..n as u32 {
            let (t0, b0, t1, b1) = (
                first + i * 2,
                first + i * 2 + 1,
                first + i * 2 + 2,
                first + i * 2 + 3,
            );
            indices.extend([t0, b0, t1, b0, b1, t1]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Centre line lift over package roads, whose surface is drawn 2 cm above
/// the profile (the same height would z-fight and hide parts of the line).
const CENTRE_LINE_LIFT_PACKAGE_M: f32 = 0.06;

/// Dashed centre line: 3 m dash, 6 m gap, `lift` above the road.
fn center_line_mesh(view: &RideView, lift: f32) -> Mesh {
    let route = &view.route;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut d = 0.0;
    while d + 3.0 < route.length() {
        if route.surface_at(d) != ocw_world::Surface::Asphalt {
            d += 9.0; // no markings on gravel
            continue;
        }
        let base = positions.len() as u32;
        for (dd, side) in [(0.0, -0.08), (0.0, 0.08), (3.0, -0.08), (3.0, 0.08)] {
            let mut v = to_vec3(route.sample(d + dd).offset_position(side));
            v.y += lift;
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
/// Bevy batches them (instancing, spec §4.5). They are see-through for the
/// camera: they never move it and fade out where they block the view.
fn spawn_trees(
    commands: &mut Commands,
    trees: &[(Vec3, f32)],
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SeeThroughMaterial>,
) {
    let crown = meshes.add(Cone {
        radius: 2.2,
        height: 7.0,
    });
    let trunk = meshes.add(Cylinder::new(0.25, 2.0));
    let crown_mat = materials.add(see_through(StandardMaterial {
        base_color: Color::srgb(0.13, 0.33, 0.15),
        perceptual_roughness: 1.0,
        ..default()
    }));
    let trunk_mat = materials.add(see_through(StandardMaterial {
        base_color: Color::srgb(0.35, 0.24, 0.15),
        perceptual_roughness: 1.0,
        ..default()
    }));
    for &(pos, scale) in trees {
        commands
            .spawn((
                SceneEntity,
                Transform::from_translation(pos).with_scale(Vec3::splat(scale)),
                Visibility::default(),
                CameraInteraction::SeeThrough,
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
    }
}

/// How deep tree trunks are sunk into the ground (hides the cut on slopes).
const TREE_SINK_M: f64 = 0.3;

/// Deterministic roadside tree positions (base of the trunk) and scales.
/// Trees stand on the terrain and never inside any road corridor.
pub fn tree_positions(route: &Route, terrain: &Terrain) -> Vec<(Vec3, f32)> {
    let mut rng = Lcg(42);
    let mut trees = Vec::new();
    let mut d = 0.0;
    while d < route.length() {
        let p = route.sample(d);
        let side = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
        let dist = route.road_width_m() / 2.0 + 6.0 + f64::from(rng.next_f32()) * 40.0;
        let scale = 0.7 + rng.next_f32() * 0.8;
        d += 6.0 + f64::from(rng.next_f32()) * 14.0;
        let [x, _, z] = p.offset_position(side * dist);
        let clear = terrain
            .road_proximity(x, z)
            .is_none_or(|r| r.distance_m > terrain.corridor_half_width_m() + 1.0);
        if clear {
            let y = terrain.height(x, z) - TREE_SINK_M;
            trees.push((Vec3::new(x as f32, y as f32, z as f32), scale));
        }
    }
    trees
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
            SceneEntity,
            Mesh3d(post.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(pos),
        ));
    }
    commands.spawn((
        SceneEntity,
        Mesh3d(meshes.add(Cuboid::new(half as f32 * 2.0 + 0.4, 0.8, 0.4))),
        MeshMaterial3d(mat),
        arch_bar_transform(&p),
    ));
}

/// Transform of the start arch's top bar: 5 m above the road, its long
/// (local X) axis across the road from post to post.
pub fn arch_bar_transform(p: &ocw_world::RoutePoint) -> Transform {
    let mut top = to_vec3(p.position);
    top.y += 5.0;
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    // looking_to points local -Z along the road, so local X is the road's right.
    Transform::from_translation(top).looking_to(forward, Vec3::Y)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_arch_bar_spans_the_road_between_the_posts() {
        for route in [Route::test_loop(), Route::test_hills()] {
            let p = route.sample(0.0);
            let t = arch_bar_transform(&p);
            let along = t.rotation * Vec3::X;
            let right = p.right();
            let right = Vec3::new(right[0] as f32, 0.0, right[1] as f32);
            assert!(
                along.dot(right).abs() > 0.999,
                "bar along {along}, road right {right}"
            );
            // Both bar ends sit above the posts.
            let half = route.road_width_m() / 2.0 + 0.8;
            for side in [-half, half] {
                let post = to_vec3(p.offset_position(side));
                let end = t.translation + along * side as f32 * along.dot(right).signum();
                assert!(
                    (end.xz() - post.xz()).length() < 1e-3,
                    "end {end} post {post}"
                );
            }
        }
    }

    #[test]
    fn road_edges_reach_below_the_terrain() {
        // The edge faces must end below the terrain at the road edge, or a gap
        // would show between road and terrain.
        let route = Route::test_loop();
        let params = TerrainParams::default();
        let terrain = Terrain::from_route(&route, params);
        let depth = f64::from(ROAD_EDGE_DEPTH_M);
        assert!(depth > params.road_clearance_m + 0.2);
        let half = route.road_width_m() / 2.0;
        let mut d = 0.0;
        while d < route.length() {
            let p = route.sample(d);
            for side in [-1.0, 1.0] {
                let q = p.offset_position(side * half);
                assert!(q[1] - depth < terrain.height(q[0], q[2]) - 0.1);
            }
            d += 11.0;
        }
    }

    #[test]
    fn trees_stand_on_the_terrain_and_off_the_road() {
        let route = Route::test_loop();
        let terrain = Terrain::from_route(&route, TerrainParams::default());
        let trees = tree_positions(&route, &terrain);
        assert!(trees.len() > 200, "{} trees", trees.len());
        for (pos, _) in &trees {
            let (x, z) = (f64::from(pos.x), f64::from(pos.z));
            let ground = terrain.height(x, z);
            assert!((f64::from(pos.y) - (ground - TREE_SINK_M)).abs() < 1e-3);
            let road = terrain.road_proximity(x, z);
            assert!(road.is_none_or(|r| r.distance_m > terrain.corridor_half_width_m()));
        }
        // Deterministic.
        assert_eq!(trees, tree_positions(&route, &terrain));
    }
}
