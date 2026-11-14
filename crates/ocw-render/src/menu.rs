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
    /// The workshop scene is open (spec §14 Phase 4, 4A): its own input.
    Workshop,
}

impl InputContext {
    /// The context for these overlay states: the console wins over the menu.
    pub fn of(console_open: bool, menu_open: bool) -> Self {
        Self::with_workshop(console_open, menu_open, false)
    }

    /// [`InputContext::of`] with the workshop scene: the console wins,
    /// then the menu, then the workshop.
    pub fn with_workshop(console_open: bool, menu_open: bool, workshop_open: bool) -> Self {
        if console_open {
            Self::Console
        } else if menu_open {
            Self::Menu
        } else if workshop_open {
            Self::Workshop
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
    /// Description paragraphs (placeholder pages, confirmation questions).
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
    workshop: Option<Res<crate::workshop::WorkshopView>>,
    mut context: ResMut<InputContext>,
) {
    let c = InputContext::with_workshop(
        console.is_some_and(|c| c.is_open()),
        view.open,
        workshop.is_some_and(|w| w.open),
    );
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

/// Panel layout in % of the window height (vh). [`page_height_vh`] adds
/// these up; `spawn_menu` / `spawn_rows` use the same numbers.
const PANEL_WIDTH_VH: f32 = 110.0;
/// … but at most this share of the window width (narrow windows).
const PANEL_MAX_VW: f32 = 94.0;
/// The panel's height budget: the page must fit into it.
pub const PANEL_MAX_VH: f32 = 94.0;
const PANEL_PAD_X_VH: f32 = 4.0;
const PANEL_PAD_Y_VH: f32 = 2.5;
const PANEL_BORDER_VH: f32 = 0.3;
const PANEL_GAP_VH: f32 = 1.0;
const LIST_GAP_VH: f32 = 0.8;
const LIST_MARGIN_VH: f32 = 0.6;
const ROW_PAD_X_VH: f32 = 2.5;
const ROW_PAD_Y_VH: f32 = 0.7;
const ROW_BORDER_VH: f32 = 0.45;
/// Bevy's default line height, in font sizes.
const LINE_HEIGHT: f32 = 1.2;
/// Advance of every glyph of the UI font (Fira Mono: 600 / 1000 em).
const ADVANCE_EM: f32 = 0.6;

/// Lines `text` takes in a box `width_vh` wide at font size `font_vh`:
/// each paragraph (`\n`) wrapped at word boundaries, as the text layout
/// does with the monospaced UI font; an empty text is one line.
pub fn wrapped_lines(text: &str, width_vh: f32, font_vh: f32) -> usize {
    let cols = ((width_vh / (ADVANCE_EM * font_vh)).floor() as usize).max(1);
    text.split('\n')
        .map(|para| {
            let (mut lines, mut used) = (1, 0usize);
            for word in para.split_whitespace() {
                let mut len = word.chars().count();
                let need = if used == 0 { len } else { used + 1 + len };
                if need <= cols {
                    used = need;
                    continue;
                }
                if used > 0 {
                    lines += 1;
                }
                // A word longer than a line is broken.
                while len > cols {
                    lines += 1;
                    len -= cols;
                }
                used = len;
            }
            lines
        })
        .sum()
}

/// Width of the menu panel's content (vh) in a window of `aspect`
/// (width / height).
pub fn content_width_vh(aspect: f32) -> f32 {
    PANEL_WIDTH_VH.min(PANEL_MAX_VW * aspect) - 2.0 * (PANEL_PAD_X_VH + PANEL_BORDER_VH)
}

/// Height of the menu panel for `view` (vh) in a window of `aspect`,
/// showing the first `paragraphs` paragraphs of its text.
pub fn page_height_vh(view: &MenuView, aspect: f32, paragraphs: usize) -> f32 {
    let w = content_width_vh(aspect);
    // Empty status and text lines are hidden (no room, no gap).
    let lines = |s: &str, font: f32| {
        if s.is_empty() {
            -PANEL_GAP_VH
        } else {
            wrapped_lines(s, w, font) as f32 * font * LINE_HEIGHT
        }
    };
    let text = view.text[..paragraphs.min(view.text.len())].join("\n");
    let row_w = w - 2.0 * (ROW_PAD_X_VH + ROW_BORDER_VH);
    let rows: f32 = view
        .items
        .iter()
        .map(|label| {
            wrapped_lines(label, row_w, ITEM_VH) as f32 * ITEM_VH * LINE_HEIGHT
                + 2.0 * (ROW_PAD_Y_VH + ROW_BORDER_VH)
        })
        .sum();
    let list =
        rows + LIST_GAP_VH * view.items.len().saturating_sub(1) as f32 + 2.0 * LIST_MARGIN_VH;
    2.0 * (PANEL_PAD_Y_VH + PANEL_BORDER_VH)
        + 4.0 * PANEL_GAP_VH
        + lines(&view.title, TITLE_VH)
        + lines(&view.status, TEXT_VH)
        + lines(&text, TEXT_VH)
        + list
        + lines(&view.hint, HINT_VH)
}

/// How many of the page's text paragraphs fit (all of them, if the page
/// fits into [`PANEL_MAX_VH`]): a page too tall for the window drops its
/// description from the end rather than cutting off its last items.
pub fn fitted_paragraphs(view: &MenuView, aspect: f32) -> usize {
    (0..=view.text.len())
        .rev()
        .find(|&k| page_height_vh(view, aspect, k) <= PANEL_MAX_VH)
        .unwrap_or(0)
}

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
        .observe(
            |click: On<PointerClick>, mut pointer: ResMut<MenuPointer>| {
                if click.button == PointerButton::Secondary {
                    pointer.0.push(MenuPointerEvent::Back);
                }
            },
        )
        .with_children(|root| {
            root.spawn((
                Node {
                    width: vh(PANEL_WIDTH_VH),
                    max_width: vw(PANEL_MAX_VW),
                    max_height: vh(PANEL_MAX_VH),
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
            .with_children(|panel| {
                panel.spawn((Text::new(""), font(TITLE_VH), TextColor(TEXT), MenuTitle));
                panel.spawn((
                    Node {
                        display: Display::None,
                        ..default()
                    },
                    Text::new(""),
                    font(TEXT_VH),
                    TextColor(FOCUS),
                    MenuStatus,
                ));
                panel.spawn((
                    Node {
                        display: Display::None,
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
                        row_gap: vh(LIST_GAP_VH),
                        margin: UiRect::vertical(vh(LIST_MARGIN_VH)),
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
                    padding: UiRect::axes(vh(ROW_PAD_X_VH), vh(ROW_PAD_Y_VH)),
                    border: UiRect::all(vh(ROW_BORDER_VH)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    ..default()
                },
                BackgroundColor(ROW),
                BorderColor::all(Color::NONE),
                MenuRow(i),
                children![(Text::new(label.clone()), font(ITEM_VH), TextColor(TEXT))],
            ))
            .observe(
                move |_: On<PointerOver>, mut pointer: ResMut<MenuPointer>| {
                    pointer.0.push(MenuPointerEvent::Hover(i));
                },
            )
            .observe(
                move |click: On<PointerClick>, mut pointer: ResMut<MenuPointer>| {
                    if click.button == PointerButton::Primary {
                        pointer.0.push(MenuPointerEvent::Click(i));
                    }
                },
            );
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
    mut rows: Query<(Ref<MenuRow>, &mut BackgroundColor, &mut BorderColor)>,
    mut texts: ParamSet<(
        Query<&mut Text, With<MenuTitle>>,
        Query<(&mut Text, &mut Node), (With<MenuStatus>, Without<MenuRoot>)>,
        Query<(&mut Text, &mut Node), (With<MenuText>, Without<MenuRoot>)>,
        Query<&mut Text, With<MenuHint>>,
    )>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut last_aspect: Local<f32>,
) {
    let aspect = windows
        .single()
        .map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    let resized = (aspect - *last_aspect).abs() > 1e-3;
    *last_aspect = aspect;
    // Rows of a new page appear a frame after they were spawned: style them
    // then, too.
    if !view.is_changed() && !resized && !rows.iter().any(|(r, ..)| r.is_added()) {
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
    set_text_or_hide(&mut texts.p1(), &view.status);
    // A page too tall for the window drops description paragraphs.
    let shown = fitted_paragraphs(&view, aspect);
    set_text_or_hide(&mut texts.p2(), &view.text[..shown].join("\n"));
    set_text(&mut texts.p3(), &view.hint);
}

/// Sets the text; an empty one takes no room (as [`page_height_vh`] counts).
fn set_text_or_hide(
    q: &mut Query<(&mut Text, &mut Node), impl bevy::ecs::query::QueryFilter>,
    s: &str,
) {
    for (mut t, mut node) in q.iter_mut() {
        if t.0 != s {
            s.clone_into(&mut t.0);
        }
        let display = if s.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
    }
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
    fn text_wraps_at_words_like_the_monospaced_font() {
        // 10 columns at 2 vh (1.2 vh per glyph) in 12 vh.
        assert_eq!(wrapped_lines("", 12.0, 2.0), 1);
        assert_eq!(wrapped_lines("0123456789", 12.0, 2.0), 1);
        assert_eq!(wrapped_lines("aaaa bbbbb", 12.0, 2.0), 1);
        assert_eq!(wrapped_lines("aaaa bbbbbb", 12.0, 2.0), 2);
        assert_eq!(wrapped_lines("a\nb", 12.0, 2.0), 2);
        // A word longer than a line is broken.
        assert_eq!(wrapped_lines("0123456789012345678901", 12.0, 2.0), 3);
    }

    fn page(text: &[&str], items: usize) -> MenuView {
        MenuView {
            open: true,
            title: "Ride".into(),
            status: String::new(),
            text: text.iter().map(|s| s.to_string()).collect(),
            items: (0..items).map(|i| format!("Item {i}")).collect(),
            focused: 0,
            hint: "Up/Down move   Enter/A select   Esc/B back".into(),
        }
    }

    #[test]
    fn the_page_height_adds_up_the_parts() {
        let empty = page(&[], 0);
        let one = page(&[], 1);
        let row = ITEM_VH * LINE_HEIGHT + 2.0 * (ROW_PAD_Y_VH + ROW_BORDER_VH);
        let h = |v: &MenuView| page_height_vh(v, 16.0 / 9.0, v.text.len());
        assert!((h(&one) - h(&empty) - row).abs() < 1e-4);
        let two = page(&[], 2);
        assert!((h(&two) - h(&one) - row - LIST_GAP_VH).abs() < 1e-4);
        // A text line and its gap; a status line likewise.
        let text = page(&["Short."], 2);
        assert!((h(&text) - h(&two) - TEXT_VH * LINE_HEIGHT - PANEL_GAP_VH).abs() < 1e-4);
        // A narrower window wraps more.
        let words = "word ".repeat(40);
        let long = page(&[words.trim_end()], 2);
        assert!(page_height_vh(&long, 1.0, 1) > page_height_vh(&long, 16.0 / 9.0, 1));
        assert!(content_width_vh(1.0) < content_width_vh(16.0 / 9.0));
    }

    /// Regression: the Ride page (seven items and two paragraphs) ran off
    /// the bottom of a 1280x720 window, "Back" was cut off. A page too
    /// tall drops description paragraphs from the end, never items.
    #[test]
    fn a_page_too_tall_drops_its_text_not_its_items() {
        let para = "Local events on this route: everyone lines up at the start, 3-2-1-GO. Races and time trials end at the finish with a results table.";
        let v = page(&[para, para, para], 7);
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 1.0] {
            let k = fitted_paragraphs(&v, aspect);
            assert!(k < 3, "{aspect}: {k}");
            assert!(page_height_vh(&v, aspect, k) <= PANEL_MAX_VH, "{aspect}");
        }
        // Short pages keep everything.
        let v = page(&[para], 3);
        assert_eq!(fitted_paragraphs(&v, 16.0 / 9.0), 1);
    }

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
        app.add_plugins((
            MinimalPlugins,
            crate::console::ConsoleCorePlugin,
            MenuCorePlugin,
        ));
        #[derive(Resource, Default)]
        struct Runs(u32);
        app.init_resource::<Runs>()
            .add_systems(Update, (|mut r: ResMut<Runs>| r.0 += 1).run_if(ride_input));
        app.update();
        assert_eq!(app.world().resource::<Runs>().0, 1);
        app.world_mut().resource_mut::<MenuView>().open = true;
        app.update();
        assert_eq!(*app.world().resource::<InputContext>(), InputContext::Menu);
        assert_eq!(app.world().resource::<Runs>().0, 1);
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_open(true);
        app.update();
        assert_eq!(
            *app.world().resource::<InputContext>(),
            InputContext::Console
        );
        app.world_mut().resource_mut::<MenuView>().open = false;
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_open(false);
        app.update();
        assert_eq!(app.world().resource::<Runs>().0, 2);
    }
}
