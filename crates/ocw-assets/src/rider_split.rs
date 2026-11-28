//! The rider in several model files (spec §14 Phase 4, 4N N8; Q25
//! resolved by following N8): the hair pieces, the moustaches and beards
//! and the base bodies' shoulders and chests are their own files with
//! their own budgets, so the near rider file has room for real hands.
//!
//! - `riders/rider-body.glb`: the rig, the body, the kit, the face, the
//!   glasses – every look shares it;
//! - `wear/hair-<piece>.glb`: one hair piece each (a style is one or two
//!   pieces, [`crate::rider::hair_nodes`]), at the head;
//! - `wear/<moustache|beard>-<id>.glb`: the facial hair, at the head;
//! - `wear/body-<shoulders|chest>-<a|b|c>.glb`: the base bodies' parts,
//!   at the torso / the jersey;
//! - `wear/helmet-<style>.glb`: the four helmets, at the head, near and
//!   far (spec 14 Phase 15 M3: the rider files hold no helmet; the shells
//!   come from these files only, [`helmet_part`]).
//!
//! (N8 named them `hair/<id>.glb`, `face/…`, `rider-a|b|c.glb`; the file
//! contract of Phase 15, 15H, puts every wearable in `wear/<name>.glb`.)
//!
//! Each part file is a wearable (spec §14 Phase 15, 15E): its root node is
//! named after the rider node it hangs from (its `anchor`), the piece below
//! it as in the generator. Loading attaches every part under its anchor
//! ([`merge`]): the renderer, the pose and the clothing checks keep the
//! one rig and the optional nodes' names as before.

use crate::helmet::Helmet;
use crate::model::{Model, Node};
use crate::rider::{Detail, opt};

/// One part file: its path in `artwork/3dmodels/` and the generator's
/// node it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFile {
    pub file: String,
    pub node: &'static str,
}

/// The hair pieces (every node a hair style uses).
pub const HAIR_PIECES: [&str; 19] = [
    opt::HAIR_CAP,
    opt::HAIR_BUZZ,
    opt::HAIR_LONG,
    opt::HAIR_TAIL,
    opt::HAIR_BUN,
    opt::HAIR_SHAVED,
    opt::HAIR_FRINGE,
    opt::HAIR_TEXTURED,
    opt::HAIR_SIDE_PART,
    opt::HAIR_CURLS,
    opt::HAIR_AFRO,
    opt::HAIR_MID,
    opt::HAIR_LONG_CURLY,
    opt::HAIR_TAIL_LOW,
    opt::HAIR_BRAID,
    opt::HAIR_BRAIDS,
    opt::HAIR_BUN_LOW,
    opt::HAIR_LOCS,
    opt::HAIR_ROWS,
];

/// Triangle budgets of the part files (N8): a hair piece, a beard, a
/// moustache, a base body's part.
pub const HAIR_BUDGET: usize = 1_200;
pub const BEARD_BUDGET: usize = 700;
pub const MOUSTACHE_BUDGET: usize = 150;
pub const BODY_PART_BUDGET: usize = 600;
pub const HAND_BUDGET: usize = 700;

/// `hair_side_part` → `side-part`, `moustache_classic` → `classic`.
fn stem(node: &str, prefix: &str) -> String {
    node.strip_prefix(prefix).unwrap_or(node).replace('_', "-")
}

/// Every part file, in a fixed order (the merged model's).
pub fn part_files() -> Vec<PartFile> {
    let hair = HAIR_PIECES
        .iter()
        .map(|n| PartFile {
            file: format!("wear/hair-{}.glb", stem(n, "hair_")),
            node: n,
        })
        // The bare-head shapes (4N Q28).
        .chain(crate::rider::HAIR_BARE.iter().map(|(p, b, _)| PartFile {
            file: format!("wear/hair-bare-{}.glb", stem(p, "hair_")),
            node: b,
        }));
    let moustache = opt::MOUSTACHE.iter().map(|n| PartFile {
        file: format!("wear/moustache-{}.glb", stem(n, "moustache_")),
        node: n,
    });
    let beard = opt::BEARD.iter().map(|n| PartFile {
        file: format!("wear/beard-{}.glb", stem(n, "beard_")),
        node: n,
    });
    let body = opt::SHOULDERS.iter().chain(&opt::CHEST).map(|n| PartFile {
        file: format!("wear/body-{}.glb", n.replace('_', "-")),
        node: n,
    });
    // The hands' shapes (4N, the hands): one file per shape and hand.
    let hands = crate::rider::hand::nodes().into_iter().map(|n| PartFile {
        file: format!("wear/{}.glb", n.replace('_', "-")),
        node: n,
    });
    hair.chain(moustache)
        .chain(beard)
        .chain(body)
        .chain(hands)
        .collect()
}

/// The budget of a part file's node.
pub fn budget(node: &str) -> usize {
    if opt::MOUSTACHE.contains(&node) {
        MOUSTACHE_BUDGET
    } else if opt::BEARD.contains(&node) {
        BEARD_BUDGET
    } else if HAIR_PIECES.contains(&node)
        || crate::rider::HAIR_BARE.iter().any(|(_, b, _)| *b == node)
    {
        HAIR_BUDGET
    } else if crate::rider::hand::of_node(node).is_some() {
        HAND_BUDGET
    } else {
        BODY_PART_BUDGET
    }
}

/// Copies `src`'s node `i` and everything below it into `dst` under
/// `parent`, its materials added to `dst`'s.
fn copy_below(dst: &mut Model, src: &Model, i: usize, parent: usize) {
    let mut map = vec![usize::MAX; src.nodes.len()];
    for (k, n) in src.nodes.iter().enumerate() {
        if !src.is_below(k, i) {
            continue;
        }
        let p = if k == i {
            parent
        } else {
            map[n.parent.expect("below the root")]
        };
        let mut node = n.clone();
        node.parent = Some(p);
        for prim in &mut node.primitives {
            prim.material = dst.material_or_add(src.materials[prim.material].clone());
        }
        map[k] = dst.add(node);
    }
}

/// The part file of `node`: a root named after its parent in `full` (the
/// anchor), the piece below it.
pub fn extract(full: &Model, node: &str) -> Option<Model> {
    let i = full.node(node)?;
    let anchor = &full.nodes[full.nodes[i].parent?].name;
    let mut m = Model::default();
    let root = m.add(Node::new(anchor, None, glam::Vec3::ZERO));
    copy_below(&mut m, full, i, root);
    Some(m)
}

/// The anchor a part file hangs from (its root's name).
pub fn anchor(part: &Model) -> Option<&str> {
    part.nodes
        .iter()
        .find(|n| n.parent.is_none())
        .map(|n| n.name.as_str())
}

/// The rider node the helmet files are worn at (their manifest `anchor`).
pub const HELMET_ANCHOR: &str = crate::pose::nodes::HEAD;

/// The rider nodes of the helmets: not in the rider files (near and
/// far), drawn from the helmet files ([`helmet_part`]).
pub fn helmet_nodes() -> [&'static str; 4] {
    Helmet::ALL.map(|h| h.node())
}

/// The helmet of style `h` from its file (`wear/helmet-*.glb`, spec 14
/// Phase 15 M3): a part with the root [`HELMET_ANCHOR`] and below it the
/// rider node `h.node()` holding the near shell (the file's root) or, for
/// [`Detail::Low`], the far one (its `lod1`) – both in the head's frame,
/// so the shell is worn at the anchor as it is. `None` when the file has
/// no such shell.
pub fn helmet_part(file: &Model, h: Helmet, detail: Detail) -> Option<Model> {
    let src = match detail {
        Detail::High => file.nodes.iter().position(|n| n.parent.is_none())?,
        Detail::Low => file.node(crate::pose::bike_nodes::LOD1)?,
    };
    if file.nodes[src].primitives.is_empty() {
        return None;
    }
    let mut m = Model::default();
    let root = m.add(Node::new(HELMET_ANCHOR, None, glam::Vec3::ZERO));
    let shell = m.add(Node::new(h.node(), Some(root), glam::Vec3::ZERO));
    for p in &file.nodes[src].primitives {
        let mut q = p.clone();
        q.material = m.material_or_add(file.materials[p.material].clone());
        m.nodes[shell].primitives.push(q);
    }
    Some(m)
}

/// `full` without the part files' nodes and the helmets (and what is
/// below them): the near rider file.
pub fn strip(full: &Model) -> Model {
    let drop: Vec<&str> = part_files()
        .iter()
        .map(|p| p.node)
        .chain(helmet_nodes())
        .collect();
    without(full, &drop)
}

/// The far rider without its helmets: the far rider file (its hair and
/// face stay in it).
pub fn strip_low(low: &Model) -> Model {
    without(low, &helmet_nodes())
}

/// `full` without the nodes `drop` (and what is below them).
fn without(full: &Model, drop: &[&str]) -> Model {
    let parts: Vec<usize> = drop.iter().filter_map(|n| full.node(n)).collect();
    let mut m = Model {
        materials: Vec::new(),
        nodes: Vec::new(),
        ..full.clone()
    };
    let mut map = vec![usize::MAX; full.nodes.len()];
    for (k, n) in full.nodes.iter().enumerate() {
        if parts.iter().any(|p| full.is_below(k, *p)) {
            continue;
        }
        let mut node = n.clone();
        node.parent = n.parent.map(|p| map[p]);
        for prim in &mut node.primitives {
            prim.material = m.material_or_add(full.materials[prim.material].clone());
        }
        map[k] = m.add(node);
    }
    m
}

/// The rider as the renderer uses it: `base` with every part attached
/// under its anchor (in the order given). A part whose anchor is missing
/// is an error.
pub fn merge(base: &Model, parts: &[&Model]) -> Result<Model, String> {
    let mut m = base.clone();
    for part in parts {
        let a = anchor(part).ok_or("a part file without a root")?;
        let at = m
            .node(a)
            .ok_or_else(|| format!("the rider has no anchor `{a}`"))?;
        let root = part
            .nodes
            .iter()
            .position(|n| n.parent.is_none())
            .expect("a root");
        for (k, n) in part.nodes.iter().enumerate() {
            if n.parent == Some(root) {
                copy_below(&mut m, part, k, at);
            }
        }
    }
    Ok(m)
}

/// The near rider of the generator, split as exported and merged again
/// (what [`crate::shipped::rider_model`] loads from the files).
pub fn assembled(full: &Model) -> Model {
    let parts: Vec<Model> = part_files()
        .iter()
        .filter_map(|p| extract(full, p.node))
        .chain(Helmet::ALL.iter().filter_map(|h| {
            helmet_part(
                &crate::export::helmet_model(*h, full, full),
                *h,
                Detail::High,
            )
        }))
        .collect();
    let refs: Vec<&Model> = parts.iter().collect();
    merge(&strip(full), &refs).expect("the generator's anchors")
}

/// The far rider of the generator as exported and loaded again (what
/// [`crate::shipped::rider_low_model`] loads): the far file with the
/// helmet files' far shells attached at the head.
pub fn assembled_low(near: &Model, low: &Model) -> Model {
    let parts: Vec<Model> = Helmet::ALL
        .iter()
        .filter_map(|h| helmet_part(&crate::export::helmet_model(*h, near, low), *h, Detail::Low))
        .collect();
    let refs: Vec<&Model> = parts.iter().collect();
    merge(&strip_low(low), &refs).expect("the generator's anchors")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rider::{Detail, build};

    fn world(m: &Model, i: usize) -> glam::Affine3A {
        let mut t = m.nodes[i].local();
        let mut p = m.nodes[i].parent;
        while let Some(k) = p {
            t = m.nodes[k].local() * t;
            p = m.nodes[k].parent;
        }
        t
    }

    /// Split and merged again, the rider is the generator's: every node
    /// by name at the same place with the same meshes and materials –
    /// only the order changes (the parts come last, below their anchors).
    #[test]
    fn the_split_rider_merges_back_into_the_generators() {
        let full = build(Detail::High);
        let back = assembled(&full);
        assert_eq!(back.nodes.len(), full.nodes.len());
        assert_eq!(back.triangles(), full.triangles());
        for (i, n) in full.nodes.iter().enumerate() {
            let k = back.node(&n.name).unwrap_or_else(|| panic!("{}", n.name));
            let b = &back.nodes[k];
            assert_eq!(
                b.parent.map(|p| &back.nodes[p].name),
                n.parent.map(|p| &full.nodes[p].name),
                "{}",
                n.name
            );
            assert!(
                world(&back, k).abs_diff_eq(world(&full, i), 1e-6),
                "{}",
                n.name
            );
            assert_eq!(b.primitives.len(), n.primitives.len());
            for (pb, pf) in b.primitives.iter().zip(&n.primitives) {
                assert_eq!(pb.triangles(), pf.triangles());
                assert_eq!(
                    back.materials[pb.material].name, full.materials[pf.material].name,
                    "{}",
                    n.name
                );
            }
        }
        // Parents still come before their children.
        for (i, n) in back.nodes.iter().enumerate() {
            assert!(n.parent.is_none_or(|p| p < i));
        }
    }

    /// The shipped files load as one rider (`shipped::rider_model`): the
    /// near file and every part file from the library, each part below its
    /// anchor, node for node the generator's rider; the near file alone
    /// holds none of the parts.
    #[test]
    fn the_shipped_rider_loads_from_its_split_files() {
        let full = build(Detail::High);
        let shipped = crate::shipped::rider_model();
        assert_eq!(shipped.nodes.len(), full.nodes.len());
        assert_eq!(shipped.triangles(), full.triangles());
        let base = crate::shipped::model(crate::RIDER_ID);
        for p in part_files() {
            assert!(base.node(p.node).is_none(), "{} in the near file", p.node);
            let part = crate::shipped::model(crate::kinds::id_of(&p.file));
            let a = anchor(&part).expect("a root");
            let i = shipped.node(p.node).unwrap_or_else(|| panic!("{}", p.node));
            let parent = shipped.nodes[i].parent.expect("attached");
            assert_eq!(shipped.nodes[parent].name, a, "{}", p.file);
            assert_eq!(
                full.nodes[full.node(p.node).unwrap()]
                    .parent
                    .map(|k| full.nodes[k].name.as_str()),
                Some(a)
            );
        }
        // Every node of the generator's rider is there, by name.
        for n in &full.nodes {
            assert!(shipped.node(&n.name).is_some(), "{}", n.name);
        }
    }

    /// Every part file is small and within its budget (N8), hangs from a
    /// rider node of the near file, and holds only its piece.
    #[test]
    fn every_part_file_keeps_its_budget_and_its_anchor() {
        let full = build(Detail::High);
        let base = strip(&full);
        let files = part_files();
        assert_eq!(files.len(), 19 + 19 + 5 + 7 + 5 + 12);
        let mut seen = std::collections::BTreeSet::new();
        for p in &files {
            assert!(seen.insert(p.file.clone()), "{} twice", p.file);
            let m = extract(&full, p.node).unwrap_or_else(|| panic!("{}", p.node));
            let a = anchor(&m).unwrap();
            assert!(base.node(a).is_some(), "{}: anchor {a}", p.file);
            assert!(m.node(p.node).is_some());
            let tris = m.triangles();
            assert!(tris > 0 && tris <= budget(p.node), "{}: {tris}", p.file);
            assert!(
                base.node(p.node).is_none(),
                "{} left the rider file",
                p.node
            );
        }
        assert_eq!(budget("beard_full"), BEARD_BUDGET);
        assert_eq!(budget("moustache_pencil"), MOUSTACHE_BUDGET);
        assert_eq!(budget("hair_afro"), HAIR_BUDGET);
    }

    /// The budget of a rider (15B: 8,000 near) holds for the heaviest
    /// look – the near file with the largest hair style, moustache, beard
    /// and base body – and the near file alone has room left for the hands.
    #[test]
    fn the_heaviest_look_stays_in_the_riders_budget() {
        let full = build(Detail::High);
        let base = strip(&full);
        let tris = |n: &str| extract(&full, n).map_or(0, |m| m.triangles());
        let hair = (0..ocw_core::character::HAIR_STYLES.len() as u8)
            .map(|i| {
                let under: usize = crate::rider::hair_nodes(i).iter().map(|n| tris(n)).sum();
                let bare: usize = crate::rider::hair_nodes_bare(i)
                    .iter()
                    .map(|n| tris(n))
                    .sum();
                under.max(bare)
            })
            .max()
            .unwrap();
        let most = |ns: &[&str]| ns.iter().map(|n| tris(n)).max().unwrap_or(0);
        let body = most(&opt::SHOULDERS) + most(&opt::CHEST);
        // One shape per hand is drawn at a time.
        let hands: usize = (0..2)
            .map(|s| {
                crate::rider::hand::HandShape::ALL
                    .iter()
                    .map(|h| tris(crate::rider::hand::node(*h, s)))
                    .max()
                    .unwrap_or(0)
            })
            .sum();
        assert!(hands > 0);
        // One helmet is worn at a time (Phase 15 M3: from its file).
        let helmet = most(&helmet_nodes());
        assert!(helmet > 0);
        let heaviest = base.triangles()
            + hair
            + most(&opt::MOUSTACHE)
            + most(&opt::BEARD)
            + body
            + hands
            + helmet;
        println!(
            "heaviest look {heaviest}: file {}, hair {hair}, moustache {}, beard {}, body {body}, hands {hands}, helmet {helmet}",
            base.triangles(),
            most(&opt::MOUSTACHE),
            most(&opt::BEARD)
        );
        assert!(
            heaviest <= crate::manifest::MAX_TRIANGLES_RIDER,
            "{heaviest} (file {}, hair {hair}, body {body}, helmet {helmet})",
            base.triangles()
        );
        assert!(base.triangles() < full.triangles());
    }

    /// The vertices of node `name` in model coordinates, girth applied
    /// for `c` and the whole rider scaled by `s` (the height).
    fn shell(m: &Model, name: &str, c: &ocw_core::Character, s: f32) -> Vec<glam::Vec3> {
        let i = m.node(name).unwrap_or_else(|| panic!("{name}"));
        let (g, t) = crate::rider::girth_transform(m, i, c);
        let w = m.world(i);
        m.nodes[i]
            .primitives
            .iter()
            .flat_map(|p| p.positions.iter())
            .map(|v| s * w.transform_point3(g * glam::Vec3::from(*v) + t))
            .collect()
    }

    /// The helmets come from their files only (Phase 15 M3): neither
    /// rider file holds a helmet node, the shipped riders (near and far)
    /// hold each helmet once, below `head`, and the far rider loads like
    /// the generator's.
    #[test]
    fn the_helmets_are_worn_from_their_files_only() {
        let near_file = crate::shipped::model(crate::RIDER_ID);
        let far_file = crate::shipped::model(crate::RIDER_LOW_ID);
        let near = crate::shipped::rider_model();
        let far = crate::shipped::rider_low_model();
        for n in helmet_nodes() {
            assert!(near_file.node(n).is_none(), "{n} in the near rider file");
            assert!(far_file.node(n).is_none(), "{n} in the far rider file");
            for m in [near, far] {
                let at: Vec<usize> = (0..m.nodes.len())
                    .filter(|i| m.nodes[*i].name == n)
                    .collect();
                assert_eq!(at.len(), 1, "{n} drawn once");
                let parent = m.nodes[at[0]].parent.expect("attached");
                assert_eq!(m.nodes[parent].name, HELMET_ANCHOR, "{n}");
                assert!(!m.nodes[at[0]].primitives.is_empty(), "{n}");
                // Nothing else hangs from a helmet.
                assert!(m.nodes.iter().all(|k| k.parent != Some(at[0])), "{n}");
            }
        }
        // The far rider: the generator's, node for node (names, places,
        // triangles), with the helmets from the files.
        let gen_low = build(Detail::Low);
        let back = assembled_low(&build(Detail::High), &gen_low);
        // In memory exact; read back from the file within 15F's 0.1 mm
        // (the node transforms go through the file's f32 numbers).
        for (want, got, eps) in [(&gen_low, &back, 1e-6), (&gen_low, far, 1e-4)] {
            assert_eq!(got.nodes.len(), want.nodes.len());
            assert_eq!(got.triangles(), want.triangles());
            for (i, n) in want.nodes.iter().enumerate() {
                let k = got.node(&n.name).unwrap_or_else(|| panic!("{}", n.name));
                assert!(world(got, k).abs_diff_eq(world(want, i), eps), "{}", n.name);
                assert_eq!(
                    got.triangles_below(Some(k)),
                    want.triangles_below(Some(i)),
                    "{}",
                    n.name
                );
            }
        }
        // A helmet file without the far shell is no far helmet.
        let mut bare = crate::shipped::model("wear/helmet-road-vented")
            .as_ref()
            .clone();
        let lod = bare.node(crate::pose::bike_nodes::LOD1).unwrap();
        bare.nodes[lod].primitives.clear();
        assert!(helmet_part(&bare, Helmet::Road, Detail::Low).is_none());
        assert!(helmet_part(&bare, Helmet::Road, Detail::High).is_some());
    }

    /// Each helmet from its file sits at the head as the generator's
    /// helmet did, for every base body, body shape and height (1.50 –
    /// 2.05 m), near and far, standing at ease: the same vertices within
    /// 0.1 mm (15F), the shell unscaled by the build (the head keeps its size).
    #[test]
    fn each_helmet_sits_at_the_head_for_every_body_and_height() {
        use ocw_core::{BaseBody, BodyShape, Character};
        let gen_near = build(Detail::High);
        let gen_far = build(Detail::Low);
        let rig = crate::pose::RiderRig::from_model(&gen_near).unwrap();
        let sizing = crate::shipped::bike_sizing(ocw_core::ProfileId::Road);
        let pose = crate::pose::idle(&rig, 1.3);
        let posed = |m: &Model| {
            let mut m = m.clone();
            pose.apply(&mut m);
            m
        };
        let pairs = [
            (posed(&gen_near), posed(crate::shipped::rider_model())),
            (posed(&gen_far), posed(crate::shipped::rider_low_model())),
        ];
        for base in BaseBody::ALL {
            for shape in BodyShape::ALL {
                let c = Character {
                    base,
                    shape,
                    ..Character::default()
                };
                for cm in [150.0f32, 160.0, 170.0, 178.0, 190.0, 205.0] {
                    let s = sizing.scale(cm / 100.0);
                    for (want, got) in &pairs {
                        for n in helmet_nodes() {
                            let i = got.node(n).unwrap();
                            assert_eq!(crate::rider::girth(got, i, &c), glam::Vec3::ONE);
                            let a = shell(want, n, &c, s);
                            let b = shell(got, n, &c, s);
                            assert_eq!(a.len(), b.len(), "{n}");
                            assert!(!a.is_empty());
                            for (p, q) in a.iter().zip(&b) {
                                assert!(p.distance(*q) < 1e-4, "{n} {c:?} {cm}: {p} {q}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// The helmet choice and the helmet off (spec §14 Phase 4, 4N Q28)
    /// with the helmets from their files: each kit shows exactly its
    /// helmet, present once in the near and the far rider; off shows none.
    #[test]
    fn each_helmet_choice_and_helmet_off_show_one_helmet_or_none() {
        use crate::rider::{HELMET_OFF, visible};
        let c = ocw_core::Character::default();
        let near = crate::shipped::rider_model();
        let far = crate::shipped::rider_low_model();
        for (kit, h) in [
            (None, Helmet::Road),
            (Some("helmet-light"), Helmet::Light),
            (Some("helmet-aero-road"), Helmet::AeroRoad),
            (Some("helmet-tt"), Helmet::TimeTrial),
        ] {
            let kits: Vec<&str> = kit.into_iter().collect();
            let shown = visible(&c, &kits);
            let helmets: Vec<&str> = shown
                .iter()
                .copied()
                .filter(|n| helmet_nodes().contains(n))
                .collect();
            assert_eq!(helmets, [h.node()], "{kit:?}");
            for m in [near, far] {
                assert!(m.node(h.node()).is_some());
            }
            let mut off = kits.clone();
            off.push(HELMET_OFF);
            let bare = visible(&c, &off);
            assert!(
                bare.iter().all(|n| !helmet_nodes().contains(n)),
                "{kit:?} off: {bare:?}"
            );
        }
    }
}
