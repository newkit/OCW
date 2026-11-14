//! What falls, as the renderer draws it (spec §14 Phase 9, S4): how far one
//! sees through fog and precipitation, how fast drops, flakes and hail
//! fall, and how many particles of which size fill the box around the
//! camera for an intensity, within a particle budget.
//!
//! Visibility follows Koschmieder (V = 3.912 / σ) with extinction
//! coefficients of the usual empirical form σ = a · Rᵇ (R in mm/h water
//! equivalent, σ in 1/km): rain scatters little per millimetre (big drops),
//! drizzle more, snow most – heavy snow is a white-out long before heavy
//! rain is.

use crate::state::{CLEAR_VISIBILITY_M, MIN_VISIBILITY_M, Precip};

/// Koschmieder's constant (2 % contrast threshold).
pub const KOSCHMIEDER: f32 = 3.912;

/// Extinction coefficient (1/km) of precipitation `p` at `mm_h`.
pub fn extinction_per_km(p: Precip, mm_h: f32) -> f32 {
    if mm_h <= 0.0 {
        return 0.0;
    }
    let (a, b) = match p {
        Precip::None => return 0.0,
        Precip::Rain => (0.25, 0.63),
        Precip::Drizzle => (1.0, 0.6),
        Precip::Snow => (3.0, 0.8),
        Precip::Sleet => (1.4, 0.7),
        Precip::Hail => (0.4, 0.63),
    };
    a * mm_h.powf(b)
}

/// Visibility (m) through fog of visibility `fog_m` and precipitation:
/// the extinctions add.
pub fn visibility_m(fog_m: f32, p: Precip, mm_h: f32) -> f32 {
    let fog = KOSCHMIEDER / (fog_m.max(MIN_VISIBILITY_M) / 1000.0);
    let sigma = fog + extinction_per_km(p, mm_h);
    (KOSCHMIEDER / sigma * 1000.0).clamp(MIN_VISIBILITY_M, CLEAR_VISIBILITY_M)
}

/// Terminal fall speed (m/s): raindrops grow with the intensity (≈ 4 m/s
/// in light rain, 8–9 m/s in a cloudburst), drizzle drifts, snow floats,
/// hail falls fast.
pub fn fall_speed_mps(p: Precip, mm_h: f32) -> f32 {
    match p {
        Precip::None => 0.0,
        Precip::Drizzle => 1.5,
        Precip::Rain => (4.0 + 1.6 * mm_h.max(0.0).ln_1p()).min(9.2),
        Precip::Snow => 1.0,
        Precip::Sleet => 3.0,
        Precip::Hail => (12.0 + 0.15 * mm_h).min(25.0),
    }
}

/// The particles around the camera for one kind of precipitation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particles {
    pub kind: Precip,
    /// How many of the budget are drawn.
    pub count: u32,
    /// Half size of the box around the camera (m; x/z, y).
    pub half_box: [f32; 2],
    /// Fall speed (m/s).
    pub fall_mps: f32,
    /// Width of a particle (m) and the length of its streak (m: the
    /// distance it falls in a 1/45 s exposure, at least its width).
    pub width_m: f32,
    pub streak_m: f32,
    /// Opacity of one particle (0–1).
    pub opacity: f32,
    /// How much it is carried by the wind (0–1).
    pub drift: f32,
    /// Side-to-side swaying (m; snow flakes tumble).
    pub sway_m: f32,
}

/// Default particle budget (the mesh holds this many quads).
pub const BUDGET: u32 = 40_000;

/// The particles for `p` at `mm_h` with `budget` quads at most: the
/// share of the budget grows with the square root of the intensity
/// (doubling the rain does not double what one sees: drops overlap,
/// visibility drops), from a few hundred in a sprinkle to the whole budget
/// in a cloudburst.
pub fn particles(p: Precip, mm_h: f32, budget: u32) -> Option<Particles> {
    if p == Precip::None || mm_h <= 0.0 || budget == 0 {
        return None;
    }
    // (intensity of the full budget, least share, box half sizes, width,
    // opacity, drift, sway)
    let (full, least, half_box, width, opacity, drift, sway) = match p {
        Precip::None => unreachable!(),
        Precip::Rain => (60.0, 0.03, [22.0, 14.0], 0.012, 0.28, 0.95, 0.0),
        Precip::Drizzle => (2.0, 0.08, [14.0, 9.0], 0.006, 0.22, 1.0, 0.0),
        Precip::Snow => (8.0, 0.05, [18.0, 11.0], 0.03, 0.85, 1.0, 0.35),
        Precip::Sleet => (6.0, 0.05, [18.0, 11.0], 0.016, 0.5, 0.9, 0.08),
        Precip::Hail => (40.0, 0.04, [16.0, 12.0], 0.012, 0.9, 0.6, 0.0),
    };
    let share = (mm_h / full).sqrt().clamp(least, 1.0);
    let fall = fall_speed_mps(p, mm_h);
    let count = ((budget as f32 * share).round() as u32).clamp(1, budget);
    Some(Particles {
        kind: p,
        count,
        half_box,
        fall_mps: fall,
        width_m: width,
        streak_m: (fall / 45.0).max(width),
        opacity,
        drift,
        sway_m: sway,
    })
}

impl Particles {
    /// Particles per cubic metre of the box (for comparisons).
    pub fn density_per_m3(&self) -> f32 {
        let [h, v] = self.half_box;
        self.count as f32 / (8.0 * h * h * v)
    }

    /// Fade of a particle at `distance_m` from the camera: soft in at the
    /// lens (no drop smears the view), out towards the box's edge and in
    /// fog (a drop 50 m away is not seen in 40 m fog).
    pub fn fade(&self, distance_m: f32, visibility_m: f32) -> f32 {
        let near = ((distance_m - 0.4) / 1.2).clamp(0.0, 1.0);
        let edge = (1.0 - (distance_m / self.half_box[0] - 0.7) / 0.3).clamp(0.0, 1.0);
        let fog = (-KOSCHMIEDER * distance_m / visibility_m.max(1.0)).exp();
        near * edge * fog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_falls_with_intensity_and_snow_is_worst() {
        let clear = visibility_m(CLEAR_VISIBILITY_M, Precip::None, 0.0);
        assert_eq!(clear, CLEAR_VISIBILITY_M);
        let light = visibility_m(CLEAR_VISIBILITY_M, Precip::Rain, 1.0);
        let heavy = visibility_m(CLEAR_VISIBILITY_M, Precip::Rain, 25.0);
        let burst = visibility_m(CLEAR_VISIBILITY_M, Precip::Rain, 100.0);
        assert!(light > heavy && heavy > burst);
        // Light rain: kilometres; a cloudburst: under a kilometre.
        assert!(light > 8000.0, "{light}");
        assert!((1500.0..3000.0).contains(&heavy), "{heavy}");
        assert!(burst < 1000.0, "{burst}");
        // Snow: 5 mm/h is a few hundred metres, far less than 5 mm/h rain.
        let snow = visibility_m(CLEAR_VISIBILITY_M, Precip::Snow, 5.0);
        assert!((250.0..500.0).contains(&snow), "{snow}");
        assert!(snow < visibility_m(CLEAR_VISIBILITY_M, Precip::Rain, 5.0) / 10.0);
        // Fog and rain add up: worse than either alone.
        let both = visibility_m(300.0, Precip::Rain, 10.0);
        assert!(both < 300.0 && both < visibility_m(CLEAR_VISIBILITY_M, Precip::Rain, 10.0));
        // Koschmieder: fog of 100 m alone is 100 m.
        assert!((visibility_m(100.0, Precip::None, 0.0) - 100.0).abs() < 0.01);
    }

    #[test]
    fn fall_speeds() {
        let drizzle = fall_speed_mps(Precip::Drizzle, 0.3);
        let light = fall_speed_mps(Precip::Rain, 1.0);
        let heavy = fall_speed_mps(Precip::Rain, 50.0);
        let snow = fall_speed_mps(Precip::Snow, 3.0);
        let hail = fall_speed_mps(Precip::Hail, 30.0);
        assert!(snow < drizzle && drizzle < light && light < heavy && heavy < hail);
        assert!((4.0..6.0).contains(&light), "{light}");
        assert!(heavy <= 9.2);
        assert!((0.5..1.5).contains(&snow));
    }

    #[test]
    fn particle_density_grows_with_intensity_within_the_budget() {
        assert!(particles(Precip::None, 5.0, BUDGET).is_none());
        assert!(particles(Precip::Rain, 0.0, BUDGET).is_none());
        let mut prev = 0;
        for mm in [0.2, 1.0, 4.0, 15.0, 30.0, 60.0, 200.0] {
            let p = particles(Precip::Rain, mm, BUDGET).unwrap();
            assert!(p.count >= prev && p.count <= BUDGET, "{mm}: {}", p.count);
            prev = p.count;
        }
        assert_eq!(prev, BUDGET, "a cloudburst fills the budget");
        // Square root: four times the rain, twice the drops.
        let a = particles(Precip::Rain, 2.0, BUDGET).unwrap().count as f32;
        let b = particles(Precip::Rain, 8.0, BUDGET).unwrap().count as f32;
        assert!((b / a - 2.0).abs() < 0.01, "{a} {b}");
        // A smaller budget (a weak GPU, the browser) scales the count.
        let small = particles(Precip::Rain, 8.0, BUDGET / 4).unwrap().count as f32;
        assert!((small / b - 0.25).abs() < 0.01);
        // Snow flakes: wide, slow, swaying, opaque; rain: thin streaks.
        let snow = particles(Precip::Snow, 2.0, BUDGET).unwrap();
        let rain = particles(Precip::Rain, 2.0, BUDGET).unwrap();
        assert!(snow.width_m > rain.width_m && snow.sway_m > 0.0 && rain.sway_m == 0.0);
        assert!(rain.streak_m > 5.0 * rain.width_m, "streaks");
        assert!(snow.streak_m <= snow.width_m * 1.01, "flakes, not streaks");
        assert!(rain.density_per_m3() > 0.0);
    }

    #[test]
    fn particles_fade_at_the_lens_the_edge_and_in_fog() {
        let p = particles(Precip::Rain, 4.0, BUDGET).unwrap();
        assert_eq!(p.fade(0.1, 40_000.0), 0.0, "nothing on the lens");
        assert!(p.fade(5.0, 40_000.0) > 0.99);
        assert_eq!(p.fade(p.half_box[0], 40_000.0), 0.0, "edge of the box");
        assert!(p.fade(10.0, 50.0) < 0.5 * p.fade(10.0, 40_000.0), "fog");
    }
}
