//! The model contract per kind (spec §14 Phase 15, 15B): where a kind's
//! files live in `artwork/3dmodels/`, where its origin is, its triangle
//! budgets and the file-name pattern (15H: function first, then shape).
//! `artwork/3dmodels/README.md` has the same table for artists.

use serde::{Deserialize, Serialize};

/// What a model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A bike (one per physics profile; `profile` names it).
    Bike,
    /// The rider (shared rig).
    Rider,
    /// Worn on the rider at an anchor (helmets, glasses, caps).
    Wearable,
    /// A catalogue part, placed at a bike's `hs_<slot>` hotspot.
    Part,
    /// Placed in the world: bollards, gates, lamps, benches.
    Prop,
    /// A plant species' shape variant.
    Plant,
    /// The unicorn and other creatures.
    Creature,
    /// A piece of the workshop room.
    Workshop,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Bike,
        Kind::Rider,
        Kind::Wearable,
        Kind::Part,
        Kind::Prop,
        Kind::Plant,
        Kind::Creature,
        Kind::Workshop,
    ];

    /// The manifest's name of the kind.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bike => "bike",
            Kind::Rider => "rider",
            Kind::Wearable => "wearable",
            Kind::Part => "part",
            Kind::Prop => "prop",
            Kind::Plant => "plant",
            Kind::Creature => "creature",
            Kind::Workshop => "workshop",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The folder of `artwork/3dmodels/` (15C).
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Bike => "bikes",
            Kind::Rider => "riders",
            Kind::Wearable => "wear",
            Kind::Part => "parts",
            Kind::Prop => "props",
            Kind::Plant => "flora",
            Kind::Creature => "creatures",
            Kind::Workshop => "workshop",
        }
    }

    /// Where the model's origin is (15B).
    pub fn origin(self) -> &'static str {
        match self {
            Kind::Bike => "ground contact under the bottom bracket",
            Kind::Rider => "between the sit bones",
            Kind::Wearable => "its anchor on the rider (`head`, `face`, `foot_l`, …)",
            Kind::Part => "its hotspot `hs_<slot>` on the bike",
            Kind::Prop => "the foot centre, on the ground",
            Kind::Plant => "the stem base",
            Kind::Creature => "under the body's centre",
            Kind::Workshop => {
                "the point where `workshop::room` places it: on the floor, its wall or the ceiling"
            }
        }
    }

    /// Triangle budgets: the whole file, and its `lod1` (15B; `None`: no
    /// far model).
    pub fn budget(self) -> (usize, Option<usize>) {
        match self {
            Kind::Bike => (15_000, Some(4_000)),
            Kind::Rider => (8_000, Some(4_000)),
            // The unicorn (Phase 13C: "about 3k near, 600 far") is 3,014 / 682.
            Kind::Creature => (3_500, Some(800)),
            Kind::Wearable => (1_500, None),
            Kind::Part => (1_500, None),
            Kind::Prop => (1_500, Some(300)),
            Kind::Plant => (2_000, Some(200)),
            Kind::Workshop => (10_000, None),
        }
    }

    /// Whether a file of this kind must have a `lod1` node.
    pub fn needs_lod1(self) -> bool {
        self == Kind::Plant
    }
}

/// The `extras.ocw.schema` this code writes and reads.
pub const SCHEMA: u64 = 1;

/// The validator's warning size for one model file (15C).
pub const WARN_FILE_BYTES: usize = 2 * 1024 * 1024;

/// The id of a model file: its path in `artwork/3dmodels/` without
/// `.glb` / `.gltf`.
pub fn id_of(file: &str) -> &str {
    file.strip_suffix(".glb")
        .or_else(|| file.strip_suffix(".gltf"))
        .unwrap_or(file)
}

/// Checks a file path against the naming rule of 15H:
/// `<kind dir>/[<slot>/]<function>-<shape>[-<detail>][-<variant>].glb`,
/// lower-case ASCII letters and digits with hyphens, no colours or
/// versions in it (colours are paint slots).
pub fn check_name(file: &str, kind: Kind) -> Result<(), String> {
    let Some(stem) = file.strip_suffix(".glb") else {
        return Err("a model file ends in `.glb`".into());
    };
    let parts: Vec<&str> = stem.split('/').collect();
    let depth = if kind == Kind::Part { 3 } else { 2 };
    if parts.len() != depth || parts[0] != kind.dir() {
        let want = if kind == Kind::Part {
            format!("{}/<slot>/<name>.glb", kind.dir())
        } else {
            format!("{}/<name>.glb", kind.dir())
        };
        return Err(format!("a {} lives in {want}", kind.name()));
    }
    let name = parts[depth - 1];
    let words: Vec<&str> = name.split('-').collect();
    let plain = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    if words.len() < 2 || !words.iter().all(|w| plain(w)) {
        return Err(format!(
            "`{name}`: function first, then shape (`function-shape[-detail][-variant]`), lower-case letters and digits with hyphens"
        ));
    }
    const COLOURS: [&str; 10] = [
        "red", "green", "blue", "black", "white", "yellow", "orange", "grey", "gray", "pink",
    ];
    if let Some(w) = words.iter().find(|w| COLOURS.contains(w)) {
        return Err(format!(
            "`{name}`: no colours in names (`{w}`): colours are paint slots"
        ));
    }
    if words
        .iter()
        .any(|w| w.len() > 1 && w.starts_with('v') && w[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("`{name}`: no version numbers in names"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_function_then_shape() {
        assert!(check_name("bikes/bike-road-endurance.glb", Kind::Bike).is_ok());
        assert!(check_name("flora/tree-spruce-conical-a.glb", Kind::Plant).is_ok());
        assert!(check_name("parts/wheel/wheel-carbon-deep-60mm.glb", Kind::Part).is_ok());
        assert!(check_name("bikes/bike.glb", Kind::Bike).is_err());
        assert!(check_name("riders/bike-road.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/Bike-Road.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike-road-red.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike-road-v2.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike_road.glb", Kind::Bike).is_err());
        assert!(check_name("parts/wheel-carbon.glb", Kind::Part).is_err());
        assert_eq!(
            id_of("wear/helmet-tt-teardrop.glb"),
            "wear/helmet-tt-teardrop"
        );
    }

    #[test]
    fn every_kind_has_a_folder_and_a_budget() {
        let mut dirs: Vec<&str> = Kind::ALL.iter().map(|k| k.dir()).collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), Kind::ALL.len());
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
            assert!(k.budget().0 > 0 && !k.origin().is_empty());
        }
    }
}
