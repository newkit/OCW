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
static CPU_IMAGE_BYTES: AtomicU64 = AtomicU64::new(0);
static CPU_MESH_BYTES: AtomicU64 = AtomicU64::new(0);
/// The biggest images kept on the CPU ("1280×1280×1 6.6 MB", …).
static CPU_IMAGES_TOP: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The biggest images the main world keeps (for `window.ocw.heap()`).
pub fn cpu_images_top() -> Vec<String> {
    CPU_IMAGES_TOP.lock().map(|v| v.clone()).unwrap_or_default()
}

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
    /// The main world's copies: image data and mesh arrays kept on the
    /// CPU (assets made with `RenderAssetUsages::MAIN_WORLD`, on the wasm
    /// heap; the iPhone's memory, v5.42).
    pub cpu_image_bytes: u64,
    pub cpu_mesh_bytes: u64,
}

impl GpuMemory {
    pub fn now() -> Self {
        Self {
            mesh_bytes: MESH_BYTES.load(Relaxed),
            meshes: MESH_COUNT.load(Relaxed),
            slab_bytes: SLAB_BYTES.load(Relaxed),
            image_bytes: IMAGE_BYTES.load(Relaxed),
            images: IMAGE_COUNT.load(Relaxed),
            cpu_image_bytes: CPU_IMAGE_BYTES.load(Relaxed),
            cpu_mesh_bytes: CPU_MESH_BYTES.load(Relaxed),
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

/// The renderer's allocations as the page reads them
/// (`window.ocw.gpuAllocated()`, the page log's memory lines; 10G).
pub fn allocated_json(a: &bevy::render::render_resource::GpuAllocated) -> String {
    serde_json::json!({
        "textures": a.textures,
        "texture_bytes": a.texture_bytes,
        "buffers": a.buffers,
        "buffer_bytes": a.buffer_bytes,
        "total": a.total_bytes(),
        "peak": a.peak_bytes,
    })
    .to_string()
}

/// One line for the log: "gpu allocated 61 MB (textures 48 MB in 120,
/// buffers 13 MB in 300; peak 70 MB)".
pub fn allocated_line(a: &bevy::render::render_resource::GpuAllocated) -> String {
    let mb = |b: u64| b as f64 / 1048576.0;
    format!(
        "gpu allocated {:.0} MB (textures {:.0} MB in {}, buffers {:.0} MB in {}; peak {:.0} MB)",
        mb(a.total_bytes()),
        mb(a.texture_bytes),
        a.textures,
        mb(a.buffer_bytes),
        a.buffers,
        mb(a.peak_bytes)
    )
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

/// The bytes of a mesh's arrays still in the main world (0 once they
/// went to the render world only).
pub fn cpu_mesh_bytes(m: &Mesh) -> usize {
    let attributes = m
        .try_attributes()
        .map(|a| a.map(|(_, v)| v.get_bytes().len()).sum::<usize>())
        .unwrap_or(0);
    let indices = match m.try_indices_option() {
        Ok(Some(bevy::mesh::Indices::U16(i))) => i.len() * 2,
        Ok(Some(bevy::mesh::Indices::U32(i))) => i.len() * 4,
        _ => 0,
    };
    attributes + indices
}

/// The main world's image data and mesh arrays (what stays on the wasm
/// heap after the upload).
fn sum_cpu_memory(
    mut frame: Local<u32>,
    images: Option<Res<Assets<Image>>>,
    meshes: Option<Res<Assets<Mesh>>>,
) {
    *frame += 1;
    let (Some(images), Some(meshes)) = (images, meshes) else {
        return;
    };
    if *frame % EVERY_FRAMES != 1 {
        return;
    }
    let mut kept: Vec<(usize, String)> = images
        .iter()
        .filter_map(|(_, i)| {
            let n = i.data.as_ref().map_or(0, Vec::len);
            let s = i.texture_descriptor.size;
            (n > 0).then(|| {
                (
                    n,
                    format!(
                        "{}×{}×{} {:.1} MB",
                        s.width,
                        s.height,
                        s.depth_or_array_layers,
                        n as f64 / 1048576.0
                    ),
                )
            })
        })
        .collect();
    let img: usize = kept.iter().map(|k| k.0).sum();
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    if let Ok(mut top) = CPU_IMAGES_TOP.lock() {
        *top = kept.into_iter().take(8).map(|k| k.1).collect();
    }
    let mesh: usize = meshes.iter().map(|(_, m)| cpu_mesh_bytes(m)).sum();
    CPU_IMAGE_BYTES.store(img as u64, Relaxed);
    CPU_MESH_BYTES.store(mesh as u64, Relaxed);
}

/// The page log's line of the renderer's allocations whenever their total
/// moved by [`LOG_STEP_BYTES`] (10G: the next phone log shows the GPU
/// memory before a kill).
pub const LOG_STEP_BYTES: u64 = 8 << 20;

fn log_allocated(mut last: Local<Option<u64>>) {
    let a = bevy::render::render_resource::gpu_allocated();
    let total = a.total_bytes();
    if total == 0 {
        return;
    }
    if last.is_none_or(|l| total.abs_diff(l) >= LOG_STEP_BYTES) {
        *last = Some(total);
        info!("{}", allocated_line(&a));
    }
}

/// Sums the GPU memory in the render world (no-op without one: the
/// native tests).
pub struct GpuMemoryPlugin;

impl Plugin for GpuMemoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, (sum_cpu_memory, log_allocated));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(Render, sum_gpu_memory.in_set(RenderSystems::Cleanup));
            // The heap's trace in the render world (`heap::mark`).
            render.add_systems(
                Render,
                (
                    (|| crate::heap::mark(6)).in_set(RenderSystems::PrepareAssets),
                    (|| crate::heap::mark(7)).in_set(RenderSystems::Queue),
                    (|| crate::heap::mark(8)).in_set(RenderSystems::Render),
                    (|| crate::heap::mark(9)).in_set(RenderSystems::Cleanup),
                ),
            );
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

    /// The vendored renderer's count of a texture (10G): mips, layers,
    /// samples; depth formats that cannot be copied.
    #[test]
    fn the_renderer_counts_texture_bytes() {
        use bevy::render::render_resource::gpu_allocated::texture_bytes;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let e = |width, height, depth_or_array_layers| Extent3d {
            width,
            height,
            depth_or_array_layers,
        };
        let d2 = TextureDimension::D2;
        // A 1024² sun shadow map in Depth32Float: 4 MB; 2048²: 16 MB.
        let depth = TextureFormat::Depth32Float;
        assert_eq!(texture_bytes(e(1024, 1024, 1), d2, depth, 1, 1), 4 << 20);
        assert_eq!(texture_bytes(e(2048, 2048, 1), d2, depth, 1, 1), 16 << 20);
        // Depth24Plus cannot be copied: 4 bytes a texel; MSAA 4×.
        let d24 = TextureFormat::Depth24Plus;
        assert_eq!(texture_bytes(e(10, 10, 1), d2, d24, 1, 4), 1600);
        // Mips and layers.
        let rgba = TextureFormat::Rgba8Unorm;
        assert_eq!(texture_bytes(e(4, 4, 2), d2, rgba, 3, 1), 2 * (64 + 16 + 4));
        // A 3D texture's depth halves with each level.
        let d3 = TextureDimension::D3;
        assert_eq!(texture_bytes(e(4, 4, 4), d3, rgba, 2, 1), 256 + 32);
    }

    /// The renderer's allocations for the page log (10G).
    #[test]
    fn the_allocations_read_as_json_and_a_line() {
        let a = bevy::render::render_resource::GpuAllocated {
            texture_bytes: 48 << 20,
            textures: 120,
            buffer_bytes: 13 << 20,
            buffers: 300,
            peak_bytes: 70 << 20,
        };
        assert_eq!(
            allocated_line(&a),
            "gpu allocated 61 MB (textures 48 MB in 120, buffers 13 MB in 300; peak 70 MB)"
        );
        let v: serde_json::Value = serde_json::from_str(&allocated_json(&a)).unwrap();
        assert_eq!(v["total"], 61u64 << 20);
        assert_eq!(v["textures"], 120);
    }
}
