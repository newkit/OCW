//! Lightning (spec §14 Phase 9, 9E; decision Q9: thunderstorms with
//! lightning): when and where it strikes (deterministic from the session's
//! seed: one draw per second of session time at the weather's rate), the
//! flash it gives, the thunder that follows at the speed of sound, and the
//! jagged channel the renderer draws.
//!
//! A strike lies at a distance and bearing *from each rider* (the storm
//! surrounds the session's riders alike), so every client sees and hears
//! the same strike at the same delay.

/// Speed of sound (m/s, air at about 20 °C).
pub const SPEED_OF_SOUND_MPS: f32 = 343.0;
/// Strikes fall between these distances (m).
pub const MIN_DISTANCE_M: f32 = 250.0;
pub const MAX_DISTANCE_M: f32 = 15_000.0;
/// Thunder is not heard beyond this distance (m).
pub const AUDIBLE_M: f32 = 20_000.0;

/// One lightning strike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strike {
    /// Unique within the session.
    pub id: u32,
    /// Session time of the flash (s).
    pub at_s: f64,
    /// Distance from the rider (m) and bearing (° clockwise from north).
    pub distance_m: f32,
    pub bearing_deg: f32,
    /// Strength (0.3–1).
    pub intensity: f32,
    /// Shape of the channel and the strokes.
    pub seed: u32,
}

impl Strike {
    /// Seconds from the flash to the thunder: three seconds per kilometre.
    pub fn thunder_delay_s(&self) -> f64 {
        f64::from(self.distance_m / SPEED_OF_SOUND_MPS)
    }

    /// Session time the thunder arrives.
    pub fn thunder_at_s(&self) -> f64 {
        self.at_s + self.thunder_delay_s()
    }

    /// Where it strikes relative to the rider (m; scene x east, z south).
    pub fn offset(&self) -> [f32; 2] {
        let b = self.bearing_deg.to_radians();
        [b.sin() * self.distance_m, -b.cos() * self.distance_m]
    }
}

/// splitmix64 of a seed and two keys, as a number in [0, 1).
pub fn unit(seed: u64, a: u64, b: u64) -> f64 {
    let mut z = seed
        .wrapping_add(a.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(b.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// A strike in the second `slot` of session time at `rate_per_min`, if
/// one falls: (seconds into the slot, distance, bearing, intensity, seed).
/// At most one per second; the chance is 1 − e^(−rate/60) (Poisson).
pub fn slot_strike(seed: u64, slot: i64, rate_per_min: f32) -> Option<(f64, f32, f32, f32, u32)> {
    if rate_per_min <= 0.0 {
        return None;
    }
    let s = slot as u64;
    let chance = 1.0 - (-f64::from(rate_per_min) / 60.0).exp();
    if unit(seed, s, 1) >= chance {
        return None;
    }
    // Uniform over the area of the ring: more far strikes than near ones.
    let (r0, r1) = (MIN_DISTANCE_M as f64, MAX_DISTANCE_M as f64);
    let d = (r0 * r0 + unit(seed, s, 3) * (r1 * r1 - r0 * r0)).sqrt();
    Some((
        unit(seed, s, 2),
        d as f32,
        (unit(seed, s, 4) * 360.0) as f32,
        (0.3 + 0.7 * unit(seed, s, 5)) as f32,
        (unit(seed, s, 6) * f64::from(u32::MAX)) as u32,
    ))
}

/// Number of return strokes of a strike (1–4): the flicker.
pub fn strokes(seed: u32) -> u32 {
    1 + seed % 4
}

/// Brightness of the flash at the rider `age_s` after the strike (0 –
/// about 1; 0 before and after): each return stroke flashes and decays in
/// a few hundredths of a second, 50–110 ms apart; near strikes light the
/// landscape, far ones only the clouds.
pub fn flash(age_s: f32, s: &Strike) -> f32 {
    if !(0.0..1.0).contains(&age_s) {
        return 0.0;
    }
    let n = strokes(s.seed);
    let mut sum = 0.0;
    let mut t0 = 0.0;
    for k in 0..n {
        let dt = age_s - t0;
        if dt >= 0.0 {
            let amp = if k == 0 { 1.0 } else { 0.75 };
            sum += amp * (-dt / 0.03).exp();
        }
        let gap = 0.05 + 0.06 * (((s.seed >> (k * 3)) & 7) as f32 / 7.0);
        t0 += gap;
    }
    let near = 1.0 / (1.0 + (s.distance_m / 3000.0).powi(2));
    (sum * s.intensity * (0.15 + 0.85 * near)).min(1.5)
}

/// How a strike's thunder sounds at the rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thunder {
    /// Loudness (0–1).
    pub loudness: f32,
    /// A sharp crack first (strikes within 1.5 km).
    pub crack: bool,
    /// Length of the rumble (s): longer from farther away (the sound of
    /// the whole channel arrives over seconds).
    pub rumble_s: f32,
    /// The air takes the highs: cutoff of the rumble (Hz).
    pub cutoff_hz: f32,
}

/// The thunder of a strike `distance_m` away; `None` beyond hearing.
pub fn thunder(distance_m: f32, intensity: f32) -> Option<Thunder> {
    if distance_m >= AUDIBLE_M {
        return None;
    }
    let km = distance_m / 1000.0;
    Some(Thunder {
        loudness: (intensity * (1.0 / (1.0 + 0.35 * km)).powf(1.3)).clamp(0.0, 1.0),
        crack: distance_m < 1500.0,
        rumble_s: (1.8 + 0.55 * km).min(9.0),
        cutoff_hz: (1800.0 / (1.0 + 0.6 * km)).max(120.0),
    })
}

/// The channel of a strike: the main channel from the cloud base at
/// height `top_m` down to the ground (0), and a few branches, as
/// polylines in the strike's frame (m; x, y, z, y up), jagged by the seed.
pub fn bolt(seed: u32, top_m: f32) -> Vec<Vec<[f32; 3]>> {
    let s = u64::from(seed);
    let r = |k: u64| unit(s, k, 77) as f32 * 2.0 - 1.0;
    let segments = 24u64;
    let mut main = Vec::with_capacity(segments as usize + 1);
    let (mut x, mut z) = (0.0f32, 0.0f32);
    for i in 0..=segments {
        let y = top_m * (1.0 - i as f32 / segments as f32);
        if i > 0 && i < segments {
            let step = top_m / segments as f32;
            x += r(i * 2) * step * 0.6;
            z += r(i * 2 + 1) * step * 0.6;
        }
        // The last point is the ground under the wandering channel.
        main.push([x, y.max(0.0), z]);
    }
    let mut out = vec![main.clone()];
    let branches = 2 + (seed % 3) as usize;
    for b in 0..branches as u64 {
        let from = 3 + (unit(s, b, 91) * (segments as f64 * 0.6)) as usize;
        let start = main[from.min(main.len() - 2)];
        let len = 3 + (unit(s, b, 92) * 6.0) as usize;
        let mut p = start;
        let mut branch = vec![p];
        let dir = [r(100 + b * 7), r(101 + b * 7)];
        for k in 0..len as u64 {
            let step = top_m / segments as f32;
            p = [
                p[0] + (dir[0] + 0.5 * r(200 + b * 31 + k)) * step * 0.7,
                (p[1] - step * (0.6 + 0.4 * r(300 + b * 31 + k).abs())).max(0.0),
                p[2] + (dir[1] + 0.5 * r(400 + b * 31 + k)) * step * 0.7,
            ];
            branch.push(p);
        }
        out.push(branch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike(distance_m: f32) -> Strike {
        Strike {
            id: 1,
            at_s: 100.0,
            distance_m,
            bearing_deg: 90.0,
            intensity: 1.0,
            seed: 6,
        }
    }

    /// Thunder timing: three seconds per kilometre.
    #[test]
    fn thunder_follows_at_the_speed_of_sound() {
        let s = strike(3430.0);
        assert!((s.thunder_delay_s() - 10.0).abs() < 1e-4);
        assert!((s.thunder_at_s() - 110.0).abs() < 1e-4);
        assert!((strike(1000.0).thunder_delay_s() - 2.915).abs() < 0.01);
        // East of the rider: +x.
        let o = s.offset();
        assert!((o[0] - 3430.0).abs() < 0.1 && o[1].abs() < 0.1);
    }

    #[test]
    fn near_thunder_cracks_far_thunder_rumbles_low_and_long() {
        let near = thunder(600.0, 1.0).unwrap();
        let far = thunder(9000.0, 1.0).unwrap();
        assert!(near.crack && !far.crack);
        assert!(near.loudness > 2.0 * far.loudness);
        assert!(far.rumble_s > near.rumble_s);
        assert!(far.cutoff_hz < near.cutoff_hz);
        assert!(thunder(AUDIBLE_M + 1.0, 1.0).is_none());
    }

    /// Strikes are deterministic per seed and come at the weather's rate.
    #[test]
    fn strikes_come_at_the_rate_and_repeat_per_seed() {
        assert!(slot_strike(7, 5, 0.0).is_none());
        let count = |rate: f32| {
            (0..36_000)
                .filter(|&s| slot_strike(42, s, rate).is_some())
                .count()
        };
        // 10 hours at 6 per minute: 3600 expected (1 − e^−0.1 per second).
        let n = count(6.0) as f64;
        let expected = 36_000.0 * (1.0 - (-0.1f64).exp());
        assert!((n / expected - 1.0).abs() < 0.05, "{n} vs {expected}");
        assert!(count(1.0) < count(6.0));
        assert_eq!(slot_strike(42, 1234, 30.0), slot_strike(42, 1234, 30.0));
        let all: Vec<_> = (0..20_000)
            .filter_map(|s| slot_strike(9, s, 20.0))
            .collect();
        assert!(all.iter().all(|s| (0.0..1.0).contains(&s.0)
            && (MIN_DISTANCE_M..=MAX_DISTANCE_M).contains(&s.1)
            && (0.0..360.0).contains(&s.2)
            && (0.3..=1.0).contains(&s.3)));
        // More far strikes than near ones (uniform over the area).
        let near = all.iter().filter(|s| s.1 < 5000.0).count();
        assert!(near < all.len() / 3, "{near} of {}", all.len());
        // Another seed, other strikes.
        let other: Vec<_> = (0..200).filter_map(|s| slot_strike(10, s, 20.0)).collect();
        assert_ne!(
            other,
            (0..200)
                .filter_map(|s| slot_strike(9, s, 20.0))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_flash_flickers_and_fades_within_a_second() {
        let s = Strike {
            seed: 3, // four strokes
            ..strike(800.0)
        };
        assert_eq!(strokes(3), 4);
        assert_eq!(flash(-0.01, &s), 0.0);
        assert!(flash(0.0, &s) > 0.8);
        assert!(flash(0.04, &s) < flash(0.0, &s), "decays");
        assert_eq!(flash(1.0, &s), 0.0);
        assert!(flash(0.6, &s) < 0.01);
        // The flicker: brightness rises again for a later stroke.
        let samples: Vec<f32> = (0..40).map(|i| flash(i as f32 * 0.01, &s)).collect();
        let rises = samples.windows(2).filter(|w| w[1] > w[0] + 0.05).count();
        assert!(rises >= 1, "{samples:?}");
        // Far strikes are dimmer at the rider.
        assert!(flash(0.0, &strike(12_000.0)) < 0.4 * flash(0.0, &strike(500.0)));
    }

    #[test]
    fn the_channel_runs_from_the_cloud_to_the_ground() {
        let b = bolt(12345, 900.0);
        assert!(b.len() >= 3, "main channel and branches");
        let main = &b[0];
        assert_eq!(main[0], [0.0, 900.0, 0.0]);
        assert_eq!(main.last().unwrap()[1], 0.0);
        assert!(main.windows(2).all(|w| w[1][1] <= w[0][1]), "downwards");
        for branch in &b[1..] {
            assert!(branch.iter().all(|p| p[1] >= 0.0 && p[1] <= 900.0));
            assert!(branch.last().unwrap()[1] < branch[0][1]);
        }
        assert_eq!(bolt(12345, 900.0), b, "deterministic");
    }
}
