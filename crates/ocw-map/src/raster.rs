//! The navigation map's CPU rasteriser (tiny-skia, pure Rust, the same
//! pixels on every platform): north-up map tiles of [`TILE_PX`]² texels –
//! the base map (land use, water, buildings, roads by class) and the
//! route layer (the planned path and the junction's exits, transparent
//! elsewhere) – and the device's icons (the rider arrow, the turn arrow
//! of the banner, the compass needle).
//!
//! Tile `(ix, iz)` of a texture of `m_per_texel` covers the home metres
//! `x ∈ [ix·T·m, (ix+1)·T·m)`, `z ∈ [iz·T·m, (iz+1)·T·m)`; texel (u, v)
//! is the square whose corner is at `(ix·T + u, iz·T + v) · m`. Pixels
//! are RGBA, premultiplied (the base tiles are opaque).

use ocw_world::package::TileKey;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, StrokeDash,
    Transform,
};

use crate::source::MapSource;
use crate::style::{NAVI_BUILDINGS_BELOW_M_PER_PX, Palette, Rgba, navi_road};

/// A map tile's side in texels.
pub const TILE_PX: u32 = 128;

/// One map tile to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileSpec {
    pub ix: i32,
    pub iz: i32,
    /// Metres per texel of the texture.
    pub m_per_texel: f64,
    /// Texels per map point (the screen's pixel density, ≥ 1): line
    /// widths are in map points.
    pub density: f64,
}

impl TileSpec {
    /// Metres per map point (picks the styles).
    pub fn m_per_px(&self) -> f64 {
        self.m_per_texel * self.density
    }

    /// The tile's box in home metres: (min, max).
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let s = f64::from(TILE_PX) * self.m_per_texel;
        (
            [f64::from(self.ix) * s, f64::from(self.iz) * s],
            [f64::from(self.ix + 1) * s, f64::from(self.iz + 1) * s],
        )
    }

    /// A home point in the tile's texels.
    pub fn texel(&self, p: [f64; 2]) -> (f32, f32) {
        let t = f64::from(TILE_PX);
        (
            (p[0] / self.m_per_texel - f64::from(self.ix) * t) as f32,
            (p[1] / self.m_per_texel - f64::from(self.iz) * t) as f32,
        )
    }
}

fn paint(c: Rgba, aa: bool) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = aa;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn polyline(spec: &TileSpec, points: &[[f64; 2]]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = spec.texel(*p);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

fn grow(b: ([f64; 2], [f64; 2]), m: f64) -> ([f64; 2], [f64; 2]) {
    ([b.0[0] - m, b.0[1] - m], [b.1[0] + m, b.1[1] + m])
}

fn new_pixmap() -> Pixmap {
    Pixmap::new(TILE_PX, TILE_PX).expect("a tile has a size")
}

/// Fills the home-metre box `a`…`b` (no anti-aliasing), reaching one
/// texel further right and down: the next cell, drawn after it, covers
/// that again, so neighbouring cells never leave a seam of ground between
/// them (they did, as faint stripes, in the first screenshots).
fn fill_box(pm: &mut Pixmap, spec: &TileSpec, a: [f64; 2], b: [f64; 2], c: Rgba) {
    let (x0, y0) = spec.texel(a);
    let (x1, y1) = spec.texel(b);
    let t = TILE_PX as f32;
    let (l, r) = (x0.min(x1).max(-1.0), (x0.max(x1) + 1.0).min(t + 1.0));
    let (top, bot) = (y0.min(y1).max(-1.0), (y0.max(y1) + 1.0).min(t + 1.0));
    if let Some(rect) = Rect::from_ltrb(l, top, r, bot) {
        pm.fill_rect(rect, &paint(c, false), Transform::identity(), None);
    }
}

/// The base map tile (opaque, premultiplied RGBA, [`TILE_PX`]²).
pub fn render_base_tile(src: &MapSource, pal: &Palette, spec: &TileSpec) -> Vec<u8> {
    let mut pm = new_pixmap();
    let o = pal.outside;
    pm.fill(Color::from_rgba8(o[0], o[1], o[2], 255));
    let bounds = spec.bounds();
    let tiles = src.tiles_in(bounds.0, bounds.1);
    // The ground of the world's tiles, then the land use cell by cell
    // (rows of equal cells as one box).
    for (sp, key) in &tiles {
        let size = f64::from(sp.pkg.tile_size_m);
        let corner = |x: f64, z: f64| sp.frame.apply([x, z]);
        let (kx, kz) = (f64::from(key.x) * size, f64::from(key.z) * size);
        fill_box(
            &mut pm,
            spec,
            corner(kx, kz),
            corner(kx + size, kz + size),
            pal.ground,
        );
        let Some(lu) = &sp.pkg.landuse else {
            continue;
        };
        let Some(t) = lu.tiles.get(key) else {
            continue;
        };
        let n = lu.cells.max(1) as usize;
        if t.classes.len() < n * n {
            continue;
        }
        let cs = size / n as f64;
        // Only the cells over this map tile.
        let lo = sp.frame.invert(bounds.0);
        let hi = sp.frame.invert(bounds.1);
        let i0 = (((lo[0].min(hi[0]) - kx) / cs).floor().max(0.0) as usize).min(n);
        let i1 = (((lo[0].max(hi[0]) - kx) / cs).ceil().max(0.0) as usize).min(n);
        let k0 = (((lo[1].min(hi[1]) - kz) / cs).floor().max(0.0) as usize).min(n);
        let k1 = (((lo[1].max(hi[1]) - kz) / cs).ceil().max(0.0) as usize).min(n);
        for k in k0..k1 {
            let mut i = i0;
            while i < i1 {
                let class = t.classes[k * n + i];
                let mut j = i + 1;
                while j < i1 && t.classes[k * n + j] == class {
                    j += 1;
                }
                if let Some(c) = pal.land(class) {
                    let z = kz + k as f64 * cs;
                    fill_box(
                        &mut pm,
                        spec,
                        corner(kx + i as f64 * cs, z),
                        corner(kx + j as f64 * cs, z + cs),
                        c,
                    );
                }
                i = j;
            }
        }
    }
    let m_per_px = spec.m_per_px();
    // Buildings as small blocks at close zoom.
    if m_per_px < NAVI_BUILDINGS_BELOW_M_PER_PX {
        let fill = paint(pal.building, true);
        let edge = paint(pal.building_edge, true);
        let edged = m_per_px < 1.2;
        for (sp, key) in &tiles {
            let Some(list) = sp.pkg.buildings.as_ref().and_then(|b| b.get(key)) else {
                continue;
            };
            let lo = sp.frame.invert(bounds.0);
            let hi = sp.frame.invert(bounds.1);
            for b in list {
                let inside = b.outline.iter().any(|p| {
                    let (x, z) = (f64::from(p[0]), f64::from(p[1]));
                    x >= lo[0] - 60.0 && x <= hi[0] + 60.0 && z >= lo[1] - 60.0 && z <= hi[1] + 60.0
                });
                if !inside || b.outline.len() < 3 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                for ring in std::iter::once(&b.outline).chain(&b.holes) {
                    for (i, p) in ring.iter().enumerate() {
                        let (x, y) = spec.texel(sp.frame.apply([f64::from(p[0]), f64::from(p[1])]));
                        if i == 0 {
                            pb.move_to(x, y);
                        } else {
                            pb.line_to(x, y);
                        }
                    }
                    pb.close();
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &fill, FillRule::EvenOdd, Transform::identity(), None);
                    if edged {
                        pm.stroke_path(
                            &path,
                            &edge,
                            &stroke(0.8 * spec.density as f32),
                            Transform::identity(),
                            None,
                        );
                    }
                }
            }
        }
    }
    // Roads by class: every casing first, then the fills from the small
    // roads up, so crossings merge and the main roads lie on top.
    let margin = 12.0 * spec.density * spec.m_per_texel;
    let near = grow(bounds, margin);
    let mut roads: Vec<_> = src
        .roads_in(near.0, near.1)
        .into_iter()
        .filter_map(|r| navi_road(r.class, m_per_px, pal).map(|s| (r, s)))
        .collect();
    roads.sort_by_key(|(_, s)| s.2);
    let d = spec.density as f32;
    let casing = paint(pal.casing, true);
    let paths: Vec<_> = roads
        .iter()
        .filter_map(|(r, s)| polyline(spec, &r.points).map(|p| (p, r, s)))
        .collect();
    for (path, r, s) in &paths {
        if !r.tunnel {
            pm.stroke_path(
                path,
                &casing,
                &stroke(s.0 * d + 1.6 * d),
                Transform::identity(),
                None,
            );
        }
    }
    for (path, r, s) in &paths {
        let mut st = stroke(s.0 * d);
        let mut c = s.1;
        if r.tunnel {
            // Tunnels: a paler dashed line.
            c[3] = 150;
            st.dash = StrokeDash::new(vec![6.0 * d, 4.0 * d], 0.0);
        }
        pm.stroke_path(path, &paint(c, true), &st, Transform::identity(), None);
    }
    pm.take()
}

/// A line of the route layer (home metres).
#[derive(Debug, Clone, PartialEq)]
pub struct MapLine {
    pub points: Vec<[f64; 2]>,
    /// Fill width (map points) and colour.
    pub width: f32,
    pub colour: Rgba,
    /// The casing around it: extra width (map points) and colour.
    pub casing: Option<(f32, Rgba)>,
    /// Drawn after (over) lines with a lower layer.
    pub layer: u8,
}

impl MapLine {
    fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        self.points.iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(lo, hi), p| {
                (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                )
            },
        )
    }
}

/// The route layer's tile (premultiplied RGBA, transparent where no line
/// is); `None`: no line comes near the tile (it stays empty).
pub fn render_line_tile(lines: &[MapLine], spec: &TileSpec) -> Option<Vec<u8>> {
    let margin = 16.0 * spec.density * spec.m_per_texel;
    let near = grow(spec.bounds(), margin);
    let mut hits: Vec<&MapLine> = lines
        .iter()
        .filter(|l| {
            let (lo, hi) = l.bounds();
            l.points.len() >= 2
                && hi[0] >= near.0[0]
                && lo[0] <= near.1[0]
                && hi[1] >= near.0[1]
                && lo[1] <= near.1[1]
        })
        .collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|l| l.layer);
    let mut pm = new_pixmap();
    let d = spec.density as f32;
    let mut layers: Vec<u8> = hits.iter().map(|l| l.layer).collect();
    layers.dedup();
    for layer in layers {
        let group: Vec<_> = hits.iter().filter(|l| l.layer == layer).collect();
        for l in &group {
            if let (Some((w, c)), Some(path)) = (l.casing, polyline(spec, &l.points)) {
                pm.stroke_path(
                    &path,
                    &paint(c, true),
                    &stroke((l.width + w) * d),
                    Transform::identity(),
                    None,
                );
            }
        }
        for l in &group {
            if let Some(path) = polyline(spec, &l.points) {
                pm.stroke_path(
                    &path,
                    &paint(l.colour, true),
                    &stroke(l.width * d),
                    Transform::identity(),
                    None,
                );
            }
        }
    }
    Some(pm.take())
}

/// Turns premultiplied RGBA into straight RGBA in place (Bevy's images
/// are not premultiplied).
pub fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

fn icon(px: u32, draw: impl FnOnce(&mut Pixmap, f32)) -> Vec<u8> {
    let mut pm = Pixmap::new(px.max(1), px.max(1)).expect("an icon has a size");
    draw(&mut pm, px.max(1) as f32);
    let mut v = pm.take();
    unpremultiply(&mut v);
    v
}

/// The rider's arrow (pointing up), `px`² straight RGBA: a navi chevron
/// in `fill` with a white edge and a soft shadow.
pub fn rider_arrow(px: u32, fill: Rgba) -> Vec<u8> {
    icon(px, |pm, s| {
        let shape = |pm: &mut Pixmap, dx: f32, dy: f32, k: f32, c: Rgba| {
            let mut pb = PathBuilder::new();
            let (cx, cy) = (s * 0.5 + dx, s * 0.5 + dy);
            pb.move_to(cx, cy - s * 0.42 * k);
            pb.line_to(cx + s * 0.34 * k, cy + s * 0.38 * k);
            pb.line_to(cx, cy + s * 0.2 * k);
            pb.line_to(cx - s * 0.34 * k, cy + s * 0.38 * k);
            pb.close();
            if let Some(p) = pb.finish() {
                pm.fill_path(
                    &p,
                    &paint(c, true),
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        };
        shape(pm, 0.0, s * 0.04, 1.0, [0, 0, 0, 90]);
        shape(pm, 0.0, 0.0, 1.0, [255, 255, 255, 255]);
        shape(pm, 0.0, s * 0.015, 0.78, fill);
    })
}

/// The banner's turn arrow for a turn of `angle_deg` (positive right; 0
/// straight on; beyond ±150° a U-turn), `px`² straight RGBA in `colour`.
pub fn turn_arrow(angle_deg: f64, px: u32, colour: Rgba) -> Vec<u8> {
    icon(px, |pm, s| {
        let w = s * 0.13;
        let base = (s * 0.5, s * 0.94);
        let pivot = (s * 0.5, s * 0.55);
        let mut pb = PathBuilder::new();
        let tip;
        let dir;
        if angle_deg.abs() > 150.0 {
            // U-turn: up, round, down on the side of the turn.
            let side = if angle_deg >= 0.0 { 1.0 } else { -1.0 };
            let r = s * 0.17;
            let top = s * 0.3;
            pb.move_to(base.0 - side * r, base.1);
            pb.line_to(base.0 - side * r, top);
            pb.quad_to(base.0 - side * r, top - r * 1.2, base.0, top - r * 1.2);
            pb.quad_to(base.0 + side * r, top - r * 1.2, base.0 + side * r, top);
            tip = (base.0 + side * r, s * 0.62);
            pb.line_to(tip.0, tip.1 - s * 0.08);
            dir = (0.0f32, 1.0f32);
        } else {
            let a = (angle_deg as f32).to_radians();
            dir = (a.sin(), -a.cos());
            let len = s * 0.3;
            tip = (pivot.0 + dir.0 * len, pivot.1 + dir.1 * len);
            pb.move_to(base.0, base.1);
            pb.line_to(pivot.0, pivot.1);
            pb.line_to(tip.0 - dir.0 * s * 0.08, tip.1 - dir.1 * s * 0.08);
        }
        if let Some(p) = pb.finish() {
            let mut st = stroke(w);
            st.line_cap = LineCap::Butt;
            pm.stroke_path(&p, &paint(colour, true), &st, Transform::identity(), None);
        }
        // The head.
        let n = (-dir.1, dir.0);
        let h = s * 0.2;
        let mut pb = PathBuilder::new();
        pb.move_to(tip.0 + dir.0 * h * 0.35, tip.1 + dir.1 * h * 0.35);
        pb.line_to(
            tip.0 - dir.0 * h * 0.65 + n.0 * h * 0.75,
            tip.1 - dir.1 * h * 0.65 + n.1 * h * 0.75,
        );
        pb.line_to(
            tip.0 - dir.0 * h * 0.65 - n.0 * h * 0.75,
            tip.1 - dir.1 * h * 0.65 - n.1 * h * 0.75,
        );
        pb.close();
        if let Some(p) = pb.finish() {
            pm.fill_path(
                &p,
                &paint(colour, true),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    })
}

/// The compass needle (north up: the north half red, the south half
/// light), on a dark disc, `px`² straight RGBA.
pub fn compass(px: u32) -> Vec<u8> {
    icon(px, |pm, s| {
        let c = s * 0.5;
        if let Some(p) = PathBuilder::from_circle(c, c, s * 0.48) {
            pm.fill_path(
                &p,
                &paint([20, 22, 26, 200], true),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let half = |pm: &mut Pixmap, up: bool, col: Rgba| {
            let k = if up { -1.0 } else { 1.0 };
            let mut pb = PathBuilder::new();
            pb.move_to(c, c + k * s * 0.4);
            pb.line_to(c + s * 0.12, c);
            pb.line_to(c - s * 0.12, c);
            pb.close();
            if let Some(p) = pb.finish() {
                pm.fill_path(
                    &p,
                    &paint(col, true),
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        };
        half(pm, true, [235, 60, 50, 255]);
        half(pm, false, [225, 228, 232, 255]);
    })
}

/// The package tile keys a map tile reads (for tests and tools).
pub fn tile_keys(src: &MapSource, spec: &TileSpec) -> Vec<TileKey> {
    let b = spec.bounds();
    src.tiles_in(b.0, b.1).into_iter().map(|(_, k)| k).collect()
}
