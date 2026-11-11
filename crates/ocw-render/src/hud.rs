//! In-ride heads-up display (Bevy UI, spec §3.2).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use ocw_core::mps_to_kmh;

use crate::RideView;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        app.init_resource::<ShowFps>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, (update_hud, update_gps));
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

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct HudSmallText;

/// The GPS line, bottom-left.
#[derive(Component)]
struct HudGps;

#[derive(Component)]
struct HudGpsText;

fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(12),
                left: px(12),
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

/// The large HUD line: power, speed, cadence, road gradient, heart rate.
pub fn main_line(
    power_w: f64,
    speed_kmh: f64,
    cadence_rpm: Option<f64>,
    grade_percent: f64,
    heart_rate_bpm: Option<f64>,
) -> String {
    let cadence = cadence_rpm.map_or_else(|| "- rpm".into(), |c| format!("{c:.0} rpm"));
    // Avoid "-0.0 %" on the flat.
    let grade = if grade_percent.abs() < 0.05 {
        0.0
    } else {
        grade_percent
    };
    let heart = heart_rate_bpm
        .map(|h| format!("   HR {h:.0}"))
        .unwrap_or_default();
    format!("{power_w:>4.0} W   {speed_kmh:>5.1} km/h   {cadence}   {grade:+.1} %{heart}")
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
    big.0 = main_line(
        local.power_w,
        mps_to_kmh(local.state.speed),
        local.cadence_rpm,
        grade,
        local.heart_rate_bpm,
    );
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
    #[test]
    fn time_format() {
        assert_eq!(super::format_time(3725.0), "1:02:05");
    }

    #[test]
    fn main_line_shows_the_gradient() {
        assert_eq!(
            super::main_line(330.0, 10.66, Some(86.0), 11.43, None),
            " 330 W    10.7 km/h   86 rpm   +11.4 %"
        );
        assert_eq!(
            super::main_line(0.0, 0.0, None, -0.02, Some(142.0)),
            "   0 W     0.0 km/h   - rpm   +0.0 %   HR 142"
        );
        assert!(super::main_line(200.0, 35.0, None, -8.25, None).ends_with("-8.2 %"));
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
