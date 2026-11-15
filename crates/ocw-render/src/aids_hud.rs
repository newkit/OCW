//! The riding aids of spec §14 Phase 12D drawn as graphics (not text):
//!
//! - the **"last moment" warning** ([`HitWarningView`]): an amber warning
//!   triangle with an exclamation mark, flashing at [`FLASH_HZ`], with a
//!   small amber arrow towards the predicted hit and a short word under it
//!   ("Careful!", or "Sliding - lean less" when the tyres slide on a wet
//!   road);
//! - the **proximity arrow** ([`RideView::proximity_arrow`]): an arrow on a
//!   ring around the own rider, pointing at a rider close beside, in front
//!   or behind.
//!
//! And the view types of the chase camera's reaction to a crash
//! ([`CameraFx`], used by `camera.rs`). The app decides when to warn
//! (pure, `ocw_client::aids`); this module only lays the graphics out. The
//! pictures are made in code ([`warning_image`], [`arrow_image`]); the
//! flash runs on the session time, so captures are the same every run.

use bevy::prelude::*;

use crate::RideView;

/// The chase camera's reaction to the own rider's crash or a hard bump
/// (spec §14 Phase 12C): the shake and the wider orbit while down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CameraFx {
    /// The eye's shake (m: to the right, up, forwards) …
    pub shake: [f32; 3],
    /// … and the camera's roll (rad).
    pub roll: f32,
    /// How far the orbit is widened while down (0–1).
    pub orbit: f32,
    /// The side the rider fell to (+1 right, −1 left): the orbit turns
    /// that way.
    pub side: f32,
}

/// What the warning warns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// A predicted hit within about a second.
    Touch,
    /// The tyres slide (a lean beyond the wet road's grip, Phase 9 S6).
    Slide,
}

/// The "last moment" warning (12D).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitWarningView {
    pub kind: WarningKind,
    /// Seconds to the predicted hit (0 for a slide).
    pub in_s: f32,
    /// Direction to the hit (rad: 0 ahead, positive to the right).
    pub angle: f32,
}

/// The flash's rate (Hz).
pub const FLASH_HZ: f64 = 4.0;
/// Size of the pictures (px).
pub const IMAGE_PX: u32 = 64;
/// The warning triangle's size and where it stands (vh from the top).
const WARN_VH: f32 = 11.0;
const WARN_TOP_PCT: f32 = 22.0;
/// The proximity arrow: its size, the ring's radius and centre (the own
/// rider on the screen, % of the window).
const ARROW_VH: f32 = 7.0;
const RING_VH: f32 = 15.0;
const RING_TOP_PCT: f32 = 66.0;

/// The warning's amber and the arrow's colours (sRGB 0–255).
const AMBER: [u8; 3] = [250, 186, 36];
const ARROW: [u8; 3] = [255, 236, 150];
const OUTLINE: [u8; 3] = [24, 18, 6];

pub struct AidsHudPlugin;

impl Plugin for AidsHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aid {
    /// The warning's root (triangle, its arrow, its word).
    Warning,
    WarningTriangle,
    WarningArrow,
    WarningWord,
    /// The proximity arrow.
    Proximity,
}

/// Coverage (0–1) of a pixel at signed distance `d` (px, negative inside).
fn cover(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Signed distance to a convex polygon (counter-clockwise in screen y-down
/// space does not matter: the sign comes from the inside test).
fn poly_dist(p: Vec2, pts: &[Vec2]) -> f32 {
    let mut d = f32::MAX;
    let mut inside = true;
    let n = pts.len();
    let mut sign = 0.0f32;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        d = d.min(seg_dist(p, a, b));
        let c = (b - a).perp_dot(p - a);
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign && c != 0.0 {
            inside = false;
        }
    }
    if inside { -d } else { d }
}

/// Paints a shape given by its signed distance: `fill` inside, `outline`
/// in a band of `line` px around it (RGBA8, row-major from the top).
fn paint(px: u32, line: f32, fill: [u8; 3], dist: impl Fn(Vec2) -> (f32, f32)) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; (px * px) as usize];
    for y in 0..px {
        for x in 0..px {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let (shape, mark) = dist(p);
            let outer = cover(shape - line);
            let inner = cover(shape);
            // The mark (the exclamation mark) is drawn in the outline colour.
            let ink = cover(mark);
            let fill_share = (inner - ink).max(0.0);
            let a = outer;
            if a <= 0.0 {
                continue;
            }
            let mix = |f: u8, o: u8| {
                let f = f32::from(f) * fill_share + f32::from(o) * (a - fill_share);
                (f / a).round().clamp(0.0, 255.0) as u8
            };
            out[(y * px + x) as usize] = [
                mix(fill[0], OUTLINE[0]),
                mix(fill[1], OUTLINE[1]),
                mix(fill[2], OUTLINE[2]),
                (a * 255.0).round() as u8,
            ];
        }
    }
    out
}

/// The warning triangle (amber, a dark outline and exclamation mark),
/// `px` × `px`, pointing up.
pub fn warning_image(px: u32) -> Vec<[u8; 4]> {
    let s = px as f32;
    let tri = [
        Vec2::new(s * 0.5, s * 0.08),
        Vec2::new(s * 0.94, s * 0.88),
        Vec2::new(s * 0.06, s * 0.88),
    ];
    let line = s * 0.05;
    paint(px, line, AMBER, |p| {
        // Shrunk by the outline so the outline stays inside the picture.
        let shape = poly_dist(p, &tri) + line;
        let bar = seg_dist(
            p,
            Vec2::new(s * 0.5, s * 0.36),
            Vec2::new(s * 0.5, s * 0.62),
        ) - s * 0.055;
        let dot = (p - Vec2::new(s * 0.5, s * 0.75)).length() - s * 0.06;
        (shape, bar.min(dot))
    })
}

/// The arrow (pale yellow, a dark outline), `px` × `px`, pointing up.
pub fn arrow_image(px: u32) -> Vec<[u8; 4]> {
    let s = px as f32;
    let head = [
        Vec2::new(s * 0.5, s * 0.06),
        Vec2::new(s * 0.9, s * 0.5),
        Vec2::new(s * 0.1, s * 0.5),
    ];
    let line = s * 0.05;
    paint(px, line, ARROW, |p| {
        let h = poly_dist(p, &head) + line;
        let shaft = seg_dist(
            p,
            Vec2::new(s * 0.5, s * 0.45),
            Vec2::new(s * 0.5, s * 0.86),
        ) - s * 0.12
            + line;
        (h.min(shaft), f32::MAX)
    })
}

/// The warning's alpha at session time `t_s` (flashing, never quite gone).
pub fn flash_alpha(t_s: f64) -> f32 {
    (0.6 + 0.4 * (std::f64::consts::TAU * FLASH_HZ * t_s).cos()) as f32
}

/// The proximity arrow's offset from the ring's centre (vh: right, down)
/// for a rider in direction `angle` (rad: 0 ahead = up on the screen).
pub fn ring_offset(angle: f32) -> Vec2 {
    Vec2::new(angle.sin(), -angle.cos()) * RING_VH
}

/// The word under the warning.
pub fn warning_word(w: &HitWarningView) -> &'static str {
    match w.kind {
        WarningKind::Touch => "Careful!",
        WarningKind::Slide => "Sliding - lean less",
    }
}

fn picture(images: Option<&mut Assets<Image>>, rgba: Vec<[u8; 4]>) -> Option<Handle<Image>> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    Some(images?.add(Image::new(
        Extent3d {
            width: IMAGE_PX,
            height: IMAGE_PX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.into_iter().flatten().collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )))
}

fn spawn(mut commands: Commands, mut images: Option<ResMut<Assets<Image>>>) {
    let warning = picture(images.as_deref_mut(), warning_image(IMAGE_PX));
    let arrow = picture(images.as_deref_mut(), arrow_image(IMAGE_PX));
    let image = |h: &Option<Handle<Image>>| h.clone().map(ImageNode::new).unwrap_or_default();
    // The warning: centred near the top, under the HUD's top line.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(50.0),
                top: percent(WARN_TOP_PCT),
                width: px(0.0),
                height: px(0.0),
                ..default()
            },
            Visibility::Hidden,
            Aid::Warning,
        ))
        .with_children(|w| {
            w.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: vh(-WARN_VH / 2.0),
                    top: vh(0.0),
                    width: vh(WARN_VH),
                    height: vh(WARN_VH),
                    ..default()
                },
                image(&warning),
                Aid::WarningTriangle,
            ));
            w.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: vh(WARN_VH / 2.0 + 0.5),
                    top: vh(WARN_VH * 0.25),
                    width: vh(WARN_VH * 0.5),
                    height: vh(WARN_VH * 0.5),
                    ..default()
                },
                image(&arrow).with_color(Color::srgb(0.98, 0.73, 0.14)),
                UiTransform::default(),
                Aid::WarningArrow,
            ));
            w.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: vh(-20.0),
                    top: vh(WARN_VH + 0.6),
                    width: vh(40.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                children![(
                    Text::new(""),
                    TextFont {
                        font_size: 28.0.into(),
                        ..default()
                    },
                    TextColor(Color::srgb(0.98, 0.73, 0.14)),
                    TextShadow {
                        offset: Vec2::splat(1.0),
                        color: Color::srgba(0.0, 0.0, 0.0, 0.8),
                    },
                    Aid::WarningWord,
                )],
            ));
        });
    // The proximity arrow on its ring around the own rider.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(RING_TOP_PCT),
            width: vh(ARROW_VH),
            height: vh(ARROW_VH),
            ..default()
        },
        image(&arrow),
        UiTransform::default(),
        Visibility::Hidden,
        Aid::Proximity,
    ));
}

#[allow(clippy::type_complexity)]
fn update(
    view: Res<RideView>,
    mut nodes: Query<(
        &Aid,
        &mut Node,
        &mut Visibility,
        Option<&mut UiTransform>,
        Option<&mut ImageNode>,
        Option<&mut Text>,
    )>,
    access: Option<Res<crate::access::Accessibility>>,
) {
    let warning = view.hit_warning.filter(|_| view.flight.is_none());
    let arrow = view.proximity_arrow.filter(|_| view.flight.is_none());
    // Reduced motion (spec §3.2.1 M9): the warning stays, without flashing.
    let alpha = if access.is_some_and(|a| a.reduced_motion) {
        1.0
    } else {
        flash_alpha(view.elapsed_s)
    };
    for (aid, mut node, mut vis, transform, image, text) in &mut nodes {
        match aid {
            Aid::Warning => {
                vis.set_if_neq(if warning.is_some() {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            Aid::WarningTriangle => {
                if let Some(mut img) = image {
                    let a = if warning.is_some() { alpha } else { 1.0 };
                    if img.color.alpha() != a {
                        img.color.set_alpha(a);
                    }
                }
            }
            Aid::WarningArrow => {
                let Some(w) = warning else { continue };
                // A slide has no direction: the arrow is hidden.
                vis.set_if_neq(if w.kind == WarningKind::Touch {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
                if let Some(mut t) = transform {
                    let r = UiTransform::from_rotation(Rot2::radians(w.angle));
                    if t.rotation != r.rotation {
                        t.rotation = r.rotation;
                    }
                }
            }
            Aid::WarningWord => {
                if let (Some(w), Some(mut t)) = (warning, text) {
                    let word = warning_word(&w);
                    if t.0 != word {
                        t.0 = word.to_owned();
                    }
                }
            }
            Aid::Proximity => {
                let Some(angle) = arrow else {
                    vis.set_if_neq(Visibility::Hidden);
                    continue;
                };
                vis.set_if_neq(Visibility::Inherited);
                let o = ring_offset(angle);
                let (left, top) = (o.x - ARROW_VH / 2.0, o.y - ARROW_VH / 2.0);
                // Relative to the ring's centre (the node's own left / top
                // are the centre in % and an offset as margin).
                let margin = UiRect {
                    left: vh(left),
                    top: vh(top),
                    ..default()
                };
                if node.margin != margin {
                    node.margin = margin;
                }
                if let Some(mut t) = transform {
                    let r = UiTransform::from_rotation(Rot2::radians(angle));
                    if t.rotation != r.rotation {
                        t.rotation = r.rotation;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    fn alpha(img: &[[u8; 4]], x: u32, y: u32) -> u8 {
        img[(y * IMAGE_PX + x) as usize][3]
    }

    #[test]
    fn the_pictures_are_drawn_in_code() {
        let w = warning_image(IMAGE_PX);
        assert_eq!(w.len(), (IMAGE_PX * IMAGE_PX) as usize);
        // The corners are clear, the triangle's middle is amber with the
        // dark exclamation mark in it, the apex at the top.
        assert_eq!(alpha(&w, 0, 0), 0);
        assert_eq!(alpha(&w, 63, 0), 0);
        let fill = w[(70 * IMAGE_PX / 100 * IMAGE_PX + 20) as usize];
        assert!(fill[3] == 255 && fill[0] > 200 && fill[2] < 90, "{fill:?}");
        let mark = w[(48 * IMAGE_PX / 100 * IMAGE_PX + 32) as usize];
        assert!(mark[3] == 255 && mark[0] < 60, "{mark:?}");
        assert!(alpha(&w, 32, 8) > 0 && alpha(&w, 32, 2) == 0);
        // The arrow points up: wide near the top half, narrow shaft below.
        let a = arrow_image(IMAGE_PX);
        let width = |y: u32| (0..IMAGE_PX).filter(|x| alpha(&a, *x, y) > 128).count();
        assert!(width(28) > 2 * width(48), "{} {}", width(28), width(48));
        assert!(width(48) > 0 && width(2) == 0);
    }

    #[test]
    fn the_ring_and_the_flash() {
        // Ahead: above the rider; to the right: right; behind: below.
        assert!(ring_offset(0.0).y < -RING_VH * 0.99);
        assert!(ring_offset(std::f32::consts::FRAC_PI_2).x > RING_VH * 0.99);
        assert!(ring_offset(std::f32::consts::PI).y > RING_VH * 0.99);
        assert!((flash_alpha(0.0) - 1.0).abs() < 1e-6);
        assert!((flash_alpha(0.125) - 0.2).abs() < 1e-5);
        assert_eq!(
            warning_word(&HitWarningView {
                kind: WarningKind::Slide,
                in_s: 0.0,
                angle: 0.0
            }),
            "Sliding - lean less"
        );
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(AidsHudPlugin);
        app.update();
        app
    }

    fn shown(app: &mut App, which: Aid) -> (Visibility, Option<Rot2>, UiRect) {
        let w = app.world_mut();
        let mut q = w.query::<(&Aid, &Visibility, Option<&UiTransform>, &Node)>();
        let (_, v, t, n) = q.iter(w).find(|(a, ..)| **a == which).unwrap();
        (*v, t.map(|t| t.rotation), n.margin)
    }

    #[test]
    fn the_hud_shows_the_warning_and_turns_the_arrow() {
        let mut app = app();
        assert_eq!(shown(&mut app, Aid::Warning).0, Visibility::Hidden);
        assert_eq!(shown(&mut app, Aid::Proximity).0, Visibility::Hidden);
        {
            let mut v = app.world_mut().resource_mut::<RideView>();
            v.hit_warning = Some(HitWarningView {
                kind: WarningKind::Touch,
                in_s: 0.8,
                angle: -1.2,
            });
            v.proximity_arrow = Some(std::f32::consts::FRAC_PI_2);
        }
        app.update();
        assert_eq!(shown(&mut app, Aid::Warning).0, Visibility::Inherited);
        let (_, rot, _) = shown(&mut app, Aid::WarningArrow);
        assert!((rot.unwrap().as_radians() + 1.2).abs() < 1e-5);
        let (vis, rot, margin) = shown(&mut app, Aid::Proximity);
        assert_eq!(vis, Visibility::Inherited);
        assert!((rot.unwrap().as_radians() - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // To the right of the rider.
        assert_eq!(margin.left, vh(RING_VH - ARROW_VH / 2.0));
        {
            let w = app.world_mut();
            let mut q = w.query::<(&Aid, &Text)>();
            let (_, t) = q.iter(w).find(|(a, _)| **a == Aid::WarningWord).unwrap();
            assert_eq!(t.0, "Careful!");
        }
        // Flying in unicorn mode: nothing.
        app.world_mut().resource_mut::<RideView>().flight = Some(Default::default());
        app.update();
        assert_eq!(shown(&mut app, Aid::Warning).0, Visibility::Hidden);
        assert_eq!(shown(&mut app, Aid::Proximity).0, Visibility::Hidden);
    }
}
