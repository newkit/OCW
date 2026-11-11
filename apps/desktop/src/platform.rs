//! Reports which window system the app actually runs on (spec §11.1).
//!
//! On Linux, winit uses Wayland when `WAYLAND_DISPLAY` is set and X11
//! otherwise. The automated runs check this log line to prove which path ran.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, RawHandleWrapper};
use raw_window_handle::RawDisplayHandle;

pub struct PlatformPlugin;

impl Plugin for PlatformPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, log_window_system);
    }
}

/// Short name of the window system behind a display handle.
pub fn window_system(handle: &RawDisplayHandle) -> &'static str {
    match handle {
        RawDisplayHandle::Wayland(_) => "wayland",
        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => "x11",
        RawDisplayHandle::Windows(_) => "windows",
        RawDisplayHandle::AppKit(_) => "appkit",
        RawDisplayHandle::Web(_) => "web",
        _ => "other",
    }
}

fn log_window_system(
    windows: Query<&RawHandleWrapper, (With<PrimaryWindow>, Added<RawHandleWrapper>)>,
) {
    for handle in &windows {
        info!(
            window_system = window_system(&handle.get_display_handle()),
            "window created"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use raw_window_handle::{
        AppKitDisplayHandle, WaylandDisplayHandle, WindowsDisplayHandle, XcbDisplayHandle,
        XlibDisplayHandle,
    };

    use super::*;

    #[test]
    fn names_window_systems() {
        let wl = WaylandDisplayHandle::new(NonNull::<u8>::dangling().cast());
        assert_eq!(window_system(&RawDisplayHandle::Wayland(wl)), "wayland");
        assert_eq!(
            window_system(&RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0))),
            "x11"
        );
        assert_eq!(
            window_system(&RawDisplayHandle::Xcb(XcbDisplayHandle::new(None, 0))),
            "x11"
        );
        assert_eq!(
            window_system(&RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            "windows"
        );
        assert_eq!(
            window_system(&RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
            "appkit"
        );
    }
}
