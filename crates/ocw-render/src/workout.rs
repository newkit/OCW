//! The workout panel of the HUD (spec §2.8, §3.2): target, time left,
//! next step, and the whole workout's profile with the progress. All
//! content comes from [`RideView::workout`] (computed in
//! `ocw_sim::workout::WorkoutPanel`); this only draws it.

use bevy::prelude::*;
use ocw_sim::workout::{ProfileBlock, Zone};

use crate::RideView;

pub struct WorkoutPanelPlugin;

impl Plugin for WorkoutPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_panel)
            .add_systems(Update, update_panel);
    }
}

/// Size of the profile chart (logical pixels).
const PROFILE_W: f32 = 300.0;
const PROFILE_H: f32 = 56.0;

#[derive(Component)]
struct Panel;

#[derive(Component)]
struct Title;

#[derive(Component)]
struct TargetText;

#[derive(Component)]
struct Lines;

#[derive(Component)]
struct Profile;

#[derive(Component)]
struct Block;

/// The ridden part of the profile (darkened).
#[derive(Component)]
struct Done;

#[derive(Component)]
struct Marker;

/// Colour of a training zone (profile blocks and the target).
pub fn zone_color(zone: Zone) -> Color {
    match zone {
        Zone::Free => Color::srgb(0.55, 0.55, 0.55),
        Zone::Recovery => Color::srgb(0.62, 0.65, 0.70),
        Zone::Endurance => Color::srgb(0.25, 0.55, 0.95),
        Zone::Tempo => Color::srgb(0.25, 0.78, 0.40),
        Zone::Threshold => Color::srgb(0.98, 0.80, 0.20),
        Zone::Vo2Max => Color::srgb(0.98, 0.52, 0.15),
        Zone::Anaerobic => Color::srgb(0.92, 0.22, 0.20),
    }
}

fn spawn_panel(mut commands: Commands) {
    let text = |size: f32, color: Color| {
        (
            Text::new(""),
            TextFont {
                font_size: size.into(),
                ..default()
            },
            TextColor(color),
        )
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(24),
                right: px(12),
                padding: UiRect::all(px(10)),
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            Panel,
        ))
        .with_children(|p| {
            p.spawn((text(14.0, Color::srgb(0.85, 0.85, 0.85)), Title));
            p.spawn((text(30.0, Color::WHITE), TargetText));
            p.spawn((text(14.0, Color::srgb(0.85, 0.85, 0.85)), Lines));
            p.spawn((
                Node {
                    width: px(PROFILE_W),
                    height: px(PROFILE_H),
                    margin: UiRect::top(px(4)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                Profile,
            ))
            .with_children(|prof| {
                prof.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(0),
                        top: px(0),
                        height: Val::Percent(100.0),
                        width: Val::Percent(0.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                    ZIndex(1),
                    Done,
                ));
                prof.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(0.0),
                        top: px(0),
                        height: Val::Percent(100.0),
                        width: px(2),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                    ZIndex(2),
                    Marker,
                ));
            });
        });
}

fn block_node(b: &ProfileBlock) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(b.start * 100.0),
            width: Val::Percent(b.width * 100.0),
            bottom: px(0),
            height: Val::Percent(b.height * 100.0),
            ..default()
        },
        BackgroundColor(zone_color(b.zone)),
        Block,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_panel(
    mut commands: Commands,
    view: Res<RideView>,
    mut shown: Local<Vec<ProfileBlock>>,
    mut panel: Query<&mut Node, (With<Panel>, Without<Done>, Without<Marker>)>,
    mut title: Query<&mut Text, (With<Title>, Without<TargetText>, Without<Lines>)>,
    mut target: Query<
        (&mut Text, &mut TextColor),
        (With<TargetText>, Without<Title>, Without<Lines>),
    >,
    mut lines: Query<&mut Text, (With<Lines>, Without<Title>, Without<TargetText>)>,
    profile: Query<Entity, With<Profile>>,
    blocks: Query<Entity, With<Block>>,
    mut done: Query<&mut Node, (With<Done>, Without<Panel>, Without<Marker>)>,
    mut marker: Query<&mut Node, (With<Marker>, Without<Panel>, Without<Done>)>,
) {
    if !view.is_changed() {
        return;
    }
    let Ok(mut node) = panel.single_mut() else {
        return;
    };
    let Some(w) = &view.workout else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    if let Ok(mut t) = title.single_mut()
        && t.0 != w.title
    {
        t.0.clone_from(&w.title);
    }
    if let Ok((mut t, mut c)) = target.single_mut() {
        if t.0 != w.target {
            t.0.clone_from(&w.target);
        }
        let color = TextColor(match w.zone {
            // Grey would be hard to read: white without a target.
            Zone::Free | Zone::Recovery => Color::WHITE,
            z => zone_color(z),
        });
        if *c != color {
            *c = color;
        }
    }
    let text = w.lines.join("\n");
    if let Ok(mut t) = lines.single_mut()
        && t.0 != text
    {
        t.0 = text;
    }
    let progress = Val::Percent(w.progress * 100.0);
    if let Ok(mut d) = done.single_mut()
        && d.width != progress
    {
        d.width = progress;
    }
    if let Ok(mut m) = marker.single_mut()
        && m.left != progress
    {
        m.left = progress;
    }
    if *shown != w.blocks
        && let Ok(parent) = profile.single()
    {
        for e in &blocks {
            commands.entity(e).despawn();
        }
        commands.entity(parent).with_children(|p| {
            for b in &w.blocks {
                p.spawn(block_node(b));
            }
        });
        shown.clone_from(&w.blocks);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ocw_sim::workout::{WorkoutPanel, WorkoutRun, parse_toml};
    use ocw_world::Route;

    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(WorkoutPanelPlugin);
        app
    }

    fn panel() -> WorkoutPanel {
        let w = parse_toml(
            "name = \"P\"\n[[steps]]\ntype = \"steady\"\nduration = 60\npower = 100\n[[steps]]\ntype = \"free\"\nduration = 60\n",
        )
        .unwrap();
        let mut run = WorkoutRun::new(Arc::new(w));
        for _ in 0..30 {
            run.advance(1.0, 200.0);
        }
        WorkoutPanel::new(&run, 250.0)
    }

    #[test]
    fn panel_shows_only_with_a_workout() {
        let mut app = app();
        app.update();
        let display = |app: &mut App| {
            let mut q = app.world_mut().query_filtered::<&Node, With<Panel>>();
            q.single(app.world()).unwrap().display
        };
        assert_eq!(display(&mut app), Display::None);
        app.world_mut().resource_mut::<RideView>().workout = Some(panel());
        app.update();
        assert_eq!(display(&mut app), Display::Flex);
        let mut q = app.world_mut().query_filtered::<&Text, With<TargetText>>();
        assert_eq!(q.single(app.world()).unwrap().0, "250 W");
        let mut q = app.world_mut().query_filtered::<&Text, With<Lines>>();
        assert!(
            q.single(app.world())
                .unwrap()
                .0
                .starts_with("step 1/2: 0:30 left")
        );
        // One block per step; the marker at a quarter.
        let mut q = app.world_mut().query_filtered::<(), With<Block>>();
        assert_eq!(q.iter(app.world()).count(), 2);
        let mut q = app.world_mut().query_filtered::<&Node, With<Marker>>();
        assert_eq!(q.single(app.world()).unwrap().left, Val::Percent(25.0));
        // Unchanged blocks are not rebuilt; gone again without a workout.
        app.world_mut().resource_mut::<RideView>().set_changed();
        app.update();
        let mut q = app.world_mut().query_filtered::<(), With<Block>>();
        assert_eq!(q.iter(app.world()).count(), 2);
        app.world_mut().resource_mut::<RideView>().workout = None;
        app.update();
        assert_eq!(display(&mut app), Display::None);
    }

    #[test]
    fn zone_colours_differ() {
        let zones = [
            Zone::Free,
            Zone::Recovery,
            Zone::Endurance,
            Zone::Tempo,
            Zone::Threshold,
            Zone::Vo2Max,
            Zone::Anaerobic,
        ];
        for (i, a) in zones.iter().enumerate() {
            for b in &zones[i + 1..] {
                assert_ne!(zone_color(*a), zone_color(*b));
            }
        }
    }
}
