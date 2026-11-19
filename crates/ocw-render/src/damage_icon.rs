//! The damage widget's two icons drawn in code (spec 14 Phase 12F, the
//! user 2026-11-19: "rework the bike logo for the bike condition – smaller,
//! and make it look like a bike"): a side-view **bicycle** silhouette – two
//! round wheels with their tyres, the frame triangle (top tube, down tube,
//! seat tube, seat stays, chain stays) with the head tube and the fork, the
//! chainring with the chain, the stem with a drop bar, the seat post with
//! the saddle – and a small **rider** (head, body, legs).
//!
//! Each damageable part is its own layer (a white mask the widget tints in
//! the part's status colour), so a puncture lights the tyre, a buckled
//! wheel the rim and hub, a broken chain the chainring and chain, a bent
//! bar the stem and bar, frame damage the tubes and the fork. The shapes
//! are signed-distance capsules, rings and discs in icon pixels (y down),
//! rasterised with anti-aliased edges at [`ICON_SCALE`] texels per pixel –
//! plain RGBA8 images, so the same in both apps (WebGPU and WebGL 2).

use bevy::prelude::*;
use ocw_sim::damage::Part;

/// The bike icon's size (UI px): two thirds of the old 48 px high icon.
pub const BIKE_ICON: Vec2 = Vec2::new(48.0, 32.0);
/// The rider icon's size (UI px), as high as the bike.
pub const RIDER_ICON: Vec2 = Vec2::new(22.0, 32.0);
/// Texels per UI pixel of the icon images (2: sharp at HUD 100 % and on a
/// 2× screen, smooth when sampled down).
pub const ICON_SCALE: u32 = 2;

/// A piece of an icon (icon px, y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A segment `a`–`b` with round ends, `r` thick on each side.
    Capsule(Vec2, Vec2, f32),
    /// A ring around `c` from radius `inner` to `outer`.
    Ring(Vec2, f32, f32),
    /// A filled circle.
    Disc(Vec2, f32),
}

impl Shape {
    /// Signed distance from `p` (negative inside).
    pub fn distance(self, p: Vec2) -> f32 {
        match self {
            Shape::Capsule(a, b, r) => {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (p - (a + ab * t)).length() - r
            }
            Shape::Ring(c, inner, outer) => {
                let mid = (inner + outer) * 0.5;
                ((p - c).length() - mid).abs() - (outer - inner) * 0.5
            }
            Shape::Disc(c, r) => (p - c).length() - r,
        }
    }
}

/// A layer of the bike icon: a damageable part, or the saddle and seat
/// post (never damaged: always the fine grey).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BikeLayer {
    Part(Part),
    Saddle,
}

/// The bike's layers in drawing order (later on top): the wheels under
/// their tyres, the drivetrain under the frame, the bar on top.
pub const BIKE_LAYERS: [BikeLayer; 8] = [
    BikeLayer::Part(Part::RearWheel),
    BikeLayer::Part(Part::FrontWheel),
    BikeLayer::Part(Part::RearTyre),
    BikeLayer::Part(Part::FrontTyre),
    BikeLayer::Part(Part::Drivetrain),
    BikeLayer::Part(Part::Frame),
    BikeLayer::Saddle,
    BikeLayer::Part(Part::Cockpit),
];

/// The bike's geometry (icon px): the axles, the bottom bracket, the top
/// of the seat tube, the head tube's top and bottom.
pub const REAR_AXLE: Vec2 = Vec2::new(10.4, 21.5);
pub const FRONT_AXLE: Vec2 = Vec2::new(37.6, 21.5);
pub const BOTTOM_BRACKET: Vec2 = Vec2::new(21.5, 22.5);
const SEAT_CLUSTER: Vec2 = Vec2::new(18.2, 10.0);
const HEAD_TOP: Vec2 = Vec2::new(34.0, 10.0);
const HEAD_BOTTOM: Vec2 = Vec2::new(34.6, 13.2);
/// The tyre's outer radius and its thickness (icon px).
pub const WHEEL_R: f32 = 9.6;
pub const TYRE: f32 = 2.3;
/// The rim's ring (icon px) inside the tyre.
const RIM: (f32, f32) = (5.6, 6.6);
const HUB_R: f32 = 1.3;
/// The chainring's ring (icon px).
const CHAINRING: (f32, f32) = (2.4, 3.6);

/// The shapes of a bike layer.
pub fn bike_shapes(layer: BikeLayer) -> Vec<Shape> {
    use Shape::{Capsule, Disc, Ring};
    let wheel = |c: Vec2| vec![Ring(c, RIM.0, RIM.1), Disc(c, HUB_R)];
    let tyre = |c: Vec2| vec![Ring(c, WHEEL_R - TYRE, WHEEL_R)];
    match layer {
        BikeLayer::Part(Part::RearWheel) => wheel(REAR_AXLE),
        BikeLayer::Part(Part::FrontWheel) => wheel(FRONT_AXLE),
        BikeLayer::Part(Part::RearTyre) => tyre(REAR_AXLE),
        BikeLayer::Part(Part::FrontTyre) => tyre(FRONT_AXLE),
        // The chainring and the chain's lower run to the rear sprocket
        // (the upper run lies along the chain stay).
        BikeLayer::Part(Part::Drivetrain) => vec![
            Ring(BOTTOM_BRACKET, CHAINRING.0, CHAINRING.1),
            Capsule(
                BOTTOM_BRACKET + Vec2::new(0.0, CHAINRING.1 - 0.6),
                REAR_AXLE + Vec2::new(0.0, 1.6),
                0.6,
            ),
        ],
        // The main triangle, the rear triangle, the head tube and the fork.
        BikeLayer::Part(Part::Frame) => vec![
            Capsule(SEAT_CLUSTER, HEAD_TOP, 1.05),
            Capsule(BOTTOM_BRACKET, HEAD_BOTTOM, 1.2),
            Capsule(BOTTOM_BRACKET, SEAT_CLUSTER, 1.05),
            Capsule(SEAT_CLUSTER, REAR_AXLE, 0.85),
            Capsule(BOTTOM_BRACKET, REAR_AXLE, 0.85),
            Capsule(HEAD_TOP, HEAD_BOTTOM, 1.25),
            Capsule(HEAD_BOTTOM, FRONT_AXLE, 0.95),
            Disc(BOTTOM_BRACKET, 1.6),
        ],
        BikeLayer::Saddle => vec![
            Capsule(Vec2::new(17.0, 4.6), SEAT_CLUSTER, 0.75),
            Capsule(Vec2::new(13.8, 4.4), Vec2::new(20.4, 4.2), 1.05),
        ],
        // The stem rising forwards and a drop bar.
        BikeLayer::Part(Part::Cockpit) => vec![
            Capsule(HEAD_TOP, Vec2::new(36.6, 7.4), 0.9),
            Capsule(Vec2::new(36.6, 7.4), Vec2::new(40.4, 7.6), 0.9),
            Capsule(Vec2::new(40.4, 7.6), Vec2::new(41.6, 9.6), 0.85),
            Capsule(Vec2::new(41.6, 9.6), Vec2::new(40.2, 11.4), 0.8),
        ],
    }
}

/// The rider: head, body, legs.
pub fn rider_shapes() -> Vec<Shape> {
    use Shape::{Capsule, Disc};
    vec![
        Disc(Vec2::new(11.0, 5.2), 4.4),
        Capsule(Vec2::new(11.0, 13.0), Vec2::new(11.0, 18.0), 5.0),
        Capsule(Vec2::new(8.2, 22.0), Vec2::new(8.2, 29.8), 2.2),
        Capsule(Vec2::new(13.8, 22.0), Vec2::new(13.8, 29.8), 2.2),
    ]
}

/// Whether point `p` (icon px) is drawn by `shapes`.
pub fn covers(shapes: &[Shape], p: Vec2) -> bool {
    distance(shapes, p) <= 0.0
}

fn distance(shapes: &[Shape], p: Vec2) -> f32 {
    shapes
        .iter()
        .map(|s| s.distance(p))
        .fold(f32::MAX, f32::min)
}

/// The mask of `shapes` in an icon of `size` px: white RGBA8 with the
/// coverage as alpha, [`ICON_SCALE`] texels per px, row-major from the top.
pub fn mask(shapes: &[Shape], size: Vec2) -> (UVec2, Vec<u8>) {
    let s = ICON_SCALE as f32;
    let dim = UVec2::new((size.x * s).round() as u32, (size.y * s).round() as u32);
    let mut out = Vec::with_capacity((dim.x * dim.y * 4) as usize);
    for y in 0..dim.y {
        for x in 0..dim.x {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / s;
            // Anti-aliased over one texel.
            let a = (0.5 - distance(shapes, p) * s).clamp(0.0, 1.0);
            out.extend_from_slice(&[255, 255, 255, (a * 255.0).round() as u8]);
        }
    }
    (dim, out)
}

/// A mask as an image for the UI.
pub fn mask_image(shapes: &[Shape], size: Vec2) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (dim, data) = mask(shapes, size);
    Image::new(
        Extent3d {
            width: dim.x,
            height: dim.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer_at(p: Vec2) -> Vec<BikeLayer> {
        BIKE_LAYERS
            .into_iter()
            .filter(|l| covers(&bike_shapes(*l), p))
            .collect()
    }

    /// Each part's highlight sits on that part of the drawing, and only
    /// there: the tyre's outer edge, the rim, the chainring, the bar's
    /// drop, the top tube and the fork (spec 14 Phase 12F).
    #[test]
    fn each_part_lights_up_where_it_is_drawn() {
        use BikeLayer::Part as P;
        let cases = [
            // The tyres' outer edges, behind and ahead.
            (
                Vec2::new(REAR_AXLE.x - WHEEL_R + 1.0, REAR_AXLE.y),
                P(Part::RearTyre),
            ),
            (
                Vec2::new(FRONT_AXLE.x + WHEEL_R - 1.0, FRONT_AXLE.y),
                P(Part::FrontTyre),
            ),
            (
                Vec2::new(REAR_AXLE.x, REAR_AXLE.y - WHEEL_R + 1.0),
                P(Part::RearTyre),
            ),
            // The rims at the bottom.
            (REAR_AXLE + Vec2::new(0.0, 6.1), P(Part::RearWheel)),
            (FRONT_AXLE + Vec2::new(0.0, 6.1), P(Part::FrontWheel)),
            (REAR_AXLE + Vec2::new(-6.1, 0.0), P(Part::RearWheel)),
            // The chainring under the bottom bracket.
            (BOTTOM_BRACKET + Vec2::new(0.0, 3.0), P(Part::Drivetrain)),
            (BOTTOM_BRACKET + Vec2::new(3.0, 0.0), P(Part::Drivetrain)),
            // The top tube's middle, the fork's middle.
            ((SEAT_CLUSTER + HEAD_TOP) * 0.5, P(Part::Frame)),
            ((HEAD_BOTTOM + FRONT_AXLE) * 0.5, P(Part::Frame)),
            // The bar's drop, the stem.
            (Vec2::new(41.6, 9.6), P(Part::Cockpit)),
            (Vec2::new(35.3, 8.7), P(Part::Cockpit)),
            // The saddle.
            (Vec2::new(15.0, 4.4), BikeLayer::Saddle),
        ];
        for (p, want) in cases {
            assert_eq!(layer_at(p), [want], "{p}");
        }
    }

    #[test]
    fn rear_parts_are_behind_and_front_parts_ahead() {
        let centre = |l: BikeLayer| {
            let shapes = bike_shapes(l);
            let (mask_dim, data) = mask(&shapes, BIKE_ICON);
            let (mut sum, mut n) = (0.0, 0.0);
            for (i, px) in data.chunks(4).enumerate() {
                let a = f32::from(px[3]) / 255.0;
                sum += (i as u32 % mask_dim.x) as f32 * a;
                n += a;
            }
            sum / n / ICON_SCALE as f32
        };
        let mid = BIKE_ICON.x / 2.0;
        for p in [Part::RearWheel, Part::RearTyre, Part::Drivetrain] {
            assert!(centre(BikeLayer::Part(p)) < mid, "{p:?}");
        }
        for p in [Part::FrontWheel, Part::FrontTyre, Part::Cockpit] {
            assert!(centre(BikeLayer::Part(p)) > mid, "{p:?}");
        }
    }

    /// Smaller than before (48 px high) and a bicycle's proportions: two
    /// round wheels side by side filling the width, the frame between them
    /// above the axles, everything inside the icon.
    #[test]
    fn the_bike_is_a_bike_two_thirds_of_the_old_size() {
        assert_eq!(BIKE_ICON.y, 48.0 * 2.0 / 3.0);
        assert_eq!(RIDER_ICON.y, BIKE_ICON.y);
        let (dim, _) = mask(&bike_shapes(BIKE_LAYERS[0]), BIKE_ICON);
        assert_eq!(dim, UVec2::new(96, 64));
        // Wheels round, the same size, apart by more than their diameter.
        const { assert!(FRONT_AXLE.x - REAR_AXLE.x > 2.0 * WHEEL_R) };
        assert_eq!(REAR_AXLE.y, FRONT_AXLE.y);
        // Inside the icon: the wheels touch neither edge, no layer draws
        // on the border.
        const { assert!(REAR_AXLE.x - WHEEL_R > 0.0 && FRONT_AXLE.x + WHEEL_R < BIKE_ICON.x) };
        const { assert!(REAR_AXLE.y + WHEEL_R < BIKE_ICON.y) };
        let mut width = (f32::MAX, f32::MIN);
        for l in BIKE_LAYERS {
            let (dim, data) = mask(&bike_shapes(l), BIKE_ICON);
            for (i, px) in data.chunks(4).enumerate() {
                let (x, y) = (i as u32 % dim.x, i as u32 / dim.x);
                if px[3] > 0 {
                    assert!(x > 0 && y > 0 && x < dim.x - 1 && y < dim.y - 1, "{l:?}");
                    width = (width.0.min(x as f32), width.1.max(x as f32));
                }
            }
        }
        assert!(
            (width.1 - width.0) / ICON_SCALE as f32 > 0.9 * BIKE_ICON.x,
            "fills the width"
        );
        // The frame triangle: the top tube above the axles, the bottom
        // bracket between them, the saddle and the bar on top.
        const { assert!(SEAT_CLUSTER.y < REAR_AXLE.y - WHEEL_R / 2.0) };
        assert!((REAR_AXLE.x..FRONT_AXLE.x).contains(&BOTTOM_BRACKET.x));
        assert!(covers(
            &bike_shapes(BikeLayer::Saddle),
            Vec2::new(17.0, 4.4)
        ));
        // The wheel's hole: the tyre is a ring, not a disc.
        for l in BIKE_LAYERS {
            if l != BikeLayer::Part(Part::Frame) {
                assert!(
                    !covers(&bike_shapes(l), REAR_AXLE + Vec2::new(-3.5, -3.5)),
                    "{l:?}"
                );
            }
        }
    }

    #[test]
    fn the_rider_fits_its_icon() {
        let (dim, data) = mask(&rider_shapes(), RIDER_ICON);
        assert_eq!(dim, UVec2::new(44, 64));
        let drawn = data.chunks(4).filter(|p| p[3] > 128).count();
        let share = drawn as f32 / (dim.x * dim.y) as f32;
        assert!((0.3..0.7).contains(&share), "{share}");
        // Head on top, the legs at the bottom, apart.
        assert!(covers(&rider_shapes(), Vec2::new(11.0, 3.0)));
        assert!(covers(&rider_shapes(), Vec2::new(8.2, 30.0)));
        assert!(!covers(&rider_shapes(), Vec2::new(11.0, 28.0)));
    }

    #[test]
    fn masks_are_white_with_soft_edges() {
        let (_, data) = mask(&bike_shapes(BikeLayer::Part(Part::RearTyre)), BIKE_ICON);
        assert!(
            data.chunks(4)
                .all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
        );
        let alphas: Vec<u8> = data.chunks(4).map(|p| p[3]).collect();
        assert!(alphas.contains(&255) && alphas.contains(&0));
        assert!(alphas.iter().any(|a| (40..215).contains(a)), "anti-aliased");
    }
}
