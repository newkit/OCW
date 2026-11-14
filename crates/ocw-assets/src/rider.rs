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
}

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
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The optional nodes shown for a look: the kit `variants` (rider variant
/// ids of the worn items; slots without one wear the base item) and the
/// character's hair, face and glasses (a TT helmet's visor replaces the
/// glasses). The kit patterns are drawn by the renderer's gear shader
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
    if !out.contains(&HELMET_AERO) {
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
    let seat = |p: &mut Parts, slot: &'static str, k: f32| {
        p.slot(slot).loft(
            &[
                ring(-0.07, 0.15 + k, 0.1 + k, 0.01),
                ring(0.0, 0.165 + k, 0.11 + k, 0.01),
                ring(0.07, 0.155 + k, 0.105 + k, 0.015),
            ],
            n + 2,
            false,
            Cap::Point(Vec3::new(0.0, -0.11 - k, 0.0)),
            Cap::Flat,
        );
    };
    seat(&mut p, "shorts", 0.0);
    p.attach(&mut m, pelvis);
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
                &[hip + Vec3::Y * 0.03, hip - Vec3::Y * 0.12, shorts_end],
                &[0.075 + k, 0.072 + k, 0.06 + k],
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
    let skull = neck + Vec3::new(0.0, 0.15, -0.015);
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
        p.slot("hair").sweep(
            &[
                skull + Vec3::new(0.0, 0.0, 0.085),
                skull + Vec3::new(0.0, -0.05, 0.125),
                skull + Vec3::new(0.0, -0.15, 0.135),
            ],
            &[0.024, 0.026, 0.01],
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
    let hc = skull + Vec3::new(0.0, 0.0, 0.01);
    let vented = |p: &mut Parts, vents: &[f32], width: f32| {
        p.slot("helmet").loft(
            &dome(0.1, 0.13, 0.135, hc, if hi { 5 } else { 3 }),
            n + 4,
            false,
            Cap::Flat,
            Cap::Point(hc + Vec3::Y * 0.135),
        );
        if hi {
            // Vents: dark grooves front to back.
            for x in vents {
                p.slot("helmet_detail").sweep(
                    &[
                        hc + Vec3::new(*x, 0.07, -0.105),
                        hc + Vec3::new(*x, 0.12, -0.03),
                        hc + Vec3::new(*x, 0.115, 0.06),
                    ],
                    &[width, width + 0.002, width],
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
    };
    opt_node(&mut m, HELMET_ROAD, head, |p| {
        vented(p, &[-0.045, 0.0, 0.045], 0.012)
    });
    opt_node(&mut m, opt::HELMET_LIGHT, head, |p| {
        vented(p, &[-0.066, -0.033, 0.0, 0.033, 0.066], 0.013)
    });
    opt_node(&mut m, opt::HELMET_AERO_ROAD, head, |p| {
        // A smooth, closed shell, a little longer at the back.
        p.slot("helmet").loft(
            &dome(
                0.102,
                0.13,
                0.145,
                hc + Vec3::Z * 0.008,
                if hi { 5 } else { 3 },
            ),
            n + 4,
            false,
            Cap::Flat,
            Cap::Point(hc + Vec3::new(0.0, 0.132, 0.01)),
        );
        if hi {
            p.slot("helmet_detail").tube(
                hc + Vec3::new(-0.06, 0.06, -0.122),
                hc + Vec3::new(0.06, 0.06, -0.122),
                0.008,
                0.008,
                4,
                false,
            );
            for s in [-1.0f32, 1.0] {
                p.slot("helmet_detail").tube(
                    hc + Vec3::new(s * 0.087, 0.0, -0.01),
                    skull + Vec3::new(s * 0.05, -0.1, -0.03),
                    0.004,
                    0.004,
                    3,
                    false,
                );
            }
        }
    });
    opt_node(&mut m, HELMET_AERO, head, |p| {
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
    });
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
        // Short brim, flipped up.
        p.slot("jersey").cuboid(
            cc + Vec3::new(0.0, 0.025, -0.125),
            Vec3::new(0.065, 0.005, 0.035),
            Quat::from_rotation_x(0.55),
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
        assert!(tt.contains(&CAP));
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
