//! In-game menu overlay (spec §3.2.1) and the input context.
//!
//! The menu's model (pages, focus, page stack) lives in the app; this module
//! only draws the [`MenuView`] resource, as the HUD draws `RideView`, and
//! reports mouse hover / clicks as [`MenuPointer`] events. Ten-foot theme:
//! text sizes are a share of the window height, high contrast, a clearly
//! visible focus frame.
//!
//! [`InputContext`] says who gets the keyboard, mouse and gamepad this
//! frame: the ride, the menu or the console. Ride input systems run only in
//! [`InputContext::Ride`] (see [`ride_input`]).

use bevy::picking::events::{PointerClick, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::text::FontSize;

use crate::console::ConsoleState;

/// Who receives the input this frame (spec §3.2.1).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum InputContext {
    /// Ride inputs (power, steering, camera, R, V, …) are active.
    #[default]
    Ride,
    /// The menu is open: arrows, Enter, the gamepad's buttons belong to it.
    Menu,
    /// The console is open (over the ride or over the menu): it gets the keys.
    Console,
}

impl InputContext {
    /// The context for these overlay states: the console wins over the menu.
    pub fn of(console_open: bool, menu_open: bool) -> Self {
        if console_open {
            Self::Console
        } else if menu_open {
            Self::Menu
        } else {
            Self::Ride
        }
    }
}

/// Run condition for ride input systems: only in [`InputContext::Ride`]
/// (also when there is no context at all, e.g. in a test without the menu).
pub fn ride_input(context: Option<Res<InputContext>>) -> bool {
    context.is_none_or(|c| *c == InputContext::Ride)
}

/// What the menu shows; written by the app's menu model.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct MenuView {
    pub open: bool,
    pub title: String,
    /// Status strip under the title (top page: power, speed, trainer).
    pub status: String,
    /// Description lines (placeholder pages, confirmation questions).
    pub text: Vec<String>,
    pub items: Vec<String>,
    pub focused: usize,
    /// Control hint at the bottom.
    pub hint: String,
}

/// Mouse events on the menu since the app last read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPointerEvent {
    /// The pointer moved onto item `i`.
    Hover(usize),
    /// Item `i` was clicked (primary button).
    Click(usize),
    /// Right click anywhere on the menu: back.
    Back,
}

#[derive(Resource, Debug, Default)]
pub struct MenuPointer(pub Vec<MenuPointerEvent>);

/// Input context and menu resources, without drawing (headless tests).
pub struct MenuCorePlugin;

impl Plugin for MenuCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputContext>()
            .init_resource::<MenuView>()
            .init_resource::<MenuPointer>()
            .add_systems(PreUpdate, update_input_context);
    }
}

/// Decided once per frame, before any Update system: a key that closes the
/// menu (Enter, B) is not seen by the ride in the same frame.
fn update_input_context(
    console: Option<Res<ConsoleState>>,
    view: Res<MenuView>,
    mut context: ResMut<InputContext>,
) {
    let c = InputContext::of(console.is_some_and(|c| c.is_open()), view.open);
    context.set_if_neq(c);
}

/// Draws the menu (Bevy UI).
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuCorePlugin>() {
            app.add_plugins(MenuCorePlugin);
        }
        app.add_systems(Startup, spawn_menu)
            .add_systems(Update, draw_menu);
    }
}

/// Text sizes in % of the window height (spec: at least 3 %).
const TITLE_VH: f32 = 6.0;
const ITEM_VH: f32 = 4.2;
const TEXT_VH: f32 = 3.2;
const HINT_VH: f32 = 3.0;

const PANEL: Color = Color::srgba(0.02, 0.03, 0.05, 0.88);
const ROW: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
const ROW_FOCUSED: Color = Color::srgba(1.0, 0.78, 0.24, 0.22);
const FOCUS: Color = Color::srgb(1.0, 0.78, 0.24);
const TEXT: Color = Color::WHITE;
const DIM: Color = Color::srgb(0.78, 0.8, 0.84);

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct MenuTitle;
#[derive(Component)]
struct MenuStatus;
#[derive(Component)]
struct MenuText;
#[derive(Component)]
struct MenuList;
#[derive(Component)]
struct MenuHint;
#[derive(Component)]
struct MenuRow(usize);

fn font(vh: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    }
}

fn spawn_menu(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            // Above the HUD and the junction overlay.
            GlobalZIndex(100),
            MenuRoot,
        ))
        .observe(|click: On<PointerClick>, mut pointer: ResMut<MenuPointer>| {
            if click.button == PointerButton::Secondary {
                pointer.0.push(MenuPointerEvent::Back);
            }
        })
        .with_children(|root| {
            root.spawn((
                Node {
                    width: vh(90),
                    max_width: vw(94),
                    max_height: vh(94),
                    padding: UiRect::axes(vh(4), vh(3)),
                    flex_direction: FlexDirection::Column,
                    row_gap: vh(1.2),
                    border: UiRect::all(vh(0.3)),
                    border_radius: BorderRadius::all(vh(1.5)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
            ))
            .with_children(|panel| {
                panel.spawn((Text::new(""), font(TITLE_VH), TextColor(TEXT), MenuTitle));
                panel.spawn((Text::new(""), font(TEXT_VH), TextColor(FOCUS), MenuStatus));
                panel.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: vh(0.6),
                        ..default()
                    },
                    Text::new(""),
                    font(TEXT_VH),
                    TextColor(DIM),
                    MenuText,
                ));
                panel.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: vh(1.0),
                        margin: UiRect::vertical(vh(1.0)),
                        ..default()
                    },
                    MenuList,
                ));
                panel.spawn((Text::new(""), font(HINT_VH), TextColor(DIM), MenuHint));
            });
        });
}

fn spawn_rows(commands: &mut Commands, list: Entity, items: &[String]) {
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        for (i, label) in items.iter().enumerate() {
            l.spawn((
                Node {
                    padding: UiRect::axes(vh(2.5), vh(1.0)),
                    border: UiRect::all(vh(0.45)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    ..default()
                },
                BackgroundColor(ROW),
                BorderColor::all(Color::NONE),
                MenuRow(i),
                children![(Text::new(label.clone()), font(ITEM_VH), TextColor(TEXT))],
            ))
            .observe(move |_: On<PointerOver>, mut pointer: ResMut<MenuPointer>| {
                pointer.0.push(MenuPointerEvent::Hover(i));
            })
            .observe(move |click: On<PointerClick>, mut pointer: ResMut<MenuPointer>| {
                if click.button == PointerButton::Primary {
                    pointer.0.push(MenuPointerEvent::Click(i));
                }
            });
        }
    });
}

/// The page's layout key: rows are respawned only when it changes, not on
/// every focus move or status update.
type PageKey = (String, Vec<String>);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn draw_menu(
    mut commands: Commands,
    view: Res<MenuView>,
    mut page: Local<Option<PageKey>>,
    mut root: Query<&mut Node, With<MenuRoot>>,
    list: Query<Entity, With<MenuList>>,
    mut rows: Query<(&MenuRow, &mut BackgroundColor, &mut BorderColor)>,
    mut texts: ParamSet<(
        Query<&mut Text, With<MenuTitle>>,
        Query<&mut Text, With<MenuStatus>>,
        Query<&mut Text, With<MenuText>>,
        Query<&mut Text, With<MenuHint>>,
    )>,
) {
    if !view.is_changed() {
        return;
    }
    let Ok(mut node) = root.single_mut() else {
        return;
    };
    let display = if view.open {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if !view.open {
        *page = None;
        return;
    }
    let key = (view.title.clone(), view.items.clone());
    if page.as_ref() != Some(&key) {
        if let Ok(list) = list.single() {
            spawn_rows(&mut commands, list, &view.items);
        }
        *page = Some(key);
    }
    for (row, mut bg, mut border) in &mut rows {
        let focused = row.0 == view.focused;
        bg.0 = if focused { ROW_FOCUSED } else { ROW };
        *border = BorderColor::all(if focused { FOCUS } else { Color::NONE });
    }
    set_text(&mut texts.p0(), &view.title);
    set_text(&mut texts.p1(), &view.status);
    set_text(&mut texts.p2(), &view.text.join("\n"));
    set_text(&mut texts.p3(), &view.hint);
}

fn set_text(q: &mut Query<&mut Text, impl bevy::ecs::query::QueryFilter>, s: &str) {
    for mut t in q.iter_mut() {
        if t.0 != s {
            s.clone_into(&mut t.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_takes_the_input_before_the_menu() {
        assert_eq!(InputContext::of(false, false), InputContext::Ride);
        assert_eq!(InputContext::of(false, true), InputContext::Menu);
        assert_eq!(InputContext::of(true, false), InputContext::Console);
        assert_eq!(InputContext::of(true, true), InputContext::Console);
    }

    #[test]
    fn ride_input_only_in_the_ride_context() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::console::ConsoleCorePlugin, MenuCorePlugin));
        #[derive(Resource, Default)]
        struct Runs(u32);
        app.init_resource::<Runs>().add_systems(
            Update,
            (|mut r: ResMut<Runs>| r.0 += 1).run_if(ride_input),
        );
        app.update();
        assert_eq!(app.world().resource::<Runs>().0, 1);
        app.world_mut().resource_mut::<MenuView>().open = true;
        app.update();
        assert_eq!(
            *app.world().resource::<InputContext>(),
            InputContext::Menu
        );
        assert_eq!(app.world().resource::<Runs>().0, 1);
        app.world_mut().resource_mut::<ConsoleState>().set_open(true);
        app.update();
        assert_eq!(
            *app.world().resource::<InputContext>(),
            InputContext::Console
        );
        app.world_mut().resource_mut::<MenuView>().open = false;
        app.world_mut().resource_mut::<ConsoleState>().set_open(false);
        app.update();
        assert_eq!(app.world().resource::<Runs>().0, 2);
    }
}
