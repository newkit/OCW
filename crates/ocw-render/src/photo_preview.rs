//! The photo preview in the HUD (spec §3.2 *HUD*, §2.11): when a photo is
//! saved – a screenshot (the `screenshot` command, its key and menu row,
//! the browser's download) or a ride photographer's photo (a person or the
//! drone) – a small framed thumbnail of it shows for [`SHOW_S`] seconds,
//! like a camera's review: a white polaroid-like frame with a drop shadow,
//! the caption under the picture ("Photo saved" / "Photographer" / "Drone"
//! and where it went), sliding and fading in and out over [`FADE_S`].
//!
//! - **Where:** the last item of the HUD's top-right column (under the
//!   time stamp, REC, the app message, the Strava badge and the FPS
//!   counter), so it moves with them, scales with the HUD size (`UiScale`)
//!   and the bike computer's panel keeps clear of it (`fit_top_panel`
//!   measures the column). Its picture shrinks ([`thumb_size`]) so the card
//!   ends above the lower [`LOWER_FREE`] of the window: the lower right
//!   (the navigation map, the workout bar), the elevation profile, the
//!   voice HUD and the junction arrow stay free.
//! - **Several photos in a row:** the newest wins ([`PreviewQueue`]): it
//!   replaces the one shown (without fading in again), never a pile.
//! - **The picture** is the captured image already in memory, downscaled
//!   to a small texture ([`thumbnail`]); nothing is read from disk.
//! - **Never in a photo or a video:** the preview is made only after the
//!   capture (the screenshot's observer), and it is hidden in every frame
//!   a photo is being captured ([`PhotoShot`]: the frame of the next
//!   screenshot has no preview of the last one). The photographer's photos
//!   are rendered off-screen without any UI. While a video is recorded
//!   (`RideView::recording_s`: the desktop's V, the browser's
//!   MediaRecorder) no preview is shown at all – the video takes the HUD
//!   as seen, so a preview would be burned in. Test captures
//!   (`--screenshot`, `--capture`) show the HUD as it is, the preview
//!   included.
//! - **Switches:** `hud.photo_preview on|off` (setting
//!   `display.photo_preview`, default on); hidden with `hud off` (a HUD
//!   panel). The desktop's VR (`vr::PcVrPlugin`) keeps `enabled` false:
//!   the preview belongs on the VR bike computer later (spec 14 Phase 11
//!   X3).

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Capturing, Screenshot, ScreenshotCaptured};

use crate::RideView;

/// How long a preview shows, fades included (s).
pub const SHOW_S: f64 = 3.0;
/// Its fade (and slide) in and out (s).
pub const FADE_S: f64 = 0.2;
/// How far it slides in from the right (logical px).
pub const SLIDE_PX: f32 = 28.0;
/// The picture's width in the card (logical px), at most and at least.
pub const MAX_THUMB_W: f32 = 192.0;
pub const MIN_THUMB_W: f32 = 96.0;
/// The white frame around the picture (logical px).
pub const BORDER_PX: f32 = 5.0;
/// The gap above the card in the column (logical px; the column's gap).
pub const GAP_PX: f32 = 4.0;
/// The caption's height: the title (14 px) and up to two lines of where
/// the photo went (12 px), with its gap (logical px).
pub const CAPTION_PX: f32 = 4.0 + 14.0 * 1.2 + 2.0 * 12.0 * 1.2;
/// The lower part of the window the card never reaches (fraction of the
/// height): the navigation map's and the workout bar's lower right, the
/// elevation profile, the voice HUD, the junction arrow.
pub const LOWER_FREE: f32 = 0.4;
/// The thumbnail texture's width (px): twice the card's, sharp up to a
/// HUD size of 200 %.
pub const TEXTURE_W: u32 = 384;
/// The picture's height to width, within these (a very wide or tall
/// window is squeezed a little).
pub const MIN_ASPECT: f32 = 9.0 / 21.0;
pub const MAX_ASPECT: f32 = 3.0 / 4.0;

/// The caption: what took the photo and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Caption {
    /// "Photo saved", "Photographer", "Drone".
    pub title: String,
    /// "Saved to Pictures/OCW", "Saved to Pictures/OCW, added to your
    /// ride", "Downloaded OCW-….jpg".
    pub detail: String,
}

impl Caption {
    pub fn new(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
        }
    }

    /// A screenshot.
    pub fn screenshot(detail: impl Into<String>) -> Self {
        Self::new("Photo saved", detail)
    }

    /// A ride photographer's photo (a person's or the drone's).
    pub fn ride_photo(drone: bool, detail: impl Into<String>) -> Self {
        Self::new(if drone { "Drone" } else { "Photographer" }, detail)
    }
}

/// A small RGBA8 (sRGB) picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Thumb {
    /// Height over width.
    pub fn aspect(&self) -> f32 {
        if self.w == 0 {
            9.0 / 16.0
        } else {
            self.h as f32 / self.w as f32
        }
    }
}

/// Downscales 4-byte pixels (`bgra`: blue first) of a `w` × `h` picture to
/// `tw` × `th` RGBA, averaging 3 × 3 samples per pixel (fast: a 1440p
/// window in well under a millisecond's worth of samples).
pub fn downscale(src: &[u8], w: u32, h: u32, bgra: bool, tw: u32, th: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    if w == 0 || h == 0 || src.len() < (w * h * 4) as usize {
        out.resize((tw * th * 4) as usize, 0);
        return out;
    }
    let (sx, sy) = (w as f32 / tw as f32, h as f32 / th as f32);
    for y in 0..th {
        for x in 0..tw {
            let mut acc = [0u32; 3];
            for j in 0..3 {
                let py = (((y as f32 + (j as f32 + 0.5) / 3.0) * sy) as u32).min(h - 1);
                for i in 0..3 {
                    let px = (((x as f32 + (i as f32 + 0.5) / 3.0) * sx) as u32).min(w - 1);
                    let o = ((py * w + px) * 4) as usize;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += u32::from(src[o + c]);
                    }
                }
            }
            let [a, b, c] = acc.map(|v| (v / 9) as u8);
            if bgra {
                out.extend_from_slice(&[c, b, a, 255]);
            } else {
                out.extend_from_slice(&[a, b, c, 255]);
            }
        }
    }
    out
}

/// The thumbnail of a captured image (a window's screenshot or a rendered
/// photo), [`TEXTURE_W`] wide at most.
pub fn thumbnail(image: &Image) -> Option<Thumb> {
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        return None;
    }
    let tw = TEXTURE_W.min(w);
    let th = ((h as f32 * tw as f32 / w as f32).round() as u32).max(1);
    let bgra = match image.texture_descriptor.format {
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => Some(true),
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some(false),
        _ => None,
    };
    let data = image.data.as_deref();
    let rgba = match (bgra, data) {
        (Some(bgra), Some(d)) if d.len() == (w * h * 4) as usize => {
            downscale(d, w, h, bgra, tw, th)
        }
        // Another format: through the image crate (a copy, rare).
        _ => {
            let rgba = image.clone().try_into_dynamic().ok()?.to_rgba8();
            downscale(rgba.as_raw(), w, h, false, tw, th)
        }
    };
    Some(Thumb { w: tw, h: th, rgba })
}

/// A photo to preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub thumb: Thumb,
    pub caption: Caption,
}

/// The preview shown now.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub caption: Caption,
    pub aspect: f32,
    /// When it started (s, the real clock).
    pub start_s: f64,
}

/// How the card looks in a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Opacity 0 … 1.
    pub alpha: f32,
    /// Offset to the right (logical px).
    pub slide_px: f32,
}

/// The opacity `age` seconds after a preview started: in over [`FADE_S`],
/// full, out over the last [`FADE_S`] of [`SHOW_S`]; `None` when it is
/// over.
pub fn look_at(age: f64) -> Option<Look> {
    if !(0.0..SHOW_S).contains(&age) {
        return (age < 0.0).then_some(Look {
            alpha: 0.0,
            slide_px: SLIDE_PX,
        });
    }
    let t = if age < FADE_S {
        age / FADE_S
    } else if age > SHOW_S - FADE_S {
        (SHOW_S - age) / FADE_S
    } else {
        1.0
    };
    // Smoothstep: no jerk at either end.
    let a = (t * t * (3.0 - 2.0 * t)) as f32;
    Some(Look {
        alpha: a,
        slide_px: (1.0 - a) * SLIDE_PX,
    })
}

/// The newest photo wins: one preview at a time, a new one replaces it.
#[derive(Debug, Clone, Default)]
pub struct PreviewQueue {
    incoming: Option<Incoming>,
    shown: Option<Shown>,
}

impl PreviewQueue {
    /// A new photo (replaces one not shown yet).
    pub fn push(&mut self, photo: Incoming) {
        self.incoming = Some(photo);
    }

    /// Nothing shown, nothing waiting.
    pub fn clear(&mut self) {
        self.incoming = None;
        self.shown = None;
    }

    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    /// The frame at `now_s`: the new picture (when one came) and the look,
    /// `None` when nothing shows. A photo that comes while another shows
    /// replaces it at full opacity (it does not fade in again; one still
    /// fading in goes on) and shows its full time.
    pub fn tick(&mut self, now_s: f64) -> (Option<Thumb>, Option<Look>) {
        let mut thumb = None;
        if let Some(i) = self.incoming.take() {
            let age = match &self.shown {
                Some(s) if now_s - s.start_s < FADE_S => (now_s - s.start_s).max(0.0),
                Some(_) => FADE_S,
                None => 0.0,
            };
            self.shown = Some(Shown {
                caption: i.caption,
                aspect: i.thumb.aspect(),
                start_s: now_s - age,
            });
            thumb = Some(i.thumb);
        }
        let look = self.shown.as_ref().and_then(|s| look_at(now_s - s.start_s));
        if look.is_none() {
            self.shown = None;
        }
        (thumb, look)
    }
}

/// The preview's state and switch (`hud.photo_preview`).
#[derive(Resource, Debug, Clone)]
pub struct PhotoPreview {
    pub enabled: bool,
    pub queue: PreviewQueue,
}

impl Default for PhotoPreview {
    fn default() -> Self {
        Self {
            enabled: true,
            queue: PreviewQueue::default(),
        }
    }
}

impl PhotoPreview {
    /// Previews `image` (just captured) with `caption`, when switched on.
    pub fn show(&mut self, image: &Image, caption: Caption) {
        if !self.enabled {
            return;
        }
        if let Some(thumb) = thumbnail(image) {
            self.queue.push(Incoming { thumb, caption });
        }
    }

    /// Previews a thumbnail made before.
    pub fn show_thumb(&mut self, thumb: Thumb, caption: Caption) {
        if self.enabled {
            self.queue.push(Incoming { thumb, caption });
        }
    }
}

/// Marks a screenshot that is a photo (the `screenshot` command): the
/// frame it captures has no preview.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PhotoShot;

/// The observer that previews a screenshot once it is captured.
pub fn preview_shot(
    caption: Caption,
) -> impl FnMut(On<ScreenshotCaptured>, Option<ResMut<PhotoPreview>>) {
    move |ev: On<ScreenshotCaptured>, preview: Option<ResMut<PhotoPreview>>| {
        if let Some(mut p) = preview {
            p.show(&ev.image, caption.clone());
        }
    }
}

/// The picture's size in the card (logical px) for a window `window_h`
/// high whose top-right column ends at `column_bottom` above the card
/// (both logical px): as wide as [`MAX_THUMB_W`], narrower when the card
/// would reach the lower [`LOWER_FREE`] of the window, never under
/// [`MIN_THUMB_W`].
pub fn thumb_size(window_h: f32, column_bottom: f32, aspect: f32) -> Vec2 {
    let aspect = aspect.clamp(MIN_ASPECT, MAX_ASPECT);
    let room =
        window_h * (1.0 - LOWER_FREE) - column_bottom - GAP_PX - 2.0 * BORDER_PX - CAPTION_PX;
    let w = (room / aspect).clamp(MIN_THUMB_W, MAX_THUMB_W).floor();
    Vec2::new(w, (w * aspect).round())
}

/// The card's height for a picture `thumb_h` high (logical px).
pub fn card_height(thumb_h: f32) -> f32 {
    thumb_h + 2.0 * BORDER_PX + CAPTION_PX
}

pub struct PhotoPreviewPlugin;

impl Plugin for PhotoPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhotoPreview>()
            .add_systems(PostUpdate, update_card.before(bevy::ui::UiSystems::Layout));
    }
}

/// The card.
#[derive(Component)]
pub(crate) struct PreviewCard;

/// The picture in it.
#[derive(Component)]
pub(crate) struct PreviewImage;

/// The caption's title and detail.
#[derive(Component)]
pub(crate) struct PreviewTitle;

#[derive(Component)]
pub(crate) struct PreviewDetail;

const CARD_WHITE: [f32; 3] = [0.97, 0.97, 0.95];
const TITLE_GREY: [f32; 3] = [0.10, 0.10, 0.12];
const DETAIL_GREY: [f32; 3] = [0.32, 0.32, 0.36];
const SHADOW_ALPHA: f32 = 0.5;

fn rgba(c: [f32; 3], a: f32) -> Color {
    Color::srgba(c[0], c[1], c[2], a)
}

/// The card, the last child of the HUD's top-right column (hidden until a
/// photo comes).
pub(crate) fn spawn_card(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Relative,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(BORDER_PX)),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            BackgroundColor(rgba(CARD_WHITE, 0.0)),
            BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.0), px(0), px(3), px(0), px(8)),
            crate::hud::HudPanel,
            PreviewCard,
        ))
        .with_children(|card| {
            card.spawn((
                Node {
                    width: px(MAX_THUMB_W),
                    height: px(MAX_THUMB_W * 9.0 / 16.0),
                    ..default()
                },
                ImageNode::default(),
                PreviewImage,
            ));
            card.spawn((
                Node {
                    width: px(MAX_THUMB_W),
                    margin: UiRect::top(px(4)),
                    ..default()
                },
                Text::new(""),
                TextFont {
                    font_size: 14.0.into(),
                    ..default()
                },
                TextColor(rgba(TITLE_GREY, 0.0)),
                PreviewTitle,
            ));
            card.spawn((
                Node {
                    width: px(MAX_THUMB_W),
                    ..default()
                },
                Text::new(""),
                TextFont {
                    font_size: 12.0.into(),
                    ..default()
                },
                TextColor(rgba(DETAIL_GREY, 0.0)),
                PreviewDetail,
            ));
        });
}

/// A screenshot of the window that is a photo and not captured yet (this
/// frame is its picture).
type PendingPhoto = (With<PhotoShot>, Without<Capturing>);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn update_card(
    real: Res<Time<Real>>,
    mut preview: ResMut<PhotoPreview>,
    view: Option<Res<RideView>>,
    shots: Query<&Screenshot, PendingPhoto>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    column: Query<&ComputedNode, With<crate::hud::HudTopRight>>,
    images: Option<ResMut<Assets<Image>>>,
    mut card: Query<
        (
            &mut Node,
            &mut BackgroundColor,
            &mut BoxShadow,
            Option<&ComputedNode>,
        ),
        With<PreviewCard>,
    >,
    mut picture: Query<(&mut Node, &mut ImageNode), (With<PreviewImage>, Without<PreviewCard>)>,
    mut texts: Query<
        (&mut Text, &mut TextColor, &mut Node, Has<PreviewTitle>),
        (
            Or<(With<PreviewTitle>, With<PreviewDetail>)>,
            Without<PreviewCard>,
            Without<PreviewImage>,
        ),
    >,
) {
    // A video takes the HUD as seen: no preview while one is recorded.
    let recording = view.as_ref().is_some_and(|v| v.recording_s.is_some());
    if recording || !preview.enabled {
        preview.queue.clear();
    }
    let (thumb, look) = preview.queue.tick(real.elapsed_secs_f64());
    // The frame a photo is taken in has no preview of the last one.
    let photo_now = shots.iter().any(|s| !matches!(s.0, RenderTarget::Image(_)));
    let Ok((mut node, mut bg, mut shadow, computed)) = card.single_mut() else {
        return;
    };
    let look = look.filter(|_| !photo_now);
    let display = if look.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    let Some(look) = look else {
        return;
    };
    let shown = preview.queue.shown().cloned().unwrap_or_else(|| Shown {
        caption: Caption::default(),
        aspect: 9.0 / 16.0,
        start_s: 0.0,
    });
    // The picture's size: what the column leaves above the lower part.
    let size = match (windows.single(), column.single()) {
        (Ok(w), Ok(col)) => {
            let inv = col.inverse_scale_factor();
            let own = computed.map_or(0.0, |c| c.size().y) * inv;
            let above = 12.0 + (col.size().y * inv - own).max(0.0);
            thumb_size(
                // The UI's height (inside the safe area, spec §3.2.3).
                crate::safe_area::ui_area(w, safe.as_deref()).1.y * w.scale_factor() * inv,
                above,
                shown.aspect,
            )
        }
        _ => thumb_size(f32::MAX, 0.0, shown.aspect),
    };
    let a = look.alpha;
    node.left = px(look.slide_px);
    bg.0 = rgba(CARD_WHITE, a);
    if let Some(s) = shadow.0.first_mut() {
        s.color = Color::srgba(0.0, 0.0, 0.0, SHADOW_ALPHA * a);
    }
    if let Ok((mut pn, mut img)) = picture.single_mut() {
        if pn.width != px(size.x) || pn.height != px(size.y) {
            pn.width = px(size.x);
            pn.height = px(size.y);
        }
        img.color = Color::srgba(1.0, 1.0, 1.0, a);
        if let (Some(t), Some(mut images)) = (thumb, images) {
            let image = Image::new(
                Extent3d {
                    width: t.w,
                    height: t.h,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                t.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            );
            // The last picture's handle is dropped: its texture goes.
            img.image = images.add(image);
        }
    }
    for (mut text, mut color, mut tn, title) in &mut texts {
        let want = if title {
            &shown.caption.title
        } else {
            &shown.caption.detail
        };
        if &text.0 != want {
            text.0.clone_from(want);
        }
        if tn.width != px(size.x) {
            tn.width = px(size.x);
        }
        color.0 = rgba(if title { TITLE_GREY } else { DETAIL_GREY }, a);
    }
}

/// The name of a folder for the caption: below the Pictures folder
/// `pictures` as "Pictures/OCW" (that folder's own name: "Bilder/OCW" on
/// a German system), below `home` as "~/…", else the path; longer than
/// [`MAX_LABEL`] characters only its last two folders ("…/shots/OCW").
pub fn folder_label(
    folder: &std::path::Path,
    pictures: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> String {
    let join = |base: &str, rest: &std::path::Path| {
        let rest: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if rest.is_empty() {
            base.to_string()
        } else {
            format!("{base}/{}", rest.join("/"))
        }
    };
    let full = if let Some(p) = pictures
        && let Ok(rest) = folder.strip_prefix(p)
    {
        let name = p
            .file_name()
            .map_or("Pictures".into(), |n| n.to_string_lossy().into_owned());
        join(&name, rest)
    } else if let Some(h) = home
        && let Ok(rest) = folder.strip_prefix(h)
    {
        join("~", rest)
    } else {
        folder.display().to_string()
    };
    if full.chars().count() <= MAX_LABEL {
        return full;
    }
    let names: Vec<String> = folder
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let tail = names[names.len().saturating_sub(2)..].join("/");
    format!("…/{tail}")
}

/// The longest folder name in a caption (characters).
pub const MAX_LABEL: usize = 28;

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(title: &str, w: u32, h: u32) -> Incoming {
        Incoming {
            thumb: Thumb {
                w,
                h,
                rgba: vec![0; (w * h * 4) as usize],
            },
            caption: Caption::new(title, "Saved to Pictures/OCW"),
        }
    }

    /// 3 s in all: 0.2 s in (sliding from the right), full, 0.2 s out.
    #[test]
    fn a_preview_lives_three_seconds_and_fades() {
        let mut q = PreviewQueue::default();
        assert_eq!(q.tick(10.0), (None, None));
        q.push(incoming("Photo saved", 16, 9));
        let (thumb, look) = q.tick(10.0);
        assert!(thumb.is_some());
        let look = look.unwrap();
        assert_eq!((look.alpha, look.slide_px), (0.0, SLIDE_PX));
        let a = |q: &mut PreviewQueue, t: f64| q.tick(t).1.map(|l| l.alpha);
        let mid = a(&mut q, 10.1).unwrap();
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        assert_eq!(a(&mut q, 10.2), Some(1.0));
        assert_eq!(a(&mut q, 11.5), Some(1.0));
        assert_eq!(a(&mut q, 12.8), Some(1.0));
        let out = a(&mut q, 12.9).unwrap();
        assert!(out > 0.3 && out < 0.7, "{out}");
        assert!(a(&mut q, 12.99).unwrap() < 0.05);
        assert_eq!(a(&mut q, 13.0), None);
        assert!(q.shown().is_none(), "gone after 3 s");
        // The slide follows the fade.
        let l = look_at(0.05).unwrap();
        assert!((l.slide_px - (1.0 - l.alpha) * SLIDE_PX).abs() < 1e-4);
        assert!(look_at(-0.1).unwrap().alpha == 0.0);
        assert_eq!(look_at(SHOW_S), None);
    }

    /// Photos in a row: the newest shows, at once at full opacity, for
    /// its full 3 s; never more than one.
    #[test]
    fn the_newest_photo_wins() {
        let mut q = PreviewQueue::default();
        q.push(incoming("Photographer", 16, 9));
        q.tick(0.0);
        assert_eq!(q.tick(1.0).1.unwrap().alpha, 1.0);
        // Two before the next frame: only the newer.
        q.push(incoming("Photo saved", 16, 9));
        q.push(incoming("Drone", 4, 3));
        let (thumb, look) = q.tick(1.5);
        assert_eq!(thumb.unwrap().w, 4);
        assert_eq!(look.unwrap().alpha, 1.0, "replaces without a new fade-in");
        assert_eq!(q.shown().unwrap().caption.title, "Drone");
        assert!((q.shown().unwrap().aspect - 0.75).abs() < 1e-6);
        // Its own 3 s from when it came (less the skipped fade-in).
        assert!(q.tick(4.2).1.is_some());
        assert_eq!(q.tick(1.5 + SHOW_S - FADE_S + 0.01).1, None);
        // One still fading in goes on fading in.
        q.push(incoming("A", 16, 9));
        q.tick(10.0);
        q.push(incoming("B", 16, 9));
        let l = q.tick(10.1).1.unwrap();
        assert!(l.alpha < 1.0 && l.alpha > 0.0);
        assert_eq!(q.shown().unwrap().caption.title, "B");
        q.clear();
        assert_eq!(q.tick(10.2), (None, None));
    }

    /// The thumbnail averages the picture and swaps BGRA to RGBA.
    #[test]
    fn thumbnails_are_downscaled_from_memory() {
        // 6 × 3: left half red, right half blue (BGRA).
        let mut src = Vec::new();
        for _y in 0..3 {
            for x in 0..6 {
                src.extend_from_slice(if x < 3 {
                    &[0, 0, 255, 255]
                } else {
                    &[255, 0, 0, 255]
                });
            }
        }
        let t = downscale(&src, 6, 3, true, 2, 1);
        assert_eq!(t, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let t = downscale(&src, 6, 3, false, 2, 1);
        assert_eq!(&t[..4], &[0, 0, 255, 255]);
        // A window-sized image: TEXTURE_W wide, the aspect kept.
        let image = Image::new(
            Extent3d {
                width: 1280,
                height: 720,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![128; 1280 * 720 * 4],
            TextureFormat::Bgra8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        );
        let t = thumbnail(&image).unwrap();
        assert_eq!((t.w, t.h), (TEXTURE_W, 216));
        assert_eq!(&t.rgba[..4], &[128, 128, 128, 255]);
        // Smaller than the texture: its own size.
        let small = Image::new(
            Extent3d {
                width: 64,
                height: 48,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![10; 64 * 48 * 4],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        );
        assert_eq!(thumbnail(&small).map(|t| (t.w, t.h)), Some((64, 48)));
    }

    #[test]
    fn captions_and_folder_labels() {
        assert_eq!(Caption::screenshot("x").title, "Photo saved");
        assert_eq!(Caption::ride_photo(true, "x").title, "Drone");
        assert_eq!(Caption::ride_photo(false, "x").title, "Photographer");
        let home = std::path::Path::new("/home/ada");
        let pics = home.join("Pictures");
        assert_eq!(
            folder_label(&pics.join("OCW"), Some(&pics), Some(home)),
            "Pictures/OCW"
        );
        let bilder = home.join("Bilder");
        assert_eq!(
            folder_label(&bilder.join("OCW"), Some(&bilder), Some(home)),
            "Bilder/OCW"
        );
        assert_eq!(
            folder_label(&home.join("rides/shots"), Some(&pics), Some(home)),
            "~/rides/shots"
        );
        assert_eq!(
            folder_label(std::path::Path::new("/srv/ocw"), Some(&pics), Some(home)),
            "/srv/ocw"
        );
        // A long path: its last two folders.
        assert_eq!(
            folder_label(
                &home.join("src/ocw-checkout/test-output/photo-preview/photos"),
                Some(&pics),
                Some(home)
            ),
            "…/photo-preview/photos"
        );
    }

    /// The HUD fit (spec §3.2): at every supported window size and HUD
    /// size the card ends above the lower part of the window – clear of
    /// the junction arrow, the elevation profile, the voice HUD and the
    /// lower right (navigation map, workout bar) – also under a full
    /// top-right column; it is never wider than the time stamp, so the
    /// bike computer's panel (fitted to the column) keeps its width.
    #[test]
    fn the_card_fits_every_window() {
        let sizes = [
            (1280.0f32, 720.0f32),
            (1366.0, 768.0),
            (1600.0, 900.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3840.0, 2160.0),
            (1024.0, 768.0),
            (1280.0, 960.0),
            (1600.0, 1200.0),
        ];
        // The column's items (logical px): the stamp, the FPS counter;
        // with the app message, the Strava badge and its notice. (No REC:
        // no preview while a video is recorded.)
        let stamp = 14.0 * 1.2 + 6.0;
        let fps = 18.0 * 1.2 + 8.0;
        let line = 16.0 * 1.2 + 8.0;
        let columns = [
            12.0 + stamp,
            12.0 + stamp + GAP_PX + fps,
            12.0 + stamp + 4.0 * (GAP_PX + line) + GAP_PX + fps,
        ];
        let stamp_w = 25.0 * 14.0 * 0.6 + 16.0;
        for (w, h) in sizes {
            for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
                let (lw, lh) = (w / scale, h / scale);
                if lh < 480.0 {
                    continue; // over 150 % on a 720p / 768p window: the whole HUD is cramped
                }
                let fan = crate::junction::fan_geometry(Vec2::new(w, h), scale);
                for aspect in [9.0 / 16.0, 3.0 / 4.0, h / w] {
                    for above in columns {
                        let what = format!("{w}×{h} at {scale}, column {above}, aspect {aspect}");
                        let t = thumb_size(lh, above, aspect);
                        assert!(t.x >= MIN_THUMB_W && t.x <= MAX_THUMB_W, "{what}");
                        let card_w = t.x + 2.0 * BORDER_PX;
                        assert!(card_w <= stamp_w, "{what}: {card_w} wider than the stamp");
                        let bottom = above + GAP_PX + card_height(t.y);
                        // Above the lower part – unless the picture is at its
                        // smallest already (a full column on a small window
                        // with a large HUD: then only clear of the rest).
                        assert!(
                            bottom <= lh * (1.0 - LOWER_FREE) + 0.5 || t.x == MIN_THUMB_W,
                            "{what}: ends at {bottom} of {lh}"
                        );
                        if t.x == MIN_THUMB_W {
                            assert!(bottom <= lh * 0.75, "{what}: ends at {bottom} of {lh}");
                        }
                        let card_left = lw - 12.0 - card_w;
                        // Clear of the junction arrow (physical px).
                        assert!(
                            bottom * scale < fan.min.y || card_left * scale > fan.max.x,
                            "{what}: over the arrow"
                        );
                        // The elevation profile and the voice HUD are at the
                        // bottom left, under the card's lowest point.
                        let profile_top = lh - 12.0 - 56.0 - 30.0 - 30.0;
                        assert!(
                            bottom < profile_top || card_left > 12.0 + 316.0,
                            "{what}: over the profile"
                        );
                    }
                }
                // The usual case: the full-size picture.
                assert_eq!(
                    thumb_size(lh, columns[1], 9.0 / 16.0).x,
                    MAX_THUMB_W,
                    "{w}×{h} at {scale}"
                );
            }
        }
    }

    fn card_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<PhotoPreview>()
            .add_systems(Startup, |mut c: Commands| {
                c.spawn((Node::default(), crate::hud::HudTopRight))
                    .with_children(spawn_card);
            })
            .add_systems(Update, update_card);
        app.update();
        app
    }

    fn card_display(app: &mut App) -> Display {
        let w = app.world_mut();
        w.query_filtered::<&Node, With<PreviewCard>>()
            .single(w)
            .unwrap()
            .display
    }

    fn red_shot() -> Image {
        Image::new(
            Extent3d {
                width: 32,
                height: 18,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            [0u8, 0, 255, 255].repeat(32 * 18),
            TextureFormat::Bgra8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        )
    }

    /// The preview comes after the capture, never in a photo: the frame
    /// of the next photo has none, a video recording shows none, and the
    /// switch turns it off.
    #[test]
    fn the_preview_is_never_in_a_photo() {
        let mut app = card_app();
        assert_eq!(card_display(&mut app), Display::None);
        // A photo being taken: nothing to preview yet (it comes with the
        // capture).
        let shot = app
            .world_mut()
            .spawn((Screenshot::primary_window(), PhotoShot))
            .observe(preview_shot(Caption::screenshot("Saved to Pictures/OCW")))
            .id();
        app.update();
        assert_eq!(card_display(&mut app), Display::None);
        // Captured: the preview shows, the picture from the image.
        app.world_mut().entity_mut(shot).insert(Capturing);
        app.world_mut().trigger(ScreenshotCaptured {
            entity: shot,
            image: red_shot(),
        });
        app.world_mut().despawn(shot);
        app.update();
        assert_eq!(card_display(&mut app), Display::Flex);
        {
            let w = app.world_mut();
            let img = w
                .query_filtered::<&ImageNode, With<PreviewImage>>()
                .single(w)
                .unwrap()
                .image
                .clone();
            let images = w.resource::<Assets<Image>>();
            let data = images.get(&img).unwrap().data.clone().unwrap();
            assert_eq!(&data[..4], &[255, 0, 0, 255], "red, as RGBA");
            let title = w
                .query_filtered::<&Text, With<PreviewTitle>>()
                .single(w)
                .unwrap()
                .0
                .clone();
            assert_eq!(title, "Photo saved");
        }
        // The next photo's frame: hidden while it is taken …
        let next = app
            .world_mut()
            .spawn((Screenshot::primary_window(), PhotoShot))
            .id();
        app.update();
        assert_eq!(card_display(&mut app), Display::None);
        // … back once that frame is captured.
        app.world_mut().entity_mut(next).insert(Capturing);
        app.update();
        assert_eq!(card_display(&mut app), Display::Flex);
        // An off-screen photo (the photographer's) has no UI: no hiding.
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(red_shot());
        app.world_mut().spawn((Screenshot::image(image), PhotoShot));
        app.update();
        assert_eq!(card_display(&mut app), Display::Flex);
        // A video recording: no preview.
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.recording_s = Some(1.0);
        app.insert_resource(view);
        app.update();
        assert_eq!(card_display(&mut app), Display::None);
        app.world_mut()
            .resource_mut::<PhotoPreview>()
            .show(&red_shot(), Caption::screenshot("x"));
        app.update();
        assert_eq!(
            card_display(&mut app),
            Display::None,
            "dropped while recording"
        );
        app.world_mut().resource_mut::<RideView>().recording_s = None;
        // Switched off: nothing.
        app.world_mut().resource_mut::<PhotoPreview>().enabled = false;
        app.world_mut()
            .resource_mut::<PhotoPreview>()
            .show(&red_shot(), Caption::screenshot("x"));
        app.update();
        assert_eq!(card_display(&mut app), Display::None);
        app.world_mut().resource_mut::<PhotoPreview>().enabled = true;
        app.world_mut()
            .resource_mut::<PhotoPreview>()
            .show(&red_shot(), Caption::ride_photo(true, "added to your ride"));
        app.update();
        assert_eq!(card_display(&mut app), Display::Flex);
    }
}
