//! The app icon (artwork/logos/ocw.svg): the unicorn on a bike over a
//! rainbow, set as the window and taskbar icon and installed with the
//! desktop entry by the shared [`ocw_ui::app_icon`] (spec §11.3).
//!
//! On Windows the exe's icon resource comes from `build.rs` (`ocw-winres`);
//! on Linux `packaging/linux/ocw-desktop.desktop` with the hicolor icons is
//! installed for this user (and by `scripts/build-appimage.sh`).

use bevy::prelude::*;
use ocw_ui::app_icon::{AppIcon, AppIconPlugin};

/// The game's logo and desktop entry.
pub static ICON: AppIcon = ocw_ui::app_icon!("ocw-desktop", "ocw");

/// App id (Wayland) / WM_CLASS (X11): matches the desktop entry's name.
pub const APP_ID: &str = "ocw-desktop";

/// Installs the desktop entry and icons for this user (Linux).
#[cfg(target_os = "linux")]
pub fn install_desktop_integration() -> std::io::Result<usize> {
    ICON.install_desktop_integration()
}

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AppIconPlugin(&ICON));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_has_the_unicorn_logo() {
        assert_eq!(ICON.app_id, APP_ID);
        assert!(std::str::from_utf8(ICON.svg).unwrap().contains("unicorn"));
        let (rgba, w, h) = ICON.rgba().expect("icon decodes");
        assert_eq!((w, h, rgba.len()), (256, 256, 256 * 256 * 4));
        assert!(ICON.desktop_entry.contains("Icon=ocw-desktop\n"));
    }
}
