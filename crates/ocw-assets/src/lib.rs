//! Bike and rider 3D assets (spec §2.5, §2.6, §8): Bevy-free.
//!
//! - [`bikes`], [`rider`]: the generators of the low-poly glTF models (own
//!   work, MIT OR Apache-2.0), built from [`mesh`] primitives; [`helmet`]:
//!   the rider's helmets (sculpted shells, vents, visor);
//! - [`export`]: their export into `artwork/3dmodels/` (spec §14 Phase 15,
//!   M1), [`kinds`] the model contract per kind (15B), [`library`] the
//!   run-time loader of those files (embedded or a dev folder, 15D);
//! - [`glb`]: writing and reading `.glb` / `.gltf` files ([`model::Model`]);
//! - [`check`]: the extended validator's findings (codes, targets, fixes;
//!   spec §14 Phase 15, M2), [`diff`] the structural diff of two models,
//!   [`import`] a glTF from Blender or any tool into the canonical file and
//!   the editable export;
//! - [`manifest`]: the model manifest and the validator (licence entries,
//!   no physics in cosmetic assets, required nodes, names, budgets);
//! - [`pose`]: rider on bike – anchors, inverse kinematics, the fit to the
//!   rider's size;
//! - [`anim`]: riding state selection from cadence / speed / power / grade;
//! - [`shipped`]: the shipped models (from [`library`]) and the parts
//!   catalogue, embedded;
//! - [`parts`]: the parts catalogue of the workshop, its price sources and
//!   validator (spec §14 Phase 4, 4H–4J);
//! - [`licence`]: the licence rule for every shipped asset (spec Phase 8A);
//! - [`materials`]: the material library – pinned sources, validator,
//!   credits, loading (spec §4.6, Phase 8 T1); [`ktx2`] its texture files;
//!   with the `tools` feature [`convert`] (source maps → KTX2) and
//!   [`fetch`] (download, check, convert).

pub mod anim;
pub mod bikes;
pub mod check;
pub mod clothing;
pub mod computer;
#[cfg(feature = "tools")]
pub mod convert;
pub mod cow;
pub mod diff;
pub mod export;
#[cfg(feature = "tools")]
pub mod fetch;
pub mod glb;
pub mod helmet;
pub mod hips;
pub mod import;
pub mod kinds;
pub mod ktx2;
pub mod library;
pub mod livery;
pub mod licence;
pub mod manifest;
pub mod materials;
pub mod mesh;
pub mod model;
pub mod mouth;
pub mod parts;
pub mod pose;
pub mod props;
pub mod pylon;
pub mod rider;
pub mod rider_split;
pub mod slots;
pub mod unicorn;
pub mod vehicles;
pub mod workshop;

use ocw_core::ProfileId;

/// Who made the generated assets, and their licence (spec §13: own
/// artwork like `artwork/` is MIT OR Apache-2.0).
pub const COPYRIGHT: &str = "Open Cycling World contributors; licence MIT OR Apache-2.0";
pub const GENERATOR: &str = "ocw-assets (Open Cycling World)";

/// The model id of `p`'s bike (`bikes/…`, spec §14 Phase 15, 15H).
pub fn bike_id(p: ProfileId) -> &'static str {
    kinds::id_of(export::bike_file(p))
}

/// The model id of the rider (near) and of its far model.
pub const RIDER_ID: &str = "riders/rider-body";
pub const RIDER_LOW_ID: &str = "riders/rider-body-low";

/// The shipped models (read from [`library::library`]: embedded, or a dev
/// folder) and the parts catalogue (embedded: no asset paths to find at
/// runtime, on any platform).
pub mod shipped {
    use std::sync::{Arc, OnceLock};

    use ocw_core::ProfileId;

    use crate::model::Model;
    use crate::pose::{BikeRig, BikeSizing, RiderRig};

    /// The manifest (licence entries) of the shipped models.
    pub const MANIFEST: &str = crate::library::MANIFEST;

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

    /// Every embedded model file: (path in `artwork/3dmodels/`, bytes).
    pub fn files() -> &'static [(&'static str, &'static [u8])] {
        crate::library::embedded_files()
    }

    /// The model `id` of the app's library (a missing shipped model is a
    /// build error, so this panics).
    pub fn model(id: &str) -> Arc<Model> {
        crate::library::library()
            .get(id)
            .unwrap_or_else(|e| panic!("shipped model: {e}"))
    }

    /// The helmets as parts at the head (spec §14 Phase 15 M3): each
    /// style's file `wear/helmet-*.glb`, its near or far shell.
    fn helmet_parts(detail: crate::rider::Detail) -> Vec<Model> {
        crate::helmet::Helmet::ALL
            .iter()
            .map(|h| {
                let file = model(crate::kinds::id_of(crate::export::helmet_file(*h)));
                crate::rider_split::helmet_part(&file, *h, detail)
                    .unwrap_or_else(|| panic!("shipped helmet {h:?}: no {detail:?} shell"))
            })
            .collect()
    }

    /// The rider model: the near rider file with every part file (hair,
    /// facial hair, the base bodies' parts; spec §14 Phase 4, 4N N8) and
    /// the helmet files (Phase 15 M3) attached under their anchors
    /// ([`crate::rider_split::merge`]).
    pub fn rider_model() -> &'static Model {
        static M: OnceLock<Model> = OnceLock::new();
        M.get_or_init(|| {
            let base = model(crate::RIDER_ID);
            let parts: Vec<Arc<Model>> = crate::rider_split::part_files()
                .iter()
                .map(|p| model(crate::kinds::id_of(&p.file)))
                .collect();
            let helmets = helmet_parts(crate::rider::Detail::High);
            let refs: Vec<&Model> = parts
                .iter()
                .map(|m| m.as_ref())
                .chain(helmets.iter())
                .collect();
            crate::rider_split::merge(&base, &refs)
                .unwrap_or_else(|e| panic!("shipped rider parts: {e}"))
        })
    }

    /// The rider's far model (the bikes' `lod1` riders are built from it):
    /// the far rider file with the helmet files' far shells at the head.
    pub fn rider_low_model() -> &'static Model {
        static M: OnceLock<Model> = OnceLock::new();
        M.get_or_init(|| {
            let base = model(crate::RIDER_LOW_ID);
            let helmets = helmet_parts(crate::rider::Detail::Low);
            let refs: Vec<&Model> = helmets.iter().collect();
            crate::rider_split::merge(&base, &refs)
                .unwrap_or_else(|e| panic!("shipped far rider helmets: {e}"))
        })
    }

    /// The bike model of `p`.
    pub fn bike_model(p: ProfileId) -> &'static Model {
        static M: OnceLock<Vec<Arc<Model>>> = OnceLock::new();
        let all = M.get_or_init(|| {
            ProfileId::ALL
                .iter()
                .map(|p| model(crate::bike_id(*p)))
                .collect()
        });
        &all[index(p)]
    }

    fn index(p: ProfileId) -> usize {
        ProfileId::ALL
            .iter()
            .position(|q| *q == p)
            .expect("profile")
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
        all[index(p)]
    }

    /// How `p`'s bike file is fitted to the rider's size (Q7).
    pub fn bike_sizing(p: ProfileId) -> BikeSizing {
        BikeSizing::from_model(bike_model(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::{BodyFit, EXTENSION_SEATED, RiderRig, leg_extension};

    /// The shipped models pass the validator, one bike per profile.
    #[test]
    fn shipped_assets_are_valid_and_complete() {
        let files: Vec<(String, Vec<u8>)> = shipped::files()
            .iter()
            .map(|(n, b)| (n.to_string(), b.to_vec()))
            .collect();
        let checked =
            manifest::validate(shipped::MANIFEST, &files).unwrap_or_else(|e| panic!("{e:#?}"));
        for p in ProfileId::ALL {
            // The profile's own bike, and named bikes on its frame (the
            // "newk it" bike on road-aero, spec 14 Phase 15).
            let files: Vec<&str> = checked
                .iter()
                .filter(|c| c.entry.profile.as_deref() == Some(p.id()))
                .map(|c| c.entry.file.as_str())
                .collect();
            assert!(files.contains(&export::bike_file(p)), "{p:?}");
            let named = usize::from(p == ProfileId::RoadAero);
            assert_eq!(files.len(), 1 + named, "{p:?}: {files:?}");
            assert_eq!(shipped::bike_rig(p).aero_bars, p == ProfileId::TimeTrial);
        }
        // The "newk it" bike rides like every road-aero bike: the same rig.
        let newkit = shipped::model(kinds::id_of(livery::FILE));
        let rig = pose::BikeRig::from_model(&newkit).expect("newk it rig");
        let aero = shipped::bike_rig(ProfileId::RoadAero);
        assert!(rig.saddle.distance(aero.saddle) < 1e-4);
        assert!(rig.hoods[1].distance(aero.hoods[1]) < 1e-4);
        assert!((rig.wheelbase - aero.wheelbase).abs() < 1e-4);
        assert!(shipped::rider_rig().leg() > 0.8);
        assert!(RiderRig::from_model(shipped::rider_low_model()).is_ok());
    }

    fn same_mesh(name: &str, a: &model::Model, b: &model::Model) {
        let stale = format!("{name} is stale: run `cargo run -p ocw-assets -- model export-all`");
        assert_eq!(a.nodes.len(), b.nodes.len(), "{stale}");
        assert_eq!(a.materials.len(), b.materials.len(), "{stale}");
        for (x, y) in a.materials.iter().zip(&b.materials) {
            assert_eq!(x.name, y.name, "{stale}");
            let off = x
                .color
                .iter()
                .zip(&y.color)
                .any(|(u, v)| (u - v).abs() > 1e-4);
            assert!(!off, "{stale}: material {}", x.name);
            assert!(
                (x.metallic - y.metallic).abs() < 1e-6 && (x.roughness - y.roughness).abs() < 1e-6
            );
        }
        for (x, y) in a.nodes.iter().zip(&b.nodes) {
            assert_eq!((&x.name, x.parent), (&y.name, y.parent), "{stale}");
            assert!(
                x.translation.distance(y.translation) < 1e-4,
                "{stale}: {}",
                x.name
            );
            assert!(
                x.rotation.angle_between(y.rotation) < 1e-3,
                "{stale}: {}",
                x.name
            );
            assert_eq!(
                x.primitives.len(),
                y.primitives.len(),
                "{stale}: {}",
                x.name
            );
            for (p, q) in x.primitives.iter().zip(&y.primitives) {
                assert_eq!(p.indices, q.indices, "{stale}: {}", x.name);
                assert_eq!(p.material, q.material, "{stale}: {}", x.name);
                // Positions within a few steps of the writer's 1/65536 m
                // grid (sin / cos differ in the last bits between
                // platforms), normals within its 1/32767.
                let far = p
                    .positions
                    .iter()
                    .zip(&q.positions)
                    .any(|(u, v)| glam::Vec3::from(*u).distance(glam::Vec3::from(*v)) > 1e-4);
                assert!(!far, "{stale}: {} positions", x.name);
                let bent = p
                    .normals
                    .iter()
                    .zip(&q.normals)
                    .any(|(u, v)| glam::Vec3::from(*u).distance(glam::Vec3::from(*v)) > 1e-4);
                assert!(!bent, "{stale}: {} normals", x.name);
            }
        }
    }

    /// The files in `artwork/3dmodels/` are what the generators make
    /// (mesh by mesh, within the file's precision; byte for byte is
    /// `model export-all --check` on the export machine, marge – other
    /// machines' sin / cos may move a vertex by a grid step): `cargo run -p ocw-assets -- model
    /// export-all` after changing a generator.
    #[test]
    fn shipped_models_match_the_generators() {
        for (e, model) in export::models() {
            let (_, bytes) = shipped::files()
                .iter()
                .find(|(n, _)| *n == e.file)
                .unwrap_or_else(|| panic!("{} not shipped", e.file));
            let file = glb::read(bytes).unwrap();
            same_mesh(&e.file, &model, &file);
            let listed = library::ModelLibrary::embedded()
                .entry(e.id())
                .cloned()
                .unwrap_or_else(|| panic!("{} not in the manifest", e.file));
            assert_eq!(listed, e);
        }
    }

    /// Q7: the bikes keep their sizes – the frame from the file is fitted
    /// to riders of 1.50 – 2.05 m (saddle along the seat tube within the
    /// file's travel, legs at the seated extension), and the file carries
    /// the frame's geometry numbers.
    #[test]
    fn bike_files_fit_every_rider_height() {
        let rider = shipped::rider_rig();
        for p in ProfileId::ALL {
            let m = shipped::bike_model(p);
            let rig = shipped::bike_rig(p);
            let sizing = shipped::bike_sizing(p);
            assert_eq!(sizing, pose::BikeSizing::default(), "{p:?}");
            let frame = &m.ocw().unwrap()["frame"];
            let d = bikes::design(p);
            assert!((frame["stack_m"].as_f64().unwrap() as f32 - d.stack).abs() < 1e-4);
            assert!((frame["reach_m"].as_f64().unwrap() as f32 - d.reach).abs() < 1e-4);
            let saddle = frame["saddle_height_m"].as_f64().unwrap() as f32;
            assert!(
                (saddle - rig.saddle.distance(rig.crank)).abs() < 1e-3,
                "{p:?}"
            );
            let mut last = f32::MIN;
            for cm in [150u16, 160, 170, 178, 190, 205] {
                let h = f32::from(cm) / 100.0;
                let b = BodyFit::sized(&rider, &rig, sizing.scale(h), &sizing);
                assert!(b.saddle_offset > last || cm == 150, "{p:?} {cm}");
                last = b.saddle_offset;
                assert!(b.saddle_offset.abs() < sizing.saddle_travel_m, "{p:?} {cm}");
                let ext = leg_extension(&b.rider, &b.bike, b.fit.hip_seated);
                assert!((ext - EXTENSION_SEATED).abs() < 0.01, "{p:?} {cm}: {ext}");
                // The saddle moved along the file's seat tube.
                let moved = b.bike.saddle - rig.saddle;
                assert!(moved.cross(rig.seat_dir()).length() < 1e-4, "{p:?} {cm}");
            }
        }
    }

    /// The status sign's X sits over the new mouth (2026-11-17): the
    /// centre of `wear/mark-mouth-cross` (in the head's frame) is level
    /// with the lips and in front of them, and the X is wider than they
    /// are.
    #[test]
    fn the_mouth_sign_sits_over_the_lips() {
        let rider = shipped::rider_model();
        let head = rider.node(pose::nodes::HEAD).unwrap();
        let lips_model = rider::skull_centre() + glam::Vec3::from(rider::face::MOUTH);
        let lips = rider.world(head).inverse().transform_point3(lips_model);
        let x = shipped::model("wear/mark-mouth-cross");
        let pts: Vec<glam::Vec3> = (0..x.nodes.len())
            .flat_map(|i| {
                let w = x.world(i);
                x.nodes[i]
                    .primitives
                    .iter()
                    .flat_map(move |p| {
                        p.positions
                            .iter()
                            .map(move |v| w.transform_point3(glam::Vec3::from(*v)))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let min = pts.iter().copied().reduce(glam::Vec3::min).unwrap();
        let max = pts.iter().copied().reduce(glam::Vec3::max).unwrap();
        let centre = (min + max) / 2.0;
        assert!((centre.x - lips.x).abs() < 3e-3, "{centre} vs {lips}");
        assert!((centre.y - lips.y).abs() < 5e-3, "{centre} vs {lips}");
        assert!(max.z < lips.z, "in front of the lips: {max} vs {lips}");
        let r = rider::face::MOUTH_RADII;
        assert!(max.x - min.x > 2.0 * r[0], "the X covers the lips' width");
    }

    /// Every body (spec §14 Phase 4, 4N: each of the seven shapes on each
    /// base body A / B / C, at every age) loads from the exported rider
    /// files: the nodes its look shows are in the near and the far file,
    /// the girth is finite and only widens torso and limbs.
    #[test]
    fn every_body_loads_from_the_exported_files() {
        use ocw_core::{Age, BaseBody, BodyShape, Character};
        let near = shipped::rider_model();
        let far = shipped::rider_low_model();
        assert!(RiderRig::from_model(near).is_ok());
        for base in BaseBody::ALL {
            for shape in BodyShape::ALL {
                for age in Age::ALL {
                    let c = Character {
                        base,
                        shape,
                        age,
                        hair: (shape.index() * 3 + age as usize) as u8 % 21,
                        beard: (age as u8) % 8,
                        moustache: (shape.index() as u8) % 6,
                        ..Character::default()
                    };
                    for n in rider::visible(&c, &[]) {
                        assert!(near.node(n).is_some(), "{n} for {c:?}");
                        assert!(far.node(n).is_some(), "{n} (far) for {c:?}");
                    }
                    for i in 0..near.nodes.len() {
                        let (s, t) = rider::girth_transform(near, i, &c);
                        assert!(s.is_finite() && t.is_finite() && s.y == 1.0);
                        assert!((0.6..1.6).contains(&s.x) && (0.6..1.6).contains(&s.z));
                    }
                    let head = near.node(pose::nodes::HEAD).unwrap();
                    assert_eq!(rider::girth(near, head, &c), glam::Vec3::ONE);
                }
            }
        }
    }
}
