//! The voice indicator of the HUD (spec §2.4, §12 "visible microphone
//! activity"): while the rider is in (or joining) a voice call, a small
//! panel at the bottom left (above the profile) shows the microphone (on / muted), the speaker
//! (on / muted), the connection status and who is talking; riders who talk
//! get a marker above their avatar. All content comes from [`VoiceHud`],
//! written by the app; this only draws it.

use bevy::prelude::*;
use ocw_core::RiderId;

use crate::riders::Avatar;

/// What the voice indicator shows.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct VoiceHud {
    /// Show the indicator (in a call, joining, or failed and retrying).
    pub visible: bool,
    pub mic_muted: bool,
    pub speaker_muted: bool,
    /// "connected (3)", "connecting", "reconnecting", "failed …".
    pub status: String,
    /// The local rider is talking (the microphone line lights up).
    pub self_speaking: bool,
    /// Riders talking now (markers above their avatars).
    pub speaking: Vec<RiderId>,
    /// Their names, for the panel.
    pub speaking_names: Vec<String>,
}

/// The panel's text lines (pure, tested).
pub fn lines(v: &VoiceHud) -> (String, String) {
    let mic = if v.mic_muted {
        "MIC muted (M)"
    } else if v.self_speaking {
        "MIC on - talking"
    } else {
        "MIC on"
    };
    let spk = if v.speaker_muted {
        "SPEAKER muted (Shift+M)"
    } else {
        "SPEAKER on"
    };
    let mut second = format!("voice {}", v.status);
    if !v.speaking_names.is_empty() {
        second.push_str(&format!(" - talking: {}", v.speaking_names.join(", ")));
    }
    (format!("{mic}   {spk}"), second)
}

pub struct VoiceHudPlugin;

impl Plugin for VoiceHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceHud>()
            .add_systems(Startup, spawn_panel)
            .add_systems(Update, (update_panel, speaking_markers));
    }
}

#[derive(Component)]
struct Panel;

#[derive(Component)]
struct MicLine;

#[derive(Component)]
struct StatusLine;

/// A marker above a talking rider's avatar.
#[derive(Component)]
struct SpeakingMarker(RiderId);

const MUTED: Color = Color::srgb(1.0, 0.35, 0.3);
const LIVE: Color = Color::srgb(0.45, 1.0, 0.55);

fn spawn_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(120),
                left: px(12),
                max_width: px(420),
                padding: UiRect::axes(px(10), px(6)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: px(2),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Panel,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: 15.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                MicLine,
            ));
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0.into(),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                StatusLine,
            ));
        });
}

#[allow(clippy::type_complexity)] // a Bevy system
fn update_panel(
    hud: Res<VoiceHud>,
    mut panel: Query<&mut Node, With<Panel>>,
    mut mic: Query<(&mut Text, &mut TextColor), (With<MicLine>, Without<StatusLine>)>,
    mut status: Query<&mut Text, (With<StatusLine>, Without<MicLine>)>,
) {
    if !hud.is_changed() {
        return;
    }
    for mut n in &mut panel {
        n.display = if hud.visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    let (first, second) = lines(&hud);
    for (mut t, mut c) in &mut mic {
        t.0 = first.clone();
        c.0 = if hud.mic_muted {
            MUTED
        } else if hud.self_speaking {
            LIVE
        } else {
            Color::WHITE
        };
    }
    for mut t in &mut status {
        t.0 = second.clone();
    }
}

/// Adds a small glowing marker above the avatars of talking riders and
/// removes it when they stop.
fn speaking_markers(
    mut commands: Commands,
    hud: Res<VoiceHud>,
    avatars: Query<(Entity, &Avatar)>,
    markers: Query<(Entity, &SpeakingMarker)>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if !hud.is_changed() {
        return;
    }
    let talking = |id: RiderId| hud.visible && hud.speaking.contains(&id);
    let mut have = Vec::new();
    for (e, m) in &markers {
        if talking(m.0) {
            have.push(m.0);
        } else {
            commands.entity(e).despawn();
        }
    }
    let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) else {
        return;
    };
    for (e, a) in &avatars {
        if a.is_local || !talking(a.id) || have.contains(&a.id) {
            continue;
        }
        let mesh = meshes.add(Sphere::new(0.12));
        let material = materials.add(StandardMaterial {
            base_color: LIVE,
            emissive: LinearRgba::rgb(0.6, 2.0, 0.8),
            unlit: true,
            ..default()
        });
        let marker = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 2.15, 0.0),
                SpeakingMarker(a.id),
            ))
            .id();
        commands.entity(e).add_child(marker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_lines() {
        let mut v = VoiceHud {
            visible: true,
            status: "connected (2)".into(),
            ..Default::default()
        };
        assert_eq!(
            lines(&v),
            ("MIC on   SPEAKER on".into(), "voice connected (2)".into())
        );
        v.mic_muted = true;
        v.speaker_muted = true;
        v.speaking_names = vec!["Rider 2".into()];
        let (a, b) = lines(&v);
        assert_eq!(a, "MIC muted (M)   SPEAKER muted (Shift+M)");
        assert_eq!(b, "voice connected (2) - talking: Rider 2");
        v.mic_muted = false;
        v.self_speaking = true;
        assert!(lines(&v).0.starts_with("MIC on - talking"));
    }
}
