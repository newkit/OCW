//! Shared types, units and constants used by every Open Cycling World crate.
//!
//! This crate must stay free of engine, OS and network dependencies and must
//! build for `wasm32-unknown-unknown` (spec §3.5).

pub mod bike;
pub mod character;
pub mod hearts;
pub mod timestamp;

pub use bike::{BikeType, KitPattern, PhysicsProfile, ProfileId, Rgb, RiderLook};
pub use character::{BodyBuild, Character};

/// Physics tick rate on client and server (spec §5.2).
pub const SIM_TICK_HZ: u32 = 50;
/// Length of one physics tick in seconds.
pub const SIM_DT: f64 = 1.0 / SIM_TICK_HZ as f64;
/// Rate at which the server broadcasts rider state (spec §6.3).
pub const BROADCAST_HZ: u32 = 10;
/// Number of physics ticks between two broadcasts.
pub const TICKS_PER_BROADCAST: u32 = SIM_TICK_HZ / BROADCAST_HZ;
/// Interpolation delay used when rendering other riders (spec §6.3).
pub const INTERPOLATION_DELAY_S: f64 = 0.12;

/// Standard gravity in m/s².
pub const GRAVITY: f64 = 9.80665;
/// Air density at sea level, 15 °C, in kg/m³.
pub const AIR_DENSITY: f64 = 1.225;

/// Identifier of a rider within a session. Assigned by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RiderId(pub u32);

impl std::fmt::Display for RiderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Physical properties of a rider and their bike (physics profile, spec §2.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiderProfile {
    /// Body mass of the rider in kg.
    pub rider_mass_kg: f64,
    /// Mass of the bike in kg.
    pub bike_mass_kg: f64,
    /// Drag area (drag coefficient × frontal area) in m².
    pub cda_m2: f64,
    /// Rolling resistance coefficient (on asphalt).
    pub crr: f64,
    /// Share of the pedal power that reaches the rear wheel.
    pub drivetrain_efficiency: f64,
    /// How much of a rough surface's extra rolling resistance the bike
    /// feels ([`bike::surface_rolling_factor`]).
    pub rough_surface_scale: f64,
    /// The curated profile these numbers come from (spec §2.5).
    pub bike: ProfileId,
}

/// Lightest rider weight the simulation accepts, in kg (spec §3.2.1).
pub const MIN_RIDER_WEIGHT_KG: f64 = 30.0;
/// Heaviest rider weight the simulation accepts, in kg.
pub const MAX_RIDER_WEIGHT_KG: f64 = 200.0;

/// A rider weight within [`MIN_RIDER_WEIGHT_KG`]..=[`MAX_RIDER_WEIGHT_KG`];
/// a weight that is not a number becomes the default rider's 75 kg.
pub fn clamp_rider_weight(kg: f64) -> f64 {
    if kg.is_nan() {
        RiderProfile::default().rider_mass_kg
    } else {
        kg.clamp(MIN_RIDER_WEIGHT_KG, MAX_RIDER_WEIGHT_KG)
    }
}

impl RiderProfile {
    /// Total moving mass in kg.
    pub fn total_mass_kg(&self) -> f64 {
        self.rider_mass_kg + self.bike_mass_kg
    }

    /// The default physics profile (bike, drag, rolling resistance) for a
    /// rider of `weight_kg` (clamped, [`clamp_rider_weight`]). Riders choose
    /// only their weight; the rest comes from the curated profile (§2.5).
    pub fn for_rider_weight(weight_kg: f64) -> Self {
        Self::curated(ProfileId::default(), weight_kg)
    }

    /// The curated profile `bike` (spec §2.5) for a rider of `weight_kg`
    /// (clamped): the only way physics parameters are made.
    pub fn curated(bike: ProfileId, weight_kg: f64) -> Self {
        let p = bike.profile();
        Self {
            rider_mass_kg: clamp_rider_weight(weight_kg),
            bike_mass_kg: p.bike_mass_kg,
            cda_m2: p.cda_m2,
            crr: p.crr,
            drivetrain_efficiency: p.drivetrain_efficiency,
            rough_surface_scale: p.rough_surface_scale,
            bike,
        }
    }

    /// The factor on [`RiderProfile::crr`] on a surface with the road
    /// bike's factor `surface_factor` (asphalt 1, gravel 2.5, grass 6).
    pub fn surface_factor(&self, surface_factor: f64) -> f64 {
        bike::surface_rolling_factor(self.rough_surface_scale, surface_factor)
    }
}

impl Default for RiderProfile {
    /// A 75 kg rider on an 8 kg road bike on the hoods, on smooth asphalt.
    fn default() -> Self {
        Self::curated(ProfileId::Road, 75.0)
    }
}

/// Converts metres per second to kilometres per hour.
pub fn mps_to_kmh(mps: f64) -> f64 {
    mps * 3.6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_divides_tick_rate() {
        assert_eq!(SIM_TICK_HZ % BROADCAST_HZ, 0);
        assert_eq!(TICKS_PER_BROADCAST, 5);
    }

    #[test]
    fn default_profile_mass() {
        assert_eq!(RiderProfile::default().total_mass_kg(), 83.0);
    }

    #[test]
    fn rider_weight_is_clamped() {
        assert_eq!(RiderProfile::for_rider_weight(62.5).total_mass_kg(), 70.5);
        assert_eq!(clamp_rider_weight(5.0), MIN_RIDER_WEIGHT_KG);
        assert_eq!(clamp_rider_weight(500.0), MAX_RIDER_WEIGHT_KG);
        assert_eq!(clamp_rider_weight(f64::NAN), 75.0);
        assert_eq!(clamp_rider_weight(f64::INFINITY), MAX_RIDER_WEIGHT_KG);
        let p = RiderProfile::for_rider_weight(90.0);
        assert_eq!((p.cda_m2, p.crr, p.bike_mass_kg), (0.32, 0.004, 8.0));
    }

    #[test]
    fn curated_profiles_take_only_the_weight_from_the_rider() {
        let p = RiderProfile::curated(ProfileId::Mountain, 64.0);
        assert_eq!(p.rider_mass_kg, 64.0);
        assert_eq!(p.bike_mass_kg, 11.0);
        assert_eq!(p.bike, ProfileId::Mountain);
        assert_eq!(
            RiderProfile::curated(ProfileId::Gravel, 900.0).rider_mass_kg,
            MAX_RIDER_WEIGHT_KG
        );
        assert_eq!(RiderProfile::default().bike, ProfileId::Road);
        assert_eq!(RiderProfile::default().surface_factor(2.5), 2.5);
        assert!(p.surface_factor(2.5) < 2.5);
        assert_eq!(p.surface_factor(1.0), 1.0);
    }
}
