//! OCW patch (spec 14 Phase 10, 10G *Bisecting a phone's crash*): the GPU
//! memory allocated through Bevy's [`Texture`](super::Texture) and
//! [`Buffer`](super::Buffer) wrappers, summed while they live – render
//! targets, depth and shadow maps, uniform and storage buffers, mesh slabs
//! and images alike (the asset sums of an app miss the targets and the
//! shadow maps). An estimate: the bytes of the texels and buffers as
//! created, without the driver's own padding, the canvas' buffers or
//! textures made straight from the `wgpu::Device`.

use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

static TEXTURE_BYTES: AtomicU64 = AtomicU64::new(0);
static TEXTURES: AtomicU64 = AtomicU64::new(0);
static BUFFER_BYTES: AtomicU64 = AtomicU64::new(0);
static BUFFERS: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

/// The live sums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuAllocated {
    pub texture_bytes: u64,
    pub textures: u64,
    pub buffer_bytes: u64,
    pub buffers: u64,
    /// The highest total so far.
    pub peak_bytes: u64,
}

impl GpuAllocated {
    pub fn total_bytes(&self) -> u64 {
        self.texture_bytes + self.buffer_bytes
    }
}

/// What is allocated now.
pub fn gpu_allocated() -> GpuAllocated {
    GpuAllocated {
        texture_bytes: TEXTURE_BYTES.load(Relaxed),
        textures: TEXTURES.load(Relaxed),
        buffer_bytes: BUFFER_BYTES.load(Relaxed),
        buffers: BUFFERS.load(Relaxed),
        peak_bytes: PEAK_BYTES.load(Relaxed),
    }
}

/// The bytes of one texel block of `format` (depth formats that cannot be
/// copied: their usual size).
pub fn block_bytes(format: wgpu::TextureFormat) -> u32 {
    use wgpu::TextureFormat as F;
    format.block_copy_size(None).unwrap_or(match format {
        F::Stencil8 => 1,
        F::Depth16Unorm => 2,
        F::Depth32FloatStencil8 => 8,
        _ => 4,
    })
}

/// The bytes of a texture: every mip level, layer and sample.
pub fn texture_bytes(
    size: wgpu::Extent3d,
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    mips: u32,
    samples: u32,
) -> u64 {
    let (bw, bh) = format.block_dimensions();
    let block = u64::from(block_bytes(format));
    (0..mips.max(1))
        .map(|l| {
            let w = u64::from((size.width >> l).max(1).div_ceil(bw));
            let h = u64::from((size.height >> l).max(1).div_ceil(bh));
            let d = match dimension {
                wgpu::TextureDimension::D3 => u64::from((size.depth_or_array_layers >> l).max(1)),
                _ => u64::from(size.depth_or_array_layers.max(1)),
            };
            w * h * d * block
        })
        .sum::<u64>()
        * u64::from(samples.max(1))
}

/// One allocation, counted while any clone of its wrapper lives.
#[derive(Debug)]
pub(crate) struct Live {
    bytes: u64,
    texture: bool,
}

impl Live {
    fn new(bytes: u64, texture: bool) -> Arc<Self> {
        let (sum, count) = if texture {
            (&TEXTURE_BYTES, &TEXTURES)
        } else {
            (&BUFFER_BYTES, &BUFFERS)
        };
        sum.fetch_add(bytes, Relaxed);
        count.fetch_add(1, Relaxed);
        let total = TEXTURE_BYTES.load(Relaxed) + BUFFER_BYTES.load(Relaxed);
        PEAK_BYTES.fetch_max(total, Relaxed);
        Arc::new(Self { bytes, texture })
    }

    pub(crate) fn texture(t: &wgpu::Texture) -> Arc<Self> {
        Self::new(
            texture_bytes(
                t.size(),
                t.dimension(),
                t.format(),
                t.mip_level_count(),
                t.sample_count(),
            ),
            true,
        )
    }

    pub(crate) fn buffer(b: &wgpu::Buffer) -> Arc<Self> {
        Self::new(b.size(), false)
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let (sum, count) = if self.texture {
            (&TEXTURE_BYTES, &TEXTURES)
        } else {
            (&BUFFER_BYTES, &BUFFERS)
        };
        sum.fetch_sub(self.bytes.min(sum.load(Relaxed)), Relaxed);
        count.fetch_sub(1.min(count.load(Relaxed)), Relaxed);
    }
}

// Tested in the app (`ocw-web`'s `gpu_memory::tests`): this crate's own
// tests need Bevy's dev-dependencies outside the workspace.
