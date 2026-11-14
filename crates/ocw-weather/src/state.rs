//! The weather at one moment (spec §14 Phase 9, 9E): clouds, precipitation,
//! fog, wind, temperature, humidity, pressure and lightning, and how two
//! states blend ([`WeatherState::lerp`]).
//!
//! Scene axes as everywhere in OCW: x east, y up, z south (north is −z).

/// The dominant cloud type (spec 9E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CloudType {
    /// Fair-weather heaps: patchy, bright tops, sun between them.
    #[default]
    Cumulus,
    /// A grey layer: flat light, no shadows when it closes.
    Stratus,
    /// High ice veils: thin, the sun shines through.
    Cirrus,
    /// Thunderstorm towers: very dense, dark bases, lightning.
    Cumulonimbus,
}

impl CloudType {
    pub const ALL: [CloudType; 4] = [
        CloudType::Cumulus,
        CloudType::Stratus,
        CloudType::Cirrus,
        CloudType::Cumulonimbus,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cumulus => "cumulus",
            Self::Stratus => "stratus",
            Self::Cirrus => "cirrus",
            Self::Cumulonimbus => "cumulonimbus",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.name() == s || (s == "cb" && *t == Self::Cumulonimbus))
    }

    /// How much light a full cover of this type stops (0–1).
    pub fn opacity(self) -> f32 {
        match self {
            Self::Cumulus => 0.85,
            Self::Stratus => 0.93,
            Self::Cirrus => 0.3,
            Self::Cumulonimbus => 0.99,
        }
    }
}

/// What falls (spec 9E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Precip {
    #[default]
    None,
    Drizzle,
    Rain,
    Snow,
    Sleet,
    Hail,
}

impl Precip {
    pub const ALL: [Precip; 6] = [
        Precip::None,
        Precip::Drizzle,
        Precip::Rain,
        Precip::Snow,
        Precip::Sleet,
        Precip::Hail,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Drizzle => "drizzle",
            Self::Rain => "rain",
            Self::Snow => "snow",
            Self::Sleet => "sleet",
            Self::Hail => "hail",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    /// Share of the precipitation that lands as ice or snow at
    /// `temperature_c` (the rest is water): snow melts in warm air, sleet
    /// is half and half, hail is ice.
    pub fn frozen_share(self, temperature_c: f32) -> f32 {
        match self {
            Self::None | Self::Drizzle | Self::Rain => 0.0,
            Self::Snow => 1.0 - smooth(0.5, 3.0, temperature_c),
            Self::Sleet => 0.5 * (1.0 - smooth(1.0, 4.0, temperature_c)),
            Self::Hail => 0.85,
        }
    }
}

/// The weather at one moment, at the rider's place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherState {
    /// Share of the sky covered (0–1).
    pub cloud_cover: f32,
    pub cloud_type: CloudType,
    /// Height of the cloud base above the ground at the rider (m).
    pub cloud_base_m: f32,
    pub precip: Precip,
    /// Precipitation intensity, water equivalent (mm/h; no cap, spec Q9).
    pub precip_mm_h: f32,
    /// Visibility in fog or haze (m), without the precipitation's own
    /// ([`WeatherState::visibility_m`] combines both).
    pub fog_visibility_m: f32,
    /// Mean wind and peak gusts (m/s, 10 m above the ground).
    pub wind_mps: f32,
    pub gust_mps: f32,
    /// Where the wind comes from (° clockwise from north, meteorological).
    pub wind_from_deg: f32,
    pub temperature_c: f32,
    /// Relative humidity (0–1).
    pub humidity: f32,
    /// Air pressure at the place (hPa).
    pub pressure_hpa: f32,
    /// Lightning strikes per minute within ~15 km.
    pub lightning_per_min: f32,
}

impl Default for WeatherState {
    /// Clear and calm (until Phase 9 S7 the default of every session).
    fn default() -> Self {
        Self {
            cloud_cover: 0.05,
            cloud_type: CloudType::Cumulus,
            cloud_base_m: 1500.0,
            precip: Precip::None,
            precip_mm_h: 0.0,
            fog_visibility_m: 40_000.0,
            wind_mps: 2.0,
            gust_mps: 4.0,
            wind_from_deg: 250.0,
            temperature_c: 18.0,
            humidity: 0.55,
            pressure_hpa: 1018.0,
            lightning_per_min: 0.0,
        }
    }
}

/// Smoothstep of `x` between `a` and `b` (0 below, 1 above).
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The shortest way from angle `a` to `b` (°), at `t`.
pub fn mix_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + 540.0).rem_euclid(360.0) - 180.0;
    (a + d * t).rem_euclid(360.0)
}

/// Visibility in clear air: the default and the upper bound (m).
pub const CLEAR_VISIBILITY_M: f32 = 40_000.0;
/// The densest fog accepted (m).
pub const MIN_VISIBILITY_M: f32 = 10.0;

/// The 16 compass points.
const COMPASS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];

/// "SW" for 225°.
pub fn compass(deg: f32) -> &'static str {
    COMPASS[((deg.rem_euclid(360.0) / 22.5).round() as usize) % 16]
}

/// What the HUD and the menu call the weather.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Clear,
    Fair,
    Cloudy,
    Overcast,
    Fog,
    Drizzle,
    Rain,
    HeavyRain,
    Thunderstorm,
    Snow,
    HeavySnow,
    Sleet,
    Hail,
}

impl Condition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Fair => "fair",
            Self::Cloudy => "cloudy",
            Self::Overcast => "overcast",
            Self::Fog => "fog",
            Self::Drizzle => "drizzle",
            Self::Rain => "rain",
            Self::HeavyRain => "heavy rain",
            Self::Thunderstorm => "thunderstorm",
            Self::Snow => "snow",
            Self::HeavySnow => "heavy snow",
            Self::Sleet => "sleet",
            Self::Hail => "hail",
        }
    }
}

impl WeatherState {
    /// Within the ranges the simulation and the renderer handle. Extreme
    /// weather stays (spec Q9: no cap), only the impossible goes.
    pub fn clamped(self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        let wind = fin(self.wind_mps, d.wind_mps).clamp(0.0, 90.0);
        Self {
            cloud_cover: fin(self.cloud_cover, d.cloud_cover).clamp(0.0, 1.0),
            cloud_type: self.cloud_type,
            cloud_base_m: fin(self.cloud_base_m, d.cloud_base_m).clamp(0.0, 13_000.0),
            precip: self.precip,
            precip_mm_h: fin(self.precip_mm_h, 0.0).clamp(0.0, 400.0),
            fog_visibility_m: fin(self.fog_visibility_m, d.fog_visibility_m)
                .clamp(MIN_VISIBILITY_M, CLEAR_VISIBILITY_M),
            wind_mps: wind,
            gust_mps: fin(self.gust_mps, wind).clamp(wind, 120.0),
            wind_from_deg: fin(self.wind_from_deg, d.wind_from_deg).rem_euclid(360.0),
            temperature_c: fin(self.temperature_c, d.temperature_c).clamp(-60.0, 60.0),
            humidity: fin(self.humidity, d.humidity).clamp(0.0, 1.0),
            pressure_hpa: fin(self.pressure_hpa, d.pressure_hpa).clamp(300.0, 1100.0),
            lightning_per_min: fin(self.lightning_per_min, 0.0).clamp(0.0, 120.0),
        }
    }

    /// Whether anything falls.
    pub fn precipitating(&self) -> bool {
        self.precip != Precip::None && self.precip_mm_h > 0.0
    }

    /// The state at `t` (0 = `self`, 1 = `to`) of a smooth transition:
    /// numbers blend linearly (visibility in log space, so fog thickens
    /// evenly; the wind direction the short way round); the cloud type
    /// changes half way; what falls starts with the new intensity rising
    /// from zero and stops when the old one has fallen to zero.
    pub fn lerp(&self, to: &WeatherState, t: f32) -> WeatherState {
        if t <= 0.0 || self == to {
            return *self;
        }
        if t >= 1.0 {
            return *to;
        }
        let (a, b) = (self, to);
        let (precip, mm) = match (a.precipitating(), b.precipitating()) {
            (false, false) => (b.precip, 0.0),
            (false, true) => (b.precip, b.precip_mm_h * t),
            (true, false) => (a.precip, a.precip_mm_h * (1.0 - t)),
            (true, true) => (
                if t < 0.5 { a.precip } else { b.precip },
                mix(a.precip_mm_h, b.precip_mm_h, t),
            ),
        };
        let vis = (a.fog_visibility_m.max(1.0).ln() * (1.0 - t)
            + b.fog_visibility_m.max(1.0).ln() * t)
            .exp();
        WeatherState {
            cloud_cover: mix(a.cloud_cover, b.cloud_cover, t),
            cloud_type: if t < 0.5 { a.cloud_type } else { b.cloud_type },
            cloud_base_m: mix(a.cloud_base_m, b.cloud_base_m, t),
            precip,
            precip_mm_h: mm,
            fog_visibility_m: vis,
            wind_mps: mix(a.wind_mps, b.wind_mps, t),
            gust_mps: mix(a.gust_mps, b.gust_mps, t),
            wind_from_deg: mix_angle(a.wind_from_deg, b.wind_from_deg, t),
            temperature_c: mix(a.temperature_c, b.temperature_c, t),
            humidity: mix(a.humidity, b.humidity, t),
            pressure_hpa: mix(a.pressure_hpa, b.pressure_hpa, t),
            lightning_per_min: mix(a.lightning_per_min, b.lightning_per_min, t),
        }
    }

    /// Visibility (m) through fog and what falls: their extinction
    /// coefficients add (Koschmieder, V = 3.912 / σ).
    pub fn visibility_m(&self) -> f32 {
        crate::particles::visibility_m(self.fog_visibility_m, self.precip, self.precip_mm_h)
    }

    /// The wind's velocity (m/s, scene axes x east, z south) at `speed`:
    /// it blows away from `wind_from_deg`.
    pub fn wind_velocity(&self, speed: f32) -> [f32; 2] {
        let a = self.wind_from_deg.to_radians();
        [-a.sin() * speed, a.cos() * speed]
    }

    /// What it is, for the HUD and the menu.
    pub fn condition(&self) -> Condition {
        let mm = if self.precipitating() {
            self.precip_mm_h
        } else {
            0.0
        };
        if mm > 0.0 {
            return match self.precip {
                Precip::Rain if self.lightning_per_min > 0.2 => Condition::Thunderstorm,
                Precip::Hail => Condition::Hail,
                Precip::Rain if mm >= 8.0 => Condition::HeavyRain,
                Precip::Rain => Condition::Rain,
                Precip::Drizzle => Condition::Drizzle,
                Precip::Snow if mm >= 4.0 => Condition::HeavySnow,
                Precip::Snow => Condition::Snow,
                Precip::Sleet => Condition::Sleet,
                Precip::None => unreachable!("precipitating"),
            };
        }
        if self.lightning_per_min > 0.2 {
            return Condition::Thunderstorm;
        }
        if self.fog_visibility_m < 1000.0 {
            return Condition::Fog;
        }
        match self.cloud_cover {
            c if c < 0.15 => Condition::Clear,
            c if c < 0.45 => Condition::Fair,
            c if c < 0.85 => Condition::Cloudy,
            _ => Condition::Overcast,
        }
    }

    /// "rain 4.0 mm/h, 9 °C, wind 5 m/s from SW, gusts 9" (menu, console).
    pub fn summary(&self) -> String {
        let mut s = self.condition().label().to_owned();
        if self.precipitating() {
            s += &format!(" {:.1} mm/h", self.precip_mm_h);
        }
        if self.visibility_m() < 5000.0 {
            s += &format!(", visibility {:.0} m", self.visibility_m());
        }
        s += &format!(
            ", {:.0} °C, wind {:.0} m/s from {}",
            self.temperature_c,
            self.wind_mps,
            compass(self.wind_from_deg)
        );
        if self.gust_mps > self.wind_mps + 2.0 {
            s += &format!(", gusts {:.0}", self.gust_mps);
        }
        s
    }

    /// "rain 9 °C" (the HUD line, short).
    pub fn hud(&self) -> String {
        format!("{} {:.0} °C", self.condition().label(), self.temperature_c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset;

    #[test]
    fn transitions_blend_smoothly_and_end_at_the_target() {
        let clear = preset("clear").unwrap();
        let rain = preset("rain").unwrap();
        assert_eq!(clear.lerp(&rain, 0.0), clear);
        assert_eq!(clear.lerp(&rain, 1.0), rain);
        let half = clear.lerp(&rain, 0.5);
        assert!(half.cloud_cover > clear.cloud_cover && half.cloud_cover < rain.cloud_cover);
        // The rain starts at once, its intensity rises from zero.
        assert_eq!(clear.lerp(&rain, 0.1).precip, Precip::Rain);
        assert!((clear.lerp(&rain, 0.1).precip_mm_h - 0.1 * rain.precip_mm_h).abs() < 1e-4);
        // ...and stops when it has faded out the other way round.
        let back = rain.lerp(&clear, 0.9);
        assert_eq!(back.precip, Precip::Rain);
        assert!(back.precip_mm_h < 0.11 * rain.precip_mm_h);
        // Fog thickens evenly in log space: half way is the geometric mean.
        let fog = preset("fog").unwrap();
        let v = clear.lerp(&fog, 0.5).fog_visibility_m;
        let geo = (clear.fog_visibility_m * fog.fog_visibility_m).sqrt();
        assert!((v / geo - 1.0).abs() < 1e-3, "{v} {geo}");
        // Monotonic in between.
        let mut prev = clear.fog_visibility_m;
        for i in 1..=20 {
            let v = clear.lerp(&fog, i as f32 / 20.0).fog_visibility_m;
            assert!(v < prev);
            prev = v;
        }
    }

    #[test]
    fn the_wind_turns_the_short_way() {
        assert!((mix_angle(350.0, 10.0, 0.5) - 0.0).abs() < 1e-3);
        assert!((mix_angle(10.0, 350.0, 0.25) - 5.0).abs() < 1e-3);
        assert!((mix_angle(90.0, 260.0, 0.5) - 175.0).abs() < 1e-3);
        // A westerly blows towards the east (+x).
        let w = WeatherState {
            wind_from_deg: 270.0,
            ..Default::default()
        };
        let v = w.wind_velocity(10.0);
        assert!((v[0] - 10.0).abs() < 1e-4 && v[1].abs() < 1e-4, "{v:?}");
        // A northerly blows south (+z).
        let n = WeatherState {
            wind_from_deg: 0.0,
            ..w
        };
        assert!((n.wind_velocity(5.0)[1] - 5.0).abs() < 1e-4);
        assert_eq!(compass(225.0), "SW");
        assert_eq!(compass(359.0), "N");
    }

    #[test]
    fn extreme_weather_stays_impossible_weather_goes() {
        let storm = WeatherState {
            wind_mps: 45.0,
            gust_mps: 70.0,
            precip: Precip::Rain,
            precip_mm_h: 150.0,
            ..Default::default()
        };
        assert_eq!(storm.clamped(), storm, "no cap on extremes (Q9)");
        let bad = WeatherState {
            cloud_cover: 3.0,
            gust_mps: 1.0,
            wind_mps: f32::NAN,
            humidity: -1.0,
            fog_visibility_m: 0.0,
            wind_from_deg: -90.0,
            ..Default::default()
        }
        .clamped();
        assert_eq!(bad.cloud_cover, 1.0);
        assert_eq!(bad.wind_mps, WeatherState::default().wind_mps);
        assert!(bad.gust_mps >= bad.wind_mps);
        assert_eq!(bad.humidity, 0.0);
        assert_eq!(bad.fog_visibility_m, MIN_VISIBILITY_M);
        assert_eq!(bad.wind_from_deg, 270.0);
    }

    #[test]
    fn conditions_and_summaries() {
        for (name, want) in [
            ("clear", Condition::Clear),
            ("overcast", Condition::Overcast),
            ("fog", Condition::Fog),
            ("drizzle", Condition::Drizzle),
            ("light-rain", Condition::Rain),
            ("heavy-rain", Condition::HeavyRain),
            ("thunderstorm", Condition::Thunderstorm),
            ("heavy-snow", Condition::HeavySnow),
            ("hail", Condition::Hail),
        ] {
            assert_eq!(preset(name).unwrap().condition(), want, "{name}");
        }
        let rain = preset("rain").unwrap();
        let s = rain.summary();
        assert!(s.starts_with("rain 4.0 mm/h"), "{s}");
        assert!(s.contains("°C") && s.contains("from"), "{s}");
        assert!(preset("fog").unwrap().summary().contains("visibility"));
        assert_eq!(preset("clear").unwrap().hud(), "clear 18 °C");
    }

    #[test]
    fn frozen_shares() {
        assert_eq!(Precip::Snow.frozen_share(-5.0), 1.0);
        assert_eq!(Precip::Snow.frozen_share(5.0), 0.0);
        assert_eq!(Precip::Rain.frozen_share(-5.0), 0.0);
        assert!((Precip::Sleet.frozen_share(0.0) - 0.5).abs() < 1e-6);
        assert!(Precip::Hail.frozen_share(20.0) > 0.5);
    }
}
