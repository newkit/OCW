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

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use ocw_core::{RiderId, mps_to_kmh};
use ocw_world::{AVERAGE_AHEAD_M, ElevationProfile, GradeBand, ProfileSpec, RidePath, Track};

use crate::RideView;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        app.init_resource::<ShowFps>()
            .init_resource::<ShowProfile>()
            .init_resource::<LookAhead>()
            .add_systems(Startup, (spawn_hud, spawn_recording))
            .add_systems(
                Update,
                (
                    update_look_ahead,
                    (update_hud, update_profile),
                    update_gps,
                    update_recording,
                )
                    .chain(),
            );
        add_version_label(app);
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

/// Whether the FPS counter is shown (console command `fps`).
#[derive(Resource, Default)]
pub struct ShowFps(pub bool);

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
struct ProfilePanel;

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
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(12),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: px(6),
            ..default()
        })
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
                // up); Bevy wraps text that is still too long.
                max_width: Val::Vw(94.0),
                padding: UiRect::all(px(10)),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_children(|hud| {
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: 26.0.into(),
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
) -> String {
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
    format!("{power_w:>4.0} W   {speed_kmh:>5.1} km/h   {cadence}   {grade:+.1} %{ahead}{heart}")
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

/// Top right: a small red dot + "REC m:ss" while a video is recorded, and
/// the app message (e.g. "Video saved: ...") below it.
fn spawn_recording(mut commands: Commands) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: px(12),
            right: px(12),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            row_gap: px(4),
            ..default()
        })
        .with_children(|top| {
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
        });
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

/// The GPS line: "47.12345 N  9.53210 E  812 m" (5 decimals ≈ 1 m; no
/// degree sign: Bevy's default font has no glyph for it).
pub fn gps_line(lat: f64, lon: f64, altitude_m: f64) -> String {
    fn coord(v: f64, pos: char, neg: char) -> String {
        // Round first, so a value that rounds to zero shows no "S" / "W".
        let r = (v * 1e5).round() / 1e5;
        let h = if r < 0.0 { neg } else { pos };
        format!("{:.5} {h}", r.abs())
    }
    format!(
        "{}  {}  {:.0} m",
        coord(lat, 'N', 'S'),
        coord(lon, 'E', 'W'),
        altitude_m
    )
}

/// Shows the GPS line while the position is known (not on test routes).
fn update_gps(
    view: Res<RideView>,
    mut boxes: Query<&mut Node, With<HudGps>>,
    mut texts: Query<&mut Text, With<HudGpsText>>,
) {
    let (Ok(mut node), Ok(mut text)) = (boxes.single_mut(), texts.single_mut()) else {
        return;
    };
    let display = if view.position.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if let Some((lat, lon, alt)) = view.position {
        let line = gps_line(lat, lon, alt);
        if text.0 != line {
            text.0 = line;
        }
    }
}

/// Font size of the small HUD lines; their glyphs (Bevy's monospaced
/// default font) are about 0.6 of it wide.
const SMALL_FONT: f32 = 15.0;
const SMALL_CHAR_W: f32 = SMALL_FONT * 0.6;

/// Breaks each line of `text` that is longer than `max_chars` at its
/// " | " separators (then at ", "), greedily, so the HUD stays inside the
/// window; a single piece longer than `max_chars` stays whole (Bevy wraps
/// it at word boundaries). Continuation lines start with no separator.
pub fn wrap_hud_lines(text: &str, max_chars: usize) -> String {
    fn wrap(line: &str, sep: &str, max: usize, out: &mut Vec<String>) {
        let mut cur = String::new();
        for piece in line.split(sep) {
            let len = |s: &str| s.chars().count();
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
    let max = max_chars.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        if line.chars().count() <= max {
            out.push(line.to_string());
            continue;
        }
        let mut parts = Vec::new();
        wrap(line, " | ", max, &mut parts);
        for part in parts {
            if part.chars().count() > max {
                wrap(&part, ", ", max, &mut out);
            } else {
                out.push(part);
            }
        }
    }
    out.join("\n")
}

/// Characters of the small HUD font that fit in a window `width` logical
/// pixels wide (panel margins and padding taken off).
fn small_chars_for(width: f32) -> usize {
    ((width * 0.94 - 2.0 * 10.0 - 4.0) / SMALL_CHAR_W).max(10.0) as usize
}

fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn update_hud(
    view: Res<RideView>,
    ahead: Res<LookAhead>,
    show_fps: Res<ShowFps>,
    diagnostics: Res<DiagnosticsStore>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut big: Query<&mut Text, (With<HudText>, Without<HudSmallText>)>,
    mut small: Query<&mut Text, (With<HudSmallText>, Without<HudText>)>,
) {
    let (Ok(mut big), Ok(mut small)) = (big.single_mut(), small.single_mut()) else {
        return;
    };
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
    );
    if big.0 != line {
        big.0 = line;
    }
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
    let mut lines = format!(
        "{place} | {:.2} km | {} | riders: {}\n{keys} | mouse: camera, C: camera reset, R: restart, V: video | F11: screen | ` or F12: console",
        local.state.distance / 1000.0,
        format_time(view.elapsed_s),
        view.riders.len()
    );
    if !view.status.is_empty() {
        lines.push('\n');
        lines.push_str(&view.status);
    }
    if show_fps.0
        && let Some(fps) = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
    {
        lines.push_str(&format!("\nFPS: {fps:.0}"));
    }
    let width = windows.single().map_or(1280.0, |w| w.width());
    let lines = wrap_hud_lines(&lines, small_chars_for(width));
    if small.0 != lines {
        small.0 = lines;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

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
    fn time_format() {
        assert_eq!(super::format_time(3725.0), "1:02:05");
    }

    #[test]
    fn main_line_shows_the_gradient() {
        assert_eq!(
            super::main_line(330.0, 10.66, Some(86.0), 11.43, None, None),
            " 330 W    10.7 km/h   86 rpm   +11.4 %"
        );
        assert_eq!(
            super::main_line(0.0, 0.0, None, -0.02, Some(0.01), Some(142.0)),
            "   0 W     0.0 km/h   - rpm   +0.0 % (100 m +0.0 %)   HR 142"
        );
        assert!(super::main_line(200.0, 35.0, None, -8.25, None, None).ends_with("-8.2 %"));
        // The average over the next 100 m in brackets.
        assert!(
            super::main_line(250.0, 14.0, Some(80.0), 9.4, Some(7.81), None)
                .ends_with("+9.4 % (100 m +7.8 %)")
        );
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
        // Line breaks of the input are kept; an unbreakable piece stays whole.
        assert_eq!(w("x\nabcdefghij", 4), "x\nabcdefghij");
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

    #[test]
    fn hud_width_in_characters() {
        // 800x600: about 80 characters of the small font.
        assert!((75..=85).contains(&super::small_chars_for(800.0)));
        assert!(super::small_chars_for(2560.0) > 250);
        assert_eq!(super::small_chars_for(10.0), 10);
    }

    #[test]
    fn gps_line_hemispheres_and_rounding() {
        assert_eq!(
            super::gps_line(47.123454, 9.532104, 812.4),
            "47.12345 N  9.53210 E  812 m"
        );
        assert_eq!(
            super::gps_line(-33.856785, -151.215296, -3.6),
            "33.85679 S  151.21530 W  -4 m"
        );
        // Rounds to zero: no southern / western hemisphere.
        assert_eq!(
            super::gps_line(-0.000001, -0.000004, 0.0),
            "0.00000 N  0.00000 E  0 m"
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
