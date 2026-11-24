//! VR in the renderer, shared by the headset backends (spec §14 Phase 11
//! for the desktop's OpenXR on branch `agent/vr-x1`, Phase 14 14I W2 for
//! WebXR in the browser app): the bike computer on the bars
//! ([`computer`]) and the controllers as a gamepad ([`controllers`]).
//!
//! The backend (the browser's `webxr` module, later the desktop's OpenXR
//! layer) writes [`VrSession`] every frame – whether the headset shows
//! the ride, the controllers' state, the lines for the bike computer's
//! status row – and gives its eye cameras [`eye_layers`], so they see the
//! bike computer, which no other camera draws.
//!
//! Ported from `agent/vr-x1` (`ocw-render::vr::{computer, controllers}`),
//! with that branch's `VrState` / `VrConfig` replaced by [`VrSession`] and
//! its layer 7 (the workshop's on main) by [`VR_LAYER`].

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

pub mod computer;
pub mod controllers;

/// The render layer only the eye cameras see (the bike computer). Not 7
/// (the workshop's), 8 (its mirror), 9 (thumbnails) or 11 (the studio's).
pub const VR_LAYER: usize = 12;

/// What the eye cameras draw: the world and the VR-only things.
pub fn eye_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, VR_LAYER])
}

/// The headset session as the renderer needs it (written by the backend).
#[derive(Resource, Debug, Clone, Default)]
pub struct VrSession {
    /// The headset shows the ride now (the bike computer's screen is drawn
    /// only then; the controls hint starts each time it becomes true).
    pub shown: bool,
    /// The controls in one line, on the screen for [`computer::HINT_S`]
    /// when the headset starts showing the ride (empty: no hint).
    pub controls_hint: String,
    /// The bike computer's status row: the sensor bridge's state …
    pub link: String,
    /// … and the frame time (14I W3: the in-headset readout).
    pub frame: String,
    /// This frame's controllers (`None`: none, or not focused).
    pub controllers: Option<ocw_xr::controllers::ControllerState>,
    /// Set by the controllers' recentre gesture; the backend takes it.
    pub recentre_now: bool,
    /// The left stick's sideways flicks are the junction exits while the
    /// menu is closed (WebXR, 14I W2; `PadMapper::exits_on_left_stick`).
    pub exits_on_left_stick: bool,
}

/// The bike computer and the controllers' gamepad.
pub struct VrPlugin;

impl Plugin for VrPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VrSession>();
        computer::plugin(app);
        controllers::plugin(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eyes_see_the_world_and_the_vr_layer() {
        let eyes = eye_layers();
        assert!(eyes.intersects(&RenderLayers::layer(0)));
        assert!(eyes.intersects(&RenderLayers::layer(VR_LAYER)));
        // The other cameras (the default layer only) never see it.
        assert!(!RenderLayers::default().intersects(&RenderLayers::layer(VR_LAYER)));
        // Not a layer main already uses.
        for used in [
            crate::workshop::LAYER,
            crate::workshop::MIRROR_LAYER,
            crate::thumbs::THUMB_LAYER,
        ] {
            assert_ne!(used, VR_LAYER);
        }
    }
}
