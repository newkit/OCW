//! The neck between the head and the jersey (bug 2026-11-28, spec §14
//! Phase 4, 4N *Fixed: the neck and the jersey*: "in some photos it
//! appears that the neck of the rider comes off the jersey, there is a gap
//! between jersey and the lower end of the neck"): the head and the torso
//! are rigid nodes, so the head's neck turns against the jersey's
//! shoulders in every pose. This module looks at the seam as a camera
//! does: rays from all round (front, back, sides, three-quarter, above
//! and below) through the neck's core – from inside the jersey up past
//! the head's pivot – must meet a surface that faces them (skin or
//! jersey; back faces are culled when drawn), so no background shows
//! through between the jersey and the neck.
//!
//! The geometry only depends on the head's turn against the torso
//! ([`crate::pose::Pose::head`]), the build's girth on the torso and the
//! level of detail, so the check runs on the distinct head turns of every
//! pose of the pose system ([`crate::clothing::poses`]) plus a grid of
//! extreme turns ([`extreme_turns`]), for every body of
//! [`crate::clothing::bodies`], near and far.

use glam::{Affine3A, Quat, Vec3};
use ocw_core::Character;

use crate::model::Model;
use crate::pose::{RiderRig, nodes};
use crate::rider::{self, Detail};

/// How far round the neck's core the rays pass (m): well inside the
/// neck's skin ([`rider::neck::RINGS`], at least 4.4 cm by 4.7 cm round)
/// as drawn with the near model's twelve sides and the far model's six.
pub fn probe_radius(detail: Detail) -> f32 {
    match detail {
        Detail::High => 0.03,
        Detail::Low => 0.02,
    }
}

/// The head turns beyond the poses (rad, in the torso's frame: x the
/// nod – positive lifts the face –, y the turn, z the tilt): further than
/// any pose goes, so a later pose stays covered.
pub fn extreme_turns() -> Vec<Quat> {
    let mut out = Vec::new();
    for pitch in [-0.6f32, -0.3, 0.0, 0.4, 0.8, 1.1, 1.3] {
        for yaw in [-1.4f32, -0.8, 0.0, 0.8, 1.4] {
            for roll in [-0.4f32, 0.0, 0.4] {
                out.push(
                    Quat::from_rotation_y(yaw)
                        * Quat::from_rotation_z(roll)
                        * Quat::from_rotation_x(pitch),
                );
            }
        }
    }
    out
}

/// The distinct head turns of the pose system (against the torso),
/// rounded to 0.01 rad, with [`extreme_turns`].
pub fn head_turns(rig: &RiderRig) -> Vec<Quat> {
    let mut out: Vec<Quat> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for p in crate::clothing::poses(rig)
        .into_iter()
        .map(|p| p.pose.head)
        .chain(extreme_turns())
    {
        let q = if p.w < 0.0 { -p } else { p };
        let key = q.to_array().map(|c| (c * 100.0).round() as i32);
        if seen.insert(key) {
            out.push(q);
        }
    }
    out
}

/// The surfaces round the neck of `m` drawn for `body` with the head
/// turned by `turn` (rest-pose model coordinates of the torso, the girth
/// as drawn): every primitive of the torso, the head and the nodes they
/// carry that every look shows (not the kit's or the character's
/// optional nodes: hair, beards and helmets must not be needed to hide a
/// gap), within `reach` of the head's pivot.
pub fn surfaces(m: &Model, body: &Character, turn: Quat, reach: f32) -> Vec<[Vec3; 3]> {
    let optional = rider::optional();
    let Some(torso) = m.node(nodes::TORSO) else {
        return Vec::new();
    };
    let worlds = posed(m, turn);
    let pivot = neck_joint(m);
    let below = |i: usize, top: usize| {
        let mut at = Some(i);
        while let Some(n) = at {
            if n == top {
                return true;
            }
            at = m.nodes[n].parent;
        }
        false
    };
    let mut out = Vec::new();
    for (i, node) in m.nodes.iter().enumerate() {
        // The torso and the head with what they carry, not the arms (they
        // are not where the pose puts them here).
        let arm = nodes::UPPER_ARM
            .iter()
            .filter_map(|n| m.node(n))
            .any(|a| below(i, a));
        if !below(i, torso) || arm || optional.contains(&node.name.as_str()) {
            continue;
        }
        // An optional node's children are optional too.
        let mut at = node.parent;
        let mut shown = true;
        while let Some(n) = at {
            if optional.contains(&m.nodes[n].name.as_str()) {
                shown = false;
            }
            at = m.nodes[n].parent;
        }
        if !shown {
            continue;
        }
        let w = worlds[i];
        let (s, t) = rider::girth_transform(m, i, body);
        for p in &node.primitives {
            let pts: Vec<Vec3> = p
                .positions
                .iter()
                .map(|v| w.transform_point3(Vec3::from(*v) * s + t))
                .collect();
            for tri in p.indices.as_chunks::<3>().0 {
                let tri = tri.map(|k| pts[k as usize]);
                if tri.iter().any(|q| q.distance(pivot) < reach) {
                    out.push(tri);
                }
            }
        }
    }
    out
}

/// The model-space transforms of the nodes of `m` with the head turned
/// `turn` against the torso (the rest of the body at rest): the neck and
/// the head each take their share ([`crate::pose::neck_split`]).
pub fn posed(m: &Model, turn: Quat) -> Vec<Affine3A> {
    let mut k = Model {
        nodes: m
            .nodes
            .iter()
            .map(|n| crate::model::Node {
                primitives: Vec::new(),
                ..n.clone()
            })
            .collect(),
        ..Default::default()
    };
    let (neck, head) = crate::pose::neck_split(turn);
    for (name, q) in [(nodes::NECK, neck), (nodes::HEAD, head)] {
        if let Some(i) = k.node(name) {
            k.nodes[i].rotation = q;
        }
    }
    (0..k.nodes.len()).map(|i| k.world(i)).collect()
}

/// The neck's joint (rest-pose model coordinates; the head's for a model
/// without a neck node).
pub fn neck_joint(m: &Model) -> Vec3 {
    m.node(nodes::NECK)
        .or_else(|| m.node(nodes::HEAD))
        .map_or(Vec3::ZERO, |i| m.world(i).translation.into())
}

/// Where a ray from `o` along `d` meets the front of triangle `t` (its
/// counter-clockwise side; a little tolerance at the edges, so a ray along
/// a shared edge meets one of the two).
fn front_hit(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    if e1.cross(e2).dot(d) >= 0.0 {
        return None;
    }
    let h = d.cross(e2);
    let a = e1.dot(h);
    if a.abs() < 1e-14 {
        return None;
    }
    let f = 1.0 / a;
    let s = o - t[0];
    let u = f * s.dot(h);
    let q = s.cross(e1);
    let v = f * d.dot(q);
    let k = f * e2.dot(q);
    const E: f32 = 1e-4;
    (u >= -E && v >= -E && u + v <= 1.0 + E && k > 0.0).then_some(k)
}

/// The camera directions of the rays: the 26 of a cube's faces, edges and
/// corners, without straight up from under the rider.
pub fn directions() -> Vec<Vec3> {
    let mut out = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) == (0, 0, 0) || (x, y, z) == (0, -1, 0) {
                    continue;
                }
                // From the camera at (x, y, z) towards the neck.
                out.push(-Vec3::new(x as f32, y as f32, z as f32).normalize());
            }
        }
    }
    out
}

/// A ray that shows the background through the neck.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    /// Where it passes the neck's core (rest-pose model coordinates of the
    /// torso).
    pub at: Vec3,
    /// Its direction.
    pub dir: Vec3,
}

/// The neck's core with the head turned by `turn`: from inside the
/// jersey's collar (3.5 cm under the neck's joint, along the torso) up the
/// bent neck to its top under the skull.
pub fn core(pivot: Vec3, turn: Quat) -> Vec<Vec3> {
    let (neck, _) = crate::pose::neck_split(turn);
    let up = neck * Vec3::Y;
    vec![
        pivot - Vec3::Y * 0.035,
        pivot - Vec3::Y * 0.018,
        pivot,
        pivot + up * 0.025,
        pivot + up * 0.05,
        pivot + up * 0.075,
    ]
}

/// The first ray through the neck's core that meets no front face (the
/// background seen between the neck and the jersey), or `None`.
pub fn gap(m: &Model, detail: Detail, body: &Character, turn: Quat) -> Option<Gap> {
    m.node(nodes::HEAD)?;
    let pivot = neck_joint(m);
    let tris = surfaces(m, body, turn, 0.4);
    let r = probe_radius(detail);
    for at in core(pivot, turn) {
        for dir in directions() {
            let (e1, e2) = dir.any_orthonormal_pair();
            let offsets = std::iter::once(Vec3::ZERO).chain((0..8).map(|k| {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                (e1 * a.cos() + e2 * a.sin()) * r
            }));
            for off in offsets {
                let o = at + off - dir * 1.0;
                if !tris.iter().any(|t| front_hit(o, dir, t).is_some()) {
                    return Some(Gap { at: at + off, dir });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every gap of `m` over the bodies and head turns, as messages.
    fn gaps(m: &Model, detail: Detail, what: &str) -> Vec<String> {
        let rig = RiderRig::from_model(m).unwrap();
        let turns = head_turns(&rig);
        assert!(turns.len() > 100, "{} head turns", turns.len());
        // The distinct torso girths (the neck's surfaces depend on nothing
        // else of the body).
        let mut bodies: Vec<Character> = Vec::new();
        for b in crate::clothing::bodies() {
            let g = rider::girth_of(nodes::TORSO, &b);
            if !bodies
                .iter()
                .any(|o| rider::girth_of(nodes::TORSO, o).distance(g) < 1e-4)
            {
                bodies.push(b);
            }
        }
        std::thread::scope(|s| {
            let jobs: Vec<_> = bodies
                .iter()
                .map(|b| {
                    let turns = &turns;
                    s.spawn(move || {
                        let mut out = Vec::new();
                        for q in turns {
                            if let Some(g) = gap(m, detail, b, *q) {
                                let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
                                out.push(format!(
                                    "{what} {detail:?} {:?} {:?} {:?}: background through the \
                                     neck at {:?} along {:?}, head turned (yaw {y:.2}, nod \
                                     {x:.2}, tilt {z:.2})",
                                    b.base, b.shape, b.age, g.at, g.dir
                                ));
                                break;
                            }
                        }
                        out
                    })
                })
                .collect();
            jobs.into_iter()
                .flat_map(|j| j.join().expect("checked"))
                .collect()
        })
    }

    /// The regression test of the bug of 2026-11-28 (a gap between the
    /// jersey and the neck in riding pictures): for every pose's head turn
    /// and the extreme ones, every body build and both levels of detail,
    /// in the generator and in the shipped files, no ray through the
    /// neck's core sees the background.
    #[test]
    fn no_gap_between_the_jersey_and_the_neck() {
        let mut f = Vec::new();
        for detail in [Detail::High, Detail::Low] {
            f.extend(gaps(&rider::build(detail), detail, "generator"));
        }
        f.extend(gaps(
            &crate::shipped::model(crate::RIDER_ID),
            Detail::High,
            "file",
        ));
        f.extend(gaps(
            &crate::shipped::model(crate::RIDER_LOW_ID),
            Detail::Low,
            "file",
        ));
        assert!(f.is_empty(), "{} gaps:\n{}", f.len(), f.join("\n"));
    }

    /// The check finds a missing neck: without the neck's skin the riding
    /// turn shows the background between the jersey and the skull.
    #[test]
    fn the_check_finds_a_missing_neck() {
        let mut m = rider::build(Detail::High);
        let c = Character::default();
        let hoods = Quat::from_rotation_x(0.8);
        assert!(gap(&m, Detail::High, &c, hoods).is_none());
        let neck = m.node(nodes::NECK).unwrap();
        m.nodes[neck].primitives.clear();
        assert!(gap(&m, Detail::High, &c, hoods).is_some());
    }

    /// How many head turns show a gap in the shipped files, per build
    /// (`cargo test -p ocw-assets --lib neck::tests::report -- --ignored
    /// --nocapture`): the before / after numbers of the fix.
    #[test]
    #[ignore]
    fn report() {
        for (id, detail) in [
            (crate::RIDER_ID, Detail::High),
            (crate::RIDER_LOW_ID, Detail::Low),
        ] {
            let m = crate::shipped::model(id);
            let rig = RiderRig::from_model(&m).unwrap();
            let posed: Vec<Quat> = crate::clothing::poses(&rig)
                .into_iter()
                .map(|p| p.pose.head)
                .collect();
            for shape in ocw_core::BodyShape::ALL {
                let c = Character {
                    shape,
                    ..Character::default()
                };
                let n = posed
                    .iter()
                    .step_by(7)
                    .filter(|q| gap(&m, detail, &c, **q).is_some())
                    .count();
                let total = posed.len().div_ceil(7);
                println!("{id} {shape:?}: {n} of {total} poses with a gap");
            }
        }
    }

    /// The neck follows a share of the build's girth: thicker on a heavy
    /// build, slimmer on a thin one, never the head's (which keeps 1).
    #[test]
    fn the_neck_follows_the_build() {
        use ocw_core::BodyShape;
        let m = rider::build(Detail::High);
        let i = m.node(nodes::NECK).unwrap();
        let g = |shape| {
            let c = Character {
                shape,
                ..Character::default()
            };
            rider::girth(&m, i, &c).x
        };
        assert_eq!(g(BodyShape::Normal), 1.0);
        assert!(g(BodyShape::SuperSkinny) < 0.95 && g(BodyShape::SuperSkinny) > 0.9);
        assert!(g(BodyShape::Obese) > 1.08 && g(BodyShape::Obese) < 1.15);
    }

    /// The neck is one smooth skin (bug 2026-11-28: "do not make the skin
    /// color different"): one primitive in the head's skin material, every
    /// position with one normal (no hard edge), the normals of the throat
    /// turning less than 35 degrees from ring to ring, near and far, in the
    /// generator and the files.
    #[test]
    fn the_neck_is_one_smooth_skin() {
        for (what, m) in [
            ("near", std::sync::Arc::new(rider::build(Detail::High))),
            ("far", std::sync::Arc::new(rider::build(Detail::Low))),
            ("near file", crate::shipped::model(crate::RIDER_ID)),
            ("far file", crate::shipped::model(crate::RIDER_LOW_ID)),
        ] {
            let skin = m.material("skin").expect("skin");
            let head = m.node(nodes::HEAD).unwrap();
            assert!(
                m.nodes[head].primitives.iter().any(|p| p.material == skin),
                "{what}: the head's skin"
            );
            let neck = &m.nodes[m.node(nodes::NECK).unwrap()];
            assert_eq!(neck.primitives.len(), 1, "{what}");
            let p = &neck.primitives[0];
            assert_eq!(p.material, skin, "{what}: the head's skin material");
            let mut seen: Vec<([f32; 3], [f32; 3])> = Vec::new();
            for (q, n) in p.positions.iter().zip(&p.normals) {
                for (q2, n2) in &seen {
                    if Vec3::from(*q).distance(Vec3::from(*q2)) < 1e-5 {
                        assert!(
                            Vec3::from(*n).distance(Vec3::from(*n2)) < 1e-3,
                            "{what}: a hard edge at {q:?}"
                        );
                    }
                }
                seen.push((*q, *n));
            }
            // The front line of the neck (the throat): the vertex nearest
            // the front on each ring (the node's frame is the neck's joint),
            // bottom to top.
            let front: Vec<Vec3> = rider::neck::RINGS
                .iter()
                .map(|r| {
                    (0..p.positions.len())
                        .filter(|&k| (p.positions[k][1] - r[0]).abs() < 0.004)
                        .min_by(|&a, &b| p.positions[a][2].total_cmp(&p.positions[b][2]))
                        .map(|k| Vec3::from(p.normals[k]))
                        .unwrap_or_else(|| panic!("{what}: the ring at {}", r[0]))
                })
                .collect();
            for w in front.windows(2) {
                let a = w[0].angle_between(w[1]).to_degrees();
                assert!(a < 35.0, "{what}: the throat's shading steps {a} degrees");
            }
        }
    }
}
