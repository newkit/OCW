//! Around riders (spec 14 Phase 18, 18B *Around riders*; milestone T4):
//! the legal passing distance of the region, the drivers' manners (Q8)
//! and the lunatics (Q4), the sight distance along the road, the pass
//! manoeuvre's length, the lorry gust, the collision bodies of the
//! vehicles and the events the server hands to the riders' physics, the
//! audio and the HUD.
//!
//! Everything here is pure and deterministic; the per-tick logic that
//! uses it lives in [`super::TrafficSim`] (`step`).

use ocw_world::RoadNetwork;
use ocw_world::graph::RoadClass;
pub use ocw_world::traffic::Country;

use super::kin::{RoutePos, step_len};
use super::plan::{Step, Tick};
use super::rules::VehicleClass;

/// Half the width of a rider and bike (m; 12B's footprint).
pub const RIDER_HALF_W: f64 = crate::collide::BODY_WIDTH_M / 2.0;
/// The closest any driver of Q8 (c) *some realism* passes (m, between
/// the vehicle's side and the rider's bars): "a close pass that never
/// touches".
pub const PASS_FLOOR_M: f64 = 1.0;
/// A vehicle closing in from behind within this many seconds gives the
/// proximity cue (12D, 18B *Autodrive and bots*).
pub const PROXIMITY_S: f64 = 4.0;
/// … and only within this distance (m).
pub const PROXIMITY_M: f64 = 120.0;
/// Riders closer together than this (m, rear wheel to front wheel) plus
/// the vehicle's own length and twice its cut-in gap are one block: a
/// vehicle never cuts in between them (18B: a group is overtaken as one).
pub const BUNCH_EXTRA_M: f64 = 4.0;
/// How far beside the road a driver sees (m beyond the road edge, on
/// both sides): verges, not hedges or forest – a generous guess, since
/// the package has no line-of-sight data.
pub const VERGE_M: f64 = 1.5;
/// The driver's eye and the oncoming car's roof above the road (m).
pub const EYE_M: f64 = 1.1;
pub const OBJECT_M: f64 = 1.1;
/// Farther than this the sight distance is not measured (m).
pub const MAX_SIGHT_M: f64 = 600.0;
/// A vehicle stays this far inside the road edge while it pulls out (m).
pub const EDGE_KEEP_M: f64 = 0.2;

/// The legal passing distance between a motor vehicle and a rider (m;
/// spec 18B *Overtaking*): DE (StVO 5 (4)) and AT (StVO 15 (4), since
/// 2022) 1.5 m in town and 2 m outside; ES 1.5 m (Reglamento General de
/// Circulación 85); CH and LI "sufficient" – we use 1.5 m; AD 1.5 m; FR
/// 1 m in town, 1.5 m outside (R414-4); elsewhere 1.5 m.
pub fn legal_pass_m(country: Country, urban: bool) -> f64 {
    match country {
        Country::De | Country::At => {
            if urban {
                1.5
            } else {
                2.0
            }
        }
        Country::Fr => {
            if urban {
                1.0
            } else {
                1.5
            }
        }
        Country::Es | Country::Ch | Country::Li | Country::Ad | Country::Other => 1.5,
    }
}

/// What the law says about passing riders on one edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassRule {
    pub country: Country,
    /// In a built-up area.
    pub urban: bool,
    /// The legal lateral distance ([`legal_pass_m`]).
    pub distance_m: f64,
    /// Overtaking forbidden forward / backward (`overtaking=no`, solid
    /// lines). In Spain a car may cross the solid line to pass a rider, so
    /// there it never forbids passing riders ([`Self::may_pass`]).
    pub no_overtaking: [bool; 2],
}

impl PassRule {
    pub fn new(country: Country, urban: bool, no_overtaking: [bool; 2]) -> Self {
        Self {
            country,
            urban,
            distance_m: legal_pass_m(country, urban),
            no_overtaking,
        }
    }

    /// Without the package's `TRFC` data: the German rule, in town by the
    /// road class (residential and service roads) or a limit of 50 km/h
    /// or less.
    pub fn implied(class: RoadClass, speed_ms: f64) -> Self {
        let urban = matches!(class, RoadClass::Residential | RoadClass::Service)
            || speed_ms <= 50.0 / 3.6 + 0.01;
        Self::new(Country::De, urban, [false, false])
    }

    /// Whether a rider may be passed through the oncoming half in this
    /// direction (ES: crossing the solid line to pass a rider is allowed).
    pub fn may_pass(&self, forward: bool) -> bool {
        self.country == Country::Es || !self.no_overtaking[usize::from(!forward)]
    }
}

/// How one driver behaves around riders (from the session's manners, Q8,
/// and the lunatic level, Q4). Part of the server's vehicle, not of the
/// plan (clients only see the result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manner {
    /// The distance this driver keeps when passing (m); `None` = the
    /// legal distance of the road.
    pub pass_m: Option<f64>,
    /// Extra time (s) kept free before an oncoming vehicle at the end of
    /// a pass, and added to the sight distance.
    pub margin_s: f64,
    /// Overtakes blind: no sight-distance check, no junction check, no
    /// `overtaking=no` (the lunatics).
    pub blind: bool,
    /// Starts a pass when an oncoming vehicle is farther than this share
    /// of what is needed (1 = all of it; a lunatic misjudges).
    pub judge: f64,
    /// Cut-in gap ahead of the rider block before moving back (m, plus
    /// [`Self::cut_in_s`] × speed).
    pub cut_in_m: f64,
    pub cut_in_s: f64,
    /// Lateral speed when pulling out and back in (m/s).
    pub shift_rate: f64,
    /// Seconds of waiting behind a rider before a short honk
    /// (`f64::INFINITY`: never).
    pub patience_s: f64,
    /// A reckless driver (Q4).
    pub lunatic: bool,
}

impl Manner {
    /// Polite and legal: the full distance, a free oncoming lane with a
    /// wide margin, patient (Q8 (a)).
    pub const POLITE: Manner = Manner {
        pass_m: None,
        margin_s: 2.5,
        blind: false,
        judge: 1.0,
        cut_in_m: 8.0,
        cut_in_s: 0.6,
        shift_rate: 1.2,
        patience_s: f64::INFINITY,
        lunatic: false,
    };

    /// The driver of vehicle `u`: `u(salt)` is the driver's own random
    /// number in [0, 1) for a salt (hashed from the seed and the id).
    pub fn of(manners: super::DriverManners, lunatic: bool, u: impl Fn(u64) -> f64) -> Self {
        use super::DriverManners as M;
        if lunatic {
            // Q4: blind crests and bends, too close, oncoming traffic
            // misjudged, cutting back in.
            return Manner {
                pass_m: Some(0.3 + 0.6 * u(11)),
                margin_s: 0.3,
                blind: true,
                judge: 0.45 + 0.2 * u(12),
                cut_in_m: 1.0 + 2.0 * u(13),
                cut_in_s: 0.0,
                shift_rate: 2.5,
                patience_s: 4.0 + 6.0 * u(14),
                lunatic: true,
            };
        }
        match manners {
            M::Polite => Self::POLITE,
            // Q8 (b): legal, a little less patient, an occasional short
            // honk after waiting long.
            M::Honking => Manner {
                margin_s: 1.8,
                cut_in_m: 6.0,
                cut_in_s: 0.5,
                shift_rate: 1.4,
                patience_s: 40.0 + 50.0 * u(15),
                ..Self::POLITE
            },
            // Q8 (c): most drivers like (b); a share pass closer (never
            // under the floor), another share squeezes past in marginal
            // sight.
            M::Realistic => {
                let close = u(16) < manners.close_pass_share();
                let squeeze = !close && u(17) < manners.close_pass_share();
                Manner {
                    pass_m: close.then(|| PASS_FLOOR_M + 0.4 * u(18)),
                    margin_s: if squeeze { 0.6 } else { 1.5 },
                    cut_in_m: if close { 3.0 } else { 5.0 },
                    cut_in_s: 0.4,
                    shift_rate: 1.6,
                    patience_s: 30.0 + 40.0 * u(19),
                    ..Self::POLITE
                }
            }
        }
    }

    /// The lateral distance this driver keeps on a road whose legal
    /// distance is `legal` (m).
    pub fn distance(&self, legal: f64) -> f64 {
        match self.pass_m {
            Some(d) if self.lunatic => d,
            // A close passer never keeps more than the law asks.
            Some(d) => d.min(legal),
            None => legal,
        }
    }
}

/// The time and the distances of a pass (m, s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassPlan {
    /// The distance to gain on the block (from the vehicle's front behind
    /// it to its rear past the block plus the cut-in gap).
    pub rel_m: f64,
    pub time_s: f64,
    /// How far the vehicle travels meanwhile.
    pub travel_m: f64,
}

/// The pass of a block moving at `v_block` from `v_now` with acceleration
/// `accel` up to `v_pass`, gaining `rel_m` metres, after `lead_s`
/// seconds of pulling out at the block's speed. `None` when the vehicle
/// is not faster than the block.
pub fn pass_plan(
    v_now: f64,
    v_pass: f64,
    v_block: f64,
    accel: f64,
    rel_m: f64,
    lead_s: f64,
) -> Option<PassPlan> {
    if v_pass < v_block + 1.0 || accel <= 0.0 {
        return None;
    }
    let v0 = v_now.min(v_pass);
    let dv0 = v0 - v_block;
    let t_acc = (v_pass - v0) / accel;
    let rel_acc = dv0 * t_acc + 0.5 * accel * t_acc * t_acc;
    let t = if rel_acc >= rel_m {
        // Solves dv0 t + a t² / 2 = rel.
        let disc = dv0 * dv0 + 2.0 * accel * rel_m;
        (-dv0 + disc.max(0.0).sqrt()) / accel
    } else {
        t_acc + (rel_m - rel_acc) / (v_pass - v_block)
    };
    let time_s = t + lead_s;
    Some(PassPlan {
        rel_m,
        time_s,
        travel_m: v_block * time_s + rel_m,
    })
}

/// The sight distance ahead along a route (m, up to `need`, at most
/// [`MAX_SIGHT_M`]): where the line from the driver's eye ([`EYE_M`]) to
/// an oncoming car's roof ([`OBJECT_M`]) leaves the road and its verges
/// (`half` m either side of the centreline: a bend) or passes under the
/// road surface (a crest). Sampled every 5 m.
pub fn sight_distance(
    net: &RoadNetwork,
    route: &[Step],
    pos: RoutePos,
    need: f64,
    half: f64,
) -> f64 {
    const STEP_M: f64 = 5.0;
    let need = need.min(MAX_SIGHT_M);
    // (distance ahead, x, y, z)
    let mut pts: Vec<[f64; 4]> = Vec::with_capacity((need / STEP_M) as usize + 3);
    let mut base = -pos.s;
    let mut k = pos.step;
    let mut d = 0.0;
    'outer: while let Some(st) = route.get(k) {
        let len = step_len(net, st);
        let g = net.geometry(st.edge as usize);
        while d <= base + len {
            let along = d - base;
            let s = if st.forward { along } else { len - along };
            let p = g.point_at(s);
            pts.push([d, p[0], p[1], p[2]]);
            if d >= need {
                break 'outer;
            }
            d += STEP_M;
        }
        base += len;
        k += 1;
    }
    let Some(&first) = pts.first() else {
        return 0.0;
    };
    let reach = pts.last().map_or(0.0, |p| p[0]);
    let half2 = half * half;
    for j in 2..pts.len() {
        let pj = pts[j];
        let (cx, cz) = (pj[1] - first[1], pj[3] - first[3]);
        let c2 = cx * cx + cz * cz;
        if c2 < 1e-6 {
            continue;
        }
        let (y0, y1) = (first[2] + EYE_M, pj[2] + OBJECT_M);
        for pi in &pts[1..j] {
            // Horizontal: the point's distance from the chord.
            let (px, pz) = (pi[1] - first[1], pi[3] - first[3]);
            let cross = px * cz - pz * cx;
            if cross * cross > half2 * c2 {
                return pts[j - 1][0];
            }
            // Vertical: the road above the line of sight.
            let t = (pi[0] - first[0]) / (pj[0] - first[0]);
            if pi[2] > y0 + (y1 - y0) * t {
                return pts[j - 1][0];
            }
        }
    }
    reach.min(need)
}

/// The size of a vehicle's collision body (m: length, width) – the width
/// from the `col_box` proxies of the class's model file (spec 18B: a box
/// from the model's `col_` proxies; T6), the length the core's.
pub fn body_size(class: VehicleClass) -> (f64, f64) {
    static WIDTHS: std::sync::OnceLock<[f64; 6]> = std::sync::OnceLock::new();
    let w = WIDTHS.get_or_init(|| {
        use ocw_assets::vehicles::Vehicle as M;
        let file = |c: VehicleClass| match c {
            VehicleClass::Car => M::CarSaloon,
            VehicleClass::Van => M::VanPanel,
            VehicleClass::Lorry => M::LorryRigid,
            VehicleClass::Bus => M::BusCity,
            VehicleClass::Motorbike => M::MotorbikeTouring,
            VehicleClass::Tractor => M::TractorFarm,
        };
        VehicleClass::ALL.map(|c| {
            let m = ocw_assets::shipped::model(ocw_assets::kinds::id_of(file(c).file()));
            let w = ocw_assets::props::proxies(&m)
                .iter()
                .filter(|p| p.name.starts_with("col_box"))
                .map(|p| f64::from(p.half.x) * 2.0)
                .fold(0.0, f64::max);
            // The body without its mirrors: at most the legal 2.55 m.
            if w > 0.3 {
                w.min(MAX_WIDTH_M)
            } else {
                default_width(c)
            }
        })
    });
    (class.traits().length_m, w[class.index()])
}

/// The widest a road vehicle may be (m; EU 2.55 m).
pub const MAX_WIDTH_M: f64 = 2.55;

fn default_width(c: VehicleClass) -> f64 {
    match c {
        VehicleClass::Car => 1.8,
        VehicleClass::Van => 2.0,
        VehicleClass::Lorry | VehicleClass::Bus | VehicleClass::Tractor => 2.5,
        VehicleClass::Motorbike => 0.9,
    }
}

/// How strongly a class's air wave acts on a rider beside it (the share
/// of the dynamic pressure on the rider's side area at 1 m).
pub fn gust_coefficient(class: VehicleClass) -> f64 {
    match class {
        VehicleClass::Lorry | VehicleClass::Bus => 0.5,
        VehicleClass::Tractor => 0.35,
        VehicleClass::Van => 0.25,
        VehicleClass::Car => 0.15,
        VehicleClass::Motorbike => 0.05,
    }
}

/// Air density (kg/m³), a rider's side and front area (m²).
const RHO: f64 = 1.2;
const SIDE_AREA_M2: f64 = 0.7;
const FRONT_AREA_M2: f64 = 0.4;
/// The distance scale of the wave's decay sideways (m).
const GUST_D0_M: f64 = 1.0;

/// The air push of a vehicle passing a rider (spec 18B *Slipstream and
/// wind*): `rel_speed` (m/s) between them, `clearance` (m) between the
/// vehicle's side and the rider's bars, `x` the rider's centre ahead of
/// the vehicle's front (m; negative = beside or behind it) and `length`
/// the vehicle's. Returns (lateral, drag) in newtons: lateral positive =
/// away from the vehicle (the bow wave pushes, the wake behind the rear
/// pulls back towards it), drag positive = against the rider's motion.
/// For a vehicle going the same way the drag change is a short push at
/// the bow and an equal pull in the wake – its sum over a pass is zero,
/// so there is no drafting behind vehicles (18B); for an oncoming one
/// (`oncoming`) it is a short headwind only. Only `+ − × ÷`.
pub fn gust(
    class: VehicleClass,
    rel_speed: f64,
    clearance: f64,
    x: f64,
    length: f64,
    oncoming: bool,
) -> (f64, f64) {
    let c = clearance.max(0.05);
    let decay = GUST_D0_M / (GUST_D0_M + c);
    let k = gust_coefficient(class) * decay * decay;
    let q = 0.5 * RHO * rel_speed * rel_speed;
    let bow = 1.0 / (1.0 + (x / 2.0) * (x / 2.0));
    let xr = x + length;
    // Two thirds of the bow's peak over a 1.5 × wider bell: the same area.
    let wake = (2.0 / 3.0) / (1.0 + (xr / 3.0) * (xr / 3.0));
    let lateral = q * SIDE_AREA_M2 * k * (bow - wake);
    let drag = if oncoming {
        q * FRONT_AREA_M2 * k * 0.5 * (bow + wake)
    } else {
        q * FRONT_AREA_M2 * k * (bow - wake)
    };
    (lateral, drag)
}

/// What happened between a vehicle and a rider (for the server's riders,
/// the audio and the HUD; T5 sends what the clients need).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiderEvent {
    /// A vehicle finished passing a rider (its rear went past the rider's
    /// front wheel).
    Passed(PassRecord),
    /// A vehicle and a rider touched (only with collisions on).
    Contact(VehicleContact),
    /// A short honk at a rider (Q8 (b), (c); lunatics).
    Honk {
        tick: Tick,
        vehicle: u32,
        rider: u32,
        class: VehicleClass,
    },
    /// A vehicle closes in from behind within [`PROXIMITY_S`] (the
    /// proximity cue of 12D: a sound and a HUD arrow).
    Approaching {
        tick: Tick,
        vehicle: u32,
        rider: u32,
        class: VehicleClass,
        /// Seconds until it reaches the rider at the speeds now.
        seconds: f64,
        /// Metres between its front and the rider's rear wheel.
        gap_m: f64,
    },
}

/// One overtake of a rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassRecord {
    pub tick: Tick,
    pub vehicle: u32,
    pub rider: u32,
    pub class: VehicleClass,
    /// The smallest lateral gap while beside the rider (m, the vehicle's
    /// side to the rider's bars).
    pub clearance_m: f64,
    /// The legal distance of the road (m).
    pub legal_m: f64,
    /// The vehicle's speed minus the rider's when it passed (m/s).
    pub rel_speed: f64,
    pub lunatic: bool,
    /// Pulled into the oncoming half to pass.
    pub oncoming_half: bool,
}

/// How a vehicle and a rider touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactSide {
    /// The vehicle's front into the rider's back (a car that could not
    /// stop in time, Q4).
    Rear,
    /// Side by side (a vehicle cutting in, a rider steering into it).
    Side,
    /// Opposite directions (a rider in the oncoming lane).
    HeadOn,
}

/// A contact between a vehicle and a rider (12B with vehicles, Q4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleContact {
    pub tick: Tick,
    pub vehicle: u32,
    pub rider: u32,
    pub class: VehicleClass,
    pub side: ContactSide,
    /// The closing speed along the contact normal (m/s, ≥ 0).
    pub closing_mps: f64,
    /// The vehicle's velocity relative to the rider in the rider's frame
    /// (m/s: along the rider's heading, to the rider's right).
    pub relative: [f64; 2],
    /// Which way the rider is pushed in its frame (+1 = to its right).
    pub push_side: f64,
}

/// The air of passing vehicles on one rider this tick (summed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gust {
    pub rider: u32,
    /// The strongest vehicle's id (for the sound).
    pub vehicle: u32,
    /// Sideways force on rider and bike (N, positive = to the rider's right).
    pub lateral_n: f64,
    /// Extra aerodynamic drag (N, positive = against the motion).
    pub drag_n: f64,
}

impl Gust {
    /// The lean the push gives a rider and bike of `mass` kg (rad): the
    /// lateral acceleration over g, as the weather's gusts do.
    pub fn lean_rad(&self, mass: f64) -> f64 {
        self.lateral_n / (mass.max(30.0) * 9.81)
    }
}

/// A vehicle as a solid body in the world (for the client's prediction
/// and the collision model of 12B: a box from the model's `col_box`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleBody {
    pub id: u32,
    pub class: VehicleClass,
    /// The centre of the box (x, z).
    pub pos: [f64; 2],
    /// Unit heading (x, z).
    pub forward: [f64; 2],
    pub half_length: f64,
    pub half_width: f64,
    /// Velocity (m/s, x, z).
    pub velocity: [f64; 2],
}
