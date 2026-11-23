//! A light smoothing of the terrain off the roads (spec §4.8.8 Q7 (d),
//! v5.52: "smoothen the whole terrain a bit without losing too much
//! precision"). The DEM-derived grid carries small bumps of a few
//! decimetres (the DTM's noise, cut edges, single trees left in it) that
//! low sun shows on meadows and forest edges. Each grid vertex moves
//! towards a local quadratic surface fitted to its neighbours within
//! [`RADIUS_M`] (Gaussian-weighted least squares, a Savitzky–Golay
//! filter: bumps smaller than the window go, slopes and the curvature of
//! hills, ridges and valleys stay) – by at most [`MAX_CHANGE_M`] – except
//! where the terrain belongs to something placed on it: the roads
//! (their half width plus a grid spacing, conformed to the road profiles,
//! spec 4.3), the junction surfaces and side-road blends, water (its
//! levels) and the building sites (4.8.5). Between those and the free
//! terrain the change fades in over the kernel's radius, so the road cut
//! zones keep their edges. Pure.

use std::collections::{BTreeMap, HashSet};

use ocw_world::package::{Tile, TileKey, WorldPackage};

/// The window's radius (m; its Gaussian weights' σ is half of it), so
/// bumps of about 10 – 20 m are smoothed.
pub const RADIUS_M: f64 = 10.0;
/// Most a vertex moves (m).
pub const MAX_CHANGE_M: f64 = 0.5;
/// Changes below this (m) are not counted as changed.
const CHANGED_M: f64 = 0.001;

/// What [`smooth_terrain`] changed (over the distinct grid vertices).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmoothStats {
    pub vertices: usize,
    /// Kept as they were: roads, junction surfaces, water, buildings.
    pub kept: usize,
    /// Moved by more than 1 mm, and of those limited to the largest change.
    pub changed: usize,
    pub capped: usize,
    pub max_change_m: f64,
    /// Mean absolute change over all vertices and over the changed ones.
    pub mean_change_m: f64,
    pub mean_changed_m: f64,
    /// Mean signed change over all vertices (smoothing moves mass from
    /// bumps into hollows: about 0).
    pub mean_signed_m: f64,
}

/// The weights of a Gaussian-weighted (σ `sigma` cells) least-squares
/// quadratic surface (1, x, z, x², xz, z²) over the square window of
/// radius `r` cells, for its value at the centre: (di, dk, weight). They
/// sum to 1 and reproduce any quadratic exactly.
pub fn quadratic_kernel(r: i64, sigma: f64) -> Vec<(i64, i64, f64)> {
    let pts: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|dk| (-r..=r).map(move |di| (di, dk)))
        .collect();
    let basis = |di: i64, dk: i64| {
        let (x, z) = (di as f64, dk as f64);
        [1.0, x, z, x * x, x * z, z * z]
    };
    let wt = |di: i64, dk: i64| (-((di * di + dk * dk) as f64) / (2.0 * sigma * sigma)).exp();
    // M = Aᵀ W A, solved for e0: the centre value's weights are
    // w_k · a_kᵀ M⁻¹ e0.
    let mut m = [[0.0f64; 6]; 6];
    for &(di, dk) in &pts {
        let a = basis(di, dk);
        let w = wt(di, dk);
        for i in 0..6 {
            for j in 0..6 {
                m[i][j] += w * a[i] * a[j];
            }
        }
    }
    // Gauss–Jordan on [M | e0].
    let mut aug = [[0.0f64; 7]; 6];
    for i in 0..6 {
        aug[i][..6].copy_from_slice(&m[i]);
        aug[i][6] = if i == 0 { 1.0 } else { 0.0 };
    }
    for col in 0..6 {
        let piv = (col..6)
            .max_by(|&a, &b| aug[a][col].abs().total_cmp(&aug[b][col].abs()))
            .unwrap_or(col);
        aug.swap(col, piv);
        let p = aug[col][col];
        if p.abs() < 1e-12 {
            // Too small a window for a quadratic: the weighted mean.
            let sum: f64 = pts.iter().map(|&(i, k)| wt(i, k)).sum();
            return pts.iter().map(|&(i, k)| (i, k, wt(i, k) / sum)).collect();
        }
        for v in aug[col].iter_mut() {
            *v /= p;
        }
        let pivot_row = aug[col];
        for (row, r) in aug.iter_mut().enumerate() {
            let f = r[col];
            if row != col && f != 0.0 {
                for (x, pv) in r.iter_mut().zip(&pivot_row) {
                    *x -= f * pv;
                }
            }
        }
    }
    let y: Vec<f64> = (0..6).map(|i| aug[i][6]).collect();
    pts.iter()
        .map(|&(di, dk)| {
            let a = basis(di, dk);
            let v: f64 = a.iter().zip(&y).map(|(a, y)| a * y).sum();
            (di, dk, wt(di, dk) * v)
        })
        .collect()
}

/// Global grid index of the vertex nearest to `p`.
fn grid(p: [f64; 2], sp: f64) -> (i64, i64) {
    ((p[0] / sp).round() as i64, (p[1] / sp).round() as i64)
}

/// The vertices kept besides `on_road`: the corners of the grid cells
/// under the `surfaces` points (junction surfaces, blends), every vertex
/// with a water level, and the vertices under each building footprint's
/// box grown by a metre and a grid spacing.
fn fixed_vertices(pkg: &WorldPackage, surfaces: &[[f64; 3]]) -> HashSet<(i64, i64)> {
    let sp = pkg.spacing_m();
    let n = pkg.samples as usize;
    let step = (n - 1) as i64;
    let mut out = HashSet::new();
    for q in surfaces {
        let (gi, gk) = ((q[0] / sp).floor() as i64, (q[2] / sp).floor() as i64);
        for (di, dk) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            out.insert((gi + di, gk + dk));
        }
    }
    if let Some(lu) = &pkg.landuse {
        for (key, t) in &lu.tiles {
            for &(idx, _) in &t.water {
                let (i, k) = (idx as usize % n, idx as usize / n);
                out.insert((
                    i64::from(key.x) * step + i as i64,
                    i64::from(key.z) * step + k as i64,
                ));
            }
        }
    }
    if let Some(b) = &pkg.buildings {
        for bd in b.values().flatten() {
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &bd.outline {
                lo = [lo[0].min(f64::from(p[0])), lo[1].min(f64::from(p[1]))];
                hi = [hi[0].max(f64::from(p[0])), hi[1].max(f64::from(p[1]))];
            }
            if !lo[0].is_finite() {
                continue;
            }
            let g = 1.0 + sp;
            let (a, b) = (
                grid([lo[0] - g, lo[1] - g], sp),
                grid([hi[0] + g, hi[1] + g], sp),
            );
            for gk in a.1..=b.1 {
                for gi in a.0..=b.0 {
                    out.insert((gi, gk));
                }
            }
        }
    }
    out
}

/// Whether this tile counts vertex (`i`, `k`) of its grid (each grid
/// vertex once: in the tile that owns it, `i, k < n − 1`; on the package's
/// last row or column, in the first tile holding it).
fn counts(tiles: &BTreeMap<TileKey, Tile>, key: TileKey, i: usize, k: usize, step: usize) -> bool {
    let (ei, ek) = (i == step, k == step);
    if !ei && !ek {
        return true;
    }
    // The tiles that hold the vertex, the owner first.
    let owner = TileKey {
        x: key.x + i32::from(ei),
        z: key.z + i32::from(ek),
    };
    if tiles.contains_key(&owner) {
        return false;
    }
    let mut holders = vec![key];
    if ei && ek {
        holders.push(TileKey {
            x: key.x + 1,
            z: key.z,
        });
        holders.push(TileKey {
            x: key.x,
            z: key.z + 1,
        });
    }
    holders
        .into_iter()
        .filter(|h| tiles.contains_key(h))
        .min()
        .is_some_and(|h| h == key)
}

/// Smooths the terrain grid of `pkg` (see the module): towards the local
/// quadratic surface over the vertices within `radius_m` (weights of σ =
/// `radius_m` / 2, [`quadratic_kernel`]), each vertex moved by at most
/// `max_change_m`; the vertices where `on_road` (position) holds,
/// under the `surfaces` points, with a water level or under a building
/// stay, and the change fades in over the radius from them. Shared border
/// vertices get the same height in every tile.
pub fn smooth_terrain(
    pkg: &mut WorldPackage,
    on_road: &(dyn Fn([f64; 2]) -> bool + Sync),
    surfaces: &[[f64; 3]],
    radius_m: f64,
    max_change_m: f64,
) -> SmoothStats {
    let n = pkg.samples as usize;
    if n < 2 || pkg.tiles.is_empty() {
        return SmoothStats::default();
    }
    let step = n - 1;
    let sp = pkg.spacing_m();
    let r = ((radius_m / sp).round() as i64).max(1);
    let sigma = (radius_m / 2.0 / sp).max(0.5);
    let pad = r + 1;
    let fixed = fixed_vertices(pkg, surfaces);
    let shared: &WorldPackage = pkg;
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let kernel = quadratic_kernel(r, sigma);
    let per_tile = crate::par::map(&keys, crate::par::threads(), |&key| {
        let pkg = shared;
        let w = n as i64 + 2 * pad;
        let (gi0, gk0) = (
            i64::from(key.x) * step as i64 - pad,
            i64::from(key.z) * step as i64 - pad,
        );
        // The padded window: heights (NaN outside the package) and kept.
        let mut h = vec![f64::NAN; (w * w) as usize];
        let mut keep = vec![false; (w * w) as usize];
        for wk in 0..w {
            for wi in 0..w {
                let (gi, gk) = (gi0 + wi, gk0 + wk);
                let tk = TileKey {
                    x: gi.div_euclid(step as i64) as i32,
                    z: gk.div_euclid(step as i64) as i32,
                };
                let (li, lk) = (
                    gi.rem_euclid(step as i64) as usize,
                    gk.rem_euclid(step as i64) as usize,
                );
                // A vertex on a tile's first row / column is also the last
                // of the tile before: either holds it.
                let t = pkg.tiles.get(&tk).map(|t| (t, li, lk)).or_else(|| {
                    let mut alt = None;
                    for (dx, dz) in [(-1i32, 0i32), (0, -1), (-1, -1)] {
                        if (dx != 0 && li != 0) || (dz != 0 && lk != 0) {
                            continue;
                        }
                        let k2 = TileKey {
                            x: tk.x + dx,
                            z: tk.z + dz,
                        };
                        if let Some(t) = pkg.tiles.get(&k2) {
                            alt = Some((
                                t,
                                if dx != 0 { step } else { li },
                                if dz != 0 { step } else { lk },
                            ));
                            break;
                        }
                    }
                    alt
                });
                let Some((t, i, k)) = t else { continue };
                if t.heights.len() != n * n {
                    continue;
                }
                let at = (wk * w + wi) as usize;
                h[at] = f64::from(t.heights[k * n + i]);
                keep[at] = fixed.contains(&(gi, gk)) || on_road([gi as f64 * sp, gk as f64 * sp]);
            }
        }
        let mut out = vec![0.0f32; n * n];
        let mut st = SmoothStats::default();
        let Some(tile) = pkg.tiles.get(&key) else {
            return (key, out, st);
        };
        if tile.heights.len() != n * n {
            return (key, Vec::new(), st);
        }
        for k in 0..n {
            for i in 0..n {
                let (wi, wk) = (i as i64 + pad, k as i64 + pad);
                let at = (wk * w + wi) as usize;
                let old = f64::from(tile.heights[k * n + i]);
                out[k * n + i] = tile.heights[k * n + i];
                let counted = counts(&pkg.tiles, key, i, k, step);
                if counted {
                    st.vertices += 1;
                }
                if keep[at] {
                    if counted {
                        st.kept += 1;
                    }
                    continue;
                }
                // Distance (cells, Chebyshev) to the nearest kept vertex,
                // up to r + 1: the change fades in from it.
                let mut d = pad;
                'ring: for ring in 1..=pad {
                    for dk in -ring..=ring {
                        for di in -ring..=ring {
                            if di.abs() != ring && dk.abs() != ring {
                                continue;
                            }
                            if keep[((wk + dk) * w + wi + di) as usize] {
                                d = ring;
                                break 'ring;
                            }
                        }
                    }
                }
                let fade = (d as f64 / pad as f64).min(1.0);
                // The fitted surface's height here (the whole window in
                // the package; at its border the terrain stays).
                let mut fit = 0.0;
                let mut whole = true;
                for &(di, dk, kw) in &kernel {
                    let v = h[((wk + dk) * w + wi + di) as usize];
                    if !v.is_finite() {
                        whole = false;
                        break;
                    }
                    fit += kw * v;
                }
                if !whole {
                    continue;
                }
                let want = fade * (fit - old);
                let dh = want.clamp(-max_change_m, max_change_m);
                let new = (old + dh) as f32;
                out[k * n + i] = new;
                if counted {
                    let real = f64::from(new) - old;
                    if real.abs() > CHANGED_M {
                        st.changed += 1;
                        st.mean_changed_m += real.abs();
                    }
                    if want.abs() > max_change_m {
                        st.capped += 1;
                    }
                    st.max_change_m = st.max_change_m.max(real.abs());
                    st.mean_change_m += real.abs();
                    st.mean_signed_m += real;
                }
            }
        }
        (key, out, st)
    });
    let mut total = SmoothStats::default();
    for (key, heights, st) in per_tile {
        if !heights.is_empty()
            && let Some(t) = pkg.tiles.get_mut(&key)
        {
            t.heights = heights;
        }
        total.vertices += st.vertices;
        total.kept += st.kept;
        total.changed += st.changed;
        total.capped += st.capped;
        total.max_change_m = total.max_change_m.max(st.max_change_m);
        total.mean_change_m += st.mean_change_m;
        total.mean_changed_m += st.mean_changed_m;
        total.mean_signed_m += st.mean_signed_m;
    }
    if total.vertices > 0 {
        total.mean_change_m /= total.vertices as f64;
        total.mean_signed_m /= total.vertices as f64;
    }
    if total.changed > 0 {
        total.mean_changed_m /= total.changed as f64;
    }
    total
}

/// [`smooth_terrain`] for a built package with its road graph: the roads
/// on the ground (their half width plus a grid spacing, not on bridge
/// decks), its junction surfaces and blends kept; then, if `settle`, the
/// terrain is settled under the roads and junction surfaces again (4.8.3:
/// nothing rises through them). Returns what the smoothing and the
/// settling did.
pub fn smooth_package_terrain(
    pkg: &mut WorldPackage,
    radius_m: f64,
    max_change_m: f64,
    settle: bool,
) -> (SmoothStats, crate::clean::SettleStats) {
    let sp = pkg.spacing_m();
    let surfaces = match pkg.graph.as_ref() {
        Some(g) => {
            let js = match pkg.junctions.as_ref() {
                Some(j) => ocw_world::junction::JunctionSet::with_index(g, j.index_for(g)),
                None => ocw_world::junction::JunctionSet::new(g),
            };
            crate::junctions::surface_samples(g, &js, 0.5)
        }
        None => Vec::new(),
    };
    let st = match pkg.graph.clone() {
        Some(g) => {
            let spans = crate::junctions::bridge_spans(pkg, &g);
            let zone = crate::clean::RoadZone::new(&g, &spans);
            let on_road = |p: [f64; 2]| zone.covers(p, sp);
            smooth_terrain(pkg, &on_road, &surfaces, radius_m, max_change_m)
        }
        None => smooth_terrain(pkg, &|_| false, &surfaces, radius_m, max_change_m),
    };
    let settled = if settle {
        crate::clean::settle_terrain_under_surfaces(pkg, 0.05, &surfaces)
    } else {
        crate::clean::SettleStats::default()
    };
    (st, settled)
}

#[cfg(test)]
mod tests;
