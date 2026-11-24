//! Putting the rider on the bike (spec §2.6: "bicycle and character must be
//! correctly attached"): the rider rig and the bike's anchors are read from
//! the models; the pose places the hips on the saddle (or over the bike
//! when standing), the hands on the bars and the feet on the pedals with
//! two-bone inverse kinematics. Pure and Bevy-free; the app writes the
//! result into the rider's node transforms each frame.

use glam::{Mat3, Quat, Vec3};

use crate::model::Model;
use crate::rider::hand::{self, HandShape};

/// The rider's joints and segment lengths, from the rider model's rest
/// pose (all node rotations identity, limbs hanging along −Y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiderRig {
    /// Hip joints in the pelvis frame (left, right).
    pub hips: [Vec3; 2],
    pub thigh: f32,
    pub shin: f32,
    /// Ankle → cleat (ball of the foot under the sole) in the foot frame.
    pub cleat: Vec3,
    /// Lower back (torso pivot) in the pelvis frame.
    pub torso: Vec3,
    /// Shoulders in the torso frame (left, right).
    pub shoulders: [Vec3; 2],
    /// Neck (head pivot) in the torso frame.
    pub neck: Vec3,
    pub upper_arm: f32,
    /// Elbow → grip (forearm plus hand).
    pub forearm: f32,
    /// The jersey's centre back pocket in the torso frame (spec §2.6: the
    /// hand takes the phone from it): the rider model's `anchor_pocket_c`
    /// node, else [`POCKET`].
    pub pocket: Vec3,
}

/// Where the bike holds the rider, from the bike model's anchor nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BikeRig {
    /// Top of the saddle.
    pub saddle: Vec3,
    /// Hands on the hoods (TT: the extensions), the tops (TT: the base
    /// bar), the drops; left, right.
    pub hoods: [Vec3; 2],
    pub tops: [Vec3; 2],
    pub drops: [Vec3; 2],
    /// Bottom bracket (crank axle).
    pub crank: Vec3,
    pub crank_length: f32,
    /// Pedal spindle offsets from the frame's centre (left < 0 < right).
    pub pedal_x: [f32; 2],
    /// Hub height = outer wheel radius.
    pub wheel_radius: f32,
    /// Height above the ground contact of the tread's centre of curvature
    /// (about half the tyre width): a leaning bike rolls about the line
    /// through it, so the tyre stays on the road (0 for models without the
    /// [`bike_nodes::TYRE_CROWN`] anchor: about the contact line).
    pub tyre_crown: f32,
    /// Distance between the hubs; the model's origin lies halfway.
    pub wheelbase: f32,
    /// TT bike: the elbows rest on pads, they bend down instead of out.
    pub aero_bars: bool,
}

/// Rig nodes of the rider model.
pub mod nodes {
    pub const PELVIS: &str = "pelvis";
    pub const TORSO: &str = "torso";
    pub const HEAD: &str = "head";
    pub const THIGH: [&str; 2] = ["thigh_l", "thigh_r"];
    pub const SHIN: [&str; 2] = ["shin_l", "shin_r"];
    pub const FOOT: [&str; 2] = ["foot_l", "foot_r"];
    pub const CLEAT: [&str; 2] = ["cleat_l", "cleat_r"];
    pub const UPPER_ARM: [&str; 2] = ["upper_arm_l", "upper_arm_r"];
    pub const FOREARM: [&str; 2] = ["forearm_l", "forearm_r"];
    pub const HAND: [&str; 2] = ["hand_l", "hand_r"];
    pub const GRIP: [&str; 2] = ["grip_l", "grip_r"];
    /// Head wear and kit pattern variants (one of each shown).
    pub const HELMET_ROAD: &str = "helmet_road";
    pub const HELMET_AERO: &str = "helmet_aero";
    pub const CAP: &str = "cap";
    pub const KIT_BAND: &str = "kit_band";
    pub const KIT_SASH: &str = "kit_sash";

    /// Every node a rider file must have (the helmets are not among them:
    /// they are worn from their own files, `wear/helmet-*.glb`, spec 14
    /// Phase 15 M3).
    pub const REQUIRED: [&str; 23] = [
        PELVIS,
        TORSO,
        HEAD,
        THIGH[0],
        THIGH[1],
        SHIN[0],
        SHIN[1],
        FOOT[0],
        FOOT[1],
        CLEAT[0],
        CLEAT[1],
        UPPER_ARM[0],
        UPPER_ARM[1],
        FOREARM[0],
        FOREARM[1],
        HAND[0],
        HAND[1],
        GRIP[0],
        GRIP[1],
        CAP,
        KIT_BAND,
        KIT_SASH,
        "rider",
    ];
}

/// Nodes of a bike model.
pub mod bike_nodes {
    pub const ROOT: &str = "bike";
    pub const WHEEL_FRONT: &str = "wheel_front";
    pub const WHEEL_REAR: &str = "wheel_rear";
    pub const CRANK: &str = "crank";
    pub const PEDAL: [&str; 2] = ["pedal_l", "pedal_r"];
    pub const HANDLEBAR: &str = "handlebar";
    pub const SADDLE: &str = "saddle";
    pub const HOODS: [&str; 2] = ["grip_hoods_l", "grip_hoods_r"];
    pub const TOPS: [&str; 2] = ["grip_tops_l", "grip_tops_r"];
    pub const DROPS: [&str; 2] = ["grip_drops_l", "grip_drops_r"];
    /// The far level of detail: bike and a seated rider in a few meshes.
    pub const LOD1: &str = "lod1";
    /// Anchor above the front tyre's ground contact: the centre of
    /// curvature of the tread's cross section, the line a leaning bike
    /// rolls about (optional; see [`super::BikeRig::tyre_crown`]).
    pub const TYRE_CROWN: &str = "tyre_crown";

    /// The workshop's hotspot anchors, `hs_<slot>` for every slot of
    /// [`crate::bikes::HOTSPOT_SLOTS`] (spec §14 Phase 4, 4B): required in
    /// every bike model.
    pub const HOTSPOTS: [&str; 17] = [
        "hs_frame",
        "hs_wheels",
        "hs_tyres",
        "hs_groupset",
        "hs_crankset",
        "hs_cassette",
        "hs_chain",
        "hs_cockpit",
        "hs_bar_tape",
        "hs_saddle",
        "hs_seatpost",
        "hs_pedals",
        "hs_bottle_cages",
        "hs_lights",
        "hs_computer",
        "hs_bag",
        "hs_paint",
    ];

    pub const REQUIRED: [&str; 15] = [
        ROOT,
        WHEEL_FRONT,
        WHEEL_REAR,
        CRANK,
        PEDAL[0],
        PEDAL[1],
        HANDLEBAR,
        SADDLE,
        HOODS[0],
        HOODS[1],
        TOPS[0],
        TOPS[1],
        DROPS[0],
        DROPS[1],
        LOD1,
    ];
}

fn local(m: &Model, name: &str) -> Result<Vec3, String> {
    m.node(name)
        .map(|i| m.nodes[i].translation)
        .ok_or_else(|| format!("missing node {name}"))
}

fn world(m: &Model, name: &str) -> Result<Vec3, String> {
    m.position(name)
        .ok_or_else(|| format!("missing node {name}"))
}

impl RiderRig {
    /// The rig of a rider model (rest pose).
    pub fn from_model(m: &Model) -> Result<Self, String> {
        use nodes::*;
        let pair = |names: [&str; 2]| -> Result<[Vec3; 2], String> {
            Ok([local(m, names[0])?, local(m, names[1])?])
        };
        let hips = pair(THIGH)?;
        let rig = Self {
            hips,
            thigh: local(m, SHIN[0])?.length(),
            shin: local(m, FOOT[0])?.length(),
            cleat: local(m, CLEAT[0])?,
            torso: local(m, TORSO)?,
            shoulders: pair(UPPER_ARM)?,
            neck: local(m, HEAD)?,
            upper_arm: local(m, FOREARM[0])?.length(),
            forearm: local(m, HAND[0])?.length() + local(m, GRIP[0])?.length(),
            pocket: match (m.node(POCKET_ANCHOR), m.node(TORSO)) {
                (Some(a), Some(t)) => {
                    Vec3::from(m.world(a).translation) - Vec3::from(m.world(t).translation)
                }
                _ => POCKET,
            },
        };
        if hips[0].x >= 0.0 || hips[1].x <= 0.0 {
            return Err("thigh_l must be on the left (-x), thigh_r on the right".into());
        }
        if rig.thigh < 0.2 || rig.shin < 0.2 || rig.upper_arm < 0.15 || rig.forearm < 0.15 {
            return Err(format!("implausible limb lengths: {rig:?}"));
        }
        Ok(rig)
    }

    pub fn leg(&self) -> f32 {
        self.thigh + self.shin
    }

    /// The rig of the same model drawn `s` times as large (the body
    /// height, spec §14 Phase 4, 4C: the rigid parts scale together, so
    /// the rig's lengths and joints scale by `s`).
    pub fn scaled(&self, s: f32) -> Self {
        Self {
            hips: self.hips.map(|h| h * s),
            thigh: self.thigh * s,
            shin: self.shin * s,
            cleat: self.cleat * s,
            torso: self.torso * s,
            shoulders: self.shoulders.map(|h| h * s),
            neck: self.neck * s,
            upper_arm: self.upper_arm * s,
            forearm: self.forearm * s,
            pocket: self.pocket * s,
        }
    }

    /// The rig of the model drawn for the character `c` (spec §14 Phase
    /// 4, 4N): the hip joints as wide as its seat
    /// ([`crate::rider::node_translation`]; bug 2026-11-19), so the legs
    /// reach the pedals from where they are drawn. The shoulders keep the
    /// rest rig's (the hands find the bar from either).
    pub fn for_body(&self, c: &ocw_core::Character) -> Self {
        let g = crate::rider::girth_of(nodes::TORSO, c);
        Self {
            hips: self.hips.map(|h| Vec3::new(h.x * g.x, h.y, h.z)),
            ..*self
        }
    }

    pub fn arm(&self) -> f32 {
        self.upper_arm + self.forearm
    }

    /// Wrist → grip (the hand's share of [`Self::forearm`]).
    pub fn hand(&self) -> f32 {
        use crate::rider::body::{FOREARM, HAND};
        self.forearm * HAND / (FOREARM + HAND)
    }
}

impl BikeRig {
    /// The anchors of a bike model.
    pub fn from_model(m: &Model) -> Result<Self, String> {
        use bike_nodes::*;
        let pair = |names: [&str; 2]| -> Result<[Vec3; 2], String> {
            Ok([world(m, names[0])?, world(m, names[1])?])
        };
        let crank = world(m, CRANK)?;
        let pedal = [local(m, PEDAL[0])?, local(m, PEDAL[1])?];
        let rig = Self {
            saddle: world(m, SADDLE)?,
            hoods: pair(HOODS)?,
            tops: pair(TOPS)?,
            drops: pair(DROPS)?,
            crank,
            crank_length: Vec3::new(0.0, pedal[1].y, pedal[1].z).length(),
            pedal_x: [pedal[0].x, pedal[1].x],
            wheel_radius: world(m, WHEEL_FRONT)?.y,
            tyre_crown: m.position(TYRE_CROWN).map_or(0.0, |p| p.y),
            wheelbase: world(m, WHEEL_REAR)?.z - world(m, WHEEL_FRONT)?.z,
            aero_bars: m.node("aero_pads").is_some(),
        };
        if rig.pedal_x[0] >= 0.0 || rig.pedal_x[1] <= 0.0 {
            return Err("pedal_l must be on the left (-x), pedal_r on the right".into());
        }
        if !(0.12..0.2).contains(&rig.crank_length) {
            return Err(format!("crank length {} m", rig.crank_length));
        }
        Ok(rig)
    }

    /// The seat tube's direction (bottom bracket → saddle): the saddle
    /// moves along it.
    pub fn seat_dir(&self) -> Vec3 {
        (self.saddle - self.crank).normalize_or(Vec3::Y)
    }

    /// The bike with its saddle (and seat post) moved `d` m along the
    /// seat tube.
    pub fn with_saddle_offset(&self, d: f32) -> Self {
        Self {
            saddle: self.saddle + self.seat_dir() * d,
            ..*self
        }
    }

    /// Right pedal position at crank angle `theta` (0 = up, increasing
    /// forward: π/2 = forward); the left pedal is at `theta + π`.
    pub fn pedal(&self, side: usize, theta: f32) -> Vec3 {
        let t = if side == 0 {
            theta + std::f32::consts::PI
        } else {
            theta
        };
        self.crank
            + Vec3::new(
                self.pedal_x[side],
                self.crank_length * t.cos(),
                -self.crank_length * t.sin(),
            )
    }
}

/// The crank node's rotation for crank angle `theta`.
pub fn crank_rotation(theta: f32) -> Quat {
    Quat::from_rotation_x(-theta)
}

/// Foot pitch (rad about +X, negative = toes down) over the pedal stroke
/// of a pedal at angle `t`: toes down at the bottom and the back, almost
/// flat over the top (ankling).
pub fn foot_pitch(t: f32) -> f32 {
    -0.32 + 0.2 * (t - 0.5).cos()
}

/// Ankle position for a cleat on a pedal at `pedal` with foot pitch `pitch`.
fn ankle(rider: &RiderRig, pedal: Vec3, pitch: f32) -> Vec3 {
    pedal - Quat::from_rotation_x(pitch) * rider.cleat
}

/// Two-bone IK: the middle joint for a chain `root` → `target` with
/// lengths `a` and `b`, bending towards `pole`; and the reached end
/// (the target, pulled in when out of reach).
pub fn two_bone(root: Vec3, target: Vec3, a: f32, b: f32, pole: Vec3) -> (Vec3, Vec3) {
    let to = target - root;
    let len = to.length().max(1e-5);
    let u = to / len;
    let d = len.clamp((a - b).abs() + 1e-4, a + b - 1e-4);
    let end = root + u * d;
    let cos = ((a * a + d * d - b * b) / (2.0 * a * d)).clamp(-1.0, 1.0);
    let sin = (1.0 - cos * cos).sqrt();
    let mut v = pole - u * pole.dot(u);
    if v.length_squared() < 1e-8 {
        v = u.any_orthonormal_vector();
    }
    let v = v.normalize();
    (root + u * (a * cos) + v * (a * sin), end)
}

/// The rotation of a limb segment hanging along −Y at rest, now pointing
/// along `dir`, with its joint axis (+X at rest) along `hinge`.
pub fn segment_rotation(dir: Vec3, hinge: Vec3) -> Quat {
    let y = -dir.normalize();
    let z = hinge.cross(y).normalize_or(Vec3::Z);
    let x = y.cross(z);
    Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize()
}

/// How the rider sits: blend weights, each 0…1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PoseParams {
    /// Crank angle (rad).
    pub crank: f32,
    /// Out of the saddle.
    pub stand: f32,
    /// Hands: on the tops / on the drops (the rest on the hoods).
    pub tops: f32,
    pub drops: f32,
    /// Lower and more bent arms (fast descent, sprint).
    pub tuck: f32,
    /// Side-to-side rocking of the bike (rad amplitude).
    pub rock: f32,
    /// Cornering beyond the lean (spec §2.6; −1 … 1, + = a right-hand
    /// bend): the upper body drops a little to the inside and the inside
    /// knee opens.
    pub corner: f32,
    /// Head turn (rad, + = to the rider's right): into a bend, over the
    /// shoulder before a U-turn; the shoulders turn [`LOOK_TORSO`] of it.
    pub look: f32,
    /// Head roll back towards level against the bike's lean (rad, + = to
    /// the rider's left).
    pub head_roll: f32,
    /// One foot off its pedal: 0 clipped in … 1 on the ground beside the
    /// bike (a stop); about half: held out low (a U-turn).
    pub foot: f32,
    /// The foot that comes off: 0 left, 1 right.
    pub foot_side: usize,
    /// Off the saddle onto the top tube with the bike tilted towards the
    /// foot (standing at a stop).
    pub straddle: f32,
    /// Both arms up in a V, the rider sitting up (victory).
    pub arms_up: f32,
    /// Sitting up after the finish: upright, hands on the tops, the head
    /// dropped.
    pub sit_up: f32,
    /// The bike throw at the finish line: hips back, arms straight.
    pub throw: f32,
    /// The phone's timeline (spec §2.6 *Rider status signs*): 0 in the back
    /// pocket … the right hand at the pocket … 1 held in front of the stem
    /// ([`crate::anim::StatusAnim`]).
    pub phone: f32,
    /// The one-handed pose while the phone is out: the right hand off the
    /// bar, the torso a little more upright.
    pub one_hand: f32,
    /// Extra head tilt down (rad; a VR rider in the menu).
    pub head_down: f32,
}

/// The bike's tilt towards the foot at a stop (rad, ~10°).
pub const FOOT_TILT: f32 = 0.17;
/// Share of that tilt the rider's body leans with the bike.
pub const FOOT_TILT_BODY: f32 = 0.3;
/// The ball of the foot on the ground this far out from the bike's centre
/// plane (m).
pub const FOOT_OUT_M: f32 = 0.27;
/// Straddling the top tube at a stop: the hips this far forward of the
/// seated hips (m), in front of the saddle's nose.
pub const STRADDLE_FORWARD_M: f32 = 0.17;
/// The foot rises this much on its way between pedal and ground (m).
pub const FOOT_LIFT_M: f32 = 0.06;
/// The inside knee opens by this (the knee's pole, sideways) in a full
/// bend.
pub const KNEE_OUT: f32 = 0.55;
/// Share of the head turn the shoulders take.
pub const LOOK_TORSO: f32 = 0.2;
/// The upper body's extra roll to the inside in a full bend (rad).
pub const CORNER_TORSO: f32 = 0.09;
/// The torso lean (from vertical, rad) with the arms up.
pub const ARMS_UP_LEAN: f32 = 0.2;
/// The bike throw: the hips this far back (m).
pub const THROW_BACK_M: f32 = 0.13;

/// What the app writes into the nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Roll of the whole bike (rad about the travel direction; + = to the
    /// rider's left, i.e. rotation about +Z).
    pub bike_roll: f32,
    pub crank: Quat,
    pub pedals: [Quat; 2],
    pub pelvis: (Vec3, Quat),
    pub torso: Quat,
    pub head: Quat,
    pub thighs: [Quat; 2],
    pub shins: [Quat; 2],
    pub feet: [Quat; 2],
    pub upper_arms: [Quat; 2],
    pub forearms: [Quat; 2],
    /// The phone in the right hand (spec §2.6): its centre and rotation in
    /// the bike's frame (the screen, the model's front −Z, towards the
    /// eyes); `None` while it is in the pocket.
    pub phone: Option<(Vec3, Quat)>,
    /// The hands' turn at the wrist (left, right; spec §14 Phase 4, 4N,
    /// the hands): on a bar the fingers wrap it, else none.
    pub hands: [Quat; 2],
    /// The hands' shapes (which `hand_<shape>_<side>` node is shown).
    pub hand_shapes: [HandShape; 2],
}

/// The back jersey pocket (the centre one) in the torso's frame of the
/// reference rider when the model has no [`POCKET_ANCHOR`] (spec §2.6: the
/// hand reaches it): the centre of the lower back, about 12 cm above the
/// jersey's hem, 2 cm outside the jersey.
pub const POCKET: Vec3 = Vec3::new(0.0, 0.06, 0.12);
/// The rider model's node at the centre back pocket's opening (added by the
/// jersey pockets; until then [`POCKET`]).
pub const POCKET_ANCHOR: &str = "anchor_pocket_c";
/// The reference rider's lower back to neck (m): the rig's scale.
const REFERENCE_NECK: f32 = 0.5;
/// Torso lean taken off in the one-handed pose (rad).
pub const ONE_HAND_UPRIGHT: f32 = 0.12;

impl Pose {
    /// `(node, local rotation)` for every posed node except the pelvis.
    pub fn rotations(&self) -> [(&'static str, Quat); 17] {
        use nodes::*;
        [
            (TORSO, self.torso),
            (HEAD, self.head),
            (THIGH[0], self.thighs[0]),
            (THIGH[1], self.thighs[1]),
            (SHIN[0], self.shins[0]),
            (SHIN[1], self.shins[1]),
            (FOOT[0], self.feet[0]),
            (FOOT[1], self.feet[1]),
            (UPPER_ARM[0], self.upper_arms[0]),
            (UPPER_ARM[1], self.upper_arms[1]),
            (FOREARM[0], self.forearms[0]),
            (FOREARM[1], self.forearms[1]),
            (HAND[0], self.hands[0]),
            (HAND[1], self.hands[1]),
            (bike_nodes::CRANK, self.crank),
            (bike_nodes::PEDAL[0], self.pedals[0]),
            (bike_nodes::PEDAL[1], self.pedals[1]),
        ]
    }

    /// Writes the pose into a rider model (the rider's own nodes; bike
    /// nodes are skipped when missing).
    pub fn apply(&self, m: &mut Model) {
        if let Some(i) = m.node(nodes::PELVIS) {
            m.nodes[i].translation = self.pelvis.0;
            m.nodes[i].rotation = self.pelvis.1;
        }
        for (name, q) in self.rotations() {
            if let Some(i) = m.node(name) {
                m.nodes[i].rotation = q;
            }
        }
    }
}

/// The rider fitted to a bike once (per avatar): where the hips are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub hip_seated: Vec3,
    pub hip_standing: Vec3,
}

/// Hip joint centre over the sit bones on the saddle (m, bike frame).
/// The saddle anchor is where the sit bones rest; the hip joints are above
/// and a little ahead of them.
pub const HIP_ABOVE_SADDLE: Vec3 = Vec3::new(0.0, 0.09, -0.03);

/// Largest hip → ankle distance over a crank turn, as a share of the leg.
pub fn leg_extension(rider: &RiderRig, bike: &BikeRig, hip: Vec3) -> f32 {
    (0..48)
        .map(|i| {
            let t = i as f32 / 48.0 * std::f32::consts::TAU;
            (0..2)
                .map(|s| {
                    let a = ankle(
                        rider,
                        bike.pedal(s, t),
                        foot_pitch(if s == 0 { t + std::f32::consts::PI } else { t }),
                    );
                    (a - (hip + rider.hips[s])).length()
                })
                .fold(0.0f32, f32::max)
        })
        .fold(0.0, f32::max)
        / rider.leg()
}

/// The height `y` in `lo..hi` at which `f(y)` (increasing in `y`) reaches
/// `target`.
pub fn bisect(mut lo: f32, mut hi: f32, target: f32, f: impl Fn(f32) -> f32) -> f32 {
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Leg extension seated (the saddle height is set for it) and standing.
pub const EXTENSION_SEATED: f32 = 0.95;
pub const EXTENSION_STANDING: f32 = 0.985;

impl Fit {
    pub fn new(rider: &RiderRig, bike: &BikeRig) -> Self {
        Self::with_hip(rider, bike, HIP_ABOVE_SADDLE)
    }

    /// The fit with the hip joints `hip_above` the saddle anchor.
    pub fn with_hip(rider: &RiderRig, bike: &BikeRig, hip_above: Vec3) -> Self {
        let hip_seated = bike.saddle + hip_above;
        // Standing: hips forward over the cranks and up until the legs are
        // almost straight at the bottom of the stroke.
        let forward = (hip_seated.z - bike.crank.z).max(0.0) * 0.55 + 0.05;
        let base = hip_seated - Vec3::Z * forward;
        let y = bisect(base.y - 0.1, base.y + 0.4, EXTENSION_STANDING, |y| {
            leg_extension(rider, bike, Vec3::new(base.x, y, base.z))
        });
        Self {
            hip_seated,
            hip_standing: Vec3::new(base.x, y, base.z),
        }
    }
}

/// The largest saddle move along the seat tube (m) for the body heights
/// of `ocw_core::character::HEIGHT_RANGE_CM` (the seat post is long
/// enough for it).
pub const MAX_SADDLE_OFFSET_M: f32 = 0.16;

/// The rider height the models are made for (body scale 1).
pub const REFERENCE_HEIGHT_M: f32 = ocw_core::character::DEFAULT_HEIGHT_CM as f32 / 100.0;

/// How a bike model is fitted to a rider's size (spec §14 Phase 15, Q7):
/// read from the file's `extras.ocw.sizing`, so an exported (or
/// hand-made) frame serves every body height without a file per size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BikeSizing {
    /// The rider height the frame was made for.
    pub reference_height_m: f32,
    /// How far the saddle (and post) may move along the seat tube.
    pub saddle_travel_m: f32,
}

impl Default for BikeSizing {
    fn default() -> Self {
        Self {
            reference_height_m: REFERENCE_HEIGHT_M,
            saddle_travel_m: MAX_SADDLE_OFFSET_M,
        }
    }
}

impl BikeSizing {
    /// The sizing data of a bike model (the defaults where the file has
    /// none).
    pub fn from_model(m: &Model) -> Self {
        let d = Self::default();
        let Some(s) = m.ocw().and_then(|o| o.get("sizing")) else {
            return d;
        };
        let num = |k: &str, or: f32| {
            s.get(k)
                .and_then(serde_json::Value::as_f64)
                .map(|v| v as f32)
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(or)
        };
        Self {
            reference_height_m: num("reference_rider_height_m", d.reference_height_m),
            saddle_travel_m: num("saddle_travel_m", d.saddle_travel_m),
        }
    }

    /// The body scale of a rider `height_m` tall on this frame.
    pub fn scale(&self, height_m: f32) -> f32 {
        height_m / self.reference_height_m
    }
}

/// A rider of body scale `scale` (height / 1.78 m) fitted to a bike (spec
/// §14 Phase 4, 4C): the scaled rig, the bike with the saddle at the
/// height that fits the legs (the bikes are generated for the default
/// rider; a taller rider raises the seat post, a smaller one lowers it,
/// so the legs reach [`EXTENSION_SEATED`] again) and the fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyFit {
    pub scale: f32,
    pub rider: RiderRig,
    pub bike: BikeRig,
    pub fit: Fit,
    /// The saddle's move along the seat tube (m, + = up).
    pub saddle_offset: f32,
}

impl BodyFit {
    /// `rider`: the model's own rig (scale 1); `bike`: as generated.
    pub fn new(rider: &RiderRig, bike: &BikeRig, scale: f32) -> Self {
        Self::sized(rider, bike, scale, &BikeSizing::default())
    }

    /// [`BodyFit::new`] with the bike file's own sizing data.
    pub fn sized(rider: &RiderRig, bike: &BikeRig, scale: f32, sizing: &BikeSizing) -> Self {
        let scale = if scale.is_finite() {
            scale.clamp(0.7, 1.3)
        } else {
            1.0
        };
        if (scale - 1.0).abs() < 1e-4 {
            return Self {
                scale: 1.0,
                rider: *rider,
                bike: *bike,
                fit: Fit::new(rider, bike),
                saddle_offset: 0.0,
            };
        }
        let r = rider.scaled(scale);
        let hip = HIP_ABOVE_SADDLE * scale;
        let lim = sizing.saddle_travel_m;
        let saddle_offset = bisect(-lim, lim, EXTENSION_SEATED, |d| {
            // A higher saddle stretches the legs more.
            let b = bike.with_saddle_offset(d);
            leg_extension(&r, &b, b.saddle + hip)
        });
        let b = bike.with_saddle_offset(saddle_offset);
        Self {
            scale,
            rider: r,
            bike: b,
            fit: Fit::with_hip(&r, &b, hip),
            saddle_offset,
        }
    }
}

/// Share of the arm's length from shoulder to grip (elbows a bit bent).
const ARM_REACH: f32 = 0.9;

/// Poses the rider on the bike.
pub fn pose(rider: &RiderRig, bike: &BikeRig, fit: &Fit, p: &PoseParams) -> Pose {
    use std::f32::consts::PI;
    let theta = p.crank;
    let stand = p.stand.clamp(0.0, 1.0);
    let straddle = p.straddle.clamp(0.0, 1.0);
    let foot_w = p.foot.clamp(0.0, 1.0);
    let fs = p.foot_side.min(1);
    let fsign = if fs == 0 { -1.0 } else { 1.0 };
    // A stop: the bike tilts towards the foot on the ground (+ roll = to
    // the rider's left).
    let tilt = -fsign * FOOT_TILT * straddle;
    let rock_roll = p.rock * theta.sin();
    let roll = rock_roll + tilt;
    // The rider stays above the contact line while the bike rocks under
    // them, and leans only a little with the bike tilted to the foot.
    let frame = Quat::from_rotation_z(-0.8 * rock_roll - (1.0 - FOOT_TILT_BODY) * tilt);
    let mut hip = fit.hip_seated.lerp(fit.hip_standing, stand);
    // The bike throw at the line: the hips go back.
    hip += Vec3::new(0.0, -0.03, THROW_BACK_M) * p.throw;
    // The foot on the ground (the ankle, in the rolled bike frame), flat
    // on the road; and the straddle: forward off the saddle and down until
    // that leg reaches the ground.
    let unroll = Quat::from_rotation_z(roll).inverse();
    let crown = Vec3::Y * bike.tyre_crown;
    let to_sway = |q: Vec3| unroll * (q - (crown - Quat::from_rotation_z(roll) * crown));
    let straddle_hip = fit.hip_seated - Vec3::Z * STRADDLE_FORWARD_M;
    let ground_foot = unroll;
    let ground_ankle =
        to_sway(Vec3::new(fsign * FOOT_OUT_M, 0.0, straddle_hip.z - 0.06) - rider.cleat);
    if straddle > 0.0 {
        let mut h = straddle_hip;
        let d = frame * (h + rider.hips[fs]) - ground_ankle;
        let reach = 0.96 * rider.leg();
        let flat = (d.x * d.x + d.z * d.z).min(reach * reach * 0.9);
        let up = (reach * reach - flat).sqrt();
        h.y -= (d.y - up).max(0.0);
        hip = hip.lerp(h, straddle);
    }
    // Seated: the hips rock a little with the pedal strokes.
    let rock = Quat::from_rotation_z(0.035 * (1.0 - stand) * theta.sin());
    let w_hoods = (1.0 - p.tops - p.drops).max(0.0);
    let grips: [Vec3; 2] = std::array::from_fn(|s| {
        bike.hoods[s] * w_hoods + bike.tops[s] * p.tops + bike.drops[s] * p.drops
    });
    let grip_centre = 0.5 * (grips[0] + grips[1]);
    let reach =
        rider.arm() * (ARM_REACH - 0.1 * p.tuck - 0.04 * stand + 0.08 * p.throw + 0.06 * p.sit_up);
    let pelvis_tilt = |lean: f32| Quat::from_rotation_x(-0.45 * lean);
    let shoulder_centre = |lean: f32| {
        let sh_y = 0.5 * (rider.shoulders[0].y + rider.shoulders[1].y);
        hip + pelvis_tilt(lean) * rider.torso
            + Quat::from_rotation_x(-lean) * Vec3::new(0.0, sh_y, 0.0)
    };
    // Torso lean: the angle at which the shoulders are `reach` from the grips.
    let lean = {
        let dist = |l: f32| shoulder_centre(l).distance(grip_centre);
        let (mut lo, mut hi) = (0.05f32, 1.5f32);
        if dist(lo) <= reach {
            lo
        } else if dist(hi) >= reach {
            hi
        } else {
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if dist(mid) > reach {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        }
    };
    // Arms up (victory): no hands on the bars, the rider sits up.
    let arms_up = p.arms_up.clamp(0.0, 1.0);
    let lean = lean + (ARMS_UP_LEAN - lean) * arms_up;
    // The phone (spec §2.6): one hand on the bar, a little more upright;
    // the torso twists to the right while the hand reaches the pocket.
    let one_hand = p.one_hand.clamp(0.0, 1.0);
    let lean = (lean - ONE_HAND_UPRIGHT * one_hand).max(0.05);
    let ph = p.phone.clamp(0.0, 1.0);
    let (pocket_a, pocket_b) = crate::anim::PHONE_AT_POCKET;
    let twist = crate::anim::PHONE_TWIST
        * crate::anim::window(
            f64::from(ph),
            0.05,
            f64::from(pocket_a),
            f64::from(pocket_b),
            0.85,
        ) as f32;
    let pelvis_rot = frame * pelvis_tilt(lean) * rock;
    let pelvis_pos = frame * hip;
    // The shoulders turn a little with the head and drop to the inside of
    // a bend.
    let torso_world = frame
        * Quat::from_rotation_y(-LOOK_TORSO * p.look - twist)
        * Quat::from_rotation_z(-CORNER_TORSO * p.corner)
        * Quat::from_rotation_x(-lean)
        * rock.inverse();
    let torso_pos = pelvis_pos + pelvis_rot * rider.torso;
    // Legs.
    let mut thighs = [Quat::IDENTITY; 2];
    let mut shins = [Quat::IDENTITY; 2];
    let mut feet = [Quat::IDENTITY; 2];
    let mut pedals = [Quat::IDENTITY; 2];
    let crank = crank_rotation(theta);
    let forward = pelvis_rot * Vec3::NEG_Z;
    for s in 0..2 {
        let t = if s == 0 { theta + PI } else { theta };
        let pedal = bike.pedal(s, theta);
        let pitch = foot_pitch(t);
        let pedal_world = Quat::from_rotation_x(pitch);
        let mut foot_world = pedal_world;
        let mut target = ankle(rider, pedal, pitch);
        // Unclipped: the foot goes out and down to the ground (lifted a
        // little on the way), flat on the road.
        if s == fs && foot_w > 0.0 {
            target =
                target.lerp(ground_ankle, foot_w) + Vec3::Y * (FOOT_LIFT_M * (PI * foot_w).sin());
            foot_world = pedal_world.slerp(ground_foot, foot_w);
        }
        let hip_joint = pelvis_pos + pelvis_rot * rider.hips[s];
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        // The inside knee opens in a bend (and a foot held out its knee).
        let inside = (s == 1 && p.corner > 0.0) || (s == 0 && p.corner < 0.0);
        let mut knee_out = 0.08;
        if inside {
            knee_out += KNEE_OUT * p.corner.abs();
        }
        if s == fs {
            knee_out += 0.3 * foot_w;
        }
        let (knee, end) = two_bone(
            hip_joint,
            target,
            rider.thigh,
            rider.shin,
            forward + out * knee_out,
        );
        let hinge = (end - hip_joint)
            .cross(knee - hip_joint)
            .normalize_or(Vec3::X);
        let thigh_w = segment_rotation(knee - hip_joint, hinge);
        let shin_w = segment_rotation(end - knee, hinge);
        thighs[s] = pelvis_rot.inverse() * thigh_w;
        shins[s] = thigh_w.inverse() * shin_w;
        feet[s] = shin_w.inverse() * foot_world;
        pedals[s] = crank.inverse() * pedal_world;
    }
    // Arms.
    let mut upper_arms = [Quat::IDENTITY; 2];
    let mut forearms = [Quat::IDENTITY; 2];
    let mut hands = [Quat::IDENTITY; 2];
    let mut hand_shapes = [HandShape::Relaxed; 2];
    let mut hand_r = Vec3::ZERO;
    let hand_len = rider.hand();
    for s in 0..2 {
        let shoulder = torso_pos + torso_world * rider.shoulders[s];
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let mut pole = if bike.aero_bars {
            Vec3::new(0.0, -1.0, 0.25)
        } else {
            out * 0.7 + Vec3::new(0.0, -0.35, 0.6)
        };
        let mut grip = grips[s];
        if s == 1 && ph > 0.0 {
            // The right hand: bar → back pocket → in front of the stem.
            let (target, phone_pole) = phone_hand(
                rider,
                ph,
                torso_pos,
                torso_world,
                grip,
                0.5 * (bike.tops[0] + bike.tops[1]),
            );
            grip = target;
            pole = phone_pole;
        }
        if arms_up > 0.0 {
            // Up in a V above the shoulders, the elbows out.
            let up = shoulder + Vec3::new(out.x * 0.45, 0.88, 0.1).normalize() * rider.arm() * 0.93;
            grip = grip.lerp(up, arms_up);
            pole = pole.lerp(out * 0.8 + Vec3::new(0.0, 0.0, 0.5), arms_up);
        }
        // On the bar the hand turns at the wrist so the fingers wrap it
        // (spec §14 Phase 4, 4N, the hands): the arm reaches the wrist.
        // Off the bar (the phone, arms up) the hand follows the forearm;
        // in between both blend, so the hand never jumps.
        let phone_t = if s == 1 {
            crate::anim::smoothstep(0.0, 0.12, f64::from(ph)) as f32
        } else {
            0.0
        };
        let free_t = phone_t.max(arms_up);
        let (free_elbow, free_end) = two_bone(shoulder, grip, rider.upper_arm, rider.forearm, pole);
        let free_wrist = free_end - (free_end - free_elbow).normalize_or(Vec3::NEG_Y) * hand_len;
        let held = (free_t < 1.0).then(|| {
            let (shape, rot, mut w) = hand_on_bar(
                bike,
                s,
                grips[s],
                shoulder,
                [w_hoods, p.tops, p.drops],
                hand_len,
            );
            // A TT bar too far for a short rider: the hand slides back
            // along the extension until the arm reaches it.
            let reach = rider.upper_arm + rider.forearm - hand_len;
            let short = w.distance(shoulder) - 0.995 * reach;
            if bike.aero_bars && short > 0.0 {
                w -= aero_axis([w_hoods, p.tops, p.drops]) * short.min(0.15);
            }
            (shape, rot, w)
        });
        let wrist = match held {
            Some((_, _, w)) => w.lerp(free_wrist, free_t),
            None => free_wrist,
        };
        let (elbow, end) = two_bone(
            shoulder,
            wrist,
            rider.upper_arm,
            rider.forearm - hand_len,
            pole,
        );
        let hinge = (end - shoulder)
            .cross(elbow - shoulder)
            .normalize_or(Vec3::X);
        let upper_w = segment_rotation(elbow - shoulder, hinge);
        let fore_w = segment_rotation(end - elbow, hinge);
        upper_arms[s] = torso_world.inverse() * upper_w;
        forearms[s] = upper_w.inverse() * fore_w;
        // Holding the phone up, the hand turns at the wrist so the palm
        // faces the eyes, the phone in the fingers in front of it (bug
        // 2026-11-17: the back of the hand was up, over the screen).
        let free_w = if s == 1 && ph > 0.0 {
            let hold_t = crate::anim::smoothstep(
                f64::from(crate::anim::PHONE_AT_POCKET.1),
                1.0,
                f64::from(ph),
            ) as f32;
            let face = torso_pos + torso_world * rider.neck;
            fore_w.slerp(palm_towards(fore_w, s, face - end), hold_t)
        } else {
            fore_w
        };
        let hand_w = match held {
            Some((_, on_bar, _)) => on_bar.slerp(free_w, free_t),
            None => free_w,
        };
        hands[s] = fore_w.inverse() * hand_w;
        hand_shapes[s] = match held {
            Some((shape, _, _)) if free_t < 0.5 => shape,
            _ if arms_up >= 0.5 => HandShape::Open,
            _ if ph >= crate::anim::PHONE_PICK => HandShape::Phone,
            _ => HandShape::Relaxed,
        };
        if s == 1 {
            // The phone in the fingers, beside the palm: where the phone
            // shape's fingers close.
            let scale = hand_len / crate::rider::body::HAND;
            let b = hand::bar_centre(HandShape::Phone, 1).unwrap_or(Vec3::ZERO);
            hand_r = end + hand_w * (b * scale);
        }
    }
    // Head: looking up the road, a little down; turned by `look`, held
    // nearer level in a lean, dropped when sitting up after the line;
    // down to the phone, following half the torso's twist.
    let reading = crate::anim::smoothstep(f64::from(pocket_b), 1.0, f64::from(ph)) as f32;
    let head_world = frame
        * Quat::from_rotation_y(-p.look - 0.5 * twist)
        * Quat::from_rotation_z(p.head_roll)
        * Quat::from_rotation_x(
            -(0.12 + 0.12 * lean)
                - 0.25 * p.sit_up
                - 0.12 * p.throw
                - crate::anim::PHONE_HEAD_DOWN * reading
                - p.head_down,
        );
    // The phone in the hand, its screen towards the eyes.
    let phone = (ph >= crate::anim::PHONE_PICK).then(|| {
        let k = rider.neck.length() / REFERENCE_NECK;
        let eyes = torso_pos + torso_world * rider.neck + head_world * (EYES * k);
        let to_eyes = (eyes - hand_r).normalize_or(Vec3::Y);
        // The model's front (−Z) to the eyes, its top (+Y) forward-up.
        let up = (Vec3::NEG_Z + Vec3::Y * 0.3).normalize();
        let z = -to_eyes;
        let x = up.cross(z).normalize_or(Vec3::X);
        let y = z.cross(x);
        (
            hand_r,
            Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize(),
        )
    });
    Pose {
        bike_roll: roll,
        crank,
        pedals,
        pelvis: (pelvis_pos, pelvis_rot),
        torso: pelvis_rot.inverse() * torso_world,
        head: torso_world.inverse() * head_world,
        thighs,
        shins,
        feet,
        upper_arms,
        forearms,
        phone,
        hands,
        hand_shapes,
    }
}

/// The bar a hand holds (spec §14 Phase 4, 4N, the hands): the shape, the
/// axis and the centre of the bar at grip `grip` (bike frame) for grip
/// weights `w` (hoods, tops, drops), the most held one deciding. Drop bars:
/// the drops run forward, the tops (and the hand across the hoods) across;
/// TT bars (extensions, base bar ends) forward; flat bars across.
pub fn bar_at(bike: &BikeRig, grip: Vec3, w: [f32; 3]) -> (HandShape, Vec3, Vec3) {
    let flat = !bike.aero_bars && bike.hoods == bike.drops;
    if bike.aero_bars {
        // The extensions and the base bar's ends run forward (the
        // extensions rising to their tips): round them.
        return (HandShape::Wrap, aero_axis(w), grip);
    }
    if flat {
        return (HandShape::Wrap, Vec3::X, grip);
    }
    let most = (0..3).max_by(|a, b| w[*a].total_cmp(&w[*b])).unwrap_or(0);
    match most {
        // The hood's body: below and ahead of where the hand rests on it;
        // the palm on top, the fingers over its front (not round it: the
        // bar runs under it).
        0 => (
            HandShape::Hoods,
            Vec3::X,
            grip + Vec3::new(0.0, -HOOD_BELOW_GRIP, -0.007) * w[0],
        ),
        1 => (HandShape::Wrap, Vec3::X, grip),
        // The drops: a little behind the hook's bend, so the thumb over the
        // top and the index finger stay clear of it.
        _ => (
            HandShape::Wrap,
            Vec3::Z,
            grip + Vec3::Z * (DROPS_BACK * w[2]),
        ),
    }
}

/// The TT bar's direction at the hands (forward; the extensions rise to
/// their tips by a third, the base bar's ends run level).
fn aero_axis(w: [f32; 3]) -> Vec3 {
    Vec3::new(0.0, w[0] / 3.0, -1.0).normalize()
}

/// On the tops and flat grips, how much the hand's long axis follows the
/// forearm (the rest: lying flatter, the wrist bent back a little).
pub const TOPS_WRIST_FOLLOW: f32 = 0.2;

/// How far behind the drops' grip anchor the hand holds them (m).
pub const DROPS_BACK: f32 = 0.035;

/// How far the hood's body lies below its grip anchor (m).
pub const HOOD_BELOW_GRIP: f32 = 0.011;

/// A hand holding the bar: its shape, its rotation (bike frame) and where
/// the wrist goes, for side `s` at grip `grip` with the shoulder at
/// `shoulder`, a hand `hand_len` long. The hand's knuckle line lies along
/// the bar, its long axis points from the bar towards the wrist (the
/// shoulder's side), and the palm faces the way the grip is held: down on
/// the hoods, the tops and flat grips (the knuckles up), inward on the
/// drops and the TT bars (the knuckles out, the thumb on top), so the
/// fingers wrap it ([`crate::rider::hand::bar_centre`] at the bar's
/// centre).
pub fn hand_on_bar(
    bike: &BikeRig,
    s: usize,
    grip: Vec3,
    shoulder: Vec3,
    w: [f32; 3],
    hand_len: f32,
) -> (HandShape, Quat, Vec3) {
    let (shape, k, c) = bar_at(bike, grip, w);
    let n = hand::palm_sign(s);
    let to_sh = (shoulder - c).normalize_or(Vec3::Y);
    let inward = if s == 0 { Vec3::X } else { Vec3::NEG_X };
    // The hand's long axis (bar → wrist) and the way the palm faces
    // (bike frame; bug 2026-11-17: "the hand position on the handlebar
    // looks wrong (upside down)"): the palm was turned by a test against
    // a fixed down-and-forward direction, which flipped it to face up
    // under the tops when the shoulder was high above the bar and to face
    // out on the drops and the TT bars (the fingers wrapped from the
    // inside). Now each grip says which way the palm faces:
    // - the hoods: the palm on top of the hood, the wrist behind it;
    // - the tops and flat / MTB grips (across): the palm down on the bar,
    //   the wrist a little bent back so the hand lies flatter than the
    //   forearm;
    // - the drops, the TT extensions and base bar ends (along): the palm
    //   inward to the bike, the fingers round the bar from the outside.
    let across = k.x.abs() > 0.7;
    let (y_dir, palm_to) = if shape == HandShape::Hoods && !bike.aero_bars {
        (Vec3::new(0.0, 0.25, 1.0), Vec3::NEG_Y)
    } else if across && !bike.aero_bars {
        let flat = Vec3::new(0.0, 0.3, 1.0).normalize();
        (
            (to_sh * TOPS_WRIST_FOLLOW + flat * (1.0 - TOPS_WRIST_FOLLOW)),
            Vec3::new(0.0, -1.0, 0.3),
        )
    } else {
        (to_sh, inward)
    };
    let y = (y_dir - k * y_dir.dot(k)).normalize_or(Vec3::Y);
    let mut nn = k.cross(y).normalize_or(palm_to);
    if nn.dot(palm_to) < 0.0 {
        nn = -nn;
    }
    let x = nn * n;
    let rot = Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y))).normalize();
    let scale = hand_len / crate::rider::body::HAND;
    let b = hand::bar_centre(shape, s).unwrap_or(Vec3::NEG_Y * hand_len) * scale;
    (shape, rot, c - rot * b)
}

/// The hand's rotation (bike frame) along the forearm rotation `fore`
/// (its long axis kept), turned about that axis so the palm of hand `s`
/// faces along `to` as far as it can.
pub fn palm_towards(fore: Quat, s: usize, to: Vec3) -> Quat {
    let y = fore * Vec3::Y;
    let d = to - y * to.dot(y);
    if d.length_squared() < 1e-8 {
        return fore;
    }
    // The palm faces the hand's x times its palm sign.
    let x = d.normalize() * hand::palm_sign(s);
    Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y))).normalize()
}

/// Where the right hand is on the phone's timeline `ph` (spec §2.6) and
/// the elbow's pole: from its `grip` on the bar back to the jersey's
/// centre pocket (the elbow out and back), out of it and forward to in
/// front of the stem at chest height (the elbow down): halfway from the
/// chest to the middle of the tops `bar` (the TT bike: its base bar).
pub fn phone_hand(
    rider: &RiderRig,
    ph: f32,
    torso_pos: Vec3,
    torso_world: Quat,
    grip: Vec3,
    bar: Vec3,
) -> (Vec3, Vec3) {
    use crate::anim::{PHONE_AT_POCKET, smoothstep};
    let k = rider.neck.length() / REFERENCE_NECK;
    let pocket = torso_pos + torso_world * rider.pocket;
    let sc = torso_pos + torso_world * (0.5 * (rider.shoulders[0] + rider.shoulders[1]));
    // In front of the stem, chest high: halfway from the chest to the bars,
    // a little up and to the right.
    let hold = sc.lerp(bar, 0.5) + Vec3::new(0.05 * k, 0.04 * k, 0.0);
    let (a, b) = PHONE_AT_POCKET;
    let ph = f64::from(ph);
    let to_pocket = smoothstep(0.0, f64::from(a), ph) as f32;
    let to_hold = smoothstep(f64::from(b), 1.0, ph) as f32;
    // The bar's grip → the pocket: an arc out to the side past the hip.
    let side = Vec3::X * (0.12 * k) * (std::f32::consts::PI * to_pocket).sin();
    let reach = grip.lerp(pocket, to_pocket) + side * (1.0 - to_hold);
    let target = reach.lerp(hold, to_hold);
    let back_pole = Vec3::new(0.9, 0.1, 0.5);
    let fwd_pole = Vec3::new(0.7, -0.35, 0.6);
    let down_pole = Vec3::new(0.5, -0.9, 0.1);
    let pole = if to_hold > 0.0 {
        back_pole.lerp(down_pole, to_hold)
    } else {
        fwd_pole.lerp(back_pole, to_pocket)
    };
    (target, pole)
}

/// How far the hips sink in the idle stance (m): the knees a little soft.
const IDLE_SINK: f32 = 0.012;

/// The rider standing at ease (spec §14 Phase 4, 4C: the changing
/// corner's idle animation), `t` seconds into it, in the rider's own frame
/// (the feet where the rest pose has them, flat on the floor): the arms
/// hang relaxed beside the body with soft elbows (not the rest pose's
/// straight arms held out), the weight shifts slowly from foot to foot
/// with the hips, the chest rises with the breath and the head looks
/// about now and then. Smooth and periodic-ish (incommensurate periods),
/// so it never visibly loops.
pub fn idle(rider: &RiderRig, t: f32) -> Pose {
    idle_aged(rider, t, 0.0)
}

/// [`idle`] with the shoulders rounded forward by `hunch` (rad; the
/// appearance age's, spec §14 Phase 4, 4N N3 / Q12:
/// `ocw_core::Age::idle_hunch`), the head lifted back as much so the
/// rider still looks ahead.
pub fn idle_aged(rider: &RiderRig, t: f32, hunch: f32) -> Pose {
    use std::f32::consts::TAU;
    let wave = |period: f32, phase: f32| (TAU * t / period + phase).sin();
    // Weight shift (+: onto the right foot), breath, a slow arm sway.
    let shift = wave(7.3, 0.0);
    let breath = wave(4.2, 0.6);
    let pelvis_pos = Vec3::new(0.018 * shift, -IDLE_SINK - 0.004 * shift.abs(), 0.0);
    // The loaded hip rises a little.
    let pelvis_rot = Quat::from_rotation_z(0.03 * shift);
    // The upper body stays nearly upright over the hips; the chest lifts
    // back a touch on the breath in.
    let torso_world =
        Quat::from_rotation_z(-0.012 * shift) * Quat::from_rotation_x(0.014 * breath - hunch);
    // Legs: the feet stay where they stand (two-bone IK to the rest
    // ankles), flat on the floor.
    let mut thighs = [Quat::IDENTITY; 2];
    let mut shins = [Quat::IDENTITY; 2];
    let mut feet = [Quat::IDENTITY; 2];
    for s in 0..2 {
        let hip = pelvis_pos + pelvis_rot * rider.hips[s];
        let ankle = rider.hips[s] - Vec3::Y * rider.leg();
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let (knee, end) = two_bone(hip, ankle, rider.thigh, rider.shin, Vec3::NEG_Z + out * 0.1);
        let hinge = (end - hip).cross(knee - hip).normalize_or(Vec3::X);
        let thigh_w = segment_rotation(knee - hip, hinge);
        let shin_w = segment_rotation(end - knee, hinge);
        thighs[s] = pelvis_rot.inverse() * thigh_w;
        shins[s] = thigh_w.inverse() * shin_w;
        feet[s] = shin_w.inverse();
    }
    // Arms: hanging beside the body (a little away from the hips), a touch
    // forward, the elbows soft; they sway with the breath.
    let mut upper_arms = [Quat::IDENTITY; 2];
    let mut forearms = [Quat::IDENTITY; 2];
    let mut hands = [Quat::IDENTITY; 2];
    for s in 0..2 {
        let side = if s == 0 { -1.0 } else { 1.0 };
        let sway = wave(6.1, 1.3 + s as f32 * 2.1);
        let abduct = 0.11 + 0.008 * breath;
        let forward = 0.07 + 0.025 * sway;
        upper_arms[s] = Quat::from_rotation_z(side * abduct) * Quat::from_rotation_x(forward);
        forearms[s] = Quat::from_rotation_x(0.26 + 0.03 * sway);
        // The relaxed hands (bug 2026-11-17: loose hands standing): the
        // wrist a little flexed towards the palm, swinging with the arm.
        hands[s] = idle_wrist(s, sway);
    }
    // The head: looking about (two slow turns) and a little down.
    let yaw = 0.2 * wave(11.0, 0.4) + 0.07 * wave(3.7, 1.0);
    let pitch = -0.05 + 0.025 * wave(5.3, 2.2);
    Pose {
        bike_roll: 0.0,
        crank: Quat::IDENTITY,
        pedals: [Quat::IDENTITY; 2],
        pelvis: (pelvis_pos, pelvis_rot),
        torso: pelvis_rot.inverse() * torso_world,
        head: Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch + hunch),
        thighs,
        shins,
        feet,
        upper_arms,
        forearms,
        phone: None,
        hands,
        hand_shapes: [HandShape::Relaxed; 2],
    }
}

/// The idle wrist of hand `side` at the arm's sway `sway` (−1 … 1): flexed
/// towards the palm by [`IDLE_WRIST_FLEX`] ± a little, so the relaxed hand
/// follows the arm's swing (about the knuckles' axis, z of the hand's
/// frame; the palm faces +x on the left hand).
pub fn idle_wrist(side: usize, sway: f32) -> Quat {
    Quat::from_rotation_z(hand::palm_sign(side) * (IDLE_WRIST_FLEX + 0.04 * sway))
}

/// The idle wrist's flex towards the palm (rad).
pub const IDLE_WRIST_FLEX: f32 = 0.1;

/// How far the hips drop when the photographer crouches (m), and how far
/// back they go.
const CROUCH_DROP: f32 = 0.42;
const CROUCH_BACK: f32 = 0.14;
/// The eyes in the head's frame (from the neck pivot), and the camera's
/// lens in front of them (m).
pub const EYES: Vec3 = Vec3::new(0.0, 0.11, -0.085);
pub const LENS_AHEAD: f32 = 0.13;

/// What the ride photographer holds (spec §2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// A camera to the eye.
    Camera,
    /// A drone's remote at the chest, the head up at the drone.
    Remote,
}

/// The ride photographer (spec §2.11): the rider model on foot, standing
/// or crouching, the camera held to the eye with both hands (or the
/// drone's remote at the chest), the head (and the camera) pitched by
/// `pitch` (rad, up), breathing a little with `t`. In the rider's own
/// frame, the feet planted where the rest pose has them. Returns the pose
/// and the lens (the camera's front, where the photo is taken from) and
/// the hands' centre (the camera's body or the remote).
pub fn photographer(
    rider: &RiderRig,
    crouch: bool,
    pitch: f32,
    holding: Holding,
    t: f32,
) -> (Pose, Vec3, Vec3) {
    use std::f32::consts::TAU;
    let breath = (TAU * t / 4.4).sin();
    let pitch = pitch.clamp(-0.9, 1.1);
    let (drop, back, lean) = if crouch {
        (CROUCH_DROP, CROUCH_BACK, 0.42)
    } else {
        (IDLE_SINK, 0.0, 0.05)
    };
    let pelvis_pos = Vec3::new(0.0, -drop, back);
    let pelvis_rot = Quat::IDENTITY;
    // The upper body leans forward over the knees when crouching; the
    // chest rises with the breath.
    let torso_world = Quat::from_rotation_x(-lean + 0.01 * breath);
    let torso_pos = pelvis_pos + pelvis_rot * rider.torso;
    // Legs: the feet stay where they stand (two-bone IK to the rest
    // ankles), the knees forward.
    let mut thighs = [Quat::IDENTITY; 2];
    let mut shins = [Quat::IDENTITY; 2];
    let mut feet = [Quat::IDENTITY; 2];
    for s in 0..2 {
        let hip = pelvis_pos + pelvis_rot * rider.hips[s];
        let ankle = rider.hips[s] - Vec3::Y * rider.leg();
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let (knee, end) = two_bone(
            hip,
            ankle,
            rider.thigh,
            rider.shin,
            Vec3::NEG_Z + out * 0.15,
        );
        let hinge = (end - hip).cross(knee - hip).normalize_or(Vec3::X);
        let thigh_w = segment_rotation(knee - hip, hinge);
        let shin_w = segment_rotation(end - knee, hinge);
        thighs[s] = pelvis_rot.inverse() * thigh_w;
        shins[s] = thigh_w.inverse() * shin_w;
        feet[s] = shin_w.inverse();
    }
    // The head: level with the world (whatever the lean) and pitched.
    let head_world = Quat::from_rotation_x(pitch);
    let neck = torso_pos + torso_world * rider.neck;
    let eyes = neck + head_world * EYES;
    let look = head_world * Vec3::NEG_Z;
    let lens = eyes + look * LENS_AHEAD;
    // The hands: either side of the camera body (behind the lens), or on
    // the remote in front of the chest.
    let (centre, frame) = match holding {
        Holding::Camera => (
            eyes + look * (LENS_AHEAD * 0.45) - Vec3::Y * 0.035,
            head_world,
        ),
        Holding::Remote => (
            torso_pos + torso_world * Vec3::new(0.0, rider.neck.y * 0.55, -0.3),
            torso_world * Quat::from_rotation_x(0.5),
        ),
    };
    let mut upper_arms = [Quat::IDENTITY; 2];
    let mut forearms = [Quat::IDENTITY; 2];
    for s in 0..2 {
        let side = if s == 0 { -1.0 } else { 1.0 };
        let shoulder = torso_pos + torso_world * rider.shoulders[s];
        let grip = centre + frame * Vec3::new(side * 0.075, 0.0, 0.0);
        let pole = Vec3::new(side * 0.8, -0.9, 0.2);
        let (elbow, end) = two_bone(shoulder, grip, rider.upper_arm, rider.forearm, pole);
        let hinge = (end - shoulder)
            .cross(elbow - shoulder)
            .normalize_or(Vec3::X);
        let upper_w = segment_rotation(elbow - shoulder, hinge);
        let fore_w = segment_rotation(end - elbow, hinge);
        upper_arms[s] = torso_world.inverse() * upper_w;
        forearms[s] = upper_w.inverse() * fore_w;
    }
    (
        Pose {
            bike_roll: 0.0,
            crank: Quat::IDENTITY,
            pedals: [Quat::IDENTITY; 2],
            pelvis: (pelvis_pos, pelvis_rot),
            torso: pelvis_rot.inverse() * torso_world,
            head: torso_world.inverse() * head_world,
            thighs,
            shins,
            feet,
            upper_arms,
            forearms,
            phone: None,
            hands: [Quat::IDENTITY; 2],
            hand_shapes: [HandShape::Relaxed; 2],
        },
        lens,
        centre,
    )
}

/// The eyes of a posed rider in the bike's frame (the ground at y = 0, −Z
/// ahead): the neck on the torso, [`EYES`] in the head scaled with the
/// rider.
pub fn eyes(rider: &RiderRig, p: &Pose) -> Vec3 {
    let torso_world = p.pelvis.1 * p.torso;
    let head_world = torso_world * p.head;
    let torso_pos = p.pelvis.0 + p.pelvis.1 * rider.torso;
    let k = rider.neck.length() / REFERENCE_NECK;
    torso_pos + torso_world * rider.neck + head_world * (EYES * k)
}

/// Where a rider's eyes are on a fitted bike (spec 14 Phase 14, 14I Q8):
/// seated (`stand` 0) or out of the saddle (1), the hands on the hoods,
/// the cranks level – the bike's frame, ground at y = 0. The saddle stands
/// where [`BodyFit`] put it for the rider profile's body height, so this
/// is the eye height that rider has on that bike.
pub fn eye_point(body: &BodyFit, stand: f32) -> Vec3 {
    eye_point_on(&body.rider, &body.bike, &body.fit, stand)
}

/// [`eye_point`] for a rider rig already fitted to its bike.
pub fn eye_point_on(rider: &RiderRig, bike: &BikeRig, fit: &Fit, stand: f32) -> Vec3 {
    let p = pose(
        rider,
        bike,
        fit,
        &PoseParams {
            stand,
            ..Default::default()
        },
    );
    eyes(rider, &p)
}

/// Where hand `side` of a posed rider model holds (tests): the centre of
/// the bar its fingers wrap, else the grip node.
#[cfg(test)]
pub(crate) fn held_point(m: &Model, p: &Pose, side: usize) -> Vec3 {
    let shape = p.hand_shapes[side];
    match (m.node(nodes::HAND[side]), hand::bar_centre(shape, side)) {
        (Some(i), Some(b)) if shape != HandShape::Phone => m.world(i).transform_point3(b),
        _ => m.position(nodes::GRIP[side]).unwrap_or(Vec3::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ride photographer (spec §2.11): the feet planted, the hands at
    /// the camera in front of the eyes, the lens ahead of the face and
    /// along the pitch; crouching lowers the lens by about 0.6 m; with the
    /// remote the hands are at the chest.
    #[test]
    fn the_photographer_holds_the_camera_to_the_eye() {
        let mut m = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&m).unwrap();
        let rest = m.clone();
        let at =
            |m: &Model, name: &str| m.world(m.node(name).unwrap()).transform_point3(Vec3::ZERO);
        let ankles = nodes::FOOT.map(|f| at(&rest, f));
        let mut lens_heights = Vec::new();
        for crouch in [false, true] {
            for pitch in [-0.2f32, 0.0, 0.3] {
                let (p, lens, _) = photographer(&rig, crouch, pitch, Holding::Camera, 1.3);
                p.apply(&mut m);
                for (s, ankle) in ankles.iter().enumerate() {
                    let a = at(&m, nodes::FOOT[s]);
                    assert!(
                        a.distance(*ankle) < 3e-3,
                        "crouch {crouch}: ankle {s} at {a}"
                    );
                }
                let head = at(&m, nodes::HEAD);
                for s in 0..2 {
                    let hand = at(&m, nodes::GRIP[s]);
                    assert!(hand.distance(lens) < 0.2, "hand {s} {hand} lens {lens}");
                    assert!(hand.z < head.z, "hands in front of the face");
                }
                assert!(lens.z < head.z - 0.1, "the lens ahead of the face");
                let eyes = head + Quat::from_rotation_x(pitch) * EYES;
                let dir = (lens - eyes).normalize();
                assert!((dir.y - pitch.sin()).abs() < 0.15, "pitch {pitch}: {dir}");
                if pitch == 0.0 {
                    lens_heights.push(lens.y);
                }
            }
        }
        let drop = lens_heights[0] - lens_heights[1];
        assert!(
            (0.3..0.8).contains(&drop),
            "crouching lowers the lens: {drop}"
        );
        // With the remote: the hands in front of the chest, the head up.
        let (p, _, hands) = photographer(&rig, false, 0.6, Holding::Remote, 0.0);
        p.apply(&mut m);
        let head = at(&m, nodes::HEAD);
        for s in 0..2 {
            let hand = at(&m, nodes::GRIP[s]);
            assert!(hand.y < head.y - 0.2 && hand.z < head.z, "{hand}");
            assert!(hand.distance(hands) < 0.2, "on the remote: {hand} {hands}");
        }
    }

    /// The idle stance (spec 4C): both feet stay planted and flat, the
    /// arms hang beside the body with soft elbows (not out like the rest
    /// pose's), and it moves – smoothly.
    #[test]
    fn the_idle_pose_stands_on_both_feet_with_the_arms_down() {
        let mut m = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&m).unwrap();
        let rest = m.clone();
        let at =
            |m: &Model, name: &str| m.world(m.node(name).unwrap()).transform_point3(Vec3::ZERO);
        let ankles = nodes::FOOT.map(|f| at(&rest, f));
        for i in 0..90 {
            let t = i as f32 * 0.37;
            let p = idle(&rig, t);
            p.apply(&mut m);
            for (s, ankle) in ankles.iter().enumerate() {
                let a = at(&m, nodes::FOOT[s]);
                assert!(a.distance(*ankle) < 2e-3, "t {t}: ankle {s} moved to {a}");
                let toe = m
                    .world(m.node(nodes::FOOT[s]).unwrap())
                    .transform_vector3(Vec3::NEG_Z);
                assert!(
                    toe.y.abs() < 0.01 && toe.z < -0.99,
                    "t {t}: foot {s} flat {toe}"
                );
                let shoulder = at(&m, nodes::UPPER_ARM[s]);
                let hand = at(&m, nodes::HAND[s]);
                assert!(hand.y < shoulder.y - 0.4, "t {t}: hand {s} hangs: {hand}");
                // Beside the hips, not held out.
                let x = hand.x.abs();
                assert!((0.12..0.42).contains(&x), "t {t}: hand {s} at x {x}");
                assert!(hand.z.abs() < 0.2, "t {t}: hand {s} at z {}", hand.z);
                // Soft elbows.
                let elbow = at(&m, nodes::FOREARM[s]);
                let bend = (elbow - shoulder).angle_between(hand - elbow);
                assert!(bend > 0.15, "t {t}: elbow {s} bent {bend}");
            }
        }
        // Smooth: at 60 fps every joint turns by a hair at most.
        for i in 0..600 {
            let (t0, t1) = (i as f32 / 60.0, (i + 1) as f32 / 60.0);
            let (q, p) = (idle(&rig, t0), idle(&rig, t1));
            for ((_, a), (_, b)) in q.rotations().iter().zip(p.rotations().iter()) {
                assert!(a.angle_between(*b) < 0.01, "t {t0}");
            }
            assert!(q.pelvis.0.distance(p.pelvis.0) < 1e-3);
        }
        // It moves: the head turns, the weight shifts.
        let (a, b) = (idle(&rig, 0.0), idle(&rig, 3.0));
        assert!(a.head.angle_between(idle(&rig, 5.5).head) > 0.1);
        assert!(a.pelvis.0.distance(b.pelvis.0) > 0.005);
        // Not the rest pose: the arms are not straight down.
        assert!(a.upper_arms[0].angle_between(Quat::IDENTITY) > 0.1);
    }

    #[test]
    fn two_bone_ik_reaches_and_bends_towards_the_pole() {
        let (root, target) = (Vec3::ZERO, Vec3::new(0.0, -0.7, 0.0));
        let (knee, end) = two_bone(root, target, 0.43, 0.43, Vec3::NEG_Z);
        assert!(end.distance(target) < 1e-5);
        assert!((knee.distance(root) - 0.43).abs() < 1e-4);
        assert!((knee.distance(end) - 0.43).abs() < 1e-4);
        assert!(knee.z < -0.1, "bends forward: {knee}");
        // Out of reach: straight, pulled in.
        let (knee, end) = two_bone(root, Vec3::new(0.0, -2.0, 0.0), 0.43, 0.43, Vec3::NEG_Z);
        assert!((end.y + 0.86).abs() < 1e-3 && knee.z.abs() < 0.02);
    }

    #[test]
    fn segment_rotation_is_identity_at_rest_and_points_the_limb() {
        assert!(segment_rotation(Vec3::NEG_Y, Vec3::X).angle_between(Quat::IDENTITY) < 1e-5);
        let dir = Vec3::new(0.0, -0.6, -0.8);
        let q = segment_rotation(dir, Vec3::X);
        assert!((q * Vec3::NEG_Y).distance(dir) < 1e-5);
        assert!((q * Vec3::X).distance(Vec3::X) < 1e-5);
    }

    #[test]
    fn pedals_go_round_forwards() {
        let bike = crate::bikes::rig(ocw_core::ProfileId::Road);
        let top = bike.pedal(1, 0.0);
        let front = bike.pedal(1, std::f32::consts::FRAC_PI_2);
        assert!(top.y > bike.crank.y + 0.15);
        assert!(front.z < bike.crank.z - 0.15, "forward is -z");
        // The left pedal is opposite.
        assert!(bike.pedal(0, 0.0).y < bike.crank.y - 0.15);
        // The crank node's rotation carries the right arm (+y at rest) there.
        let arm = crank_rotation(std::f32::consts::FRAC_PI_2) * Vec3::Y;
        assert!(arm.distance(Vec3::NEG_Z) < 1e-5);
    }

    /// The posed rider's feet are on the pedals and the hands on the bars,
    /// for every bike, seated and standing, all round the crank.
    #[test]
    fn hands_and_feet_stay_on_the_bike() {
        let mut rider = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&rider).unwrap();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::bikes::rig(profile);
            let fit = Fit::new(&rig, &bike);
            for stand in [0.0, 1.0] {
                for i in 0..12 {
                    let params = PoseParams {
                        crank: i as f32 / 12.0 * std::f32::consts::TAU,
                        stand,
                        rock: 0.1 * stand,
                        ..Default::default()
                    };
                    let p = pose(&rig, &bike, &fit, &params);
                    p.apply(&mut rider);
                    // Rider nodes are in the bike frame (rolled with it is
                    // the app's job), so compare with the bike's anchors.
                    for s in 0..2 {
                        let cleat = rider.position(nodes::CLEAT[s]).unwrap();
                        let pedal = bike.pedal(s, params.crank);
                        assert!(
                            cleat.distance(pedal) < 0.03,
                            "{profile:?} stand {stand} crank {i}: cleat {cleat} pedal {pedal}"
                        );
                        let grip = held_point(&rider, &p, s);
                        assert!(
                            grip.distance(bike.hoods[s]) < 0.05,
                            "{profile:?} stand {stand}: grip {grip} hoods {}",
                            bike.hoods[s]
                        );
                    }
                    // Seated on the saddle, standing clearly above it.
                    let pelvis = rider.position(nodes::PELVIS).unwrap();
                    let above = pelvis.y - bike.saddle.y;
                    if stand == 0.0 {
                        assert!(
                            (above - HIP_ABOVE_SADDLE.y).abs() < 0.02,
                            "{profile:?} {above}"
                        );
                    } else {
                        assert!(above > 0.1, "{profile:?} standing {above}");
                    }
                    // Knees bend forwards, never backwards.
                    for s in 0..2 {
                        let knee = rider.position(nodes::SHIN[s]).unwrap();
                        let hip = rider.position(nodes::THIGH[s]).unwrap();
                        let ankle = rider.position(nodes::FOOT[s]).unwrap();
                        let mid = 0.5 * (hip + ankle);
                        assert!(knee.z < mid.z + 0.005, "{profile:?} knee {knee} mid {mid}");
                    }
                }
            }
        }
    }

    /// Body heights from 1.50 to 2.05 m (spec §14 Phase 4, 4C): the rig
    /// scales, the saddle moves along the seat tube until the legs fit
    /// again, and feet stay on the pedals and hands on the bars on every
    /// bike, seated and standing.
    #[test]
    fn every_body_height_fits_every_bike() {
        let model = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&model).unwrap();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::bikes::rig(profile);
            assert_eq!(BodyFit::new(&rig, &bike, 1.0).saddle_offset, 0.0);
            let mut last = f32::MIN;
            for cm in [150u16, 160, 170, 178, 190, 205] {
                let s = f32::from(cm) / 178.0;
                let b = BodyFit::new(&rig, &bike, s);
                assert!(b.saddle_offset > last, "{profile:?} {cm}: taller, higher");
                last = b.saddle_offset;
                assert!(
                    b.saddle_offset.abs() < MAX_SADDLE_OFFSET_M - 0.005,
                    "{profile:?} {cm}: {b:?}"
                );
                let e = leg_extension(&b.rider, &b.bike, b.fit.hip_seated);
                assert!((e - EXTENSION_SEATED).abs() < 0.01, "{profile:?} {cm}: {e}");
                for stand in [0.0, 1.0] {
                    for i in 0..8 {
                        let params = PoseParams {
                            crank: i as f32 / 8.0 * std::f32::consts::TAU,
                            stand,
                            ..Default::default()
                        };
                        let p = pose(&b.rider, &b.bike, &b.fit, &params);
                        // The model drawn s times as large: positions of the
                        // posed (unscaled) model times s, the pelvis in the
                        // bike frame divided by s (as the renderer writes it).
                        let mut posed = model.clone();
                        p.apply(&mut posed);
                        let pi = posed.node(nodes::PELVIS).unwrap();
                        posed.nodes[pi].translation = p.pelvis.0 / s;
                        for side in 0..2 {
                            let cleat = posed.position(nodes::CLEAT[side]).unwrap() * s;
                            let pedal = b.bike.pedal(side, params.crank);
                            assert!(
                                cleat.distance(pedal) < 0.03,
                                "{profile:?} {cm} stand {stand}: cleat {cleat} pedal {pedal}"
                            );
                            let grip = held_point(&posed, &p, side) * s;
                            // A short rider's hands slide back along a TT
                            // bar's extensions until the arms reach.
                            let tol = if b.bike.aero_bars { 0.16 } else { 0.05 };
                            assert!(
                                grip.distance(b.bike.hoods[side]) < tol,
                                "{profile:?} {cm} stand {stand}: grip {grip} hoods {}",
                                b.bike.hoods[side]
                            );
                        }
                    }
                }
            }
        }
    }

    /// A point of the posed rider (bike frame, rolled with the bike by
    /// `roll` about the tyres' crown) back in the upright bike frame: the
    /// road is y = 0 there.
    fn unrolled(bike: &BikeRig, roll: f32, p: Vec3) -> Vec3 {
        let r = Quat::from_rotation_z(roll);
        let c = Vec3::Y * bike.tyre_crown;
        r * p + (c - r * c)
    }

    /// Foot down at a stop (spec §2.6, v3.47): on every bike, either foot,
    /// the unclipped foot stands flat on the road beside the bike, the bike
    /// tilts towards it, the other foot stays on its pedal and the hands on
    /// the bars.
    #[test]
    fn a_foot_goes_down_at_a_stop() {
        let mut rider = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&rider).unwrap();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::bikes::rig(profile);
            let fit = Fit::new(&rig, &bike);
            for side in 0..2 {
                let params = PoseParams {
                    crank: if side == 0 {
                        crate::anim::READY_CRANK
                    } else {
                        crate::anim::READY_CRANK + std::f32::consts::PI
                    },
                    foot: 1.0,
                    straddle: 1.0,
                    foot_side: side,
                    ..Default::default()
                };
                let p = pose(&rig, &bike, &fit, &params);
                p.apply(&mut rider);
                let sign = if side == 0 { -1.0 } else { 1.0 };
                // Tilted towards the foot (+ roll = left).
                assert!(
                    p.bike_roll * sign < -0.1,
                    "{profile:?} {side}: {}",
                    p.bike_roll
                );
                let cleat = unrolled(
                    &bike,
                    p.bike_roll,
                    rider.position(nodes::CLEAT[side]).unwrap(),
                );
                let ankle = unrolled(
                    &bike,
                    p.bike_roll,
                    rider.position(nodes::FOOT[side]).unwrap(),
                );
                assert!(
                    cleat.y.abs() < 0.03,
                    "{profile:?} {side}: sole on the road {cleat}"
                );
                assert!(
                    cleat.x * sign > 0.18,
                    "{profile:?} {side}: beside the bike {cleat}"
                );
                assert!(
                    ankle.y > cleat.y,
                    "{profile:?} {side}: ankle above the sole"
                );
                // Flat: the toes point forward along the road.
                let toe = Quat::from_rotation_z(p.bike_roll)
                    * rider
                        .world(rider.node(nodes::FOOT[side]).unwrap())
                        .transform_vector3(Vec3::NEG_Z);
                assert!(toe.y.abs() < 0.15, "{profile:?} {side}: flat foot {toe}");
                let other = 1 - side;
                let on = rider.position(nodes::CLEAT[other]).unwrap();
                assert!(
                    on.distance(bike.pedal(other, params.crank)) < 0.03,
                    "{profile:?} {side}: the other foot on its pedal"
                );
                for s in 0..2 {
                    let grip = held_point(&rider, &p, s);
                    assert!(
                        grip.distance(bike.hoods[s]) < 0.05,
                        "{profile:?} {side}: hands {grip}"
                    );
                }
                // Off the saddle, in front of it.
                let pelvis = rider.position(nodes::PELVIS).unwrap();
                assert!(
                    pelvis.z < fit.hip_seated.z - 0.1,
                    "{profile:?}: forward {pelvis}"
                );
                assert!(pelvis.y < fit.hip_seated.y, "{profile:?}: lower {pelvis}");
            }
        }
    }

    /// Victory, sitting up, the throw and cornering change the body where
    /// they should (spec §2.6, v3.47).
    #[test]
    fn victory_finish_and_cornering_poses() {
        let mut rider = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&rider).unwrap();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::bikes::rig(profile);
            let fit = Fit::new(&rig, &bike);
            // Arms up: both hands above the head, off the bars, wide.
            let p = pose(
                &rig,
                &bike,
                &fit,
                &PoseParams {
                    arms_up: 1.0,
                    ..Default::default()
                },
            );
            p.apply(&mut rider);
            let head = rider.position(nodes::HEAD).unwrap();
            for s in 0..2 {
                let hand = rider.position(nodes::GRIP[s]).unwrap();
                assert!(
                    hand.y > head.y + 0.1,
                    "{profile:?}: hand {s} up {hand} head {head}"
                );
                assert!(
                    hand.distance(bike.hoods[s]) > 0.4,
                    "{profile:?}: off the bars"
                );
                let sign = if s == 0 { -1.0 } else { 1.0 };
                assert!(hand.x * sign > 0.2, "{profile:?}: a V {hand}");
            }
            // Sitting up on the tops: the hands there, the torso more upright.
            let lean_of = |p: &Pose| ((p.pelvis.1 * p.torso) * Vec3::Y).angle_between(Vec3::Y);
            let base = pose(&rig, &bike, &fit, &PoseParams::default());
            let up = PoseParams {
                tops: 1.0,
                sit_up: 1.0,
                ..Default::default()
            };
            let p = pose(&rig, &bike, &fit, &up);
            p.apply(&mut rider);
            for s in 0..2 {
                let grip = held_point(&rider, &p, s);
                assert!(
                    grip.distance(bike.tops[s]) < 0.05,
                    "{profile:?}: tops {grip}"
                );
            }
            assert!(lean_of(&p) < lean_of(&base), "{profile:?}: sits up");
            // The throw: the hips back, the hands still on the bars.
            let p = pose(
                &rig,
                &bike,
                &fit,
                &PoseParams {
                    throw: 1.0,
                    ..Default::default()
                },
            );
            p.apply(&mut rider);
            assert!(
                p.pelvis.0.z > base.pelvis.0.z + 0.1,
                "{profile:?}: hips back"
            );
            for s in 0..2 {
                let grip = held_point(&rider, &p, s);
                // Where the hand holds the hoods (on a TT bar: over the
                // extension's end).
                let at = bar_at(&bike, bike.hoods[s], [1.0, 0.0, 0.0]).2;
                let tol = if bike.aero_bars { 0.16 } else { 0.05 };
                assert!(
                    grip.distance(at) < tol,
                    "{profile:?}: throw hands {grip} {at}"
                );
            }
            // A right-hand bend: the right (inside) knee further out than in
            // a straight line, the head turned right, feet on the pedals.
            let straight = PoseParams {
                crank: 0.4,
                ..Default::default()
            };
            pose(&rig, &bike, &fit, &straight).apply(&mut rider);
            let knee0 = rider.position(nodes::SHIN[1]).unwrap();
            let bend = PoseParams {
                corner: 1.0,
                look: 0.3,
                ..straight
            };
            let p = pose(&rig, &bike, &fit, &bend);
            p.apply(&mut rider);
            let knee = rider.position(nodes::SHIN[1]).unwrap();
            assert!(
                knee.x > knee0.x + 0.03,
                "{profile:?}: inside knee out {knee} vs {knee0}"
            );
            let fwd = (p.head * Vec3::NEG_Z).x;
            assert!(fwd > 0.1, "{profile:?}: head turned right {fwd}");
            for s in 0..2 {
                let cleat = rider.position(nodes::CLEAT[s]).unwrap();
                assert!(cleat.distance(bike.pedal(s, bend.crank)) < 0.03);
            }
        }
    }

    /// The TT rider is lowest, the MTB rider most upright (spec §2.5).
    #[test]
    fn torso_lean_follows_the_bike() {
        let rider = crate::rider::build(crate::rider::Detail::High);
        let rig = RiderRig::from_model(&rider).unwrap();
        let lean = |profile| {
            let bike = crate::bikes::rig(profile);
            let p = pose(&rig, &bike, &Fit::new(&rig, &bike), &PoseParams::default());
            let torso = p.pelvis.1 * p.torso;
            // Angle of the spine from vertical.
            (torso * Vec3::Y).angle_between(Vec3::Y)
        };
        use ocw_core::ProfileId::*;
        let (road, tt, mtb) = (lean(Road), lean(TimeTrial), lean(Mountain));
        assert!(tt > road + 0.15, "tt {tt} road {road}");
        assert!(mtb < road - 0.1, "mtb {mtb} road {road}");
        assert!((0.6..1.2).contains(&road), "road {road}");
    }

    /// 14I Q8: the eyes sit above the saddle the profile's height gives,
    /// higher for a taller rider (a higher saddle) and higher again out of
    /// the saddle; a 1.78 m rider's eyes are where a real one's are on a
    /// road bike (1.1 – 1.65 m above the floor, low on the time-trial bike).
    #[test]
    fn the_eye_point_follows_the_saddle_height() {
        let base = crate::shipped::rider_rig();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::shipped::bike_rig(profile);
            let mut last: Option<(f32, f32)> = None;
            for scale in [0.86f32, 1.0, 1.14] {
                let body = BodyFit::new(&base, &bike, scale);
                let seated = eye_point(&body, 0.0);
                let standing = eye_point(&body, 1.0);
                assert!(
                    seated.y > body.bike.saddle.y + 0.15 * scale,
                    "{profile:?} {scale}: eyes {seated} saddle {}",
                    body.bike.saddle
                );
                assert!(
                    standing.y > seated.y + 0.01,
                    "{profile:?} {scale}: standing {standing} seated {seated}"
                );
                if (scale - 1.0).abs() < 1e-6 {
                    assert!(
                        (1.1..1.65).contains(&seated.y),
                        "{profile:?}: seated eyes {seated}"
                    );
                }
                if let Some((saddle, eye)) = last {
                    assert!(body.bike.saddle.y > saddle, "{profile:?} {scale}: saddle");
                    assert!(seated.y > eye, "{profile:?} {scale}: eyes");
                }
                last = Some((body.bike.saddle.y, seated.y));
            }
        }
    }

    #[test]
    fn the_saddle_height_fits_the_legs() {
        let rig = RiderRig::from_model(&crate::rider::build(crate::rider::Detail::High)).unwrap();
        for profile in ocw_core::ProfileId::ALL {
            let bike = crate::bikes::rig(profile);
            let fit = Fit::new(&rig, &bike);
            let e = leg_extension(&rig, &bike, fit.hip_seated);
            assert!((e - EXTENSION_SEATED).abs() < 0.01, "{profile:?}: {e}");
            let e = leg_extension(&rig, &bike, fit.hip_standing);
            assert!((e - EXTENSION_STANDING).abs() < 0.01, "{profile:?}: {e}");
        }
    }
}

/// The phone's pose (spec §2.6 *Rider status signs*).
#[cfg(test)]
mod phone_tests {
    use ocw_core::ProfileId;

    use super::*;
    use crate::anim::{PHONE_AT_POCKET, PHONE_PICK};

    /// The right hand's end (the grip point) of a pose.
    /// The right hand's grasp: where the phone shape's fingers close (the
    /// hand turned at the wrist; spec §14 Phase 4, 4N, the hands), before
    /// it holds the phone the grip point.
    fn hand_at(rider: &RiderRig, p: &Pose, grasp: bool) -> Vec3 {
        let pelvis = p.pelvis;
        let torso_w = pelvis.1 * p.torso;
        let torso_pos = pelvis.0 + pelvis.1 * rider.torso;
        let shoulder = torso_pos + torso_w * rider.shoulders[1];
        let upper = torso_w * p.upper_arms[1];
        let elbow = shoulder + upper * (Vec3::NEG_Y * rider.upper_arm);
        let fore = upper * p.forearms[1];
        let wrist = elbow + fore * (Vec3::NEG_Y * (rider.forearm - rider.hand()));
        let scale = rider.hand() / crate::rider::body::HAND;
        let b = if grasp {
            crate::rider::hand::bar_centre(HandShape::Phone, 1).unwrap()
        } else {
            Vec3::new(0.0, crate::rider::hand::BAR_Y, 0.0)
        };
        wrist + fore * p.hands[1] * (b * scale)
    }

    /// The right hand's grip point (its palm's end).
    fn hand(rider: &RiderRig, p: &Pose) -> Vec3 {
        hand_at(rider, p, false)
    }

    #[test]
    fn the_hand_reaches_the_pocket_and_holds_the_phone_before_the_stem() {
        let base = crate::shipped::rider_rig();
        for profile in ProfileId::ALL {
            let bike = crate::shipped::bike_rig(profile);
            for scale in [0.86f32, 1.0, 1.14] {
                let body = BodyFit::new(&base, &bike, scale);
                let (r, b, fit) = (body.rider, body.bike, body.fit);
                let at = |ph: f32, stand: f32| {
                    let params = PoseParams {
                        phone: ph,
                        one_hand: crate::anim::smoothstep(0.0, 0.2, f64::from(ph)) as f32,
                        stand,
                        ..Default::default()
                    };
                    pose(&r, &b, &fit, &params)
                };
                let k = r.neck.length() / REFERENCE_NECK;
                // At the pocket: the hand on the jersey's back.
                let p = at(0.5 * (PHONE_AT_POCKET.0 + PHONE_AT_POCKET.1), 0.0);
                let torso_w = p.pelvis.1 * p.torso;
                let torso_pos = p.pelvis.0 + p.pelvis.1 * r.torso;
                let pocket = torso_pos + torso_w * r.pocket;
                let h = hand(&r, &p);
                assert!(
                    h.distance(pocket) < 0.03 * k,
                    "{profile:?} {scale}: hand {h} pocket {pocket}"
                );
                assert!(p.phone.is_some(), "picked up at the pocket");
                // Held: in front of the rider's chest, behind the bars,
                // between the bars' height and the shoulders, the screen
                // towards the eyes.
                let p = at(1.0, 0.0);
                let (ph, q) = p.phone.expect("held");
                let h = hand_at(&r, &p, true);
                assert!(h.distance(ph) < 1e-4, "in the fingers");
                let bar = 0.5 * (b.tops[0] + b.tops[1]);
                let torso_w = p.pelvis.1 * p.torso;
                let head = p.pelvis.0 + p.pelvis.1 * r.torso + torso_w * r.neck;
                let what = format!("{profile:?} {scale}: phone {ph} bar {bar} head {head}");
                assert!(ph.z > bar.z && ph.z < head.z + 0.05, "{what}");
                assert!(ph.y > bar.y - 0.02 && ph.y < head.y, "{what}");
                let eyes = head + torso_w * p.head * (EYES * k);
                let screen = q * Vec3::NEG_Z;
                let facing = screen.dot((eyes - ph).normalize());
                assert!(facing > 0.95, "{what}: screen {facing}");
                // The left hand stays on the bar; the head looks down.
                let rest = at(0.0, 0.0);
                let left = p.upper_arms[0].angle_between(rest.upper_arms[0]);
                assert!(left < 0.5, "{what}: left arm {left}");
                assert!(rest.phone.is_none());
                let pitch = |q: Quat| (q * Vec3::NEG_Z).y;
                let hw = |p: &Pose| p.pelvis.1 * p.torso * p.head;
                let (down, before) = (pitch(hw(&p)), pitch(hw(&rest)));
                assert!(down < before - 0.2, "{what}: head {down} vs {before}");
            }
        }
    }

    #[test]
    fn the_hand_moves_smoothly_along_the_timeline() {
        let r = crate::shipped::rider_rig();
        let b = crate::shipped::bike_rig(ProfileId::Road);
        let fit = Fit::new(&r, &b);
        let mut last: Option<Vec3> = None;
        for i in 0..=120 {
            let ph = i as f32 / 120.0;
            let params = PoseParams {
                phone: ph,
                one_hand: crate::anim::smoothstep(0.0, 0.2, f64::from(ph)) as f32,
                ..Default::default()
            };
            let p = pose(&r, &b, &fit, &params);
            let h = hand(&r, &p);
            if let Some(l) = last {
                assert!(h.distance(l) < 0.04, "{ph}: {h} {l}");
            }
            last = Some(h);
            assert_eq!(p.phone.is_some(), ph >= PHONE_PICK, "{ph}");
        }
    }

    #[test]
    fn the_vr_head_tilt_looks_down_a_little() {
        let r = crate::shipped::rider_rig();
        let b = crate::shipped::bike_rig(ProfileId::Road);
        let fit = Fit::new(&r, &b);
        let rest = pose(&r, &b, &fit, &PoseParams::default());
        let vr = pose(
            &r,
            &b,
            &fit,
            &PoseParams {
                head_down: crate::anim::VR_HEAD_DOWN,
                ..Default::default()
            },
        );
        assert!((vr.head.angle_between(rest.head) - crate::anim::VR_HEAD_DOWN).abs() < 1e-3);
        assert!(vr.phone.is_none());
    }
}

/// The hands (asked by the user 2026-11-17: "give the rider real hands";
/// spec §14 Phase 4, 4N).
#[cfg(test)]
mod hand_tests {
    use super::*;
    use crate::rider::hand::{self, HandShape};
    use crate::rider::{Detail, build};
    use ocw_core::ProfileId;

    /// The near rider posed with `p` at body scale `scale` (bike frame).
    fn posed(m: &Model, p: &Pose, scale: f32) -> Model {
        let mut m = m.clone();
        p.apply(&mut m);
        if let Some(i) = m.node(nodes::PELVIS) {
            m.nodes[i].translation = p.pelvis.0 / scale;
        }
        if let Some(i) = m.node("rider") {
            m.nodes[i].scale = Vec3::splat(scale);
        }
        m
    }

    /// The digits of hand `side` in shape `shape`, posed (bike frame).
    fn digits(m: &Model, side: usize, shape: HandShape) -> Vec<Vec<Vec3>> {
        let w = m.world(m.node(nodes::HAND[side]).expect("hand"));
        hand::fingers(shape, side)
            .into_iter()
            .chain([hand::thumb(shape, side)])
            .map(|d| d.points.iter().map(|q| w.transform_point3(*q)).collect())
            .collect()
    }

    /// Distance from `p` to a tube's centre line, and the tube's radius
    /// there.
    fn to_tube(p: Vec3, pts: &[Vec3], radii: &[f32]) -> (f32, f32) {
        pts.windows(2)
            .zip(radii.windows(2))
            .map(|(seg, r)| {
                let ab = seg[1] - seg[0];
                let t = ((p - seg[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (p.distance(seg[0] + ab * t), r[0] + (r[1] - r[0]) * t)
            })
            .min_by(|a, b| (a.0 - a.1).total_cmp(&(b.0 - b.1)))
            .expect("a segment")
    }

    /// Every bike's grips (hoods, tops, drops; the TT extensions and base
    /// bar; the MTB grips) for a short, a default and a tall rider: the
    /// hand takes the grip's shape, no finger or thumb cuts into a bar, a
    /// grip or a hood's body, and the fingers wrap the bar they hold (the
    /// fingertips close round it).
    #[test]
    fn fingers_wrap_every_bikes_grips_without_clipping() {
        let near = build(Detail::High);
        let rig = RiderRig::from_model(&near).unwrap();
        for profile in ProfileId::ALL {
            let held = crate::bikes::held_bars(profile);
            let bike = crate::shipped::bike_rig(profile);
            for scale in [0.84, 1.0, 1.15] {
                let body = BodyFit::new(&rig, &bike, scale);
                for (name, tops, drops) in
                    [("hoods", 0.0, 0.0), ("tops", 1.0, 0.0), ("drops", 0.0, 1.0)]
                {
                    let params = PoseParams {
                        tops,
                        drops,
                        ..Default::default()
                    };
                    let p = pose(&body.rider, &body.bike, &body.fit, &params);
                    let what = format!("{profile:?} {name} ×{scale}");
                    let drop_hoods = name == "hoods" && !bike.aero_bars && bike.hoods != bike.drops;
                    let want = if drop_hoods {
                        HandShape::Hoods
                    } else {
                        HandShape::Wrap
                    };
                    assert_eq!(p.hand_shapes, [want; 2], "{what}");
                    let m = posed(&near, &p, scale);
                    let finger_r = hand::TIP_RADIUS * scale - 0.0015;
                    for side in 0..2 {
                        let ds = digits(&m, side, want);
                        for (k, d) in ds.iter().enumerate() {
                            for q in d {
                                for (t, (pts, radii)) in held.tubes.iter().enumerate() {
                                    let (dist, r) = to_tube(*q, pts, radii);
                                    assert!(
                                        dist >= r + finger_r,
                                        "{what} side {side} digit {k} point {q}: {dist:.4} into bar {t} of {r:.4} ({pts:?}); held at {} for {}",
                                        held_point(&m, &p, side),
                                        bar_at(
                                            &body.bike,
                                            [body.bike.hoods, body.bike.tops, body.bike.drops]
                                                [if tops > 0.0 {
                                                    1
                                                } else if drops > 0.0 {
                                                    2
                                                } else {
                                                    0
                                                }][side],
                                            [1.0 - tops - drops, tops, drops]
                                        )
                                        .2
                                    );
                                }
                                for (c, radii, rot) in &held.hoods {
                                    let l = rot.inverse() * (*q - *c);
                                    let e = l / (*radii + Vec3::splat(finger_r));
                                    assert!(
                                        e.length() >= 1.0,
                                        "{what} side {side} digit {k}: into the hood ({})",
                                        e.length()
                                    );
                                }
                            }
                        }
                        // The fingertips close round (or over the end of)
                        // what the hand holds.
                        for d in &ds[..4] {
                            let tip = *d.last().unwrap();
                            let near_bar = held
                                .tubes
                                .iter()
                                .map(|(pts, radii)| {
                                    let (dist, r) = to_tube(tip, pts, radii);
                                    dist - r
                                })
                                .fold(f32::MAX, f32::min);
                            let near_hood = held
                                .hoods
                                .iter()
                                .map(|(c, _, _)| tip.distance(*c) - 0.04)
                                .fold(f32::MAX, f32::min);
                            assert!(
                                near_bar.min(near_hood) < 0.06,
                                "{what} side {side}: a fingertip {near_bar:.3} / {near_hood:.3} away"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The palm's normal (out of its inner face), the palm's centre and
    /// the thumb's tip of hand `side` in shape `shape`, posed (bike frame).
    fn palm_frame(m: &Model, side: usize, shape: HandShape) -> (Vec3, Vec3, Vec3) {
        let w = m.world(m.node(nodes::HAND[side]).expect("hand"));
        let normal = w
            .transform_vector3(Vec3::X * hand::palm_sign(side))
            .normalize();
        let centre = w.transform_point3(Vec3::new(0.0, hand::PALM_Y, 0.0));
        let thumb = w.transform_point3(*hand::thumb(shape, side).points.last().unwrap());
        (normal, centre, thumb)
    }

    /// Regression (bug 2026-11-17: "the hand position on the handlebar
    /// looks wrong (upside down)"): on every bike, at three heights, both
    /// hands hold each grip the way riders do – the palm down on the
    /// hoods, the tops and the flat grips (within 30°, the knuckles up),
    /// inward on the drops and the TT bars (within 30°, the knuckles out);
    /// the thumb inside the hood, behind and under the tops and flat grips
    /// and inward of the palm, over the drops and the TT bars; the
    /// fingertips under the tops and flat grips, inward round the drops
    /// and TT bars; the left hand the right one's mirror image.
    #[test]
    fn the_hands_hold_every_grip_the_right_way_up() {
        let near = build(Detail::High);
        let rig = RiderRig::from_model(&near).unwrap();
        let cos30 = 30f32.to_radians().cos();
        for profile in ProfileId::ALL {
            let bike = crate::shipped::bike_rig(profile);
            let flat = !bike.aero_bars && bike.hoods == bike.drops;
            for scale in [0.84, 1.0, 1.15] {
                let body = BodyFit::new(&rig, &bike, scale);
                for (name, tops, drops) in
                    [("hoods", 0.0, 0.0), ("tops", 1.0, 0.0), ("drops", 0.0, 1.0)]
                {
                    let params = PoseParams {
                        tops,
                        drops,
                        ..Default::default()
                    };
                    let p = pose(&body.rider, &body.bike, &body.fit, &params);
                    let m = posed(&near, &p, scale);
                    let what = format!("{profile:?} {name} ×{scale}");
                    let along = bike.aero_bars || (!flat && name == "drops");
                    let mut normals = Vec::with_capacity(2);
                    for side in 0..2 {
                        let shape = p.hand_shapes[side];
                        let inward = if side == 0 { Vec3::X } else { Vec3::NEG_X };
                        let (normal, centre, thumb) = palm_frame(&m, side, shape);
                        normals.push(normal);
                        let bar = held_point(&m, &p, side);
                        let tips: Vec<Vec3> = digits(&m, side, shape)[..4]
                            .iter()
                            .map(|d| *d.last().unwrap())
                            .collect();
                        let s = format!("{what} side {side}");
                        if along {
                            assert!(
                                normal.dot(inward) >= cos30,
                                "{s}: the palm faces {normal}, not inward"
                            );
                            assert!(thumb.y > bar.y, "{s}: the thumb over the bar");
                            for t in &tips {
                                assert!(
                                    (*t - bar).dot(inward) > 0.0,
                                    "{s}: a fingertip {t} not round the bar's inside {bar}"
                                );
                            }
                        } else {
                            assert!(
                                normal.dot(Vec3::NEG_Y) >= cos30,
                                "{s}: the palm faces {normal}, not down"
                            );
                            assert!(
                                (thumb - centre).dot(inward) > 0.01,
                                "{s}: the thumb {thumb} not inward of the palm {centre}"
                            );
                            assert!(thumb.y < centre.y, "{s}: the thumb under the palm");
                            if shape == HandShape::Wrap {
                                assert!(thumb.z > bar.z, "{s}: the thumb behind the bar");
                                for t in &tips {
                                    assert!(t.y < bar.y, "{s}: a fingertip {t} not under the bar");
                                }
                            } else {
                                // Over the hood: the fingers forward and
                                // down over its front.
                                for t in &tips {
                                    assert!(
                                        t.z < centre.z && t.y < centre.y,
                                        "{s}: a fingertip {t} not over the hood's front"
                                    );
                                }
                            }
                        }
                    }
                    // Left and right mirror each other.
                    let mirrored = normals[1] * Vec3::new(-1.0, 1.0, 1.0);
                    assert!(
                        normals[0].dot(mirrored) > 15f32.to_radians().cos(),
                        "{what}: the palms {normals:?} not mirrored"
                    );
                }
            }
        }
    }

    /// Regression (bug 2026-11-17, the phone grip upside down): holding
    /// the phone up, the right palm faces the rider's face, the phone in
    /// front of it.
    #[test]
    fn the_phone_lies_in_the_palm_facing_the_eyes() {
        let near = build(Detail::High);
        let rig = RiderRig::from_model(&near).unwrap();
        for profile in ProfileId::ALL {
            let bike = crate::shipped::bike_rig(profile);
            let body = BodyFit::new(&rig, &bike, 1.0);
            let p = pose(
                &body.rider,
                &body.bike,
                &body.fit,
                &PoseParams {
                    phone: 1.0,
                    one_hand: 1.0,
                    ..Default::default()
                },
            );
            let m = posed(&near, &p, 1.0);
            let (normal, centre, _) = palm_frame(&m, 1, HandShape::Phone);
            let head = m.position(nodes::HEAD).expect("head");
            let to_head = (head - centre).normalize();
            assert!(
                normal.dot(to_head) > 0.5,
                "{profile:?}: the palm faces {normal}, the head is {to_head}"
            );
            let (phone, _) = p.phone.expect("the phone out");
            assert!(
                (phone - centre).dot(normal) > 0.0,
                "{profile:?}: the phone on the palm's side"
            );
        }
    }

    /// Regression (2026-11-17, the workshop's standing rider showed loose
    /// hands): standing, both hands are relaxed – every finger bent a
    /// little at each joint towards the palm, the little finger most, the
    /// thumb along the index finger – and the wrist, a little flexed,
    /// moves with the idle animation's arm swing.
    #[test]
    fn standing_hands_are_relaxed_and_follow_the_idle_animation() {
        let near = build(Detail::High);
        let rig = RiderRig::from_model(&near).unwrap();
        let frames: Vec<Pose> = (0..40).map(|i| idle(&rig, i as f32 * 0.25)).collect();
        for side in 0..2 {
            let n = hand::palm_sign(side);
            let fingers = hand::fingers(HandShape::Relaxed, side);
            let curl = |d: &hand::Digit| {
                let p = &d.points;
                let a = p[1] - p[0];
                let b = p[p.len() - 1] - p[p.len() - 2];
                a.angle_between(b)
            };
            for (k, f) in fingers.iter().enumerate() {
                let tip = *f.points.last().unwrap();
                assert!(n * tip.x > 0.025, "finger {k} curls to the palm: {tip}");
                assert!(tip.y < hand::PALM_END_Y, "finger {k} below the palm");
                assert!((0.5..2.0).contains(&curl(f)), "finger {k}: {}", curl(f));
            }
            assert!(curl(&fingers[3]) > curl(&fingers[0]) + 0.15);
            let thumb = hand::thumb(HandShape::Relaxed, side);
            let index_mid = fingers[0].points[fingers[0].base_end];
            assert!(thumb.points.last().unwrap().distance(index_mid) < 0.04);
            // The wrist over the animation: flexed towards the palm, and
            // moving.
            let fingertip = |q: Quat| q * Vec3::new(0.0, -0.15, 0.0);
            let mut seen = Vec::new();
            for p in &frames {
                assert_eq!(p.hand_shapes, [HandShape::Relaxed; 2]);
                let flex = n * fingertip(p.hands[side]).x;
                assert!(flex > 0.005 && flex < 0.05, "side {side}: {flex}");
                seen.push(flex);
            }
            let (lo, hi) = seen
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            assert!(hi - lo > 0.005, "the wrist follows the sway: {lo} … {hi}");
        }
    }

    /// The other shapes: open hands with the arms up, the phone held in
    /// the right hand's fingers (the left on the bar), loose hands standing.
    #[test]
    fn the_hands_open_hold_the_phone_and_hang_loose() {
        let near = build(Detail::High);
        let rig = RiderRig::from_model(&near).unwrap();
        let bike = crate::shipped::bike_rig(ProfileId::Road);
        let body = BodyFit::new(&rig, &bike, 1.0);
        let up = pose(
            &body.rider,
            &body.bike,
            &body.fit,
            &PoseParams {
                arms_up: 1.0,
                ..Default::default()
            },
        );
        assert_eq!(up.hand_shapes, [HandShape::Open; 2]);
        assert_eq!(up.hands, [Quat::IDENTITY; 2]);
        let ph = pose(
            &body.rider,
            &body.bike,
            &body.fit,
            &PoseParams {
                phone: 1.0,
                one_hand: 1.0,
                ..Default::default()
            },
        );
        assert_eq!(ph.hand_shapes, [HandShape::Hoods, HandShape::Phone]);
        let (phone, _) = ph.phone.expect("the phone out");
        let m = posed(&near, &ph, 1.0);
        let w = m.world(m.node(nodes::HAND[1]).unwrap());
        let palm = w.transform_point3(Vec3::new(0.0, hand::PALM_Y, 0.0));
        let grasp = w.transform_point3(hand::bar_centre(HandShape::Phone, 1).unwrap());
        assert!(
            phone.distance(grasp) < 0.003,
            "the phone where the fingers close"
        );
        assert!(phone.distance(palm) < 0.05, "beside the palm");
        for d in digits(&m, 1, HandShape::Phone).iter().take(4) {
            assert!(
                d.last().unwrap().distance(phone) < 0.06,
                "a finger round the phone"
            );
        }
        let idle = idle(&rig, 1.0);
        assert_eq!(idle.hand_shapes, [HandShape::Relaxed; 2]);
        // Every shape's node exists on both hands of the near model, none
        // on the far one (its mitten).
        let low = build(Detail::Low);
        for shape in HandShape::ALL {
            for side in 0..2 {
                let n = hand::node(shape, side);
                assert!(near.node(n).is_some(), "{n}");
                assert!(low.node(n).is_none(), "{n} far");
                assert_eq!(hand::of_node(n), Some((shape, side)));
            }
        }
    }
}
