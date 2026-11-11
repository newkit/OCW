//! Buildings for world packages (spec §4.2 step 10): OSM footprints (ways
//! and multipolygons) with heights from `height` / `building:levels` or
//! defaults by building type, placed on the terrain (section `BLDG`).

use ocw_world::buildings::{Building, BuildingKind, Buildings, Roof, signed_area};
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

/// Wall height and roof of a footprint from its tags and size; the
/// footprint becomes its oriented rectangle when it gets a gable roof.
pub fn shape(fp: &Footprint) -> (f64, Roof, Vec<[f64; 2]>) {
    let tag = |k: &str| fp.tags.get(k).map(String::as_str);
    let kind = building_kind(&fp.tags);
    let area = signed_area(&fp.outer).abs() / 2.0;
    let rect = oriented_rectangle(&fp.outer);
    let rectangular = fp.holes.is_empty()
        && rect
            .as_ref()
            .is_some_and(|(_, a)| *a > 0.0 && area / a >= 0.85);
    let pitched_tag = tag("roof:shape").map(|s| {
        matches!(
            s,
            "gabled"
                | "hipped"
                | "half-hipped"
                | "pyramidal"
                | "gambrel"
                | "mansard"
                | "saltbox"
                | "side_hipped"
        )
    });
    let pitched_kind = match kind {
        BuildingKind::House
        | BuildingKind::Farm
        | BuildingKind::Small
        | BuildingKind::Religious => true,
        BuildingKind::Other | BuildingKind::Apartments => area < 250.0,
        _ => false,
    };
    let max_area = if kind == BuildingKind::Farm {
        1500.0
    } else {
        GABLE_MAX_AREA_M2
    };
    let gabled = rectangular && area <= max_area && pitched_tag.unwrap_or(pitched_kind);
    let levels = tag("building:levels")
        .and_then(parse_length)
        .filter(|l| *l <= 100.0);
    let height = tag("height")
        .and_then(parse_length)
        .filter(|h| (1.0..=400.0).contains(h));
    let mut wall = match (height, levels) {
        (Some(h), _) => h,
        (None, Some(l)) => (l * LEVEL_M).max(LEVEL_M),
        _ => default_wall_m(kind, area),
    };
    if gabled {
        let (corners, _) = rect.expect("rectangular has a rectangle");
        let short = ((corners[2][0] - corners[1][0]).powi(2)
            + (corners[2][1] - corners[1][1]).powi(2))
        .sqrt();
        let roof_h = tag("roof:height")
            .and_then(parse_length)
            .unwrap_or(
                (if kind == BuildingKind::Small {
                    0.3
                } else {
                    0.4
                }) * short,
            )
            .clamp(0.5, 8.0);
        if height.is_some() {
            wall = (wall - roof_h).max(2.0); // `height` includes the roof
        }
        return (
            wall.max(2.0),
            Roof::Gabled {
                height_m: roof_h as f32,
            },
            corners.to_vec(),
        );
    }
    (wall.max(2.0), Roof::Flat, fp.outer.clone())
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
    let (wall, roof, ring) = shape(&Footprint {
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
    })
}

/// What [`add_buildings`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildingStats {
    pub footprints: usize,
    pub placed: usize,
    pub gabled: usize,
    pub with_holes: usize,
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
        let h = |t: &[(&str, &str)]| shape(&fp(t, big.clone())).0;
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
            shape(&fp(&[("building", "yes")], rect(0.0, 0.0, 5.0, 4.0))).0,
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
        let (wall, roof, ring) = shape(&fp(&[("building", "house")], pts));
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
        let (wall, _, _) = shape(&fp(
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
            shape(&fp(&[("building", "house")], l.clone())).1,
            Roof::Flat
        );
        assert_eq!(
            shape(&fp(&[("building", "house")], rect(0.0, 0.0, 30.0, 30.0))).1,
            Roof::Flat
        );
        assert_eq!(
            shape(&fp(
                &[("building", "house"), ("roof:shape", "flat")],
                rect(0.0, 0.0, 10.0, 8.0)
            ))
            .1,
            Roof::Flat
        );
        // An office gets a gable roof only when tagged.
        assert_eq!(
            shape(&fp(&[("building", "office")], rect(0.0, 0.0, 10.0, 8.0))).1,
            Roof::Flat
        );
        assert!(matches!(
            shape(&fp(
                &[("building", "office"), ("roof:shape", "hipped")],
                rect(0.0, 0.0, 10.0, 8.0)
            ))
            .1,
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
            routes: Vec::new(),
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
