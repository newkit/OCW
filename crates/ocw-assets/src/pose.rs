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
}

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
        let lim = MAX_SADDLE_OFFSET_M;
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
    let roll = p.rock * theta.sin();
    // The rider stays above the contact line while the bike rocks under them.
    let frame = Quat::from_rotation_z(-0.8 * roll);
    let hip = fit.hip_seated.lerp(fit.hip_standing, stand);
    // Seated: the hips rock a little with the pedal strokes.
    let rock = Quat::from_rotation_z(0.035 * (1.0 - stand) * theta.sin());
    let w_hoods = (1.0 - p.tops - p.drops).max(0.0);
    let grips: [Vec3; 2] = std::array::from_fn(|s| {
        bike.hoods[s] * w_hoods + bike.tops[s] * p.tops + bike.drops[s] * p.drops
    });
    let grip_centre = 0.5 * (grips[0] + grips[1]);
    let reach = rider.arm() * (ARM_REACH - 0.1 * p.tuck - 0.04 * stand);
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
    let pelvis_rot = frame * pelvis_tilt(lean) * rock;
    let pelvis_pos = frame * hip;
    let torso_world = frame * Quat::from_rotation_x(-lean) * rock.inverse();
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
        let foot_world = Quat::from_rotation_x(pitch);
        let hip_joint = pelvis_pos + pelvis_rot * rider.hips[s];
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let (knee, end) = two_bone(
            hip_joint,
            ankle(rider, pedal, pitch),
            rider.thigh,
            rider.shin,
            forward + out * 0.08,
        );
        let hinge = (end - hip_joint)
            .cross(knee - hip_joint)
            .normalize_or(Vec3::X);
        let thigh_w = segment_rotation(knee - hip_joint, hinge);
        let shin_w = segment_rotation(end - knee, hinge);
        thighs[s] = pelvis_rot.inverse() * thigh_w;
        shins[s] = thigh_w.inverse() * shin_w;
        feet[s] = shin_w.inverse() * foot_world;
        pedals[s] = crank.inverse() * foot_world;
    }
    // Arms.
    let mut upper_arms = [Quat::IDENTITY; 2];
    let mut forearms = [Quat::IDENTITY; 2];
    for s in 0..2 {
        let shoulder = torso_pos + torso_world * rider.shoulders[s];
        let out = if s == 0 { -Vec3::X } else { Vec3::X };
        let pole = if bike.aero_bars {
            Vec3::new(0.0, -1.0, 0.25)
        } else {
            out * 0.7 + Vec3::new(0.0, -0.35, 0.6)
        };
        let (elbow, end) = two_bone(shoulder, grips[s], rider.upper_arm, rider.forearm, pole);
        let hinge = (end - shoulder)
            .cross(elbow - shoulder)
            .normalize_or(Vec3::X);
        let upper_w = segment_rotation(elbow - shoulder, hinge);
        let fore_w = segment_rotation(end - elbow, hinge);
        upper_arms[s] = torso_world.inverse() * upper_w;
        forearms[s] = upper_w.inverse() * fore_w;
    }
    // Head: looking up the road, a little down.
    let head_world = frame * Quat::from_rotation_x(-(0.12 + 0.12 * lean));
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

#[cfg(test)]
mod tests {
    use super::*;

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
