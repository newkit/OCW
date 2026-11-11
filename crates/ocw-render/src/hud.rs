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
            .add_systems(Update, update_hud);
    }
}

/// Whether the FPS counter is shown (console command `fps`).
#[derive(Resource, Default)]
pub struct ShowFps(pub bool);

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct HudSmallText;

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
    small.0 = lines;
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
}
