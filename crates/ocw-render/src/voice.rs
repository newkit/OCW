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
    /// The echo test (spec §7 *Echo bot*), while it is on.
    pub echo: Option<EchoHud>,
}

/// The echo test's line: a hint and two level meters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EchoHud {
    /// Near the Echo rider: "speak now".
    pub near: bool,
    /// "echo test - speak now", "Echo rider 120 m away", …
    pub text: String,
    /// The microphone and the echo as played (0 … 1, linear RMS).
    pub input: f32,
    pub echo: f32,
}

/// How full a level meter is drawn (0 … 1): −60 dBFS empty, 0 dBFS full.
pub fn meter_fraction(level: f32) -> f32 {
    if level <= 0.0 || !level.is_finite() {
        return 0.0;
    }
    ((20.0 * level.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
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

#[derive(Component)]
struct EchoLine;

#[derive(Component)]
struct EchoRow;

/// The fill of a level meter: the input (false) or the echo (true).
#[derive(Component)]
struct MeterFill(bool);

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
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    display: Display::None,
                    ..default()
                },
                EchoRow,
            ))
            .with_children(|e| {
                e.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 14.0.into(),
                        ..default()
                    },
                    TextColor(LIVE),
                    EchoLine,
                ));
                for (label, echo) in [("mic ", false), ("echo", true)] {
                    e.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: 12.0.into(),
                                ..default()
                            },
                            TextColor(Color::srgb(0.85, 0.85, 0.85)),
                        ));
                        row.spawn((
                            Node {
                                width: px(140),
                                height: px(8),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                Node {
                                    width: Val::Percent(0.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(if echo {
                                    Color::srgb(0.4, 0.75, 1.0)
                                } else {
                                    LIVE
                                }),
                                MeterFill(echo),
                            ));
                        });
                    });
                }
            });
        });
}

#[allow(clippy::type_complexity)] // a Bevy system
fn update_panel(
    hud: Res<VoiceHud>,
    mut panel: Query<&mut Node, (With<Panel>, Without<EchoRow>, Without<MeterFill>)>,
    mut mic: Query<
        (&mut Text, &mut TextColor),
        (With<MicLine>, Without<StatusLine>, Without<EchoLine>),
    >,
    mut status: Query<&mut Text, (With<StatusLine>, Without<MicLine>, Without<EchoLine>)>,
    mut echo_text: Query<
        (&mut Text, &mut TextColor),
        (With<EchoLine>, Without<MicLine>, Without<StatusLine>),
    >,
    mut echo_row: Query<&mut Node, (With<EchoRow>, Without<Panel>, Without<MeterFill>)>,
    mut fills: Query<(&mut Node, &MeterFill), (Without<Panel>, Without<EchoRow>)>,
) {
    if !hud.is_changed() {
        return;
    }
    for mut n in &mut panel {
        n.display = if hud.visible || hud.echo.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut n in &mut echo_row {
        n.display = if hud.echo.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Some(e) = &hud.echo {
        for (mut t, mut c) in &mut echo_text {
            t.0 = e.text.clone();
            c.0 = if e.near {
                LIVE
            } else {
                Color::srgb(0.85, 0.85, 0.85)
            };
        }
        for (mut n, m) in &mut fills {
            let level = if m.0 { e.echo } else { e.input };
            n.width = Val::Percent(100.0 * meter_fraction(level));
        }
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
    fn meters_in_decibels() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_fraction(f32::NAN), 0.0);
        assert_eq!(meter_fraction(1.0), 1.0);
        assert!((meter_fraction(0.001) - 0.0).abs() < 1e-6, "-60 dB");
        assert!((meter_fraction(0.0316) - 0.5).abs() < 0.01, "-30 dB");
        assert!(meter_fraction(0.1) > meter_fraction(0.01));
    }

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
