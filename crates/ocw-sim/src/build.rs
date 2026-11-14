//! Build physics: parts within the curated profiles (spec §14 Phase 4,
//! 4G; §2.5). Pure, shared by the server (the truth, §6.1), the client's
//! prediction and the workshop's effect lines.
//!
//! The frameset decides the profile ([`ProfileId`]); the other parts carry
//! real weights and a few curated **class steps** – `rolling` (tyres; each
//! step ±[`ROLLING_STEP`] of the profile's Crr, negative = rolls better),
//! `aero` (wheels, cockpits, frames, bags; each step ∓[`AERO_STEP_M2`] of
//! CdA, positive = less drag) and `efficiency` (groupsets; each step
//! ±[`EFFICIENCY_STEP`] of drivetrain efficiency). The client sends only
//! part ids; the values come from the server's copy of the catalogue
//! ([`PartTable`], `assets/parts.toml`).
//!
//! [`build`] adds them up – bike mass = profile mass + Σ (part − base
//! part), at least [`min_bike_mass_kg`] – and then **caps the whole build**:
//! at the reference ride ([`REFERENCE_POWER_W`], [`REFERENCE_RIDER_KG`])
//! the build may be at most [`FLAT_CAP`] faster or slower than the
//! profile's base build on flat asphalt and [`CLIMB_CAP`] on a
//! [`CLIMB_GRADE`] climb. A build beyond the band keeps the largest share
//! of its change that fits (every value moves back towards the profile by
//! the same share), so the best stacked build sits exactly on the cap.
//!
//! Race modes ([`PhysicsMode`]): *Open* (parts count, within the caps),
//! *Profile* (only the frame's profile counts; the default for races and
//! group rides) and *Equal* (one profile for everyone).

use std::sync::OnceLock;

use ocw_core::{BikeType, ProfileId, RiderProfile};
use serde::Deserialize;

use crate::event::EventKind;
use crate::physics::{Environment, steady_speed};

/// Rolling resistance per `rolling` class step (share of the profile's Crr).
pub const ROLLING_STEP: f64 = 0.04;
/// Drag area per `aero` class step (m²; a positive class has less drag).
pub const AERO_STEP_M2: f64 = 0.004;
/// Drivetrain efficiency per `efficiency` class step (absolute share).
pub const EFFICIENCY_STEP: f64 = 0.003;
/// Class ranges per part: `rolling` and `aero` −2 … +2, `efficiency` −1 … +1.
pub const CLASS_RANGE: i32 = 2;
pub const EFFICIENCY_RANGE: i32 = 1;

/// Largest speed difference of a build against its profile on flat asphalt
/// at the reference ride (spec 4G, Q2: +1.5 %).
pub const FLAT_CAP: f64 = 0.015;
/// The same on a [`CLIMB_GRADE`] climb (+2.5 %).
pub const CLIMB_CAP: f64 = 0.025;
/// The reference ride of the caps: power and rider weight (as for the
/// profile caps of §2.5).
pub const REFERENCE_POWER_W: f64 = 250.0;
pub const REFERENCE_RIDER_KG: f64 = 75.0;
/// The climb of the climbing cap.
pub const CLIMB_GRADE: f64 = 0.06;
/// Lightest bike (kg): the UCI limit for road, TT and gravel bikes.
pub const MIN_BIKE_MASS_KG: f64 = 6.8;
/// Lightest mountain bike (kg).
pub const MIN_MTB_MASS_KG: f64 = 9.0;
/// The profile everyone rides in *Equal* mode.
pub const EQUAL_PROFILE: ProfileId = ProfileId::Road;

/// The mass floor of a bike type (spec 4G).
pub fn min_bike_mass_kg(bike: BikeType) -> f64 {
    match bike {
        BikeType::Mountain => MIN_MTB_MASS_KG,
        _ => MIN_BIKE_MASS_KG,
    }
}

/// Whose physics count in a race (spec §2.5 *Race modes*, 4G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PhysicsMode {
    /// The profile and the parts (within the caps): free ride, single
    /// player, workouts, time trials.
    #[default]
    Open,
    /// Only the frame's profile; the parts are cosmetic. The default for
    /// races and group rides.
    Profile,
    /// One profile ([`EQUAL_PROFILE`]) for everyone; bikes and parts are
    /// only shown.
    Equal,
}

impl PhysicsMode {
    pub const ALL: [PhysicsMode; 3] = [Self::Open, Self::Profile, Self::Equal];

    /// Id in commands and files.
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Profile => "profile",
            Self::Equal => "equal",
        }
    }

    /// Name for the event panel and the cards.
    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Profile => "Profile",
            Self::Equal => "Equal",
        }
    }

    /// What it means, in a few words.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Open => "parts count (within the caps)",
            Self::Profile => "only the frame's profile counts, parts are cosmetic",
            Self::Equal => "everyone rides the same bike physics",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(s.trim()))
    }

    /// The mode of an event whose organiser chose none: *Profile* for
    /// races and group rides, *Open* for a time trial against the own
    /// ghost (spec 4G).
    pub fn for_event(kind: &EventKind) -> Self {
        match kind {
            EventKind::TimeTrial { .. } => Self::Open,
            EventKind::Race { .. } | EventKind::GroupRide { .. } => Self::Profile,
        }
    }
}

/// The physics of one catalogue part (the server's copy of
/// `assets/parts.toml`; names and prices are the workshop's).
#[derive(Debug, Clone, PartialEq)]
pub struct PartPhysics {
    pub id: String,
    pub slot: String,
    /// A free stock part (tier `base`).
    pub base: bool,
    pub weight_g: u32,
    pub fits: Vec<BikeType>,
    /// Frames: the physics profile.
    pub profile: Option<ProfileId>,
    /// Class steps, clamped to [`CLASS_RANGE`] / [`EFFICIENCY_RANGE`].
    pub rolling: i32,
    pub aero: i32,
    pub efficiency: i32,
}

impl PartPhysics {
    /// Whether the part can be on a bike of `profile`: its bike type, and
    /// a frameset only on its own profile.
    pub fn fits(&self, profile: ProfileId) -> bool {
        self.fits.contains(&profile.bike())
            && (self.slot != "frame" || self.profile == Some(profile))
    }
}

#[derive(Deserialize)]
struct RawPart {
    id: String,
    slot: String,
    tier: String,
    weight_g: u32,
    fits: Vec<String>,
    profile: Option<String>,
    #[serde(default)]
    rolling: i32,
    #[serde(default)]
    aero: i32,
    #[serde(default)]
    efficiency: i32,
}

#[derive(Deserialize)]
struct RawTable {
    #[serde(default)]
    part: Vec<RawPart>,
}

/// The parts' physics, as the server knows them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartTable {
    parts: Vec<PartPhysics>,
}

/// The shipped catalogue (the same file the workshop reads).
pub const SHIPPED_PARTS: &str = include_str!("../../../assets/parts.toml");

impl PartTable {
    /// Reads the physics keys of `assets/parts.toml` (the other keys are
    /// the workshop's and are skipped here).
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawTable = toml::from_str(text).map_err(|e| format!("parts: {e}"))?;
        let mut parts = Vec::with_capacity(raw.part.len());
        for r in raw.part {
            let fits = r
                .fits
                .iter()
                .map(|f| BikeType::parse(f).ok_or_else(|| format!("{}: bike type {f:?}", r.id)))
                .collect::<Result<Vec<_>, _>>()?;
            let profile = match r.profile.as_deref() {
                None => None,
                Some(p) => {
                    Some(ProfileId::parse(p).ok_or_else(|| format!("{}: profile {p:?}", r.id))?)
                }
            };
            parts.push(PartPhysics {
                base: r.tier == "base",
                weight_g: r.weight_g,
                fits,
                profile,
                rolling: r.rolling.clamp(-CLASS_RANGE, CLASS_RANGE),
                aero: r.aero.clamp(-CLASS_RANGE, CLASS_RANGE),
                efficiency: r.efficiency.clamp(-EFFICIENCY_RANGE, EFFICIENCY_RANGE),
                slot: r.slot,
                id: r.id,
            });
        }
        Ok(Self { parts })
    }

    /// The shipped table (parsed once).
    pub fn shipped() -> &'static PartTable {
        static T: OnceLock<PartTable> = OnceLock::new();
        T.get_or_init(|| {
            PartTable::parse(SHIPPED_PARTS).unwrap_or_else(|e| panic!("shipped parts.toml: {e}"))
        })
    }

    pub fn parts(&self) -> &[PartPhysics] {
        &self.parts
    }

    pub fn get(&self, id: &str) -> Option<&PartPhysics> {
        self.parts.iter().find(|p| p.id == id)
    }

    /// The base part of `slot` on a bike of `profile` (frames: the
    /// profile's base frameset).
    pub fn base(&self, slot: &str, profile: ProfileId) -> Option<&PartPhysics> {
        self.parts
            .iter()
            .find(|p| p.base && p.slot == slot && p.fits(profile))
    }

    /// The slots, in catalogue order.
    fn slots(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for p in &self.parts {
            if !out.contains(&p.slot.as_str()) {
                out.push(&p.slot);
            }
        }
        out
    }

    /// The parts that count of `parts` on a bike of `profile`: known, not
    /// a base part, fitting, the first of each slot; in slot order. Unknown,
    /// unfitting and (with accounts) not owned parts count as the base part.
    pub fn fitted(&self, profile: ProfileId, parts: &[String]) -> Vec<&PartPhysics> {
        let mut by_slot: Vec<&PartPhysics> = Vec::new();
        for id in parts {
            let Some(p) = self.get(id) else { continue };
            if p.base || !p.fits(profile) || by_slot.iter().any(|q| q.slot == p.slot) {
                continue;
            }
            by_slot.push(p);
        }
        let order = self.slots();
        by_slot.sort_by_key(|p| order.iter().position(|s| *s == p.slot));
        by_slot
    }
}

/// The values of a build that the physics uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildParams {
    pub bike_mass_kg: f64,
    pub cda_m2: f64,
    pub crr: f64,
    pub drivetrain_efficiency: f64,
}

impl BuildParams {
    fn of(profile: ProfileId) -> Self {
        let p = profile.profile();
        Self {
            bike_mass_kg: p.bike_mass_kg,
            cda_m2: p.cda_m2,
            crr: p.crr,
            drivetrain_efficiency: p.drivetrain_efficiency,
        }
    }

    /// `self` moved `t` of the way to `to`.
    fn lerp(&self, to: &Self, t: f64) -> Self {
        let l = |a: f64, b: f64| a + (b - a) * t;
        Self {
            bike_mass_kg: l(self.bike_mass_kg, to.bike_mass_kg),
            cda_m2: l(self.cda_m2, to.cda_m2),
            crr: l(self.crr, to.crr),
            drivetrain_efficiency: l(self.drivetrain_efficiency, to.drivetrain_efficiency),
        }
    }

    /// The rider profile of these values for a rider of `weight_kg`.
    pub fn rider_profile(&self, profile: ProfileId, weight_kg: f64) -> RiderProfile {
        RiderProfile {
            bike_mass_kg: self.bike_mass_kg,
            cda_m2: self.cda_m2,
            crr: self.crr,
            drivetrain_efficiency: self.drivetrain_efficiency,
            ..RiderProfile::curated(profile, weight_kg)
        }
    }
}

/// A bike's build: the profile plus the fitted parts, capped.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    pub profile: ProfileId,
    /// The fitted (non-base) part ids that count, in slot order.
    pub parts: Vec<String>,
    /// The sum of the parts before the caps (mass floor applied).
    pub raw: BuildParams,
    /// What the physics uses.
    pub params: BuildParams,
    /// The mass floor held the bike up.
    pub mass_floored: bool,
    /// Share of the raw change that is kept (1 = within the band).
    pub scale: f64,
    /// Speed against the profile's base build at the reference ride, on
    /// the flat and on the climb (0.012 = 1.2 % faster), after the caps.
    pub flat_gain: f64,
    pub climb_gain: f64,
}

impl Build {
    /// The profile's base build (no parts).
    pub fn stock(profile: ProfileId) -> Self {
        let p = BuildParams::of(profile);
        Self {
            profile,
            parts: Vec::new(),
            raw: p,
            params: p,
            mass_floored: false,
            scale: 1.0,
            flat_gain: 0.0,
            climb_gain: 0.0,
        }
    }

    /// Whether the caps cut the build back.
    pub fn capped(&self) -> bool {
        self.scale < 1.0 - 1e-9
    }

    pub fn is_stock(&self) -> bool {
        self.parts.is_empty()
    }

    /// The rider profile of this build for a rider of `weight_kg`.
    pub fn rider_profile(&self, weight_kg: f64) -> RiderProfile {
        self.params.rider_profile(self.profile, weight_kg)
    }

    /// The build's hash ([`build_hash`]).
    pub fn hash(&self) -> u32 {
        build_hash(self.profile, &self.parts)
    }

    /// The effect of going from `from` to this build.
    pub fn effect_from(&self, from: &Build) -> Effect {
        Effect {
            flat: self.flat_gain - from.flat_gain,
            climb: self.climb_gain - from.climb_gain,
            mass_kg: self.params.bike_mass_kg - from.params.bike_mass_kg,
            capped: self.capped(),
        }
    }
}

/// A compact hash of a build for the snapshots (0 = the profile's stock
/// bike): FNV-1a of the profile id and the fitted part ids.
pub fn build_hash(profile: ProfileId, parts: &[String]) -> u32 {
    if parts.is_empty() {
        return 0;
    }
    let mut ids: Vec<&str> = parts.iter().map(String::as_str).collect();
    ids.sort_unstable();
    let mut h: u32 = 0x811c_9dc5;
    let mut eat = |b: u8| {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    };
    profile.id().bytes().for_each(&mut eat);
    for id in ids {
        eat(b';');
        id.bytes().for_each(&mut eat);
    }
    h.max(1)
}

/// The speeds of `p` against `base` at the reference ride: (flat, climb).
pub fn gains(base: &RiderProfile, p: &RiderProfile) -> (f64, f64) {
    let rel = |grade: f64| {
        let env = Environment::with_grade(grade);
        steady_speed(p, &env, REFERENCE_POWER_W) / steady_speed(base, &env, REFERENCE_POWER_W) - 1.0
    };
    (rel(0.0), rel(CLIMB_GRADE))
}

/// The build of a bike of `profile` with `parts` (ids; see
/// [`PartTable::fitted`]), capped (module docs).
pub fn build(table: &PartTable, profile: ProfileId, parts: &[String]) -> Build {
    let fitted = table.fitted(profile, parts);
    if fitted.is_empty() {
        return Build::stock(profile);
    }
    let base = BuildParams::of(profile);
    let mut mass_g = 0i64;
    let (mut rolling, mut aero, mut efficiency) = (0i32, 0i32, 0i32);
    for p in &fitted {
        let stock = table.base(&p.slot, profile).map_or(0, |b| b.weight_g);
        mass_g += i64::from(p.weight_g) - i64::from(stock);
        rolling += p.rolling;
        aero += p.aero;
        efficiency += p.efficiency;
    }
    let floor = min_bike_mass_kg(profile.bike()).min(base.bike_mass_kg);
    let summed = base.bike_mass_kg + mass_g as f64 / 1000.0;
    let raw = BuildParams {
        bike_mass_kg: summed.max(floor),
        cda_m2: (base.cda_m2 - f64::from(aero) * AERO_STEP_M2).max(0.1),
        crr: base.crr * (1.0 + f64::from(rolling) * ROLLING_STEP).max(0.1),
        drivetrain_efficiency: (base.drivetrain_efficiency
            + f64::from(efficiency) * EFFICIENCY_STEP)
            .min(0.999),
    };
    let reference = base.rider_profile(profile, REFERENCE_RIDER_KG);
    let gains_at = |t: f64| {
        gains(
            &reference,
            &base
                .lerp(&raw, t)
                .rider_profile(profile, REFERENCE_RIDER_KG),
        )
    };
    let within = |(flat, climb): (f64, f64)| flat.abs() <= FLAT_CAP && climb.abs() <= CLIMB_CAP;
    let scale = if within(gains_at(1.0)) {
        1.0
    } else {
        // The largest share that stays in the band (the gains grow
        // steadily with the share).
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if within(gains_at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let params = base.lerp(&raw, scale);
    let (flat_gain, climb_gain) = gains_at(scale);
    Build {
        profile,
        parts: fitted.iter().map(|p| p.id.clone()).collect(),
        raw,
        params,
        mass_floored: summed < floor,
        scale,
        flat_gain,
        climb_gain,
    }
}

/// The fastest part of every slot for `profile` (by its own effect, flat
/// and climb together; slots where nothing is faster than the base part
/// keep it): the "best build" the caps are made for (dev tools, tests).
pub fn best_parts(table: &PartTable, profile: ProfileId) -> Vec<String> {
    let mut out = Vec::new();
    for slot in table.slots() {
        let best = table
            .parts()
            .iter()
            .filter(|p| p.slot == slot && !p.base && p.fits(profile))
            .map(|p| (build(table, profile, std::slice::from_ref(&p.id)), p))
            .filter(|(b, _)| b.flat_gain > 0.0 || b.climb_gain > 0.0)
            .max_by(|a, b| {
                (a.0.flat_gain + a.0.climb_gain).total_cmp(&(b.0.flat_gain + b.0.climb_gain))
            });
        if let Some((_, p)) = best {
            out.push(p.id.clone());
        }
    }
    out
}

/// The physics a rider rides with: `base` (their weight and the frame's
/// profile, as the Hello or the workshop named it) and the bike's `build`
/// under `mode`.
pub fn effective(mode: PhysicsMode, base: &RiderProfile, build: Option<&Build>) -> RiderProfile {
    match mode {
        PhysicsMode::Equal => RiderProfile::curated(EQUAL_PROFILE, base.rider_mass_kg),
        PhysicsMode::Profile => *base,
        PhysicsMode::Open => match build {
            Some(b) if b.profile == base.bike && !b.is_stock() => {
                b.rider_profile(base.rider_mass_kg)
            }
            _ => *base,
        },
    }
}

/// The effect of a change of build at the reference ride, for the cards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    /// Speed difference on the flat and on the climb (shares).
    pub flat: f64,
    pub climb: f64,
    /// Bike mass difference (kg).
    pub mass_kg: f64,
    /// The resulting build is at the cap.
    pub capped: bool,
}

impl Effect {
    /// "+0.4 % flat, −0.6 % at 6 %, capped" (to a tenth of a percent).
    pub fn line(&self) -> String {
        let pct = |x: f64| {
            let v = (x * 1000.0).round() / 10.0;
            if v == 0.0 {
                "±0.0 %".to_string()
            } else if v < 0.0 {
                format!("−{:.1} %", -v)
            } else {
                format!("+{v:.1} %")
            }
        };
        let mut s = format!(
            "{} flat, {} at {:.0} %",
            pct(self.flat),
            pct(self.climb),
            CLIMB_GRADE * 100.0
        );
        if self.capped {
            s.push_str(", capped");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> &'static PartTable {
        PartTable::shipped()
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn best_parts(profile: ProfileId) -> Vec<String> {
        super::best_parts(t(), profile)
    }

    /// The best build of every profile (for the spec's table; run with
    /// `--nocapture`).
    #[test]
    fn best_builds_per_profile() {
        for profile in ProfileId::ALL {
            let b = build(t(), profile, &best_parts(profile));
            println!(
                "{:14} {:+.2} % flat {:+.2} % climb {:.2} kg scale {:.2} {}",
                profile.id(),
                b.flat_gain * 100.0,
                b.climb_gain * 100.0,
                b.params.bike_mass_kg,
                b.scale,
                b.parts.join(",")
            );
            assert!(!b.parts.is_empty());
        }
    }

    #[test]
    fn the_shipped_table_parses_and_has_a_base_for_every_slot() {
        let table = t();
        assert_eq!(table.parts().len(), 76);
        for profile in ProfileId::ALL {
            for slot in table.slots() {
                assert!(table.base(slot, profile).is_some(), "{slot} {profile:?}");
            }
            assert_eq!(table.base("frame", profile).unwrap().profile, Some(profile));
        }
        assert!(PartTable::parse("[[part]]\nid = 1").is_err());
    }

    #[test]
    fn no_parts_is_the_profile_itself() {
        for profile in ProfileId::ALL {
            let b = build(t(), profile, &[]);
            assert_eq!(b, Build::stock(profile));
            assert_eq!(b.rider_profile(70.0), RiderProfile::curated(profile, 70.0));
            assert_eq!(b.hash(), 0);
            // Base parts, unknown ids and parts of other bike types count
            // as the base part.
            let same = build(
                t(),
                profile,
                &ids(&["tyres.road.base", "rocket.wheels", "frame.mtb.upper"])
                    .into_iter()
                    .filter(|id| !t().get(id).is_some_and(|p| p.fits(profile)))
                    .collect::<Vec<_>>(),
            );
            assert!(same.is_stock(), "{profile:?}: {:?}", same.parts);
        }
        // An MTB frame on a road bike is not fitted.
        assert!(build(t(), ProfileId::Road, &ids(&["frame.mtb.upper"])).is_stock());
        // An aero frame only on the aero profile.
        assert!(build(t(), ProfileId::Road, &ids(&["frame.road-aero.top"])).is_stock());
        assert!(!build(t(), ProfileId::RoadAero, &ids(&["frame.road-aero.top"])).is_stock());
    }

    /// Each class steps the value by its curated amount; weights are real.
    #[test]
    fn class_steps_per_part_class() {
        let road = ProfileId::Road.profile();
        // Race tyres: rolling −2 (−8 % Crr), 200 g lighter than the base.
        let b = build(t(), ProfileId::Road, &ids(&["tyres.road.race"]));
        assert!(!b.capped());
        assert!((b.params.crr - road.crr * 0.92).abs() < 1e-12);
        assert!((b.params.bike_mass_kg - (road.bike_mass_kg - 0.2)).abs() < 1e-9);
        assert_eq!(b.params.cda_m2, road.cda_m2);
        // Training tyres: rolling +1, slower.
        let slow = build(t(), ProfileId::Road, &ids(&["tyres.road.training"]));
        assert!((slow.params.crr - road.crr * 1.04).abs() < 1e-12);
        assert!(slow.flat_gain < 0.0);
        // The disc: aero +2 (−0.008 m² CdA) on the TT bike.
        let tt = ProfileId::TimeTrial.profile();
        let disc = build(t(), ProfileId::TimeTrial, &ids(&["wheels.tt.disc"]));
        assert!((disc.params.cda_m2 - (tt.cda_m2 - 2.0 * AERO_STEP_M2)).abs() < 1e-12);
        // A top-tube bag: aero −1, more drag.
        let bag = build(t(), ProfileId::Road, &ids(&["bag.top_tube"]));
        assert!((bag.params.cda_m2 - (road.cda_m2 + AERO_STEP_M2)).abs() < 1e-12);
        assert!(bag.flat_gain < 0.0);
        // An electronic top groupset: efficiency +1 (+0.3 %).
        let g = build(
            t(),
            ProfileId::Road,
            &ids(&["groupset.road-electronic.top"]),
        );
        assert!(
            (g.params.drivetrain_efficiency - (road.drivetrain_efficiency + EFFICIENCY_STEP)).abs()
                < 1e-12
        );
        // Every single part is a small effect: well inside the band.
        for p in t().parts().iter().filter(|p| !p.base) {
            for profile in ProfileId::ALL.into_iter().filter(|q| p.fits(*q)) {
                let b = build(t(), profile, std::slice::from_ref(&p.id));
                assert!(b.flat_gain.abs() < 0.012, "{} {profile:?}: {b:?}", p.id);
                assert_eq!(b.parts, vec![p.id.clone()]);
            }
        }
    }

    /// Lighter parts help on the climb more than on the flat; drag the
    /// other way round.
    #[test]
    fn weight_counts_uphill_and_drag_on_the_flat() {
        let light = build(
            t(),
            ProfileId::Road,
            &ids(&["seatpost.carbon", "saddle.road.top"]),
        );
        assert!(light.climb_gain > 2.0 * light.flat_gain && light.flat_gain > 0.0);
        let aero = build(t(), ProfileId::Road, &ids(&["cockpit.road.integrated"]));
        assert!(aero.flat_gain > aero.climb_gain);
    }

    /// The best stacked build hits the cap exactly; nothing gets past it.
    #[test]
    fn stacked_best_builds_hit_exactly_the_cap() {
        let mut hit = 0;
        for profile in ProfileId::ALL {
            let parts = best_parts(profile);
            let b = build(t(), profile, &parts);
            assert!(b.flat_gain <= FLAT_CAP + 1e-12, "{profile:?}: {b:?}");
            assert!(b.climb_gain <= CLIMB_CAP + 1e-12, "{profile:?}: {b:?}");
            assert!(
                b.flat_gain > 0.0 && b.climb_gain > 0.0,
                "{profile:?}: {b:?}"
            );
            if b.capped() {
                // On the cap: one of the two at the boundary (1e-9).
                let at_flat = (b.flat_gain - FLAT_CAP).abs() < 1e-9;
                let at_climb = (b.climb_gain - CLIMB_CAP).abs() < 1e-9;
                assert!(at_flat || at_climb, "{profile:?}: {b:?}");
                hit += 1;
            }
        }
        // The aero road bike's best build (aero frame, wheels, cockpit,
        // race tyres, top groupset) is past the band and lands on the
        // flat cap.
        let aero = build(t(), ProfileId::RoadAero, &best_parts(ProfileId::RoadAero));
        assert!(aero.capped(), "{aero:?}");
        assert!((aero.flat_gain - FLAT_CAP).abs() < 1e-9, "{aero:?}");
        assert!(hit >= 1);
    }

    /// A build is within ±cap the other way too (a heavy, draggy build).
    #[test]
    fn slow_builds_are_capped_too() {
        let table = PartTable::parse(
            r#"
[[part]]
id = "frame.road.base"
slot = "frame"
tier = "base"
weight_g = 1350
fits = ["road"]
profile = "road"
[[part]]
id = "bag.base"
slot = "bag"
tier = "base"
weight_g = 0
fits = ["road"]
[[part]]
id = "bag.anchor"
slot = "bag"
tier = "top"
weight_g = 9000
fits = ["road"]
aero = -9
"#,
        )
        .unwrap();
        assert_eq!(table.get("bag.anchor").unwrap().aero, -CLASS_RANGE);
        let b = build(&table, ProfileId::Road, &ids(&["bag.anchor"]));
        assert!(b.capped());
        assert!(b.flat_gain >= -FLAT_CAP - 1e-12 && b.climb_gain >= -CLIMB_CAP - 1e-12);
        assert!((b.climb_gain + CLIMB_CAP).abs() < 1e-9 || (b.flat_gain + FLAT_CAP).abs() < 1e-9);
    }

    /// The mass floor: 6.8 kg for road, TT and gravel bikes, 9.0 kg for
    /// mountain bikes.
    #[test]
    fn minimum_bike_mass() {
        assert_eq!(min_bike_mass_kg(BikeType::Road), 6.8);
        assert_eq!(min_bike_mass_kg(BikeType::Mountain), 9.0);
        // The climbing frame is 6.8 kg already: every lighter part hits
        // the floor.
        let light = build(
            t(),
            ProfileId::RoadClimbing,
            &ids(&[
                "frame.road-climbing.top",
                "wheels.road-carbon.top",
                "seatpost.carbon",
            ]),
        );
        assert!(light.mass_floored);
        assert_eq!(light.raw.bike_mass_kg, 6.8);
        assert!(light.params.bike_mass_kg >= 6.8);
        for profile in ProfileId::ALL {
            let b = build(t(), profile, &best_parts(profile));
            assert!(
                b.params.bike_mass_kg >= min_bike_mass_kg(profile.bike()) - 1e-12,
                "{profile:?}: {b:?}"
            );
        }
        let mtb = build(
            t(),
            ProfileId::Mountain,
            &ids(&[
                "frame.mtb.upper",
                "groupset.mtb.top",
                "tyres.mtb.xc",
                "seatpost.carbon",
            ]),
        );
        assert!(mtb.params.bike_mass_kg >= 9.0);
    }

    /// Profile ignores parts, Equal equalises, Open applies them within
    /// the caps.
    #[test]
    fn the_three_race_modes() {
        let base = RiderProfile::curated(ProfileId::TimeTrial, 68.0);
        let b = build(
            t(),
            ProfileId::TimeTrial,
            &ids(&["wheels.tt.disc", "tyres.road.race"]),
        );
        let open = effective(PhysicsMode::Open, &base, Some(&b));
        assert_eq!(open, b.rider_profile(68.0));
        assert!(open.cda_m2 < base.cda_m2 && open.crr < base.crr);
        assert_eq!(effective(PhysicsMode::Profile, &base, Some(&b)), base);
        let eq = effective(PhysicsMode::Equal, &base, Some(&b));
        assert_eq!(eq, RiderProfile::curated(EQUAL_PROFILE, 68.0));
        let other = RiderProfile::curated(ProfileId::Mountain, 68.0);
        assert_eq!(effective(PhysicsMode::Equal, &other, None), eq);
        // A build of another profile than the rider's is not used.
        assert_eq!(effective(PhysicsMode::Open, &other, Some(&b)), other);
        assert_eq!(effective(PhysicsMode::Open, &base, None), base);
        // Defaults: Profile for races and group rides, Open for time trials.
        assert_eq!(
            PhysicsMode::for_event(&EventKind::Race {
                length_m: 1000.0,
                laps: None
            }),
            PhysicsMode::Profile
        );
        assert_eq!(
            PhysicsMode::for_event(&EventKind::GroupRide { target_w: 150.0 }),
            PhysicsMode::Profile
        );
        assert_eq!(
            PhysicsMode::for_event(&EventKind::TimeTrial {
                length_m: 1000.0,
                laps: None
            }),
            PhysicsMode::Open
        );
        for m in PhysicsMode::ALL {
            assert_eq!(PhysicsMode::parse(m.id()), Some(m));
        }
        assert_eq!(PhysicsMode::parse("pay-to-win"), None);
    }

    #[test]
    fn hashes_and_effect_lines() {
        let a = build(
            t(),
            ProfileId::Road,
            &ids(&["tyres.road.race", "wheels.road-carbon.top"]),
        );
        let b = build(
            t(),
            ProfileId::Road,
            &ids(&["wheels.road-carbon.top", "tyres.road.race"]),
        );
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.parts, b.parts, "slot order, not the order sent");
        assert_ne!(a.hash(), 0);
        assert_ne!(
            a.hash(),
            build(t(), ProfileId::Road, &ids(&["tyres.road.race"])).hash()
        );
        assert_ne!(
            build_hash(ProfileId::Road, &ids(&["x"])),
            build_hash(ProfileId::RoadAero, &ids(&["x"]))
        );
        let e = a.effect_from(&Build::stock(ProfileId::Road));
        assert!(e.flat > 0.0 && e.mass_kg < 0.0);
        let line = e.line();
        assert!(
            line.starts_with('+') && line.contains("flat") && line.contains("at 6 %"),
            "{line}"
        );
        let back = Build::stock(ProfileId::Road).effect_from(&a);
        assert!(back.line().starts_with('−'), "{}", back.line());
        let capped = Effect {
            flat: 0.004,
            climb: -0.006,
            mass_kg: 0.0,
            capped: true,
        };
        assert_eq!(capped.line(), "+0.4 % flat, −0.6 % at 6 %, capped");
    }
}
