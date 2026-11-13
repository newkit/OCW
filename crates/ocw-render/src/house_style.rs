//! Regional house styles (spec §4.6, §14 Phase 8 T4): the presets of
//! `assets/building-styles.toml` (built in) and how a building's look –
//! roof shape and height, eaves, walls, roof, windows – is resolved from its
//! OSM facts (`ocw_world::buildings::BuildingTags`, section `BTAG`), its
//! measured roof colour (section `ROOF`, T3) and the preset of its region,
//! kind and use. Pure and deterministic: every choice a preset leaves to
//! chance is drawn from a hash of the building's position, so a house looks
//! the same on every machine and after every rebuild.
//!
//! Style selection follows the material sets (§4.6 `[[region]]`): by the
//! package's time zone, optionally a latitude / longitude box, and the
//! building's elevation (Alpine villages above the Rhine valley).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use bevy::color::{Color, LinearRgba, Srgba};
use ocw_world::buildings::{
    Building, BuildingKind, BuildingUse, Roof, RoofMaterial, RoofShape, WallMaterial,
};
use ocw_world::look::RoofColour;
use serde::Deserialize;

/// The shipped style file.
pub const STYLES_TOML: &str = include_str!("../../../assets/building-styles.toml");
/// Window styles the shader holds.
pub const MAX_WINDOW_STYLES: usize = 16;
/// Walls reach this far below the lowest ground (as the world builder's
/// `SINK_M`): the ground floor starts this much above the wall foot.
pub const SINK_M: f32 = 0.5;
/// Least wall height left after the eaves come down (m).
pub const MIN_EAVE_M: f32 = 2.4;

/// The styles of the shipped file (parsed once; the file is checked by the
/// tests, so this cannot fail at run time).
pub static STYLES: LazyLock<BuildingStyles> = LazyLock::new(|| {
    BuildingStyles::parse(STYLES_TOML).unwrap_or_else(|e| panic!("building-styles.toml: {e:?}"))
});

/// A material layer of the building texture arrays
/// (`ocw_assets::materials::BUILDING_IDS` order); glass has no texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Layer {
    RoofTiles = 0,
    Plaster = 1,
    Timber = 2,
    RoofSlates = 3,
    Metal = 4,
    Brick = 5,
    Stone = 6,
    Concrete = 7,
    /// Untextured, glossy.
    Glass = 15,
}

/// Number of textured building layers.
pub const LAYERS: usize = 8;

impl Layer {
    /// The library material id of a textured layer.
    pub fn material_id(self) -> Option<&'static str> {
        Some(match self {
            Layer::RoofTiles => "roof_tiles",
            Layer::Plaster => "plaster",
            Layer::Timber => "timber",
            Layer::RoofSlates => "roof_slates",
            Layer::Metal => "metal_sheet",
            Layer::Brick => "brick",
            Layer::Stone => "stone_wall",
            Layer::Concrete => "concrete",
            Layer::Glass => return None,
        })
    }

    /// The colour of the material itself (linear) without a library
    /// (with one: its measured mean, [`HouseLook::natural`]).
    pub fn fallback_colour(self) -> [f32; 3] {
        let srgb = match self {
            Layer::RoofTiles => [0.55, 0.26, 0.18],
            Layer::Plaster => [0.88, 0.86, 0.81],
            Layer::Timber => [0.45, 0.32, 0.21],
            Layer::RoofSlates => [0.30, 0.30, 0.32],
            Layer::Metal => [0.56, 0.58, 0.60],
            Layer::Brick => [0.56, 0.29, 0.22],
            Layer::Stone => [0.62, 0.58, 0.50],
            Layer::Concrete => [0.63, 0.63, 0.61],
            Layer::Glass => [0.10, 0.13, 0.16],
        };
        linear(srgb)
    }

    fn of_wall(m: WallMaterial) -> Self {
        match m {
            WallMaterial::Plaster => Layer::Plaster,
            WallMaterial::Timber => Layer::Timber,
            WallMaterial::Stone => Layer::Stone,
            WallMaterial::Brick => Layer::Brick,
            WallMaterial::Concrete => Layer::Concrete,
            WallMaterial::Glass => Layer::Glass,
            WallMaterial::Metal => Layer::Metal,
        }
    }

    /// The layer of a roof material and, for wood and grass, the colour
    /// they are drawn in (sRGB) on another material.
    fn of_roof(m: RoofMaterial) -> (Self, Option<[f32; 3]>) {
        match m {
            RoofMaterial::Tiles => (Layer::RoofTiles, None),
            RoofMaterial::Slate => (Layer::RoofSlates, None),
            RoofMaterial::Metal => (Layer::Metal, None),
            RoofMaterial::Concrete => (Layer::Concrete, None),
            RoofMaterial::Glass => (Layer::Glass, None),
            RoofMaterial::Wood => (Layer::Timber, Some([0.33, 0.25, 0.19])),
            RoofMaterial::Grass => (Layer::Concrete, Some([0.36, 0.42, 0.24])),
        }
    }
}

/// sRGB (0–1) to linear.
pub fn linear(c: [f32; 3]) -> [f32; 3] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue]
}

/// Linear to sRGB (0–1).
pub fn srgb(c: [f32; 3]) -> [f32; 3] {
    let s = Srgba::from(LinearRgba::rgb(c[0], c[1], c[2]));
    [s.red, s.green, s.blue]
}

/// Parses `#rgb` / `#rrggbb` (sRGB 0–1).
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().strip_prefix('#')?;
    let v = |a: &str| u8::from_str_radix(a, 16).ok().map(|x| f32::from(x) / 255.0);
    match h.len() {
        6 => Some([v(&h[0..2])?, v(&h[2..4])?, v(&h[4..6])?]),
        3 => {
            let d = |i: usize| v(&h[i..=i]).map(|x| x * 17.0);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

// --- The style file ---------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowFile {
    id: String,
    bay_m: f32,
    width_m: f32,
    height_m: f32,
    sill_m: f32,
    frame: String,
    #[serde(default)]
    shutters: Option<String>,
    door: String,
    #[serde(default = "door_width")]
    door_width_m: f32,
    #[serde(default = "door_height")]
    door_height_m: f32,
    #[serde(default)]
    shopfront: bool,
}

fn door_width() -> f32 {
    1.1
}

fn door_height() -> f32 {
    2.2
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    roofs: Option<BTreeMap<String, f32>>,
    pitch: Option<f32>,
    eave_drop_m: Option<f32>,
    overhang_m: Option<f32>,
    parapet_m: Option<f32>,
    walls: Option<BTreeMap<String, f32>>,
    upper: Option<BTreeMap<String, f32>>,
    wall_colours: Option<Vec<String>>,
    roof_materials: Option<BTreeMap<String, f32>>,
    roof_colours: Option<Vec<String>>,
    window: Option<String>,
    chimney: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleFile {
    id: String,
    name: String,
    #[serde(default)]
    time_zones: Vec<String>,
    #[serde(default)]
    bbox: Option<[f64; 4]>,
    #[serde(default)]
    min_elevation_m: Option<f64>,
    #[serde(default)]
    kind: BTreeMap<String, PresetFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    window: Vec<WindowFile>,
    style: Vec<StyleFile>,
}

// --- Resolved styles ---------------------------------------------------------

/// A window style (one per shader slot), colours linear.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowStyle {
    pub id: String,
    pub bay_m: f32,
    pub width_m: f32,
    pub height_m: f32,
    pub sill_m: f32,
    pub frame: [f32; 3],
    pub shutters: Option<[f32; 3]>,
    pub door: [f32; 3],
    pub door_width_m: f32,
    pub door_height_m: f32,
    pub shopfront: bool,
}

/// A weighted choice (sorted by name, weights > 0).
type Mix<T> = Vec<(T, f32)>;

/// A preset with its names checked; `None`: not set here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preset {
    pub roofs: Option<Mix<RoofShape>>,
    pub pitch: Option<f32>,
    pub eave_drop_m: Option<f32>,
    pub overhang_m: Option<f32>,
    pub parapet_m: Option<f32>,
    pub walls: Option<Mix<WallMaterial>>,
    /// The upper floors' material and its probability.
    pub upper: Option<Mix<WallMaterial>>,
    /// sRGB.
    pub wall_colours: Option<Vec<[f32; 3]>>,
    pub roof_materials: Option<Mix<RoofMaterial>>,
    /// sRGB.
    pub roof_colours: Option<Vec<[f32; 3]>>,
    /// Window style index (`Some(None)`: no windows).
    pub window: Option<Option<u8>>,
    pub chimney: Option<f32>,
}

/// One regional style.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub id: String,
    pub name: String,
    pub time_zones: Vec<String>,
    /// South, west, north, east (degrees).
    pub bbox: Option<[f64; 4]>,
    pub min_elevation_m: Option<f64>,
    /// Presets by kind or use name.
    pub presets: BTreeMap<String, Preset>,
}

/// All styles; the first is the default.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildingStyles {
    pub windows: Vec<WindowStyle>,
    pub styles: Vec<Style>,
}

/// The preset key names: building kinds, then the finer uses.
pub fn kind_name(k: BuildingKind) -> &'static str {
    match k {
        BuildingKind::Other => "other",
        BuildingKind::House => "house",
        BuildingKind::Apartments => "apartments",
        BuildingKind::Commercial => "commercial",
        BuildingKind::Industrial => "industrial",
        BuildingKind::Farm => "farm",
        BuildingKind::Religious => "religious",
        BuildingKind::Public => "public",
        BuildingKind::Small => "small",
    }
}

fn mix<T: Copy>(
    m: &BTreeMap<String, f32>,
    what: &str,
    parse: impl Fn(&str) -> Option<T>,
    errors: &mut Vec<String>,
) -> Mix<T> {
    let mut out = Vec::new();
    for (k, w) in m {
        match parse(k) {
            Some(v) if w.is_finite() && *w >= 0.0 => {
                if *w > 0.0 {
                    out.push((v, *w));
                }
            }
            Some(_) => errors.push(format!("{what}: weight of {k} must be ≥ 0")),
            None => errors.push(format!("{what}: unknown name {k}")),
        }
    }
    out
}

fn colours(v: &[String], what: &str, errors: &mut Vec<String>) -> Vec<[f32; 3]> {
    let out: Vec<[f32; 3]> = v
        .iter()
        .filter_map(|c| {
            let p = parse_hex(c);
            if p.is_none() {
                errors.push(format!("{what}: bad colour {c}"));
            }
            p
        })
        .collect();
    if out.is_empty() {
        errors.push(format!("{what}: no colours"));
    }
    out
}

impl BuildingStyles {
    /// Parses and checks a style file: names of kinds, shapes, materials
    /// and windows, colours, weights, at least one style, the first one
    /// without a region (the default).
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let file: File = toml::from_str(text).map_err(|e| vec![e.to_string()])?;
        let mut errors = Vec::new();
        if file.window.len() > MAX_WINDOW_STYLES {
            errors.push(format!(
                "{} window styles, at most {MAX_WINDOW_STYLES}",
                file.window.len()
            ));
        }
        let mut windows = Vec::new();
        for w in &file.window {
            let c = |s: &str, errors: &mut Vec<String>| {
                parse_hex(s).map(linear).unwrap_or_else(|| {
                    errors.push(format!("window {}: bad colour {s}", w.id));
                    [0.0; 3]
                })
            };
            if w.bay_m <= 0.5 || w.width_m < 0.0 || w.width_m > w.bay_m {
                errors.push(format!(
                    "window {}: needs 0 ≤ width ≤ bay, bay > 0.5 m",
                    w.id
                ));
            }
            windows.push(WindowStyle {
                id: w.id.clone(),
                bay_m: w.bay_m,
                width_m: w.width_m,
                height_m: w.height_m,
                sill_m: w.sill_m,
                frame: c(&w.frame, &mut errors),
                shutters: w.shutters.as_deref().map(|s| c(s, &mut errors)),
                door: c(&w.door, &mut errors),
                door_width_m: w.door_width_m,
                door_height_m: w.door_height_m,
                shopfront: w.shopfront,
            });
        }
        let known_key = |k: &str| {
            BuildingKind::ALL.iter().any(|b| kind_name(*b) == k)
                || BuildingUse::from_name(k).is_some_and(|u| u != BuildingUse::Unknown)
        };
        let mut styles = Vec::new();
        for (i, s) in file.style.iter().enumerate() {
            if i == 0
                && (!s.time_zones.is_empty() || s.bbox.is_some() || s.min_elevation_m.is_some())
            {
                errors.push(format!(
                    "style {}: the first style is the default (no region)",
                    s.id
                ));
            }
            if i > 0 && s.time_zones.is_empty() {
                errors.push(format!("style {}: needs time_zones", s.id));
            }
            let mut presets = BTreeMap::new();
            for (k, p) in &s.kind {
                let what = format!("style {}, {k}", s.id);
                if !known_key(k) {
                    errors.push(format!("{what}: unknown kind or use"));
                }
                let window = p.window.as_deref().map(|w| {
                    if w == "none" {
                        None
                    } else {
                        let i = windows.iter().position(|x| x.id == w);
                        if i.is_none() {
                            errors.push(format!("{what}: unknown window {w}"));
                        }
                        i.map(|i| i as u8)
                    }
                });
                presets.insert(
                    k.clone(),
                    Preset {
                        roofs: p.roofs.as_ref().map(|m| {
                            mix(
                                m,
                                &format!("{what} roofs"),
                                RoofShape::from_name,
                                &mut errors,
                            )
                        }),
                        pitch: p.pitch,
                        eave_drop_m: p.eave_drop_m,
                        overhang_m: p.overhang_m,
                        parapet_m: p.parapet_m,
                        walls: p.walls.as_ref().map(|m| {
                            mix(
                                m,
                                &format!("{what} walls"),
                                WallMaterial::from_name,
                                &mut errors,
                            )
                        }),
                        upper: p.upper.as_ref().map(|m| {
                            mix(
                                m,
                                &format!("{what} upper"),
                                WallMaterial::from_name,
                                &mut errors,
                            )
                        }),
                        wall_colours: p
                            .wall_colours
                            .as_ref()
                            .map(|v| colours(v, &format!("{what} wall_colours"), &mut errors)),
                        roof_materials: p.roof_materials.as_ref().map(|m| {
                            mix(
                                m,
                                &format!("{what} roof_materials"),
                                RoofMaterial::from_name,
                                &mut errors,
                            )
                        }),
                        roof_colours: p
                            .roof_colours
                            .as_ref()
                            .map(|v| colours(v, &format!("{what} roof_colours"), &mut errors)),
                        window,
                        chimney: p.chimney,
                    },
                );
            }
            styles.push(Style {
                id: s.id.clone(),
                name: s.name.clone(),
                time_zones: s.time_zones.clone(),
                bbox: s.bbox,
                min_elevation_m: s.min_elevation_m,
                presets,
            });
        }
        if styles.is_empty() {
            errors.push("no style".into());
        }
        // The default style's `other` must give every field: it is the end
        // of every lookup.
        if let Some(o) = styles.first().and_then(|s| s.presets.get("other")) {
            let complete = o.roofs.is_some()
                && o.pitch.is_some()
                && o.overhang_m.is_some()
                && o.parapet_m.is_some()
                && o.walls.as_ref().is_some_and(|w| !w.is_empty())
                && o.wall_colours.is_some()
                && o.roof_materials.as_ref().is_some_and(|w| !w.is_empty())
                && o.roof_colours.is_some()
                && o.window.is_some()
                && o.chimney.is_some();
            if !complete {
                errors.push("the default style's `other` must set every field".into());
            }
        } else {
            errors.push("the default style needs an `other` preset".into());
        }
        if errors.is_empty() {
            Ok(Self { windows, styles })
        } else {
            Err(errors)
        }
    }

    /// The style for a building: of the styles listing the package's time
    /// zone (and containing `lat_lon` in their box), those whose
    /// `min_elevation_m` the building reaches; the highest such threshold
    /// wins; else the default.
    pub fn select(&self, time_zone: Option<&str>, lat_lon: (f64, f64), elevation: f64) -> usize {
        let Some(tz) = time_zone else {
            return 0;
        };
        self.styles
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, s)| s.time_zones.iter().any(|z| z == tz))
            .filter(|(_, s)| {
                s.bbox.is_none_or(|b| {
                    (b[0]..=b[2]).contains(&lat_lon.0) && (b[1]..=b[3]).contains(&lat_lon.1)
                })
            })
            .filter(|(_, s)| s.min_elevation_m.is_none_or(|m| elevation >= m))
            .max_by(|a, b| {
                let m = |s: &Style| s.min_elevation_m.unwrap_or(f64::NEG_INFINITY);
                m(a.1).total_cmp(&m(b.1)).then(b.0.cmp(&a.0))
            })
            .map_or(0, |(i, _)| i)
    }

    /// The lookup chain of presets for a building of `kind` and `use_` in
    /// style `style`: the style's use, its kind, the default's use (a
    /// garage stays a garage in every region), the style's `other`, the
    /// default's kind, the default's `other`.
    pub fn chain(
        &self,
        style: usize,
        kind: BuildingKind,
        use_: Option<BuildingUse>,
    ) -> Vec<&Preset> {
        let mut out = Vec::new();
        let s = &self.styles[style.min(self.styles.len() - 1)];
        let d = &self.styles[0];
        let u = use_
            .filter(|u| *u != BuildingUse::Unknown)
            .map(|u| u.name());
        for (st, key) in [
            (s, u),
            (s, Some(kind_name(kind))),
            (d, u),
            (s, Some("other")),
            (d, Some(kind_name(kind))),
            (d, Some("other")),
        ] {
            if let Some(p) = key.and_then(|k| st.presets.get(k)) {
                out.push(p);
            }
        }
        out
    }
}

/// The first preset in `chain` that sets a field.
fn field<'a, T>(chain: &[&'a Preset], f: impl Fn(&'a Preset) -> Option<&'a T>) -> Option<&'a T> {
    chain.iter().find_map(|p| f(p))
}

/// A deterministic number in [0, 1) for decision `k` of a building.
pub fn chance(seed: u64, k: u64) -> f32 {
    let mut h = seed ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// The seed of a building: its first outline point.
pub fn seed_of(b: &Building) -> u64 {
    let p = b.outline.first().copied().unwrap_or([0.0, 0.0]);
    (u64::from(p[0].to_bits()) << 32) ^ u64::from(p[1].to_bits()) ^ 0x5EED
}

fn pick<T: Copy>(m: &[(T, f32)], r: f32) -> Option<T> {
    let total: f32 = m.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return None;
    }
    let mut at = r * total;
    for (v, w) in m {
        if at < *w {
            return Some(*v);
        }
        at -= w;
    }
    m.last().map(|(v, _)| *v)
}

fn pick_colour(v: &[[f32; 3]], r: f32) -> Option<[f32; 3]> {
    (!v.is_empty()).then(|| v[((r * v.len() as f32) as usize).min(v.len() - 1)])
}

/// A drawn surface: material layer and colour (linear).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    pub layer: Layer,
    pub colour: [f32; 3],
}

/// Everything the geometry needs to draw one building (heights absolute).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HouseSpec {
    pub shape: RoofShape,
    /// Roof height above the eaves (0 for flat roofs).
    pub roof_h: f32,
    /// The eaves (or the flat roof).
    pub eave_y: f32,
    /// Foot of the walls (below the ground, or above it for canopies and
    /// buildings on stilts).
    pub wall_bottom: f32,
    /// Floor of the ground storey (windows count from here).
    pub ground_y: f32,
    pub overhang: f32,
    pub parapet: f32,
    pub wall: Surface,
    /// Floors above the ground floor and the gables.
    pub upper: Surface,
    pub roof: Surface,
    /// Window style (index into [`BuildingStyles::windows`]).
    pub window: Option<u8>,
    pub levels: u8,
    pub storey_h: f32,
    pub chimney: bool,
    /// A roof on posts (`building=roof`, carports).
    pub canopy: bool,
}

impl HouseSpec {
    /// Top of the building (ridge, spire or parapet).
    pub fn top(&self) -> f32 {
        self.eave_y + self.roof_h.max(self.parapet)
    }
}

/// What the colours of the materials are (linear): the library's means,
/// else [`Layer::fallback_colour`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HouseLook {
    pub means: Option<[[f32; 3]; LAYERS]>,
}

impl HouseLook {
    pub fn natural(&self, l: Layer) -> [f32; 3] {
        match (self.means, l) {
            (_, Layer::Glass) | (None, _) => l.fallback_colour(),
            (Some(m), l) => m[l as usize],
        }
    }
}

/// Resolves a building's look from its OSM facts, its measured roof colour
/// and the preset chain of its style (see [`BuildingStyles::chain`]).
pub fn resolve(
    b: &Building,
    measured_roof: Option<RoofColour>,
    styles: &BuildingStyles,
    style: usize,
    look: &HouseLook,
) -> HouseSpec {
    let t = &b.tags;
    let chain = styles.chain(style, b.kind, t.building_use);
    let seed = seed_of(b);
    let r = |k: u64| chance(seed, k);
    let canopy = t.building_use == Some(BuildingUse::Canopy);
    let ground_y = b.base_y + SINK_M;
    // Roof shape and height.
    let rect = matches!(b.roof, Roof::Gabled { .. }) && b.outline.len() == 4;
    let bldg_h = match b.roof {
        Roof::Gabled { height_m } => height_m,
        Roof::Flat => 0.0,
    };
    let (mut shape, mut roof_h, tagged_h) = if rect {
        let shape = t.roof_shape.filter(|s| s.is_pitched()).unwrap_or_else(|| {
            field(&chain, |p| p.roofs.as_ref())
                .and_then(|m| pick(m, r(1)))
                .unwrap_or(RoofShape::Gabled)
        });
        match t.roof_height_m {
            Some(h) => (shape, h, true),
            None => {
                let pitch = *field(&chain, |p| p.pitch.as_ref()).unwrap_or(&1.0);
                (shape, bldg_h * pitch.clamp(0.2, 3.0), false)
            }
        }
    } else if t.roof_shape == Some(RoofShape::Pyramidal)
        && let Some(h) = t.roof_height_m
    {
        (RoofShape::Pyramidal, h, true)
    } else {
        (RoofShape::Flat, 0.0, false)
    };
    if !shape.is_pitched() {
        roof_h = 0.0;
        shape = RoofShape::Flat;
    }
    // The eaves come down for the big farm roofs (untagged heights only).
    let mut eave_y = b.eave_y;
    if shape.is_pitched() && !tagged_h {
        let drop = field(&chain, |p| p.eave_drop_m.as_ref())
            .copied()
            .unwrap_or(0.0)
            .min(b.eave_y - ground_y - MIN_EAVE_M)
            .max(0.0);
        eave_y -= drop;
        roof_h += drop;
    }
    let min_h = t.min_height_m.unwrap_or(0.0);
    let wall_bottom = if canopy {
        // A slab: the roof's own thickness.
        eave_y - 0.35
    } else if min_h > 0.0 {
        (b.base_y + SINK_M + min_h).min(eave_y - 0.5)
    } else {
        b.base_y
    };
    // Walls.
    let natural = |l: Layer| look.natural(l);
    let wall_mat = t
        .wall
        .or_else(|| field(&chain, |p| p.walls.as_ref()).and_then(|m| pick(m, r(2))))
        .unwrap_or(WallMaterial::Plaster);
    let wall_layer = Layer::of_wall(wall_mat);
    let plaster_colour = field(&chain, |p| p.wall_colours.as_ref())
        .and_then(|v| pick_colour(v, r(3)))
        .map(linear);
    let wall_colour = match (t.wall_colour, wall_layer) {
        (Some(c), _) => linear(c.map(|v| f32::from(v) / 255.0)),
        (None, Layer::Plaster) => plaster_colour.unwrap_or(natural(Layer::Plaster)),
        (None, l) => natural(l),
    };
    let wall = Surface {
        layer: wall_layer,
        colour: wall_colour,
    };
    let upper = match field(&chain, |p| p.upper.as_ref()) {
        // The upper floors only change when the walls are not tagged.
        Some(m) if t.wall.is_none() && t.wall_colour.is_none() => {
            let total: f32 = m.iter().map(|(_, w)| w).sum();
            if r(4) < total.min(1.0) {
                let up = pick(m, r(5)).map(Layer::of_wall).unwrap_or(wall_layer);
                if up == wall_layer {
                    wall
                } else {
                    Surface {
                        layer: up,
                        colour: natural(up),
                    }
                }
            } else {
                wall
            }
        }
        _ => wall,
    };
    // Roof.
    let roof_mat = t
        .roof_material
        .or_else(|| field(&chain, |p| p.roof_materials.as_ref()).and_then(|m| pick(m, r(6))))
        .unwrap_or(RoofMaterial::Tiles);
    let (roof_layer, roof_tint) = Layer::of_roof(roof_mat);
    let default_roof = match (roof_tint, roof_layer) {
        (Some(c), _) => c,
        (None, Layer::RoofTiles) => field(&chain, |p| p.roof_colours.as_ref())
            .and_then(|v| pick_colour(v, r(7)))
            .unwrap_or(srgb(natural(Layer::RoofTiles))),
        (None, l) => srgb(natural(l)),
    };
    // Measured (ROOF) first, then the tagged roof:colour like an OSM one.
    let measured = measured_roof.or(t.roof_colour.map(|rgb| RoofColour {
        rgb,
        origin: ocw_world::look::RoofOrigin::Osm,
    }));
    let roof = Surface {
        layer: roof_layer,
        colour: linear(crate::region_look::roof_colour(measured, default_roof)),
    };
    // Storeys and windows.
    let wall_h = (eave_y - ground_y).max(1.0);
    let levels = t
        .levels
        .map(|l| l.min(60))
        .unwrap_or_else(|| ((wall_h / 3.0).round() as u8).max(1))
        .max(1);
    let storey_h = (wall_h / f32::from(levels)).clamp(2.2, 6.0);
    let window = if canopy {
        None
    } else {
        field(&chain, |p| p.window.as_ref()).copied().flatten()
    };
    let chimney = shape.is_pitched()
        && !canopy
        && r(8)
            < field(&chain, |p| p.chimney.as_ref())
                .copied()
                .unwrap_or(0.0);
    let overhang = if shape.is_pitched() {
        field(&chain, |p| p.overhang_m.as_ref())
            .copied()
            .unwrap_or(0.4)
            .clamp(0.0, 2.0)
    } else {
        0.0
    };
    let parapet = if shape.is_pitched() || canopy {
        0.0
    } else {
        field(&chain, |p| p.parapet_m.as_ref())
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.5)
    };
    HouseSpec {
        shape,
        roof_h,
        eave_y,
        wall_bottom,
        ground_y,
        overhang,
        parapet,
        wall,
        upper,
        roof,
        window,
        levels,
        storey_h,
        chimney,
        canopy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::buildings::BuildingTags;

    fn house(kind: BuildingKind, tags: BuildingTags, at: [f32; 2]) -> Building {
        Building {
            kind,
            outline: vec![
                at,
                [at[0] + 12.0, at[1]],
                [at[0] + 12.0, at[1] + 8.0],
                [at[0], at[1] + 8.0],
            ],
            holes: vec![],
            base_y: 449.5,
            eave_y: 456.0,
            roof: Roof::Gabled { height_m: 3.2 },
            tags,
        }
    }

    fn style(id: &str) -> usize {
        STYLES.styles.iter().position(|s| s.id == id).unwrap()
    }

    const LOOK: HouseLook = HouseLook { means: None };

    /// The shipped file parses, every window fits the shader and every
    /// region has its style.
    #[test]
    fn the_shipped_styles_are_valid() {
        let s = BuildingStyles::parse(STYLES_TOML).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(s.styles[0].id, "central");
        assert!(s.windows.len() <= MAX_WINDOW_STYLES);
        for id in ["alpine", "blackforest", "canary"] {
            assert!(s.styles.iter().any(|x| x.id == id), "{id}");
        }
        // Mistakes are reported, not ignored.
        let bad = STYLES_TOML.replace("half-hipped = 0.1", "half_hiped = 0.1");
        let e = BuildingStyles::parse(&bad).unwrap_err();
        assert!(e.iter().any(|e| e.contains("half_hiped")), "{e:?}");
        let bad = STYLES_TOML.replace("window = \"farm\"", "window = \"barnyard\"");
        assert!(BuildingStyles::parse(&bad).is_err());
        assert!(BuildingStyles::parse("[[style]]\nid = \"x\"\nname = \"x\"\n").is_err());
    }

    /// Styles by time zone, box and elevation (spec §14 Phase 8 T4).
    #[test]
    fn styles_are_selected_by_region_and_elevation() {
        let s = &*STYLES;
        let vaduz = (47.14, 9.52);
        assert_eq!(
            s.select(Some("Europe/Vaduz"), vaduz, 455.0),
            0,
            "Rhine valley"
        );
        assert_eq!(
            s.select(Some("Europe/Vaduz"), vaduz, 1600.0),
            style("alpine")
        );
        assert_eq!(
            s.select(Some("Europe/Zurich"), vaduz, 900.0),
            style("alpine")
        );
        let bf = (47.95, 8.10);
        assert_eq!(
            s.select(Some("Europe/Berlin"), bf, 900.0),
            style("blackforest")
        );
        assert_eq!(s.select(Some("Europe/Berlin"), bf, 280.0), 0, "Freiburg");
        assert_eq!(
            s.select(Some("Europe/Berlin"), (52.5, 13.4), 900.0),
            0,
            "outside the box"
        );
        assert_eq!(
            s.select(Some("Atlantic/Canary"), (28.3, -16.5), 50.0),
            style("canary")
        );
        assert_eq!(s.select(None, vaduz, 1600.0), 0);
        assert_eq!(s.select(Some("Asia/Tokyo"), (35.0, 139.0), 0.0), 0);
    }

    #[test]
    fn presets_are_looked_up_along_the_chain() {
        let s = &*STYLES;
        // A Black Forest barn: the barn preset, then farm, then the default's barn …
        let chain = s.chain(
            style("blackforest"),
            BuildingKind::Farm,
            Some(BuildingUse::Barn),
        );
        assert!(chain.len() >= 4);
        assert_eq!(chain[0].eave_drop_m, Some(1.5));
        assert_eq!(chain[1].eave_drop_m, Some(2.5));
        // The default's `other` ends every chain.
        let chain = s.chain(style("canary"), BuildingKind::Public, None);
        assert_eq!(chain.last().unwrap(), &&s.styles[0].presets["other"]);
    }

    #[test]
    fn tagged_facts_win_over_the_preset() {
        let s = &*STYLES;
        let tags = BuildingTags {
            roof_shape: Some(RoofShape::Mansard),
            roof_height_m: Some(5.0),
            levels: Some(3),
            wall: Some(WallMaterial::Brick),
            wall_colour: Some([200, 60, 50]),
            roof_material: Some(RoofMaterial::Slate),
            ..Default::default()
        };
        let b = house(BuildingKind::House, tags, [10.0, 20.0]);
        // Even in Tenerife (flat roofs by default) the tagged mansard stays.
        let h = resolve(&b, None, s, style("canary"), &LOOK);
        assert_eq!(h.shape, RoofShape::Mansard);
        assert_eq!(h.roof_h, 5.0);
        assert_eq!(h.eave_y, 456.0, "no eave drop on a tagged height");
        assert_eq!(h.levels, 3);
        assert!((h.storey_h - 6.0 / 3.0).max(2.2) - h.storey_h < 1e-6);
        assert_eq!(h.wall.layer, Layer::Brick);
        assert_eq!(
            h.wall.colour,
            linear([200.0 / 255.0, 60.0 / 255.0, 50.0 / 255.0])
        );
        assert_eq!(h.upper, h.wall, "tagged walls have no other upper floors");
        assert_eq!(h.roof.layer, Layer::RoofSlates);
        // A tagged flat roof stays flat (the BLDG outline is not a rectangle then).
        let flat = Building {
            roof: Roof::Flat,
            ..house(BuildingKind::House, BuildingTags::default(), [0.0, 0.0])
        };
        let h = resolve(&flat, None, s, style("alpine"), &LOOK);
        assert_eq!((h.shape, h.roof_h), (RoofShape::Flat, 0.0));
        // A pyramid on a polygon (spire) with its height.
        let tower = Building {
            roof: Roof::Flat,
            outline: (0..8)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::FRAC_PI_4;
                    [3.0 * a.cos(), 3.0 * a.sin()]
                })
                .collect(),
            tags: BuildingTags {
                roof_shape: Some(RoofShape::Pyramidal),
                roof_height_m: Some(12.0),
                ..Default::default()
            },
            ..house(BuildingKind::Religious, BuildingTags::default(), [0.0, 0.0])
        };
        let h = resolve(&tower, None, s, 0, &LOOK);
        assert_eq!((h.shape, h.roof_h), (RoofShape::Pyramidal, 12.0));
        assert_eq!(h.top(), 456.0 + 12.0);
    }

    /// Untagged buildings take their region's preset: flat roofs with a
    /// parapet in Tenerife, low pitch and wide eaves in the Alps, the big
    /// roof coming down on a Black Forest farm.
    #[test]
    fn regional_defaults_shape_untagged_houses() {
        let s = &*STYLES;
        let many = |kind: BuildingKind, st: usize| -> Vec<HouseSpec> {
            (0..200)
                .map(|i| {
                    let b = house(kind, BuildingTags::default(), [i as f32 * 37.0, 11.0]);
                    resolve(&b, None, s, st, &LOOK)
                })
                .collect()
        };
        let canary = many(BuildingKind::House, style("canary"));
        let flat = canary.iter().filter(|h| h.shape == RoofShape::Flat).count();
        assert!((130..=190).contains(&flat), "{flat} of 200 flat");
        assert!(
            canary
                .iter()
                .filter(|h| h.shape == RoofShape::Flat)
                .all(|h| h.parapet > 0.5)
        );
        let alpine = many(BuildingKind::House, style("alpine"));
        assert!(alpine.iter().all(|h| h.shape == RoofShape::Gabled));
        assert!(alpine.iter().all(|h| (h.roof_h - 3.2 * 0.6).abs() < 1e-5));
        assert!(alpine.iter().all(|h| h.overhang >= 1.0));
        assert!(alpine.iter().any(|h| h.upper.layer == Layer::Timber));
        let farms = many(BuildingKind::Farm, style("blackforest"));
        assert!(
            farms
                .iter()
                .all(|h| matches!(h.shape, RoofShape::Hipped | RoofShape::HalfHipped))
        );
        // Eaves 2.5 m lower, the ridge as high as the roof's pitch makes it.
        let h = &farms[0];
        assert!((h.eave_y - (456.0 - 2.5)).abs() < 1e-4);
        assert!((h.eave_y + h.roof_h - (456.0 + 3.2 * 1.25)).abs() < 1e-4);
        // Central: gabled, hipped and half-hipped houses, clay tile colours varied.
        let central = many(BuildingKind::House, 0);
        for shape in [RoofShape::Gabled, RoofShape::Hipped, RoofShape::HalfHipped] {
            assert!(central.iter().any(|h| h.shape == shape), "{shape:?}");
        }
        let colours: std::collections::BTreeSet<String> = central
            .iter()
            .map(|h| format!("{:?}", h.roof.colour))
            .collect();
        assert!(colours.len() >= 4);
        // Deterministic: the same house resolves the same.
        assert_eq!(many(BuildingKind::House, 0), central);
    }

    #[test]
    fn roof_colours_measured_first() {
        let s = &*STYLES;
        let b = house(BuildingKind::House, BuildingTags::default(), [0.0, 0.0]);
        let measured = RoofColour {
            rgb: [90, 90, 90],
            origin: ocw_world::look::RoofOrigin::Orthophoto,
        };
        let h = resolve(&b, Some(measured), s, 0, &LOOK);
        assert_eq!(h.roof.colour, linear([90.0 / 255.0; 3]));
        // A tagged roof:colour without a measurement: a quarter towards the default.
        let red = Building {
            tags: BuildingTags {
                roof_colour: Some([255, 0, 0]),
                ..Default::default()
            },
            ..b.clone()
        };
        let h = resolve(&red, None, s, 0, &LOOK);
        let c = srgb(h.roof.colour);
        assert!(c[0] > 0.8 && c[1] < 0.2, "{c:?}");
    }

    #[test]
    fn canopies_storeys_and_stilts() {
        let s = &*STYLES;
        let canopy = Building {
            kind: BuildingKind::Small,
            roof: Roof::Flat,
            tags: BuildingTags {
                building_use: Some(BuildingUse::Canopy),
                ..Default::default()
            },
            ..house(BuildingKind::Small, BuildingTags::default(), [0.0, 0.0])
        };
        let h = resolve(&canopy, None, s, 0, &LOOK);
        assert!(h.canopy && h.window.is_none() && h.parapet == 0.0);
        assert!((h.wall_bottom - (h.eave_y - 0.35)).abs() < 1e-5);
        // Storeys from the wall height (6 m: two of 3 m) when untagged.
        let b = house(
            BuildingKind::Apartments,
            BuildingTags::default(),
            [5.0, 5.0],
        );
        let h = resolve(&b, None, s, 0, &LOOK);
        assert_eq!(h.levels, 2);
        assert!((h.storey_h - 3.0).abs() < 1e-5);
        assert_eq!(
            h.window,
            STYLES
                .windows
                .iter()
                .position(|w| w.id == "apartments")
                .map(|i| i as u8)
        );
        // On stilts: walls from min_height.
        let stilts = Building {
            tags: BuildingTags {
                min_height_m: Some(3.0),
                ..Default::default()
            },
            ..b
        };
        let h = resolve(&stilts, None, s, 0, &LOOK);
        assert!((h.wall_bottom - (450.0 + 3.0)).abs() < 1e-5);
    }

    #[test]
    fn chance_is_uniform_enough_and_picks_follow_weights() {
        let n = 10_000;
        let mean: f32 = (0..n).map(|i| chance(i, 3)).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        assert!((0..n).all(|i| (0.0..1.0).contains(&chance(i, 7))));
        let m = [(1, 3.0), (2, 1.0)];
        assert_eq!(pick(&m, 0.0), Some(1));
        assert_eq!(pick(&m, 0.74), Some(1));
        assert_eq!(pick(&m, 0.76), Some(2));
        assert_eq!(pick::<u8>(&[], 0.5), None);
        assert_eq!(parse_hex("#fff"), Some([1.0; 3]));
        assert_eq!(parse_hex("#000000"), Some([0.0; 3]));
        assert_eq!(parse_hex("red"), None);
    }
}
