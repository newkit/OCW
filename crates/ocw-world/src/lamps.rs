//! Night lighting of the world (spec §14 Phase 9, S2): which roads are lit
//! (OSM `lit`), where their street lamps stand, and how much light a
//! place's towns spill into the night sky.
//!
//! In a world package the lit state is the section `LITR`: per tile, one
//! byte per road strip ([`RoadLit::code`]) in the tile's strip order, like
//! the road surfaces (`RSRF`). Only tiles with a tagged strip are written,
//! so a package built without `lit` data has no section, and older readers
//! skip it. Roads without the tag get lamps where they run through a town
//! (residential, commercial or industrial land use) – the usual case in
//! the Alps and the Black Forest, where `lit` is often missing.
//!
//! The lamps are placed by the client from the strips (no positions in
//! the package): every [`spacing_m`] along the road, at a fixed phase of
//! the edge's length so they continue across tiles, on the right-hand side
//! (both sides, staggered, on wide roads) – and never in a road: a foot
//! that would stand on a carriageway or a junction surface is moved along
//! the road or left out ([`place_lamp`], [`LampSpace`]).
//!
//! Street lamps mapped in OSM (`highway=street_lamp` nodes) are stored by
//! the world builder in the section `OBST` as points of the kind
//! [`ObstacleKind::StreetLamp`] where they are mapped (an addition to the
//! section: older readers leave unknown kinds out). Each one stands where
//! it is mapped unless that is in a road – then it is moved to the nearest
//! side of the nearest carriageway ([`osm_lamp`]) – with its arm towards
//! that road; the regular row along a lit road skips the spots near a
//! mapped lamp ([`tile_lamps`]). [`package_lamps`] places every lamp of a
//! package, for the server's obstacles.

use std::collections::BTreeMap;

use crate::graph::RoadClass;
use crate::landuse::LandClass;
use crate::obstacles::{ObstacleKind, PointObstacle};
use crate::package::{Bytes, RoadStrip, TileKey, WorldPackage};

#[cfg(test)]
mod osm_tests;
#[cfg(test)]
mod rule_tests;
mod space;
pub use space::{Blocked, FOOT_CLEAR_M, LEVEL_M, LampSpace, NearRoad, POLE_R_M, ring, ring_edges};

/// Section tag of the roads' lit state.
pub const ROAD_LIT_TAG: &[u8; 4] = b"LITR";

/// Whether a road is lit at night (OSM `lit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum RoadLit {
    /// No tag: lit where it runs through a town.
    #[default]
    Unknown = 0,
    /// `lit=yes` (also `24/7`, `automatic`, `sunset-sunrise`, …).
    Yes = 1,
    /// `lit=no` (also `disused`).
    No = 2,
}

impl RoadLit {
    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Yes),
            2 => Some(Self::No),
            _ => None,
        }
    }

    /// From the value of an OSM `lit` tag.
    pub fn from_osm(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(
                "yes" | "24/7" | "automatic" | "limited" | "interval" | "sunset-sunrise"
                | "dusk-dawn" | "sunset-23:00" | "sunset-24:00",
            ) => Self::Yes,
            Some(v) if v.starts_with("sunset") || v.contains(':') => Self::Yes,
            Some("no" | "disused" | "none") => Self::No,
            _ => Self::Unknown,
        }
    }
}

/// Writes the section payload: tiles (in `tiles` order) with at least one
/// strip whose lit state is known. `false` (nothing written) without any.
pub fn write_road_lit<'a>(
    tiles: impl Iterator<Item = (TileKey, &'a [RoadStrip])>,
    b: &mut Vec<u8>,
) -> bool {
    let mut payload = Vec::new();
    let mut count = 0u32;
    for (key, strips) in tiles {
        if strips.iter().all(|s| s.lit == RoadLit::Unknown) {
            continue;
        }
        count += 1;
        payload.extend(key.x.to_le_bytes());
        payload.extend(key.z.to_le_bytes());
        payload.extend((strips.len() as u32).to_le_bytes());
        payload.extend(strips.iter().map(|s| s.lit.code()));
    }
    if count == 0 {
        return false;
    }
    b.extend(count.to_le_bytes());
    b.extend(payload);
    true
}

/// Reads the section payload: per tile, the lit state of each strip
/// (unknown codes from newer builders read as [`RoadLit::Unknown`]).
pub fn read_road_lit(bytes: &[u8]) -> Result<Vec<(TileKey, Vec<RoadLit>)>, String> {
    let mut r = Bytes::new(bytes);
    let n = r.u32()?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let strips = r.u32()? as usize;
        let codes = r.take(strips)?;
        out.push((
            key,
            codes
                .iter()
                .map(|&c| RoadLit::from_code(c).unwrap_or_default())
                .collect(),
        ));
    }
    r.finish("road lit section")?;
    Ok(out)
}

/// A street lamp beside a road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    /// Foot of the pole (x, road height, z; package metres).
    pub foot: [f32; 3],
    /// Unit vector (x, z) from the foot towards the road: the arm and the
    /// lamp head point that way.
    pub toward: [f32; 2],
    /// Height of the lamp head above the foot (m).
    pub height_m: f32,
    /// Reach of the arm over the road (m).
    pub arm_m: f32,
}

impl Lamp {
    /// Where the light comes from (x, y, z).
    pub fn head(&self) -> [f32; 3] {
        [
            self.foot[0] + self.toward[0] * self.arm_m,
            self.foot[1] + self.height_m,
            self.foot[2] + self.toward[1] * self.arm_m,
        ]
    }
}

/// Distance between lamps along a road of this class (m).
pub fn spacing_m(class: RoadClass) -> f32 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 36.0,
        RoadClass::Tertiary | RoadClass::Unclassified | RoadClass::Residential => 32.0,
        RoadClass::Service => 30.0,
        RoadClass::Track | RoadClass::Cycleway | RoadClass::Path => 26.0,
    }
}

/// Height of the lamp heads (m).
pub fn lamp_height_m(class: RoadClass) -> f32 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 8.0,
        RoadClass::Track | RoadClass::Cycleway | RoadClass::Path => 4.5,
        _ => 6.0,
    }
}

/// Classes that are lit in towns without a `lit` tag.
fn lit_in_towns(class: RoadClass) -> bool {
    !matches!(
        class,
        RoadClass::Track | RoadClass::Path | RoadClass::Cycleway
    )
}

/// Land use of a town: lamps on untagged roads there.
pub fn is_town(class: LandClass) -> bool {
    matches!(
        class,
        LandClass::Residential | LandClass::Commercial | LandClass::Industrial
    )
}

/// A lamp of the regular row along a strip before the placement rule
/// ([`nominal_lamps`]): where along the strip it belongs and on which side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nominal {
    /// Metres along the strip's drawn points.
    pub at: f32,
    /// 1: right of the drawing direction, -1: left.
    pub sign: f32,
    /// The lamp at its regular place (the placement before the rule).
    pub lamp: Lamp,
}

/// The point of a strip `at` metres along its drawn points and the unit
/// direction there (the first segment that reaches `at`).
fn strip_frame(points: &[[f32; 3]], at: f32) -> Option<([f32; 3], [f32; 2])> {
    let mut s0 = 0.0f32;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-4 {
            continue;
        }
        if at <= s0 + len {
            let t = ((at - s0) / len).max(0.0);
            let p = [a[0] + dx * t, a[1] + (b[1] - a[1]) * t, a[2] + dz * t];
            return Some((p, [dx / len, dz / len]));
        }
        s0 += len;
    }
    None
}

/// Drawn length of a strip (m).
fn strip_length(points: &[[f32; 3]]) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][2] - w[0][2]);
            let len = (dx * dx + dz * dz).sqrt();
            if len < 1e-4 { 0.0 } else { len }
        })
        .sum()
}

/// How far from the centre line a lamp of this strip stands normally:
/// beyond the sidewalk (m).
fn side_offset(strip: &RoadStrip) -> f32 {
    let side_m = strip.dress.sidewalk_m[0].max(strip.dress.sidewalk_m[1]);
    strip.width_m / 2.0 + side_m + 0.6
}

/// The lamp standing at `p` on the strip's side `sign`, `offset` metres
/// from its centre line, the arm over the road.
fn lamp_at(strip: &RoadStrip, p: [f32; 3], dir: [f32; 2], sign: f32, offset: f32) -> Lamp {
    let right = [-dir[1], dir[0]];
    Lamp {
        foot: [
            p[0] + right[0] * offset * sign,
            p[1],
            p[2] + right[1] * offset * sign,
        ],
        toward: [-right[0] * sign, -right[1] * sign],
        height_m: lamp_height_m(strip.class),
        arm_m: (strip.width_m * 0.25).clamp(0.6, 2.0),
    }
}

/// The regular row of lamps along one road strip, before the rule that
/// keeps them out of the roads ([`strip_lamps`]): every [`spacing_m`] at a
/// fixed phase of the edge's length, right of the road beyond the
/// sidewalk (staggered on both sides of a road from 9 m width). `town(x,
/// z)` tells whether a place is in a town (for roads without a `lit` tag).
pub fn nominal_lamps(strip: &RoadStrip, town: impl Fn(f32, f32) -> bool) -> Vec<Nominal> {
    if strip.lit == RoadLit::No || strip.points.len() < 2 {
        return Vec::new();
    }
    if strip.lit == RoadLit::Unknown && !lit_in_towns(strip.class) {
        return Vec::new();
    }
    let spacing = spacing_m(strip.class);
    let wide = strip.width_m >= 9.0;
    let offset = side_offset(strip);
    let total = strip_length(&strip.points);
    let mut out = Vec::new();
    // Lamp k stands at edge distance (k + 0.5) · spacing.
    let start = strip.dress.start_m;
    let mut k = ((start / spacing) - 0.5).ceil().max(0.0);
    loop {
        let at = (k + 0.5) * spacing - start;
        if at > total {
            break;
        }
        if at >= 0.0
            && let Some((p, dir)) = strip_frame(&strip.points, at)
        {
            // Right of the drawing direction (x east, z south), or
            // staggered on both sides of a wide road.
            let sign = if wide && (k as i64) % 2 == 1 {
                -1.0
            } else {
                1.0
            };
            let lamp = lamp_at(strip, p, dir, sign, offset);
            if strip.lit == RoadLit::Yes || town(lamp.foot[0], lamp.foot[2]) {
                out.push(Nominal { at, sign, lamp });
            }
        }
        k += 1.0;
    }
    out
}

/// Steps (m) a lamp is moved along its road to find a free spot …
pub const MOVE_STEP_M: f32 = 1.5;
/// … at most this share of the spacing either way (its neighbours stay
/// apart).
pub const MOVE_SHARE: f32 = 0.4;

/// Where a regular lamp ends up under the rule (spec: world look, *Street
/// lamps*, the rule of 2026-11-22): its foot outside every carriageway and
/// junction surface ([`LampSpace::blocked`]). Tried in this order at its
/// place and then [`MOVE_STEP_M`] further and back along the strip, up to
/// [`MOVE_SHARE`] of the spacing, never past the strip's ends: beyond the
/// sidewalk, then at the kerb (0.45 m outside the carriageway); on a bridge
/// deck on its parapet or rail (a bridge of the graph without a deck has
/// no room). `None`: no free spot, the lamp is left out.
pub fn place_lamp(strip: &RoadStrip, n: &Nominal, space: &LampSpace) -> Option<(Lamp, bool)> {
    let total = strip_length(&strip.points);
    let max = (MOVE_SHARE * spacing_m(strip.class) / MOVE_STEP_M).floor() as i32;
    let far = side_offset(strip);
    let near = strip.width_m / 2.0 + 0.45;
    for i in 0..=2 * max {
        // 0, +1, -1, +2, -2, … steps.
        let step = if i % 2 == 1 { (i + 1) / 2 } else { -(i / 2) };
        let at = n.at + step as f32 * MOVE_STEP_M;
        if !(0.0..=total).contains(&at) {
            continue;
        }
        let Some((p, dir)) = strip_frame(&strip.points, at) else {
            continue;
        };
        let centre = [f64::from(p[0]), f64::from(p[2])];
        let offsets: Vec<f32> = match space.deck_foot(centre, f64::from(p[1])) {
            Some(d) => vec![d as f32],
            None if space.on_bridge(centre) => continue,
            None if near + 0.1 < far => vec![far, near],
            None => vec![far],
        };
        for off in offsets {
            let lamp = lamp_at(strip, p, dir, n.sign, off);
            if space.blocked(lamp.foot.map(f64::from)).is_none() {
                return Some((lamp, step != 0 || off != far));
            }
        }
    }
    None
}

/// The lamps along one road strip, kept out of the roads ([`place_lamp`]).
pub fn strip_lamps(
    strip: &RoadStrip,
    town: impl Fn(f32, f32) -> bool,
    space: &LampSpace,
) -> Vec<Lamp> {
    nominal_lamps(strip, town)
        .iter()
        .filter_map(|n| place_lamp(strip, n, space).map(|(l, _)| l))
        .collect()
}

/// Whether a place is in a town by the package's land use.
pub fn town_at(pkg: &WorldPackage) -> impl Fn(f32, f32) -> bool + '_ {
    let size = f64::from(pkg.tile_size_m);
    move |x: f32, z: f32| {
        pkg.landuse
            .as_ref()
            .and_then(|l| l.class_at(size, f64::from(x), f64::from(z)))
            .is_some_and(is_town)
    }
}

/// A mapped street lamp stands at most this far beside the edge of a
/// carriageway (m); farther ones are left out (they light no road).
pub const OSM_REACH_M: f64 = 15.0;
/// Steps (m) of the search for a free foot across the road.
pub const OSM_STEP_M: f64 = 0.25;
/// How far beyond the kerb a mapped lamp on a carriageway may be moved
/// (m), and how far a lamp mapped in a building or on a wall.
pub const OSM_MOVE_M: f64 = 6.0;
/// The kerb: a moved lamp's centre this far outside the carriageway (m;
/// as [`place_lamp`]'s kerb).
pub const KERB_OFF_M: f64 = 0.45;

/// The street lamps mapped in OSM in tile `key` (section `OBST`).
pub fn osm_nodes(pkg: &WorldPackage, key: TileKey) -> impl Iterator<Item = &PointObstacle> {
    pkg.obstacles
        .as_ref()
        .and_then(|o| o.tiles.get(&key))
        .into_iter()
        .flat_map(|t| t.points.iter())
        .filter(|p| p.kind == ObstacleKind::StreetLamp)
}

/// Where a mapped street lamp stands (the world look's *Street lamps*
/// rule): where it is mapped, its arm towards the nearest carriageway –
/// unless its foot would stand in a road (a carriageway or a junction
/// surface: an OSM node on the road), a building or on a wall; then on
/// the nearest free spot across that road, at its kerb or beyond (up to
/// [`OSM_MOVE_M`]; on a bridge deck on its parapet), else the nearest free
/// spot round the node (a node in a junction's mouth, rings of
/// [`OSM_STEP_M`] up to [`OSM_MOVE_M`]). The height is OSM's `height`
/// (3–14 m), else the road class's. `None`: no carriageway within
/// [`OSM_REACH_M`], or no free spot. The `f32` is how far it was moved
/// (m; 0 where mapped).
pub fn osm_lamp(p: &PointObstacle, space: &LampSpace) -> Option<(Lamp, f32)> {
    let at = [f64::from(p.pos[0]), f64::from(p.pos[1])];
    let near = space.nearest_road(at, OSM_REACH_M)?;
    let y = if near.y.is_finite() { near.y } else { 0.0 };
    let free = |q: [f64; 2]| space.blocked([q[0], y, q[1]]).is_none();
    let foot = if free(at) && !space.on_bridge(at) {
        at
    } else {
        moved_foot(at, &near, y, space, &free)?
    };
    // The arm towards the nearest carriageway from where it stands.
    let to = space
        .nearest_road(foot, OSM_REACH_M + OSM_MOVE_M)
        .unwrap_or(near);
    let (dx, dz) = (to.closest[0] - foot[0], to.closest[1] - foot[1]);
    let l = dx.hypot(dz);
    let toward = if l > 1e-6 {
        [(dx / l) as f32, (dz / l) as f32]
    } else {
        [(-to.dir[1]) as f32, to.dir[0] as f32]
    };
    let height_m = if (3.0..=14.0).contains(&p.height_m) {
        p.height_m
    } else {
        lamp_height_m(to.class)
    };
    let lamp = Lamp {
        foot: [foot[0] as f32, y as f32, foot[1] as f32],
        toward,
        height_m,
        arm_m: ((to.half * 0.5) as f32).clamp(0.6, 2.0),
    };
    let moved = (foot[0] - at[0]).hypot(foot[1] - at[1]);
    Some((lamp, moved as f32))
}

/// The free foot nearest to a mapped lamp `at` that cannot stand there
/// ([`osm_lamp`]).
fn moved_foot(
    at: [f64; 2],
    near: &NearRoad,
    y: f64,
    space: &LampSpace,
    free: &dyn Fn([f64; 2]) -> bool,
) -> Option<[f64; 2]> {
    // Across the road, towards the mapped spot (the right-hand side for a
    // node on the centre line).
    let (n, d) = if near.centre_m > 1e-3 {
        (
            [
                (at[0] - near.closest[0]) / near.centre_m,
                (at[1] - near.closest[1]) / near.centre_m,
            ],
            near.centre_m,
        )
    } else {
        ([-near.dir[1], near.dir[0]], 0.0)
    };
    let across = |o: f64| [near.closest[0] + n[0] * o, near.closest[1] + n[1] * o];
    if let Some(deck) = space.deck_foot(near.closest, y) {
        // On a bridge: on its parapet, the side nearest to the node.
        return [deck, -deck].into_iter().map(across).find(|q| free(*q));
    }
    if space.on_bridge(near.closest) {
        // A bridge of the graph without a deck has no room.
        return None;
    }
    let steps = (OSM_MOVE_M / OSM_STEP_M).round() as i32;
    let kerb = near.half + KERB_OFF_M;
    let mut best: Option<f64> = None;
    let mut consider = |o: f64| {
        if best.is_some_and(|b: f64| (b - d).abs() <= (o - d).abs()) {
            return;
        }
        if free(across(o)) {
            best = Some(o);
        }
    };
    // Never on the carriageway, at most OSM_MOVE_M beyond the kerb …
    for side in [1.0, -1.0] {
        for k in 0..=steps {
            consider(side * (kerb + f64::from(k) * OSM_STEP_M));
        }
    }
    // … and off the road (in a building, on a wall) towards and away from
    // it on its own side.
    if d > kerb {
        for k in 1..=steps {
            for o in [d - f64::from(k) * OSM_STEP_M, d + f64::from(k) * OSM_STEP_M] {
                if o >= kerb {
                    consider(o);
                }
            }
        }
    }
    if let Some(o) = best {
        return Some(across(o));
    }
    // Round the node (a junction's mouth, between two roads): the first
    // ring with a free spot, its first direction from +x.
    const DIRECTIONS: usize = 32;
    (1..=steps).find_map(|k| {
        let r = f64::from(k) * OSM_STEP_M;
        (0..DIRECTIONS).find_map(|i| {
            let a = i as f64 * std::f64::consts::TAU / DIRECTIONS as f64;
            let q = [at[0] + r * a.cos(), at[1] + r * a.sin()];
            free(q).then_some(q)
        })
    })
}

/// Whether a regular lamp of `strip` at `foot` is near a mapped one
/// (`osm`: the mapped feet round the tile): within half the spacing of
/// it, the mapped lamp beside the same road (within the lamps' reach of
/// its centre line).
fn near_mapped(strip: &RoadStrip, foot: [f32; 3], osm: &[[f32; 2]]) -> bool {
    let r = spacing_m(strip.class) / 2.0;
    let reach = f64::from(strip.width_m) / 2.0 + 12.0;
    osm.iter().any(|q| {
        let (dx, dz) = (q[0] - foot[0], q[1] - foot[2]);
        if dx * dx + dz * dz >= r * r {
            return false;
        }
        let p = [f64::from(q[0]), f64::from(q[1])];
        strip.points.windows(2).any(|w| {
            crate::obstacles::segment_distance(
                p,
                [f64::from(w[0][0]), f64::from(w[0][2])],
                [f64::from(w[1][0]), f64::from(w[1][2])],
            ) < reach
        })
    })
}

/// What [`tile_lamps_counted`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LampCounts {
    /// Mapped lamps (OSM) placed, of them moved out of a road, left out.
    pub osm: usize,
    pub osm_moved: usize,
    pub osm_left_out: usize,
    /// Regular lamps along lit roads placed, and skipped near a mapped one.
    pub rows: usize,
    pub rows_skipped: usize,
}

impl std::ops::AddAssign for LampCounts {
    fn add_assign(&mut self, o: Self) {
        self.osm += o.osm;
        self.osm_moved += o.osm_moved;
        self.osm_left_out += o.osm_left_out;
        self.rows += o.rows;
        self.rows_skipped += o.rows_skipped;
    }
}

/// All lamps of a package tile, kept out of the roads in `space` (the
/// space of this tile, [`LampSpace::new`]): the mapped ones of the tile
/// ([`osm_lamp`]), then the regular rows along its lit strips without the
/// spots near a mapped lamp.
pub fn tile_lamps(pkg: &WorldPackage, key: TileKey, space: &LampSpace) -> Vec<Lamp> {
    tile_lamps_counted(pkg, key, space).0
}

/// [`tile_lamps`] with what it did.
pub fn tile_lamps_counted(
    pkg: &WorldPackage,
    key: TileKey,
    space: &LampSpace,
) -> (Vec<Lamp>, LampCounts) {
    let mut c = LampCounts::default();
    let mut out = Vec::new();
    let Some(tile) = pkg.tiles.get(&key) else {
        return (out, c);
    };
    for p in osm_nodes(pkg, key) {
        match osm_lamp(p, space) {
            Some((l, moved)) => {
                c.osm += 1;
                if moved > 0.01 {
                    c.osm_moved += 1;
                }
                out.push(l);
            }
            None => c.osm_left_out += 1,
        }
    }
    let osm: Vec<[f32; 2]> = ring(key)
        .flat_map(|k| osm_nodes(pkg, k).map(|p| p.pos))
        .collect();
    let town = town_at(pkg);
    for s in &tile.roads {
        for l in strip_lamps(s, &town, space) {
            if !osm.is_empty() && near_mapped(s, l.foot, &osm) {
                c.rows_skipped += 1;
            } else {
                c.rows += 1;
                out.push(l);
            }
        }
    }
    (out, c)
}

/// What the lamps of a whole package are placed with: the graph edges
/// near each tile and the package's junctions (stored, else computed),
/// as both apps' renderers have them.
pub struct PackageSpaces<'a> {
    pkg: &'a WorldPackage,
    tile_edges: std::collections::HashMap<TileKey, Vec<u32>>,
    junctions: Option<crate::junction::JunctionSet>,
}

impl<'a> PackageSpaces<'a> {
    pub fn new(pkg: &'a WorldPackage) -> Self {
        let graph = pkg.graph.as_deref();
        Self {
            pkg,
            tile_edges: graph
                .map(|g| crate::junction::section::tile_edges(g, f64::from(pkg.tile_size_m)))
                .unwrap_or_default(),
            junctions: graph.map(|g| {
                crate::junction::JunctionSet::with_index(
                    g,
                    pkg.junctions.as_ref().and_then(|j| j.index_for(g)),
                )
            }),
        }
    }

    /// The space round tile `key`.
    pub fn space(&self, key: TileKey) -> LampSpace<'_> {
        let none: &[u32] = &[];
        let edges = ring_edges(key, |k| self.tile_edges.get(&k).map_or(none, Vec::as_slice));
        LampSpace::new(self.pkg, key, &edges, self.junctions.as_ref())
    }
}

/// Every street lamp of a package by the tile whose strips (or mapped
/// lamps) placed it, as the apps draw them ([`tile_lamps`]): for the
/// server's obstacles (lamp poles a rider can hit) and the lint. Empty
/// for a package without tiles (a streamed base). Native builds place the
/// tiles on all cores; the result does not depend on it.
pub fn package_lamps(pkg: &WorldPackage) -> BTreeMap<TileKey, Vec<Lamp>> {
    package_lamps_counted(pkg).0
}

/// [`package_lamps`] with what the placement did.
pub fn package_lamps_counted(pkg: &WorldPackage) -> (BTreeMap<TileKey, Vec<Lamp>>, LampCounts) {
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    if keys.is_empty() {
        return Default::default();
    }
    let spaces = PackageSpaces::new(pkg);
    let one = |key: TileKey| tile_lamps_counted(pkg, key, &spaces.space(key));
    let results: Vec<(Vec<Lamp>, LampCounts)> = each_tile(&keys, &one);
    let mut total = LampCounts::default();
    let mut out = BTreeMap::new();
    for (key, (lamps, c)) in keys.into_iter().zip(results) {
        total += c;
        if !lamps.is_empty() {
            out.insert(key, lamps);
        }
    }
    (out, total)
}

/// `f` of every key, in order (on all cores natively).
#[cfg(not(target_arch = "wasm32"))]
fn each_tile<T: Send>(keys: &[TileKey], f: &(dyn Fn(TileKey) -> T + Sync)) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 16);
    if threads == 1 || keys.len() < 8 {
        return keys.iter().map(|k| f(*k)).collect();
    }
    let chunk = keys.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = keys
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(|k| f(*k)).collect::<Vec<T>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("lamp placement thread"))
            .collect()
    })
}

#[cfg(target_arch = "wasm32")]
fn each_tile<T>(keys: &[TileKey], f: &dyn Fn(TileKey) -> T) -> Vec<T> {
    keys.iter().map(|k| f(*k)).collect()
}

/// Buildings within this distance count towards light pollution (m).
pub const POLLUTION_RADIUS_M: f64 = 12_000.0;

/// Light pollution at a place (0 dark sky … 1 a town centre), from the
/// buildings of the package around it: each tile's buildings weigh
/// 1 / (1 + (d / 1.5 km)²), and 3000 weighted buildings make a sky
/// e⁻¹ from dark. Vaduz is bright, the Malbun valley a little, the
/// Teide plateau dark.
pub fn light_pollution(pkg: &WorldPackage, p: [f64; 2]) -> f64 {
    let Some(buildings) = &pkg.buildings else {
        return 0.0;
    };
    let size = f64::from(pkg.tile_size_m);
    let mut sum = 0.0;
    for (key, list) in buildings {
        if list.is_empty() {
            continue;
        }
        let o = key.origin(size);
        let c = [o[0] + size / 2.0, o[1] + size / 2.0];
        let d = ((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2)).sqrt();
        if d > POLLUTION_RADIUS_M {
            continue;
        }
        let d = d.max(size / 3.0);
        sum += list.len() as f64 / (1.0 + (d / 1500.0).powi(2));
    }
    1.0 - (-sum / 3000.0).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RoadDress;
    use crate::RoadSurface;

    fn strip(class: RoadClass, lit: RoadLit, len: f32) -> RoadStrip {
        let n = (len / 10.0) as usize + 1;
        RoadStrip {
            class,
            width_m: 6.0,
            // Northwards (towards -z), 10 m per point, at 500 m height.
            points: (0..n).map(|i| [0.0, 500.0, -(i as f32) * 10.0]).collect(),
            surface: RoadSurface::of_class(class),
            dress: RoadDress::of_class(class, 6.0),
            lit,
            condition: Default::default(),
        }
    }

    /// The regular row (before the rule) of a strip.
    fn row(s: &RoadStrip, town: impl Fn(f32, f32) -> bool) -> Vec<Lamp> {
        nominal_lamps(s, town).iter().map(|n| n.lamp).collect()
    }

    #[test]
    fn osm_lit_tags() {
        assert_eq!(RoadLit::from_osm(Some("yes")), RoadLit::Yes);
        assert_eq!(RoadLit::from_osm(Some("24/7")), RoadLit::Yes);
        assert_eq!(RoadLit::from_osm(Some("sunset-01:00")), RoadLit::Yes);
        assert_eq!(RoadLit::from_osm(Some("no")), RoadLit::No);
        assert_eq!(RoadLit::from_osm(Some("disused")), RoadLit::No);
        assert_eq!(RoadLit::from_osm(Some("maybe")), RoadLit::Unknown);
        assert_eq!(RoadLit::from_osm(None), RoadLit::Unknown);
        for l in [RoadLit::Unknown, RoadLit::Yes, RoadLit::No] {
            assert_eq!(RoadLit::from_code(l.code()), Some(l));
        }
    }

    /// Street lamps from `lit=yes`: every 32 m on a residential road, on
    /// its right, beside the carriageway, the arm over the road.
    #[test]
    fn lamps_stand_along_lit_roads() {
        let s = strip(RoadClass::Residential, RoadLit::Yes, 320.0);
        let lamps = row(&s, |_, _| false);
        assert_eq!(lamps.len(), 10, "{lamps:?}");
        for (i, l) in lamps.iter().enumerate() {
            // Northwards: right is east (+x), beyond the 3 m half width.
            assert!(l.foot[0] > 3.0 && l.foot[0] < 7.0, "{l:?}");
            assert!((l.foot[2] + (i as f32 + 0.5) * 32.0).abs() < 1e-3, "{l:?}");
            assert_eq!(l.foot[1], 500.0);
            assert_eq!(l.toward, [-1.0, 0.0]);
            assert!(l.head()[1] > 505.0 && l.head()[0] < l.foot[0]);
        }
        // lit=no: none, even in a town; untagged: only in town.
        assert!(
            row(
                &strip(RoadClass::Residential, RoadLit::No, 320.0),
                |_, _| true
            )
            .is_empty()
        );
        let untagged = strip(RoadClass::Residential, RoadLit::Unknown, 320.0);
        assert!(row(&untagged, |_, _| false).is_empty());
        // Town on the first 100 m only.
        let town = row(&untagged, |_, z| z > -100.0);
        assert_eq!(town.len(), 3);
        // Tracks are not lit in towns without a tag, but with one.
        assert!(
            row(&strip(RoadClass::Track, RoadLit::Unknown, 300.0), |_, _| {
                true
            })
            .is_empty()
        );
        assert!(!row(&strip(RoadClass::Track, RoadLit::Yes, 300.0), |_, _| false).is_empty());
    }

    /// The spacing continues across tiles: a strip starting 100 m into
    /// its edge puts its first lamp at edge distance 112 m (3.5 × 32).
    #[test]
    fn lamps_continue_across_tiles() {
        let mut s = strip(RoadClass::Residential, RoadLit::Yes, 200.0);
        s.dress.start_m = 100.0;
        let lamps = row(&s, |_, _| false);
        assert!((lamps[0].foot[2] + 12.0).abs() < 1e-3, "{:?}", lamps[0]);
        // Wide roads: staggered on both sides.
        let mut w = strip(RoadClass::Primary, RoadLit::Yes, 200.0);
        w.width_m = 12.0;
        let lamps = row(&w, |_, _| false);
        assert!(lamps.iter().any(|l| l.foot[0] > 0.0) && lamps.iter().any(|l| l.foot[0] < 0.0));
    }

    #[test]
    fn the_section_round_trips() {
        let a = strip(RoadClass::Residential, RoadLit::Yes, 50.0);
        let b = strip(RoadClass::Residential, RoadLit::Unknown, 50.0);
        let c = strip(RoadClass::Primary, RoadLit::No, 50.0);
        let t1 = [a.clone(), b.clone()];
        let t2 = [b.clone()];
        let t3 = [c.clone(), b];
        let tiles = [
            (TileKey { x: 0, z: 0 }, &t1[..]),
            (TileKey { x: 1, z: 0 }, &t2[..]),
            (TileKey { x: -1, z: 2 }, &t3[..]),
        ];
        let mut bytes = Vec::new();
        assert!(write_road_lit(tiles.iter().copied(), &mut bytes));
        let back = read_road_lit(&bytes).unwrap();
        assert_eq!(
            back,
            vec![
                (TileKey { x: 0, z: 0 }, vec![RoadLit::Yes, RoadLit::Unknown]),
                (TileKey { x: -1, z: 2 }, vec![RoadLit::No, RoadLit::Unknown]),
            ]
        );
        let mut none = Vec::new();
        assert!(!write_road_lit(
            [(TileKey { x: 1, z: 0 }, &t2[..])].into_iter(),
            &mut none
        ));
        assert!(none.is_empty());
    }

    fn package_with_town(buildings: usize) -> WorldPackage {
        use crate::buildings::{Building, BuildingKind, Roof};
        let mut pkg = WorldPackage {
            name: "town".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
        };
        let b = Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            holes: Vec::new(),
            base_y: 0.0,
            eave_y: 6.0,
            roof: Roof::Flat,
            tags: Default::default(),
            parts: Default::default(),
        };
        pkg.buildings = Some(std::collections::BTreeMap::from([(
            TileKey { x: 0, z: 0 },
            vec![b; buildings],
        )]));
        pkg
    }

    /// Light pollution: bright in a town, fading with distance, none
    /// without buildings.
    #[test]
    fn light_pollution_near_towns() {
        let town = package_with_town(6000);
        let size = f64::from(town.tile_size_m);
        let centre = [size / 2.0, size / 2.0];
        let near = light_pollution(&town, centre);
        assert!(near > 0.8, "{near}");
        let five_km = light_pollution(&town, [centre[0] + 5000.0, centre[1]]);
        assert!(five_km < 0.3 * near && five_km > 0.0, "{five_km}");
        assert_eq!(
            light_pollution(&town, [centre[0] + 20_000.0, centre[1]]),
            0.0
        );
        let hamlet = light_pollution(&package_with_town(40), centre);
        assert!(hamlet < 0.3, "{hamlet}");
        let mut empty = town.clone();
        empty.buildings = None;
        assert_eq!(light_pollution(&empty, centre), 0.0);
    }
}
