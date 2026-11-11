//! Deterministic generated world for the spike: noise terrain, a ~25 km
//! road loop, road-conformed terrain and tile contents. No Bevy types here,
//! so it is unit-testable and can run on worker threads.
//!
//! Stand-ins for Phase 3 data: the noise replaces the DEM, the loop replaces
//! the OSM road graph. Tile sizes, LOD and density follow spec §3.1.1 / §4.5.

use std::sync::Arc;

use ocw_world::{Route, Terrain, TerrainParams};

/// World extent (square), in metres.
pub const WORLD_SIZE_M: f64 = 10_000.0;
/// Tile edge length (spec §4.5).
pub const TILE_SIZE_M: f64 = 1_000.0;
pub const TILES_PER_SIDE: i32 = (WORLD_SIZE_M / TILE_SIZE_M) as i32;

/// Integer lattice hash → [0, 1).
fn hash(x: i64, z: i64, seed: u64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in [0, 1) with the given wavelength.
fn value_noise(x: f64, z: f64, wavelength: f64, seed: u64) -> f64 {
    let (fx, fz) = (x / wavelength, z / wavelength);
    let (ix, iz) = (fx.floor() as i64, fz.floor() as i64);
    let (tx, tz) = (fx - ix as f64, fz - iz as f64);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (sx, sz) = (s(tx), s(tz));
    let a = hash(ix, iz, seed) + (hash(ix + 1, iz, seed) - hash(ix, iz, seed)) * sx;
    let b = hash(ix, iz + 1, seed) + (hash(ix + 1, iz + 1, seed) - hash(ix, iz + 1, seed)) * sx;
    a + (b - a) * sz
}

/// Rolling hills, roughly 0–150 m (stand-in for the DEM).
pub fn base_height(x: f64, z: f64) -> f64 {
    const OCTAVES: [(f64, f64); 4] = [(2500.0, 90.0), (900.0, 35.0), (320.0, 10.0), (110.0, 3.0)];
    OCTAVES
        .iter()
        .enumerate()
        .map(|(i, (wl, amp))| value_noise(x, z, *wl, i as u64 + 1) * amp)
        .sum()
}

/// 0–1 forest density (patches of forest and open fields).
pub fn forest_density(x: f64, z: f64) -> f64 {
    let n = value_noise(x, z, 700.0, 17) * 0.7 + value_noise(x, z, 180.0, 18) * 0.3;
    ((n - 0.35) / 0.3).clamp(0.0, 1.0)
}

/// The road loop: ~25 km around the centre, elevation following the
/// smoothed terrain with a grade limit (like the DEM smoothing of spec §4.3).
pub fn main_route() -> Route {
    let n = 5000;
    let c = WORLD_SIZE_M / 2.0;
    let mut pts: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            let r = 3600.0 + 500.0 * (3.0 * t).sin() + 250.0 * (7.0 * t + 1.0).sin();
            let (x, z) = (c + r * t.cos(), c + r * t.sin());
            [x, base_height(x, z), z]
        })
        .collect();
    // Smooth the elevation along the road (±40 samples ≈ ±200 m), twice.
    for _ in 0..2 {
        let ys: Vec<f64> = pts.iter().map(|p| p[1]).collect();
        for (i, p) in pts.iter_mut().enumerate() {
            let sum: f64 = (-40..=40)
                .map(|k| ys[(i as i64 + k).rem_euclid(n as i64) as usize])
                .sum();
            p[1] = sum / 81.0;
        }
    }
    Route::from_points("spike-loop", "Spike Loop", pts, true, 7.0).expect("valid loop")
}

/// The whole generated world, shared by the tile builder threads.
pub struct World {
    pub route: Route,
    pub terrain: Terrain,
}

impl World {
    pub fn generate() -> Arc<Self> {
        let route = main_route();
        let params = TerrainParams {
            // Coarse tile grids (10–50 m) interpolate across the road; keep
            // more clearance and let the road edge faces cover the step.
            road_clearance_m: 0.25,
            ..TerrainParams::default()
        };
        let terrain = Terrain::with_base(
            &route,
            params,
            [0.0, 0.0],
            [WORLD_SIZE_M, WORLD_SIZE_M],
            base_height,
        );
        Arc::new(Self { route, terrain })
    }
}

/// Tile coordinates (0..TILES_PER_SIDE on both axes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub x: i32,
    pub z: i32,
}

impl TileId {
    pub fn containing(x: f64, z: f64) -> Self {
        Self {
            x: ((x / TILE_SIZE_M).floor() as i32).clamp(0, TILES_PER_SIDE - 1),
            z: ((z / TILE_SIZE_M).floor() as i32).clamp(0, TILES_PER_SIDE - 1),
        }
    }

    pub fn origin(&self) -> [f64; 2] {
        [
            f64::from(self.x) * TILE_SIZE_M,
            f64::from(self.z) * TILE_SIZE_M,
        ]
    }

    /// Chebyshev distance in tiles.
    pub fn ring(&self, other: TileId) -> i32 {
        (self.x - other.x).abs().max((self.z - other.z).abs())
    }

    /// Tiles within `radius` rings of `self`, inside the world.
    pub fn neighbourhood(&self, radius: i32) -> Vec<TileId> {
        let mut out = Vec::new();
        for z in self.z - radius..=self.z + radius {
            for x in self.x - radius..=self.x + radius {
                if (0..TILES_PER_SIDE).contains(&x) && (0..TILES_PER_SIDE).contains(&z) {
                    out.push(TileId { x, z });
                }
            }
        }
        out
    }
}

/// Terrain grid spacing for a tile `ring` tiles away from the viewer.
pub fn lod_spacing(ring: i32) -> f64 {
    match ring {
        0 | 1 => 10.0,
        2 => 25.0,
        _ => 50.0,
    }
}

/// Mesh arrays in tile-local coordinates (origin at the tile corner).
#[derive(Debug, Clone, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// One tree: tile-local trunk base, uniform scale, variant (0 = conifer, 1 = bush).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeInstance {
    pub position: [f32; 3],
    pub scale: f32,
    pub kind: u8,
}

/// Everything needed to show one tile.
#[derive(Debug, Clone)]
pub struct TileData {
    pub id: TileId,
    pub spacing: f64,
    pub terrain: MeshData,
    pub road: MeshData,
    pub trees: Vec<TreeInstance>,
}

/// Linear-RGB colours for the vertex-coloured terrain (sRGB values converted).
fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn terrain_color(slope: f64, forest: f64, verge: f64) -> [f32; 4] {
    let grass = [0.33, 0.52, 0.25];
    let forest_floor = [0.24, 0.36, 0.18];
    let rock = [0.48, 0.46, 0.42];
    let verge_c = [0.47, 0.55, 0.3];
    let f = forest as f32;
    let r = ((slope - 0.35) / 0.25).clamp(0.0, 1.0) as f32;
    let v = verge as f32;
    let mut c = [0.0f32; 4];
    for k in 0..3 {
        let base = grass[k] + (forest_floor[k] - grass[k]) * f;
        let base = base + (rock[k] - base) * r;
        c[k] = lin(base + (verge_c[k] - base) * v);
    }
    c[3] = 1.0;
    c
}

/// Builds the terrain grid, road ribbon and trees of one tile.
pub fn build_tile(world: &World, id: TileId, spacing: f64, trees_per_km2: u32) -> TileData {
    let [ox, oz] = id.origin();
    let n = (TILE_SIZE_M / spacing).round() as usize + 1;
    let t = &world.terrain;
    let road_half = world.route.road_width_m() / 2.0;
    let h = |x: f64, z: f64| t.height(x, z);

    let mut terrain = MeshData::default();
    for k in 0..n {
        for i in 0..n {
            let (x, z) = (ox + i as f64 * spacing, oz + k as f64 * spacing);
            let y = h(x, z);
            let dx = (h(x + spacing, z) - h(x - spacing, z)) / (2.0 * spacing);
            let dz = (h(x, z + spacing) - h(x, z - spacing)) / (2.0 * spacing);
            let len = (dx * dx + dz * dz + 1.0).sqrt();
            terrain
                .positions
                .push([(x - ox) as f32, y as f32, (z - oz) as f32]);
            terrain
                .normals
                .push([(-dx / len) as f32, (1.0 / len) as f32, (-dz / len) as f32]);
            let verge = t.road_proximity(x, z).map_or(0.0, |p| {
                1.0 - ((p.distance_m - road_half) / 8.0).clamp(0.0, 1.0)
            });
            terrain.colors.push(terrain_color(
                (dx * dx + dz * dz).sqrt(),
                forest_density(x, z),
                verge,
            ));
        }
    }
    for k in 0..n - 1 {
        for i in 0..n - 1 {
            let v = |i: usize, k: usize| (k * n + i) as u32;
            let (a, b, c, d) = (v(i, k), v(i + 1, k), v(i, k + 1), v(i + 1, k + 1));
            terrain.indices.extend([a, c, b, b, c, d]);
        }
    }

    TileData {
        id,
        spacing,
        terrain,
        road: road_mesh(world, id),
        trees: trees(world, id, trees_per_km2),
    }
}

/// Road surface (and edge faces) for the route parts inside the tile.
fn road_mesh(world: &World, id: TileId) -> MeshData {
    let [ox, oz] = id.origin();
    let route = &world.route;
    let half = route.road_width_m() / 2.0;
    let step = 4.0;
    let mut m = MeshData::default();
    let asphalt = [lin(0.22), lin(0.22), lin(0.24), 1.0];
    let edge = [lin(0.45), lin(0.43), lin(0.38), 1.0];
    let inside = |d: f64| {
        let p = route.sample(d).position;
        p[0] >= ox && p[0] < ox + TILE_SIZE_M && p[2] >= oz && p[2] < oz + TILE_SIZE_M
    };
    let mut d = 0.0;
    while d < route.length() {
        // A segment belongs to the tile containing its start; it may reach
        // slightly into the next tile, which hides seams.
        if inside(d) {
            let (a, b) = (route.sample(d), route.sample(d + step));
            let local = |p: [f64; 3], drop: f64| {
                [(p[0] - ox) as f32, (p[1] - drop) as f32, (p[2] - oz) as f32]
            };
            let quads = [
                // surface
                (
                    a.offset_position(-half),
                    a.offset_position(half),
                    b.offset_position(-half),
                    b.offset_position(half),
                    0.0,
                    asphalt,
                    [0.0, 1.0, 0.0],
                ),
            ];
            for (l0, r0, l1, r1, _, color, normal) in quads {
                let base = m.positions.len() as u32;
                for p in [l0, r0, l1, r1] {
                    m.positions.push(local(p, 0.0));
                    m.normals.push(normal);
                    m.colors.push(color);
                }
                m.indices
                    .extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
            }
            // Edge faces down into the terrain on both sides.
            for side in [-1.0, 1.0] {
                let (ta, tb) = (
                    a.offset_position(side * half),
                    b.offset_position(side * half),
                );
                let base = m.positions.len() as u32;
                for (p, drop) in [(ta, 0.0), (ta, 1.5), (tb, 0.0), (tb, 1.5)] {
                    m.positions.push(local(p, drop));
                    let r = a.right();
                    m.normals
                        .push([(side * r[0]) as f32, 0.0, (side * r[1]) as f32]);
                    m.colors.push(edge);
                }
                m.indices
                    .extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
                m.indices
                    .extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
            }
        }
        d += step;
    }
    m
}

/// Deterministic trees on a jittered grid, denser in forests, never on the
/// road corridor.
fn trees(world: &World, id: TileId, per_km2: u32) -> Vec<TreeInstance> {
    let [ox, oz] = id.origin();
    let t = &world.terrain;
    let cell = TILE_SIZE_M / f64::from(per_km2).sqrt().max(1.0);
    let n = (TILE_SIZE_M / cell).floor() as i64;
    let seed = (id.x as u64) << 32 | id.z as u64;
    let mut out = Vec::new();
    for k in 0..n {
        for i in 0..n {
            let r = |s: u64| hash(i, k, seed.wrapping_add(s));
            let (x, z) = (ox + (i as f64 + r(1)) * cell, oz + (k as f64 + r(2)) * cell);
            // Fields get a few scattered trees, forests are full.
            let keep = 0.08 + 0.92 * forest_density(x, z);
            if r(3) > keep {
                continue;
            }
            if t.road_proximity(x, z)
                .is_some_and(|p| p.distance_m < t.corridor_half_width_m() + 2.0)
            {
                continue;
            }
            out.push(TreeInstance {
                position: [
                    (x - ox) as f32,
                    (t.height(x, z) - 0.3) as f32,
                    (z - oz) as f32,
                ],
                scale: 0.7 + 0.8 * r(4) as f32,
                kind: u8::from(r(5) < 0.25),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for (x, z) in [(0.0, 0.0), (1234.5, 987.6), (9999.0, 5.0)] {
            let h = base_height(x, z);
            assert_eq!(h, base_height(x, z));
            assert!((0.0..138.0).contains(&h), "{h}");
            assert!((0.0..=1.0).contains(&forest_density(x, z)));
        }
        // Continuous: no jumps between neighbouring metres.
        let mut max_step: f64 = 0.0;
        for i in 0..2000 {
            let x = f64::from(i) * 3.7;
            max_step = max_step.max((base_height(x + 1.0, 500.0) - base_height(x, 500.0)).abs());
        }
        assert!(max_step < 1.0, "{max_step}");
    }

    #[test]
    fn route_is_a_rideable_loop_inside_the_world() {
        let r = main_route();
        assert!(r.is_loop());
        assert!((20_000.0..32_000.0).contains(&r.length()), "{}", r.length());
        let mut max_grade: f64 = 0.0;
        let mut d = 0.0;
        while d < r.length() {
            let p = r.sample(d);
            assert!(p.position[0] > 500.0 && p.position[0] < WORLD_SIZE_M - 500.0);
            assert!(p.position[2] > 500.0 && p.position[2] < WORLD_SIZE_M - 500.0);
            max_grade = max_grade.max(p.grade.abs());
            d += 10.0;
        }
        assert!(max_grade < 0.12, "max grade {max_grade}");
    }

    #[test]
    fn tiles_and_rings() {
        let t = TileId::containing(5500.0, 1200.0);
        assert_eq!(t, TileId { x: 5, z: 1 });
        assert_eq!(t.origin(), [5000.0, 1000.0]);
        assert_eq!(TileId::containing(-5.0, 1e9), TileId { x: 0, z: 9 });
        assert_eq!(t.ring(TileId { x: 7, z: 0 }), 2);
        assert_eq!(t.neighbourhood(2).len(), 20); // clipped at z = -1
        assert_eq!(TileId { x: 5, z: 5 }.neighbourhood(2).len(), 25);
        assert_eq!(lod_spacing(0), 10.0);
        assert_eq!(lod_spacing(2), 25.0);
        assert_eq!(lod_spacing(5), 50.0);
    }

    #[test]
    fn tile_contents() {
        let world = World::generate();
        // A tile the loop passes through.
        let p = world.route.sample(1000.0).position;
        let id = TileId::containing(p[0], p[2]);
        let tile = build_tile(&world, id, 25.0, 4000);
        assert_eq!(tile.terrain.positions.len(), 41 * 41);
        assert_eq!(tile.terrain.indices.len(), 40 * 40 * 6);
        assert!(!tile.road.positions.is_empty());
        assert!(tile.trees.len() > 300, "{} trees", tile.trees.len());
        for tree in &tile.trees {
            assert!((0.0..1000.0).contains(&tree.position[0]));
            assert!((0.0..1000.0).contains(&tree.position[2]));
        }
        // Deterministic.
        assert_eq!(tile.trees, build_tile(&world, id, 25.0, 4000).trees);
        // Neighbouring tiles share the border heights (no cracks at equal LOD).
        let right = build_tile(
            &world,
            TileId {
                x: id.x + 1,
                z: id.z,
            },
            25.0,
            0,
        );
        for k in 0..41 {
            let a = tile.terrain.positions[k * 41 + 40][1];
            let b = right.terrain.positions[k * 41][1];
            assert!((a - b).abs() < 1e-3);
        }
    }
}
