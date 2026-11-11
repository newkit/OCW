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
    let grade = view.route.sample(local.state.distance).grade * 100.0;
    let cadence = local
        .cadence_rpm
        .map(|c| format!("{c:.0} rpm"))
        .unwrap_or_else(|| "- rpm".into());
    let heart = local
        .heart_rate_bpm
        .map(|h| format!("   HR {h:.0}"))
        .unwrap_or_default();
    big.0 = format!(
        "{:>4.0} W   {:>5.1} km/h   {}{}",
        local.power_w,
        mps_to_kmh(local.state.speed),
        cadence,
        heart
    );
    let mut lines = format!(
        "{} | {:.2} km | {:+.1} % | {} | riders: {}\nPgUp/PgDn: virtual power +/-10 W | ` or F12: console",
        view.route.name(),
        local.state.distance / 1000.0,
        grade,
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
}
