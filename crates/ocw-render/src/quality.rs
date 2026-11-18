//! Render quality presets (spec §14 Phase 10 B7): Low, Medium and High,
//! one set of numbers per level for what costs the GPU most – the sun's
//! shadow maps and cascades, MSAA, the view distance, how far trees are
//! drawn, the houses' levels of detail, the weather's particles and (in
//! the browser) the pixel ratio of the canvas, and how much road damage
//! is drawn (`crate::road_material::sync_damage`).
//!
//! **High is what the renderer always drew** (the desktop's look, the
//! WebGPU build's): with High nothing changes. Medium and Low are cheaper
//! for weaker GPUs, WebGL 2 and phones. Which level runs is the app's
//! choice (`display.quality`, `auto` by GPU API and device:
//! `ocw_client::quality`); the renderer only applies [`RenderQuality`].
//!
//! What a level changes applies at once ([`apply_quality`]): the shadow
//! map and cascades, MSAA, particles, the view distance, and the
//! visibility ranges of the trees and houses already loaded.

use bevy::camera::visibility::VisibilityRange;
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;
use bevy::render::view::Msaa;

use crate::camera::FollowCamera;

/// A quality level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum QualityLevel {
    Low,
    Medium,
    /// The renderer's own look (the default).
    #[default]
    High,
}

impl QualityLevel {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == s.trim())
    }
}

impl std::fmt::Display for QualityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Shadow cascades per light the renderer can draw: Bevy's limit of this
/// build (1 on WebGL 2, 4 elsewhere).
pub const MAX_CASCADES: usize = bevy::pbr::MAX_CASCADES_PER_LIGHT;

/// The numbers of a level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityPreset {
    pub level: QualityLevel,
    /// Width and height of each shadow cascade's map (texels).
    pub shadow_map_px: usize,
    /// Shadow cascades of the sun (at most [`MAX_CASCADES`] are drawn).
    pub shadow_cascades: usize,
    /// How far the sun's shadows reach (m).
    pub shadow_distance_m: f32,
    /// The first cascade's far bound (m).
    pub first_cascade_m: f32,
    /// MSAA samples (1: off).
    pub msaa_samples: u32,
    /// The view distance (m): the far plane, with fog before it; `None`:
    /// as far as the world reaches.
    pub view_m: Option<f32>,
    /// How far a tile's trees are drawn (m from the tile's centre);
    /// beyond, the ground's forest colour stands in. `None`: while the
    /// tile is loaded.
    pub tree_view_m: Option<f32>,
    /// The houses' level-of-detail distances, scaled (1: as built).
    pub house_lod_scale: f32,
    /// The share of the weather's particle budget.
    pub particle_share: f32,
    /// The canvas' largest pixel ratio (browser only; `None`: the
    /// device's own). A phone at 3 device pixels per CSS pixel draws a
    /// ninth of the pixels at 1.
    pub max_pixel_ratio: Option<f32>,
    /// The share of the road damage drawn (cracks, patches, potholes;
    /// `ocw_world::road_wear::wear_density`); above 0.6 with the alligator
    /// crack pattern.
    pub road_damage_share: f32,
    /// How far the road damage is drawn (m).
    pub road_damage_m: f32,
}

impl QualityPreset {
    /// The numbers of `level`.
    pub const fn of(level: QualityLevel) -> Self {
        match level {
            // The look before the presets (world.rs, camera.rs).
            QualityLevel::High => Self {
                level,
                shadow_map_px: 4096,
                shadow_cascades: 4,
                shadow_distance_m: 250.0,
                first_cascade_m: 20.0,
                msaa_samples: 4,
                view_m: None,
                tree_view_m: None,
                house_lod_scale: 1.0,
                particle_share: 1.0,
                max_pixel_ratio: None,
                road_damage_share: 1.0,
                road_damage_m: 160.0,
            },
            QualityLevel::Medium => Self {
                level,
                shadow_map_px: 2048,
                shadow_cascades: 3,
                shadow_distance_m: 180.0,
                first_cascade_m: 18.0,
                msaa_samples: 4,
                view_m: Some(30_000.0),
                tree_view_m: Some(3_000.0),
                house_lod_scale: 0.75,
                particle_share: 0.5,
                max_pixel_ratio: Some(2.0),
                road_damage_share: 0.8,
                road_damage_m: 110.0,
            },
            // WebGL 2 and phones: one cascade (WebGL 2 has no more) over
            // the rider's surroundings, no MSAA.
            QualityLevel::Low => Self {
                level,
                shadow_map_px: 2048,
                shadow_cascades: 1,
                shadow_distance_m: 70.0,
                first_cascade_m: 70.0,
                msaa_samples: 1,
                view_m: Some(8_000.0),
                tree_view_m: Some(1_200.0),
                house_lod_scale: 0.5,
                particle_share: 0.25,
                max_pixel_ratio: Some(1.0),
                road_damage_share: 0.55,
                road_damage_m: 60.0,
            },
        }
    }

    /// The cascades this build draws: at most [`MAX_CASCADES`]. With fewer
    /// than asked (High on WebGL 2) the shadows reach only as far as the
    /// one cascade of Low, so they stay as sharp near the rider.
    pub fn cascades_drawn(&self) -> (usize, f32) {
        if self.shadow_cascades <= MAX_CASCADES {
            (self.shadow_cascades, self.shadow_distance_m)
        } else {
            let low = Self::of(QualityLevel::Low);
            (
                MAX_CASCADES,
                self.shadow_distance_m.min(low.shadow_distance_m),
            )
        }
    }

    /// The sun's cascade configuration.
    pub fn cascades(&self) -> CascadeShadowConfig {
        let (n, distance) = self.cascades_drawn();
        CascadeShadowConfigBuilder {
            num_cascades: n,
            minimum_distance: 0.1,
            maximum_distance: distance,
            first_cascade_far_bound: if n > 1 {
                self.first_cascade_m
            } else {
                distance
            },
            // Generous overlap: softer cascade transitions.
            overlap_proportion: 0.3,
        }
        .build()
    }

    pub fn msaa(&self) -> Msaa {
        match self.msaa_samples {
            0 | 1 => Msaa::Off,
            2 => Msaa::Sample2,
            8 => Msaa::Sample8,
            _ => Msaa::Sample4,
        }
    }

    /// The weather's particle budget.
    pub fn particles(&self) -> u32 {
        (ocw_weather::particles::BUDGET as f32 * self.particle_share).round() as u32
    }

    /// The far plane and the fog for a view that would reach `far_m`
    /// with this fog: the preset's view distance, fogged out before it.
    pub fn limit_view(
        &self,
        far_m: f32,
        falloff: bevy::pbr::FogFalloff,
    ) -> (f32, bevy::pbr::FogFalloff) {
        limit_view_to(self.view_m, far_m, falloff)
    }

    /// The preset with its view distance set to `view_m` (`None`:
    /// unlimited; the render distance of spec §14 Phase 10, 10G): the far
    /// plane and the fog follow it both ways; the trees' range and the
    /// houses' levels of detail only come down with a shorter distance
    /// ([`tree_view_for`], [`house_lod_for`]), never above the preset's
    /// own. At the preset's own distance nothing changes (Low at 8 km,
    /// Medium at 30 km, High unlimited).
    pub fn with_view_distance(&self, view_m: Option<f32>) -> Self {
        let tree_view_m = match (self.tree_view_m, tree_view_for(view_m)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Self {
            view_m,
            tree_view_m,
            house_lod_scale: self.house_lod_scale.min(house_lod_for(view_m)),
            ..*self
        }
    }

    /// The preset in one line (the console's `quality` and `gpu`).
    pub fn summary(&self) -> String {
        let (n, distance) = self.cascades_drawn();
        let km = |m: Option<f32>| match m {
            Some(m) => format!("{:.1} km", m / 1000.0),
            None => "full".into(),
        };
        format!(
            "shadows {px}² × {n} over {distance:.0} m, MSAA {msaa}, view {view}, trees {trees}, house LOD × {lod:.2}, particles {p}, pixel ratio {ratio}",
            px = self.shadow_map_px,
            msaa = match self.msaa_samples {
                0 | 1 => "off".to_string(),
                s => format!("{s}×"),
            },
            view = km(self.view_m),
            trees = km(self.tree_view_m),
            lod = self.house_lod_scale,
            p = self.particles(),
            ratio = match self.max_pixel_ratio {
                Some(r) => format!("≤ {r}"),
                None => "native".into(),
            },
        )
    }
}

impl Default for QualityPreset {
    fn default() -> Self {
        Self::of(QualityLevel::High)
    }
}

/// The least render distance of the slider (m).
pub const MIN_VIEW_M: f32 = 1_000.0;
/// Trees are drawn to this share of the render distance …
pub const TREE_VIEW_SHARE: f32 = 0.15;
/// … but at least this far (m) …
pub const MIN_TREE_VIEW_M: f32 = 800.0;
/// … in steps of this many metres (the ranges are not rebuilt for every
/// small step of the dynamic mode).
pub const TREE_VIEW_STEP_M: f32 = 250.0;
/// The houses' LOD scale is [`MIN_HOUSE_LOD`] up to this render distance
/// (m) …
pub const HOUSE_LOD_NEAR_M: f32 = 2_000.0;
/// … and 1 (as built) from this one on, linear between.
pub const HOUSE_LOD_FULL_M: f32 = 20_000.0;
pub const MIN_HOUSE_LOD: f32 = 0.5;
/// Below this render distance the renderer meshes only the 3 × 3 tiles
/// around the rider (else [`crate::package::STREAM_RADIUS`]).
pub const NEAR_STREAM_VIEW_M: f32 = 2_500.0;

/// How far trees are drawn at a render distance (`None`: while loaded):
/// 15 % of it, at least 800 m, rounded up to 250 m steps (so at least
/// Low's 1.2 km at its 8 km and Medium's 3 km at its 30 km: with the
/// preset's own as the cap, nothing changes there).
pub fn tree_view_for(view_m: Option<f32>) -> Option<f32> {
    view_m.map(|v| {
        let m = (TREE_VIEW_SHARE * v).max(MIN_TREE_VIEW_M);
        (m / TREE_VIEW_STEP_M - 1e-4).ceil().max(1.0) * TREE_VIEW_STEP_M
    })
}

/// The houses' LOD scale at a render distance: 0.5 up to 2 km, 1 from
/// 20 km on (and unlimited), linear between, in steps of 0.05.
pub fn house_lod_for(view_m: Option<f32>) -> f32 {
    let Some(v) = view_m else {
        return 1.0;
    };
    let t = ((v - HOUSE_LOD_NEAR_M) / (HOUSE_LOD_FULL_M - HOUSE_LOD_NEAR_M)).clamp(0.0, 1.0);
    let s = MIN_HOUSE_LOD + (1.0 - MIN_HOUSE_LOD) * t;
    ((s * 20.0 + 1e-4).floor() / 20.0).clamp(MIN_HOUSE_LOD, 1.0)
}

/// The ring of near tiles the renderer meshes around the rider at a
/// render distance: 1 (3 × 3 tiles) below 2.5 km, else
/// [`crate::package::STREAM_RADIUS`] (5 × 5; never more than before).
pub fn stream_radius_for(view_m: Option<f32>) -> i32 {
    match view_m {
        Some(v) if v < NEAR_STREAM_VIEW_M => 1,
        _ => crate::package::STREAM_RADIUS,
    }
}

/// Up to this view distance the fog has the atmosphere's horizon colour
/// (distant terrain fades into the sky) …
pub const HORIZON_FOG_FULL_M: f32 = 30_000.0;
/// … from this one on (and unlimited) the light model's, the look of High
/// before the render distance; linear between.
pub const HORIZON_FOG_NONE_M: f32 = 60_000.0;

/// How much of the fog's colour is the atmosphere's horizon at a view
/// distance (0 … 1; `None`: unlimited → 0): a limited view fogs whole
/// mountains out, and they must fade into the sky, not stand in front of
/// it as flat shapes in a darker colour (spec §14 Phase 10, 10G). A
/// weather fog (`weather_vis_m`) nearer than the view distance keeps the
/// weather's colour (none at or below the distance, all from twice it).
pub fn horizon_fog_share(view_m: Option<f32>, weather_vis_m: Option<f32>) -> f32 {
    let Some(v) = view_m else {
        return 0.0;
    };
    let by_view =
        ((HORIZON_FOG_NONE_M - v) / (HORIZON_FOG_NONE_M - HORIZON_FOG_FULL_M)).clamp(0.0, 1.0);
    let by_weather = weather_vis_m.map_or(1.0, |w| (w / v.max(1.0) - 1.0).clamp(0.0, 1.0));
    by_view * by_weather
}

/// The atmosphere's own haze (its aerial perspective) is added over the
/// fogged terrain, and the farther the fully fogged band lies the more:
/// the horizon colour is scaled down (linear light) from 1 up to
/// [`HAZE_NEAR_M`] to [`HAZE_SCALE_FAR`] from [`HAZE_FAR_M`] on (measured:
/// the fogged band at 8 – 12 km from the Schauinsland was about 0.05
/// brighter than the sky above it, at 2 km in Las Cañadas level with it).
pub const HAZE_NEAR_M: f32 = 3_000.0;
pub const HAZE_FAR_M: f32 = 10_000.0;
pub const HAZE_SCALE_FAR: f32 = 0.85;

/// [`HAZE_SCALE_FAR`]'s ramp at a view distance (`None`: unlimited).
pub fn horizon_fog_scale(view_m: Option<f32>) -> f32 {
    let v = view_m.unwrap_or(f32::INFINITY);
    let t = ((v - HAZE_NEAR_M) / (HAZE_FAR_M - HAZE_NEAR_M)).clamp(0.0, 1.0);
    1.0 + (HAZE_SCALE_FAR - 1.0) * t
}

/// Seconds the fog takes to follow a new render distance (95 % of the
/// way).
pub const FOG_EASE_S: f32 = 2.0;

/// The view distance the fog shows while it eases from `shown` to
/// `target` (`None`: unlimited) over [`FOG_EASE_S`]: exponentially, so a
/// step of the dynamic mode never jumps. Unlimited eases towards
/// `unlimited_m` (the far terrain's reach) and is reached there.
pub fn ease_view(
    shown: Option<f32>,
    target: Option<f32>,
    dt_s: f32,
    unlimited_m: f32,
) -> Option<f32> {
    let from = shown.unwrap_or(unlimited_m);
    let to = target.unwrap_or(unlimited_m);
    // 95 % within FOG_EASE_S: e^-3 ≈ 0.05.
    let k = 1.0 - (-3.0 * dt_s.max(0.0) / FOG_EASE_S).exp();
    let next = from + (to - from) * k;
    if (next - to).abs() <= 0.002 * to.max(1.0) {
        target
    } else {
        Some(next)
    }
}

/// [`QualityPreset::limit_view`] for the view distance `view_m` (`None`:
/// unlimited): the far plane at 1.5 × it, with an exponential fog whose
/// visibility is there – unless the given fog (the weather's) is nearer.
pub fn limit_view_to(
    view_m: Option<f32>,
    far_m: f32,
    falloff: bevy::pbr::FogFalloff,
) -> (f32, bevy::pbr::FogFalloff) {
    match view_m {
        Some(v) if v < far_m => {
            let reach = crate::far_terrain::fog_reach(&falloff);
            // Exponential fog with its visibility at `v`: 5 % contrast
            // there, about 1 % at the far plane.
            let fog = if v < reach {
                bevy::pbr::FogFalloff::from_visibility(v)
            } else {
                falloff
            };
            ((1.5 * v).min(far_m), fog)
        }
        _ => (far_m, falloff),
    }
}

/// The quality the renderer draws with (the app sets it; High by
/// default).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderQuality(pub QualityPreset);

/// A house mesh of a tile with its level of detail (0 near, 1 mid, 2
/// far; `houses::lod_ranges`).
#[derive(Component, Debug, Clone, Copy)]
pub struct HouseLod(pub u8);

/// A tile's far tree mesh (its draw distance follows the preset).
#[derive(Component, Debug, Clone, Copy)]
pub struct TileTrees;

/// No end to a range (drawn while the tile is loaded).
const ENDLESS_M: f32 = 1.0e7;

/// The visibility range of a chunk's far trees (spec §14 Phase 8 T8): from
/// where the near models fade out (`vegetation::NEAR_FADE_M`) to `end` m
/// (`None`: while the tile is loaded), faded out (dithered) over the last
/// 15 %.
pub fn far_tree_range(end: Option<f32>) -> VisibilityRange {
    let end = end.unwrap_or(ENDLESS_M);
    VisibilityRange {
        start_margin: crate::vegetation::NEAR_FADE_M,
        end_margin: 0.85 * end..end,
        use_aabb: true,
    }
}

/// The visibility range of a chunk's near plants (the detailed models,
/// vines, field stands): up to `vegetation::NEAR_FADE_M`.
pub fn near_tree_range() -> VisibilityRange {
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: crate::vegetation::NEAR_FADE_M,
        use_aabb: true,
    }
}

/// The visibility range of a detail chunk's plants (banana plants, vine
/// rows): up to `vegetation::DETAIL_FADE_M`.
pub fn detail_plant_range() -> VisibilityRange {
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: crate::vegetation::DETAIL_FADE_M,
        use_aabb: true,
    }
}

/// The visibility range of a tile's far trees for this preset.
pub fn tree_range(preset: &QualityPreset) -> VisibilityRange {
    far_tree_range(preset.tree_view_m)
}

pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderQuality>()
            .add_systems(PostUpdate, apply_quality);
    }
}

/// Applies the preset: on a change everything, and to what was spawned
/// since (new tiles' trees and houses, a new camera or sun) its part.
/// What a level leaves as it is (High) is never touched, so commands
/// like `weather.fx particles N` keep their value until the level
/// changes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn apply_quality(
    mut commands: Commands,
    quality: Res<RenderQuality>,
    mut last: Local<Option<QualityPreset>>,
    mut shadow_map: Option<ResMut<DirectionalLightShadowMap>>,
    mut fx: Option<ResMut<crate::weather::WeatherFx>>,
    mut suns: Query<&mut CascadeShadowConfig, With<crate::sky::Sun>>,
    cameras: Query<(Entity, Option<&Msaa>, Ref<FollowCamera>)>,
    mut houses: Query<(Ref<HouseLod>, &mut VisibilityRange)>,
    trees: Query<(Entity, Ref<TileTrees>, Has<VisibilityRange>)>,
) {
    let q = quality.0;
    let before = last.unwrap_or_default();
    let changed = *last != Some(q);
    *last = Some(q);
    if changed && q.shadow_map_px != before.shadow_map_px {
        if let Some(map) = shadow_map.as_deref_mut() {
            map.size = q.shadow_map_px;
        } else {
            commands.insert_resource(DirectionalLightShadowMap {
                size: q.shadow_map_px,
            });
        }
    }
    if changed
        && q.particle_share != before.particle_share
        && let Some(fx) = fx.as_deref_mut()
    {
        fx.particles = q.particles();
    }
    let new = q.cascades();
    for mut config in &mut suns {
        if (changed || config.is_added()) && !same_cascades(&config, &new) {
            *config = new.clone();
        }
    }
    let msaa = q.msaa();
    for (e, has, seen) in &cameras {
        if (changed || seen.is_added()) && has != Some(&msaa) {
            commands.entity(e).insert(msaa);
        }
    }
    let ranges = crate::houses::lod_ranges_scaled(q.house_lod_scale);
    for (lod, mut range) in &mut houses {
        if changed || lod.is_added() {
            let want = &ranges[usize::from(lod.0.min(2))];
            if *range != *want {
                *range = want.clone();
            }
        }
    }
    let tree = tree_range(&q);
    for (e, seen, _) in &trees {
        if changed || seen.is_added() {
            commands.entity(e).insert(tree.clone());
        }
    }
}

fn same_cascades(a: &CascadeShadowConfig, b: &CascadeShadowConfig) -> bool {
    a.bounds == b.bounds
        && a.overlap_proportion == b.overlap_proportion
        && a.minimum_distance == b.minimum_distance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same_range(a: Option<&VisibilityRange>, b: Option<&VisibilityRange>) -> bool {
        a == b
    }

    /// High is the look before the presets: the numbers world.rs and the
    /// camera had.
    #[test]
    fn high_is_the_renderers_own_look() {
        let h = QualityPreset::of(QualityLevel::High);
        assert_eq!(h, QualityPreset::default());
        assert_eq!((h.shadow_map_px, h.shadow_cascades), (4096, 4));
        assert_eq!(h.msaa(), Msaa::Sample4);
        assert_eq!(h.msaa(), Msaa::default(), "Bevy's camera default");
        assert_eq!(h.particles(), ocw_weather::particles::BUDGET);
        assert_eq!(
            (h.view_m, h.tree_view_m, h.max_pixel_ratio),
            (None, None, None)
        );
        assert_eq!(h.house_lod_scale, 1.0);
        assert!(crate::houses::lod_ranges_scaled(1.0) == crate::houses::lod_ranges());
        let c = h.cascades();
        assert_eq!(c.bounds.len(), 4);
        assert!((c.bounds[3] - 250.0).abs() < 1e-3);
        // High: the far trees from the near models' fade on, without an end.
        let r = tree_range(&h);
        assert_eq!(r.start_margin, crate::vegetation::NEAR_FADE_M);
        assert!(r.end_margin.start > 1.0e6);
    }

    /// Each level is cheaper than the one above in every number.
    #[test]
    fn lower_levels_cost_less() {
        for pair in QualityLevel::ALL.windows(2) {
            let (lo, hi) = (QualityPreset::of(pair[0]), QualityPreset::of(pair[1]));
            assert!(lo.shadow_map_px <= hi.shadow_map_px);
            assert!(lo.shadow_cascades <= hi.shadow_cascades);
            assert!(lo.shadow_distance_m <= hi.shadow_distance_m);
            assert!(lo.msaa_samples <= hi.msaa_samples);
            assert!(lo.view_m.unwrap_or(f32::MAX) <= hi.view_m.unwrap_or(f32::MAX));
            assert!(lo.tree_view_m.unwrap_or(f32::MAX) <= hi.tree_view_m.unwrap_or(f32::MAX));
            assert!(lo.house_lod_scale <= hi.house_lod_scale);
            assert!(lo.particles() <= hi.particles());
            assert!(
                lo.max_pixel_ratio.unwrap_or(f32::MAX) <= hi.max_pixel_ratio.unwrap_or(f32::MAX)
            );
        }
        let low = QualityPreset::of(QualityLevel::Low);
        assert_eq!(low.msaa(), Msaa::Off);
        assert_eq!(low.cascades().bounds.len(), 1, "one cascade: WebGL 2");
        assert_eq!(low.particles(), 10_000);
    }

    /// The render distance (10G) → fog, trees, houses, tiles: monotone,
    /// and at each preset's own distance the preset's own numbers.
    #[test]
    fn the_render_distance_maps_to_fog_trees_houses_and_tiles() {
        for l in QualityLevel::ALL {
            let p = QualityPreset::of(l);
            assert_eq!(p.with_view_distance(p.view_m), p, "{l}");
        }
        let km = |k: f32| Some(k * 1000.0);
        let mut last: Option<(f32, f32, i32)> = None;
        for k in 1..=60 {
            let d = km(k as f32);
            let t = tree_view_for(d).unwrap();
            let h = house_lod_for(d);
            let r = stream_radius_for(d);
            assert!(
                t >= MIN_TREE_VIEW_M && t % TREE_VIEW_STEP_M == 0.0,
                "{k}: {t}"
            );
            assert!((MIN_HOUSE_LOD..=1.0).contains(&h), "{k}: {h}");
            if let Some((t0, h0, r0)) = last {
                assert!(t >= t0 && h >= h0 && r >= r0, "monotone at {k} km");
            }
            last = Some((t, h, r));
        }
        assert_eq!(tree_view_for(None), None);
        assert_eq!(house_lod_for(None), 1.0);
        assert_eq!(house_lod_for(km(2.0)), 0.5);
        assert_eq!(house_lod_for(km(20.0)), 1.0);
        assert_eq!(stream_radius_for(km(1.0)), 1);
        assert_eq!(stream_radius_for(km(2.0)), 1);
        assert_eq!(stream_radius_for(km(3.0)), crate::package::STREAM_RADIUS);
        assert_eq!(stream_radius_for(None), crate::package::STREAM_RADIUS);

        // High at 12 km: the far plane at 18 km, trees to 2 km, houses
        // × 0.75 – less than High, never more than the preset.
        let high = QualityPreset::of(QualityLevel::High).with_view_distance(km(12.0));
        assert_eq!(high.view_m, km(12.0));
        assert_eq!(high.tree_view_m, Some(2_000.0));
        assert_eq!(high.house_lod_scale, 0.75);
        assert_eq!(high.shadow_map_px, 4096, "the rest of the preset stays");
        // Low at 60 km: the far plane goes out, trees and houses stay Low's.
        let low = QualityPreset::of(QualityLevel::Low);
        let far = low.with_view_distance(km(60.0));
        assert_eq!(far.view_m, km(60.0));
        assert_eq!(far.tree_view_m, low.tree_view_m);
        assert_eq!(far.house_lod_scale, low.house_lod_scale);
        // Unlimited on Medium: the preset's trees and houses.
        let m = QualityPreset::of(QualityLevel::Medium).with_view_distance(None);
        assert_eq!((m.view_m, m.tree_view_m), (None, Some(3_000.0)));
        // The fog follows: a 2 km distance fogs out at 2 km, far plane 3 km.
        let clear = bevy::pbr::FogFalloff::Linear {
            start: 50_000.0,
            end: 100_000.0,
        };
        let (far_m, fog) = limit_view_to(km(2.0), 120_000.0, clear.clone());
        assert_eq!(far_m, 3_000.0);
        assert!((crate::far_terrain::fog_reach(&fog) - 2_000.0).abs() < 100.0);
        let (far_m, _) = limit_view_to(None, 120_000.0, clear);
        assert_eq!(far_m, 120_000.0);
    }

    /// Regression (10G, the flat silhouette): the fog takes the
    /// atmosphere's horizon colour where the view is limited – fully up to
    /// 30 km (Medium, Low, short distances), not at all unlimited (High's
    /// look unchanged) – and the mix is in linear light.
    #[test]
    fn the_fog_colour_follows_the_view_limit() {
        assert_eq!(horizon_fog_share(None, None), 0.0);
        for km in [1.0, 2.0, 8.0, 12.0, 30.0] {
            assert_eq!(horizon_fog_share(Some(km * 1000.0), None), 1.0, "{km}");
        }
        assert!((horizon_fog_share(Some(45_000.0), None) - 0.5).abs() < 1e-6);
        assert_eq!(horizon_fog_share(Some(60_000.0), None), 0.0);
        assert_eq!(horizon_fog_share(Some(200_000.0), None), 0.0);
        // A weather fog nearer than the view: the weather's colour.
        assert_eq!(horizon_fog_share(Some(12_000.0), Some(600.0)), 0.0);
        assert_eq!(horizon_fog_share(Some(12_000.0), Some(12_000.0)), 0.0);
        assert!((horizon_fog_share(Some(12_000.0), Some(18_000.0)) - 0.5).abs() < 1e-6);
        assert_eq!(horizon_fog_share(Some(12_000.0), Some(50_000.0)), 1.0);
        // The haze scale: 1 at short distances, 0.85 from 10 km.
        assert_eq!(horizon_fog_scale(Some(2_000.0)), 1.0);
        assert_eq!(horizon_fog_scale(Some(10_000.0)), HAZE_SCALE_FAR);
        assert_eq!(horizon_fog_scale(None), HAZE_SCALE_FAR);
        let mid = horizon_fog_scale(Some(6_500.0));
        assert!(mid < 1.0 && mid > HAZE_SCALE_FAR, "{mid}");
        let mut last = 1.0;
        for km in 1..=60 {
            let s = horizon_fog_share(Some(km as f32 * 1000.0), None);
            assert!(s <= last, "falls with the distance");
            last = s;
        }
        use crate::sky::mix_linear;
        let a = [0.62, 0.78, 0.95];
        let b = [1.2, 1.3, 1.4];
        let same = |x: [f32; 3], y: [f32; 3]| x.iter().zip(y).all(|(p, q)| (p - q).abs() < 1e-5);
        assert!(same(mix_linear(a, b, 0.0), a));
        assert!(same(mix_linear(a, b, 1.0), b));
        // Halfway in linear light is brighter than halfway in sRGB.
        let mid = mix_linear([0.0; 3], [1.0; 3], 0.5);
        assert!((mid[0] - 0.7354).abs() < 1e-3, "{mid:?}");
        assert!(same(mix_linear(a, b, 7.0), b), "clamped");
    }

    /// The fog eases to a new distance over about 2 s (no jump), also to
    /// and from unlimited.
    #[test]
    fn the_fog_eases_to_a_new_distance() {
        let u = 120_000.0;
        let mut shown = Some(10_000.0);
        let target = Some(8_500.0);
        let first = ease_view(shown, target, 1.0 / 60.0, u).unwrap();
        assert!(first < 10_000.0 && first > 9_900.0, "{first}");
        for _ in 0..120 {
            shown = ease_view(shown, target, 1.0 / 60.0, u);
        }
        let s = shown.unwrap();
        assert!((s - 8_500.0).abs() <= 0.06 * 1_500.0, "95 % in 2 s: {s}");
        for _ in 0..600 {
            shown = ease_view(shown, target, 1.0 / 60.0, u);
        }
        assert_eq!(shown, target, "arrives exactly");
        // To unlimited and back.
        let mut shown = Some(60_000.0);
        for _ in 0..1200 {
            shown = ease_view(shown, None, 1.0 / 60.0, u);
        }
        assert_eq!(shown, None);
        let next = ease_view(None, Some(60_000.0), 1.0 / 60.0, u).unwrap();
        assert!(next > 115_000.0, "{next}");
        // A pause (dt 0) changes nothing.
        assert_eq!(
            ease_view(Some(5_000.0), Some(1_000.0), 0.0, u),
            Some(5_000.0)
        );
    }

    #[test]
    fn levels_by_name() {
        for l in QualityLevel::ALL {
            assert_eq!(QualityLevel::parse(l.name()), Some(l));
        }
        assert_eq!(QualityLevel::parse(" low "), Some(QualityLevel::Low));
        assert_eq!(QualityLevel::parse("ultra"), None);
        assert!(
            QualityPreset::of(QualityLevel::Low)
                .summary()
                .contains("MSAA off")
        );
    }

    /// The view distance: the far plane at 1.5 × and an exponential fog
    /// with its visibility there; a nearer fog (weather) stays.
    #[test]
    fn the_view_distance_fogs_out_before_the_far_plane() {
        use bevy::pbr::FogFalloff;
        let low = QualityPreset::of(QualityLevel::Low);
        let clear = FogFalloff::Linear {
            start: 50_000.0,
            end: 100_000.0,
        };
        let (far, fog) = low.limit_view(120_000.0, clear.clone());
        assert_eq!(far, 12_000.0);
        let reach = crate::far_terrain::fog_reach(&fog);
        assert!((reach - 8_000.0).abs() < 400.0, "{reach}");
        // A thick fog of the weather (300 m) is kept.
        let thick = FogFalloff::from_visibility(300.0);
        let (_, fog) = low.limit_view(120_000.0, thick.clone());
        assert_eq!(
            crate::far_terrain::fog_reach(&fog),
            crate::far_terrain::fog_reach(&thick)
        );
        // A test route's 1 km view: nothing to limit.
        let (far, _) = low.limit_view(1000.0, clear.clone());
        assert_eq!(far, 1000.0);
        let (far, _) = QualityPreset::of(QualityLevel::High).limit_view(120_000.0, clear);
        assert_eq!(far, 120_000.0);
    }

    /// The preset reaches the sun, the camera, the particles, the houses
    /// and the trees – at once and for what spawns later; High changes
    /// nothing.
    #[test]
    fn a_level_applies_to_the_scene() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(QualityPlugin)
            .insert_resource(DirectionalLightShadowMap { size: 4096 })
            .insert_resource(crate::weather::WeatherFx {
                particles: 5000,
                ..default()
            });
        let high = QualityPreset::default();
        let sun = app
            .world_mut()
            .spawn((crate::sky::Sun, high.cascades()))
            .id();
        let cam = app.world_mut().spawn(FollowCamera).id();
        let ranges = crate::houses::lod_ranges();
        let house = app.world_mut().spawn((HouseLod(0), ranges[0].clone())).id();
        let tree = app.world_mut().spawn(TileTrees).id();
        app.update();
        // High: as spawned; the particle budget set by a command stays.
        let w = app.world();
        assert_eq!(w.resource::<DirectionalLightShadowMap>().size, 4096);
        assert_eq!(w.resource::<crate::weather::WeatherFx>().particles, 5000);
        assert_eq!(w.get::<Msaa>(cam), Some(&Msaa::Sample4));
        assert!(same_range(w.get(house), Some(&ranges[0])));
        assert!(same_range(w.get(tree), Some(&tree_range(&high))));
        assert_eq!(w.get::<CascadeShadowConfig>(sun).unwrap().bounds.len(), 4);

        app.insert_resource(RenderQuality(QualityPreset::of(QualityLevel::Low)));
        app.update();
        let low = QualityPreset::of(QualityLevel::Low);
        let w = app.world();
        assert_eq!(
            w.resource::<DirectionalLightShadowMap>().size,
            low.shadow_map_px
        );
        assert_eq!(w.resource::<crate::weather::WeatherFx>().particles, 10_000);
        assert_eq!(w.get::<Msaa>(cam), Some(&Msaa::Off));
        assert!(same_range(
            w.get(house),
            Some(&crate::houses::lod_ranges_scaled(0.5)[0])
        ));
        assert!(same_range(w.get(tree), Some(&tree_range(&low))));
        assert_eq!(w.get::<CascadeShadowConfig>(sun).unwrap().bounds.len(), 1);

        // A tile loaded later gets the level too.
        let later = app.world_mut().spawn((HouseLod(1), ranges[1].clone())).id();
        let trees = app.world_mut().spawn(TileTrees).id();
        app.update();
        let w = app.world();
        assert!(same_range(
            w.get(later),
            Some(&crate::houses::lod_ranges_scaled(0.5)[1])
        ));
        assert!(w.get::<VisibilityRange>(trees).is_some());

        // Back to High: everything as before.
        app.insert_resource(RenderQuality(high));
        app.update();
        let w = app.world();
        assert_eq!(w.resource::<DirectionalLightShadowMap>().size, 4096);
        assert_eq!(
            w.resource::<crate::weather::WeatherFx>().particles,
            ocw_weather::particles::BUDGET
        );
        assert_eq!(w.get::<Msaa>(cam), Some(&Msaa::Sample4));
        assert!(same_range(w.get(house), Some(&ranges[0])));
        assert!(same_range(w.get(tree), Some(&tree_range(&high))));
    }
}
