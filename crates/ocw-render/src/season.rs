//! Seasons in the renderer (spec §14 Phase 9, 9C, milestone S8): the
//! world date (the sky's local date at the rider, i.e. the session's world
//! clock and its time machine) turned into what the landscape looks like.
//!
//! The model is `ocw_sky::season` (pure): phenology by degree days, the
//! crop calendar and the climatological snowpack, by date and altitude.
//! Here it is drawn:
//!
//! - **Trees** (baked into the tile meshes, rebuilt every season bucket of
//!   [`crate::region_look::BUCKET_DAYS`] days like the seasonal imagery):
//!   each tree gets a leaf type – measured (spec §14 Phase 8 T3, the
//!   [`ocw_world::trees::measured_leaf`] hook) or the region's mix by land
//!   use and altitude – and deciduous trees (round crowns) and larches leaf
//!   out, are green, colour and go bare (branches only); conifers stay
//!   green ([`tree_colour`], `package::seasonal_tree_arrays`).
//! - **Meadows and fields:** the near ground layers are recoloured in the
//!   terrain shader by altitude ramps ([`terrain_uniform`]): grass from
//!   winter straw to lush green (and dry straw in hot dry summers), fields
//!   through the crop calendar (sown soil, green, golden, stubble,
//!   ploughed; cereals and maize in patches). The colour maps and the far
//!   terrain take the same colours on the CPU ([`ground_colour`],
//!   [`canopy_colour`]). The colours change the hue and keep most of the
//!   layer's brightness, which the seasonal imagery tints set
//!   (`region_look::season_factors`), so the two stay consistent.
//! - **Snow:** the climatological snow line by date and altitude
//!   ([`ocw_sky::season::SeasonDay::snow_ramp`]) on meadows, trees and
//!   roofs and the verges, combined with the session weather's fallen snow
//!   by [`ocw_sky::season::lying_snow`] (the larger cover wins); roads stay
//!   cleared.

use std::sync::Arc;

use bevy::prelude::*;
use clap::Command;
use ocw_sky::season::{
    CEREAL, Crop, CropCalendar, FROST_CDD, Foliage, LeafType, MAIZE, Ramp, SeasonDay, SeasonModel,
    SeasonRamps, SnowObservation, TreeSetting, ramp,
};
use ocw_world::landuse::LandClass;

use crate::RideView;
use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// The world date's season, cached: the model per place, the day per
/// season bucket, the shader ramps per rider altitude.
#[derive(Resource, Default)]
pub struct SeasonCache {
    model: Option<(ModelKey, Arc<SeasonModel>)>,
    day: Option<(ModelKey, u32, Arc<SeasonDay>)>,
    ramps: Option<(i64, SeasonRamps)>,
    /// A measured snow depth from real weather (spec §14 Phase 9 9F); the
    /// snow line follows it ([`SeasonDay::snow_ramp`]).
    pub observed: Option<SnowObservation>,
    /// Seasons drawn (console `season.fx`; off: summer without snow, for
    /// comparisons).
    pub enabled: bool,
}

type ModelKey = (&'static str, i32);

/// The model key of a place: its region, and the latitude in half degrees
/// for climates by latitude.
pub fn model_key(lat: f64, lon: f64) -> ModelKey {
    let c = ocw_sky::season::Climate::for_place(lat, lon);
    let band = if c.region == "latitude" {
        (lat * 2.0).round() as i32
    } else {
        0
    };
    (c.region, band)
}

impl SeasonCache {
    pub fn new() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// The season at the rider for this frame's view (`None` without a
    /// sky – no world clock yet – or with the seasons switched off).
    pub fn day(&mut self, view: &RideView) -> Option<Arc<SeasonDay>> {
        if !self.enabled {
            return None;
        }
        let sky = &view.sky.as_ref()?.sky;
        let (lat, lon) = sky.place;
        let key = model_key(lat, lon);
        let bucket = crate::region_look::season_bucket(ocw_world::look::day_of_year(
            sky.local.year,
            sky.local.month,
            sky.local.day,
        ));
        if let Some((k, b, d)) = &self.day
            && *k == key
            && *b == bucket
        {
            return Some(d.clone());
        }
        let model = match &self.model {
            Some((k, m)) if *k == key => m.clone(),
            _ => {
                let m = Arc::new(SeasonModel::for_place(lat, lon));
                self.model = Some((key, m.clone()));
                m
            }
        };
        let day = Arc::new(model.day(crate::region_look::bucket_day(bucket)));
        self.day = Some((key, bucket, day.clone()));
        self.ramps = None;
        Some(day)
    }

    /// The shader ramps at the rider's altitude (refitted every 25 m).
    pub fn ramps(&mut self, view: &RideView) -> Option<SeasonRamps> {
        let day = self.day(view)?;
        let alt = rider_alt(view);
        let k = (alt / 25.0).round() as i64;
        if let Some((kk, r)) = self.ramps
            && kk == k
        {
            return Some(r);
        }
        let r = day.ramps(alt, self.observed);
        self.ramps = Some((k, r));
        Some(r)
    }

    /// The snow ramp the tiles bake into the far terrain (climatology, or
    /// moved by an observation).
    pub fn snow(&mut self, view: &RideView) -> Ramp {
        self.ramps(view).map_or_else(Ramp::default, |r| r.snow)
    }
}

/// The local rider's altitude (m; 500 without a position).
pub fn rider_alt(view: &RideView) -> f64 {
    view.position.map_or(500.0, |p| p.2)
}

// --- Colours ----------------------------------------------------------------

fn lin3(r: f32, g: f32, b: f32) -> [f32; 3] {
    let c = Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// How much of the ground's brightness the seasonal colours keep (the
/// imagery's seasonal factor sets it, `region_look::season_factors`).
pub const KEEP_BRIGHTNESS: f32 = 0.65;

/// `base` moved by `t` towards the hue of `target`, at a brightness
/// `keep` of the way from the target's to the base's (linear RGB). The
/// terrain shader's `retint` mirrors it.
pub fn retint(base: [f32; 3], target: [f32; 3], t: f32, keep: f32) -> [f32; 3] {
    if t <= 0.0 {
        return base;
    }
    let lt = lum(target).max(1e-4);
    let k = (lum(base).max(1e-4) / lt).powf(keep);
    mix3(base, target.map(|x| x * k), t)
}

/// The seasonal ground colours (linear RGB), in the order of the terrain
/// uniform's `season_colours`.
pub fn ground_palette() -> [[f32; 3]; 10] {
    static PALETTE: std::sync::OnceLock<[[f32; 3]; 10]> = std::sync::OnceLock::new();
    *PALETTE.get_or_init(palette_srgb)
}

fn palette_srgb() -> [[f32; 3]; 10] {
    [
        lin3(0.50, 0.49, 0.37), // 0 dormant grass (winter, olive straw)
        lin3(0.74, 0.64, 0.40), // 1 dry summer straw
        lin3(0.38, 0.29, 0.21), // 2 ploughed soil
        lin3(0.40, 0.37, 0.26), // 3 sown soil with the first green
        lin3(0.33, 0.50, 0.20), // 4 green cereal
        lin3(0.80, 0.68, 0.36), // 5 ripe cereal
        lin3(0.74, 0.66, 0.46), // 6 stubble
        lin3(0.27, 0.43, 0.15), // 7 green maize
        lin3(0.62, 0.55, 0.33), // 8 dry maize
        lin3(0.55, 0.48, 0.33), // 9 maize stubble
    ]
}

const DORMANT: usize = 0;
const STRAW: usize = 1;
const SOIL: usize = 2;
const SOWN: usize = 3;

/// The grass colour of a meadow layer `base` (linear) that is `green`
/// (0 dormant – 1 lush) and `dry` (summer straw).
pub fn grass_colour(base: [f32; 3], green: f32, dry: f32) -> [f32; 3] {
    let p = ground_palette();
    let c = retint(
        base,
        p[DORMANT],
        (1.0 - green) * DORMANT_SHARE,
        KEEP_BRIGHTNESS,
    );
    retint(c, p[STRAW], dry * 0.85, KEEP_BRIGHTNESS * 0.8)
}

/// The colour of a field of `crop` at `gdd` / `cdd` (linear): the stages
/// of its calendar blended in order; fields still standing when the
/// autumn frost comes are cut. The terrain shader's `crop_target`
/// mirrors it.
pub fn crop_target(crop: Crop, gdd: f32, cdd: f32) -> [f32; 3] {
    let p = ground_palette();
    let (cal, start, green, ripe, stubble) = match crop {
        Crop::Cereal => (&CEREAL, p[SOWN], p[4], p[5], p[6]),
        Crop::Maize => (&MAIZE, p[SOIL], p[7], p[8], p[9]),
    };
    crop_path(cal, start, green, ripe, stubble, gdd, cdd)
}

fn crop_path(
    cal: &CropCalendar,
    start: [f32; 3],
    green: [f32; 3],
    ripe: [f32; 3],
    stubble: [f32; 3],
    gdd: f32,
    cdd: f32,
) -> [f32; 3] {
    let p = ground_palette();
    let r = |x: f32, lohi: (f32, f32)| {
        ramp(f64::from(x), (f64::from(lohi.0), f64::from(lohi.1))) as f32
    };
    let mut c = mix3(start, green, r(gdd, cal.sprout));
    c = mix3(c, ripe, r(gdd, cal.ripen));
    c = mix3(c, stubble, r(gdd, cal.harvest));
    c = mix3(c, p[SOIL], r(gdd, cal.plough));
    c = mix3(c, p[SOWN], r(gdd, cal.sow));
    let cut = r(cdd, FROST_CDD) * (1.0 - r(gdd, cal.harvest));
    mix3(c, stubble, cut)
}

/// How strongly the crop colour replaces a farmland layer.
pub const CROP_WEIGHT: f32 = 0.85;
/// How much of the layer's brightness a field keeps (less than the
/// grass: ploughed soil is darker than any crop).
pub const CROP_KEEP: f32 = 0.45;
/// How far a dormant meadow turns to straw (lowland meadows keep some
/// green over winter).
pub const DORMANT_SHARE: f32 = 0.8;

/// A farmland layer `base` (linear) with the crop of the season: a mix
/// of cereal and maize fields by `maize` share.
pub fn crop_colour(base: [f32; 3], gdd: f32, cdd: f32, maize: f32) -> [f32; 3] {
    let t = mix3(
        crop_target(Crop::Cereal, gdd, cdd),
        crop_target(Crop::Maize, gdd, cdd),
        maize,
    );
    retint(base, t, CROP_WEIGHT, CROP_KEEP)
}

/// The colour of a land class in the colour map / far terrain (sRGB) with
/// the season at `alt_m`: its meadow share as the grass, its farmland
/// share as the fields (cereal and maize mixed).
pub fn ground_colour(class: LandClass, srgb: [f32; 3], alt_m: f64, season: &SeasonDay) -> [f32; 3] {
    ground_colour_with(
        class,
        srgb,
        alt_m,
        season,
        season.climate.maize_share as f32,
    )
}

/// The colour map under a plantation of section `VEGE` (sRGB): banana
/// plantations and avocado / citrus groves darker green from afar (their
/// plants are drawn only near).
pub fn plantation_ground(crop: ocw_world::vegetation::FieldCrop, srgb: [f32; 3]) -> [f32; 3] {
    use ocw_world::vegetation::FieldCrop;
    let (target, t) = match crop {
        FieldCrop::Banana => ([0.24, 0.38, 0.13], 0.6),
        FieldCrop::Subtropical => ([0.16, 0.29, 0.12], 0.5),
        FieldCrop::Orchard => ([0.26, 0.40, 0.17], 0.25),
        _ => return srgb,
    };
    mix3(srgb, target, t)
}

/// [`ground_colour`] with the field's own share of summer crops (`maize`:
/// 1 maize, potatoes, sunflowers; 0 winter cereals, rapeseed).
pub fn ground_colour_with(
    class: LandClass,
    srgb: [f32; 3],
    alt_m: f64,
    season: &SeasonDay,
    maize: f32,
) -> [f32; 3] {
    let w = crate::ground::class_weights(class);
    let (wm, wf) = (w[crate::ground::MEADOW], w[crate::ground::FARMLAND]);
    if wm <= 0.0 && wf <= 0.0 {
        return srgb;
    }
    let base = Color::srgb(srgb[0], srgb[1], srgb[2]).to_linear();
    let base = [base.red, base.green, base.blue];
    let s = season.state(alt_m);
    let g = ocw_sky::season::grass(&season.climate, &s);
    let mut c = mix3(base, grass_colour(base, g.green, g.dry), wm);
    c = mix3(c, crop_colour(c, s.gdd, s.cdd, maize), wf);
    let out = Color::linear_rgb(c[0], c[1], c[2]).to_srgba();
    [out.red, out.green, out.blue]
}

/// The snow colour of the far terrain (sRGB).
pub const FAR_SNOW: [f32; 3] = [0.90, 0.92, 0.95];

/// Snow on a far ground colour (sRGB) where `cover` lies on a slope
/// (rise over run): only on the ground that faces up (not on cliffs, as
/// the terrain shader), less where trees stand out of it (`trees`, their
/// cover 0 – 1).
pub fn far_snow(srgb: [f32; 3], cover: f32, slope: f32, trees: f32) -> [f32; 3] {
    if cover <= 0.0 {
        return srgb;
    }
    let up = 1.0 / (1.0 + slope * slope).sqrt();
    let s = smoothstep(0.62, 0.86, up) * cover * (1.0 - 0.55 * trees.clamp(0.0, 1.0));
    mix3(srgb, FAR_SNOW, s)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A bare deciduous wood from afar (sRGB): grey-brown twigs over the
/// litter.
pub const BARE_WOOD: [f32; 3] = [0.36, 0.31, 0.26];

/// The colour of the forest canopy from afar (sRGB) at `alt_m`: the
/// conifers' `conifer` green with the deciduous and larch shares of the
/// region's mix in their seasonal colour.
pub fn canopy_colour(conifer: [f32; 3], alt_m: f64, season: &SeasonDay) -> [f32; 3] {
    CanopyTones::new(conifer, alt_m, season).fallback()
}

/// The far canopy's colours at one altitude and date: the conifers', the
/// deciduous trees' and the larches' (sRGB), and the region's fallback
/// shares there; mixed per point by the measured broadleaf share where the
/// package has the forest type (spec §14 Phase 8 T3, [`CanopyTones::at`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyTones {
    conifer: [f32; 3],
    deciduous: [f32; 3],
    larch: [f32; 3],
    /// (deciduous, larch) of the fallback mix.
    fallback: (f64, f64),
    /// The region's larch share among its conifers at this altitude.
    larch_of_conifers: f64,
    evergreen_broadleaves: bool,
}

impl CanopyTones {
    pub fn new(conifer: [f32; 3], alt_m: f64, season: &SeasonDay) -> Self {
        let mix = &season.climate.trees;
        let tone = |leaf: LeafType| {
            let f = season.foliage(leaf, alt_m, 0.0);
            let leafy = srgb_of(tree_colour(leaf, &f, 0.5, 0.35));
            mix3(BARE_WOOD, leafy, f.leaves)
        };
        let fallback = mix.shares(TreeSetting::Forest, alt_m);
        Self {
            conifer,
            deciduous: tone(LeafType::Deciduous),
            larch: tone(LeafType::Larch),
            fallback,
            larch_of_conifers: mix.measured_shares(TreeSetting::Forest, alt_m, 0.0).1,
            evergreen_broadleaves: mix.evergreen_broadleaves,
        }
    }

    fn mix(&self, (d, l): (f64, f64)) -> [f32; 3] {
        if d + l <= 0.0 {
            return self.conifer;
        }
        std::array::from_fn(|i| {
            self.conifer[i] * (1.0 - d - l) as f32
                + self.deciduous[i] * d as f32
                + self.larch[i] * l as f32
        })
    }

    /// The canopy by the region's mix (no measured forest type).
    pub fn fallback(&self) -> [f32; 3] {
        self.mix(self.fallback)
    }

    /// The canopy where `broadleaf` (0 … 1) of the trees were measured
    /// broadleaved (`None`: not measured, the fallback mix).
    pub fn at(&self, broadleaf: Option<f32>) -> [f32; 3] {
        match broadleaf {
            Some(b) => {
                let b = f64::from(b.clamp(0.0, 1.0));
                let d = if self.evergreen_broadleaves { 0.0 } else { b };
                self.mix((d, (1.0 - b) * self.larch_of_conifers))
            }
            None => self.fallback(),
        }
    }
}

fn srgb_of(lin: [f32; 3]) -> [f32; 3] {
    let c = Color::linear_rgb(lin[0], lin[1], lin[2]).to_srgba();
    [c.red, c.green, c.blue]
}

// --- Trees --------------------------------------------------------------------

/// The twigs and branches of a bare tree (linear).
pub fn twig() -> [f32; 3] {
    lin3(0.36, 0.31, 0.27)
}

/// The crown colour (linear) of a tree of `leaf` type with `f` leaves;
/// `shade` (0 – 1) picks its own green, `hue` (0 – 1) its autumn colour
/// (beech gold, maple orange and red, oak brown).
pub fn tree_colour(leaf: LeafType, f: &Foliage, shade: f32, hue: f32) -> [f32; 3] {
    let pick = |c: &[[f32; 3]]| c[((shade * c.len() as f32) as usize).min(c.len() - 1)];
    let (summer, fresh, autumn) = match leaf {
        LeafType::Evergreen => {
            return pick(&[
                lin3(0.13, 0.33, 0.15),
                lin3(0.11, 0.28, 0.13),
                lin3(0.17, 0.36, 0.14),
                lin3(0.20, 0.38, 0.17),
            ]);
        }
        LeafType::Deciduous => (
            pick(&[
                lin3(0.22, 0.40, 0.15),
                lin3(0.26, 0.44, 0.17),
                lin3(0.20, 0.36, 0.13),
                lin3(0.29, 0.45, 0.19),
            ]),
            lin3(0.47, 0.64, 0.24),
            if hue < 0.4 {
                lin3(0.82, 0.63, 0.17)
            } else if hue < 0.7 {
                lin3(0.84, 0.46, 0.12)
            } else if hue < 0.9 {
                lin3(0.56, 0.39, 0.19)
            } else {
                lin3(0.68, 0.22, 0.10)
            },
        ),
        LeafType::Larch => (
            lin3(0.33, 0.48, 0.20),
            lin3(0.56, 0.70, 0.28),
            lin3(0.86, 0.64, 0.18),
        ),
    };
    let mut c = mix3(summer, fresh, f.fresh);
    c = mix3(c, autumn, f.autumn);
    // Thinning autumn crowns turn browner.
    mix3(c, lin3(0.50, 0.36, 0.22), (1.0 - f.leaves) * f.autumn * 0.5)
}

/// The setting of a tree for the leaf-type fallback, by its land class
/// (`None`: the provisional patches – forest).
pub fn tree_setting(class: Option<LandClass>) -> TreeSetting {
    ocw_world::flora::tree_setting(class)
}

/// The measured leaf type in `ocw_sky`'s terms.
pub fn measured(
    m: Option<ocw_world::trees::DominantLeaf>,
) -> Option<ocw_sky::season::MeasuredLeaf> {
    ocw_world::flora::measured_leaf_type(m)
}

// --- Uniforms -----------------------------------------------------------------

/// The terrain's season uniform (`ground.wesl`, `TerrainLook.season`):
/// - 0: snow ramp a, b; grass green-up ramp a, b
/// - 1: grass dormancy ramp a, b; summer dryness ramp a, b
/// - 2: growing degree days a + b · alt; cold degree days a + b · alt
/// - 3: x on (1) / off (0), y maize share, z / w the frost (CDD)
/// - 4 – 6: the cereal calendar (sprout, ripen, harvest, plough, sow; lo, hi)
/// - 6.zw – 8: the maize calendar
pub fn terrain_uniform(r: Option<&SeasonRamps>, maize: f32) -> [Vec4; 9] {
    let Some(r) = r else {
        return [Vec4::ZERO; 9];
    };
    let c = &CEREAL;
    let m = &MAIZE;
    let p = |a: (f32, f32), b: (f32, f32)| Vec4::new(a.0, a.1, b.0, b.1);
    [
        Vec4::new(r.snow.a, r.snow.b, r.grass_up.a, r.grass_up.b),
        Vec4::new(r.grass_dormant.a, r.grass_dormant.b, r.dry.a, r.dry.b),
        Vec4::new(r.gdd.a, r.gdd.b, r.cdd.a, r.cdd.b),
        Vec4::new(1.0, maize, FROST_CDD.0, FROST_CDD.1),
        p(c.sprout, c.ripen),
        p(c.harvest, c.plough),
        p(c.sow, m.sprout),
        p(m.ripen, m.harvest),
        p(m.plough, m.sow),
    ]
}

/// The season uniform of the other surfaces (roads, buildings, trees): x,
/// y the snow ramp (`clamp(x + y · alt)`), z on.
pub fn surface_uniform(r: Option<&SeasonRamps>) -> Vec4 {
    r.map_or(Vec4::ZERO, |r| Vec4::new(r.snow.a, r.snow.b, 1.0, 0.0))
}

/// The seasonal colours of the terrain uniform (`season_colours`).
pub fn palette_uniform() -> [Vec4; 10] {
    ground_palette().map(|c| Vec4::new(c[0], c[1], c[2], 1.0))
}

pub struct SeasonPlugin;

impl Plugin for SeasonPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SeasonCache::new())
            .add_systems(Update, sync_season);
        register_commands(app);
    }
}

/// Hands the season to every terrain, road, building and tree material
/// (also to ones created later).
fn sync_season(
    view: Option<Res<RideView>>,
    mut cache: ResMut<SeasonCache>,
    terrain: Option<ResMut<Assets<crate::ground::TerrainMaterial>>>,
    roads: Option<ResMut<Assets<crate::road_material::RoadMaterial>>>,
    buildings: Option<ResMut<Assets<crate::building_material::BuildingMaterial>>>,
    trees: Option<ResMut<Assets<crate::see_through::SeeThroughMaterial>>>,
) {
    // Real weather's measured snow moves the snow line (S8).
    let observed = view
        .as_deref()
        .and_then(|v| v.weather.as_ref())
        .and_then(|w| w.snow);
    if cache.observed != observed {
        cache.observed = observed;
        cache.ramps = None;
    }
    let (ramps, maize) = match view.as_deref() {
        Some(v) => (
            cache.ramps(v),
            cache.day(v).map_or(0.0, |d| d.climate.maize_share as f32),
        ),
        None => (None, 0.0),
    };
    let t = terrain_uniform(ramps.as_ref(), maize);
    let s = surface_uniform(ramps.as_ref());
    if let Some(mut assets) = terrain {
        let stale: Vec<_> = assets
            .iter()
            .filter(|(_, m)| m.extension.look.season != t)
            .map(|(id, _)| id)
            .collect();
        for id in stale {
            if let Some(mut m) = assets.get_mut(id) {
                m.extension.look.season = t;
            }
        }
    }
    macro_rules! sync {
        ($assets:expr, $($field:ident).+) => {
            if let Some(mut assets) = $assets {
                let stale: Vec<_> = assets
                    .iter()
                    .filter(|(_, m)| m.extension.$($field).+ != s)
                    .map(|(id, _)| id)
                    .collect();
                for id in stale {
                    if let Some(mut m) = assets.get_mut(id) {
                        m.extension.$($field).+ = s;
                    }
                }
            }
        };
    }
    sync!(roads, look.season);
    sync!(buildings, look.season);
    sync!(trees, season);
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("season")
            .about(
                "The season at the rider (spec 9C): leaves, grass, fields, the climatological snow line; season set SEASON|DAY jumps the date there",
            )
            .arg(
                clap::Arg::new("args")
                    .num_args(0..)
                    .action(clap::ArgAction::Append),
            ),
        |world, m| {
            let args: Vec<&str> = m
                .get_many::<String>("args")
                .map(|v| v.map(String::as_str).collect())
                .unwrap_or_default();
            // `season set autumn`: the date jumps to the season's day of the
            // place, the time of day kept (spec §14 Phase 9 S9b, decision
            // Q11: no season apart from the date) – `time season`.
            match args.as_slice() {
                [] => {}
                ["set", "follow"] => {
                    return Ok("the season always follows the date (the time machine's)".into());
                }
                ["set", pick] => {
                    if ocw_sky::machine::SeasonPick::parse(pick).is_none() {
                        return Err(
                            "season set spring|early-summer|summer|autumn|late-autumn|winter|DAY"
                                .into(),
                        );
                    }
                    world
                        .resource_mut::<crate::console::ConsoleState>()
                        .queue(format!("time season {pick}"));
                    return Ok(String::new());
                }
                _ => {
                    return Err(
                        "season [set spring|early-summer|summer|autumn|late-autumn|winter|DAY]"
                            .into(),
                    );
                }
            }
            world.resource_scope(|world, mut cache: Mut<SeasonCache>| {
                let Some(view) = world.get_resource::<RideView>() else {
                    return Err("no ride".into());
                };
                if !cache.enabled {
                    return Ok("seasons off (season.fx on)".into());
                }
                let day = cache.day(view).ok_or("no world clock yet")?;
                Ok(day.describe(rider_alt(view)))
            })
        },
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("season.fx")
            .about(
                "Seasons drawn: season.fx on|off (off: summer colours without climatological snow)",
            )
            .arg(clap::Arg::new("value").value_parser(["on", "off"])),
        |world, m| {
            let mut cache = world.resource_mut::<SeasonCache>();
            if let Some(v) = m.get_one::<String>("value") {
                cache.enabled = v == "on";
                cache.day = None;
                cache.ramps = None;
            }
            Ok(format!(
                "seasons {}",
                if cache.enabled { "on" } else { "off" }
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_sky::season::SeasonModel;

    fn vaduz() -> SeasonModel {
        SeasonModel::for_place(47.14, 9.52)
    }

    #[test]
    fn retint_keeps_most_of_the_brightness() {
        let base = [0.10, 0.25, 0.06];
        let straw = ground_palette()[STRAW];
        let c = retint(base, straw, 1.0, KEEP_BRIGHTNESS);
        // The hue is straw's (red over green), the brightness between.
        assert!(c[0] / c[1] > 0.8 && base[0] / base[1] < 0.5);
        let (lb, ls, lc) = (lum(base), lum(straw), lum(c));
        assert!(lc > lb.min(ls) && lc < lb.max(ls));
        assert!((lc - lb).abs() < (ls - lb).abs() * 0.5);
        assert_eq!(retint(base, straw, 0.0, KEEP_BRIGHTNESS), base);
    }

    #[test]
    fn meadows_and_fields_through_the_year() {
        let m = vaduz();
        let base = lin3(0.40, 0.58, 0.27);
        let at = |d: f64| ground_colour(LandClass::Meadow, [0.40, 0.58, 0.27], 455.0, &m.day(d));
        // Summer: the meadow as it is; winter: straw (redder, less green).
        let summer = at(182.0);
        assert!((summer[1] - 0.58).abs() < 0.01, "{summer:?}");
        let winter = at(15.0);
        assert!(
            winter[0] / winter[1] > summer[0] / summer[1] + 0.2,
            "{winter:?}"
        );
        let _ = base;
        // Fields: green in June, golden in July (cereal), brown when
        // ploughed (cereal) in September.
        let cereal = |d: f64| {
            let s = m.day(d).state(455.0);
            srgb_of(crop_target(Crop::Cereal, s.gdd, s.cdd))
        };
        let june = cereal(160.0);
        let july = cereal(200.0);
        let sept = cereal(255.0);
        assert!(june[1] > june[0], "June green {june:?}");
        assert!(
            july[0] > july[1] * 1.05 && july[0] > 0.6,
            "July golden {july:?}"
        );
        assert!(
            sept[0] > sept[1] && sept[1] < 0.45,
            "September soil {sept:?}"
        );
        // Maize is still green when the cereals are golden.
        let s = m.day(200.0).state(455.0);
        let maize = srgb_of(crop_target(Crop::Maize, s.gdd, s.cdd));
        assert!(maize[1] > maize[0], "{maize:?}");
        // Forests and rock do not change their ground colour.
        assert_eq!(
            ground_colour(LandClass::Rock, [0.5; 3], 455.0, &m.day(15.0)),
            [0.5; 3]
        );
    }

    #[test]
    fn trees_by_season() {
        let m = vaduz();
        let colour = |d: f64, leaf| {
            let f = m.day(d).foliage(leaf, 455.0, 0.0);
            (f, srgb_of(tree_colour(leaf, &f, 0.3, 0.2)))
        };
        let (jul, green) = colour(182.0, LeafType::Deciduous);
        assert!(jul.leaves > 0.99 && green[1] > green[0] * 1.3);
        let (_, gold) = colour(300.0, LeafType::Deciduous);
        assert!(gold[0] > gold[1] && gold[0] > 0.7, "{gold:?}");
        let (jan, _) = colour(15.0, LeafType::Deciduous);
        assert!(jan.leaves < 0.01);
        let (_, fir) = colour(15.0, LeafType::Evergreen);
        assert_eq!(fir, colour(182.0, LeafType::Evergreen).1);
        // The far canopy: brown-grey in a valley winter, green in summer,
        // nearly unchanged high up where conifers dominate.
        let conifer = [0.11, 0.25, 0.11];
        let w = canopy_colour(conifer, 455.0, &m.day(15.0));
        let s = canopy_colour(conifer, 455.0, &m.day(182.0));
        assert!(w[0] > s[0] + 0.08, "{w:?} {s:?}");
        let high = canopy_colour(conifer, 1900.0, &m.day(15.0));
        assert!((high[1] - conifer[1]).abs() < 0.05);
        // With the measured forest type (spec §14 Phase 8 T3): a conifer
        // wood stays green in the valley's winter, a broadleaf wood is bare
        // even high up; unmeasured keeps the regional mix.
        let t = CanopyTones::new(conifer, 455.0, &m.day(15.0));
        assert_eq!(t.at(None), w);
        assert!((t.at(Some(0.0))[1] - conifer[1]).abs() < 0.01);
        let bare = t.at(Some(1.0));
        assert!(
            (0..3).all(|i| (bare[i] - BARE_WOOD[i]).abs() < 0.03),
            "{bare:?}"
        );
        let half = t.at(Some(0.5));
        assert!(half[0] > t.at(Some(0.0))[0] && half[0] < bare[0]);
        let high = CanopyTones::new(conifer, 1900.0, &m.day(15.0)).at(Some(1.0));
        assert!(high[0] > conifer[0] + 0.15, "{high:?}");
        // Settings by land class.
        assert_eq!(tree_setting(Some(LandClass::Orchard)), TreeSetting::Orchard);
        assert_eq!(tree_setting(Some(LandClass::Meadow)), TreeSetting::Open);
        assert_eq!(tree_setting(None), TreeSetting::Forest);
    }

    #[test]
    fn far_snow_lies_on_flat_ground() {
        let g = [0.3, 0.5, 0.2];
        assert_eq!(far_snow(g, 0.0, 0.0, 0.0), g);
        let flat = far_snow(g, 1.0, 0.0, 0.0);
        assert!((flat[0] - FAR_SNOW[0]).abs() < 1e-5);
        assert_eq!(far_snow(g, 1.0, 2.0, 0.0), g, "cliffs stay bare");
        let forest = far_snow(g, 1.0, 0.0, 1.0);
        assert!(forest[0] < flat[0] && forest[0] > g[0]);
    }

    #[test]
    fn uniforms() {
        assert_eq!(terrain_uniform(None, 0.4), [Vec4::ZERO; 9]);
        assert_eq!(surface_uniform(None), Vec4::ZERO);
        let m = vaduz();
        let r = m.day(15.0).ramps(455.0, None);
        let t = terrain_uniform(Some(&r), 0.4);
        assert_eq!(t[3].x, 1.0);
        assert_eq!(t[3].y, 0.4);
        // The snow ramp: none in Vaduz, white in Malbun in January.
        let snow = |alt: f32| (t[0].x + t[0].y * alt).clamp(0.0, 1.0);
        assert!(snow(455.0) < 0.05 && snow(1600.0) > 0.9);
        let s = surface_uniform(Some(&r));
        assert_eq!((s.x, s.y, s.z), (r.snow.a, r.snow.b, 1.0));
        assert_eq!(
            t[4],
            Vec4::new(
                CEREAL.sprout.0,
                CEREAL.sprout.1,
                CEREAL.ripen.0,
                CEREAL.ripen.1
            )
        );
    }

    #[test]
    fn the_cache_follows_the_date_and_place() {
        assert_eq!(model_key(47.14, 9.52), ("alpine-rhine", 0));
        assert_eq!(model_key(60.2, 10.0), ("latitude", 120));
    }
}
