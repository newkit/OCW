//! The app icon (artwork/logos): the unicorn on a bike over a rainbow.
//!
//! - X11 and Windows take the window icon from the app: set here.
//! - Wayland compositors (and docks, menus, file managers) take it from the
//!   desktop entry whose name matches the window's app id, [`APP_ID`]
//!   (`packaging/linux/ocw-desktop.desktop`, installed with the icons by
//!   `scripts/deploy-test.sh`).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;

/// App id (Wayland) / WM_CLASS (X11): matches the desktop entry's name.
pub const APP_ID: &str = "ocw-desktop";

const ICON_PNG: &[u8] = include_bytes!("../../../artwork/logos/ocw-256.png");

// Desktop integration (desktop entry, icon theme files) is Linux-only;
// its tests run on every OS.

/// The icon in the sizes of the freedesktop icon theme.
#[cfg(any(target_os = "linux", test))]
const ICON_SIZES: [(u32, &[u8]); 7] = [
    (16, include_bytes!("../../../artwork/logos/ocw-16.png")),
    (32, include_bytes!("../../../artwork/logos/ocw-32.png")),
    (48, include_bytes!("../../../artwork/logos/ocw-48.png")),
    (64, include_bytes!("../../../artwork/logos/ocw-64.png")),
    (128, include_bytes!("../../../artwork/logos/ocw-128.png")),
    (256, ICON_PNG),
    (512, include_bytes!("../../../artwork/logos/ocw-512.png")),
];
#[cfg(any(target_os = "linux", test))]
const ICON_SVG: &[u8] = include_bytes!("../../../artwork/logos/ocw.svg");
#[cfg(any(target_os = "linux", test))]
const DESKTOP_ENTRY: &str = include_str!("../../../packaging/linux/ocw-desktop.desktop");

/// Files of the desktop integration (paths relative to the XDG data dir,
/// e.g. `~/.local/share`): the desktop entry, launching `exe`, and the
/// icons. Menus, docks, file managers and Wayland compositors use them.
#[cfg(any(target_os = "linux", test))]
pub fn desktop_integration_files(exe: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    use std::path::PathBuf;
    let exec = format!("Exec=\"{}\"", exe.display());
    let entry = DESKTOP_ENTRY.replace("Exec=ocw-desktop", &exec);
    let mut files = vec![(
        PathBuf::from(format!("applications/{APP_ID}.desktop")),
        entry.into_bytes(),
    )];
    for (size, png) in ICON_SIZES {
        files.push((
            PathBuf::from(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png")),
            png.to_vec(),
        ));
    }
    files.push((
        PathBuf::from(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")),
        ICON_SVG.to_vec(),
    ));
    files
}

/// Installs the desktop integration for this user (Linux), rewriting only
/// files that changed. Returns how many files were written.
#[cfg(target_os = "linux")]
pub fn install_desktop_integration() -> std::io::Result<usize> {
    let (Some(data), Ok(exe)) = (dirs::data_dir(), std::env::current_exe()) else {
        return Ok(0);
    };
    write_changed(&data, &desktop_integration_files(&exe))
}

#[cfg(any(target_os = "linux", test))]
fn write_changed(
    root: &std::path::Path,
    files: &[(std::path::PathBuf, Vec<u8>)],
) -> std::io::Result<usize> {
    let mut written = 0;
    for (rel, bytes) in files {
        let path = root.join(rel);
        if std::fs::read(&path).is_ok_and(|old| old == *bytes) {
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, bytes)?;
        written += 1;
    }
    Ok(written)
}

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, set_window_icon);
    }
}

/// The icon as RGBA8 pixels: (pixels, width, height).
pub fn icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let image = Image::from_buffer(
        ICON_PNG,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .ok()?;
    let image = if image.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb {
        image
    } else {
        image.convert(TextureFormat::Rgba8UnormSrgb)?
    };
    let (w, h) = (image.width(), image.height());
    Some((image.data?, w, h))
}

/// Sets the window icon once the primary window exists (winit calls must
/// run on the main thread: `NonSendMarker`).
fn set_window_icon(
    window: Query<Entity, With<PrimaryWindow>>,
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(entity) = window.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        let Some(w) = windows.get_window(entity) else {
            return;
        };
        *done = true;
        match icon_rgba().and_then(|(rgba, width, height)| {
            winit::window::Icon::from_rgba(rgba, width, height).ok()
        }) {
            Some(icon) => w.set_window_icon(Some(icon)),
            None => warn!("could not decode the app icon"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes_to_rgba() {
        let (rgba, w, h) = icon_rgba().expect("icon decodes");
        assert_eq!((w, h), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
        // Transparent corners (rounded tile), opaque middle.
        assert_eq!(rgba[3], 0, "top-left corner is transparent");
        let mid = ((128 * 256) + 128) * 4;
        assert_eq!(rgba[mid + 3], 255, "the middle is opaque");
        assert!(winit::window::Icon::from_rgba(rgba, w, h).is_ok());
    }

    #[test]
    fn desktop_integration_is_written_once() {
        let root = std::env::temp_dir().join(format!("ocw-icon-test-{}", std::process::id()));
        let files = desktop_integration_files(std::path::Path::new("/opt/ocw/ocw-desktop"));
        assert_eq!(write_changed(&root, &files).unwrap(), files.len());
        // Unchanged: nothing rewritten.
        assert_eq!(write_changed(&root, &files).unwrap(), 0);
        let entry = std::fs::read_to_string(root.join("applications/ocw-desktop.desktop")).unwrap();
        assert!(entry.contains("Exec=\"/opt/ocw/ocw-desktop\""), "{entry}");
        assert!(
            root.join("icons/hicolor/256x256/apps/ocw-desktop.png")
                .exists()
        );
        assert!(
            root.join("icons/hicolor/scalable/apps/ocw-desktop.svg")
                .exists()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_desktop_entry_matches_the_app_id() {
        let entry = include_str!("../../../packaging/linux/ocw-desktop.desktop");
        assert!(entry.contains(&format!("Icon={APP_ID}")));
        assert!(entry.contains(&format!("StartupWMClass={APP_ID}")));
    }
}
