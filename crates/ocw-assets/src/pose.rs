//! Putting the rider on the bike (spec §2.6: "bicycle and character must be
//! correctly attached"): the rider rig and the bike's anchors are read from
//! the models; the pose places the hips on the saddle (or over the bike
//! when standing), the hands on the bars and the feet on the pedals with
//! two-bone inverse kinematics. Pure and Bevy-free; the app writes the
//! result into the rider's node transforms each frame.

use glam::{Mat3, Quat, Vec3};

use crate::model::Model;

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

    /// Every node a rider model must have.
    pub const REQUIRED: [&str; 25] = [
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
        HELMET_ROAD,
        HELMET_AERO,
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
        }
    }

    pub fn arm(&self) -> f32 {
        self.upper_arm + self.forearm
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
}

impl Pose {
    /// `(node, local rotation)` for every posed node except the pelvis.
    pub fn rotations(&self) -> [(&'static str, Quat); 15] {
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
    let pelvis_rot = frame * pelvis_tilt(lean) * rock;
    let pelvis_pos = frame * hip;
    // The shoulders turn a little with the head and drop to the inside of
    // a bend.
    let torso_world = frame
        * Quat::from_rotation_y(-LOOK_TORSO * p.look)
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
    for s in 0..2 {
        let shoulder = torso_pos + torso_world * rider.shoulders[s];
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let mut pole = if bike.aero_bars {
            Vec3::new(0.0, -1.0, 0.25)
        } else {
            out * 0.7 + Vec3::new(0.0, -0.35, 0.6)
        };
        let mut grip = grips[s];
        if arms_up > 0.0 {
            // Up in a V above the shoulders, the elbows out.
            let up = shoulder + Vec3::new(out.x * 0.45, 0.88, 0.1).normalize() * rider.arm() * 0.93;
            grip = grip.lerp(up, arms_up);
            pole = pole.lerp(out * 0.8 + Vec3::new(0.0, 0.0, 0.5), arms_up);
        }
        let (elbow, end) = two_bone(shoulder, grip, rider.upper_arm, rider.forearm, pole);
        let hinge = (end - shoulder)
            .cross(elbow - shoulder)
            .normalize_or(Vec3::X);
        let upper_w = segment_rotation(elbow - shoulder, hinge);
        let fore_w = segment_rotation(end - elbow, hinge);
        upper_arms[s] = torso_world.inverse() * upper_w;
        forearms[s] = upper_w.inverse() * fore_w;
    }
    // Head: looking up the road, a little down; turned by `look`, held
    // nearer level in a lean, dropped when sitting up after the line.
    let head_world = frame
        * Quat::from_rotation_y(-p.look)
        * Quat::from_rotation_z(p.head_roll)
        * Quat::from_rotation_x(-(0.12 + 0.12 * lean) - 0.25 * p.sit_up - 0.12 * p.throw);
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
    }
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
    let torso_world = Quat::from_rotation_z(-0.012 * shift) * Quat::from_rotation_x(0.014 * breath);
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
    for s in 0..2 {
        let side = if s == 0 { -1.0 } else { 1.0 };
        let sway = wave(6.1, 1.3 + s as f32 * 2.1);
        let abduct = 0.11 + 0.008 * breath;
        let forward = 0.07 + 0.025 * sway;
        upper_arms[s] = Quat::from_rotation_z(side * abduct) * Quat::from_rotation_x(forward);
        forearms[s] = Quat::from_rotation_x(0.26 + 0.03 * sway);
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
        head: Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch),
        thighs,
        shins,
        feet,
        upper_arms,
        forearms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                        let grip = rider.position(nodes::GRIP[s]).unwrap();
                        assert!(
                            grip.distance(bike.hoods[s]) < 0.04,
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
                            let grip = posed.position(nodes::GRIP[side]).unwrap() * s;
                            assert!(
                                grip.distance(b.bike.hoods[side]) < 0.04,
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
                    let grip = rider.position(nodes::GRIP[s]).unwrap();
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
                let grip = rider.position(nodes::GRIP[s]).unwrap();
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
                let grip = rider.position(nodes::GRIP[s]).unwrap();
                assert!(
                    grip.distance(bike.hoods[s]) < 0.05,
                    "{profile:?}: throw hands {grip}"
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
