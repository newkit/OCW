//! The road damage where the wheels roll (spec Phase 8 Q12 (c), v5.33):
//! which damaged strip lies under a point of the world, its wear grade and
//! surface, and the potholes, edge breaks, cracks and alligator areas a
//! wheel meets moving from one point to the next – computed from the same
//! pure data the renderer draws ([`crate::road_wear`], the strips' frame:
//! across `u` from the drawn centre line, along `v` the strip's arc length
//! from `RoadDress::start_m`), so what a rider sees is what they ride over.
//!
//! The game server and the client's prediction both ask it (through the
//! road network, [`crate::RoadNetwork::road_feel`], and
//! [`crate::Track::road_feel`]); `ocw-sim::road_feel` turns the answers
//! into physics (rolling resistance, a bump's speed loss) and the client
//! into the trainer kick, the camera shake and the sound.
//!
//! Only strips from [`FEEL_MIN_GRADE`] on are kept (nothing is felt on
//! excellent and good roads, grade 1 – 2): after Phase 8 Q14 (b) that is
//! the tagged intermediate and worse roads and the untagged tracks. The
//! amounts are the full ones of the grade (the High preset's share 1; the
//! potholes are drawn in full by every preset, `ocw-render::road_material`).
//! Like the static obstacles, the map holds the home package's tiles; the
//! browser app adds tiles as they are streamed in ([`RoadFeelMap::add_tile`]).

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::RoadSurface;
use crate::graph::RoadClass;
use crate::package::{RoadStrip, Tile, TileKey, WorldPackage};
use crate::road_dress::patch_at;
use crate::road_wear::{self, ALLIGATOR_CELL_M, POTHOLE_CELL_M, TRANSVERSE_CELL_M};

/// Strips of this wear grade and worse are felt; grades 1 – 2 (excellent,
/// good) are not.
pub const FEEL_MIN_GRADE: u8 = 3;
/// Cell size of a tile's segment index (m).
const CELL_M: f64 = 16.0;
/// A wheel is in a pothole within this share of its drawn radius (the
/// outline is ragged, the rim breaks off).
pub const HOLE_SHARE: f32 = 0.8;
/// Tiles this close to a point are searched too (a strip's segment may
/// reach over the tile border).
const BORDER_M: f64 = 8.0;

/// What a wheel meets on a damaged road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeatureKind {
    /// A pothole (on gravel and dirt a hollow, on paving a sunken spot),
    /// with its radius (m).
    Pothole { r: f32 },
    /// A pothole that bites into the road edge.
    EdgeBreak { r: f32 },
    /// An open transverse crack (sealed ones are smooth).
    Crack,
    /// Into an alligator (fatigue cracking) area.
    Alligator,
}

/// A feature met, with an id that is the same every time the same feature
/// is met (the client plays each once).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Feature {
    pub kind: FeatureKind,
    /// The road's surface (holes on gravel are softer hollows).
    pub surface: RoadSurface,
    pub grade: u8,
    pub id: u64,
}

/// The damaged strip under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripFeel {
    pub grade: u8,
    pub surface: RoadSurface,
    /// Across (m from the drawn centre line, + right of the strip's
    /// direction) and along (m, the strip frame).
    pub u: f32,
    pub v: f32,
}

#[derive(Debug)]
struct FeelStrip {
    points: Vec<[f32; 2]>,
    /// The strip frame's along at each point (3D arc length, as drawn).
    along: Vec<f32>,
    half: f32,
    grade: u8,
    surface: RoadSurface,
    track: bool,
    /// Cracks, sealing, alligator areas and patches on this surface.
    cracks: bool,
    transverse: f32,
    alligator: f32,
    pothole: f32,
    patch: f32,
    sealed: f32,
    zone: f32,
}

impl FeelStrip {
    fn of(s: &RoadStrip) -> Option<Self> {
        let grade = road_wear::wear_grade(s.class, s.surface, &s.condition);
        if grade < FEEL_MIN_GRADE || s.points.len() < 2 {
            return None;
        }
        let d = road_wear::wear_density(grade, 1.0);
        let (cracks, factor) = road_wear::surface_factors(s.surface);
        let mut along = Vec::with_capacity(s.points.len());
        let mut a = s.dress.start_m;
        along.push(a);
        for w in s.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            a += ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
            along.push(a);
        }
        Some(Self {
            points: s.points.iter().map(|p| [p[0], p[2]]).collect(),
            along,
            half: s.width_m / 2.0,
            grade,
            surface: s.surface,
            track: s.class == RoadClass::Track,
            cracks,
            transverse: d.transverse,
            alligator: d.alligator,
            pothole: d.pothole * factor,
            patch: d.patch,
            sealed: d.sealed,
            zone: road_wear::pothole_zone(d.pothole_zone_m, road_wear::main_road(s.class)),
        })
    }

    /// The frame of `p` on segment `i` (t clamped for v, u along the
    /// segment's normal) and the distance to the segment.
    fn frame(&self, i: usize, p: [f64; 2]) -> (f32, f32, f64) {
        let (a, b) = (self.points[i], self.points[i + 1]);
        let (ax, az) = (f64::from(a[0]), f64::from(a[1]));
        let (dx, dz) = (f64::from(b[0]) - ax, f64::from(b[1]) - az);
        let len2 = (dx * dx + dz * dz).max(1e-12);
        let (px, pz) = (p[0] - ax, p[1] - az);
        let t = ((px * dx + pz * dz) / len2).clamp(0.0, 1.0);
        let len = len2.sqrt();
        // Right of the direction: (-dz, dx), as the road mesh.
        let u = (px * -dz + pz * dx) / len;
        let (qx, qz) = (t * dx - px, t * dz - pz);
        let dist = (qx * qx + qz * qz).sqrt();
        let v = self.along[i] + (self.along[i + 1] - self.along[i]) * t as f32;
        (u as f32, v, dist)
    }
}

#[derive(Debug, Default)]
struct FeelTile {
    strips: Vec<FeelStrip>,
    /// (strip, segment) per index cell.
    cells: BTreeMap<(i32, i32), Vec<(u32, u32)>>,
}

impl FeelTile {
    fn of(tile: &Tile) -> Self {
        let strips: Vec<FeelStrip> = tile.roads.iter().filter_map(FeelStrip::of).collect();
        let mut cells: BTreeMap<(i32, i32), Vec<(u32, u32)>> = BTreeMap::new();
        for (si, s) in strips.iter().enumerate() {
            let m = f64::from(s.half) + 0.5;
            for (i, w) in s.points.windows(2).enumerate() {
                let lo = [
                    f64::from(w[0][0].min(w[1][0])) - m,
                    f64::from(w[0][1].min(w[1][1])) - m,
                ];
                let hi = [
                    f64::from(w[0][0].max(w[1][0])) + m,
                    f64::from(w[0][1].max(w[1][1])) + m,
                ];
                let (c0, c1) = (cell(lo), cell(hi));
                for x in c0.0..=c1.0 {
                    for z in c0.1..=c1.1 {
                        cells.entry((x, z)).or_default().push((si as u32, i as u32));
                    }
                }
            }
        }
        Self { strips, cells }
    }

    /// The nearest damaged strip whose carriageway holds `p`: (strip,
    /// segment, u, v, distance).
    fn locate(&self, p: [f64; 2]) -> Option<(usize, usize, f32, f32, f64)> {
        let mut best: Option<(usize, usize, f32, f32, f64)> = None;
        for &(si, i) in self.cells.get(&cell(p))? {
            let s = &self.strips[si as usize];
            let (u, v, dist) = s.frame(i as usize, p);
            if dist <= f64::from(s.half) + 0.01 && best.is_none_or(|b| dist < b.4) {
                best = Some((si as usize, i as usize, u, v, dist));
            }
        }
        best
    }
}

fn cell(p: [f64; 2]) -> (i32, i32) {
    (
        (p[0] / CELL_M).floor() as i32,
        (p[1] / CELL_M).floor() as i32,
    )
}

/// The damaged roads of a world, indexed for the riders' wheels.
pub struct RoadFeelMap {
    tile_size_m: f64,
    tiles: RwLock<BTreeMap<TileKey, Arc<FeelTile>>>,
}

impl std::fmt::Debug for RoadFeelMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoadFeelMap")
            .field("tile_size_m", &self.tile_size_m)
            .field("tiles", &self.tile_count())
            .finish()
    }
}

impl RoadFeelMap {
    /// An empty map of tiles of `tile_size_m`.
    pub fn new(tile_size_m: f32) -> Self {
        Self {
            tile_size_m: f64::from(tile_size_m),
            tiles: RwLock::new(BTreeMap::new()),
        }
    }

    /// The map of a package's tiles (those it holds: a streamed base has
    /// none yet).
    pub fn from_package(pkg: &WorldPackage) -> Self {
        let map = Self::new(pkg.tile_size_m);
        for tile in pkg.tiles.values() {
            map.add_tile(tile);
        }
        map
    }

    /// Adds (or replaces) a tile's damaged strips; a tile without any is
    /// kept empty, so it is not added again.
    pub fn add_tile(&self, tile: &Tile) {
        let t = Arc::new(FeelTile::of(tile));
        if let Ok(mut tiles) = self.tiles.write() {
            tiles.insert(tile.key, t);
        }
    }

    /// Whether the map has `key` (the browser adds streamed tiles once).
    pub fn has_tile(&self, key: TileKey) -> bool {
        self.tiles.read().is_ok_and(|t| t.contains_key(&key))
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.read().map_or(0, |t| t.len())
    }

    /// Damaged strips held (statistics, tests).
    pub fn strip_count(&self) -> usize {
        self.tiles
            .read()
            .map_or(0, |t| t.values().map(|t| t.strips.len()).sum())
    }

    /// The tiles that may hold a strip under `p`.
    fn tiles_near(&self, p: [f64; 2]) -> Vec<(TileKey, Arc<FeelTile>)> {
        let Ok(tiles) = self.tiles.read() else {
            return Vec::new();
        };
        let size = self.tile_size_m;
        let mut keys: Vec<TileKey> = Vec::with_capacity(4);
        for dx in [-BORDER_M, BORDER_M] {
            for dz in [-BORDER_M, BORDER_M] {
                let k = TileKey::containing([p[0] + dx, p[1] + dz], size);
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys.into_iter()
            .filter_map(|k| tiles.get(&k).map(|t| (k, t.clone())))
            .collect()
    }

    /// The nearest damaged strip under `p` with its tile.
    fn locate(&self, p: [f64; 2]) -> Option<(TileKey, Arc<FeelTile>, usize, usize, f32, f32)> {
        let mut best: Option<(TileKey, Arc<FeelTile>, usize, usize, f32, f32, f64)> = None;
        for (k, t) in self.tiles_near(p) {
            if let Some((si, i, u, v, d)) = t.locate(p)
                && best.as_ref().is_none_or(|b| d < b.6)
            {
                best = Some((k, t, si, i, u, v, d));
            }
        }
        best.map(|(k, t, si, i, u, v, _)| (k, t, si, i, u, v))
    }

    /// The damaged strip under the point `p` (home metres, x, z); `None`
    /// on good roads and off the road.
    pub fn at(&self, p: [f64; 2]) -> Option<StripFeel> {
        let (_, t, si, _, u, v) = self.locate(p)?;
        let s = &t.strips[si];
        Some(StripFeel {
            grade: s.grade,
            surface: s.surface,
            u,
            v,
        })
    }

    /// What a wheel meets moving from `from` to `to` (home metres; one
    /// physics tick, well under a metre): the features of the strip under
    /// `to` it enters on the way, in no particular order.
    pub fn crossed(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Feature> {
        let mut out = Vec::new();
        let Some((key, t, si, i, u1, v1)) = self.locate(to) else {
            return out;
        };
        let s = &t.strips[si];
        // The start in the same segment's frame (a tick is short).
        let (u0, v0, _) = s.frame(i, from);
        let id = |kind: u64, k: i32, side: i32| -> u64 {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for x in [
                key.x as i64 as u64,
                key.z as i64 as u64,
                si as u64,
                kind,
                k as i64 as u64,
                side as i64 as u64,
            ] {
                h ^= x;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            h
        };
        let feature = |kind: FeatureKind, id: u64| Feature {
            kind,
            surface: s.surface,
            grade: s.grade,
            id,
        };
        let (lo, hi) = (v0.min(v1), v0.max(v1));
        // Potholes and edge breaks: the wheel's way passes within the
        // hole's radius and did not start in it.
        if s.pothole > 0.0 {
            let k0 = ((lo - road_wear::MAX_POTHOLE_R) / POTHOLE_CELL_M).floor() as i32;
            let k1 = ((hi + road_wear::MAX_POTHOLE_R) / POTHOLE_CELL_M).floor() as i32;
            for k in k0..=k1 {
                for side in 0..2 {
                    let Some(p) =
                        road_wear::pothole_in_cell(k, side, s.half, s.pothole, s.zone, s.track)
                    else {
                        continue;
                    };
                    let reach = p.r * HOLE_SHARE;
                    let start = ((u0 - p.u).powi(2) + (v0 - p.v).powi(2)).sqrt();
                    if start > reach && seg_dist([u0, v0], [u1, v1], [p.u, p.v]) <= reach {
                        let kind = if p.edge {
                            FeatureKind::EdgeBreak { r: p.r }
                        } else {
                            FeatureKind::Pothole { r: p.r }
                        };
                        out.push(feature(kind, id(1, k, side)));
                    }
                }
            }
        }
        if !s.cracks {
            return out;
        }
        let on = |u: f32| u.abs() <= s.half + 0.01;
        // Open transverse cracks crossed (none in a repaired patch).
        if s.transverse > 0.0 && on(u1) {
            let k0 = (lo / TRANSVERSE_CELL_M).floor() as i32;
            let k1 = (hi / TRANSVERSE_CELL_M).floor() as i32;
            for k in k0..=k1 {
                let Some(c) = road_wear::transverse_in_cell(k, s.half, s.transverse, s.sealed)
                else {
                    continue;
                };
                if c.sealed || u1.abs() > c.reach {
                    continue;
                }
                let at0 = road_wear::transverse_v_at(&c, u0);
                let at1 = road_wear::transverse_v_at(&c, u1);
                let crossed = (v0 - at0).signum() != (v1 - at1).signum() && v0 != at0;
                if crossed && !patch_at(s.half, u1, at1, s.patch) {
                    out.push(feature(FeatureKind::Crack, id(2, k, 0)));
                }
            }
        }
        // Into an alligator area (its ellipse, without the shader's noise).
        if s.alligator > 0.0 {
            let k0 = (lo / ALLIGATOR_CELL_M).floor() as i32;
            let k1 = (hi / ALLIGATOR_CELL_M).floor() as i32;
            for k in k0..=k1 {
                for side in 0..2 {
                    let Some(a) =
                        road_wear::alligator_in_cell(k, side, s.half, s.alligator, s.track)
                    else {
                        continue;
                    };
                    let inside = |u: f32, v: f32| {
                        let x = (u - a.u) / a.half_w;
                        let h = 0.5 * (a.v.1 - a.v.0);
                        let y = (v - a.v.0 - h) / h;
                        x * x + y * y < 0.85
                    };
                    if inside(u1, v1) && !inside(u0, v0) && !patch_at(s.half, u1, v1, s.patch) {
                        out.push(feature(FeatureKind::Alligator, id(3, k, side)));
                    }
                }
            }
        }
        out
    }
}

/// The distance of point `p` from the segment `a`–`b`.
fn seg_dist(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 1e-12 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    (qx * qx + qy * qy).sqrt()
}

/// A straight test world for the crates' tests (not used by the apps):
/// one road along +x at z = 500 from x = 10, `len` m long (within the
/// first 1 km tile), with its drawn strip in `smoothness` (`None`: no
/// damage data at all).
#[doc(hidden)]
pub mod fixture {
    use super::*;
    use crate::RoadCondition;
    use crate::graph::{Edge, EdgeId, Oneway, RoadGraph, VertexId};
    use crate::road_condition::Smoothness;
    use crate::road_dress::RoadDress;

    /// The strip of the road.
    pub fn strip(class: RoadClass, sm: Smoothness, width: f32, len: f32) -> RoadStrip {
        let n = (len / 5.0).round() as usize;
        RoadStrip {
            class,
            width_m: width,
            points: (0..=n)
                .map(|i| [10.0 + len * i as f32 / n as f32, 0.0, 500.0])
                .collect(),
            surface: RoadSurface::Asphalt,
            dress: RoadDress {
                start_m: 0.0,
                ..RoadDress::of_class(class, width)
            },
            lit: Default::default(),
            condition: RoadCondition {
                smoothness: sm,
                ..RoadCondition::UNKNOWN
            },
        }
    }

    /// A map of `strips` on tile (0, 0) of 1 km tiles.
    pub fn map_of(strips: Vec<RoadStrip>) -> RoadFeelMap {
        let map = RoadFeelMap::new(1000.0);
        map.add_tile(&Tile {
            key: TileKey { x: 0, z: 0 },
            heights: Vec::new(),
            roads: strips,
        });
        map
    }

    /// The road network of the straight road, with the strip's damage.
    pub fn network(
        class: RoadClass,
        sm: Option<Smoothness>,
        width: f32,
        len: f32,
    ) -> Arc<crate::RoadNetwork> {
        let a = [10.0, 500.0];
        let b = [10.0 + f64::from(len), 500.0];
        let graph = RoadGraph {
            vertices: vec![a, b],
            edges: vec![Edge {
                id: EdgeId(1),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![a, b],
                class,
                name: Some("Pothole Lane".into()),
                width_m: f64::from(width),
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: 1,
                profile: Vec::new(),
            }],
            boundary_vertices: Default::default(),
        };
        let feel = sm.map(|sm| Arc::new(map_of(vec![strip(class, sm, width, len)])));
        Arc::new(crate::RoadNetwork::new(graph).with_road_feel(feel))
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{map_of, strip};
    use super::*;
    use crate::road_condition::Smoothness;

    /// Rides 900 m along the strip at `u` across in 0.3 m steps.
    fn ride(map: &RoadFeelMap, u: f64) -> Vec<Feature> {
        let mut out = Vec::new();
        let mut x = 12.0;
        while x < 910.0 {
            out.extend(map.crossed([x, 500.0 + u], [x + 0.3, 500.0 + u]));
            x += 0.3;
        }
        out
    }

    #[test]
    fn the_frame_is_the_road_meshes() {
        let map = map_of(vec![strip(
            RoadClass::Residential,
            Smoothness::Bad,
            6.0,
            900.0,
        )]);
        // +x direction: right is +z.
        let f = map.at([110.0, 501.5]).unwrap();
        assert!(
            (f.u - 1.5).abs() < 1e-4 && (f.v - 100.0).abs() < 1e-3,
            "{f:?}"
        );
        assert_eq!((f.grade, f.surface), (5, RoadSurface::Asphalt));
        // Off the carriageway: nothing.
        assert!(map.at([110.0, 503.2]).is_none());
        // A good road is not kept at all.
        let good = map_of(vec![strip(
            RoadClass::Residential,
            Smoothness::Good,
            6.0,
            900.0,
        )]);
        assert_eq!(good.strip_count(), 0);
        assert!(good.at([110.0, 500.0]).is_none());
        assert!(ride(&good, -1.5).is_empty());
    }

    #[test]
    fn the_wheel_meets_the_drawn_potholes_and_cracks() {
        let half = 3.0;
        let s = strip(
            RoadClass::Residential,
            Smoothness::VeryBad,
            2.0 * half,
            900.0,
        );
        let map = map_of(vec![s]);
        let d = road_wear::wear_density(6, 1.0);
        // Every pothole met is one road_wear places, and the wheel's line
        // passes within its radius.
        let mut holes = 0;
        for u in [-2.6f32, -1.8, -0.9, 0.9, 1.8, 2.6] {
            for f in ride(&map, f64::from(u)) {
                if let FeatureKind::Pothole { r } | FeatureKind::EdgeBreak { r } = f.kind {
                    holes += 1;
                    let found = (-10..140).any(|k| {
                        (0..2).any(|side| {
                            road_wear::pothole_in_cell(
                                k,
                                side,
                                half,
                                d.pothole,
                                d.pothole_zone_m,
                                false,
                            )
                            .is_some_and(|p| p.r == r && (p.u - u).abs() <= p.r)
                        })
                    });
                    assert!(found, "{f:?} at {u}");
                }
            }
        }
        assert!(holes > 5, "{holes}");
        // Cracks: about as many as the open transverse cracks reaching
        // the line.
        let cracks = ride(&map, 0.5)
            .iter()
            .filter(|f| f.kind == FeatureKind::Crack)
            .count();
        let expect = (0..82)
            .filter_map(|k| road_wear::transverse_in_cell(k, half, d.transverse, d.sealed))
            .filter(|c| !c.sealed && c.reach >= 0.5)
            .count();
        assert!(cracks > 0 && cracks <= expect, "{cracks} of {expect}");
        // The same feature has the same id on every ride.
        assert_eq!(ride(&map, 1.8), ride(&map, 1.8));
    }

    #[test]
    fn worse_roads_are_felt_more() {
        let count = |sm| {
            let map = map_of(vec![strip(RoadClass::Residential, sm, 6.0, 900.0)]);
            [-2.2, -0.8, 0.8, 2.2]
                .iter()
                .map(|&u| ride(&map, u).len())
                .sum::<usize>()
        };
        let (im, bad, horrible) = (
            count(Smoothness::Intermediate),
            count(Smoothness::Bad),
            count(Smoothness::Horrible),
        );
        assert!(im < bad && bad < horrible, "{im} {bad} {horrible}");
        assert_eq!(count(Smoothness::Excellent), 0);
        assert_eq!(count(Smoothness::Good), 0);
    }
}
