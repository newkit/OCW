//! The timestamp of every picture and video (spec §3.2, §11.4): an ISO 8601
//! local time with UTC offset at the very top of the HUD's top-right column
//! (above REC, the app message and the FPS counter), always shown – also
//! with `hud off` – because it is part of every capture, not of the
//! optional HUD; and the metadata of the screenshots the app writes
//! ([`save_shot`]: JPEG with EXIF, or PNG with text chunks).
//!
//! The time comes from [`MediaClockSetting`]: the wall clock, or during
//! frame capture (`--capture`) the capture start plus the stepped game
//! time, so it runs smoothly in the video ([`MediaClock`]).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;
pub use ocw_core::timestamp::MediaClock;

use crate::hud::VersionLabel;
use crate::media::{self, MediaTime};

pub struct StampPlugin;

impl Plugin for StampPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MediaClockSetting>()
            .init_resource::<MediaNow>()
            .add_systems(First, tick.after(bevy::time::TimeSystems))
            .add_systems(Update, update_stamp);
    }
}

/// Which clock the stamp follows (the app sets [`MediaClock::Stepped`] for
/// frame capture).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct MediaClockSetting(pub MediaClock);

impl Default for MediaClockSetting {
    fn default() -> Self {
        Self(MediaClock::Wall)
    }
}

impl MediaClockSetting {
    /// Frame capture starting now (game time 0 = the wall clock now).
    pub fn stepped_from_now() -> Self {
        Self(MediaClock::Stepped {
            start_unix_s: wall_unix_s(),
            start_game_s: 0.0,
        })
    }
}

/// The time of this frame, to the second (HUD stamp and file metadata).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaNow(pub MediaTime);

/// Wall clock in Unix seconds.
pub fn wall_unix_s() -> f64 {
    // std's clock panics on wasm32-unknown-unknown; web-time asks the
    // browser (`Date.now()`) there (spec §14 Phase 10).
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn tick(clock: Res<MediaClockSetting>, real: Res<Time<Real>>, mut now: ResMut<MediaNow>) {
    let t = clock
        .0
        .unix_s(wall_unix_s(), real.elapsed_secs_f64())
        .floor() as i64;
    if t != now.0.unix_s || now.0 == MediaTime::default() {
        // The offset (daylight saving time) is looked up once a second.
        now.0 = MediaTime {
            unix_s: t,
            offset_s: media::local_offset_s(t),
        };
    }
}

/// Marks the timestamp text.
#[derive(Component)]
pub(crate) struct StampText;

/// The stamp's node, the first child of the top-right column; above every
/// other UI (menus included), never hidden.
pub(crate) fn stamp_bundle() -> impl Bundle {
    (
        Node {
            padding: UiRect::axes(px(8), px(3)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        Text::new(""),
        TextFont {
            font_size: 14.0.into(),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        GlobalZIndex(i32::MAX - 1),
        StampText,
    )
}

fn update_stamp(now: Res<MediaNow>, mut text: Query<&mut Text, With<StampText>>) {
    let label = now.0.iso_local();
    for mut t in &mut text {
        if t.0 != label {
            t.0.clone_from(&label);
        }
    }
}

/// What a screenshot file says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotMeta {
    pub time: MediaTime,
    /// `Open Cycling World <build id>`.
    pub software: String,
}

impl ShotMeta {
    pub fn new(now: &MediaNow, label: Option<&VersionLabel>) -> Self {
        Self {
            time: now.0,
            software: media::software(label.map_or("", |l| l.0.as_str())),
        }
    }

    /// The metadata for a screenshot taken in this frame of `world`.
    pub fn of(world: &World) -> Self {
        Self::new(
            &world
                .get_resource::<MediaNow>()
                .copied()
                .unwrap_or_default(),
            world.get_resource::<VersionLabel>(),
        )
    }
}

/// Whether `path` is written as PNG (`.png`); everything else is JPEG.
pub fn is_png(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// The file bytes of an RGB8 screenshot for `path`: PNG with text chunks
/// for `.png`, else JPEG (quality [`media::JPEG_QUALITY`]) with EXIF.
pub fn encode_shot(
    rgb: &[u8],
    w: u32,
    h: u32,
    path: &Path,
    meta: &ShotMeta,
) -> Result<Vec<u8>, String> {
    if is_png(path) {
        media::encode_png(rgb, w, h, &media::png_entries(&meta.time, &meta.software))
    } else {
        media::encode_jpeg(
            rgb,
            w,
            h,
            media::JPEG_QUALITY,
            media::exif_tiff(&meta.time, &meta.software),
        )
    }
}

/// Writes a captured screenshot to `path` with its metadata.
pub fn write_shot(image: &Image, path: &Path, meta: &ShotMeta) -> Result<(), String> {
    let rgb = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| format!("cannot convert the screenshot: {e}"))?
        .to_rgb8();
    let bytes = encode_shot(rgb.as_raw(), rgb.width(), rgb.height(), path, meta)?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The observer that saves a screenshot (Bevy's `save_to_disk`, with
/// metadata and JPEG by default).
pub fn save_shot(path: impl Into<PathBuf>, meta: ShotMeta) -> impl FnMut(On<ScreenshotCaptured>) {
    let path = path.into();
    move |ev: On<ScreenshotCaptured>| match write_shot(&ev.image, &path, &meta) {
        Ok(()) => info!("Screenshot saved to {}", path.display()),
        Err(e) => error!("Cannot save screenshot: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: ShotMeta = ShotMeta {
        time: MediaTime {
            unix_s: 1_794_518_045,
            offset_s: 3600,
        },
        software: String::new(),
    };

    #[test]
    fn format_by_extension() {
        let rgb = vec![200u8; 8 * 4 * 3];
        let png = encode_shot(&rgb, 8, 4, Path::new("a/b.PNG"), &META).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        for p in ["a.jpg", "a.jpeg", "screenshot"] {
            let jpg = encode_shot(&rgb, 8, 4, Path::new(p), &META).unwrap();
            assert!(jpg.starts_with(b"\xff\xd8"), "{p}");
        }
    }

    /// The stamp is in the HUD column, follows the clock, and stays with
    /// `hud off` (it is not a HUD panel).
    #[test]
    fn stamp_follows_the_stepped_clock() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StampPlugin)
            .insert_resource(MediaClockSetting(MediaClock::Stepped {
                start_unix_s: 1_794_518_045.0,
                start_game_s: 0.0,
            }))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(400),
            ))
            .add_systems(Startup, |mut c: Commands| {
                c.spawn(stamp_bundle());
            });
        let text = |app: &mut App| {
            let w = app.world_mut();
            w.query_filtered::<&Text, With<StampText>>()
                .single(w)
                .unwrap()
                .0
                .clone()
        };
        app.update();
        let first = app.world().resource::<MediaNow>().0;
        assert_eq!(first.unix_s, 1_794_518_045);
        assert_eq!(text(&mut app), first.iso_local());
        // 0.4 s per frame: the stamp is a second later after frames 2–3.
        for _ in 0..3 {
            app.update();
        }
        let now = app.world().resource::<MediaNow>().0;
        assert_eq!(now.unix_s, 1_794_518_046);
        assert_eq!(text(&mut app), now.iso_local());
        assert!(
            text(&mut app).starts_with("2026-11-1"),
            "{}",
            text(&mut app)
        );
    }
}
