//! The studio's icon (artwork/logos/ocw-studio.svg): the pencil drawing the
//! rainbow, the game's logo's sibling; set as the window and taskbar icon
//! and installed with the desktop entry by the shared
//! [`ocw_ui::app_icon`] (spec §11.3, §14 Phase 16).
//!
//! On Windows the exe's icon resource comes from `build.rs` (`ocw-winres`,
//! `OCW_STUDIO`); on Linux `packaging/linux/ocw-studio.desktop` with the
//! hicolor icons is installed for this user.

use ocw_ui::app_icon::AppIcon;

/// The studio's logo and desktop entry.
pub static ICON: AppIcon = ocw_ui::app_icon!("ocw-studio", "ocw-studio");

/// App id (Wayland) / WM_CLASS (X11): matches the desktop entry's name.
pub const APP_ID: &str = "ocw-studio";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_studio_has_its_own_logo() {
        assert_eq!(ICON.app_id, APP_ID);
        let svg = std::str::from_utf8(ICON.svg).unwrap();
        assert!(svg.contains("OCW Studio logo"), "the studio's SVG");
        assert!(svg.contains("MIT OR Apache-2.0"), "licence comment");
        let (rgba, w, h) = ICON.rgba().expect("the studio icon decodes");
        assert_eq!((w, h, rgba.len()), (256, 256, 256 * 256 * 4));
        let game = ocw_ui::app_icon!("ocw-desktop", "ocw");
        assert_ne!(rgba, game.rgba().unwrap().0, "not the game's logo");
        assert!(ICON.desktop_entry.contains("Name=OCW Studio\n"));
        assert!(ICON.desktop_entry.contains("Icon=ocw-studio\n"));
    }
}
