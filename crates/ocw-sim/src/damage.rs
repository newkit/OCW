//! Damage (spec §14 Phase 12E, K4): the rider-condition (injury) and
//! bike-condition meters (0–100 %), what a hit takes off them by its
//! energy and type, the injury levels and their effects on the
//! **virtual** rider (the real rider's power is never touched), the bike's
//! parts and their effects (a buckled wheel, a puncture, the drivetrain, the
//! handlebar, a broken bike), the stops by the road (medical stop, puncture
//! and chain repairs, a spare bike, the organiser's neutral service), the
//! recovery while riding easily, the healing over real time between rides
//! (Phase 12 Q6: damage and injuries carry over) and the workshop's repair
//! price (§14 Phase 4, 4E, W6).
//!
//! Pure and deterministic (no random numbers: a puncture or a broken chain
//! follows from the part's condition after the hit), so the server, replays
//! and the client agree; builds for wasm32. The server owns a [`Damage`] per
//! rider ([`crate::session::Session`]); the clients get it in
//! `RiderUpdate.damage` and take the same [`Effects`] for their prediction.

use ocw_core::RiderProfile;

use crate::collide::{ContactKind, DOWN_REF_ENERGY_J, Hit};

/// Number of bike parts with their own condition.
pub const PARTS: usize = 7;

/// A part of the bike with its own condition (12E), chosen by the impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    FrontWheel,
    RearWheel,
    FrontTyre,
    RearTyre,
    /// Derailleur and chain.
    Drivetrain,
    /// Handlebar, stem and brakes.
    Cockpit,
    /// Frame and fork.
    Frame,
}

impl Part {
    pub const ALL: [Part; PARTS] = [
        Part::FrontWheel,
        Part::RearWheel,
        Part::FrontTyre,
        Part::RearTyre,
        Part::Drivetrain,
        Part::Cockpit,
        Part::Frame,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Its bit in `DamageState.parts`.
    pub fn bit(self) -> u32 {
        1 << self.index()
    }

    /// The stable id (files, console).
    pub fn id(self) -> &'static str {
        match self {
            Part::FrontWheel => "front_wheel",
            Part::RearWheel => "rear_wheel",
            Part::FrontTyre => "front_tyre",
            Part::RearTyre => "rear_tyre",
            Part::Drivetrain => "drivetrain",
            Part::Cockpit => "cockpit",
            Part::Frame => "frame",
        }
    }

    pub fn from_id(id: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::FrontWheel => "front wheel",
            Part::RearWheel => "rear wheel",
            Part::FrontTyre => "front tyre",
            Part::RearTyre => "rear tyre",
            Part::Drivetrain => "drivetrain",
            Part::Cockpit => "handlebar",
            Part::Frame => "frame",
        }
    }

    /// Its weight in the bike condition (the weights sum to 1).
    pub fn weight(self) -> f64 {
        match self {
            Part::FrontWheel | Part::RearWheel | Part::Drivetrain => 0.15,
            Part::FrontTyre | Part::RearTyre => 0.075,
            Part::Cockpit => 0.1,
            Part::Frame => 0.3,
        }
    }

    /// The workshop slot it belongs to (spec §14 Phase 4, 4B) – its repair
    /// is priced from the part on that slot.
    pub fn slot(self) -> &'static str {
        match self {
            Part::FrontWheel | Part::RearWheel => "wheels",
            Part::FrontTyre | Part::RearTyre => "tyres",
            Part::Drivetrain => "groupset",
            Part::Cockpit => "cockpit",
            Part::Frame => "frame",
        }
    }

    /// Its share of the slot's price: wheels and tyres are sold in pairs.
    pub fn price_share(self) -> f64 {
        match self {
            Part::FrontWheel | Part::RearWheel | Part::FrontTyre | Part::RearTyre => 0.5,
            _ => 1.0,
        }
    }

    /// Frames and wheels may be beyond repair (below [`BEYOND_REPAIR`]):
    /// they are replaced, at the full price.
    pub fn can_be_beyond_repair(self) -> bool {
        matches!(self, Part::Frame | Part::FrontWheel | Part::RearWheel)
    }
}

/// Injury levels by rider condition (12E).
pub const HURT_BELOW: f64 = 0.70;
pub const INJURED_BELOW: f64 = 0.40;
pub const BADLY_INJURED_BELOW: f64 = 0.15;
/// A wheel below this is buckled; a handlebar below it steers less precisely.
pub const BUCKLED_BELOW: f64 = 0.80;
pub const BENT_BAR_BELOW: f64 = 0.80;
/// A tyre hit below this has a puncture.
pub const PUNCTURE_BELOW: f64 = 0.70;
/// A drivetrain hit in a crash below this breaks the chain.
pub const CHAIN_BREAKS_BELOW: f64 = 0.35;
/// Below this bike condition the bike is broken (12E).
pub const BROKEN_BELOW: f64 = 0.10;
/// Frames and wheels below this are beyond repair (4E).
pub const BEYOND_REPAIR: f64 = 0.10;

/// The stops (s, 12E).
pub const MEDICAL_STOP_S: f64 = 60.0;
pub const PUNCTURE_REPAIR_S: f64 = 30.0;
pub const CHAIN_REPAIR_S: f64 = 45.0;
pub const SPARE_BIKE_S: f64 = 60.0;
/// Standing still this long at the organiser's neutral service restores
/// the bike (a "service" stop, 12E recovery).
pub const SERVICE_STOP_S: f64 = 30.0;
/// Standing still (below [`STOPPED_MPS`]) this long with a flat tyre
/// starts the puncture repair.
pub const FLAT_STOP_S: f64 = 2.0;
pub const STOPPED_MPS: f64 = 0.3;
/// Riding on a flat: rolling resistance ×4, at most 15 km/h.
pub const FLAT_CRR_FACTOR: f64 = 4.0;
pub const FLAT_MAX_MPS: f64 = 15.0 / 3.6;
/// After a medical stop a badly injured rider rides on at most this share
/// of the power.
pub const LIMITED_POWER: f64 = 0.5;
/// … until the rider is better than injured again.
pub const LIMIT_ENDS_ABOVE: f64 = INJURED_BELOW;
/// Recovery while riding easily (below [`EASY_SHARE`] of the FTP): 1 %
/// per minute (12E). The server has no FTP: it is estimated from the
/// rider's weight ([`FTP_W_PER_KG`]).
pub const RECOVERY_PER_S: f64 = 0.01 / 60.0;
pub const EASY_SHARE: f64 = 0.5;
pub const FTP_W_PER_KG: f64 = 2.5;
/// Healing over real time between rides (Phase 12 Q6): 10 % per hour, so
/// a night heals every injury.
pub const HEAL_PER_HOUR: f64 = 0.10;
/// The workshop's repair: this share of the price for the damage (4E).
pub const REPAIR_SHARE: f64 = 0.3;
/// A soft bump's damage reaches its 2 % at this energy (J: two 80 kg
/// riders meeting at 2.5 m/s, the crash threshold).
pub const BUMP_REF_ENERGY_J: f64 = 125.0;
/// A shoulder rub below this closing speed costs nothing (m/s).
pub const RUB_FREE_MPS: f64 = 0.5;

/// The status palette's classes (12F): always shown with an icon and a
/// word, never by colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    Good,
    Warning,
    Serious,
    Critical,
}

impl Status {
    /// Its colour (sRGB 0–1): green, amber, orange, red.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Status::Good => [0.42, 0.88, 0.45],
            Status::Warning => [0.98, 0.76, 0.18],
            Status::Serious => [0.98, 0.52, 0.12],
            Status::Critical => [0.92, 0.22, 0.20],
        }
    }

    /// Its icon (shapes of the bundled UI font).
    pub fn icon(self) -> &'static str {
        match self {
            Status::Good => "●",
            Status::Warning => "▲",
            Status::Serious => "◆",
            Status::Critical => "■",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Good => "good",
            Status::Warning => "warning",
            Status::Serious => "serious",
            Status::Critical => "critical",
        }
    }
}

/// The rider's injury level (12E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Injury {
    /// Not hurt at all (100 %).
    Fine,
    /// 100–70 %: no effect.
    Bruised,
    /// 70–40 %: 3–8 % less virtual power, longer down after a crash.
    Hurt,
    /// 40–15 %: 10–20 % less, every further crash takes longer.
    Injured,
    /// Below 15 %: a medical stop, then at most 50 % power.
    BadlyInjured,
}

impl Injury {
    pub fn of(rider: f64) -> Self {
        if rider >= 0.9995 {
            Injury::Fine
        } else if rider >= HURT_BELOW {
            Injury::Bruised
        } else if rider >= INJURED_BELOW {
            Injury::Hurt
        } else if rider >= BADLY_INJURED_BELOW {
            Injury::Injured
        } else {
            Injury::BadlyInjured
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Injury::Fine => "fine",
            Injury::Bruised => "bruised",
            Injury::Hurt => "hurt",
            Injury::Injured => "injured",
            Injury::BadlyInjured => "badly injured",
        }
    }

    pub fn status(self) -> Status {
        match self {
            Injury::Fine | Injury::Bruised => Status::Good,
            Injury::Hurt => Status::Warning,
            Injury::Injured => Status::Serious,
            Injury::BadlyInjured => Status::Critical,
        }
    }

    /// The wire code (`InjuryLevel`).
    pub fn code(self) -> u32 {
        self as u32
    }
}

/// The bike's level by bike condition (12F words).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BikeLevel {
    Fine,
    Scratched,
    Damaged,
    BadlyDamaged,
    Broken,
}

impl BikeLevel {
    pub fn of(bike: f64) -> Self {
        if bike >= 0.9995 {
            BikeLevel::Fine
        } else if bike >= HURT_BELOW {
            BikeLevel::Scratched
        } else if bike >= INJURED_BELOW {
            BikeLevel::Damaged
        } else if bike >= BROKEN_BELOW {
            BikeLevel::BadlyDamaged
        } else {
            BikeLevel::Broken
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BikeLevel::Fine => "fine",
            BikeLevel::Scratched => "scratched",
            BikeLevel::Damaged => "damaged",
            BikeLevel::BadlyDamaged => "badly damaged",
            BikeLevel::Broken => "broken",
        }
    }

    pub fn status(self) -> Status {
        match self {
            BikeLevel::Fine | BikeLevel::Scratched => Status::Good,
            BikeLevel::Damaged => Status::Warning,
            BikeLevel::BadlyDamaged => Status::Serious,
            BikeLevel::Broken => Status::Critical,
        }
    }
}

/// The status of one part by its condition (the workshop's hotspots, the
/// HUD's bike icon): good above 80 %, then warning, serious below 40 %,
/// critical below 10 %.
pub fn part_status(condition: f64) -> Status {
    if condition >= BUCKLED_BELOW {
        Status::Good
    } else if condition >= INJURED_BELOW {
        Status::Warning
    } else if condition >= BEYOND_REPAIR {
        Status::Serious
    } else {
        Status::Critical
    }
}

/// A stop by the road that holds the rider (12E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopKind {
    /// Badly injured: 60 s, then at most 50 % power (or DNF).
    Medical,
    /// Mending a puncture: 30 s.
    Puncture,
    /// Mending a broken chain: 45 s.
    Chain,
    /// The bike is broken: waiting 60 s for a spare bike.
    SpareBike,
}

impl StopKind {
    pub fn duration_s(self) -> f64 {
        match self {
            StopKind::Medical => MEDICAL_STOP_S,
            StopKind::Puncture => PUNCTURE_REPAIR_S,
            StopKind::Chain => CHAIN_REPAIR_S,
            StopKind::SpareBike => SPARE_BIKE_S,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StopKind::Medical => "medical stop",
            StopKind::Puncture => "mending the puncture",
            StopKind::Chain => "fixing the chain",
            StopKind::SpareBike => "waiting for a spare bike",
        }
    }

    /// The wire code (`DamageStop`).
    pub fn code(self) -> u32 {
        match self {
            StopKind::Medical => 1,
            StopKind::Puncture => 2,
            StopKind::Chain => 3,
            StopKind::SpareBike => 4,
        }
    }

    pub fn from_code(c: u32) -> Option<Self> {
        Some(match c {
            1 => StopKind::Medical,
            2 => StopKind::Puncture,
            3 => StopKind::Chain,
            4 => StopKind::SpareBike,
            _ => return None,
        })
    }
}

/// A stop under way: its time runs once the rider stands still.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub kind: StopKind,
    /// Seconds of it still to go.
    pub left_s: f64,
}

/// What the last hit took off (the HUD's animation, "−12 % bike").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loss {
    /// The server tick of the hit.
    pub tick: u64,
    /// Condition lost (0–1).
    pub rider: f64,
    pub bike: f64,
    /// The parts it damaged ([`Part::bit`]).
    pub parts: u32,
    pub crash: bool,
}

/// The organiser's damage rules for an event (12G, minimal K6 options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rules {
    /// `ride.damage` for the event (`None`: the session's switch).
    pub on: Option<bool>,
    /// A badly injured entrant is DNF instead of riding on after the
    /// medical stop.
    pub dnf_badly_injured: bool,
    /// Neutral service: a spare bike for a broken one (else DNF), and a
    /// 30 s stop restores the bike.
    pub neutral_service: bool,
}

/// What a rider's damage does to the physics now (the server and the
/// client's prediction use the same).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effects {
    /// Share of the power that reaches the virtual drivetrain (injury).
    pub power_factor: f64,
    /// Rolling resistance (buckled wheels, a flat tyre).
    pub crr_factor: f64,
    /// Drivetrain efficiency (a damaged drivetrain).
    pub efficiency_factor: f64,
    /// A flat tyre: at most this speed (m/s).
    pub speed_cap: Option<f64>,
    /// Manual steering's lean (a bent handlebar: −20 %); autodrive is
    /// unaffected.
    pub lean_factor: f64,
    /// Stopped by the road (a repair, a medical stop, a spare bike, a
    /// broken chain): the power has no effect, the rider brakes to a stop.
    pub held: bool,
}

impl Effects {
    /// No damage, or `ride.damage` off.
    pub const NONE: Effects = Effects {
        power_factor: 1.0,
        crr_factor: 1.0,
        efficiency_factor: 1.0,
        speed_cap: None,
        lean_factor: 1.0,
        held: false,
    };

    /// The profile the rider rides with.
    pub fn profile(&self, p: &RiderProfile) -> RiderProfile {
        RiderProfile {
            crr: p.crr * self.crr_factor,
            drivetrain_efficiency: p.drivetrain_efficiency * self.efficiency_factor,
            ..*p
        }
    }

    /// The power that counts for the virtual rider (the real rider's power
    /// is never touched: the trainer and the hearts see the real one).
    pub fn power(&self, watts: f64) -> f64 {
        if self.held {
            0.0
        } else {
            watts * self.power_factor
        }
    }

    pub fn is_none(&self) -> bool {
        *self == Effects::NONE
    }
}

/// Virtual power lost by injury (12E): none down to 70 %, 3–8 % when
/// hurt, 10–20 % when injured, 20 % when badly injured.
pub fn injury_power_loss(rider: f64) -> f64 {
    if rider >= HURT_BELOW {
        0.0
    } else if rider >= INJURED_BELOW {
        0.03 + 0.05 * (HURT_BELOW - rider) / (HURT_BELOW - INJURED_BELOW)
    } else if rider >= BADLY_INJURED_BELOW {
        0.10 + 0.10 * (INJURED_BELOW - rider) / (INJURED_BELOW - BADLY_INJURED_BELOW)
    } else {
        0.20
    }
}

/// Extra rolling resistance of a buckled wheel (12E: +10–40 %).
pub fn buckle_crr(wheel: f64) -> f64 {
    if wheel >= BUCKLED_BELOW {
        0.0
    } else {
        0.10 + 0.30 * ((BUCKLED_BELOW - wheel) / BUCKLED_BELOW).clamp(0.0, 1.0)
    }
}

/// Efficiency lost by a damaged drivetrain (12E: 2–10 %).
pub fn drivetrain_loss(drivetrain: f64) -> f64 {
    if drivetrain >= 0.95 {
        0.0
    } else {
        0.02 + 0.08 * ((0.95 - drivetrain) / 0.95).clamp(0.0, 1.0)
    }
}

/// How hard a contact type hits (head-on and the world hardest, 12E).
pub fn hardness(kind: ContactKind) -> f64 {
    match kind {
        ContactKind::HeadOn | ContactKind::World(_) => 1.0,
        ContactKind::RoadEdge => 0.8,
        ContactKind::Object => 0.6,
        ContactKind::RearEnd => 0.5,
        ContactKind::WheelTouch | ContactKind::WheelToWheel => 0.3,
        ContactKind::Side => 0.2,
    }
}

/// A crash's severity (0–1) by energy and type.
pub fn severity(hit: &Hit) -> f64 {
    let e = (hit.energy_j / DOWN_REF_ENERGY_J).clamp(0.0, 1.0);
    (0.65 * e + 0.35 * hardness(hit.kind)).clamp(0.0, 1.0)
}

/// What a hit takes off (rider, bike; 0–1), 12E: a soft bump 0–2 % each
/// (a shoulder rub nothing, a wheel touch a little; a fallen bike ridden
/// into or the road edge: the bike only), a crash 5–25 % rider and 5–30 %
/// bike by energy and type.
pub fn losses(hit: &Hit, crash: bool) -> (f64, f64) {
    if crash {
        let s = severity(hit);
        return (0.05 + 0.20 * s, 0.05 + 0.25 * s);
    }
    if hit.kind == ContactKind::Side && hit.closing_mps < RUB_FREE_MPS {
        return (0.0, 0.0);
    }
    let base = 0.02 * (hit.energy_j / BUMP_REF_ENERGY_J).clamp(0.0, 1.0);
    match hit.kind {
        ContactKind::Side => (0.5 * base, 0.5 * base),
        ContactKind::Object | ContactKind::RoadEdge => (0.0, base),
        ContactKind::World(_) => (0.5 * base, base),
        _ => (base, base),
    }
}

/// Where the impact was in the rider's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Rear,
    Left,
    Right,
}

/// The impact's side from the push (the velocity change in the rider's
/// frame: along the heading, to the right): slowed = front, pushed on =
/// rear, pushed sideways = the bike falls to that side.
pub fn impact_side(hit: &Hit) -> Side {
    let [along, right] = hit.impulse;
    if along.abs() >= right.abs() {
        if along <= 0.0 {
            Side::Front
        } else {
            Side::Rear
        }
    } else if right > 0.0 {
        Side::Right
    } else {
        Side::Left
    }
}

/// How a hit's bike damage spreads over the parts by its side (the shares
/// sum to 1; the derailleur is on the right).
pub fn shares(side: Side) -> &'static [(Part, f64)] {
    match side {
        Side::Front => &[
            (Part::FrontWheel, 0.40),
            (Part::FrontTyre, 0.15),
            (Part::Cockpit, 0.20),
            (Part::Frame, 0.25),
        ],
        Side::Rear => &[
            (Part::RearWheel, 0.40),
            (Part::RearTyre, 0.15),
            (Part::Drivetrain, 0.20),
            (Part::Frame, 0.25),
        ],
        Side::Right => &[
            (Part::Drivetrain, 0.35),
            (Part::RearWheel, 0.20),
            (Part::Cockpit, 0.15),
            (Part::FrontWheel, 0.10),
            (Part::RearTyre, 0.05),
            (Part::Frame, 0.15),
        ],
        Side::Left => &[
            (Part::Cockpit, 0.35),
            (Part::FrontWheel, 0.20),
            (Part::RearWheel, 0.15),
            (Part::FrontTyre, 0.05),
            (Part::Drivetrain, 0.10),
            (Part::Frame, 0.15),
        ],
    }
}

/// What the damage shows on the bike and the rider (12E, K5): simple and
/// cheap – scuffs, a bent wheel, a dropped chain, dust and a torn kit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Visible {
    /// Scratches and scuffs on the frame (0–1, by the bike's damage).
    pub scuffs: f32,
    /// How bent each wheel is (front, rear; 0 = true, 1 = badly buckled).
    pub bent: [f32; 2],
    pub flat: [bool; 2],
    /// The chain hangs off (broken).
    pub chain_dropped: bool,
    /// Dust on the rider (0–1, by the injury and the crashes).
    pub dust: f32,
    /// The kit is torn (hurt or worse).
    pub torn: bool,
}

impl Visible {
    pub fn any(&self) -> bool {
        *self != Visible::default()
    }
}

/// What happened in a [`Damage::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOut {
    Nothing,
    /// A stop began (it holds the rider).
    StopStarted(StopKind),
    /// A stop is over: riding again (a spare bike, a mended tyre …).
    StopDone(StopKind),
    /// The neutral service restored the bike.
    Serviced,
    /// The rider abandons the event (DNF): badly injured with the
    /// organiser's rule, or a broken bike without neutral service.
    Abandon,
}

/// The context of a damage tick (the server's rider this tick).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickCtx {
    pub dt: f64,
    pub speed: f64,
    /// The real power (W) and the rider's weight (the FTP estimate).
    pub power_w: f64,
    pub rider_kg: f64,
    /// A bot repairs a puncture at once.
    pub bot: bool,
    /// The rider rides in an event that is not over.
    pub in_event: bool,
    pub rules: Rules,
}

/// A rider's damage (12E): the meters, the parts, a flat or a broken chain,
/// the stop under way and the last hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Damage {
    /// Rider condition (0–1; injury).
    pub rider: f64,
    /// Every part's condition (0–1), by [`Part::index`].
    pub parts: [f64; PARTS],
    /// A flat tyre (front, rear).
    pub flat: [bool; 2],
    pub chain_broken: bool,
    pub stop: Option<Stop>,
    /// After a medical stop: at most [`LIMITED_POWER`].
    pub limited: bool,
    /// Riding a spare bike (the own one broke): its damage is not the
    /// garage bike's (the client keeps the broken bike's).
    pub spare: bool,
    /// Crashes in this ride.
    pub crashes: u32,
    /// Out of the event (DNF).
    pub abandoned: bool,
    pub last_hit: Option<Loss>,
    /// Server only: how long the rider has stood still (s).
    pub still_s: f64,
}

impl Default for Damage {
    fn default() -> Self {
        Self::fresh()
    }
}

impl Damage {
    /// A rider and bike without damage.
    pub fn fresh() -> Self {
        Self {
            rider: 1.0,
            parts: [1.0; PARTS],
            flat: [false; 2],
            chain_broken: false,
            stop: None,
            limited: false,
            spare: false,
            crashes: 0,
            abandoned: false,
            last_hit: None,
            still_s: 0.0,
        }
    }

    /// The condition carried over from earlier rides (Phase 12 Q6): the
    /// rider's injury and the bike's parts. A broken bike is ridden as a
    /// spare (a loaner, fresh) until it is repaired.
    pub fn carried(rider: f64, parts: [f64; PARTS]) -> Self {
        let mut d = Self::fresh();
        d.rider = rider.clamp(0.0, 1.0);
        for (p, c) in d.parts.iter_mut().zip(parts) {
            *p = if c.is_finite() {
                c.clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
        if d.bike() < BROKEN_BELOW {
            d.parts = [1.0; PARTS];
            d.spare = true;
        }
        d.limited = d.rider < BADLY_INJURED_BELOW;
        d
    }

    pub fn part(&self, p: Part) -> f64 {
        self.parts[p.index()]
    }

    /// Bike condition: the parts weighted ([`Part::weight`]).
    pub fn bike(&self) -> f64 {
        if self.parts.iter().all(|c| *c >= 1.0) {
            return 1.0;
        }
        Part::ALL
            .iter()
            .map(|p| p.weight() * self.part(*p))
            .sum::<f64>()
            .clamp(0.0, 1.0)
    }

    pub fn injury(&self) -> Injury {
        Injury::of(self.rider)
    }

    pub fn bike_level(&self) -> BikeLevel {
        BikeLevel::of(self.bike())
    }

    pub fn broken(&self) -> bool {
        self.bike() < BROKEN_BELOW
    }

    /// Nothing damaged at all.
    pub fn is_fresh(&self) -> bool {
        self.rider >= 1.0
            && self.parts.iter().all(|c| *c >= 1.0)
            && !self.flat.iter().any(|f| *f)
            && !self.chain_broken
            && self.stop.is_none()
    }

    /// The parts whose damage has an effect or shows a label (the HUD's
    /// bike icon): buckled wheels, flats, a damaged drivetrain or broken
    /// chain, a bent handlebar, a frame below 80 %.
    pub fn affected(&self) -> u32 {
        let mut bits = 0;
        for p in Part::ALL {
            let c = self.part(p);
            let hit = match p {
                Part::FrontWheel | Part::RearWheel => c < BUCKLED_BELOW,
                Part::FrontTyre => self.flat[0] || c < PUNCTURE_BELOW,
                Part::RearTyre => self.flat[1] || c < PUNCTURE_BELOW,
                Part::Drivetrain => self.chain_broken || c < 0.95,
                Part::Cockpit => c < BENT_BAR_BELOW,
                Part::Frame => c < BUCKLED_BELOW,
            };
            if hit {
                bits |= p.bit();
            }
        }
        bits
    }

    /// The short labels of the damaged parts (12F: "rear wheel buckled",
    /// "puncture: stop to repair").
    pub fn labels(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, wheel) in [Part::FrontWheel, Part::RearWheel].into_iter().enumerate() {
            if self.flat[i] {
                let tyre = if i == 0 { "front" } else { "rear" };
                out.push(match self.stop {
                    Some(Stop {
                        kind: StopKind::Puncture,
                        ..
                    }) => format!("{tyre} puncture: mending"),
                    _ => format!("{tyre} puncture: stop to repair"),
                });
            }
            if self.part(wheel) < BUCKLED_BELOW {
                out.push(format!("{} buckled", wheel.label()));
            }
        }
        if self.chain_broken {
            out.push("chain broken".into());
        } else if self.part(Part::Drivetrain) < 0.95 {
            out.push("drivetrain damaged".into());
        }
        if self.part(Part::Cockpit) < BENT_BAR_BELOW {
            out.push("handlebar bent".into());
        }
        if self.broken() {
            out.push("frame broken".into());
        } else if self.part(Part::Frame) < BUCKLED_BELOW {
            out.push("frame damaged".into());
        }
        out
    }

    /// The physics effects now (12E).
    pub fn effects(&self) -> Effects {
        let mut fx = Effects::NONE;
        fx.power_factor = 1.0 - injury_power_loss(self.rider);
        if self.limited {
            fx.power_factor = fx.power_factor.min(LIMITED_POWER);
        }
        fx.crr_factor =
            1.0 + buckle_crr(self.part(Part::FrontWheel)) + buckle_crr(self.part(Part::RearWheel));
        if self.flat.iter().any(|f| *f) {
            fx.crr_factor *= FLAT_CRR_FACTOR;
            fx.speed_cap = Some(FLAT_MAX_MPS);
        }
        fx.efficiency_factor = 1.0 - drivetrain_loss(self.part(Part::Drivetrain));
        if self.part(Part::Cockpit) < BENT_BAR_BELOW {
            fx.lean_factor = 0.8;
        }
        fx.held = self.stop.is_some() || self.chain_broken || (self.abandoned && self.broken());
        fx
    }

    /// How much longer a crash keeps the rider down (12E: hurt riders a
    /// little longer, every further crash of an injured rider longer).
    pub fn down_factor(&self) -> f64 {
        match self.injury() {
            Injury::Fine | Injury::Bruised => 1.0,
            Injury::Hurt => 1.25,
            Injury::Injured => (1.5 + 0.1 * f64::from(self.crashes.saturating_sub(1))).min(2.0),
            Injury::BadlyInjured => 2.0,
        }
    }

    /// Applies a hit (a crash, or a contact that did not bring the rider
    /// down) at server tick `tick`: the rider and the bike lose by its
    /// energy and type, the bike's loss spread over the parts by the
    /// impact's side (the frame only from very hard hits); a tyre hit below
    /// [`PUNCTURE_BELOW`] goes flat, a drivetrain hit in a crash below
    /// [`CHAIN_BREAKS_BELOW`] breaks the chain. Returns what it took off.
    pub fn apply(&mut self, hit: &Hit, crash: bool, tick: u64) -> Loss {
        let (rider_loss, bike_loss) = losses(hit, crash);
        let (rider_before, bike_before) = (self.rider, self.bike());
        if crash {
            self.crashes += 1;
        }
        self.rider = (self.rider - rider_loss).max(0.0);
        let mut bits = 0;
        if bike_loss > 0.0 {
            let s = if crash { severity(hit) } else { 0.0 };
            // The frame only from very hard hits: its share goes to the
            // first part otherwise.
            let frame_part = ((s - 0.5) / 0.4).clamp(0.0, 1.0);
            let list = shares(impact_side(hit));
            let mut give: Vec<(Part, f64)> = list.to_vec();
            let frame_share: f64 = list
                .iter()
                .filter(|(p, _)| *p == Part::Frame)
                .map(|(_, s)| *s)
                .sum();
            for g in give.iter_mut() {
                if g.0 == Part::Frame {
                    g.1 = frame_share * frame_part;
                }
            }
            give[0].1 += frame_share * (1.0 - frame_part);
            let mut overflow = 0.0;
            for (p, share) in give {
                if share <= 0.0 {
                    continue;
                }
                let drop = bike_loss * share / p.weight();
                let c = &mut self.parts[p.index()];
                let taken = drop.min(*c);
                overflow += (drop - taken) * p.weight();
                *c -= taken;
                if taken > 0.0 {
                    bits |= p.bit();
                }
            }
            // What a wrecked part cannot take any more reaches the frame.
            if overflow > 0.0 {
                let f = &mut self.parts[Part::Frame.index()];
                let taken = (overflow / Part::Frame.weight()).min(*f);
                *f -= taken;
                if taken > 0.0 {
                    bits |= Part::Frame.bit();
                }
            }
            for (i, tyre) in [Part::FrontTyre, Part::RearTyre].into_iter().enumerate() {
                if bits & tyre.bit() != 0 && self.part(tyre) < PUNCTURE_BELOW {
                    self.flat[i] = true;
                }
            }
            if crash
                && bits & Part::Drivetrain.bit() != 0
                && self.part(Part::Drivetrain) < CHAIN_BREAKS_BELOW
            {
                self.chain_broken = true;
            }
        }
        let loss = Loss {
            tick,
            rider: rider_before - self.rider,
            bike: bike_before - self.bike(),
            parts: bits,
            crash,
        };
        if loss.rider > 0.0 || loss.bike > 0.0 || crash {
            self.last_hit = Some(loss);
        }
        loss
    }

    /// Starts the puncture repair (the rider asked, or stood still with a
    /// flat): the rider stops, the 30 s run once standing. False without a
    /// flat or while another stop is under way.
    pub fn start_repair(&mut self) -> bool {
        if !self.flat.iter().any(|f| *f) || self.stop.is_some() {
            return false;
        }
        self.stop = Some(Stop {
            kind: StopKind::Puncture,
            left_s: PUNCTURE_REPAIR_S,
        });
        true
    }

    fn begin(&mut self, kind: StopKind) -> TickOut {
        self.stop = Some(Stop {
            kind,
            left_s: kind.duration_s(),
        });
        TickOut::StopStarted(kind)
    }

    /// One server tick of a riding rider (not while down after a crash):
    /// the stops (they begin once the rider rides again after a crash),
    /// recovery while riding easily, the neutral service.
    pub fn tick(&mut self, ctx: &TickCtx) -> TickOut {
        let still = ctx.speed < STOPPED_MPS;
        self.still_s = if still { self.still_s + ctx.dt } else { 0.0 };
        if let Some(stop) = &mut self.stop {
            if still {
                stop.left_s -= ctx.dt;
            }
            if stop.left_s > 0.0 {
                return TickOut::Nothing;
            }
            let kind = stop.kind;
            self.stop = None;
            match kind {
                StopKind::Medical => self.limited = true,
                StopKind::Puncture => self.flat = [false; 2],
                StopKind::Chain => {
                    self.chain_broken = false;
                    // Back on, but still a damaged drivetrain.
                    let c = &mut self.parts[Part::Drivetrain.index()];
                    *c = c.max(CHAIN_BREAKS_BELOW);
                }
                StopKind::SpareBike => {
                    self.parts = [1.0; PARTS];
                    self.flat = [false; 2];
                    self.chain_broken = false;
                    self.spare = true;
                }
            }
            return TickOut::StopDone(kind);
        }
        if self.abandoned && self.broken() {
            return TickOut::Nothing;
        }
        // A broken bike first, then the rider, then the chain.
        if self.broken() {
            if ctx.in_event && !ctx.rules.neutral_service {
                // No spare in this event: out of it, then a spare to ride home.
                if !self.abandoned {
                    self.abandoned = true;
                    self.begin(StopKind::SpareBike);
                    return TickOut::Abandon;
                }
            }
            return self.begin(StopKind::SpareBike);
        }
        if self.rider < BADLY_INJURED_BELOW && !self.limited {
            if ctx.in_event && ctx.rules.dnf_badly_injured && !self.abandoned {
                self.abandoned = true;
                self.begin(StopKind::Medical);
                return TickOut::Abandon;
            }
            return self.begin(StopKind::Medical);
        }
        if self.chain_broken {
            return self.begin(StopKind::Chain);
        }
        if self.flat.iter().any(|f| *f) && (ctx.bot || self.still_s >= FLAT_STOP_S) {
            self.start_repair();
            return TickOut::StopStarted(StopKind::Puncture);
        }
        if self.limited && self.rider >= LIMIT_ENDS_ABOVE {
            self.limited = false;
        }
        // Easy riding heals (riding on does not): 1 % a minute.
        let ftp = FTP_W_PER_KG * ctx.rider_kg.max(30.0);
        if ctx.speed > 1.0 && ctx.power_w < EASY_SHARE * ftp && self.rider < 1.0 {
            self.rider = (self.rider + RECOVERY_PER_S * ctx.dt).min(1.0);
        }
        // The organiser's neutral service: a stop restores the bike.
        if ctx.in_event
            && ctx.rules.neutral_service
            && self.still_s >= SERVICE_STOP_S
            && self.parts.iter().any(|c| *c < 1.0)
        {
            self.parts = [1.0; PARTS];
            self.flat = [false; 2];
            return TickOut::Serviced;
        }
        TickOut::Nothing
    }

    /// What shows on the bike and the rider.
    pub fn visible(&self) -> Visible {
        let bent = |w: Part| {
            let c = self.part(w);
            if c >= BUCKLED_BELOW {
                0.0
            } else {
                (((BUCKLED_BELOW - c) / BUCKLED_BELOW) as f32 * 0.8 + 0.2).min(1.0)
            }
        };
        Visible {
            scuffs: ((1.0 - self.bike()) * 2.5).clamp(0.0, 1.0) as f32,
            bent: [bent(Part::FrontWheel), bent(Part::RearWheel)],
            flat: self.flat,
            chain_dropped: self.chain_broken,
            dust: ((1.0 - self.rider) * 2.0 + 0.25 * f64::from(self.crashes.min(4))).clamp(0.0, 1.0)
                as f32,
            torn: self.rider < HURT_BELOW,
        }
    }

    /// The HUD's stop line ("Medical stop: 42 s").
    pub fn stop_line(&self) -> Option<String> {
        let s = self.stop?;
        Some(format!(
            "{}{}: {:.0} s",
            s.kind.label()[..1].to_uppercase(),
            &s.kind.label()[1..],
            s.left_s.max(0.0).ceil()
        ))
    }
}

/// The rider condition after `hours` of real time off the bike (Phase 12
/// Q6: injuries heal over time).
pub fn heal(rider: f64, hours: f64) -> f64 {
    (rider + HEAL_PER_HOUR * hours.max(0.0)).clamp(0.0, 1.0)
}

/// Hours until the rider is healed (0 when fine).
pub fn hours_to_heal(rider: f64) -> f64 {
    ((1.0 - rider) / HEAL_PER_HOUR).max(0.0)
}

/// "4 h 30 min" / "25 min".
pub fn heal_text(hours: f64) -> String {
    let min = (hours * 60.0).ceil() as u64;
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// A condition in per mille (the wire's and the files' unit, so prices and
/// round trips are exact).
pub fn per_mille(c: f64) -> u32 {
    (c.clamp(0.0, 1.0) * 1000.0).round() as u32
}

/// The workshop's repair price (spec §14 Phase 4, 4E, W6) for a part
/// worth `price` hearts at `condition`: `ceil(price × (1 − condition) ×
/// 0.3)` ♥ (a part at 60 % costs 12 % of its price); a frame or wheel
/// beyond repair (below 10 %) is replaced at the full price. Exact in
/// per mille, so 60 % of 100 ♥ is 12 ♥, not 13.
pub fn repair_price(price: u32, condition: f64, part: Part) -> u32 {
    let c = per_mille(condition);
    if c >= 1000 {
        return 0;
    }
    if part.can_be_beyond_repair() && condition < BEYOND_REPAIR {
        return price;
    }
    let num = u64::from(price) * u64::from(1000 - c) * 3;
    num.div_ceil(10_000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::ObstacleKind;
    use ocw_core::RiderId;

    fn hit(kind: ContactKind, energy_j: f64, impulse: [f64; 2]) -> Hit {
        Hit {
            kind,
            other: RiderId(2),
            closing_mps: (2.0 * energy_j / 40.0).sqrt(),
            energy_j,
            impulse,
        }
    }

    fn ctx() -> TickCtx {
        TickCtx {
            dt: ocw_core::SIM_DT,
            speed: 8.0,
            power_w: 250.0,
            rider_kg: 75.0,
            bot: false,
            in_event: false,
            rules: Rules::default(),
        }
    }

    #[test]
    fn part_weights_sum_to_one_and_ids_round_trip() {
        let w: f64 = Part::ALL.iter().map(|p| p.weight()).sum();
        assert!((w - 1.0).abs() < 1e-12);
        for p in Part::ALL {
            assert_eq!(Part::from_id(p.id()), Some(p));
        }
        for side in [Side::Front, Side::Rear, Side::Left, Side::Right] {
            let s: f64 = shares(side).iter().map(|(_, s)| s).sum();
            assert!((s - 1.0).abs() < 1e-12, "{side:?}");
        }
    }

    #[test]
    fn meters_per_hit_type() {
        // A shoulder rub costs nothing, a soft bump at most 2 %.
        let rub = hit(ContactKind::Side, 2.0, [0.0, 0.3]);
        let rub = Hit {
            closing_mps: 0.3,
            ..rub
        };
        assert_eq!(losses(&rub, false), (0.0, 0.0));
        let touch = hit(ContactKind::WheelTouch, 125.0, [-1.0, 0.0]);
        let (r, b) = losses(&touch, false);
        assert!((r - 0.02).abs() < 1e-9 && (b - 0.02).abs() < 1e-9);
        let gentle = hit(ContactKind::WheelToWheel, 30.0, [0.0, 0.5]);
        let (r, b) = losses(&gentle, false);
        assert!(r > 0.0 && r < 0.01 && b > 0.0 && b < 0.01);
        // A fallen bike ridden into or the road edge: the bike only.
        let (r, b) = losses(&hit(ContactKind::Object, 125.0, [-1.0, 0.0]), false);
        assert_eq!(r, 0.0);
        assert!(b > 0.0 && b <= 0.02);
        let (r, b) = losses(&hit(ContactKind::RoadEdge, 60.0, [0.0, -1.0]), false);
        assert_eq!(r, 0.0);
        assert!(b > 0.0);
        // Crashes: 5–25 % rider, 5–30 % bike; head-on and objects hardest.
        for kind in [
            ContactKind::Side,
            ContactKind::WheelTouch,
            ContactKind::RearEnd,
            ContactKind::HeadOn,
            ContactKind::World(ObstacleKind::Tree),
        ] {
            for e in [0.0, 100.0, 1280.0, 5000.0] {
                let (r, b) = losses(&hit(kind, e, [-3.0, 0.0]), true);
                assert!((0.05..=0.25 + 1e-9).contains(&r), "{kind:?} {e}: {r}");
                assert!((0.05..=0.30 + 1e-9).contains(&b), "{kind:?} {e}: {b}");
            }
        }
        let wheel = losses(&hit(ContactKind::WheelTouch, 200.0, [-2.0, 0.0]), true);
        let tree = losses(
            &hit(ContactKind::World(ObstacleKind::Tree), 200.0, [-2.0, 0.0]),
            true,
        );
        let head_on = losses(&hit(ContactKind::HeadOn, 200.0, [-2.0, 0.0]), true);
        assert!(tree.0 > wheel.0 && tree.1 > wheel.1);
        assert!(head_on.0 > wheel.0);
        let hard = losses(&hit(ContactKind::WheelTouch, 1280.0, [-2.0, 0.0]), true);
        assert!(hard.0 > wheel.0, "more energy, more damage");
    }

    #[test]
    fn a_crash_spreads_the_bike_damage_by_its_side() {
        let mut d = Damage::fresh();
        let loss = d.apply(&hit(ContactKind::WheelTouch, 200.0, [-3.0, 0.4]), true, 7);
        assert!(loss.crash && loss.tick == 7);
        assert!((loss.bike - (1.0 - d.bike())).abs() < 1e-9);
        assert!(d.part(Part::FrontWheel) < 1.0 && d.part(Part::Cockpit) < 1.0);
        assert_eq!(d.part(Part::RearWheel), 1.0, "a front hit");
        assert_eq!(
            d.part(Part::Frame),
            1.0,
            "the frame only from very hard hits"
        );
        assert_eq!(d.crashes, 1);
        assert_eq!(d.last_hit, Some(loss));
        // Falling to the right: the derailleur.
        let mut d = Damage::fresh();
        d.apply(&hit(ContactKind::Side, 300.0, [0.2, 3.0]), true, 1);
        assert!(d.part(Part::Drivetrain) < d.part(Part::Cockpit));
        // A tree at speed: hardest, the frame too.
        let mut d = Damage::fresh();
        let l = d.apply(
            &hit(ContactKind::World(ObstacleKind::Tree), 2100.0, [-7.0, 0.0]),
            true,
            1,
        );
        assert!(d.part(Part::Frame) < 1.0);
        assert!((l.bike - 0.30).abs() < 1e-9, "{}", l.bike);
        assert!((l.rider - 0.25).abs() < 1e-9);
    }

    #[test]
    fn injury_levels_and_their_effects() {
        let lv = |r: f64| Injury::of(r);
        assert_eq!(lv(1.0), Injury::Fine);
        assert_eq!(lv(0.85), Injury::Bruised);
        assert_eq!(lv(0.70), Injury::Bruised);
        assert_eq!(lv(0.55), Injury::Hurt);
        assert_eq!(lv(0.30), Injury::Injured);
        assert_eq!(lv(0.10), Injury::BadlyInjured);
        assert_eq!(injury_power_loss(0.8), 0.0);
        assert!((injury_power_loss(0.70 - 1e-9) - 0.03).abs() < 1e-6);
        assert!((injury_power_loss(0.40) - 0.08).abs() < 1e-9);
        assert!((injury_power_loss(0.40 - 1e-9) - 0.10).abs() < 1e-6);
        assert!((injury_power_loss(0.15) - 0.20).abs() < 1e-9);
        // The effect is on the virtual power only.
        let mut d = Damage::fresh();
        d.rider = 0.5;
        let fx = d.effects();
        assert!((fx.power(200.0) - 200.0 * (1.0 - injury_power_loss(0.5))).abs() < 1e-9);
        assert!(fx.power_factor < 0.97 && fx.power_factor > 0.92);
        // Longer down: hurt, injured with every further crash.
        assert_eq!(d.down_factor(), 1.25);
        d.rider = 0.3;
        d.crashes = 3;
        assert!((d.down_factor() - 1.7).abs() < 1e-9);
        // Badly injured: the medical stop, then at most 50 %.
        d.rider = 0.1;
        let out = d.tick(&ctx());
        assert_eq!(out, TickOut::StopStarted(StopKind::Medical));
        assert!(d.effects().held);
        let mut c = ctx();
        c.speed = 0.0;
        let mut t = 0.0;
        while d.stop.is_some() {
            d.tick(&c);
            t += c.dt;
        }
        assert!((t - MEDICAL_STOP_S).abs() < 0.05, "{t}");
        assert!(d.limited);
        assert!((d.effects().power_factor - 0.5).abs() < 1e-9);
        // In an event with the organiser's rule: DNF.
        let mut d = Damage::fresh();
        d.rider = 0.1;
        let mut c = ctx();
        c.in_event = true;
        c.rules.dnf_badly_injured = true;
        assert_eq!(d.tick(&c), TickOut::Abandon);
        assert!(d.abandoned);
    }

    #[test]
    fn part_damage_effects() {
        let mut d = Damage::fresh();
        assert!(d.effects().is_none());
        // A buckled wheel: +10–40 % rolling resistance.
        d.parts[Part::RearWheel.index()] = 0.79;
        let fx = d.effects();
        assert!((fx.crr_factor - 1.10).abs() < 0.01, "{}", fx.crr_factor);
        d.parts[Part::RearWheel.index()] = 0.0;
        assert!((d.effects().crr_factor - 1.40).abs() < 1e-9);
        assert!(d.labels().contains(&"rear wheel buckled".to_string()));
        // The drivetrain: 2–10 % efficiency.
        let mut d = Damage::fresh();
        d.parts[Part::Drivetrain.index()] = 0.9;
        let e = d.effects().efficiency_factor;
        assert!(e < 0.98 && e > 0.97, "{e}");
        d.parts[Part::Drivetrain.index()] = 0.0;
        assert!((d.effects().efficiency_factor - 0.90).abs() < 1e-9);
        // The handlebar: manual lean −20 %.
        let mut d = Damage::fresh();
        d.parts[Part::Cockpit.index()] = 0.5;
        assert_eq!(d.effects().lean_factor, 0.8);
        // The profile and the power.
        let p = RiderProfile::default();
        let mut d = Damage::fresh();
        d.parts[Part::FrontWheel.index()] = 0.4;
        d.parts[Part::Drivetrain.index()] = 0.5;
        let q = d.effects().profile(&p);
        assert!(q.crr > p.crr && q.drivetrain_efficiency < p.drivetrain_efficiency);
        assert_eq!(q.cda_m2, p.cda_m2);
    }

    #[test]
    fn punctures_and_repairs() {
        let mut d = Damage::fresh();
        // A very hard front hit: the front tyre goes flat.
        d.apply(&hit(ContactKind::HeadOn, 2000.0, [-6.0, 0.0]), true, 1);
        assert!(d.flat[0], "{:?}", d.parts);
        let fx = d.effects();
        assert!(fx.crr_factor >= FLAT_CRR_FACTOR);
        assert_eq!(fx.speed_cap, Some(FLAT_MAX_MPS));
        assert!(
            d.labels()
                .iter()
                .any(|l| l == "front puncture: stop to repair")
        );
        // Riding on slowly: no repair yet.
        let c = ctx();
        assert_eq!(d.tick(&c), TickOut::Nothing);
        // Standing still for 2 s starts the 30 s repair.
        let mut still = ctx();
        still.speed = 0.0;
        let mut started = false;
        for _ in 0..200 {
            if d.tick(&still) == TickOut::StopStarted(StopKind::Puncture) {
                started = true;
                break;
            }
        }
        assert!(started);
        assert!(d.effects().held);
        let mut t = 0.0;
        let mut done = None;
        for _ in 0..3000 {
            t += still.dt;
            if let TickOut::StopDone(k) = d.tick(&still) {
                done = Some(k);
                break;
            }
        }
        assert_eq!(done, Some(StopKind::Puncture));
        assert!((t - PUNCTURE_REPAIR_S).abs() < 0.05, "{t}");
        assert!(!d.flat[0] && !d.effects().held);
        // A bot repairs at once.
        let mut d = Damage::fresh();
        d.flat[1] = true;
        let mut bot = ctx();
        bot.bot = true;
        assert_eq!(d.tick(&bot), TickOut::StopStarted(StopKind::Puncture));
        // The repair request, only with a flat.
        let mut d = Damage::fresh();
        assert!(!d.start_repair());
        d.flat[0] = true;
        assert!(d.start_repair());
        // The time only runs when standing still.
        let before = d.stop.unwrap().left_s;
        d.tick(&ctx());
        assert_eq!(d.stop.unwrap().left_s, before);
    }

    #[test]
    fn a_broken_chain_stops_the_rider_for_45_s() {
        let mut d = Damage::fresh();
        d.parts[Part::Drivetrain.index()] = 0.4;
        d.apply(&hit(ContactKind::Side, 900.0, [0.0, 4.0]), true, 1);
        assert!(d.chain_broken);
        assert!(d.effects().held, "the power has no effect");
        assert!(d.visible().chain_dropped);
        let mut still = ctx();
        still.speed = 0.0;
        assert_eq!(d.tick(&still), TickOut::StopStarted(StopKind::Chain));
        let mut n = 0;
        while d.stop.is_some() {
            d.tick(&still);
            n += 1;
        }
        assert!(((f64::from(n) * still.dt) - CHAIN_REPAIR_S).abs() < 0.05);
        assert!(!d.chain_broken && !d.effects().held);
    }

    #[test]
    fn a_broken_bike_waits_for_a_spare_or_ends_the_event() {
        let mut d = Damage::fresh();
        d.parts = [0.05; PARTS];
        assert!(d.broken());
        assert_eq!(d.bike_level(), BikeLevel::Broken);
        assert_eq!(d.tick(&ctx()), TickOut::StopStarted(StopKind::SpareBike));
        let mut still = ctx();
        still.speed = 0.0;
        let mut out = TickOut::Nothing;
        while d.stop.is_some() {
            out = d.tick(&still);
        }
        assert_eq!(out, TickOut::StopDone(StopKind::SpareBike));
        assert!(d.spare && d.bike() == 1.0);
        // In an event without neutral service: DNF.
        let mut d = Damage::fresh();
        d.parts = [0.05; PARTS];
        let mut ev = ctx();
        ev.in_event = true;
        assert_eq!(d.tick(&ev), TickOut::Abandon);
        // With it: the spare, still in the race.
        let mut d = Damage::fresh();
        d.parts = [0.05; PARTS];
        ev.rules.neutral_service = true;
        assert_eq!(d.tick(&ev), TickOut::StopStarted(StopKind::SpareBike));
        assert!(!d.abandoned);
        // Carried over broken: ridden as a loaner.
        let d = Damage::carried(0.9, [0.05; PARTS]);
        assert!(d.spare && !d.broken());
    }

    #[test]
    fn recovery_by_easy_riding_and_the_neutral_service() {
        let mut d = Damage::fresh();
        d.rider = 0.5;
        // Hard riding does not heal.
        let mut hard = ctx();
        hard.power_w = 250.0;
        for _ in 0..3000 {
            d.tick(&hard);
        }
        assert_eq!(d.rider, 0.5);
        // Easy riding: 1 % a minute (below 50 % of 2.5 W/kg).
        let mut easy = ctx();
        easy.power_w = 80.0;
        for _ in 0..(60 * 50) {
            d.tick(&easy);
        }
        assert!((d.rider - 0.51).abs() < 1e-6, "{}", d.rider);
        // A stop restores no parts outside an event …
        let mut d = Damage::fresh();
        d.parts[Part::RearWheel.index()] = 0.5;
        let mut still = ctx();
        still.speed = 0.0;
        for _ in 0..(40 * 50) {
            d.tick(&still);
        }
        assert_eq!(d.part(Part::RearWheel), 0.5);
        // … but at the organiser's neutral service after 30 s.
        still.in_event = true;
        still.rules.neutral_service = true;
        d.still_s = 0.0;
        let mut out = TickOut::Nothing;
        for _ in 0..(31 * 50) {
            out = d.tick(&still);
            if out == TickOut::Serviced {
                break;
            }
        }
        assert_eq!(out, TickOut::Serviced);
        assert_eq!(d.bike(), 1.0);
    }

    #[test]
    fn real_time_healing() {
        assert_eq!(heal(0.5, 0.0), 0.5);
        assert!((heal(0.5, 2.0) - 0.7).abs() < 1e-12);
        assert_eq!(heal(0.5, 10.0), 1.0);
        assert_eq!(heal(0.5, -3.0), 0.5, "a clock going back heals nothing");
        assert!((hours_to_heal(0.55) - 4.5).abs() < 1e-9);
        assert_eq!(heal_text(4.5), "4 h 30 min");
        assert_eq!(heal_text(0.25), "15 min");
        assert_eq!(heal_text(2.0), "2 h");
    }

    #[test]
    fn repair_cost_formula() {
        // ceil(price × (1 − condition) × 0.3): 60 % of 100 ♥ = 12 ♥.
        assert_eq!(repair_price(100, 0.6, Part::Cockpit), 12);
        assert_eq!(repair_price(100, 1.0, Part::Cockpit), 0);
        assert_eq!(repair_price(164, 0.5, Part::Drivetrain), 25); // 24.6 → 25
        assert_eq!(repair_price(1, 0.999, Part::Cockpit), 1, "rounded up");
        // Frames and wheels beyond repair: replaced at the full price.
        assert_eq!(repair_price(800, 0.05, Part::Frame), 800);
        assert_eq!(repair_price(800, 0.05, Part::Cockpit), 228); // 0.95 × 0.3 × 800
        assert_eq!(repair_price(800, 0.1, Part::Frame), 216);
    }

    #[test]
    fn visible_damage_follows_the_meters() {
        let d = Damage::fresh();
        assert!(!d.visible().any());
        let mut d = Damage::fresh();
        d.parts[Part::FrontWheel.index()] = 0.3;
        d.rider = 0.6;
        d.crashes = 1;
        let v = d.visible();
        assert!(v.bent[0] > 0.5 && v.bent[1] == 0.0);
        assert!(v.scuffs > 0.0 && v.dust > 0.0 && v.torn);
    }

    #[test]
    fn status_words_and_icons() {
        assert_eq!(Injury::of(0.5).status(), Status::Warning);
        assert_eq!(BikeLevel::of(0.05).label(), "broken");
        assert_eq!(BikeLevel::of(0.9).label(), "scratched");
        assert_eq!(part_status(0.9), Status::Good);
        assert_eq!(part_status(0.5), Status::Warning);
        assert_eq!(part_status(0.2), Status::Serious);
        assert_eq!(part_status(0.05), Status::Critical);
        let icons: std::collections::BTreeSet<_> = [
            Status::Good,
            Status::Warning,
            Status::Serious,
            Status::Critical,
        ]
        .iter()
        .map(|s| s.icon())
        .collect();
        assert_eq!(icons.len(), 4, "each status has its own icon");
    }
}
