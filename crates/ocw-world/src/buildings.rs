//! Buildings (spec §4.1, §4.7): OSM footprints extruded from the terrain,
//! stored per tile in the package section `BLDG`, and the triangulation of
//! their roofs (ear clipping with holes).
//!
//! The OSM facts the houses are styled by (spec §14 Phase 8 T4) – roof
//! shape and height, levels, wall and roof material and colour, the finer
//! building use – are the optional section `BTAG` ([`BuildingTags`], read
//! into [`Building::tags`]); readers that do not know it see the plain
//! `BLDG` buildings (flat or gable roofs).
//!
//! Footprints are in local metres (x, z). The outer ring winds
//! counter-clockwise in the (x, z) plane (positive [`signed_area`]), holes
//! clockwise; rings are open (the first point is not repeated). With this
//! convention the outward normal of an edge `a → b` is `(b.z − a.z, a.x − b.x)`
//! for the outer ring and the holes alike.

use std::collections::BTreeMap;

use crate::package::{Bytes, TileKey};

/// Section tag of the buildings.
pub const BUILDINGS_TAG: &[u8; 4] = b"BLDG";
/// Layout version inside the buildings section.
const BUILDINGS_VERSION: u32 = 1;

/// What a building is used for (its colours). Codes are stored in
/// packages: only add new kinds at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum BuildingKind {
    #[default]
    Other = 0,
    House = 1,
    Apartments = 2,
    Commercial = 3,
    Industrial = 4,
    Farm = 5,
    Religious = 6,
    Public = 7,
    /// Garages, sheds, huts, carports.
    Small = 8,
}

impl BuildingKind {
    pub const ALL: [BuildingKind; 9] = [
        BuildingKind::Other,
        BuildingKind::House,
        BuildingKind::Apartments,
        BuildingKind::Commercial,
        BuildingKind::Industrial,
        BuildingKind::Farm,
        BuildingKind::Religious,
        BuildingKind::Public,
        BuildingKind::Small,
    ];

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

/// Roof shape in `BLDG`: what every reader draws. The finer shape
/// (hipped, pyramidal, …) is [`BuildingTags::roof_shape`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Roof {
    Flat,
    /// A gable roof over a rectangular footprint (exactly four points):
    /// the ridge runs parallel to the first edge, `height_m` above the eaves.
    Gabled {
        height_m: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Building {
    pub kind: BuildingKind,
    /// Outer ring, counter-clockwise in (x, z).
    pub outline: Vec<[f32; 2]>,
    /// Inner rings (courtyards), clockwise.
    pub holes: Vec<Vec<[f32; 2]>>,
    /// Foot of the walls: a little below the lowest terrain under the footprint.
    pub base_y: f32,
    /// Top of the walls (the eaves, or the flat roof).
    pub eave_y: f32,
    pub roof: Roof,
    /// OSM facts for the style (section `BTAG`; default: none).
    pub tags: BuildingTags,
}

/// Buildings of a package by tile (a building belongs to the tile of its
/// first outline point's centroid, see `world-builder`).
pub type Buildings = BTreeMap<TileKey, Vec<Building>>;

/// Section tag of the buildings' OSM facts ([`BuildingTags`]).
pub const BUILDING_TAGS_TAG: &[u8; 4] = b"BTAG";
/// Layout version inside the `BTAG` section.
const BUILDING_TAGS_VERSION: u32 = 1;

/// Defines a small enum stored as a byte in `BTAG` (codes are stored in
/// packages: only add new values at the end) with its style-file names.
macro_rules! coded_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $code:literal => $label:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum $name {
            $($(#[$vm])* $v = $code),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            pub fn from_code(code: u8) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| *v as u8 == code)
            }

            /// The name used in `assets/building-styles.toml`.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$v => $label),+
                }
            }

            pub fn from_name(s: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.name() == s)
            }
        }
    };
}

coded_enum! {
    /// Roof shape (OSM `roof:shape`). Everything but `Flat` needs the
    /// rectangular `BLDG` footprint (`Roof::Gabled`), except a pyramid,
    /// which also sits on a convex polygon (towers).
    RoofShape {
        Flat = 0 => "flat",
        Gabled = 1 => "gabled",
        Hipped = 2 => "hipped",
        HalfHipped = 3 => "half-hipped",
        Pyramidal = 4 => "pyramidal",
        Skillion = 5 => "skillion",
        Mansard = 6 => "mansard",
        Gambrel = 7 => "gambrel",
    }
}

impl RoofShape {
    /// The shape of an OSM `roof:shape` value; similar shapes map to the
    /// nearest one drawn (saltbox and round → gabled, side-hipped →
    /// half-hipped, cone / dome / onion → pyramidal).
    pub fn from_osm(v: &str) -> Option<Self> {
        Some(match v.trim() {
            "flat" => RoofShape::Flat,
            "gabled" | "saltbox" | "round" | "gabled_height_moved" => RoofShape::Gabled,
            "hipped" | "hipped-and-gabled" => RoofShape::Hipped,
            "half-hipped" | "half_hipped" | "side_hipped" | "side_half-hipped" => {
                RoofShape::HalfHipped
            }
            "pyramidal" | "cone" | "dome" | "onion" => RoofShape::Pyramidal,
            "skillion" | "lean_to" => RoofShape::Skillion,
            "mansard" => RoofShape::Mansard,
            "gambrel" => RoofShape::Gambrel,
            _ => return None,
        })
    }

    pub fn is_pitched(self) -> bool {
        self != RoofShape::Flat
    }
}

coded_enum! {
    /// The finer use of a building (OSM `building=*`) beyond its
    /// [`BuildingKind`]: what a style preset may treat differently.
    BuildingUse {
        Unknown = 0 => "unknown",
        Chalet = 1 => "chalet",
        Farmhouse = 2 => "farmhouse",
        Barn = 3 => "barn",
        Garage = 4 => "garage",
        Shed = 5 => "shed",
        Church = 6 => "church",
        Chapel = 7 => "chapel",
        Greenhouse = 8 => "greenhouse",
        /// `building=roof` and carports: a roof on posts.
        Canopy = 9 => "canopy",
        Hut = 10 => "hut",
        Hotel = 11 => "hotel",
        Retail = 12 => "retail",
        Terrace = 13 => "terrace",
    }
}

impl BuildingUse {
    /// The use of an OSM `building=*` value (`None` when the kind says it all).
    pub fn from_osm(v: &str) -> Option<Self> {
        Some(match v {
            "chalet" => BuildingUse::Chalet,
            "farm" | "farmhouse" => BuildingUse::Farmhouse,
            "barn" | "farm_auxiliary" | "stable" | "cowshed" | "sty" => BuildingUse::Barn,
            "garage" | "garages" => BuildingUse::Garage,
            "shed" | "boathouse" => BuildingUse::Shed,
            "church" | "cathedral" => BuildingUse::Church,
            "chapel" | "shrine" => BuildingUse::Chapel,
            "greenhouse" => BuildingUse::Greenhouse,
            "roof" | "carport" => BuildingUse::Canopy,
            "hut" | "cabin" => BuildingUse::Hut,
            "hotel" => BuildingUse::Hotel,
            "retail" | "supermarket" | "kiosk" => BuildingUse::Retail,
            "terrace" => BuildingUse::Terrace,
            _ => return None,
        })
    }
}

coded_enum! {
    /// Wall material (OSM `building:material`, `building:facade:material`).
    WallMaterial {
        Plaster = 1 => "plaster",
        Timber = 2 => "timber",
        Stone = 3 => "stone",
        Brick = 4 => "brick",
        Concrete = 5 => "concrete",
        Glass = 6 => "glass",
        Metal = 7 => "metal",
    }
}

impl WallMaterial {
    pub fn from_osm(v: &str) -> Option<Self> {
        Some(match v.trim() {
            "plaster" | "render" | "stucco" | "plastered" | "cement_block" => WallMaterial::Plaster,
            "wood" | "timber" | "timber_framing" | "log" | "wooden" => WallMaterial::Timber,
            "stone" | "sandstone" | "limestone" | "granite" | "marble" | "masonry" | "tuff"
            | "basalt" | "volcanic_rock" => WallMaterial::Stone,
            "brick" | "bricks" | "clay" | "adobe" => WallMaterial::Brick,
            "concrete" | "reinforced_concrete" | "panel" => WallMaterial::Concrete,
            "glass" | "mirror" => WallMaterial::Glass,
            "metal" | "steel" | "metal_plates" | "aluminium" | "sheet_metal" | "copper" => {
                WallMaterial::Metal
            }
            _ => return None,
        })
    }
}

coded_enum! {
    /// Roof material (OSM `roof:material`).
    RoofMaterial {
        Tiles = 1 => "tiles",
        Slate = 2 => "slate",
        Metal = 3 => "metal",
        Concrete = 4 => "concrete",
        Glass = 5 => "glass",
        Wood = 6 => "wood",
        Grass = 7 => "grass",
    }
}

impl RoofMaterial {
    pub fn from_osm(v: &str) -> Option<Self> {
        Some(match v.trim() {
            "roof_tiles" | "tile" | "tiles" | "clay" | "ceramic" | "clay_tiles" => {
                RoofMaterial::Tiles
            }
            "slate" | "eternit" | "asbestos" | "stone" | "fibre_cement" | "shingle" => {
                RoofMaterial::Slate
            }
            "metal" | "metal_sheet" | "tin" | "copper" | "zinc" | "steel" | "aluminium" => {
                RoofMaterial::Metal
            }
            "concrete" | "tar_paper" | "asphalt" | "bitumen" | "gravel" | "plastic"
            | "pebblestone" | "cement" => RoofMaterial::Concrete,
            "glass" | "acrylic_glass" | "solar_panels" | "pv" => RoofMaterial::Glass,
            "wood" | "wooden" | "wood_shingles" | "thatch" | "reed" => RoofMaterial::Wood,
            "grass" | "plants" | "green" => RoofMaterial::Grass,
            _ => return None,
        })
    }
}

/// What OSM says about a building's look (spec §14 Phase 8 T4), stored in
/// the section `BTAG`. Everything is optional: the client's regional style
/// preset fills in what OSM does not say.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BuildingTags {
    pub building_use: Option<BuildingUse>,
    /// Tagged `roof:shape`. A pitched shape needs the four-point `BLDG`
    /// outline (`Roof::Gabled`), a pyramid alternatively a polygon with a
    /// [`Self::roof_height_m`].
    pub roof_shape: Option<RoofShape>,
    /// Roof height from `roof:height`, `roof:angle` or `roof:levels` (the
    /// `BLDG` gable height is then the same), or the height of a pyramid on
    /// a polygon. `None`: the style may scale the default.
    pub roof_height_m: Option<f32>,
    /// `building:levels` (above ground, without the roof).
    pub levels: Option<u8>,
    /// `roof:levels`.
    pub roof_levels: Option<u8>,
    /// Bottom of the walls above the ground (`min_height`,
    /// `building:min_level`): canopies, buildings over a passage.
    pub min_height_m: Option<f32>,
    pub wall: Option<WallMaterial>,
    /// `building:colour` (sRGB).
    pub wall_colour: Option<[u8; 3]>,
    pub roof_material: Option<RoofMaterial>,
    /// `roof:colour` (sRGB); a measured `ROOF` colour wins.
    pub roof_colour: Option<[u8; 3]>,
}

impl BuildingTags {
    pub fn is_empty(&self) -> bool {
        *self == BuildingTags::default()
    }
}

/// Writes the `BTAG` payload; `false` (nothing written): no building has
/// tags, so no section.
///
/// ```text
/// layout version: u32 (1)
/// tile count: u32, then per tile:
///   tile x, z: i32 i32   record count: u32, then per record:
///     building index in the tile's BLDG list: u32
///     record length: u8 (the bytes after it), flags: u16, then the fields
///     whose flag is set, in flag order:
///       0 use u8, 1 roof shape u8, 2 roof height f32, 3 levels u8,
///       4 roof levels u8, 5 min height f32, 6 wall material u8,
///       7 wall colour 3 × u8, 8 roof material u8, 9 roof colour 3 × u8
/// ```
///
/// Readers skip the flags and bytes they do not know (the record length),
/// so fields can be added without a new layout.
pub fn write_building_tags(t: &Buildings, b: &mut Vec<u8>) -> bool {
    let tiles: Vec<(&TileKey, Vec<(usize, &BuildingTags)>)> = t
        .iter()
        .map(|(k, list)| {
            let recs: Vec<(usize, &BuildingTags)> = list
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.tags.is_empty())
                .map(|(i, b)| (i, &b.tags))
                .collect();
            (k, recs)
        })
        .filter(|(_, v)| !v.is_empty())
        .collect();
    if tiles.is_empty() {
        return false;
    }
    b.extend(BUILDING_TAGS_VERSION.to_le_bytes());
    b.extend((tiles.len() as u32).to_le_bytes());
    for (key, recs) in tiles {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((recs.len() as u32).to_le_bytes());
        for (i, t) in recs {
            b.extend((i as u32).to_le_bytes());
            let mut flags = 0u16;
            let mut f = Vec::new();
            let mut field = |bit: u16, bytes: &[u8]| {
                flags |= 1 << bit;
                f.extend_from_slice(bytes);
            };
            if let Some(u) = t.building_use {
                field(0, &[u as u8]);
            }
            if let Some(s) = t.roof_shape {
                field(1, &[s as u8]);
            }
            if let Some(h) = t.roof_height_m {
                field(2, &h.to_le_bytes());
            }
            if let Some(l) = t.levels {
                field(3, &[l]);
            }
            if let Some(l) = t.roof_levels {
                field(4, &[l]);
            }
            if let Some(h) = t.min_height_m {
                field(5, &h.to_le_bytes());
            }
            if let Some(m) = t.wall {
                field(6, &[m as u8]);
            }
            if let Some(c) = t.wall_colour {
                field(7, &c);
            }
            if let Some(m) = t.roof_material {
                field(8, &[m as u8]);
            }
            if let Some(c) = t.roof_colour {
                field(9, &c);
            }
            b.push((f.len() + 2) as u8);
            b.extend(flags.to_le_bytes());
            b.extend(f);
        }
    }
    true
}

/// Building tags per tile: (index in the tile's `BLDG` list, tags).
pub type BuildingTagRecords = BTreeMap<TileKey, Vec<(usize, BuildingTags)>>;

/// Reads a `BTAG` payload written by [`write_building_tags`]. Unknown
/// codes become `None`; unknown fields are skipped.
pub fn read_building_tags(bytes: &[u8]) -> Result<BuildingTagRecords, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != BUILDING_TAGS_VERSION {
        return Err(format!("building tags layout {version} is not supported"));
    }
    let mut out = BuildingTagRecords::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()?;
        let mut list = Vec::with_capacity(n.min(1 << 16) as usize);
        for _ in 0..n {
            let index = r.u32()? as usize;
            let len = usize::from(r.u8()?);
            let mut f = Bytes::new(r.take(len)?);
            let flags = u16::from_le_bytes(f.arr()?);
            let has = |bit: u16| flags & (1 << bit) != 0;
            let mut t = BuildingTags::default();
            if has(0) {
                t.building_use = BuildingUse::from_code(f.u8()?);
            }
            if has(1) {
                t.roof_shape = RoofShape::from_code(f.u8()?);
            }
            if has(2) {
                t.roof_height_m = Some(f.f32()?);
            }
            if has(3) {
                t.levels = Some(f.u8()?);
            }
            if has(4) {
                t.roof_levels = Some(f.u8()?);
            }
            if has(5) {
                t.min_height_m = Some(f.f32()?);
            }
            if has(6) {
                t.wall = WallMaterial::from_code(f.u8()?);
            }
            if has(7) {
                t.wall_colour = Some(f.arr()?);
            }
            if has(8) {
                t.roof_material = RoofMaterial::from_code(f.u8()?);
            }
            if has(9) {
                t.roof_colour = Some(f.arr()?);
            }
            // Later fields (higher flags) are skipped with the rest of the record.
            list.push((index, t));
        }
        out.insert(key, list);
    }
    r.finish("building tags section")?;
    Ok(out)
}

/// Puts the tags read from `BTAG` on their buildings; records of buildings
/// that do not exist are dropped (returns how many).
pub fn apply_building_tags(buildings: &mut Buildings, tags: BuildingTagRecords) -> usize {
    let mut dropped = 0;
    for (key, recs) in tags {
        let Some(list) = buildings.get_mut(&key) else {
            dropped += recs.len();
            continue;
        };
        for (i, t) in recs {
            match list.get_mut(i) {
                Some(b) => b.tags = t,
                None => dropped += 1,
            }
        }
    }
    dropped
}

/// Twice the signed area of a ring in the (x, z) plane (positive:
/// counter-clockwise).
pub fn signed_area<T: Copy + Into<f64>>(ring: &[[T; 2]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0].into() * b[1].into() - b[0].into() * a[1].into()
        })
        .sum::<f64>()
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Whether segments `p1 p2` and `q1 q2` cross in their interiors.
fn segments_cross(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
    let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Triangulates a polygon with holes by ear clipping (holes are first
/// bridged into the outer ring). Vertex indices refer to the outer ring
/// followed by the holes in order. Triangles wind like the outer ring
/// (counter-clockwise in (x, z)). Rings may wind either way; degenerate
/// input yields fewer triangles, never a panic.
pub fn triangulate(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> Vec<[u32; 3]> {
    let mut pts: Vec<[f64; 2]> = outer.iter().map(|p| p.map(f64::from)).collect();
    let mut poly: Vec<u32> = (0..outer.len() as u32).collect();
    if signed_area(outer) < 0.0 {
        poly.reverse();
    }
    // Holes, rightmost first, each wound clockwise and bridged from its
    // rightmost vertex to the nearest visible vertex of the polygon so far.
    let mut hole_rings: Vec<Vec<u32>> = Vec::new();
    for h in holes {
        let start = pts.len() as u32;
        pts.extend(h.iter().map(|p| p.map(f64::from)));
        if h.len() < 3 {
            continue;
        }
        let mut ring: Vec<u32> = (start..start + h.len() as u32).collect();
        if signed_area(h) > 0.0 {
            ring.reverse();
        }
        hole_rings.push(ring);
    }
    let max_x = |r: &Vec<u32>| {
        r.iter()
            .map(|&i| pts[i as usize][0])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    hole_rings.sort_by(|a, b| max_x(b).total_cmp(&max_x(a)));
    for (hi, ring) in hole_rings.iter().enumerate() {
        let m_pos = (0..ring.len())
            .max_by(|&a, &b| pts[ring[a] as usize][0].total_cmp(&pts[ring[b] as usize][0]))
            .expect("hole has points");
        let m = pts[ring[m_pos] as usize];
        // All edges that a bridge must not cross: the polygon and the holes
        // not yet bridged.
        let mut edges: Vec<(u32, u32)> = (0..poly.len())
            .map(|i| (poly[i], poly[(i + 1) % poly.len()]))
            .collect();
        for r in &hole_rings[hi..] {
            edges.extend((0..r.len()).map(|i| (r[i], r[(i + 1) % r.len()])));
        }
        let visible = |v: u32| {
            let p = pts[v as usize];
            edges.iter().all(|&(a, b)| {
                a == v
                    || b == v
                    || a == ring[m_pos]
                    || b == ring[m_pos]
                    || !segments_cross(m, p, pts[a as usize], pts[b as usize])
            })
        };
        let best = (0..poly.len())
            .filter(|&i| visible(poly[i]))
            .min_by(|&a, &b| {
                let d = |i: usize| {
                    let p = pts[poly[i] as usize];
                    (p[0] - m[0]).powi(2) + (p[1] - m[1]).powi(2)
                };
                d(a).total_cmp(&d(b))
            });
        let Some(at) = best else {
            continue; // no visible vertex: the hole is dropped
        };
        // poly[..=at], hole from m around back to m, poly[at], poly[at+1..]
        let mut spliced = Vec::with_capacity(poly.len() + ring.len() + 2);
        spliced.extend_from_slice(&poly[..=at]);
        for j in 0..=ring.len() {
            spliced.push(ring[(m_pos + j) % ring.len()]);
        }
        spliced.push(poly[at]);
        spliced.extend_from_slice(&poly[at + 1..]);
        poly = spliced;
    }
    // Ear clipping.
    let mut tris = Vec::new();
    let mut guard = 0;
    while poly.len() > 3 && guard < poly.len() * poly.len() + 16 {
        guard += 1;
        let n = poly.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let (a, b, c) = (pts[ia as usize], pts[ib as usize], pts[ic as usize]);
            let turn = cross(a, b, c);
            if turn < 0.0 {
                continue; // reflex
            }
            if turn == 0.0 {
                // Collinear (or a bridge's doubled vertex): drop it.
                poly.remove(i);
                clipped = true;
                break;
            }
            let blocked = poly.iter().any(|&j| {
                let p = pts[j as usize];
                p != a && p != b && p != c && in_triangle(p, a, b, c)
            });
            if !blocked {
                tris.push([ia, ib, ic]);
                poly.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break; // self-intersecting input: give up on the rest
        }
    }
    if poly.len() == 3 {
        let (a, b, c) = (
            pts[poly[0] as usize],
            pts[poly[1] as usize],
            pts[poly[2] as usize],
        );
        if cross(a, b, c) > 0.0 {
            tris.push([poly[0], poly[1], poly[2]]);
        }
    }
    tris
}

/// Writes the buildings section payload:
///
/// ```text
/// layout version: u32 (1)
/// tile count: u32, then per tile:
///   tile x, z: i32 i32   building count: u32, then per building:
///     kind: u8   roof: u8 (0 flat, 1 gabled)   [roof height: f32]
///     base y, eave y: f32 f32
///     ring count: u16 (outline first), per ring: point count: u32, points: [f32; 2]
/// ```
pub fn write_buildings(t: &Buildings, b: &mut Vec<u8>) {
    b.extend(BUILDINGS_VERSION.to_le_bytes());
    b.extend((t.len() as u32).to_le_bytes());
    for (key, list) in t {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((list.len() as u32).to_le_bytes());
        for bl in list {
            b.push(bl.kind as u8);
            match bl.roof {
                Roof::Flat => b.push(0),
                Roof::Gabled { height_m } => {
                    b.push(1);
                    b.extend(height_m.to_le_bytes());
                }
            }
            b.extend(bl.base_y.to_le_bytes());
            b.extend(bl.eave_y.to_le_bytes());
            let rings = bl.holes.len().min(usize::from(u16::MAX) - 1) + 1;
            b.extend((rings as u16).to_le_bytes());
            for ring in std::iter::once(&bl.outline).chain(&bl.holes).take(rings) {
                b.extend((ring.len() as u32).to_le_bytes());
                for p in ring {
                    b.extend(p[0].to_le_bytes());
                    b.extend(p[1].to_le_bytes());
                }
            }
        }
    }
}

/// Reads a buildings section payload written by [`write_buildings`].
pub fn read_buildings(bytes: &[u8]) -> Result<Buildings, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != BUILDINGS_VERSION {
        return Err(format!("buildings layout {version} is not supported"));
    }
    let mut out = Buildings::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()?;
        let mut list = Vec::with_capacity(n.min(1 << 16) as usize);
        for _ in 0..n {
            let code = r.u8()?;
            let kind =
                BuildingKind::from_code(code).ok_or(format!("unknown building kind {code}"))?;
            let roof = match r.u8()? {
                0 => Roof::Flat,
                1 => Roof::Gabled { height_m: r.f32()? },
                x => return Err(format!("unknown roof shape {x}")),
            };
            let (base_y, eave_y) = (r.f32()?, r.f32()?);
            let rings = u16::from_le_bytes(r.arr()?);
            if rings == 0 {
                return Err("building without an outline".into());
            }
            let mut ring_list = Vec::with_capacity(usize::from(rings));
            for _ in 0..rings {
                let np = r.u32()?;
                let ring = (0..np)
                    .map(|_| Ok([r.f32()?, r.f32()?]))
                    .collect::<Result<Vec<_>, String>>()?;
                if ring.len() < 3 {
                    return Err("building ring with fewer than three points".into());
                }
                ring_list.push(ring);
            }
            let outline = ring_list.remove(0);
            if matches!(roof, Roof::Gabled { .. }) && outline.len() != 4 {
                return Err("gabled roof needs a rectangular footprint".into());
            }
            list.push(Building {
                kind,
                outline,
                holes: ring_list,
                base_y,
                eave_y,
                roof,
                tags: BuildingTags::default(),
            });
        }
        out.insert(key, list);
    }
    r.finish("buildings section")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(pts: &[[f64; 2]], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| cross(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]) / 2.0)
            .sum()
    }

    fn all_points(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> Vec<[f64; 2]> {
        outer
            .iter()
            .chain(holes.iter().flatten())
            .map(|p| p.map(f64::from))
            .collect()
    }

    #[test]
    fn convex_and_concave_polygons_are_triangulated() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let t = triangulate(&square, &[]);
        assert_eq!(t.len(), 2);
        assert!((area_of(&all_points(&square, &[]), &t) - 100.0).abs() < 1e-9);
        // Clockwise input gives counter-clockwise triangles of the same area.
        let mut cw = square;
        cw.reverse();
        let t = triangulate(&cw, &[]);
        assert!((area_of(&all_points(&cw, &[]), &t) - 100.0).abs() < 1e-9);
        // An L shape (concave): 6 points, 4 triangles, area 75.
        let l = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        let t = triangulate(&l, &[]);
        assert_eq!(t.len(), 4);
        assert!((area_of(&all_points(&l, &[]), &t) - 75.0).abs() < 1e-9);
        // Collinear points are dropped, not turned into slivers.
        let c = [
            [0.0, 0.0],
            [5.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ];
        let t = triangulate(&c, &[]);
        assert!((area_of(&all_points(&c, &[]), &t) - 100.0).abs() < 1e-9);
        // Degenerate input does not panic.
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0]], &[]).is_empty());
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], &[]).is_empty());
    }

    #[test]
    fn holes_are_left_open() {
        // 20 × 20 courtyard building with two 4 × 4 courtyards: area 368.
        let outer = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let holes = vec![
            vec![[4.0, 4.0], [8.0, 4.0], [8.0, 8.0], [4.0, 8.0]],
            vec![[12.0, 12.0], [12.0, 16.0], [16.0, 16.0], [16.0, 12.0]],
        ];
        let t = triangulate(&outer, &holes);
        let pts = all_points(&outer, &holes);
        assert!(
            (area_of(&pts, &t) - 368.0).abs() < 1e-9,
            "{}",
            area_of(&pts, &t)
        );
        // No triangle covers the courtyard centres.
        for c in [[6.0, 6.0], [14.0, 14.0]] {
            assert!(!t.iter().any(|tr| in_triangle(
                c,
                pts[tr[0] as usize],
                pts[tr[1] as usize],
                pts[tr[2] as usize]
            )));
        }
        // All triangles wind counter-clockwise.
        assert!(t.iter().all(|tr| cross(
            pts[tr[0] as usize],
            pts[tr[1] as usize],
            pts[tr[2] as usize]
        ) > 0.0));
    }

    #[test]
    fn section_round_trip() {
        let house = Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]],
            holes: vec![],
            base_y: 449.5,
            eave_y: 456.0,
            roof: Roof::Gabled { height_m: 3.0 },
            tags: BuildingTags::default(),
        };
        let block = Building {
            kind: BuildingKind::Apartments,
            outline: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
            holes: vec![vec![[4.0, 4.0], [4.0, 8.0], [8.0, 8.0]]],
            base_y: 450.0,
            eave_y: 465.0,
            roof: Roof::Flat,
            tags: BuildingTags::default(),
        };
        let t = Buildings::from([
            (TileKey { x: 0, z: 0 }, vec![house.clone(), block]),
            (TileKey { x: 3, z: -1 }, vec![]),
        ]);
        let mut b = Vec::new();
        write_buildings(&t, &mut b);
        assert_eq!(read_buildings(&b).unwrap(), t);
        assert!(read_buildings(&b[..b.len() - 2]).is_err());
        // A gabled roof on anything but four points is rejected.
        let bad = Buildings::from([(
            TileKey { x: 0, z: 0 },
            vec![Building {
                outline: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                ..house
            }],
        )]);
        let mut b = Vec::new();
        write_buildings(&bad, &mut b);
        assert!(read_buildings(&b).unwrap_err().contains("rectangular"));
    }

    #[test]
    fn building_tags_round_trip_sparse_and_forward_compatible() {
        let plain = Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]],
            holes: vec![],
            base_y: 449.5,
            eave_y: 456.0,
            roof: Roof::Gabled { height_m: 3.0 },
            tags: BuildingTags::default(),
        };
        let full = BuildingTags {
            building_use: Some(BuildingUse::Farmhouse),
            roof_shape: Some(RoofShape::HalfHipped),
            roof_height_m: Some(6.5),
            levels: Some(2),
            roof_levels: Some(1),
            min_height_m: Some(0.0),
            wall: Some(WallMaterial::Timber),
            wall_colour: Some([200, 180, 150]),
            roof_material: Some(RoofMaterial::Slate),
            roof_colour: Some([60, 60, 66]),
        };
        let tower = BuildingTags {
            roof_shape: Some(RoofShape::Pyramidal),
            roof_height_m: Some(9.0),
            ..Default::default()
        };
        let mut t = Buildings::from([
            (
                TileKey { x: 0, z: 0 },
                vec![
                    plain.clone(),
                    Building {
                        tags: full,
                        ..plain.clone()
                    },
                ],
            ),
            (TileKey { x: 1, z: 0 }, vec![plain.clone()]),
            (
                TileKey { x: 2, z: 0 },
                vec![Building {
                    tags: tower,
                    ..plain.clone()
                }],
            ),
        ]);
        let mut b = Vec::new();
        assert!(write_building_tags(&t, &mut b));
        let recs = read_building_tags(&b).unwrap();
        // Only buildings with tags, only tiles with such buildings.
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[&TileKey { x: 0, z: 0 }], vec![(1, full)]);
        assert_eq!(recs[&TileKey { x: 2, z: 0 }], vec![(0, tower)]);
        // Applied onto tag-less buildings they give the original back.
        let mut bare = t.clone();
        for l in bare.values_mut() {
            for b in l {
                b.tags = BuildingTags::default();
            }
        }
        assert_eq!(apply_building_tags(&mut bare, recs.clone()), 0);
        assert_eq!(bare, t);
        // Records of missing buildings are dropped, not fatal.
        t.remove(&TileKey { x: 2, z: 0 });
        assert_eq!(apply_building_tags(&mut t, recs), 1);
        // No tags: no section.
        let none = Buildings::from([(TileKey { x: 0, z: 0 }, vec![plain.clone()])]);
        assert!(!write_building_tags(&none, &mut Vec::new()));
        // A newer writer's extra field (flag 15, two more bytes) is skipped.
        let mut newer = Vec::new();
        newer.extend(1u32.to_le_bytes());
        newer.extend(1u32.to_le_bytes());
        newer.extend(0i32.to_le_bytes());
        newer.extend(0i32.to_le_bytes());
        newer.extend(1u32.to_le_bytes());
        newer.extend(0u32.to_le_bytes()); // building 0
        newer.push(2 + 1 + 2); // flags, levels, the unknown field
        newer.extend((1u16 << 3 | 1 << 15).to_le_bytes());
        newer.push(4);
        newer.extend([0xAB, 0xCD]);
        let r = read_building_tags(&newer).unwrap();
        assert_eq!(r[&TileKey { x: 0, z: 0 }][0].1.levels, Some(4));
        assert!(read_building_tags(&newer[..newer.len() - 1]).is_err());
        // Unknown codes (a newer shape) read as untagged.
        assert_eq!(RoofShape::from_code(200), None);
    }

    #[test]
    fn osm_values_map_to_shapes_uses_and_materials() {
        assert_eq!(RoofShape::from_osm("hipped"), Some(RoofShape::Hipped));
        assert_eq!(
            RoofShape::from_osm("half-hipped"),
            Some(RoofShape::HalfHipped)
        );
        assert_eq!(
            RoofShape::from_osm("side_hipped"),
            Some(RoofShape::HalfHipped)
        );
        assert_eq!(RoofShape::from_osm("saltbox"), Some(RoofShape::Gabled));
        assert_eq!(RoofShape::from_osm("onion"), Some(RoofShape::Pyramidal));
        assert_eq!(RoofShape::from_osm("lean_to"), Some(RoofShape::Skillion));
        assert_eq!(RoofShape::from_osm("cylinder"), None);
        assert!(!RoofShape::Flat.is_pitched() && RoofShape::Mansard.is_pitched());
        for s in RoofShape::ALL {
            assert_eq!(RoofShape::from_code(*s as u8), Some(*s));
            assert_eq!(RoofShape::from_name(s.name()), Some(*s));
        }
        assert_eq!(BuildingUse::from_osm("barn"), Some(BuildingUse::Barn));
        assert_eq!(BuildingUse::from_osm("carport"), Some(BuildingUse::Canopy));
        assert_eq!(BuildingUse::from_osm("house"), None);
        assert_eq!(WallMaterial::from_osm("wood"), Some(WallMaterial::Timber));
        assert_eq!(
            WallMaterial::from_osm("sandstone"),
            Some(WallMaterial::Stone)
        );
        assert_eq!(WallMaterial::from_osm("unobtainium"), None);
        assert_eq!(
            RoofMaterial::from_osm("roof_tiles"),
            Some(RoofMaterial::Tiles)
        );
        assert_eq!(RoofMaterial::from_osm("eternit"), Some(RoofMaterial::Slate));
        assert_eq!(
            RoofMaterial::from_osm("tar_paper"),
            Some(RoofMaterial::Concrete)
        );
    }
}
