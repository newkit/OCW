//! Sky and time for Open Cycling World (spec §14 Phase 9, milestones S1,
//! S2 and S3): the time zone and local time of the rider's place in the
//! virtual world ([`zone`]), the session's world clock and its client-side
//! prediction ([`clock`]), the time machine's requests and presets
//! ([`machine`]), the sun's position and day ([`sun`]) and the light it
//! gives ([`light`]); the night sky without stars (decision Q7): the moon
//! ([`moon`], its disc [`moon_disc`]), the planets ([`planets`]), the
//! Milky Way ([`milky_way`]) and night light ([`night`]), on the shared
//! astronomy of [`astro`].
//!
//! Pure Rust without Bevy, threads, files or clock reads (instants are
//! passed in), so it builds for `wasm32-unknown-unknown` (spec §3.5).

pub mod astro;
pub mod clock;
pub mod light;
pub mod machine;
pub mod milky_way;
pub mod moon;
pub mod moon_disc;
pub mod night;
pub mod planets;
pub mod sun;
pub mod time;
pub mod zone;

pub use clock::{ClockMode, ClockPredictor, WorldClock};
pub use light::Light;
pub use machine::{ClockChange, Preset, TimeCommand};
pub use moon::MoonPosition;
pub use night::Moonlight;
pub use planets::{Planet, PlanetPosition};
pub use sun::SunPosition;
pub use time::UnixMs;
pub use zone::{LocalTime, Zone};

/// The sky at the rider: local time, sun and light, moon, planets.
#[derive(Debug, Clone, PartialEq)]
pub struct Sky {
    pub instant: UnixMs,
    pub local: LocalTime,
    pub sun: SunPosition,
    pub light: Light,
    pub moon: MoonPosition,
    pub moonlight: Moonlight,
    pub planets: [PlanetPosition; 5],
    /// Where (latitude, longitude): the sidereal frame of the Milky Way.
    pub place: (f64, f64),
}

impl Sky {
    /// The sky at instant `ms` at (lat, lon) in `zone`, with the night
    /// brightness floor `night_floor` (see [`light::light`]).
    pub fn at(ms: UnixMs, zone: &Zone, (lat, lon): (f64, f64), night_floor: f32) -> Self {
        let sun = sun::position(ms, lat, lon);
        let moon = moon::position(ms, lat, lon);
        Self {
            instant: ms,
            local: zone.local(ms),
            light: light::light(sun.elevation_deg, night_floor),
            sun,
            moonlight: night::moonlight(moon.elevation_deg, moon.phase_angle_deg),
            moon,
            planets: planets::all(ms, lat, lon),
            place: (lat, lon),
        }
    }

    /// Local sidereal time (°): turns the Milky Way's frame.
    pub fn lst_deg(&self) -> f64 {
        astro::lst_deg(self.instant, self.place.1)
    }

    /// "moon 87 % waxing gibbous, azimuth 120°, 35° high".
    pub fn moon_line(&self) -> String {
        format!(
            "moon {:.0} % {}, azimuth {:.0}°, {:.0}° high",
            self.moon.illuminated * 100.0,
            self.moon.phase_name(),
            self.moon.azimuth_deg,
            self.moon.elevation_deg
        )
    }

    /// "☼ 18:42" for the HUD.
    pub fn hud_clock(&self) -> String {
        format!(
            "{} {}",
            light::glyph(self.sun.elevation_deg),
            self.local.hh_mm()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_over_vaduz_at_a_summer_sunset() {
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let (at, _) = zone.instant((2026, 6, 21), (21, 21, 0)).unwrap();
        let sky = Sky::at(at, &zone, zone::DEFAULT_PLACE, light::DEFAULT_NIGHT_FLOOR);
        assert_eq!(sky.hud_clock(), "☼ 21:21");
        assert!(sky.sun.elevation_deg.abs() < 0.6, "{:?}", sky.sun);
        assert!(sky.light.sun_rgb[1] < 0.4, "a red sun");
        let night = Sky::at(at + 3 * time::MS_PER_HOUR, &zone, zone::DEFAULT_PLACE, 0.1);
        assert_eq!(night.hud_clock(), "● 00:21");
        assert!(night.light.lights_on);
    }
}
