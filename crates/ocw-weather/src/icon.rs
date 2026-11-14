//! Weather icons (spec §14 Phase 9, S5: the weather machine's preset grid;
//! later the HUD's weather icon, 9E): what an icon shows follows from a
//! [`WeatherState`] ([`Icon::of`], so a custom weather gets one too), and
//! [`rgba`] draws it – anti-aliased signed-distance shapes (sun and rays,
//! cloud, drops, flakes, hail, a bolt, fog bars, wind lines) on a
//! transparent square, own work, no font or image assets. Pure, so the
//! browser app's menu (B6) draws the same.

use crate::state::{CloudType, Precip, WeatherState};

/// The sun in an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sun {
    None,
    /// Alone in a clear sky, or beside a small cloud.
    Full,
    /// Peeking out behind a big cloud.
    Behind,
}

/// The cloud in an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cloud {
    None,
    /// A small white cloud (fair weather).
    Small,
    /// A big white cloud.
    White,
    /// Grey (overcast, rain, snow).
    Grey,
    /// Dark (a thunder cloud).
    Dark,
}

/// What falls in an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fall {
    None,
    Drizzle,
    /// Rain with 1–3 levels of intensity.
    Rain(u8),
    /// Snow with 1–2 levels.
    Snow(u8),
    Sleet,
    Hail,
}

/// What an icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icon {
    pub sun: Sun,
    pub cloud: Cloud,
    pub fall: Fall,
    pub bolt: bool,
    /// Fog: 0 none, 1 mist, 2 dense.
    pub fog: u8,
    pub wind: bool,
}

impl Icon {
    /// The icon of a weather.
    pub fn of(w: &WeatherState) -> Self {
        let c = w.cloud_cover;
        let fall = if w.precipitating() {
            let mm = w.precip_mm_h;
            match w.precip {
                Precip::Drizzle => Fall::Drizzle,
                Precip::Rain if mm < 2.0 => Fall::Rain(1),
                Precip::Rain if mm < 8.0 => Fall::Rain(2),
                Precip::Rain => Fall::Rain(3),
                Precip::Snow if mm < 3.0 => Fall::Snow(1),
                Precip::Snow => Fall::Snow(2),
                Precip::Sleet => Fall::Sleet,
                Precip::Hail => Fall::Hail,
                Precip::None => Fall::None,
            }
        } else {
            Fall::None
        };
        let bolt = w.lightning_per_min > 0.2;
        let fog = match w.fog_visibility_m {
            v if v < 300.0 => 2,
            v if v < 1000.0 => 1,
            _ => 0,
        };
        let cloud = if fog == 2 {
            Cloud::None
        } else if w.cloud_type == CloudType::Cumulonimbus || bolt {
            Cloud::Dark
        } else if c < 0.15 && fall == Fall::None {
            Cloud::None
        } else if fall != Fall::None || c >= 0.85 {
            Cloud::Grey
        } else if c < 0.45 {
            Cloud::Small
        } else {
            Cloud::White
        };
        let sun = match cloud {
            Cloud::None if fog == 0 => Sun::Full,
            Cloud::Small => Sun::Full,
            Cloud::White => Sun::Behind,
            _ => Sun::None,
        };
        Self {
            sun,
            cloud,
            fall,
            bolt,
            fog,
            // A windy day and a gale (a thunderstorm's 9 m/s: the bolt says enough).
            wind: w.wind_mps >= 10.0,
        }
    }
}

type Rgb = [f32; 3];

const SUN: Rgb = [1.0, 0.79, 0.16];
const CLOUD_WHITE: Rgb = [0.96, 0.97, 0.98];
const CLOUD_GREY: Rgb = [0.66, 0.70, 0.75];
const CLOUD_DARK: Rgb = [0.33, 0.36, 0.43];
const OUTLINE: Rgb = [0.18, 0.2, 0.25];
const RAIN: Rgb = [0.33, 0.65, 1.0];
const SNOW: Rgb = [0.93, 0.97, 1.0];
const HAIL: Rgb = [0.86, 0.91, 0.96];
const BOLT: Rgb = [1.0, 0.86, 0.18];
const FOG: Rgb = [0.74, 0.77, 0.81];
const WIND: Rgb = [0.78, 0.86, 0.92];

/// A shape as a signed distance (in icon units, 0–1; negative inside).
enum Shape {
    Circle([f32; 2], f32),
    Capsule([f32; 2], [f32; 2], f32),
    Polygon(Vec<[f32; 2]>),
    /// The union of shapes (one outline, no seams).
    Union(Vec<Shape>),
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn len(a: [f32; 2]) -> f32 {
    dot(a, a).sqrt()
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (pa, ba) = (sub(p, a), sub(b, a));
    let h = (dot(pa, ba) / dot(ba, ba).max(1e-12)).clamp(0.0, 1.0);
    len([pa[0] - ba[0] * h, pa[1] - ba[1] * h])
}

impl Shape {
    fn distance(&self, p: [f32; 2]) -> f32 {
        match self {
            Shape::Circle(c, r) => len(sub(p, *c)) - r,
            Shape::Capsule(a, b, r) => segment_distance(p, *a, *b) - r,
            Shape::Polygon(pts) => {
                let mut d = f32::MAX;
                let mut inside = false;
                let n = pts.len();
                for i in 0..n {
                    let (a, b) = (pts[i], pts[(i + n - 1) % n]);
                    d = d.min(segment_distance(p, a, b));
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                if inside { -d } else { d }
            }
            Shape::Union(parts) => parts.iter().map(|s| s.distance(p)).fold(f32::MAX, f32::min),
        }
    }
}

/// One layer: a shape filled with a colour, optionally outlined.
struct Layer {
    shape: Shape,
    color: Rgb,
    outline: Option<(Rgb, f32)>,
}

fn layer(shape: Shape, color: Rgb) -> Layer {
    Layer {
        shape,
        color,
        outline: None,
    }
}

fn outlined(shape: Shape, color: Rgb, width: f32) -> Layer {
    Layer {
        shape,
        color,
        outline: Some((OUTLINE, width)),
    }
}

/// A cloud centred at `c`, `s` times the full size.
fn cloud(c: [f32; 2], s: f32) -> Shape {
    let at = |x: f32, y: f32| [c[0] + x * s, c[1] + y * s];
    Shape::Union(vec![
        Shape::Circle(at(-0.17, 0.03), 0.13 * s),
        Shape::Circle(at(0.0, -0.07), 0.17 * s),
        Shape::Circle(at(0.17, 0.02), 0.12 * s),
        Shape::Capsule(at(-0.18, 0.08), at(0.18, 0.08), 0.09 * s),
    ])
}

fn sun(layers: &mut Vec<Layer>, c: [f32; 2], r: f32) {
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (s, co) = a.sin_cos();
        layers.push(layer(
            Shape::Capsule(
                [c[0] + co * r * 1.4, c[1] + s * r * 1.4],
                [c[0] + co * r * 1.95, c[1] + s * r * 1.95],
                r * 0.16,
            ),
            SUN,
        ));
    }
    layers.push(layer(Shape::Circle(c, r), SUN));
}

fn flake(layers: &mut Vec<Layer>, c: [f32; 2], r: f32) {
    for i in 0..3 {
        let a = i as f32 * std::f32::consts::PI / 3.0 + std::f32::consts::FRAC_PI_2;
        let (s, co) = a.sin_cos();
        layers.push(layer(
            Shape::Capsule(
                [c[0] - co * r, c[1] - s * r],
                [c[0] + co * r, c[1] + s * r],
                r * 0.22,
            ),
            SNOW,
        ));
    }
}

fn drop(layers: &mut Vec<Layer>, x: f32, y: f32, long: f32) {
    layers.push(layer(
        Shape::Capsule([x + 0.035, y], [x - 0.035 * long, y + 0.09 * long], 0.026),
        RAIN,
    ));
}

/// The icon's shapes, back to front.
fn layers(icon: Icon) -> Vec<Layer> {
    let mut l = Vec::new();
    let falls = icon.fall != Fall::None || icon.bolt;
    // The sun first (clouds cover it).
    match (icon.sun, icon.cloud) {
        (Sun::Full, Cloud::Small) => sun(&mut l, [0.4, 0.38], 0.15),
        (Sun::Full, _) => sun(&mut l, [0.5, 0.5], 0.19),
        (Sun::Behind, _) => sun(&mut l, [0.34, 0.32], 0.13),
        (Sun::None, _) => {}
    }
    let cloud_at = if falls || icon.fog == 1 {
        [0.5, 0.42]
    } else {
        [0.53, 0.53]
    };
    let w = 0.022;
    match icon.cloud {
        Cloud::None => {}
        Cloud::Small => l.push(outlined(cloud([0.62, 0.64], 0.62), CLOUD_WHITE, w)),
        Cloud::White => l.push(outlined(cloud(cloud_at, 1.0), CLOUD_WHITE, w)),
        Cloud::Grey => l.push(outlined(cloud(cloud_at, 1.0), CLOUD_GREY, w)),
        Cloud::Dark => l.push(outlined(cloud(cloud_at, 1.0), CLOUD_DARK, w)),
    }
    // What falls, under the cloud (to the right when the wind blows).
    let xs: &[f32] = match (icon.fall, icon.wind) {
        (Fall::Rain(3), false) | (Fall::Snow(2), false) => &[0.3, 0.44, 0.58, 0.72],
        (_, true) => &[0.58, 0.74],
        _ => &[0.36, 0.52, 0.68],
    };
    let y = 0.7;
    match icon.fall {
        Fall::None => {}
        Fall::Drizzle => {
            for (i, x) in xs.iter().enumerate() {
                let dy = if i % 2 == 0 { 0.0 } else { 0.08 };
                l.push(layer(Shape::Circle([*x, y + 0.04 + dy], 0.022), RAIN));
            }
        }
        Fall::Rain(n) => {
            let picked: Vec<f32> = match n {
                1 => vec![xs[0], xs[xs.len() - 1]],
                _ => xs.to_vec(),
            };
            for (i, x) in picked.iter().enumerate() {
                let dy = if i % 2 == 0 { 0.0 } else { 0.06 };
                drop(&mut l, *x, y + dy, if n >= 3 { 1.5 } else { 1.0 });
            }
        }
        Fall::Snow(n) => {
            let picked: Vec<f32> = match n {
                1 => vec![xs[0], xs[xs.len() - 1]],
                _ => xs.to_vec(),
            };
            for (i, x) in picked.iter().enumerate() {
                let dy = if i % 2 == 0 { 0.03 } else { 0.13 };
                flake(&mut l, [*x, y + dy + 0.04], 0.055);
            }
        }
        Fall::Sleet => {
            for (i, x) in xs.iter().enumerate() {
                if i % 2 == 0 {
                    drop(&mut l, *x, y, 1.0);
                } else {
                    flake(&mut l, [*x, y + 0.12], 0.05);
                }
            }
        }
        Fall::Hail => {
            for (i, x) in xs.iter().enumerate() {
                let dy = if i % 2 == 0 { 0.06 } else { 0.15 };
                l.push(outlined(Shape::Circle([*x, y + dy], 0.04), HAIL, 0.012));
            }
        }
    }
    if icon.bolt {
        l.push(outlined(
            Shape::Polygon(vec![
                [0.55, 0.5],
                [0.41, 0.73],
                [0.5, 0.73],
                [0.43, 0.95],
                [0.64, 0.66],
                [0.54, 0.66],
                [0.62, 0.5],
            ]),
            BOLT,
            0.012,
        ));
    }
    match icon.fog {
        1 => {
            for (i, y) in [0.74, 0.86].iter().enumerate() {
                let x0 = if i == 0 { 0.2 } else { 0.3 };
                l.push(layer(Shape::Capsule([x0, *y], [x0 + 0.52, *y], 0.032), FOG));
            }
        }
        2 => {
            for (i, y) in [0.28, 0.41, 0.54, 0.67, 0.8].iter().enumerate() {
                let (a, b) = if i % 2 == 0 {
                    (0.14, 0.86)
                } else {
                    (0.24, 0.76)
                };
                l.push(layer(Shape::Capsule([a, *y], [b, *y], 0.038), FOG));
            }
        }
        _ => {}
    }
    if icon.wind {
        // Two streaks with a curl at their end, lower left.
        let (y1, y2, x1) = if falls {
            (0.74, 0.86, 0.46)
        } else {
            (0.8, 0.9, 0.66)
        };
        for (y, x_end) in [(y1, x1), (y2, x1 - 0.12)] {
            l.push(layer(Shape::Capsule([0.08, y], [x_end, y], 0.022), WIND));
            l.push(layer(
                Shape::Capsule([x_end, y], [x_end + 0.05, y - 0.04], 0.022),
                WIND,
            ));
        }
    }
    l
}

/// The icon drawn into a `size` × `size` RGBA8 image (sRGB, straight
/// alpha, row by row from the top).
pub fn rgba(icon: Icon, size: u32) -> Vec<u8> {
    let size = size.max(8);
    let layers = layers(icon);
    let px = 1.0 / size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for j in 0..size {
        for i in 0..size {
            let p = [(i as f32 + 0.5) * px, (j as f32 + 0.5) * px];
            // Premultiplied colour, composited front over back.
            let mut acc = [0.0f32; 4];
            for layer in &layers {
                let d = layer.shape.distance(p);
                let mut paint = |color: Rgb, d: f32| {
                    let a = (0.5 - d / px).clamp(0.0, 1.0);
                    if a > 0.0 {
                        for k in 0..3 {
                            acc[k] = color[k] * a + acc[k] * (1.0 - a);
                        }
                        acc[3] = a + acc[3] * (1.0 - a);
                    }
                };
                if let Some((oc, ow)) = layer.outline {
                    paint(oc, d - ow);
                }
                paint(layer.color, d);
            }
            let o = ((j * size + i) * 4) as usize;
            let a = acc[3];
            for k in 0..3 {
                let c = if a > 0.0 { acc[k] / a } else { 0.0 };
                out[o + k] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out[o + 3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{PRESETS, preset};

    fn icon(name: &str) -> Icon {
        Icon::of(&preset(name).unwrap())
    }

    fn pixel(img: &[u8], size: u32, x: f32, y: f32) -> [u8; 4] {
        let (i, j) = ((x * size as f32) as u32, (y * size as f32) as u32);
        let o = ((j * size + i) * 4) as usize;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    }

    /// Pixels in a region that look like `pred`.
    fn count(img: &[u8], size: u32, y0: f32, y1: f32, pred: impl Fn([u8; 4]) -> bool) -> usize {
        (0..size * size)
            .filter(|k| {
                let y = (k / size) as f32 / size as f32;
                let o = (*k * 4) as usize;
                (y0..y1).contains(&y) && pred([img[o], img[o + 1], img[o + 2], img[o + 3]])
            })
            .count()
    }

    fn yellow(p: [u8; 4]) -> bool {
        p[3] > 200 && p[0] > 230 && p[1] > 170 && p[2] < 90
    }

    fn blue(p: [u8; 4]) -> bool {
        p[3] > 200 && p[2] > 220 && p[0] < 120
    }

    #[test]
    fn every_preset_has_its_own_icon() {
        let icons: Vec<Icon> = PRESETS.iter().map(|p| icon(p.0)).collect();
        for (i, a) in icons.iter().enumerate() {
            for (j, b) in icons.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "{} and {}", PRESETS[i].0, PRESETS[j].0);
            }
        }
        let images: Vec<Vec<u8>> = icons.iter().map(|i| rgba(*i, 48)).collect();
        for (i, a) in images.iter().enumerate() {
            // Something is drawn, the corners stay transparent.
            assert!(
                a.chunks(4).filter(|p| p[3] > 128).count() > 150,
                "{}",
                PRESETS[i].0
            );
            assert_eq!(a[3], 0, "{}", PRESETS[i].0);
            for b in images.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn icons_show_what_the_weather_is() {
        let s = 64;
        assert_eq!(icon("clear").sun, Sun::Full);
        let clear = rgba(icon("clear"), s);
        assert!(yellow(pixel(&clear, s, 0.5, 0.5)), "the sun in the middle");
        let rain = rgba(icon("rain"), s);
        assert!(
            count(&rain, s, 0.66, 1.0, blue) > 20,
            "drops below the cloud"
        );
        assert_eq!(count(&rain, s, 0.0, 1.0, yellow), 0, "no sun in the rain");
        let ts = rgba(icon("thunderstorm"), s);
        assert!(
            count(&ts, s, 0.6, 1.0, yellow) > 20,
            "a bolt below the cloud"
        );
        assert_eq!(icon("thunderstorm").cloud, Cloud::Dark);
        assert_eq!(icon("fog").fog, 2);
        assert_eq!(icon("mist").fog, 1);
        assert!(icon("windy").wind && icon("storm").wind);
        assert_eq!(icon("heavy-snow").fall, Fall::Snow(2));
        assert_eq!(icon("hail").fall, Fall::Hail);
        assert_eq!(icon("fair").cloud, Cloud::Small);
        assert_eq!(icon("cloudy").sun, Sun::Behind);
        // A custom weather gets an icon too.
        let mut w = preset("clear").unwrap();
        w.precip = Precip::Snow;
        w.precip_mm_h = 0.5;
        assert_eq!(Icon::of(&w).fall, Fall::Snow(1));
        assert_eq!(Icon::of(&w).cloud, Cloud::Grey);
        // Tiny sizes still work.
        assert_eq!(rgba(icon("storm"), 1).len(), 8 * 8 * 4);
    }
}
