//! The generated low-poly bikes (spec §2.5, §8.2): one per physics
//! profile, built from a handful of real geometry numbers (stack, reach,
//! angles, chainstay, wheel size) so the silhouettes differ the way real
//! bikes do. Nodes: `bike` (frame, fork, drivetrain), `wheel_front`,
//! `wheel_rear` (spin about X), `crank` (with `pedal_l`, `pedal_r`),
//! `handlebar` (with the `grip_*` anchors), `saddle` (the sit-bone anchor)
//! and `lod1` (bike and a seated rider in a few meshes, for far away).

use std::f32::consts::{PI, TAU};

use glam::{Quat, Vec3};
use ocw_core::ProfileId;

use crate::mesh::{Cap, Parts, Ring, Shape};
use crate::model::{Model, Node};
use crate::pose::{self, BikeRig, Fit, PoseParams, RiderRig, bike_nodes::*};
use crate::rider;

/// Handlebars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bars {
    /// Road / gravel drop bar: centre-to-centre width, reach, drop, flare
    /// (how much wider the drops are).
    Drop {
        width: f32,
        reach: f32,
        drop: f32,
        flare: f32,
    },
    /// Time trial: base bar with extensions and arm pads.
    Aero,
    /// Mountain bike: wide flat bar with rise.
    Flat { width: f32, rise: f32 },
}

/// A wheel's build.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Wheel {
    Spoked(u32),
    ThreeSpoke,
    Disc,
}

/// The numbers a bike model is built from (m, degrees).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Design {
    /// Outer wheel radius (with the tyre).
    pub wheel_radius: f32,
    /// Tyre width.
    pub tyre: f32,
    /// Tread knobs (0 = slick).
    pub knobs: f32,
    /// Tan side walls.
    pub skinwall: bool,
    pub rim_depth: f32,
    pub rim_width: f32,
    pub front: Wheel,
    pub rear: Wheel,
    pub chainstay: f32,
    pub bb_drop: f32,
    pub seat_angle: f32,
    pub stack: f32,
    pub reach: f32,
    pub head_angle: f32,
    pub head_tube: f32,
    pub rake: f32,
    /// Main tube radius and depth ratio (aero tubes are deeper than wide).
    pub tube: f32,
    pub aero: f32,
    /// Where the top tube and the seat stays meet the seat tube (from the
    /// bottom bracket).
    pub top_tube_at: f32,
    pub stays_at: f32,
    pub stem: f32,
    /// Stem rise from horizontal (degrees).
    pub stem_rise: f32,
    pub spacers: f32,
    pub bars: Bars,
    pub suspension: bool,
    pub disc_brakes: bool,
    pub bottles: u8,
    pub crank: f32,
    /// Chainring radii (big, small).
    pub rings: (f32, Option<f32>),
    /// A small bag on the top tube.
    pub bag: bool,
}

/// The design of a profile's bike.
pub fn design(p: ProfileId) -> Design {
    let road = Design {
        wheel_radius: 0.34,
        tyre: 0.027,
        knobs: 0.0,
        skinwall: false,
        rim_depth: 0.035,
        rim_width: 0.021,
        front: Wheel::Spoked(24),
        rear: Wheel::Spoked(24),
        chainstay: 0.405,
        bb_drop: 0.07,
        seat_angle: 73.5,
        stack: 0.565,
        reach: 0.385,
        head_angle: 73.0,
        head_tube: 0.15,
        rake: 0.045,
        tube: 0.017,
        aero: 1.2,
        top_tube_at: 0.5,
        stays_at: 0.47,
        stem: 0.11,
        stem_rise: 5.0,
        spacers: 0.02,
        bars: Bars::Drop {
            width: 0.42,
            reach: 0.08,
            drop: 0.125,
            flare: 0.0,
        },
        suspension: false,
        disc_brakes: false,
        bottles: 2,
        crank: 0.1725,
        rings: (0.105, Some(0.082)),
        bag: false,
    };
    match p {
        ProfileId::Road => road,
        ProfileId::RoadAero => Design {
            rim_depth: 0.06,
            front: Wheel::Spoked(20),
            tube: 0.016,
            aero: 2.1,
            stays_at: 0.36,
            stack: 0.55,
            stem_rise: -4.0,
            spacers: 0.005,
            bottles: 1,
            ..road
        },
        ProfileId::RoadClimbing => Design {
            tyre: 0.025,
            rim_depth: 0.024,
            front: Wheel::Spoked(28),
            rear: Wheel::Spoked(28),
            tube: 0.0145,
            aero: 1.0,
            top_tube_at: 0.55,
            stays_at: 0.53,
            stack: 0.57,
            bars: Bars::Drop {
                width: 0.4,
                reach: 0.08,
                drop: 0.125,
                flare: 0.0,
            },
            bottles: 1,
            ..road
        },
        ProfileId::TimeTrial => Design {
            tyre: 0.024,
            rim_depth: 0.08,
            front: Wheel::ThreeSpoke,
            rear: Wheel::Disc,
            seat_angle: 78.0,
            stack: 0.5,
            reach: 0.43,
            head_tube: 0.11,
            tube: 0.017,
            aero: 2.7,
            top_tube_at: 0.5,
            stays_at: 0.34,
            stem: 0.09,
            stem_rise: -8.0,
            spacers: 0.0,
            bars: Bars::Aero,
            bottles: 0,
            rings: (0.11, Some(0.09)),
            ..road
        },
        ProfileId::Gravel => Design {
            wheel_radius: 0.35,
            tyre: 0.04,
            knobs: 0.12,
            skinwall: true,
            rim_depth: 0.025,
            rim_width: 0.025,
            front: Wheel::Spoked(28),
            rear: Wheel::Spoked(28),
            chainstay: 0.425,
            bb_drop: 0.072,
            seat_angle: 73.0,
            stack: 0.59,
            reach: 0.38,
            head_angle: 71.5,
            head_tube: 0.17,
            rake: 0.05,
            tube: 0.018,
            aero: 1.1,
            top_tube_at: 0.47,
            stays_at: 0.44,
            stem: 0.09,
            stem_rise: 8.0,
            spacers: 0.03,
            bars: Bars::Drop {
                width: 0.44,
                reach: 0.075,
                drop: 0.115,
                flare: 0.045,
            },
            disc_brakes: true,
            rings: (0.09, None),
            bag: true,
            ..road
        },
        ProfileId::Mountain => Design {
            wheel_radius: 0.37,
            tyre: 0.058,
            knobs: 0.3,
            rim_depth: 0.024,
            rim_width: 0.03,
            front: Wheel::Spoked(28),
            rear: Wheel::Spoked(28),
            chainstay: 0.435,
            bb_drop: 0.055,
            seat_angle: 74.5,
            stack: 0.62,
            reach: 0.44,
            head_angle: 66.0,
            head_tube: 0.11,
            rake: 0.044,
            tube: 0.021,
            aero: 1.15,
            top_tube_at: 0.4,
            stays_at: 0.38,
            stem: 0.05,
            stem_rise: 5.0,
            spacers: 0.02,
            bars: Bars::Flat {
                width: 0.76,
                rise: 0.02,
            },
            suspension: true,
            disc_brakes: true,
            bottles: 1,
            crank: 0.175,
            rings: (0.075, None),
            ..road
        },
    }
}

/// Key points of a bike in model space (origin on the ground between the
/// wheels, −Z forward).
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub bb: Vec3,
    pub rear_hub: Vec3,
    pub front_hub: Vec3,
    /// Along the seat tube, upwards.
    pub seat_dir: Vec3,
    pub head_top: Vec3,
    pub head_bottom: Vec3,
    /// Down the steering axis.
    pub steer_dir: Vec3,
    pub steerer_top: Vec3,
    pub bar_clamp: Vec3,
    pub saddle: Vec3,
}

impl Frame {
    pub fn new(d: &Design, rider: &RiderRig) -> Self {
        let rad = |deg: f32| deg.to_radians();
        let (sa, ha) = (rad(d.seat_angle), rad(d.head_angle));
        // In the bottom bracket's frame first.
        let seat_dir = Vec3::new(0.0, sa.sin(), sa.cos());
        let rear_hub = Vec3::new(0.0, d.bb_drop, d.chainstay);
        let head_top = Vec3::new(0.0, d.stack, -d.reach);
        let steer_dir = Vec3::new(0.0, -ha.sin(), -ha.cos());
        let head_bottom = head_top + steer_dir * d.head_tube;
        let normal = Vec3::new(0.0, ha.cos(), -ha.sin());
        let t = (head_bottom.y + normal.y * d.rake - d.bb_drop) / ha.sin();
        let front_hub = head_bottom + steer_dir * t + normal * d.rake;
        let shift = Vec3::new(
            0.0,
            d.wheel_radius - d.bb_drop,
            -0.5 * (rear_hub.z + front_hub.z),
        );
        let steerer_top = head_top - steer_dir * (d.spacers + 0.015);
        let rise = rad(d.stem_rise);
        let bar_clamp = steerer_top + Vec3::new(0.0, rise.sin(), -rise.cos()) * d.stem;
        let bb = shift;
        let mut f = Self {
            bb,
            rear_hub: rear_hub + shift,
            front_hub: front_hub + shift,
            seat_dir,
            head_top: head_top + shift,
            head_bottom: head_bottom + shift,
            steer_dir,
            steerer_top: steerer_top + shift,
            bar_clamp: bar_clamp + shift,
            saddle: bb + seat_dir * 0.7,
        };
        // Saddle height: the legs reach the pedals with the knee a little bent.
        let h = pose::bisect(0.45, 0.95, pose::EXTENSION_SEATED, |h| {
            let mut rig = rig_of(d, &f);
            rig.saddle = bb + seat_dir * h;
            pose::leg_extension(rider, &rig, rig.saddle + pose::HIP_ABOVE_SADDLE)
        });
        f.saddle = bb + seat_dir * h;
        f
    }
}

fn grips(d: &Design) -> ([Vec3; 2], [Vec3; 2], [Vec3; 2]) {
    let pair = |v: Vec3| [Vec3::new(-v.x, v.y, v.z), v];
    match d.bars {
        Bars::Drop {
            width,
            reach,
            drop,
            flare,
        } => (
            pair(Vec3::new(width * 0.5, 0.025, -reach - 0.005)),
            pair(Vec3::new(0.13, 0.005, 0.0)),
            pair(Vec3::new(width * 0.5 + flare, -drop, -0.03)),
        ),
        Bars::Aero => (
            pair(Vec3::new(0.05, 0.09, -0.3)),
            pair(Vec3::new(0.215, 0.0, -0.1)),
            pair(Vec3::new(0.215, 0.0, -0.1)),
        ),
        Bars::Flat { width, rise } => {
            let g = pair(Vec3::new(width * 0.5 - 0.07, rise, 0.022));
            (g, g, g)
        }
    }
}

/// The anchors of a design (what [`BikeRig::from_model`] reads back).
fn rig_of(d: &Design, f: &Frame) -> BikeRig {
    let (hoods, tops, drops) = grips(d);
    let at = |g: [Vec3; 2]| g.map(|v| f.bar_clamp + v);
    BikeRig {
        saddle: f.saddle,
        hoods: at(hoods),
        tops: at(tops),
        drops: at(drops),
        crank: f.bb,
        crank_length: d.crank,
        pedal_x: [-PEDAL_X, PEDAL_X],
        wheel_radius: d.wheel_radius,
        aero_bars: d.bars == Bars::Aero,
    }
}

/// Pedal spindle offset from the frame's centre (Q-factor / 2 + pedal).
pub const PEDAL_X: f32 = 0.11;

/// A frame tube from `a` to `b`: round, or deeper than wide (`aero` > 1).
fn frame_tube(s: &mut Shape, a: Vec3, b: Vec3, r: f32, aero: f32, sides: u32) {
    if (aero - 1.0).abs() < 0.05 {
        s.tube(a, b, r, r, sides, true);
    } else {
        let axis = b - a;
        let r0 = Ring::ellipse(a, axis, Vec3::X, r, r * aero);
        let r1 = Ring { centre: b, ..r0 };
        s.loft(&[r0, r1], sides, false, Cap::Flat, Cap::Flat);
    }
}

fn wheel(p: &mut Parts, d: &Design, kind: Wheel, hub: Vec3, rear: bool, lod: rider::Detail) {
    let hi = lod == rider::Detail::High;
    let r = d.wheel_radius;
    let segs = if hi { 40 } else { 12 };
    let knobs = if hi { d.knobs } else { 0.0 };
    p.slot("tyre").torus(
        hub,
        Vec3::X,
        r - d.tyre * 0.5,
        d.tyre * 0.5,
        if knobs > 0.0 { 48 } else { segs },
        if hi { 6 } else { 3 },
        knobs,
    );
    let rim_out = r - d.tyre + 0.004;
    let rim_in = r - d.tyre - d.rim_depth;
    if !hi {
        if kind == Wheel::Disc {
            p.slot("tyre").annulus(hub, Vec3::X, 0.0, rim_out, 0.02, 10);
        }
        return;
    }
    if d.skinwall {
        for s in [-1.0f32, 1.0] {
            p.slot("tyre_wall").annulus(
                hub + Vec3::X * s * d.tyre * 0.36,
                Vec3::X,
                r - d.tyre * 1.02,
                r - d.tyre * 0.55,
                0.003,
                segs,
            );
        }
    }
    match kind {
        Wheel::Disc => {
            p.slot("rim")
                .annulus(hub, Vec3::X, 0.0, rim_out, 0.024, segs);
            for s in [-1.0f32, 1.0] {
                // A stripe on each face: the spin shows.
                p.slot("decal").cuboid(
                    hub + Vec3::new(s * 0.0125, 0.0, -0.17),
                    Vec3::new(0.001, 0.016, 0.1),
                    Quat::IDENTITY,
                );
                p.slot("decal").cuboid(
                    hub + Vec3::new(s * 0.0125, 0.0, 0.17),
                    Vec3::new(0.001, 0.016, 0.06),
                    Quat::IDENTITY,
                );
            }
        }
        Wheel::ThreeSpoke | Wheel::Spoked(_) => {
            p.slot("rim")
                .annulus(hub, Vec3::X, rim_in, rim_out, d.rim_width, segs);
            if d.rim_depth >= 0.04 {
                for z in [-1.0f32, 1.0] {
                    for s in [-1.0f32, 1.0] {
                        p.slot("decal").cuboid(
                            hub + Vec3::new(
                                s * (d.rim_width * 0.5 + 0.0005),
                                0.0,
                                z * (rim_in + d.rim_depth * 0.5),
                            ),
                            Vec3::new(0.001, 0.08, d.rim_depth * 0.28),
                            Quat::IDENTITY,
                        );
                    }
                }
            }
        }
    }
    match kind {
        Wheel::Spoked(n) => {
            for k in 0..n {
                let a = k as f32 / n as f32 * TAU;
                let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                let lace = if (k / 2) % 2 == 0 { 0.5 } else { -0.5 };
                let dir = |a: f32| Vec3::new(0.0, a.cos(), a.sin());
                let at_rim = hub + dir(a) * (rim_in + 0.002);
                let at_hub = hub + Vec3::X * side * 0.032 + dir(a + lace) * 0.024;
                p.slot("metal")
                    .tube(at_hub, at_rim, 0.0016, 0.0016, 3, false);
            }
        }
        Wheel::ThreeSpoke => {
            for k in 0..3 {
                let a = k as f32 / 3.0 * TAU;
                let rot = Quat::from_rotation_x(a);
                let len = rim_in - 0.02;
                p.slot("rim").cuboid(
                    hub + rot * Vec3::new(0.0, 0.02 + len * 0.5, 0.0),
                    Vec3::new(0.01, len * 0.5 + 0.01, 0.026),
                    rot,
                );
            }
        }
        Wheel::Disc => {}
    }
    // Hub, flanges.
    p.slot("metal").tube(
        hub - Vec3::X * 0.05,
        hub + Vec3::X * 0.05,
        0.016,
        0.016,
        8,
        true,
    );
    if let Wheel::Spoked(_) = kind {
        for s in [-1.0f32, 1.0] {
            p.slot("metal")
                .annulus(hub + Vec3::X * s * 0.032, Vec3::X, 0.0, 0.028, 0.004, 10);
        }
    }
    if rear {
        p.slot("metal")
            .annulus(hub + Vec3::X * 0.042, Vec3::X, 0.012, 0.052, 0.022, 14);
    }
    if d.disc_brakes {
        p.slot("metal")
            .annulus(hub - Vec3::X * 0.048, Vec3::X, 0.058, 0.08, 0.002, 20);
    }
}

/// The handlebar parts around the clamp `c`; returns nothing (anchors are
/// added by the caller).
fn bars(p: &mut Parts, d: &Design, c: Vec3, sides: u32, hi: bool) {
    match d.bars {
        Bars::Drop {
            width,
            reach,
            drop,
            flare,
        } => {
            let hw = width * 0.5;
            let path = [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(hw - 0.04, 0.0, 0.0),
                Vec3::new(hw, 0.0, -0.03),
                Vec3::new(hw + flare * 0.1, -0.006, -reach + 0.012),
                Vec3::new(hw + flare * 0.3, -0.032, -reach - 0.004),
                Vec3::new(hw + flare * 0.65, -drop * 0.62, -reach + 0.004),
                Vec3::new(hw + flare, -drop, -reach + 0.045),
                Vec3::new(hw + flare, -drop - 0.004, -reach + 0.12),
            ];
            let radii = [0.0155, 0.0135, 0.012, 0.012, 0.012, 0.012, 0.012, 0.012];
            for side in [-1.0f32, 1.0] {
                let pts: Vec<Vec3> = path
                    .iter()
                    .map(|v| c + Vec3::new(v.x * side, v.y, v.z))
                    .collect();
                p.slot("bar_tape").sweep(&pts, &radii, sides, true);
                if hi {
                    // Hood and brake lever.
                    let hood = c + Vec3::new(side * (hw + flare * 0.15), 0.014, -reach - 0.012);
                    p.slot("component").ellipsoid(
                        hood,
                        Vec3::new(0.016, 0.024, 0.038),
                        Quat::from_rotation_x(-0.25),
                        4,
                        6,
                    );
                    p.slot("component").sweep(
                        &[
                            hood + Vec3::new(0.0, 0.0, -0.03),
                            c + Vec3::new(side * (hw + flare * 0.4), -drop * 0.45, -reach - 0.03),
                            c + Vec3::new(side * (hw + flare * 0.7), -drop * 0.8, -reach + 0.0),
                        ],
                        &[0.007, 0.006, 0.005],
                        4,
                        false,
                    );
                }
            }
        }
        Bars::Aero => {
            let base = [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.16, 0.0, -0.01),
                Vec3::new(0.205, -0.005, -0.05),
                Vec3::new(0.218, 0.0, -0.13),
            ];
            for side in [-1.0f32, 1.0] {
                let s = |v: Vec3| c + Vec3::new(v.x * side, v.y, v.z);
                let pts: Vec<Vec3> = base.iter().map(|v| s(*v)).collect();
                p.slot("bar_tape")
                    .sweep(&pts, &[0.016, 0.014, 0.013, 0.013], sides, true);
                // Extension with a ski bend at the tip.
                let ext = [
                    Vec3::new(0.06, 0.062, 0.05),
                    Vec3::new(0.055, 0.07, -0.24),
                    Vec3::new(0.05, 0.1, -0.33),
                ];
                let pts: Vec<Vec3> = ext.iter().map(|v| s(*v)).collect();
                p.slot("component")
                    .sweep(&pts, &[0.011, 0.011, 0.011], sides, true);
                // Arm pad on a riser.
                p.slot("component").tube(
                    s(Vec3::new(0.075, 0.0, 0.0)),
                    s(Vec3::new(0.075, 0.062, 0.0)),
                    0.009,
                    0.009,
                    sides,
                    false,
                );
                p.slot("bar_tape").cuboid(
                    s(Vec3::new(0.075, 0.07, 0.0)),
                    Vec3::new(0.04, 0.008, 0.055),
                    Quat::IDENTITY,
                );
                if hi {
                    p.slot("component").tube(
                        s(Vec3::new(0.218, 0.0, -0.13)),
                        s(Vec3::new(0.218, -0.01, -0.17)),
                        0.007,
                        0.006,
                        4,
                        true,
                    );
                }
            }
        }
        Bars::Flat { width, rise } => {
            let hw = width * 0.5;
            let pts = [
                Vec3::new(-hw, rise, 0.025),
                Vec3::new(-0.16, rise, 0.008),
                Vec3::new(-0.06, 0.0, 0.0),
                Vec3::new(0.06, 0.0, 0.0),
                Vec3::new(0.16, rise, 0.008),
                Vec3::new(hw, rise, 0.025),
            ]
            .map(|v| c + v);
            p.slot("component").sweep(
                &pts,
                &[0.011, 0.0115, 0.0155, 0.0155, 0.0115, 0.011],
                sides,
                true,
            );
            for side in [-1.0f32, 1.0] {
                let s = |v: Vec3| c + Vec3::new(v.x * side, v.y, v.z);
                p.slot("tyre").tube(
                    s(Vec3::new(hw - 0.13, rise, 0.022)),
                    s(Vec3::new(hw + 0.005, rise, 0.026)),
                    0.017,
                    0.017,
                    sides,
                    true,
                );
                if hi {
                    p.slot("component").tube(
                        s(Vec3::new(hw - 0.15, rise + 0.01, 0.018)),
                        s(Vec3::new(hw - 0.09, rise + 0.005, -0.06)),
                        0.005,
                        0.004,
                        4,
                        true,
                    );
                }
            }
        }
    }
}

/// Builds the bike of `profile` with the rider (for the saddle height and
/// the far LOD) of `rider_lod0` / `rider_lod1`.
pub fn build(profile: ProfileId, rider_model: &Model, rider_low: &Model) -> Model {
    let rig = RiderRig::from_model(rider_model).expect("rider rig");
    let d = design(profile);
    let f = Frame::new(&d, &rig);
    let mut m = Model::default();
    let root = m.add(Node::new(ROOT, None, Vec3::ZERO));
    for lod in [rider::Detail::High, rider::Detail::Low] {
        let hi = lod == rider::Detail::High;
        let sides = lod.sides();
        let mut p = Parts::new();
        let t = d.tube;
        let aero = if hi { d.aero } else { 1.0 };
        let cluster = f.bb + f.seat_dir * d.top_tube_at;
        let stays = f.bb + f.seat_dir * d.stays_at;
        // Main triangle and head tube.
        {
            let s = p.slot("frame");
            frame_tube(
                s,
                f.head_bottom - f.steer_dir * 0.01,
                f.head_top + f.steer_dir * -0.01,
                t * 1.3,
                if hi { 1.0 + (d.aero - 1.0) * 0.5 } else { 1.0 },
                sides,
            );
            frame_tube(
                s,
                cluster,
                f.head_top + f.steer_dir * 0.025,
                t * 0.9,
                aero.min(1.6),
                sides,
            );
            frame_tube(
                s,
                f.bb + Vec3::new(0.0, 0.01, -0.01),
                f.head_bottom - f.steer_dir * 0.025,
                t * 1.15,
                aero,
                sides,
            );
            frame_tube(s, f.bb, cluster + f.seat_dir * 0.03, t, aero, sides);
            s.tube(
                f.bb - Vec3::X * 0.036,
                f.bb + Vec3::X * 0.036,
                0.021,
                0.021,
                sides,
                true,
            );
            for side in [-1.0f32, 1.0] {
                let hub = f.rear_hub + Vec3::X * side * 0.066;
                s.tube(
                    f.bb + Vec3::new(side * 0.03, 0.0, 0.02),
                    hub,
                    t * 0.6,
                    t * 0.45,
                    sides,
                    true,
                );
                s.tube(
                    stays + Vec3::X * side * 0.018,
                    hub,
                    t * 0.5,
                    t * 0.4,
                    sides,
                    true,
                );
            }
        }
        // Fork.
        if d.suspension {
            p.slot("component").cuboid(
                f.head_bottom + f.steer_dir * 0.03,
                Vec3::new(0.075, 0.018, 0.03),
                Quat::from_rotation_arc(Vec3::NEG_Y, f.steer_dir),
            );
            for side in [-1.0f32, 1.0] {
                let top = f.head_bottom + f.steer_dir * 0.03 + Vec3::X * side * 0.058;
                let mid = top + f.steer_dir * 0.2;
                let hub = f.front_hub + Vec3::X * side * 0.058 - f.steer_dir * 0.0;
                p.slot("metal").tube(top, mid, 0.017, 0.017, sides, false);
                p.slot("component").tube(
                    mid - f.steer_dir * 0.04,
                    hub + f.steer_dir * 0.02,
                    0.023,
                    0.02,
                    sides,
                    true,
                );
            }
            if hi {
                let arch = f.head_bottom + f.steer_dir * 0.2;
                p.slot("component").sweep(
                    &[
                        arch + Vec3::new(-0.058, 0.0, -0.01),
                        arch + Vec3::new(0.0, 0.04, -0.035),
                        arch + Vec3::new(0.058, 0.0, -0.01),
                    ],
                    &[0.012, 0.012, 0.012],
                    sides,
                    true,
                );
            }
        } else {
            // Forward and up, at right angles to the steering axis (the rake).
            let normal = Vec3::new(0.0, -f.steer_dir.z, f.steer_dir.y).normalize();
            for side in [-1.0f32, 1.0] {
                let top = f.head_bottom + Vec3::X * side * 0.04;
                let hub = f.front_hub + Vec3::X * side * 0.052;
                let mid = top.lerp(hub, 0.55) + normal * (d.rake * 0.3);
                p.slot("frame")
                    .sweep(&[top, mid, hub], &[t * 0.95, t * 0.7, t * 0.5], sides, true);
            }
            p.slot("frame").cuboid(
                f.head_bottom + f.steer_dir * 0.012,
                Vec3::new(0.05, 0.016, 0.026),
                Quat::from_rotation_arc(Vec3::NEG_Y, f.steer_dir),
            );
        }
        // Steerer, spacers and stem.
        p.slot("component").tube(
            f.head_top,
            f.steerer_top + f.steer_dir * -0.02,
            0.018,
            0.018,
            sides,
            true,
        );
        p.slot("component").tube(
            f.steerer_top - f.steer_dir * 0.005,
            f.bar_clamp,
            0.017,
            0.016,
            sides,
            true,
        );
        // Seat post.
        let post_top = f.saddle - Vec3::Y * 0.035 + Vec3::Z * -0.025;
        frame_tube(
            p.slot("component"),
            cluster - f.seat_dir * 0.05,
            post_top,
            0.0135,
            if d.aero > 1.5 { 1.8 } else { 1.0 },
            sides,
        );
        // Drivetrain: chain runs, rear derailleur.
        let ring = d.rings.0;
        let cog = f.rear_hub + Vec3::X * 0.045;
        let chain_x = Vec3::X * 0.048;
        p.slot("metal").tube(
            f.bb + chain_x + Vec3::Y * ring,
            cog + Vec3::Y * 0.045,
            0.004,
            0.004,
            3,
            false,
        );
        let derailleur = f.rear_hub + Vec3::new(0.065, -0.075, 0.02);
        p.slot("metal").tube(
            f.bb + chain_x - Vec3::Y * ring,
            derailleur,
            0.004,
            0.004,
            3,
            false,
        );
        if hi {
            p.slot("component").cuboid(
                f.rear_hub + Vec3::new(0.062, -0.045, 0.015),
                Vec3::new(0.008, 0.035, 0.014),
                Quat::from_rotation_x(0.3),
            );
            if d.rings.1.is_some() {
                p.slot("component").cuboid(
                    f.bb + f.seat_dir * 0.17 + Vec3::new(0.035, 0.0, -0.025),
                    Vec3::new(0.008, 0.02, 0.03),
                    Quat::IDENTITY,
                );
            }
            // Brakes: callipers at the crown and the stay bridge, or discs.
            if d.disc_brakes {
                p.slot("component").cuboid(
                    f.front_hub + Vec3::new(-0.06, 0.06, 0.04),
                    Vec3::new(0.012, 0.025, 0.02),
                    Quat::IDENTITY,
                );
                p.slot("component").cuboid(
                    f.rear_hub + Vec3::new(-0.06, 0.07, -0.02),
                    Vec3::new(0.012, 0.02, 0.025),
                    Quat::IDENTITY,
                );
            } else if d.bars != Bars::Aero {
                p.slot("component").cuboid(
                    f.head_bottom + f.steer_dir * 0.03 + Vec3::new(0.0, 0.0, -0.02),
                    Vec3::new(0.035, 0.012, 0.012),
                    Quat::IDENTITY,
                );
                p.slot("component").cuboid(
                    stays + (f.rear_hub - stays) * 0.35 + Vec3::Z * 0.01,
                    Vec3::new(0.035, 0.012, 0.012),
                    Quat::IDENTITY,
                );
            }
            // Down tube decal.
            let (a, b) = (f.bb, f.head_bottom);
            frame_tube(
                p.slot("decal"),
                a.lerp(b, 0.38),
                a.lerp(b, 0.62),
                t * 1.15 * 1.04,
                aero,
                sides,
            );
            // Bottles.
            let down = (b - a).normalize();
            let up_from_down =
                Vec3::new(0.0, -down.z, down.y).normalize() * if down.z < 0.0 { 1.0 } else { -1.0 };
            let mut spots = Vec::new();
            if d.bottles >= 1 {
                spots.push((
                    a.lerp(b, 0.45) + up_from_down * (t * 1.15 * d.aero.min(1.6) + 0.04),
                    down,
                ));
            }
            if d.bottles >= 2 {
                let fwd = Vec3::new(0.0, f.seat_dir.z, -f.seat_dir.y).normalize();
                let fwd = if fwd.z > 0.0 { -fwd } else { fwd };
                spots.push((
                    f.bb + f.seat_dir * 0.28 + fwd * (t * d.aero.min(1.6) + 0.04),
                    f.seat_dir,
                ));
            }
            for (c, dir) in spots {
                p.slot("bottle")
                    .tube(c - dir * 0.1, c + dir * 0.08, 0.036, 0.036, 10, true);
                p.slot("component")
                    .tube(c + dir * 0.08, c + dir * 0.105, 0.018, 0.012, 6, true);
            }
            if d.bag {
                let tt = (f.head_top - cluster).normalize();
                let c = cluster.lerp(f.head_top, 0.82) + Vec3::Y * 0.05;
                p.slot("component").cuboid(
                    c,
                    Vec3::new(0.028, 0.035, 0.07),
                    Quat::from_rotation_arc(Vec3::NEG_Z, tt),
                );
            }
        }
        if hi {
            p.attach(&mut m, root);
        } else {
            // The far LOD: the parts so far plus wheels, bars, saddle, cranks
            // and a seated rider, all in one node.
            let lod1 = m.add(Node::new(LOD1, Some(root), Vec3::ZERO));
            for (hub, kind, rear) in [(f.front_hub, d.front, false), (f.rear_hub, d.rear, true)] {
                wheel(&mut p, &d, kind, hub, rear, lod);
            }
            bars(&mut p, &d, f.bar_clamp, 3, false);
            p.slot("component").cuboid(
                f.saddle + Vec3::new(0.0, -0.01, -0.03),
                Vec3::new(0.05, 0.015, 0.13),
                Quat::IDENTITY,
            );
            let bike_rig = rig_of(&d, &f);
            let params = PoseParams {
                crank: 1.1,
                ..Default::default()
            };
            let low_rig = RiderRig::from_model(rider_low).expect("rider rig");
            let pose = pose::pose(&low_rig, &bike_rig, &Fit::new(&low_rig, &bike_rig), &params);
            for (s, side) in [(0usize, -1.0f32), (1, 1.0)] {
                let ped = bike_rig.pedal(s, params.crank);
                p.slot("component").tube(
                    f.bb + Vec3::X * side * 0.07,
                    ped - Vec3::X * side * 0.03,
                    0.01,
                    0.01,
                    3,
                    false,
                );
            }
            let mut posed = rider_low.clone();
            pose.apply(&mut posed);
            let skip: Vec<usize> = [
                if d.bars == Bars::Aero {
                    pose::nodes::HELMET_ROAD
                } else {
                    pose::nodes::HELMET_AERO
                },
                pose::nodes::CAP,
                pose::nodes::KIT_BAND,
                pose::nodes::KIT_SASH,
            ]
            .iter()
            .filter_map(|n| posed.node(n))
            .collect();
            for (i, node) in posed.nodes.iter().enumerate() {
                if skip.iter().any(|&s| posed.is_below(i, s)) {
                    continue;
                }
                let w = posed.world(i);
                for prim in &node.primitives {
                    let shape = p.slot(
                        crate::slots::get(&posed.materials[prim.material].name)
                            .expect("slot")
                            .name,
                    );
                    let base = shape.positions.len() as u32;
                    shape.positions.extend(
                        prim.positions
                            .iter()
                            .map(|v| w.transform_point3(Vec3::from(*v))),
                    );
                    shape.normals.extend(
                        prim.normals
                            .iter()
                            .map(|n| w.transform_vector3(Vec3::from(*n)).normalize()),
                    );
                    shape.indices.extend(prim.indices.iter().map(|i| base + i));
                }
            }
            p.remap(far_slot).attach(&mut m, lod1);
        }
    }
    // Wheels.
    for (name, hub, kind, rear) in [
        (WHEEL_FRONT, f.front_hub, d.front, false),
        (WHEEL_REAR, f.rear_hub, d.rear, true),
    ] {
        let n = m.add(Node::new(name, Some(root), hub));
        let mut p = Parts::new();
        wheel(&mut p, &d, kind, hub, rear, rider::Detail::High);
        p.attach(&mut m, n);
    }
    // Cranks (rest: the right arm up) and pedals.
    let crank = m.add(Node::new(CRANK, Some(root), f.bb));
    let mut p = Parts::new();
    let l = d.crank;
    p.slot("metal").tube(
        f.bb - Vec3::X * 0.075,
        f.bb + Vec3::X * 0.075,
        0.011,
        0.011,
        8,
        true,
    );
    let (big, small) = d.rings;
    p.slot("metal")
        .annulus(f.bb + Vec3::X * 0.05, Vec3::X, big - 0.013, big, 0.004, 32);
    if let Some(r) = small {
        p.slot("metal")
            .annulus(f.bb + Vec3::X * 0.043, Vec3::X, r - 0.012, r, 0.004, 28);
    }
    for k in 0..4 {
        let rot = Quat::from_rotation_x(k as f32 / 4.0 * TAU + PI / 4.0);
        let len = big - 0.015;
        p.slot("component").cuboid(
            f.bb + Vec3::X * 0.054 + rot * Vec3::new(0.0, len * 0.5, 0.0),
            Vec3::new(0.004, len * 0.5, 0.008),
            rot,
        );
    }
    p.slot("component").cuboid(
        f.bb + Vec3::new(0.068, l * 0.5, 0.0),
        Vec3::new(0.007, l * 0.5 + 0.014, 0.013),
        Quat::IDENTITY,
    );
    p.slot("component").cuboid(
        f.bb + Vec3::new(-0.068, -l * 0.5, 0.0),
        Vec3::new(0.007, l * 0.5 + 0.014, 0.013),
        Quat::IDENTITY,
    );
    p.attach(&mut m, crank);
    for (s, side) in [(0usize, -1.0f32), (1, 1.0)] {
        let at = Vec3::new(side * PEDAL_X, if s == 0 { -l } else { l }, 0.0);
        let n = m.add(Node::new(PEDAL[s], Some(crank), at));
        let mut p = Parts::new();
        let c = f.bb + at;
        p.slot("metal")
            .tube(c - Vec3::X * side * 0.04, c, 0.006, 0.006, 6, false);
        p.slot("component").cuboid(
            c + Vec3::new(side * 0.004, -0.009, -0.005),
            Vec3::new(0.035, 0.008, 0.045),
            Quat::IDENTITY,
        );
        p.attach(&mut m, n);
    }
    // Handlebar with the grip anchors.
    let bar = m.add(Node::new(HANDLEBAR, Some(root), f.bar_clamp));
    let mut p = Parts::new();
    bars(&mut p, &d, f.bar_clamp, rider::Detail::High.sides(), true);
    p.attach(&mut m, bar);
    let (hoods, tops, drops) = grips(&d);
    for (names, g) in [(HOODS, hoods), (TOPS, tops), (DROPS, drops)] {
        for s in 0..2 {
            m.add(Node::new(names[s], Some(bar), g[s]));
        }
    }
    if d.bars == Bars::Aero {
        m.add(Node::new(
            "aero_pads",
            Some(bar),
            Vec3::new(0.0, 0.075, 0.0),
        ));
    }
    // Saddle (anchor = where the sit bones rest).
    let saddle = m.add(Node::new(SADDLE, Some(root), f.saddle));
    let mut p = Parts::new();
    let s = f.saddle;
    let r = |z: f32, hw: f32, hh: f32| Ring {
        centre: s + Vec3::new(0.0, -0.014, z),
        u: Vec3::X * hw,
        v: Vec3::Y * hh,
    };
    let (back, nose) = if d.bars == Bars::Aero {
        (0.06, -0.17)
    } else {
        (0.08, -0.19)
    };
    p.slot("component").loft(
        &[
            r(back, 0.068, 0.017),
            r(back - 0.06, 0.066, 0.02),
            r(-0.06, 0.032, 0.017),
            r(nose + 0.03, 0.019, 0.013),
        ],
        10,
        false,
        Cap::Flat,
        Cap::Point(s + Vec3::new(0.0, -0.016, nose)),
    );
    for x in [-0.022f32, 0.022] {
        p.slot("metal").tube(
            s + Vec3::new(x, -0.03, back - 0.02),
            s + Vec3::new(x, -0.03, nose + 0.05),
            0.0035,
            0.0035,
            4,
            false,
        );
    }
    p.attach(&mut m, saddle);
    m
}

/// Slot of a part in the far LOD: few materials (few draw calls).
fn far_slot(name: &str) -> &'static str {
    match name {
        "skin" | "hair" => "skin",
        "shoe" | "sock" => "shoe",
        "helmet" | "helmet_detail" => "helmet",
        "jersey" => "jersey",
        "accent" => "jersey",
        "shorts" => "shorts",
        "frame" | "decal" => "frame",
        _ => "tyre",
    }
}

/// A fresh bike rig of `profile` with the default rider (tests, tools).
pub fn rig(profile: ProfileId) -> BikeRig {
    let r = rider::build(rider::Detail::High);
    let rig = RiderRig::from_model(&r).expect("rider rig");
    let d = design(profile);
    rig_of(&d, &Frame::new(&d, &rig))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_built_from_their_geometry() {
        let rig = RiderRig::from_model(&rider::build(rider::Detail::High)).unwrap();
        for p in ProfileId::ALL {
            let d = design(p);
            let f = Frame::new(&d, &rig);
            // Both wheels on the ground, centred on the origin.
            assert!((f.front_hub.y - d.wheel_radius).abs() < 1e-5, "{p:?}");
            assert!((f.rear_hub.y - d.wheel_radius).abs() < 1e-5);
            assert!((f.front_hub.z + f.rear_hub.z).abs() < 1e-5);
            let wb = f.rear_hub.z - f.front_hub.z;
            let expect = if p == ProfileId::Mountain {
                1.15..1.3
            } else {
                0.95..1.1
            };
            assert!(expect.contains(&wb), "{p:?} wheelbase {wb}");
            // Bottom bracket below the hubs by the drop.
            assert!((f.bb.y - (d.wheel_radius - d.bb_drop)).abs() < 1e-5);
            // Saddle 0.65–0.8 m above the bottom bracket.
            let h = f.saddle.distance(f.bb);
            assert!((0.62..0.82).contains(&h), "{p:?} saddle height {h}");
        }
    }

    #[test]
    fn bike_models_have_their_nodes_and_rig() {
        let r = rider::build(rider::Detail::High);
        let low = rider::build(rider::Detail::Low);
        for p in ProfileId::ALL {
            let m = build(p, &r, &low);
            for n in REQUIRED {
                assert!(m.node(n).is_some(), "{p:?}: {n}");
            }
            let rig = BikeRig::from_model(&m).unwrap();
            let fresh = super::rig(p);
            assert!(
                rig.saddle.distance(fresh.saddle) < 1e-5,
                "{p:?}: anchors read back"
            );
            assert!(rig.hoods[1].distance(fresh.hoods[1]) < 1e-5);
            assert!((rig.crank_length - fresh.crank_length).abs() < 1e-5);
            assert!((rig.wheel_radius - fresh.wheel_radius).abs() < 1e-5);
            let lod1 = m.node(LOD1).unwrap();
            let far = m.triangles_below(Some(lod1));
            let near = m.triangles() - far;
            assert!(far * 3 < near, "{p:?}: far {far} near {near}");
            assert!(near < 12_000, "{p:?}: {near}");
            // The far LOD is a handful of draw calls.
            assert!(m.nodes[lod1].primitives.len() <= 8, "{p:?}");
            assert_eq!(rig.aero_bars, p == ProfileId::TimeTrial);
        }
    }
}
