//! Procedural splat texture layers (stand-ins for CC0 terrain textures) with
//! a CPU-built mip chain. Pure; turned into a Bevy texture array in main.rs.

/// Layer order matches the vertex weights: grass, forest floor, rock, verge.
pub const LAYERS: usize = 4;

fn hash(x: i64, y: i64, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Tileable value noise: the lattice wraps every `period` cells.
fn tile_noise(x: f32, y: f32, cell: f32, period: i64, seed: u64) -> f32 {
    let (fx, fy) = (x / cell, y / cell);
    let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let h = |x: i64, y: i64| hash(x.rem_euclid(period), y.rem_euclid(period), seed);
    let a = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * s(tx);
    let b = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * s(tx);
    a + (b - a) * s(ty)
}

/// RGBA8 (sRGB) pixels of one layer, `size` × `size`, tileable.
pub fn layer(index: usize, size: u32) -> Vec<u8> {
    let (base, var): ([f32; 3], [f32; 3]) = match index {
        0 => ([0.33, 0.52, 0.25], [0.10, 0.12, 0.06]), // grass
        1 => ([0.20, 0.31, 0.13], [0.08, 0.09, 0.05]), // forest floor
        2 => ([0.48, 0.46, 0.42], [0.14, 0.14, 0.13]), // rock
        _ => ([0.50, 0.47, 0.34], [0.10, 0.09, 0.07]), // verge / dirt
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let sz = size as f32;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            // Octaves whose periods divide the texture size, so it tiles.
            let n = tile_noise(fx, fy, sz / 4.0, 4, index as u64 * 10 + 1) * 0.5
                + tile_noise(fx, fy, sz / 16.0, 16, index as u64 * 10 + 2) * 0.3
                + tile_noise(fx, fy, sz / 64.0, 64, index as u64 * 10 + 3) * 0.2
                - 0.5;
            for c in 0..3 {
                out.push(((base[c] + var[c] * n * 2.0).clamp(0.0, 1.0) * 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

/// Number of mip levels down to 1 × 1.
pub fn mip_count(size: u32) -> u32 {
    32 - size.leading_zeros()
}

/// One mip level from the previous one (2 × 2 box filter, RGBA8).
pub fn downsample(src: &[u8], size: u32) -> Vec<u8> {
    let half = (size / 2).max(1);
    let mut out = Vec::with_capacity((half * half * 4) as usize);
    for y in 0..half {
        for x in 0..half {
            for c in 0..4 {
                let px = |dx: u32, dy: u32| {
                    let (sx, sy) = ((2 * x + dx).min(size - 1), (2 * y + dy).min(size - 1));
                    u32::from(src[((sy * size + sx) * 4 + c) as usize])
                };
                out.push(((px(0, 0) + px(1, 0) + px(0, 1) + px(1, 1) + 2) / 4) as u8);
            }
        }
    }
    out
}

/// All layers with all mips, layer-major (layer 0 mips 0..n, layer 1, ...),
/// the order Bevy's `Image` expects by default.
pub fn texture_array_data(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for l in 0..LAYERS {
        let mut level = layer(l, size);
        let mut s = size;
        loop {
            out.extend_from_slice(&level);
            if s == 1 {
                break;
            }
            level = downsample(&level, s);
            s /= 2;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_tile_seamlessly() {
        let size = 64;
        let l = layer(0, size);
        let px = |x: u32, y: u32| l[((y * size + x) * 4) as usize] as i32;
        // The noise at x = size equals x = 0 (lattice wraps), so the last
        // column is close to the first one.
        for y in 0..size {
            assert!((px(size - 1, y) - px(0, y)).abs() <= 12, "row {y}");
        }
        assert_eq!(l.len(), (size * size * 4) as usize);
        assert!(l.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn layers_differ() {
        let mean = |l: &[u8], c: usize| {
            l.chunks(4).map(|p| u32::from(p[c])).sum::<u32>() / (l.len() as u32 / 4)
        };
        let grass = layer(0, 32);
        let rock = layer(2, 32);
        assert!(mean(&grass, 1) > mean(&grass, 0)); // greenish
        assert!(mean(&rock, 0).abs_diff(mean(&rock, 1)) < 15); // greyish
    }

    #[test]
    fn mip_chain_sizes() {
        assert_eq!(mip_count(256), 9);
        assert_eq!(mip_count(1), 1);
        assert_eq!(
            downsample(
                &[
                    10, 20, 30, 255, 30, 40, 50, 255, 50, 60, 70, 255, 70, 80, 90, 255
                ],
                2
            ),
            vec![40, 50, 60, 255]
        );
        let per_layer: u32 = (0..mip_count(16)).map(|m| (16 >> m) * (16 >> m) * 4).sum();
        assert_eq!(
            texture_array_data(16).len() as u32,
            per_layer * LAYERS as u32
        );
    }
}
