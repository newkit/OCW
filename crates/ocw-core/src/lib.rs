//! Shared types, units and constants used by every Open Cycling World crate.
//!
//! This crate must stay free of engine, OS and network dependencies and must
//! build for `wasm32-unknown-unknown` (spec §3.5).

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
    /// Rolling resistance coefficient.
    pub crr: f64,
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
        Self {
            rider_mass_kg: clamp_rider_weight(weight_kg),
            ..Self::default()
        }
    }
}

impl Default for RiderProfile {
    /// A 75 kg rider on an 8 kg road bike on the hoods, on smooth asphalt.
    fn default() -> Self {
        Self {
            rider_mass_kg: 75.0,
            bike_mass_kg: 8.0,
            cda_m2: 0.32,
            crr: 0.004,
        }
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
}
