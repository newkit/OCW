//! The HUD's size follows the screen (spec 3.2, *Phone layout*): the HUD
//! is laid out in UI px – logical (CSS) px over Bevy's `UiScale` – and its
//! pixel sizes (fonts, the profile, the map) were chosen for desktop
//! windows of 1280 × 720 and more. On a phone (an iPhone 15: 393 × 852 CSS
//! px, 852 × 393 turned) the same sizes took half the screen. So:
//!
//! - **The scale:** `UiScale` = the HUD size setting (`display.ui_scale`,
//!   [`HudSize`]) × [`screen_fit`] of the UI area (the window inside the
//!   safe area, logical px): 1 from a short side of
//!   [`FULL_SIZE_SHORT_PX`] up – every desktop and tablet window keeps its
//!   HUD –, down to [`MIN_FIT`] on a phone. It follows the CSS size, never
//!   the device pixel ratio (logical px already are CSS px).
//! - **The compact layout** ([`HudLayout::compact`], a short side under
//!   [`COMPACT_SHORT_PX`]): the bike computer's big line holds only the
//!   ride's numbers (power, speed, cadence, gradient), the weather, wind
//!   and clock move shortened into the small lines (`crate::hud`), the
//!   profile narrows, and on a narrow screen ([`HudLayout::narrow`],
//!   portrait) the top panel takes the whole width with the top-right
//!   column (timestamp, REC, FPS) below it on the right.
//!
//! The menu is sized by the window's height and stays as it is.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::safe_area::{SafeArea, ui_area};

/// From this short side (logical px of the UI area) up the HUD has its
/// full size.
pub const FULL_SIZE_SHORT_PX: f32 = 540.0;
/// The smallest screen fit (a phone).
pub const MIN_FIT: f32 = 0.75;
/// Below this short side (logical px) the HUD is compact.
pub const COMPACT_SHORT_PX: f32 = 500.0;
/// A compact HUD narrower than this (UI px) is narrow: the top panel takes
/// the whole width.
pub const NARROW_UI_PX: f32 = 700.0;

/// The HUD size setting (`display.ui_scale` / `ui.scale`: 1 = 100 %); the
/// applied `UiScale` is this × [`screen_fit`].
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct HudSize(pub f32);

impl Default for HudSize {
    fn default() -> Self {
        Self(1.0)
    }
}

/// The screen's factor on the HUD size for a UI area of `ui` logical px.
pub fn screen_fit(ui: Vec2) -> f32 {
    let short = ui.x.min(ui.y);
    if short <= 0.0 {
        return 1.0;
    }
    (short / FULL_SIZE_SHORT_PX).clamp(MIN_FIT, 1.0)
}

/// What the HUD's layout follows this frame.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct HudLayout {
    /// The factor of [`screen_fit`] in the applied `UiScale`.
    pub fit: f32,
    /// A phone-sized screen: the compact HUD.
    pub compact: bool,
    /// Compact and narrow (a phone in portrait): the top panel takes the
    /// whole width, the top-right column goes below it.
    pub narrow: bool,
    /// The UI area in UI px (logical px over `UiScale`).
    pub ui_px: Vec2,
}

impl Default for HudLayout {
    fn default() -> Self {
        Self {
            fit: 1.0,
            compact: false,
            narrow: false,
            ui_px: Vec2::new(1280.0, 720.0),
        }
    }
}

impl HudLayout {
    /// The layout for a UI area of `ui` logical px at the HUD size `size`.
    pub fn of(ui: Vec2, size: f32) -> Self {
        let fit = screen_fit(ui);
        let scale = (size * fit).max(0.1);
        let ui_px = ui / scale;
        let compact = ui.x > 0.0 && ui.y > 0.0 && ui.x.min(ui.y) < COMPACT_SHORT_PX;
        Self {
            fit,
            compact,
            narrow: compact && ui_px.x < NARROW_UI_PX,
            ui_px,
        }
    }

    /// The `UiScale` for the HUD size `size`.
    pub fn ui_scale(&self, size: f32) -> f32 {
        size * self.fit
    }
}

pub struct HudFitPlugin;

impl Plugin for HudFitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudSize>()
            .init_resource::<HudLayout>()
            .init_resource::<UiScale>()
            .add_systems(First, fit_hud)
            .add_systems(PostUpdate, fit_hud.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Sets [`HudLayout`] and `UiScale` from the window (inside the safe
/// area) and the HUD size setting.
fn fit_hud(
    windows: Query<&Window, With<PrimaryWindow>>,
    safe: Option<Res<SafeArea>>,
    size: Res<HudSize>,
    mut layout: ResMut<HudLayout>,
    mut scale: ResMut<UiScale>,
) {
    let l = windows
        .single()
        .map(|w| HudLayout::of(ui_area(w, safe.as_deref()).1, size.0))
        .unwrap_or(HudLayout {
            fit: 1.0,
            ..HudLayout::of(Vec2::new(1280.0, 720.0), size.0)
        });
    layout.set_if_neq(l);
    let want = l.ui_scale(size.0);
    if (scale.0 - want).abs() > 1e-4 {
        scale.0 = want;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// iPhone 15 (393 × 852 CSS px) inside its safe areas: portrait
    /// 59 / 0 / 34 / 0, landscape 0 / 59 / 21 / 59.
    pub(crate) const PHONE_PORTRAIT: Vec2 = Vec2::new(393.0, 852.0 - 59.0 - 34.0);
    pub(crate) const PHONE_LANDSCAPE: Vec2 = Vec2::new(852.0 - 2.0 * 59.0, 393.0 - 21.0);

    #[test]
    fn desktop_and_tablet_windows_keep_their_hud() {
        for (w, h) in [
            (1280.0, 720.0),
            (1366.0, 768.0),
            (1920.0, 1080.0),
            (3840.0, 2160.0),
            (1024.0, 768.0),
            (800.0, 600.0),
            (820.0, 1180.0), // an iPad in portrait
            (1180.0, 820.0),
            (2064.0, 2208.0), // a VR mirror window
        ] {
            let l = HudLayout::of(Vec2::new(w, h), 1.0);
            assert_eq!(l.fit, 1.0, "{w}×{h}");
            assert!(!l.compact && !l.narrow, "{w}×{h}");
            assert_eq!(l.ui_scale(1.5), 1.5, "the HUD size setting stays");
        }
    }

    #[test]
    fn a_phone_gets_the_compact_hud() {
        let p = HudLayout::of(PHONE_PORTRAIT, 1.0);
        assert!(p.compact && p.narrow, "{p:?}");
        // 393 / 540 is below the smallest fit.
        assert_eq!(p.fit, MIN_FIT);
        assert!(p.ui_px.x > 500.0 && p.ui_px.x < 560.0, "{p:?}");
        let l = HudLayout::of(PHONE_LANDSCAPE, 1.0);
        assert!(l.compact && !l.narrow, "{l:?}");
        assert_eq!(l.fit, MIN_FIT);
        assert!((l.ui_px.y - 372.0 / MIN_FIT).abs() < 0.01);
        // The setting still scales on a phone.
        assert_eq!(l.ui_scale(2.0), 2.0 * MIN_FIT);
        // A small phone (320 CSS px) stays at the smallest fit.
        assert_eq!(screen_fit(Vec2::new(320.0, 560.0)), MIN_FIT);
        // Nothing known: full size.
        assert_eq!(screen_fit(Vec2::ZERO), 1.0);
        assert!(!HudLayout::of(Vec2::ZERO, 1.0).compact);
    }

    fn app(w: f32, h: f32, safe: SafeArea) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(HudFitPlugin);
        app.insert_resource(safe);
        let mut window = Window::default();
        window.resolution.set(w, h);
        app.world_mut().spawn((window, PrimaryWindow));
        app.update();
        app
    }

    #[test]
    fn the_ui_scale_follows_the_window_and_the_setting() {
        let mut desk = app(1280.0, 720.0, SafeArea::default());
        assert_eq!(desk.world().resource::<UiScale>().0, 1.0);
        desk.world_mut().resource_mut::<HudSize>().0 = 1.5;
        desk.update();
        assert_eq!(desk.world().resource::<UiScale>().0, 1.5);
        assert!(!desk.world().resource::<HudLayout>().compact);
        let safe = SafeArea {
            top: 59.0,
            right: 0.0,
            bottom: 34.0,
            left: 0.0,
        };
        let mut phone = app(393.0, 852.0, safe);
        let s = phone.world().resource::<UiScale>().0;
        assert_eq!(s, MIN_FIT);
        // A larger phone or a small tablet (480 px) lies between.
        assert!((screen_fit(Vec2::new(480.0, 900.0)) - 480.0 / FULL_SIZE_SHORT_PX).abs() < 1e-4);
        assert!(phone.world().resource::<HudLayout>().narrow);
        // Turned: landscape.
        phone.insert_resource(SafeArea {
            top: 0.0,
            right: 59.0,
            bottom: 21.0,
            left: 59.0,
        });
        {
            let mut q = phone
                .world_mut()
                .query_filtered::<&mut Window, With<PrimaryWindow>>();
            q.single_mut(phone.world_mut())
                .unwrap()
                .resolution
                .set(852.0, 393.0);
        }
        phone.update();
        assert_eq!(phone.world().resource::<UiScale>().0, MIN_FIT);
        let l = *phone.world().resource::<HudLayout>();
        assert!(l.compact && !l.narrow, "{l:?}");
    }
}
