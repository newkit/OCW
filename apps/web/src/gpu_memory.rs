//! What the browser app keeps on the GPU (spec §14 Phase 10 B2, the
//! memory per resident tile): the bytes of the meshes' vertex and index
//! data, of the slab buffers the mesh allocator keeps them in and of the
//! textures, summed in the render world every [`EVERY_FRAMES`] frames.
//! The status line, `world.info` and `window.ocw.gpuMemory()` read them.
//! GPU memory is not wasm memory (it lives in the browser's GPU process),
//! but a phone shares one memory for both.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::IndexFormat;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};

/// Frames between two sums (a sum walks every mesh and image).
pub const EVERY_FRAMES: u32 = 30;

static MESH_BYTES: AtomicU64 = AtomicU64::new(0);
static MESH_COUNT: AtomicU64 = AtomicU64::new(0);
static SLAB_BYTES: AtomicU64 = AtomicU64::new(0);
static IMAGE_BYTES: AtomicU64 = AtomicU64::new(0);
static IMAGE_COUNT: AtomicU64 = AtomicU64::new(0);

/// The last sums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuMemory {
    /// Vertex and index bytes of all meshes.
    pub mesh_bytes: u64,
    pub meshes: u64,
    /// The mesh allocator's buffers that hold them (with their free room).
    pub slab_bytes: u64,
    /// All textures with their mip levels.
    pub image_bytes: u64,
    pub images: u64,
}

impl GpuMemory {
    pub fn now() -> Self {
        Self {
            mesh_bytes: MESH_BYTES.load(Relaxed),
            meshes: MESH_COUNT.load(Relaxed),
            slab_bytes: SLAB_BYTES.load(Relaxed),
            image_bytes: IMAGE_BYTES.load(Relaxed),
            images: IMAGE_COUNT.load(Relaxed),
        }
    }

    /// The status line's part: "gpu meshes 120 MB in 2,345 (buffers
    /// 160 MB), textures 300 MB".
    pub fn summary(&self) -> String {
        format!(
            "gpu meshes {:.0} MB in {} (buffers {:.0} MB), textures {:.0} MB in {}",
            self.mesh_bytes as f64 / 1e6,
            self.meshes,
            self.slab_bytes as f64 / 1e6,
            self.image_bytes as f64 / 1e6,
            self.images
        )
    }
}

/// The bytes of a texture of `size` (width, height, layers) in a format
/// whose blocks of `block` texels take `block_bytes`, with `mips` levels.
pub fn texture_bytes(size: [u32; 3], block: (u32, u32), block_bytes: u32, mips: u32) -> u64 {
    (0..mips.max(1))
        .map(|l| {
            let w = (size[0] >> l).max(1).div_ceil(block.0);
            let h = (size[1] >> l).max(1).div_ceil(block.1);
            u64::from(w) * u64::from(h) * u64::from(size[2].max(1)) * u64::from(block_bytes)
        })
        .sum()
}

/// The vertex and index bytes of a mesh.
pub fn mesh_bytes(mesh: &RenderMesh) -> u64 {
    let stride = mesh.layout.0.layout().array_stride;
    let indices = match mesh.buffer_info {
        RenderMeshBufferInfo::Indexed {
            count,
            index_format,
        } => {
            u64::from(count)
                * match index_format {
                    IndexFormat::Uint16 => 2,
                    IndexFormat::Uint32 => 4,
                }
        }
        RenderMeshBufferInfo::NonIndexed => 0,
    };
    u64::from(mesh.vertex_count) * stride + indices
}

fn sum_gpu_memory(
    mut frame: Local<u32>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    allocator: Res<MeshAllocator>,
) {
    *frame += 1;
    if *frame % EVERY_FRAMES != 1 {
        return;
    }
    let (mut bytes, mut count) = (0, 0);
    let mut buffers = HashMap::new();
    for (id, m) in meshes.iter() {
        bytes += mesh_bytes(m);
        count += 1;
        for s in [
            allocator.mesh_vertex_slice(&id),
            allocator.mesh_index_slice(&id),
        ]
        .into_iter()
        .flatten()
        {
            buffers.insert(s.buffer.id(), s.buffer.size());
        }
    }
    MESH_BYTES.store(bytes, Relaxed);
    MESH_COUNT.store(count, Relaxed);
    SLAB_BYTES.store(buffers.values().sum(), Relaxed);
    let (mut bytes, mut count) = (0, 0);
    for (_, i) in images.iter() {
        let d = &i.texture_descriptor;
        let f = d.format;
        bytes += texture_bytes(
            [d.size.width, d.size.height, d.size.depth_or_array_layers],
            f.block_dimensions(),
            f.block_copy_size(None).unwrap_or(4),
            d.mip_level_count,
        );
        count += 1;
    }
    IMAGE_BYTES.store(bytes, Relaxed);
    IMAGE_COUNT.store(count, Relaxed);
}

/// Sums the GPU memory in the render world (no-op without one: the
/// native tests).
pub struct GpuMemoryPlugin;

impl Plugin for GpuMemoryPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(Render, sum_gpu_memory.in_set(RenderSystems::Cleanup));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texture sizes: RGBA8 with a full mip chain, BC3 blocks of 4 × 4.
    #[test]
    fn texture_bytes_count_mips_and_blocks() {
        assert_eq!(texture_bytes([4, 4, 1], (1, 1), 4, 1), 64);
        // 4×4 + 2×2 + 1×1 texels.
        assert_eq!(texture_bytes([4, 4, 1], (1, 1), 4, 3), 84);
        // BC3: 16 bytes per 4 × 4 block; a 2 × 2 level still takes a block.
        assert_eq!(texture_bytes([8, 8, 1], (4, 4), 16, 3), 64 + 16 + 16);
        assert_eq!(texture_bytes([4, 4, 6], (1, 1), 4, 1), 384);
        assert!(
            GpuMemory::default()
                .summary()
                .starts_with("gpu meshes 0 MB")
        );
    }
}
