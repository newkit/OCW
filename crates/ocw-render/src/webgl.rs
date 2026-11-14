//! What WebGL 2 needs that the other GPU APIs do not (spec §14 Phase 10
//! B7). Everything here is decided by the adapter at run time and does
//! nothing on Vulkan, Metal, DirectX or WebGPU, so the desktop and the
//! WebGPU build draw exactly as before.
//!
//! **Texture arrays without a texture.** The materials' `texture_2d_array`
//! slots (road, ground, houses, gear) are empty without the material
//! library; Bevy then binds its fallback image, a texture of **one**
//! layer. WebGL 2 decides a texture's view type when the texture is made –
//! one layer is a plain 2D texture (wgpu: "heuristics assumed that the
//! view dimension will be equal to `D2` rather than `D2Array`") – and the
//! array view of it fails validation, which stops the renderer. On WebGL 2
//! the empty slots get [`ArrayFallback`]: white like Bevy's fallback, but
//! two layers, so it is an array texture.
//!
//! **No cross-fading levels of detail.** A `VisibilityRange` with margins
//! fades (dithers) between levels of detail; the vertex shader then reads
//! Bevy's table of ranges, which WebGL 2 (no storage buffers) gets as a
//! uniform array of 64 entries – but Bevy's bind group layout announces
//! only 16 bytes for it, and the pipeline fails validation. On WebGL 2
//! every range is made abrupt ([`abrupt`]: each margin's middle): the
//! levels switch at once instead of fading.
//!
//! (The sky's atmosphere needs compute shaders: `sky::AtmosphereSupport`.)

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::RenderAdapterInfo;

use crate::building_material::BuildingMaterial;
use crate::gear::GearMaterial;
use crate::ground::TerrainMaterial;
use crate::road_material::RoadMaterial;

/// The two-layer white array for empty `texture_2d_array` slots; `None`
/// where the GPU API needs none (everything but WebGL 2) or before the
/// adapter is known.
#[derive(Resource, Debug, Clone, Default)]
pub struct ArrayFallback(pub Option<Handle<Image>>);

/// Whether this adapter is WebGL 2 (wgpu's GL backend in a browser; the
/// desktop's GL fallback has the same limits).
pub fn is_gl(info: &RenderAdapterInfo) -> bool {
    bevy::render::settings::Backends::from(info.backend) == bevy::render::settings::Backends::GL
}

/// The renderer runs on WebGL 2 (GL): set once the adapter is known.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OnWebGl(pub bool);

/// `range` without cross-fading: each margin collapsed to its middle.
pub fn abrupt(range: &VisibilityRange) -> VisibilityRange {
    // (Written so that an open end, f32::MAX, stays f32::MAX.)
    let mid = |r: &std::ops::Range<f32>| r.start + 0.5 * (r.end - r.start);
    let (start, end) = (mid(&range.start_margin), mid(&range.end_margin));
    VisibilityRange {
        start_margin: start..start,
        end_margin: end..end,
        use_aabb: range.use_aabb,
    }
}

/// On WebGL 2: every new or changed range becomes [`abrupt`].
fn abrupt_ranges(
    gl: Res<OnWebGl>,
    mut ranges: Query<&mut VisibilityRange, Changed<VisibilityRange>>,
) {
    if !gl.0 {
        return;
    }
    for mut r in &mut ranges {
        if !r.is_abrupt() {
            *r = abrupt(&r);
        }
    }
}

/// White, 1 × 1, two layers, viewed as an array (Bevy's fallback is white
/// too: an unset slot looks the same).
pub fn array_placeholder() -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 2,
        },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        label: Some("array fallback"),
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image
}

/// A material with `texture_2d_array` slots.
pub trait ArraySlots {
    fn array_slots(&mut self) -> [&mut Option<Handle<Image>>; 2];
    /// Whether a slot is empty.
    fn has_empty_array(&self) -> bool;
}

/// The materials' array slots are their extension's `albedo` and
/// `normal`.
macro_rules! array_slots {
    ($($m:ty),*) => {$(
        impl ArraySlots for $m {
            fn array_slots(&mut self) -> [&mut Option<Handle<Image>>; 2] {
                [&mut self.extension.albedo, &mut self.extension.normal]
            }
            fn has_empty_array(&self) -> bool {
                self.extension.albedo.is_none() || self.extension.normal.is_none()
            }
        }
    )*};
}

array_slots!(
    RoadMaterial,
    TerrainMaterial,
    BuildingMaterial,
    GearMaterial
);

/// Fills `slots`' empty ones with `fallback`; whether one was empty.
pub fn fill_slots(slots: [&mut Option<Handle<Image>>; 2], fallback: &Handle<Image>) -> bool {
    let mut filled = false;
    for slot in slots {
        if slot.is_none() {
            *slot = Some(fallback.clone());
            filled = true;
        }
    }
    filled
}

pub struct WebGlPlugin;

impl Plugin for WebGlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArrayFallback>()
            .init_resource::<OnWebGl>()
            .add_systems(
                PostUpdate,
                (
                    detect,
                    (
                        fill_empty_arrays::<RoadMaterial>,
                        fill_empty_arrays::<TerrainMaterial>,
                        fill_empty_arrays::<BuildingMaterial>,
                        fill_empty_arrays::<GearMaterial>,
                        abrupt_ranges.after(crate::quality::apply_quality),
                    ),
                )
                    .chain()
                    // This frame's new materials are filled before the
                    // render world extracts them.
                    .after(bevy::asset::AssetEventSystems),
            );
    }
}

fn detect(
    info: Option<Res<RenderAdapterInfo>>,
    mut gl: ResMut<OnWebGl>,
    mut fallback: ResMut<ArrayFallback>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let (Some(info), Some(mut images)) = (info, images) else {
        return;
    };
    if fallback.0.is_none() && is_gl(&info) {
        info!(
            "WebGL 2: empty texture arrays get a two-layer fallback, levels of detail switch without fading"
        );
        fallback.0 = Some(images.add(array_placeholder()));
        gl.0 = true;
    }
}

/// On WebGL 2: every material of type `M` made or changed since gets the
/// fallback in its empty array slots.
fn fill_empty_arrays<M: Asset + ArraySlots>(
    fallback: Res<ArrayFallback>,
    mut events: MessageReader<AssetEvent<M>>,
    assets: Option<ResMut<Assets<M>>>,
    mut first: Local<bool>,
) {
    let (Some(handle), Some(mut assets)) = (fallback.0.as_ref(), assets) else {
        events.clear();
        return;
    };
    let ids: Vec<AssetId<M>> = if !*first {
        // The ones made before the adapter was known.
        *first = true;
        events.clear();
        assets.ids().collect()
    } else {
        events
            .read()
            .filter_map(|e| match e {
                AssetEvent::Added { id } | AssetEvent::Modified { id } => Some(*id),
                _ => None,
            })
            .collect()
    };
    for id in ids {
        // Only touched (and so re-uploaded) when a slot is empty.
        let empty = assets.get(id).is_some_and(ArraySlots::has_empty_array);
        if empty && let Some(mut m) = assets.get_mut(id) {
            fill_slots(m.array_slots(), handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_a_white_two_layer_array() {
        let i = array_placeholder();
        assert_eq!(i.texture_descriptor.size.depth_or_array_layers, 2);
        assert_eq!(
            i.texture_view_descriptor.as_ref().unwrap().dimension,
            Some(TextureViewDimension::D2Array)
        );
        assert!(i.data.as_ref().unwrap().iter().all(|&b| b == 255));
        assert_eq!(i.data.as_ref().unwrap().len(), 8);
    }

    #[test]
    fn only_empty_slots_are_filled() {
        let mut images = Assets::<Image>::default();
        let fallback = images.add(array_placeholder());
        let own = images.add(Image::default());
        let (mut a, mut b) = (None, Some(own.clone()));
        assert!(fill_slots([&mut a, &mut b], &fallback));
        assert_eq!(a, Some(fallback.clone()));
        assert_eq!(b, Some(own));
        assert!(!fill_slots([&mut a, &mut b], &fallback));
    }

    /// The ranges without fading switch in the middle of each margin; an
    /// open end stays open.
    #[test]
    fn ranges_become_abrupt() {
        let [near, mid, far] = crate::houses::lod_ranges();
        let n = abrupt(&near);
        assert!(n.is_abrupt());
        let [a, b] = crate::houses::NEAR_END_M;
        assert_eq!(n.end_margin.start, 0.5 * (a + b));
        assert!(abrupt(&mid).is_abrupt());
        let f = abrupt(&far);
        assert!(f == far, "already abrupt");
        assert_eq!(f.end_margin.end, f32::MAX);
    }

    /// Off WebGL 2 (no adapter in the test, as on a GPU that needs none)
    /// nothing is made and no material is touched.
    #[test]
    fn nothing_happens_without_webgl() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<RoadMaterial>()
            .init_asset::<TerrainMaterial>()
            .init_asset::<BuildingMaterial>()
            .init_asset::<GearMaterial>()
            .add_plugins(WebGlPlugin);
        app.update();
        assert!(app.world().resource::<ArrayFallback>().0.is_none());
    }
}
