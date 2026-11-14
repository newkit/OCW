//! How clouds, fog and precipitation change the light (spec §14 Phase 9,
//! S4: "light and exposure follow the cloud cover; overcast = flat light,
//! no hard shadows"). Factors on the clear-sky light of `ocw_sky::light`:
//! the renderer multiplies the sun and the sky light with them.
//!
//! On a clear day about four fifths of the light on the ground come
//! directly from the sun and one fifth from the sky. A closed stratus deck
//! lets through about a quarter of the clear-sky light, all of it diffuse
//! (brighter sky light than on a clear day, no sun); a thunderstorm's
//! anvil a tenth or less; broken fair-weather cumulus brighten the sky
//! light while the sun still shines between them.

use crate::state::{CloudType, WeatherState};

/// The weather's change of the light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudLight {
    /// Share of the direct sunlight that gets through (0–1).
    pub direct: f32,
    /// Sky light relative to a clear sky's (above 1: white clouds).
    pub diffuse: f32,
    /// Whether the sun casts (hard) shadows.
    pub shadows: bool,
    /// The change of the light on the ground (stops, ≤ ~0): the exposure
    /// follows part of it.
    pub stops: f32,
    /// How grey the sky and the fog are (0 blue – 1 grey).
    pub grey: f32,
    /// How dark the clouds' undersides are (0 white – 1 storm).
    pub darkness: f32,
}

/// Direct share of the clear-sky light on the ground.
pub const CLEAR_DIRECT_SHARE: f32 = 0.8;

/// Sky light under a full cover of `t`, relative to the clear sky's.
fn overcast_diffuse(t: CloudType) -> f32 {
    match t {
        CloudType::Cumulus => 0.9,
        CloudType::Stratus => 1.25,
        CloudType::Cirrus => 1.15,
        CloudType::Cumulonimbus => 0.35,
    }
}

/// The light under the weather `w`.
pub fn cloud_light(w: &WeatherState) -> CloudLight {
    let c = w.cloud_cover.clamp(0.0, 1.0);
    let op = w.cloud_type.opacity();
    // Heaps leave gaps: the sun is blocked less than the cover says until
    // the heaps close up.
    let blocked = match w.cloud_type {
        CloudType::Cumulus => c.powf(1.6),
        _ => c,
    } * op;
    // Fog and what falls dim the sun on its way through the lowest air.
    let haze = (-3.912 * 120.0 / w.visibility_m().max(1.0)).exp();
    let direct = ((1.0 - blocked) * haze).clamp(0.0, 1.0);
    let broken = if w.cloud_type == CloudType::Cumulus {
        1.2 * c * (1.0 - c)
    } else {
        0.0
    };
    let mm = if w.precipitating() {
        w.precip_mm_h
    } else {
        0.0
    };
    let diffuse = (1.0 + (overcast_diffuse(w.cloud_type) - 1.0) * c.powf(1.5) + broken)
        * (0.65 + 0.35 * haze)
        / (1.0 + 0.015 * mm);
    let total = CLEAR_DIRECT_SHARE * direct + (1.0 - CLEAR_DIRECT_SHARE) * diffuse;
    let darkness = match w.cloud_type {
        CloudType::Cumulonimbus => 0.55 + 0.4 * c,
        CloudType::Stratus => 0.25 + 0.3 * c,
        CloudType::Cumulus => 0.15 * c,
        CloudType::Cirrus => 0.0,
    } + (0.004 * mm).min(0.2);
    CloudLight {
        direct,
        diffuse,
        shadows: direct > 0.3,
        stops: total.max(1e-3).log2().max(-6.0),
        grey: (c * op).max(1.0 - haze).clamp(0.0, 1.0),
        darkness: darkness.clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset;

    fn light(name: &str) -> CloudLight {
        cloud_light(&preset(name).unwrap())
    }

    #[test]
    fn overcast_is_flat_light_without_shadows() {
        let clear = light("clear");
        assert!(clear.direct > 0.95 && clear.shadows);
        assert!(clear.stops > -0.1 && clear.grey < 0.1);
        let overcast = light("overcast");
        assert!(!overcast.shadows, "{overcast:?}");
        assert!(overcast.direct < 0.1);
        assert!(overcast.diffuse > 1.0, "a bright grey sky: {overcast:?}");
        // About a quarter of the clear-sky light: two stops down.
        assert!((-2.6..-1.4).contains(&overcast.stops), "{}", overcast.stops);
        assert!(overcast.grey > 0.9);
        // Fair weather: sun and shadows, brighter sky light.
        let fair = light("fair");
        assert!(fair.shadows && fair.direct > 0.75 && fair.diffuse > 1.0);
    }

    #[test]
    fn storms_are_dark_and_fog_dims_the_sun() {
        let storm = light("thunderstorm");
        let overcast = light("overcast");
        assert!(storm.stops < overcast.stops - 1.0, "{storm:?}");
        assert!(storm.darkness > 0.8 && !storm.shadows);
        let fog = light("fog");
        assert!(fog.direct < 0.01 && !fog.shadows && fog.grey > 0.99);
        // Thin cirrus: the sun shines through, shadows stay.
        let cirrus = cloud_light(&WeatherState {
            cloud_cover: 0.8,
            cloud_type: CloudType::Cirrus,
            ..Default::default()
        });
        assert!(cirrus.shadows && cirrus.direct > 0.7);
        // Rain darkens a little more.
        assert!(light("heavy-rain").stops < light("overcast").stops);
    }
}
