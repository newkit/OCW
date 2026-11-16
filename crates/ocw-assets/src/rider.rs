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
    pub const FACE_BEARD: &str = "face_beard";
    pub const FACE_MOUSTACHE: &str = "face_moustache";
    pub const FACE_STUBBLE: &str = "face_stubble";
    pub const GLASSES_SPORT: &str = "glasses_sport";
    pub const GLASSES_ROUND: &str = "glasses_round";
    /// The race number's paper plates on the hips (spec §14 Phase 8 T6),
    /// on the pelvis: shown when the look has a race number.
    pub const HIP_NUMBER: [&str; 2] = ["hip_number_l", "hip_number_r"];
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
            opt::FACE_BEARD,
            opt::FACE_MOUSTACHE,
            opt::FACE_STUBBLE,
            opt::GLASSES_SPORT,
            opt::GLASSES_ROUND,
        ])
        .chain(opt::HIP_NUMBER)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
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
    let mut out: Vec<&'static str> = Vec::new();
    for slot in KIT_SLOTS {
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
    out.extend(match c.hair {
        0 => &[opt::HAIR_CAP][..],
        1 => &[opt::HAIR_BUZZ],
        2 => &[opt::HAIR_CAP, opt::HAIR_LONG],
        3 => &[opt::HAIR_CAP, opt::HAIR_TAIL],
        4 => &[opt::HAIR_CAP, opt::HAIR_BUN],
        _ => &[],
    });
    out.extend(match c.face {
        1 => Some(opt::FACE_BEARD),
        2 => Some(opt::FACE_MOUSTACHE),
        3 => Some(opt::FACE_STUBBLE),
        _ => None,
    });
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

/// The scale of node `i`'s meshes for the body `build` (spec 4C): wider
/// and deeper (x, z; the limbs hang along −y at rest) on the torso and
/// limbs and the kit worn on them; 1 on the head, hands and feet (and
/// what hangs on them), whose rig nodes are not in [`GIRTH_NODES`].
pub fn girth(m: &Model, i: usize, build: ocw_core::BodyBuild) -> Vec3 {
    let g = build.girth();
    if (g - 1.0).abs() < 1e-6 {
        return Vec3::ONE;
    }
    // The nearest rig node at or above `i`.
    let mut at = Some(i);
    while let Some(n) = at {
        let name = m.nodes[n].name.as_str();
        if REQUIRED.contains(&name) {
            return if GIRTH_NODES.contains(&name) {
                Vec3::new(g, 1.0, g)
            } else {
                Vec3::ONE
            };
        }
        at = m.nodes[n].parent;
    }
    Vec3::ONE
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
    // Faces: a beard, a moustache, stubble (in the hair colour).
    opt_node(&mut m, opt::FACE_BEARD, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.062, -0.042),
            Vec3::new(0.072, 0.055, 0.06),
            Quat::from_rotation_x(0.25),
            if hi { 5 } else { 3 },
            n,
        )
    });
    opt_node(&mut m, opt::FACE_MOUSTACHE, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.038, -0.094),
            Vec3::new(0.028, 0.008, 0.012),
            Quat::IDENTITY,
            3,
            if hi { 8 } else { 4 },
        )
    });
    opt_node(&mut m, opt::FACE_STUBBLE, head, |p| {
        p.slot("hair").ellipsoid(
            skull + Vec3::new(0.0, -0.058, -0.03),
            Vec3::new(0.0715, 0.05, 0.068),
            Quat::from_rotation_x(0.2),
            if hi { 5 } else { 3 },
            n,
        )
    });
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
        opt_node(&mut m, opt::GLOVE_MITT[s], hand, |p| {
            p.slot("glove")
                .ellipsoid(palm, Vec3::new(0.03, 0.055, 0.03), Quat::IDENTITY, 4, n)
        });
        opt_node(&mut m, opt::GLOVE_GEL[s], hand, |p| {
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
            p.slot("glove").ellipsoid(
                palm - Vec3::Y * 0.01,
                Vec3::new(0.032, 0.07, 0.032),
                Quat::IDENTITY,
                4,
                n,
            )
        });
        opt_node(&mut m, opt::GLOVE_WINTER[s], hand, |p| {
            p.slot("glove").ellipsoid(
                palm - Vec3::Y * 0.01,
                Vec3::new(0.038, 0.075, 0.038),
                Quat::IDENTITY,
                4,
                n,
            );
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
        m.add(Node::new(GRIP[s], Some(hand), Vec3::NEG_Y * body::HAND));
    }
    m
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

    #[test]
    fn the_rider_has_its_rig_and_stays_low_poly() {
        let m = build(Detail::High);
        for name in REQUIRED {
            assert!(m.node(name).is_some(), "{name}");
        }
        let rig = RiderRig::from_model(&m).unwrap();
        assert!((rig.leg() - 0.86).abs() < 1e-5);
        assert!((rig.forearm - 0.33).abs() < 1e-5);
        let shown = base_only(&m);
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
        // The character: bald, a beard, round glasses.
        let ch = ocw_core::Character {
            hair: 5,
            face: 1,
            glasses: 2,
            ..c
        };
        let v = visible(&ch, &[]);
        assert!(!v.iter().any(|n| n.starts_with("hair_")));
        assert!(v.contains(&opt::FACE_BEARD) && v.contains(&opt::GLASSES_ROUND));
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

    /// The build widens the torso and limbs, never the head, hands, feet.
    #[test]
    fn the_body_build_scales_torso_and_limbs_only() {
        let m = build(Detail::High);
        let at = |n: &str| m.node(n).unwrap();
        use ocw_core::BodyBuild::*;
        assert_eq!(girth(&m, at(TORSO), Medium), Vec3::ONE);
        let g = Strong.girth();
        assert_eq!(girth(&m, at(TORSO), Strong), Vec3::new(g, 1.0, g));
        assert_eq!(
            girth(&m, at(opt::SLEEVE_LONG[0]), Strong),
            Vec3::new(g, 1.0, g)
        );
        assert_eq!(girth(&m, at(opt::TIGHTS_SHIN[1]), Slim).x, Slim.girth());
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
            assert_eq!(girth(&m, at(n), Strong), Vec3::ONE, "{n}");
        }
    }
}
