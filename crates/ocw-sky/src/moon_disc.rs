//! The moon's disc as an image (spec §14 Phase 9, S2), drawn on the CPU so
//! the renderer needs no shader for it: the lit part from the phase angle
//! and the bright limb's direction, the Lommel–Seeliger law of a dusty
//! surface (the full moon is evenly bright to its edge), the maria as a
//! hand-placed sketch (own work, no lunar map or photo), and the earthshine
//! on the dark part. Also the soft point sprite of the planets.

/// The larger maria and two bright craters: selenographic longitude,
/// latitude, radius (°) and albedo factor (maria ~0.6 of the highlands).
#[rustfmt::skip]
const FEATURES: [(f64, f64, f64, f64); 16] = [
    (-16.0, 33.0, 17.0, 0.62),  // Mare Imbrium
    (17.5, 28.0, 10.5, 0.60),   // Mare Serenitatis
    (31.0, 8.5, 12.0, 0.58),    // Mare Tranquillitatis
    (59.0, 17.0, 7.5, 0.60),    // Mare Crisium
    (51.0, -8.0, 9.5, 0.64),    // Mare Fecunditatis
    (35.0, -15.0, 5.5, 0.64),   // Mare Nectaris
    (-17.0, -21.0, 10.0, 0.66), // Mare Nubium
    (-39.0, -24.0, 6.0, 0.64),  // Mare Humorum
    (-57.0, 18.0, 20.0, 0.64),  // Oceanus Procellarum (north)
    (-48.0, -3.0, 15.0, 0.66),  // Oceanus Procellarum (south)
    (-31.0, 7.0, 8.0, 0.68),    // Mare Insularum
    (4.0, 13.0, 5.0, 0.66),     // Mare Vaporum
    (0.0, 56.0, 7.0, 0.72),     // Mare Frigoris
    (25.0, 56.0, 6.0, 0.74),    // Mare Frigoris (east)
    (-11.0, -43.0, 2.5, 1.25),  // Tycho
    (-20.0, 9.7, 2.0, 1.15),    // Copernicus
];

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Albedo of the near side at selenographic (lon, lat), relative to the
/// highlands (1).
pub fn albedo(lon: f64, lat: f64) -> f64 {
    let p = crate::astro::equatorial_vector(crate::astro::Equatorial {
        ra_deg: lon,
        dec_deg: lat,
    });
    let mut a = 1.0;
    for (flon, flat, r, f) in FEATURES {
        let q = crate::astro::equatorial_vector(crate::astro::Equatorial {
            ra_deg: flon,
            dec_deg: flat,
        });
        let d = (p[0] * q[0] + p[1] * q[1] + p[2] * q[2])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        let w = 1.0 - smoothstep(r * 0.7, r * 1.15, d);
        a *= 1.0 + (f - 1.0) * w;
    }
    a
}

/// What to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscParams {
    /// Sun – moon – earth angle (0 full … 180° new).
    pub phase_angle_deg: f64,
    /// Direction of the bright limb in the image: 0 right, 90° up
    /// (counter-clockwise).
    pub limb_dir_deg: f64,
    /// Brightness of the earthlit dark part relative to the lit part.
    pub earthshine: f64,
    /// 0 at night … 1 by day: by day the unlit part is transparent and
    /// the lit part only partly opaque (the moon is pale in a blue sky).
    pub day: f64,
}

impl DiscParams {
    /// Earthshine for a phase angle: strongest for a thin crescent (the
    /// earth, seen from the moon, is then nearly full).
    pub fn earthshine_for(phase_angle_deg: f64) -> f64 {
        0.035 * (1.0 - (phase_angle_deg.to_radians()).cos()) / 2.0
    }

    /// Rounded so that small changes do not ask for a new image.
    pub fn quantised(self) -> Self {
        let q = |v: f64, s: f64| (v / s).round() * s;
        Self {
            phase_angle_deg: q(self.phase_angle_deg, 0.5),
            limb_dir_deg: q(self.limb_dir_deg.rem_euclid(360.0), 1.0),
            earthshine: q(self.earthshine, 0.001),
            day: q(self.day, 0.05),
        }
    }
}

fn to_srgb8(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

/// The disc in a `size` × `size` RGBA image (sRGB, rows from the top),
/// the disc filling it edge to edge; linear brightness 1 = the lit
/// highlands.
pub fn disc(size: u32, p: DiscParams) -> Vec<[u8; 4]> {
    let n = size as usize;
    let (si, ci) = p.phase_angle_deg.to_radians().sin_cos();
    let (st, ct) = p.limb_dir_deg.to_radians().sin_cos();
    let sun = [ct * si, st * si, ci];
    let px = 2.0 / size as f64;
    let mut out = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            let x = (col as f64 + 0.5) * px - 1.0;
            let y = 1.0 - (row as f64 + 0.5) * px;
            let r = (x * x + y * y).sqrt();
            let cover = smoothstep(1.0, 1.0 - 1.5 * px, r);
            if cover <= 0.0 {
                out.push([0, 0, 0, 0]);
                continue;
            }
            let z = (1.0 - (x * x + y * y).min(1.0)).sqrt();
            let mu0 = x * sun[0] + y * sun[1] + z * sun[2];
            // Lommel–Seeliger, 1 at the centre of a full moon; a soft
            // terminator.
            let lit = 2.0 * mu0.max(0.0) / (mu0.max(0.0) + z.max(0.02));
            let lit = lit * smoothstep(-0.015, 0.03, mu0);
            let lon = x.atan2(z).to_degrees();
            let lat = y.clamp(-1.0, 1.0).asin().to_degrees();
            let a = albedo(lon, lat);
            let lum = a * (lit + p.earthshine * (1.0 - smoothstep(0.0, 0.05, mu0)));
            let lit_alpha = (lum * 1.6).clamp(0.0, 1.0);
            let alpha = cover * (1.0 + (lit_alpha * 0.85 - 1.0) * p.day.clamp(0.0, 1.0));
            let warm = [1.0, 0.975, 0.93];
            out.push([
                to_srgb8(lum * warm[0]),
                to_srgb8(lum * warm[1]),
                to_srgb8(lum * warm[2]),
                (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            ]);
        }
    }
    out
}

/// A soft round point of light (white, alpha from a narrow core and a
/// faint halo), `size` × `size`.
pub fn point_sprite(size: u32) -> Vec<[u8; 4]> {
    let n = size as usize;
    let mut out = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            let x = (col as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let y = (row as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let r2 = x * x + y * y;
            let v = (-r2 / 0.035).exp() + 0.25 * (-r2 / 0.12).exp() * (1.0 - r2).max(0.0);
            let v = v.clamp(0.0, 1.0);
            out.push([255, 255, 255, (v * 255.0 + 0.5) as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(img: &[[u8; 4]], size: usize, x: f64, y: f64) -> u8 {
        let col = ((x + 1.0) / 2.0 * size as f64) as usize;
        let row = ((1.0 - y) / 2.0 * size as f64) as usize;
        img[row.min(size - 1) * size + col.min(size - 1)][1]
    }

    fn params(i: f64, dir: f64) -> DiscParams {
        DiscParams {
            phase_angle_deg: i,
            limb_dir_deg: dir,
            earthshine: DiscParams::earthshine_for(i),
            day: 0.0,
        }
    }

    #[test]
    fn full_new_and_quarter_moons() {
        let n = 64;
        let full = disc(n, params(0.0, 0.0));
        // Evenly bright to the edge (Lommel–Seeliger), highlands near white.
        let (c, e) = (lum(&full, 64, 0.0, -0.6), lum(&full, 64, 0.0, -0.93));
        assert!(c > 200 && e > 200, "{c} {e}");
        // Outside the disc: transparent.
        assert_eq!(full[0][3], 0);
        assert_eq!(full[32 * 64 + 32][3], 255);
        // New moon: only the faint earthshine.
        let new = disc(n, params(179.0, 0.0));
        assert!(new.iter().all(|p| p[1] < 70), "dark");
        assert!(lum(&new, 64, 0.0, 0.0) > 10, "earthshine");
        // First quarter lit towards +x (right): bright right, dark left.
        let q = disc(n, params(90.0, 0.0));
        assert!(lum(&q, 64, 0.5, -0.3) > 150);
        assert!(lum(&q, 64, -0.5, -0.3) < 60);
        // Lit towards up: bright top, dark bottom.
        let up = disc(n, params(90.0, 90.0));
        assert!(lum(&up, 64, 0.0, 0.6) > 120 && lum(&up, 64, 0.0, -0.6) < 60);
    }

    #[test]
    fn maria_darken_the_near_side() {
        // Mare Tranquillitatis against the southern highlands.
        assert!(albedo(31.0, 8.5) < 0.7);
        assert!(albedo(10.0, -60.0) > 0.95);
        assert!(albedo(-11.0, -43.0) > 1.1, "Tycho");
    }

    #[test]
    fn by_day_the_dark_part_vanishes() {
        let mut p = params(120.0, 0.0);
        p.day = 1.0;
        let img = disc(64, p);
        let dark = img[32 * 64 + 8][3];
        let bright = img[32 * 64 + 60][3];
        assert!(dark < 10 && bright > 100, "{dark} {bright}");
        assert_eq!(
            params(12.34, 361.2).quantised().limb_dir_deg,
            1.0,
            "wrapped and rounded"
        );
        let s = point_sprite(16);
        assert!(s[8 * 16 + 8][3] > 200 && s[0][3] < 5);
    }
}
