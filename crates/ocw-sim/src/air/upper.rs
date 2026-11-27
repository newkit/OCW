//! The air aloft from the weather at the ground (spec 18C: contrails "when
//! the upper air is cold and humid enough", gliders "on days with
//! thermals", balloons "on calm mornings"). The weather of Phase 9 knows
//! the ground only, so the upper air is estimated from it:
//!
//! - **temperature** by the standard lapse rate (6.5 K per km above the
//!   ground, constant above 11 km, the tropopause);
//! - **humidity** aloft from the ground's humidity and the clouds: ice
//!   veils (cirrus) mean saturated upper air, a grey layer or a front
//!   some, fair-weather heaps little, a dry clear day little;
//! - **contrails** by the Schmidt–Appleman criterion (Schumann 1996): the
//!   exhaust's mixing line reaches water saturation below the threshold
//!   temperature `T_LM = −46.46 + 9.43 ln(G − 0.053) + 0.72 ln²(G − 0.053)`
//!   (°C; `G` ≈ 6.7·10⁻⁵ K⁻¹ × the pressure in Pa for kerosene at an
//!   overall efficiency of 0.3), lowered by up to 6 K in dry air; a trail
//!   that forms lasts only where the air is saturated over ice
//!   (persistent, spreading, drifting with the wind) and is short where
//!   it is drier – and none is seen in dry air (below 70 % over ice).

use ocw_weather::state::{CloudType, Precip, WeatherState};

/// The weather at the ground and what follows for the air above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperAir {
    /// The weather's temperature (°C) at `ground_m` above the sea.
    pub ground_c: f64,
    pub ground_m: f64,
    /// Relative humidity over water aloft (0–1, the same at every height
    /// of the airways).
    pub humidity_aloft: f64,
    /// The wind at the ground (m/s, toward x, z) and from where (°).
    pub wind: [f64; 2],
    pub wind_from_deg: f64,
}

/// What an airliner's exhaust leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Contrail {
    /// Too warm, or the trail evaporates at once in dry air.
    None,
    /// A short trail that fades within half a minute.
    Short,
    /// A long trail that spreads and drifts (ice-saturated air).
    Persistent,
}

impl Contrail {
    /// How long a piece of the trail is seen (s).
    pub fn lifetime_s(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Short => 25.0,
            Self::Persistent => 600.0,
        }
    }
}

/// Saturation vapour pressure over water (hPa), Magnus (WMO).
pub fn e_water(t_c: f64) -> f64 {
    6.112 * (17.62 * t_c / (243.12 + t_c)).exp()
}

/// Saturation vapour pressure over ice (hPa), Magnus.
pub fn e_ice(t_c: f64) -> f64 {
    6.112 * (22.46 * t_c / (272.62 + t_c)).exp()
}

/// Pressure of the standard atmosphere (hPa) at `alt_m`.
pub fn pressure_hpa(alt_m: f64) -> f64 {
    1013.25
        * (1.0 - 2.255_77e-5 * alt_m.clamp(0.0, 11_000.0)).powf(5.255_88)
        * if alt_m > 11_000.0 {
            (-(alt_m - 11_000.0) / 6341.6).exp()
        } else {
            1.0
        }
}

/// The Schmidt–Appleman threshold (°C) at `p_hpa` for relative humidity
/// over water `rh` (0–1).
pub fn appleman_threshold_c(p_hpa: f64, rh: f64) -> f64 {
    let g = 6.67e-5 * p_hpa * 100.0;
    let l = (g - 0.053).max(1e-3).ln();
    let t_lm = -46.46 + 9.43 * l + 0.72 * l * l;
    t_lm - 6.0 * (1.0 - rh.clamp(0.0, 1.0))
}

impl UpperAir {
    /// From the session's weather at a rider standing `ground_m` above the
    /// sea.
    pub fn from_weather(w: &WeatherState, ground_m: f64) -> Self {
        let cover = f64::from(w.cloud_cover.clamp(0.0, 1.0));
        let cloud = match w.cloud_type {
            CloudType::Cirrus => 0.4,
            CloudType::Cumulonimbus => 0.3,
            CloudType::Stratus => 0.12,
            CloudType::Cumulus => 0.05,
        } * cover;
        let front = match w.precip {
            Precip::None => 0.0,
            _ => 0.12,
        };
        let humidity_aloft =
            (0.25 + 0.45 * f64::from(w.humidity.clamp(0.0, 1.0)) + cloud + front).clamp(0.05, 1.0);
        let from = f64::from(w.wind_from_deg).to_radians();
        let speed = f64::from(w.wind_mps.max(0.0));
        // Blowing toward the opposite of where it comes from (x east, z south).
        let wind = [-from.sin() * speed, from.cos() * speed];
        Self {
            ground_c: f64::from(w.temperature_c),
            ground_m,
            humidity_aloft,
            wind,
            wind_from_deg: f64::from(w.wind_from_deg),
        }
    }

    /// Temperature (°C) at `alt_m` above the sea.
    pub fn temperature_c(&self, alt_m: f64) -> f64 {
        let top = alt_m.min(11_000.0).max(self.ground_m);
        self.ground_c - 6.5e-3 * (top - self.ground_m)
    }

    /// Relative humidity over ice at `alt_m` (above 1: supersaturated).
    pub fn humidity_ice(&self, alt_m: f64) -> f64 {
        let t = self.temperature_c(alt_m);
        self.humidity_aloft * e_water(t) / e_ice(t)
    }

    /// The trail an airliner leaves at `alt_m`.
    pub fn contrail(&self, alt_m: f64) -> Contrail {
        if alt_m < 6000.0 {
            return Contrail::None;
        }
        let t = self.temperature_c(alt_m);
        if t > appleman_threshold_c(pressure_hpa(alt_m), self.humidity_aloft) {
            return Contrail::None;
        }
        let rhi = self.humidity_ice(alt_m);
        if rhi >= 1.0 {
            Contrail::Persistent
        } else if rhi >= 0.7 {
            Contrail::Short
        } else {
            Contrail::None
        }
    }

    /// The wind at `alt_m` (m/s toward x, z): stronger aloft (about
    /// 10 m/s more per 3 km, from the same side).
    pub fn wind_at(&self, alt_m: f64) -> [f64; 2] {
        let up = ((alt_m - self.ground_m).max(0.0) / 3000.0).min(4.0);
        let from = self.wind_from_deg.to_radians();
        let extra = 10.0 * up;
        [
            self.wind[0] - from.sin() * extra,
            self.wind[1] + from.cos() * extra,
        ]
    }
}

/// How strong the thermals are (0 none … 1 a classic soaring day): sun
/// high (over 25°, best above 45°), fair-weather heaps or a clear sky, no
/// rain, little wind; spring to early autumn (all year in the
/// subtropics, below 35° latitude).
pub fn thermals(w: &WeatherState, sun_elevation_deg: f64, month: u32, lat: f64) -> f64 {
    let season = lat.abs() < 35.0 || (3..=9).contains(&month);
    if !season
        || w.precip != Precip::None
        || w.lightning_per_min > 0.0
        || w.wind_mps > 9.0
        || !matches!(w.cloud_type, CloudType::Cumulus) && w.cloud_cover > 0.3
        || w.cloud_cover > 0.75
        || w.fog_visibility_m < 5000.0
    {
        return 0.0;
    }
    let sun = ((sun_elevation_deg - 25.0) / 20.0).clamp(0.0, 1.0);
    let cover = 1.0 - (f64::from(w.cloud_cover) - 0.5).max(0.0) * 2.0;
    sun * cover
}

/// A calm morning for balloons: little wind, no rain, no storm, a clear
/// enough view.
pub fn calm(w: &WeatherState) -> bool {
    w.wind_mps < 3.5
        && w.gust_mps < 7.0
        && w.precip == Precip::None
        && w.lightning_per_min == 0.0
        && w.fog_visibility_m > 3000.0
        && !(w.cloud_type == CloudType::Cumulonimbus && w.cloud_cover > 0.2)
}

/// Weather for small planes, helicopters' tours and cross-country flights
/// (visual flight rules): no more than light rain, 5 km to see, the
/// cloud base over 450 m (or few clouds), no storm, under 13 m/s.
pub fn fine(w: &WeatherState) -> bool {
    w.precip_mm_h < 0.5
        && w.fog_visibility_m > 5000.0
        && (w.cloud_base_m > 450.0 || w.cloud_cover < 0.5)
        && w.lightning_per_min == 0.0
        && w.wind_mps < 13.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(t: f32, rh: f32, cover: f32, kind: CloudType) -> WeatherState {
        WeatherState {
            temperature_c: t,
            humidity: rh,
            cloud_cover: cover,
            cloud_type: kind,
            ..Default::default()
        }
    }

    #[test]
    fn contrails_only_in_cold_humid_upper_air() {
        // A cold, humid winter day with ice veils: long trails at cruise.
        let cold = UpperAir::from_weather(&day(2.0, 0.85, 0.4, CloudType::Cirrus), 400.0);
        assert_eq!(cold.contrail(10_700.0), Contrail::Persistent);
        assert_eq!(cold.contrail(11_900.0), Contrail::Persistent);
        // A hot dry summer day: none at all.
        let hot = UpperAir::from_weather(&day(33.0, 0.25, 0.0, CloudType::Cumulus), 0.0);
        for alt in [8000.0, 9500.0, 10_700.0, 11_900.0] {
            assert_eq!(hot.contrail(alt), Contrail::None, "{alt} m");
        }
        // A mild clear day: short trails that fade.
        let mild = UpperAir::from_weather(&WeatherState::default(), 400.0);
        assert_eq!(mild.contrail(11_000.0), Contrail::Short);
        // Warm enough low down: never below 6 km.
        assert_eq!(cold.contrail(5000.0), Contrail::None);
        // Dry air aloft even when cold: none seen.
        let dry = UpperAir {
            humidity_aloft: 0.2,
            ..cold
        };
        assert_eq!(dry.contrail(11_000.0), Contrail::None);
    }

    #[test]
    fn the_threshold_is_near_minus_40_at_cruise() {
        let t = appleman_threshold_c(250.0, 1.0);
        assert!((-44.0..-39.0).contains(&t), "{t}");
        assert!(appleman_threshold_c(250.0, 0.0) < t);
        assert!((pressure_hpa(10_363.0) - 250.0).abs() < 8.0);
        assert!(e_ice(-40.0) < e_water(-40.0));
    }

    #[test]
    fn wind_and_thermals_and_calm() {
        let w = WeatherState {
            wind_mps: 4.0,
            wind_from_deg: 270.0,
            ..Default::default()
        };
        let u = UpperAir::from_weather(&w, 0.0);
        // From the west: blowing east.
        assert!(u.wind[0] > 3.9 && u.wind[1].abs() < 1e-6);
        assert!(u.wind_at(10_000.0)[0] > 30.0);
        let summer = day(26.0, 0.5, 0.3, CloudType::Cumulus);
        assert!(thermals(&summer, 55.0, 7, 48.0) > 0.9);
        assert_eq!(thermals(&summer, 55.0, 12, 48.0), 0.0, "winter in the Alps");
        assert!(thermals(&summer, 55.0, 12, 28.3) > 0.9, "Tenerife");
        assert_eq!(thermals(&summer, 15.0, 7, 48.0), 0.0, "low sun");
        let rain = WeatherState {
            precip: Precip::Rain,
            precip_mm_h: 3.0,
            ..summer
        };
        assert_eq!(thermals(&rain, 55.0, 7, 48.0), 0.0);
        assert!(!fine(&rain));
        assert!(calm(&WeatherState::default()));
        assert!(!calm(&WeatherState {
            wind_mps: 8.0,
            ..Default::default()
        }));
    }
}
