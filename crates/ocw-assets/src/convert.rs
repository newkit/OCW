//! Source maps (albedo, normal, roughness, AO; JPEG / PNG from ambientCG
//! and Poly Haven) → the library's GPU textures (spec §4.6, Phase 8 T1):
//!
//! - `<id>.albedo.ktx2`: BC3 sRGB, albedo in RGB (ambient occlusion baked
//!   in, softened), roughness in alpha;
//! - `<id>.normal.ktx2`: BC5, the tangent-space normal's x and y (OpenGL
//!   convention: +y = up in the image); z is rebuilt in the shader.
//!
//! Both at [`SIZE`] (the library's texture size) with a full mip chain (albedo averaged in linear light,
//! normals renormalised per level), compressed with `texpresso` (pure
//! Rust, deterministic). The mean albedo (linear RGB) goes into the
//! metadata, for the far colour of the land classes.

use crate::ktx2::{TexFormat, Texture, full_chain};
pub use crate::materials::{META_ID, META_MEAN, TEXTURE_SIZE as SIZE};

/// How strongly the ambient occlusion map darkens the albedo (0 … 1).
pub const AO_STRENGTH: f32 = 0.6;

/// sRGB (0 … 1) → linear.
pub fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear → sRGB (0 … 1).
pub fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// An image as floats, `channels` per texel, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<f32>,
}

impl Plane {
    pub fn new(width: u32, height: u32, channels: usize) -> Self {
        Self {
            width,
            height,
            channels,
            data: vec![0.0; width as usize * height as usize * channels],
        }
    }

    /// From 8-bit texels (`channels` per texel), values 0 … 1.
    pub fn from_u8(width: u32, height: u32, channels: usize, bytes: &[u8]) -> Self {
        Self {
            width,
            height,
            channels,
            data: bytes.iter().map(|&b| f32::from(b) / 255.0).collect(),
        }
    }

    pub fn at(&self, x: u32, y: u32, c: usize) -> f32 {
        self.data[(y as usize * self.width as usize + x as usize) * self.channels + c]
    }

    fn set(&mut self, x: u32, y: u32, c: usize, v: f32) {
        let i = (y as usize * self.width as usize + x as usize) * self.channels + c;
        self.data[i] = v;
    }

    /// Bilinear resample to `w × h` (wrapping: the maps tile).
    pub fn resized(&self, w: u32, h: u32) -> Self {
        if (w, h) == (self.width, self.height) {
            return self.clone();
        }
        let mut out = Plane::new(w, h, self.channels);
        let (sx, sy) = (self.width as f32 / w as f32, self.height as f32 / h as f32);
        // Area average when shrinking by an integer factor, else bilinear.
        for y in 0..h {
            for x in 0..w {
                let fx = (x as f32 + 0.5) * sx - 0.5;
                let fy = (y as f32 + 0.5) * sy - 0.5;
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let wrap = |v: f32, n: u32| (v as i64).rem_euclid(i64::from(n)) as u32;
                let (xa, xb) = (wrap(x0, self.width), wrap(x0 + 1.0, self.width));
                let (ya, yb) = (wrap(y0, self.height), wrap(y0 + 1.0, self.height));
                for c in 0..self.channels {
                    let top = self.at(xa, ya, c) * (1.0 - tx) + self.at(xb, ya, c) * tx;
                    let bot = self.at(xa, yb, c) * (1.0 - tx) + self.at(xb, yb, c) * tx;
                    out.set(x, y, c, top * (1.0 - ty) + bot * ty);
                }
            }
        }
        out
    }

    /// The next mip level: 2 × 2 box average.
    pub fn half(&self) -> Self {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut out = Plane::new(w, h, self.channels);
        for y in 0..h {
            for x in 0..w {
                for c in 0..self.channels {
                    let at = |dx: u32, dy: u32| {
                        self.at(
                            (2 * x + dx).min(self.width - 1),
                            (2 * y + dy).min(self.height - 1),
                            c,
                        )
                    };
                    out.set(x, y, c, (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4.0);
                }
            }
        }
        out
    }

    /// Mean of channel `c`.
    pub fn mean(&self, c: usize) -> f32 {
        let n = (self.width * self.height) as usize;
        (0..n)
            .map(|i| self.data[i * self.channels + c])
            .sum::<f32>()
            / n as f32
    }
}

/// The source maps of a material, decoded (values 0 … 1, as stored:
/// albedo sRGB, the others linear).
pub struct SourceMaps {
    /// RGB.
    pub albedo: Plane,
    /// RGB, OpenGL convention.
    pub normal: Plane,
    /// One channel.
    pub roughness: Plane,
    /// One channel; `None`: no occlusion map.
    pub ao: Option<Plane>,
}

/// Albedo (linear RGB, AO applied) and roughness (A) at `size`.
pub fn albedo_rough(maps: &SourceMaps, size: u32) -> Plane {
    let a = maps.albedo.resized(size, size);
    let r = maps.roughness.resized(size, size);
    let ao = maps.ao.as_ref().map(|p| p.resized(size, size));
    let mut out = Plane::new(size, size, 4);
    for y in 0..size {
        for x in 0..size {
            let occ = ao
                .as_ref()
                .map_or(1.0, |p| 1.0 - AO_STRENGTH * (1.0 - p.at(x, y, 0)));
            for c in 0..3 {
                out.set(x, y, c, to_linear(a.at(x, y, c)) * occ);
            }
            out.set(x, y, 3, r.at(x, y, 0));
        }
    }
    out
}

/// Unit normals (xyz, −1 … 1) at `size`.
pub fn normals(maps: &SourceMaps, size: u32) -> Plane {
    let n = maps.normal.resized(size, size);
    let mut out = Plane::new(size, size, 3);
    for y in 0..size {
        for x in 0..size {
            let v = [0, 1, 2].map(|c| n.at(x, y, c) * 2.0 - 1.0);
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
            for (c, vc) in v.iter().enumerate() {
                out.set(x, y, c, vc / len);
            }
        }
    }
    out
}

fn compress(format: texpresso::Format, rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; format.compressed_size(w as usize, h as usize)];
    let params = texpresso::Params {
        algorithm: texpresso::Algorithm::ClusterFit,
        ..Default::default()
    };
    format.compress(rgba, w as usize, h as usize, params, &mut out);
    out
}

/// Albedo + roughness → BC3 sRGB texture with mips.
pub fn encode_albedo(plane: &Plane, id: &str) -> Texture {
    assert_eq!(plane.channels, 4);
    let n = full_chain(plane.width, plane.height);
    let mut levels = Vec::new();
    let mut p = plane.clone();
    for i in 0..n {
        if i > 0 {
            p = p.half();
        }
        let rgba: Vec<u8> = p
            .data
            .chunks(4)
            .flat_map(|t| {
                [
                    (to_srgb(t[0]) * 255.0).round() as u8,
                    (to_srgb(t[1]) * 255.0).round() as u8,
                    (to_srgb(t[2]) * 255.0).round() as u8,
                    (t[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                ]
            })
            .collect();
        levels.push(compress(texpresso::Format::Bc3, &rgba, p.width, p.height));
    }
    let mean = [plane.mean(0), plane.mean(1), plane.mean(2)];
    Texture {
        format: TexFormat::Bc3Srgb,
        width: plane.width,
        height: plane.height,
        levels,
        metadata: vec![
            (META_ID.into(), id.into()),
            (
                META_MEAN.into(),
                format!("{:.5} {:.5} {:.5}", mean[0], mean[1], mean[2]),
            ),
        ],
    }
}

/// Unit normals → BC5 texture (x, y) with mips (renormalised per level).
pub fn encode_normal(plane: &Plane, id: &str) -> Texture {
    assert_eq!(plane.channels, 3);
    let n = full_chain(plane.width, plane.height);
    let mut levels = Vec::new();
    let mut p = plane.clone();
    for i in 0..n {
        if i > 0 {
            p = p.half();
            for t in p.data.chunks_mut(3) {
                let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-6);
                t.iter_mut().for_each(|v| *v /= len);
            }
        }
        let rg: Vec<u8> = p
            .data
            .chunks(3)
            .flat_map(|t| {
                let q = |v: f32| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                [q(t[0]), q(t[1]), 0, 255]
            })
            .collect();
        levels.push(compress(texpresso::Format::Bc5, &rg, p.width, p.height));
    }
    Texture {
        format: TexFormat::Bc5,
        width: plane.width,
        height: plane.height,
        levels,
        metadata: vec![(META_ID.into(), id.into())],
    }
}

/// Decodes level `i` of a BC3 / BC5 texture to RGBA8 (for tests and the
/// CPU fallback).
pub fn decode_level(t: &Texture, i: usize) -> Vec<u8> {
    let (w, h) = t.level_size(i);
    let mut out = vec![0u8; (w * h * 4) as usize];
    let f = match t.format {
        TexFormat::Bc3Srgb => texpresso::Format::Bc3,
        TexFormat::Bc5 => texpresso::Format::Bc5,
        TexFormat::Rgba8 | TexFormat::Rgba8Srgb => {
            return t.levels[i].clone();
        }
    };
    f.decompress(&t.levels[i], w as usize, h as usize, &mut out);
    out
}

/// Decodes an encoded JPEG / PNG source map (`channels`: 3 = RGB, 1 = the
/// first channel / luma).
pub fn decode_image(bytes: &[u8], channels: usize) -> Result<Plane, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    Ok(match channels {
        1 => Plane::from_u8(w, h, 1, img.to_luma8().as_raw()),
        3 => Plane::from_u8(w, h, 3, img.to_rgb8().as_raw()),
        _ => return Err(format!("{channels} channels")),
    })
}

/// One channel of a decoded RGB plane (Poly Haven's packed AO / rough /
/// metal map: R = AO, G = roughness).
pub fn channel(p: &Plane, c: usize) -> Plane {
    Plane {
        width: p.width,
        height: p.height,
        channels: 1,
        data: p.data.chunks(p.channels).map(|t| t[c]).collect(),
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)] // parallel arrays
mod tests {
    use super::*;

    /// A synthetic material: stones on a gradient, with bumps.
    fn sample(size: u32) -> SourceMaps {
        let mut albedo = Plane::new(size, size, 3);
        let mut normal = Plane::new(size, size, 3);
        let mut roughness = Plane::new(size, size, 1);
        let mut ao = Plane::new(size, size, 1);
        for y in 0..size {
            for x in 0..size {
                let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
                let stone = ((u * 8.0).fract() - 0.5).hypot((v * 8.0).fract() - 0.5) < 0.35;
                let base = if stone {
                    [0.55, 0.52, 0.48]
                } else {
                    [0.30, 0.42, 0.18]
                };
                for c in 0..3 {
                    albedo.set(x, y, c, base[c] * (0.9 + 0.2 * u));
                }
                let (nx, ny) = ((u * 50.0).sin() * 0.3, (v * 50.0).cos() * 0.3);
                let nz = (1.0 - nx * nx - ny * ny).sqrt();
                for (c, val) in [nx, ny, nz].iter().enumerate() {
                    normal.set(x, y, c, val * 0.5 + 0.5);
                }
                roughness.set(x, y, 0, if stone { 0.6 } else { 0.9 });
                ao.set(x, y, 0, if stone { 1.0 } else { 0.7 });
            }
        }
        SourceMaps {
            albedo,
            normal,
            roughness,
            ao: Some(ao),
        }
    }

    #[test]
    fn srgb_round_trip() {
        for i in 0..=255u8 {
            let c = f32::from(i) / 255.0;
            assert!((to_srgb(to_linear(c)) - c).abs() < 1e-4);
        }
    }

    #[test]
    fn mips_average_in_linear_light_and_resizing_wraps() {
        // A black / white checker averages to 0.5 linear = 0.735 sRGB.
        let mut p = Plane::new(2, 2, 4);
        p.set(0, 0, 0, 1.0);
        p.set(1, 1, 0, 1.0);
        let h = p.half();
        assert_eq!((h.width, h.height), (1, 1));
        assert!((h.at(0, 0, 0) - 0.5).abs() < 1e-6);
        let m = sample(64);
        let r = m.albedo.resized(32, 32);
        assert!((r.mean(1) - m.albedo.mean(1)).abs() < 0.01);
    }

    /// Statistical image test of a material sample (spec Phase 8E): after
    /// BC3 / BC5 compression the albedo keeps its mean colour and detail
    /// (PSNR), the roughness its values, the normals their direction.
    #[test]
    fn compressed_material_sample_keeps_its_look() {
        let size = 128;
        let maps = sample(size);
        let ar = albedo_rough(&maps, size);
        let t = encode_albedo(&ar, "sample");
        assert_eq!(t.levels.len(), 8);
        t.check().unwrap();
        let decoded = decode_level(&t, 0);
        let mut se = 0.0f64;
        let mut sum = [0.0f64; 3];
        for (i, px) in decoded.chunks(4).enumerate() {
            for c in 0..3 {
                let want = to_srgb(ar.data[i * 4 + c]) * 255.0;
                let got = f32::from(px[c]);
                se += f64::from((want - got).powi(2));
                sum[c] += f64::from(to_linear(got / 255.0));
            }
            let rough = f32::from(px[3]) / 255.0;
            assert!((rough - ar.data[i * 4 + 3]).abs() < 0.03);
        }
        let n = f64::from(size * size);
        let psnr = 10.0 * (255.0f64.powi(2) / (se / (3.0 * n))).log10();
        assert!(psnr > 30.0, "PSNR {psnr:.1} dB");
        for c in 0..3 {
            let mean = sum[c] / n;
            assert!((mean - f64::from(ar.mean(c))).abs() < 0.01, "channel {c}");
        }
        // The mean colour travels in the metadata.
        let mean: Vec<f32> = t
            .meta(META_MEAN)
            .unwrap()
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        assert!((mean[1] - ar.mean(1)).abs() < 1e-4);
        // AO darkens the grass between the stones.
        let lit = albedo_rough(
            &SourceMaps {
                ao: None,
                ..sample(size)
            },
            size,
        );
        assert!(lit.mean(1) > ar.mean(1));

        let nt = encode_normal(&normals(&maps, size), "sample");
        let nd = decode_level(&nt, 0);
        let src = normals(&maps, size);
        let mut worst = 0.0f32;
        for (i, px) in nd.chunks(4).enumerate() {
            let x = f32::from(px[0]) / 255.0 * 2.0 - 1.0;
            let y = f32::from(px[1]) / 255.0 * 2.0 - 1.0;
            worst = worst
                .max((x - src.data[i * 3]).abs())
                .max((y - src.data[i * 3 + 1]).abs());
        }
        assert!(worst < 0.08, "normal error {worst}");
        // The smallest levels are flat: the bumps average out.
        let last = decode_level(&nt, nt.levels.len() - 1);
        assert!((i32::from(last[0]) - 128).abs() < 12);
    }

    #[test]
    fn encoding_is_deterministic_and_survives_ktx2() {
        let maps = sample(32);
        let a = encode_albedo(&albedo_rough(&maps, 32), "x");
        let b = encode_albedo(&albedo_rough(&maps, 32), "x");
        assert_eq!(a, b);
        let back = crate::ktx2::read(&crate::ktx2::write(&a).unwrap()).unwrap();
        assert_eq!(back.levels, a.levels);
        assert_eq!(back.meta(META_ID), Some("x"));
    }
}
