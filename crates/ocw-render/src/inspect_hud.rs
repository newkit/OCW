//! God mode's inspection aids on screen (spec §14 Phase 13D): the
//! inspection panel (I: position in package metres, latitude / longitude,
//! tile, local time), the crosshair in the middle with what it points at
//! below it, the pins (P: glowing beams with their number, kept for the
//! ride) and the touch controls of the flight (spec §3.2.2: the climb and
//! descend buttons at the right edge).
//!
//! The app computes the texts ([`crate::FlightView::panel`],
//! [`crate::FlightView::crosshair`]) and the pins ([`RideView::pins`]);
//! this module only lays them out. The touch buttons' layout
//! ([`touch_buttons`]) is pure, so the app's touch mapping hits the same
//! circles that are drawn.

use std::collections::BTreeMap;

use bevy::prelude::*;

use crate::camera::FollowCamera;
use crate::{RideView, to_vec3};

pub struct InspectHudPlugin;

impl Plugin for InspectHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PinEntities>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (update_panel, update_touch, pins));
    }
}

/// Height of a pin's beam (m).
pub const PIN_BEAM_M: f32 = 60.0;
/// The pins' colour (a bright magenta, apart from the landing ring's gold).
const PIN_COLOUR: Color = Color::srgb(1.0, 0.25, 0.75);

/// A touch button of the flight (spec §3.2.2, §14 Phase 13B).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchButton {
    /// Centre and radius (logical px from the window's top left).
    pub centre: Vec2,
    pub radius: f32,
    /// Climbs (+) or descends (−).
    pub climb: bool,
}

impl TouchButton {
    pub fn contains(&self, p: Vec2) -> bool {
        p.distance(self.centre) <= self.radius
    }
}

/// The smallest touch target (logical px, spec §3.2.2: ≥ 48 px).
pub const TOUCH_TARGET_PX: f32 = 48.0;

/// The climb (+, upper) and descend (−, lower) buttons at the right edge
/// of a window `size` (logical px), vertically centred: at least 72 px
/// wide (and 9 % of the height on big screens), 16 px from the edge.
pub fn touch_buttons(size: Vec2) -> [TouchButton; 2] {
    let radius = (size.y * 0.045).max(TOUCH_TARGET_PX * 0.75);
    let x = size.x - 16.0 - radius;
    let gap = 12.0;
    [
        TouchButton {
            centre: Vec2::new(x, size.y / 2.0 - radius - gap / 2.0),
            radius,
            climb: true,
        },
        TouchButton {
            centre: Vec2::new(x, size.y / 2.0 + radius + gap / 2.0),
            radius,
            climb: false,
        },
    ]
}

#[derive(Component)]
struct PanelRoot;

#[derive(Component)]
struct PanelText;

#[derive(Component)]
struct CrosshairRoot;

#[derive(Component)]
struct CrosshairLabel;

#[derive(Component)]
struct TouchRoot;

/// One touch button (its index in [`touch_buttons`]).
#[derive(Component)]
struct TouchButtonNode(usize);

fn spawn(mut commands: Commands) {
    // The panel: top right, under the REC indicator.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(56),
                right: px(12),
                padding: UiRect::all(px(8)),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            PanelRoot,
            crate::hud::HudPanel,
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font_size: 15.0.into(),
                ..default()
            },
            TextColor(Color::srgb(0.92, 0.92, 0.92)),
            PanelText,
        ));
    // The crosshair: a small cross in the middle, the label under it.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
            CrosshairRoot,
            crate::hud::HudPanel,
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            let bar = |w: f32, h: f32| {
                (
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(50.0),
                        top: Val::Percent(50.0),
                        width: px(w),
                        height: px(h),
                        margin: UiRect {
                            left: px(-w / 2.0),
                            top: px(-h / 2.0),
                            ..default()
                        },
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    Outline::new(px(1), Val::ZERO, Color::srgba(0.0, 0.0, 0.0, 0.6)),
                )
            };
            c.spawn(bar(22.0, 2.0));
            c.spawn(bar(2.0, 22.0));
            // The label right of the cross (below it the unicorn's head
            // would cover it in the flight view).
            c.spawn(Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                margin: UiRect {
                    left: px(22),
                    top: px(-14),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        padding: UiRect::axes(px(8), px(4)),
                        border_radius: BorderRadius::all(px(4)),
                        max_width: Val::Vw(45.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                ))
                .with_child((
                    Text::new(""),
                    TextFont {
                        font_size: 16.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::justify(Justify::Left),
                    CrosshairLabel,
                ));
            });
        });
    // The touch buttons (placed each frame for the window size).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
            TouchRoot,
            crate::hud::HudPanel,
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            for (i, label) in ["+", "\u{2212}"].into_iter().enumerate() {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::MAX,
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.4)),
                    BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.7)),
                    TouchButtonNode(i),
                ))
                .with_child((
                    Text::new(label),
                    TextFont {
                        font_size: 34.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            }
        });
}

#[allow(clippy::type_complexity)] // Bevy queries
fn update_panel(
    view: Res<RideView>,
    mut panel: Query<&mut Node, (With<PanelRoot>, Without<CrosshairRoot>)>,
    mut panel_text: Query<&mut Text, (With<PanelText>, Without<CrosshairLabel>)>,
    mut cross: Query<&mut Node, (With<CrosshairRoot>, Without<PanelRoot>)>,
    mut label: Query<&mut Text, (With<CrosshairLabel>, Without<PanelText>)>,
) {
    let f = view.flight.as_ref();
    let lines = f.map(|f| f.panel.join("\n")).unwrap_or_default();
    let show = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut panel {
        let d = show(!lines.is_empty());
        if n.display != d {
            n.display = d;
        }
    }
    for mut t in &mut panel_text {
        if t.0 != lines {
            t.0.clone_from(&lines);
        }
    }
    let cross_lines = f.and_then(|f| f.crosshair.as_ref());
    for mut n in &mut cross {
        let d = show(cross_lines.is_some());
        if n.display != d {
            n.display = d;
        }
    }
    let text = cross_lines.map(|l| l.join("\n")).unwrap_or_default();
    for mut t in &mut label {
        if t.0 != text {
            t.0.clone_from(&text);
        }
    }
}

#[allow(clippy::type_complexity)] // Bevy queries
fn update_touch(
    view: Res<RideView>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut root: Query<&mut Node, (With<TouchRoot>, Without<TouchButtonNode>)>,
    mut buttons: Query<(&mut Node, &TouchButtonNode), Without<TouchRoot>>,
) {
    let on = view.flight.as_ref().is_some_and(|f| f.touch);
    for mut n in &mut root {
        let d = if on { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
    if !on {
        return;
    }
    let size = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let layout = touch_buttons(size);
    for (mut n, b) in &mut buttons {
        let t = layout[b.0];
        n.left = px(t.centre.x - t.radius);
        n.top = px(t.centre.y - t.radius);
        n.width = px(2.0 * t.radius);
        n.height = px(2.0 * t.radius);
    }
}

/// The pins' entities: the beam and the number tag of each.
#[derive(Resource, Default)]
struct PinEntities {
    pins: BTreeMap<u32, (Entity, Entity)>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

#[derive(Component)]
struct PinBeam;

#[derive(Component)]
struct PinTag;

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn pins(
    mut commands: Commands,
    view: Res<RideView>,
    mut ents: ResMut<PinEntities>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    mut beams: Query<&mut Transform, With<PinBeam>>,
    mut tags: Query<(&mut Node, &mut Visibility), With<PinTag>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let ents = &mut *ents;
    let wanted: Vec<u32> = view.pins.iter().map(|p| p.number).collect();
    ents.pins.retain(|n, (beam, tag)| {
        let keep = wanted.contains(n);
        if !keep {
            commands.entity(*beam).despawn();
            commands.entity(*tag).despawn();
        }
        keep
    });
    let cam = cameras.iter().next();
    for p in &view.pins {
        let foot = to_vec3(p.pos);
        let (beam, tag) = *ents.pins.entry(p.number).or_insert_with(|| {
            let mesh = ents
                .mesh
                .get_or_insert_with(|| {
                    meshes.add(Cylinder::new(0.6, PIN_BEAM_M).mesh().resolution(10))
                })
                .clone();
            let material = ents
                .material
                .get_or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: PIN_COLOUR,
                        emissive: LinearRgba::rgb(5.0, 1.0, 3.2),
                        unlit: true,
                        ..default()
                    })
                })
                .clone();
            let beam = commands
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_translation(foot + Vec3::Y * PIN_BEAM_M / 2.0),
                    bevy::light::NotShadowCaster,
                    PinBeam,
                ))
                .id();
            let tag = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        padding: UiRect::axes(px(6), px(2)),
                        border_radius: BorderRadius::all(px(4)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.45, 0.0, 0.3, 0.7)),
                    Text::new(format!("Pin {}", p.number)),
                    TextFont {
                        font_size: 15.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Visibility::Hidden,
                    PinTag,
                ))
                .id();
            (beam, tag)
        });
        if let Ok(mut t) = beams.get_mut(beam) {
            t.translation = foot + Vec3::Y * PIN_BEAM_M / 2.0;
        }
        if let Ok((mut node, mut vis)) = tags.get_mut(tag) {
            let top = foot + Vec3::Y * (PIN_BEAM_M + 2.0);
            let shown = cam.and_then(|(c, g)| {
                (g.translation().distance(top) < 20_000.0)
                    .then(|| c.world_to_viewport(g, top).ok())
                    .flatten()
            });
            match shown {
                Some(s) => {
                    node.left = px(s.x - 22.0);
                    node.top = px(s.y - 24.0);
                    *vis = Visibility::Visible;
                }
                None => *vis = Visibility::Hidden,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_touch_buttons_are_big_enough_and_apart() {
        for size in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(800.0, 480.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            let [up, down] = touch_buttons(size);
            assert!(up.climb && !down.climb);
            assert!(2.0 * up.radius >= TOUCH_TARGET_PX, "{size}");
            // Inside the window, at the right edge, the climb button above.
            assert!(up.centre.x + up.radius <= size.x - 15.0);
            assert!(up.centre.x > size.x * 0.8);
            assert!(up.centre.y < down.centre.y);
            // 2 mm (≥ 8 px) apart.
            assert!(down.centre.y - up.centre.y - 2.0 * up.radius >= 8.0);
            assert!(up.contains(up.centre) && !up.contains(down.centre));
        }
    }
}
