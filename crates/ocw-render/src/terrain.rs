//! Terrain meshes from the road-conformed height field (`ocw_world::Terrain`).
//!
//! Three levels of detail, without overlap:
//! - near: 5 m grid around the route (road corridor and blend zone),
//! - middle: 25 m grid out to where the terrain reaches the mean level,
//! - far: a flat ring at the mean level out to the horizon.
//!
//! The near grid is aligned to the middle grid, so their borders share edges.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_world::Terrain;

pub const NEAR_SPACING_M: f64 = 5.0;
pub const MIDDLE_SPACING_M: f64 = 25.0;
/// Margin of the near grid around the route's bounding box.
const NEAR_MARGIN_M: f64 = 125.0;
/// Half-size of the far ring (fog hides the edge).
const FAR_HALF_SIZE_M: f64 = 6000.0;

/// sRGB colours; vertex colours are linear, see [`linear`].
const GRASS: [f32; 3] = [0.33, 0.52, 0.25];
const SHOULDER: [f32; 3] = [0.47, 0.55, 0.3];

/// Bevy interprets vertex colours as linear RGB.
fn linear(srgb: [f32; 3]) -> [f32; 3] {
    let c = Color::srgb(srgb[0], srgb[1], srgb[2]).to_linear();
    [c.red, c.green, c.blue]
}

/// Heights on a regular grid; triangles split along the (i+1, k)–(i, k+1) diagonal.
#[derive(Debug, Clone)]
pub struct HeightGrid {
    pub origin: [f64; 2],
    pub spacing: f64,
    pub nx: usize,
    pub nz: usize,
    pub heights: Vec<f64>,
    /// Vertex colours (linear blend grass ↔ shoulder).
    pub colors: Vec<[f32; 3]>,
}

impl HeightGrid {
    /// Samples `terrain` on a grid covering `[min, max]` (rounded outwards to the spacing).
    pub fn sample(terrain: &Terrain, min: [f64; 2], max: [f64; 2], spacing: f64) -> Self {
        let nx = ((max[0] - min[0]) / spacing).ceil() as usize + 1;
        let nz = ((max[1] - min[1]) / spacing).ceil() as usize + 1;
        let road_half = terrain.corridor_half_width_m() - terrain.params().shoulder_m;
        let mut heights = Vec::with_capacity(nx * nz);
        let mut colors = Vec::with_capacity(nx * nz);
        for k in 0..nz {
            for i in 0..nx {
                let (x, z) = (min[0] + i as f64 * spacing, min[1] + k as f64 * spacing);
                heights.push(terrain.height(x, z));
                let shoulder = terrain.road_proximity(x, z).map_or(0.0, |p| {
                    let t = ((p.distance_m - road_half) / 8.0).clamp(0.0, 1.0) as f32;
                    1.0 - t * t * (3.0 - 2.0 * t)
                });
                let (grass, verge) = (linear(GRASS), linear(SHOULDER));
                colors.push(std::array::from_fn(|c| {
                    grass[c] + (verge[c] - grass[c]) * shoulder
                }));
            }
        }
        Self {
            origin: min,
            spacing,
            nx,
            nz,
            heights,
            colors,
        }
    }

    pub fn max(&self) -> [f64; 2] {
        [
            self.origin[0] + self.spacing * (self.nx - 1) as f64,
            self.origin[1] + self.spacing * (self.nz - 1) as f64,
        ]
    }

    fn h(&self, i: usize, k: usize) -> f64 {
        self.heights[k * self.nx + i]
    }

    /// Height of the rendered (triangulated) surface at `(x, z)`, if inside.
    pub fn surface_height(&self, x: f64, z: f64) -> Option<f64> {
        let fx = (x - self.origin[0]) / self.spacing;
        let fz = (z - self.origin[1]) / self.spacing;
        if fx < 0.0 || fz < 0.0 || fx > (self.nx - 1) as f64 || fz > (self.nz - 1) as f64 {
            return None;
        }
        let i = (fx.floor() as usize).min(self.nx - 2);
        let k = (fz.floor() as usize).min(self.nz - 2);
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let (h00, h10, h01, h11) = (
            self.h(i, k),
            self.h(i + 1, k),
            self.h(i, k + 1),
            self.h(i + 1, k + 1),
        );
        Some(if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        })
    }

    /// Mesh of all cells except those inside `hole` (`[min, max]`).
    pub fn mesh(&self, hole: Option<([f64; 2], [f64; 2])>) -> Mesh {
        let mut positions = Vec::with_capacity(self.heights.len());
        let mut normals = Vec::with_capacity(self.heights.len());
        let mut colors = Vec::with_capacity(self.heights.len());
        for k in 0..self.nz {
            for i in 0..self.nx {
                let x = self.origin[0] + i as f64 * self.spacing;
                let z = self.origin[1] + k as f64 * self.spacing;
                positions.push([x as f32, self.h(i, k) as f32, z as f32]);
                // Central differences (one-sided at the border).
                let (il, ir) = (i.saturating_sub(1), (i + 1).min(self.nx - 1));
                let (ku, kd) = (k.saturating_sub(1), (k + 1).min(self.nz - 1));
                let dx = (self.h(ir, k) - self.h(il, k)) / ((ir - il) as f64 * self.spacing);
                let dz = (self.h(i, kd) - self.h(i, ku)) / ((kd - ku) as f64 * self.spacing);
                normals.push(
                    Vec3::new(-dx as f32, 1.0, -dz as f32)
                        .normalize()
                        .to_array(),
                );
                let c = self.colors[k * self.nx + i];
                colors.push([c[0], c[1], c[2], 1.0]);
            }
        }
        let eps = 1e-6;
        let mut indices = Vec::new();
        for k in 0..self.nz - 1 {
            for i in 0..self.nx - 1 {
                if let Some((lo, hi)) = hole {
                    let (x0, z0) = (
                        self.origin[0] + i as f64 * self.spacing,
                        self.origin[1] + k as f64 * self.spacing,
                    );
                    let (x1, z1) = (x0 + self.spacing, z0 + self.spacing);
                    if x0 >= lo[0] - eps
                        && x1 <= hi[0] + eps
                        && z0 >= lo[1] - eps
                        && z1 <= hi[1] + eps
                    {
                        continue;
                    }
                }
                let v = |i: usize, k: usize| (k * self.nx + i) as u32;
                let (v00, v10, v01, v11) = (v(i, k), v(i + 1, k), v(i, k + 1), v(i + 1, k + 1));
                // Counter-clockwise seen from above (x east, z south).
                indices.extend([v00, v01, v10, v10, v01, v11]);
            }
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_indices(Indices::U32(indices));
        mesh
    }
}

/// Near and middle grids for a terrain whose route has the given bounding box.
pub fn terrain_grids(
    terrain: &Terrain,
    route_min: [f64; 2],
    route_max: [f64; 2],
) -> (HeightGrid, HeightGrid) {
    let (mid_min, mid_max) = terrain.bounds();
    let middle = HeightGrid::sample(terrain, mid_min, mid_max, MIDDLE_SPACING_M);
    // Align the near grid to middle-grid lines, so the hole matches exactly.
    let snap_down = |v: f64, o: f64| o + ((v - o) / MIDDLE_SPACING_M).floor() * MIDDLE_SPACING_M;
    let snap_up = |v: f64, o: f64| o + ((v - o) / MIDDLE_SPACING_M).ceil() * MIDDLE_SPACING_M;
    let near_min = [
        snap_down(route_min[0] - NEAR_MARGIN_M, mid_min[0]),
        snap_down(route_min[1] - NEAR_MARGIN_M, mid_min[1]),
    ];
    let near_max = [
        snap_up(route_max[0] + NEAR_MARGIN_M, mid_min[0]),
        snap_up(route_max[1] + NEAR_MARGIN_M, mid_min[1]),
    ];
    let near = HeightGrid::sample(terrain, near_min, near_max, NEAR_SPACING_M);
    (near, middle)
}

/// Flat ring at `height` from the `inner` rectangle out to the horizon.
pub fn far_ring_mesh(inner_min: [f64; 2], inner_max: [f64; 2], height: f64) -> Mesh {
    let (h, f) = (height as f32, FAR_HALF_SIZE_M as f32);
    let (cx, cz) = (
        ((inner_min[0] + inner_max[0]) / 2.0) as f32,
        ((inner_min[1] + inner_max[1]) / 2.0) as f32,
    );
    let (a0, a1) = (inner_min[0] as f32, inner_max[0] as f32);
    let (b0, b1) = (inner_min[1] as f32, inner_max[1] as f32);
    let (o0, o1, p0, p1) = (cx - f, cx + f, cz - f, cz + f);
    // Four quads: north, south, west, east of the inner rectangle.
    let quads = [
        ([o0, p0], [o1, b0]),
        ([o0, b1], [o1, p1]),
        ([o0, b0], [a0, b1]),
        ([a1, b0], [o1, b1]),
    ];
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for ([x0, z0], [x1, z1]) in quads {
        let base = positions.len() as u32;
        positions.extend([[x0, h, z0], [x1, h, z0], [x0, h, z1], [x1, h, z1]]);
        indices.extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }
    let n = positions.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, {
        let g = linear(GRASS);
        vec![[g[0], g[1], g[2], 1.0]; n]
    });
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use ocw_world::{Route, TerrainParams};

    use super::*;

    fn setup() -> (Route, Terrain, HeightGrid, HeightGrid) {
        let route = Route::test_loop();
        let terrain = Terrain::from_route(&route, TerrainParams::default());
        let (min, max) = crate::route_bounds(&route);
        let (near, middle) = terrain_grids(&terrain, min, max);
        (route, terrain, near, middle)
    }

    /// The rendered terrain never covers the road surface (spec §4.3: roads
    /// never float or sink).
    #[test]
    fn rendered_terrain_stays_below_the_road() {
        let (route, _, near, _) = setup();
        let half = route.road_width_m() / 2.0;
        let mut worst = f64::INFINITY;
        let mut d = 0.0;
        while d < route.length() {
            let p = route.sample(d);
            for k in -10..=10 {
                let q = p.offset_position(half * f64::from(k) / 10.0);
                let below = q[1] - near.surface_height(q[0], q[2]).expect("inside near grid");
                worst = worst.min(below);
            }
            d += 0.7;
        }
        assert!(
            worst > 0.02,
            "terrain reaches {:.3} m below the road",
            worst
        );
        assert!(worst < 0.2, "road floats {:.3} m above the terrain", worst);
    }

    #[test]
    fn grids_are_aligned_and_cover_the_route() {
        let (route, terrain, near, middle) = setup();
        let ratio = MIDDLE_SPACING_M / NEAR_SPACING_M;
        assert_eq!(ratio.fract(), 0.0);
        for (a, b) in [
            (near.origin[0], middle.origin[0]),
            (near.origin[1], middle.origin[1]),
        ] {
            assert!(((a - b) / MIDDLE_SPACING_M).fract().abs() < 1e-9);
        }
        let (rmin, rmax) = crate::route_bounds(&route);
        assert!(near.origin[0] < rmin[0] && near.origin[1] < rmin[1]);
        assert!(near.max()[0] > rmax[0] && near.max()[1] > rmax[1]);
        // On shared border vertices both grids have the same height.
        let x = near.origin[0];
        let mut z = near.origin[1];
        while z <= near.max()[1] {
            let (a, b) = (
                near.surface_height(x, z).unwrap(),
                middle.surface_height(x, z).unwrap(),
            );
            if ((z - middle.origin[1]) / MIDDLE_SPACING_M).fract().abs() < 1e-9 {
                assert!((a - b).abs() < 1e-9);
            }
            // Between shared vertices the T-junction gap stays tiny.
            assert!((a - b).abs() < 0.05, "gap {:.3} m at z {z}", (a - b).abs());
            z += NEAR_SPACING_M;
        }
        // The middle grid's border is at the far ring's height.
        let m = middle.max();
        for (x, z) in [(middle.origin[0], middle.origin[1]), (m[0], m[1])] {
            assert!((middle.surface_height(x, z).unwrap() - terrain.mean_height()).abs() < 1e-6);
        }
    }

    #[test]
    fn hole_removes_exactly_the_near_area() {
        let (_, _, near, middle) = setup();
        let full = middle.mesh(None).indices().unwrap().len();
        let holed = middle
            .mesh(Some((near.origin, near.max())))
            .indices()
            .unwrap()
            .len();
        let cells = |g: &HeightGrid| (g.nx - 1) * (g.nz - 1);
        let near_cells_in_middle =
            cells(&near) as f64 / (MIDDLE_SPACING_M / NEAR_SPACING_M).powi(2);
        assert_eq!((full - holed) / 6, near_cells_in_middle as usize);
    }

    #[test]
    fn mesh_faces_up() {
        let (_, _, near, _) = setup();
        let mesh = near.mesh(None);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        let idx: Vec<u32> = mesh.indices().unwrap().iter().map(|i| i as u32).collect();
        for tri in idx.chunks(3).step_by(997) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(pos[tri[j] as usize]));
            assert!((b - a).cross(c - a).y > 0.0);
        }
    }
}
