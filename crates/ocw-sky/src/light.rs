//! Day and night light from the sun's elevation (spec §14 Phase 9, 9B):
//! pure functions the renderer applies to the sun light, the ambient
//! light, the camera exposure, the fog and the bike lights.
//!
//! The model is physically motivated, in the renderer's scene units (a
//! high sun is [`DAY_SUN_LUX`], the daylight ambient [`DAY_AMBIENT`], at
//! exposure [`DAY_EV100`] – the values the scene was tuned with):
//! - **Sun:** clear-sky transmittance `exp(-τ·m)` per colour channel
//!   (optical depth τ of Rayleigh scattering plus aerosols, air mass m
//!   after Kasten & Young 1989): white-yellow at noon, orange and dim at
//!   5°, red at the horizon, gone once the upper limb has set.
//! - **Sky (ambient):** the diffuse skylight relative to a high sun, about
//!   5 % at sunset, falling ten-fold every 2.4° below the horizon (civil
//!   dusk ~2·10⁻⁴, nautical ~10⁻⁶ of daylight, as measured).
//! - **Exposure (auto-exposure):** the eye adapts to 75 % of the darkening
//!   in stops, by at most 8 stops: night stays clearly darker than day.
//!   The adaptation is a pure function of the light (deterministic
//!   screenshots), eased over time by [`adapt_ev`].
//! - **Night brightness:** a floor for the displayed ambient light, a
//!   fraction of the daylight ambient (setting `display.night_brightness`,
//!   Phase 9 Q3: dark but rideable).

/// The scene's sun illuminance for a high sun (renderer units).
pub const DAY_SUN_LUX: f32 = 12_000.0;
/// The scene's ambient light by day.
pub const DAY_AMBIENT: f32 = 600.0;
/// The camera exposure by day (Bevy's default EV100).
pub const DAY_EV100: f32 = 9.7;
/// Most stops the eye adapts to darkness.
pub const MAX_ADAPTATION_STOPS: f32 = 8.0;
/// Share of the darkening (in stops) the adaptation makes up.
pub const ADAPTATION: f32 = 0.75;
/// Default night brightness (share of the daylight ambient).
pub const DEFAULT_NIGHT_FLOOR: f32 = 0.10;

/// Optical depth of the clear atmosphere for red, green and blue light
/// (Rayleigh ~0.04 / 0.10 / 0.24 plus aerosols ~0.10).
const TAU: [f64; 3] = [0.14, 0.20, 0.34];

/// Relative air mass towards elevation `el_deg` (Kasten & Young 1989; the
/// horizon value below it).
pub fn air_mass(el_deg: f64) -> f64 {
    let h = el_deg.max(0.0);
    1.0 / (h.to_radians().sin() + 0.50572 * (h + 6.07995).powf(-1.6364))
}

/// Direct-sun transmittance per channel (linear RGB).
pub fn sun_transmittance(el_deg: f64) -> [f64; 3] {
    let m = air_mass(el_deg);
    TAU.map(|t| (-t * m).exp())
}

fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Share of the sun's disk above the horizon (apparent centre elevation;
/// radius 0.267°).
pub fn sun_visible(el_deg: f64) -> f64 {
    ((el_deg + 0.267) / 0.533).clamp(0.0, 1.0)
}

/// Diffuse skylight relative to a sun 60° high.
pub fn sky_diffuse(el_deg: f64) -> f64 {
    if el_deg >= 0.0 {
        let s = el_deg.to_radians().sin() / 60f64.to_radians().sin();
        (0.05 + 0.95 * s.powf(0.7)).min(1.1)
    } else {
        (0.05 * 10f64.powf(el_deg / 2.4)).max(2e-8)
    }
}

/// What the renderer sets for a sun elevation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Sun illuminance (scene units) and colour (linear RGB, max 1), the
    /// atmosphere's reddening included.
    pub sun_lux: f32,
    pub sun_rgb: [f32; 3],
    /// The sun above the atmosphere (scene units): for a renderer whose
    /// atmosphere applies the transmittance itself (Bevy's atmosphere).
    pub sun_lux_space: f32,
    /// Ambient brightness (scene units) and colour (linear RGB).
    pub ambient: f32,
    pub ambient_rgb: [f32; 3],
    /// Camera exposure (EV100): lower is brighter.
    pub ev100: f32,
    /// Fog and plain-sky colour as displayed (sRGB).
    pub fog_srgb: [f32; 3],
    /// 0 by day … 1 at night (twilight in between).
    pub darkness: f32,
    /// Bike lights on (automatic mode).
    pub lights_on: bool,
    /// The sun casts shadows.
    pub shadows: bool,
}

/// Sky colour at the horizon (sRGB, display) by sun elevation.
const HORIZON: [(f64, [f64; 3]); 8] = [
    (-18.0, [0.010, 0.012, 0.025]),
    (-10.0, [0.05, 0.07, 0.13]),
    (-6.0, [0.16, 0.20, 0.34]),
    (-3.0, [0.42, 0.36, 0.48]),
    (0.0, [0.86, 0.58, 0.42]),
    (3.0, [0.82, 0.74, 0.68]),
    (10.0, [0.62, 0.78, 0.95]),
    (90.0, [0.62, 0.78, 0.95]),
];

fn keyed(keys: &[(f64, [f64; 3])], x: f64) -> [f64; 3] {
    if x <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        let ((x0, a), (x1, b)) = (w[0], w[1]);
        if x <= x1 {
            let t = (x - x0) / (x1 - x0);
            return [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
        }
    }
    keys[keys.len() - 1].1
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Light for an apparent sun elevation (°); `night_floor` is the least
/// displayed ambient as a share of the daylight ambient (0–1).
pub fn light(el_deg: f64, night_floor: f32) -> Light {
    let zenith = sun_transmittance(90.0);
    let t = sun_transmittance(el_deg);
    let visible = sun_visible(el_deg);
    let sun_rel = luminance(t) / luminance(zenith) * visible;
    let max = t[0].max(t[1]).max(t[2]);
    let sun_rgb = t.map(|c| (c / max) as f32);
    let sun_lux = DAY_SUN_LUX * sun_rel as f32;
    let sun_lux_space = DAY_SUN_LUX / luminance(zenith) as f32;
    let diffuse = sky_diffuse(el_deg);
    let s60 = 60f64.to_radians().sin();
    let direct = sun_rel * el_deg.max(0.0).to_radians().sin() / s60;
    let level = (0.45 * diffuse + 0.55 * direct).max(1e-9);
    let stops = (-(ADAPTATION as f64) * level.log2()).clamp(0.0, MAX_ADAPTATION_STOPS as f64);
    let gain = 2f64.powf(-stops);
    let floor = f64::from(night_floor.clamp(0.0, 1.0));
    let ambient = (f64::from(DAY_AMBIENT) * diffuse).max(f64::from(DAY_AMBIENT) * floor * gain);
    // Skylight: white by day, blue in the twilight and at night.
    let blue = 1.0 - smoothstep(-6.0, 4.0, el_deg);
    let ambient_rgb = [1.0 - 0.45 * blue, 1.0 - 0.35 * blue, 1.0].map(|c| c as f32);
    // Fog: the horizon colour, not darker than the night floor allows.
    let day = HORIZON[HORIZON.len() - 1].1;
    let fog = keyed(&HORIZON, el_deg);
    let fog_srgb = [0, 1, 2].map(|i| fog[i].max(day[i] * floor * 0.5) as f32);
    let darkness = (1.0 - smoothstep(-8.0, 3.0, el_deg)) as f32;
    Light {
        sun_lux,
        sun_rgb,
        sun_lux_space,
        ambient: ambient as f32,
        ambient_rgb,
        ev100: DAY_EV100 - stops as f32,
        fog_srgb,
        darkness,
        lights_on: el_deg < 1.0,
        shadows: sun_lux > 30.0,
    }
}

/// Eases the camera exposure towards `target` (eye adaptation, ~1.5 s);
/// jumps of more than 3 stops (time machine, a new ride) are taken at once.
pub fn adapt_ev(current: f32, target: f32, dt_s: f32) -> f32 {
    if (target - current).abs() > 3.0 || !current.is_finite() {
        return target;
    }
    current + (target - current) * (1.0 - (-dt_s / 1.5).exp())
}

/// Glyph for the HUD clock: sun by day, half disc in the twilight, full
/// disc (the moon's place) at night; all in the HUD font.
pub fn glyph(el_deg: f64) -> char {
    if el_deg >= -0.833 {
        '☼'
    } else if el_deg >= -12.0 {
        '◐'
    } else {
        '●'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn the_sun_dims_and_reddens_towards_the_horizon() {
        let noon = light(65.0, DEFAULT_NIGHT_FLOOR);
        assert!((noon.sun_lux - DAY_SUN_LUX).abs() / DAY_SUN_LUX < 0.05, "{noon:?}");
        assert!(noon.sun_rgb[2] > 0.75, "near white at noon: {:?}", noon.sun_rgb);
        let golden = light(5.0, DEFAULT_NIGHT_FLOOR);
        assert!(golden.sun_lux < 0.5 * noon.sun_lux && golden.sun_lux > 0.05 * noon.sun_lux);
        assert!(golden.sun_rgb[2] < 0.4 && golden.sun_rgb[1] < 0.75, "orange: {:?}", golden.sun_rgb);
        let horizon = light(0.0, DEFAULT_NIGHT_FLOOR);
        assert!(horizon.sun_rgb[1] < 0.3, "red: {:?}", horizon.sun_rgb);
        assert!(horizon.sun_lux < 0.02 * noon.sun_lux);
        // Monotonic in elevation.
        let mut prev = 0.0;
        for e in -2..=90 {
            let l = light(f64::from(e), DEFAULT_NIGHT_FLOOR);
            assert!(l.sun_lux >= prev, "{e}");
            prev = l.sun_lux;
        }
        // Set: no direct light, no shadows.
        let set = light(-1.0, DEFAULT_NIGHT_FLOOR);
        assert_eq!(set.sun_lux, 0.0);
        assert!(!set.shadows && noon.shadows);
        assert!(noon.sun_lux_space > noon.sun_lux);
    }

    #[test]
    fn night_is_dark_but_not_black() {
        let day = light(50.0, DEFAULT_NIGHT_FLOOR);
        let night = light(-30.0, DEFAULT_NIGHT_FLOOR);
        // Displayed ambient = scene ambient × 2^(DAY_EV - ev).
        let shown = |l: &Light| l.ambient * 2f32.powf(DAY_EV100 - l.ev100);
        assert!((shown(&night) / shown(&day) - DEFAULT_NIGHT_FLOOR).abs() < 0.01, "the floor holds");
        assert!(night.ev100 >= DAY_EV100 - MAX_ADAPTATION_STOPS - 1e-4);
        assert!(night.ev100 < day.ev100 - 7.0, "the eye adapts to the dark");
        // Without a floor it is very dark.
        let pitch = light(-30.0, 0.0);
        assert!(shown(&pitch) / shown(&day) < 1e-3);
        // A level white surface darkens steadily into the night (ambient
        // plus the sun's E·sin(h)/π, as displayed).
        let mut prev = f32::INFINITY;
        for e in (-24..=60).rev() {
            let l = light(f64::from(e), 0.0);
            let sun = l.sun_lux * (e.max(0) as f32).to_radians().sin() / std::f32::consts::PI;
            let v = shown(&l) + sun * 2f32.powf(DAY_EV100 - l.ev100);
            assert!(v <= prev * 1.0001, "brighter at {e}");
            prev = v;
        }
        assert!(night.lights_on && !day.lights_on);
        assert!(light(-3.0, 0.1).lights_on, "lights on in the twilight");
        assert_eq!((day.darkness, night.darkness), (0.0, 1.0));
    }

    #[test]
    fn skylight_turns_blue_and_the_fog_follows_the_sky() {
        let day = light(40.0, DEFAULT_NIGHT_FLOOR);
        assert_eq!(day.ambient_rgb, [1.0, 1.0, 1.0]);
        let blue_hour = light(-5.0, DEFAULT_NIGHT_FLOOR);
        assert!(blue_hour.ambient_rgb[0] < blue_hour.ambient_rgb[2]);
        assert!(blue_hour.fog_srgb[2] > blue_hour.fog_srgb[0], "{:?}", blue_hour.fog_srgb);
        let sunset = light(0.0, DEFAULT_NIGHT_FLOOR);
        assert!(sunset.fog_srgb[0] > sunset.fog_srgb[2], "warm at sunset");
        for (a, b) in day.fog_srgb.iter().zip([0.62, 0.78, 0.95]) {
            assert!((a - b).abs() < 1e-6);
        }
        let night = light(-40.0, 0.2);
        assert!(lum(night.fog_srgb) < 0.12 && lum(night.fog_srgb) > 0.05);
    }

    #[test]
    fn diffuse_falls_tenfold_every_few_degrees_below_the_horizon() {
        assert!((sky_diffuse(0.0) - 0.05).abs() < 1e-9);
        let civil = sky_diffuse(-6.0);
        assert!((1e-4..3e-4).contains(&civil), "{civil}");
        assert!(sky_diffuse(-12.0) < 1e-6);
        assert!((air_mass(90.0) - 1.0).abs() < 0.01);
        assert!((air_mass(0.0) - 38.0).abs() < 1.0);
        assert_eq!(sun_visible(-0.267), 0.0);
        assert_eq!(sun_visible(0.3), 1.0);
    }

    #[test]
    fn exposure_eases_and_snaps() {
        let e = adapt_ev(9.7, 8.7, 1.5);
        assert!((e - (9.7 - 0.632)).abs() < 0.01, "{e}");
        assert_eq!(adapt_ev(9.7, 2.0, 0.016), 2.0, "a jump is taken at once");
        assert_eq!(adapt_ev(5.0, 5.0, 1.0), 5.0);
    }

    #[test]
    fn hud_glyphs_are_in_the_hud_font_range() {
        assert_eq!(glyph(30.0), '☼');
        assert_eq!(glyph(-5.0), '◐');
        assert_eq!(glyph(-20.0), '●');
    }
}
