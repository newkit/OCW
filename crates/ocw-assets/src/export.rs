//! The export of today's generated models into `artwork/3dmodels/` (spec
//! §14 Phase 15, M1 / 15H): every generator of this crate writes one file
//! per model (and per variant the game uses), named by function and
//! shape, with its manifest entry. The plants are exported by
//! `ocw-render` (their generator lives there): `cargo run -p ocw-render
//! --example export-flora`. Both write through [`write_manifest`], so the
//! manifest is one sorted file whichever exporter ran last.
//!
//! Until the generators are removed (Q6), `export-all --check` and the
//! tests keep generator and files in step (the writer is deterministic,
//! 15B).

use ocw_core::ProfileId;
use serde_json::json;

use crate::helmet::Helmet;
use crate::kinds::{self, Kind};
use crate::manifest::Entry;
use crate::model::{Model, Node};
use crate::pose::{BikeRig, RiderRig, bike_nodes};
use crate::{bikes, rider};

/// The licence of own models (like `artwork/`, spec §13).
pub const LICENSE: &str = "MIT OR Apache-2.0";
pub const AUTHOR: &str = "Open Cycling World contributors";
/// `source` of every exported file.
pub const SOURCE: &str = "own work, exported from the OCW generators (spec §14 Phase 15, M1)";

/// The rider's files.
pub const RIDER: &str = "riders/rider-body.glb";
pub const RIDER_LOW: &str = "riders/rider-body-low.glb";

/// The file of `p`'s bike (15H names).
pub fn bike_file(p: ProfileId) -> &'static str {
    match p {
        ProfileId::Road => "bikes/bike-road-endurance.glb",
        ProfileId::RoadAero => "bikes/bike-road-aero.glb",
        ProfileId::RoadClimbing => "bikes/bike-road-climbing.glb",
        ProfileId::TimeTrial => "bikes/bike-tt.glb",
        ProfileId::Gravel => "bikes/bike-gravel.glb",
        ProfileId::Mountain => "bikes/bike-mtb-hardtail.glb",
        ProfileId::Folding => "bikes/bike-folding.glb",
    }
}

/// The file of a helmet style.
pub fn helmet_file(h: Helmet) -> &'static str {
    match h {
        Helmet::Road => "wear/helmet-road-vented.glb",
        Helmet::Light => "wear/helmet-road-lightweight.glb",
        Helmet::AeroRoad => "wear/helmet-aero-road.glb",
        Helmet::TimeTrial => "wear/helmet-tt-teardrop.glb",
    }
}

/// The rider node the helmets are worn at.
pub const HEAD_ANCHOR: &str = crate::pose::nodes::HEAD;

/// A manifest entry for an exported file.
pub fn entry(file: &str, kind: Kind, generator: &str) -> Entry {
    Entry {
        file: file.into(),
        kind,
        profile: None,
        species: None,
        variant: None,
        anchor: None,
        generator: Some(generator.into()),
        license: LICENSE.into(),
        source: SOURCE.into(),
        author: AUTHOR.into(),
    }
}

/// Stamps a model with the contract's metadata (15B): copyright,
/// generator, `extras.ocw = { schema, kind, id, … }` (plus `more`).
pub fn stamp(m: &mut Model, e: &Entry, more: serde_json::Value) {
    m.copyright = crate::COPYRIGHT.into();
    m.generator = crate::GENERATOR.into();
    let mut ocw = json!({
        "schema": kinds::SCHEMA,
        "kind": e.kind.name(),
        "id": e.id(),
    });
    if let (Some(o), serde_json::Value::Object(more)) = (ocw.as_object_mut(), more) {
        o.extend(more);
    }
    m.extras = json!({ "ocw": ocw });
}

/// The frame's geometry numbers and the sizing data a bike file carries
/// (Q7: the loader fits the exported frame to the rider's size; no new
/// file per size).
pub fn bike_extras(p: ProfileId, rig: &BikeRig) -> serde_json::Value {
    let d = bikes::design(p);
    let r = |v: f32| (v * 10_000.0).round() / 10_000.0;
    json!({
        "profile": p.id(),
        "frame": {
            "stack_m": r(d.stack),
            "reach_m": r(d.reach),
            "seat_angle_deg": r(d.seat_angle),
            "head_angle_deg": r(d.head_angle),
            "head_tube_m": r(d.head_tube),
            "chainstay_m": r(d.chainstay),
            "bb_drop_m": r(d.bb_drop),
            "wheel_radius_m": r(d.wheel_radius),
            "crank_m": r(d.crank),
            "stem_m": r(d.stem),
            "saddle_height_m": r(rig.saddle.distance(rig.crank)),
            "wheelbase_m": r(rig.wheelbase),
        },
        "sizing": {
            "reference_rider_height_m": crate::pose::REFERENCE_HEIGHT_M,
            "saddle_travel_m": crate::pose::MAX_SADDLE_OFFSET_M,
            "saddle_node": bike_nodes::SADDLE,
        },
    })
}

/// A helmet as its own file (15E: exported once at the reference skull;
/// the fit is the anchor's transform): the root `helmet` holds the near
/// shell in the head's frame, `lod1` the far one.
pub fn helmet_model(h: Helmet, near: &Model, far: &Model) -> Model {
    let mut m = Model::default();
    let root = m.add(Node::new("helmet", None, glam::Vec3::ZERO));
    let lod = m.add(Node::new(bike_nodes::LOD1, Some(root), glam::Vec3::ZERO));
    for (src, node) in [(near, root), (far, lod)] {
        let i = src.node(h.node()).expect("helmet node");
        for p in &src.nodes[i].primitives {
            let mat = m.material_or_add(src.materials[p.material].clone());
            let mut q = p.clone();
            q.material = mat;
            m.nodes[node].primitives.push(q);
        }
    }
    m
}

/// Every model this crate exports: (manifest entry, model).
pub fn models() -> Vec<(Entry, Model)> {
    let near = rider::build(rider::Detail::High);
    let low = rider::build(rider::Detail::Low);
    let mut out = Vec::new();
    for p in ProfileId::ALL {
        let mut m = bikes::build(p, &near, &low);
        let rig = BikeRig::from_model(&m).expect("bike rig");
        let mut e = entry(
            bike_file(p),
            Kind::Bike,
            &format!("ocw_assets::bikes::build({})", p.id()),
        );
        e.profile = Some(p.id().into());
        stamp(&mut m, &e, bike_extras(p, &rig));
        out.push((e, m));
    }
    // The near rider without its part files (spec §14 Phase 4, 4N N8) and
    // both riders without the helmets: those are worn from their own
    // files (Phase 15 M3).
    let near_file = crate::rider_split::strip(&near);
    let low_file = crate::rider_split::strip_low(&low);
    for (file, detail, src) in [(RIDER, "High", &near_file), (RIDER_LOW, "Low", &low_file)] {
        let mut m = src.clone();
        let e = entry(
            file,
            Kind::Rider,
            &format!("ocw_assets::rider::build(Detail::{detail})"),
        );
        let rig = RiderRig::from_model(&m).expect("rider rig");
        stamp(
            &mut m,
            &e,
            json!({ "height_m": crate::pose::REFERENCE_HEIGHT_M, "leg_m": (rig.leg() * 10_000.0).round() / 10_000.0 }),
        );
        out.push((e, m));
    }
    // The part files: hair pieces, facial hair, the base bodies' parts,
    // each at its anchor on the rider.
    for p in crate::rider_split::part_files() {
        let mut m = crate::rider_split::extract(&near, p.node).expect("a part node");
        let anchor = crate::rider_split::anchor(&m).expect("a root").to_string();
        let mut e = entry(
            &p.file,
            Kind::Wearable,
            &format!(
                "ocw_assets::rider_split::extract(rider::build(Detail::High), {:?})",
                p.node
            ),
        );
        e.anchor = Some(anchor.clone());
        stamp(
            &mut m,
            &e,
            json!({ "anchor": anchor, "rider_node": p.node }),
        );
        out.push((e, m));
    }
    for h in Helmet::ALL {
        let mut m = helmet_model(h, &near, &low);
        let mut e = entry(
            helmet_file(h),
            Kind::Wearable,
            &format!("ocw_assets::helmet::build(Helmet::{h:?})"),
        );
        e.anchor = Some(HEAD_ANCHOR.into());
        stamp(
            &mut m,
            &e,
            json!({ "anchor": HEAD_ANCHOR, "rider_node": h.node() }),
        );
        out.push((e, m));
    }
    // The unicorn (Phase 15 M4): its joints are the nodes the flight
    // animation turns.
    {
        let mut m = crate::unicorn::model();
        let e = entry(
            crate::unicorn::FILE,
            Kind::Creature,
            "ocw_assets::unicorn::model()",
        );
        stamp(
            &mut m,
            &e,
            json!({ "standing_height_m": crate::unicorn::STANDING_HEIGHT_M }),
        );
        out.push((e, m));
    }
    // The cow of the UFOs' beam scene (spec 18E T9).
    {
        let m = crate::cow::model();
        out.push((crate::cow::entry(), m));
    }
    // The workshop's pieces (Phase 15 M3): `workshop::room` places them
    // for the number of bikes.
    for p in crate::workshop::Piece::ALL {
        let mut m = p.model();
        let e = entry(
            p.file(),
            Kind::Workshop,
            &format!("ocw_assets::workshop::Piece::{p:?}.model()"),
        );
        stamp(&mut m, &e, json!({}));
        out.push((e, m));
    }
    // The VR bike computer (spec §14 Phase 11, v4.68 on `agent/vr-x1`;
    // WebXR 14I W2).
    {
        let mut m = crate::computer::build();
        let e = entry(
            crate::computer::FILE,
            Kind::Part,
            "ocw_assets::computer::build()",
        );
        stamp(
            &mut m,
            &e,
            json!({ "slot": "computer", "anchor": crate::computer::MOUNT_ANCHOR, "screen": crate::computer::SCREEN_ANCHOR }),
        );
        out.push((e, m));
    }
    // The traffic (spec 14 Phase 18, 18E T6).
    for v in crate::vehicles::Vehicle::ALL {
        out.push((v.entry(), v.model()));
    }
    out
}

/// [`models`] as files: (path in `artwork/3dmodels/`, bytes).
pub fn files() -> Vec<(Entry, Vec<u8>)> {
    models()
        .into_iter()
        .map(|(e, m)| {
            let bytes = crate::glb::write(&m);
            (e, bytes)
        })
        .collect()
}

/// The manifest's header (the contract is in the folder's README).
pub const MANIFEST_HEADER: &str = "\
# The model files of Open Cycling World (spec §14 Phase 15, 15B / 15C):
# one [[asset]] per file below this folder, its path as `file` (the id is
# the path without `.glb`). Every file needs `license`, `source` and
# `author` (spec §13); the validator (`cargo run -p ocw-assets -- validate
# artwork/3dmodels`) rejects anything else, physics values included
# (cosmetic models only name a physics profile).
#
# Written by the exporters (`cargo run -p ocw-assets -- model export-all`,
# `cargo run -p ocw-render --example export-flora`); keep it sorted.
";

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The manifest text of `entries` (sorted by file).
pub fn manifest_text(entries: &[Entry]) -> String {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.file.cmp(&b.file));
    let mut out = String::from(MANIFEST_HEADER);
    for e in sorted {
        out.push_str("\n[[asset]]\n");
        out.push_str(&format!("file = {}\n", toml_str(&e.file)));
        out.push_str(&format!("kind = {}\n", toml_str(e.kind.name())));
        for (k, v) in [
            ("profile", &e.profile),
            ("species", &e.species),
            ("variant", &e.variant),
            ("anchor", &e.anchor),
            ("generator", &e.generator),
        ] {
            if let Some(v) = v {
                out.push_str(&format!("{k} = {}\n", toml_str(v)));
            }
        }
        out.push_str(&format!("license = {}\n", toml_str(&e.license)));
        out.push_str(&format!("source = {}\n", toml_str(&e.source)));
        out.push_str(&format!("author = {}\n", toml_str(&e.author)));
    }
    out
}

/// The manifest `old` with every exported entry (one with a `generator`)
/// whose file starts with one of `prefixes` replaced by `new`. Files made
/// as files (no generator, spec §14 Phase 15 Q6: the file is the source,
/// e.g. the status signs of §2.6 in `wear/` and `props/`) are kept.
pub fn write_manifest(old: &str, prefixes: &[&str], new: &[Entry]) -> Result<String, Vec<String>> {
    let old = if old.trim().is_empty() {
        Vec::new()
    } else {
        crate::manifest::Manifest::parse(old)?.assets
    };
    let mut all: Vec<Entry> = old
        .into_iter()
        .filter(|e| e.generator.is_none() || !prefixes.iter().any(|p| e.file.starts_with(p)))
        .collect();
    all.extend(new.iter().cloned());
    Ok(manifest_text(&all))
}

/// The folders this crate's exporter owns in the manifest.
pub const OWN_PREFIXES: [&str; 7] = [
    "bikes/",
    "creatures/",
    "parts/computer/",
    "riders/",
    "vehicles/",
    "wear/",
    "workshop/",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_is_deterministic_and_named_by_the_contract() {
        let a = files();
        let b = files();
        // Bikes, the riders, the rider's part files (4N N8), helmets.
        // … the unicorn, the workshop pieces and the VR bike computer.
        // … the traffic and the UFOs' cow.
        let more =
            1 + crate::workshop::Piece::ALL.len() + 1 + crate::vehicles::Vehicle::ALL.len() + 1;
        assert_eq!(
            a.len(),
            ProfileId::ALL.len() + 2 + crate::rider_split::part_files().len() + 4 + more
        );
        for ((ea, ba), (eb, bb)) in a.iter().zip(&b) {
            assert_eq!(ea, eb);
            assert_eq!(ba, bb, "{}", ea.file);
            kinds::check_name(&ea.file, ea.kind).unwrap();
        }
        let text = manifest_text(&a.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>());
        let files: Vec<(String, Vec<u8>)> = a.into_iter().map(|(e, b)| (e.file, b)).collect();
        let checked = crate::manifest::validate(&text, &files).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(
            checked.len(),
            ProfileId::ALL.len() + 6 + crate::rider_split::part_files().len() + more
        );
    }

    #[test]
    fn manifests_merge_by_folder() {
        let own: Vec<Entry> = models().into_iter().map(|(e, _)| e).collect();
        let mut plant = entry("flora/tree-test-round-a.glb", Kind::Plant, "test");
        plant.species = Some("Beech".into());
        let first = write_manifest("", &["flora/"], std::slice::from_ref(&plant)).unwrap();
        let both = write_manifest(&first, &OWN_PREFIXES, &own).unwrap();
        let again = write_manifest(&both, &OWN_PREFIXES, &own).unwrap();
        assert_eq!(both, again);
        // A file made as a file (no generator) in an exporter's folder
        // stays (spec §2.6's status signs in `wear/`).
        let mut made = entry("wear/headphones-over-ear.glb", Kind::Wearable, "x");
        made.generator = None;
        made.anchor = Some("head".into());
        let with = write_manifest(&both, &[], std::slice::from_ref(&made)).unwrap();
        let kept = write_manifest(&with, &OWN_PREFIXES, &own).unwrap();
        assert!(kept.contains("wear/headphones-over-ear.glb"));
        let m = crate::manifest::Manifest::parse(&both).unwrap();
        assert_eq!(m.assets.len(), own.len() + 1);
        assert!(m.assets.windows(2).all(|w| w[0].file < w[1].file));
    }

    /// A helmet file is the rider's helmet node: worn at the head anchor
    /// it is the same mesh.
    #[test]
    fn helmet_files_are_the_riders_helmets() {
        let near = rider::build(rider::Detail::High);
        let low = rider::build(rider::Detail::Low);
        for h in Helmet::ALL {
            let m = helmet_model(h, &near, &low);
            let i = near.node(h.node()).unwrap();
            assert_eq!(near.nodes[i].translation, glam::Vec3::ZERO);
            assert_eq!(near.nodes[i].parent, near.node(HEAD_ANCHOR));
            let tris: usize = near.nodes[i].primitives.iter().map(|p| p.triangles()).sum();
            let lod = m.node(bike_nodes::LOD1).unwrap();
            assert_eq!(m.triangles() - m.triangles_below(Some(lod)), tris, "{h:?}");
            assert!(m.triangles_below(Some(lod)) > 0, "{h:?}");
            assert!(tris <= crate::helmet::MAX_TRIANGLES, "{h:?}");
        }
    }
}
