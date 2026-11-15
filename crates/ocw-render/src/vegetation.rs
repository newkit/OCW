//! A tile's plants (spec §4.6.1, §14 Phase 8 T8): the trees of the tree
//! grid, the mapped trees and the plantations' rows of section `VEGE`
//! (`ocw_world::plantings`), each with its species
//! (`ocw_world::flora::pick_species`: the measured leaf type or the
//! seasons' regional mix decides the leaf group, the regional tables the
//! species), drawn with the species models of [`crate::flora`] in chunks of
//! [`CHUNK_M`]: every chunk has a near mesh (detailed models, vines,
//! banana plants, the stands of maize and cereals) drawn up to
//! [`NEAR_FADE_M`] and a far mesh (simple models) drawn beyond it, the two
//! cross-faded (dithered) by their visibility ranges.

use bevy::prelude::*;
use ocw_sky::season::{Crop, CropStage, LeafType, SeasonDay};
use ocw_world::flora::{FloraRegion, LeafGroup, Setting, Site, Species, pick_species, stature};
use ocw_world::landuse::LandClass;
use ocw_world::plantings;
use ocw_world::trees::hash;
use ocw_world::vegetation::FieldCrop;
use ocw_world::{TileKey, WorldPackage};

use crate::flora::{Detail, PlantAt};
use crate::package::{Keep, MeshArrays, TreeSite};

/// Chunks per tile side (200 m chunks for 1 km tiles).
pub const CHUNKS: u32 = 5;
/// Detail chunks per tile side (100 m): the plants drawn only up close –
/// banana plants, vine rows – in their own smaller chunks with a shorter
/// range ([`DETAIL_FADE_M`]); beyond, the colour map shows the plantation.
pub const DETAIL_CHUNKS: u32 = 10;
/// The detail plants fade out between these distances from the camera to
/// a detail chunk's centre (m).
pub const DETAIL_FADE_M: std::ops::Range<f32> = 130.0..180.0;
/// The near models give way to the far ones between these distances
/// from the camera to a chunk's centre (m).
pub const NEAR_FADE_M: std::ops::Range<f32> = 230.0..310.0;

/// The plants of one chunk of a tile; positions relative to `origin`.
#[derive(Default)]
pub struct TreeChunk {
    pub origin: [f32; 3],
    pub near: MeshArrays,
    pub far: MeshArrays,
    /// Trees (grid, mapped and orchard trees) in the chunk.
    pub trees: usize,
}

/// A tile's plants: its chunks (empty ones left out) and the crowns (x,
/// z, radius, top) for the flight's obstacles.
#[derive(Default)]
pub struct TilePlants {
    pub chunks: Vec<TreeChunk>,
    /// The detail chunks (only their `near` mesh; [`DETAIL_CHUNKS`]).
    pub details: Vec<TreeChunk>,
    pub crowns: Vec<[f32; 4]>,
}

impl TilePlants {
    pub fn trees(&self) -> usize {
        self.chunks.iter().map(|c| c.trees).sum()
    }
}

/// The leaf group of a tree from the seasons' leaf type.
fn group(leaf: LeafType) -> LeafGroup {
    match leaf {
        LeafType::Evergreen => LeafGroup::Evergreen,
        LeafType::Deciduous => LeafGroup::Deciduous,
        LeafType::Larch => LeafGroup::Larch,
    }
}

/// Whether water or wetland lies within 15 m of (x, z) (alders and
/// willows on the banks).
fn near_water(pkg: &WorldPackage, x: f64, z: f64) -> bool {
    let Some(lu) = pkg.landuse.as_ref() else {
        return false;
    };
    let size = f64::from(pkg.tile_size_m);
    [
        (0.0, 0.0),
        (15.0, 0.0),
        (-15.0, 0.0),
        (0.0, 15.0),
        (0.0, -15.0),
    ]
    .iter()
    .any(|(dx, dz)| {
        matches!(
            lu.class_at(size, x + dx, z + dz),
            Some(LandClass::Water | LandClass::Wetland)
        )
    })
}

/// The stand of a field crop on the day: its height (m; 0: nothing
/// standing) and colour (linear).
pub fn stand(crop: FieldCrop, day: Option<&SeasonDay>, alt_m: f64) -> (f32, [f32; 3]) {
    let (cal, tall) = match crop {
        FieldCrop::Maize => (Crop::Maize, 2.5),
        FieldCrop::Sunflower => (Crop::Maize, 1.8),
        FieldCrop::Potato | FieldCrop::Beet => (Crop::Maize, 0.5),
        FieldCrop::Cereal => (Crop::Cereal, 0.9),
        FieldCrop::Rapeseed => (Crop::Cereal, 1.3),
        _ => return (0.0, [0.0; 3]),
    };
    let (stage, gdd, cdd) = match day {
        Some(d) => {
            let s = d.state(alt_m);
            (d.crop(cal, alt_m), s.gdd, s.cdd)
        }
        None => (CropStage::Green, 900.0, 0.0),
    };
    let h = match stage {
        CropStage::Sprouting => 0.35,
        CropStage::Green => 0.85,
        CropStage::Ripening | CropStage::Ripe => 1.0,
        _ => 0.0,
    } * tall;
    let mut c = crate::season::crop_target(cal, gdd, cdd);
    if crop == FieldCrop::Rapeseed {
        // Yellow in flower (late April, May).
        let t = ((gdd - 300.0) / 80.0).clamp(0.0, 1.0) * ((620.0 - gdd) / 80.0).clamp(0.0, 1.0);
        let yellow = Color::srgb(0.90, 0.80, 0.12).to_linear();
        c = std::array::from_fn(|i| c[i] + ([yellow.red, yellow.green, yellow.blue][i] - c[i]) * t);
    }
    if crop == FieldCrop::Sunflower && stage == CropStage::Green && gdd > 1100.0 {
        let y = Color::srgb(0.85, 0.65, 0.10).to_linear();
        c = [y.red, y.green, y.blue];
    }
    (if h < 0.3 { 0.0 } else { h }, c)
}

/// Builds a tile's plants: the grid's `classed` trees (already filtered by
/// `keep`), the mapped trees, the plantations' rows and the fields'
/// stands, on the `season` day.
pub fn tile_plants(
    pkg: &WorldPackage,
    key: TileKey,
    classed: &[(TreeSite, Option<LandClass>)],
    keep: Keep,
    season: Option<&SeasonDay>,
) -> TilePlants {
    let size = pkg.tile_size_m;
    let o = key.origin(f64::from(size));
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let mix = ocw_sky::season::Climate::for_place(pkg.origin.0, pkg.origin.1).trees;
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let (lat, lon) = proj.unproject([o[0] + f64::from(size) / 2.0, o[1] + f64::from(size) / 2.0]);
    let windward = ocw_world::flora::windward(lat, lon);
    let grid = |per: u32| -> Vec<TreeChunk> {
        let cs = size / per as f32;
        let n = per as usize;
        (0..n * n)
            .map(|i| {
                let (c, r) = (i % n, i / n);
                let ox = o[0] as f32 + (c as f32 + 0.5) * cs;
                let oz = o[1] as f32 + (r as f32 + 0.5) * cs;
                let oy = pkg.height(f64::from(ox), f64::from(oz)).unwrap_or(0.0) as f32;
                TreeChunk {
                    origin: [ox, oy, oz],
                    ..Default::default()
                }
            })
            .collect()
    };
    let cell_of = |per: u32, x: f32, z: f32| -> usize {
        let cs = size / per as f32;
        let n = per as usize;
        let c = (((x - o[0] as f32) / cs).floor().max(0.0) as usize).min(n - 1);
        let r = (((z - o[1] as f32) / cs).floor().max(0.0) as usize).min(n - 1);
        r * n + c
    };
    let mut chunks = grid(CHUNKS);
    let mut details = grid(DETAIL_CHUNKS);
    let chunk_of = |x: f32, z: f32| cell_of(CHUNKS, x, z);
    let detail_of = |x: f32, z: f32| cell_of(DETAIL_CHUNKS, x, z);
    let kept = |x: f32, z: f32| keep.is_none_or(|f| f([f64::from(x), f64::from(z)]));
    let mut crowns = Vec::new();
    let site_of = |class: Option<LandClass>, x: f32, z: f32, alt: f64| Site {
        region,
        setting: Setting::of_class(class),
        alt_m: alt,
        measured: ocw_world::trees::measured_leaf(pkg, f64::from(x), f64::from(z)),
        windward,
        near_water: near_water(pkg, f64::from(x), f64::from(z)),
    };
    let leaf_of = |class: Option<LandClass>, x: f32, z: f32, alt: f64, bits: (i64, i64)| {
        let setting = crate::season::tree_setting(class);
        let measured = crate::season::measured(ocw_world::trees::measured_leaf(
            pkg,
            f64::from(x),
            f64::from(z),
        ));
        let mix = season.map_or(&mix, |d| &d.climate.trees);
        mix.leaf_type(setting, alt, measured, hash(bits.0, bits.1, 13))
    };
    let put = |chunks: &mut Vec<TreeChunk>,
               crowns: &mut Vec<[f32; 4]>,
               sp: Species,
               base: Vec3,
               scale: f32,
               bits: (i64, i64),
               near_only: bool| {
        // Plants drawn only up close go to the detail chunks.
        let ch = if near_only {
            &mut chunks[detail_of(base.x, base.z)]
        } else {
            &mut chunks[chunk_of(base.x, base.z)]
        };
        let origin = Vec3::from(ch.origin);
        let alt = f64::from(base.y);
        let jitter = hash(bits.0, bits.1, 14) * 2.0 - 1.0;
        let look = crate::flora::plant_look(season, sp, alt, jitter);
        let at = PlantAt {
            base: base - origin,
            scale,
            turn: hash(bits.0, bits.1, 16) * std::f32::consts::TAU,
            shade: hash(bits.0, bits.1, 11),
            hue: hash(bits.0, bits.1, 15),
            pick: hash(bits.0, bits.1, 18),
        };
        crate::flora::plant(&mut ch.near, sp, &at, &look, Detail::Near);
        if !near_only {
            crate::flora::plant(&mut ch.far, sp, &at, &look, Detail::Far);
        }
        ch.trees += 1;
        if sp.solid() {
            let (top, r) = crate::flora::crown_top(sp, scale);
            crowns.push([base.x, base.z, r, base.y + top]);
        }
    };
    // The grid's trees.
    for &((base, s), class) in classed {
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let alt = f64::from(base.y);
        let leaf = leaf_of(class, base.x, base.z, alt, bits);
        let site = site_of(class, base.x, base.z, alt);
        let sp = pick_species(&site, group(leaf), hash(bits.0, bits.1, 19));
        // The grid's scale by land class (0.35 … 1.5) around its class's
        // mean: the species' own size varies by ±30 %.
        let (_, s0, s1) = ocw_world::trees::tree_density(class.unwrap_or_default());
        let scale = s / ((s0 + s1) / 2.0).max(0.1) * stature(region, sp, alt);
        put(&mut chunks, &mut crowns, sp, base, scale, bits, false);
    }
    // The mapped trees.
    for (foot, t) in plantings::mapped_trees(pkg, key) {
        if !kept(foot[0], foot[2]) {
            continue;
        }
        let base = Vec3::from(foot);
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let alt = f64::from(base.y);
        let sp = if t.species == Species::Unknown {
            let class = pkg
                .landuse
                .as_ref()
                .and_then(|l| l.class_at(f64::from(size), f64::from(base.x), f64::from(base.z)));
            // Mapped trees stand in towns and along roads: the town mix.
            let class = class.or(Some(LandClass::Residential));
            let leaf = leaf_of(class, base.x, base.z, alt, bits);
            pick_species(
                &site_of(class, base.x, base.z, alt),
                group(leaf),
                hash(bits.0, bits.1, 19),
            )
        } else {
            t.species
        };
        let scale = if t.height_m > 0.0 {
            (t.height_m / sp.height_m()).clamp(0.3, 2.0)
        } else {
            0.75 + 0.35 * hash(bits.0, bits.1, 4)
        };
        put(&mut chunks, &mut crowns, sp, base, scale, bits, false);
    }
    // The plantations' rows.
    let (runs, plants) = plantings::tile_rows(pkg, key);
    for p in plants {
        if !kept(p.pos[0], p.pos[2]) {
            continue;
        }
        let base = Vec3::from(p.pos);
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let near_only = p.species == Species::Banana;
        put(
            if near_only { &mut details } else { &mut chunks },
            &mut crowns,
            p.species,
            base,
            p.scale * 0.8,
            bits,
            near_only,
        );
    }
    let red_vines = matches!(region, FloraRegion::AlpineRhine);
    for r in runs {
        // Pieces of at most 10 m following the ground.
        let (a, b) = (Vec2::from(r.a), Vec2::from(r.b));
        let len = a.distance(b);
        let pieces = (len / 10.0).ceil().max(1.0) as u32;
        for k in 0..pieces {
            let p0 = a.lerp(b, k as f32 / pieces as f32);
            let p1 = a.lerp(b, (k + 1) as f32 / pieces as f32);
            let mid = (p0 + p1) / 2.0;
            if !kept(mid.x, mid.y) {
                continue;
            }
            let y = |p: Vec2| pkg.height(f64::from(p.x), f64::from(p.y)).unwrap_or(0.0) as f32;
            let ch = &mut details[detail_of(mid.x, mid.y)];
            let origin = Vec3::from(ch.origin);
            let (q0, q1) = (
                Vec3::new(p0.x, y(p0), p0.y) - origin,
                Vec3::new(p1.x, y(p1), p1.y) - origin,
            );
            let bits = ((mid.x * 4.0) as i64, (mid.y * 4.0) as i64);
            let jitter = hash(bits.0, bits.1, 14) * 2.0 - 1.0;
            let look = crate::flora::vine_look(season, f64::from(y(mid)), jitter);
            let red = red_vines && hash(bits.0 / 400, bits.1 / 400, 44) < 0.6;
            let colour = crate::flora::vine_colour(&look, red, hash(bits.0, bits.1, 11));
            let height = if region == FloraRegion::Tenerife {
                1.1
            } else {
                1.6
            };
            crate::flora::vine_row(&mut ch.near, q0, q1, height, &look, colour);
        }
    }
    // The fields' stands.
    stands(pkg, key, &mut chunks, &chunk_of, keep, season);
    chunks.retain(|c| !c.near.indices.is_empty() || !c.far.indices.is_empty());
    details.retain(|c| !c.near.indices.is_empty());
    TilePlants {
        chunks,
        details,
        crowns,
    }
}

/// The stands of maize, cereals, sunflowers, rapeseed and potatoes: a slab
/// per cell of section `VEGE` (near chunks only), its sides where the
/// neighbour is not the same crop.
fn stands(
    pkg: &WorldPackage,
    key: TileKey,
    chunks: &mut [TreeChunk],
    chunk_of: &dyn Fn(f32, f32) -> usize,
    keep: Keep,
    season: Option<&SeasonDay>,
) {
    let Some(veg) = pkg.look.vegetation.as_ref() else {
        return;
    };
    let Some(v) = veg.tiles.get(&key).filter(|v| !v.crops.is_empty()) else {
        return;
    };
    let size = f64::from(pkg.tile_size_m);
    let o = key.origin(size);
    let n = veg.cells as usize;
    let cs = size / n as f64;
    // The stand per crop at the tile's mean altitude band (100 m steps).
    let mut cache: std::collections::HashMap<(u8, i64), (f32, [f32; 3])> = Default::default();
    let crop_at = |c: isize, r: isize| -> Option<u8> {
        (c >= 0 && r >= 0 && (c as usize) < n && (r as usize) < n)
            .then(|| v.crops[r as usize * n + c as usize])
    };
    for r in 0..n {
        for c in 0..n {
            let code = v.crops[r * n + c];
            let Some(crop) = FieldCrop::from_code(code) else {
                continue;
            };
            if !matches!(
                crop,
                FieldCrop::Maize
                    | FieldCrop::Sunflower
                    | FieldCrop::Cereal
                    | FieldCrop::Rapeseed
                    | FieldCrop::Potato
                    | FieldCrop::Beet
            ) {
                continue;
            }
            let (x, z) = (o[0] + (c as f64 + 0.5) * cs, o[1] + (r as f64 + 0.5) * cs);
            if keep.is_some_and(|f| !f([x, z])) || plantings_blocked(pkg, x, z) {
                continue;
            }
            let h = |dx: f64, dz: f64| pkg.height(x + dx, z + dz).unwrap_or(0.0) as f32;
            let hc = h(0.0, 0.0);
            let band = (f64::from(hc) / 100.0).round() as i64;
            let (height, colour) = *cache
                .entry((code, band))
                .or_insert_with(|| stand(crop, season, band as f64 * 100.0));
            if height <= 0.0 {
                continue;
            }
            let hs = cs / 2.0;
            let ys = [h(-hs, -hs), h(hs, -hs), h(-hs, hs), h(hs, hs)];
            let (ci, ri) = (c as isize, r as isize);
            let open = [
                crop_at(ci - 1, ri) != Some(code),
                crop_at(ci + 1, ri) != Some(code),
                crop_at(ci, ri - 1) != Some(code),
                crop_at(ci, ri + 1) != Some(code),
            ];
            // Global corner indices: neighbouring cells share their corners'
            // heights and shades, so the canopy is one bumpy surface.
            let (gc, gr) = (
                c as i64 + i64::from(key.x) * n as i64,
                r as i64 + i64::from(key.z) * n as i64,
            );
            let corner = |dc: i64, dr: i64, seed: u64| hash(gc + dc, gr + dr, seed);
            let bump = |v: f32| height * (0.86 + 0.24 * v);
            let tops = [
                bump(corner(0, 0, 45)),
                bump(corner(1, 0, 45)),
                bump(corner(0, 1, 45)),
                bump(corner(1, 1, 45)),
                height * (0.95 + 0.2 * hash(gc, gr, 47)),
            ];
            let shade = |v: f32| 0.84 + 0.3 * v;
            let shades = [
                shade(corner(0, 0, 46)),
                shade(corner(1, 0, 46)),
                shade(corner(0, 1, 46)),
                shade(corner(1, 1, 46)),
                shade(hash(gc, gr, 48)),
            ];
            let i = chunk_of(x as f32, z as f32);
            let origin = Vec3::from(chunks[i].origin);
            let centre = Vec3::new(x as f32, hc, z as f32) - origin;
            let ys = ys.map(|y| y - origin.y);
            crate::flora::stand_cell(
                &mut chunks[i].near,
                centre,
                cs as f32,
                ys,
                tops,
                shades,
                open,
                colour,
            );
        }
    }
}

/// Whether a field cell's centre is on a road (the stands leave the road
/// and its verge free).
fn plantings_blocked(pkg: &WorldPackage, x: f64, z: f64) -> bool {
    let key = TileKey::containing([x, z], f64::from(pkg.tile_size_m));
    pkg.tiles.get(&key).is_some_and(|t| {
        t.roads.iter().any(|r| {
            let reach = f64::from(r.width_m) / 2.0 + 4.0;
            r.points.windows(2).any(|w| {
                ocw_world::obstacles::segment_distance(
                    [x, z],
                    [f64::from(w[0][0]), f64::from(w[0][2])],
                    [f64::from(w[1][0]), f64::from(w[1][2])],
                ) < reach
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stands_follow_the_crop_calendar() {
        let m = ocw_sky::season::SeasonModel::for_place(47.99, 7.85);
        // July: maize tall and green, cereals ripe; late August: cereal
        // stubble (nothing standing), maize still there; March: no maize.
        let jul = m.day(195.0);
        let (mh, mc) = stand(FieldCrop::Maize, Some(&jul), 200.0);
        assert!(mh > 1.8 && mc[1] > mc[0], "{mh} {mc:?}");
        let (ch, cc) = stand(FieldCrop::Cereal, Some(&jul), 200.0);
        assert!(ch > 0.6 && cc[0] > cc[2], "{ch} {cc:?}");
        let aug = m.day(240.0);
        assert_eq!(stand(FieldCrop::Cereal, Some(&aug), 200.0).0, 0.0);
        assert!(stand(FieldCrop::Maize, Some(&aug), 200.0).0 > 1.8);
        let mar = m.day(75.0);
        assert_eq!(stand(FieldCrop::Maize, Some(&mar), 200.0).0, 0.0);
        // Rapeseed in flower in early May: yellow.
        let may = m.day(125.0);
        let (rh, rc) = stand(FieldCrop::Rapeseed, Some(&may), 200.0);
        assert!(rh > 0.5 && rc[0] > rc[2] * 3.0, "{rh} {rc:?}");
        // Grass and vineyards have no stand.
        assert_eq!(stand(FieldCrop::Grass, Some(&jul), 200.0).0, 0.0);
        assert_eq!(stand(FieldCrop::Vineyard, Some(&jul), 200.0).0, 0.0);
        // No season: summer.
        assert!(stand(FieldCrop::Maize, None, 200.0).0 > 1.8);
    }
}
