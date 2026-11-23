//! The model tools of spec §14 Phase 15, M2 on every committed model
//! (`artwork/3dmodels/`, embedded): the extended validator finds no error
//! (the warnings are listed), every model diffs empty against itself, and
//! the editable export read back – and imported again – is structurally
//! the same model (the round trip a Blender edit takes, 15F).

use ocw_assets::check::Severity;
use ocw_assets::{diff, glb, import, manifest, shipped};

fn files() -> Vec<(String, Vec<u8>)> {
    shipped::files()
        .iter()
        .map(|(n, b)| (n.to_string(), b.to_vec()))
        .collect()
}

#[test]
fn the_validator_finds_no_error_in_the_committed_models() {
    let report = manifest::report(shipped::MANIFEST, &files()).unwrap();
    let errors: Vec<String> = report.errors().map(ToString::to_string).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(report.checked.len(), files().len());
    // The warnings, listed (they do not fail).
    let mut by_code = std::collections::BTreeMap::<&str, usize>::new();
    for w in report.warnings() {
        assert_eq!(w.severity, Severity::Warning);
        *by_code.entry(w.code).or_default() += 1;
        eprintln!("{w}");
    }
    eprintln!("warnings by code: {by_code:?}");
    // Phase 15 Q10 (a): every committed model stands on its origin – the
    // plants with their lowest point at the stem base, the held phone
    // marked held.
    let origin: Vec<String> = report
        .warnings()
        .filter(|w| matches!(w.code, "bounds.below-ground" | "bounds.above-ground"))
        .map(ToString::to_string)
        .collect();
    assert!(origin.is_empty(), "{origin:#?}");
    let phone = report
        .checked
        .iter()
        .find(|c| c.entry.file == "props/phone-slab.glb")
        .unwrap();
    assert!(phone.model.is_held(), "the phone is held in the hand");
}

#[test]
fn every_model_diffs_empty_against_itself_and_round_trips() {
    let man = manifest::Manifest::parse(shipped::MANIFEST).unwrap();
    for e in &man.assets {
        let bytes = shipped::files()
            .iter()
            .find(|(n, _)| *n == e.file)
            .map(|(_, b)| b.to_vec())
            .unwrap();
        let m = glb::read(&bytes).unwrap();
        assert!(diff::diff(&m, &m).is_empty(), "{}", e.file);
        // The editable .gltf copy reads back as the same model.
        let out = import::export_files(e.id(), &m, true);
        let bin = out[1].1.clone();
        let bin_name = out[1].0.clone();
        let back =
            glb::read_with(&out[0].1, &|uri| (uri == bin_name).then(|| bin.clone())).unwrap();
        let d = diff::diff(&m, &back);
        assert!(d.is_empty(), "{}: {:#?}", e.file, d.lines());
        // Imported again under its own entry: the same model.
        let opts = import::ImportOptions {
            kind: e.kind,
            id: e.id().into(),
            license: e.license.clone(),
            source: e.source.clone(),
            author: e.author.clone(),
            profile: e.profile.clone(),
            species: e.species.clone(),
            variant: e.variant.clone(),
            anchor: e.anchor.clone(),
        };
        let imported = import::import(back, vec![], &opts)
            .unwrap_or_else(|errs| panic!("{}: {errs:#?}", e.file));
        let d = diff::diff(&m, &imported.model);
        assert!(d.is_empty(), "{}: {:#?}", e.file, d.lines());
    }
}
