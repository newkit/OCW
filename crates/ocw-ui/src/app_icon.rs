//! An app's logo (`artwork/logos/`) as its icon on every desktop platform
//! (spec §11.3), shared by the game (`ocw-desktop`, the unicorn on a bike)
//! and the editor (`ocw-studio`, the pencil drawing the rainbow):
//!
//! - X11 and Windows take the window icon from the app: set here. On
//!   Windows the taskbar / Alt+Tab icon too (from the exe's icon resource,
//!   which the app's `build.rs` links in with `ocw-winres`; Explorer and
//!   shortcuts show that resource).
//! - Wayland compositors (and docks, menus, file managers) take it from the
//!   desktop entry whose name matches the window's app id,
//!   [`AppIcon::app_id`] (`packaging/linux/<app id>.desktop`), which the
//!   app installs with the hicolor icons ([`AppIcon::install_desktop_integration`]).
//! - macOS has no window icons: the Dock shows the app bundle's `.icns`
//!   (`scripts/build-macos-app.sh`). An unbundled binary (`cargo run`)
//!   shows the default one – setting the application's icon image needs
//!   an `unsafe` AppKit call, which the workspace forbids.
//!
//! Native only (the browser app has its favicon).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;

/// The icon group's resource id in the Windows exe (`ocw_winres::ICON_ID`).
#[cfg(target_os = "windows")]
const WINDOWS_ICON_RESOURCE: u16 = 1;

/// The PNG sizes of a logo in `artwork/logos/` (the freedesktop icon
/// theme's sizes).
pub const ICON_SIZES: [u32; 7] = [16, 32, 48, 64, 128, 256, 512];

/// One app's logo and desktop entry, embedded in its binary. Made with
/// [`app_icon!`](crate::app_icon!).
#[derive(Debug)]
pub struct AppIcon {
    /// App id (Wayland) / WM_CLASS (X11), the desktop entry's and the icon
    /// theme files' name, e.g. `ocw-desktop`.
    pub app_id: &'static str,
    /// The PNG renders by size ([`ICON_SIZES`]).
    pub pngs: [(u32, &'static [u8]); 7],
    /// The SVG source (the icon theme's scalable icon).
    pub svg: &'static [u8],
    /// `packaging/linux/<app id>.desktop`; its `Exec=<app id>` line is
    /// replaced by the running binary's path when installed.
    pub desktop_entry: &'static str,
}

/// `app_icon!("ocw-desktop", "ocw")`: the [`AppIcon`] of app id
/// `ocw-desktop` with the logo `artwork/logos/ocw.svg` and its PNG renders
/// and the desktop entry `packaging/linux/ocw-desktop.desktop` (paths from
/// the calling package, two levels below the repository root: `apps/…`).
#[macro_export]
macro_rules! app_icon {
    ($app_id:literal, $logo:literal) => {
        $crate::app_icon::AppIcon {
            app_id: $app_id,
            pngs: [
                (16, $crate::app_icon!(@png $logo, "16")),
                (32, $crate::app_icon!(@png $logo, "32")),
                (48, $crate::app_icon!(@png $logo, "48")),
                (64, $crate::app_icon!(@png $logo, "64")),
                (128, $crate::app_icon!(@png $logo, "128")),
                (256, $crate::app_icon!(@png $logo, "256")),
                (512, $crate::app_icon!(@png $logo, "512")),
            ],
            svg: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../artwork/logos/",
                $logo,
                ".svg"
            )),
            desktop_entry: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../packaging/linux/",
                $app_id,
                ".desktop"
            )),
        }
    };
    (@png $logo:literal, $size:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../artwork/logos/",
            $logo,
            "-",
            $size,
            ".png"
        ))
    };
}

impl AppIcon {
    /// The PNG render of `size` px.
    pub fn png(&self, size: u32) -> Option<&'static [u8]> {
        self.pngs.iter().find(|(s, _)| *s == size).map(|(_, p)| *p)
    }

    /// The window icon (256 px) as RGBA8 pixels: (pixels, width, height).
    pub fn rgba(&self) -> Option<(Vec<u8>, u32, u32)> {
        let image = Image::from_buffer(
            self.png(256)?,
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

    /// Files of the desktop integration (paths relative to the XDG data
    /// dir, e.g. `~/.local/share`): the desktop entry, launching `exe`, and
    /// the icons. Menus, docks, file managers and Wayland compositors use
    /// them.
    pub fn desktop_integration_files(
        &self,
        exe: &std::path::Path,
    ) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        use std::path::PathBuf;
        let id = self.app_id;
        let exec = format!("Exec=\"{}\"", exe.display());
        let entry = self.desktop_entry.replace(&format!("Exec={id}"), &exec);
        let mut files = vec![(
            PathBuf::from(format!("applications/{id}.desktop")),
            entry.into_bytes(),
        )];
        for (size, png) in self.pngs {
            files.push((
                PathBuf::from(format!("icons/hicolor/{size}x{size}/apps/{id}.png")),
                png.to_vec(),
            ));
        }
        files.push((
            PathBuf::from(format!("icons/hicolor/scalable/apps/{id}.svg")),
            self.svg.to_vec(),
        ));
        files
    }

    /// Installs the desktop integration for this user (Linux), rewriting
    /// only files that changed. Returns how many files were written.
    #[cfg(target_os = "linux")]
    pub fn install_desktop_integration(&self) -> std::io::Result<usize> {
        let (Some(data), Ok(exe)) = (dirs::data_dir(), std::env::current_exe()) else {
            return Ok(0);
        };
        write_changed(&data, &self.desktop_integration_files(&exe))
    }
}

/// Writes the files under `root`, skipping those whose content is already
/// there. Returns how many files were written.
pub fn write_changed(
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

/// Sets the app's icon on its primary window once it exists.
pub struct AppIconPlugin(pub &'static AppIcon);

#[derive(Resource)]
struct TheIcon(&'static AppIcon);

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TheIcon(self.0))
            .add_systems(Update, set_window_icon);
    }
}

/// Sets the window icon once the primary window exists (winit calls must
/// run on the main thread: `NonSendMarker`).
fn set_window_icon(
    window: Query<Entity, With<PrimaryWindow>>,
    icon: Res<TheIcon>,
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(entity) = window.single() else {
        return;
    };
    let icon = icon.0;
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        let Some(w) = windows.get_window(entity) else {
            return;
        };
        *done = true;
        let logo = || {
            icon.rgba().and_then(|(rgba, width, height)| {
                winit::window::Icon::from_rgba(rgba, width, height).ok()
            })
        };
        match logo() {
            Some(i) => w.set_window_icon(Some(i)),
            None => warn!("could not decode the app icon"),
        }
        // Windows: `set_window_icon` sets only the small (title bar) icon;
        // the taskbar and Alt+Tab use the big one. The exe's resource gives
        // it at the system's size (all sizes are in it), the logo otherwise.
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::{IconExtWindows, WindowExtWindows};
            let big = winit::window::Icon::from_resource(WINDOWS_ICON_RESOURCE, None)
                .ok()
                .or_else(logo);
            w.set_taskbar_icon(big);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static GAME: AppIcon = crate::app_icon!("ocw-desktop", "ocw");
    static STUDIO: AppIcon = crate::app_icon!("ocw-studio", "ocw-studio");

    #[test]
    fn every_size_decodes_with_a_transparent_corner() {
        for icon in [&GAME, &STUDIO] {
            for (size, png) in icon.pngs {
                let image = Image::from_buffer(
                    png,
                    ImageType::Extension("png"),
                    CompressedImageFormats::NONE,
                    true,
                    ImageSampler::Default,
                    RenderAssetUsages::default(),
                )
                .unwrap_or_else(|e| panic!("{} {size} px: {e}", icon.app_id));
                assert_eq!((image.width(), image.height()), (size, size));
            }
            let (rgba, w, h) = icon.rgba().expect("icon decodes");
            assert_eq!((w, h), (256, 256));
            assert_eq!(rgba.len(), 256 * 256 * 4);
            // Transparent corners (rounded tile), opaque middle.
            let id = icon.app_id;
            assert_eq!(rgba[3], 0, "{id}: top-left corner is transparent");
            let mid = ((128 * 256) + 128) * 4;
            assert_eq!(rgba[mid + 3], 255, "{id}: the middle is opaque");
            assert!(winit::window::Icon::from_rgba(rgba, w, h).is_ok());
        }
    }

    #[test]
    fn the_studio_has_its_own_logo() {
        assert_ne!(GAME.rgba().unwrap().0, STUDIO.rgba().unwrap().0);
        assert_ne!(GAME.svg, STUDIO.svg);
        for size in ICON_SIZES {
            assert_ne!(GAME.png(size), STUDIO.png(size), "{size} px");
        }
    }

    #[test]
    fn each_desktop_entry_matches_its_app_id() {
        for icon in [&GAME, &STUDIO] {
            let entry = icon.desktop_entry;
            let id = icon.app_id;
            assert!(entry.contains(&format!("Icon={id}\n")), "{id}");
            assert!(entry.contains(&format!("StartupWMClass={id}\n")), "{id}");
            assert!(entry.contains(&format!("Exec={id}\n")), "{id}");
        }
    }

    #[test]
    fn desktop_integration_is_written_once() {
        for icon in [&GAME, &STUDIO] {
            let id = icon.app_id;
            let root =
                std::env::temp_dir().join(format!("ocw-icon-test-{id}-{}", std::process::id()));
            let exe = std::path::PathBuf::from(format!("/opt/ocw/{id}"));
            let files = icon.desktop_integration_files(&exe);
            assert_eq!(files.len(), 1 + ICON_SIZES.len() + 1);
            assert_eq!(write_changed(&root, &files).unwrap(), files.len());
            // Unchanged: nothing rewritten.
            assert_eq!(write_changed(&root, &files).unwrap(), 0);
            let entry =
                std::fs::read_to_string(root.join(format!("applications/{id}.desktop"))).unwrap();
            let exec = format!("Exec=\"/opt/ocw/{id}\"");
            assert!(entry.contains(&exec), "{entry}");
            let png =
                std::fs::read(root.join(format!("icons/hicolor/256x256/apps/{id}.png"))).unwrap();
            assert_eq!(Some(png.as_slice()), icon.png(256));
            assert!(
                root.join(format!("icons/hicolor/scalable/apps/{id}.svg"))
                    .exists()
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }
}
