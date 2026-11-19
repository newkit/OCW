//! The rider's legs joined to the body (spec 2.6, Phase 4 4N; bug
//! 2026-11-19 "the legs of the rider are not connected to the body"):
//! the thighs are rigid nodes turning about the hip joints in the
//! pelvis, so their tops must stay in the body in every pose. This
//! module measures it for every body (each shape on each base body, the
//! oldest age's fuller middle, [`crate::clothing::bodies`]) at every
//! body height and in every pose of the pose system
//! ([`crate::clothing::poses`]):
//!
//! - the hip joints lie inside the bib's seat on the pelvis
//!   ([`crate::rider::SEAT`]): the shorts cover the joint;
//! - every point of each leg's top ring (the leg garment's first ring:
//!   the bib's, the skinsuit's) lies in the body – the seat or the
//!   jersey's shell above its hem – within [`MAX_GAP_M`]: no open tube
//!   end beside the seat, no gap between the seat and the thigh.
//!
//! The body is measured as drawn: the faceted seat and jersey of the
//! level of detail, each node's girth ([`rider::girth_transform`]) and
//! the hips as wide as the body's seat ([`rider::node_translation`]).

use glam::{Affine3A, Vec2, Vec3};
use ocw_core::Character;

use crate::clothing::{inside_distance, posed_worlds, shell_section};
use crate::model::Model;
use crate::pose::{Pose, nodes};
use crate::rider::{self, Detail};

/// The largest gap (m) allowed between a leg's top ring and the body:
/// below what a close orbit shows.
pub const MAX_GAP_M: f32 = 0.004;

/// How deep (m) the hip joints lie at least inside the seat (its faceted
/// sides and its top).
pub const JOINT_DEPTH_M: f32 = 0.005;

/// The seat's faceted section at height `y` (rest-pose pelvis frame,
/// girth taken out): the loft's corners in (x, z), the rings joined by
/// straight edges, closing to the bottom point; `None` outside its
/// height.
pub fn seat_section(detail: Detail, y: f32) -> Option<Vec<Vec2>> {
    let n = rider::seat_sides(detail);
    let top = rider::SEAT[rider::SEAT.len() - 1][0];
    if !(rider::SEAT_BOTTOM..=top).contains(&y) {
        return None;
    }
    let corner = |r: &[f32; 4], k: u32| {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        Vec2::new(r[1] * a.cos(), r[3] - r[2] * a.sin())
    };
    Some(
        (0..n)
            .map(|k| {
                let first = &rider::SEAT[0];
                if y <= first[0] {
                    // Down to the point under the hips' centre.
                    let t = (y - rider::SEAT_BOTTOM) / (first[0] - rider::SEAT_BOTTOM);
                    return corner(first, k) * t;
                }
                let i = rider::SEAT
                    .windows(2)
                    .position(|w| y <= w[1][0])
                    .unwrap_or(rider::SEAT.len() - 2);
                let (a, b) = (&rider::SEAT[i], &rider::SEAT[i + 1]);
                let t = (y - a[0]) / (b[0] - a[0]);
                corner(a, k).lerp(corner(b, k), t)
            })
            .collect(),
    )
}

/// How far `p` (rest-pose pelvis frame, girth taken out) lies outside the
/// seat (m; negative: inside).
pub fn outside_seat(detail: Detail, p: Vec3) -> f32 {
    let top = rider::SEAT[rider::SEAT.len() - 1][0];
    let y = p.y.clamp(rider::SEAT_BOTTOM, top);
    let section = seat_section(detail, y).expect("in the seat's height");
    let across = -inside_distance(&section, Vec2::new(p.x, p.z));
    let along = (p.y - top).max(rider::SEAT_BOTTOM - p.y);
    if along > 0.0 {
        across.max(0.0).hypot(along)
    } else {
        // Inside its height: the nearer of the sides and the ends.
        across.max(along)
    }
}

/// How far `p` (rest-pose torso frame, girth taken out) lies outside the
/// jersey's shell above its hem (m; negative: inside).
pub fn outside_jersey(detail: Detail, p: Vec3) -> f32 {
    let Some(section) = shell_section(detail, p.y) else {
        return f32::MAX;
    };
    let across = -inside_distance(&section, Vec2::new(p.x, p.z));
    across.max(-rider::above_hem(p))
}

/// The worst place of a check: the gap (m; negative: inside the body by
/// that much), the node, the pose and the point (pelvis frame).
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    pub gap: f32,
    pub node: String,
    pub pose: String,
    pub at: Vec3,
}

fn worse(a: Option<Gap>, b: Gap) -> Option<Gap> {
    Some(match a {
        Some(a) if a.gap >= b.gap => a,
        _ => b,
    })
}

/// The thighs' leg garments checked: (node, primitive) of every `shorts`
/// or skinsuit (`jersey`) mesh on a thigh node or below it on an
/// optional node of the thigh that starts at the hip (the long shorts
/// and tights begin lower, on the thigh's skin).
fn leg_tops(m: &Model) -> Vec<(usize, usize)> {
    let mats: Vec<usize> = ["shorts", "jersey"]
        .iter()
        .filter_map(|s| m.material(s))
        .collect();
    let mut out = Vec::new();
    for (s, name) in nodes::THIGH.iter().enumerate() {
        let thigh = m.node(name).expect("thigh");
        let suit = m.node(rider::opt::SUIT_THIGH[s]);
        for i in std::iter::once(thigh).chain(suit) {
            for (k, p) in m.nodes[i].primitives.iter().enumerate() {
                if mats.contains(&p.material) {
                    out.push((i, k));
                }
            }
        }
    }
    out
}

/// The points of a leg garment's top ring (rest-pose, node `i`'s frame):
/// the highest ring of at least three vertices, and whatever closes it
/// above (a dome's apex, a cap).
fn top_ring(m: &Model, i: usize, k: usize) -> Vec<Vec3> {
    let pts: Vec<Vec3> = m.nodes[i].primitives[k]
        .positions
        .iter()
        .map(|v| Vec3::from(*v))
        .collect();
    let mut ys: Vec<f32> = pts.iter().map(|p| p.y).collect();
    ys.sort_by(|a, b| b.total_cmp(a));
    let ring = ys
        .iter()
        .find(|y| ys.iter().filter(|o| (*o - *y).abs() < 1e-4).count() >= 3)
        .copied()
        .unwrap_or(ys[0]);
    pts.into_iter().filter(|p| p.y >= ring - 1e-4).collect()
}

/// The legs against the body in one set of node transforms (`worlds`,
/// [`posed_worlds`]) for `body`: the worst hip joint (how far it lies
/// less than [`JOINT_DEPTH_M`] inside the seat; ≤ 0 passes) and the
/// worst leg top (its gap to the body; ≤ [`MAX_GAP_M`] passes).
pub fn legs_on_body(
    m: &Model,
    detail: Detail,
    body: &Character,
    worlds: &[Affine3A],
    pose: &str,
) -> (Gap, Gap) {
    let pelvis = m.node(nodes::PELVIS).expect("pelvis");
    let torso = m.node(nodes::TORSO).expect("torso");
    let gp = rider::girth(m, pelvis, body);
    let cp = rider::girth_centre(m, pelvis);
    let gt = rider::girth(m, torso, body);
    let to_pelvis = worlds[pelvis].inverse();
    let to_torso = worlds[torso].inverse();
    let torso_rest = m.world(torso);
    // A posed point in the pelvis's and the torso's rest frames, the
    // girth taken out (the seat and the shell are drawn scaled).
    let in_pelvis = |w: Vec3| {
        let l = to_pelvis.transform_point3(w);
        Vec3::new(cp.x + (l.x - cp.x) / gp.x, l.y, cp.z + (l.z - cp.z) / gp.z)
    };
    let in_torso = |w: Vec3| {
        let l = to_torso.transform_point3(w);
        torso_rest.transform_point3(Vec3::new(l.x / gt.x, l.y, l.z / gt.z))
    };
    let mut joint: Option<Gap> = None;
    for name in nodes::THIGH {
        let i = m.node(name).expect("thigh");
        let at = in_pelvis(worlds[i].transform_point3(Vec3::ZERO));
        // The faceted seat's depth, scaled back to metres (the smaller
        // girth: the conservative one).
        let depth = -outside_seat(detail, at) * gp.x.min(gp.z);
        joint = worse(
            joint,
            Gap {
                gap: JOINT_DEPTH_M - depth,
                node: name.to_string(),
                pose: pose.to_string(),
                at,
            },
        );
    }
    let mut top: Option<Gap> = None;
    for (i, k) in leg_tops(m) {
        let (s, t) = rider::girth_transform(m, i, body);
        // The skinsuit's legs meet the skinsuit's seat, a little larger.
        let suit = if rider::opt::SUIT_THIGH.contains(&m.nodes[i].name.as_str()) {
            rider::SUIT_SEAT
        } else {
            0.0
        };
        for v in top_ring(m, i, k) {
            let w = worlds[i].transform_point3(v * s + t);
            let p = in_pelvis(w);
            let seat = (outside_seat(detail, p) - suit) * gp.x.max(gp.z);
            let jersey = outside_jersey(detail, in_torso(w)) * gt.x.max(gt.z);
            top = worse(
                top,
                Gap {
                    gap: seat.min(jersey),
                    node: m.nodes[i].name.clone(),
                    pose: pose.to_string(),
                    at: p,
                },
            );
        }
    }
    (joint.expect("two hips"), top.expect("leg tops"))
}

/// [`legs_on_body`] in `pose` (`None`: at rest).
pub fn legs_in_pose(
    m: &Model,
    detail: Detail,
    body: &Character,
    pose: Option<&Pose>,
    name: &str,
) -> (Gap, Gap) {
    legs_on_body(m, detail, body, &posed_worlds(m, pose, body), name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clothing::{bodies, poses};
    use crate::pose::RiderRig;

    /// The worst joint and leg top of `m` over every pose for `body`.
    fn worst(m: &Model, detail: Detail, body: &Character) -> (Gap, Gap) {
        let rig = RiderRig::from_model(m).unwrap().for_body(body);
        let mut out = legs_in_pose(m, detail, body, None, "rest");
        for p in poses(&rig) {
            let (j, t) = legs_in_pose(m, detail, body, Some(&p.pose), &p.name);
            if j.gap > out.0.gap {
                out.0 = j;
            }
            if t.gap > out.1.gap {
                out.1 = t;
            }
        }
        out
    }

    /// The failures of `m` over every body (in parallel).
    fn failures(m: &Model, detail: Detail) -> Vec<String> {
        let found: Vec<(Character, (Gap, Gap))> = std::thread::scope(|s| {
            let jobs: Vec<_> = bodies()
                .into_iter()
                .map(|b| s.spawn(move || (b, worst(m, detail, &b))))
                .collect();
            jobs.into_iter()
                .map(|j| j.join().expect("checked"))
                .collect()
        });
        let mut out = Vec::new();
        for (b, (joint, top)) in found {
            let build = (b.base, b.shape, b.age);
            if joint.gap > 0.0 {
                out.push(format!(
                    "{detail:?} {build:?}: hip joint of {} only {:.1} mm inside the seat \
                     (min {:.1} mm) at {:?} in pose {}",
                    joint.node,
                    (JOINT_DEPTH_M - joint.gap) * 1000.0,
                    JOINT_DEPTH_M * 1000.0,
                    joint.at,
                    joint.pose
                ));
            }
            if top.gap > MAX_GAP_M {
                out.push(format!(
                    "{detail:?} {build:?}: the top of {} is {:.1} mm off the body \
                     (max {:.1} mm) at {:?} in pose {}",
                    top.node,
                    top.gap * 1000.0,
                    MAX_GAP_M * 1000.0,
                    top.at,
                    top.pose
                ));
            }
        }
        out
    }

    /// The regression test of the bug of 2026-11-19 ("the legs of the
    /// rider are not connected to the body"): for every shape on every
    /// base body (and the 70+), every body height (the pose system's
    /// three scales) and every pose (riding, standing, the tuck, the
    /// sprint, the idle stance at the mirror, the photographer), both
    /// levels of detail: the hip joints lie inside the bib's seat and
    /// each leg's top ring meets the body.
    #[test]
    fn the_legs_meet_the_body_in_every_pose() {
        let mut f = Vec::new();
        for detail in [Detail::High, Detail::Low] {
            f.extend(failures(&rider::build(detail), detail));
        }
        assert!(f.is_empty(), "{}", f.join("\n"));
    }

    /// The check finds the bug (it would have caught it): the old leg, an
    /// open tube from 6 cm under the joint, swings out of the seat on the
    /// bent hip; and on a thin body the old hips (as wide as a normal
    /// one's) put the thighs' tops out through the jersey's sides.
    #[test]
    fn the_check_finds_the_open_thigh_and_the_wide_hips() {
        use crate::mesh::Parts;
        let mut m = rider::build(Detail::High);
        let shorts = m.material("shorts").unwrap();
        for s in 0..2 {
            let thigh = m.node(nodes::THIGH[s]).unwrap();
            m.nodes[thigh].primitives.retain(|p| p.material != shorts);
            let hip = m.world(thigh).transform_point3(Vec3::ZERO);
            let mut p = Parts::new();
            p.slot("shorts").sweep(
                &[
                    hip - Vec3::Y * 0.06,
                    hip - Vec3::Y * 0.14,
                    hip - Vec3::Y * 0.3,
                ],
                &[0.069, 0.07, 0.06],
                8,
                false,
            );
            p.attach(&mut m, thigh);
        }
        let body = Character::default();
        let (_, top) = worst(&m, Detail::High, &body);
        assert!(top.gap > MAX_GAP_M, "{top:?}");
        // The old hips (the rest translation whatever the girth): on the
        // thinnest body the thighs' tops stand out through the jersey's
        // sides standing; drawn as wide as the seat they stay inside.
        use crate::clothing::{MIN_CLEARANCE_M, Reach, bib_under_jersey_at};
        let thin = Character {
            shape: ocw_core::BodyShape::ALL[0],
            ..Character::default()
        };
        let m = rider::build(Detail::High);
        let old: Vec<Affine3A> = (0..m.nodes.len()).map(|i| m.world(i)).collect();
        let w = bib_under_jersey_at(&m, Detail::High, &thin, Reach::WithLegs, &old, "old hips")
            .unwrap();
        assert!(w.clearance < 0.0, "{w:?}");
        let now = posed_worlds(&m, None, &thin);
        let w =
            bib_under_jersey_at(&m, Detail::High, &thin, Reach::WithLegs, &now, "rest").unwrap();
        assert!(w.clearance >= MIN_CLEARANCE_M, "{w:?}");
    }

    /// The hips follow the seat's width: the joints scale with the
    /// pelvis's girth (and the rig with them).
    #[test]
    fn the_hips_are_as_wide_as_the_seat() {
        let m = rider::build(Detail::High);
        let rig = RiderRig::from_model(&m).unwrap();
        for b in bodies() {
            let g = rider::girth_of(nodes::PELVIS, &b);
            for (s, name) in nodes::THIGH.iter().enumerate() {
                let i = m.node(name).unwrap();
                let x = rider::node_translation(&m, i, &b).x;
                assert!((x - m.nodes[i].translation.x * g.x).abs() < 1e-6);
                assert!((rig.for_body(&b).hips[s].x - x).abs() < 1e-6);
            }
        }
    }
}
