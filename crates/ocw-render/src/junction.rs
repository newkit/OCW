//! Junction overlay in free ride (spec §2.2.1): a fan of arrows at the
//! lower centre of the screen, one per exit at its real turn angle; the
//! selected one highlighted (solid green once locked), with the distance
//! to the junction, the street name and icons for dead end, unpaved and
//! steep climb.

use bevy::prelude::*;
use bevy::text::Justify;

use crate::{JunctionView, RideView};

pub struct JunctionPlugin;

impl Plugin for JunctionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_overlay)
            .add_systems(Update, update_overlay);
    }
}

/// Overlay size and the arrows' pivot (bottom centre) in pixels.
const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 160.0;
const PIVOT_Y: f32 = 108.0;
const ARROW_LEN: f32 = 82.0;

#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct Fan;

#[derive(Component)]
struct Label;

#[derive(Component)]
struct NoticeText;

fn spawn_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(6),
                left: percent(50),
                margin: UiRect::left(px(-WIDTH / 2.0)),
                width: px(WIDTH),
                height: px(HEIGHT),
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.3)),
            Visibility::Hidden,
            Overlay,
        ))
        .with_children(|o| {
            o.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(WIDTH),
                    height: px(HEIGHT),
                    ..default()
                },
                Fan,
            ));
            o.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    width: px(WIDTH),
                    bottom: px(4),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                children![(
                    Text::new(""),
                    TextFont {
                        font_size: 16.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::justify(Justify::Center),
                    Label,
                )],
            ));
        });
    // Notices ("Dead end in 800 m", "U-turn") below the HUD panel.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(150),
            left: px(0),
            right: px(0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            Text::new(""),
            TextFont {
                font_size: 22.0.into(),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.85, 0.3)),
            TextShadow::default(),
            NoticeText,
        )],
    ));
}

/// The line under the arrows: distance, street of the selected exit, icons.
pub fn label(j: &JunctionView) -> String {
    let e = &j.exits[j.selected];
    let mut s = if j.distance_m > 0.5 {
        format!("{:.0} m  {}", j.distance_m, e.name)
    } else {
        e.name.clone()
    };
    for (on, icon) in [
        (e.dead_end, "dead end"),
        (e.unpaved, "unpaved"),
        (e.steep, "steep"),
    ] {
        if on {
            s.push_str(&format!("  [{icon}]"));
        }
    }
    s
}

/// Colour and width of exit `i`'s arrow: selected (green when locked,
/// yellow before), others grey; dead ends reddish.
pub fn arrow_style(j: &JunctionView, i: usize) -> (Color, f32) {
    let e = &j.exits[i];
    if i == j.selected {
        let c = if j.locked {
            Color::srgb(0.2, 0.95, 0.35)
        } else {
            Color::srgb(1.0, 0.9, 0.25)
        };
        (c, 14.0)
    } else if e.dead_end {
        (Color::srgba(0.95, 0.4, 0.35, 0.8), 9.0)
    } else {
        (Color::srgba(0.85, 0.85, 0.85, 0.7), 9.0)
    }
}

/// One arrow of the fan: (angle in degrees, selected, locked, dead end).
type FanArrow = (i32, bool, bool, bool);

/// What the fan shows (to respawn the arrows only when it changes).
fn fan_key(j: &JunctionView) -> Vec<FanArrow> {
    j.exits
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                e.angle_deg.round() as i32,
                i == j.selected,
                j.locked,
                e.dead_end,
            )
        })
        .collect()
}

fn update_overlay(
    mut commands: Commands,
    view: Res<RideView>,
    mut overlay: Query<&mut Visibility, With<Overlay>>,
    fan: Query<Entity, With<Fan>>,
    mut label_text: Query<&mut Text, (With<Label>, Without<NoticeText>)>,
    mut notice: Query<&mut Text, (With<NoticeText>, Without<Label>)>,
    mut shown: Local<Option<Vec<FanArrow>>>,
) {
    if let Ok(mut t) = notice.single_mut() {
        let want = view.notice.clone().unwrap_or_default();
        if t.0 != want {
            t.0 = want;
        }
    }
    let (Ok(mut vis), Ok(fan)) = (overlay.single_mut(), fan.single()) else {
        return;
    };
    let Some(j) = &view.junction else {
        *vis = Visibility::Hidden;
        *shown = None;
        return;
    };
    *vis = Visibility::Inherited;
    if let Ok(mut t) = label_text.single_mut() {
        t.0 = label(j);
    }
    let key = fan_key(j);
    if shown.as_ref() == Some(&key) {
        return;
    }
    *shown = Some(key);
    commands.entity(fan).despawn_children();
    let centre = WIDTH / 2.0;
    commands.entity(fan).with_children(|f| {
        // The rider's own way in: a short stub below the pivot.
        f.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(centre - 5.0),
                top: px(PIVOT_Y),
                width: px(10),
                height: px(20),
                ..default()
            },
            BackgroundColor(Color::srgba(0.85, 0.85, 0.85, 0.7)),
        ));
        for (i, e) in j.exits.iter().enumerate() {
            let (colour, w) = arrow_style(j, i);
            let l = ARROW_LEN;
            f.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(centre - l),
                    top: px(PIVOT_Y - l),
                    width: px(2.0 * l),
                    height: px(2.0 * l),
                    ..default()
                },
                // Clockwise = to the right, like the turn angle.
                UiTransform::from_rotation(Rot2::degrees(e.angle_deg as f32)),
                // The selected arrow on top.
                ZIndex(i32::from(i == j.selected)),
            ))
            .with_children(|a| {
                a.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(l - w / 2.0),
                        top: px(22),
                        width: px(w),
                        height: px(l - 22.0),
                        ..default()
                    },
                    BackgroundColor(colour),
                ));
                a.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(l - w * 1.2),
                        top: px(12),
                        width: px(w * 2.4),
                        height: px(w * 2.4),
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::degrees(45.0)),
                    BackgroundColor(colour),
                ));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExitView;

    fn view() -> JunctionView {
        let exit = |angle: f64, name: &str| ExitView {
            angle_deg: angle,
            name: name.into(),
            dead_end: false,
            unpaved: false,
            steep: false,
        };
        JunctionView {
            distance_m: 84.6,
            exits: vec![
                exit(-90.0, "Feldweg"),
                ExitView {
                    dead_end: true,
                    steep: true,
                    ..exit(2.0, "Talstrasse")
                },
                exit(95.0, "Landstrasse"),
            ],
            selected: 1,
            default: 2,
            locked: false,
        }
    }

    #[test]
    fn label_shows_distance_street_and_icons() {
        let mut j = view();
        assert_eq!(label(&j), "85 m  Talstrasse  [dead end]  [steep]");
        j.selected = 2;
        j.distance_m = 0.0;
        assert_eq!(label(&j), "Landstrasse");
    }

    #[test]
    fn selected_arrow_is_highlighted_and_turns_green_when_locked() {
        let mut j = view();
        let (sel, w) = arrow_style(&j, 1);
        let (other, w2) = arrow_style(&j, 0);
        assert!(w > w2);
        assert_ne!(sel, other);
        j.locked = true;
        assert_ne!(arrow_style(&j, 1).0, sel);
        // Dead ends that are not selected are reddish.
        j.selected = 0;
        let (dead, _) = arrow_style(&j, 1);
        assert!(dead.to_srgba().red > dead.to_srgba().green);
        assert_ne!(fan_key(&j), fan_key(&view()));
    }
}
