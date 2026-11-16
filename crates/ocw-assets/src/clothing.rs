//! The layering of the rider's clothes (spec §2.6, §14 Phase 4 4C, Phase 8
//! T6; bug 2026-11-16 "the bib is visible through the jersey"): the
//! rider's body parts are rigid nodes, so a garment on one node (the bib
//! shorts on the pelvis and thighs) and one on another (the jersey on the
//! torso) move against each other in every pose. This module measures
//! that: the bib the jersey covers at rest (above the hem and in a band
//! just under it) stays inside the jersey's shell by [`MIN_CLEARANCE_M`]
//! wherever it comes above the hem, and inside the hem's outline just
//! under it – the bib's torso part (on the pelvis) in every pose of the
//! pose system ([`poses`]), the legs' tops standing ([`Reach`]) – for
//! every body build; the back pockets lie on the shell, outside it,
//! above the hem.
//!
//! The jersey's shell is the loft of [`crate::rider::JERSEY_SHELL`] with
//! the far or near model's number of sides ([`crate::rider::jersey_sides`]):
//! the check uses the same faceted cross-sections the meshes have, so a
//! six-sided bib inside an eight-sided jersey is measured as drawn.

use glam::{Affine3A, Vec2, Vec3};
use ocw_core::{BodyBuild, ProfileId};

use crate::anim::{self, Riding};
use crate::model::Model;
use crate::pose::{self, BodyFit, Holding, Pose, PoseParams, RiderRig, nodes};
use crate::rider::{self, Detail};

/// The bib's smallest distance inside the jersey's shell (m): far more
/// than the depth buffer resolves at a close orbit, so the two never
/// fight, and enough for the shading not to show the bib's facets.
pub const MIN_CLEARANCE_M: f32 = 0.004;

/// The band under the hem (m) where the jersey still hangs over the bib:
/// the bib there stays inside the hem's outline, else the hem would seem
/// to go into the shorts.
pub const HEM_BAND_M: f32 = 0.015;

/// The pockets' panel stands at least this far outside the shell (m).
pub const POCKET_OFFSET_M: f32 = 0.002;

/// The jersey's shell at height `y` (rest-pose model coordinates, in
/// the torso's frame): the polygon's corners in (x, z) – below the hem
/// the shell's sides continued straight down – or `None` above the
/// shoulders.
pub fn shell_section(detail: Detail, y: f32) -> Option<Vec<Vec2>> {
    let n = rider::jersey_sides(detail);
    (0..n)
        .map(|k| rider::shell_corner(n, k, y, 0.0).map(|q| Vec2::new(q.x, q.z)))
        .collect()
}

/// The signed distance of `p` inside the convex polygon `poly` (m;
/// negative: outside).
pub fn inside_distance(poly: &[Vec2], p: Vec2) -> f32 {
    let c = poly.iter().copied().sum::<Vec2>() / poly.len() as f32;
    let mut d = f32::MAX;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        let e = (b - a).normalize_or_zero();
        let mut n = Vec2::new(-e.y, e.x);
        if n.dot(c - a) < 0.0 {
            n = -n;
        }
        d = d.min(n.dot(p - a));
    }
    d
}

/// One pose to check, with its name for messages.
pub struct NamedPose {
    pub name: String,
    pub pose: Pose,
}

/// Every pose the game draws the rider in (spec §2.6, v3.47): each
/// riding state's targets on every bike at three body heights and eight
/// crank angles, each with the body motions (cornering both ways, the
/// head turned both ways, arms up, sitting up, the bike throw, a foot
/// down at a stop, a fast descent's tuck), the changing corner's idle
/// stance (breathing, weight shift) and the photographer's stances.
pub fn poses(rider_rig: &RiderRig) -> Vec<NamedPose> {
    let mut out = Vec::new();
    let states = [
        (Riding::Stopped, 0.0, 0.0),
        (Riding::Seated, 9.0, 0.0),
        (Riding::Climbing, 4.0, 0.08),
        (Riding::Standing, 3.5, 0.11),
        (Riding::Sprinting, 15.0, 0.0),
        (Riding::Coasting, 16.0, -0.06),
    ];
    type Motion = (&'static str, fn(&mut PoseParams));
    let motions: [Motion; 11] = [
        ("plain", |_| {}),
        ("corner right", |p| p.corner = 1.0),
        ("corner left", |p| p.corner = -1.0),
        ("look right", |p| p.look = anim::LOOK_BACK as f32),
        ("look left", |p| p.look = -anim::LOOK_BACK as f32),
        ("arms up", |p| p.arms_up = 1.0),
        ("sit up", |p| {
            p.sit_up = 1.0;
            p.tops = 1.0;
        }),
        ("throw", |p| p.throw = 1.0),
        ("foot down", |p| {
            p.foot = 1.0;
            p.straddle = 1.0;
        }),
        ("foot out", |p| p.foot = 0.5),
        ("tuck", |p| {
            p.drops = 1.0;
            p.tuck = 1.0;
        }),
    ];
    for profile in ProfileId::ALL {
        let bike = crate::shipped::bike_rig(profile);
        let aero = bike.aero_bars;
        for scale in [0.85, 1.0, 1.15] {
            let fit = BodyFit::new(rider_rig, &bike, scale);
            // Other body heights sit differently on the bike (the lean):
            // their plain riding; the motions at the model's own height.
            let motions = if scale == 1.0 {
                &motions[..]
            } else {
                &motions[..1]
            };
            for (state, speed, grade) in states {
                let target = anim::targets(state, speed, grade, aero);
                for k in 0..8 {
                    for &(motion, apply) in motions {
                        let mut p = target;
                        p.crank = k as f32 / 8.0 * std::f32::consts::TAU;
                        apply(&mut p);
                        out.push(NamedPose {
                            name: format!(
                                "{} {state:?} {motion} crank {k}/8 scale {scale}",
                                profile.id()
                            ),
                            pose: pose::pose(&fit.rider, &fit.bike, &fit.fit, &p),
                        });
                    }
                }
            }
        }
    }
    for i in 0..120 {
        let t = i as f32 * 0.25;
        out.push(NamedPose {
            name: format!("idle t {t}"),
            pose: pose::idle(rider_rig, t),
        });
    }
    for crouch in [false, true] {
        for holding in [Holding::Camera, Holding::Remote] {
            for pitch in [-0.9, 0.0, 1.1] {
                out.push(NamedPose {
                    name: format!("photographer crouch {crouch} {holding:?} pitch {pitch}"),
                    pose: pose::photographer(rider_rig, crouch, pitch, holding, 1.0).0,
                });
            }
        }
    }
    out
}

/// The worst place of one check: its clearance (m; negative = through
/// the other layer), the node and the pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Worst {
    pub clearance: f32,
    pub node: String,
    pub pose: String,
    pub at: Vec3,
    /// The same point at rest.
    pub rest: Vec3,
}

/// The rig node carrying node `i` (the nearest one at or above it).
fn rig_node(m: &Model, i: usize) -> Option<&str> {
    let mut at = Some(i);
    while let Some(n) = at {
        let name = m.nodes[n].name.as_str();
        if nodes::REQUIRED.contains(&name) {
            return Some(name);
        }
        at = m.nodes[n].parent;
    }
    None
}

/// Which bib a check covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The bib's torso part, on the pelvis: under the jersey in every
    /// pose.
    Torso,
    /// The legs too (on the thighs): under the jersey at rest. Folded up
    /// on the pedals, the thighs' tops lie against the belly in front of
    /// the jersey (the hip crease), as real thighs do.
    WithLegs,
}

/// The nodes whose `slot` meshes are checked: the base model and every
/// optional node of the kit (a look shows some of them), without the
/// hip plates (paper on the outside of the shorts, below the hem).
fn layer_nodes(m: &Model, slot: &str, reach: Reach) -> Vec<(usize, usize)> {
    let Some(mat) = m.material(slot) else {
        return Vec::new();
    };
    let skip: Vec<usize> = rider::opt::HIP_NUMBER
        .iter()
        .filter_map(|n| m.node(n))
        .collect();
    let mut out = Vec::new();
    for (i, node) in m.nodes.iter().enumerate() {
        if skip.contains(&i) || (reach == Reach::Torso && rig_node(m, i) != Some(nodes::PELVIS)) {
            continue;
        }
        for (k, p) in node.primitives.iter().enumerate() {
            if p.material == mat {
                out.push((i, k));
            }
        }
    }
    out
}

/// The posed model's points of `parts`, in the torso's rest frame with
/// the torso's girth taken out (so they compare with the unscaled shell).
fn in_torso_frame(
    rest: &Model,
    worlds: &[Affine3A],
    torso: usize,
    build: BodyBuild,
    parts: &[(usize, usize)],
) -> Vec<(usize, Vec3)> {
    let g = build.girth();
    let to_torso = worlds[torso].inverse();
    let torso_rest = rest.world(torso);
    let mut out = Vec::new();
    for &(i, k) in parts {
        let w = worlds[i];
        let s = rider::girth(rest, i, build);
        for v in &rest.nodes[i].primitives[k].positions {
            let world = w.transform_point3(Vec3::from(*v) * s);
            let local = to_torso.transform_point3(world);
            let local = Vec3::new(local.x / g, local.y, local.z / g);
            out.push((i, torso_rest.transform_point3(local)));
        }
    }
    out
}

/// How far the bib stays inside the jersey (the smallest clearance of
/// any bib point above the jersey's hem) in `pose` for `build`; `None`
/// when no bib point comes above the hem.
pub fn bib_under_jersey(
    m: &Model,
    detail: Detail,
    build: BodyBuild,
    reach: Reach,
    pose: &NamedPose,
) -> Option<Worst> {
    bib_under_jersey_at(
        m,
        detail,
        build,
        reach,
        &posed_worlds(m, &pose.pose),
        &pose.name,
    )
}

/// The model-space transforms of every node of `m` in `pose`.
pub fn posed_worlds(m: &Model, pose: &Pose) -> Vec<Affine3A> {
    let mut skeleton = Model {
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
    pose.apply(&mut skeleton);
    (0..skeleton.nodes.len()).map(|i| skeleton.world(i)).collect()
}

/// [`bib_under_jersey`] with the pose's node transforms ([`posed_worlds`]).
pub fn bib_under_jersey_at(
    m: &Model,
    detail: Detail,
    build: BodyBuild,
    reach: Reach,
    worlds: &[Affine3A],
    pose: &str,
) -> Option<Worst> {
    let torso = m.node(nodes::TORSO)?;
    let bib = layer_nodes(m, "shorts", reach);
    let rest_worlds: Vec<Affine3A> = (0..m.nodes.len()).map(|i| m.world(i)).collect();
    let rest = in_torso_frame(m, &rest_worlds, torso, build, &bib);
    let posed = in_torso_frame(m, worlds, torso, build, &bib);
    let mut worst: Option<Worst> = None;
    for ((_, r), (i, p)) in rest.into_iter().zip(posed) {
        // The bib the jersey covers at rest (and the band just under the
        // hem): the rest of the legs is not the jersey's business.
        if rider::above_hem(r) < -HEM_BAND_M {
            continue;
        }
        let above = rider::above_hem(p);
        if above < -HEM_BAND_M {
            continue;
        }
        let Some(poly) = shell_section(detail, p.y) else {
            continue;
        };
        let d = inside_distance(&poly, Vec2::new(p.x, p.z));
        // Just under the hem the jersey hangs over the bib: inside the
        // hem's outline continued down (no clearance needed: nothing to
        // fight with).
        let d = if above >= 0.0 { d } else { d + MIN_CLEARANCE_M };
        if worst.as_ref().is_none_or(|w| d < w.clearance) {
            worst = Some(Worst {
                clearance: d,
                node: m.nodes[i].name.clone(),
                pose: pose.to_string(),
                at: p,
                rest: r,
            });
        }
    }
    worst
}

/// The pockets' panel against the jersey (rest frame of the torso, on
/// which both are drawn): the smallest distance outside the shell
/// (negative: inside) and the lowest point's height above the hem.
pub fn pockets_on_jersey(m: &Model, detail: Detail) -> Option<(f32, f32)> {
    let mut outside = f32::MAX;
    let mut low = f32::MAX;
    for name in [rider::POCKETS, rider::POCKET_BAND] {
        let i = m.node(name)?;
        let w = m.world(i);
        for p in &m.nodes[i].primitives {
            for v in &p.positions {
                let q = w.transform_point3(Vec3::from(*v));
                low = low.min(rider::above_hem(q));
                if let Some(poly) = shell_section(detail, q.y) {
                    outside = outside.min(-inside_distance(&poly, Vec2::new(q.x, q.z)));
                }
            }
        }
    }
    Some((outside, low))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worst of `check` over both levels of detail and every body
    /// build, as failure messages (empty: all clear by `min`).
    fn failures(
        check: impl Fn(&Model, Detail, BodyBuild) -> Option<Worst>,
        min: f32,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for detail in [Detail::High, Detail::Low] {
            let m = rider::build(detail);
            for build in BodyBuild::ALL {
                let w = check(&m, detail, build).expect("the bib comes up under the jersey");
                if w.clearance < min {
                    out.push(format!(
                        "{detail:?} {build:?}: the bib is {:.1} mm from the jersey's surface \
                         (min {:.1} mm) at {:?} (at rest {:?}) of {} in pose {}",
                        w.clearance * 1000.0,
                        min * 1000.0,
                        w.at,
                        w.rest,
                        w.node,
                        w.pose
                    ));
                }
            }
        }
        out
    }

    fn worse(a: Option<Worst>, b: Option<Worst>) -> Option<Worst> {
        match (a, b) {
            (Some(a), Some(b)) => Some(if b.clearance < a.clearance { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// The regression test of the bug of 2026-11-16 (the bib shorts showed
    /// through the jersey's lower back and sides): in every pose of the
    /// pose system (every riding state on every bike, three body heights,
    /// the crank all round, cornering, looking back, arms up, sitting up,
    /// the bike throw, a foot down, the tuck, the idle stance, the
    /// photographer), for every body build and both levels of detail, the
    /// bib's torso part stays inside the jersey's shell by at least
    /// [`MIN_CLEARANCE_M`] wherever it comes above the hem, and inside the
    /// hem's outline just under it.
    #[test]
    fn the_bib_stays_under_the_jersey_in_every_pose() {
        let f = failures(
            |m, detail, build| {
                let rig = RiderRig::from_model(m).unwrap();
                let mut worst = None;
                for p in poses(&rig) {
                    let w = bib_under_jersey(m, detail, build, Reach::Torso, &p);
                    worst = worse(worst, w);
                }
                worst
            },
            MIN_CLEARANCE_M,
        );
        assert!(f.is_empty(), "{}", f.join("\n"));
    }

    /// Standing (the rest pose, the changing corner's idle stance at the
    /// mirror): the whole bib, the legs' tops included, is under the
    /// jersey (the old legs began above the hip joint and came out
    /// through the jersey's sides).
    #[test]
    fn the_bib_is_under_the_jersey_standing() {
        let f = failures(
            |m, detail, build| {
                let rig = RiderRig::from_model(m).unwrap();
                let rest: Vec<Affine3A> = (0..m.nodes.len()).map(|i| m.world(i)).collect();
                let mut worst = bib_under_jersey_at(m, detail, build, Reach::WithLegs, &rest, "rest");
                for i in 0..120 {
                    let p = NamedPose {
                        name: format!("idle t {}", i as f32 * 0.25),
                        pose: pose::idle(&rig, i as f32 * 0.25),
                    };
                    worst = worse(worst, bib_under_jersey(m, detail, build, Reach::WithLegs, &p));
                }
                worst
            },
            MIN_CLEARANCE_M,
        );
        assert!(f.is_empty(), "{}", f.join("\n"));
    }

    /// The check finds the bug of 2026-11-16 (the test would have caught
    /// it): the old bib waist (as wide as the jersey there, 2 cm deeper
    /// at the front) in today's jersey.
    #[test]
    fn the_check_finds_a_bib_through_the_jersey() {
        use crate::mesh::{Cap, Parts, Ring};
        let mut m = rider::build(Detail::High);
        let pelvis = m.node(nodes::PELVIS).unwrap();
        let shorts = m.material("shorts").unwrap();
        m.nodes[pelvis].primitives.retain(|p| p.material != shorts);
        let ring = |y: f32, hw: f32, hd: f32, z: f32| Ring {
            centre: Vec3::new(0.0, y, z),
            u: Vec3::X * hw,
            v: Vec3::NEG_Z * hd,
        };
        let mut p = Parts::new();
        p.slot("shorts").loft(
            &[
                ring(-0.07, 0.15, 0.1, 0.01),
                ring(0.0, 0.165, 0.11, 0.01),
                ring(0.07, 0.155, 0.105, 0.015),
            ],
            10,
            false,
            Cap::Point(Vec3::new(0.0, -0.11, 0.0)),
            Cap::Flat,
        );
        p.attach(&mut m, pelvis);
        let rest: Vec<Affine3A> = (0..m.nodes.len()).map(|i| m.world(i)).collect();
        let w = bib_under_jersey_at(
            &m,
            Detail::High,
            BodyBuild::Medium,
            Reach::Torso,
            &rest,
            "rest",
        )
        .unwrap();
        assert!(w.clearance < 0.0, "{w:?}");
    }

    /// The back pockets lie on the jersey (outside its shell, not inside
    /// it) and above the hem, for both levels of detail.
    #[test]
    fn the_pockets_lie_on_the_jersey_above_the_hem() {
        for detail in [Detail::High, Detail::Low] {
            let m = rider::build(detail);
            let (outside, low) = pockets_on_jersey(&m, detail).expect("pockets");
            assert!(outside >= POCKET_OFFSET_M * 0.99, "{detail:?}: {outside}");
            assert!(low >= 0.0, "{detail:?}: {low}");
        }
    }

    #[test]
    fn inside_distance_of_a_square() {
        let sq = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ];
        assert!((inside_distance(&sq, Vec2::ZERO) - 1.0).abs() < 1e-6);
        assert!((inside_distance(&sq, Vec2::new(1.5, 0.0)) + 0.5).abs() < 1e-6);
    }
}
