//! The generated low-poly rider (spec §2.6, §8.3): rigid body parts on a
//! node hierarchy (pelvis → thighs → shins → feet; pelvis → torso → head,
//! upper arms → forearms → hands), in the rest pose (standing, arms
//! hanging, facing −Z). Paint slots: jersey, accent (kit pattern), shorts,
//! skin, sock, sock_aero, shoe, shoe_race, overshoe, glove, glove_pad,
//! helmet, helmet_detail, lens, metal, hair.
//!
//! The kit's variants (spec §14 Phase 4, 4C, W4: helmets, cap, sleeves,
//! skinsuit, tights, socks, shoes, overshoes, gloves) and the character's
//! hair styles, faces and glasses are optional nodes ([`opt`],
//! [`VARIANTS`]); [`visible`] picks the ones a look shows. All generated
//! here: own work, MIT OR Apache-2.0 (licence rule 8A).

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

/// The optional nodes of the rider model (spec §14 Phase 4, 4C, W4): the
/// kit's variants, the base items they replace, and the character's hair,
/// faces and glasses. One choice per group is shown ([`visible`]).
pub mod opt {
    pub const HELMET_LIGHT: &str = "helmet_light";
    pub const HELMET_AERO_ROAD: &str = "helmet_aero_road";
    pub const SLEEVE_UPPER: [&str; 2] = ["sleeve_upper_l", "sleeve_upper_r"];
    pub const SLEEVE_LONG: [&str; 2] = ["sleeve_long_l", "sleeve_long_r"];
    pub const SUIT_PELVIS: &str = "suit_pelvis";
    pub const SUIT_THIGH: [&str; 2] = ["suit_thigh_l", "suit_thigh_r"];
    pub const SHORTS_LONG: [&str; 2] = ["shorts_long_l", "shorts_long_r"];
    pub const TIGHTS_THIGH: [&str; 2] = ["tights_thigh_l", "tights_thigh_r"];
    pub const TIGHTS_SHIN: [&str; 2] = ["tights_shin_l", "tights_shin_r"];
    pub const SOCK_TALL: [&str; 2] = ["sock_tall_l", "sock_tall_r"];
    pub const SOCK_AERO: [&str; 2] = ["sock_aero_l", "sock_aero_r"];
    pub const SHOE_ROAD: [&str; 2] = ["shoe_road_l", "shoe_road_r"];
    pub const SHOE_RACE: [&str; 2] = ["shoe_race_l", "shoe_race_r"];
    pub const OVERSHOE: [&str; 2] = ["overshoe_l", "overshoe_r"];
    pub const GLOVE_MITT: [&str; 2] = ["glove_mitt_l", "glove_mitt_r"];
    pub const GLOVE_GEL: [&str; 2] = ["glove_gel_l", "glove_gel_r"];
    pub const GLOVE_FULL: [&str; 2] = ["glove_full_l", "glove_full_r"];
    pub const GLOVE_WINTER: [&str; 2] = ["glove_winter_l", "glove_winter_r"];
    pub const HAIR_CAP: &str = "hair_cap";
    pub const HAIR_BUZZ: &str = "hair_buzz";
    pub const HAIR_LONG: &str = "hair_long";
    pub const HAIR_TAIL: &str = "hair_tail";
    pub const HAIR_BUN: &str = "hair_bun";
    // The hair catalogue of spec §14 Phase 4, 4N N5: every shape lies
    // under every helmet's shell; tails, braids and buns leave below its
    // rim at the back (`crate::helmet` checks it).
    pub const HAIR_SHAVED: &str = "hair_shaved";
    pub const HAIR_FRINGE: &str = "hair_fringe";
    pub const HAIR_TEXTURED: &str = "hair_textured";
    pub const HAIR_SIDE_PART: &str = "hair_side_part";
    pub const HAIR_CURLS: &str = "hair_curls";
    pub const HAIR_AFRO: &str = "hair_afro";
    pub const HAIR_MID: &str = "hair_mid";
    pub const HAIR_LONG_CURLY: &str = "hair_long_curly";
    pub const HAIR_TAIL_LOW: &str = "hair_tail_low";
    pub const HAIR_BRAID: &str = "hair_braid";
    pub const HAIR_BRAIDS: &str = "hair_braids";
    pub const HAIR_BUN_LOW: &str = "hair_bun_low";
    pub const HAIR_LOCS: &str = "hair_locs";
    pub const HAIR_ROWS: &str = "hair_cornrows";
    /// The moustaches of `ocw_core::character::MOUSTACHES` after "None"
    /// (4N N6; paint slot `facial_hair`).
    pub const MOUSTACHE: [&str; 5] = [
        "moustache_classic",
        "moustache_chevron",
        "moustache_handlebar",
        "moustache_horseshoe",
        "moustache_pencil",
    ];
    /// The beards of `ocw_core::character::BEARDS` after "None".
    pub const BEARD: [&str; 7] = [
        "beard_stubble",
        "beard_short_stubble",
        "beard_short_boxed",
        "beard_full",
        "beard_goatee",
        "beard_chin_strap",
        "beard_long",
    ];
    /// The base bodies' shoulders (A, B, C; 4N N2) and chests (B, C).
    pub const SHOULDERS: [&str; 3] = ["shoulders_a", "shoulders_b", "shoulders_c"];
    pub const CHEST: [&str; 2] = ["chest_b", "chest_c"];
    /// The face's lines with the age (light, deeper) and the softer jaw
    /// (4N N3, Q12; paint slots `skin_line`, `skin`).
    pub const FACE_LINES: [&str; 2] = ["face_lines_light", "face_lines_deep"];
    pub const FACE_JAW: &str = "face_jaw_soft";
    pub const GLASSES_SPORT: &str = "glasses_sport";
    pub const GLASSES_ROUND: &str = "glasses_round";
    /// The race number's paper plates on the hips (spec §14 Phase 8 T6),
    /// on the pelvis: shown when the look has a race number.
    pub const HIP_NUMBER: [&str; 2] = ["hip_number_l", "hip_number_r"];
}

/// The rider's hands (asked by the user 2026-11-17: "give the rider real
/// hands"; spec §14 Phase 4, 4N, Q25): a palm, a thumb and four fingers
/// with their knuckle segments, in one shape per grip ([`HandShape`]) –
/// nodes `hand_<shape>_<l|r>` under the hand, one shown at a time (the
/// pose picks it, [`crate::pose::Pose::hand_shapes`]). Each lies in the
/// hand's frame: the wrist at the origin, the fingers towards −y, the
/// knuckles along z (the index finger in front, −z), the palm's inner face
/// towards the body's middle (+x on the left hand, −x on the right). A
/// grip's bar runs along z through [`bar_centre`], the fingers wrap it at
/// its radius plus a finger's. Paint: the palm and the first finger
/// joints `glove` (every glove covers them), the middle and outer joints
/// `fingertip` (the skin out of fingerless mitts, the glove under full
/// and winter gloves). The far rider keeps the mitten (the low model's
/// glove ellipsoid).
pub mod hand {
    use glam::Vec3;

    /// A hand's shape.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum HandShape {
        /// Round a bar: tops, drops, the TT extensions and base bar, the
        /// MTB grips.
        Wrap,
        /// Round a road lever's hood (thicker).
        Hoods,
        /// Holding the phone of the status signs.
        Phone,
        /// Loose, a little curled: standing, the hand off the bar.
        Relaxed,
        /// Open, the fingers straight: arms up at the finish.
        Open,
    }

    impl HandShape {
        pub const ALL: [HandShape; 5] = [
            HandShape::Wrap,
            HandShape::Hoods,
            HandShape::Phone,
            HandShape::Relaxed,
            HandShape::Open,
        ];

        pub fn id(self) -> &'static str {
            match self {
                HandShape::Wrap => "wrap",
                HandShape::Hoods => "hoods",
                HandShape::Phone => "phone",
                HandShape::Relaxed => "relaxed",
                HandShape::Open => "open",
            }
        }

        /// The radius the fingers wrap (model metres, at the default
        /// height; a short rider's hand is smaller, so it is generous):
        /// `None` for the free shapes.
        pub fn wrap_radius(self) -> Option<f32> {
            match self {
                HandShape::Wrap => Some(WRAP_RADIUS),
                HandShape::Hoods => Some(HOODS_RADIUS),
                HandShape::Phone => Some(PHONE_RADIUS),
                HandShape::Relaxed | HandShape::Open => None,
            }
        }
    }

    /// Bar radii the shapes wrap (model metres): bars and grips up to
    /// 1.65 cm (with tape) for a rider 0.84 times the default height; a
    /// hood's body; the phone's edge.
    pub const WRAP_RADIUS: f32 = 0.0205;
    pub const HOODS_RADIUS: f32 = 0.042;
    pub const PHONE_RADIUS: f32 = 0.03;
    /// A finger's radius at its base and at its tip.
    pub const FINGER_RADIUS: f32 = 0.0085;
    pub const TIP_RADIUS: f32 = 0.0068;
    /// The palm: its half thickness (x), half length (y), half width (z),
    /// its centre's y, and the knuckles' y.
    pub const PALM_HALF: [f32; 3] = [0.012, 0.041, 0.037];
    pub const PALM_Y: f32 = -0.046;
    pub const KNUCKLE_Y: f32 = -0.085;
    /// Where a grip's bar crosses the hand's long axis (y).
    pub const BAR_Y: f32 = -0.07;
    /// The wrap thumb's joints' angles round the bar (rad, in the hand's
    /// (x, y) plane about the bar's centre: 0 = the side away from the
    /// palm, π/2 = towards the wrist): from the palm's edge over the
    /// wrist's side round to the far side, against the fingers (a power
    /// grip), over the index finger.
    pub const WRAP_THUMB_PHI: [f32; 3] = [2.2, 1.3, 0.5];
    /// The thumb over a hood past its base (x towards the palm's side,
    /// y, z): down the hood's inner side and forward.
    pub const HOOD_THUMB: [[f32; 3]; 3] = [
        [0.02, -0.045, -0.04],
        [0.034, -0.06, -0.038],
        [0.044, -0.075, -0.034],
    ];
    /// The relaxed hand's bend at the index finger's base, middle and
    /// outer joint (rad; each finger towards the little one a little more).
    pub const RELAXED_BEND: [f32; 3] = [0.25, 0.45, 0.25];
    /// The fingers: knuckle z (index in front), joint lengths (base,
    /// middle, outer).
    pub const FINGERS: [(f32, [f32; 3]); 4] = [
        (-0.027, [0.040, 0.025, 0.020]),
        (-0.009, [0.044, 0.028, 0.021]),
        (0.009, [0.041, 0.026, 0.020]),
        (0.026, [0.032, 0.020, 0.018]),
    ];

    /// The node of `shape` on hand `side` (0 left, 1 right).
    pub fn node(shape: HandShape, side: usize) -> &'static str {
        const NAMES: [[&str; 2]; 5] = [
            ["hand_wrap_l", "hand_wrap_r"],
            ["hand_hoods_l", "hand_hoods_r"],
            ["hand_phone_l", "hand_phone_r"],
            ["hand_relaxed_l", "hand_relaxed_r"],
            ["hand_open_l", "hand_open_r"],
        ];
        let i = HandShape::ALL.iter().position(|s| *s == shape).unwrap_or(0);
        NAMES[i][side.min(1)]
    }

    /// The shape and side of a hand node.
    pub fn of_node(name: &str) -> Option<(HandShape, usize)> {
        HandShape::ALL.into_iter().find_map(|s| {
            (0..2)
                .find(|side| node(s, *side) == name)
                .map(|side| (s, side))
        })
    }

    /// Every hand shape node.
    pub fn nodes() -> Vec<&'static str> {
        HandShape::ALL
            .into_iter()
            .flat_map(|s| [node(s, 0), node(s, 1)])
            .collect()
    }

    /// The hand nodes not shown while both hands are `shape`.
    pub fn others(shape: HandShape) -> Vec<&'static str> {
        HandShape::ALL
            .into_iter()
            .filter(|s| *s != shape)
            .flat_map(|s| [node(s, 0), node(s, 1)])
            .collect()
    }

    /// Whether a worn kit's gloves cover the fingertips (full and winter
    /// gloves; the base mitts and gel gloves are fingerless). `kit`: the
    /// rider variant ids of the worn items.
    pub fn covers_fingertips(kit: &[String]) -> bool {
        kit.iter()
            .any(|k| k == "gloves-full" || k == "gloves-winter")
    }

    /// +1 on the left hand (the palm faces +x), −1 on the right.
    pub fn palm_sign(side: usize) -> f32 {
        if side == 0 { 1.0 } else { -1.0 }
    }

    /// The centre of the bar `shape` wraps, in the hand's frame (its axis
    /// along z): beside the palm's inner face, level with the base of the
    /// fingers.
    pub fn bar_centre(shape: HandShape, side: usize) -> Option<Vec3> {
        let r = shape.wrap_radius()?;
        Some(Vec3::new(palm_sign(side) * (PALM_HALF[0] + r), BAR_Y, 0.0))
    }

    /// The palm's ends (y): the heel's rounded end at the wrist and the
    /// knuckle end, hidden in the fingers' bases.
    pub const PALM_HEEL_Y: f32 = -0.003;
    pub const PALM_END_Y: f32 = -0.09;
    /// Rings along the palm and points per ring.
    const PALM_RINGS: usize = 8;
    const PALM_SIDES: usize = 12;

    /// A smooth bump: 1 at `c`, falling to 0 at `half` away (cos²).
    fn bump(x: f32, c: f32, half: f32) -> f32 {
        let d = ((x - c) / half).abs();
        if d >= 1.0 {
            0.0
        } else {
            (d * std::f32::consts::FRAC_PI_2).cos().powi(2)
        }
    }

    /// The palm of hand `side` in the hand's frame (bug 2026-11-17: "the
    /// palm is a plain box"): a rounded slab, narrower at the wrist than
    /// at the knuckles, its heel rounded, the thumb's ball (thenar) and
    /// the little finger's edge (hypothenar) raised on the inner face
    /// above where a bar lies, a ridge of knuckles on the back over each
    /// finger's base. Its inner face stays at [`PALM_HALF`]`[0]` where the
    /// grips' bars lie ([`bar_centre`]). Vertices and outward triangles
    /// (smooth normals).
    pub fn palm(side: usize) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        use std::f32::consts::TAU;
        let n = palm_sign(side);
        // Along the palm (0 = heel, 1 = knuckles) and the end rounding.
        const T: [f32; PALM_RINGS] = [0.0, 0.08, 0.22, 0.42, 0.62, 0.8, 0.92, 1.0];
        const ROUND: [f32; PALM_RINGS] = [0.55, 0.84, 0.97, 1.0, 1.0, 0.99, 0.9, 0.62];
        let mut verts = Vec::with_capacity(PALM_RINGS * PALM_SIDES + 2);
        for (t, round) in T.iter().zip(ROUND) {
            let y = PALM_HEEL_Y + (PALM_END_Y - PALM_HEEL_Y) * t;
            // Tapered: the wrist narrower than the knuckle line, thinner
            // towards the knuckles.
            let widen = (t / 0.7).min(1.0);
            let hz = (0.029 + (PALM_HALF[2] - 0.029) * widen * (2.0 - widen)) * round;
            let hx = (0.0128 - 0.0015 * t) * round.sqrt();
            for k in 0..PALM_SIDES {
                let a = (k as f32 + 0.5) / PALM_SIDES as f32 * TAU;
                let (s, c) = a.sin_cos();
                // A rounded box's section (superellipse).
                let shape = |v: f32| v.signum() * v.abs().powf(0.55);
                let mut u = hx * shape(c);
                let z = hz * shape(s);
                if c > 0.0 {
                    // The inner face: the thumb's ball (index side, −z)
                    // and the little finger's edge, above the bars.
                    let thenar = 0.0065 * bump(y, -0.03, 0.026) * bump(z, -0.021, 0.022);
                    let hypo = 0.0035 * bump(y, -0.034, 0.02) * bump(z, 0.022, 0.018);
                    u += (thenar + hypo) * c.powf(0.5);
                } else {
                    // The back: the knuckles over the fingers' bases.
                    let ridge: f32 = FINGERS.iter().map(|(fz, _)| bump(z, *fz, 0.0085)).sum();
                    u -= 0.003
                        * (0.5 + 0.5 * ridge.min(1.0))
                        * bump(y, KNUCKLE_Y + 0.001, 0.011)
                        * (-c).powf(0.5);
                }
                verts.push(Vec3::new(n * u, y, z));
            }
        }
        let heel = verts.len() as u32;
        verts.push(Vec3::new(0.0, PALM_HEEL_Y + 0.004, 0.0));
        let tip = heel + 1;
        verts.push(Vec3::new(0.0, PALM_END_Y - 0.0035, 0.0));
        let at = |i: usize, k: usize| (i * PALM_SIDES + k % PALM_SIDES) as u32;
        let mut tris = Vec::with_capacity(2 * PALM_SIDES * PALM_RINGS);
        for i in 0..PALM_RINGS - 1 {
            for k in 0..PALM_SIDES {
                tris.push([at(i, k), at(i, k + 1), at(i + 1, k)]);
                tris.push([at(i, k + 1), at(i + 1, k + 1), at(i + 1, k)]);
            }
        }
        for k in 0..PALM_SIDES {
            tris.push([heel, at(0, k + 1), at(0, k)]);
            tris.push([tip, at(PALM_RINGS - 1, k), at(PALM_RINGS - 1, k + 1)]);
        }
        // Outward: away from the palm's centre line.
        for t in &mut tris {
            let [a, b, c] = t.map(|i| verts[i as usize]);
            let centre = (a + b + c) / 3.0;
            let axis = Vec3::new(0.0, centre.y.clamp(PALM_END_Y, PALM_HEEL_Y), 0.0);
            if (b - a).cross(c - a).dot(centre - axis) < 0.0 {
                t.swap(1, 2);
            }
        }
        (verts, tris)
    }

    /// A finger's or the thumb's centre line in the hand's frame, from its
    /// knuckle to its tip, with the index (in the line's points) where the
    /// base joint ends (the glove's edge of a fingerless mitt).
    #[derive(Debug, Clone, PartialEq)]
    pub struct Digit {
        pub points: Vec<Vec3>,
        pub base_end: usize,
    }

    /// Points per joint along a finger's line.
    const PER_JOINT: [usize; 3] = [3, 2, 2];

    /// A line `len` long through the 2D path `at(t)` (t in 0 … `len`),
    /// sampled per joint; `z` across.
    fn sampled(lens: [f32; 3], z: f32, at: impl Fn(f32) -> (f32, f32), n: f32) -> Digit {
        let mut points = Vec::new();
        let mut s = 0.0;
        let mut base_end = 0;
        let (u, v) = at(0.0);
        points.push(Vec3::new(n * u, v, z));
        for (j, len) in lens.iter().enumerate() {
            for k in 1..=PER_JOINT[j] {
                let (u, v) = at(s + len * k as f32 / PER_JOINT[j] as f32);
                points.push(Vec3::new(n * u, v, z));
            }
            s += len;
            if j == 0 {
                base_end = points.len() - 1;
            }
        }
        Digit { points, base_end }
    }

    /// The four fingers of `shape` on hand `side` (index first).
    pub fn fingers(shape: HandShape, side: usize) -> Vec<Digit> {
        let n = palm_sign(side);
        // In the hand's frame across the palm: u towards the bar (n·x), v
        // = y.
        let knuckle = (PALM_HALF[0] * 0.35, KNUCKLE_Y + 0.002);
        FINGERS
            .iter()
            .enumerate()
            .map(|(i, (z, lens))| {
                let spread = (i as f32 - 1.5) * 0.004;
                match shape.wrap_radius() {
                    Some(r) => {
                        // Round the bar at its radius plus a finger's.
                        let (bu, bv) = (PALM_HALF[0] + r, BAR_Y);
                        let rf = r + FINGER_RADIUS + 0.0008;
                        let phi0 = (knuckle.1 - bv).atan2(knuckle.0 - bu);
                        // Over a hood's (or a bar end's) front and the
                        // phone's edge only partly: then straight on.
                        let limit = match shape {
                            HandShape::Hoods => 1.35,
                            HandShape::Phone => 1.6,
                            _ => f32::MAX,
                        };
                        sampled(
                            *lens,
                            *z,
                            |t| {
                                let a = (t / rf).min(limit);
                                let phi = phi0 + a;
                                let rest = t - a * rf;
                                (
                                    bu + rf * phi.cos() - rest * phi.sin(),
                                    bv + rf * phi.sin() + rest * phi.cos(),
                                )
                            },
                            n,
                        )
                    }
                    None if shape == HandShape::Open => sampled(
                        *lens,
                        *z + spread,
                        |t| (knuckle.0 + 0.04 * t, knuckle.1 - t),
                        n,
                    ),
                    None => {
                        // A relaxed hand (bug 2026-11-17: the standing
                        // rider's hands looked loose): every joint bent
                        // a little, the middle joint most, the little
                        // finger curled most, as a hand hangs at rest.
                        let k = i as f32;
                        let bend = RELAXED_BEND;
                        let angles = [bend[0] + 0.04 * k, bend[1] + 0.05 * k, bend[2] + 0.02 * k];
                        sampled(
                            *lens,
                            *z,
                            |t| {
                                let (mut u, mut v) = knuckle;
                                let mut a = 0.0;
                                let mut rest = t;
                                for (j, (bent, len)) in angles.iter().zip(lens).enumerate() {
                                    a += bent;
                                    let (su, cv) = a.sin_cos();
                                    if rest <= *len || j == 2 {
                                        return (u + su * rest, v - cv * rest);
                                    }
                                    u += su * len;
                                    v -= cv * len;
                                    rest -= len;
                                }
                                (u, v)
                            },
                            n,
                        )
                    }
                }
            })
            .collect()
    }

    /// The thumb of `shape` on hand `side`: from the palm's front edge.
    /// Round a bar it wraps it against the fingers (bug 2026-11-17, the
    /// hands upside down): behind and under the tops and the flat grips,
    /// over the drops and the TT bars. Over a hood it hooks down the
    /// hood's inner side, forward.
    pub fn thumb(shape: HandShape, side: usize) -> Digit {
        let n = palm_sign(side);
        let base = Vec3::new(n * 0.006, -0.028, -0.034);
        if shape == HandShape::Hoods {
            let at = |p: [f32; 3]| Vec3::new(n * p[0], p[1], p[2]);
            return Digit {
                points: vec![
                    base,
                    at(HOOD_THUMB[0]),
                    at(HOOD_THUMB[1]),
                    at(HOOD_THUMB[2]),
                ],
                base_end: 2,
            };
        }
        match bar_centre(shape, side).filter(|_| shape != HandShape::Phone) {
            Some(b) => {
                let r = shape.wrap_radius().unwrap_or(WRAP_RADIUS);
                let rf = r + FINGER_RADIUS + 0.0008;
                let on = |z: f32, phi: f32| b + Vec3::new(n * rf * phi.cos(), rf * phi.sin(), z);
                let [a, m, e] = WRAP_THUMB_PHI;
                Digit {
                    points: vec![base, on(-0.03, a), on(-0.026, m), on(-0.024, e)],
                    base_end: 2,
                }
            }
            None if shape == HandShape::Relaxed => Digit {
                // Resting along the index finger's side.
                points: vec![
                    base,
                    Vec3::new(n * 0.017, -0.052, -0.047),
                    Vec3::new(n * 0.026, -0.075, -0.05),
                    Vec3::new(n * 0.029, -0.104, -0.043),
                ],
                base_end: 2,
            },
            None => {
                let curl = match shape {
                    HandShape::Open => 0.0,
                    _ => 1.0,
                };
                Digit {
                    points: vec![
                        base,
                        Vec3::new(n * (0.016 + 0.004 * curl), -0.05, -0.05),
                        Vec3::new(n * (0.022 + 0.008 * curl), -0.066, -0.058),
                        Vec3::new(n * (0.024 + 0.012 * curl), -0.08, -0.06),
                    ],
                    base_end: 2,
                }
            }
        }
    }
}

/// The face on the head (2026-11-17: "give the rider a mouth … and ears,
/// eyes etc."; spec §14 Phase 4, 4N *Face*): eyes in their sockets under
/// lids, eyebrows (in the hair's colour, greying with it), ears, the
/// lips (paint slot `lips`), a chin; always shown, near model only (the
/// far head stays plain).
pub const FACE: &str = "face_features";

/// Where the face's parts sit (rest-pose model coordinates, from the
/// skull's centre, [`skull_centre`]), for the model and its tests.
pub mod face {
    /// The eyes' centres (x either side), under the round glasses' lenses.
    pub const EYE: [f32; 3] = [0.031, 0.013, -0.0815];
    pub const EYE_RADII: [f32; 3] = [0.0105, 0.006, 0.0035];
    /// The eyebrows, above the round glasses' rims.
    pub const BROW: [f32; 3] = [0.031, 0.0375, -0.081];
    /// The ears' centres, behind the glasses' temples, under the helmet.
    pub const EAR: [f32; 3] = [0.079, -0.011, -0.012];
    pub const EAR_RADII: [f32; 3] = [0.0065, 0.016, 0.009];
    /// The lips' centre: where the status sign's X is (`wear/mark-mouth-cross`).
    pub const MOUTH: [f32; 3] = [0.0, -0.051, -0.0835];
    pub const MOUTH_RADII: [f32; 3] = [0.019, 0.0048, 0.0055];
}

/// The hip plates (spec §14 Phase 8 T6): (|x| of the outer face, its
/// half thickness, centre (y, z), half size (y, z)). Outside the widest
/// thigh (a skinsuit's) by a few millimetres: the thighs turn about the x
/// axis, so they never cut through.
pub const HIP_PLATE: (f32, f32, [f32; 2], [f32; 2]) = (0.178, 0.0015, [-0.07, 0.0], [0.03, 0.05]);

/// A kit variant of the rider model: a catalogue item's `model` (spec 4C,
/// `assets/parts.toml`, slot = a kit slot), and the nodes it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub id: &'static str,
    pub slot: &'static str,
    pub nodes: &'static [&'static str],
}

const fn var(id: &'static str, slot: &'static str, nodes: &'static [&'static str]) -> Variant {
    Variant { id, slot, nodes }
}

/// Every kit variant of the rider generator.
pub const VARIANTS: &[Variant] = &[
    var("helmet-light", "helmet", &[opt::HELMET_LIGHT]),
    var("helmet-aero-road", "helmet", &[opt::HELMET_AERO_ROAD]),
    var("helmet-tt", "helmet", &[HELMET_AERO]),
    var("cap-cotton", "cap", &[CAP]),
    var(
        "jersey-long-sleeve",
        "jersey",
        &[
            opt::SLEEVE_UPPER[0],
            opt::SLEEVE_UPPER[1],
            opt::SLEEVE_LONG[0],
            opt::SLEEVE_LONG[1],
        ],
    ),
    var(
        "jersey-aero",
        "jersey",
        &[opt::SLEEVE_UPPER[0], opt::SLEEVE_UPPER[1]],
    ),
    var(
        "skinsuit",
        "jersey",
        &[
            opt::SLEEVE_UPPER[0],
            opt::SLEEVE_UPPER[1],
            opt::SUIT_PELVIS,
            opt::SUIT_THIGH[0],
            opt::SUIT_THIGH[1],
        ],
    ),
    var(
        "shorts-tights",
        "shorts",
        &[
            opt::TIGHTS_THIGH[0],
            opt::TIGHTS_THIGH[1],
            opt::TIGHTS_SHIN[0],
            opt::TIGHTS_SHIN[1],
        ],
    ),
    var(
        "shorts-aero",
        "shorts",
        &[opt::SHORTS_LONG[0], opt::SHORTS_LONG[1]],
    ),
    var(
        "socks-tall",
        "socks",
        &[opt::SOCK_TALL[0], opt::SOCK_TALL[1]],
    ),
    var(
        "socks-aero",
        "socks",
        &[opt::SOCK_AERO[0], opt::SOCK_AERO[1]],
    ),
    var(
        "shoes-race",
        "shoes",
        &[opt::SHOE_RACE[0], opt::SHOE_RACE[1]],
    ),
    var(
        "shoes-overshoes",
        "shoes",
        &[opt::OVERSHOE[0], opt::OVERSHOE[1]],
    ),
    var(
        "gloves-gel",
        "gloves",
        &[opt::GLOVE_GEL[0], opt::GLOVE_GEL[1]],
    ),
    var(
        "gloves-full",
        "gloves",
        &[opt::GLOVE_FULL[0], opt::GLOVE_FULL[1]],
    ),
    var(
        "gloves-winter",
        "gloves",
        &[opt::GLOVE_WINTER[0], opt::GLOVE_WINTER[1]],
    ),
];

/// The kit variant `id`.
pub fn variant(id: &str) -> Option<&'static Variant> {
    VARIANTS.iter().find(|v| v.id == id)
}

/// The nodes of a kit slot's base item (the free base kit), replaced by a
/// variant of that slot; slots without (jersey, shorts, socks, cap) show
/// their variants over the base model.
pub fn base_nodes(slot: &str) -> &'static [&'static str] {
    match slot {
        "helmet" => &[HELMET_ROAD],
        "shoes" => &opt::SHOE_ROAD,
        "gloves" => &opt::GLOVE_MITT,
        _ => &[],
    }
}

/// The kit slots of the variants.
pub const KIT_SLOTS: [&str; 7] = [
    "jersey", "shorts", "socks", "shoes", "gloves", "helmet", "cap",
];

/// Every optional node: shown only when the look chooses it.
pub fn optional() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = VARIANTS
        .iter()
        .flat_map(|v| v.nodes.iter().copied())
        .chain(KIT_SLOTS.iter().flat_map(|s| base_nodes(s).iter().copied()))
        .chain([
            KIT_BAND,
            KIT_SASH,
            POCKETS,
            POCKET_BAND,
            opt::HAIR_CAP,
            opt::HAIR_BUZZ,
            opt::HAIR_LONG,
            opt::HAIR_TAIL,
            opt::HAIR_BUN,
            opt::GLASSES_SPORT,
            opt::GLASSES_ROUND,
            opt::FACE_JAW,
        ])
        .chain(HAIR_EXTRA)
        .chain(opt::MOUSTACHE)
        .chain(opt::BEARD)
        .chain(opt::SHOULDERS)
        .chain(opt::CHEST)
        .chain(opt::FACE_LINES)
        .chain(opt::HIP_NUMBER)
        .chain(HAIR_BARE.iter().map(|(_, b, _)| *b))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The helmet taken off (spec §14 Phase 4, 4N Q28, decided 2026-11-17:
/// "show without helmet. hang helmet on a hook on the wall"): a pseudo
/// kit id; with it in a look's kit [`visible`] shows no helmet or cap and
/// the bare-head hair ([`HAIR_BARE`]).
pub const HELMET_OFF: &str = "helmet-off";

/// Every hair piece and its bare-head shape (not pressed under a helmet:
/// fuller, as the hair falls without one).
pub const HAIR_BARE: [(&str, &str, f32); 19] = [
    (opt::HAIR_CAP, "hair_cap_bare", 1.08),
    (opt::HAIR_BUZZ, "hair_buzz_bare", 1.02),
    (opt::HAIR_LONG, "hair_long_bare", 1.05),
    (opt::HAIR_TAIL, "hair_tail_bare", 1.03),
    (opt::HAIR_BUN, "hair_bun_bare", 1.04),
    (opt::HAIR_SHAVED, "hair_shaved_bare", 1.01),
    (opt::HAIR_FRINGE, "hair_fringe_bare", 1.07),
    (opt::HAIR_TEXTURED, "hair_textured_bare", 1.13),
    (opt::HAIR_SIDE_PART, "hair_side_part_bare", 1.09),
    (opt::HAIR_CURLS, "hair_curls_bare", 1.16),
    (opt::HAIR_AFRO, "hair_afro_bare", 1.38),
    (opt::HAIR_MID, "hair_mid_bare", 1.08),
    (opt::HAIR_LONG_CURLY, "hair_long_curly_bare", 1.11),
    (opt::HAIR_TAIL_LOW, "hair_tail_low_bare", 1.03),
    (opt::HAIR_BRAID, "hair_braid_bare", 1.03),
    (opt::HAIR_BRAIDS, "hair_braids_bare", 1.03),
    (opt::HAIR_BUN_LOW, "hair_bun_low_bare", 1.04),
    (opt::HAIR_LOCS, "hair_locs_bare", 1.07),
    (opt::HAIR_ROWS, "hair_cornrows_bare", 1.02),
];

/// The bare-head shape of hair piece `piece`.
pub fn bare_of(piece: &str) -> Option<&'static str> {
    HAIR_BARE
        .iter()
        .find(|(p, _, _)| *p == piece)
        .map(|(_, b, _)| *b)
}

/// The nodes of hair style `i` without a helmet.
pub fn hair_nodes_bare(i: u8) -> Vec<&'static str> {
    hair_nodes(i).iter().filter_map(|n| bare_of(n)).collect()
}

/// The hair nodes of the 4N catalogue.
const HAIR_EXTRA: [&str; 14] = [
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

/// The nodes of hair style `i` (`ocw_core::character::HAIR_STYLES`).
pub fn hair_nodes(i: u8) -> &'static [&'static str] {
    use opt::*;
    match i {
        0 => &[HAIR_CAP],
        1 => &[HAIR_BUZZ],
        2 => &[HAIR_CAP, HAIR_LONG],
        3 => &[HAIR_CAP, HAIR_TAIL],
        4 => &[HAIR_CAP, HAIR_BUN],
        5 => &[],
        6 => &[HAIR_SHAVED],
        7 => &[HAIR_BUZZ, HAIR_FRINGE],
        8 => &[HAIR_CAP, HAIR_TEXTURED],
        9 => &[HAIR_CAP, HAIR_SIDE_PART],
        10 => &[HAIR_CAP, HAIR_CURLS],
        11 => &[HAIR_CAP, HAIR_AFRO],
        12 => &[HAIR_CAP, HAIR_FRINGE, HAIR_TEXTURED],
        13 => &[HAIR_CAP, HAIR_MID],
        14 => &[HAIR_CAP, HAIR_LONG_CURLY],
        15 => &[HAIR_CAP, HAIR_TAIL_LOW],
        16 => &[HAIR_CAP, HAIR_BRAID],
        17 => &[HAIR_CAP, HAIR_BRAIDS],
        18 => &[HAIR_CAP, HAIR_BUN_LOW],
        19 => &[HAIR_CAP, HAIR_LOCS],
        20 => &[HAIR_BUZZ, HAIR_ROWS],
        _ => &[HAIR_CAP],
    }
}

/// The optional nodes shown for a look: the kit `variants` (rider variant
/// ids of the worn items; slots without one wear the base item) and the
/// character's hair, face and glasses (a TT helmet's visor replaces the
/// glasses, and no cap is worn under it: its brim would cut through the
/// visor, spec §2.6 v3.40). The kit patterns are drawn by the renderer's gear shader
/// (spec §14 Phase 8 T6, `ocw_core::kit`): the chest band and sash nodes
/// of W4 stay in the model for older apps but are never shown.
pub fn visible(character: &ocw_core::Character, variants: &[&str]) -> Vec<&'static str> {
    let c = character.validated();
    let bare = variants.contains(&HELMET_OFF);
    let mut out: Vec<&'static str> = Vec::new();
    for slot in KIT_SLOTS {
        // The helmet off (Q28): no helmet, no cap.
        if bare && (slot == "helmet" || slot == "cap") {
            continue;
        }
        match variants
            .iter()
            .filter_map(|id| variant(id))
            .find(|v| v.slot == slot)
        {
            Some(v) => out.extend(v.nodes),
            None => out.extend(base_nodes(slot)),
        }
    }
    // The back pockets on every jersey but the skinsuit (its band in the
    // pattern colour is the renderer's choice: the jersey designer's).
    if !out.contains(&opt::SUIT_PELVIS) {
        out.push(POCKETS);
    }
    if bare {
        out.extend(hair_nodes_bare(c.hair));
    } else {
        out.extend(hair_nodes(c.hair));
    }
    // Facial hair (4N N6) and the age's face (N3).
    if c.moustache > 0 {
        out.extend(opt::MOUSTACHE.get(c.moustache as usize - 1));
    }
    if c.beard > 0 {
        out.extend(opt::BEARD.get(c.beard as usize - 1));
    }
    match c.age.face_lines() {
        0 => {}
        1 => out.push(opt::FACE_LINES[0]),
        _ => out.push(opt::FACE_LINES[1]),
    }
    if c.age.soft_jaw() {
        out.push(opt::FACE_JAW);
    }
    // The base body's shoulders and chest (N2).
    use ocw_core::BaseBody;
    out.push(match c.base {
        BaseBody::A => opt::SHOULDERS[0],
        BaseBody::B => opt::SHOULDERS[1],
        BaseBody::C => opt::SHOULDERS[2],
    });
    match c.base {
        BaseBody::A => {}
        BaseBody::B => out.push(opt::CHEST[0]),
        BaseBody::C => out.push(opt::CHEST[1]),
    }
    if out.contains(&HELMET_AERO) {
        out.retain(|n| *n != CAP);
    } else {
        out.extend(match c.glasses {
            1 => Some(opt::GLASSES_SPORT),
            2 => Some(opt::GLASSES_ROUND),
            _ => None,
        });
    }
    out
}

/// The look of the shipped model and of the far LOD: the base kit and
/// the default character.
pub fn base_visible() -> Vec<&'static str> {
    visible(&ocw_core::Character::default(), &[])
}

/// The body region of node `i` (spec §14 Phase 8 T6: where a garment's
/// pattern and race number go): from the nearest rig node at or above it.
pub fn body_region(m: &Model, i: usize) -> ocw_core::kit::BodyRegion {
    use ocw_core::kit::BodyRegion;
    let mut at = Some(i);
    while let Some(n) = at {
        let name = m.nodes[n].name.as_str();
        let region = if name == PELVIS || name == TORSO {
            Some(BodyRegion::Torso)
        } else if name == HEAD {
            Some(BodyRegion::Head)
        } else if UPPER_ARM.contains(&name) || FOREARM.contains(&name) {
            Some(BodyRegion::Arm)
        } else if HAND.contains(&name) {
            Some(BodyRegion::Hand)
        } else if THIGH.contains(&name) || SHIN.contains(&name) {
            Some(BodyRegion::Leg)
        } else if FOOT.contains(&name) {
            Some(BodyRegion::Foot)
        } else {
            None
        };
        if let Some(r) = region {
            return r;
        }
        at = m.nodes[n].parent;
    }
    BodyRegion::Torso
}

/// The rig nodes whose meshes (and kit over them) get the body build's
/// girth: the torso and the limbs, not the head, hands and feet.
pub const GIRTH_NODES: [&str; 10] = [
    PELVIS,
    TORSO,
    THIGH[0],
    THIGH[1],
    SHIN[0],
    SHIN[1],
    UPPER_ARM[0],
    UPPER_ARM[1],
    FOREARM[0],
    FOREARM[1],
];

/// The nearest rig node at or above node `i`.
fn rig_of(m: &Model, i: usize) -> Option<usize> {
    let mut at = Some(i);
    while let Some(n) = at {
        if REQUIRED.contains(&m.nodes[n].name.as_str()) {
            return Some(n);
        }
        at = m.nodes[n].parent;
    }
    None
}

/// The girth of rig node `name` for the character `c` (spec 4C, 4N):
/// x and z of its meshes (the limbs hang along −y at rest). The shape's
/// torso girth on the torso and pelvis (the same on both: the jersey
/// over the bib), wider for body B's hips, deeper with the age; the
/// shape's limb girth on the limbs, the base body's on the thighs (hips)
/// and upper arms (shoulders); 1 on the head, hands and feet.
pub fn girth_of(name: &str, c: &ocw_core::Character) -> Vec3 {
    use ocw_core::BaseBody;
    let c = c.validated();
    let (torso, limbs) = c.shape.girth();
    let (middle, muscle) = c.age.body();
    // Body A is today's model (W4); B has wider hips and narrower arms,
    // C is in between.
    let hips = match c.base {
        BaseBody::A => 1.0,
        BaseBody::B => 1.03,
        BaseBody::C => 1.015,
    };
    let (thigh, arm) = match c.base {
        BaseBody::A => (1.0, 1.0),
        BaseBody::B => (1.07, 0.94),
        BaseBody::C => (1.035, 0.97),
    };
    let l = limbs * muscle;
    if name == PELVIS || name == TORSO {
        Vec3::new(torso * hips, 1.0, torso * middle)
    } else if THIGH.contains(&name) {
        Vec3::new(l * thigh, 1.0, l * thigh)
    } else if UPPER_ARM.contains(&name) {
        Vec3::new(l * arm, 1.0, l * arm)
    } else if GIRTH_NODES.contains(&name) {
        Vec3::new(l, 1.0, l)
    } else {
        Vec3::ONE
    }
}

/// The scale of node `i`'s meshes for the character `c` (spec 4C, 4N;
/// [`girth_of`] its rig node); about [`girth_centre`].
pub fn girth(m: &Model, i: usize, c: &ocw_core::Character) -> Vec3 {
    rig_of(m, i).map_or(Vec3::ONE, |r| girth_of(&m.nodes[r].name, c))
}

/// The point node `i`'s meshes are scaled about (node `i`'s frame): the
/// torso's pivot for the torso and pelvis and what they carry, so the
/// bib on the pelvis and the jersey on the torso grow together (the
/// layers' fit, `crate::clothing`); the joint of a limb.
pub fn girth_centre(m: &Model, i: usize) -> Vec3 {
    let Some(r) = rig_of(m, i) else {
        return Vec3::ZERO;
    };
    let name = m.nodes[r].name.as_str();
    let at = if name == PELVIS || name == TORSO {
        // The pivot in the pelvis's frame.
        let pelvis = if name == PELVIS {
            r
        } else {
            m.nodes[r].parent.unwrap_or(r)
        };
        m.world(pelvis).transform_point3(v(body::TORSO_PIVOT))
    } else {
        m.world(r).transform_point3(Vec3::ZERO)
    };
    m.world(i).inverse().transform_point3(at)
}

/// The node translation of node `i` for the character `c` (its rest
/// translation otherwise): the upper arms sit on the torso's shoulders,
/// which the torso's girth moves in or out (spec §14 Phase 4, 4N: no
/// arms standing off a thin body or sunk into a heavy one). The poses
/// only turn the joints, so the arms keep it.
pub fn node_translation(m: &Model, i: usize, c: &ocw_core::Character) -> Vec3 {
    let node = &m.nodes[i];
    if !UPPER_ARM.contains(&node.name.as_str()) {
        return node.translation;
    }
    let g = girth_of(TORSO, c);
    Vec3::new(
        node.translation.x * g.x,
        node.translation.y,
        node.translation.z,
    )
}

/// The mesh transform of node `i` for the character `c`: its girth
/// about [`girth_centre`] as (scale, translation) in node `i`'s frame.
pub fn girth_transform(m: &Model, i: usize, c: &ocw_core::Character) -> (Vec3, Vec3) {
    let s = girth(m, i, c);
    if s == Vec3::ONE {
        return (s, Vec3::ZERO);
    }
    let centre = girth_centre(m, i);
    (s, centre - s * centre)
}

/// Adds node `name` under `parent` (at the parent's origin) with the
/// shapes `f` builds (rest-pose model coordinates).
fn opt_node(m: &mut Model, name: &str, parent: usize, f: impl FnOnce(&mut Parts)) -> usize {
    let i = m.add(Node::new(name, Some(parent), Vec3::ZERO));
    let mut p = Parts::new();
    f(&mut p);
    p.attach(m, i);
    i
}

/// The centre of the skull (rest-pose model coordinates): what the head
/// wear is built round.
pub fn skull_centre() -> Vec3 {
    v(body::TORSO_PIVOT) + Vec3::Y * (body::TORSO + 0.15) + Vec3::new(0.0, 0.0, -0.015)
}

/// The jersey's torso shell (spec §2.6): rings from the hem to the
/// shoulders as (height above the torso's pivot, half width, half depth,
/// z offset from the pivot, the ring's tilt), m. The hem hangs over the
/// hips (bug 2026-11-16: the old hem was narrower than the bib under it)
/// and is cut higher at the front than at the back (a dropped tail: the
/// tilt, the hem's back this much lower and its front this much higher
/// than its centre), so the bib stays under the jersey when the rider
/// folds forward (`crate::clothing` checks it).
pub const JERSEY_SHELL: [[f32; 5]; 5] = [
    [-0.055, 0.16, 0.106, -0.008, 0.02],
    [0.03, 0.162, 0.106, -0.007, 0.0],
    [0.14, 0.16, 0.1, -0.005, 0.0],
    [0.3, 0.18, 0.115, 0.005, 0.0],
    [0.42, 0.19, 0.105, 0.01, 0.0],
];

/// [`JERSEY_SHELL`] as loft rings (rest-pose model coordinates), each
/// pushed `off` outwards (0: the jersey itself).
pub fn jersey_rings_out(off: f32) -> Vec<Ring> {
    let pivot = v(body::TORSO_PIVOT);
    JERSEY_SHELL
        .iter()
        .map(|[y, hw, hd, z, tilt]| Ring {
            centre: Vec3::new(0.0, pivot.y + y, pivot.z + z),
            u: Vec3::X * (hw + off),
            v: Vec3::new(0.0, *tilt, -(hd + off)),
        })
        .collect()
}

/// [`JERSEY_SHELL`] as loft rings (rest-pose model coordinates).
pub fn jersey_rings() -> Vec<Ring> {
    jersey_rings_out(0.0)
}

/// The sides of the jersey's shell (the far one ten, not eight: the
/// bib's six sides need the rounder outline inside it).
pub fn jersey_sides(detail: Detail) -> u32 {
    match detail {
        Detail::High => detail.sides() + 4,
        Detail::Low => detail.sides() + 6,
    }
}

/// Corner `k` (of `sides`, at angle `k / sides · 2π`: 0 the rider's
/// right, a quarter turn the front, three quarters the back) of the
/// jersey's faceted shell pushed `off` outwards, at height `y`: on the
/// straight edge between the rings the loft draws, continued straight
/// below the hem. `None` above the shoulders.
pub fn shell_corner(sides: u32, k: u32, y: f32, off: f32) -> Option<Vec3> {
    let rings = jersey_rings_out(off);
    let a = k as f32 / sides as f32 * std::f32::consts::TAU;
    let pts: Vec<Vec3> = rings
        .iter()
        .map(|r| r.centre + r.u * a.cos() + r.v * a.sin())
        .collect();
    if y > pts[pts.len() - 1].y {
        return None;
    }
    let i = pts
        .windows(2)
        .position(|w| y <= w[1].y)
        .unwrap_or(pts.len() - 2);
    let (p, q) = (pts[i], pts[i + 1]);
    Some(p + (q - p) * ((y - p.y) / (q.y - p.y)))
}

/// Whether `p` (rest-pose model coordinates, the torso's frame) is above
/// the jersey's hem, and how far (m, negative below; measured
/// vertically).
pub fn above_hem(p: Vec3) -> f32 {
    let hem = jersey_rings()[0];
    let n = hem.u.cross(hem.v).normalize();
    let n = if n.y < 0.0 { -n } else { n };
    (p - hem.centre).dot(n) / n.y
}

/// The node of the jersey's back pockets (on the torso; spec §2.6): an
/// optional node, shown with every jersey but the skinsuit.
pub const POCKETS: &str = "jersey_pockets";
/// The pockets' top band in the kit pattern's colour, shown when the
/// jersey designer has a second colour (spec §14 Phase 8 T6).
pub const POCKET_BAND: &str = "jersey_pockets_band";
/// The anchors at the openings of the left, centre and right pocket
/// (model contract, spec §14 Phase 15, 15B): children of the torso, the
/// origin in the opening, +z out of the back, +y up along it. The phone
/// of the status signs is taken from `anchor_pocket_c` (spec §2.6).
pub const POCKET_ANCHORS: [&str; 3] = ["anchor_pocket_l", "anchor_pocket_c", "anchor_pocket_r"];

/// The back pockets' measures (spec §2.6, reference: three open pockets
/// side by side across the lower back, the opening 12–15 cm above the
/// hem, an elastic band at the top, the same fabric as the jersey).
pub mod pocket {
    /// The panel's bottom above the hem where it is highest under the
    /// panel (its sides), the opening (its top) above the hem's lowest
    /// point (the middle of the back), m: the pockets sit level.
    pub const BOTTOM: f32 = 0.012;
    pub const TOP: f32 = 0.14;
    /// The elastic band at the top (m).
    pub const BAND: f32 = 0.018;
    /// The panel reaches this far round the body each side of the
    /// spine (rad; ~55°: most of the back's width).
    pub const HALF_ANGLE: f32 = 0.96;
    /// The panel's distance outside the jersey's shell: at the seams
    /// between the pockets (a crease), on a pocket, at its bulge (half way
    /// down: something inside), on the band and its top lip (m).
    pub const SEAM: f32 = 0.0026;
    pub const FLAT: f32 = 0.007;
    pub const BULGE: f32 = 0.013;
    pub const BAND_OUT: f32 = 0.009;
    pub const LIP: f32 = 0.0026;
}

/// The point of the jersey's faceted shell (the loft of [`jersey_rings`]
/// with `sides` corners) at height `y` and angle `theta` (0 = the
/// rider's right, 3π/2 = the middle of the back), pushed `off` outwards:
/// on the shell's facets, so a surface through such points runs parallel
/// to the jersey.
pub fn shell_point(sides: u32, y: f32, theta: f32, off: f32) -> Vec3 {
    let step = std::f32::consts::TAU / sides as f32;
    let th = theta.rem_euclid(std::f32::consts::TAU);
    let k = (th / step).floor();
    let f = th / step - k;
    let corner = |k: u32| shell_corner(sides, k % sides, y, off).expect("below the shoulders");
    corner(k as u32).lerp(corner(k as u32 + 1), f)
}

/// The back pockets' panel on the torso and the top band in the
/// pattern colour (rest-pose model coordinates), and the anchors at the
/// three openings: (position, outward direction).
fn pockets(detail: Detail) -> (Parts, Parts, [(Vec3, Vec3); 3]) {
    use pocket::*;
    use std::f32::consts::PI;
    let sides = jersey_sides(detail);
    // The pockets sit level across the back: their bottom above the hem
    // where it is highest under them (at the panel's sides: the hem drops
    // to the back), the opening measured from the hem's lowest point.
    let tilt = JERSEY_SHELL[0][4];
    let centre = jersey_rings()[0].centre.y;
    let hem = centre - tilt;
    let side_hem = centre - tilt * HALF_ANGLE.cos();
    let back = 1.5 * PI;
    // Columns: per pocket its left seam, its middle; the last seam.
    let cols: Vec<(f32, bool)> = (0..=6)
        .map(|c| {
            let theta = back - HALF_ANGLE + 2.0 * HALF_ANGLE * c as f32 / 6.0;
            (theta, c % 2 == 0)
        })
        .collect();
    // Rows: bottom, the bulge, the band's lower edge, the top, the lip.
    let rows = [
        (side_hem + BOTTOM, 0),
        (0.5 * (side_hem + BOTTOM + hem + TOP - BAND), 1),
        (hem + TOP - BAND, 2),
        (hem + TOP, 3),
        (hem + TOP + 0.004, 4),
    ];
    let off = |row: usize, seam: bool| match (row, seam) {
        (4, _) => LIP,
        (2 | 3, _) => BAND_OUT,
        (_, true) => SEAM,
        (1, false) => BULGE,
        _ => FLAT,
    };
    let grid = |rows: &[(f32, usize)], extra: f32| -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let w = cols.len();
        let verts: Vec<Vec3> = rows
            .iter()
            .flat_map(|&(y, r)| {
                cols.iter()
                    .map(move |&(th, seam)| shell_point(sides, y, th, off(r, seam) + extra))
            })
            .collect();
        let mut tris = Vec::new();
        for r in 0..rows.len() - 1 {
            for c in 0..w - 1 {
                let a = (r * w + c) as u32;
                let (b, d, e) = (a + 1, a + w as u32, a + w as u32 + 1);
                // Columns run from the rider's left to the right (−x … +x,
                // left to right seen from behind), rows upwards: outward
                // faces.
                tris.push([a, b, d]);
                tris.push([b, e, d]);
            }
        }
        (verts, tris)
    };
    // The panel with its seams and bulges, the band and its lip: flat
    // shaded, so the seams read as creases.
    let mut panel = Parts::new();
    let (v, t) = grid(&rows, 0.0);
    panel.slot("jersey").add_flat(&v, &t);
    // The band's accent copy just outside it.
    let mut band = Parts::new();
    let (v, t) = grid(&rows[2..4], 0.0012);
    band.slot("accent").add_flat(&v, &t);
    let anchors = [1usize, 3, 5].map(|c| {
        let th = cols[c].0;
        let y = hem + TOP - 0.01;
        let at = shell_point(sides, y, th, 0.5 * FLAT);
        let side = shell_point(sides, y, th + 0.01, 0.5 * FLAT) - at;
        (at, side.cross(Vec3::Y).normalize())
    });
    (panel, band, anchors)
}

/// The 4N hair catalogue, the moustaches and beards, and the age's face
/// (spec §14 Phase 4, 4N N3, N5, N6) on the head, round the skull's
/// centre `skull`: own work, generated (licence rule 8A). Every hair
/// shape stays inside each helmet's shell or leaves below its rim at the
/// back; the facial hair lies on the face under the straps
/// (`crate::helmet` checks both).
fn hair_and_face(m: &mut Model, head: usize, skull: Vec3, detail: Detail) {
    use std::f32::consts::PI;
    let hi = detail == Detail::High;
    let n = detail.sides();
    let bands = |h: u32, l: u32| if hi { h } else { l };
    // A point on the back of the head below the rim: angle `a` (0 the
    // back, ±π/2 the sides) at height `y` and radius scale `k`.
    let round = |a: f32, y: f32, k: f32| {
        skull + Vec3::new(0.079 * k * a.sin(), y, 0.012 + 0.087 * k * a.cos())
    };
    let bumps = |p: &mut Parts, at: &[Vec3], r: f32| {
        for c in at {
            p.slot("hair")
                .ellipsoid(*c, Vec3::splat(r), Quat::IDENTITY, 3, 4);
        }
    };
    opt_node(m, opt::HAIR_SHAVED, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, 0.004, 0.01),
            Vec3::new(0.0788, 0.0995, 0.0868),
            Quat::from_rotation_x(-0.2),
            bands(4, 3),
            n,
        )
    });
    opt_node(m, opt::HAIR_FRINGE, head, |p| {
        // A short fringe over the forehead, under the helmet's brow.
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, 0.03, -0.083),
            Vec3::new(0.05, 0.014, 0.014),
            Quat::from_rotation_x(0.35),
            bands(3, 3),
            n,
        )
    });
    opt_node(m, opt::HAIR_TEXTURED, head, |p| {
        let at: Vec<Vec3> = (0..4)
            .map(|k| {
                round(
                    (k as f32 - 1.5) * 0.6,
                    -0.035 - 0.006 * ((k % 2) as f32),
                    0.98,
                )
            })
            .collect();
        bumps(p, &at, 0.014);
    });
    opt_node(m, opt::HAIR_SIDE_PART, head, |p| {
        // The longer side swept over the temple.
        p.slot("hair").ellipsoid(
            skull + Vec3::new(-0.07, 0.0, -0.045),
            Vec3::new(0.014, 0.03, 0.034),
            Quat::from_rotation_z(0.25),
            bands(3, 3),
            n,
        )
    });
    opt_node(m, opt::HAIR_CURLS, head, |p| {
        let mut at: Vec<Vec3> = (0..5)
            .map(|k| round((k as f32 - 2.0) * 0.5, -0.05, 0.92))
            .collect();
        at.extend((0..3).map(|k| round((k as f32 - 1.0) * 0.7, -0.072, 0.9)));
        bumps(p, &at, 0.015);
    });
    opt_node(m, opt::HAIR_AFRO, head, |p| {
        // Pressed flat by the helmet: the volume below its rim.
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.062, 0.03),
            Vec3::new(0.086, 0.05, 0.088),
            Quat::IDENTITY,
            bands(4, 3),
            n,
        );
        let at: Vec<Vec3> = (0..4)
            .map(|k| round((k as f32 - 1.5) * 0.7, -0.085, 1.0))
            .collect();
        bumps(p, &at, 0.022);
    });
    opt_node(m, opt::HAIR_MID, head, |p| {
        // To the jaw at the sides and the back.
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.07, 0.03),
            Vec3::new(0.086, 0.075, 0.072),
            Quat::from_rotation_x(0.1),
            bands(4, 3),
            n,
        )
    });
    opt_node(m, opt::HAIR_LONG_CURLY, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.11, 0.064),
            Vec3::new(0.09, 0.13, 0.045),
            Quat::from_rotation_x(0.12),
            bands(4, 3),
            n,
        );
        let at: Vec<Vec3> = (0..3)
            .map(|k| {
                let a = (k as f32 - 1.0) * 0.7;
                let y = -0.08 - 0.035 * k as f32;
                skull + Vec3::new(0.075 * a.sin(), y, 0.075 + 0.025 * a.cos())
            })
            .collect();
        bumps(p, &at, 0.02);
    });
    opt_node(m, opt::HAIR_TAIL_LOW, head, |p| {
        p.slot("hair").sweep(
            &[
                skull + Vec3::new(0.0, -0.075, 0.088),
                skull + Vec3::new(0.0, -0.12, 0.112),
                skull + Vec3::new(0.0, -0.215, 0.118),
            ],
            &[0.022, 0.022, 0.009],
            bands(6, 4),
            true,
        )
    });
    // A braid: a chain of beads, narrowing.
    let braid = |p: &mut Parts, top: Vec3, bottom: Vec3, beads: u32| {
        for k in 0..beads {
            let t = k as f32 / (beads - 1) as f32;
            p.slot("hair").ellipsoid(
                top + (bottom - top) * t,
                Vec3::new(0.017, 0.022, 0.015) * (1.0 - 0.35 * t),
                Quat::IDENTITY,
                3,
                4,
            );
        }
    };
    let beads = if hi { 4 } else { 3 };
    opt_node(m, opt::HAIR_BRAID, head, |p| {
        braid(
            p,
            skull + Vec3::new(0.0, -0.07, 0.095),
            skull + Vec3::new(0.0, -0.25, 0.11),
            beads,
        )
    });
    opt_node(m, opt::HAIR_BRAIDS, head, |p| {
        for s in [-1.0f32, 1.0] {
            braid(
                p,
                skull + Vec3::new(s * 0.055, -0.07, 0.07),
                skull + Vec3::new(s * 0.075, -0.24, 0.085),
                beads,
            );
        }
    });
    opt_node(m, opt::HAIR_BUN_LOW, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.075, 0.105),
            Vec3::new(0.04, 0.034, 0.034),
            Quat::IDENTITY,
            bands(4, 3),
            n,
        )
    });
    opt_node(m, opt::HAIR_LOCS, head, |p| {
        // Locs gathered at the nape and tied back.
        let tie = skull + Vec3::new(0.0, -0.065, 0.1);
        for k in 0..4 {
            let a = (k as f32 - 1.5) * 0.55;
            p.slot("hair")
                .tube(round(a, -0.04, 1.0), tie, 0.007, 0.009, 4, false);
        }
        for k in 0..3 {
            let x = (k as f32 - 1.0) * 0.014;
            p.slot("hair").tube(
                tie + Vec3::new(x, 0.0, 0.0),
                tie + Vec3::new(1.4 * x, -0.17, 0.02),
                0.008,
                0.006,
                4,
                true,
            );
        }
    });
    opt_node(m, opt::HAIR_ROWS, head, |p| {
        // Rows from the hairline to the nape, close on the scalp (over the
        // buzz cut, inside every helmet).
        for k in 0..2 {
            let x = (k as f32 - 0.5) * 0.034;
            let side = (1.0 - (x / 0.08).powi(2)).max(0.0).sqrt();
            let path: Vec<Vec3> = (0..7)
                .map(|j| {
                    let t = -0.35 * PI + j as f32 / 6.0 * 1.25 * PI;
                    let r = Vec3::new(0.0, 0.1, 0.088) * 1.012;
                    skull
                        + Vec3::new(0.0, 0.006, 0.012)
                        + Vec3::new(x, r.y * t.cos() * side, r.z * t.sin() * side)
                })
                .collect();
            let radii = vec![0.0045; path.len()];
            p.slot("hair").sweep(&path, &radii, 3, false);
        }
    });
    // Moustaches (4N N6), in the facial hair's colour.
    let lip = skull + Vec3::new(0.0, -0.038, -0.094);
    let tash = |p: &mut Parts, r: Vec3| {
        p.slot("facial_hair")
            .ellipsoid(lip, r, Quat::IDENTITY, 3, if hi { 6 } else { 4 })
    };
    opt_node(m, opt::MOUSTACHE[0], head, |p| {
        tash(p, Vec3::new(0.028, 0.008, 0.012))
    });
    opt_node(m, opt::MOUSTACHE[1], head, |p| {
        tash(p, Vec3::new(0.032, 0.011, 0.014))
    });
    opt_node(m, opt::MOUSTACHE[2], head, |p| {
        tash(p, Vec3::new(0.027, 0.0075, 0.012));
        for s in [-1.0f32, 1.0] {
            p.slot("facial_hair").ellipsoid(
                lip + Vec3::new(s * 0.033, 0.006, 0.008),
                Vec3::new(0.012, 0.004, 0.006),
                Quat::from_rotation_z(s * 0.8),
                3,
                4,
            );
        }
    });
    opt_node(m, opt::MOUSTACHE[3], head, |p| {
        tash(p, Vec3::new(0.03, 0.009, 0.012));
        for s in [-1.0f32, 1.0] {
            p.slot("facial_hair").ellipsoid(
                lip + Vec3::new(s * 0.027, -0.022, 0.008),
                Vec3::new(0.006, 0.022, 0.009),
                Quat::IDENTITY,
                3,
                4,
            );
        }
    });
    opt_node(m, opt::MOUSTACHE[4], head, |p| {
        tash(p, Vec3::new(0.025, 0.0035, 0.0105))
    });
    // Beards.
    let chin = |p: &mut Parts, c: Vec3, r: Vec3, tilt: f32| {
        p.slot("facial_hair").ellipsoid(
            skull + c,
            r,
            Quat::from_rotation_x(tilt),
            if hi { 4 } else { 3 },
            n,
        )
    };
    opt_node(m, opt::BEARD[0], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.056, -0.024),
            Vec3::new(0.066, 0.042, 0.064),
            0.15,
        )
    });
    opt_node(m, opt::BEARD[1], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.057, -0.029),
            Vec3::new(0.0665, 0.043, 0.066),
            0.15,
        )
    });
    opt_node(m, opt::BEARD[2], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.058, -0.038),
            Vec3::new(0.067, 0.043, 0.064),
            0.15,
        )
    });
    opt_node(m, opt::BEARD[3], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.058, -0.043),
            Vec3::new(0.066, 0.043, 0.062),
            0.15,
        )
    });
    opt_node(m, opt::BEARD[4], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.08, -0.08),
            Vec3::new(0.022, 0.02, 0.017),
            0.3,
        )
    });
    opt_node(m, opt::BEARD[5], head, |p| {
        // Along the jaw from ear to ear.
        let path: Vec<Vec3> = (0..7)
            .map(|k| {
                let a = (k as f32 / 6.0 - 0.5) * 2.6;
                skull
                    + Vec3::new(
                        0.068 * a.sin(),
                        -0.035 - 0.05 * a.cos(),
                        -0.083 * a.cos() + 0.012,
                    )
            })
            .collect();
        let radii = vec![0.0065; path.len()];
        p.slot("facial_hair")
            .sweep(&path, &radii, if hi { 4 } else { 3 }, true);
    });
    opt_node(m, opt::BEARD[6], head, |p| {
        chin(
            p,
            Vec3::new(0.0, -0.058, -0.043),
            Vec3::new(0.066, 0.043, 0.062),
            0.15,
        );
        chin(
            p,
            Vec3::new(0.0, -0.1, -0.072),
            Vec3::new(0.04, 0.035, 0.028),
            0.15,
        );
    });
    // The age (4N N3, Q12): lines at the eyes and mouth, deeper from the
    // 60s with the forehead's; a softer jaw (near model only).
    let lines = |p: &mut Parts, k: f32| {
        if !hi {
            return;
        }
        for s in [-1.0f32, 1.0] {
            for (j, tilt) in [-0.3f32, 0.3].iter().enumerate() {
                p.slot("skin_line").ellipsoid(
                    skull + Vec3::new(s * 0.066, 0.012 - 0.008 * (j as f32 - 0.5), -0.054),
                    Vec3::new(0.0022, 0.0022 * k, 0.012 * k),
                    Quat::from_rotation_y(s * 0.9) * Quat::from_rotation_x(*tilt * s),
                    3,
                    3,
                );
            }
            p.slot("skin_line").ellipsoid(
                skull + Vec3::new(s * 0.03, -0.042, -0.081),
                Vec3::new(0.0025 * k, 0.016, 0.0025),
                Quat::from_rotation_z(-s * 0.3),
                3,
                3,
            );
        }
    };
    opt_node(m, opt::FACE_LINES[0], head, |p| lines(p, 1.0));
    opt_node(m, opt::FACE_LINES[1], head, |p| {
        lines(p, 1.5);
        if hi {
            // A forehead line.
            p.slot("skin_line").ellipsoid(
                skull + Vec3::new(0.0, 0.05 + 0.006f32, -0.088 + 0.006f32 * 0.12),
                Vec3::new(0.03, 0.0022, 0.004),
                Quat::from_rotation_x(-0.5),
                3,
                4,
            );
        }
    });
    opt_node(m, opt::FACE_JAW, head, |p| {
        if hi {
            for s in [-1.0f32, 1.0] {
                p.slot("skin").ellipsoid(
                    skull + Vec3::new(s * 0.048, -0.074, -0.05),
                    Vec3::new(0.026, 0.02, 0.03),
                    Quat::IDENTITY,
                    3,
                    5,
                );
            }
        }
    });
}

/// The face's node [`FACE`] on the head (near model only).
fn opt_node_face(m: &mut Model, head: usize, skull: Vec3, detail: Detail) {
    use face::*;
    let i = m.add(Node::new(FACE, Some(head), Vec3::ZERO));
    if detail != Detail::High {
        return;
    }
    let mut p = Parts::new();
    let at = |c: [f32; 3], s: f32| skull + Vec3::new(s * c[0], c[1], c[2]);
    for s in [-1.0f32, 1.0] {
        // The eye, a dark almond in its socket, the lid over it.
        p.slot("eye").ellipsoid(
            at(EYE, s),
            Vec3::from(EYE_RADII),
            Quat::from_rotation_y(s * 0.35),
            3,
            5,
        );
        p.slot("skin").ellipsoid(
            at(EYE, s) + Vec3::new(0.0, 0.0055, 0.0005),
            Vec3::new(0.0115, 0.0028, 0.0042),
            Quat::from_rotation_y(s * 0.35),
            3,
            4,
        );
        // The eyebrow, in the hair's colour.
        p.slot("hair").ellipsoid(
            at(BROW, s),
            Vec3::new(0.0135, 0.0028, 0.0038),
            Quat::from_rotation_y(s * 0.4) * Quat::from_rotation_z(-s * 0.12),
            3,
            4,
        );
        // The ear, flat against the head, tilted back a little.
        p.slot("skin").ellipsoid(
            at(EAR, s),
            Vec3::from(EAR_RADII),
            Quat::from_rotation_x(-0.2),
            3,
            4,
        );
    }
    // The lips.
    p.slot("lips").ellipsoid(
        at(MOUTH, 1.0),
        Vec3::from(MOUTH_RADII),
        Quat::IDENTITY,
        3,
        6,
    );
    // The chin.
    p.slot("skin").ellipsoid(
        skull + Vec3::new(0.0, -0.088, -0.066),
        Vec3::new(0.024, 0.016, 0.018),
        Quat::from_rotation_x(0.3),
        3,
        4,
    );
    p.attach(m, i);
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
    // The bib's seat: widest over the hips under the jersey's hem, then
    // narrower up to the hip joint's height, closed there just above the
    // hem (spec §2.6; bug
    // 2026-11-16: the old waist was as wide as the jersey and reached 9 cm
    // above its hem, showing through it; `crate::clothing` checks the
    // layers).
    let seat = |p: &mut Parts, slot: &'static str, k: f32| {
        p.slot(slot).loft(
            &[
                ring(-0.07, 0.15 + k, 0.1 + k, 0.01),
                ring(-0.03, 0.138 + k, 0.094 + k, 0.016),
                ring(0.0, 0.122 + k, 0.08 + k, 0.025),
            ],
            n + 2,
            false,
            Cap::Point(Vec3::new(0.0, -0.11 - k, 0.0)),
            Cap::Flat,
        );
    };
    seat(&mut p, "shorts", 0.0);
    p.attach(&mut m, pelvis);
    // The race number's paper plates on the hips (in the shorts' slot: the
    // gear shader draws the paper and the digits).
    for (s, name) in opt::HIP_NUMBER.iter().enumerate() {
        let sign = if s == 0 { -1.0 } else { 1.0 };
        let (outer, half_t, [cy, cz], [hy, hz]) = HIP_PLATE;
        opt_node(&mut m, name, pelvis, |p| {
            p.slot("shorts").cuboid(
                Vec3::new(sign * (outer - half_t), cy, cz),
                Vec3::new(half_t, hy, hz),
                Quat::IDENTITY,
            );
        });
    }
    // The skinsuit over the shorts.
    opt_node(&mut m, opt::SUIT_PELVIS, pelvis, |p| {
        seat(p, "jersey", 0.005)
    });
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
        let short_leg = |p: &mut Parts, slot: &'static str, k: f32| {
            p.slot(slot).sweep(
                // From 6 cm under the hip joint, inside the seat (the old
                // leg began 3 cm above it, out through the jersey's sides).
                &[hip - Vec3::Y * 0.06, hip - Vec3::Y * 0.14, shorts_end],
                &[0.069 + k, 0.07 + k, 0.06 + k],
                n,
                false,
            );
        };
        short_leg(&mut p, "shorts", 0.0);
        p.slot("skin")
            .tube(shorts_end, knee, 0.057, 0.048, n, false);
        p.attach(&mut m, thigh);
        opt_node(&mut m, opt::SUIT_THIGH[s], thigh, |p| {
            short_leg(p, "jersey", 0.004)
        });
        opt_node(&mut m, opt::SHORTS_LONG[s], thigh, |p| {
            p.slot("shorts").tube(
                shorts_end + Vec3::Y * 0.01,
                shorts_end - Vec3::Y * 0.07,
                0.063,
                0.059,
                n,
                false,
            )
        });
        opt_node(&mut m, opt::TIGHTS_THIGH[s], thigh, |p| {
            p.slot("shorts")
                .tube(shorts_end + Vec3::Y * 0.01, knee, 0.062, 0.052, n, false)
        });
        let shin = m.add(Node::new(SHIN[s], Some(thigh), Vec3::NEG_Y * body::THIGH));
        let mut p = Parts::new();
        let ankle = knee - Vec3::Y * body::SHIN;
        let sock = ankle + Vec3::Y * 0.11;
        if hi {
            p.slot("skin")
                .ellipsoid(knee, Vec3::splat(0.05), Quat::IDENTITY, 4, n);
        }
        let calf = [knee, knee - Vec3::Y * 0.12 + Vec3::Z * 0.012, sock];
        p.slot("skin")
            .sweep(&calf, &[0.048, 0.054, 0.036], n, false);
        p.slot("sock").tube(sock, ankle, 0.036, 0.033, n, false);
        p.attach(&mut m, shin);
        opt_node(&mut m, opt::TIGHTS_SHIN[s], shin, |p| {
            if hi {
                p.slot("shorts")
                    .ellipsoid(knee, Vec3::splat(0.054), Quat::IDENTITY, 4, n);
            }
            p.slot("shorts")
                .sweep(&calf, &[0.052, 0.058, 0.04], n, false);
            p.slot("shorts")
                .tube(sock, ankle + Vec3::Y * 0.02, 0.04, 0.037, n, false);
        });
        let tall = |p: &mut Parts, slot: &'static str| {
            p.slot(slot).sweep(
                &[ankle + Vec3::Y * 0.23, ankle + Vec3::Y * 0.11, ankle],
                &[0.05, 0.04, 0.037],
                n,
                false,
            );
        };
        opt_node(&mut m, opt::SOCK_TALL[s], shin, |p| tall(p, "sock"));
        opt_node(&mut m, opt::SOCK_AERO[s], shin, |p| tall(p, "sock_aero"));
        let foot = m.add(Node::new(FOOT[s], Some(shin), Vec3::NEG_Y * body::SHIN));
        let mut p = Parts::new();
        if hi {
            p.slot("sock")
                .ellipsoid(ankle, Vec3::splat(0.035), Quat::IDENTITY, 4, n);
        }
        p.attach(&mut m, foot);
        // Shoes: heel to toe along -Z, the sole at the cleat's height.
        let sole = ankle.y + body::CLEAT[1];
        let shoe = |p: &mut Parts, slot: &'static str, k: f32, toe: f32| {
            let r = |z: f32, hw: f32, top: f32| Ring {
                centre: Vec3::new(x, 0.5 * (sole - k + top), ankle.z + z),
                u: Vec3::X * (hw + k),
                v: Vec3::Y * (0.5 * (top - sole + k)),
            };
            p.slot(slot).loft(
                &[
                    r(0.055 + k, 0.036, ankle.y + 0.0 + 2.0 * k),
                    r(-0.02, 0.045, ankle.y - 0.005 + k),
                    r(-0.12, 0.045, ankle.y - 0.04 + k),
                    r(-0.19, 0.035 - toe, ankle.y - 0.06 + k),
                ],
                n,
                false,
                Cap::Flat,
                Cap::Point(Vec3::new(x, sole + 0.012, ankle.z - 0.215 - k - toe)),
            );
        };
        opt_node(&mut m, opt::SHOE_ROAD[s], foot, |p| {
            shoe(p, "shoe", 0.0, 0.0)
        });
        // Race shoes: sleeker, a longer pointed toe, carbon black.
        opt_node(&mut m, opt::SHOE_RACE[s], foot, |p| {
            shoe(p, "shoe_race", 0.0, 0.006)
        });
        // Overshoes: over the shoe and up the ankle.
        opt_node(&mut m, opt::OVERSHOE[s], foot, |p| {
            shoe(p, "overshoe", 0.005, 0.0);
            p.slot("overshoe").tube(
                ankle - Vec3::Y * 0.01,
                ankle + Vec3::Y * 0.08,
                0.046,
                0.04,
                n,
                false,
            );
        });
        m.add(Node::new(CLEAT[s], Some(foot), v(body::CLEAT)));
    }
    // Torso: jersey, from the waist to the shoulders.
    let pivot = v(TORSO_PIVOT);
    let torso = m.add(Node::new(TORSO, Some(pelvis), pivot));
    let mut p = Parts::new();
    let t = |y: f32, hw: f32, hd: f32, z: f32| ring(pivot.y + y, hw, hd, pivot.z + z);
    let neck = pivot + Vec3::Y * body::TORSO;
    p.slot("jersey").loft(
        &jersey_rings(),
        jersey_sides(detail),
        false,
        Cap::Open,
        Cap::Point(neck - Vec3::Y * 0.01),
    );
    if hi {
        // The jersey's zip in the accent colour runs down the chest.
        p.slot("accent").tube(
            pivot + Vec3::new(0.0, 0.43, -0.103),
            pivot + Vec3::new(0.0, 0.05, -0.108),
            0.006,
            0.006,
            4,
            false,
        );
    }
    // The bib's top inside the jersey, carried by the torso: a lid in the
    // bib's colour across the jersey 2.5 cm above the hem's middle, 8 mm
    // inside it. A look up under the hem (the jersey rides up at the back
    // in the crouch) meets the bib, not the hollow torso.
    let lid_y = jersey_rings()[0].centre.y + 0.025;
    let sides = jersey_sides(detail);
    let rim: Vec<Vec3> = (0..sides)
        .map(|k| shell_corner(sides, k, lid_y, -0.008).expect("below the shoulders"))
        .collect();
    let mut lid = vec![rim.iter().copied().sum::<Vec3>() / sides as f32];
    lid.extend(&rim);
    let fan: Vec<[u32; 3]> = (0..sides)
        .map(|k| [0, 1 + (k + 1) % sides, 1 + k])
        .collect();
    p.slot("shorts").add_flat(&lid, &fan);
    p.attach(&mut m, torso);
    // The base bodies (spec §14 Phase 4, 4N N2): the shoulders' caps –
    // broader on A, narrower on B – and the chest of B and C, on the
    // jersey (near model only: one far body for all three, N9).
    for (k, name) in opt::SHOULDERS.iter().enumerate() {
        // A: today's caps; B narrower; C in between.
        let (inset, r) = [
            (0.01, Vec3::new(0.06, 0.055, 0.06)),
            (0.026, Vec3::new(0.052, 0.05, 0.055)),
            (0.018, Vec3::new(0.056, 0.052, 0.058)),
        ][k];
        opt_node(&mut m, name, torso, |p| {
            if hi {
                for s in [-1.0f32, 1.0] {
                    p.slot("jersey").ellipsoid(
                        pivot
                            + Vec3::new(
                                s * (SHOULDER_HALF_WIDTH - inset),
                                SHOULDER_HEIGHT - 0.01,
                                0.0,
                            ),
                        r,
                        Quat::IDENTITY,
                        3,
                        6,
                    );
                }
            }
        });
    }
    for (name, r, z) in [
        (opt::CHEST[0], Vec3::new(0.05, 0.044, 0.04), -0.097),
        (opt::CHEST[1], Vec3::new(0.043, 0.037, 0.026), -0.1),
    ] {
        opt_node(&mut m, name, torso, |p| {
            if hi {
                for s in [-1.0f32, 1.0] {
                    p.slot("jersey").ellipsoid(
                        pivot + Vec3::new(s * 0.052, 0.3, z),
                        r,
                        Quat::from_rotation_x(-0.25),
                        3,
                        6,
                    );
                }
            }
        });
    }
    // The back pockets (spec §2.6) and the anchors at their openings.
    let (panel, band, anchors) = pockets(detail);
    let i = m.add(Node::new(POCKETS, Some(torso), Vec3::ZERO));
    panel.attach(&mut m, i);
    let i = m.add(Node::new(POCKET_BAND, Some(torso), Vec3::ZERO));
    band.attach(&mut m, i);
    for (name, (at, out)) in POCKET_ANCHORS.iter().zip(anchors) {
        let i = m.add(Node::new(name, Some(torso), at - pivot));
        m.nodes[i].rotation = Quat::from_rotation_arc(Vec3::Z, out);
    }
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
    // Head: neck, face (nose).
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
    let skull = skull_centre();
    p.slot("skin").ellipsoid(
        skull,
        Vec3::new(0.078, 0.105, 0.095),
        Quat::IDENTITY,
        if hi { 7 } else { 4 },
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
    }
    p.attach(&mut m, head);
    // Hair styles (spec 4C: generated, own work).
    let hb = if hi { 6 } else { 3 };
    opt_node(&mut m, opt::HAIR_CAP, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, 0.0, 0.02),
            Vec3::new(0.081, 0.1, 0.09),
            Quat::from_rotation_x(-0.2),
            hb,
            n,
        )
    });
    opt_node(&mut m, opt::HAIR_BUZZ, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, 0.006, 0.012),
            Vec3::new(0.0795, 0.1, 0.088),
            Quat::from_rotation_x(-0.2),
            hb,
            n,
        )
    });
    opt_node(&mut m, opt::HAIR_LONG, head, |p| {
        // Down the back of the head to the shoulders.
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.1, 0.062),
            Vec3::new(0.083, 0.13, 0.04),
            Quat::from_rotation_x(0.12),
            if hi { 5 } else { 3 },
            n,
        );
    });
    opt_node(&mut m, opt::HAIR_TAIL, head, |p| {
        // Out under the helmet's rim at the back (spec §2.6).
        p.slot("hair").sweep(
            &[
                skull + Vec3::new(0.0, -0.02, 0.084),
                skull + Vec3::new(0.0, -0.07, 0.112),
                skull + Vec3::new(0.0, -0.165, 0.122),
            ],
            &[0.022, 0.024, 0.01],
            if hi { 6 } else { 4 },
            true,
        );
    });
    opt_node(&mut m, opt::HAIR_BUN, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, 0.045, 0.095),
            Vec3::new(0.042, 0.038, 0.04),
            Quat::IDENTITY,
            if hi { 4 } else { 3 },
            n,
        )
    });
    // The face (eyes, brows, ears, lips, chin).
    opt_node_face(&mut m, head, skull, detail);
    hair_and_face(&mut m, head, skull, detail);
    // The bare-head hair (Q28): each piece as it falls without a helmet,
    // fuller about the skull.
    {
        let head_at = Vec3::from(m.world(head).translation);
        let centre = skull - head_at;
        for (piece, name, k) in HAIR_BARE {
            let Some(i) = m.node(piece) else { continue };
            let mut node = Node::new(name, Some(head), m.nodes[i].translation);
            let off = m.nodes[i].translation;
            node.primitives = m.nodes[i]
                .primitives
                .iter()
                .map(|p| {
                    let mut q = p.clone();
                    for v in &mut q.positions {
                        let at = Vec3::from(*v) + off;
                        let out = centre + (at - centre) * k - off;
                        *v = out.to_array();
                    }
                    q
                })
                .collect();
            m.add(node);
        }
    }
    // Glasses (free): the sport wrap and round frames.
    opt_node(&mut m, opt::GLASSES_SPORT, head, |p| {
        if hi {
            p.slot("lens").ellipsoid(
                skull + Vec3::new(0.0, 0.012, -0.075),
                Vec3::new(0.083, 0.023, 0.035),
                Quat::IDENTITY,
                3,
                10,
            );
        }
    });
    opt_node(&mut m, opt::GLASSES_ROUND, head, |p| {
        if hi {
            for x in [-0.032f32, 0.032] {
                p.slot("lens").ellipsoid(
                    skull + Vec3::new(x, 0.012, -0.094),
                    Vec3::new(0.022, 0.02, 0.008),
                    Quat::IDENTITY,
                    3,
                    10,
                );
            }
            p.slot("metal").tube(
                skull + Vec3::new(-0.012, 0.016, -0.1),
                skull + Vec3::new(0.012, 0.016, -0.1),
                0.0025,
                0.0025,
                3,
                false,
            );
            for x in [-1.0f32, 1.0] {
                p.slot("metal").tube(
                    skull + Vec3::new(x * 0.053, 0.014, -0.092),
                    skull + Vec3::new(x * 0.079, 0.012, -0.01),
                    0.0025,
                    0.0025,
                    3,
                    false,
                );
            }
        }
    });
    // Head wear: road helmet, lightweight and aero road helmets, the
    // time-trial helmet, the cotton cap.
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
    // The helmets (spec §2.6, v3.40): crate::helmet.
    for h in crate::helmet::Helmet::ALL {
        opt_node(&mut m, h.node(), head, |p| {
            crate::helmet::build(p, h, skull, detail)
        });
    }
    opt_node(&mut m, CAP, head, |p| {
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
        // Short brim, flipped up a little, out under the helmet's brow.
        p.slot("jersey").cuboid(
            cc + Vec3::new(0.0, 0.012, -0.125),
            Vec3::new(0.065, 0.004, 0.035),
            Quat::from_rotation_x(0.35),
        );
    });
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
        // Sleeves to the elbow (aero jersey, skinsuit, long sleeves).
        opt_node(&mut m, opt::SLEEVE_UPPER[s], upper, |p| {
            p.slot("jersey")
                .tube(sleeve + Vec3::Y * 0.01, elbow, 0.046, 0.041, n, false)
        });
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
        opt_node(&mut m, opt::SLEEVE_LONG[s], fore, |p| {
            if hi {
                p.slot("jersey")
                    .ellipsoid(elbow, Vec3::splat(0.042), Quat::IDENTITY, 4, n);
            }
            p.slot("jersey")
                .tube(elbow, wrist + Vec3::Y * 0.01, 0.041, 0.03, n, false)
        });
        let hand = m.add(Node::new(HAND[s], Some(fore), Vec3::NEG_Y * body::FOREARM));
        // Gloves round the bar: fingers forward of the grip point.
        let palm = wrist - Vec3::Y * 0.06 + Vec3::Z * -0.005;
        let cuff = |p: &mut Parts, r: f32, len: f32| {
            p.slot("glove").tube(
                wrist + Vec3::Y * 0.004,
                wrist - Vec3::Y * len,
                r,
                r * 0.95,
                n,
                false,
            );
        };
        let ns = hand::palm_sign(s);
        opt_node(&mut m, opt::GLOVE_MITT[s], hand, |p| {
            if hi {
                // The hand shapes are the glove; a short cuff at the wrist.
                cuff(p, 0.03, 0.012);
            } else {
                p.slot("glove")
                    .ellipsoid(palm, Vec3::new(0.03, 0.055, 0.03), Quat::IDENTITY, 4, n)
            }
        });
        opt_node(&mut m, opt::GLOVE_GEL[s], hand, |p| {
            if hi {
                cuff(p, 0.03, 0.014);
                // The padded back in a light grey, on the back of the hand.
                p.slot("glove_pad").ellipsoid(
                    wrist + Vec3::new(-ns * (hand::PALM_HALF[0] + 0.002), hand::PALM_Y, 0.0),
                    Vec3::new(0.004, 0.03, 0.028),
                    Quat::IDENTITY,
                    3,
                    n,
                );
                return;
            }
            p.slot("glove")
                .ellipsoid(palm, Vec3::new(0.031, 0.056, 0.031), Quat::IDENTITY, 4, n);
            // The padded back in a light grey.
            p.slot("glove_pad").ellipsoid(
                palm + Vec3::new(if s == 0 { -0.012 } else { 0.012 }, 0.01, 0.0),
                Vec3::new(0.024, 0.03, 0.026),
                Quat::IDENTITY,
                3,
                n,
            );
        });
        opt_node(&mut m, opt::GLOVE_FULL[s], hand, |p| {
            if hi {
                cuff(p, 0.031, 0.02);
                return;
            }
            p.slot("glove").ellipsoid(
                palm - Vec3::Y * 0.01,
                Vec3::new(0.032, 0.07, 0.032),
                Quat::IDENTITY,
                4,
                n,
            )
        });
        opt_node(&mut m, opt::GLOVE_WINTER[s], hand, |p| {
            if !hi {
                p.slot("glove").ellipsoid(
                    palm - Vec3::Y * 0.01,
                    Vec3::new(0.038, 0.075, 0.038),
                    Quat::IDENTITY,
                    4,
                    n,
                );
            }
            // The cuff over the wrist.
            p.slot("glove").tube(
                wrist + Vec3::Y * 0.05,
                wrist - Vec3::Y * 0.01,
                0.034,
                0.036,
                n,
                false,
            );
        });
        // The hands' shapes (4N, the hands): near model only.
        if hi {
            for shape in hand::HandShape::ALL {
                opt_node(&mut m, hand::node(shape, s), hand, |p| {
                    hand_parts(p, wrist, shape, s)
                });
            }
        }
        m.add(Node::new(GRIP[s], Some(hand), Vec3::NEG_Y * body::HAND));
    }
    m
}

/// One hand shape's parts ([`hand`]) at the wrist `wrist` (rest-pose model
/// coordinates).
fn hand_parts(p: &mut Parts, wrist: Vec3, shape: hand::HandShape, side: usize) {
    // The palm and the back of the hand: tapered, the heel and the thumb's
    // ball rounded, the knuckles raised (`hand::palm`).
    let (verts, tris) = hand::palm(side);
    let verts: Vec<Vec3> = verts.into_iter().map(|q| wrist + q).collect();
    p.slot("glove").add_smooth(&verts, &tris);
    let sides = 5;
    let digit = |p: &mut Parts, d: &hand::Digit, r0: f32| {
        let pts: Vec<Vec3> = d.points.iter().map(|q| wrist + *q).collect();
        let n = pts.len();
        let radius = |k: usize| r0 + (hand::TIP_RADIUS - r0) * k as f32 / (n - 1) as f32;
        let base: Vec<f32> = (0..=d.base_end).map(radius).collect();
        let tip: Vec<f32> = (d.base_end..n).map(radius).collect();
        p.slot("glove")
            .sweep(&pts[..=d.base_end], &base, sides, true);
        p.slot("fingertip")
            .sweep(&pts[d.base_end..], &tip, sides, true);
    };
    for f in hand::fingers(shape, side) {
        digit(p, &f, hand::FINGER_RADIUS);
    }
    digit(p, &hand::thumb(shape, side), hand::FINGER_RADIUS + 0.001);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::RiderRig;

    /// The model's nodes with only the base look's optional nodes.
    fn base_only(m: &Model) -> Vec<usize> {
        let shown = base_visible();
        let hidden: Vec<usize> = optional()
            .into_iter()
            .filter(|n| !shown.contains(n))
            .filter_map(|n| m.node(n))
            .collect();
        (0..m.nodes.len())
            .filter(|i| !hidden.iter().any(|h| m.is_below(*i, *h)))
            .collect()
    }

    fn triangles(m: &Model, nodes: &[usize]) -> usize {
        nodes
            .iter()
            .flat_map(|i| &m.nodes[*i].primitives)
            .map(|p| p.indices.len() / 3)
            .sum()
    }

    /// Regression (2026-11-17, "the palm is a plain box"): the palm is a
    /// closed, smooth shape – narrower at the wrist than at the knuckles,
    /// the thumb's ball raised on the inner face, the knuckles on the back
    /// – with few triangles, and its inner face stays out of every bar and
    /// hood the hand holds.
    #[test]
    fn the_palm_is_shaped_and_stays_out_of_the_bars() {
        use hand::{HandShape, palm, palm_sign};
        for side in 0..2 {
            let n = palm_sign(side);
            let (v, tris) = palm(side);
            assert!(tris.len() <= 220, "{} triangles", tris.len());
            assert!(v.len() > 8, "not a box");
            // Closed: every edge in two triangles, once each way.
            let mut edges = std::collections::HashMap::new();
            for t in &tris {
                for k in 0..3 {
                    *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
                }
            }
            for ((a, b), c) in &edges {
                assert_eq!(*c, 1, "edge {a}-{b}");
                assert_eq!(edges.get(&(*b, *a)), Some(&1), "edge {a}-{b} open");
            }
            // Outward faces.
            for t in &tris {
                let [a, b, c] = t.map(|i| v[i as usize]);
                let centre = (a + b + c) / 3.0;
                let out = centre - Vec3::new(0.0, hand::PALM_Y, 0.0);
                assert!((b - a).cross(c - a).dot(out) > 0.0, "{t:?}");
            }
            // Tapered: the width (z) near the wrist and at the knuckles.
            let width = |y0: f32, y1: f32| {
                let zs = v.iter().filter(|q| q.y <= y0 && q.y >= y1).map(|q| q.z);
                zs.clone().fold(f32::MIN, f32::max) - zs.fold(f32::MAX, f32::min)
            };
            let (wrist, knuckles) = (width(-0.01, -0.03), width(-0.065, -0.085));
            assert!(wrist + 0.008 < knuckles, "{wrist} vs {knuckles}");
            // The thumb's ball stands out of the inner face; the knuckles
            // out of the back.
            let inner = |pick: &dyn Fn(&Vec3) -> bool| {
                v.iter()
                    .filter(|q| pick(q))
                    .map(|q| n * q.x)
                    .fold(f32::MIN, f32::max)
            };
            let ball = inner(&|q| q.z < -0.01 && (-0.04..=-0.02).contains(&q.y));
            let flat = inner(&|q| (-0.08..=-0.06).contains(&q.y));
            assert!(ball > flat + 0.003, "thumb ball {ball} vs {flat}");
            let back = |pick: &dyn Fn(&Vec3) -> bool| {
                v.iter()
                    .filter(|q| pick(q))
                    .map(|q| -n * q.x)
                    .fold(f32::MIN, f32::max)
            };
            let knuckle = back(&|q| (-0.09..=-0.078).contains(&q.y));
            let mid = back(&|q| (-0.06..=-0.04).contains(&q.y));
            assert!(knuckle > mid + 0.0005, "knuckles {knuckle} vs {mid}");
            // Out of what the hand holds: every vertex outside the bar's
            // (or the hood's) radius in the hand's frame.
            for shape in HandShape::ALL {
                let (Some(r), Some(c)) = (shape.wrap_radius(), hand::bar_centre(shape, side))
                else {
                    continue;
                };
                for q in &v {
                    let d = glam::Vec2::new(q.x - c.x, q.y - c.y).length();
                    assert!(d >= r - 0.0004, "{shape:?} {side}: {q} {d:.4} < {r}");
                }
            }
        }
    }

    #[test]
    fn the_rider_has_its_rig_and_stays_low_poly() {
        let m = build(Detail::High);
        for name in REQUIRED {
            assert!(m.node(name).is_some(), "{name}");
        }
        let rig = RiderRig::from_model(&m).unwrap();
        assert!((rig.leg() - 0.86).abs() < 1e-5);
        assert!((rig.forearm - 0.33).abs() < 1e-5);
        // One hand shape per hand is shown at a time.
        let others = hand::others(hand::HandShape::Relaxed);
        let shown: Vec<usize> = base_only(&m)
            .into_iter()
            .filter(|i| {
                !others
                    .iter()
                    .any(|o| m.node(o).is_some_and(|h| m.is_below(*i, h)))
            })
            .collect();
        let tris = triangles(&m, &shown);
        assert!((1500..6000).contains(&tris), "{tris} triangles");
        assert!(m.triangles() < 16_000, "all variants: {}", m.triangles());
        let low = build(Detail::Low);
        let low_tris = triangles(&low, &base_only(&low));
        assert!(low_tris * 2 < tris, "far LOD {low_tris} vs {tris}");
        // Standing upright, about 1.78 m tall from the soles (pelvis at the
        // hips), the base helmet on.
        let sole = -(body::THIGH + body::SHIN) + body::CLEAT[1];
        let top = shown
            .iter()
            .flat_map(|&i| {
                let w = m.world(i);
                m.nodes[i].primitives.iter().flat_map(move |p| {
                    p.positions
                        .iter()
                        .map(move |q| w.transform_point3(Vec3::from(*q)).y)
                })
            })
            .fold(f32::MIN, f32::max);
        let height = top - sole;
        assert!((1.7..1.9).contains(&height), "{height}");
    }

    /// Every variant names existing nodes of its slot; every optional node
    /// exists in both levels of detail; a slot's variant replaces its base
    /// item; the TT helmet's visor replaces the glasses.
    #[test]
    fn variants_and_the_visible_nodes_of_a_look() {
        for detail in [Detail::High, Detail::Low] {
            let m = build(detail);
            for n in optional() {
                assert!(m.node(n).is_some(), "{n} ({detail:?})");
            }
        }
        for v in VARIANTS {
            assert!(KIT_SLOTS.contains(&v.slot), "{}", v.id);
            assert!(!v.nodes.is_empty(), "{}", v.id);
            assert_eq!(variant(v.id), Some(v));
        }
        let base = base_visible();
        assert!(base.contains(&HELMET_ROAD) && base.contains(&opt::SHOE_ROAD[0]));
        assert!(base.contains(&opt::HAIR_CAP) && base.contains(&opt::GLASSES_SPORT));
        assert!(!base.contains(&CAP) && !base.contains(&KIT_BAND));
        let c = ocw_core::Character::default();
        let tt = visible(&c, &["helmet-tt", "shoes-race", "cap-cotton"]);
        assert!(tt.contains(&HELMET_AERO) && !tt.contains(&HELMET_ROAD));
        assert!(tt.contains(&opt::SHOE_RACE[1]) && !tt.contains(&opt::SHOE_ROAD[1]));
        // No cap under the TT helmet (its brim would cut the visor).
        assert!(!tt.contains(&CAP));
        let aero = visible(&c, &["helmet-aero-road", "cap-cotton"]);
        assert!(aero.contains(&CAP) && aero.contains(&opt::HELMET_AERO_ROAD));
        // The patterns are the gear shader's (T6): no band or sash nodes.
        assert!(!tt.contains(&KIT_BAND) && !tt.contains(&KIT_SASH));
        assert!(
            !tt.contains(&opt::GLASSES_SPORT),
            "the visor replaces the glasses"
        );
        // The character: bald, a full beard, round glasses.
        let ch = ocw_core::Character {
            hair: 5,
            beard: 4,
            glasses: 2,
            ..c
        };
        let v = visible(&ch, &[]);
        assert!(!v.iter().any(|n| n.starts_with("hair_")));
        assert!(v.contains(&opt::BEARD[3]) && v.contains(&opt::GLASSES_ROUND));
        let none = ocw_core::Character { glasses: 0, ..c };
        assert!(!visible(&none, &[]).iter().any(|n| n.starts_with("glasses")));
    }

    /// The back pockets (spec §2.6): three pockets across the lower back,
    /// the anchors at their openings on the torso (model contract 15B) –
    /// left, centre, right seen from behind, in the pocket (between the
    /// jersey and the panel), +z out of the back; the pockets shown with
    /// every jersey but the skinsuit.
    #[test]
    fn the_jersey_has_three_back_pockets_with_anchors() {
        for detail in [Detail::High, Detail::Low] {
            let m = build(detail);
            let torso = m.node(TORSO).unwrap();
            let sides = jersey_sides(detail);
            let at: Vec<Vec3> = POCKET_ANCHORS
                .iter()
                .map(|n| {
                    let i = m.node(n).unwrap_or_else(|| panic!("{n} ({detail:?})"));
                    assert_eq!(m.nodes[i].parent, Some(torso), "{n}");
                    assert!(m.nodes[i].primitives.is_empty());
                    let w = m.world(i);
                    let out = w.transform_vector3(Vec3::Z);
                    let p = w.transform_point3(Vec3::ZERO);
                    assert!(out.z > 0.7, "{n}: {out}");
                    // Between the jersey and the panel, at the opening.
                    assert!(
                        p.z > shell_point(sides, p.y, 1.5 * std::f32::consts::PI, 0.0).z - 0.05
                    );
                    assert!(
                        above_hem(p) > 0.08 && above_hem(p) < pocket::TOP,
                        "{n}: {p}"
                    );
                    p
                })
                .collect();
            assert!(
                at[0].x < -0.05 && at[1].x.abs() < 1e-4 && at[2].x > 0.05,
                "{at:?}"
            );
            assert!(at.iter().all(|p| p.z > 0.1), "on the back: {at:?}");
            assert!(m.node(POCKETS).is_some() && m.node(POCKET_BAND).is_some());
        }
        let c = ocw_core::Character::default();
        assert!(visible(&c, &[]).contains(&POCKETS));
        assert!(visible(&c, &["jersey-aero"]).contains(&POCKETS));
        assert!(!visible(&c, &["skinsuit"]).contains(&POCKETS));
        assert!(
            !visible(&c, &[]).contains(&POCKET_BAND),
            "the renderer's choice"
        );
    }

    /// Every node knows its body region (T6: patterns and numbers).
    #[test]
    fn nodes_have_their_body_regions() {
        use ocw_core::kit::BodyRegion::*;
        let m = build(Detail::High);
        let at = |n: &str| body_region(&m, m.node(n).unwrap());
        assert_eq!(at(TORSO), Torso);
        assert_eq!(at(opt::SUIT_PELVIS), Torso);
        assert_eq!(at(opt::SLEEVE_LONG[0]), Arm);
        assert_eq!(at(opt::SUIT_THIGH[1]), Leg);
        assert_eq!(at(opt::TIGHTS_SHIN[0]), Leg);
        assert_eq!(at(HELMET_ROAD), Head);
        assert_eq!(at(opt::GLOVE_GEL[1]), Hand);
        assert_eq!(at(opt::SHOE_RACE[0]), Foot);
        assert_eq!(at("rider"), Torso);
    }

    /// The shape widens the torso and limbs, never the head, hands, feet
    /// (spec 4C, 4N); torso and pelvis alike, about the torso's pivot.
    #[test]
    fn the_body_shape_scales_torso_and_limbs_only() {
        use ocw_core::{BaseBody, BodyShape, Character};
        let m = build(Detail::High);
        let at = |n: &str| m.node(n).unwrap();
        let with = |shape| Character {
            shape,
            ..Character::default()
        };
        let normal = with(BodyShape::Normal);
        for n in [TORSO, PELVIS, THIGH[0], UPPER_ARM[1], SHIN[0]] {
            assert_eq!(girth(&m, at(n), &normal), Vec3::ONE, "{n}");
        }
        let fat = with(BodyShape::Fat);
        let (t, l) = BodyShape::Fat.girth();
        assert_eq!(girth(&m, at(TORSO), &fat), Vec3::new(t, 1.0, t));
        assert_eq!(girth(&m, at(PELVIS), &fat), girth(&m, at(TORSO), &fat));
        assert_eq!(
            girth(&m, at(opt::SLEEVE_LONG[0]), &fat),
            Vec3::new(l, 1.0, l)
        );
        let slim = with(BodyShape::Slim);
        assert_eq!(
            girth(&m, at(opt::TIGHTS_SHIN[1]), &slim).x,
            BodyShape::Slim.girth().1
        );
        // The pelvis's meshes grow about the torso's pivot, the torso's
        // about its own origin (the pivot): the bib keeps its place under
        // the jersey.
        assert_eq!(girth_centre(&m, at(TORSO)), Vec3::ZERO);
        assert!(girth_centre(&m, at(PELVIS)).distance(v(body::TORSO_PIVOT)) < 1e-6);
        assert!(girth_centre(&m, at(opt::SUIT_PELVIS)).distance(v(body::TORSO_PIVOT)) < 1e-6);
        let (sc, tr) = girth_transform(&m, at(PELVIS), &fat);
        let pivot = v(body::TORSO_PIVOT);
        assert!((sc * pivot + tr).distance(pivot) < 1e-6, "the pivot stays");
        assert_eq!(girth_transform(&m, at(HEAD), &fat), (Vec3::ONE, Vec3::ZERO));
        // The arms follow the shoulders' width.
        let arm = at(UPPER_ARM[1]);
        let x = m.nodes[arm].translation.x;
        assert_eq!(node_translation(&m, arm, &normal).x, x);
        assert!((node_translation(&m, arm, &fat).x - x * t).abs() < 1e-6);
        assert_eq!(
            node_translation(&m, at(HEAD), &fat),
            m.nodes[at(HEAD)].translation
        );
        // The base bodies: wider hips (thighs) and narrower arms on B.
        let b = Character {
            base: BaseBody::B,
            ..normal
        };
        assert!(girth(&m, at(THIGH[0]), &b).x > 1.0);
        assert!(girth(&m, at(UPPER_ARM[0]), &b).x < 1.0);
        assert!(girth(&m, at(PELVIS), &b).x > 1.0);
        let strong = with(BodyShape::Obese);
        for n in [
            HEAD,
            HELMET_ROAD,
            opt::HAIR_CAP,
            HAND[0],
            opt::GLOVE_MITT[1],
            FOOT[0],
            opt::SHOE_ROAD[0],
            "rider",
        ] {
            assert_eq!(girth(&m, at(n), &strong), Vec3::ONE, "{n}");
        }
    }

    /// The face (2026-11-17): eyes, lids, brows, ears, lips and chin on
    /// the head of the near model, at their places (the lips under the
    /// nose, over the chin, the eyes level with the glasses, the ears at
    /// the sides), in their paint slots; the far model's head stays plain;
    /// the face is shown with every look.
    #[test]
    fn the_rider_has_a_face() {
        let near = build(Detail::High);
        let far = build(Detail::Low);
        let i = near.node(FACE).expect("the face");
        assert_eq!(near.nodes[i].parent, near.node(HEAD));
        assert!(
            far.node(FACE)
                .is_some_and(|f| far.nodes[f].primitives.is_empty())
        );
        let w = near.world(i);
        let skull = skull_centre();
        let slot = |name: &str| near.material(name).expect(name);
        let centre = |mat: usize| {
            let pts: Vec<Vec3> = near.nodes[i]
                .primitives
                .iter()
                .filter(|p| p.material == mat)
                .flat_map(|p| {
                    p.positions
                        .iter()
                        .map(|v| w.transform_point3(Vec3::from(*v)))
                })
                .collect();
            assert!(!pts.is_empty());
            (
                pts.iter().copied().sum::<Vec3>() / pts.len() as f32 - skull,
                pts,
            )
        };
        let (lips, _) = centre(slot("lips"));
        assert!(lips.distance(Vec3::from(face::MOUTH)) < 2e-3, "{lips}");
        let (eyes, eye_pts) = centre(slot("eye"));
        assert!(eyes.x.abs() < 1e-4 && (eyes.y - face::EYE[1]).abs() < 2e-3);
        assert!(eye_pts.iter().any(|p| p.x - skull.x > 0.025));
        assert!(eye_pts.iter().any(|p| p.x - skull.x < -0.025));
        // Lips below the nose, above the chin, in front of the face.
        assert!(lips.y < -0.035 && lips.y > -0.07 && lips.z < -0.075);
        // The ears at the sides, behind the eyes.
        const { assert!(face::EAR[0] > 0.07 && face::EAR[2] > face::EYE[2] + 0.05) };
        for c in [
            ocw_core::Character::default(),
            ocw_core::Character {
                age: ocw_core::Age::Seventies,
                base: ocw_core::BaseBody::B,
                shape: ocw_core::BodyShape::Obese,
                beard: 7,
                ..Default::default()
            },
        ] {
            let shown = visible(&c, &[]);
            assert!(!optional().contains(&FACE), "always shown");
            assert!(base_nodes("helmet").iter().all(|n| *n != FACE));
            let _ = shown;
            assert_eq!(girth(&near, i, &c), Vec3::ONE, "the head keeps its size");
        }
    }

    /// Every hair style, moustache, beard, base body and age shows its
    /// own nodes (4N), which exist in both levels of detail.
    #[test]
    fn the_4n_character_shows_its_nodes() {
        use ocw_core::character::{Age, BEARDS, HAIR_STYLES, MOUSTACHES};
        use ocw_core::{BaseBody, Character};
        let c = Character::default();
        let low = build(Detail::Low);
        let mut styles = std::collections::BTreeSet::new();
        for h in 0..HAIR_STYLES.len() as u8 {
            let v = visible(&Character { hair: h, ..c }, &[]);
            let hair: Vec<&str> = v
                .iter()
                .copied()
                .filter(|n| n.starts_with("hair_"))
                .collect();
            assert_eq!(hair, hair_nodes(h), "{}", HAIR_STYLES[h as usize]);
            for n in &hair {
                assert!(low.node(n).is_some(), "{n}");
            }
            assert!(
                styles.insert(hair),
                "{} is its own",
                HAIR_STYLES[h as usize]
            );
        }
        for (i, name) in MOUSTACHES.iter().enumerate().skip(1) {
            let v = visible(
                &Character {
                    moustache: i as u8,
                    ..c
                },
                &[],
            );
            assert!(v.contains(&opt::MOUSTACHE[i - 1]), "{name}");
        }
        for (i, name) in BEARDS.iter().enumerate().skip(1) {
            let v = visible(
                &Character {
                    beard: i as u8,
                    ..c
                },
                &[],
            );
            assert!(v.contains(&opt::BEARD[i - 1]), "{name}");
            assert!(!v.iter().any(|n| n.starts_with("moustache")));
        }
        assert!(!visible(&c, &[]).iter().any(|n| n.starts_with("beard")));
        for (k, b) in BaseBody::ALL.into_iter().enumerate() {
            let v = visible(&Character { base: b, ..c }, &[]);
            assert!(v.contains(&opt::SHOULDERS[k]), "{b:?}");
            assert_eq!(v.iter().filter(|n| n.starts_with("shoulders")).count(), 1);
            assert_eq!(v.iter().any(|n| n.starts_with("chest")), b != BaseBody::A);
        }
        let face = |a: Age| -> Vec<&str> {
            visible(&Character { age: a, ..c }, &[])
                .into_iter()
                .filter(|n| n.starts_with("face_"))
                .collect()
        };
        assert!(face(Age::Thirties).is_empty());
        assert_eq!(face(Age::Forties), [opt::FACE_LINES[0]]);
        assert_eq!(face(Age::Seventies), [opt::FACE_LINES[1], opt::FACE_JAW]);
    }

    /// The hands' gloves (spec §14 Phase 4, 4N, the hands): every hand
    /// shape has a palm, four fingers and a thumb with knuckle segments;
    /// the palm and the base joints are the glove's, the middle and outer
    /// joints `fingertip` (out of fingerless mitts and gel gloves, covered
    /// by full and winter gloves); every glove variant of the catalogue is
    /// one or the other; the near gloves no longer hide the hand (a cuff,
    /// the gel's pad), the far one stays a mitten.
    #[test]
    fn the_gloves_cover_or_expose_the_fingertips() {
        use hand::HandShape;
        let m = build(Detail::High);
        let mat = |slot: &str| m.material(slot).unwrap_or(usize::MAX);
        for shape in HandShape::ALL {
            for side in 0..2 {
                let i = m.node(hand::node(shape, side)).unwrap();
                let slots: Vec<usize> = m.nodes[i].primitives.iter().map(|p| p.material).collect();
                assert!(
                    slots.contains(&mat("glove")),
                    "{shape:?} {side}: the palm and base joints"
                );
                assert!(
                    slots.contains(&mat("fingertip")),
                    "{shape:?} {side}: the tips"
                );
                assert!(
                    slots
                        .iter()
                        .all(|s| *s == mat("glove") || *s == mat("fingertip"))
                );
                // Five digits of three (thumb: two) joints.
                assert_eq!(hand::fingers(shape, side).len(), 4);
                for f in hand::fingers(shape, side) {
                    assert_eq!(f.points.len(), 8, "base, middle and outer joints");
                    assert!(f.base_end > 0 && f.base_end < f.points.len() - 1);
                }
            }
        }
        let gloves: Vec<&Variant> = VARIANTS.iter().filter(|v| v.slot == "gloves").collect();
        assert!(gloves.len() >= 3);
        for v in gloves {
            let full = hand::covers_fingertips(&[v.id.to_string()]);
            assert_eq!(
                full,
                v.id == "gloves-full" || v.id == "gloves-winter",
                "{}",
                v.id
            );
        }
        assert!(
            !hand::covers_fingertips(&[]),
            "the base mitts are fingerless"
        );
        // The near glove nodes are cuffs now: small next to a hand shape.
        let tris = |n: &str| {
            let i = m.node(n).unwrap();
            m.nodes[i]
                .primitives
                .iter()
                .map(|p| p.triangles())
                .sum::<usize>()
        };
        let hand_tris = tris(hand::node(HandShape::Wrap, 0));
        for g in [opt::GLOVE_MITT[0], opt::GLOVE_FULL[0]] {
            assert!(tris(g) > 0 && tris(g) < hand_tris, "{g}");
        }
        let low = build(Detail::Low);
        let i = low.node(opt::GLOVE_MITT[0]).unwrap();
        assert!(
            low.nodes[i].primitives.iter().any(|p| p.triangles() > 20),
            "the far mitten"
        );
    }

    /// The helmet off (spec §14 Phase 4, 4N Q28): no helmet or cap shown,
    /// and every one of the 21 hair styles has its bare-head shape (fuller
    /// than under the helmet, no piece left out) as well as its shape under
    /// the helmet.
    #[test]
    fn every_hair_style_has_a_bare_head_shape() {
        let m = build(Detail::High);
        let size = |n: &str| {
            let i = m.node(n).unwrap_or_else(|| panic!("{n}"));
            let mut r = 0.0f32;
            let head = Vec3::from(m.world(m.nodes[i].parent.unwrap()).translation);
            let centre = skull_centre() - head;
            for p in &m.nodes[i].primitives {
                for v in &p.positions {
                    r = r.max((Vec3::from(*v) + m.nodes[i].translation - centre).length());
                }
            }
            r
        };
        for i in 0..ocw_core::character::HAIR_STYLES.len() as u8 {
            let c = ocw_core::Character {
                hair: i,
                ..Default::default()
            };
            let on = visible(&c, &[]);
            let off = visible(&c, &[HELMET_OFF]);
            assert!(
                on.contains(&HELMET_ROAD) && !off.contains(&HELMET_ROAD),
                "{i}"
            );
            assert!(!off.contains(&CAP));
            let under = hair_nodes(i);
            let bare = hair_nodes_bare(i);
            assert_eq!(under.len(), bare.len(), "style {i}: every piece bare");
            for (u, b) in under.iter().zip(&bare) {
                assert!(
                    on.contains(u) && !on.contains(b),
                    "{i}: {u} under the helmet"
                );
                assert!(off.contains(b) && !off.contains(u), "{i}: {b} bare");
                assert!(size(b) >= size(u), "{b} as full as {u}");
            }
        }
        assert!(
            size("hair_afro_bare") > size(opt::HAIR_AFRO) * 1.3,
            "the afro unpressed"
        );
    }
}
