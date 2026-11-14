//! Render quality presets (spec §14 Phase 10 B7): Low, Medium and High,
//! one set of numbers per level for what costs the GPU most – the sun's
//! shadow maps and cascades, MSAA, the view distance, how far trees are
//! drawn, the houses' levels of detail, the weather's particles and (in
//! the browser) the pixel ratio of the canvas.
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
        match self.view_m {
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

/// The quality the renderer draws with (the app sets it; High by
/// default).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderQuality(pub QualityPreset);

/// A house mesh of a tile with its level of detail (0 near, 1 mid, 2
/// far; `houses::lod_ranges`).
#[derive(Component, Debug, Clone, Copy)]
pub struct HouseLod(pub u8);

/// A tile's tree mesh (its draw distance follows the preset).
#[derive(Component, Debug, Clone, Copy)]
pub struct TileTrees;

/// The visibility range of a tile's trees for this preset (`None`: no
/// range – drawn while the tile is loaded).
pub fn tree_range(preset: &QualityPreset) -> Option<VisibilityRange> {
    preset.tree_view_m.map(|end| VisibilityRange {
        start_margin: 0.0..0.0,
        // Fade out (dithered) over the last 15 %.
        end_margin: 0.85 * end..end,
        use_aabb: true,
    })
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
    for (e, seen, has) in &trees {
        if !(changed || seen.is_added()) {
            continue;
        }
        match (&tree, has) {
            (Some(r), _) => {
                commands.entity(e).insert(r.clone());
            }
            (None, true) => {
                commands.entity(e).remove::<VisibilityRange>();
            }
            (None, false) => {}
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
        assert!(tree_range(&h).is_none());
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
        assert!(w.get::<VisibilityRange>(tree).is_none());
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
        assert!(same_range(w.get(tree), tree_range(&low).as_ref()));
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
        assert!(w.get::<VisibilityRange>(tree).is_none());
    }
}
