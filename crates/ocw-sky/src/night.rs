//! Night light (spec §14 Phase 9, S2): moonlight, how bright the planets
//! and the moon look against the sky, light pollution's sky glow, in the
//! renderer's scene units of [`crate::light`].
//!
//! Brightness follows the eye adapted to the dark ([`crate::light`] lets
//! the exposure open by up to [`MAX_ADAPTATION_STOPS`]): a full moon high
//! in the sky lights the landscape with a few per cent of daylight as
//! displayed (real: 2·10⁻⁶ of the sun – the eye's rods make up most of the
//! difference), bluish (the Purkinje shift), casting soft shadows; a
//! quarter moon gives a tenth of that (the moon's phase law, Allen:
//! m = −12.73 + 0.026·|i| + 4·10⁻⁹·i⁴).

use crate::light::{DAY_AMBIENT, DAY_SUN_LUX, MAX_ADAPTATION_STOPS, sun_transmittance};

/// Moonlight of a high full moon as displayed, share of the noon sun.
pub const FULL_MOON_SHARE: f32 = 0.035;
/// The moon's skylight (ambient) at full moon as displayed, share of the
/// daylight ambient.
pub const FULL_MOON_AMBIENT_SHARE: f32 = 0.05;
/// Moonlight colour after the eye's adaptation: bluish.
pub const MOON_RGB: [f32; 3] = [0.70, 0.82, 1.0];

/// The moon's brightness relative to full, by phase angle `i` (°).
pub fn phase_brightness(i: f64) -> f64 {
    let i = i.abs().min(180.0);
    10f64.powf(-0.4 * (0.026 * i + 4e-9 * i.powi(4)))
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the moon lights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moonlight {
    /// Directional light (scene units), with the atmosphere's extinction.
    pub lux: f32,
    /// The same above the atmosphere (for a renderer whose atmosphere
    /// applies the transmittance itself).
    pub lux_space: f32,
    /// Colour (linear RGB, max 1), reddened near the horizon.
    pub rgb: [f32; 3],
    /// Extra ambient light from the moonlit sky (scene units).
    pub ambient: f32,
    /// Strong enough for (soft) shadows.
    pub shadows: bool,
}

/// Moonlight for the moon at apparent elevation `el` (°) and phase angle
/// `i` (°).
pub fn moonlight(el: f64, phase_angle_deg: f64) -> Moonlight {
    let gain = 2f64.powf(-f64::from(MAX_ADAPTATION_STOPS));
    let phase = phase_brightness(phase_angle_deg);
    let up = crate::light::sun_visible(el);
    let t = sun_transmittance(el);
    let z = sun_transmittance(90.0);
    let lum = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let ext = lum(t) / lum(z);
    let space = f64::from(DAY_SUN_LUX) * f64::from(FULL_MOON_SHARE) * gain * phase * up;
    let lux = space * ext;
    let max = t[0].max(t[1]).max(t[2]);
    let rgb = [0, 1, 2].map(|k| (f64::from(MOON_RGB[k]) * t[k] / max) as f32);
    let sky = smoothstep(-2.0, 30.0, el);
    let ambient = f64::from(DAY_AMBIENT) * f64::from(FULL_MOON_AMBIENT_SHARE) * gain * phase * sky;
    let full = f64::from(DAY_SUN_LUX * FULL_MOON_SHARE) * gain;
    Moonlight {
        lux: lux as f32,
        lux_space: space as f32,
        rgb,
        ambient: ambient as f32,
        shadows: lux > 0.15 * full,
    }
}

/// The faintest star magnitude the eye sees: by the sun's elevation (day
/// −4.5 – Venus only just –, twilight, night 6.0), less in moonlight (up
/// to 1.5 magnitudes at a high full moon) and under light pollution (up
/// to 3 magnitudes in a town centre).
pub fn limiting_magnitude(sun_el: f64, moon_el: f64, moon_illuminated: f64, pollution: f64) -> f64 {
    // Twilight: from -4.5 at sunrise to 6.0 at astronomical night (about
    // magnitude 0 at -4°, 1 at -6°, 3.7 at -12°).
    let t = (-sun_el / 18.0).clamp(0.0, 1.0).powf(0.6);
    let base = -4.5 + 10.5 * t;
    let moon = 1.5 * moon_illuminated.clamp(0.0, 1.0) * smoothstep(-1.0, 30.0, moon_el);
    let town = 3.0 * pollution.clamp(0.0, 1.0);
    // Pollution and moon only matter once the sky is dark.
    base - (moon + town) * t
}

/// How visible a point of magnitude `mag` is (0 hidden … 1 clearly): it
/// fades in over the last 1.5 magnitudes above the limit.
pub fn point_visibility(mag: f64, limit: f64) -> f64 {
    smoothstep(0.0, 1.5, limit - mag)
}

/// Brightness of a point of light on screen for magnitude `mag` (Venus at
/// −4.4 → about 1; each magnitude 2.512× fainter, compressed by a square
/// root as the eye sees points).
pub fn point_brightness(mag: f64) -> f64 {
    10f64.powf(-0.2 * (mag + 4.4))
}

/// Light pollution's sky glow above the horizon (displayed share of the
/// night sky, 0–1) at elevation `el` (°): strongest at the horizon, ~e⁻¹
/// at 12°, for pollution 0–1, only at night (`darkness` of the light
/// model).
pub fn sky_glow(pollution: f64, el: f64, darkness: f64) -> f64 {
    let shape = (-(el.max(0.0)) / 12.0).exp() * smoothstep(-6.0, 0.0, el);
    (pollution.clamp(0.0, 1.0) * shape * darkness.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// Sky glow colour (linear RGB): sodium and LED light scattered in the
/// haze, orange-pink.
pub const GLOW_RGB: [f32; 3] = [1.0, 0.55, 0.28];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::{DAY_EV100, light};

    #[test]
    fn the_moons_phase_law() {
        assert!((phase_brightness(0.0) - 1.0).abs() < 1e-12);
        let q = phase_brightness(90.0);
        assert!((0.07..0.11).contains(&q), "quarter {q}");
        assert!(phase_brightness(150.0) < 0.01);
        assert!(phase_brightness(-30.0) == phase_brightness(30.0));
    }

    #[test]
    fn full_moonlight_is_a_few_percent_of_daylight_and_blue() {
        let full = moonlight(50.0, 0.0);
        let night = light(-30.0, crate::light::DEFAULT_NIGHT_FLOOR);
        // As displayed: lux × 2^(DAY_EV − ev).
        let shown = full.lux * 2f32.powf(DAY_EV100 - night.ev100);
        let share = shown / DAY_SUN_LUX;
        assert!((0.02..0.04).contains(&share), "{share}");
        assert!(full.rgb[2] > full.rgb[0], "bluish: {:?}", full.rgb);
        assert!(full.shadows);
        // A quarter moon: a tenth, no shadows to speak of; set: nothing.
        let quarter = moonlight(50.0, 90.0);
        assert!(quarter.lux < 0.12 * full.lux && quarter.lux > 0.05 * full.lux);
        assert!(!moonlight(50.0, 120.0).shadows);
        let set = moonlight(-2.0, 0.0);
        assert_eq!((set.lux, set.ambient), (0.0, 0.0));
        // Low: dimmer and redder.
        let low = moonlight(3.0, 0.0);
        assert!(low.lux < 0.3 * full.lux);
        assert!(low.rgb[0] > low.rgb[2], "{:?}", low.rgb);
        assert!(full.lux_space >= full.lux);
        // The moon's skylight adds to the night floor.
        assert!(full.ambient > 0.0 && full.ambient < night.ambient);
    }

    #[test]
    fn the_eye_sees_fainter_points_in_a_dark_sky() {
        let day = limiting_magnitude(30.0, -10.0, 0.0, 0.0);
        let dusk = limiting_magnitude(-4.0, -10.0, 0.0, 0.0);
        let night = limiting_magnitude(-25.0, -10.0, 0.0, 0.0);
        assert!(day < -4.0 && night > 5.9, "{day} {night}");
        assert!(dusk > -1.0 && dusk < 3.0, "{dusk}");
        // Venus (−4.4) already shows in the early twilight; Saturn (0.7)
        // only later; a town and a full moon dim the night.
        assert!(point_visibility(-4.4, limiting_magnitude(-2.0, -10.0, 0.0, 0.0)) > 0.8);
        assert!(point_visibility(0.7, limiting_magnitude(-2.0, -10.0, 0.0, 0.0)) < 0.01);
        assert!(point_visibility(0.7, night) == 1.0);
        let town = limiting_magnitude(-25.0, -10.0, 0.0, 1.0);
        let moon = limiting_magnitude(-25.0, 50.0, 1.0, 0.0);
        assert!(
            (town - 3.0).abs() < 0.01 && (moon - 4.5).abs() < 0.05,
            "{town} {moon}"
        );
        assert!((point_brightness(-4.4) - 1.0).abs() < 1e-9);
        assert!(point_brightness(0.6) < 0.11);
    }

    #[test]
    fn sky_glow_hugs_the_horizon_at_night() {
        assert_eq!(sky_glow(0.0, 0.0, 1.0), 0.0);
        let h = sky_glow(0.8, 0.5, 1.0);
        let z = sky_glow(0.8, 60.0, 1.0);
        assert!(h > 0.7 && z < 0.01, "{h} {z}");
        assert!(sky_glow(0.8, 12.0, 1.0) < 0.4 * h);
        assert_eq!(sky_glow(1.0, 5.0, 0.0), 0.0, "not by day");
    }
}
