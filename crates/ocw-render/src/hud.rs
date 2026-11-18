//! In-ride heads-up display (Bevy UI, spec §3.2).
//!
//! Gradient feel (spec §3.2): next to the gradient at the rider the main
//! line shows the average over the next 100 m, and a mini elevation
//! profile (bottom left, above the GPS line; console `hud.profile`) shows
//! the road from 150 m behind to 1.5 km ahead, coloured by gradient band
//! ([`GradeBand`]). In free ride both look ahead on the path's
//! [`RidePath::preview`]: the exit currently selected at the next junction,
//! the default exits after it.

use std::sync::Arc;

use bevy::prelude::*;
use ocw_core::{RiderId, mps_to_kmh};
use ocw_world::{AVERAGE_AHEAD_M, ElevationProfile, GradeBand, ProfileSpec, RidePath, Track};

use crate::RideView;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::frame_rate::FrameRatePlugin>() {
            app.add_plugins(crate::frame_rate::FrameRatePlugin);
        }
        if !app.is_plugin_added::<crate::stamp::StampPlugin>() {
            app.add_plugins(crate::stamp::StampPlugin);
        }
        if !app.is_plugin_added::<crate::photo_preview::PhotoPreviewPlugin>() {
            app.add_plugins(crate::photo_preview::PhotoPreviewPlugin);
        }
        app.init_resource::<ShowFps>()
            .init_resource::<ShowHud>()
            .init_resource::<Units>()
            .init_resource::<ShowProfile>()
            .init_resource::<ShowGps>()
            .init_resource::<HudGpsLine>()
            .init_resource::<LookAhead>()
            .init_resource::<AppKeys>()
            .init_resource::<TopPanelWidth>()
            .add_systems(Startup, (spawn_hud, spawn_recording))
            .add_systems(
                Update,
                (
                    update_look_ahead,
                    (update_hud, update_profile),
                    update_gps,
                    (update_recording, update_strava),
                    show_hud,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, fit_top_panel);
        add_version_label(app);
    }
}

/// The app's own keys at the end of the HUD's key line (after the ride
/// keys): the desktop's by default; the browser app has no restart,
/// video or screen keys (spec §14 Phase 10).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct AppKeys(pub String);

/// The desktop app's keys in the HUD.
pub const DESKTOP_KEYS: &str =
    "mouse: camera, C: camera reset, R: restart, V: video | F11: screen | ` or F12: console";

impl Default for AppKeys {
    fn default() -> Self {
        Self(DESKTOP_KEYS.into())
    }
}

/// The build id shown small in the lower right corner (set by the app,
/// e.g. `0.1.0-7d538c0-dirty (debug)`; empty = not shown).
#[derive(Resource, Default, Clone, Debug, PartialEq, Eq)]
pub struct VersionLabel(pub String);

#[derive(Component)]
struct VersionText;

fn add_version_label(app: &mut App) {
    app.init_resource::<VersionLabel>()
        .add_systems(Startup, spawn_version_label)
        .add_systems(Update, update_version_label);
}

fn spawn_version_label(mut commands: Commands, label: Res<VersionLabel>) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(4),
            right: px(6),
            ..default()
        },
        Text::new(label.0.clone()),
        TextFont {
            font_size: 11.0.into(),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.45)),
        VersionText,
    ));
}

fn update_version_label(label: Res<VersionLabel>, mut text: Query<&mut Text, With<VersionText>>) {
    if label.is_changed() {
        for mut t in &mut text {
            t.0.clone_from(&label.0);
        }
    }
}

/// Whether the FPS counter is shown (console command `fps`, setting
/// `display.fps`): the real, wall-clock render rate ([`crate::frame_rate`]).
#[derive(Resource, Default)]
pub struct ShowFps(pub bool);

/// Whether the HUD is shown (console command `hud`, setting `display.hud`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowHud(pub bool);

impl Default for ShowHud {
    fn default() -> Self {
        Self(true)
    }
}

/// Units of speed and distance in the HUD (console command `hud.units`,
/// setting `display.units`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    /// km/h and km.
    #[default]
    Metric,
    /// mph and miles.
    Imperial,
}

/// Metres in a mile.
pub const METRES_PER_MILE: f64 = 1609.344;

impl std::str::FromStr for Units {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "metric" => Ok(Self::Metric),
            "imperial" => Ok(Self::Imperial),
            other => Err(format!("unknown units '{other}' (metric, imperial)")),
        }
    }
}

impl Units {
    /// A speed given in km/h, in these units, and its unit.
    pub fn speed(self, kmh: f64) -> (f64, &'static str) {
        match self {
            Self::Metric => (kmh, "km/h"),
            Self::Imperial => (kmh * 1000.0 / METRES_PER_MILE, "mph"),
        }
    }

    /// A distance given in metres, in these units, and its unit.
    pub fn distance(self, m: f64) -> (f64, &'static str) {
        match self {
            Self::Metric => (m / 1000.0, "km"),
            Self::Imperial => (m / METRES_PER_MILE, "mi"),
        }
    }
}

/// The bike computer's panel at the top left.
#[derive(Component)]
pub struct HudTopPanel;

/// The column at the top right: the timestamp, REC, the app message, the
/// FPS counter, the photo preview.
#[derive(Component)]
pub struct HudTopRight;

/// The margin between the HUD's top panels and the window edges (px).
const TOP_MARGIN_PX: f32 = 12.0;
/// The narrowest the top panel gets (px): its text wraps below that.
const TOP_PANEL_MIN_PX: f32 = 160.0;

/// The widest the top-left panel may be (in UI px, as `Val::Px`), so it
/// ends a margin left of the top-right column: `window_px` and `right_px`
/// are physical pixels, `inverse_scale` the UI's (window scale factor ×
/// `UiScale`)⁻¹ – the HUD size setting (spec §3.2.1 M9) scales both
/// panels, never into each other.
pub fn top_panel_max_px(window_px: f32, right_px: f32, inverse_scale: f32) -> f32 {
    ((window_px - right_px) * inverse_scale - 3.0 * TOP_MARGIN_PX).max(TOP_PANEL_MIN_PX)
}

/// Keeps the top-left panel clear of the top-right column (regression:
/// at 150 % HUD size the bike computer ran into the FPS counter).
fn fit_top_panel(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    right: Query<&ComputedNode, With<HudTopRight>>,
    mut panel: Query<&mut Node, With<HudTopPanel>>,
    mut width: ResMut<TopPanelWidth>,
) {
    let (Ok(w), Ok(r)) = (windows.single(), right.single()) else {
        return;
    };
    let max = top_panel_max_px(
        w.resolution.physical_width() as f32,
        r.size().x,
        r.inverse_scale_factor(),
    );
    // Not wider than the window either (the panel's 94 vw).
    let max = max.min(w.resolution.physical_width() as f32 * r.inverse_scale_factor() * 0.94);
    width.set_if_neq(TopPanelWidth(max.floor()));
    for mut n in &mut panel {
        let want = Val::Px(max.floor());
        if n.max_width != want {
            n.max_width = want;
        }
    }
}

/// The HUD's panels (hidden with `hud off`).
#[derive(Component)]
pub(crate) struct HudPanel;

/// Shows or hides the HUD panels.
fn show_hud(show: Res<ShowHud>, mut panels: Query<&mut Visibility, With<HudPanel>>) {
    let want = if show.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut panels {
        if *v != want {
            *v = want;
        }
    }
}

/// Whether the GPS line is shown (console command `hud.gps`, setting
/// `display.gps`; on by default). The line needs a position: routes
/// without a geographic origin (the built-in test routes) have none.
#[derive(Resource)]
pub struct ShowGps(pub bool);

impl Default for ShowGps {
    fn default() -> Self {
        Self(true)
    }
}

/// The GPS line the HUD shows now (`None`: hidden) – for the browser
/// test's page hook and the tests.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct HudGpsLine(pub Option<String>);

/// Whether the elevation profile is shown (console command `hud.profile`).
#[derive(Resource)]
pub struct ShowProfile(pub bool);

impl Default for ShowProfile {
    fn default() -> Self {
        Self(true)
    }
}

/// How far the HUD looks ahead (m): the profile's reach.
const LOOK_AHEAD_M: f64 = 1500.0;

/// The local rider's path ahead for the HUD (free ride: the
/// [`RidePath::preview`], rebuilt when the path or the selected exit
/// changes; route mode: `None`, the route is known).
#[derive(Resource, Default)]
pub struct LookAhead {
    key: Option<(usize, u64, u64, Option<usize>)>,
    rider_m: f64,
    path: Option<Arc<RidePath>>,
}

impl LookAhead {
    /// What the HUD looks ahead on for rider `id` (the local rider).
    pub fn track<'a>(&'a self, view: &'a RideView, id: RiderId) -> &'a dyn Track {
        match &self.path {
            Some(p) => p.as_ref(),
            None => view.track(id),
        }
    }
}

fn update_look_ahead(view: Res<RideView>, mut ahead: ResMut<LookAhead>) {
    let path = view.local().and_then(|l| Some((view.paths.get(&l.id)?, l)));
    let Some((path, local)) = path else {
        if ahead.key.is_some() {
            *ahead = LookAhead::default();
        }
        return;
    };
    let d = local.state.distance;
    // The rider's current choice at the next junction (not yet locked).
    let first = view
        .junction
        .as_ref()
        .filter(|j| !j.locked)
        .map(|j| j.selected);
    let key = (
        Arc::as_ptr(path) as usize,
        path.next_index(),
        path.end_m().to_bits(),
        first,
    );
    // Reaching too short by now (the rider rode on): look again.
    let reach = ahead.path.as_ref().map_or(path.end_m(), |p| p.end_m());
    let stale = reach < d + LOOK_AHEAD_M && (d - ahead.rider_m).abs() > 50.0;
    if ahead.key == Some(key) && !stale {
        return;
    }
    ahead.key = Some(key);
    ahead.rider_m = d;
    ahead.path = path.preview(d, LOOK_AHEAD_M, first).map(Arc::new);
}

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct HudSmallText;

/// The "REC" indicator row (top right, only while recording).
#[derive(Component)]
struct RecRow;

/// The blinking red dot of the indicator.
#[derive(Component)]
struct RecDot;

#[derive(Component)]
struct RecText;

/// The app message under the indicator ([`RideView::app_message`]).
#[derive(Component)]
struct AppMessageText;

const REC_RED: Color = Color::srgb(1.0, 0.22, 0.18);

/// The GPS line, bottom-left.
#[derive(Component)]
struct HudGps;

/// The elevation profile box (bottom left, above the GPS line).
#[derive(Component)]
pub(crate) struct ProfilePanel;

#[derive(Component)]
struct ProfileCaption;

/// Bar `i` of the profile (from slot `i` to `i + 1`).
#[derive(Component)]
struct ProfileBar(usize);

/// The rider's position in the profile.
#[derive(Component)]
struct ProfileRider;

/// Size of the profile chart (logical px).
const PROFILE_W: f32 = 300.0;
const PROFILE_H: f32 = 56.0;
/// The lowest point of the profile keeps this much of the height.
const PROFILE_BASE: f64 = 0.08;

#[derive(Component)]
struct HudGpsText;

fn spawn_hud(mut commands: Commands) {
    // Bottom left: the elevation profile above the GPS line (the version
    // label is bottom right, the REC indicator top right).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(12),
                left: px(12),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: px(6),
                ..default()
            },
            HudPanel,
        ))
        .with_children(|col| {
            spawn_profile(col);
            col.spawn((
                Node {
                    padding: UiRect::axes(px(8), px(4)),
                    border_radius: BorderRadius::all(px(4)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                HudGps,
            ))
            .with_child((
                Text::new(""),
                TextFont {
                    font_size: 15.0.into(),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                HudGpsText,
            ));
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                // Never wider than the window (12 px margins at 400 px and
                // up); Bevy wraps text that is still too long. Narrowed to
                // the room left of the top-right column ([`fit_top_panel`]).
                max_width: Val::Vw(94.0),
                padding: UiRect::all(px(PANEL_PAD)),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            HudPanel,
            HudTopPanel,
            crate::navi_map::MapAvoid,
        ))
        .with_children(|hud| {
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: BIG_FONT.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                HudText,
            ));
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: SMALL_FONT.into(),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
                HudSmallText,
            ));
            // The damage indicator beside the bike computer's lines
            // (spec §14 Phase 12F), hidden without damage.
            crate::damage_hud::spawn_widget(hud);
        });
}

/// The large HUD line: power, speed, cadence, road gradient (at the rider
/// and, in brackets, the average over the next 100 m), heart rate.
pub fn main_line(
    power_w: f64,
    speed_kmh: f64,
    cadence_rpm: Option<f64>,
    grade_percent: f64,
    ahead_percent: Option<f64>,
    heart_rate_bpm: Option<f64>,
    units: Units,
) -> String {
    let (speed, unit) = units.speed(speed_kmh);
    let cadence = cadence_rpm.map_or_else(|| "- rpm".into(), |c| format!("{c:.0} rpm"));
    // Avoid "-0.0 %" on the flat.
    let tidy = |g: f64| if g.abs() < 0.05 { 0.0 } else { g };
    let grade = tidy(grade_percent);
    let ahead = ahead_percent
        .map(|a| format!(" ({:.0} m {:+.1} %)", AVERAGE_AHEAD_M, tidy(a)))
        .unwrap_or_default();
    let heart = heart_rate_bpm
        .map(|h| format!("   HR {h:.0}"))
        .unwrap_or_default();
    format!("{power_w:>4.0} W   {speed:>5.1} {unit}   {cadence}   {grade:+.1} %{ahead}{heart}")
}

/// Colour of a profile bar of gradient band `band` (dimmed behind the
/// rider): blue descents, green flat, yellow, orange, red, and wine-red
/// walls – the usual climb-profile colours.
pub fn band_color(band: GradeBand, behind: bool) -> Color {
    let (r, g, b) = match band {
        GradeBand::Descent => (0.35, 0.62, 0.95),
        GradeBand::Flat => (0.35, 0.78, 0.38),
        GradeBand::Moderate => (0.96, 0.84, 0.25),
        GradeBand::Hard => (0.98, 0.55, 0.16),
        GradeBand::VeryHard => (0.90, 0.20, 0.16),
        GradeBand::Wall => (0.62, 0.08, 0.30),
    };
    Color::srgba(r, g, b, if behind { 0.4 } else { 0.92 })
}

/// One bar of the profile: its height (fraction of the chart), gradient
/// band and whether it is behind the rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileBarView {
    pub level: f64,
    pub band: GradeBand,
    pub behind: bool,
}

/// The bars of `profile`, one per pair of neighbouring slots; `None` where
/// the road is not known (before the start, after a route's finish).
pub fn profile_bars(profile: &ElevationProfile) -> Vec<Option<ProfileBarView>> {
    let n = profile.heights.len().saturating_sub(1);
    (0..n)
        .map(|i| {
            let (a, b) = (
                profile.level(i, PROFILE_BASE)?,
                profile.level(i + 1, PROFILE_BASE)?,
            );
            Some(ProfileBarView {
                level: (a + b) / 2.0,
                band: GradeBand::of(profile.grade(i)?),
                behind: (i as f64 + 0.5) / n as f64 <= profile.rider_frac,
            })
        })
        .collect()
}

/// The profile caption: "next 1.5 km  +86 m  -12 m" (climbing and
/// descending ahead).
pub fn profile_caption(profile: &ElevationProfile, ahead_m: f64) -> String {
    let (up, down) = profile.ahead_gain_loss();
    format!("next {:.1} km  +{up:.0} m  -{down:.0} m", ahead_m / 1000.0)
}

fn spawn_profile(parent: &mut ChildSpawnerCommands) {
    let spec = ProfileSpec::default();
    let bars = ((spec.behind_m + spec.ahead_m) / spec.spacing_m).round() as usize;
    parent
        .spawn((
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                padding: UiRect::axes(px(8), px(5)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ProfilePanel,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(""),
                TextFont {
                    font_size: 12.0.into(),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
                ProfileCaption,
            ));
            panel
                .spawn(Node {
                    width: px(PROFILE_W),
                    max_width: Val::Vw(60.0),
                    height: px(PROFILE_H),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::FlexEnd,
                    ..default()
                })
                .with_children(|chart| {
                    for i in 0..bars {
                        chart.spawn((
                            Node {
                                width: Val::Percent(100.0 / bars as f32),
                                height: Val::Percent(0.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            ProfileBar(i),
                        ));
                    }
                    chart.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(0.0),
                            top: px(0),
                            width: px(2),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
                        ProfileRider,
                    ));
                });
        });
}

type BarParts = (
    &'static ProfileBar,
    &'static mut Node,
    &'static mut BackgroundColor,
);

#[allow(clippy::type_complexity)] // a Bevy system: disjoint UI queries
fn update_profile(
    view: Res<RideView>,
    show: Res<ShowProfile>,
    ahead: Res<LookAhead>,
    mut panel: Query<
        &mut Node,
        (
            With<ProfilePanel>,
            Without<ProfileBar>,
            Without<ProfileRider>,
        ),
    >,
    mut bars: Query<BarParts, (Without<ProfilePanel>, Without<ProfileRider>)>,
    mut rider: Query<
        &mut Node,
        (
            With<ProfileRider>,
            Without<ProfilePanel>,
            Without<ProfileBar>,
        ),
    >,
    mut caption: Query<&mut Text, With<ProfileCaption>>,
) {
    let Ok(mut panel) = panel.single_mut() else {
        return;
    };
    let local = view.local().filter(|_| show.0);
    let display = if local.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if panel.display != display {
        panel.display = display;
    }
    let Some(local) = local else {
        return;
    };
    let spec = ProfileSpec::default();
    let track = ahead.track(&view, local.id);
    let profile = ElevationProfile::sample(track, local.state.distance, &spec);
    let views = profile_bars(&profile);
    for (bar, mut node, mut color) in &mut bars {
        let (h, c) = match views.get(bar.0).copied().flatten() {
            Some(v) => (
                Val::Percent((v.level * 100.0) as f32),
                band_color(v.band, v.behind),
            ),
            None => (Val::Percent(0.0), Color::NONE),
        };
        if node.height != h {
            node.height = h;
        }
        if color.0 != c {
            color.0 = c;
        }
    }
    if let Ok(mut node) = rider.single_mut() {
        let left = Val::Percent((profile.rider_frac * 100.0) as f32);
        if node.left != left {
            node.left = left;
        }
    }
    if let Ok(mut text) = caption.single_mut() {
        let line = profile_caption(&profile, spec.ahead_m);
        if text.0 != line {
            text.0 = line;
        }
    }
}

/// The recording indicator's text: "REC 0:12" (minutes:seconds, hours
/// in front from one hour on).
pub fn recording_label(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("REC {}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("REC {}:{:02}", s / 60, s % 60)
    }
}

/// Top right, from the top: the timestamp (always, [`crate::stamp`]), a
/// small red dot + "REC m:ss" while a video is recorded, the app message
/// (e.g. "Video saved: ...") and the FPS counter.
fn spawn_recording(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                right: px(12),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(4),
                ..default()
            },
            HudTopRight,
            crate::navi_map::MapAvoid,
        ))
        .with_children(|top| {
            top.spawn(crate::stamp::stamp_bundle());
            top.spawn((
                Node {
                    display: Display::None,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    padding: UiRect::axes(px(8), px(4)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                RecRow,
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: px(10),
                        height: px(10),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(REC_RED),
                    RecDot,
                ));
                row.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 16.0.into(),
                        ..default()
                    },
                    TextColor(REC_RED),
                    RecText,
                ));
            });
            top.spawn((
                Node {
                    display: Display::None,
                    padding: UiRect::axes(px(8), px(4)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                Text::new(""),
                TextFont {
                    font_size: 15.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                AppMessageText,
            ));
            // The Strava recording (spec §2.8.1): "● Strava 12:34" and its
            // notice under it.
            top.spawn((
                Node {
                    display: Display::None,
                    padding: UiRect::axes(px(8), px(4)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                Text::new(""),
                TextFont {
                    font_size: 16.0.into(),
                    ..default()
                },
                TextColor(STRAVA_ORANGE),
                StravaBadge,
            ));
            top.spawn((
                Node {
                    display: Display::None,
                    padding: UiRect::axes(px(8), px(4)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                Text::new(""),
                TextFont {
                    font_size: 15.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                StravaNotice,
            ));
            top.spawn(crate::frame_rate::badge_bundle());
            // The preview of a photo just saved, under the FPS counter.
            crate::photo_preview::spawn_card(top);
        });
}

/// The Strava badge and its notice ([`RideView::strava_s`],
/// [`RideView::strava_message`]).
#[derive(Component)]
struct StravaBadge;

#[derive(Component)]
struct StravaNotice;

const STRAVA_ORANGE: Color = Color::srgb(0.99, 0.32, 0.0);

/// The badge's text: "● Strava 12:34".
pub fn strava_label(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("● Strava {}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("● Strava {}:{:02}", s / 60, s % 60)
    }
}

type StravaParts = (&'static mut Node, &'static mut Text);

fn update_strava(
    view: Res<RideView>,
    mut badge: Query<StravaParts, (With<StravaBadge>, Without<StravaNotice>)>,
    mut notice: Query<StravaParts, (With<StravaNotice>, Without<StravaBadge>)>,
) {
    let show = |node: &mut Node, text: &mut Text, s: Option<String>| {
        let display = if s.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let s = s.unwrap_or_default();
        if text.0 != s {
            text.0 = s;
        }
    };
    if let Ok((mut n, mut t)) = badge.single_mut() {
        show(&mut n, &mut t, view.strava_s.map(strava_label));
    }
    if let Ok((mut n, mut t)) = notice.single_mut() {
        show(&mut n, &mut t, view.strava_message.clone());
    }
}

type MessageParts = (&'static mut Node, &'static mut Text);

fn update_recording(
    view: Res<RideView>,
    mut row: Query<&mut Node, (With<RecRow>, Without<AppMessageText>)>,
    mut dot: Query<&mut Visibility, With<RecDot>>,
    mut rec_text: Query<&mut Text, (With<RecText>, Without<AppMessageText>)>,
    mut message: Query<MessageParts, (With<AppMessageText>, Without<RecRow>)>,
) {
    let want_row = if view.recording_s.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if let Ok(mut node) = row.single_mut()
        && node.display != want_row
    {
        node.display = want_row;
    }
    if let Some(rec) = view.recording_s {
        // The dot blinks: on for the first 0.6 s of every second.
        let on = rec.rem_euclid(1.0) < 0.6;
        if let Ok(mut vis) = dot.single_mut() {
            vis.set_if_neq(if on {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
        if let Ok(mut text) = rec_text.single_mut() {
            let label = recording_label(rec);
            if text.0 != label {
                text.0 = label;
            }
        }
    }
    if let Ok((mut node, mut text)) = message.single_mut() {
        let msg = view.app_message.clone().unwrap_or_default();
        let display = if msg.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
        if text.0 != msg {
            text.0 = msg;
        }
    }
}

/// The GPS line: "47.12345° N  9.53210° E  812 m" (5 decimals ≈ 1 m).
pub fn gps_line(lat: f64, lon: f64, altitude_m: f64) -> String {
    fn coord(v: f64, pos: char, neg: char) -> String {
        // Round first, so a value that rounds to zero shows no "S" / "W".
        let r = (v * 1e5).round() / 1e5;
        let h = if r < 0.0 { neg } else { pos };
        format!("{:.5}° {h}", r.abs())
    }
    format!(
        "{}  {}  {:.0} m",
        coord(lat, 'N', 'S'),
        coord(lon, 'E', 'W'),
        altitude_m
    )
}

/// Shows the GPS line while the position is known (not on test routes)
/// and the line is on ([`ShowGps`]).
fn update_gps(
    view: Res<RideView>,
    show: Res<ShowGps>,
    mut shown: ResMut<HudGpsLine>,
    mut boxes: Query<&mut Node, With<HudGps>>,
    mut texts: Query<&mut Text, With<HudGpsText>>,
) {
    let (Ok(mut node), Ok(mut text)) = (boxes.single_mut(), texts.single_mut()) else {
        return;
    };
    let line = view
        .position
        .filter(|_| show.0)
        .map(|(lat, lon, alt)| gps_line(lat, lon, alt));
    let display = if line.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if let Some(line) = &line
        && text.0 != *line
    {
        text.0.clone_from(line);
    }
    if shown.0 != line {
        shown.0 = line;
    }
}

/// Font size of the small HUD lines; their glyphs (the monospaced UI font,
/// [`crate::font`]) are about 0.6 of it wide.
const SMALL_FONT: f32 = 15.0;
const SMALL_CHAR_W: f32 = SMALL_FONT * 0.6;

/// Breaks each line of `text` that is longer than `max_chars` at its
/// " | " separators (then at ", ", then between words), greedily, so the
/// HUD stays inside its panel; a word longer than `max_chars` is cut.
/// Continuation lines start with no separator. Every line of the result
/// fits, so Bevy never wraps it again (bug 2026-11-17: a line Bevy
/// wrapped by itself ran below the panel's bottom edge).
pub fn wrap_hud_lines(text: &str, max_chars: usize) -> String {
    wrap_with(text, max_chars, &[" | ", ", "])
}

/// The main line broken between its fields (three spaces apart), then
/// between words, at `max_chars`.
pub fn wrap_main_line(text: &str, max_chars: usize) -> String {
    wrap_with(text, max_chars, &["   "])
}

fn len(s: &str) -> usize {
    s.chars().count()
}

/// Joins the pieces of `line` split at `sep` greedily into lines of at
/// most `max` characters (a longer piece stays whole).
fn join_greedy(line: &str, sep: &str, max: usize, out: &mut Vec<String>) {
    let mut cur = String::new();
    for piece in line.split(sep) {
        if !cur.is_empty() && len(&cur) + len(sep) + len(piece) > max {
            out.push(std::mem::take(&mut cur));
        }
        if cur.is_empty() {
            cur.push_str(piece);
        } else {
            cur.push_str(sep);
            cur.push_str(piece);
        }
    }
    out.push(cur);
}

/// `line` in lines of at most `max` characters: split at the first of
/// `seps`, the pieces still too long at the next, at last between words
/// and inside a too long word.
fn wrap_piece(line: &str, max: usize, seps: &[&str], out: &mut Vec<String>) {
    if len(line) <= max {
        out.push(line.to_string());
        return;
    }
    let mut parts = Vec::new();
    match seps.split_first() {
        Some((sep, rest)) => {
            join_greedy(line, sep, max, &mut parts);
            for part in parts {
                wrap_piece(&part, max, rest, out);
            }
        }
        None => {
            join_greedy(line.trim(), " ", max, &mut parts);
            for part in parts {
                let chars: Vec<char> = part.chars().collect();
                out.extend(chars.chunks(max).map(|c| c.iter().collect::<String>()));
            }
        }
    }
}

fn wrap_with(text: &str, max_chars: usize, seps: &[&str]) -> String {
    let max = max_chars.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        wrap_piece(line, max, seps, &mut out);
    }
    out.join("\n")
}

/// Characters of the small HUD font that fit in a window `width` logical
/// pixels wide (panel margins and padding taken off) – before the panel's
/// own width is known ([`TopPanelWidth`]).
fn small_chars_for(width: f32) -> usize {
    ((width * 0.94 - 2.0 * 10.0 - 4.0) / SMALL_CHAR_W).max(10.0) as usize
}

/// Font size of the big HUD line.
const BIG_FONT: f32 = 26.0;
/// The top panel's padding (px).
const PANEL_PAD: f32 = 10.0;
/// Bevy's line height (of the font size).
const LINE_HEIGHT: f32 = 1.2;

/// Characters of a monospaced line of `font` px that fit in the top panel
/// `panel_px` wide (its padding taken off, one character to spare).
pub fn panel_chars(panel_px: f32, font: f32) -> usize {
    (((panel_px - 2.0 * PANEL_PAD) / (font * 0.6)).floor() as usize)
        .saturating_sub(1)
        .max(10)
}

/// The top panel's text broken for a panel `panel_px` wide (UI px): the
/// big line and the small lines, each line inside the panel.
pub fn top_panel_text(panel_px: f32, big: &str, small: &str) -> (String, String) {
    (
        wrap_main_line(big, panel_chars(panel_px, BIG_FONT)),
        wrap_hud_lines(small, panel_chars(panel_px, SMALL_FONT)),
    )
}

/// The top panel's height (UI px) with these (broken) lines: it grows with
/// them (Bevy lays them out at [`LINE_HEIGHT`]).
pub fn top_panel_height(big: &str, small: &str) -> f32 {
    let lines = |t: &str| t.lines().count().max(1) as f32;
    2.0 * PANEL_PAD
        + lines(big) * BIG_FONT * LINE_HEIGHT
        + 4.0
        + lines(small) * SMALL_FONT * LINE_HEIGHT
}

/// The width the top panel may take now (UI px; [`fit_top_panel`]), 0
/// before the first layout.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct TopPanelWidth(pub f32);

/// The compass point of a heading (degrees, 0 = north).
pub fn compass(deg: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[((deg.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

/// "riders: 4", with how many of them fly in god mode (spec §14 Phase
/// 13E: who flies is seen at a glance): "riders: 4 (1 flying)".
pub fn riders_count(view: &RideView) -> String {
    let flying = view.unicorns.len();
    if flying == 0 {
        format!("riders: {}", view.riders.len())
    } else {
        format!("riders: {} ({flying} flying)", view.riders.len())
    }
}

/// The flight panel's line (god mode, spec §14 Phase 13D): altitude above
/// sea level and above the ground, speed, the wheel's speed factor and
/// the heading.
pub fn flight_line(f: &crate::FlightView, units: Units) -> String {
    let (speed, unit) = units.speed(mps_to_kmh(f.speed_mps));
    let heading = ocw_sim::god::heading_deg(f.yaw);
    format!(
        "{}  {:.0} m  ({:.0} m above ground)   {speed:.0} {unit}   x{:.2}   {heading:03.0}° {}",
        if f.leaving { "Landing" } else { "Flying" },
        f.pos[1],
        f.height_m.max(0.0),
        f.scale,
        compass(heading)
    )
}

/// The main line with the weather (spec §14 Phase 9, S4): its summary and
/// the wind relative to the rider (S6, "↓ headwind 18 km/h").
pub fn weather_hud(line: &str, w: &crate::weather::WeatherView) -> String {
    match w.wind_hud() {
        Some(wind) => format!("{line}   {}   {wind}", w.summary),
        None => format!("{line}   {}", w.summary),
    }
}

/// " | Weather: MET Norway, CC BY 4.0" while the weather is real (spec
/// §14 Phase 9, S7: the provider's credit stays on screen), else "".
pub fn attribution_suffix(view: &RideView) -> String {
    view.weather
        .as_ref()
        .and_then(|w| w.attribution.as_deref())
        .map(|a| format!(" | {a}"))
        .unwrap_or_default()
}

fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn update_hud(
    view: Res<RideView>,
    ahead: Res<LookAhead>,
    units: Res<Units>,
    app_keys: Res<AppKeys>,
    panel: Res<TopPanelWidth>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut big: Query<&mut Text, (With<HudText>, Without<HudSmallText>)>,
    mut small: Query<&mut Text, (With<HudSmallText>, Without<HudText>)>,
) {
    let (Ok(mut big), Ok(mut small)) = (big.single_mut(), small.single_mut()) else {
        return;
    };
    // God mode (spec §14 Phase 13D): a compact flight panel instead of the
    // bike computer.
    if let Some(f) = &view.flight {
        let line = flight_line(f, *units);
        let line = match &view.weather {
            Some(w) => weather_hud(&line, w),
            None => line,
        };
        let line = match &view.sky {
            Some(s) => format!("{line}   {}", s.hud),
            None => line,
        };
        let mut lines = format!(
            "{}{} | {}",
            format_time(view.elapsed_s),
            attribution_suffix(&view),
            f.keys
        );
        if !view.status.is_empty() {
            lines.push('\n');
            lines.push_str(&view.status);
        }
        let (line, lines) = panel_text(panel_px(&panel, &windows), &line, &lines);
        if big.0 != line {
            big.0 = line;
        }
        if small.0 != lines {
            small.0 = lines;
        }
        return;
    }
    let Some(local) = view.local() else {
        big.0 = "Connecting...".into();
        return;
    };
    let track = view.track(local.id);
    let grade = track.sample(local.state.distance).grade * 100.0;
    let avg = ocw_world::average_grade(
        ahead.track(&view, local.id),
        local.state.distance,
        AVERAGE_AHEAD_M,
    ) * 100.0;
    let line = main_line(
        local.power_w,
        mps_to_kmh(local.state.speed),
        local.cadence_rpm,
        grade,
        Some(avg),
        local.heart_rate_bpm,
        *units,
    );
    // The weather (spec §14 Phase 9, S4) and the world clock of the
    // rider's place (spec §14 Phase 9; the ISO timestamp of captures is
    // separate, top right).
    let line = match &view.weather {
        Some(w) => weather_hud(&line, w),
        None => line,
    };
    let line = match &view.sky {
        Some(s) => format!("{line}   {}", s.hud),
        None => line,
    };
    let (place, keys) = if let Some(route) = &view.route_status {
        (
            format!("{route}: {}", track.name_at(local.state.distance)),
            "up: faster, down: stop, left/right: steer or choose the exit (another than the route's leaves it), Q/E: exit, J: rejoin the route, U: U-turn, A: autodrive",
        )
    } else if view.free_ride {
        (
            format!("Free ride: {}", track.name_at(local.state.distance)),
            "up: faster, down: stop, left/right: steer or choose the exit at junctions, Q/E: exit, U: U-turn, A: autodrive, W: wild",
        )
    } else {
        (
            view.route.name().to_string(),
            "up: faster, down: stop, left/right: steer, A: autodrive, W: wild",
        )
    };
    let (distance, distance_unit) = units.distance(local.state.distance);
    let mut lines = format!(
        "{place} | {distance:.2} {distance_unit} | {} | {}{}\n{keys} | {}",
        format_time(view.elapsed_s),
        riders_count(&view),
        attribution_suffix(&view),
        app_keys.0
    );
    if !view.status.is_empty() {
        lines.push('\n');
        lines.push_str(&view.status);
    }
    let (line, lines) = panel_text(panel_px(&panel, &windows), &line, &lines);
    if big.0 != line {
        big.0 = line;
    }
    if small.0 != lines {
        small.0 = lines;
    }
}

/// The top panel's width now (UI px): as [`fit_top_panel`] set it
/// (`Ok`), or before the first layout only the window's width (`Err`).
fn panel_px(
    panel: &TopPanelWidth,
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
) -> Result<f32, f32> {
    if panel.0 > 0.0 {
        Ok(panel.0)
    } else {
        Err(windows.single().map_or(1280.0, |w| w.width()))
    }
}

/// The big and the small lines broken for the panel.
fn panel_text(width: Result<f32, f32>, big: &str, small: &str) -> (String, String) {
    match width {
        Ok(px) => top_panel_text(px, big, small),
        Err(window) => (
            big.to_string(),
            wrap_hud_lines(small, small_chars_for(window)),
        ),
    }
}

/// Test helpers for the other HUD modules (the navigation map's fit).
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// The top panel's width and its bottom edge (UI px) in a window `w`
    /// UI px wide, with the longest lines of the top panel's fit test
    /// (the long weather line, the route ride's keys, a long status).
    pub(crate) fn tall_top_panel(w: f32) -> (f32, f32) {
        let stamp = 25.0 * 14.0 * 0.6 + 16.0;
        let panel = top_panel_max_px(w, stamp, 1.0).min(w * 0.94);
        let big = format!(
            "{}   clear 18 °C   ↗ crosswind from the left 7 km/h   ☼ 09:44",
            main_line(
                200.0,
                20.8,
                Some(80.0),
                7.7,
                Some(8.3),
                Some(142.0),
                Units::Metric
            )
        );
        let small = format!(
            "Test Hills | 2.01 km | 0:00:24 | riders: 3\nup: faster, down: stop, left/right: steer or choose the exit (another than the route's leaves it), Q/E: exit, J: rejoin the route, U: U-turn, A: autodrive | {DESKTOP_KEYS}\nVirtual trainer 200 W (no Web Bluetooth here) | trainer SIM +1.9 % (difficulty 50 %) | autodrive | gravel"
        );
        let (b, sm) = top_panel_text(panel, &big, &small);
        (panel, TOP_MARGIN_PX + top_panel_height(&b, &sm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (HUD at 150 %, `access-hud`): the top-left panel ran into
    /// the FPS counter. For HUD sizes 100 – 200 %, window widths and
    /// top-right columns of any width the panel ends a margin left of the
    /// column (in physical pixels).
    #[test]
    fn the_top_panel_stays_clear_of_the_top_right_column() {
        for window in [1280.0f32, 1920.0, 2560.0] {
            for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
                let inv = 1.0 / scale;
                for right_logical in [120.0f32, 230.0, 320.0] {
                    let right = right_logical * scale;
                    let max = top_panel_max_px(window, right, inv);
                    let panel_end = (TOP_MARGIN_PX + max) * scale;
                    let column_start = window - TOP_MARGIN_PX * scale - right;
                    if max > TOP_PANEL_MIN_PX {
                        assert!(
                            panel_end + TOP_MARGIN_PX * scale <= column_start + 0.01,
                            "{window} px, {scale}x, column {right}: {panel_end} > {column_start}"
                        );
                    }
                }
            }
        }
        // The case of the picture: 2560 px at 150 %, a 345 px column.
        assert!((top_panel_max_px(2560.0, 345.0, 1.0 / 1.5) - 1440.67).abs() < 0.01);
        // Never narrower than its minimum (the text wraps).
        assert_eq!(top_panel_max_px(400.0, 380.0, 1.0), TOP_PANEL_MIN_PX);
    }
    use ocw_world::Route;

    /// Spec §14 Phase 9, S6: the HUD shows the wind relative to the rider
    /// next to the weather, in glyphs the HUD font has.
    #[test]
    fn the_hud_shows_the_wind_relative_to_the_rider() {
        let mut w = crate::weather::tests::view_of("windy");
        let line = weather_hud("250 W", &w);
        assert_eq!(line, format!("250 W   {}", w.summary), "no rider: no arrow");
        w.rider_wind = Some(ocw_weather::RelativeWind {
            head_mps: 5.0,
            cross_mps: 0.0,
        });
        let line = weather_hud("250 W", &w);
        assert!(line.ends_with("↓ headwind 18 km/h"), "{line}");
        w.rider_wind = Some(ocw_weather::RelativeWind {
            head_mps: -3.0,
            cross_mps: 3.0,
        });
        // From behind on the left: blowing ahead to the right.
        let line = weather_hud("", &w);
        assert!(
            line.ends_with("↗ crosswind from the left 15 km/h"),
            "{line}"
        );
        w.rider_wind = Some(ocw_weather::RelativeWind::default());
        assert_eq!(weather_hud("x", &w), format!("x   {}", w.summary));
        for arrow in "↑↗→↘↓↙←↖·".chars() {
            assert!(crate::font::covers(&arrow.to_string()), "{arrow}");
        }
    }

    #[test]
    fn recording_label_format() {
        assert_eq!(recording_label(0.0), "REC 0:00");
        assert_eq!(recording_label(12.7), "REC 0:12");
        assert_eq!(recording_label(605.0), "REC 10:05");
        assert_eq!(recording_label(3725.0), "REC 1:02:05");
        assert_eq!(recording_label(-3.0), "REC 0:00");
    }

    fn rec_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .add_systems(Startup, spawn_recording)
            .add_systems(Update, update_recording);
        app.update();
        app
    }

    /// The Strava badge (spec §2.8.1) shows while recording, with its
    /// notice under it.
    #[test]
    fn the_strava_badge_follows_the_view() {
        let mut app = rec_app();
        app.add_systems(Update, update_strava);
        let state = |app: &mut App| {
            let w = app.world_mut();
            let (n, t) = w
                .query_filtered::<(&Node, &Text), With<StravaBadge>>()
                .single(w)
                .unwrap();
            let badge = (n.display, t.0.clone());
            let (n, t) = w
                .query_filtered::<(&Node, &Text), With<StravaNotice>>()
                .single(w)
                .unwrap();
            (badge, (n.display, t.0.clone()))
        };
        app.update();
        assert_eq!(state(&mut app).0.0, Display::None);
        {
            let mut v = app.world_mut().resource_mut::<RideView>();
            v.strava_s = Some(754.0);
            v.strava_message = Some("Strava recording started".into());
        }
        app.update();
        let (badge, notice) = state(&mut app);
        assert_eq!(badge, (Display::Flex, "● Strava 12:34".into()));
        assert_eq!(notice, (Display::Flex, "Strava recording started".into()));
        assert_eq!(strava_label(3725.0), "● Strava 1:02:05");
    }

    fn rec_state(app: &mut App) -> (Display, String, Display, String) {
        let w = app.world_mut();
        let row = w
            .query_filtered::<&Node, With<RecRow>>()
            .single(w)
            .unwrap()
            .display;
        let text = w
            .query_filtered::<&Text, With<RecText>>()
            .single(w)
            .unwrap()
            .0
            .clone();
        let (node, msg) = w
            .query_filtered::<(&Node, &Text), With<AppMessageText>>()
            .single(w)
            .unwrap();
        (row, text, node.display, msg.0.clone())
    }

    #[test]
    fn rec_indicator_follows_the_view() {
        let mut app = rec_app();
        assert_eq!(rec_state(&mut app).0, Display::None);
        assert_eq!(rec_state(&mut app).2, Display::None);

        app.world_mut().resource_mut::<RideView>().recording_s = Some(12.3);
        app.update();
        let (row, text, msg_display, _) = rec_state(&mut app);
        assert_eq!(row, Display::Flex);
        assert_eq!(text, "REC 0:12");
        assert_eq!(msg_display, Display::None);

        let mut view = app.world_mut().resource_mut::<RideView>();
        view.recording_s = None;
        view.app_message = Some("Video saved: a.mp4".into());
        app.update();
        let (row, _, msg_display, msg) = rec_state(&mut app);
        assert_eq!(row, Display::None);
        assert_eq!(msg_display, Display::Flex);
        assert_eq!(msg, "Video saved: a.mp4");
    }

    #[test]
    fn the_riders_count_says_who_flies() {
        let mut v = RideView::new(ocw_world::Route::test_loop());
        assert_eq!(riders_count(&v), "riders: 0");
        v.unicorns.push(crate::UnicornView {
            id: ocw_core::RiderId(2),
            name: "Ada".into(),
            is_local: false,
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed_mps: 0.0,
            climb_mps: 0.0,
            turn_rate: 0.0,
            accel: 0.0,
            age_s: 1.0,
            landing: false,
            colour: ocw_core::Rgb(0),
        });
        assert_eq!(riders_count(&v), "riders: 0 (1 flying)");
    }

    #[test]
    fn the_flight_panel_shows_altitude_speed_factor_and_heading() {
        let f = crate::FlightView {
            pos: [0.0, 1234.4, 0.0],
            yaw: -std::f64::consts::FRAC_PI_2,
            speed_mps: 100.0,
            height_m: 380.0,
            scale: 2.0,
            ..Default::default()
        };
        let line = flight_line(&f, Units::Metric);
        assert!(
            line.starts_with("Flying  1234 m  (380 m above ground)"),
            "{line}"
        );
        assert!(
            line.contains("360 km/h") && line.contains("x2.00"),
            "{line}"
        );
        // Yaw −90°: turned right from north, flying east.
        assert!(line.ends_with("090° E"), "{line}");
        assert_eq!(compass(359.0), "N");
        assert_eq!(compass(225.0), "SW");
    }

    #[test]
    fn time_format() {
        assert_eq!(super::format_time(3725.0), "1:02:05");
    }

    #[test]
    fn main_line_shows_the_gradient() {
        assert_eq!(
            super::main_line(330.0, 10.66, Some(86.0), 11.43, None, None, Units::Metric),
            " 330 W    10.7 km/h   86 rpm   +11.4 %"
        );
        assert_eq!(
            super::main_line(
                0.0,
                0.0,
                None,
                -0.02,
                Some(0.01),
                Some(142.0),
                Units::Metric
            ),
            "   0 W     0.0 km/h   - rpm   +0.0 % (100 m +0.0 %)   HR 142"
        );
        assert!(
            super::main_line(200.0, 35.0, None, -8.25, None, None, Units::Metric)
                .ends_with("-8.2 %")
        );
        // The average over the next 100 m in brackets.
        assert!(
            super::main_line(
                250.0,
                14.0,
                Some(80.0),
                9.4,
                Some(7.81),
                None,
                Units::Metric
            )
            .ends_with("+9.4 % (100 m +7.8 %)")
        );
        // 40 km/h = 24.9 mph.
        assert_eq!(
            super::main_line(250.0, 40.0, None, 0.0, None, None, Units::Imperial),
            " 250 W    24.9 mph   - rpm   +0.0 %"
        );
    }

    #[test]
    fn units_convert_speed_and_distance() {
        assert_eq!(Units::Metric.distance(12_340.0), (12.34, "km"));
        let (mi, unit) = Units::Imperial.distance(METRES_PER_MILE * 3.0);
        assert!((mi - 3.0).abs() < 1e-12 && unit == "mi");
        assert_eq!("imperial".parse(), Ok(Units::Imperial));
        assert!("furlongs".parse::<Units>().is_err());
    }

    /// A straight road along x: flat 200 m, +8 % for 500 m, -4 % to 1 km.
    fn ramp() -> Route {
        let pts = (0..=200)
            .map(|i| {
                let x = f64::from(i) * 5.0;
                let y = if x <= 200.0 {
                    0.0
                } else if x <= 700.0 {
                    (x - 200.0) * 0.08
                } else {
                    40.0 - (x - 700.0) * 0.04
                };
                [x, y, 0.0]
            })
            .collect();
        Route::from_points("ramp", "Ramp", pts, false, 6.0).unwrap()
    }

    #[test]
    fn profile_bars_are_coloured_by_gradient() {
        let r = ramp();
        let p = ElevationProfile::sample(&r, 150.0, &ProfileSpec::default());
        let bars = profile_bars(&p);
        assert_eq!(bars.len(), p.heights.len() - 1);
        // Slots from 0 m (rider at 150 m): flat behind, then the climb,
        // then the descent, then nothing after the finish (1 km).
        let first = bars[0].unwrap();
        assert_eq!((first.band, first.behind), (GradeBand::Flat, true));
        assert!((first.level - PROFILE_BASE).abs() < 0.02, "{first:?}");
        let climb = bars[20].unwrap(); // 300 … 315 m
        assert_eq!((climb.band, climb.behind), (GradeBand::Hard, false));
        assert_eq!(bars[60].unwrap().band, GradeBand::Descent); // 900 m
        assert!(bars[70].is_none() && bars.last().unwrap().is_none());
        // The top of the climb is the top of the chart.
        let top = bars.iter().flatten().map(|b| b.level).fold(0.0, f64::max);
        assert!(top > 0.95, "{top}");
        assert_eq!(profile_caption(&p, 1500.0), "next 1.5 km  +40 m  -11 m");
    }

    #[test]
    fn band_colours_differ_and_dim_behind() {
        use GradeBand::*;
        let all = [Descent, Flat, Moderate, Hard, VeryHard, Wall];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(band_color(*a, false), band_color(*b, false));
            }
            assert!(band_color(*a, true).alpha() < band_color(*a, false).alpha());
        }
    }

    fn panel_display(app: &mut App) -> Display {
        let w = app.world_mut();
        w.query_filtered::<&Node, With<ProfilePanel>>()
            .single(w)
            .unwrap()
            .display
    }

    #[test]
    fn profile_panel_follows_the_rider_and_the_toggle() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(ramp()))
            .init_resource::<ShowProfile>()
            .init_resource::<LookAhead>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, (update_look_ahead, update_profile).chain());
        app.update();
        // No rider yet: hidden.
        assert_eq!(panel_display(&mut app), Display::None);
        app.world_mut()
            .resource_mut::<RideView>()
            .riders
            .push(crate::RiderView {
                id: RiderId(1),
                name: "me".into(),
                state: ocw_sim::RiderState {
                    distance: 400.0,
                    ..default()
                },
                power_w: 0.0,
                cadence_rpm: None,
                heart_rate_bpm: None,
                is_local: true,
                bike: Default::default(),
                look: None,
                parts: Vec::new(),
                crash: None,
                wobble: 0.0,
                damage: None,
                cue: Default::default(),
                signs: Default::default(),
            });
        app.update();
        assert_eq!(panel_display(&mut app), Display::Flex);
        let w = app.world_mut();
        let caption = w
            .query_filtered::<&Text, With<ProfileCaption>>()
            .single(w)
            .unwrap()
            .0
            .clone();
        assert!(caption.starts_with("next 1.5 km  +23 m"), "{caption}");
        let coloured = w
            .query::<(&ProfileBar, &BackgroundColor)>()
            .iter(w)
            .filter(|(_, c)| c.0 != Color::NONE)
            .count();
        assert!(coloured > 30, "{coloured}");
        app.world_mut().resource_mut::<ShowProfile>().0 = false;
        app.update();
        assert_eq!(panel_display(&mut app), Display::None);
    }

    #[test]
    fn long_hud_lines_break_at_separators() {
        use super::wrap_hud_lines as w;
        assert_eq!(w("short | line", 40), "short | line");
        assert_eq!(w("aaaa | bbbb | cccc", 11), "aaaa | bbbb\ncccc");
        // Too long between separators: break at commas.
        assert_eq!(
            w("up: faster, down: stop, A: autodrive | F11: screen", 24),
            "up: faster, down: stop\nA: autodrive\nF11: screen"
        );
        // Line breaks of the input are kept; then words, a too long word
        // is cut (bug 2026-11-17: Bevy wrapped what was left by itself).
        assert_eq!(w("x\nabcdefghij", 4), "x\nabcd\nefgh\nij");
        assert_eq!(w("one two three", 9), "one two\nthree");
        // Nothing longer than the limit where a break is possible.
        let keys = "Vaduz-Malbun: Landstrasse | 12.34 km | 0:42:00 | riders: 4\nup: faster, down: stop, left/right: steer, A: autodrive, W: wild | mouse: camera, C: camera reset, R: restart, V: video | F11: screen | ` or F12: console";
        for max in [40, 80, 120] {
            assert!(
                w(keys, max).lines().all(|l| l.chars().count() <= max),
                "{max}"
            );
        }
        assert_eq!(w(keys, 1000), keys);
    }

    /// Regression (2026-11-17, the browser at 1280×720: the "Virtual
    /// trainer …" line ran below the top panel's bottom edge): at every
    /// 16:9 size down to 1280×720 and at 4:3, with the HUD at 100 – 150 %,
    /// in both apps (their key lines), with the long weather main line and
    /// a long device status, every broken line fits the panel's width (so
    /// Bevy never wraps it again and the panel grows with every line), and
    /// the panel stays in the upper part of the window, clear of the
    /// top-right column.
    #[test]
    fn the_top_panel_fits_every_window() {
        let big = format!(
            "{}   clear 18 °C   ↗ crosswind from the left 7 km/h   ☼ 09:44",
            super::main_line(
                200.0,
                20.8,
                Some(80.0),
                7.7,
                Some(8.3),
                Some(142.0),
                Units::Metric
            )
        );
        let status = "Virtual trainer 200 W (no Web Bluetooth here) | trainer SIM +1.9 % (difficulty 50 %) | autodrive | gravel";
        let keys = [
            DESKTOP_KEYS,
            "mouse: camera, C: camera reset | ` or F12: console",
        ];
        let route_keys = "up: faster, down: stop, left/right: steer or choose the exit (another than the route's leaves it), Q/E: exit, J: rejoin the route, U: U-turn, A: autodrive";
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
        // The top-right column: the timestamp (14 px font, padding 8).
        let stamp = 25.0 * 14.0 * 0.6 + 16.0;
        for (w, h) in sizes {
            for scale in [1.0f32, 1.25, 1.5] {
                let panel = top_panel_max_px(w, stamp * scale, 1.0 / scale).min(w / scale * 0.94);
                for app in keys {
                    for k in [
                        "up: faster, down: stop, left/right: steer, A: autodrive, W: wild",
                        route_keys,
                    ] {
                        let small = format!(
                            "Test Hills | 2.01 km | 0:00:24 | riders: 3\n{k} | {app}\n{status}"
                        );
                        let (b, sm) = top_panel_text(panel, &big, &small);
                        let what = format!("{w}×{h} at {scale}: {app}");
                        let fits = |t: &str, font: f32| {
                            t.lines().all(|l| {
                                l.chars().count() as f32 * font * 0.6 <= panel - 2.0 * PANEL_PAD
                            })
                        };
                        assert!(fits(&b, BIG_FONT), "{what}: {b}");
                        assert!(fits(&sm, SMALL_FONT), "{what}: {sm}");
                        // Nothing lost or reordered, only broken.
                        let words = |t: &str| t.split_whitespace().collect::<String>();
                        assert_eq!(words(&b), words(&big), "{what}");
                        let bare = |t: &str| words(t).replace(['|', ','], "");
                        assert_eq!(bare(&sm), bare(&small), "{what}");
                        assert!(sm.lines().last().unwrap().ends_with("gravel"), "{what}");
                        // On screen and, at 100 %, in the upper half (a
                        // larger HUD on a small window takes more).
                        let height = (TOP_MARGIN_PX + top_panel_height(&b, &sm)) * scale;
                        let room = if scale < 1.1 {
                            Some(0.55)
                        } else if scale < 1.3 {
                            Some(0.75)
                        } else {
                            (w >= 1280.0).then_some(1.0)
                        };
                        if let Some(room) = room {
                            assert!(height < h * room, "{what}: {height} px of {h}");
                        }
                    }
                }
            }
        }
        // The case of the picture: the browser at 1280×720.
        let panel = top_panel_max_px(1280.0, stamp, 1.0);
        let small = format!(
            "Test Hills | 2.01 km | 0:00:24 | riders: 3\nup: faster, down: stop, left/right: steer, A: autodrive, W: wild | {} \n{status}",
            keys[1]
        );
        let (b, sm) = top_panel_text(panel, &big, &small);
        assert!(
            b.lines().count() >= 2 && sm.lines().count() >= 4,
            "{b}\n{sm}"
        );
        assert!(top_panel_height(&b, &sm) > 2.0 * PANEL_PAD + 6.0 * 18.0);
    }

    #[test]
    fn hud_width_in_characters() {
        // The panel's width: its padding off, one character to spare.
        assert_eq!(panel_chars(920.0, SMALL_FONT), 99);
        assert_eq!(panel_chars(100.0, BIG_FONT), 10);
        // 800x600: about 80 characters of the small font.
        assert!((75..=85).contains(&super::small_chars_for(800.0)));
        assert!(super::small_chars_for(2560.0) > 250);
        assert_eq!(super::small_chars_for(10.0), 10);
    }

    /// Regression (spec 3.40): the GPS line shows while the view has a
    /// position and the line is on, bottom left under the elevation
    /// profile, and hides without a position.
    #[test]
    fn the_gps_line_follows_the_view() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .init_resource::<ShowProfile>()
            .init_resource::<ShowGps>()
            .init_resource::<HudGpsLine>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, update_gps);
        app.update();
        let state = |app: &mut App| {
            let w = app.world_mut();
            let display = w
                .query_filtered::<&Node, With<HudGps>>()
                .single(w)
                .unwrap()
                .display;
            let text = w
                .query_filtered::<&Text, With<HudGpsText>>()
                .single(w)
                .unwrap()
                .0
                .clone();
            (display, text, w.resource::<HudGpsLine>().0.clone())
        };
        assert_eq!(state(&mut app).0, Display::None, "no position: hidden");
        app.world_mut().resource_mut::<RideView>().position = Some((47.139961, 9.521512, 455.2));
        app.update();
        let line = "47.13996° N  9.52151° E  455 m".to_string();
        assert_eq!(
            state(&mut app),
            (Display::Flex, line.clone(), Some(line.clone()))
        );
        // hud.gps off hides it, on shows it again.
        app.world_mut().resource_mut::<ShowGps>().0 = false;
        app.update();
        assert_eq!(state(&mut app).0, Display::None);
        assert_eq!(state(&mut app).2, None);
        app.world_mut().resource_mut::<ShowGps>().0 = true;
        app.update();
        assert_eq!(state(&mut app).0, Display::Flex);
        // Its place: in the bottom-left column, after the profile.
        let w = app.world_mut();
        let gps = w
            .query_filtered::<Entity, With<HudGps>>()
            .single(w)
            .unwrap();
        let parent = w.get::<ChildOf>(gps).unwrap().parent();
        let node = w.get::<Node>(parent).unwrap();
        assert!(node.bottom != Val::Auto && node.left != Val::Auto);
        let kids: Vec<Entity> = w.get::<Children>(parent).unwrap().iter().collect();
        assert_eq!(kids.last(), Some(&gps), "under the elevation profile");
        assert!(kids.iter().any(|k| w.get::<ProfilePanel>(*k).is_some()));
    }

    #[test]
    fn gps_line_hemispheres_and_rounding() {
        assert_eq!(
            super::gps_line(47.123454, 9.532104, 812.4),
            "47.12345° N  9.53210° E  812 m"
        );
        assert_eq!(
            super::gps_line(-33.856785, -151.215296, -3.6),
            "33.85679° S  151.21530° W  -4 m"
        );
        // Rounds to zero: no southern / western hemisphere.
        assert_eq!(
            super::gps_line(-0.000001, -0.000004, 0.0),
            "0.00000° N  0.00000° E  0 m"
        );
    }

    #[test]
    fn version_label_in_the_lower_right_corner() {
        use super::*;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(VersionLabel("0.1.0-abc1234".into()));
        add_version_label(&mut app);
        app.update();
        let mut q = app
            .world_mut()
            .query_filtered::<(&Text, &Node), With<VersionText>>();
        let (text, node) = q.single(app.world()).unwrap();
        assert_eq!(text.0, "0.1.0-abc1234");
        assert_eq!(node.position_type, PositionType::Absolute);
        assert!(node.bottom != Val::Auto && node.right != Val::Auto);
        assert!(node.top == Val::Auto && node.left == Val::Auto);
        // A later change reaches the text.
        app.world_mut().resource_mut::<VersionLabel>().0 = "0.2.0".into();
        app.update();
        let mut q = app.world_mut().query_filtered::<&Text, With<VersionText>>();
        assert_eq!(q.single(app.world()).unwrap().0, "0.2.0");
    }
}
