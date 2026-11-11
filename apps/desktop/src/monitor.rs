//! `--monitor <name|index|WIDTHxHEIGHT>`: borderless fullscreen on a chosen
//! screen (test/kiosk machines with several monitors). All detected
//! monitors are logged at startup, so the right value is easy to find.

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PrimaryWindow, WindowMode};

use crate::Args;

pub struct MonitorPlugin;

impl Plugin for MonitorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MonitorChoice>()
            .add_systems(Update, (log_monitors, place_on_monitor).chain());
    }
}

#[derive(Resource, Default)]
struct MonitorChoice {
    done: bool,
    frames_waited: u32,
}

/// One detected monitor, in detection order (= winit's index).
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Picks a monitor for `spec`: an index (`1`), a resolution (`2560x1600`),
/// or a name, matched case-insensitively as a substring (`DP`, `hdmi`).
pub fn pick_monitor(spec: &str, monitors: &[MonitorInfo]) -> Option<usize> {
    let spec = spec.trim();
    if let Ok(i) = spec.parse::<usize>() {
        return (i < monitors.len()).then_some(i);
    }
    if let Some((w, h)) = spec.split_once('x')
        && let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
    {
        return monitors.iter().position(|m| m.width == w && m.height == h);
    }
    let needle = spec.to_ascii_lowercase();
    // Exact name first, then substring.
    monitors
        .iter()
        .position(|m| m.name.to_ascii_lowercase() == needle)
        .or_else(|| {
            monitors
                .iter()
                .position(|m| m.name.to_ascii_lowercase().contains(&needle))
        })
}

fn infos(monitors: &[(Entity, &Monitor)]) -> Vec<MonitorInfo> {
    monitors
        .iter()
        .map(|(_, m)| MonitorInfo {
            name: m.name.clone().unwrap_or_default(),
            width: m.physical_width,
            height: m.physical_height,
        })
        .collect()
}

fn log_monitors(added: Query<&Monitor, Added<Monitor>>) {
    for m in &added {
        info!(
            name = m.name.as_deref().unwrap_or("?"),
            width = m.physical_width,
            height = m.physical_height,
            x = m.physical_position.x,
            y = m.physical_position.y,
            "monitor"
        );
    }
}

/// Once the monitors are known, switch the window to fullscreen on the
/// chosen one.
fn place_on_monitor(
    args: Res<Args>,
    mut choice: ResMut<MonitorChoice>,
    monitors: Query<(Entity, &Monitor)>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some(spec) = &args.monitor else { return };
    if choice.done {
        return;
    }
    let mut list: Vec<(Entity, &Monitor)> = monitors.iter().collect();
    if list.is_empty() {
        choice.frames_waited += 1;
        if choice.frames_waited > 300 {
            warn!("no monitors detected, staying windowed");
            choice.done = true;
        }
        return;
    }
    // Detection order: sort by position, left to right, then top to bottom.
    list.sort_by_key(|(_, m)| (m.physical_position.x, m.physical_position.y));
    let Ok(mut window) = window.single_mut() else {
        return;
    };
    choice.done = true;
    match pick_monitor(spec, &infos(&list)) {
        Some(i) => {
            let (entity, m) = list[i];
            info!(
                monitor = m.name.as_deref().unwrap_or("?"),
                "fullscreen on chosen monitor"
            );
            window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Entity(entity));
        }
        None => warn!(
            spec = %spec,
            available = ?infos(&list),
            "--monitor matches no monitor, staying windowed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                name: "DisplayPort-0".into(),
                width: 2560,
                height: 1600,
            },
            MonitorInfo {
                name: "HDMI-A-0".into(),
                width: 3840,
                height: 1080,
            },
        ]
    }

    #[test]
    fn picks_by_index_resolution_or_name() {
        let m = monitors();
        assert_eq!(pick_monitor("0", &m), Some(0));
        assert_eq!(pick_monitor("1", &m), Some(1));
        assert_eq!(pick_monitor("2", &m), None);
        assert_eq!(pick_monitor("3840x1080", &m), Some(1));
        assert_eq!(pick_monitor("1920x1080", &m), None);
        assert_eq!(pick_monitor("hdmi", &m), Some(1));
        assert_eq!(pick_monitor("DisplayPort", &m), Some(0));
        assert_eq!(pick_monitor(" displayport-0 ", &m), Some(0));
        assert_eq!(pick_monitor("VGA", &m), None);
    }
}
