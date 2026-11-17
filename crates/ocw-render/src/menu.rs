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
    /// Lines drawn large between the status and the text, never dropped
    /// for room: the trust dialog's fingerprint (spec §4.7 *User keys*).
    pub emphasis: Vec<String>,
    /// Draw the panel wide (as a grid page): a page with long text that
    /// must all fit (the trust dialog).
    pub wide: bool,
    pub items: Vec<String>,
    pub focused: usize,
    /// Control hint at the bottom.
    pub hint: String,
    /// Items drawn dimmed (they do nothing: not available, not yours);
    /// missing entries are not dimmed.
    pub dimmed: Vec<bool>,
    /// Draw the items as a grid of these cells (one per item).
    pub grid: Option<MenuGrid>,
    /// Markers drawn with the items (the rider list: who flies in unicorn
    /// mode, the damage dot); missing entries have none.
    pub marks: Vec<RowMark>,
    /// A bar chart between the text and the items (the training history's
    /// power curve, spec §14 Phase 7).
    pub chart: Option<MenuChart>,
    /// Draw the items as a month calendar (spec §14 Phase 9 S9a): items
    /// 0–41 the days, then the buttons ([`MenuCalendar`]).
    pub calendar: Option<MenuCalendar>,
    /// Dock the panel at the window's left edge, narrower and without the
    /// dark veil, so the scene beside it stays visible (the character
    /// editor: the rider framed in the free space, spec §14 Phase 4, 4N
    /// *Visual editor*).
    pub side: bool,
}

/// A month view (World → Time → Calendar): the buttons over the grid (items
/// [`CALENDAR_DAYS`] … `+ 6`), the weekdays, six weeks of days (items 0 –
/// 41), the time stepper, Select and Back under it, the legend of the bands.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MenuCalendar {
    /// "November 2026".
    pub title: String,
    /// "Mon" … "Sun", in the grid's order.
    pub weekdays: Vec<String>,
    pub days: Vec<CalendarDay>,
    /// The labels of the items after the days: seven buttons over the grid,
    /// then the time stepper, Select ("Select Tue 17 Nov 2026") and Back.
    pub controls: Vec<String>,
    /// The bands' colours and names (sRGB 0–1), then the moon's glyphs.
    pub legend: Vec<(String, [f32; 3])>,
    pub moon_legend: String,
}

/// Days of a [`MenuCalendar`] (six weeks).
pub const CALENDAR_DAYS: usize = 42;
/// Buttons over the grid.
pub const CALENDAR_BUTTONS: usize = 7;
/// Select's place among the items under the grid (the time stepper, Select,
/// Back).
pub const CALENDAR_SELECT: usize = 1;

/// One day of a [`MenuCalendar`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CalendarDay {
    /// "16".
    pub label: String,
    /// The look of the day at the rider's place and altitude (sRGB 0–1).
    pub band: Option<[f32; 3]>,
    /// The moon's main phase that day ("●", "○", "◑", "◐").
    pub moon: Option<String>,
    /// A day of the neighbouring months (drawn faint).
    pub outside: bool,
    /// Today (the real date at the place).
    pub today: bool,
    /// The world clock's date (a frame, as a grid's current choice).
    pub selected: bool,
}

/// A bar chart of a menu page: one bar per value, an optional mark per bar
/// (the personal best), drawn between the description and the items.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MenuChart {
    /// The line over the bars ("Power curve: bars this ride, lines your best").
    pub caption: String,
    pub bars: Vec<ChartBar>,
}

/// One bar of a [`MenuChart`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChartBar {
    /// Under the bar ("5s", "20m").
    pub label: String,
    /// Its height, 0–1 of the chart.
    pub value: f32,
    /// Over the bar ("812").
    pub text: String,
    /// A line across the bar at this height (0–1): the best so far.
    pub mark: Option<f32>,
    /// Drawn highlighted (the bar is the best so far).
    pub best: bool,
}

/// Markers of one list item (spec §14 Phase 13E, 12F: the rider list).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RowMark {
    /// A small unicorn before the label: the rider flies in unicorn mode.
    pub unicorn: bool,
    /// A dot after the label: its icon (the status shapes of the UI font,
    /// never colour alone) and colour (sRGB 0–1).
    pub dot: Option<(String, [f32; 3])>,
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
    /// A colour swatch instead of an icon (sRGB; skin, hair, facial hair).
    pub swatch: Option<[u8; 3]>,
    /// A rendering of the model instead of an icon (the character
    /// editor's tiles, spec §14 Phase 4, 4N: [`crate::thumbs`]).
    pub thumb: Option<crate::thumbs::ThumbKey>,
    /// The current choice (a thin frame).
    pub marked: bool,
}

impl MenuView {
    fn is_dimmed(&self, i: usize) -> bool {
        self.dimmed.get(i).copied().unwrap_or(false)
    }

    fn mark(&self, i: usize) -> Option<&RowMark> {
        self.marks.get(i)
    }
}

/// The unicorn marker's size (vh; the impostor picture's 4 : 3).
const UNICORN_MARK_VH: (f32, f32) = (4.4, 3.3);

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
    /// A horizontal swipe over the menu (touch): +1 to the left (the next
    /// month of the calendar), −1 to the right.
    Swipe(i8),
}

/// A finished touch moved by (`dx`, `dy`) px in a window `width` px wide:
/// a swipe if it went mostly sideways over 8 % of the width.
pub fn swipe_dir(dx: f32, dy: f32, width: f32) -> Option<i8> {
    (dx.abs() > 0.08 * width && dx.abs() > 2.0 * dy.abs()).then_some(if dx < 0.0 { 1 } else { -1 })
}

/// Swipes over the open menu (the calendar's months, spec §14 Phase 9 S9a).
fn detect_swipe(
    touches: Option<Res<Touches>>,
    view: Res<MenuView>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut pointer: ResMut<MenuPointer>,
) {
    let Some(touches) = touches else { return };
    if !view.open {
        return;
    }
    let width = windows.single().map_or(1280.0, |w| w.width());
    for t in touches.iter_just_released() {
        let d = t.position() - t.start_position();
        if let Some(dir) = swipe_dir(d.x, d.y, width) {
            pointer.0.push(MenuPointerEvent::Swipe(dir));
        }
    }
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
            .add_systems(PreUpdate, update_input_context)
            .add_systems(Update, detect_swipe);
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
        // The character editor's tiles show renderings (spec §14 Phase 4,
        // 4N).
        if !app.is_plugin_added::<crate::thumbs::ThumbnailPlugin>() {
            app.add_plugins(crate::thumbs::ThumbnailPlugin);
        }
        app.add_systems(Startup, spawn_menu)
            .add_systems(Update, (draw_menu, draw_chart));
    }
}

/// Text sizes in % of the window height (spec: at least 3 %).
const TITLE_VH: f32 = 6.0;
const ITEM_VH: f32 = 4.2;
const TEXT_VH: f32 = 3.2;
const HINT_VH: f32 = 3.0;
/// The emphasised lines (a fingerprint: monospaced, larger than the text).
const EMPHASIS_VH: f32 = 4.0;

/// The chart: bar area, the texts over and under the bars (vh).
const CHART_BAR_VH: f32 = 9.0;
const CHART_TEXT_VH: f32 = 2.4;
const CHART_GAP_VH: f32 = 0.4;

/// Height of a chart (vh): caption, values, bars, labels.
pub fn chart_height_vh(chart: &MenuChart) -> f32 {
    let caption = if chart.caption.is_empty() {
        0.0
    } else {
        TEXT_VH * LINE_HEIGHT + CHART_GAP_VH
    };
    caption + 2.0 * (CHART_TEXT_VH * LINE_HEIGHT + CHART_GAP_VH) + CHART_BAR_VH
}

/// Panel layout in % of the window height (vh). [`page_height_vh`] adds
/// these up; `spawn_menu` / `spawn_rows` use the same numbers.
const PANEL_WIDTH_VH: f32 = 110.0;
/// A grid page's panel is wider (five cells with one-line labels).
const GRID_PANEL_WIDTH_VH: f32 = 150.0;
/// Grid cells: icon size, label size, padding, gaps (vh).
const GRID_ICON_VH: f32 = 8.0;
const GRID_LABEL_VH: f32 = 3.0;
/// The names under rendered pictures (the character's tiles) are smaller:
/// a picture says most of it.
const GRID_THUMB_LABEL_VH: f32 = 2.2;

/// The label size of `grid`'s cells.
fn grid_label_vh(grid: &MenuGrid) -> f32 {
    if grid_thumbs(grid) {
        GRID_THUMB_LABEL_VH
    } else {
        GRID_LABEL_VH
    }
}
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

/// The width of the side panel (vh): about a third of a 16:9 window.
pub const SIDE_PANEL_WIDTH_VH: f32 = 72.0;

/// The width of `view`'s panel (vh): a grid's is wider, the side panel
/// narrower.
fn view_panel_vh(view: &MenuView) -> f32 {
    if view.side {
        SIDE_PANEL_WIDTH_VH
    } else if view.grid.is_some() || view.calendar.is_some() || view.wide {
        GRID_PANEL_WIDTH_VH
    } else {
        PANEL_WIDTH_VH
    }
}

/// The content width of `view`'s panel (a grid's is wider).
fn view_content_vh(view: &MenuView, aspect: f32) -> f32 {
    panel_content_vh(view_panel_vh(view), aspect)
}

/// Width of a grid cell (% of the grid's width, its border included).
fn grid_cell_pct(columns: usize) -> f32 {
    let c = columns.max(1) as f32;
    // A hair under the exact share: the layout's rounding must not wrap
    // the last cell of a row onto the next (the editor's five columns
    // showed as four).
    (100.0 - GRID_GAP_PCT * (c - 1.0)) / c - 0.1
}

/// Width of a grid cell (vh, its border included) in content `w` wide.
fn grid_cell_vh(columns: usize, w: f32) -> f32 {
    w * grid_cell_pct(columns) / 100.0
}

/// Whether a grid's cells have pictures (icons, swatches or renderings);
/// named tiles without any are drawn without the picture's room.
fn grid_pictures(grid: &MenuGrid) -> bool {
    grid.cells
        .iter()
        .any(|c| c.icon.is_some() || c.swatch.is_some() || c.thumb.is_some())
}

/// Whether a grid's pictures are renderings of the models (the character
/// editor's tiles).
pub fn grid_thumbs(grid: &MenuGrid) -> bool {
    grid.cells.iter().any(|c| c.thumb.is_some())
}

/// Rows of rendered tiles that still show their names under the pictures:
/// a larger grid of renderings (the 21 hair styles) shows only the
/// pictures, and the page's status line names the focused tile.
pub const THUMB_LABEL_ROWS: usize = 2;

/// Whether the rendered tiles of `grid` show their names (see
/// [`THUMB_LABEL_ROWS`]); always for the other grids.
pub fn grid_labels(grid: &MenuGrid) -> bool {
    !grid_thumbs(grid) || grid.cells.len().div_ceil(grid.columns.max(1)) <= THUMB_LABEL_ROWS
}

/// What a grid cell shows: (the picture's room, its label).
fn cell_parts(grid: &MenuGrid, cell: &GridCell) -> (bool, bool) {
    if !grid_thumbs(grid) {
        return (grid_pictures(grid), true);
    }
    // A grid of renderings: Back and the like are a name only.
    let picture = cell.thumb.is_some() || cell.icon.is_some() || cell.swatch.is_some();
    (picture, !picture || grid_labels(grid))
}

/// Height of a grid (vh) in content `w` wide.
fn grid_height_vh(grid: &MenuGrid, w: f32) -> f32 {
    let cols = grid.columns.max(1);
    let cell = grid_cell_vh(cols, w);
    let label_w = cell - 2.0 * (GRID_PAD_VH + ROW_BORDER_VH);
    grid.cells
        .chunks(cols)
        .map(|row| {
            row.iter()
                .map(|c| {
                    let (picture, label) = cell_parts(grid, c);
                    let lines = if label {
                        wrapped_lines(&c.label, label_w, grid_label_vh(grid)) as f32
                    } else {
                        0.0
                    };
                    (if picture { GRID_ICON_VH } else { 0.0 })
                        + lines * grid_label_vh(grid) * LINE_HEIGHT
                        + 2.0 * (GRID_PAD_VH + ROW_BORDER_VH)
                })
                .fold(0.0, f32::max)
        })
        .sum::<f32>()
        + GRID_GAP_VH * (grid.cells.len().div_ceil(cols).saturating_sub(1)) as f32
}

/// The calendar's sizes (vh): button labels, weekdays, day numbers, the
/// day's band, paddings, gaps, the legend.
const CAL_LABEL_VH: f32 = 3.0;
const CAL_WEEKDAY_VH: f32 = 2.6;
const CAL_NUM_VH: f32 = 3.0;
const CAL_BAND_VH: f32 = 0.9;
const CAL_PAD_VH: f32 = 0.4;
const CAL_BORDER_VH: f32 = 0.3;
const CAL_ROW_GAP_VH: f32 = 0.5;
const CAL_SECTION_GAP_VH: f32 = 0.8;
const CAL_LEGEND_VH: f32 = 2.6;
/// The legend's swatch (vh).
const CAL_SWATCH_VH: (f32, f32) = (2.6, 1.2);

fn cal_button_vh() -> f32 {
    CAL_LABEL_VH * LINE_HEIGHT + 2.0 * (CAL_PAD_VH + CAL_BORDER_VH)
}

fn cal_day_vh() -> f32 {
    CAL_NUM_VH * LINE_HEIGHT + CAL_BAND_VH + 2.0 * (CAL_PAD_VH + CAL_BORDER_VH) + CAL_ROW_GAP_VH
}

/// The legend as one text (for its wrapped height).
fn cal_legend_text(c: &MenuCalendar) -> String {
    c.legend
        .iter()
        .map(|(n, _)| format!("XX {n}"))
        .chain([c.moon_legend.clone()])
        .collect::<Vec<_>>()
        .join("   ")
}

/// Height of a calendar (vh) in content `w` wide: the buttons, the
/// weekdays, six weeks, the time and Back, the legend.
pub fn calendar_height_vh(c: &MenuCalendar, w: f32) -> f32 {
    let weeks = c.days.len().div_ceil(7) as f32;
    let legend =
        wrapped_lines(&cal_legend_text(c), w, CAL_LEGEND_VH) as f32 * CAL_LEGEND_VH * LINE_HEIGHT;
    2.0 * cal_button_vh() + CAL_WEEKDAY_VH * LINE_HEIGHT + weeks * cal_day_vh() - CAL_ROW_GAP_VH
        + legend
        + 4.0 * CAL_SECTION_GAP_VH
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
    let list = match (&view.grid, &view.calendar) {
        (_, Some(c)) => calendar_height_vh(c, w) + 2.0 * LIST_MARGIN_VH,
        (Some(g), _) => grid_height_vh(g, w) + 2.0 * LIST_MARGIN_VH,
        (None, None) => {
            rows + LIST_GAP_VH * view.items.len().saturating_sub(1) as f32 + 2.0 * LIST_MARGIN_VH
        }
    };
    let chart = view
        .chart
        .as_ref()
        .map_or(0.0, |c| chart_height_vh(c) + PANEL_GAP_VH);
    2.0 * (PANEL_PAD_Y_VH + PANEL_BORDER_VH)
        + chart
        + 4.0 * PANEL_GAP_VH
        + lines(&view.title, TITLE_VH)
        + lines(&view.status, TEXT_VH)
        + lines(&text, TEXT_VH)
        + PANEL_GAP_VH
        + lines(&view.emphasis.join("\n"), EMPHASIS_VH)
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
/// A chart's bars (the best one is drawn in the focus colour).
const CHART_BAR: Color = Color::srgb(0.45, 0.68, 0.95);

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct MenuTitle;
#[derive(Component)]
struct MenuStatus;
#[derive(Component)]
struct MenuText;
#[derive(Component)]
struct MenuEmphasis;
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
/// The chart's container (empty and hidden without a chart).
#[derive(Component)]
struct MenuChartNode;
/// A bar of the chart (tests).
#[derive(Component)]
pub struct MenuChartBar;

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
            // The veil over the ride stays see-through with high contrast.
            crate::access::AccessExempt,
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
                    font(EMPHASIS_VH),
                    TextColor(TEXT),
                    MenuEmphasis,
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
                        display: Display::None,
                        flex_direction: FlexDirection::Column,
                        row_gap: vh(CHART_GAP_VH),
                        ..default()
                    },
                    MenuChartNode,
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

fn spawn_rows(
    commands: &mut Commands,
    list: Entity,
    view: &MenuView,
    unicorn: Option<Handle<Image>>,
) {
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        for (i, label) in view.items.iter().enumerate() {
            let color = TextColor(if view.is_dimmed(i) { DISABLED } else { TEXT });
            let mark = view.mark(i).cloned().unwrap_or_default();
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
            row.with_children(|r| {
                if mark.unicorn {
                    r.spawn((
                        Node {
                            width: vh(UNICORN_MARK_VH.0),
                            height: vh(UNICORN_MARK_VH.1),
                            margin: UiRect::right(vh(0.8)),
                            ..default()
                        },
                        unicorn.clone().map(ImageNode::new).unwrap_or_default(),
                        MenuUnicornMark,
                    ));
                }
                match split_value(label) {
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
                }
                if let Some((icon, rgb)) = &mark.dot {
                    r.spawn((
                        Text::new(format!("  {icon}")),
                        font(ITEM_VH),
                        TextColor(Color::srgb(rgb[0], rgb[1], rgb[2])),
                        MenuDotMark,
                    ));
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
    mut thumbs: Option<&mut crate::thumbs::Thumbnails>,
) {
    let cell_pct = grid_cell_pct(grid.columns);
    let label_vh = grid_label_vh(grid);
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        for (i, cell) in grid.cells.iter().enumerate() {
            let dimmed = view.is_dimmed(i);
            let color = TextColor(if dimmed { DISABLED } else { TEXT });
            let mut node = l.spawn((
                Node {
                    width: Val::Percent(cell_pct),
                    // Never wider than its share (a long word would
                    // push the row's last cell onto the next row).
                    min_width: Val::ZERO,
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
            let (picture, label) = cell_parts(grid, cell);
            let thumb = match (cell.thumb.as_ref(), thumbs.as_mut()) {
                (Some(k), Some(t)) => Some((k.aspect(), t.image(k, images))),
                _ => None,
            };
            node.with_children(|c| {
                if !picture {
                    c.spawn((
                        Text::new(cell.label.clone()),
                        font(label_vh),
                        color,
                        TextLayout::justify(Justify::Center),
                    ));
                    return;
                }
                let mut icon = c.spawn(Node {
                    width: vh(GRID_ICON_VH),
                    height: vh(GRID_ICON_VH),
                    ..default()
                });
                if let Some((aspect, img)) = thumb {
                    // The rendering (square for the character's tiles),
                    // on a dark ground until it has been drawn.
                    icon.insert((
                        ImageNode::new(img),
                        BackgroundColor(Color::srgb(0.16, 0.16, 0.17)),
                    ));
                    icon.entry::<Node>().and_modify(move |mut n| {
                        n.width = vh(GRID_ICON_VH * aspect);
                        n.border_radius = BorderRadius::all(vh(0.6));
                    });
                } else if let Some(ic) = cell.icon {
                    let mut img = ImageNode::new(icon_image(ic, icons, images));
                    if dimmed {
                        img.color = Color::srgba(1.0, 1.0, 1.0, 0.45);
                    }
                    icon.insert(img);
                } else if let Some([r, g, b]) = cell.swatch {
                    icon.insert((
                        BackgroundColor(Color::srgb_u8(r, g, b)),
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                    ));
                    icon.entry::<Node>().and_modify(|mut n| {
                        n.border = UiRect::all(vh(0.3));
                        n.border_radius = BorderRadius::all(vh(GRID_ICON_VH / 2.0));
                    });
                }
                if label {
                    c.spawn((
                        Text::new(cell.label.clone()),
                        font(label_vh),
                        color,
                        TextLayout::justify(Justify::Center),
                    ));
                }
            });
            hover_and_click(&mut node, i);
        }
    });
}

/// The calendar (spec §14 Phase 9 S9a): the buttons over the grid, the
/// weekdays, the days with their bands and moon phases, the time stepper
/// and Back, the legend. Items keep their indices (days 0 – 41, then the
/// buttons, the time, Back), so focus and pointer work as on any page.
fn spawn_calendar(commands: &mut Commands, list: Entity, view: &MenuView, c: &MenuCalendar) {
    let cell_pct = grid_cell_pct(7);
    let days = c.days.len();
    let row = |gap: f32| Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Percent(GRID_GAP_PCT),
        margin: UiRect::bottom(vh(gap)),
        ..default()
    };
    let cell = |width: f32| Node {
        width: Val::Percent(width),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        padding: UiRect::all(vh(CAL_PAD_VH)),
        border: UiRect::all(vh(CAL_BORDER_VH)),
        border_radius: BorderRadius::all(vh(0.8)),
        ..default()
    };
    let colour = |i: usize| TextColor(if view.is_dimmed(i) { DISABLED } else { TEXT });
    commands.entity(list).despawn_children();
    commands.entity(list).with_children(|l| {
        // The buttons over the grid.
        l.spawn(row(CAL_SECTION_GAP_VH)).with_children(|r| {
            for k in 0..CALENDAR_BUTTONS {
                let i = days + k;
                let label = c.controls.get(k).cloned().unwrap_or_default();
                let mut b = r.spawn((
                    cell(cell_pct),
                    BackgroundColor(ROW),
                    BorderColor::all(Color::NONE),
                    MenuRow(i),
                ));
                b.with_children(|b| {
                    b.spawn((Text::new(label), font(CAL_LABEL_VH), colour(i)));
                });
                hover_and_click(&mut b, i);
            }
        });
        // The weekdays.
        l.spawn(row(CAL_ROW_GAP_VH)).with_children(|r| {
            for wd in &c.weekdays {
                r.spawn(Node {
                    width: Val::Percent(cell_pct),
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_children(|n| {
                    n.spawn((Text::new(wd.clone()), font(CAL_WEEKDAY_VH), TextColor(DIM)));
                });
            }
        });
        // Six weeks.
        for week in 0..days.div_ceil(7) {
            l.spawn(row(CAL_ROW_GAP_VH)).with_children(|r| {
                for i in week * 7..(week * 7 + 7).min(days) {
                    let d = &c.days[i];
                    let mut n = r.spawn((
                        cell(cell_pct),
                        BackgroundColor(ROW),
                        BorderColor::all(Color::NONE),
                        MenuRow(i),
                        Marked(d.selected),
                    ));
                    n.with_children(|n| {
                        let num = if d.today {
                            FOCUS
                        } else if d.outside || view.is_dimmed(i) {
                            DISABLED
                        } else {
                            TEXT
                        };
                        n.spawn(Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            padding: UiRect::horizontal(vh(0.6)),
                            // Today: underlined as well as coloured.
                            border: UiRect::bottom(vh(if d.today { 0.3 } else { 0.0 })),
                            ..default()
                        })
                        .insert(BorderColor::all(FOCUS))
                        .with_children(|t| {
                            t.spawn((Text::new(d.label.clone()), font(CAL_NUM_VH), TextColor(num)));
                            t.spawn((
                                Text::new(d.moon.clone().unwrap_or_default()),
                                font(CAL_NUM_VH),
                                TextColor(if d.outside { DISABLED } else { DIM }),
                                CalendarMoon,
                            ));
                        });
                        let band = d.band.map_or(Color::NONE, |b| {
                            let a = if d.outside { 0.45 } else { 1.0 };
                            Color::srgba(b[0], b[1], b[2], a)
                        });
                        n.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: vh(CAL_BAND_VH),
                                border_radius: BorderRadius::all(vh(0.3)),
                                ..default()
                            },
                            BackgroundColor(band),
                            CalendarBand,
                        ));
                    });
                    hover_and_click(&mut n, i);
                }
            });
        }
        // Under the grid: the time stepper (columns 0 – 2), Select (3 – 5:
        // the date picker's OK, framed and in the focus colour while it
        // may set the date) and Back (6).
        l.spawn(row(CAL_SECTION_GAP_VH)).with_children(|r| {
            let time = days + CALENDAR_BUTTONS;
            let select = time + CALENDAR_SELECT;
            for (i, width) in [
                (time, 3.0 * cell_pct + 2.0 * GRID_GAP_PCT),
                (select, 3.0 * cell_pct + 2.0 * GRID_GAP_PCT),
                (time + 2, cell_pct),
            ] {
                let label = c.controls.get(i - days).cloned().unwrap_or_default();
                let primary = i == select && !view.is_dimmed(i);
                let mut b = r.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        ..cell(width)
                    },
                    BackgroundColor(ROW),
                    BorderColor::all(Color::NONE),
                    MenuRow(i),
                    Marked(primary),
                ));
                let colour = |i: usize| {
                    if primary { TextColor(FOCUS) } else { colour(i) }
                };
                b.with_children(|b| match split_value(&label) {
                    Some((name, value)) => {
                        b.spawn((Text::new(name), font(CAL_LABEL_VH), colour(i)));
                        spawn_arrow(b, i, -1);
                        b.spawn((
                            Text::new(format!(" {value} ")),
                            font(CAL_LABEL_VH),
                            colour(i),
                        ));
                        spawn_arrow(b, i, 1);
                    }
                    None => {
                        b.spawn((Text::new(label), font(CAL_LABEL_VH), colour(i)));
                    }
                });
                hover_and_click(&mut b, i);
            }
        });
        // The legend: the bands, the moon.
        l.spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: vh(1.6),
            ..default()
        })
        .with_children(|r| {
            for (name, rgb) in &c.legend {
                r.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: vh(0.6),
                    ..default()
                })
                .with_children(|e| {
                    e.spawn((
                        Node {
                            width: vh(CAL_SWATCH_VH.0),
                            height: vh(CAL_SWATCH_VH.1),
                            border_radius: BorderRadius::all(vh(0.3)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(rgb[0], rgb[1], rgb[2])),
                    ));
                    e.spawn((Text::new(name.clone()), font(CAL_LEGEND_VH), TextColor(DIM)));
                });
            }
            r.spawn((
                Text::new(c.moon_legend.clone()),
                font(CAL_LEGEND_VH),
                TextColor(DIM),
            ));
        });
    });
}

/// A calendar day's band (tests).
#[derive(Component)]
pub struct CalendarBand;
/// A calendar day's moon glyph (tests).
#[derive(Component)]
pub struct CalendarMoon;

/// The page's layout key: rows are respawned only when it changes, not on
/// every focus move or status update.
type PageKey = (
    String,
    Vec<String>,
    Vec<bool>,
    Option<MenuGrid>,
    Vec<RowMark>,
    Option<MenuCalendar>,
);

/// The unicorn before a rider list item (spec §14 Phase 13E).
#[derive(Component)]
pub struct MenuUnicornMark;

/// The damage dot after a rider list item (spec §14 Phase 12F).
#[derive(Component)]
pub struct MenuDotMark;

/// The unicorn marker's picture (the impostor's, `unicorn::impostor_image`).
fn unicorn_mark(images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (w, h) = crate::unicorn::IMPOSTOR_PX;
    images.add(Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        crate::unicorn::impostor_image()
            .into_iter()
            .flatten()
            .collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    ))
}

/// The list's and the panel's nodes, disjoint from the root's and the
/// texts' (`Node` is borrowed mutably by several queries).
type ListFilter = (
    With<MenuList>,
    Without<MenuRoot>,
    Without<MenuStatus>,
    Without<MenuText>,
    Without<MenuEmphasis>,
);
type PanelFilter = (
    With<MenuPanel>,
    Without<MenuRoot>,
    Without<MenuList>,
    Without<MenuStatus>,
    Without<MenuText>,
    Without<MenuEmphasis>,
);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn draw_menu(
    mut commands: Commands,
    view: Res<MenuView>,
    mut page: Local<Option<PageKey>>,
    mut root: Query<(&mut Node, &mut BackgroundColor), With<MenuRoot>>,
    mut list: Query<(Entity, &mut Node), ListFilter>,
    mut panel: Query<&mut Node, PanelFilter>,
    mut rows: Query<
        (
            Ref<MenuRow>,
            &mut BackgroundColor,
            &mut BorderColor,
            Option<&Marked>,
        ),
        Without<MenuRoot>,
    >,
    (mut images, mut icons, mut unicorn, mut thumbs): (
        ResMut<Assets<Image>>,
        Local<std::collections::HashMap<ocw_weather::icon::Icon, Handle<Image>>>,
        Local<Option<Handle<Image>>>,
        Option<ResMut<crate::thumbs::Thumbnails>>,
    ),
    mut texts: ParamSet<(
        Query<&mut Text, With<MenuTitle>>,
        Query<(&mut Text, &mut Node), (With<MenuStatus>, Without<MenuRoot>)>,
        Query<(&mut Text, &mut Node), (With<MenuText>, Without<MenuRoot>)>,
        Query<&mut Text, With<MenuHint>>,
        Query<(&mut Text, &mut Node), (With<MenuEmphasis>, Without<MenuRoot>)>,
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
    let Ok((mut node, mut veil)) = root.single_mut() else {
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
    // The side panel (the character editor): docked left, the scene
    // beside it free.
    let justify = if view.side {
        JustifyContent::FlexStart
    } else {
        JustifyContent::Center
    };
    let veil_colour = Color::srgba(0.0, 0.0, 0.0, if view.side { 0.0 } else { 0.45 });
    if veil.0 != veil_colour {
        veil.0 = veil_colour;
    }
    if node.justify_content != justify {
        node.justify_content = justify;
        node.padding = if view.side {
            UiRect::left(vh(2.0))
        } else {
            UiRect::ZERO
        };
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
        view.marks.clone(),
        view.calendar.clone(),
    );
    if page.as_ref() != Some(&key) {
        if let Ok((list, mut node)) = list.single_mut() {
            // A grid wraps its cells into rows; a list is a column (the
            // calendar's rows of cells too).
            let (dir, wrap, gap) = match (&view.grid, &view.calendar) {
                (_, Some(_)) => (FlexDirection::Column, FlexWrap::NoWrap, Val::ZERO),
                (Some(_), _) => (FlexDirection::Row, FlexWrap::Wrap, vh(GRID_GAP_VH)),
                (None, None) => (FlexDirection::Column, FlexWrap::NoWrap, vh(LIST_GAP_VH)),
            };
            node.flex_direction = dir;
            node.flex_wrap = wrap;
            node.row_gap = gap;
            node.column_gap = match (&view.grid, &view.calendar) {
                (Some(_), None) => Val::Percent(GRID_GAP_PCT),
                _ => Val::ZERO,
            };
            match (&view.grid, &view.calendar) {
                (_, Some(c)) => spawn_calendar(&mut commands, list, &view, c),
                (Some(g), _) => spawn_grid(
                    &mut commands,
                    list,
                    &view,
                    g,
                    &mut icons,
                    &mut images,
                    thumbs.as_deref_mut(),
                ),
                (None, None) => {
                    let mark = view.marks.iter().any(|m| m.unicorn).then(|| {
                        unicorn
                            .get_or_insert_with(|| unicorn_mark(&mut images))
                            .clone()
                    });
                    spawn_rows(&mut commands, list, &view, mark)
                }
            }
        }
        if let Ok(mut p) = panel.single_mut() {
            p.width = vh(view_panel_vh(&view));
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
    set_text_or_hide(&mut texts.p4(), &view.emphasis.join("\n"));
}

/// The chart's bars (spawned again when the chart changes).
fn draw_chart(
    mut commands: Commands,
    view: Res<MenuView>,
    mut node: Query<(Entity, &mut Node), With<MenuChartNode>>,
    mut last: Local<Option<MenuChart>>,
) {
    let chart = view.chart.as_ref().filter(|_| view.open);
    if chart == last.as_ref() {
        return;
    }
    *last = chart.cloned();
    let Ok((e, mut n)) = node.single_mut() else {
        return;
    };
    commands.entity(e).despawn_children();
    let Some(chart) = chart else {
        n.display = Display::None;
        return;
    };
    n.display = Display::Flex;
    let width = 100.0 / chart.bars.len().max(1) as f32;
    commands.entity(e).with_children(|c| {
        if !chart.caption.is_empty() {
            c.spawn((
                Text::new(chart.caption.clone()),
                font(TEXT_VH),
                TextColor(DIM),
            ));
        }
        c.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::FlexEnd,
            ..default()
        })
        .with_children(|row| {
            for bar in &chart.bars {
                let colour = if bar.best { FOCUS } else { CHART_BAR };
                row.spawn(Node {
                    width: Val::Percent(width),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: vh(CHART_GAP_VH),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((
                        Text::new(bar.text.clone()),
                        font(CHART_TEXT_VH),
                        TextColor(TEXT),
                    ));
                    col.spawn(Node {
                        width: Val::Percent(64.0),
                        height: vh(CHART_BAR_VH),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::FlexEnd,
                        ..default()
                    })
                    .with_children(|area| {
                        area.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: vh(bar.value.clamp(0.0, 1.0) * CHART_BAR_VH),
                                border_radius: BorderRadius::top(vh(0.5)),
                                ..default()
                            },
                            BackgroundColor(colour),
                            MenuChartBar,
                        ));
                        if let Some(m) = bar.mark {
                            area.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Percent(-12.0),
                                    width: Val::Percent(124.0),
                                    bottom: vh(m.clamp(0.0, 1.0) * CHART_BAR_VH),
                                    height: vh(0.5),
                                    ..default()
                                },
                                BackgroundColor(TEXT),
                            ));
                        }
                    });
                    col.spawn((
                        Text::new(bar.label.clone()),
                        font(CHART_TEXT_VH),
                        TextColor(DIM),
                    ));
                });
            }
        });
    });
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

    /// A month calendar as World → Time → Calendar shows it (spec §14
    /// Phase 9 S9a): 42 days, seven buttons, the time, Back.
    fn calendar_view() -> MenuView {
        let days: Vec<CalendarDay> = (0..CALENDAR_DAYS)
            .map(|i| CalendarDay {
                label: format!("{}", i % 31 + 1),
                band: Some([0.9, 0.5, 0.1]),
                moon: (i % 7 == 3).then(|| "●".to_string()),
                outside: i < 3,
                today: i == 20,
                selected: i == 21,
            })
            .collect();
        let controls: Vec<String> = [
            "« 2025",
            "‹ Oct",
            "Today",
            "Dates",
            "Season",
            "Dec ›",
            "2027 »",
            "Time:  < 14:20 >",
            "Select Tue 17 Nov 2026",
            "Back",
        ]
        .map(str::to_owned)
        .to_vec();
        let mut v = page(&[], 0);
        v.title = "November 2026".into();
        v.status = "Mon 16 Nov 2026: sunrise 07:33 · sunset 16:47 · day 9 h 14 min · beech bare · snow line 1,350 m · full moon".into();
        v.items = days
            .iter()
            .map(|d| d.label.clone())
            .chain(controls.clone())
            .collect();
        v.hint = "←→ day  ↑↓ week  PgUp/PgDn, LB/RB month  Enter/A set  Esc/B back".into();
        v.calendar = Some(MenuCalendar {
            title: v.title.clone(),
            weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                .map(str::to_owned)
                .to_vec(),
            days,
            controls,
            legend: ["fresh green", "summer", "autumn colours", "bare", "snow"]
                .map(|n| (n.to_owned(), [0.5, 0.5, 0.5]))
                .to_vec(),
            moon_legend: "● full  ○ new  ◑ ◐ quarters".into(),
        });
        v
    }

    /// The calendar fits the ten-foot panel at 16:9 and 4:3 with its
    /// status line, its texts have glyphs; the swipe rule.
    #[test]
    fn the_calendar_fits_the_panel() {
        let v = calendar_view();
        for aspect in [16.0 / 9.0, 4.0 / 3.0] {
            let h = page_height_vh(&v, aspect, 0);
            assert!(h <= PANEL_MAX_VH, "{aspect}: {h}");
            assert!(h > 50.0, "{h}");
        }
        let c = v.calendar.as_ref().unwrap();
        for s in c
            .controls
            .iter()
            .chain([&c.moon_legend, &v.hint, &v.status])
        {
            assert!(crate::font::covers(s), "{s}: {:?}", crate::font::missing(s));
        }
        assert_eq!(
            swipe_dir(-200.0, 20.0, 1280.0),
            Some(1),
            "to the left: next"
        );
        assert_eq!(swipe_dir(200.0, -30.0, 1280.0), Some(-1));
        assert_eq!(swipe_dir(40.0, 0.0, 1280.0), None, "a tap");
        assert_eq!(swipe_dir(200.0, 150.0, 1280.0), None, "mostly down");
    }

    /// Drawn headless: every item a pointer target (42 days, seven
    /// buttons, the time, Select, Back), a band per day, the moon glyphs;
    /// Select framed as the page's main button.
    #[test]
    fn the_calendar_draws_headless() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            MenuPlugin,
        ));
        app.init_asset::<Image>();
        app.update();
        *app.world_mut().resource_mut::<MenuView>() = calendar_view();
        app.update();
        app.update();
        let rows = app
            .world_mut()
            .query::<&MenuRow>()
            .iter(app.world())
            .map(|r| r.0)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(rows.len(), CALENDAR_DAYS + CALENDAR_BUTTONS + 3);
        assert_eq!(
            rows.iter().max(),
            Some(&(CALENDAR_DAYS + CALENDAR_BUTTONS + 2))
        );
        let select = CALENDAR_DAYS + CALENDAR_BUTTONS + CALENDAR_SELECT;
        let framed = app
            .world_mut()
            .query::<(&MenuRow, &Marked)>()
            .iter(app.world())
            .filter(|(r, m)| m.0 && r.0 >= CALENDAR_DAYS)
            .map(|(r, _)| r.0)
            .collect::<Vec<_>>();
        assert_eq!(framed, [select]);
        let bands = app
            .world_mut()
            .query::<&CalendarBand>()
            .iter(app.world())
            .count();
        assert_eq!(bands, CALENDAR_DAYS);
        let moons = app
            .world_mut()
            .query::<(&CalendarMoon, &Text)>()
            .iter(app.world())
            .filter(|(_, t)| t.0 == "●")
            .count();
        assert_eq!(moons, 6);
        let texts = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect::<Vec<_>>();
        assert!(texts.iter().any(|t| t == "Today"));
        assert!(texts.iter().any(|t| t == "Select Tue 17 Nov 2026"));
        assert!(texts.iter().any(|t| t == "<") && texts.iter().any(|t| t == ">"));
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

    /// A grid of renderings (the character editor's tiles, spec §14 Phase
    /// 4, 4N): every tile gets the picture's room; up to two rows show the
    /// names under the pictures, a larger grid only the pictures (the
    /// status line names the focused one); Back is a name.
    #[test]
    fn rendered_tiles_get_their_pictures_and_names_when_they_fit() {
        use crate::thumbs::{RiderShot, RiderThumb, ThumbKey};
        let tile = |i: u8| GridCell {
            label: format!("Style {i}"),
            thumb: Some(ThumbKey::Rider(RiderThumb {
                look: ocw_core::RiderLook::default(),
                kit: Vec::new(),
                shot: RiderShot::Face,
            })),
            ..Default::default()
        };
        let back = GridCell {
            label: "Back".into(),
            ..Default::default()
        };
        let small = MenuGrid {
            columns: 5,
            cells: (0..7).map(tile).chain([back.clone()]).collect(),
        };
        let large = MenuGrid {
            columns: 5,
            cells: (0..21).map(tile).chain([back.clone()]).collect(),
        };
        assert!(grid_thumbs(&small) && grid_labels(&small));
        assert!(grid_thumbs(&large) && !grid_labels(&large));
        assert_eq!(cell_parts(&small, &small.cells[0]), (true, true));
        assert_eq!(cell_parts(&large, &large.cells[0]), (true, false));
        assert_eq!(cell_parts(&large, &back), (false, true));
        // The heights: a picture row each, the names only on the small one.
        let w = 60.0;
        let row = GRID_ICON_VH + 2.0 * (GRID_PAD_VH + ROW_BORDER_VH);
        let h = grid_height_vh(&large, w);
        assert!((h - (5.0 * row + 4.0 * GRID_GAP_VH)).abs() < 1e-3, "{h}");
        assert!(grid_height_vh(&small, w) > 2.0 * row);
        // A weather grid keeps its labels.
        assert!(grid_labels(&MenuGrid::default()));
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
                swatch: None,
                marked: false,
                ..Default::default()
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

    /// The training history's power curve (spec §14 Phase 7): its height
    /// is counted, and its bars are drawn.
    #[test]
    fn a_chart_is_counted_and_drawn() {
        let chart = MenuChart {
            caption: "Power curve".into(),
            bars: (0..10)
                .map(|i| ChartBar {
                    label: format!("{i}m"),
                    value: 1.0 - i as f32 * 0.08,
                    text: format!("{}", 800 - i * 50),
                    mark: Some(0.9),
                    best: i == 3,
                })
                .collect(),
        };
        let mut v = page(&["Short."], 3);
        let without = page_height_vh(&v, 16.0 / 9.0, 1);
        v.chart = Some(chart.clone());
        let with = page_height_vh(&v, 16.0 / 9.0, 1);
        assert!((with - without - chart_height_vh(&chart) - PANEL_GAP_VH).abs() < 1e-4);
        assert!(with <= PANEL_MAX_VH);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            MenuPlugin,
        ));
        app.init_asset::<Image>();
        app.update();
        *app.world_mut().resource_mut::<MenuView>() = MenuView { open: true, ..v };
        app.update();
        app.update();
        let bars = app
            .world_mut()
            .query::<&MenuChartBar>()
            .iter(app.world())
            .count();
        assert_eq!(bars, 10);
        app.world_mut().resource_mut::<MenuView>().chart = None;
        app.update();
        app.update();
        let bars = app
            .world_mut()
            .query::<&MenuChartBar>()
            .iter(app.world())
            .count();
        assert_eq!(bars, 0);
    }

    #[test]
    fn the_console_takes_the_input_before_the_menu() {
        assert_eq!(InputContext::of(false, false), InputContext::Ride);
        assert_eq!(InputContext::of(false, true), InputContext::Menu);
        assert_eq!(InputContext::of(true, false), InputContext::Console);
        assert_eq!(InputContext::of(true, true), InputContext::Console);
    }

    /// The rider list's markers (spec §14 Phase 13E, 12F): a unicorn before
    /// a flying rider, the damage dot in its colour after a hurt one.
    #[test]
    fn the_rider_list_draws_its_unicorn_and_dots() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            MenuPlugin,
        ));
        app.init_asset::<Image>();
        app.update();
        *app.world_mut().resource_mut::<MenuView>() = MenuView {
            open: true,
            title: "Riders in this ride".into(),
            items: vec![
                "Nils (you):  km 1.2".into(),
                "Kim:  flying, 20 m ahead".into(),
                "Bot 3:  down, 40 m behind".into(),
                "Back".into(),
            ],
            marks: vec![
                RowMark::default(),
                RowMark {
                    unicorn: true,
                    dot: None,
                },
                RowMark {
                    unicorn: false,
                    dot: Some(("■".into(), [0.92, 0.22, 0.2])),
                },
            ],
            ..Default::default()
        };
        app.update();
        app.update();
        let w = app.world_mut();
        let unicorns = w
            .query_filtered::<&ImageNode, With<MenuUnicornMark>>()
            .iter(w)
            .count();
        assert_eq!(unicorns, 1);
        let dots: Vec<(String, Color)> = w
            .query_filtered::<(&Text, &TextColor), With<MenuDotMark>>()
            .iter(w)
            .map(|(t, c)| (t.0.clone(), c.0))
            .collect();
        assert_eq!(dots, [("  ■".to_string(), Color::srgb(0.92, 0.22, 0.2))]);
        assert!(crate::font::covers("■▲"));
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
                swatch: None,
                marked: p.0 == "rain",
                ..Default::default()
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
