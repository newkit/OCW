//! The *Sharing* screen (spec 14 Phase 10, 10D.2 *Built (local bridge)*):
//! while the desktop shares its trainer with a browser, the window shows
//! this simple screen instead of its own ride – the paired browser, the
//! live power / cadence / heart rate, the grade the browser sets, the
//! bridge's state and round trip, the headset's last report, the pairing
//! code and *Stop sharing*.
//!
//! As the menu draws the [`crate::menu::MenuView`], this module only draws
//! the [`SharingView`] resource the app writes, in the menu's look (its
//! panel, rows and text sizes). While it is open:
//!
//! - the world's camera ([`FollowCamera`]) is off – no 3D frame is drawn –
//!   and a UI camera of its own ([`SharingCamera`], the default UI camera)
//!   draws the screen (the menu and the console still open over it);
//! - the HUD is off ([`ShowHud`]); both come back as they were when it
//!   closes;
//! - a click on *Stop sharing* is reported in [`SharingPointer`]; the keys
//!   and the gamepad are the app's (the [`InputContext::Sharing`]
//!   context).
//!
//! [`InputContext::Sharing`]: crate::menu::InputContext::Sharing

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::picking::events::PointerClick;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;

use crate::camera::FollowCamera;
use crate::hud::ShowHud;
use crate::menu::{
    DIM, FOCUS, HINT_VH, ITEM_VH, PANEL, PANEL_BORDER_VH, PANEL_GAP_VH, PANEL_MAX_VW,
    PANEL_PAD_X_VH, PANEL_PAD_Y_VH, PANEL_WIDTH_VH, ROW, ROW_BORDER_VH, ROW_FOCUSED, ROW_PAD_X_VH,
    ROW_PAD_Y_VH, TEXT, TEXT_VH, TITLE_VH, font,
};

/// What the Sharing screen shows; written by the app.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct SharingView {
    /// The screen is up (a browser is paired).
    pub open: bool,
    /// "Sharing".
    pub title: String,
    /// "Your trainer rides in Quest Browser".
    pub subtitle: String,
    /// The live numbers, large: power, cadence, heart rate.
    pub tiles: Vec<SharingTile>,
    /// "Label" – "value" lines: the grade, the bridge, the round trip, the
    /// headset's report, the code.
    pub rows: Vec<(String, String)>,
    /// The paragraph under the rows (the own ride is paused).
    pub note: String,
    /// The button ("Stop sharing").
    pub button: String,
    /// The keys ("Enter / A: stop sharing · Esc: menu").
    pub hint: String,
}

/// One live number of the screen.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SharingTile {
    /// "Power".
    pub label: String,
    /// "182" ("–" while there is none).
    pub value: String,
    /// "W".
    pub unit: String,
}

/// *Stop sharing* was clicked (the app takes it and sets it back).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SharingPointer(pub bool);

/// The screen's UI camera (only while it is open).
#[derive(Component)]
pub struct SharingCamera;

/// The screen's root node.
#[derive(Component)]
pub struct SharingRoot;

/// The *Stop sharing* button (tests: a click on it).
#[derive(Component)]
pub struct SharingButton;

/// After the world's camera (0) and the workshop's (1), before the safe
/// area's UI camera (10).
pub const SHARING_CAMERA_ORDER: isize = 5;

/// The screen's ground (the menu panel's colour, opaque: nothing of the
/// ride shows through).
const BACKDROP: Color = Color::srgb(0.035, 0.045, 0.07);
/// The live numbers (vh).
const TILE_VALUE_VH: f32 = 10.0;
const TILE_LABEL_VH: f32 = 3.0;

/// The world's cameras (the screen turns them off).
type WorldCams<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Camera), (With<FollowCamera>, Without<SharingCamera>)>;

/// What the screen changed while it is open, to give back.
#[derive(Resource, Default)]
struct Opened {
    camera: Option<Entity>,
    root: Option<Entity>,
    /// The world's cameras it turned off.
    cameras_off: Vec<Entity>,
    /// The HUD's setting before.
    hud_was: Option<bool>,
    /// The view last drawn.
    drawn: Option<SharingView>,
}

pub struct SharingPlugin;

impl Plugin for SharingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SharingView>()
            .init_resource::<SharingPointer>()
            .init_resource::<Opened>()
            .add_systems(PostUpdate, (open_close, draw).chain());
    }
}

/// Opens or closes the screen: the cameras, the HUD, the root node.
fn open_close(
    mut commands: Commands,
    view: Res<SharingView>,
    mut opened: ResMut<Opened>,
    mut world_cams: WorldCams,
    mut hud: Option<ResMut<ShowHud>>,
) {
    let is_open = opened.root.is_some();
    if view.open == is_open {
        // A world camera that came on meanwhile (another screen closed)
        // stays off while the screen shows.
        if is_open {
            for (e, mut cam) in &mut world_cams {
                if cam.is_active {
                    cam.is_active = false;
                    if !opened.cameras_off.contains(&e) {
                        opened.cameras_off.push(e);
                    }
                }
            }
        }
        return;
    }
    if view.open {
        for (e, mut cam) in &mut world_cams {
            if cam.is_active {
                cam.is_active = false;
                opened.cameras_off.push(e);
            }
        }
        if let Some(h) = hud.as_mut() {
            opened.hud_was = Some(h.0);
            h.0 = false;
        }
        opened.camera = Some(
            commands
                .spawn((
                    Camera2d,
                    Camera {
                        order: SHARING_CAMERA_ORDER,
                        clear_color: ClearColorConfig::Custom(BACKDROP),
                        ..default()
                    },
                    Tonemapping::None,
                    bevy::ui::IsDefaultUiCamera,
                    SharingCamera,
                    Name::new("Sharing screen camera"),
                ))
                .id(),
        );
        opened.root = Some(spawn_root(&mut commands));
        opened.drawn = None;
        info!("sharing screen: shown, the world is not drawn meanwhile");
    } else {
        for e in [opened.root.take(), opened.camera.take()]
            .into_iter()
            .flatten()
        {
            commands.entity(e).despawn();
        }
        let off = std::mem::take(&mut opened.cameras_off);
        for (e, mut cam) in &mut world_cams {
            if off.contains(&e) {
                cam.is_active = true;
            }
        }
        if let (Some(h), Some(was)) = (hud.as_mut(), opened.hud_was.take()) {
            h.0 = was;
        }
        opened.drawn = None;
        info!("sharing screen: closed, back to the ride");
    }
}

fn spawn_root(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BACKDROP),
            // Over the HUD and the ride's overlays, under the menu (100).
            GlobalZIndex(90),
            SharingRoot,
            crate::access::AccessExempt,
            Name::new("Sharing screen"),
        ))
        .id()
}

/// Draws the view into the root when it changed (a few times a second:
/// the readings come at 4 Hz).
fn draw(mut commands: Commands, view: Res<SharingView>, mut opened: ResMut<Opened>) {
    let Some(root) = opened.root else { return };
    if opened.drawn.as_ref() == Some(&*view) {
        return;
    }
    opened.drawn = Some(view.clone());
    commands.entity(root).despawn_children();
    commands.entity(root).with_children(|r| {
        r.spawn((
            Node {
                width: vh(PANEL_WIDTH_VH),
                max_width: vw(PANEL_MAX_VW),
                padding: UiRect::axes(vh(PANEL_PAD_X_VH), vh(PANEL_PAD_Y_VH)),
                flex_direction: FlexDirection::Column,
                row_gap: vh(PANEL_GAP_VH),
                border: UiRect::all(vh(PANEL_BORDER_VH)),
                border_radius: BorderRadius::all(vh(1.5)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(view.title.clone()),
                font(TITLE_VH),
                TextColor(TEXT),
            ));
            if !view.subtitle.is_empty() {
                p.spawn((
                    Text::new(view.subtitle.clone()),
                    font(ITEM_VH),
                    TextColor(FOCUS),
                ));
            }
            // The live numbers.
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: vh(1.5),
                margin: UiRect::vertical(vh(0.8)),
                ..default()
            })
            .with_children(|row| {
                for t in &view.tiles {
                    row.spawn((
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::ZERO,
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(vh(1.0), vh(1.0)),
                            border_radius: BorderRadius::all(vh(1.0)),
                            ..default()
                        },
                        BackgroundColor(ROW),
                    ))
                    .with_children(|tile| {
                        tile.spawn((
                            Text::new(t.label.clone()),
                            font(TILE_LABEL_VH),
                            TextColor(DIM),
                        ));
                        tile.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Baseline,
                            column_gap: vh(0.8),
                            ..default()
                        })
                        .with_children(|v| {
                            v.spawn((
                                Text::new(t.value.clone()),
                                font(TILE_VALUE_VH),
                                TextColor(TEXT),
                            ));
                            v.spawn((
                                Text::new(t.unit.clone()),
                                font(TILE_LABEL_VH),
                                TextColor(DIM),
                            ));
                        });
                    });
                }
            });
            // The label – value lines.
            for (label, value) in &view.rows {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: vh(2.0),
                    ..default()
                })
                .with_children(|l| {
                    l.spawn((Text::new(label.clone()), font(TEXT_VH), TextColor(DIM)));
                    l.spawn((
                        Text::new(value.clone()),
                        font(TEXT_VH),
                        TextColor(TEXT),
                        TextLayout::justify(Justify::Right),
                    ));
                });
            }
            if !view.note.is_empty() {
                p.spawn((Text::new(view.note.clone()), font(TEXT_VH), TextColor(DIM)));
            }
            // Stop sharing: a focused menu row.
            p.spawn((
                Node {
                    padding: UiRect::axes(vh(ROW_PAD_X_VH), vh(ROW_PAD_Y_VH)),
                    border: UiRect::all(vh(ROW_BORDER_VH)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    margin: UiRect::top(vh(0.8)),
                    ..default()
                },
                BackgroundColor(ROW_FOCUSED),
                BorderColor::all(FOCUS),
                SharingButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(view.button.clone()),
                    font(ITEM_VH),
                    TextColor(TEXT),
                ));
            })
            .observe(
                |click: On<PointerClick>, mut pointer: ResMut<SharingPointer>| {
                    if click.button == PointerButton::Primary {
                        pointer.0 = true;
                    }
                },
            );
            p.spawn((Text::new(view.hint.clone()), font(HINT_VH), TextColor(DIM)));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ShowHud>()
            .add_plugins(SharingPlugin);
        app.world_mut().spawn((Camera::default(), FollowCamera));
        app
    }

    fn world_cam_on(app: &mut App) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&Camera, With<FollowCamera>>();
        q.single(app.world()).unwrap().is_active
    }

    fn count<C: Component>(app: &mut App) -> usize {
        let mut q = app.world_mut().query_filtered::<(), With<C>>();
        q.iter(app.world()).count()
    }

    /// Open: the world's camera off, a UI camera of its own, the HUD off,
    /// the content drawn; closed: everything as it was.
    #[test]
    fn the_screen_replaces_the_ride_and_gives_it_back() {
        let mut app = app();
        app.update();
        assert!(world_cam_on(&mut app));
        assert_eq!(count::<SharingRoot>(&mut app), 0);
        {
            let mut v = app.world_mut().resource_mut::<SharingView>();
            v.open = true;
            v.title = "Sharing".into();
            v.button = "Stop sharing".into();
            v.tiles = vec![SharingTile {
                label: "Power".into(),
                value: "182".into(),
                unit: "W".into(),
            }];
            v.rows = vec![("Grade set by the browser".into(), "4.5 %".into())];
        }
        app.update();
        assert!(!world_cam_on(&mut app), "no 3D frame while sharing");
        assert_eq!(count::<SharingCamera>(&mut app), 1);
        assert_eq!(count::<SharingRoot>(&mut app), 1);
        assert_eq!(count::<SharingButton>(&mut app), 1);
        assert!(!app.world().resource::<ShowHud>().0);
        let texts: Vec<String> = {
            let mut q = app.world_mut().query::<&Text>();
            q.iter(app.world()).map(|t| t.0.clone()).collect()
        };
        for want in ["Sharing", "182", "W", "Power", "4.5 %", "Stop sharing"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
        // A new reading redraws; the same view does not.
        app.world_mut().resource_mut::<SharingView>().tiles[0].value = "190".into();
        app.update();
        let mut q = app.world_mut().query::<&Text>();
        assert!(q.iter(app.world()).any(|t| t.0 == "190"));
        app.world_mut().resource_mut::<SharingView>().open = false;
        app.update();
        assert!(world_cam_on(&mut app));
        assert_eq!(count::<SharingCamera>(&mut app), 0);
        assert_eq!(count::<SharingRoot>(&mut app), 0);
        assert!(app.world().resource::<ShowHud>().0, "the HUD comes back");
    }

    /// A HUD that was off stays off afterwards; a world camera that was off
    /// before (a capture's) is not turned on.
    #[test]
    fn closing_restores_only_what_it_changed() {
        let mut app = app();
        app.world_mut().resource_mut::<ShowHud>().0 = false;
        let other = app
            .world_mut()
            .spawn((
                Camera {
                    is_active: false,
                    ..default()
                },
                FollowCamera,
            ))
            .id();
        app.world_mut().resource_mut::<SharingView>().open = true;
        app.update();
        app.world_mut().resource_mut::<SharingView>().open = false;
        app.update();
        assert!(!app.world().resource::<ShowHud>().0);
        assert!(!app.world().get::<Camera>(other).unwrap().is_active);
    }
}
