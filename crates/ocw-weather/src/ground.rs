//! The ground's memory of the weather (spec §14 Phase 9, 9E): road wetness
//! builds up in rain and dries afterwards depending on sun (cloud cover),
//! wind, temperature and humidity; heavy rain fills puddles in the hollows;
//! snow settles when it is cold and melts when it is warm.
//!
//! A small water balance in millimetres, integrated by the server
//! ([`crate::session::SessionWeather`]) and sent to the clients as factors
//! (0–1) for the renderer: [`Ground::wetness`], [`Ground::puddles`],
//! [`Ground::snow_cover`]. Roads are kept clear of snow (spec 9C): the
//! snow cover is for the terrain, roofs and trees.

use crate::state::WeatherState;

/// Water film (mm) at which a road looks fully wet.
pub const WET_FILM_MM: f32 = 0.12;
/// The film a road holds (mm): more runs off, partly into the puddles.
pub const FILM_CAPACITY_MM: f32 = 0.6;
/// Share of the run-off that stays in the hollows.
pub const PUDDLE_SHARE: f32 = 0.35;
/// Puddle water (mm, averaged over the road) at which the puddles are full.
pub const PUDDLE_FULL_MM: f32 = 2.0;
/// The most the hollows hold (mm).
pub const PUDDLE_MAX_MM: f32 = 3.0;
/// Snow (mm water equivalent, ≈ 1 cm of fresh snow per mm) that covers
/// the ground completely.
pub const SNOW_COVER_MM: f32 = 3.0;
/// Puddles drain into the ground and the drains (mm/h).
pub const PUDDLE_DRAIN_MM_H: f32 = 0.5;

/// What the ground holds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ground {
    /// Water film on the road (mm).
    pub water_mm: f32,
    /// Water in the hollows (mm, averaged).
    pub puddle_mm: f32,
    /// Lying snow and hail (mm water equivalent).
    pub snow_mm: f32,
}

/// Evaporation from a wet road (mm/h): sun (little cloud), wind, warmth
/// and dry air speed it up. A sunny, breezy summer afternoon dries a wet
/// road in well under an hour, a humid overcast day hardly at all.
pub fn evaporation_mm_h(w: &WeatherState) -> f32 {
    let sun = 1.0 + 1.6 * (1.0 - w.cloud_cover * w.cloud_type.opacity());
    let wind = 1.0 + 0.25 * w.wind_mps.min(20.0);
    let dry = (1.0 - w.humidity).max(0.03);
    let warm = (0.35 + 0.06 * w.temperature_c).clamp(0.05, 3.0);
    0.18 * sun * wind * dry * warm
}

/// Snow melt (mm/h water equivalent): degree-hours above freezing, more
/// in sun and rain.
pub fn melt_mm_h(w: &WeatherState, rain_mm_h: f32) -> f32 {
    let above = (w.temperature_c - 0.5).max(0.0);
    let sun = 1.0 + 1.5 * (1.0 - w.cloud_cover);
    0.12 * above * sun + 0.0125 * rain_mm_h * w.temperature_c.max(0.0)
}

impl Ground {
    /// How wet the road looks (0 dry – 1 soaked).
    pub fn wetness(&self) -> f32 {
        (self.water_mm / WET_FILM_MM).clamp(0.0, 1.0)
    }

    /// How full the puddles are (0 none – 1 full).
    pub fn puddles(&self) -> f32 {
        (self.puddle_mm / PUDDLE_FULL_MM).clamp(0.0, 1.0)
    }

    /// How much of the ground the snow covers (0–1).
    pub fn snow_cover(&self) -> f32 {
        (self.snow_mm / SNOW_COVER_MM).clamp(0.0, 1.0)
    }

    /// The ground with these factors (the console's overrides).
    pub fn with_factors(wetness: f32, puddles: f32, snow_cover: f32) -> Self {
        Self {
            water_mm: wetness.clamp(0.0, 1.0) * WET_FILM_MM,
            puddle_mm: puddles.clamp(0.0, 1.0) * PUDDLE_FULL_MM,
            snow_mm: snow_cover.clamp(0.0, 1.0) * SNOW_COVER_MM,
        }
    }

    /// `dt_s` seconds of weather `w`.
    pub fn step(&mut self, w: &WeatherState, dt_s: f32) {
        if dt_s <= 0.0 {
            return;
        }
        let h = dt_s / 3600.0;
        let mm = if w.precipitating() {
            w.precip_mm_h
        } else {
            0.0
        };
        let frozen = mm * w.precip.frozen_share(w.temperature_c);
        let liquid = mm - frozen;
        // Snow: settles below about +1 °C (it melts on warm ground at once).
        let settles = 1.0 - crate::state::smooth(0.5, 2.5, w.temperature_c);
        let melt = (melt_mm_h(w, liquid) * h).min(self.snow_mm);
        self.snow_mm = (self.snow_mm + frozen * settles * h - melt).max(0.0);
        let melted_on_arrival = frozen * (1.0 - settles);
        // Water: rain and melt water in, evaporation out; above the film's
        // capacity it runs off, part of it into the hollows.
        let evap = evaporation_mm_h(w);
        self.water_mm += (liquid + melted_on_arrival) * h + melt * 0.3 - evap * h;
        if self.water_mm > FILM_CAPACITY_MM {
            let excess = self.water_mm - FILM_CAPACITY_MM;
            self.water_mm = FILM_CAPACITY_MM;
            self.puddle_mm += excess * PUDDLE_SHARE;
        }
        self.water_mm = self.water_mm.max(0.0);
        self.puddle_mm =
            (self.puddle_mm - (PUDDLE_DRAIN_MM_H + 0.5 * evap) * h).clamp(0.0, PUDDLE_MAX_MM);
    }

    /// The ground after `w` has gone on for two hours from a dry, bare
    /// ground: what a weather set at once (not as a transition) starts
    /// with, so a "rain" scene has wet roads and a "heavy snow" scene a
    /// white landscape.
    pub fn settled(w: &WeatherState) -> Self {
        let mut g = Ground::default();
        for _ in 0..120 {
            g.step(w, 60.0);
        }
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset;

    fn run(g: &mut Ground, w: &WeatherState, minutes: u32) {
        for _ in 0..minutes * 6 {
            g.step(w, 10.0);
        }
    }

    /// The wet-road factor from the precipitation history (spec 9E): a
    /// shower soaks the road within minutes, sun and wind dry it within an
    /// hour, a humid grey day keeps it wet.
    #[test]
    fn roads_get_wet_in_rain_and_dry_afterwards() {
        let mut g = Ground::default();
        assert_eq!((g.wetness(), g.puddles(), g.snow_cover()), (0.0, 0.0, 0.0));
        run(&mut g, &preset("light-rain").unwrap(), 10);
        assert_eq!(g.wetness(), 1.0, "10 minutes of light rain: wet");
        assert_eq!(g.puddles(), 0.0, "but no puddles");
        // Sun and breeze afterwards: dry within an hour.
        let mut sunny = g;
        let fair = WeatherState {
            wind_mps: 4.0,
            ..preset("fair").unwrap()
        };
        run(&mut sunny, &fair, 15);
        assert!(sunny.wetness() > 0.0 && sunny.wetness() < 1.0, "drying");
        run(&mut sunny, &fair, 45);
        assert_eq!(sunny.wetness(), 0.0, "dry after an hour");
        // Overcast and humid: still wet after an hour.
        let mut grey = g;
        run(&mut grey, &preset("overcast").unwrap(), 60);
        assert!(grey.wetness() > 0.5, "{}", grey.wetness());
    }

    #[test]
    fn heavy_rain_fills_the_puddles_which_drain_slowly() {
        let mut g = Ground::default();
        run(&mut g, &preset("heavy-rain").unwrap(), 30);
        assert!(g.puddles() > 0.9, "{}", g.puddles());
        let mut after = g;
        run(&mut after, &preset("fair").unwrap(), 30);
        assert!(after.puddles() > 0.2 && after.puddles() < g.puddles());
        run(&mut after, &preset("fair").unwrap(), 300);
        assert_eq!(after.puddles(), 0.0);
        // The hollows hold only so much.
        run(&mut g, &preset("heavy-rain").unwrap(), 600);
        assert!(g.puddle_mm <= PUDDLE_MAX_MM);
    }

    /// The snow cover rule: snow settles below freezing, rain or warm air
    /// melt it; snow at +5 °C does not settle.
    #[test]
    fn snow_settles_when_cold_and_melts_when_warm() {
        let mut g = Ground::default();
        run(&mut g, &preset("heavy-snow").unwrap(), 40);
        assert_eq!(g.snow_cover(), 1.0, "40 minutes of heavy snow at -5 °C");
        let mut light = Ground::default();
        run(&mut light, &preset("snow").unwrap(), 30);
        assert!(light.snow_cover() > 0.1 && light.snow_cover() < 1.0);
        // Warm snow: wet roads, no cover.
        let mut warm = Ground::default();
        let wet_snow = WeatherState {
            temperature_c: 5.0,
            ..preset("snow").unwrap()
        };
        run(&mut warm, &wet_snow, 60);
        assert_eq!(warm.snow_cover(), 0.0);
        assert!(warm.wetness() > 0.5);
        // Rain at +8 °C melts the cover within a few hours; cold
        // overcast keeps it.
        let mut thaw = g;
        run(&mut thaw, &preset("rain").unwrap(), 240);
        assert!(thaw.snow_cover() < 0.2, "{}", thaw.snow_cover());
        let mut cold = g;
        let frost = WeatherState {
            temperature_c: -3.0,
            ..preset("overcast").unwrap()
        };
        run(&mut cold, &frost, 240);
        assert_eq!(cold.snow_cover(), 1.0);
    }

    #[test]
    fn a_weather_set_at_once_starts_settled() {
        assert_eq!(
            Ground::settled(&preset("clear").unwrap()),
            Ground::default()
        );
        let rain = Ground::settled(&preset("rain").unwrap());
        assert_eq!(rain.wetness(), 1.0);
        assert!(Ground::settled(&preset("heavy-rain").unwrap()).puddles() > 0.9);
        assert_eq!(
            Ground::settled(&preset("heavy-snow").unwrap()).snow_cover(),
            1.0
        );
        assert_eq!(
            Ground::settled(&preset("light-rain").unwrap()).puddles(),
            0.0
        );
        let g = Ground::with_factors(0.5, 0.25, 1.0);
        assert!((g.wetness() - 0.5).abs() < 1e-6 && (g.puddles() - 0.25).abs() < 1e-6);
        assert_eq!(g.snow_cover(), 1.0);
    }

    #[test]
    fn evaporation_follows_sun_wind_warmth_and_dryness() {
        let fair = preset("fair").unwrap();
        let e = evaporation_mm_h(&fair);
        assert!(e > evaporation_mm_h(&preset("overcast").unwrap()));
        let windy = WeatherState {
            wind_mps: 10.0,
            ..fair
        };
        assert!(evaporation_mm_h(&windy) > e);
        let humid = WeatherState {
            humidity: 0.95,
            ..fair
        };
        assert!(evaporation_mm_h(&humid) < e / 5.0);
    }
}
