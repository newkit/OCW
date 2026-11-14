//! In-game menu overlay (spec §3.2.1) and the input context.
//!
//! The menu's model (pages, focus, page stack) lives in the app; this module
//! only draws the [`MenuView`] resource, as the HUD draws `RideView`, and
//! reports mouse hover / clicks as [`MenuPointer`] events. Ten-foot theme:
//! text sizes are a share of the window height, high contrast, a clearly
//! visible focus frame. Value rows ("Label:  < value >") have tappable
//! ‹ › arrows (mouse and touch reach every value, spec §3.2.2); a page
//! with a [`MenuGrid`] (the weather presets, spec §14 Phase 9, S5) is
//! drawn as a grid of icon cells instead of a list.
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
    /// Items drawn dimmed (they do nothing: not available, not yours);
    /// missing entries are not dimmed.
    pub dimmed: Vec<bool>,
    /// Draw the items as a grid of these cells (one per item).
    pub grid: Option<MenuGrid>,
}

/// A page drawn as a grid (spec §14 Phase 9, S5: the weather presets).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MenuGrid {
    pub columns: usize,
    /// One per item, in the items' order.
    pub cells: Vec<GridCell>,
}

/// One cell of a [`MenuGrid`]: an icon over a label.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GridCell {
    pub label: String,
    pub icon: Option<ocw_weather::icon::Icon>,
    /// The current choice (a thin frame).
    pub marked: bool,
}

impl MenuView {
    fn is_dimmed(&self, i: usize) -> bool {
        self.dimmed.get(i).copied().unwrap_or(false)
    }
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
    /// The ‹ (−1) or › (+1) arrow of value row `i` was clicked or tapped.
    Step(usize, i8),
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
/// A grid page's panel is wider (five cells with one-line labels).
const GRID_PANEL_WIDTH_VH: f32 = 150.0;
/// Grid cells: icon size, label size, padding, gaps (vh).
const GRID_ICON_VH: f32 = 8.0;
const GRID_LABEL_VH: f32 = 3.0;
const GRID_PAD_VH: f32 = 0.8;
const GRID_GAP_VH: f32 = 1.0;
/// The gap between the cells of a grid row (% of the grid's width).
const GRID_GAP_PCT: f32 = 1.0;
/// Pixels of a cell's icon image.
const GRID_ICON_PX: u32 = 128;
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
    panel_content_vh(PANEL_WIDTH_VH, aspect)
}

fn panel_content_vh(panel_vh: f32, aspect: f32) -> f32 {
    panel_vh.min(PANEL_MAX_VW * aspect) - 2.0 * (PANEL_PAD_X_VH + PANEL_BORDER_VH)
}

/// The content width of `view`'s panel (a grid's is wider).
fn view_content_vh(view: &MenuView, aspect: f32) -> f32 {
    panel_content_vh(
        if view.grid.is_some() {
            GRID_PANEL_WIDTH_VH
        } else {
            PANEL_WIDTH_VH
        },
        aspect,
    )
}

/// Width of a grid cell (% of the grid's width, its border included).
fn grid_cell_pct(columns: usize) -> f32 {
    let c = columns.max(1) as f32;
    (100.0 - GRID_GAP_PCT * (c - 1.0)) / c
}

/// Width of a grid cell (vh, its border included) in content `w` wide.
fn grid_cell_vh(columns: usize, w: f32) -> f32 {
    w * grid_cell_pct(columns) / 100.0
}

/// Height of a grid (vh) in content `w` wide.
fn grid_height_vh(grid: &MenuGrid, w: f32) -> f32 {
    let cols = grid.columns.max(1);
    let cell = grid_cell_vh(cols, w);
    let label_w = cell - 2.0 * (GRID_PAD_VH + ROW_BORDER_VH);
    grid.cells
        .chunks(cols)
        .map(|row| {
            let lines = row
                .iter()
                .map(|c| wrapped_lines(&c.label, label_w, GRID_LABEL_VH))
                .max()
                .unwrap_or(1) as f32;
            GRID_ICON_VH + lines * GRID_LABEL_VH * LINE_HEIGHT + 2.0 * (GRID_PAD_VH + ROW_BORDER_VH)
        })
        .sum::<f32>()
        + GRID_GAP_VH * (grid.cells.len().div_ceil(cols).saturating_sub(1)) as f32
}

/// Height of the menu panel for `view` (vh) in a window of `aspect`,
/// showing the first `paragraphs` paragraphs of its text.
pub fn page_height_vh(view: &MenuView, aspect: f32, paragraphs: usize) -> f32 {
    let w = view_content_vh(view, aspect);
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
    let list = match &view.grid {
        Some(g) => grid_height_vh(g, w) + 2.0 * LIST_MARGIN_VH,
        None => {
            rows + LIST_GAP_VH * view.items.len().saturating_sub(1) as f32 + 2.0 * LIST_MARGIN_VH
        }
    };
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
/// The text of a dimmed row (it does nothing).
const DISABLED: Color = Color::srgba(1.0, 1.0, 1.0, 0.42);
/// The frame of a grid's current choice.
const MARKED: Color = Color::srgba(1.0, 1.0, 1.0, 0.55);

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
/// A grid cell's current-choice frame.
#[derive(Component)]
struct Marked(bool);
#[derive(Component)]
struct MenuPanel;

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
                MenuPanel,
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

/// "Clouds:  < 60 % >" → ("Clouds:  ", "60 %"): a value row's parts.
pub fn split_value(label: &str) -> Option<(&str, &str)> {
    let at = label.find(":  < ")?;
    let value = label[at + 5..].strip_suffix(" >")?;
    Some((&label[..at + 3], value))
}

fn hover_and_click(row: &mut EntityCommands, i: usize) {
    row.observe(
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

/// A value row's ‹ or › arrow: its own pointer target (the row's click
/// does not also fire).
fn spawn_arrow(r: &mut ChildSpawnerCommands, i: usize, dir: i8) {
    r.spawn((
        Node {
            padding: UiRect::horizontal(vh(0.4)),
            ..default()
        },
        children![(
            Text::new(if dir < 0 { "<" } else { ">" }),
            font(ITEM_VH),
            TextColor(FOCUS)
        )],
    ))
    .observe(
        move |mut click: On<PointerClick>, mut pointer: ResMut<MenuPointer>| {
            if click.button == PointerButton::Primary {
                click.propagate(false);
                pointer.0.push(MenuPointerEvent::Step(i, dir));
            }
        },
    );
}

fn spawn_rows(commands: &mut Commands, list: Entity, view: &MenuView) {
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        for (i, label) in view.items.iter().enumerate() {
            let color = TextColor(if view.is_dimmed(i) { DISABLED } else { TEXT });
            let mut row = l.spawn((
                Node {
                    padding: UiRect::axes(vh(ROW_PAD_X_VH), vh(ROW_PAD_Y_VH)),
                    border: UiRect::all(vh(ROW_BORDER_VH)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(ROW),
                BorderColor::all(Color::NONE),
                MenuRow(i),
            ));
            row.with_children(|r| match split_value(label) {
                // A value row: the arrows are their own targets.
                Some((name, value)) => {
                    r.spawn((Text::new(name), font(ITEM_VH), color));
                    spawn_arrow(r, i, -1);
                    r.spawn((Text::new(format!(" {value} ")), font(ITEM_VH), color));
                    spawn_arrow(r, i, 1);
                }
                None => {
                    r.spawn((Text::new(label.clone()), font(ITEM_VH), color));
                }
            });
            hover_and_click(&mut row, i);
        }
    });
}

type IconCache = std::collections::HashMap<ocw_weather::icon::Icon, Handle<Image>>;

/// The icon of a grid cell as an image (drawn once per icon).
fn icon_image(
    icon: ocw_weather::icon::Icon,
    cache: &mut IconCache,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    cache
        .entry(icon)
        .or_insert_with(|| {
            images.add(Image::new(
                Extent3d {
                    width: GRID_ICON_PX,
                    height: GRID_ICON_PX,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                ocw_weather::icon::rgba(icon, GRID_ICON_PX),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            ))
        })
        .clone()
}

fn spawn_grid(
    commands: &mut Commands,
    list: Entity,
    view: &MenuView,
    grid: &MenuGrid,
    icons: &mut IconCache,
    images: &mut Assets<Image>,
) {
    let cell_pct = grid_cell_pct(grid.columns);
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        for (i, cell) in grid.cells.iter().enumerate() {
            let dimmed = view.is_dimmed(i);
            let color = TextColor(if dimmed { DISABLED } else { TEXT });
            let mut node = l.spawn((
                Node {
                    width: Val::Percent(cell_pct),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    padding: UiRect::all(vh(GRID_PAD_VH)),
                    border: UiRect::all(vh(ROW_BORDER_VH)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    ..default()
                },
                BackgroundColor(ROW),
                BorderColor::all(Color::NONE),
                MenuRow(i),
                Marked(cell.marked),
            ));
            node.with_children(|c| {
                let mut icon = c.spawn(Node {
                    width: vh(GRID_ICON_VH),
                    height: vh(GRID_ICON_VH),
                    ..default()
                });
                if let Some(ic) = cell.icon {
                    let mut img = ImageNode::new(icon_image(ic, icons, images));
                    if dimmed {
                        img.color = Color::srgba(1.0, 1.0, 1.0, 0.45);
                    }
                    icon.insert(img);
                }
                c.spawn((
                    Text::new(cell.label.clone()),
                    font(GRID_LABEL_VH),
                    color,
                    TextLayout::justify(Justify::Center),
                ));
            });
            hover_and_click(&mut node, i);
        }
    });
}

/// The page's layout key: rows are respawned only when it changes, not on
/// every focus move or status update.
type PageKey = (String, Vec<String>, Vec<bool>, Option<MenuGrid>);

/// The list's and the panel's nodes, disjoint from the root's and the
/// texts' (`Node` is borrowed mutably by several queries).
type ListFilter = (
    With<MenuList>,
    Without<MenuRoot>,
    Without<MenuStatus>,
    Without<MenuText>,
);
type PanelFilter = (
    With<MenuPanel>,
    Without<MenuRoot>,
    Without<MenuList>,
    Without<MenuStatus>,
    Without<MenuText>,
);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn draw_menu(
    mut commands: Commands,
    view: Res<MenuView>,
    mut page: Local<Option<PageKey>>,
    mut root: Query<&mut Node, With<MenuRoot>>,
    mut list: Query<(Entity, &mut Node), ListFilter>,
    mut panel: Query<&mut Node, PanelFilter>,
    mut rows: Query<(
        Ref<MenuRow>,
        &mut BackgroundColor,
        &mut BorderColor,
        Option<&Marked>,
    )>,
    (mut images, mut icons): (
        ResMut<Assets<Image>>,
        Local<std::collections::HashMap<ocw_weather::icon::Icon, Handle<Image>>>,
    ),
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
    let key = (
        view.title.clone(),
        view.items.clone(),
        view.dimmed.clone(),
        view.grid.clone(),
    );
    if page.as_ref() != Some(&key) {
        if let Ok((list, mut node)) = list.single_mut() {
            // A grid wraps its cells into rows; a list is a column.
            let (dir, wrap, gap) = match view.grid {
                Some(_) => (FlexDirection::Row, FlexWrap::Wrap, vh(GRID_GAP_VH)),
                None => (FlexDirection::Column, FlexWrap::NoWrap, vh(LIST_GAP_VH)),
            };
            node.flex_direction = dir;
            node.flex_wrap = wrap;
            node.row_gap = gap;
            node.column_gap = match view.grid {
                Some(_) => Val::Percent(GRID_GAP_PCT),
                None => Val::ZERO,
            };
            match &view.grid {
                Some(g) => spawn_grid(&mut commands, list, &view, g, &mut icons, &mut images),
                None => spawn_rows(&mut commands, list, &view),
            }
        }
        if let Ok(mut p) = panel.single_mut() {
            p.width = vh(if view.grid.is_some() {
                GRID_PANEL_WIDTH_VH
            } else {
                PANEL_WIDTH_VH
            });
        }
        *page = Some(key);
    }
    for (row, mut bg, mut border, marked) in &mut rows {
        let focused = row.0 == view.focused;
        bg.0 = if focused { ROW_FOCUSED } else { ROW };
        *border = BorderColor::all(match (focused, marked.is_some_and(|m| m.0)) {
            (true, _) => FOCUS,
            (false, true) => MARKED,
            _ => Color::NONE,
        });
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
            ..Default::default()
        }
    }

    #[test]
    fn value_rows_split_into_name_and_value() {
        assert_eq!(
            split_value("Clouds:  < 60 % >"),
            Some(("Clouds:  ", "60 %"))
        );
        assert_eq!(split_value("Mode:  fixed"), None);
        assert_eq!(split_value("Back"), None);
    }

    /// A grid page (the weather presets): its height counts the rows of
    /// cells, and 17 presets and Back fit a 16:9 and a 4:3 window.
    #[test]
    fn a_grid_of_eighteen_cells_fits() {
        let cells: Vec<GridCell> = ocw_weather::presets::PRESETS
            .iter()
            .map(|p| GridCell {
                label: ocw_weather::presets::title(p.0),
                icon: ocw_weather::presets::preset(p.0).map(|w| ocw_weather::icon::Icon::of(&w)),
                marked: false,
            })
            .chain([GridCell {
                label: "Back".into(),
                ..Default::default()
            }])
            .collect();
        let mut v = page(&["Choose a weather."], cells.len());
        v.status = "Thunderstorm: cloudburst, gusts, lightning - Enter: in 1 min".into();
        v.grid = Some(MenuGrid { columns: 5, cells });
        // All of it at 16:9; at 4:3 the description may give way.
        assert!(page_height_vh(&v, 16.0 / 9.0, 1) <= PANEL_MAX_VH);
        for aspect in [16.0 / 9.0, 4.0 / 3.0] {
            let h = page_height_vh(&v, aspect, fitted_paragraphs(&v, aspect));
            assert!(h <= PANEL_MAX_VH, "{aspect}: {h}");
            assert!(h > 4.0 * GRID_ICON_VH, "{h}");
        }
        // At 16:9 every label fits on one line of its cell.
        let w = view_content_vh(&v, 16.0 / 9.0);
        let cell = grid_cell_vh(5, w) - 2.0 * (GRID_PAD_VH + ROW_BORDER_VH);
        for c in &v.grid.as_ref().unwrap().cells {
            assert_eq!(
                wrapped_lines(&c.label, cell, GRID_LABEL_VH),
                1,
                "{}",
                c.label
            );
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

    /// The drawing systems run headless (regression guard: their `Node`
    /// queries must not conflict): a list page with a value row and its
    /// arrows, then the grid with its icons.
    #[test]
    fn the_menu_draws_a_list_and_a_grid_headless() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            MenuPlugin,
        ));
        app.init_asset::<Image>();
        app.update();
        let count = |app: &mut App| {
            let rows = app
                .world_mut()
                .query::<&MenuRow>()
                .iter(app.world())
                .count();
            let icons = app
                .world_mut()
                .query::<&ImageNode>()
                .iter(app.world())
                .count();
            (rows, icons)
        };
        *app.world_mut().resource_mut::<MenuView>() = MenuView {
            open: true,
            title: "Custom weather: sky".into(),
            items: vec!["Cloud cover:  < 60 % >".into(), "Back".into()],
            dimmed: vec![false, false],
            ..Default::default()
        };
        app.update();
        app.update();
        assert_eq!(count(&mut app), (2, 0));
        let texts = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect::<Vec<_>>();
        assert!(texts.iter().any(|t| t == "<") && texts.iter().any(|t| t == ">"));
        let cells: Vec<GridCell> = ocw_weather::presets::PRESETS
            .iter()
            .map(|p| GridCell {
                label: ocw_weather::presets::title(p.0),
                icon: ocw_weather::presets::preset(p.0).map(|w| ocw_weather::icon::Icon::of(&w)),
                marked: p.0 == "rain",
            })
            .chain([GridCell {
                label: "Back".into(),
                ..Default::default()
            }])
            .collect();
        {
            let mut v = app.world_mut().resource_mut::<MenuView>();
            v.title = "Weather presets".into();
            v.items = cells.iter().map(|c| c.label.clone()).collect();
            v.dimmed = Vec::new();
            v.grid = Some(MenuGrid { columns: 5, cells });
        }
        app.update();
        app.update();
        assert_eq!(count(&mut app), (18, 17));
        assert_eq!(
            app.world_mut()
                .query::<&Marked>()
                .iter(app.world())
                .filter(|m| m.0)
                .count(),
            1
        );
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
