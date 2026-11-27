//! The cow of the UFOs' beam scene (spec 14 Phase 18, 18C *UFOs*, 18E T9;
//! Q7 (c)): a grazing cow, white with brown patches, lifted by a saucer's
//! beam and set down a field away – our own low-poly shape in the style of
//! the traffic's models (T6): flat-shaded ellipsoids and tubes, the near
//! model under the root `cow` and a far one under `lod1`.
//!
//! The contract (kind `creature`, 15B): the origin on the ground under the
//! body's centre, the head at −Z in the app (+Z in the file, as the
//! vehicles), metres (1.4 m at the withers, 2.4 m from nose to tail).

use glam::{Quat, Vec3};
use serde_json::json;

use crate::kinds::Kind;
use crate::manifest::Entry;
use crate::mesh::Shape;
use crate::model::{Model, Node, Primitive};

/// The file in `artwork/3dmodels/`.
pub const FILE: &str = "creatures/cow-grazing.glb";

/// The manifest entry.
pub fn entry() -> Entry {
    let mut e = crate::export::entry(FILE, Kind::Creature, "ocw_assets::cow::model()");
    e.source =
        "own work, generic shape made by the OCW generator ocw_assets::cow (spec 14 Phase 18, 18E T9)"
            .into();
    e
}

/// The writer's grid (1/65536 m).
fn q(v: Vec3) -> Vec3 {
    (v * 65536.0).round() / 65536.0
}

/// Shapes by slot for one level.
#[derive(Default)]
struct Level {
    parts: Vec<(&'static str, Shape)>,
}

impl Level {
    fn slot(&mut self, slot: &'static str) -> &mut Shape {
        if let Some(i) = self.parts.iter().position(|p| p.0 == slot) {
            return &mut self.parts[i].1;
        }
        self.parts.push((slot, Shape::default()));
        &mut self.parts.last_mut().expect("pushed").1
    }

    fn ellipsoid(
        &mut self,
        slot: &'static str,
        c: Vec3,
        r: Vec3,
        rot: Quat,
        bands: u32,
        sides: u32,
    ) {
        self.slot(slot).ellipsoid(c, r, rot, bands, sides);
    }

    fn tube(&mut self, slot: &'static str, a: Vec3, b: Vec3, ra: f32, rb: f32, sides: u32) {
        self.slot(slot).tube(a, b, ra, rb, sides, true);
    }

    /// Into `node` of `m`: one primitive per slot, flat-shaded (three
    /// vertices per triangle, the face's normal), on the writer's grid.
    fn attach(self, m: &mut Model, node: usize) {
        for (slot, shape) in self.parts {
            let mut pos = Vec::new();
            let mut nor = Vec::new();
            for t in shape.indices.as_chunks::<3>().0 {
                // Built with the head at +Z; the app's models face −Z (turned
                // 180° about Y; the file faces +Z again).
                let [a, b, c] = [0, 1, 2].map(|k| {
                    let p = shape.positions[t[k] as usize];
                    q(v3(-p.x, p.y, -p.z))
                });
                let n = (b - a).cross(c - a);
                if n.length_squared() < 1e-12 {
                    continue;
                }
                let n = n.normalize();
                for p in [a, b, c] {
                    pos.push(p.to_array());
                    nor.push(n.to_array());
                }
            }
            let material = m.material_or_add(crate::slots::material(slot));
            let k = pos.len() as u32;
            m.nodes[node].primitives.push(Primitive {
                material,
                positions: pos,
                normals: nor,
                indices: (0..k).collect(),
                ..Default::default()
            });
        }
    }
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// One level of the cow (built with the head at +Z, turned in `attach`).
fn cow(far: bool) -> Level {
    let mut l = Level::default();
    let (b, s) = if far { (5, 6) } else { (8, 12) };
    let id = Quat::IDENTITY;
    // The barrel and the hips, the brisket.
    l.ellipsoid("hide", v3(0.0, 1.02, 0.1), v3(0.4, 0.38, 0.78), id, b, s);
    l.ellipsoid("hide", v3(0.0, 1.06, -0.55), v3(0.38, 0.36, 0.38), id, b, s);
    // The neck and the head, its muzzle.
    let neck = Quat::from_rotation_x(-0.5);
    l.ellipsoid("hide", v3(0.0, 1.12, 0.85), v3(0.2, 0.26, 0.34), neck, b, s);
    let head = Quat::from_rotation_x(0.75);
    l.ellipsoid(
        "hide_patch",
        v3(0.0, 1.02, 1.18),
        v3(0.16, 0.17, 0.3),
        head,
        b,
        s,
    );
    l.ellipsoid(
        "muzzle",
        v3(0.0, 0.83, 1.36),
        v3(0.14, 0.11, 0.11),
        id,
        b.min(6),
        s.min(8),
    );
    // The legs and hooves.
    for (x, z) in [(-0.24, 0.58), (0.24, 0.58), (-0.24, -0.62), (0.24, -0.62)] {
        let sides = if far { 4 } else { 7 };
        l.tube(
            "hide",
            v3(x, 0.9, z),
            v3(x * 0.95, 0.12, z),
            0.1,
            0.065,
            sides,
        );
        l.tube(
            "hoof",
            v3(x * 0.95, 0.12, z),
            v3(x * 0.95, 0.0, z),
            0.07,
            0.08,
            sides,
        );
    }
    if far {
        l.ellipsoid(
            "hide_patch",
            v3(0.12, 1.12, -0.1),
            v3(0.33, 0.3, 0.42),
            id,
            4,
            6,
        );
        return l;
    }
    // Brown patches over the white hide.
    for (c, r) in [
        (v3(0.2, 1.15, 0.25), v3(0.24, 0.27, 0.34)),
        (v3(-0.22, 1.05, -0.35), v3(0.22, 0.3, 0.3)),
        (v3(-0.08, 1.28, 0.6), v3(0.24, 0.16, 0.22)),
        (v3(0.17, 0.98, -0.62), v3(0.24, 0.24, 0.2)),
    ] {
        l.ellipsoid("hide_patch", c, r, id, 6, 10);
    }
    // The ears and the horns, the eyes.
    for side in [-1.0f32, 1.0] {
        l.ellipsoid(
            "hide_patch",
            v3(side * 0.2, 1.2, 1.06),
            v3(0.12, 0.035, 0.06),
            Quat::from_rotation_z(side * 0.4),
            4,
            6,
        );
        l.tube(
            "horn_cow",
            v3(side * 0.08, 1.24, 1.08),
            v3(side * 0.2, 1.34, 1.12),
            0.03,
            0.012,
            5,
        );
        l.ellipsoid(
            "eye",
            v3(side * 0.13, 1.09, 1.2),
            v3(0.025, 0.03, 0.025),
            id,
            4,
            6,
        );
    }
    // The udder and the tail with its tuft.
    l.ellipsoid(
        "muzzle",
        v3(0.0, 0.66, -0.4),
        v3(0.16, 0.12, 0.18),
        id,
        6,
        10,
    );
    l.tube(
        "hide",
        v3(0.0, 1.3, -0.92),
        v3(0.0, 0.62, -1.02),
        0.03,
        0.022,
        5,
    );
    l.ellipsoid(
        "hide_patch",
        v3(0.0, 0.55, -1.03),
        v3(0.05, 0.1, 0.05),
        id,
        4,
        6,
    );
    l
}

/// The cow's file model, stamped (15B).
pub fn model() -> Model {
    let mut m = Model::default();
    let root = m.add(Node::new("cow", None, Vec3::ZERO));
    let lod1 = m.add(Node::new("lod1", Some(root), Vec3::ZERO));
    cow(false).attach(&mut m, root);
    cow(true).attach(&mut m, lod1);
    let e = entry();
    let (lo, hi) = m.near_bounds().expect("a mesh");
    let size = hi - lo;
    let r = |x: f32| (x * 100.0).round() / 100.0;
    crate::export::stamp(
        &mut m,
        &e,
        json!({ "size_m": { "length": r(size.z), "width": r(size.x), "height": r(size.y) } }),
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cow_is_valid_cow_sized_and_shipped() {
        let m = model();
        let bytes = crate::glb::write(&m);
        let f = crate::check::check(&entry(), &m, bytes.len());
        assert!(
            f.iter().all(|f| !f.is_error()),
            "{:#?}",
            f.iter().map(|f| f.to_string()).collect::<Vec<_>>()
        );
        let (lo, hi) = m.near_bounds().unwrap();
        let d = hi - lo;
        assert!((2.2..2.8).contains(&d.z), "length {d:?}");
        assert!((1.3..1.6).contains(&d.y), "height {d:?}");
        assert!(lo.y.abs() < 0.01, "on the ground");
        assert!(lo.z < -1.3 && hi.z < 1.2, "the head at −Z in the app");
        let far = m.triangles_below(m.node("lod1"));
        let near = m.triangles() - far;
        let (nb, fb) = Kind::Creature.budget();
        assert!(
            near <= nb && far <= fb.unwrap() && far > 0 && near > 2 * far,
            "{near} / {far}"
        );
        assert_eq!(crate::glb::write(&model()), bytes, "deterministic");
        // The shipped file is this generator's.
        let shipped = crate::library::embedded_files()
            .iter()
            .find(|f| f.0 == FILE)
            .map(|f| f.1)
            .expect("shipped");
        let a = crate::glb::read(shipped).unwrap();
        assert_eq!(a.nodes.len(), m.nodes.len());
        assert_eq!(a.triangles(), m.triangles());
    }
}
