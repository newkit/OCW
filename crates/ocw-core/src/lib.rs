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

impl RiderProfile {
    /// Total moving mass in kg.
    pub fn total_mass_kg(&self) -> f64 {
        self.rider_mass_kg + self.bike_mass_kg
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
}
