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
//!   at the torso / the jersey.
//!
//! (N8 named them `hair/<id>.glb`, `face/…`, `rider-a|b|c.glb`; the file
//! contract of Phase 15, 15H, puts every wearable in `wear/<name>.glb`.)
//!
//! Each part file is a wearable (spec §14 Phase 15, 15E): its root node is
//! named after the rider node it hangs from (its `anchor`), the piece below
//! it as in the generator. Loading attaches every part under its anchor
//! ([`merge`]): the renderer, the pose and the clothing checks keep the
//! one rig and the optional nodes' names as before.

use crate::model::{Model, Node};
use crate::rider::opt;

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
pub const BEARD_BUDGET: usize = 400;
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

/// `full` without the part files' nodes (and what is below them): the
/// near rider file.
pub fn strip(full: &Model) -> Model {
    let parts: Vec<usize> = part_files()
        .iter()
        .filter_map(|p| full.node(p.node))
        .collect();
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
        .collect();
    let refs: Vec<&Model> = parts.iter().collect();
    merge(&strip(full), &refs).expect("the generator's anchors")
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
        assert_eq!(files.len(), 19 + 19 + 5 + 7 + 5 + 10);
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
        let heaviest =
            base.triangles() + hair + most(&opt::MOUSTACHE) + most(&opt::BEARD) + body + hands;
        assert!(
            heaviest <= crate::manifest::MAX_TRIANGLES_RIDER,
            "{heaviest} (file {}, hair {hair}, body {body})",
            base.triangles()
        );
        assert!(base.triangles() < full.triangles());
    }
}
