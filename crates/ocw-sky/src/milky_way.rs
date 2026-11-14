//! The Milky Way (spec §14 Phase 9, S2; Q7: no star catalogue): a
//! procedural band along the galactic plane, own work – no photo, no
//! catalogue. Galactic coordinates come from the IAU definition (the
//! ICRS ↔ galactic rotation, a published constant); the brightness is a
//! smooth model of what the eye sees: brightest and widest towards the
//! galactic centre in Sagittarius, the Great Rift splitting it from Cygnus
//! to Scutum, bright clouds in Cygnus, Scutum and Carina, faint towards the
//! anticentre, mottled by a little deterministic noise.

use crate::astro::Equatorial;

/// ICRS (≈ J2000 equatorial) → galactic rotation (rows: the galactic x
/// towards the centre, y towards l = 90°, z towards the north galactic
/// pole, in equatorial components; Hipparcos introduction vol. 1, §1.5.3).
#[rustfmt::skip]
pub const EQ_TO_GAL: [[f64; 3]; 3] = [
    [-0.054_875_560_4, -0.873_437_090_2, -0.483_835_015_5],
    [0.494_109_427_9, -0.444_829_630_0, 0.746_982_244_5],
    [-0.867_666_149_0, -0.198_076_373_4, 0.455_983_776_2],
];

/// Equatorial (J2000) direction of galactic longitude / latitude (°).
pub fn galactic_to_equatorial(l: f64, b: f64) -> Equatorial {
    let (l, b) = (l.to_radians(), b.to_radians());
    let g = [b.cos() * l.cos(), b.cos() * l.sin(), b.sin()];
    // The transpose takes galactic back to equatorial.
    let e = [0, 1, 2].map(|i| (0..3).map(|k| EQ_TO_GAL[k][i] * g[k]).sum::<f64>());
    Equatorial {
        ra_deg: e[1].atan2(e[0]).to_degrees().rem_euclid(360.0),
        dec_deg: e[2].clamp(-1.0, 1.0).asin().to_degrees(),
    }
}

/// Unit vector (equatorial J2000) of galactic (l, b) in degrees.
pub fn galactic_vector(l: f64, b: f64) -> [f64; 3] {
    crate::astro::equatorial_vector(galactic_to_equatorial(l, b))
}

fn hash(x: i64, y: i64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in 0–1 (cells of 1 unit, bilinear with a smoothstep).
fn noise(x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (x0 as i64, y0 as i64);
    let a = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let b = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    a + (b - a) * sy
}

fn bump(x: f64, centre: f64, width: f64) -> f64 {
    let d = crate::astro::wrap180(x - centre) / width;
    (-d * d).exp()
}

/// Surface brightness of the band at galactic (l, b) in degrees, 0–1
/// (1: the star clouds of Sagittarius near the centre).
pub fn brightness(l: f64, b: f64) -> f64 {
    let dl = crate::astro::wrap180(l);
    // Wider and brighter towards the centre.
    let centre = bump(dl, 0.0, 55.0);
    let width = 4.5 + 7.5 * centre;
    let along = 0.22
        + 0.48 * centre
        + 0.22 * bump(dl, 78.0, 18.0) // Cygnus
        + 0.18 * bump(dl, 27.0, 9.0) // Scutum cloud
        + 0.16 * bump(dl, 287.0 - 360.0, 22.0) // Carina
        + 0.10 * bump(dl, 315.0 - 360.0, 15.0); // Crux / Centaurus
    let disc = (-(b / width).powi(2) / 2.0).exp();
    // The bulge: a bright oval around the centre.
    let bulge = 0.55 * (-((dl / 11.0).powi(2) + ((b + 1.5) / 8.0).powi(2))).exp();
    // The Great Rift: dust along b ≈ +1° … +3° from Cygnus to Sagittarius.
    let rift_l = if (-25.0..75.0).contains(&dl) {
        let t = (dl + 25.0) / 100.0;
        (std::f64::consts::PI * t).sin().powf(0.6)
    } else {
        0.0
    };
    let rift = 1.0 - 0.65 * rift_l * (-((b - 1.8) / 2.2).powi(2)).exp();
    // Mottling: star clouds and dark lanes.
    let n = 0.5 * noise(l / 6.0, b / 3.0)
        + 0.3 * noise(l / 2.0 + 17.3, b / 1.2 + 5.1)
        + 0.2 * noise(l / 0.8 + 3.7, b / 0.6 + 9.2);
    let v = (along * disc + bulge) * rift * (0.5 + n);
    v.clamp(0.0, 1.0)
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How visible the Milky Way is (0–1): only in a dark sky – after
/// nautical twilight, fully at astronomical night – washed out by the moon
/// (by its lit share and height) and by light pollution (0 none … 1 a city
/// centre; at 0.6 it is gone).
pub fn visibility(sun_el: f64, moon_el: f64, moon_illuminated: f64, pollution: f64) -> f64 {
    let dark = smoothstep(-11.0, -17.0, sun_el);
    let moon = moon_illuminated.clamp(0.0, 1.0).powf(0.7) * smoothstep(-1.0, 25.0, moon_el);
    let polluted = (1.0 - pollution.clamp(0.0, 1.0) / 0.6).clamp(0.0, 1.0);
    (dark * (1.0 - 0.9 * moon) * polluted * polluted).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::separation;

    #[test]
    fn galactic_frame_matches_the_iau_definition() {
        // The galactic centre: α 266.405°, δ -28.936°; the north galactic
        // pole: α 192.859°, δ 27.128°.
        let gc = galactic_to_equatorial(0.0, 0.0);
        let want = Equatorial {
            ra_deg: 266.405,
            dec_deg: -28.936,
        };
        assert!(separation(gc, want) < 0.01, "{gc:?}");
        let ngp = galactic_to_equatorial(123.0, 90.0);
        let want = Equatorial {
            ra_deg: 192.859,
            dec_deg: 27.128,
        };
        assert!(separation(ngp, want) < 0.01, "{ngp:?}");
        // Deneb (α 310.358°, δ 45.280°) lies in the band at l 84.3°, b 2.0°.
        let deneb = galactic_to_equatorial(84.28, 1.99);
        let want = Equatorial {
            ra_deg: 310.358,
            dec_deg: 45.280,
        };
        assert!(separation(deneb, want) < 0.1, "{deneb:?}");
    }

    #[test]
    fn the_band_is_bright_in_sagittarius_and_fades_off_the_plane() {
        let mean = |l: f64, b: f64| {
            (0..9)
                .map(|k| brightness(l + f64::from(k) * 0.7, b))
                .sum::<f64>()
                / 9.0
        };
        let centre = mean(1.0, -4.0);
        let anticentre = mean(178.0, 0.0);
        assert!(centre > 0.5, "{centre}");
        assert!(anticentre < 0.3 && anticentre > 0.05, "{anticentre}");
        for l in [0.0, 60.0, 180.0, 280.0] {
            assert!(mean(l, 0.0) > 3.0 * mean(l, 35.0), "{l}");
        }
        assert!(mean(90.0, 60.0) < 0.01, "dark far off the plane");
        // The Great Rift: darker on it than just south of it, in Aquila.
        assert!(mean(35.0, 1.8) < mean(35.0, -5.0), "rift");
        for (l, b) in [(0.0, 0.0), (123.0, -45.0), (300.0, 3.0)] {
            assert!((0.0..=1.0).contains(&brightness(l, b)));
        }
    }

    #[test]
    fn only_a_dark_sky_shows_it() {
        // Moonless astronomical night far from towns: fully visible.
        assert!(visibility(-25.0, -20.0, 0.0, 0.0) > 0.99);
        // Twilight, a high full moon or a town wash it out.
        assert!(visibility(-8.0, -20.0, 0.0, 0.0) < 0.01);
        assert!(visibility(-25.0, 45.0, 1.0, 0.0) < 0.15);
        assert!(visibility(-25.0, -20.0, 0.0, 0.7) == 0.0);
        let thin = visibility(-25.0, 30.0, 0.1, 0.0);
        assert!(thin > 0.6, "a thin crescent hardly matters: {thin}");
        let suburb = visibility(-25.0, -20.0, 0.0, 0.25);
        assert!(suburb > 0.2 && suburb < 0.5, "{suburb}");
    }
}
