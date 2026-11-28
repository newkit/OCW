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
/// neck's skin ([`rider::neck::BASE_RADIUS`]) as drawn with the near
/// model's eight sides, and the far model's four.
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
    let (Some(torso), Some(head)) = (m.node(nodes::TORSO), m.node(nodes::HEAD)) else {
        return Vec::new();
    };
    let pivot = m.world(head).translation.into();
    let around = Affine3A::from_translation(pivot)
        * Affine3A::from_quat(turn)
        * Affine3A::from_translation(-pivot);
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
        let w = if below(i, head) {
            around * m.world(i)
        } else {
            m.world(i)
        };
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

/// The neck's core with the head turned by `turn`: from inside the jersey
/// (3.5 cm under the pivot, along the torso) up the head's neck (4.5 cm
/// along the turned head).
pub fn core(pivot: Vec3, turn: Quat) -> Vec<Vec3> {
    let up = turn * Vec3::Y;
    vec![
        pivot - Vec3::Y * 0.035,
        pivot - Vec3::Y * 0.018,
        pivot,
        pivot + up * 0.02,
        pivot + up * 0.045,
    ]
}

/// The first ray through the neck's core that meets no front face (the
/// background seen between the neck and the jersey), or `None`.
pub fn gap(m: &Model, detail: Detail, body: &Character, turn: Quat) -> Option<Gap> {
    let head = m.node(nodes::HEAD)?;
    let pivot: Vec3 = m.world(head).translation.into();
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

    /// The check finds the old neck's gap: an open tube from 5 cm under
    /// the pivot on the head, no base on the torso – its front leaves the
    /// jersey when the head looks up the road from a leaning torso.
    #[test]
    fn the_check_finds_the_old_necks_gap() {
        use crate::mesh::Parts;
        let mut m = rider::build(Detail::High);
        let head = m.node(nodes::HEAD).unwrap();
        let base = m.node(rider::NECK_BASE).unwrap();
        m.nodes[base].primitives.clear();
        let skin = m.material("skin").unwrap();
        // The head's skin without the new neck: replaced by the old tube.
        let mut p = Parts::new();
        let a = rider::neck_pivot();
        p.slot("skin").tube(
            a - Vec3::Y * 0.05,
            a + Vec3::new(0.0, 0.09, -0.01),
            0.05,
            0.047,
            8,
            false,
        );
        p.slot("skin").ellipsoid(
            rider::skull_centre(),
            Vec3::from(rider::SKULL_RADII),
            Quat::IDENTITY,
            7,
            8,
        );
        m.nodes[head].primitives.retain(|q| q.material != skin);
        p.attach(&mut m, head);
        let c = Character::default();
        assert!(
            gap(&m, Detail::High, &c, Quat::IDENTITY).is_none(),
            "at rest"
        );
        // Riding on the hoods: the head lifted ~0.8 rad against the torso.
        assert!(gap(&m, Detail::High, &c, Quat::from_rotation_x(0.8)).is_some());
        // The new neck holds.
        let m = rider::build(Detail::High);
        assert!(gap(&m, Detail::High, &c, Quat::from_rotation_x(0.8)).is_none());
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

    /// The neck's base is not widened or narrowed with the build.
    #[test]
    fn the_neck_base_keeps_its_girth() {
        use ocw_core::BodyShape;
        let m = rider::build(Detail::High);
        let i = m.node(rider::NECK_BASE).unwrap();
        for shape in BodyShape::ALL {
            let c = Character {
                shape,
                ..Character::default()
            };
            assert_eq!(rider::girth(&m, i, &c), Vec3::ONE);
        }
    }
}
