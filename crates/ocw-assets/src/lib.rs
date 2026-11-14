//! Bike and rider 3D assets (spec §2.5, §2.6, §8): Bevy-free.
//!
//! - [`bikes`], [`rider`]: the generators of the shipped low-poly glTF
//!   models (own work, MIT OR Apache-2.0), built from [`mesh`] primitives;
//! - [`glb`]: writing and reading `.glb` files ([`model::Model`]);
//! - [`manifest`]: the asset manifest and the validator (licence entries,
//!   no physics in cosmetic assets, required nodes, triangle budgets);
//! - [`pose`]: rider on bike – anchors, inverse kinematics;
//! - [`anim`]: riding state selection from cadence / speed / power / grade;
//! - [`shipped`]: the models in `assets/models/` and the parts catalogue,
//!   embedded;
//! - [`parts`]: the parts catalogue of the workshop, its price sources and
//!   validator (spec §14 Phase 4, 4H–4J);
//! - [`licence`]: the licence rule for every shipped asset (spec Phase 8A);
//! - [`materials`]: the material library – pinned sources, validator,
//!   credits, loading (spec §4.6, Phase 8 T1); [`ktx2`] its texture files;
//!   with the `tools` feature [`convert`] (source maps → KTX2) and
//!   [`fetch`] (download, check, convert).

pub mod anim;
pub mod bikes;
#[cfg(feature = "tools")]
pub mod convert;
#[cfg(feature = "tools")]
pub mod fetch;
pub mod glb;
pub mod ktx2;
pub mod licence;
pub mod manifest;
pub mod materials;
pub mod mesh;
pub mod model;
pub mod parts;
pub mod pose;
pub mod rider;
pub mod slots;
pub mod workshop;

use ocw_core::ProfileId;

/// Who made the generated assets, and their licence (spec §13: own
/// artwork like `artwork/` is MIT OR Apache-2.0).
pub const COPYRIGHT: &str = "Open Cycling World contributors; licence MIT OR Apache-2.0";
pub const GENERATOR: &str = "ocw-assets (Open Cycling World)";

/// File name of the bike model of `p`.
pub fn bike_file(p: ProfileId) -> String {
    format!("bike-{}.glb", p.id())
}

pub const RIDER_FILE: &str = "rider.glb";

/// Generates every shipped model: `(file name, .glb bytes)`.
pub fn generate() -> Vec<(String, Vec<u8>)> {
    let mut rider = rider::build(rider::Detail::High);
    let low = rider::build(rider::Detail::Low);
    let mut out = Vec::new();
    for p in ProfileId::ALL {
        let mut m = bikes::build(p, &rider, &low);
        m.copyright = COPYRIGHT.into();
        m.generator = GENERATOR.into();
        out.push((bike_file(p), glb::write(&m)));
    }
    rider.copyright = COPYRIGHT.into();
    rider.generator = GENERATOR.into();
    out.push((RIDER_FILE.into(), glb::write(&rider)));
    out
}

/// The models shipped in `assets/models/` (embedded in the binary: no
/// asset paths to find at runtime, on any platform).
pub mod shipped {
    use std::sync::OnceLock;

    use ocw_core::ProfileId;

    use crate::model::Model;
    use crate::pose::{BikeRig, RiderRig};

    macro_rules! asset {
        ($f:literal) => {
            (
                $f,
                include_bytes!(concat!("../../../assets/models/", $f)) as &[u8],
            )
        };
    }

    /// The manifest (licence entries) of the shipped models.
    pub const MANIFEST: &str = include_str!("../../../assets/models/manifest.toml");

    /// The parts catalogue (spec §14 Phase 4, 4I) and its price
    /// references (4H).
    pub const PARTS: &str = include_str!("../../../assets/parts.toml");
    pub const PARTS_SOURCES: &str = include_str!("../../../assets/parts-sources.toml");

    /// The shipped parts catalogue (parsed once).
    pub fn catalogue() -> &'static crate::parts::Catalogue {
        static C: OnceLock<crate::parts::Catalogue> = OnceLock::new();
        C.get_or_init(|| {
            crate::parts::Catalogue::parse(PARTS)
                .unwrap_or_else(|e| panic!("shipped parts.toml: {e:?}"))
        })
    }

    /// The shipped price references (parsed once).
    pub fn parts_sources() -> &'static crate::parts::Sources {
        static S: OnceLock<crate::parts::Sources> = OnceLock::new();
        S.get_or_init(|| {
            crate::parts::Sources::parse(PARTS_SOURCES)
                .unwrap_or_else(|e| panic!("shipped parts-sources.toml: {e:?}"))
        })
    }

    /// Every shipped model file.
    pub const FILES: [(&str, &[u8]); 7] = [
        asset!("bike-road.glb"),
        asset!("bike-road-aero.glb"),
        asset!("bike-road-climbing.glb"),
        asset!("bike-tt.glb"),
        asset!("bike-gravel.glb"),
        asset!("bike-mtb.glb"),
        asset!("rider.glb"),
    ];

    fn bytes(name: &str) -> &'static [u8] {
        FILES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
            .unwrap_or_else(|| panic!("no shipped asset {name}"))
    }

    fn load(name: &str) -> Model {
        crate::glb::read(bytes(name)).unwrap_or_else(|e| panic!("shipped asset {name}: {e}"))
    }

    /// The rider model.
    pub fn rider_model() -> &'static Model {
        static M: OnceLock<Model> = OnceLock::new();
        M.get_or_init(|| load(crate::RIDER_FILE))
    }

    /// The bike model of `p`.
    pub fn bike_model(p: ProfileId) -> &'static Model {
        static M: OnceLock<Vec<Model>> = OnceLock::new();
        let all = M.get_or_init(|| {
            ProfileId::ALL
                .iter()
                .map(|p| load(&crate::bike_file(*p)))
                .collect()
        });
        let i = ProfileId::ALL
            .iter()
            .position(|q| *q == p)
            .expect("profile");
        &all[i]
    }

    pub fn rider_rig() -> RiderRig {
        RiderRig::from_model(rider_model()).expect("shipped rider rig")
    }

    /// The rig of `p`'s bike model (read once).
    pub fn bike_rig(p: ProfileId) -> BikeRig {
        static R: OnceLock<Vec<BikeRig>> = OnceLock::new();
        let all = R.get_or_init(|| {
            ProfileId::ALL
                .iter()
                .map(|p| BikeRig::from_model(bike_model(*p)).expect("shipped bike rig"))
                .collect()
        });
        let i = ProfileId::ALL
            .iter()
            .position(|q| *q == p)
            .expect("profile");
        all[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped assets pass the validator, one bike per profile.
    #[test]
    fn shipped_assets_are_valid_and_complete() {
        let files: Vec<(String, Vec<u8>)> = shipped::FILES
            .iter()
            .map(|(n, b)| (n.to_string(), b.to_vec()))
            .collect();
        let checked =
            manifest::validate(shipped::MANIFEST, &files).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(checked.len(), 7);
        for p in ProfileId::ALL {
            let n = checked
                .iter()
                .filter(|c| c.entry.profile.as_deref() == Some(p.id()))
                .count();
            assert_eq!(n, 1, "{p:?}");
            assert_eq!(shipped::bike_rig(p).aero_bars, p == ProfileId::TimeTrial);
        }
        assert!(shipped::rider_rig().leg() > 0.8);
    }

    /// `assets/models/` is what the generator makes: run
    /// `cargo run -p ocw-assets -- generate` after changing it.
    #[test]
    fn shipped_assets_match_the_generator() {
        for (name, bytes) in generate() {
            let new = glb::read(&bytes).unwrap();
            let (_, old) = shipped::FILES
                .iter()
                .find(|(n, _)| *n == name)
                .expect("shipped");
            let old = glb::read(old).unwrap();
            let stale = format!("{name} is stale: run `cargo run -p ocw-assets -- generate`");
            assert_eq!(new.nodes.len(), old.nodes.len(), "{stale}");
            assert_eq!(new.materials, old.materials, "{stale}");
            for (a, b) in new.nodes.iter().zip(&old.nodes) {
                assert_eq!((&a.name, a.parent), (&b.name, b.parent), "{stale}");
                assert!(
                    a.translation.distance(b.translation) < 1e-4,
                    "{stale}: {}",
                    a.name
                );
                assert_eq!(
                    a.primitives.len(),
                    b.primitives.len(),
                    "{stale}: {}",
                    a.name
                );
                for (p, q) in a.primitives.iter().zip(&b.primitives) {
                    assert_eq!(p.indices, q.indices, "{stale}: {}", a.name);
                    let far =
                        p.positions.iter().zip(&q.positions).any(|(x, y)| {
                            glam::Vec3::from(*x).distance(glam::Vec3::from(*y)) > 1e-4
                        });
                    assert!(!far, "{stale}: {}", a.name);
                }
            }
        }
    }
}
