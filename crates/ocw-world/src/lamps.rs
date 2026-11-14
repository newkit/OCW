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
//! (both sides, staggered, on wide roads).

use crate::graph::RoadClass;
use crate::landuse::LandClass;
use crate::package::{Bytes, RoadStrip, TileKey, WorldPackage};

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

/// The lamps along one road strip. `town(x, z)` tells whether a place is
/// in a town (for roads without a `lit` tag).
pub fn strip_lamps(strip: &RoadStrip, town: impl Fn(f32, f32) -> bool) -> Vec<Lamp> {
    if strip.lit == RoadLit::No || strip.points.len() < 2 {
        return Vec::new();
    }
    if strip.lit == RoadLit::Unknown && !lit_in_towns(strip.class) {
        return Vec::new();
    }
    let spacing = spacing_m(strip.class);
    let height = lamp_height_m(strip.class);
    let wide = strip.width_m >= 9.0;
    let side_m = strip.dress.sidewalk_m[0].max(strip.dress.sidewalk_m[1]);
    let offset = strip.width_m / 2.0 + side_m + 0.6;
    let arm = (strip.width_m * 0.25).clamp(0.6, 2.0);
    let mut out = Vec::new();
    // Lamp k stands at edge distance (k + 0.5) · spacing.
    let start = strip.dress.start_m;
    let mut k = ((start / spacing) - 0.5).ceil().max(0.0);
    let mut s0 = 0.0f32;
    for w in strip.points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-4 {
            continue;
        }
        loop {
            let at = (k + 0.5) * spacing - start;
            if at > s0 + len {
                break;
            }
            if at >= s0 {
                let t = (at - s0) / len;
                let p = [a[0] + dx * t, a[1] + (b[1] - a[1]) * t, a[2] + dz * t];
                // Right of the drawing direction (x east, z south), or
                // staggered on both sides of a wide road.
                let right = [-dz / len, dx / len];
                let sign = if wide && (k as i64) % 2 == 1 {
                    -1.0
                } else {
                    1.0
                };
                let foot = [
                    p[0] + right[0] * offset * sign,
                    p[1],
                    p[2] + right[1] * offset * sign,
                ];
                if strip.lit == RoadLit::Yes || town(foot[0], foot[2]) {
                    out.push(Lamp {
                        foot,
                        toward: [-right[0] * sign, -right[1] * sign],
                        height_m: height,
                        arm_m: arm,
                    });
                }
            }
            k += 1.0;
        }
        s0 += len;
    }
    out
}

/// All lamps of a package tile.
pub fn tile_lamps(pkg: &WorldPackage, key: TileKey) -> Vec<Lamp> {
    let Some(tile) = pkg.tiles.get(&key) else {
        return Vec::new();
    };
    let size = f64::from(pkg.tile_size_m);
    let town = |x: f32, z: f32| {
        pkg.landuse
            .as_ref()
            .and_then(|l| l.class_at(size, f64::from(x), f64::from(z)))
            .is_some_and(is_town)
    };
    tile.roads
        .iter()
        .flat_map(|s| strip_lamps(s, town))
        .collect()
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
        }
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
        let lamps = strip_lamps(&s, |_, _| false);
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
            strip_lamps(
                &strip(RoadClass::Residential, RoadLit::No, 320.0),
                |_, _| true
            )
            .is_empty()
        );
        let untagged = strip(RoadClass::Residential, RoadLit::Unknown, 320.0);
        assert!(strip_lamps(&untagged, |_, _| false).is_empty());
        // Town on the first 100 m only.
        let town = strip_lamps(&untagged, |_, z| z > -100.0);
        assert_eq!(town.len(), 3);
        // Tracks are not lit in towns without a tag, but with one.
        assert!(
            strip_lamps(&strip(RoadClass::Track, RoadLit::Unknown, 300.0), |_, _| {
                true
            })
            .is_empty()
        );
        assert!(
            !strip_lamps(&strip(RoadClass::Track, RoadLit::Yes, 300.0), |_, _| false).is_empty()
        );
    }

    /// The spacing continues across tiles: a strip starting 100 m into
    /// its edge puts its first lamp at edge distance 112 m (3.5 × 32).
    #[test]
    fn lamps_continue_across_tiles() {
        let mut s = strip(RoadClass::Residential, RoadLit::Yes, 200.0);
        s.dress.start_m = 100.0;
        let lamps = strip_lamps(&s, |_, _| false);
        assert!((lamps[0].foot[2] + 12.0).abs() < 1e-3, "{:?}", lamps[0]);
        // Wide roads: staggered on both sides.
        let mut w = strip(RoadClass::Primary, RoadLit::Yes, 200.0);
        w.width_m = 12.0;
        let lamps = strip_lamps(&w, |_, _| false);
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
        };
        let b = Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            holes: Vec::new(),
            base_y: 0.0,
            eave_y: 6.0,
            roof: Roof::Flat,
            tags: Default::default(),
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
