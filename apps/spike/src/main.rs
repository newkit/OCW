//! Graphics engine spike (spec §3.1.1): a generated 10 × 10 km world with
//! streamed 1 km tiles, road-conformed terrain, roads, a floating origin,
//! 100,000+ trees with LOD and 100 animated riders. Rides a fixed distance
//! along the road loop and writes a JSON report (frame times, hitches, tile
//! build/upload times, RAM, VRAM).
//!
//! ```text
//! ocw-spike --km 10 --speed-kmh 40 --report spike.json
//! ocw-spike --km 10 --speed-kmh 150 --tree-mode merged   # streaming stress
//! ```

mod generate;
mod stats;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::{PresentMode, WindowResolution};
use clap::{Parser, ValueEnum};

use generate::{
    MeshData, TILE_SIZE_M, TileData, TileId, TreeInstance, World, build_tile, lod_spacing,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TreeMode {
    /// One entity per tree and LOD level (Bevy batches/instances them),
    /// with `VisibilityRange` LOD switching.
    Instanced,
    /// All trees of a tile merged into one mesh per tile; LOD per tile.
    Merged,
}

#[derive(Parser, Debug, Clone, Resource)]
#[command(about = "OCW graphics engine spike (spec §3.1.1)")]
struct Args {
    /// Distance to ride, in km.
    #[arg(long, default_value_t = 10.0)]
    km: f64,
    /// Riding speed of the camera rider.
    #[arg(long, default_value_t = 40.0)]
    speed_kmh: f64,
    /// Tiles streamed around the rider (radius in tiles; 2 = 5 × 5 tiles).
    #[arg(long, default_value_t = 2)]
    radius: i32,
    /// Tree density per km².
    #[arg(long, default_value_t = 4000)]
    trees: u32,
    #[arg(long, value_enum, default_value_t = TreeMode::Instanced)]
    tree_mode: TreeMode,
    /// Animated riders around the camera rider.
    #[arg(long, default_value_t = 100)]
    riders: usize,
    /// Tile uploads (mesh creation + spawn) per frame.
    #[arg(long, default_value_t = 1)]
    uploads_per_frame: usize,
    /// Window size as WIDTHxHEIGHT.
    #[arg(long, default_value = "1920x1080")]
    window: String,
    /// Enable vsync (default: uncapped, to measure headroom).
    #[arg(long)]
    vsync: bool,
    /// Where to write the JSON report.
    #[arg(long, default_value = "spike-report.json")]
    report: PathBuf,
    /// Save a screenshot at the end of the ride.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Starting point on the loop, in km.
    #[arg(long, default_value_t = 0.0)]
    start_km: f64,
}

fn main() -> AppExit {
    let args = Args::parse();
    let (w, h) = args
        .window
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .expect("--window WIDTHxHEIGHT");
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "OCW spike".into(),
            resolution: WindowResolution::new(w, h),
            present_mode: if args.vsync {
                PresentMode::AutoVsync
            } else {
                PresentMode::AutoNoVsync
            },
            ..default()
        }),
        ..default()
    }))
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(ClearColor(SKY))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        ..default()
    })
    .insert_resource(args)
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            advance,
            floating_origin,
            stream_tiles,
            upload_tiles,
            place_riders,
            follow_camera,
            record,
        )
            .chain(),
    );
    app.run()
}

const SKY: Color = Color::srgb(0.62, 0.78, 0.95);
/// Rebase the floating origin when the rider is this far from it.
const REBASE_DISTANCE_M: f64 = 1000.0;
/// Trees switch from the full mesh to the impostor at this distance.
const TREE_LOD_DISTANCE_M: f32 = 250.0;

#[derive(Resource)]
struct Shared {
    world: Arc<World>,
    tree_material: Handle<StandardMaterial>,
    terrain_material: Handle<StandardMaterial>,
    tree_lod0: [Handle<Mesh>; 2],
    tree_lod1: Handle<Mesh>,
}

/// World position (f64) that maps to render-space (0, 0, 0).
#[derive(Resource, Default)]
struct Origin {
    x: f64,
    z: f64,
    rebases: u32,
}

impl Origin {
    fn to_render(&self, p: [f64; 3]) -> Vec3 {
        Vec3::new((p[0] - self.x) as f32, p[1] as f32, (p[2] - self.z) as f32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the initial tiles.
    Loading,
    Riding,
    /// Screenshot requested; frames counted until exit.
    Finishing(u32),
}

#[derive(Resource)]
struct Bench {
    phase: Phase,
    started: Instant,
    initial_load_s: f64,
    distance_m: f64,
    ridden_m: f64,
    frame_ms: Vec<f64>,
    build_ms: Vec<f64>,
    upload_ms: Vec<f64>,
    rss_max: f64,
    vram_start: Option<f64>,
    vram_max: Option<f64>,
    last_probe: Instant,
    max_tree_entities: usize,
    max_tiles: usize,
}

#[derive(Component)]
struct Tile {
    id: TileId,
}

#[derive(Component)]
struct TreeEntity;

#[derive(Component)]
struct Rider {
    offset_m: f64,
    lane: f64,
    speed_factor: f64,
}

#[derive(Component)]
struct Leg {
    phase: f32,
}

#[derive(Component)]
struct FollowCamera;

struct Pending {
    task: Task<(TileData, f64)>,
}

#[derive(Resource, Default)]
struct Streamer {
    /// Loaded tile entity and its grid spacing.
    loaded: HashMap<TileId, (Entity, f64)>,
    in_flight: HashMap<TileId, (f64, Pending)>,
    ready: VecDeque<(TileData, f64)>,
}

fn mesh_from(data: &MeshData) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors.clone());
    m.insert_indices(Indices::U32(data.indices.clone()));
    m
}

fn lin(c: [f32; 3]) -> [f32; 4] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue, 1.0]
}

/// Low-poly tree (conifer or bush) as one vertex-coloured mesh.
fn tree_mesh_data(kind: u8) -> MeshData {
    let mut m = MeshData::default();
    let add_cone = |m: &mut MeshData, y0: f32, y1: f32, r: f32, seg: u32, color: [f32; 4]| {
        let base = m.positions.len() as u32;
        for i in 0..seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            m.positions.push([r * c, y0, r * s]);
            let n = Vec3::new(c, r / (y1 - y0), s).normalize();
            m.normals.push(n.to_array());
            m.colors.push(color);
        }
        m.positions.push([0.0, y1, 0.0]);
        m.normals.push([0.0, 1.0, 0.0]);
        m.colors.push(color);
        let apex = base + seg;
        for i in 0..seg {
            m.indices.extend([base + i, apex, base + (i + 1) % seg]);
        }
    };
    let trunk = lin([0.35, 0.24, 0.15]);
    if kind == 0 {
        add_cone(&mut m, 0.0, 2.2, 0.25, 6, trunk);
        add_cone(&mut m, 1.8, 9.0, 2.2, 8, lin([0.13, 0.33, 0.15]));
    } else {
        add_cone(&mut m, 0.0, 1.0, 0.15, 5, trunk);
        add_cone(&mut m, 0.6, 3.2, 1.6, 7, lin([0.2, 0.42, 0.18]));
    }
    m
}

/// Impostor stand-in: two crossed tree silhouettes (triangle crown, thin
/// trunk), both faces, lit like the full tree.
fn tree_impostor_data() -> MeshData {
    let mut m = MeshData::default();
    let green = lin([0.13, 0.33, 0.15]);
    let brown = lin([0.35, 0.24, 0.15]);
    for (dx, dz) in [(1.0f32, 0.0f32), (0.0, 1.0)] {
        // (x, y, colour) triangles: crown, trunk (two triangles).
        let tris: [[(f32, f32, [f32; 4]); 3]; 3] = [
            [(-2.2, 1.8, green), (2.2, 1.8, green), (0.0, 9.0, green)],
            [(-0.25, 0.0, brown), (0.25, 0.0, brown), (-0.25, 1.8, brown)],
            [(0.25, 0.0, brown), (0.25, 1.8, brown), (-0.25, 1.8, brown)],
        ];
        for tri in tris {
            let base = m.positions.len() as u32;
            for (x, y, c) in tri {
                m.positions.push([x * dx, y, x * dz]);
                // Up-tilted normal so the impostor shades like a cone.
                m.normals.push(Vec3::new(dz, 1.0, -dx).normalize().to_array());
                m.colors.push(c);
            }
            m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 1]);
        }
    }
    m
}

fn merged_trees(trees: &[TreeInstance], impostor: bool) -> MeshData {
    let parts = [tree_mesh_data(0), tree_mesh_data(1), tree_impostor_data()];
    let mut m = MeshData::default();
    for t in trees {
        let src = if impostor {
            &parts[2]
        } else {
            &parts[usize::from(t.kind)]
        };
        let base = m.positions.len() as u32;
        let p = Vec3::from(t.position);
        for (i, v) in src.positions.iter().enumerate() {
            m.positions.push((p + Vec3::from(*v) * t.scale).to_array());
            m.normals.push(src.normals[i]);
            m.colors.push(src.colors[i]);
        }
        m.indices.extend(src.indices.iter().map(|i| i + base));
    }
    m
}

fn setup(
    mut commands: Commands,
    args: Res<Args>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t0 = Instant::now();
    let world = World::generate();
    info!(
        route_km = world.route.length() / 1000.0,
        secs = t0.elapsed().as_secs_f64(),
        "world generated"
    );
    let vertex_colored = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        ..default()
    };
    let shared = Shared {
        tree_material: materials.add(vertex_colored.clone()),
        terrain_material: materials.add(vertex_colored),
        tree_lod0: [
            meshes.add(mesh_from(&tree_mesh_data(0))),
            meshes.add(mesh_from(&tree_mesh_data(1))),
        ],
        tree_lod1: meshes.add(mesh_from(&tree_impostor_data())),
        world: world.clone(),
    };
    let start = world.route.sample(args.start_km * 1000.0).position;
    let origin_tile = TileId::containing(start[0], start[2]).origin();
    commands.insert_resource(Origin {
        x: origin_tile[0],
        z: origin_tile[1],
        rebases: 0,
    });

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
    let view_m = (args.radius as f32 + 0.5) * TILE_SIZE_M as f32;
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        DistanceFog {
            color: SKY,
            falloff: FogFalloff::Linear {
                start: view_m * 0.35,
                end: view_m,
            },
            ..default()
        },
        FollowCamera,
    ));

    // Riders: a capsule body and two pedalling legs each.
    let body = meshes.add(Capsule3d::new(0.28, 0.7));
    let leg = meshes.add(Cuboid::new(0.12, 0.8, 0.12));
    let wheel = meshes.add(Torus::new(0.3, 0.34));
    let dark = materials.add(Color::srgb(0.1, 0.1, 0.12));
    for i in 0..args.riders {
        let hue = (i as f32 * 37.0) % 360.0;
        let jersey = materials.add(Color::hsl(hue, 0.7, 0.5));
        let rider = Rider {
            // Spread from 150 m behind to 1.5 km ahead of the camera rider.
            offset_m: if i == 0 {
                0.0
            } else {
                -150.0 + (i as f64 * 16.5) % 1650.0
            },
            lane: [0.0, -1.2, 1.2, -2.4, 2.4][i % 5],
            speed_factor: if i == 0 {
                1.0
            } else {
                0.97 + (i % 7) as f64 * 0.01
            },
        };
        commands
            .spawn((rider, Transform::default(), Visibility::default()))
            .with_children(|r| {
                r.spawn((
                    Mesh3d(body.clone()),
                    MeshMaterial3d(jersey),
                    Transform::from_xyz(0.0, 1.35, 0.15).with_rotation(Quat::from_rotation_x(-1.0)),
                ));
                for (side, phase) in [(-0.15f32, 0.0f32), (0.15, std::f32::consts::PI)] {
                    r.spawn((
                        Mesh3d(leg.clone()),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_xyz(side, 0.75, 0.0),
                        Leg { phase },
                    ));
                }
                for z in [-0.55f32, 0.55] {
                    r.spawn((
                        Mesh3d(wheel.clone()),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_xyz(0.0, 0.34, z)
                            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                    ));
                }
            });
    }

    commands.insert_resource(shared);
    commands.insert_resource(Streamer::default());
    commands.insert_resource(Bench {
        phase: Phase::Loading,
        started: Instant::now(),
        initial_load_s: 0.0,
        distance_m: args.start_km * 1000.0,
        ridden_m: 0.0,
        frame_ms: Vec::new(),
        build_ms: Vec::new(),
        upload_ms: Vec::new(),
        rss_max: 0.0,
        vram_start: stats::vram_used_mib(),
        vram_max: None,
        last_probe: Instant::now(),
        max_tree_entities: 0,
        max_tiles: 0,
    });
}

/// Moves the camera rider; switches phases.
#[allow(clippy::too_many_arguments)]
fn advance(
    mut commands: Commands,
    args: Res<Args>,
    time: Res<Time<Real>>,
    shared: Res<Shared>,
    streamer: Res<Streamer>,
    mut bench: ResMut<Bench>,
    mut exit: MessageWriter<AppExit>,
    adapter: Option<Res<RenderAdapterInfo>>,
    trees: Query<(), With<TreeEntity>>,
) {
    match bench.phase {
        Phase::Loading => {
            let p = rider_world(&bench, &shared.world.route);
            let wanted = TileId::containing(p[0], p[2])
                .neighbourhood(args.radius)
                .len();
            if streamer.loaded.len() >= wanted && streamer.in_flight.is_empty() {
                bench.initial_load_s = bench.started.elapsed().as_secs_f64();
                info!(
                    secs = bench.initial_load_s,
                    tiles = wanted,
                    "initial tiles loaded, riding"
                );
                bench.phase = Phase::Riding;
            }
        }
        Phase::Riding => {
            let step = args.speed_kmh / 3.6 * time.delta_secs_f64();
            bench.distance_m += step;
            bench.ridden_m += step;
            if bench.ridden_m >= args.km * 1000.0 {
                write_report(&args, &bench, adapter.as_deref(), trees.iter().count());
                if let Some(path) = &args.screenshot {
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()));
                }
                bench.phase = Phase::Finishing(0);
            }
        }
        Phase::Finishing(n) => {
            bench.phase = Phase::Finishing(n + 1);
            if n > 10 {
                exit.write(AppExit::Success);
            }
        }
    }
}

fn rider_world(bench: &Bench, route: &ocw_world::Route) -> [f64; 3] {
    route.sample(bench.distance_m).position
}

/// Keeps render coordinates small: shifts the origin to the rider's tile.
fn floating_origin(
    bench: Res<Bench>,
    shared: Res<Shared>,
    mut origin: ResMut<Origin>,
    mut tiles: Query<(&Tile, &mut Transform)>,
) {
    let p = rider_world(&bench, &shared.world.route);
    if (p[0] - origin.x).abs() < REBASE_DISTANCE_M && (p[2] - origin.z).abs() < REBASE_DISTANCE_M {
        return;
    }
    let o = TileId::containing(p[0], p[2]).origin();
    origin.x = o[0];
    origin.z = o[1];
    origin.rebases += 1;
    for (tile, mut t) in &mut tiles {
        let to = tile.id.origin();
        t.translation = Vec3::new((to[0] - origin.x) as f32, 0.0, (to[1] - origin.z) as f32);
    }
}

/// Starts background builds for missing tiles and tiles whose LOD changed;
/// collects finished builds; despawns tiles that left the area.
fn stream_tiles(
    mut commands: Commands,
    args: Res<Args>,
    bench: Res<Bench>,
    shared: Res<Shared>,
    mut streamer: ResMut<Streamer>,
) {
    let p = rider_world(&bench, &shared.world.route);
    let center = TileId::containing(p[0], p[2]);
    let pool = AsyncComputeTaskPool::get();
    let mut wanted: Vec<TileId> = center.neighbourhood(args.radius);
    // Nearest first.
    wanted.sort_by_key(|t| t.ring(center));
    for id in wanted {
        let spacing = lod_spacing(id.ring(center));
        let have = streamer.loaded.get(&id).map(|(_, s)| *s);
        let building = streamer.in_flight.get(&id).map(|(s, _)| *s);
        if have == Some(spacing) || building == Some(spacing) {
            continue;
        }
        let world = shared.world.clone();
        let trees = args.trees;
        let task = pool.spawn(async move {
            let t0 = Instant::now();
            let data = build_tile(&world, id, spacing, trees);
            (data, t0.elapsed().as_secs_f64() * 1000.0)
        });
        streamer.in_flight.insert(id, (spacing, Pending { task }));
    }
    // Finished builds.
    let ids: Vec<TileId> = streamer.in_flight.keys().copied().collect();
    for id in ids {
        let (_, pending) = streamer.in_flight.get_mut(&id).expect("key exists");
        if let Some(result) = check_ready(&mut pending.task) {
            streamer.in_flight.remove(&id);
            streamer.ready.push_back(result);
        }
    }
    // Unload far tiles (one ring of hysteresis).
    let far: Vec<TileId> = streamer
        .loaded
        .keys()
        .filter(|id| id.ring(center) > args.radius + 1)
        .copied()
        .collect();
    for id in far {
        if let Some((e, _)) = streamer.loaded.remove(&id) {
            commands.entity(e).despawn();
        }
    }
}

/// Turns finished builds into meshes and entities, a few per frame.
#[allow(clippy::too_many_arguments)]
fn upload_tiles(
    mut commands: Commands,
    args: Res<Args>,
    shared: Res<Shared>,
    origin: Res<Origin>,
    mut streamer: ResMut<Streamer>,
    mut bench: ResMut<Bench>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for _ in 0..args.uploads_per_frame {
        let Some((data, build_ms)) = streamer.ready.pop_front() else {
            break;
        };
        let p = rider_world(&bench, &shared.world.route);
        if data.id.ring(TileId::containing(p[0], p[2])) > args.radius + 1 {
            continue; // left the area while it was being built
        }
        let t0 = Instant::now();
        let to = data.id.origin();
        let tile = commands
            .spawn((
                Tile { id: data.id },
                Transform::from_xyz((to[0] - origin.x) as f32, 0.0, (to[1] - origin.z) as f32),
                Visibility::default(),
            ))
            .id();
        let terrain = meshes.add(mesh_from(&data.terrain));
        let road = meshes.add(mesh_from(&data.road));
        commands.entity(tile).with_children(|c| {
            c.spawn((
                Mesh3d(terrain),
                MeshMaterial3d(shared.terrain_material.clone()),
                bevy::light::NotShadowCaster,
            ));
            c.spawn((
                Mesh3d(road),
                MeshMaterial3d(shared.terrain_material.clone()),
            ));
            match args.tree_mode {
                TreeMode::Instanced => {
                    let far = (args.radius as f32 + 0.5) * TILE_SIZE_M as f32;
                    let near_range = VisibilityRange {
                        start_margin: 0.0..0.0,
                        end_margin: TREE_LOD_DISTANCE_M - 20.0..TREE_LOD_DISTANCE_M + 20.0,
                        use_aabb: false,
                    };
                    let far_range = VisibilityRange {
                        start_margin: TREE_LOD_DISTANCE_M - 20.0..TREE_LOD_DISTANCE_M + 20.0,
                        end_margin: far..far + 50.0,
                        use_aabb: false,
                    };
                    for t in &data.trees {
                        let tr = Transform::from_translation(Vec3::from(t.position))
                            .with_scale(Vec3::splat(t.scale));
                        c.spawn((
                            Mesh3d(shared.tree_lod0[usize::from(t.kind)].clone()),
                            MeshMaterial3d(shared.tree_material.clone()),
                            tr,
                            near_range.clone(),
                            TreeEntity,
                        ));
                        c.spawn((
                            Mesh3d(shared.tree_lod1.clone()),
                            MeshMaterial3d(shared.tree_material.clone()),
                            tr,
                            far_range.clone(),
                            bevy::light::NotShadowCaster,
                            TreeEntity,
                        ));
                    }
                }
                TreeMode::Merged => {
                    // Full trees on the near tiles, impostors further out.
                    let impostor = data.spacing > lod_spacing(1);
                    let mesh = meshes.add(mesh_from(&merged_trees(&data.trees, impostor)));
                    c.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(shared.tree_material.clone()),
                        TreeEntity,
                    ));
                }
            }
        });
        if let Some((old, _)) = streamer.loaded.insert(data.id, (tile, data.spacing)) {
            commands.entity(old).despawn();
        }
        bench.build_ms.push(build_ms);
        bench.upload_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
}

fn place_riders(
    time: Res<Time<Real>>,
    args: Res<Args>,
    bench: Res<Bench>,
    shared: Res<Shared>,
    origin: Res<Origin>,
    mut riders: Query<(&Rider, &mut Transform), Without<Leg>>,
    mut legs: Query<(&Leg, &mut Transform), Without<Rider>>,
) {
    let route = &shared.world.route;
    let t = time.elapsed_secs_f64();
    for (r, mut tr) in &mut riders {
        // Others drift relative to the camera rider by their speed factor.
        let rel = r.offset_m + (r.speed_factor - 1.0) * args.speed_kmh / 3.6 * t;
        let p = route.sample(bench.distance_m + rel);
        let pos = origin.to_render(p.offset_position(r.lane));
        let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
        *tr = Transform::from_translation(pos).looking_to(forward, Vec3::Y);
    }
    // 90 rpm pedalling.
    let crank = (t * 1.5 * std::f64::consts::TAU) as f32;
    for (leg, mut tr) in &mut legs {
        tr.rotation = Quat::from_rotation_x((crank + leg.phase).sin() * 0.6);
    }
}

fn follow_camera(
    bench: Res<Bench>,
    shared: Res<Shared>,
    origin: Res<Origin>,
    mut camera: Query<&mut Transform, With<FollowCamera>>,
) {
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let route = &shared.world.route;
    let p = route.sample(bench.distance_m);
    let pos = origin.to_render(p.position);
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let eye = pos - forward * 7.0 + Vec3::Y * 3.0;
    let target = origin.to_render(route.sample(bench.distance_m + 12.0).position) + Vec3::Y;
    *cam = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

fn record(
    time: Res<Time<Real>>,
    streamer: Res<Streamer>,
    mut bench: ResMut<Bench>,
    trees: Query<(), With<TreeEntity>>,
) {
    if bench.phase != Phase::Riding {
        return;
    }
    bench.frame_ms.push(time.delta_secs_f64() * 1000.0);
    bench.max_tiles = bench.max_tiles.max(streamer.loaded.len());
    if bench.last_probe.elapsed().as_secs_f64() >= 1.0 {
        bench.last_probe = Instant::now();
        if let Some(r) = stats::rss_mib() {
            bench.rss_max = bench.rss_max.max(r);
        }
        if let Some(v) = stats::vram_used_mib() {
            bench.vram_max = Some(bench.vram_max.map_or(v, |m| m.max(v)));
        }
        bench.max_tree_entities = bench.max_tree_entities.max(trees.iter().count());
    }
}

fn write_report(
    args: &Args,
    bench: &Bench,
    adapter: Option<&RenderAdapterInfo>,
    tree_entities: usize,
) {
    let ft = stats::summarize(&bench.frame_ms);
    let ride_s: f64 = bench.frame_ms.iter().sum::<f64>() / 1000.0;
    let (gpu, backend, driver) = adapter.map_or_else(
        || ("unknown".to_string(), "unknown".to_string(), String::new()),
        |a| {
            (
                a.name.clone(),
                format!("{:?}", a.backend),
                format!("{} {}", a.driver, a.driver_info),
            )
        },
    );
    let settings = stats::Json::default()
        .num("km", args.km)
        .num("speed_kmh", args.speed_kmh)
        .int("radius_tiles", args.radius as u64)
        .int("trees_per_km2", u64::from(args.trees))
        .str("tree_mode", &format!("{:?}", args.tree_mode))
        .int("riders", args.riders as u64)
        .int("uploads_per_frame", args.uploads_per_frame as u64)
        .str("window", &args.window)
        .bool("vsync", args.vsync);
    let json = stats::Json::default()
        .str("gpu", &gpu)
        .str("backend", &backend)
        .str("driver", &driver)
        .obj("settings", settings)
        .num("initial_load_s", bench.initial_load_s)
        .num("ride_s", ride_s)
        .num(
            "mean_fps",
            if ride_s > 0.0 {
                ft.count as f64 / ride_s
            } else {
                0.0
            },
        )
        .summary("frame_ms", ft)
        .int(
            "frames_over_16_7ms",
            stats::count_over(&bench.frame_ms, 16.7) as u64,
        )
        .int(
            "frames_over_25ms",
            stats::count_over(&bench.frame_ms, 25.0) as u64,
        )
        .int(
            "frames_over_33ms",
            stats::count_over(&bench.frame_ms, 33.3) as u64,
        )
        .int(
            "frames_over_50ms",
            stats::count_over(&bench.frame_ms, 50.0) as u64,
        )
        .int(
            "longest_run_over_16_7ms",
            stats::longest_run_over(&bench.frame_ms, 16.7) as u64,
        )
        .summary("tile_build_ms", stats::summarize(&bench.build_ms))
        .summary("tile_upload_ms", stats::summarize(&bench.upload_ms))
        .int("tiles_built", bench.build_ms.len() as u64)
        .int("max_tiles_loaded", bench.max_tiles as u64)
        .int(
            "max_tree_entities",
            bench.max_tree_entities.max(tree_entities) as u64,
        )
        .num("rss_max_mib", bench.rss_max)
        .num("vram_start_mib", bench.vram_start.unwrap_or(f64::NAN))
        .num("vram_max_mib", bench.vram_max.unwrap_or(f64::NAN))
        .finish();
    match std::fs::write(&args.report, &json) {
        Ok(()) => info!(path = %args.report.display(), "report written"),
        Err(e) => error!("cannot write report: {e}"),
    }
    println!("{json}");
}
