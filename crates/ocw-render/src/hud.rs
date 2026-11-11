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
            .add_systems(Startup, (spawn_hud, spawn_recording))
            .add_systems(Update, (update_hud, update_recording));
    }
}

/// Whether the FPS counter is shown (console command `fps`).
#[derive(Resource, Default)]
pub struct ShowFps(pub bool);

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

const REC_RED: Color = Color::srgb(0.95, 0.15, 0.12);

fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
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
                    font_size: 15.0.into(),
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
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
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
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
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

fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn update_hud(
    view: Res<RideView>,
    show_fps: Res<ShowFps>,
    diagnostics: Res<DiagnosticsStore>,
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
    let (place, keys) = if view.free_ride {
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
    small.0 = lines;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    #[test]
    fn time_format() {
        assert_eq!(super::format_time(3725.0), "1:02:05");
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
}
