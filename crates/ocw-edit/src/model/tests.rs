//! Tests of the model document (spec 11.4, 16G): every operation and its
//! exact inverse, random sequences undone to the identical document,
//! merged drags, dirty flags, the validator, the fixes, both proxy forms,
//! the save with `model diff` (a no-op edit changes nothing in any
//! committed model), the re-import.

use super::*;
use crate::DocId;
use crate::document::Document;
use ocw_assets::library::ModelLibrary;

fn lib() -> ModelLibrary {
    ModelLibrary::embedded()
}

fn open(id: &str) -> ModelDoc {
    let l = lib();
    let entry = l.entry(id).cloned().expect("listed");
    let bytes = l.bytes(id).expect("bytes");
    let model = ocw_assets::glb::read(&bytes).expect("reads");
    ModelDoc::new(entry, model, true)
}

fn bollard() -> ModelDoc {
    open("props/bollard-round")
}

fn bike() -> ModelDoc {
    open("bikes/bike-road-endurance")
}

fn doc_of(d: ModelDoc) -> Document<ModelDoc> {
    Document::new(DocId(1), "model", d).with_validator(validator())
}

/// Applies `op`, then its inverse: back to the identical document; the
/// inverse's inverse is `op` again in effect.
fn round_trip(d: &ModelDoc, op: &ModelOp) -> ModelDoc {
    let mut x = d.clone();
    let inv = x.apply(op).expect("applies");
    let after = x.clone();
    assert_ne!(&after, d, "{op:?} changed nothing");
    let again = x.apply(&inv).expect("inverse applies");
    assert_eq!(&x, d, "the inverse of {} restores", ModelDoc::label(op));
    x.apply(&again).expect("redo applies");
    assert_eq!(x, after, "redo of {} gives the same", ModelDoc::label(op));
    after
}

#[test]
fn every_operation_has_its_exact_inverse() {
    let d = bike();
    let crank = d.by_name("crank").expect("crank");
    let saddle = d.by_name("saddle").expect("saddle");
    let root = d.by_name("bike").expect("root");
    let mut trs = Trs::of(d.node(crank).unwrap());
    trs.translation += Vec3::new(0.01, 0.02, -0.03);
    trs.rotation = Quat::from_rotation_z(0.3) * trs.rotation;
    trs.scale = Vec3::splat(1.1);
    let ops = vec![
        d.transform(crank, trs),
        d.rename(saddle, "saddle_new").unwrap(),
        d.reparent(saddle, Some(crank), true).unwrap(),
        d.reparent(saddle, None, false).unwrap(),
        d.delete(crank).unwrap(),
        d.add_point(&Point::Anchor("bottle".into()), Some(root), Vec3::Y)
            .unwrap(),
        d.add_point(&Point::Level("lod2".into()), None, Vec3::ZERO)
            .unwrap(),
        d.add_proxy(
            "box",
            ProxyForm::Mesh,
            Some(root),
            Vec3::Y,
            Vec3::splat(0.2),
        )
        .unwrap(),
        d.add_proxy(
            "cylinder",
            ProxyForm::Empty,
            None,
            Vec3::Y,
            Vec3::splat(0.3),
        )
        .unwrap(),
        ModelOp::NodeExtras {
            node: saddle,
            extras: serde_json::json!({"note": "made up"}),
        },
        d.set_material(0, |m| {
            m.color = [0.1, 0.2, 0.3, 1.0];
            m.emissive = [0.5, 0.4, 0.0];
        })
        .unwrap(),
        d.set_ocw("finish", Some(serde_json::json!("matte")))
            .unwrap(),
        ModelOp::Copyright("© Ada Rider; licence CC0-1.0".into()),
        d.set_entry("author", "Ada Rider").unwrap(),
        d.set_entry("source", "own work").unwrap(),
        ModelOp::Batch {
            label: "two".into(),
            ops: vec![d.rename(saddle, "s2").unwrap(), d.delete(crank).unwrap()],
        },
    ];
    for op in &ops {
        round_trip(&d, op);
    }
    // Merging two materials (the slot of another material).
    let other = d.model.materials[1].name.clone();
    let op = d.assign_slot(0, &other, false).unwrap();
    assert!(matches!(op, ModelOp::Replace(_)));
    let after = round_trip(&d, &op);
    assert_eq!(after.model.materials.len(), d.model.materials.len() - 1);
}

#[test]
fn a_reparent_keeps_parents_before_children_and_the_place() {
    let d = bike();
    // The first node (the root) under the last one: everything below
    // moves after it.
    let last = *d.ids.last().unwrap();
    let wheel = d.by_name("wheel_front").unwrap();
    let before = d.model.world(d.index_of(wheel).unwrap());
    let crank = d.by_name("crank").unwrap();
    // Under a node that comes after it, keeping its place in the model.
    let mut x = d.clone();
    let op = x.reparent(wheel, Some(last), true);
    if d.subtree(wheel).contains(&last) {
        assert!(op.is_err());
    } else {
        let after = round_trip(&d, &op.unwrap());
        x = after;
        let i = x.index_of(wheel).unwrap();
        assert!(x.index_of(last).unwrap() < i, "the parent first");
        let now = x.model.world(i);
        assert!(now.translation.distance(before.translation) < 1e-4);
        for (k, n) in x.model.nodes.iter().enumerate() {
            assert!(
                n.parent.is_none_or(|p| p < k),
                "{} after its parent",
                n.name
            );
        }
    }
    // Below itself: refused.
    let root = d.by_name("bike").unwrap();
    assert!(d.reparent(root, Some(crank), true).is_err());
    let mut y = d.clone();
    let bad = ModelOp::Reparent {
        node: root,
        parent: Some(crank),
        trs: Trs::IDENTITY,
        order: None,
    };
    assert!(y.apply(&bad).is_err());
    assert_eq!(y, d, "unchanged on an error");
}

#[test]
fn removing_a_node_takes_its_subtree_and_undo_puts_it_back_in_place() {
    let d = bike();
    let root = d.by_name("bike").unwrap();
    let op = d.delete(root).unwrap();
    let ModelOp::Remove(ids) = &op else {
        panic!("a remove")
    };
    assert_eq!(ids.len(), d.subtree(root).len());
    let after = round_trip(&d, &op);
    assert!(after.model.node("bike").is_none());
    // Removing a parent without its children is refused.
    let mut x = d.clone();
    assert!(x.apply(&ModelOp::Remove(vec![root])).is_err());
    assert_eq!(x, d);
}

#[test]
fn renames_follow_the_naming_rules() {
    let d = bike();
    let saddle = d.by_name("saddle").unwrap();
    assert!(d.rename(saddle, "crank").is_err(), "a taken name");
    assert!(d.rename(saddle, "  ").is_err(), "an empty name");
    assert!(d.rename(saddle, "saddle").is_ok(), "its own name");
    let names = contract_names(Kind::Bike);
    assert!(names.iter().any(|n| n == "hs_saddle"));
    assert!(names.iter().any(|n| n == "wheel_front"));
    assert!(names.iter().any(|n| n == "col_box"));
    assert!(contract_names(Kind::Plant).iter().any(|n| n == "billboard"));
}

/// Spec 16G: random sequences of model edits, undos and redos keep the
/// replayed states, and undo all the way gives the identical document.
#[test]
fn random_edit_sequences_undo_to_the_identical_document() {
    let start = bollard();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n.max(1)
    };
    for round in 0..40 {
        let mut d = doc_of(start.clone());
        let mut states = vec![d.content().clone()];
        let mut redo: Vec<ModelDoc> = Vec::new();
        let mut now = 0.0;
        for _ in 0..30 {
            now += 2.0;
            let c = d.content().clone();
            if c.ids.is_empty() {
                assert!(d.undo(now).unwrap());
                redo.push(states.pop().unwrap());
                continue;
            }
            let pick = c.ids[rnd(c.ids.len() as u64) as usize];
            let other = c.ids[rnd(c.ids.len() as u64) as usize];
            let op = match rnd(9) {
                0 => {
                    let mut t = Trs::of(c.node(pick).unwrap());
                    t.translation.x += 0.1;
                    Ok(c.transform(pick, t))
                }
                1 => c.rename(pick, &format!("n{}", rnd(1000))),
                2 => c.reparent(pick, Some(other), rnd(2) == 0),
                3 => c.reparent(pick, None, true),
                4 => c.delete(pick),
                5 => c.add_point(
                    &Point::Anchor(format!("a{}", rnd(1000))),
                    Some(other),
                    Vec3::X,
                ),
                6 => c.add_proxy(
                    "box",
                    if rnd(2) == 0 {
                        ProxyForm::Mesh
                    } else {
                        ProxyForm::Empty
                    },
                    Some(other),
                    Vec3::Y,
                    Vec3::splat(0.1),
                ),
                7 => {
                    if d.undo(now).unwrap() {
                        redo.push(states.pop().unwrap());
                    }
                    assert_eq!(d.content(), states.last().unwrap());
                    continue;
                }
                _ => {
                    if d.redo(now).unwrap() {
                        states.push(redo.pop().unwrap());
                    }
                    assert_eq!(d.content(), states.last().unwrap());
                    continue;
                }
            };
            let Ok(op) = op else { continue };
            d.seal();
            if d.execute(op, now).is_ok() {
                states.push(d.content().clone());
                redo.clear();
            }
            assert_eq!(d.content(), states.last().unwrap(), "round {round}");
            let m = &d.content().model;
            for (k, n) in m.nodes.iter().enumerate() {
                assert!(n.parent.is_none_or(|p| p < k));
            }
            assert_eq!(d.content().ids.len(), m.nodes.len());
        }
        // Redo what is left first: the newest state is the end.
        while d.redo(now).unwrap() {
            states.push(redo.pop().unwrap());
            assert_eq!(d.content(), states.last().unwrap(), "round {round}");
        }
        let end = d.content().clone();
        while d.undo(now).unwrap() {}
        assert_eq!(d.content(), &start, "round {round}");
        while d.redo(now).unwrap() {}
        assert_eq!(d.content(), &end, "round {round}");
    }
}

#[test]
fn a_drag_is_one_undo_step_and_the_dirty_flag_follows_the_history() {
    let mut d = doc_of(bollard());
    let id = d.content().ids[0];
    let start = d.content().clone();
    assert!(!d.is_dirty());
    let t0 = Trs::of(d.content().node(id).unwrap());
    for k in 1..=20 {
        let mut t = t0;
        t.translation.x += k as f32 * 0.01;
        d.execute(ModelOp::Transform { node: id, trs: t }, k as f64 * 0.016)
            .unwrap();
    }
    assert_eq!(d.history().undo.len(), 1, "a drag merges");
    assert!(d.is_dirty());
    d.seal();
    let mut t = t0;
    t.translation.y += 1.0;
    d.execute(ModelOp::Transform { node: id, trs: t }, 0.5)
        .unwrap();
    assert_eq!(d.history().undo.len(), 2, "a sealed drag does not merge");
    d.mark_saved();
    assert!(!d.is_dirty());
    d.undo(1.0).unwrap();
    assert!(d.is_dirty());
    d.redo(1.0).unwrap();
    assert!(!d.is_dirty(), "back at the saved state");
    d.undo(1.0).unwrap();
    d.undo(1.0).unwrap();
    assert_eq!(d.content(), &start);
    // Another node's drag does not merge into this one.
    d.execute(ModelOp::Transform { node: id, trs: t }, 2.0)
        .unwrap();
    let other = d.content().ids[1];
    let t1 = Trs::of(d.content().node(other).unwrap());
    d.execute(
        ModelOp::Transform {
            node: other,
            trs: Trs {
                scale: Vec3::splat(2.0),
                ..t1
            },
        },
        2.1,
    )
    .unwrap();
    assert_eq!(d.history().undo.len(), 2);
}

#[test]
fn the_validator_runs_on_every_change_and_the_fixes_clear_it() {
    let mut d = doc_of(bollard());
    assert!(
        d.findings().iter().all(|f| f.severity != Severity::Error),
        "{:?}",
        d.findings()
    );
    let id = d.content().ids[0];
    let name = d.content().node(id).unwrap().name.clone();
    d.execute(d.content().rename(id, &format!("{name}.001")).unwrap(), 0.0)
        .unwrap();
    assert!(
        d.findings()
            .iter()
            .any(|f| f.message.contains("duplicate suffix")),
        "{:?}",
        d.findings()
    );
    // The fix of the checker's finding renames it back.
    let found = d.content().findings(0);
    let fix = found
        .iter()
        .find(|f| f.code == "node.blender-suffix")
        .and_then(|f| f.fix.clone())
        .expect("a fix");
    let op = d.content().fix_op(&fix).expect("an operation");
    d.execute(op, 5.0).unwrap();
    assert_eq!(d.content().node(id).unwrap().name, name);
    assert!(
        !d.findings()
            .iter()
            .any(|f| f.message.contains("duplicate suffix"))
    );
    // Broken metadata: all fixes as one step.
    let mut c = d.content().clone();
    c.model.extras = Value::Null;
    c.model.copyright.clear();
    d.execute(ModelOp::Replace(Box::new(c)), 10.0).unwrap();
    assert!(d.findings().iter().any(|f| f.severity == Severity::Error));
    let all = d.content().findings(0);
    let op = d.content().fix_all_op(&all).expect("fixes");
    d.execute(op, 20.0).unwrap();
    assert!(
        d.findings().iter().all(|f| f.severity != Severity::Error),
        "{:?}",
        d.findings()
    );
    d.undo(30.0).unwrap();
    assert!(d.findings().iter().any(|f| f.severity == Severity::Error));
}

#[test]
fn both_proxy_forms_survive_the_file_with_their_shape() {
    let d = bollard();
    for form in [ProxyForm::Mesh, ProxyForm::Empty] {
        for shape in ["box", "cylinder", "capsule"] {
            let mut x = d.clone();
            let centre = Vec3::new(0.1, 0.4, -0.2);
            let half = Vec3::new(0.15, 0.4, 0.25);
            let op = x.add_proxy(shape, form, None, centre, half).unwrap();
            x.apply(&op).unwrap();
            let back = ocw_assets::glb::read(&ocw_assets::glb::write(&x.model)).unwrap();
            let name = x.free_name(&format!("col_{shape}"));
            let _ = name;
            let p = ocw_assets::props::proxies(&back);
            let added = p
                .iter()
                .find(|p| {
                    p.shape == shape && p.centre.distance(centre) < 1e-3 && {
                        p.half.distance(half) < 1e-3
                    }
                })
                .unwrap_or_else(|| panic!("{shape} {form:?}: {p:?}"));
            assert!(added.name.starts_with(&format!("col_{shape}")));
            // No error finding for a proxy of either form.
            let f = x.findings(0);
            assert!(
                !f.iter()
                    .any(|f| f.is_error() && f.target == check::Target::Node(added.name.clone())),
                "{f:?}"
            );
        }
    }
    // A second box is `col_box_2`, still a box.
    let mut x = d.clone();
    for _ in 0..2 {
        let op = x
            .add_proxy("box", ProxyForm::Empty, None, Vec3::Y, Vec3::ONE)
            .unwrap();
        x.apply(&op).unwrap();
    }
    assert_eq!(check::collision_shape("col_box_2"), Some("box"));
    assert!(x.model.node("col_box_2").is_some());
    assert!(
        d.add_proxy("sphere", ProxyForm::Mesh, None, Vec3::Y, Vec3::ONE)
            .is_err()
    );
    assert!(
        d.add_proxy("box", ProxyForm::Mesh, None, Vec3::Y, Vec3::ZERO)
            .is_err()
    );
}

#[test]
fn points_get_contract_names() {
    let d = bike();
    let root = d.by_name("bike");
    for (p, name) in [
        (Point::Anchor("bottle".into()), "anchor_bottle"),
        (Point::Hotspot("bell".into()), "hs_bell"),
        (Point::Level("billboard".into()), "billboard"),
    ] {
        let mut x = d.clone();
        x.apply(&x.add_point(&p, root, Vec3::Y).unwrap()).unwrap();
        let i = x.model.node(name).expect(name);
        assert_eq!(x.model.nodes[i].parent, root.and_then(|r| x.index_of(r)));
        assert!(check::contract_role(name).is_some());
    }
    assert!(
        d.add_point(&Point::Level("lod9".into()), None, Vec3::ZERO)
            .is_err()
    );
    assert!(
        d.add_point(&Point::Anchor(String::new()), None, Vec3::ZERO)
            .is_err()
    );
    // A taken name is refused.
    assert!(
        d.add_point(&Point::Hotspot("saddle".into()), root, Vec3::ZERO)
            .is_err()
    );
}

#[test]
fn slots_and_materials_are_edited() {
    let d = bollard();
    let unused = ocw_assets::slots::ALL
        .iter()
        .find(|s| d.model.material(s.name).is_none())
        .unwrap();
    let mut x = d.clone();
    x.apply(&x.assign_slot(0, unused.name, true).unwrap())
        .unwrap();
    assert_eq!(x.model.materials[0].name, unused.name);
    assert_eq!(
        x.model.materials[0].color,
        ocw_assets::slots::material(unused.name).color
    );
    assert!(d.assign_slot(0, "no-such-slot", false).is_err());
    // A glowing default survives the file and shows in the diff.
    let mut y = d.clone();
    y.apply(
        &y.set_material(0, |m| m.emissive = [1.0, 0.75, 0.25])
            .unwrap(),
    )
    .unwrap();
    let plan = y.save_plan(Some((&d.model, &d.entry)), None).unwrap();
    let back = ocw_assets::glb::read(&plan.bytes).unwrap();
    assert_eq!(back.materials[0].emissive, [1.0, 0.75, 0.25]);
    assert!(
        plan.diff.lines().iter().any(|l| l.contains("emissive")),
        "{:?}",
        plan.diff.lines()
    );
}

#[test]
fn extras_and_the_manifest_entry_are_edited_within_the_rules() {
    let d = bike();
    assert!(
        d.set_ocw("mass_kg", Some(serde_json::json!(7))).is_err(),
        "physics"
    );
    assert!(d.set_ocw("kind", Some(serde_json::json!("prop"))).is_err());
    let mut x = d.clone();
    x.apply(
        &x.set_ocw("finish", Some(serde_json::json!("gloss")))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(x.model.ocw().unwrap()["finish"], "gloss");
    x.apply(&x.set_ocw("finish", None).unwrap()).unwrap();
    assert!(x.model.ocw().unwrap().get("finish").is_none());
    // The kind's keys kept.
    assert_eq!(x.model.ocw(), d.model.ocw());
    let mut y = d.clone();
    y.apply(&y.set_entry("license", "CC0-1.0").unwrap())
        .unwrap();
    y.apply(&y.set_entry("author", "Bea Climber").unwrap())
        .unwrap();
    assert_eq!(y.entry.license, "CC0-1.0");
    assert!(d.set_entry("file", "x.glb").is_err());
    let mut z = d.clone();
    let mut e = d.entry.clone();
    e.file = "bikes/other.glb".into();
    assert!(z.apply(&ModelOp::Entry(e)).is_err());
    // The diff before saving names the entry's changes, and the manifest
    // is upserted.
    let manifest = ocw_assets::library::MANIFEST;
    let plan = y
        .save_plan(Some((&d.model, &d.entry)), Some(manifest))
        .unwrap();
    let lines = plan.diff.lines();
    assert!(lines.iter().any(|l| l.contains("license")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Bea Climber")), "{lines:?}");
    let text = plan.manifest.unwrap();
    let m = ocw_assets::manifest::Manifest::parse(&text).unwrap();
    let e = m.assets.iter().find(|e| e.file == d.entry.file).unwrap();
    assert_eq!(e.author, "Bea Climber");
    assert_eq!(
        m.assets.len(),
        ocw_assets::manifest::Manifest::parse(manifest)
            .unwrap()
            .assets
            .len()
    );
}

/// Spec 11.4 / 16G: a no-op edit (open, save) changes nothing – `model
/// diff` of every committed model against its saved file is empty, also
/// after an edit and its undo.
#[test]
fn saving_without_a_change_gives_an_empty_model_diff_for_every_committed_model() {
    let l = lib();
    let manifest = ocw_assets::library::MANIFEST;
    for e in &l.manifest().assets {
        let d = open(e.id());
        let plan = d
            .save_plan(Some((&d.model, &d.entry)), Some(manifest))
            .unwrap();
        assert!(plan.diff.is_empty(), "{}: {:?}", e.file, plan.diff.lines());
        assert!(plan.errors.is_empty(), "{}: {:?}", e.file, plan.errors);
        // The manifest stays the same entries.
        let a = ocw_assets::manifest::Manifest::parse(manifest).unwrap();
        let b = ocw_assets::manifest::Manifest::parse(&plan.manifest.unwrap()).unwrap();
        assert_eq!(a, b, "{}", e.file);
        // An edit and its undo: the same file again.
        let mut doc = doc_of(d.clone());
        let id = doc.content().ids[0];
        let mut t = Trs::of(doc.content().node(id).unwrap());
        t.translation.y += 0.5;
        doc.execute(ModelOp::Transform { node: id, trs: t }, 0.0)
            .unwrap();
        doc.undo(1.0).unwrap();
        let again = doc
            .content()
            .save_plan(Some((&d.model, &d.entry)), None)
            .unwrap();
        assert_eq!(again.bytes, plan.bytes, "{}", e.file);
    }
}

#[test]
fn an_edited_file_reimports_keeping_ids_points_and_metadata() {
    let d = bike();
    // What Blender gives back: no asset extras or copyright, a duplicate
    // suffix, an anchor of the studio gone, the saddle moved.
    let mut x = d.clone();
    x.apply(
        &x.add_point(&Point::Anchor("bottle".into()), x.by_name("bike"), Vec3::Y)
            .unwrap(),
    )
    .unwrap();
    let mut blender = x.model.clone();
    blender.extras = Value::Null;
    blender.copyright.clear();
    let s = blender.node("saddle").unwrap();
    blender.nodes[s].translation.y += 0.02;
    blender.nodes[s].name = "saddle.001".into();
    let a = blender.node("anchor_bottle").unwrap();
    blender.nodes.remove(a);
    for n in &mut blender.nodes {
        if let Some(p) = n.parent.as_mut()
            && *p > a
        {
            *p -= 1;
        }
    }
    let (op, notes) = x.reimport(blender);
    let mut y = x.clone();
    let inv = y.apply(&op).unwrap();
    assert!(
        notes.iter().any(|n| n.contains("anchor_bottle")),
        "{notes:?}"
    );
    assert!(y.model.node("anchor_bottle").is_some());
    assert_eq!(y.model.ocw(), x.model.ocw(), "extras kept");
    assert_eq!(y.model.copyright, x.model.copyright);
    assert_eq!(y.by_name("saddle"), x.by_name("saddle"), "the id by name");
    assert_eq!(y.by_name("crank"), x.by_name("crank"));
    for (k, n) in y.model.nodes.iter().enumerate() {
        assert!(n.parent.is_none_or(|p| p < k));
    }
    let ids: BTreeSet<_> = y.ids.iter().collect();
    assert_eq!(ids.len(), y.ids.len(), "unique ids");
    y.apply(&inv).unwrap();
    assert_eq!(y, x, "undo of the re-import");
}

#[test]
fn documents_serialise_for_the_autosave() {
    let mut d = doc_of(bollard());
    let id = d.content().ids[0];
    d.execute(d.content().rename(id, "post_main").unwrap(), 0.0)
        .unwrap();
    let rec = d.autosave_record();
    let json = rec.to_json().unwrap();
    let back = crate::AutosaveRecord::<ModelDoc, ModelOp>::from_json(&json).unwrap();
    assert_eq!(back.content, *d.content());
    assert_eq!(back.history, *d.history());
}
