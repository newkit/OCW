//! A ride's track as a small map (spec 14 Phase 7, 7A: the ride's page in
//! the training history): the recorded GPS positions (latitude,
//! longitude) thinned to a few hundred points for the history file
//! ([`simplify`]), and drawn north-up into a small picture with the
//! navigation map's palette – the track with its casing, a green start
//! and a chequered finish, a scale bar ([`render`]). Pure (tiny-skia),
//! the same pixels on every platform; no map tiles (spec 4).

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use crate::navi::thin;
use crate::raster::unpremultiply;
use crate::style::{Palette, Rgba, Shade, scale_bar};
use crate::view::MapView;

/// The mean earth radius (m) of the local projection.
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// A track kept in the history has at most this many points.
pub const TRACK_MAX_POINTS: usize = 300;
/// … and keeps every bend wider than this (m), where the points allow.
pub const TRACK_TOLERANCE_M: f64 = 3.0;
/// Positions are kept to this many decimals of a degree (about 1 m).
const DECIMALS: f64 = 1e5;

/// Metres per degree of latitude.
fn m_per_deg() -> f64 {
    EARTH_RADIUS_M * std::f64::consts::PI / 180.0
}

/// A position (latitude, longitude in degrees) in local metres around
/// `origin`: x east, z south (the package's axes, so north is −z).
pub fn to_metres(p: [f64; 2], origin: [f64; 2]) -> [f64; 2] {
    let k = m_per_deg();
    [
        (p[1] - origin[1]) * k * origin[0].to_radians().cos(),
        -(p[0] - origin[0]) * k,
    ]
}

/// The inverse of [`to_metres`].
pub fn from_metres(m: [f64; 2], origin: [f64; 2]) -> [f64; 2] {
    let k = m_per_deg();
    let cos = origin[0].to_radians().cos().max(1e-6);
    [origin[0] - m[1] / k, origin[1] + m[0] / (k * cos)]
}

/// The middle of the positions' box (the projection's origin).
fn centre(track: &[[f64; 2]]) -> Option<[f64; 2]> {
    let first = *track.first()?;
    let (lo, hi) = track.iter().fold((first, first), |(lo, hi), p| {
        (
            [lo[0].min(p[0]), lo[1].min(p[1])],
            [hi[0].max(p[0]), hi[1].max(p[1])],
        )
    });
    Some([(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5])
}

fn round(v: f64) -> f64 {
    (v * DECIMALS).round() / DECIMALS
}

/// The track to keep: the positions (latitude, longitude) without
/// repeats, thinned (Douglas–Peucker in metres, [`TRACK_TOLERANCE_M`],
/// doubled until at most `max` points remain), rounded to about a metre.
/// Fewer than two distinct positions: no track.
pub fn simplify(positions: &[[f64; 2]], max: usize) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = positions
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite() && p[0].abs() <= 90.0)
        .map(|p| [round(p[0]), round(p[1])])
        .collect();
    pts.dedup();
    let Some(origin) = centre(&pts) else {
        return Vec::new();
    };
    if pts.len() < 2 {
        return Vec::new();
    }
    let metres: Vec<[f64; 2]> = pts.iter().map(|&p| to_metres(p, origin)).collect();
    let mut tol = TRACK_TOLERANCE_M;
    let mut kept = thin(&metres, tol);
    while kept.len() > max.max(2) {
        tol *= 2.0;
        kept = thin(&metres, tol);
    }
    kept.into_iter()
        .map(|m| {
            let p = from_metres(m, origin);
            [round(p[0]), round(p[1])]
        })
        .collect()
}

/// The track's length (m) over its kept points.
pub fn length_m(track: &[[f64; 2]]) -> f64 {
    let Some(origin) = centre(track) else {
        return 0.0;
    };
    track
        .windows(2)
        .map(|w| {
            let (a, b) = (to_metres(w[0], origin), to_metres(w[1], origin));
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
        })
        .sum()
}

/// A drawn ride map.
#[derive(Debug, Clone, PartialEq)]
pub struct RideMapImage {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) sRGB RGBA, row by row.
    pub rgba: Vec<u8>,
    /// The scale bar's label ("1 km", "0.5 mi"), drawn by the caller
    /// beside the bar at the picture's lower left.
    pub scale: String,
    /// The scale bar's length (px of the picture).
    pub scale_px: f32,
}

/// The margin around the track (px of the picture).
const MARGIN_PX: f32 = 14.0;
/// The bar's place from the lower left corner (px).
pub const SCALE_INSET_PX: f32 = 8.0;

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
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

fn dot(pm: &mut Pixmap, at: [f32; 2], r: f32, fill: Rgba, ring: Rgba) {
    for (radius, colour) in [(r + 2.0, ring), (r, fill)] {
        if let Some(path) = PathBuilder::from_circle(at[0], at[1], radius) {
            pm.fill_path(
                &path,
                &paint(colour),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// The finish: a small chequered square with a ring.
fn chequer(pm: &mut Pixmap, at: [f32; 2], r: f32, ring: Rgba) {
    let s = r * 2.0;
    if let Some(rect) = Rect::from_xywh(at[0] - r - 2.0, at[1] - r - 2.0, s + 4.0, s + 4.0) {
        pm.fill_rect(rect, &paint(ring), Transform::identity(), None);
    }
    let cell = s / 3.0;
    for i in 0..3 {
        for j in 0..3 {
            let c = if (i + j) % 2 == 0 {
                [20, 20, 24, 255]
            } else {
                [250, 250, 250, 255]
            };
            if let Some(rect) = Rect::from_xywh(
                at[0] - r + i as f32 * cell,
                at[1] - r + j as f32 * cell,
                cell,
                cell,
            ) {
                pm.fill_rect(rect, &paint(c), Transform::identity(), None);
            }
        }
    }
}

/// Draws `track` (latitude, longitude) north-up into a picture of
/// `width` × `height` px: the navigation map's ground, the track in the
/// route colour with its casing, the start (green dot) and the finish
/// (chequered), a scale bar at the lower left (its label for the caller).
/// `None` without a track (fewer than two points).
pub fn render(
    track: &[[f64; 2]],
    width: u32,
    height: u32,
    imperial: bool,
    shade: Shade,
) -> Option<RideMapImage> {
    if track.len() < 2 || width < 16 || height < 16 {
        return None;
    }
    let origin = centre(track)?;
    let metres: Vec<[f64; 2]> = track.iter().map(|&p| to_metres(p, origin)).collect();
    let (lo, hi) = metres.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    );
    // At least 200 m across, so a short or straight ride is not a blur.
    let pad = |a: f64, b: f64| ((200.0 - (b - a)).max(0.0)) * 0.5;
    let (px, pz) = (pad(lo[0], hi[0]), pad(lo[1], hi[1]));
    let (lo, hi) = ([lo[0] - px, lo[1] - pz], [hi[0] + px, hi[1] + pz]);
    let (w, h) = (width as f32, height as f32);
    let view = MapView::fit(
        lo,
        hi,
        [
            (w - 2.0 * MARGIN_PX).max(8.0),
            (h - 2.0 * MARGIN_PX).max(8.0),
        ],
    );
    let mid = [w * 0.5, h * 0.5];
    let pal = Palette::of(shade);
    let mut pm = Pixmap::new(width, height)?;
    let g = pal.ground;
    pm.fill(Color::from_rgba8(g[0], g[1], g[2], 255));
    let screen: Vec<[f32; 2]> = metres.iter().map(|&m| view.to_screen(m, mid)).collect();
    let mut pb = PathBuilder::new();
    for (i, p) in screen.iter().enumerate() {
        if i == 0 {
            pb.move_to(p[0], p[1]);
        } else {
            pb.line_to(p[0], p[1]);
        }
    }
    let line = (w.min(h) / 64.0).clamp(2.0, 6.0);
    if let Some(path) = pb.finish() {
        pm.stroke_path(
            &path,
            &paint(pal.route_casing),
            &stroke(line + 3.0),
            Transform::identity(),
            None,
        );
        pm.stroke_path(
            &path,
            &paint(pal.route),
            &stroke(line),
            Transform::identity(),
            None,
        );
    }
    let r = line + 2.0;
    let (start, end) = (screen[0], screen[screen.len() - 1]);
    chequer(&mut pm, end, r, pal.rider_ring);
    dot(&mut pm, start, r, [40, 170, 70, 255], pal.rider_ring);
    // The scale bar: at most a third of the width.
    let (bar_px, scale) = scale_bar(view.m_per_px, f64::from(w) / 3.0, imperial);
    let bar_px = bar_px as f32;
    let y = h - SCALE_INSET_PX;
    if let Some(rect) = Rect::from_xywh(SCALE_INSET_PX, y - 3.0, bar_px, 3.0) {
        pm.fill_rect(rect, &paint(pal.casing), Transform::identity(), None);
    }
    for x in [SCALE_INSET_PX, SCALE_INSET_PX + bar_px - 2.0] {
        if let Some(rect) = Rect::from_xywh(x, y - 8.0, 2.0, 8.0) {
            pm.fill_rect(rect, &paint(pal.casing), Transform::identity(), None);
        }
    }
    let mut rgba = pm.take();
    unpremultiply(&mut rgba);
    Some(RideMapImage {
        width,
        height,
        rgba,
        scale,
        scale_px: bar_px,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop of about 2.5 km near Freiburg (made up), a point a metre.
    fn lap() -> Vec<[f64; 2]> {
        (0..2500)
            .map(|i| {
                let a = i as f64 / 2500.0 * std::f64::consts::TAU;
                [47.99 + 0.0036 * a.sin(), 7.85 + 0.0053 * a.cos()]
            })
            .collect()
    }

    #[test]
    fn the_projection_round_trips_and_points_north_up() {
        let o = [47.99, 7.85];
        let north = to_metres([48.0, 7.85], o);
        assert!(
            north[0].abs() < 1e-6 && (north[1] + 1111.95).abs() < 0.1,
            "{north:?}"
        );
        let east = to_metres([47.99, 7.86], o);
        assert!(
            east[0] > 740.0 && east[0] < 750.0 && east[1].abs() < 1e-9,
            "{east:?}"
        );
        let back = from_metres(to_metres([47.993, 7.861], o), o);
        assert!((back[0] - 47.993).abs() < 1e-9 && (back[1] - 7.861).abs() < 1e-9);
    }

    #[test]
    fn a_track_is_thinned_to_its_limit_and_keeps_its_shape() {
        let full = lap();
        let kept = simplify(&full, TRACK_MAX_POINTS);
        assert!(
            kept.len() >= 20 && kept.len() <= TRACK_MAX_POINTS,
            "{}",
            kept.len()
        );
        assert_eq!(kept[0], [round(full[0][0]), round(full[0][1])]);
        // The length stays within 1 % of the loop's.
        let (a, b) = (length_m(&full), length_m(&kept));
        assert!((a - b).abs() / a < 0.01, "{a} vs {b}");
        // A tight limit thins further.
        assert!(simplify(&full, 12).len() <= 12);
        // Straight: the ends only.
        let line: Vec<[f64; 2]> = (0..100).map(|i| [48.0 + i as f64 * 1e-4, 7.9]).collect();
        assert_eq!(simplify(&line, TRACK_MAX_POINTS).len(), 2);
    }

    #[test]
    fn no_track_without_two_positions() {
        assert!(simplify(&[], 300).is_empty());
        assert!(simplify(&[[48.0, 7.9]; 50], 300).is_empty());
        assert!(simplify(&[[f64::NAN, 7.9], [48.0, 7.9]], 300).is_empty());
        assert!(render(&[[48.0, 7.9]], 128, 128, false, Shade::Day).is_none());
    }

    #[test]
    fn the_map_draws_the_track_start_finish_and_scale() {
        let track = simplify(&lap(), TRACK_MAX_POINTS);
        let img = render(&track, 200, 160, false, Shade::Day).expect("a map");
        assert_eq!((img.width, img.height), (200, 160));
        assert_eq!(img.rgba.len(), 200 * 160 * 4);
        let px = |x: u32, y: u32| {
            let i = ((y * img.width + x) * 4) as usize;
            [
                img.rgba[i],
                img.rgba[i + 1],
                img.rgba[i + 2],
                img.rgba[i + 3],
            ]
        };
        let pal = Palette::of(Shade::Day);
        // The centre of the loop is open ground; the route colour is drawn.
        assert_eq!(px(100, 80), pal.ground);
        let route = img
            .rgba
            .chunks(4)
            .filter(|p| p[..3] == pal.route[..3])
            .count();
        assert!(route > 300, "{route} route pixels");
        // A start dot in green.
        assert!(img.rgba.chunks(4).any(|p| p == [40, 170, 70, 255]));
        // The loop is about 790 m wide: the bar is a "nice" length.
        assert!(
            ["100 m", "200 m", "250 m", "500 m"].contains(&img.scale.as_str()),
            "{}",
            img.scale
        );
        assert!(img.scale_px > 10.0 && img.scale_px <= 200.0 / 3.0 + 0.5);
        let imp = render(&track, 200, 160, true, Shade::Day).unwrap();
        assert!(
            imp.scale.ends_with("ft") || imp.scale.ends_with("mi"),
            "{}",
            imp.scale
        );
        // The same track, the same pixels.
        assert_eq!(render(&track, 200, 160, false, Shade::Day), Some(img));
    }
}
