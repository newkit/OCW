//! The region's measured look in the renderer (spec §14 Phase 8 T3): the
//! package's palette, seasonal tint maps and roof colours (`ocw_world::look`)
//! fed into the T1/T2 tint hooks of the ground materials.
//!
//! - **Near:** each ground layer is tinted towards the measured colour of
//!   the land class it mostly stands for ([`layer_tints`]: meadow → the
//!   meadow layer, forest → the forest floor, …), damped so the texture's
//!   own variation stays, and times the class's seasonal change
//!   ([`season_factors`]: the class's median colour in the blended season
//!   over its summer colour).
//! - **Far:** the colour map's class colours get the same tints
//!   ([`class_tints`], so near and far agree where they fade), then the
//!   tile's Sentinel-2 map of the world date is laid over them
//!   ([`SATELLITE_WEIGHT`]): the real patchwork of fields and forests, the
//!   autumn colours, the snow in winter.
//! - **Roofs:** the sampled colour of each building ([`roof_colour`]).
//!
//! The world date comes from the sky (`ocw-sky`, the session's world clock):
//! [`season_bucket`] changes every few days, and the tiles are built anew
//! when it does.
//!
//! Everything here is pure and unit-tested; `package.rs` calls it.

use ocw_world::landuse::LandClass;
use ocw_world::look::{Palette, PaletteKey, RegionLook, RoofColour, RoofOrigin, Season};

use crate::ground::{ClassMix, GROUND_LAYERS};

/// How far the near layers move towards the measured colour, in chroma
/// (the texture keeps its own variation) …
pub const CHROMA_STRENGTH: f32 = 0.8;
/// … and in brightness (aerial photos are taken in full sun from above).
pub const LUMA_STRENGTH: f32 = 0.5;
/// The forest is measured from above (the canopy), the layer is the floor
/// beneath it: only a light pull.
pub const FOREST_STRENGTH: f32 = 0.35;
/// Tints stay within this factor range.
pub const TINT_RANGE: (f32, f32) = (0.4, 2.5);
/// The seasonal near factor follows the class colour this much in chroma
/// and in brightness, within [`SEASON_RANGE`].
pub const SEASON_CHROMA: f32 = 0.25;
pub const SEASON_LUMA: f32 = 0.5;
pub const SEASON_RANGE: (f32, f32) = (0.6, 1.6);
/// Share of the satellite colour in the far colour map.
pub const SATELLITE_WEIGHT: f32 = 0.7;
/// Days per season bucket: the tiles are rebuilt when the date moves on
/// by this much (time machine, long sessions).
pub const BUCKET_DAYS: u32 = 4;

/// sRGB (0–255) to linear.
pub fn lin(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lin3(c: [u8; 3]) -> [f32; 3] {
    c.map(lin)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The factor that moves `from` (linear) towards `to` (linear): chroma by
/// `chroma`, brightness by `luma_k`, clamped to [`TINT_RANGE`].
pub fn tint_towards(from: [f32; 3], to: [f32; 3], chroma: f32, luma_k: f32) -> [f32; 3] {
    let (lf, lt) = (luma(from).max(1e-4), luma(to).max(1e-4));
    let l = lt / lf;
    std::array::from_fn(|i| {
        let ratio = to[i].max(1e-4) / from[i].max(1e-4);
        let chroma_part = (ratio / l).powf(chroma);
        (chroma_part * l.powf(luma_k)).clamp(TINT_RANGE.0, TINT_RANGE.1)
    })
}

/// Which measured colour a ground layer is tinted towards (first found),
/// and how strongly; `None`: the layer stays neutral (snow, the
/// gravel of yards and car parks).
fn layer_keys(layer: usize) -> (&'static [PaletteKey], f32) {
    use crate::ground::*;
    use LandClass::*;
    match layer {
        MEADOW => (&[PaletteKey::Land(Meadow), PaletteKey::Land(Default)], 1.0),
        FARMLAND => (&[PaletteKey::Land(Farmland)], 1.0),
        FOREST_FLOOR => (&[PaletteKey::Land(Forest)], FOREST_STRENGTH),
        ROCK => (&[PaletteKey::Land(Rock), PaletteKey::Land(Scree)], 1.0),
        SCREE => (&[PaletteKey::Land(Scree), PaletteKey::Land(Rock)], 1.0),
        SAND => (&[PaletteKey::Land(Bare)], 0.8),
        WETLAND => (&[PaletteKey::Land(Wetland)], 0.8),
        _ => (&[], 0.0),
    }
}

/// The land class whose season a layer follows.
fn layer_class(layer: usize) -> Option<LandClass> {
    use crate::ground::*;
    use LandClass::*;
    match layer {
        MEADOW => Some(Meadow),
        FARMLAND => Some(Farmland),
        FOREST_FLOOR => Some(Forest),
        ROCK => Some(Rock),
        SCREE => Some(Scree),
        SAND => Some(Bare),
        WETLAND => Some(Wetland),
        _ => None,
    }
}

/// Near tint per ground layer from the measured palette; `means` are the
/// layers' mean albedos (linear). Neutral (1) without a measurement.
pub fn layer_tints(
    palette: &Palette,
    means: &[[f32; 3]; GROUND_LAYERS],
) -> [[f32; 3]; GROUND_LAYERS] {
    std::array::from_fn(|layer| {
        let (keys, strength) = layer_keys(layer);
        keys.iter()
            .find_map(|k| palette.colour(*k))
            .map_or([1.0; 3], |c| {
                tint_towards(
                    means[layer],
                    lin3(c),
                    CHROMA_STRENGTH * strength,
                    LUMA_STRENGTH * strength,
                )
            })
    })
}

/// The seasonal factor per ground layer at the blend `weights` of the
/// composites: the layer's class colour in the blended season over its
/// summer colour (damped like the palette tint). Neutral without seasons
/// or a summer composite.
pub fn season_factors(look: &RegionLook, weights: &[(usize, f32)]) -> [[f32; 3]; GROUND_LAYERS] {
    let Some(s) = look.seasons.as_ref() else {
        return [[1.0; 3]; GROUND_LAYERS];
    };
    let Some(summer) = s.index_of(Season::Summer) else {
        return [[1.0; 3]; GROUND_LAYERS];
    };
    std::array::from_fn(|layer| {
        let Some(class) = layer_class(layer) else {
            return [1.0; 3];
        };
        let Some(base) = s.class_mean(summer, class).map(lin3) else {
            return [1.0; 3];
        };
        let mut now = [0.0f32; 3];
        let mut total = 0.0;
        for &(i, w) in weights {
            if let Some(c) = s.class_mean(i, class).map(lin3) {
                for k in 0..3 {
                    now[k] += w * c[k];
                }
                total += w;
            }
        }
        if total <= 0.0 {
            return [1.0; 3];
        }
        let now = now.map(|v| v / total);
        // Mostly brightness: a factor cannot turn green grass into snow (the
        // first winter renders went pink), the far satellite map carries the
        // real colour.
        tint_towards(base, now, SEASON_CHROMA, SEASON_LUMA)
            .map(|v| v.clamp(SEASON_RANGE.0, SEASON_RANGE.1))
    })
}

/// The near tint per layer: palette tint × seasonal factor.
pub fn near_tints(
    look: &RegionLook,
    means: &[[f32; 3]; GROUND_LAYERS],
    weights: &[(usize, f32)],
) -> [[f32; 3]; GROUND_LAYERS] {
    let p = look
        .palette
        .as_ref()
        .map_or([[1.0; 3]; GROUND_LAYERS], |p| layer_tints(p, means));
    let s = season_factors(look, weights);
    std::array::from_fn(|i| {
        std::array::from_fn(|k| (p[i][k] * s[i][k]).clamp(TINT_RANGE.0, TINT_RANGE.1))
    })
}

/// The far tint of each land class that matches the near layers: its mix
/// of tinted layer means over its mix of plain means.
pub fn class_tints(
    mix: &ClassMix,
    means: &[[f32; 3]; GROUND_LAYERS],
    tints: &[[f32; 3]; GROUND_LAYERS],
) -> [[f32; 3]; 17] {
    std::array::from_fn(|c| {
        std::array::from_fn(|k| {
            let (mut a, mut b) = (0.0, 0.0);
            for i in 0..GROUND_LAYERS {
                a += mix[c][i] * means[i][k] * tints[i][k];
                b += mix[c][i] * means[i][k];
            }
            if b > 0.0 { a / b } else { 1.0 }
        })
    })
}

/// The season bucket of a day of the year (changes every [`BUCKET_DAYS`]).
pub fn season_bucket(day_of_year: u32) -> u32 {
    day_of_year.saturating_sub(1) / BUCKET_DAYS
}

/// The day of the year a bucket stands for (its middle).
pub fn bucket_day(bucket: u32) -> f64 {
    f64::from(bucket * BUCKET_DAYS) + f64::from(BUCKET_DAYS) / 2.0 + 0.5
}

/// The far colour of a cell (sRGB 0–1): the class colour `class_rgb` with
/// the satellite colour laid over it.
pub fn far_colour(class_rgb: [f32; 3], satellite: Option<[u8; 3]>) -> [f32; 3] {
    match satellite {
        Some(s) => std::array::from_fn(|k| {
            class_rgb[k] + (f32::from(s[k]) / 255.0 - class_rgb[k]) * SATELLITE_WEIGHT
        }),
        None => class_rgb,
    }
}

/// The default roof colour (sRGB 0–1) in a building's measured colour:
/// the sampled colour as it is; an OSM `roof:colour` (often a pure named
/// colour like "red") a quarter of the way to the default, so it reads as
/// a material, not paint.
pub fn roof_colour(measured: Option<RoofColour>, default: [f32; 3]) -> [f32; 3] {
    match measured {
        None => default,
        Some(RoofColour {
            rgb,
            origin: RoofOrigin::Orthophoto,
        }) => rgb.map(|v| f32::from(v) / 255.0),
        Some(RoofColour {
            rgb,
            origin: RoofOrigin::Osm,
        }) => std::array::from_fn(|k| 0.75 * f32::from(rgb[k]) / 255.0 + 0.25 * default[k]),
    }
}

/// Blends the decoded seasonal maps of a tile by `weights` (sRGB bytes).
pub fn blend_maps(maps: &[(Vec<[u8; 3]>, f32)]) -> Option<Vec<[u8; 3]>> {
    let n = maps.first()?.0.len();
    if maps.iter().any(|m| m.0.len() != n) {
        return None;
    }
    Some(
        (0..n)
            .map(|j| {
                let mut acc = [0.0f32; 3];
                for (m, w) in maps {
                    for k in 0..3 {
                        acc[k] += w * f32::from(m[j][k]);
                    }
                }
                acc.map(|v| v.round().clamp(0.0, 255.0) as u8)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::{FOREST_FLOOR, MEADOW, SNOW, default_mix};
    use ocw_world::look::{PaletteEntry, SeasonInfo, SeasonalTints};

    fn palette(entries: &[(PaletteKey, [u8; 3])]) -> Palette {
        Palette {
            sources: vec![],
            entries: entries
                .iter()
                .map(|&(key, rgb)| PaletteEntry {
                    key,
                    rgb,
                    samples: 1000,
                    source: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn tints_move_towards_the_measured_colour() {
        let grey = [0.2, 0.2, 0.2];
        let green = [0.12, 0.3, 0.1];
        let full = tint_towards(grey, green, 1.0, 1.0);
        for k in 0..3 {
            assert!((grey[k] * full[k] - green[k]).abs() < 1e-4, "{full:?}");
        }
        // Damped: part of the way, and the same colour is neutral.
        let half = tint_towards(grey, green, 0.5, 0.5);
        assert!(half[1] > 1.0 && half[1] < full[1]);
        assert_eq!(
            tint_towards(green, green, 0.8, 0.5).map(|v| (v * 1e4).round()),
            [1e4; 3]
        );
        // Never beyond the range, even for black.
        let wild = tint_towards([0.5; 3], [0.0, 1.0, 0.0], 1.0, 1.0);
        assert!(
            wild.iter()
                .all(|v| *v >= TINT_RANGE.0 && *v <= TINT_RANGE.1)
        );
    }

    #[test]
    fn layers_follow_their_classes_and_the_rest_stays_neutral() {
        let means = [[0.2, 0.2, 0.2]; GROUND_LAYERS];
        let p = palette(&[
            (PaletteKey::Land(LandClass::Meadow), [90, 140, 60]),
            (PaletteKey::Land(LandClass::Forest), [30, 60, 30]),
        ]);
        let t = layer_tints(&p, &means);
        // Meadow greener, the forest floor pulled less than the meadow.
        assert!(t[MEADOW][1] > t[MEADOW][0]);
        let pull = |v: [f32; 3]| v.iter().map(|x| (x - 1.0).abs()).sum::<f32>();
        assert!(
            pull(t[FOREST_FLOOR])
                < pull(tint_towards(
                    means[FOREST_FLOOR],
                    lin3([30, 60, 30]),
                    CHROMA_STRENGTH,
                    LUMA_STRENGTH
                ))
        );
        // Snow and unmeasured layers neutral.
        assert_eq!(t[SNOW], [1.0; 3]);
        assert_eq!(t[crate::ground::ROCK], [1.0; 3]);
        // Meadow falls back to the default class.
        let only_default = palette(&[(PaletteKey::Land(LandClass::Default), [90, 140, 60])]);
        assert_eq!(layer_tints(&only_default, &means)[MEADOW], t[MEADOW]);
    }

    fn seasons() -> RegionLook {
        let info = |season, centre| SeasonInfo {
            season,
            centre_doy: centre,
            first_doy: 1,
            last_doy: 2,
            scenes: 1,
        };
        RegionLook {
            seasons: Some(SeasonalTints {
                cells: 1,
                source: 0,
                seasons: vec![info(Season::Summer, 200), info(Season::Winter, 15)],
                class_means: vec![
                    vec![(LandClass::Meadow, [80, 130, 60])],
                    vec![(LandClass::Meadow, [110, 140, 100])],
                ],
                tiles: Default::default(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn the_season_changes_the_near_layers() {
        let look = seasons();
        let s = look.seasons.as_ref().unwrap();
        // Summer: neutral; winter: brighter, browner, less green.
        let summer = season_factors(&look, &s.weights(200.0));
        assert!(
            summer[MEADOW].iter().all(|v| (v - 1.0).abs() < 1e-4),
            "{summer:?}"
        );
        let winter = season_factors(&look, &s.weights(15.0));
        assert!(
            winter[MEADOW][0] > 1.15 && winter[MEADOW][2] > winter[MEADOW][1],
            "{winter:?}"
        );
        // Regression (first renders, 2026-11-13): a snowy winter median over
        // green summer grass turned the near meadow pink (red × 2.5). Now the
        // factor stays nearly grey: brighter, channels within 25 %.
        let mut snowy = look.clone();
        snowy.seasons.as_mut().unwrap().class_means[1] = vec![(LandClass::Meadow, [235, 235, 240])];
        let snow = season_factors(&snowy, &s.weights(15.0))[MEADOW];
        let (lo, hi) = (
            snow.iter().copied().fold(9.0, f32::min),
            snow.iter().copied().fold(0.0, f32::max),
        );
        assert!(
            lo > 1.2 && hi <= SEASON_RANGE.1 && hi / lo < 1.25,
            "{snow:?}"
        );
        // A layer without a measured class stays neutral.
        assert_eq!(winter[FOREST_FLOOR], [1.0; 3]);
        // Half-way between: in between.
        let mid = season_factors(&look, &s.weights(290.0));
        assert!(mid[MEADOW][0] > 1.0 && mid[MEADOW][0] < winter[MEADOW][0]);
        // Without seasons all neutral.
        assert_eq!(
            season_factors(&RegionLook::default(), &[]),
            [[1.0; 3]; GROUND_LAYERS]
        );
    }

    #[test]
    fn far_class_tints_match_the_near_layers() {
        let mix = default_mix();
        let means = [[0.2, 0.25, 0.1]; GROUND_LAYERS];
        let mut tints = [[1.0; 3]; GROUND_LAYERS];
        tints[MEADOW] = [1.2, 1.1, 0.9];
        let c = class_tints(&mix, &means, &tints);
        // Meadow is all meadow layer: the same tint; glacier untouched.
        assert!((c[LandClass::Meadow as usize][0] - 1.2).abs() < 1e-5);
        assert_eq!(c[LandClass::Glacier as usize], [1.0; 3]);
        // Farmland (45 % meadow): part of it.
        let f = c[LandClass::Farmland as usize][0];
        assert!(f > 1.0 && f < 1.2, "{f}");
    }

    #[test]
    fn season_buckets_and_far_colours() {
        assert_eq!(season_bucket(1), 0);
        assert_eq!(season_bucket(4), 0);
        assert_eq!(season_bucket(5), 1);
        assert_eq!(season_bucket(366), 91);
        assert!((bucket_day(0) - 2.5).abs() < 1e-9);
        assert_eq!(far_colour([0.5; 3], None), [0.5; 3]);
        let f = far_colour([0.0; 3], Some([255, 0, 0]));
        assert!((f[0] - SATELLITE_WEIGHT).abs() < 1e-6 && f[1] == 0.0);
        let maps = vec![(vec![[100, 0, 0]; 2], 0.25), (vec![[200, 40, 0]; 2], 0.75)];
        assert_eq!(blend_maps(&maps).unwrap(), vec![[175, 30, 0]; 2]);
        assert!(blend_maps(&[]).is_none());
    }

    #[test]
    fn roofs_take_their_measured_colour() {
        let def = [0.5, 0.25, 0.2];
        assert_eq!(roof_colour(None, def), def);
        let o = roof_colour(
            Some(RoofColour {
                rgb: [51, 102, 153],
                origin: RoofOrigin::Orthophoto,
            }),
            def,
        );
        assert!((o[0] - 0.2).abs() < 1e-6 && (o[2] - 0.6).abs() < 1e-6);
        let osm = roof_colour(
            Some(RoofColour {
                rgb: [255, 0, 0],
                origin: RoofOrigin::Osm,
            }),
            def,
        );
        assert!((osm[0] - (0.75 + 0.125)).abs() < 1e-6 && (osm[1] - 0.0625).abs() < 1e-6);
    }
}
