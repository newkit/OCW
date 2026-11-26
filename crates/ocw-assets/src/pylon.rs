//! The power line pylon (`power=tower`, spec 14 Phase 12 K3 / 15H; v6.58):
//! an open steel lattice tower with two levels of cross arms and hanging
//! insulator strings – the shape of the 110 – 220 kV towers along the
//! roads of all our worlds. It replaces the solid pyramid of the first
//! export (`props/pylon-pyramid.glb`), which stood beside the roads as a
//! grey obelisk 25 – 30 m high (the Port d'Envalira, 2026-11-26).
//!
//! Exported once into `artwork/3dmodels/props/` (`cargo run -p ocw-assets
//! --example export-pylon`); the file is the source from then on, the
//! test keeps it equal to this generator while it lives. The near model
//! has the bracing (about 1,400 triangles), the far level `lod1` the legs
//! and arms only.

use glam::Vec3;
use serde_json::json;

use crate::kinds::Kind;
use crate::manifest::Entry;
use crate::mesh::Shape;
use crate::model::{Material, Model, Node, Primitive, srgb};

/// The file (spec 15H names).
pub const FILE: &str = "props/pylon-lattice.glb";
/// The reference height (the earth wire's peak above the ground) and how
/// deep the legs stand in it.
pub const HEIGHT_M: f32 = 30.0;
pub const SINK_M: f32 = 0.2;
/// The root node stands this far up: the drawn mesh starts at the origin.
const ROOT_LIFT_M: f32 = SINK_M + 0.05;
/// Half the width of the foot (the legs' square) and of the waist.
pub const FOOT_HALF_M: f32 = 2.5;
pub const WAIST_HALF_M: f32 = 0.75;
/// The cross arms: (height, half span) of the lower and upper level.
pub const ARMS: [(f32, f32); 2] = [(19.0, 5.6), (24.0, 4.2)];
/// Length of an insulator string.
pub const INSULATOR_M: f32 = 1.7;

/// A beam of square section `w` from `a` to `b` (four flat sides).
fn beam(s: &mut Shape, a: Vec3, b: Vec3, w: f32) {
    let d = (b - a).normalize_or_zero();
    if d == Vec3::ZERO {
        return;
    }
    let up = if d.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    let u = d.cross(up).normalize() * (w / 2.0);
    let v = d.cross(u).normalize() * (w / 2.0);
    let c = [u + v, u - v, -u - v, -u + v];
    for k in 0..4 {
        let (p, q) = (c[k], c[(k + 1) % 4]);
        s.add_flat(&[a + p, a + q, b + q, b + p], &[[0, 1, 2], [0, 2, 3]]);
    }
}

/// The half width of the body at height `y`.
fn half_at(y: f32) -> f32 {
    let waist = ARMS[0].0;
    if y <= waist {
        FOOT_HALF_M + (WAIST_HALF_M - FOOT_HALF_M) * (y / waist)
    } else {
        WAIST_HALF_M - 0.15 * ((y - waist) / (HEIGHT_M - 2.0 - waist))
    }
}

/// The four legs' corners at height `y` (x, z signs).
fn corners(y: f32) -> [Vec3; 4] {
    let h = half_at(y);
    [
        Vec3::new(h, y, h),
        Vec3::new(-h, y, h),
        Vec3::new(-h, y, -h),
        Vec3::new(h, y, -h),
    ]
}

/// The tower's steel (`detail`: with the bracing).
fn steel(detail: bool) -> Shape {
    let mut s = Shape::default();
    let top = HEIGHT_M - 2.0;
    let leg_w = if detail { 0.22 } else { 0.4 };
    // Panel heights: tall panels low down, shorter towards the waist.
    let levels: Vec<f32> = [
        -SINK_M, 3.6, 6.8, 9.6, 12.2, 14.6, 16.9, ARMS[0].0, 21.5, ARMS[1].0, 26.0, top,
    ]
    .to_vec();
    for k in 0..4 {
        beam(&mut s, corners(-SINK_M)[k], corners(ARMS[0].0)[k], leg_w);
        beam(&mut s, corners(ARMS[0].0)[k], corners(top)[k], leg_w * 0.8);
    }
    if detail {
        for w in levels.windows(2) {
            let (lo, hi) = (corners(w[0]), corners(w[1]));
            for k in 0..4 {
                let n = (k + 1) % 4;
                // Horizontal struts and the X of each face.
                beam(&mut s, hi[k], hi[n], 0.1);
                beam(&mut s, lo[k], hi[n], 0.08);
                beam(&mut s, lo[n], hi[k], 0.08);
            }
        }
    }
    // The cross arms: a lower chord and two upper chords meeting at the tip.
    for (y, span) in ARMS {
        for side in [-1.0f32, 1.0] {
            let h = half_at(y);
            let tip = Vec3::new(side * span, y, 0.0);
            let (a0, a1) = (Vec3::new(side * h, y, h), Vec3::new(side * h, y, -h));
            let up = 1.6;
            let (b0, b1) = (
                Vec3::new(side * h, y + up, h * 0.6),
                Vec3::new(side * h, y + up, -h * 0.6),
            );
            for (a, w) in [(a0, 0.14), (a1, 0.14), (b0, 0.12), (b1, 0.12)] {
                beam(&mut s, a, tip, w);
            }
        }
    }
    // The earth wire peak.
    let c = corners(top);
    for p in c {
        beam(&mut s, p, Vec3::new(0.0, HEIGHT_M, 0.0), 0.12);
    }
    s
}

/// The insulator strings hanging from the arms' tips.
fn insulators() -> Shape {
    let mut s = Shape::default();
    for (y, span) in ARMS {
        for side in [-1.0f32, 1.0] {
            let tip = Vec3::new(side * (span - 0.15), y, 0.0);
            s.tube(tip, tip - Vec3::Y * INSULATOR_M, 0.11, 0.11, 6, true);
        }
    }
    s
}

fn primitive(shape: Shape, material: usize) -> Primitive {
    Primitive {
        material,
        positions: shape.positions.iter().map(|p| p.to_array()).collect(),
        normals: shape.normals.iter().map(|n| n.to_array()).collect(),
        indices: shape.indices,
        colors: Vec::new(),
        uvs: Vec::new(),
    }
}

/// The manifest entry of the file.
pub fn entry() -> Entry {
    Entry {
        file: FILE.into(),
        kind: Kind::Prop,
        profile: None,
        species: None,
        variant: None,
        anchor: None,
        generator: Some("ocw_assets::pylon::lattice()".into()),
        license: crate::export::LICENSE.into(),
        source: "own work, exported once from the OCW generator ocw_assets::pylon (spec 14 Phase 15 MX); the file is the source (Q6)".into(),
        author: crate::export::AUTHOR.into(),
    }
}

/// The lattice pylon (see the module): the root `pylon_lattice` with the
/// steel and the insulators (the slot `paint_dark`), `lod1` (legs and
/// arms) and the collision proxy `col_cylinder` of the old file (radius
/// 2.5 m). Within the props' budgets (1,500 and 300 triangles).
pub fn lattice() -> Model {
    let mut m = Model::default();
    let steel_mat = m.material_or_add(Material {
        name: "steel".into(),
        color: srgb(0x8C9095),
        metallic: 0.6,
        roughness: 0.5,
        emissive: [0.0; 3],
    });
    let glass = m.material_or_add(Material {
        name: "paint_dark".into(),
        color: srgb(0x4A5A52),
        metallic: 0.0,
        roughness: 0.3,
        emissive: [0.0; 3],
    });
    // The mesh starts at the origin (15B: the fit lowers it by `SINK_M`):
    // the legs are drawn from `-SINK_M`, a few centimetres of beam below.
    let root = m.add(Node::new("pylon_lattice", None, Vec3::Y * ROOT_LIFT_M));
    m.nodes[root].primitives = vec![
        primitive(steel(true), steel_mat),
        primitive(insulators(), glass),
    ];
    let lod = m.add(Node::new("lod1", Some(root), Vec3::ZERO));
    m.nodes[lod].primitives = vec![primitive(steel(false), steel_mat)];
    let mut col = Node::new(
        "col_cylinder",
        Some(root),
        Vec3::new(0.0, HEIGHT_M / 2.0 + SINK_M - ROOT_LIFT_M, 0.0),
    );
    col.scale = Vec3::new(FOOT_HALF_M, HEIGHT_M / 2.0, FOOT_HALF_M);
    m.add(col);
    let e = entry();
    crate::export::stamp(
        &mut m,
        &e,
        json!({
            "fit": { "height_m": HEIGHT_M, "sink_m": SINK_M },
            "osm": "power=tower",
        }),
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The front view's silhouette: the share of a grid over the model's
    /// box (x, y) that its near triangles cover.
    fn coverage(m: &Model) -> f32 {
        let tris: Vec<[Vec3; 3]> = m
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| !m.under_named(*i, |n| n.starts_with("col_") || n == "lod1"))
            .flat_map(|(i, n)| {
                let w = m.world(i);
                n.primitives.iter().flat_map(move |p| {
                    p.indices.chunks(3).map(move |t| {
                        [t[0], t[1], t[2]]
                            .map(|k| w.transform_point3(Vec3::from(p.positions[k as usize])))
                    })
                })
            })
            .collect();
        let (lo, hi) = m.near_bounds().unwrap();
        let (nx, ny) = (60, 120);
        let mut hit = 0;
        for j in 0..ny {
            for i in 0..nx {
                let x = lo.x + (i as f32 + 0.5) / nx as f32 * (hi.x - lo.x);
                let y = lo.y + (j as f32 + 0.5) / ny as f32 * (hi.y - lo.y);
                let o = Vec3::new(x, y, hi.z + 1.0);
                if tris
                    .iter()
                    .any(|t| crate::mesh::ray_triangle(o, Vec3::NEG_Z, t).is_some())
                {
                    hit += 1;
                }
            }
        }
        hit as f32 / (nx * ny) as f32
    }

    /// An open lattice, not a solid obelisk: from the front it covers
    /// little of its box (the pyramid covered half), it stands 30 m with
    /// its legs 0.2 m in the ground, has its far level and the old
    /// collision proxy.
    #[test]
    fn the_pylon_is_an_open_lattice() {
        let m = lattice();
        let (lo, hi) = m.near_bounds().unwrap();
        assert!(
            (hi.y - HEIGHT_M - ROOT_LIFT_M).abs() < 0.1 && lo.y >= 0.0 && lo.y < 0.1,
            "{lo} {hi}"
        );
        assert!(hi.x > 5.0 && lo.x < -5.0, "the arms reach out: {lo} {hi}");
        let c = coverage(&m);
        assert!(c < 0.25, "covers {:.0} % of its box", c * 100.0);
        assert!(crate::props::has_lod1(&m));
        assert!(!crate::props::proxies(&m).is_empty());
        let fit = crate::props::FileFit::of(&m).unwrap();
        assert_eq!((fit.height_m, fit.sink_m), (HEIGHT_M, SINK_M));
        let near = m.triangles_below(Some(0)) - m.triangles_below(m.node("lod1"));
        assert!(
            near <= 1_500 && m.triangles_below(m.node("lod1")) <= 300,
            "{near}"
        );
        // The pyramid it replaces covered half of its box.
        let old = crate::shipped::model("props/pylon-pyramid");
        assert!(coverage(&old) > 0.4);
    }

    /// The shipped file is this generator's export (the writer is
    /// deterministic).
    #[test]
    fn the_shipped_file_is_the_generator_s() {
        let shipped = crate::library::embedded_files()
            .iter()
            .find(|f| f.0 == FILE)
            .map(|f| f.1)
            .expect("props/pylon-lattice.glb is shipped");
        assert_eq!(shipped, crate::glb::write(&lattice()).as_slice());
    }
}
