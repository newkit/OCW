//! The heart icon beside the heart rate in the HUD's big line (spec 3.2
//! *Heart rate*): drawn in code (no emoji font; plain RGBA8 images, the
//! same on WebGPU and WebGL 2), laid out inline in the line's text in the
//! place of the letters "HR" ([`split_heart`]), so it wraps with the line.
//! It beats at the measured heart rate; while the heart rate rises it
//! turns pink and pounds, with a glow ([`crate::heart`] has the rules).
//! With reduced motion only the colour changes; with `hud off` it hides
//! with the panel; without a heart rate there is no heart (and no "HR").
//!
//! The inline box keeps one size (two characters of the big font), so the
//! beat never moves the text: the beat swaps between pre-drawn images of
//! the heart at [`SCALE_STEPS`] sizes and [`GLOW_LEVELS`] glows, tinted
//! in the heart's colour (the outline stays dark).

use bevy::prelude::*;

use crate::RideView;
use crate::heart::{Heart, HeartLook, POUND_SCALE};

/// The heart's normal colour (a soft red).
pub const NORMAL: [f32; 3] = [0.93, 0.36, 0.40];
/// Its colour while the heart rate rises (hot pink, #ff4fa3).
pub const PINK: [f32; 3] = [1.0, 79.0 / 255.0, 163.0 / 255.0];

/// The big line's font size (UI px; as `hud::BIG_FONT`).
const FONT: f32 = 26.0;
/// The inline box: as wide as the two letters it replaces, a little
/// lower than the font's ascent (so the line keeps its height).
pub const BOX: Vec2 = Vec2::new(2.0 * FONT * 0.6, FONT * 0.9);
/// The heart's height at rest (UI px).
const HEART_PX: f32 = 14.0;
/// The point the heart grows from (UI px from the box's top left): a
/// little below its middle, so at rest the heart is centred on the digits
/// (its tip 2.5 px above the baseline, the box's bottom) and the beat grows
/// mostly upwards (measured in the first screenshots, v6.77).
const CENTRE: Vec2 = Vec2::new(BOX.x * 0.5, 16.0);
/// … at this height of the unit heart (35 % up from its tip).
const ANCHOR_Y: f32 = 0.386;
/// The dark outline's width and the glow's reach (UI px).
const OUTLINE_PX: f32 = 1.1;
const GLOW_PX: f32 = 2.6;
/// Texels per UI px.
const TEXELS: f32 = 2.0;
/// Pre-drawn sizes from 1 to [`POUND_SCALE`] and glows from 0 to 1.
pub const SCALE_STEPS: usize = 13;
pub const GLOW_LEVELS: usize = 5;

/// The heart's state, shared with the VR bike computer (it tints the heart
/// rate pink while it rises).
#[derive(Resource, Debug, Default)]
pub struct HeartState {
    pub heart: Heart,
    /// The look now (`None`: no heart rate).
    pub look: Option<HeartLook>,
}

impl HeartState {
    pub fn rising(&self) -> bool {
        self.look.is_some() && self.heart.rising()
    }
}

/// Whether the big line has the heart's place now (set by the HUD).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HeartSlot(pub bool);

/// The heart's images: `[glow][scale]`.
#[derive(Resource)]
pub(crate) struct HeartIcons(Vec<Vec<Handle<Image>>>);

/// The inline heart in the big line.
#[derive(Component)]
pub(crate) struct HeartIcon;

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<HeartState>()
        .init_resource::<HeartSlot>()
        .add_systems(Startup, make_icons);
}

/// The big line split at the heart rate's "HR" (followed by a space and a
/// number): the text before and after the two letters.
pub fn split_heart(line: &str) -> Option<(&str, &str)> {
    let mut from = 0;
    while let Some(i) = line[from..].find("HR ") {
        let at = from + i;
        let rest = &line[at + 2..];
        if rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
            return Some((&line[..at], rest));
        }
        from = at + 3;
    }
    None
}

/// The heart's colour: `pink` of the way from the normal colour to pink.
pub fn heart_color(pink: f64) -> Color {
    let k = pink.clamp(0.0, 1.0) as f32;
    let c: [f32; 3] = std::array::from_fn(|i| NORMAL[i] + (PINK[i] - NORMAL[i]) * k);
    Color::srgb(c[0], c[1], c[2])
}

/// The image index of a look: (glow, scale).
pub fn icon_index(look: &HeartLook) -> (usize, usize) {
    let s = ((look.scale - 1.0) / (POUND_SCALE - 1.0)).clamp(0.0, 1.0);
    let g = look.glow.clamp(0.0, 1.0);
    (
        (g * (GLOW_LEVELS - 1) as f64).round() as usize,
        (s * (SCALE_STEPS - 1) as f64).round() as usize,
    )
}

/// Signed distance to the unit heart (tip at the origin, y up, about
/// 1.2 wide and 1.1 high; Inigo Quilez's heart).
fn heart_distance(p: Vec2) -> f32 {
    let p = Vec2::new(p.x.abs(), p.y);
    if p.y + p.x > 1.0 {
        (p - Vec2::new(0.25, 0.75)).length() - std::f32::consts::SQRT_2 / 4.0
    } else {
        let a = (p - Vec2::new(0.0, 1.0)).length_squared();
        let b = (p - 0.5 * (p.x + p.y).max(0.0) * Vec2::ONE).length_squared();
        a.min(b).sqrt() * (p.x - p.y).signum()
    }
}

/// The heart at `scale` with `glow` (0–1): straight-alpha RGBA8 texels of
/// the [`BOX`], row-major from the top – white inside (tinted by the
/// colour), a dark outline, a white glow fading outwards.
pub fn heart_texels(scale: f32, glow: f32) -> (UVec2, Vec<u8>) {
    let dim = (BOX * TEXELS).round().as_uvec2();
    let unit = HEART_PX * scale / 1.1;
    let mut out = Vec::with_capacity((dim.x * dim.y * 4) as usize);
    for y in 0..dim.y {
        for x in 0..dim.x {
            let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / TEXELS;
            let p = Vec2::new(q.x - CENTRE.x, CENTRE.y - q.y) / unit + Vec2::new(0.0, ANCHOR_Y);
            let d = heart_distance(p) * unit;
            let cover = |edge: f32| (0.5 - (d - edge) * TEXELS).clamp(0.0, 1.0);
            let fill = cover(0.0);
            let line = cover(OUTLINE_PX) * 0.85;
            let halo = glow * 0.8 * (1.0 - (d - OUTLINE_PX) / GLOW_PX).clamp(0.0, 1.0);
            // Fill over outline over glow.
            let (mut c, mut a) = (1.0f32, halo);
            for (lc, la) in [(0.0, line), (1.0, fill)] {
                let na = la + a * (1.0 - la);
                if na > 0.0 {
                    c = (lc * la + c * a * (1.0 - la)) / na;
                }
                a = na;
            }
            let v = (c * 255.0).round() as u8;
            out.extend_from_slice(&[v, v, v, (a * 255.0).round() as u8]);
        }
    }
    (dim, out)
}

fn make_icons(mut commands: Commands, images: Option<ResMut<Assets<Image>>>) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let Some(mut images) = images else { return };
    let icons = (0..GLOW_LEVELS)
        .map(|g| {
            (0..SCALE_STEPS)
                .map(|s| {
                    let scale =
                        1.0 + (POUND_SCALE as f32 - 1.0) * s as f32 / (SCALE_STEPS - 1) as f32;
                    let (dim, data) = heart_texels(scale, g as f32 / (GLOW_LEVELS - 1) as f32);
                    images.add(Image::new(
                        Extent3d {
                            width: dim.x,
                            height: dim.y,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        data,
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                    ))
                })
                .collect()
        })
        .collect();
    commands.insert_resource(HeartIcons(icons));
}

/// The inline heart's image and size before the first look (an empty box
/// of no size: no heart).
pub(crate) fn inline_heart() -> (InlineImage, HeartIcon) {
    (
        InlineImage {
            width: Some(0.0),
            height: Some(0.0),
            ..default()
        },
        HeartIcon,
    )
}

/// Beats the heart: its state from the local rider's heart rate, its image
/// and colour (after the HUD wrote the big line).
pub(crate) fn update_heart(
    time: Res<Time>,
    view: Res<RideView>,
    slot: Res<HeartSlot>,
    access: Option<Res<crate::access::Accessibility>>,
    icons: Option<Res<HeartIcons>>,
    mut state: ResMut<HeartState>,
    mut hearts: Query<&mut InlineImage, With<HeartIcon>>,
) {
    let bpm = if view.flight.is_some() {
        None
    } else {
        view.local().and_then(|l| l.heart_rate_bpm)
    };
    let still = access.as_ref().is_some_and(|a| a.reduced_motion);
    let look = state.heart.update(time.elapsed_secs_f64(), bpm, still);
    state.look = look;
    let shown = look.filter(|_| slot.0);
    for mut img in &mut hearts {
        let (size, image, color) = match (shown, &icons) {
            (Some(l), Some(icons)) => {
                let (g, s) = icon_index(&l);
                let c = heart_color(l.pink);
                let c = access
                    .as_deref()
                    .map_or(c, |a| crate::access::transform(c, a, false));
                (BOX, icons.0[g][s].clone(), c)
            }
            _ => (
                Vec2::ZERO,
                bevy::image::TRANSPARENT_IMAGE_HANDLE,
                Color::WHITE,
            ),
        };
        let want = InlineImage {
            color,
            image,
            width: Some(size.x),
            height: Some(size.y),
            ..default()
        };
        if *img != want {
            *img = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heart_takes_the_place_of_hr() {
        let line = "200 W    20.8 km/h   80 rpm   +7.7 % (100 m +8.3 %)   HR 142   clear 18 °C";
        let (a, b) = split_heart(line).unwrap();
        assert!(a.ends_with("%)   "), "{a}");
        assert!(b.starts_with(" 142   clear"), "{b}");
        assert_eq!(format!("{a}HR{b}"), line);
        // Wrapped: still found; no heart rate: no place.
        assert!(split_heart("200 W\nHR 95").is_some());
        assert_eq!(split_heart("200 W   20.8 km/h"), None);
        assert_eq!(split_heart("HR zone"), None);
        assert_eq!(split_heart("HR"), None);
    }

    #[test]
    fn the_heart_is_drawn_filled_outlined_and_glowing() {
        let at = |(dim, px): &(UVec2, Vec<u8>), q: Vec2| {
            let t = (q * TEXELS).as_uvec2().min(*dim - 1);
            let i = ((t.y * dim.x + t.x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        let rest = heart_texels(1.0, 0.0);
        assert_eq!(rest.0, (BOX * TEXELS).round().as_uvec2());
        // White and opaque at the centre, clear in the corners.
        assert_eq!(at(&rest, CENTRE), [255, 255, 255, 255]);
        assert_eq!(at(&rest, Vec2::ZERO)[3], 0);
        assert_eq!(at(&rest, BOX - 0.5)[3], 0);
        // The two lobes, the dip between them and the tip.
        let unit = HEART_PX / 1.1;
        let lobe = CENTRE + Vec2::new(0.25, -(0.75 - ANCHOR_Y)) * unit;
        assert_eq!(at(&rest, lobe)[3], 255);
        let dip = CENTRE + Vec2::new(0.0, -(1.06 - ANCHOR_Y)) * unit;
        assert!(
            at(&rest, dip)[0] < 60,
            "the dip between the lobes is not filled"
        );
        let above = CENTRE + Vec2::new(0.0, -(1.2 - ANCHOR_Y)) * unit;
        assert_eq!(at(&rest, above)[3], 0, "clear above the dip");
        // A dark outline just outside the side.
        let side = CENTRE + Vec2::new(0.604 * unit + 0.5, -(0.75 - ANCHOR_Y) * unit);
        let o = at(&rest, side);
        assert!(o[0] < 40 && o[3] > 150, "{o:?}");
        // The glow only when glowing.
        let out = CENTRE + Vec2::new(0.604 * unit + OUTLINE_PX + 1.0, -(0.75 - ANCHOR_Y) * unit);
        assert_eq!(at(&rest, out)[3], 0);
        let g = heart_texels(1.0, 1.0);
        assert!(at(&g, out)[3] > 40 && at(&g, out)[0] == 255);
        // Bigger when it beats.
        let big = heart_texels(POUND_SCALE as f32, 0.0);
        let cover = |t: &(UVec2, Vec<u8>)| t.1.chunks(4).filter(|p| p[3] > 128).count();
        assert!(cover(&big) as f32 > cover(&rest) as f32 * 1.5);
        // The pounding heart (with its outline) stays in its box.
        let edge = |t: &(UVec2, Vec<u8>)| {
            let w = t.0.x as usize;
            t.1.chunks(4)
                .enumerate()
                .filter(|(i, p)| p[3] > 128 && (i % w == 0 || i % w == w - 1))
                .count()
        };
        assert_eq!(edge(&big), 0);
    }

    #[test]
    fn looks_pick_their_image_and_colour() {
        let l = |scale, glow| HeartLook {
            scale,
            glow,
            pink: 0.0,
        };
        assert_eq!(icon_index(&l(1.0, 0.0)), (0, 0));
        assert_eq!(
            icon_index(&l(POUND_SCALE, 1.0)),
            (GLOW_LEVELS - 1, SCALE_STEPS - 1)
        );
        assert_eq!(icon_index(&l(1.15, 0.0)).1, (SCALE_STEPS - 1) / 2);
        let s = heart_color(1.0).to_srgba();
        assert!((s.red - 1.0).abs() < 1e-6 && (s.blue - 163.0 / 255.0).abs() < 1e-6);
        let n = heart_color(0.0).to_srgba();
        assert!((n.green - NORMAL[1]).abs() < 1e-6);
    }
}
