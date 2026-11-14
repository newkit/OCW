//! Buildings for world packages (spec §4.2 step 10): OSM footprints (ways
//! and multipolygons) with heights from `height` / `building:levels` or
//! defaults by building type, placed on the terrain (section `BLDG`).

use ocw_world::buildings::{
    Building, BuildingKind, BuildingTags, BuildingUse, Buildings, Roof, RoofMaterial, RoofShape,
    WallMaterial, signed_area,
};
use ocw_world::package::{TileKey, WorldPackage};

use crate::areas::{AreaData, inside_rings};
use crate::osm::Tags;
use crate::projection::LocalProjection;

/// Height of one storey in metres.
pub const LEVEL_M: f64 = 3.0;
/// Walls reach this far below the lowest terrain under the footprint.
pub const SINK_M: f64 = 0.5;
/// Footprints at most this large (m²) and nearly rectangular get a gable roof.
pub const GABLE_MAX_AREA_M2: f64 = 400.0;

/// A building footprint in local metres (rings open, any winding).
#[derive(Debug, Clone, PartialEq)]
pub struct Footprint {
    pub id: i64,
    pub tags: Tags,
    pub outer: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
}

/// Parses an OSM length: `12`, `12.5 m`, `3,5`, `40'` or `40 ft`.
pub fn parse_length(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let (num, factor) = if let Some(v) = s.strip_suffix("ft").or(s.strip_suffix('\'')) {
        (v.trim().to_string(), 0.3048)
    } else if let Some(v) = s.strip_suffix('m') {
        (v.trim().to_string(), 1.0)
    } else {
        (s.clone(), 1.0)
    };
    let v: f64 = num.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v * factor)
}

/// The footprints of an extract: closed ways and multipolygons tagged
/// `building` (not `building:part`, which overlaps its building); each
/// outer ring of a multipolygon becomes one footprint with the inner rings
/// inside it.
pub fn footprints(data: &AreaData, proj: &LocalProjection) -> Vec<Footprint> {
    let mut out = Vec::new();
    for a in data.areas() {
        if a.tags.get("building").is_none_or(|v| v == "no") {
            continue;
        }
        let project = |ring: &Vec<(f64, f64)>| -> Vec<[f64; 2]> {
            ring.iter()
                .map(|&(lat, lon)| proj.project(lat, lon))
                .collect()
        };
        let inner: Vec<Vec<[f64; 2]>> = a.inner.iter().map(project).collect();
        for o in &a.outer {
            let outer = project(o);
            let holes = inner
                .iter()
                .filter(|h| inside_rings(h[0], std::slice::from_ref(&outer)))
                .cloned()
                .collect();
            out.push(Footprint {
                id: a.id,
                tags: a.tags.clone(),
                outer,
                holes,
            });
        }
    }
    out
}

pub fn building_kind(tags: &Tags) -> BuildingKind {
    match tags.get("building").map(String::as_str).unwrap_or("yes") {
        "house" | "detached" | "semidetached_house" | "terrace" | "bungalow" | "residential"
        | "chalet" => BuildingKind::House,
        "apartments" | "dormitory" | "hotel" => BuildingKind::Apartments,
        "commercial" | "retail" | "office" | "supermarket" | "kiosk" => BuildingKind::Commercial,
        "industrial" | "warehouse" | "factory" | "manufacture" | "service"
        | "transformer_tower" => BuildingKind::Industrial,
        "farm" | "barn" | "farm_auxiliary" | "stable" | "cowshed" | "greenhouse" | "sty" => {
            BuildingKind::Farm
        }
        "church" | "chapel" | "cathedral" | "religious" | "monastery" | "shrine" => {
            BuildingKind::Religious
        }
        "school" | "public" | "civic" | "government" | "hospital" | "university"
        | "kindergarten" | "train_station" | "sports_hall" | "fire_station" | "college" => {
            BuildingKind::Public
        }
        "garage" | "garages" | "shed" | "hut" | "cabin" | "carport" | "roof" | "boathouse" => {
            BuildingKind::Small
        }
        _ => BuildingKind::Other,
    }
}

/// Default wall height in metres when OSM gives neither height nor levels.
pub fn default_wall_m(kind: BuildingKind, area_m2: f64) -> f64 {
    match kind {
        BuildingKind::House => 6.0,
        BuildingKind::Apartments => 12.0,
        BuildingKind::Commercial | BuildingKind::Public => 9.0,
        BuildingKind::Industrial => 8.0,
        BuildingKind::Farm => 6.0,
        BuildingKind::Religious => 12.0,
        BuildingKind::Small => 3.0,
        BuildingKind::Other if area_m2 < 40.0 => 3.0,
        BuildingKind::Other => 6.0,
    }
}

/// Minimum-area oriented rectangle around the points (tried along every
/// edge direction): corners counter-clockwise in (x, z), the first edge
/// along the longer side, and its area.
pub fn oriented_rectangle(pts: &[[f64; 2]]) -> Option<([[f64; 2]; 4], f64)> {
    let n = pts.len();
    let mut best: Option<([[f64; 2]; 4], f64)> = None;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if len < 1e-6 {
            continue;
        }
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let v = [-u[1], u[0]];
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in pts {
            let (s, t) = (p[0] * u[0] + p[1] * u[1], p[0] * v[0] + p[1] * v[1]);
            lo = [lo[0].min(s), lo[1].min(t)];
            hi = [hi[0].max(s), hi[1].max(t)];
        }
        let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
        if best.is_none_or(|(_, a)| area < a - 1e-9) {
            let at = |s: f64, t: f64| [s * u[0] + t * v[0], s * u[1] + t * v[1]];
            let mut c = [
                at(lo[0], lo[1]),
                at(hi[0], lo[1]),
                at(hi[0], hi[1]),
                at(lo[0], hi[1]),
            ];
            if hi[1] - lo[1] > hi[0] - lo[0] {
                c.rotate_left(1); // first edge along the longer side
            }
            best = Some((c, area));
        }
    }
    best
}

/// Parses an OSM `roof:direction`: degrees clockwise from north (`135`)
/// or a compass point (`SE`, `NNW`).
pub fn parse_direction(s: &str) -> Option<f64> {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let s = s.trim();
    if let Some(i) = POINTS.iter().position(|p| p.eq_ignore_ascii_case(s)) {
        return Some(i as f64 * 22.5);
    }
    let v: f64 = s.trim_end_matches('°').trim().parse().ok()?;
    v.is_finite().then(|| v.rem_euclid(360.0))
}

/// The horizontal unit vector of a compass direction in local (x east,
/// z south) metres.
fn compass(deg: f64) -> [f64; 2] {
    let r = deg.to_radians();
    [r.sin(), -r.cos()]
}

/// Area of the convex hull of the points (monotone chain).
pub fn hull_area(pts: &[[f64; 2]]) -> f64 {
    let mut p: Vec<[f64; 2]> = pts.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return 0.0;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f64; 2]> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0
            {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    signed_area(&hull).abs() / 2.0
}

/// Height of a storey inside the roof (`roof:levels`), metres.
pub const ROOF_LEVEL_M: f64 = 2.5;
/// Pyramids on polygons (towers) up to this footprint (m²).
pub const PYRAMID_MAX_AREA_M2: f64 = 600.0;

/// A footprint shaped from its tags: what [`place`] puts on the terrain.
#[derive(Debug, Clone, PartialEq)]
pub struct Shaped {
    /// Wall height above the ground (to the eaves or the flat roof).
    pub wall: f64,
    pub roof: Roof,
    /// The footprint: its oriented rectangle for pitched roofs, ridge
    /// along the first edge (a skillion: low eave along the first edge).
    pub ring: Vec<[f64; 2]>,
    /// The OSM facts for the style (section `BTAG`).
    pub tags: BuildingTags,
}

/// The OSM facts of a footprint that the styles use (spec §14 Phase 8 T4);
/// the roof shape and height are filled in by [`shape`].
pub fn osm_facts(tags: &Tags) -> BuildingTags {
    let tag = |k: &str| tags.get(k).map(String::as_str);
    let count = |k: &str, max: f64| {
        tag(k)
            .and_then(parse_length)
            .filter(|l| *l <= max)
            .map(|l| l.round() as u8)
    };
    BuildingTags {
        building_use: tag("building").and_then(BuildingUse::from_osm),
        levels: count("building:levels", 100.0).filter(|l| *l > 0),
        roof_levels: count("roof:levels", 20.0),
        wall: tag("building:material")
            .or(tag("building:facade:material"))
            .and_then(WallMaterial::from_osm),
        wall_colour: tag("building:colour")
            .or(tag("building:facade:colour"))
            .and_then(crate::calibrate::parse_colour),
        roof_material: tag("roof:material").and_then(RoofMaterial::from_osm),
        roof_colour: tag("roof:colour").and_then(crate::calibrate::parse_colour),
        ..Default::default()
    }
}

/// Wall height, roof and OSM facts of a footprint from its tags and size.
/// Pitched roofs (tagged `roof:shape`, else by building type and size) sit
/// on the footprint's oriented rectangle when it is nearly rectangular; the
/// ridge follows the long side unless `roof:orientation=across` or a
/// `roof:direction` says otherwise. The roof height comes from
/// `roof:height`, `roof:angle` or `roof:levels`, else 0.4 × the span (0.3
/// for small buildings). A tagged pyramid also sits on a convex polygon
/// (towers). Everything else gets a flat roof on its own outline.
pub fn shape(fp: &Footprint) -> Shaped {
    let tag = |k: &str| fp.tags.get(k).map(String::as_str);
    let kind = building_kind(&fp.tags);
    let mut facts = osm_facts(&fp.tags);
    let area = signed_area(&fp.outer).abs() / 2.0;
    let rect = oriented_rectangle(&fp.outer);
    let rectangular = fp.holes.is_empty()
        && rect
            .as_ref()
            .is_some_and(|(_, a)| *a > 0.0 && area / a >= 0.85);
    let shape_tag = tag("roof:shape").and_then(RoofShape::from_osm);
    let canopy = facts.building_use == Some(BuildingUse::Canopy);
    let pitched_kind = !canopy
        && match kind {
            BuildingKind::House
            | BuildingKind::Farm
            | BuildingKind::Small
            | BuildingKind::Religious => true,
            BuildingKind::Other | BuildingKind::Apartments => area < 250.0,
            _ => false,
        };
    let max_area = match (shape_tag, kind) {
        (Some(_), _) => 3000.0,
        (None, BuildingKind::Farm | BuildingKind::Religious) => 1500.0,
        _ => GABLE_MAX_AREA_M2,
    };
    let pitched =
        rectangular && area <= max_area && shape_tag.map_or(pitched_kind, RoofShape::is_pitched);
    let levels = tag("building:levels")
        .and_then(parse_length)
        .filter(|l| *l <= 100.0);
    let height = tag("height")
        .and_then(parse_length)
        .filter(|h| (1.0..=400.0).contains(h));
    let min_height = tag("min_height")
        .and_then(parse_length)
        .or_else(|| {
            tag("building:min_level")
                .and_then(parse_length)
                .map(|l| l * LEVEL_M)
        })
        .filter(|h| *h > 0.0 && *h <= 200.0);
    facts.min_height_m = min_height.map(|h| h as f32);
    let mut wall = match (height, levels) {
        (Some(h), _) => h,
        (None, Some(l)) => (l * LEVEL_M).max(LEVEL_M),
        _ => default_wall_m(kind, area),
    };
    // A tagged roof height: roof:height, else roof:angle over the span,
    // else the roof's storeys.
    let tagged_roof = |span: f64| -> Option<f64> {
        tag("roof:height")
            .and_then(parse_length)
            .or_else(|| {
                let a: f64 = tag("roof:angle")?
                    .trim()
                    .trim_end_matches('°')
                    .parse()
                    .ok()?;
                (a.is_finite() && (3.0..=75.0).contains(&a)).then(|| a.to_radians().tan() * span)
            })
            .or_else(|| {
                let l = facts.roof_levels.filter(|l| *l > 0)?;
                Some(f64::from(l) * ROOF_LEVEL_M + 0.5)
            })
            .map(|h| h.clamp(0.3, 25.0))
    };
    if pitched {
        let (mut corners, _) = rect.expect("rectangular has a rectangle");
        let shape = shape_tag.unwrap_or(RoofShape::Gabled);
        let edge = |c: &[[f64; 2]; 4], i: usize| {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
            [(b[0] - a[0]) / len, (b[1] - a[1]) / len]
        };
        let dir = tag("roof:direction").and_then(parse_direction).map(compass);
        let turn = match (shape, dir) {
            // Skillion: the low eave on the side the roof faces (outward
            // normal of a counter-clockwise edge (dx, dz) is (dz, −dx)).
            (RoofShape::Skillion, Some(d)) => (0..4)
                .max_by(|&i, &j| {
                    let n = |k: usize| {
                        let e = edge(&corners, k);
                        e[1] * d[0] - e[0] * d[1]
                    };
                    n(i).total_cmp(&n(j))
                })
                .unwrap_or(0),
            // The ridge across the direction the slopes face.
            (_, Some(d)) => {
                let along = |k: usize| {
                    let e = edge(&corners, k);
                    (e[0] * d[0] + e[1] * d[1]).abs()
                };
                usize::from(along(1) < along(0))
            }
            (_, None) => usize::from(tag("roof:orientation") == Some("across")),
        };
        corners.rotate_left(turn);
        let span = (corners[2][0] - corners[1][0]).hypot(corners[2][1] - corners[1][1]);
        let half = if shape == RoofShape::Skillion {
            span
        } else {
            span / 2.0
        };
        let tagged = tagged_roof(half);
        let small = kind == BuildingKind::Small;
        let roof_h =
            tagged.unwrap_or_else(|| (if small { 0.3 } else { 0.4 } * span).clamp(0.5, 8.0));
        if height.is_some() {
            wall = (wall - roof_h).max(2.0); // `height` includes the roof
        }
        facts.roof_shape = shape_tag;
        facts.roof_height_m = tagged.map(|h| h as f32);
        return Shaped {
            wall: wall.max(2.0),
            roof: Roof::Gabled {
                height_m: roof_h as f32,
            },
            ring: corners.to_vec(),
            tags: facts,
        };
    }
    // A pyramid (spire, tower) on a convex polygon.
    if shape_tag == Some(RoofShape::Pyramidal)
        && fp.holes.is_empty()
        && area <= PYRAMID_MAX_AREA_M2
        && hull_area(&fp.outer) > 0.0
        && area / hull_area(&fp.outer) >= 0.95
    {
        let diameter = 2.0 * (area / std::f64::consts::PI).sqrt();
        let roof_h = tagged_roof(diameter / 2.0).unwrap_or((0.8 * diameter).clamp(1.0, 25.0));
        if height.is_some() {
            wall = (wall - roof_h).max(2.0);
        }
        facts.roof_shape = Some(RoofShape::Pyramidal);
        facts.roof_height_m = Some(roof_h as f32);
        return Shaped {
            wall: wall.max(2.0),
            roof: Roof::Flat,
            ring: fp.outer.clone(),
            tags: facts,
        };
    }
    // Explicitly flat stays flat for the style too.
    facts.roof_shape = shape_tag.filter(|s| *s == RoofShape::Flat);
    Shaped {
        wall: wall.max(2.0),
        roof: Roof::Flat,
        ring: fp.outer.clone(),
        tags: facts,
    }
}

/// Drops repeated and collinear points (within 5 cm) of a ring.
pub fn simplify(ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(ring.len());
    for p in ring {
        if pts
            .last()
            .is_none_or(|q| (q[0] - p[0]).hypot(q[1] - p[1]) > 0.05)
        {
            pts.push(*p);
        }
    }
    while pts.len() > 1
        && (pts[0][0] - pts[pts.len() - 1][0]).hypot(pts[0][1] - pts[pts.len() - 1][1]) <= 0.05
    {
        pts.pop();
    }
    let mut changed = true;
    while changed && pts.len() > 3 {
        changed = false;
        for i in 0..pts.len() {
            let n = pts.len();
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let ac = (c[0] - a[0]).hypot(c[1] - a[1]);
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if ac > 0.0 && (cross / ac).abs() < 0.05 {
                pts.remove(i);
                changed = true;
                break;
            }
        }
    }
    pts
}

/// Places a footprint on the terrain: the walls start [`SINK_M`] below the
/// lowest terrain under it (vertices and a 4 m grid inside) and reach the
/// wall height above the mean terrain, at least 2 m above the highest.
pub fn place(fp: &Footprint, ground: impl Fn(f64, f64) -> Option<f64>) -> Option<Building> {
    let outer = simplify(&fp.outer);
    if outer.len() < 3 || signed_area(&outer).abs() < 2.0 {
        return None; // degenerate (< 1 m²)
    }
    let holes: Vec<Vec<[f64; 2]>> = fp
        .holes
        .iter()
        .map(|h| simplify(h))
        .filter(|h| h.len() >= 3)
        .collect();
    let Shaped {
        wall,
        roof,
        ring,
        tags,
    } = shape(&Footprint {
        outer: outer.clone(),
        holes: holes.clone(),
        ..fp.clone()
    });
    let mut samples: Vec<f64> = Vec::new();
    for p in &ring {
        samples.push(ground(p[0], p[1])?);
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &ring {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let rings = vec![ring.clone()];
    let mut z = lo[1] + 2.0;
    while z < hi[1] {
        let mut x = lo[0] + 2.0;
        while x < hi[0] {
            if inside_rings([x, z], &rings)
                && let Some(h) = ground(x, z)
            {
                samples.push(h);
            }
            x += 4.0;
        }
        z += 4.0;
    }
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let max = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let ccw = |r: &[[f64; 2]], want_ccw: bool| -> Vec<[f32; 2]> {
        let mut v: Vec<[f32; 2]> = r.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
        if (signed_area(r) > 0.0) != want_ccw {
            v.reverse();
        }
        v
    };
    let outline = match roof {
        // Already counter-clockwise with the ridge edge first.
        Roof::Gabled { .. } => ring.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(),
        Roof::Flat => ccw(ring.as_slice(), true),
    };
    let holes = match roof {
        Roof::Gabled { .. } => Vec::new(),
        Roof::Flat => holes.iter().map(|h| ccw(h.as_slice(), false)).collect(),
    };
    Some(Building {
        kind: building_kind(&fp.tags),
        outline,
        holes,
        base_y: (min - SINK_M) as f32,
        eave_y: (mean + wall).max(max + 2.0) as f32,
        roof,
        tags,
    })
}

/// What [`add_buildings`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildingStats {
    pub footprints: usize,
    pub placed: usize,
    pub gabled: usize,
    pub with_holes: usize,
    /// Buildings with OSM facts for the style (section `BTAG`).
    pub tagged: usize,
    /// Of these, with a tagged roof shape (pyramids on polygons included).
    pub shaped: usize,
    pub outside: usize,
}

/// Places the footprints on the package terrain and adds them (section
/// `BLDG`) to the tile of their centroid.
pub fn add_buildings(pkg: &mut WorldPackage, fps: &[Footprint]) -> BuildingStats {
    let size = f64::from(pkg.tile_size_m);
    let mut tiles = Buildings::new();
    let mut stats = BuildingStats {
        footprints: fps.len(),
        ..Default::default()
    };
    for fp in fps {
        let n = fp.outer.len().max(1) as f64;
        let c = [
            fp.outer.iter().map(|p| p[0]).sum::<f64>() / n,
            fp.outer.iter().map(|p| p[1]).sum::<f64>() / n,
        ];
        let key = TileKey::containing(c, size);
        if !pkg.tiles.contains_key(&key) {
            stats.outside += 1;
            continue;
        }
        let Some(b) = place(fp, |x, z| pkg.surface_height(x, z)) else {
            stats.outside += 1;
            continue;
        };
        stats.placed += 1;
        stats.gabled += usize::from(matches!(b.roof, Roof::Gabled { .. }));
        stats.with_holes += usize::from(!b.holes.is_empty());
        stats.tagged += usize::from(!b.tags.is_empty());
        stats.shaped += usize::from(b.tags.roof_shape.is_some());
        tiles.entry(key).or_default().push(b);
    }
    pkg.buildings = Some(tiles);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn fp(t: &[(&str, &str)], outer: Vec<[f64; 2]>) -> Footprint {
        Footprint {
            id: 1,
            tags: tags(t),
            outer,
            holes: vec![],
        }
    }

    fn rect(x0: f64, z0: f64, w: f64, d: f64) -> Vec<[f64; 2]> {
        vec![[x0, z0], [x0 + w, z0], [x0 + w, z0 + d], [x0, z0 + d]]
    }

    #[test]
    fn lengths_are_parsed() {
        assert_eq!(parse_length("12"), Some(12.0));
        assert_eq!(parse_length("12.5 m"), Some(12.5));
        assert_eq!(parse_length("3,5"), Some(3.5));
        assert_eq!(parse_length("7m"), Some(7.0));
        assert!((parse_length("40'").unwrap() - 12.192).abs() < 1e-9);
        assert!((parse_length("10 ft").unwrap() - 3.048).abs() < 1e-9);
        assert_eq!(parse_length("tall"), None);
        assert_eq!(parse_length("-3"), None);
    }

    #[test]
    fn heights_follow_tags_then_levels_then_type() {
        let big = rect(0.0, 0.0, 30.0, 30.0); // 900 m², flat roofs
        let h = |t: &[(&str, &str)]| shape(&fp(t, big.clone())).wall;
        assert_eq!(h(&[("building", "yes"), ("height", "17.5")]), 17.5);
        assert_eq!(h(&[("building", "yes"), ("building:levels", "4")]), 12.0);
        assert_eq!(
            h(&[
                ("building", "yes"),
                ("height", "20"),
                ("building:levels", "4")
            ]),
            20.0
        );
        assert_eq!(h(&[("building", "apartments")]), 12.0);
        assert_eq!(h(&[("building", "industrial")]), 8.0);
        assert_eq!(h(&[("building", "yes"), ("height", "absurd")]), 6.0);
        // Small sheds and unknown tiny buildings are low.
        assert_eq!(
            shape(&fp(&[("building", "yes")], rect(0.0, 0.0, 5.0, 4.0))).wall,
            3.0
        );
        assert_eq!(
            building_kind(&tags(&[("building", "detached")])),
            BuildingKind::House
        );
        assert_eq!(
            building_kind(&tags(&[("building", "church")])),
            BuildingKind::Religious
        );
        assert_eq!(
            building_kind(&tags(&[("building", "garage")])),
            BuildingKind::Small
        );
        assert_eq!(
            building_kind(&tags(&[("building", "yes")])),
            BuildingKind::Other
        );
    }

    #[test]
    fn small_rectangular_houses_get_gable_roofs() {
        // 12 × 8 m house, slightly irregular (5 points), rotated by 30°.
        let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
        let rot = |p: [f64; 2]| [100.0 + p[0] * c - p[1] * s, 50.0 + p[0] * s + p[1] * c];
        let pts: Vec<[f64; 2]> = [[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [6.0, 8.2], [0.0, 8.0]]
            .into_iter()
            .map(rot)
            .collect();
        let Shaped {
            wall, roof, ring, ..
        } = shape(&fp(&[("building", "house")], pts));
        assert_eq!(wall, 6.0);
        let Roof::Gabled { height_m } = roof else {
            panic!("{roof:?}")
        };
        assert!((f64::from(height_m) - 0.4 * 8.2).abs() < 0.05, "{height_m}");
        assert_eq!(ring.len(), 4);
        assert!(signed_area(&ring) > 0.0);
        // First edge along the long side (≈ 12 m), the next ≈ 8.2 m.
        let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
        assert!((len(ring[0], ring[1]) - 12.0).abs() < 0.05);
        assert!((len(ring[1], ring[2]) - 8.2).abs() < 0.05);
        // `height` includes the roof.
        let Shaped { wall, .. } = shape(&fp(
            &[("building", "house"), ("height", "9")],
            rect(0.0, 0.0, 10.0, 10.0),
        ));
        assert!((wall - 5.0).abs() < 1e-9);
        // Big, L-shaped or explicitly flat buildings keep their outline.
        let l = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        assert_eq!(
            shape(&fp(&[("building", "house")], l.clone())).roof,
            Roof::Flat
        );
        assert_eq!(
            shape(&fp(&[("building", "house")], rect(0.0, 0.0, 30.0, 30.0))).roof,
            Roof::Flat
        );
        assert_eq!(
            shape(&fp(
                &[("building", "house"), ("roof:shape", "flat")],
                rect(0.0, 0.0, 10.0, 8.0)
            ))
            .roof,
            Roof::Flat
        );
        // An office gets a gable roof only when tagged.
        assert_eq!(
            shape(&fp(&[("building", "office")], rect(0.0, 0.0, 10.0, 8.0))).roof,
            Roof::Flat
        );
        assert!(matches!(
            shape(&fp(
                &[("building", "office"), ("roof:shape", "hipped")],
                rect(0.0, 0.0, 10.0, 8.0)
            ))
            .roof,
            Roof::Gabled { .. }
        ));
    }

    #[test]
    fn footprints_are_placed_on_the_lowest_ground() {
        // Ground rising 10 % towards +x.
        let ground = |x: f64, _z: f64| Some(400.0 + 0.1 * x);
        let b = place(
            &fp(&[("building", "industrial")], rect(100.0, 0.0, 40.0, 20.0)),
            ground,
        )
        .unwrap();
        assert!((f64::from(b.base_y) - (410.0 - SINK_M)).abs() < 1e-3);
        // Wall height above the mean ground (412), at least 2 m above the top (414).
        assert!((f64::from(b.eave_y) - 420.0).abs() < 1e-3, "{}", b.eave_y);
        assert_eq!(b.roof, Roof::Flat);
        // Clockwise input comes out counter-clockwise.
        let mut cw = rect(0.0, 0.0, 40.0, 20.0);
        cw.reverse();
        let b = place(&fp(&[("building", "industrial")], cw), ground).unwrap();
        assert!(signed_area(&b.outline) > 0.0);
        // Outside the terrain or degenerate: not placed.
        assert!(
            place(
                &fp(&[("building", "yes")], rect(0.0, 0.0, 10.0, 10.0)),
                |_, _| None
            )
            .is_none()
        );
        assert!(
            place(
                &fp(
                    &[("building", "yes")],
                    vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]
                ),
                ground
            )
            .is_none()
        );
        // Holes (a courtyard) stay, wound clockwise.
        let mut court = fp(&[("building", "apartments")], rect(0.0, 0.0, 30.0, 30.0));
        court.holes = vec![rect(10.0, 10.0, 10.0, 10.0)];
        let b = place(&court, ground).unwrap();
        assert_eq!(b.holes.len(), 1);
        assert!(signed_area(&b.holes[0]) < 0.0);
        // The lowest ground is found inside the footprint too (a pit in the middle).
        let pit = |x: f64, z: f64| {
            Some(if (x - 15.0).abs() < 3.0 && (z - 15.0).abs() < 3.0 {
                390.0
            } else {
                400.0
            })
        };
        let b = place(
            &fp(&[("building", "industrial")], rect(0.0, 0.0, 30.0, 30.0)),
            pit,
        )
        .unwrap();
        assert!((f64::from(b.base_y) - (390.0 - SINK_M)).abs() < 1e-3);
    }

    #[test]
    fn simplify_drops_duplicates_and_collinear_points() {
        let r = vec![
            [0.0, 0.0],
            [0.0, 0.0],
            [5.0, 0.01],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ];
        assert_eq!(
            simplify(&r),
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
        );
    }

    #[test]
    fn directions_and_hulls() {
        assert_eq!(parse_direction("N"), Some(0.0));
        assert_eq!(parse_direction("sse"), Some(157.5));
        assert_eq!(parse_direction("270"), Some(270.0));
        assert_eq!(parse_direction("-90"), Some(270.0));
        assert_eq!(parse_direction("45°"), Some(45.0));
        assert_eq!(parse_direction("up"), None);
        let n = compass(0.0);
        assert!(
            n[0].abs() < 1e-12 && (n[1] + 1.0).abs() < 1e-12,
            "north is −z"
        );
        let e = compass(90.0);
        assert!(
            (e[0] - 1.0).abs() < 1e-12 && e[1].abs() < 1e-12,
            "east is +x"
        );
        let l = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        assert!((hull_area(&l) - 87.5).abs() < 1e-9, "{}", hull_area(&l));
        assert!((hull_area(&rect(0.0, 0.0, 4.0, 3.0)) - 12.0).abs() < 1e-9);
        assert_eq!(hull_area(&[[0.0, 0.0], [1.0, 1.0]]), 0.0);
    }

    /// Roof shapes, ridge direction and roof height from the tags (spec
    /// §14 Phase 8 T4).
    #[test]
    fn roof_tags_set_shape_direction_and_height() {
        let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
        let house = |extra: &[(&str, &str)]| {
            let mut t = vec![("building", "house")];
            t.extend_from_slice(extra);
            shape(&fp(&t, rect(0.0, 0.0, 12.0, 8.0)))
        };
        let gable_h = |s: &Shaped| match s.roof {
            Roof::Gabled { height_m } => f64::from(height_m),
            Roof::Flat => panic!("flat"),
        };
        // Untagged: a gable along the long side, no shape tag stored.
        let s = house(&[]);
        assert_eq!(s.tags.roof_shape, None);
        assert_eq!(s.tags.roof_height_m, None);
        assert!((len(s.ring[0], s.ring[1]) - 12.0).abs() < 1e-9);
        assert!((gable_h(&s) - 3.2).abs() < 1e-6);
        // A hipped roof keeps the BLDG gable (old readers) and stores the shape.
        let s = house(&[("roof:shape", "hipped")]);
        assert_eq!(s.tags.roof_shape, Some(RoofShape::Hipped));
        assert!(matches!(s.roof, Roof::Gabled { .. }));
        // Across: the ridge along the short side, the default height over the long span.
        let s = house(&[("roof:orientation", "across")]);
        assert!((len(s.ring[0], s.ring[1]) - 8.0).abs() < 1e-9);
        assert!((gable_h(&s) - 4.8).abs() < 1e-6);
        // roof:direction: the slopes face north → the ridge runs east-west (along x).
        let s = house(&[("roof:direction", "N")]);
        assert!((s.ring[1][1] - s.ring[0][1]).abs() < 1e-9, "{:?}", s.ring);
        let s = house(&[("roof:direction", "90")]);
        assert!((s.ring[1][0] - s.ring[0][0]).abs() < 1e-9, "{:?}", s.ring);
        // Skillion facing south: the low eave (first edge) is the south (+z) side.
        let s = house(&[("roof:shape", "skillion"), ("roof:direction", "S")]);
        assert_eq!(s.tags.roof_shape, Some(RoofShape::Skillion));
        assert!(s.ring[0][1] > 7.9 && s.ring[1][1] > 7.9, "{:?}", s.ring);
        assert!(signed_area(&s.ring) > 0.0, "still counter-clockwise");
        // roof:angle over the half span; roof:levels; roof:height.
        let s = house(&[("roof:angle", "45")]);
        assert!((gable_h(&s) - 4.0).abs() < 1e-6);
        assert_eq!(s.tags.roof_height_m, Some(4.0));
        let s = house(&[("roof:levels", "1")]);
        assert!((gable_h(&s) - 3.0).abs() < 1e-6);
        assert_eq!(s.tags.roof_levels, Some(1));
        let s = house(&[("roof:height", "4"), ("height", "10")]);
        assert!((s.wall - 6.0).abs() < 1e-9);
        assert!((gable_h(&s) - 4.0).abs() < 1e-6);
        // A skillion's angle rises over the whole span.
        let s = house(&[("roof:shape", "skillion"), ("roof:angle", "10")]);
        assert!((gable_h(&s) - 8.0 * 10f64.to_radians().tan()).abs() < 1e-5);
        // A tagged pitched roof on a big rectangle (a church nave, 40 × 20 m).
        let nave = shape(&fp(
            &[("building", "church"), ("roof:shape", "gabled")],
            rect(0.0, 0.0, 40.0, 20.0),
        ));
        assert!(matches!(nave.roof, Roof::Gabled { .. }));
        assert_eq!(nave.tags.building_use, Some(BuildingUse::Church));
    }

    #[test]
    fn pyramids_sit_on_convex_towers_only() {
        let octagon: Vec<[f64; 2]> = (0..8)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::FRAC_PI_4;
                [50.0 + 3.0 * a.cos(), 50.0 + 3.0 * a.sin()]
            })
            .collect();
        let s = shape(&fp(
            &[("building", "yes"), ("roof:shape", "pyramidal")],
            octagon.clone(),
        ));
        assert_eq!(s.roof, Roof::Flat, "old readers: flat");
        assert_eq!(s.tags.roof_shape, Some(RoofShape::Pyramidal));
        let area = signed_area(&octagon).abs() / 2.0;
        let d = 2.0 * (area / std::f64::consts::PI).sqrt();
        assert!((f64::from(s.tags.roof_height_m.unwrap()) - 0.8 * d).abs() < 1e-4);
        assert_eq!(s.ring.len(), 8);
        // A spire with its height tagged.
        let s = shape(&fp(
            &[
                ("building", "church"),
                ("roof:shape", "cone"),
                ("roof:height", "18"),
            ],
            octagon,
        ));
        assert_eq!(s.tags.roof_height_m, Some(18.0));
        // A concave footprint stays flat without a shape.
        let l = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        let s = shape(&fp(&[("building", "yes"), ("roof:shape", "pyramidal")], l));
        assert_eq!((s.roof, s.tags.roof_shape), (Roof::Flat, None));
    }

    #[test]
    fn osm_facts_are_kept_for_the_style() {
        let t = tags(&[
            ("building", "barn"),
            ("building:levels", "2"),
            ("roof:levels", "1"),
            ("building:material", "wood"),
            ("building:colour", "white"),
            ("roof:material", "slate"),
            ("roof:colour", "#a00"),
            ("min_height", "3"),
        ]);
        let f = osm_facts(&t);
        assert_eq!(f.building_use, Some(BuildingUse::Barn));
        assert_eq!(f.levels, Some(2));
        assert_eq!(f.roof_levels, Some(1));
        assert_eq!(f.wall, Some(WallMaterial::Timber));
        assert_eq!(f.wall_colour, Some([255, 255, 255]));
        assert_eq!(f.roof_material, Some(RoofMaterial::Slate));
        assert_eq!(f.roof_colour, Some([170, 0, 0]));
        let s = shape(&Footprint {
            id: 1,
            tags: t,
            outer: rect(0.0, 0.0, 20.0, 12.0),
            holes: vec![],
        });
        assert_eq!(s.tags.min_height_m, Some(3.0));
        assert!(osm_facts(&tags(&[("building", "yes")])).is_empty());
        assert_eq!(
            osm_facts(&tags(&[("building", "house"), ("building:levels", "0")])).levels,
            None
        );
        // Canopies (building=roof) are flat slabs unless tagged otherwise.
        let s = shape(&fp(&[("building", "roof")], rect(0.0, 0.0, 10.0, 6.0)));
        assert_eq!(s.roof, Roof::Flat);
        assert_eq!(s.tags.building_use, Some(BuildingUse::Canopy));
        // The facts travel with the placed building.
        let b = place(
            &fp(
                &[("building", "house"), ("roof:shape", "half-hipped")],
                rect(0.0, 0.0, 12.0, 9.0),
            ),
            |_, _| Some(500.0),
        )
        .unwrap();
        assert_eq!(b.tags.roof_shape, Some(RoofShape::HalfHipped));
        assert_eq!(
            BuildingTags::default(),
            place(
                &fp(&[("building", "yes")], rect(0.0, 0.0, 12.0, 9.0)),
                |_, _| Some(500.0),
            )
            .unwrap()
            .tags
        );
    }

    #[test]
    fn buildings_go_to_the_tile_of_their_centroid() {
        use ocw_world::package::Tile;
        let mut tiles = BTreeMap::new();
        for x in 0..2 {
            let key = TileKey { x, z: 0 };
            tiles.insert(
                key,
                Tile {
                    key,
                    heights: vec![500.0; 4],
                    roads: vec![],
                },
            );
        }
        let mut p = WorldPackage {
            name: "b".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: 2,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        };
        let stats = add_buildings(
            &mut p,
            &[
                fp(&[("building", "house")], rect(90.0, 10.0, 16.0, 10.0)), // centroid x 98
                fp(&[("building", "house")], rect(120.0, 10.0, 10.0, 8.0)),
                fp(&[("building", "house")], rect(500.0, 10.0, 10.0, 8.0)), // no tile
            ],
        );
        assert_eq!((stats.placed, stats.outside, stats.gabled), (2, 1, 2));
        let b = p.buildings.as_ref().unwrap();
        assert_eq!(b[&TileKey { x: 0, z: 0 }].len(), 1);
        assert_eq!(b[&TileKey { x: 1, z: 0 }].len(), 1);
        assert_eq!(WorldPackage::from_bytes(&p.to_bytes()).unwrap(), p);
    }
}
