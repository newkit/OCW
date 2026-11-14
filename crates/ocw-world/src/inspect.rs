//! What is at a point of the world (spec §14 Phase 13D): god mode's
//! crosshair casts a ray from the camera into the world ([`raycast`]) and
//! names what it hits ([`inspect`]): the road (name, OSM way, class,
//! surface, width, bridge / tunnel, gradient), the land-use area (`LUSE`),
//! a building (its `BTAG` facts), water, and the elevation there.
//!
//! Pure (no Bevy, no I/O): it reads the [`WorldSet`]'s resident packages,
//! so it builds for `wasm32-unknown-unknown` too (spec §3.5).

use crate::buildings::{Building, BuildingKind, BuildingUse, Roof, RoofShape};
use crate::geo::LocalProjection;
use crate::graph::{Edge, RoadClass};
use crate::landuse::LandClass;
use crate::package::{TileKey, WorldPackage};
use crate::road_surface::RoadSurface;
use crate::world_set::WorldSet;

/// The ray never goes farther than this (m).
pub const MAX_RAY_M: f64 = 30_000.0;
/// The ray's longest step (m): obstacles wider than this are never missed.
pub const MAX_STEP_M: f64 = 20.0;
/// A point this close to a road's edge still counts as on the road (m).
pub const ROAD_MARGIN_M: f64 = 1.0;
/// A point this close to a building's walls still counts as on it (m).
pub const BUILDING_MARGIN_M: f64 = 0.5;

/// Casts a ray from `origin` along `dir` (need not be normalised) over the
/// `surface` (the highest solid top at (x, z): terrain, water, buildings;
/// `None` outside the world) and returns the first point where it meets
/// it, within `max_m`. The ray marches with steps that shrink near the
/// surface (half the height above it, 0.5 … 20 m) and refines the hit by
/// bisection, so it neither tunnels through a ridge at a shallow angle nor
/// costs much over open ground.
pub fn raycast(
    origin: [f64; 3],
    dir: [f64; 3],
    max_m: f64,
    surface: &dyn Fn(f64, f64) -> Option<f64>,
) -> Option<[f64; 3]> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-9 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let at = |t: f64| {
        [
            origin[0] + d[0] * t,
            origin[1] + d[1] * t,
            origin[2] + d[2] * t,
        ]
    };
    // Height of the ray above the surface at t (positive = above).
    let above = |t: f64| {
        let p = at(t);
        surface(p[0], p[2]).map(|h| p[1] - h)
    };
    let max_m = max_m.min(MAX_RAY_M);
    let mut t = 0.0;
    let mut prev = 0.0;
    for _ in 0..20_000 {
        // Past the end, or climbing above everything (the flight's ceiling).
        if t > max_m || (d[1] > 0.0 && at(t)[1] > 9_000.0) {
            return None;
        }
        match above(t) {
            Some(h) if h <= 0.0 => {
                if t == 0.0 {
                    return Some(at(0.0));
                }
                // Bisect between the last point above and this one.
                let (mut lo, mut hi) = (prev, t);
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if above(mid).is_some_and(|h| h <= 0.0) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return Some(at(hi));
            }
            Some(h) => {
                prev = t;
                t += (h * 0.5).clamp(0.5, MAX_STEP_M);
            }
            // Outside the world: big steps until it comes back.
            None => {
                prev = t;
                t += 25.0;
            }
        }
    }
    None
}

/// A road at the inspected point.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadInfo {
    pub name: Option<String>,
    /// OSM way id (0: unknown, e.g. without a road graph).
    pub osm_way: i64,
    pub class: RoadClass,
    /// The look of the road (`RSRF`).
    pub surface: RoadSurface,
    pub width_m: f64,
    pub bridge: bool,
    pub tunnel: bool,
    /// Gradient (rise / run) along the looking direction where given, else
    /// along the road's own direction; `None` without heights.
    pub gradient: Option<f64>,
}

/// A building at the inspected point.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildingInfo {
    pub kind: BuildingKind,
    pub building_use: Option<BuildingUse>,
    pub levels: Option<u8>,
    pub roof_shape: Option<RoofShape>,
    /// From the foot of the walls to the top (ridge or flat roof), m.
    pub height_m: f64,
}

/// What is at a point.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inspection {
    /// The point (home metres; y up).
    pub point: [f64; 3],
    /// Terrain elevation there (m above sea level).
    pub elevation_m: Option<f64>,
    pub road: Option<RoadInfo>,
    pub building: Option<BuildingInfo>,
    /// The land-use class of the area (`LUSE`), if the package has it.
    pub area: Option<LandClass>,
    /// Water: its surface level (m).
    pub water_m: Option<f64>,
    /// The member package (its id) and the tile there.
    pub tile: Option<(String, TileKey)>,
    /// Latitude and longitude (degrees).
    pub lat_lon: Option<(f64, f64)>,
}

/// Distance from `p` to the segment a–b and the parameter (0 … 1) of the
/// nearest point.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - q[0]).hypot(p[1] - q[1]), t)
}

/// Whether `p` is inside the ring (even-odd) or within `margin` of it.
pub fn ring_covers(ring: &[[f32; 2]], p: [f64; 2], margin: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut near = false;
    for i in 0..n {
        let a = [f64::from(ring[i][0]), f64::from(ring[i][1])];
        let b = [
            f64::from(ring[(i + 1) % n][0]),
            f64::from(ring[(i + 1) % n][1]),
        ];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            inside = !inside;
        }
        if segment_distance(p, a, b).0 <= margin {
            near = true;
        }
    }
    inside || near
}

/// The top of a building (ridge or flat roof, m).
pub fn building_top(b: &Building) -> f64 {
    let ridge = match b.roof {
        Roof::Flat => 0.0,
        Roof::Gabled { height_m } => height_m,
    };
    let pyramid = match (b.tags.roof_shape, b.roof) {
        (Some(RoofShape::Pyramidal), Roof::Flat) => b.tags.roof_height_m.unwrap_or(0.0),
        _ => 0.0,
    };
    f64::from(b.eave_y + ridge.max(pyramid))
}

/// The building whose footprint covers local (x, z) of `pkg` (within
/// [`BUILDING_MARGIN_M`]); buildings are filed under the tile of their
/// outline, so the neighbouring tiles are searched too. The highest wins
/// where footprints overlap.
pub fn building_at(pkg: &WorldPackage, q: [f64; 2]) -> Option<&Building> {
    let all = pkg.buildings.as_ref()?;
    let key = TileKey::containing(q, f64::from(pkg.tile_size_m));
    let mut best: Option<&Building> = None;
    for dz in -1..=1 {
        for dx in -1..=1 {
            let k = TileKey {
                x: key.x + dx,
                z: key.z + dz,
            };
            for b in all.get(&k).into_iter().flatten() {
                if ring_covers(&b.outline, q, BUILDING_MARGIN_M)
                    && best.is_none_or(|o| building_top(b) > building_top(o))
                {
                    best = Some(b);
                }
            }
        }
    }
    best
}

/// The highest solid top at local (x, z) of `pkg`: terrain, water and
/// buildings (what the crosshair's ray meets).
pub fn solid_top(pkg: &WorldPackage, q: [f64; 2]) -> Option<f64> {
    let ground = pkg.surface_height(q[0], q[1])?;
    let water = water_level(pkg, q).unwrap_or(f64::NEG_INFINITY);
    let house = building_at(pkg, q).map_or(f64::NEG_INFINITY, building_top);
    Some(ground.max(water).max(house))
}

/// The water level at local (x, z) of `pkg`: from the `LUSE` levels at
/// the corners of the terrain cell, where the point is water (its class,
/// or a level above the terrain).
pub fn water_level(pkg: &WorldPackage, q: [f64; 2]) -> Option<f64> {
    let lu = pkg.landuse.as_ref()?;
    let size = f64::from(pkg.tile_size_m);
    let key = TileKey::containing(q, size);
    let t = lu.tiles.get(&key)?;
    if t.water.is_empty() {
        return None;
    }
    let o = key.origin(size);
    let n = pkg.samples as usize;
    let s = pkg.spacing_m();
    let fx = ((q[0] - o[0]) / s).clamp(0.0, (n - 1) as f64);
    let fz = ((q[1] - o[1]) / s).clamp(0.0, (n - 1) as f64);
    let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
    let level = [(i, k), (i + 1, k), (i, k + 1), (i + 1, k + 1)]
        .iter()
        .filter_map(|&(i, k)| t.water_level((k * n + i) as u32))
        .map(f64::from)
        .reduce(f64::max)?;
    let class = lu.class_at(size, q[0], q[1]);
    let ground = pkg.surface_height(q[0], q[1]).unwrap_or(f64::INFINITY);
    (class == Some(LandClass::Water) || level > ground + 0.05).then_some(level)
}

/// The road strip of the tile covering local `q` (the nearest within half
/// its width and [`ROAD_MARGIN_M`]): class, width, surface and the
/// gradient along the strip, with the distance to its centre line.
fn strip_at(pkg: &WorldPackage, q: [f64; 2]) -> Option<(RoadClass, f64, RoadSurface, f64, f64)> {
    let size = f64::from(pkg.tile_size_m);
    let tile = pkg.tiles.get(&TileKey::containing(q, size))?;
    let mut best: Option<(RoadClass, f64, RoadSurface, f64, f64)> = None;
    for s in &tile.roads {
        for w in s.points.windows(2) {
            let a = [f64::from(w[0][0]), f64::from(w[0][2])];
            let b = [f64::from(w[1][0]), f64::from(w[1][2])];
            let (d, _) = segment_distance(q, a, b);
            let reach = f64::from(s.width_m) / 2.0 + ROAD_MARGIN_M;
            if d <= reach && best.is_none_or(|o| d < o.4) {
                let run = (b[0] - a[0]).hypot(b[1] - a[1]);
                let grade = if run > 0.1 {
                    f64::from(w[1][1] - w[0][1]) / run
                } else {
                    0.0
                };
                best = Some((s.class, f64::from(s.width_m), s.surface, grade, d));
            }
        }
    }
    best
}

/// The graph edge nearest to local `q` (within its half width and
/// [`ROAD_MARGIN_M`], preferring `class`), with the distance along it and
/// its direction there.
fn edge_at(
    pkg: &WorldPackage,
    q: [f64; 2],
    class: Option<RoadClass>,
) -> Option<(&Edge, f64, [f64; 2])> {
    let g = pkg.graph.as_ref()?;
    let mut best: Option<(&Edge, f64, [f64; 2], f64)> = None;
    for e in &g.edges {
        let reach = e.width_m / 2.0 + ROAD_MARGIN_M + 1.0;
        let mut along = 0.0;
        for w in e.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let run = (b[0] - a[0]).hypot(b[1] - a[1]);
            // Cheap box test first.
            if q[0] < a[0].min(b[0]) - reach
                || q[0] > a[0].max(b[0]) + reach
                || q[1] < a[1].min(b[1]) - reach
                || q[1] > a[1].max(b[1]) + reach
            {
                along += run;
                continue;
            }
            let (d, t) = segment_distance(q, a, b);
            // The road of the strip's class wins at a junction.
            let penalty = if class.is_some_and(|c| c != e.class) {
                0.5
            } else {
                0.0
            };
            if d <= reach && best.is_none_or(|o| d + penalty < o.3) {
                let dir = if run > 0.0 {
                    [(b[0] - a[0]) / run, (b[1] - a[1]) / run]
                } else {
                    [0.0, 0.0]
                };
                best = Some((e, along + run * t, dir, d + penalty));
            }
            along += run;
        }
    }
    best.map(|(e, s, dir, _)| (e, s, dir))
}

/// The gradient of `e` at `s` m from its start (from its profile).
fn edge_gradient(e: &Edge, s: f64) -> Option<f64> {
    let len = e.length_m();
    if len < 1.0 {
        return None;
    }
    let (a, b) = ((s - 5.0).max(0.0), (s + 5.0).min(len));
    Some((e.height_at(b)? - e.height_at(a)?) / (b - a).max(0.1))
}

/// What is at `p` (home metres) at height `y`: the road, building, area,
/// water and elevation of the package that draws the ground there.
/// `heading` (horizontal unit vector x, z) signs the road's gradient: up
/// in the looking direction is positive.
pub fn inspect(set: &WorldSet, p: [f64; 2], y: f64, heading: Option<[f64; 2]>) -> Inspection {
    let mut out = Inspection {
        point: [p[0], y, p[1]],
        ..Default::default()
    };
    let member = set.owner(p).or_else(|| {
        set.members()
            .iter()
            .position(|m| m.is_resident() && m.has_tile_at(p))
    });
    let Some(i) = member else {
        return out;
    };
    let m = &set.members()[i];
    let Some(pkg) = m.package() else {
        return out;
    };
    let q = m.frame.invert(p);
    let size = f64::from(pkg.tile_size_m);
    out.tile = Some((m.id.clone(), TileKey::containing(q, size)));
    out.lat_lon = Some(LocalProjection::new(pkg.origin.0, pkg.origin.1).unproject(q));
    out.elevation_m = pkg.surface_height(q[0], q[1]);
    out.area = pkg
        .landuse
        .as_ref()
        .and_then(|l| l.class_at(size, q[0], q[1]));
    out.water_m = water_level(pkg, q);
    if let Some(b) = building_at(pkg, q) {
        out.building = Some(BuildingInfo {
            kind: b.kind,
            building_use: b.tags.building_use,
            levels: b.tags.levels,
            roof_shape: b.tags.roof_shape.or(match b.roof {
                Roof::Flat => None,
                Roof::Gabled { .. } => Some(RoofShape::Gabled),
            }),
            height_m: building_top(b) - f64::from(b.base_y),
        });
    }
    let strip = strip_at(pkg, q);
    // The graph is searched only on a strip, or on a tile without strips
    // (it is the whole package's: the strips are the cheap test).
    let no_strips = pkg
        .tiles
        .get(&TileKey::containing(q, size))
        .is_none_or(|t| t.roads.is_empty());
    let edge = if strip.is_some() || no_strips {
        edge_at(pkg, q, strip.map(|s| s.0))
    } else {
        None
    };
    // A road needs a strip (it is drawn there) or, without strips, the
    // graph's edge within its width.
    let on_road = strip.is_some()
        || edge.is_some_and(|(e, _, _)| {
            e.points
                .windows(2)
                .map(|w| segment_distance(q, w[0], w[1]).0)
                .fold(f64::INFINITY, f64::min)
                <= e.width_m / 2.0 + ROAD_MARGIN_M
        });
    if on_road && out.building.is_none() {
        let sign = |dir: [f64; 2]| match heading {
            Some(h) if h[0] * dir[0] + h[1] * dir[1] < 0.0 => -1.0,
            _ => 1.0,
        };
        out.road = Some(match (edge, strip) {
            (Some((e, s, dir)), strip) => RoadInfo {
                name: e.name.clone(),
                osm_way: e.osm_way,
                class: e.class,
                surface: strip.map_or(RoadSurface::of_class(e.class), |s| s.2),
                width_m: e.width_m,
                bridge: e.bridge,
                tunnel: e.tunnel,
                gradient: edge_gradient(e, s)
                    .or(strip.map(|s| s.3))
                    .map(|g| g * sign(dir)),
            },
            (None, Some((class, width, surface, grade, _))) => RoadInfo {
                name: None,
                osm_way: 0,
                class,
                surface,
                width_m: width,
                bridge: false,
                tunnel: false,
                gradient: Some(grade),
            },
            (None, None) => unreachable!("on a road"),
        });
    }
    out
}

/// The highest solid top at `p` (home metres) over the world set's
/// resident packages ([`solid_top`] of the package drawing the ground).
pub fn set_solid_top(set: &WorldSet, p: [f64; 2]) -> Option<f64> {
    let i = set.owner(p)?;
    let m = &set.members()[i];
    solid_top(m.package()?, m.frame.invert(p))
}

/// The water level at `p` (home metres), if there is water.
pub fn set_water_level(set: &WorldSet, p: [f64; 2]) -> Option<f64> {
    let i = set.owner(p)?;
    let m = &set.members()[i];
    water_level(m.package()?, m.frame.invert(p))
}

/// A lower-case label of a road class.
pub fn class_name(c: RoadClass) -> &'static str {
    match c {
        RoadClass::Trunk => "trunk",
        RoadClass::Primary => "primary",
        RoadClass::Secondary => "secondary",
        RoadClass::Tertiary => "tertiary",
        RoadClass::Unclassified => "unclassified",
        RoadClass::Residential => "residential",
        RoadClass::Service => "service",
        RoadClass::Track => "track",
        RoadClass::Cycleway => "cycleway",
        RoadClass::Path => "path",
    }
}

/// A lower-case label of a road surface.
pub fn surface_name(s: RoadSurface) -> &'static str {
    match s {
        RoadSurface::Asphalt => "asphalt",
        RoadSurface::WornAsphalt => "worn asphalt",
        RoadSurface::Gravel => "gravel",
        RoadSurface::Paving => "paving",
        RoadSurface::Dirt => "dirt",
    }
}

/// A lower-case label of a land class.
pub fn area_name(c: LandClass) -> &'static str {
    match c {
        LandClass::Default => "open land",
        LandClass::Meadow => "meadow",
        LandClass::Farmland => "farmland",
        LandClass::Forest => "forest",
        LandClass::Orchard => "orchard",
        LandClass::Vineyard => "vineyard",
        LandClass::Residential => "residential",
        LandClass::Commercial => "commercial",
        LandClass::Industrial => "industrial",
        LandClass::Park => "park",
        LandClass::Water => "water",
        LandClass::Wetland => "wetland",
        LandClass::Rock => "rock",
        LandClass::Scree => "scree",
        LandClass::Glacier => "glacier",
        LandClass::Bare => "bare ground",
        LandClass::Scrub => "scrub",
    }
}

/// A lower-case label of a building kind.
pub fn kind_name(k: BuildingKind) -> &'static str {
    match k {
        BuildingKind::Other => "building",
        BuildingKind::House => "house",
        BuildingKind::Apartments => "apartments",
        BuildingKind::Commercial => "commercial",
        BuildingKind::Industrial => "industrial",
        BuildingKind::Farm => "farm",
        BuildingKind::Religious => "religious",
        BuildingKind::Public => "public",
        BuildingKind::Small => "small building",
    }
}

impl Inspection {
    /// The crosshair's label: one line per thing found, the most specific
    /// first (road or building, then water, the area, the elevation).
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(r) = &self.road {
            let mut parts = vec![match &r.name {
                Some(n) => n.clone(),
                None => "Unnamed road".into(),
            }];
            if r.osm_way != 0 {
                parts.push(format!("way {}", r.osm_way));
            }
            parts.push(class_name(r.class).into());
            parts.push(surface_name(r.surface).into());
            parts.push(format!("{:.1} m wide", r.width_m));
            if r.bridge {
                parts.push("bridge".into());
            }
            if r.tunnel {
                parts.push("tunnel".into());
            }
            if let Some(g) = r.gradient {
                let g = if (g * 100.0).abs() < 0.05 { 0.0 } else { g };
                parts.push(format!("{:+.1} %", g * 100.0));
            }
            out.push(parts.join(" · "));
        }
        if let Some(b) = &self.building {
            let mut parts = vec![format!(
                "Building: {}",
                b.building_use
                    .filter(|u| *u != BuildingUse::Unknown)
                    .map_or(kind_name(b.kind), BuildingUse::name)
            )];
            if let Some(l) = b.levels {
                parts.push(format!("{l} levels"));
            }
            if let Some(s) = b.roof_shape {
                parts.push(format!("{} roof", s.name()));
            }
            parts.push(format!("{:.0} m high", b.height_m));
            out.push(parts.join(" · "));
        }
        if let Some(w) = self.water_m {
            out.push(format!("Water · level {w:.1} m"));
        }
        if let Some(a) = self.area
            && a != LandClass::Water
        {
            out.push(format!("Area: {}", area_name(a)));
        }
        match self.elevation_m {
            Some(e) => out.push(format!("Elevation {e:.0} m")),
            None => out.push("Outside the world".into()),
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::buildings::BuildingTags;
    use crate::graph::{EdgeId, Oneway, RoadGraph, VertexId};
    use crate::landuse::{LandUse, LandUseTile};
    use crate::package::{RoadStrip, Tile};

    /// A 100 m tile, terrain at 400 m rising 1 m per 10 m in x: a road
    /// ("Landstrasse", way 4711) along z = 50 from x 0 to 100 on a bridge,
    /// a house at (20…30, 70…80), a lake (x < 20, z < 30), forest in
    /// the north-east (z < 50, x > 50), the rest meadow.
    pub(crate) fn test_package() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let n = 11usize;
        let heights: Vec<f32> = (0..n * n).map(|i| 400.0 + (i % n) as f32).collect();
        let road: Vec<[f32; 3]> = (0..11)
            .map(|i| [i as f32 * 10.0, 400.0 + i as f32, 50.0])
            .collect();
        let tile = Tile {
            key,
            heights,
            roads: vec![RoadStrip {
                class: RoadClass::Secondary,
                width_m: 6.0,
                points: road.clone(),
                surface: RoadSurface::Paving,
                dress: crate::RoadDress::of_class(RoadClass::Secondary, 6.0),
                lit: crate::RoadLit::Unknown,
            }],
        };
        let graph = RoadGraph {
            vertices: vec![[0.0, 50.0], [100.0, 50.0]],
            edges: vec![Edge {
                id: EdgeId(1),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![[0.0, 50.0], [100.0, 50.0]],
                class: RoadClass::Secondary,
                name: Some("Landstrasse".into()),
                width_m: 6.0,
                bridge: true,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: 4711,
                profile: (0..=20).map(|i| 400.0 + f64::from(i) * 0.5).collect(),
            }],
            boundary_vertices: Default::default(),
        };
        let cells = 10u32;
        let classes = (0..cells * cells)
            .map(|i| {
                let (c, r) = (i % cells, i / cells);
                if c < 2 && r < 3 {
                    LandClass::Water
                } else if c >= 5 && r < 5 {
                    LandClass::Forest
                } else {
                    LandClass::Meadow
                }
            })
            .collect();
        // Water levels at the vertices in and around the lake (x ≤ 30,
        // z ≤ 40), 0.5 m above the highest terrain there.
        let mut water = Vec::new();
        for k in 0..=4usize {
            for i in 0..=3usize {
                water.push(((k * n + i) as u32, 403.5));
            }
        }
        water.sort_by_key(|w| w.0);
        let mut landuse = LandUse {
            cells,
            tiles: BTreeMap::new(),
        };
        landuse.tiles.insert(key, LandUseTile { classes, water });
        let house = Building {
            kind: BuildingKind::House,
            outline: vec![[20.0, 70.0], [30.0, 70.0], [30.0, 80.0], [20.0, 80.0]],
            holes: vec![],
            base_y: 401.0,
            eave_y: 408.0,
            roof: Roof::Gabled { height_m: 4.0 },
            tags: BuildingTags {
                building_use: Some(BuildingUse::Chalet),
                levels: Some(2),
                roof_shape: Some(RoofShape::Hipped),
                ..Default::default()
            },
        };
        let mut buildings = BTreeMap::new();
        buildings.insert(key, vec![house]);
        WorldPackage {
            name: "inspect".into(),
            origin: (47.14, 9.52),
            tile_size_m: 100.0,
            samples: n as u32,
            tiles: BTreeMap::from([(key, tile)]),
            graph: Some(graph),
            landuse: Some(landuse),
            buildings: Some(buildings),
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: Some("Europe/Vaduz".into()),
            look: Default::default(),
            obstacles: None,
        }
    }

    fn set() -> WorldSet {
        WorldSet::new("inspect", Arc::new(test_package()))
    }

    /// The crosshair's ray from 100 m above, looking straight down at
    /// (x, z), and what it names there.
    fn pick(set: &WorldSet, x: f64, z: f64) -> Inspection {
        let surface = |x: f64, z: f64| set_solid_top(set, [x, z]);
        let hit = raycast([x, 600.0, z], [0.0, -1.0, 0.0], 1000.0, &surface).expect("hits");
        inspect(set, [hit[0], hit[2]], hit[1], Some([1.0, 0.0]))
    }

    #[test]
    fn the_ray_meets_the_terrain_and_does_not_tunnel_through_a_ridge() {
        // A wall 30 m thick and 300 m high at x = 485 … 515 over flat
        // ground, the ray 100 m above it.
        let ridge = |x: f64, _z: f64| Some(if (x - 500.0).abs() < 15.0 { 300.0 } else { 0.0 });
        let hit = raycast([0.0, 100.0, 0.0], [1.0, 0.0, 0.0], 2000.0, &ridge).expect("hits");
        assert!((hit[0] - 485.0).abs() < 0.01, "{hit:?}");
        // As thin as the longest step, anywhere: never missed.
        for k in 0..40 {
            let at = 300.0 + f64::from(k) * 1.37;
            let wall = move |x: f64, _z: f64| {
                Some(if (x - at).abs() <= MAX_STEP_M / 2.0 {
                    300.0
                } else {
                    0.0
                })
            };
            let hit = raycast([0.0, 100.0, 0.0], [1.0, 0.0, 0.0], 2000.0, &wall);
            assert!(
                hit.is_some_and(|h| (h[0] - (at - MAX_STEP_M / 2.0)).abs() < 0.01),
                "{at}"
            );
        }
        // Looking down at 30° onto flat ground at 0 from 100 m.
        let flat = |_: f64, _: f64| Some(0.0);
        let d = [30f64.to_radians().cos(), -30f64.to_radians().sin(), 0.0];
        let hit = raycast([0.0, 100.0, 0.0], d, 2000.0, &flat).expect("hits");
        assert!(
            hit[1].abs() < 0.01 && (hit[0] - 173.2).abs() < 0.2,
            "{hit:?}"
        );
        // Looking up: nothing.
        assert!(raycast([0.0, 100.0, 0.0], [1.0, 0.5, 0.0], 2000.0, &flat).is_none());
        // Outside the world, then in it.
        let island = |x: f64, _: f64| (x > 300.0).then_some(0.0);
        let hit = raycast([0.0, 50.0, 0.0], [1.0, -0.1, 0.0], 2000.0, &island).expect("hits");
        assert!((hit[0] - 500.0).abs() < 1.0, "{hit:?}");
    }

    #[test]
    fn the_crosshair_names_the_road() {
        let s = set();
        let i = pick(&s, 55.0, 51.0);
        let r = i.road.as_ref().expect("on the road");
        assert_eq!(r.name.as_deref(), Some("Landstrasse"));
        assert_eq!(r.osm_way, 4711);
        assert_eq!(r.class, RoadClass::Secondary);
        assert_eq!(r.surface, RoadSurface::Paving, "RSRF from the strip");
        assert!(r.bridge && !r.tunnel);
        assert_eq!(r.width_m, 6.0);
        let g = r.gradient.expect("gradient");
        assert!((g - 0.1).abs() < 1e-6, "profile 0.5 m per 5 m: {g}");
        // Looking the other way, it falls.
        let back = inspect(&s, [55.0, 51.0], 405.0, Some([-1.0, 0.0]));
        assert!(back.road.unwrap().gradient.unwrap() < 0.0);
        let text = i.lines().join("\n");
        assert!(
            text.contains(
                "Landstrasse · way 4711 · secondary · paving · 6.0 m wide · bridge · +10.0 %"
            ),
            "{text}"
        );
        // Off the road (5 m beyond its edge): no road.
        assert!(pick(&s, 55.0, 59.0).road.is_none());
    }

    #[test]
    fn the_crosshair_names_the_building_with_its_tags() {
        let s = set();
        let i = pick(&s, 25.0, 75.0);
        let b = i.building.as_ref().expect("on the house");
        assert_eq!(b.kind, BuildingKind::House);
        assert_eq!(b.building_use, Some(BuildingUse::Chalet));
        assert_eq!(b.levels, Some(2));
        assert_eq!(b.roof_shape, Some(RoofShape::Hipped));
        assert!((b.height_m - 11.0).abs() < 1e-6);
        // The ray stopped on the roof (the ridge), not on the ground.
        assert!((i.point[1] - 412.0).abs() < 0.01, "{:?}", i.point);
        let text = i.lines().join("\n");
        assert!(
            text.contains("Building: chalet · 2 levels · hipped roof · 11 m high"),
            "{text}"
        );
        assert!(i.road.is_none());
    }

    #[test]
    fn the_crosshair_names_the_area_the_water_and_the_elevation() {
        let s = set();
        let forest = pick(&s, 80.0, 20.0);
        assert_eq!(forest.area, Some(LandClass::Forest));
        assert!(forest.water_m.is_none() && forest.road.is_none());
        assert!((forest.elevation_m.unwrap() - 408.0).abs() < 1e-6);
        let text = forest.lines().join("\n");
        assert!(
            text.contains("Area: forest") && text.contains("Elevation 408 m"),
            "{text}"
        );
        let lake = pick(&s, 5.0, 10.0);
        assert_eq!(lake.water_m, Some(403.5));
        assert!(
            (lake.point[1] - 403.5).abs() < 0.01,
            "the ray meets the water"
        );
        assert!(lake.lines().iter().any(|l| l == "Water · level 403.5 m"));
        let meadow = pick(&s, 60.0, 90.0);
        assert_eq!(meadow.area, Some(LandClass::Meadow));
        assert_eq!(
            meadow.tile,
            Some(("inspect".into(), TileKey { x: 0, z: 0 }))
        );
        let (lat, lon) = meadow.lat_lon.unwrap();
        assert!((lat - 47.14).abs() < 0.01 && (lon - 9.52).abs() < 0.01);
        // Outside the package.
        let out = inspect(&s, [500.0, 500.0], 0.0, None);
        assert_eq!(out.lines(), vec!["Outside the world".to_string()]);
    }
}
