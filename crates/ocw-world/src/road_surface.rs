//! What a road looks like (spec §4.4, §4.6 step 6, Phase 8 T2): the
//! surface material of a road strip, from OSM `surface` (with
//! `smoothness` and `tracktype`), else from the road category.
//!
//! This is the *look*; the rolling resistance comes from
//! [`crate::Surface`] (asphalt, gravel, grass), which [`RoadSurface::physics`]
//! maps to.
//!
//! In a world package the surfaces are the section `RSRF`: per tile, one
//! byte per road strip, in the tile's strip order. Only tiles with a strip
//! whose surface differs from its category's default are written, so a
//! package without OSM surfaces has no section, and older readers skip it.

use crate::Surface;
use crate::graph::RoadClass;
use crate::package::{Bytes, TileKey};

/// Section tag of the road surfaces.
pub const ROAD_SURFACE_TAG: &[u8; 4] = b"RSRF";

/// Surface material of a road (the renderer's road material layers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum RoadSurface {
    /// Asphalt, concrete, chip seal: paved and smooth.
    #[default]
    Asphalt = 0,
    /// Asphalt in an intermediate or bad state (`smoothness`).
    WornAsphalt = 1,
    /// Gravel, compacted, fine gravel, pebbles, generic `unpaved`.
    Gravel = 2,
    /// Paving stones, setts, cobblestones, bricks.
    Paving = 3,
    /// Dirt, earth, ground, grass, mud, sand.
    Dirt = 4,
}

impl RoadSurface {
    pub const ALL: [RoadSurface; 5] = [
        RoadSurface::Asphalt,
        RoadSurface::WornAsphalt,
        RoadSurface::Gravel,
        RoadSurface::Paving,
        RoadSurface::Dirt,
    ];

    /// The usual look of a road category without OSM tags: tracks and
    /// paths gravel, everything else asphalt (like [`Surface::of_class`]).
    pub fn of_class(class: RoadClass) -> Self {
        match class {
            RoadClass::Track | RoadClass::Path => RoadSurface::Gravel,
            _ => RoadSurface::Asphalt,
        }
    }

    /// From the OSM tags of a way: `surface` first (refined by
    /// `smoothness` for asphalt), then `tracktype` on tracks, then the
    /// category. `tag` returns a tag's value.
    pub fn from_osm<'a>(class: RoadClass, tag: impl Fn(&str) -> Option<&'a str>) -> Self {
        if let Some(s) = tag("surface").and_then(|v| Self::from_surface_tag(v.trim())) {
            let worn = matches!(
                tag("smoothness").map(str::trim),
                Some("intermediate" | "bad" | "very_bad" | "horrible" | "very_horrible")
            );
            return if s == RoadSurface::Asphalt && worn {
                RoadSurface::WornAsphalt
            } else {
                s
            };
        }
        if class == RoadClass::Track {
            match tag("tracktype").map(str::trim) {
                Some("grade1") => return RoadSurface::WornAsphalt,
                Some("grade2" | "grade3") => return RoadSurface::Gravel,
                Some("grade4" | "grade5") => return RoadSurface::Dirt,
                _ => {}
            }
        }
        Self::of_class(class)
    }

    /// An OSM `surface` value (`None`: unknown, use the fallback). The
    /// first value of a `;` list counts.
    pub fn from_surface_tag(value: &str) -> Option<Self> {
        let v = value.split(';').next().unwrap_or("").trim();
        Some(match v {
            "asphalt" | "paved" | "concrete" | "concrete:plates" | "concrete:lanes" | "chipseal"
            | "tartan" | "rubber" | "metal" | "wood" | "metal_grid" => RoadSurface::Asphalt,
            "paving_stones" | "paving_stones:lanes" | "sett" | "unhewn_cobblestone"
            | "cobblestone" | "cobblestone:flattened" | "bricks" | "grass_paver" | "stone" => {
                RoadSurface::Paving
            }
            "gravel" | "fine_gravel" | "compacted" | "pebblestone" | "unpaved" | "rock"
            | "shells" => RoadSurface::Gravel,
            "dirt" | "earth" | "ground" | "mud" | "sand" | "grass" | "woodchips" | "soil"
            | "clay" => RoadSurface::Dirt,
            _ => return None,
        })
    }

    /// The rolling-resistance class (spec §5.2.1).
    pub fn physics(self) -> Surface {
        match self {
            RoadSurface::Asphalt | RoadSurface::WornAsphalt | RoadSurface::Paving => {
                Surface::Asphalt
            }
            RoadSurface::Gravel => Surface::Gravel,
            RoadSurface::Dirt => Surface::Grass,
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }

    /// The material id in the material library (`assets/materials.toml`).
    pub fn material_id(self) -> &'static str {
        match self {
            RoadSurface::Asphalt => "asphalt",
            RoadSurface::WornAsphalt => "asphalt_worn",
            RoadSurface::Gravel => "gravel_road",
            RoadSurface::Paving => "paving",
            RoadSurface::Dirt => "dirt_road",
        }
    }
}

/// Writes the section payload: tiles (in `tiles` order) with at least one
/// strip whose surface is not its category's default.
pub fn write_road_surfaces<'a>(
    tiles: impl Iterator<Item = (TileKey, &'a [crate::RoadStrip])>,
    b: &mut Vec<u8>,
) -> bool {
    let mut payload = Vec::new();
    let mut count = 0u32;
    for (key, strips) in tiles {
        if strips
            .iter()
            .all(|s| s.surface == RoadSurface::of_class(s.class))
        {
            continue;
        }
        count += 1;
        payload.extend(key.x.to_le_bytes());
        payload.extend(key.z.to_le_bytes());
        payload.extend((strips.len() as u32).to_le_bytes());
        payload.extend(strips.iter().map(|s| s.surface.code()));
    }
    if count == 0 {
        return false;
    }
    b.extend(count.to_le_bytes());
    b.extend(payload);
    true
}

/// Reads the section payload: per tile, the surface of each strip.
pub fn read_road_surfaces(bytes: &[u8]) -> Result<Vec<(TileKey, Vec<RoadSurface>)>, String> {
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
        let surfaces = codes
            .iter()
            .map(|&c| RoadSurface::from_code(c).ok_or_else(|| format!("unknown road surface {c}")))
            .collect::<Result<Vec<_>, _>>()?;
        out.push((key, surfaces));
    }
    r.finish("road surface section")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
        move |k| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| *v)
    }

    #[test]
    fn osm_surface_tags_map_to_materials() {
        let c = RoadClass::Residential;
        for (v, want) in [
            ("asphalt", RoadSurface::Asphalt),
            ("concrete:plates", RoadSurface::Asphalt),
            ("paving_stones", RoadSurface::Paving),
            ("sett", RoadSurface::Paving),
            ("cobblestone", RoadSurface::Paving),
            ("gravel", RoadSurface::Gravel),
            ("compacted", RoadSurface::Gravel),
            ("fine_gravel", RoadSurface::Gravel),
            ("unpaved", RoadSurface::Gravel),
            ("dirt", RoadSurface::Dirt),
            ("grass", RoadSurface::Dirt),
            ("ground", RoadSurface::Dirt),
            ("gravel;asphalt", RoadSurface::Gravel),
        ] {
            assert_eq!(
                RoadSurface::from_osm(c, tags(&[("surface", v)])),
                want,
                "{v}"
            );
        }
    }

    #[test]
    fn worn_asphalt_from_smoothness_and_fallbacks_by_category() {
        let worn = [("surface", "asphalt"), ("smoothness", "bad")];
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Service, tags(&worn)),
            RoadSurface::WornAsphalt
        );
        let good = [("surface", "asphalt"), ("smoothness", "good")];
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Service, tags(&good)),
            RoadSurface::Asphalt
        );
        // Smoothness does not turn gravel into worn asphalt.
        let g = [("surface", "gravel"), ("smoothness", "bad")];
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Track, tags(&g)),
            RoadSurface::Gravel
        );
        // No or unknown surface: track grade, then the category.
        for (tt, want) in [
            ("grade1", RoadSurface::WornAsphalt),
            ("grade2", RoadSurface::Gravel),
            ("grade4", RoadSurface::Dirt),
        ] {
            assert_eq!(
                RoadSurface::from_osm(RoadClass::Track, tags(&[("tracktype", tt)])),
                want
            );
        }
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Track, tags(&[("surface", "unicorn")])),
            RoadSurface::Gravel
        );
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Primary, tags(&[])),
            RoadSurface::Asphalt
        );
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Path, tags(&[])),
            RoadSurface::Gravel
        );
        // Tracktype only counts on tracks.
        assert_eq!(
            RoadSurface::from_osm(RoadClass::Residential, tags(&[("tracktype", "grade5")])),
            RoadSurface::Asphalt
        );
    }

    #[test]
    fn codes_ids_and_physics() {
        let mut ids = std::collections::BTreeSet::new();
        for s in RoadSurface::ALL {
            assert_eq!(RoadSurface::from_code(s.code()), Some(s));
            assert!(ids.insert(s.material_id()));
        }
        assert_eq!(RoadSurface::from_code(5), None);
        assert_eq!(RoadSurface::Paving.physics(), Surface::Asphalt);
        assert_eq!(RoadSurface::Gravel.physics(), Surface::Gravel);
        // The category fallback agrees with the physics fallback.
        for c in crate::package::ROAD_CLASSES {
            assert_eq!(RoadSurface::of_class(c).physics(), Surface::of_class(c));
        }
    }
}
