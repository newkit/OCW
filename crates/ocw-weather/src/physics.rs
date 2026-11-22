//! Weather physics coefficients (spec §14 Phase 9, 9E, milestone S6): what
//! the weather does to a rider when the session's *weather affects physics*
//! switch is on (Q4) – the wind at the rider's height, the air density from
//! temperature, pressure, humidity and height, and the road's extra rolling
//! resistance and lower grip when it is wet, flooded, snowy or icy.
//!
//! The server (`ocw-sim`'s session) and the client's prediction build the
//! same [`WeatherPhysics`] from the session weather (state, gust, ground)
//! and use it the same way, so the prediction stays in step. Everything is
//! `f64` here, as in the physics; the weather state itself is `f32`.
//!
//! Scene axes as everywhere in OCW: x east, y up, z south (north is −z).

use crate::ground::Ground;
use crate::state::WeatherState;

/// Gas constant of dry air (J / (kg K)).
pub const R_DRY: f64 = 287.05;
/// Gas constant of water vapour (J / (kg K)).
pub const R_VAPOUR: f64 = 461.5;
/// Standard gravity (m/s²).
pub const G: f64 = 9.80665;
/// 0 °C in kelvin.
pub const KELVIN: f64 = 273.15;

/// Height of a rider's aerodynamic centre above the road (m).
pub const RIDER_HEIGHT_M: f64 = 1.2;
/// Roughness length of open country with hedges and scattered trees (m):
/// the logarithmic wind profile from the weather's 10 m wind down to the
/// rider.
pub const ROUGHNESS_M: f64 = 0.03;
/// Height the weather's wind is given at (m, the meteorological standard).
pub const WIND_REFERENCE_M: f64 = 10.0;

/// Share of the 10 m wind a rider feels (the log wind profile, ≈ 0.64):
/// `ln(h / z0) / ln(10 m / z0)`.
pub fn rider_wind_share() -> f64 {
    (RIDER_HEIGHT_M / ROUGHNESS_M).ln() / (WIND_REFERENCE_M / ROUGHNESS_M).ln()
}

/// Saturation vapour pressure over water (Pa) at `t_c` °C (Magnus formula,
/// Alduchov & Eskridge 1996; within 0.4 % from −40 to +50 °C).
pub fn saturation_vapour_pa(t_c: f64) -> f64 {
    610.94 * (17.625 * t_c / (t_c + 243.04)).exp()
}

/// Density of moist air (kg/m³) at `t_c` °C, `pressure_hpa` and relative
/// `humidity` (0–1): dry air and water vapour as ideal gases. 15 °C,
/// 1013.25 hPa, dry: 1.225 (the ISA sea level, `ocw_core::AIR_DENSITY`).
pub fn air_density(t_c: f64, pressure_hpa: f64, humidity: f64) -> f64 {
    let t = (t_c + KELVIN).max(150.0);
    let p = pressure_hpa.max(1.0) * 100.0;
    let vapour = (humidity.clamp(0.0, 1.0) * saturation_vapour_pa(t_c)).min(p);
    (p - vapour) / (R_DRY * t) + vapour / (R_VAPOUR * t)
}

/// Pressure (hPa) `dh_m` metres above a place with `pressure_hpa` at
/// `t_c` °C: the barometric formula for a column at that temperature
/// (within 1 % of the standard atmosphere's lapse rate over 2 km).
pub fn pressure_at(pressure_hpa: f64, t_c: f64, dh_m: f64) -> f64 {
    let t = (t_c + KELVIN).max(150.0);
    pressure_hpa * (-G * dh_m / (R_DRY * t)).exp()
}

/// Extra rolling resistance of a wet road (factor on Crr; a water film
/// costs a little, standing water more, a slushy road most).
pub fn rolling_factor(ground: &Ground) -> f64 {
    1.0 + 0.06 * f64::from(ground.wetness())
        + 0.08 * f64::from(ground.puddles())
        + 0.15 * f64::from(ground.snow_cover())
}

/// The lowest grip there is (sheet ice).
pub const MIN_GRIP: f64 = 0.3;

/// The road's grip relative to dry asphalt (1 = dry, [`MIN_GRIP`] = ice):
/// a wet road about 0.75 (tyre–road friction ~0.9 dry, ~0.65 wet), puddles
/// a little less, snow on the ground (the roads are cleared, 9C: slush)
/// down to 0.55, and a wet road below freezing (black ice) down to 0.4.
pub fn grip(ground: &Ground, temperature_c: f64) -> f64 {
    let wet = f64::from(ground.wetness());
    let puddles = f64::from(ground.puddles());
    let snow = f64::from(ground.snow_cover());
    // 0 above +1 °C, 1 below −1 °C.
    let ice = ((1.0 - temperature_c) / 2.0).clamp(0.0, 1.0);
    let g = (1.0 - 0.25 * wet - 0.05 * puddles)
        .min(1.0 - 0.45 * snow)
        .min(1.0 - 0.6 * wet * ice);
    g.clamp(MIN_GRIP, 1.0)
}

/// The weather as the physics sees it at one moment (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherPhysics {
    /// The air's velocity at the rider's height (m/s; x east, z south),
    /// with the gust of the moment.
    pub wind: [f64; 2],
    pub temperature_c: f64,
    /// Pressure (hPa) at the scene height [`WeatherPhysics::pressure_y_m`].
    pub pressure_hpa: f64,
    /// The scene height (y, m) the weather's pressure holds at: sea level
    /// for presets and the simulation, the real weather's place for real
    /// weather.
    pub pressure_y_m: f64,
    /// Relative humidity (0–1).
    pub humidity: f64,
    /// Factor on the rolling resistance ([`rolling_factor`]).
    pub rolling_factor: f64,
    /// Grip relative to a dry road ([`grip`]).
    pub grip: f64,
    /// The road's wetness (0 dry – 1 soaked, [`Ground::wetness`]): wet
    /// leaves on the road are slippery (spec 4.6.2 Q6).
    pub wetness: f64,
    /// Lying snow (0 – 1, [`Ground::snow_cover`]): from half on it hides
    /// the leaves on the road, as the renderer draws it.
    pub snow_cover: f64,
}

/// The wind relative to a rider's direction of travel (m/s).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RelativeWind {
    /// Against the rider (negative: a tailwind).
    pub head_mps: f64,
    /// Across, towards the rider's right (negative: towards the left).
    pub cross_mps: f64,
}

impl RelativeWind {
    /// The wind's speed (m/s).
    pub fn speed(&self) -> f64 {
        self.head_mps.hypot(self.cross_mps)
    }

    /// Where the wind comes from relative to the rider's heading (°, 0 =
    /// straight ahead, 90 = from the right, 180 = from behind, −90 = from
    /// the left).
    pub fn from_deg(&self) -> f64 {
        // From ahead: blowing against the rider (head > 0); from the right:
        // blowing towards the left (cross < 0).
        (-self.cross_mps).atan2(self.head_mps).to_degrees()
    }

    /// For the HUD: "headwind 18 km/h", "tailwind 9 km/h", "crosswind from
    /// the left 20 km/h"; calm below 1 km/h.
    pub fn label(&self) -> String {
        let kmh = self.speed() * 3.6;
        if kmh < 1.0 {
            return "calm".into();
        }
        let a = self.from_deg();
        let what = match a.abs() {
            x if x <= 30.0 => "headwind",
            x if x >= 150.0 => "tailwind",
            _ if a > 0.0 => "crosswind from the right",
            _ => "crosswind from the left",
        };
        format!("{what} {kmh:.0} km/h")
    }

    /// An arrow showing where the air goes relative to the rider (up =
    /// ahead): ↓ for a headwind, ↑ for a tailwind, ← for wind from the
    /// right; the eight directions of the HUD font.
    pub fn arrow(&self) -> char {
        if self.speed() * 3.6 < 1.0 {
            return '·';
        }
        // Direction the air moves, clockwise from "ahead".
        let to = (self.from_deg() + 180.0).rem_euclid(360.0);
        const ARROWS: [char; 8] = ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'];
        ARROWS[((to + 22.5) / 45.0) as usize % 8]
    }
}

impl WeatherPhysics {
    /// The physics of weather `w` with the wind `wind_mps` now (the mean
    /// and the gust of the moment, [`crate::wind::wind_now`], at 10 m), the
    /// ground, and the scene height its pressure holds at.
    pub fn new(w: &WeatherState, wind_mps: f32, ground: &Ground, pressure_y_m: f64) -> Self {
        let speed = f64::from(wind_mps.max(0.0)) * rider_wind_share();
        let from = f64::from(w.wind_from_deg).to_radians();
        let temperature_c = f64::from(w.temperature_c);
        Self {
            // The air moves away from where it comes from: from the north
            // (0°) it blows south (+z).
            wind: [-speed * from.sin(), speed * from.cos()],
            temperature_c,
            pressure_hpa: f64::from(w.pressure_hpa),
            pressure_y_m,
            humidity: f64::from(w.humidity),
            rolling_factor: rolling_factor(ground),
            grip: grip(ground, temperature_c),
            wetness: f64::from(ground.wetness()),
            snow_cover: f64::from(ground.snow_cover()),
        }
    }

    /// The session weather `w` at session time `t_s`: the gust from the
    /// session's seed (the same on the server and every client).
    pub fn at(w: &WeatherState, seed: u64, t_s: f64, ground: &Ground, pressure_y_m: f64) -> Self {
        Self::new(w, crate::wind::wind_now(w, seed, t_s), ground, pressure_y_m)
    }

    /// Sheltered from the weather (a tunnel): no wind, a dry road; the air
    /// stays the same.
    pub fn sheltered(self) -> Self {
        Self {
            wind: [0.0, 0.0],
            rolling_factor: 1.0,
            grip: 1.0,
            wetness: 0.0,
            snow_cover: 0.0,
            ..self
        }
    }

    /// Air density (kg/m³) at scene height `y_m`.
    pub fn density_at(&self, y_m: f64) -> f64 {
        let p = pressure_at(
            self.pressure_hpa,
            self.temperature_c,
            y_m - self.pressure_y_m,
        );
        air_density(self.temperature_c, p, self.humidity)
    }

    /// The wind relative to a rider travelling along the horizontal unit
    /// vector `travel` (x, z).
    pub fn relative(&self, travel: [f64; 2]) -> RelativeWind {
        // The rider's right of `travel` (as `RoutePoint::right`).
        let right = [-travel[1], travel[0]];
        RelativeWind {
            head_mps: -(self.wind[0] * travel[0] + self.wind[1] * travel[1]),
            cross_mps: self.wind[0] * right[0] + self.wind[1] * right[1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset;

    #[test]
    fn air_density_from_temperature_pressure_humidity_and_height() {
        // The standard atmosphere at sea level.
        assert!((air_density(15.0, 1013.25, 0.0) - 1.225).abs() < 0.001);
        // Colder air is denser, warmer thinner (ideal gas: ρ ∝ 1/T).
        let cold = air_density(-10.0, 1013.25, 0.0);
        let hot = air_density(35.0, 1013.25, 0.0);
        assert!((cold - 1.341).abs() < 0.002, "{cold}");
        assert!((hot - 1.146).abs() < 0.002, "{hot}");
        // Humid air is a little lighter than dry air (water vapour is
        // lighter than nitrogen): ~1 % at 30 °C.
        let dry = air_density(30.0, 1013.25, 0.0);
        let humid = air_density(30.0, 1013.25, 1.0);
        assert!(humid < dry && (dry - humid) / dry < 0.02, "{dry} {humid}");
        // Height: 1600 m (Malbun) has ~16 % less air than the sea.
        let w = WeatherPhysics::new(
            &WeatherState {
                temperature_c: 15.0,
                pressure_hpa: 1013.25,
                humidity: 0.0,
                ..Default::default()
            },
            0.0,
            &Ground::default(),
            0.0,
        );
        let sea = w.density_at(0.0);
        let malbun = w.density_at(1600.0);
        assert!((sea - 1.225).abs() < 0.001);
        let drop = 1.0 - malbun / sea;
        assert!((0.15..0.19).contains(&drop), "{drop}");
        // The pressure's reference height moves the column with it.
        let real = WeatherPhysics {
            pressure_y_m: 1600.0,
            ..w
        };
        assert!((real.density_at(1600.0) - sea).abs() < 1e-12);
        // Within 1 % of the standard atmosphere at 2000 m (795 hPa).
        let p = pressure_at(1013.25, 15.0, 2000.0);
        assert!((p - 795.0).abs() < 8.0, "{p}");
        assert!((saturation_vapour_pa(20.0) - 2339.0).abs() < 10.0);
    }

    #[test]
    fn wind_relative_to_the_rider() {
        let still = WeatherState {
            wind_mps: 10.0,
            gust_mps: 10.0,
            wind_from_deg: 0.0, // from the north
            ..Default::default()
        };
        let w = WeatherPhysics::new(&still, 10.0, &Ground::default(), 0.0);
        let share = rider_wind_share();
        assert!((0.6..0.7).contains(&share), "{share}");
        // North is −z: riding north is straight into it.
        let north = w.relative([0.0, -1.0]);
        assert!((north.head_mps - 10.0 * share).abs() < 1e-9, "{north:?}");
        assert!(north.cross_mps.abs() < 1e-9);
        assert_eq!(north.arrow(), '↓');
        assert!(
            north.label().starts_with("headwind 23 km/h"),
            "{}",
            north.label()
        );
        // South: a tailwind.
        let south = w.relative([0.0, 1.0]);
        assert!((south.head_mps + 10.0 * share).abs() < 1e-9);
        assert_eq!(south.arrow(), '↑');
        assert!(south.label().starts_with("tailwind"));
        // East (+x): the wind from the north comes from the left and blows
        // to the right.
        let east = w.relative([1.0, 0.0]);
        assert!(east.head_mps.abs() < 1e-9);
        assert!((east.cross_mps - 10.0 * share).abs() < 1e-9, "{east:?}");
        assert!((east.from_deg() + 90.0).abs() < 1e-9);
        assert_eq!(east.arrow(), '→');
        assert_eq!(east.label(), "crosswind from the left 23 km/h");
        let west = w.relative([-1.0, 0.0]);
        assert_eq!(west.arrow(), '←');
        assert!(west.label().starts_with("crosswind from the right"));
        // Calm.
        let calm = WeatherPhysics::new(&still, 0.0, &Ground::default(), 0.0);
        assert_eq!(calm.relative([1.0, 0.0]).label(), "calm");
        // A tunnel shelters from it.
        assert_eq!(w.sheltered().relative([0.0, -1.0]).speed(), 0.0);
    }

    #[test]
    fn gusts_are_the_sessions() {
        let storm = preset("storm").unwrap();
        let g = Ground::default();
        let a = WeatherPhysics::at(&storm, 7, 12.3, &g, 0.0);
        assert_eq!(a, WeatherPhysics::at(&storm, 7, 12.3, &g, 0.0));
        let speeds: Vec<f64> = (0..600)
            .map(|i| {
                let w = WeatherPhysics::at(&storm, 7, f64::from(i) * 0.5, &g, 0.0);
                w.wind[0].hypot(w.wind[1])
            })
            .collect();
        let lo = speeds.iter().cloned().fold(f64::MAX, f64::min);
        let hi = speeds.iter().cloned().fold(f64::MIN, f64::max);
        let share = rider_wind_share();
        assert!(lo >= f64::from(storm.wind_mps) * share - 1e-6);
        assert!(hi <= f64::from(storm.gust_mps) * share + 1e-6);
        assert!(hi - lo > 3.0, "{lo} {hi}");
    }

    #[test]
    fn wet_snowy_and_icy_roads() {
        let dry = Ground::default();
        assert_eq!(rolling_factor(&dry), 1.0);
        assert_eq!(grip(&dry, 15.0), 1.0);
        let wet = Ground::with_factors(1.0, 0.0, 0.0);
        let flooded = Ground::with_factors(1.0, 1.0, 0.0);
        let snowy = Ground::with_factors(1.0, 0.0, 1.0);
        // Rolling resistance a little higher (spec 9E: "slightly").
        assert!((1.05..1.08).contains(&rolling_factor(&wet)));
        assert!(rolling_factor(&flooded) > rolling_factor(&wet));
        assert!(rolling_factor(&snowy) < 1.35);
        // Grip lower: wet, flooded, snowy, icy.
        let g_wet = grip(&wet, 10.0);
        assert!((0.7..0.8).contains(&g_wet), "{g_wet}");
        assert!(grip(&flooded, 10.0) < g_wet);
        assert!(grip(&snowy, 10.0) <= 0.55 + 1e-9);
        let ice = grip(&wet, -5.0);
        assert!((ice - 0.4).abs() < 1e-9, "{ice}");
        assert!(grip(&wet, 0.0) > ice && grip(&wet, 0.0) < g_wet);
        assert!(grip(&Ground::with_factors(1.0, 1.0, 1.0), -20.0) >= MIN_GRIP);
        // Sheltered: dry.
        let w = WeatherPhysics::new(&preset("heavy-rain").unwrap(), 5.0, &flooded, 0.0);
        assert!(w.grip < 0.8 && w.rolling_factor > 1.1);
        let t = w.sheltered();
        assert_eq!((t.grip, t.rolling_factor), (1.0, 1.0));
        assert_eq!(t.density_at(100.0), w.density_at(100.0));
    }
}
