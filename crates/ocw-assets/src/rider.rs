//! The generated low-poly rider (spec §2.6, §8.3): rigid body parts on a
//! node hierarchy (pelvis → thighs → shins → feet; pelvis → torso → head,
//! upper arms → forearms → hands), in the rest pose (standing, arms
//! hanging, facing −Z). Paint slots: jersey, accent (kit pattern), shorts,
//! skin, sock, shoe, helmet, helmet_detail, lens, hair.

use glam::{Quat, Vec3};

use crate::mesh::{Cap, Parts, Ring};
use crate::model::{Model, Node};
use crate::pose::nodes::*;

/// Mesh resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Close up (LOD 0).
    High,
    /// Far away (LOD 1, merged into the bike's `lod1` node).
    Low,
}

impl Detail {
    /// Sides of a limb.
    pub fn sides(self) -> u32 {
        match self {
            Detail::High => 8,
            Detail::Low => 4,
        }
    }
}

/// Body measures (m) of the generated rider: about 1.78 m tall.
pub mod body {
    pub const HIP_HALF_WIDTH: f32 = 0.09;
    pub const THIGH: f32 = 0.43;
    pub const SHIN: f32 = 0.43;
    /// Ankle → cleat (under the ball of the foot, on the sole).
    pub const CLEAT: [f32; 3] = [0.0, -0.085, -0.1];
    /// Pelvis → lower back pivot.
    pub const TORSO_PIVOT: [f32; 3] = [0.0, 0.04, 0.03];
    /// Lower back → base of the neck.
    pub const TORSO: f32 = 0.5;
    pub const SHOULDER_HALF_WIDTH: f32 = 0.18;
    pub const SHOULDER_HEIGHT: f32 = 0.45;
    pub const UPPER_ARM: f32 = 0.29;
    pub const FOREARM: f32 = 0.26;
    /// Wrist → middle of the grip.
    pub const HAND: f32 = 0.07;
}

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::from(a)
}

/// Builds the rider model.
pub fn build(detail: Detail) -> Model {
    use body::{HIP_HALF_WIDTH, SHOULDER_HALF_WIDTH, SHOULDER_HEIGHT, TORSO_PIVOT};
    let mut m = Model::default();
    let n = detail.sides();
    let hi = detail == Detail::High;
    let root = m.add(Node::new("rider", None, Vec3::ZERO));
    let pelvis = m.add(Node::new(PELVIS, Some(root), Vec3::ZERO));
    // Hips and seat: shorts.
    let mut p = Parts::new();
    let ring = |y: f32, hw: f32, hd: f32, z: f32| Ring {
        centre: Vec3::new(0.0, y, z),
        u: Vec3::X * hw,
        v: Vec3::NEG_Z * hd,
    };
    p.slot("shorts").loft(
        &[
            ring(-0.07, 0.15, 0.1, 0.01),
            ring(0.0, 0.165, 0.11, 0.01),
            ring(0.07, 0.155, 0.105, 0.015),
        ],
        n + 2,
        false,
        Cap::Point(Vec3::new(0.0, -0.11, 0.0)),
        Cap::Flat,
    );
    p.attach(&mut m, pelvis);
    // Legs.
    for s in 0..2 {
        let x = if s == 0 {
            -HIP_HALF_WIDTH
        } else {
            HIP_HALF_WIDTH
        };
        let hip = Vec3::new(x, 0.0, 0.0);
        let thigh = m.add(Node::new(THIGH[s], Some(pelvis), hip));
        let mut p = Parts::new();
        let knee = hip - Vec3::Y * body::THIGH;
        let shorts_end = hip - Vec3::Y * 0.3;
        p.slot("shorts").sweep(
            &[hip + Vec3::Y * 0.03, hip - Vec3::Y * 0.12, shorts_end],
            &[0.075, 0.072, 0.06],
            n,
            false,
        );
        p.slot("skin")
            .tube(shorts_end, knee, 0.057, 0.048, n, false);
        p.attach(&mut m, thigh);
        let shin = m.add(Node::new(SHIN[s], Some(thigh), Vec3::NEG_Y * body::THIGH));
        let mut p = Parts::new();
        let ankle = knee - Vec3::Y * body::SHIN;
        let sock = ankle + Vec3::Y * 0.11;
        if hi {
            p.slot("skin")
                .ellipsoid(knee, Vec3::splat(0.05), Quat::IDENTITY, 4, n);
        }
        p.slot("skin").sweep(
            &[knee, knee - Vec3::Y * 0.12 + Vec3::Z * 0.012, sock],
            &[0.048, 0.054, 0.036],
            n,
            false,
        );
        p.slot("sock").tube(sock, ankle, 0.036, 0.033, n, false);
        p.attach(&mut m, shin);
        let foot = m.add(Node::new(FOOT[s], Some(shin), Vec3::NEG_Y * body::SHIN));
        let mut p = Parts::new();
        if hi {
            p.slot("sock")
                .ellipsoid(ankle, Vec3::splat(0.035), Quat::IDENTITY, 4, n);
        }
        // Shoe: heel to toe along -Z, the sole at the cleat's height.
        let sole = ankle.y + body::CLEAT[1];
        let shoe = |z: f32, hw: f32, top: f32| Ring {
            centre: Vec3::new(x, 0.5 * (sole + top), ankle.z + z),
            u: Vec3::X * hw,
            v: Vec3::Y * (0.5 * (top - sole)),
        };
        p.slot("shoe").loft(
            &[
                shoe(0.055, 0.036, ankle.y + 0.0),
                shoe(-0.02, 0.045, ankle.y - 0.005),
                shoe(-0.12, 0.045, ankle.y - 0.04),
                shoe(-0.19, 0.035, ankle.y - 0.06),
            ],
            n,
            false,
            Cap::Flat,
            Cap::Point(Vec3::new(x, sole + 0.012, ankle.z - 0.215)),
        );
        p.attach(&mut m, foot);
        m.add(Node::new(CLEAT[s], Some(foot), v(body::CLEAT)));
    }
    // Torso: jersey, from the waist to the shoulders.
    let pivot = v(TORSO_PIVOT);
    let torso = m.add(Node::new(TORSO, Some(pelvis), pivot));
    let mut p = Parts::new();
    let t = |y: f32, hw: f32, hd: f32, z: f32| ring(pivot.y + y, hw, hd, pivot.z + z);
    let neck = pivot + Vec3::Y * body::TORSO;
    p.slot("jersey").loft(
        &[
            t(-0.06, 0.152, 0.102, -0.01),
            t(0.12, 0.158, 0.1, -0.005),
            t(0.3, 0.18, 0.115, 0.005),
            t(0.42, 0.19, 0.105, 0.01),
        ],
        n + 4,
        false,
        Cap::Open,
        Cap::Point(neck - Vec3::Y * 0.01),
    );
    if hi {
        for s in [-1.0f32, 1.0] {
            p.slot("jersey").ellipsoid(
                pivot
                    + Vec3::new(
                        s * (SHOULDER_HALF_WIDTH - 0.01),
                        SHOULDER_HEIGHT - 0.01,
                        0.0,
                    ),
                Vec3::new(0.06, 0.055, 0.06),
                Quat::IDENTITY,
                4,
                n,
            );
        }
        // A back pocket row: an accent-free darker band is left out; the
        // jersey's zip in the accent colour runs down the chest.
        p.slot("accent").tube(
            pivot + Vec3::new(0.0, 0.43, -0.103),
            pivot + Vec3::new(0.0, 0.05, -0.108),
            0.006,
            0.006,
            4,
            false,
        );
    }
    p.attach(&mut m, torso);
    // Kit patterns: a chest band and a diagonal sash (one of them shown).
    let band = m.add(Node::new(KIT_BAND, Some(torso), Vec3::ZERO));
    let mut p = Parts::new();
    p.slot("accent").loft(
        &[t(0.2, 0.17, 0.11, 0.0), t(0.3, 0.183, 0.119, 0.005)],
        n + 4,
        false,
        Cap::Open,
        Cap::Open,
    );
    p.attach(&mut m, band);
    let sash = m.add(Node::new(KIT_SASH, Some(torso), Vec3::ZERO));
    let mut p = Parts::new();
    let tilt = Quat::from_rotation_z(0.62);
    let c = pivot + Vec3::new(0.0, 0.25, 0.0);
    let sring = |dy: f32, k: f32| Ring {
        centre: c + tilt * Vec3::new(0.0, dy, 0.0),
        u: tilt * Vec3::X * 0.215 * k,
        v: Vec3::NEG_Z * 0.122 * k,
    };
    p.slot("accent").loft(
        &[sring(-0.04, 1.0), sring(0.04, 1.0)],
        n + 4,
        false,
        Cap::Open,
        Cap::Open,
    );
    p.attach(&mut m, sash);
    // Head: neck, face, glasses, hair.
    let head = m.add(Node::new(HEAD, Some(torso), Vec3::Y * body::TORSO));
    let mut p = Parts::new();
    p.slot("skin").tube(
        neck - Vec3::Y * 0.05,
        neck + Vec3::new(0.0, 0.09, -0.01),
        0.05,
        0.047,
        n,
        false,
    );
    let skull = neck + Vec3::new(0.0, 0.15, -0.015);
    p.slot("skin").ellipsoid(
        skull,
        Vec3::new(0.078, 0.105, 0.095),
        Quat::IDENTITY,
        if hi { 7 } else { 4 },
        n,
    );
    p.slot("hair").ellipsoid(
        skull + Vec3::new(0.0, 0.0, 0.02),
        Vec3::new(0.081, 0.1, 0.09),
        Quat::from_rotation_x(-0.2),
        if hi { 6 } else { 3 },
        n,
    );
    if hi {
        p.slot("skin").ellipsoid(
            skull + Vec3::new(0.0, -0.01, -0.098),
            Vec3::new(0.014, 0.022, 0.016),
            Quat::from_rotation_x(0.3),
            3,
            5,
        );
        p.slot("lens").ellipsoid(
            skull + Vec3::new(0.0, 0.012, -0.075),
            Vec3::new(0.083, 0.023, 0.035),
            Quat::IDENTITY,
            3,
            10,
        );
    }
    p.attach(&mut m, head);
    // Head wear: road helmet, aero helmet, cotton cap.
    let dome = |rx: f32, ry: f32, rz: f32, c: Vec3, bands: u32| -> Vec<Ring> {
        (0..bands)
            .map(|j| {
                let a = std::f32::consts::FRAC_PI_2 * j as f32 / bands as f32;
                Ring {
                    centre: c + Vec3::Y * (ry * a.sin()),
                    u: Vec3::X * (rx * a.cos()),
                    v: Vec3::NEG_Z * (rz * a.cos()),
                }
            })
            .collect()
    };
    let helmet = m.add(Node::new(HELMET_ROAD, Some(head), Vec3::ZERO));
    let mut p = Parts::new();
    let hc = skull + Vec3::new(0.0, 0.0, 0.01);
    p.slot("helmet").loft(
        &dome(0.1, 0.13, 0.135, hc, if hi { 5 } else { 3 }),
        n + 4,
        false,
        Cap::Flat,
        Cap::Point(hc + Vec3::Y * 0.135),
    );
    if hi {
        // Vents: dark grooves front to back.
        for x in [-0.045f32, 0.0, 0.045] {
            p.slot("helmet_detail").sweep(
                &[
                    hc + Vec3::new(x, 0.07, -0.105),
                    hc + Vec3::new(x, 0.12, -0.03),
                    hc + Vec3::new(x, 0.115, 0.06),
                ],
                &[0.012, 0.014, 0.012],
                4,
                true,
            );
        }
        for s in [-1.0f32, 1.0] {
            p.slot("helmet_detail").tube(
                hc + Vec3::new(s * 0.085, 0.0, -0.01),
                skull + Vec3::new(s * 0.05, -0.1, -0.03),
                0.004,
                0.004,
                3,
                false,
            );
        }
    }
    p.attach(&mut m, helmet);
    let aero = m.add(Node::new(HELMET_AERO, Some(head), Vec3::ZERO));
    let mut p = Parts::new();
    let ac = skull + Vec3::new(0.0, 0.03, 0.0);
    let a = |z: f32, hw: f32, hh: f32, dy: f32| Ring {
        centre: ac + Vec3::new(0.0, dy, z),
        u: Vec3::X * hw,
        v: Vec3::Y * hh,
    };
    p.slot("helmet").loft(
        &[
            a(-0.09, 0.07, 0.07, 0.0),
            a(-0.03, 0.1, 0.11, 0.01),
            a(0.07, 0.095, 0.1, 0.0),
            a(0.2, 0.05, 0.05, -0.03),
        ],
        n + 4,
        false,
        Cap::Point(ac + Vec3::new(0.0, 0.0, -0.12)),
        Cap::Point(ac + Vec3::new(0.0, -0.06, 0.3)),
    );
    if hi {
        p.slot("lens").ellipsoid(
            skull + Vec3::new(0.0, 0.02, -0.085),
            Vec3::new(0.088, 0.035, 0.035),
            Quat::IDENTITY,
            3,
            10,
        );
    }
    p.attach(&mut m, aero);
    let cap = m.add(Node::new(CAP, Some(head), Vec3::ZERO));
    let mut p = Parts::new();
    let cc = skull + Vec3::new(0.0, 0.035, 0.0);
    p.slot("helmet").loft(
        &dome(0.087, 0.07, 0.1, cc, if hi { 4 } else { 2 }),
        n + 4,
        false,
        Cap::Open,
        Cap::Point(cc + Vec3::Y * 0.072),
    );
    p.slot("jersey").loft(
        &[
            Ring {
                centre: cc - Vec3::Y * 0.012,
                u: Vec3::X * 0.088,
                v: Vec3::NEG_Z * 0.101,
            },
            Ring {
                centre: cc + Vec3::Y * 0.02,
                u: Vec3::X * 0.086,
                v: Vec3::NEG_Z * 0.099,
            },
        ],
        n + 4,
        false,
        Cap::Open,
        Cap::Open,
    );
    // Short brim, flipped up.
    p.slot("jersey").cuboid(
        cc + Vec3::new(0.0, 0.025, -0.125),
        Vec3::new(0.065, 0.005, 0.035),
        Quat::from_rotation_x(0.55),
    );
    p.attach(&mut m, cap);
    // Arms.
    for s in 0..2 {
        let sx = if s == 0 {
            -SHOULDER_HALF_WIDTH
        } else {
            SHOULDER_HALF_WIDTH
        };
        let shoulder = pivot + Vec3::new(sx, SHOULDER_HEIGHT, 0.0);
        let upper = m.add(Node::new(
            UPPER_ARM[s],
            Some(torso),
            Vec3::new(sx, SHOULDER_HEIGHT, 0.0),
        ));
        let mut p = Parts::new();
        let sleeve = shoulder - Vec3::Y * 0.16;
        let elbow = shoulder - Vec3::Y * body::UPPER_ARM;
        p.slot("jersey")
            .tube(shoulder + Vec3::Y * 0.02, sleeve, 0.052, 0.047, n, false);
        p.slot("skin").tube(sleeve, elbow, 0.042, 0.037, n, false);
        p.attach(&mut m, upper);
        let fore = m.add(Node::new(
            FOREARM[s],
            Some(upper),
            Vec3::NEG_Y * body::UPPER_ARM,
        ));
        let mut p = Parts::new();
        let wrist = elbow - Vec3::Y * body::FOREARM;
        if hi {
            p.slot("skin")
                .ellipsoid(elbow, Vec3::splat(0.038), Quat::IDENTITY, 4, n);
        }
        p.slot("skin").tube(elbow, wrist, 0.037, 0.026, n, false);
        p.attach(&mut m, fore);
        let hand = m.add(Node::new(HAND[s], Some(fore), Vec3::NEG_Y * body::FOREARM));
        let mut p = Parts::new();
        // Glove round the bar: fingers forward of the grip point.
        p.slot("shorts").ellipsoid(
            wrist - Vec3::Y * 0.06 + Vec3::Z * -0.005,
            Vec3::new(0.03, 0.055, 0.03),
            Quat::IDENTITY,
            4,
            n,
        );
        p.attach(&mut m, hand);
        m.add(Node::new(GRIP[s], Some(hand), Vec3::NEG_Y * body::HAND));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::RiderRig;

    #[test]
    fn the_rider_has_its_rig_and_stays_low_poly() {
        let m = build(Detail::High);
        for name in REQUIRED {
            assert!(m.node(name).is_some(), "{name}");
        }
        let rig = RiderRig::from_model(&m).unwrap();
        assert!((rig.leg() - 0.86).abs() < 1e-5);
        assert!((rig.forearm - 0.33).abs() < 1e-5);
        let tris = m.triangles();
        assert!((1500..6000).contains(&tris), "{tris} triangles");
        let low = build(Detail::Low).triangles();
        assert!(low * 2 < tris, "far LOD {low} vs {tris}");
        // Standing upright, about 1.78 m tall from the soles (pelvis at the hips).
        let sole = -(body::THIGH + body::SHIN) + body::CLEAT[1];
        let top = m
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| !m.is_below(*i, m.node(HELMET_AERO).unwrap()))
            .flat_map(|(i, n)| {
                let w = m.world(i);
                n.primitives.iter().flat_map(move |p| {
                    p.positions
                        .iter()
                        .map(move |q| w.transform_point3(Vec3::from(*q)).y)
                })
            })
            .fold(f32::MIN, f32::max);
        let height = top - sole;
        assert!((1.7..1.9).contains(&height), "{height}");
    }
}
