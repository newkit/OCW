//! The condition of a road's surface as OSM tags it (spec §4.6 step 6,
//! *Road damage*): `smoothness`, `tracktype` and the detailed `surface`
//! value, kept per road strip so the renderer can draw cracks, potholes
//! and loose stones by the tagged condition ([`crate::road_wear`]).
//!
//! [`crate::RoadSurface`] is the material layer (asphalt, worn asphalt,
//! gravel, paving, dirt) and lumps concrete with asphalt and setts with
//! paving stones; [`SurfaceKind`] keeps the tag's own value so concrete can
//! show its joints.
//!
//! In a world package the condition is the section `RCND`: per tile, two
//! bytes per road strip ([`RoadCondition::code`]) in the tile's strip order,
//! like `LITR`. Only tiles with a strip whose condition is known are
//! written, so packages built without it have no section, older readers
//! skip it, and their strips read as [`RoadCondition::UNKNOWN`] (the
//! renderer then estimates the condition from the road class).

use crate::package::{Bytes, RoadStrip, TileKey};

/// Section tag of the road condition.
pub const ROAD_CONDITION_TAG: &[u8; 4] = b"RCND";

/// OSM `smoothness`, best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Smoothness {
    /// No tag (or an unknown value): estimated from the road.
    #[default]
    Unknown = 0,
    Excellent = 1,
    Good = 2,
    Intermediate = 3,
    Bad = 4,
    VeryBad = 5,
    Horrible = 6,
    VeryHorrible = 7,
    Impassable = 8,
}

impl Smoothness {
    pub const ALL: [Smoothness; 9] = [
        Smoothness::Unknown,
        Smoothness::Excellent,
        Smoothness::Good,
        Smoothness::Intermediate,
        Smoothness::Bad,
        Smoothness::VeryBad,
        Smoothness::Horrible,
        Smoothness::VeryHorrible,
        Smoothness::Impassable,
    ];

    /// From an OSM `smoothness` value.
    pub fn from_osm(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("excellent") => Self::Excellent,
            Some("good") => Self::Good,
            Some("intermediate") => Self::Intermediate,
            Some("bad") => Self::Bad,
            Some("very_bad") => Self::VeryBad,
            Some("horrible") => Self::Horrible,
            Some("very_horrible") => Self::VeryHorrible,
            Some("impassable") => Self::Impassable,
            _ => Self::Unknown,
        }
    }

    /// The OSM value (`"unknown"` without a tag).
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Intermediate => "intermediate",
            Self::Bad => "bad",
            Self::VeryBad => "very_bad",
            Self::Horrible => "horrible",
            Self::VeryHorrible => "very_horrible",
            Self::Impassable => "impassable",
        }
    }

    pub fn from_code(code: u8) -> Self {
        Self::ALL
            .get(usize::from(code))
            .copied()
            .unwrap_or_default()
    }
}

/// The OSM `surface` value of a road, finer than [`crate::RoadSurface`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum SurfaceKind {
    #[default]
    Unknown = 0,
    Asphalt = 1,
    /// `concrete`, `concrete:lanes`: slabs with joints.
    Concrete = 2,
    /// `concrete:plates`.
    ConcretePlates = 3,
    ChipSeal = 4,
    PavingStones = 5,
    Sett = 6,
    /// `cobblestone`, `unhewn_cobblestone`, `cobblestone:flattened`.
    Cobblestone = 7,
    Compacted = 8,
    FineGravel = 9,
    /// `gravel`, `pebblestone`, `unpaved`, `rock`, `shells`.
    Gravel = 10,
    /// `dirt`, `earth`, `ground`, `mud`, `sand`, `soil`, `clay`.
    Dirt = 11,
    /// `grass`, `grass_paver`.
    Grass = 12,
    /// Any other paved value (`paved`, `bricks`, `metal`, `wood`, …).
    OtherPaved = 13,
}

impl SurfaceKind {
    /// From an OSM `surface` value (the first of a `;` list).
    pub fn from_osm(value: Option<&str>) -> Self {
        let Some(v) = value else {
            return Self::Unknown;
        };
        match v.split(';').next().unwrap_or("").trim() {
            "asphalt" => Self::Asphalt,
            "concrete" | "concrete:lanes" => Self::Concrete,
            "concrete:plates" => Self::ConcretePlates,
            "chipseal" => Self::ChipSeal,
            "paving_stones" | "paving_stones:lanes" => Self::PavingStones,
            "sett" => Self::Sett,
            "cobblestone" | "unhewn_cobblestone" | "cobblestone:flattened" => Self::Cobblestone,
            "compacted" => Self::Compacted,
            "fine_gravel" => Self::FineGravel,
            "gravel" | "pebblestone" | "unpaved" | "rock" | "shells" => Self::Gravel,
            "dirt" | "earth" | "ground" | "mud" | "sand" | "soil" | "clay" | "woodchips" => {
                Self::Dirt
            }
            "grass" | "grass_paver" => Self::Grass,
            "paved" | "bricks" | "metal" | "wood" | "metal_grid" | "stone" | "tartan"
            | "rubber" => Self::OtherPaved,
            _ => Self::Unknown,
        }
    }

    pub fn from_code(code: u8) -> Self {
        use SurfaceKind::*;
        [
            Unknown,
            Asphalt,
            Concrete,
            ConcretePlates,
            ChipSeal,
            PavingStones,
            Sett,
            Cobblestone,
            Compacted,
            FineGravel,
            Gravel,
            Dirt,
            Grass,
            OtherPaved,
        ]
        .get(usize::from(code))
        .copied()
        .unwrap_or_default()
    }

    /// Concrete slabs: joints instead of the asphalt's random cracks.
    pub fn concrete(self) -> bool {
        matches!(self, Self::Concrete | Self::ConcretePlates)
    }
}

/// What OSM says about a road's surface condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RoadCondition {
    pub smoothness: Smoothness,
    /// OSM `tracktype` grade 1 … 5 (0: none).
    pub tracktype: u8,
    pub kind: SurfaceKind,
}

impl RoadCondition {
    pub const UNKNOWN: Self = Self {
        smoothness: Smoothness::Unknown,
        tracktype: 0,
        kind: SurfaceKind::Unknown,
    };

    /// From the OSM tags of a way (`tag` returns a tag's value).
    pub fn from_osm<'a>(tag: impl Fn(&str) -> Option<&'a str>) -> Self {
        let tracktype = match tag("tracktype").map(str::trim) {
            Some("grade1") => 1,
            Some("grade2") => 2,
            Some("grade3") => 3,
            Some("grade4") => 4,
            Some("grade5") => 5,
            _ => 0,
        };
        Self {
            smoothness: Smoothness::from_osm(tag("smoothness")),
            tracktype,
            kind: SurfaceKind::from_osm(tag("surface")),
        }
    }

    pub fn is_unknown(&self) -> bool {
        *self == Self::UNKNOWN
    }

    /// The two bytes of the section: smoothness + 16 · tracktype, kind.
    pub fn code(&self) -> [u8; 2] {
        [
            self.smoothness as u8 | self.tracktype.min(5) << 4,
            self.kind as u8,
        ]
    }

    /// Decodes [`RoadCondition::code`]; values of newer builders read as
    /// unknown.
    pub fn from_code(c: [u8; 2]) -> Self {
        let tt = c[0] >> 4;
        Self {
            smoothness: Smoothness::from_code(c[0] & 15),
            tracktype: if tt <= 5 { tt } else { 0 },
            kind: SurfaceKind::from_code(c[1]),
        }
    }
}

/// Writes the section payload: tiles (in `tiles` order) with at least one
/// strip whose condition is known. `false` (nothing written) without any.
pub fn write_road_condition<'a>(
    tiles: impl Iterator<Item = (TileKey, &'a [RoadStrip])>,
    b: &mut Vec<u8>,
) -> bool {
    let mut payload = Vec::new();
    let mut count = 0u32;
    for (key, strips) in tiles {
        if strips.iter().all(|s| s.condition.is_unknown()) {
            continue;
        }
        count += 1;
        payload.extend(key.x.to_le_bytes());
        payload.extend(key.z.to_le_bytes());
        payload.extend((strips.len() as u32).to_le_bytes());
        payload.extend(strips.iter().flat_map(|s| s.condition.code()));
    }
    if count == 0 {
        return false;
    }
    b.extend(count.to_le_bytes());
    b.extend(payload);
    true
}

/// Reads the section payload: per tile, the condition of each strip.
pub fn read_road_condition(bytes: &[u8]) -> Result<Vec<(TileKey, Vec<RoadCondition>)>, String> {
    let mut r = Bytes::new(bytes);
    let n = r.u32()?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let strips = r.u32()? as usize;
        let codes = r.take(2 * strips)?;
        out.push((
            key,
            codes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| RoadCondition::from_code(*c))
                .collect(),
        ));
    }
    r.finish("road condition section")?;
    Ok(out)
}

/// Road length (m) per smoothness value over all strips of `tiles`:
/// how much of a world is tagged and how much is estimated.
pub fn smoothness_lengths<'a>(
    tiles: impl Iterator<Item = &'a [RoadStrip]>,
) -> [(Smoothness, f64); 9] {
    let mut out = Smoothness::ALL.map(|s| (s, 0.0));
    for strips in tiles {
        for s in strips {
            let len: f64 = s
                .points
                .windows(2)
                .map(|w| {
                    let (a, b) = (w[0], w[1]);
                    f64::from(((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt())
                })
                .sum();
            out[s.condition.smoothness as usize].1 += len;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
        move |k| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| *v)
    }

    #[test]
    fn osm_tags_and_codes_round_trip() {
        for s in Smoothness::ALL {
            assert_eq!(Smoothness::from_osm(Some(s.name())), s);
        }
        assert_eq!(Smoothness::from_osm(Some("so-so")), Smoothness::Unknown);
        let c = RoadCondition::from_osm(tags(&[
            ("smoothness", "very_bad"),
            ("tracktype", "grade3"),
            ("surface", "concrete:plates;asphalt"),
        ]));
        assert_eq!(c.smoothness, Smoothness::VeryBad);
        assert_eq!(c.tracktype, 3);
        assert_eq!(c.kind, SurfaceKind::ConcretePlates);
        assert!(c.kind.concrete());
        assert_eq!(RoadCondition::from_code(c.code()), c);
        assert!(RoadCondition::from_osm(tags(&[])).is_unknown());
        assert_eq!(
            RoadCondition::from_code(RoadCondition::UNKNOWN.code()),
            RoadCondition::UNKNOWN
        );
        // Codes of a newer builder read as unknown, never as an error.
        assert_eq!(
            RoadCondition::from_code([15 | 7 << 4, 200]),
            RoadCondition::UNKNOWN
        );
        for (v, k) in [
            ("sett", SurfaceKind::Sett),
            ("compacted", SurfaceKind::Compacted),
            ("fine_gravel", SurfaceKind::FineGravel),
            ("ground", SurfaceKind::Dirt),
            ("paving_stones", SurfaceKind::PavingStones),
            ("cobblestone:flattened", SurfaceKind::Cobblestone),
        ] {
            assert_eq!(SurfaceKind::from_osm(Some(v)), k, "{v}");
        }
    }
}
