//! The HUD keeps clear of a phone's notch and home bar (spec §3.2.3
//! *Safe areas*).
//!
//! The browser page uses `viewport-fit=cover`, so the canvas – and the 3D
//! view – fill the whole screen behind the notch; the page passes the
//! insets (`env(safe-area-inset-*)`, CSS px = logical px) into
//! [`SafeArea`]. While any inset is set, the UI is drawn by a camera of
//! its own ([`SafeUiCamera`], no clear, after the world's camera) whose
//! viewport is the safe part of the window: every HUD box, the menu and
//! the touch hints are laid out inside it, as on a window of that size.
//! Without insets (the desktop, phones without a notch, every test) there
//! is no such camera and the UI is drawn by the world's camera as before.
//!
//! Code that places UI by the window's size uses [`ui_area`] (the UI's
//! top left in the window and its size), and code that places UI at a
//! point of the 3D view (name tags) subtracts the area's top left.

use bevy::camera::Viewport;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::FollowCamera;

/// The safe-area insets of the primary window (logical px; zero: none).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct SafeArea {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// The UI camera of the safe area (only while insets are set).
#[derive(Component)]
pub struct SafeUiCamera;

/// Drawn after the world's camera (0) and the workshop's (1).
pub const SAFE_UI_CAMERA_ORDER: isize = 10;

impl SafeArea {
    /// No inset on any side.
    pub fn is_zero(&self) -> bool {
        self.top <= 0.0 && self.right <= 0.0 && self.bottom <= 0.0 && self.left <= 0.0
    }

    /// The insets kept clear of in a window of `size` (logical px): these,
    /// or none when they would leave less than a quarter of the window on
    /// either axis (a broken value: the HUD keeps the whole window rather
    /// than vanish).
    pub fn usable(&self, size: Vec2) -> Self {
        let w = size.x - self.left - self.right;
        let h = size.y - self.top - self.bottom;
        if self.is_zero() || w < size.x / 4.0 || h < size.y / 4.0 {
            Self::default()
        } else {
            *self
        }
    }

    /// The UI's area in a window of `size` (logical px): its top left and
    /// its size.
    pub fn inner(&self, size: Vec2) -> (Vec2, Vec2) {
        let u = self.usable(size);
        (
            Vec2::new(u.left, u.top),
            Vec2::new(size.x - u.left - u.right, size.y - u.top - u.bottom),
        )
    }

    /// The safe camera's viewport in a window of `physical` px at `scale`
    /// (physical per logical px): position and size, `None` without
    /// (usable) insets.
    pub fn viewport(&self, physical: UVec2, scale: f32) -> Option<(UVec2, UVec2)> {
        let s = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        if physical.x == 0 || physical.y == 0 {
            return None;
        }
        let u = self.usable(physical.as_vec2() / s);
        if u.is_zero() {
            return None;
        }
        let px = |v: f32| (v * s).round().max(0.0) as u32;
        let (l, r, t, b) = (px(u.left), px(u.right), px(u.top), px(u.bottom));
        let w = physical.x.checked_sub(l + r)?.max(1);
        let h = physical.y.checked_sub(t + b)?.max(1);
        Some((UVec2::new(l, t), UVec2::new(w, h)))
    }
}

/// The UI's area in `window` (logical px): its top left and its size –
/// the whole window without insets.
pub fn ui_area(window: &Window, safe: Option<&SafeArea>) -> (Vec2, Vec2) {
    let size = Vec2::new(window.width(), window.height());
    safe.map_or((Vec2::ZERO, size), |s| s.inner(size))
}

/// Where the UI's top left is on the window (logical px): what a point of
/// the 3D view (`Camera::world_to_viewport` of the world's camera) loses
/// to become a UI position.
pub fn ui_offset(window: Option<&Window>, safe: Option<&SafeArea>) -> Vec2 {
    window.map_or(Vec2::ZERO, |w| ui_area(w, safe).0)
}

pub struct SafeAreaPlugin;

impl Plugin for SafeAreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeArea>()
            .add_systems(PostUpdate, follow_safe_area);
    }
}

/// Spawns, moves and removes the safe area's UI camera.
#[allow(clippy::type_complexity)] // a Bevy system query
fn follow_safe_area(
    mut commands: Commands,
    safe: Res<SafeArea>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cams: Query<(Entity, &mut Camera, &mut Msaa), With<SafeUiCamera>>,
    world_cam: Query<&Msaa, (With<FollowCamera>, Without<SafeUiCamera>)>,
) {
    let view = windows.single().ok().and_then(|w| {
        safe.viewport(
            UVec2::new(w.physical_width(), w.physical_height()),
            w.scale_factor(),
        )
    });
    // The same MSAA as the world's camera: both draw into one target.
    let msaa = world_cam.iter().next().copied().unwrap_or_default();
    match (view, cams.single_mut()) {
        (Some((pos, size)), Ok((_, mut cam, mut m))) => {
            let same = cam
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position == pos && v.physical_size == size);
            if !same {
                cam.viewport = Some(Viewport {
                    physical_position: pos,
                    physical_size: size,
                    ..default()
                });
            }
            if *m != msaa {
                *m = msaa;
            }
        }
        (Some((pos, size)), Err(_)) => {
            info!(
                "safe area: the HUD keeps clear of the insets ({} {} {} {} px)",
                safe.top, safe.right, safe.bottom, safe.left
            );
            commands.spawn((
                Camera2d,
                Camera {
                    order: SAFE_UI_CAMERA_ORDER,
                    clear_color: ClearColorConfig::None,
                    viewport: Some(Viewport {
                        physical_position: pos,
                        physical_size: size,
                        ..default()
                    }),
                    ..default()
                },
                Tonemapping::None,
                msaa,
                SafeUiCamera,
                Name::new("safe area UI camera"),
            ));
        }
        (None, Ok((e, ..))) => {
            info!("safe area: none, the HUD uses the whole window");
            commands.entity(e).despawn();
        }
        (None, Err(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hud_s_viewport_keeps_clear_of_the_insets() {
        // 852 × 393 CSS px at pixel ratio 3 (iPhone 15, landscape).
        let i = SafeArea {
            top: 0.0,
            right: 59.0,
            bottom: 21.0,
            left: 59.0,
        };
        assert_eq!(
            i.viewport(UVec2::new(2556, 1179), 3.0),
            Some((UVec2::new(177, 0), UVec2::new(2556 - 354, 1179 - 63)))
        );
        assert_eq!(
            i.inner(Vec2::new(852.0, 393.0)),
            (Vec2::new(59.0, 0.0), Vec2::new(734.0, 372.0))
        );
        // Portrait: the notch on top, the home bar below.
        let p = SafeArea {
            top: 59.0,
            bottom: 34.0,
            ..default()
        };
        assert_eq!(
            p.viewport(UVec2::new(1179, 2556), 3.0),
            Some((UVec2::new(0, 177), UVec2::new(1179, 2556 - 279)))
        );
        // No insets: the whole window (no camera of its own).
        let none = SafeArea::default();
        assert_eq!(none.viewport(UVec2::new(1280, 720), 1.0), None);
        assert_eq!(
            none.inner(Vec2::new(1280.0, 720.0)),
            (Vec2::ZERO, Vec2::new(1280.0, 720.0))
        );
        // Insets that leave too little: ignored, the HUD keeps the window.
        let huge = SafeArea {
            left: 300.0,
            right: 300.0,
            ..default()
        };
        assert_eq!(huge.viewport(UVec2::new(700, 400), 1.0), None);
        assert_eq!(
            huge.inner(Vec2::new(700.0, 400.0)),
            (Vec2::ZERO, Vec2::new(700.0, 400.0))
        );
        assert_eq!(p.viewport(UVec2::ZERO, 1.0), None);
        // A broken scale counts as 1.
        assert_eq!(
            p.viewport(UVec2::new(393, 852), f32::NAN),
            Some((UVec2::new(0, 59), UVec2::new(393, 852 - 93)))
        );
    }

    fn app(safe: SafeArea) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(SafeAreaPlugin);
        app.world_mut().spawn((
            Window {
                resolution: bevy::window::WindowResolution::new(852, 393),
                ..default()
            },
            PrimaryWindow,
        ));
        app.insert_resource(safe);
        app
    }

    fn cams(app: &mut App) -> Vec<Option<Viewport>> {
        app.world_mut()
            .query_filtered::<&Camera, With<SafeUiCamera>>()
            .iter(app.world())
            .map(|c| c.viewport.clone())
            .collect()
    }

    #[test]
    fn a_ui_camera_only_while_insets_are_set() {
        let mut app = app(SafeArea::default());
        app.update();
        assert!(cams(&mut app).is_empty(), "no insets: the world's camera");
        *app.world_mut().resource_mut::<SafeArea>() = SafeArea {
            left: 59.0,
            right: 59.0,
            bottom: 21.0,
            top: 0.0,
        };
        app.update();
        app.update();
        let c = cams(&mut app);
        assert_eq!(c.len(), 1);
        let v = c[0].clone().expect("a viewport");
        assert_eq!(v.physical_position, UVec2::new(59, 0));
        assert_eq!(v.physical_size, UVec2::new(734, 372));
        // Turned: the insets change, the viewport follows.
        *app.world_mut().resource_mut::<SafeArea>() = SafeArea {
            left: 0.0,
            right: 0.0,
            top: 47.0,
            bottom: 21.0,
        };
        app.update();
        let c = cams(&mut app);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].clone().unwrap().physical_position, UVec2::new(0, 47));
        // Gone again: the camera goes.
        *app.world_mut().resource_mut::<SafeArea>() = SafeArea::default();
        app.update();
        app.update();
        assert!(cams(&mut app).is_empty());
    }
}
