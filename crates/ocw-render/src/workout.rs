//! The workout panel of the HUD (spec §2.8, §3.2): target, time left,
//! next step, and the whole workout's profile with the progress. All
//! content comes from [`RideView::workout`] (computed in
//! `ocw_sim::workout::WorkoutPanel`); this only draws it.
//!
//! **Its place (spec 3.2 Q1, 2026-11-19):** the left column, under the
//! bike computer's panel (top left, with the damage widget in it) and
//! above the voice panel and the elevation profile – no longer in the
//! lower right, which is the navigation map's. [`fit_workout`] drops it
//! from the top of the left column to the first place clear of every HUD
//! box (the top panel, the top-right column, the event panel, the touch
//! chevrons, the voice panel, the elevation profile, the junction fan while
//! it shows and the map); where the full panel finds no room it is shown
//! **compact** (the target and the lines, without the title and the
//! profile chart), then the same in the middle column; where even that
//! finds no room the **elevation profile yields** (with the GPS line: the
//! workout's own profile replaces it while the workout runs, as on a small
//! window with a large HUD); with no room at all it is hidden rather than
//! drawn over another box.

use bevy::prelude::*;
use ocw_sim::workout::{ProfileBlock, Zone};

use crate::RideView;

pub struct WorkoutPanelPlugin;

impl Plugin for WorkoutPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorkoutPanelInfo>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (update_panel, place_panel)
                    .chain()
                    .after(crate::navi_map::MapFitSet),
            );
    }
}

/// Marks a HUD node the workout panel keeps clear of besides the map's
/// [`crate::navi_map::MapAvoid`] boxes: the voice panel.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct WorkoutAvoid;

/// The elevation profile's column (bottom left, with the GPS line): kept
/// clear of too, but hidden while the workout panel has no other room.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct YieldsToWorkout;

/// Where the workout panel is now (UI px), for the tests and the console;
/// `rect: None` while no workout runs or there is no room.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct WorkoutPanelInfo {
    pub rect: Option<Rect>,
    pub compact: bool,
    /// The elevation profile is hidden to make room.
    pub profile_yields: bool,
    /// The box on the window (logical px: left, top, width, height), for
    /// the browser page (`window.ocw.hudMap().workout`).
    pub screen: Option<[f32; 4]>,
}

/// The panel's margin from the window's edges and its gap to the other
/// HUD boxes (UI px).
pub const MARGIN_PX: f32 = 12.0;
pub const GAP_PX: f32 = 8.0;
/// The panel's padding (UI px) and the gap between its rows.
const PAD_PX: f32 = 10.0;
const ROW_GAP_PX: f32 = 3.0;
/// The title line's height (14 px font).
const TITLE_PX: f32 = 14.0 * 1.2;
/// The compact panel's padding (UI px).
const COMPACT_PAD_PX: f32 = 6.0;
/// What the compact panel leaves out: the title and the profile chart,
/// each with its row gap (the chart also its top margin), and some of
/// its padding.
pub const COMPACT_DROP_PX: f32 =
    TITLE_PX + ROW_GAP_PX + PROFILE_H + 4.0 + ROW_GAP_PX + 2.0 * (PAD_PX - COMPACT_PAD_PX);
/// The full panel's size before its first layout (UI px): the chart and
/// the padding wide, the title, the target (30 px), three lines (14 px)
/// and the chart high.
pub const FULL_SIZE: Vec2 = Vec2::new(
    PROFILE_W + 2.0 * PAD_PX,
    2.0 * PAD_PX + TITLE_PX + 30.0 * 1.2 + 3.0 * 14.0 * 1.2 + PROFILE_H + 4.0 + 3.0 * ROW_GAP_PX,
);

/// Where the panel goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkoutFit {
    pub rect: Rect,
    /// Without the title and the profile chart.
    pub compact: bool,
    /// The elevation profile (`yielding` of [`place_workout`]) is hidden.
    pub profile_yields: bool,
}

/// [`fit_workout`] clear of `avoid` and of the elevation profile's column
/// `yielding`; with no such place, clear of `avoid` alone with the
/// elevation profile hidden (`profile_yields`).
pub fn place_workout(
    window: Vec2,
    full: Vec2,
    avoid: &[Rect],
    yielding: Option<Rect>,
) -> Option<WorkoutFit> {
    let mut all = avoid.to_vec();
    all.extend(yielding);
    fit_workout(window, full, &all).or_else(|| {
        yielding?;
        fit_workout(window, full, avoid).map(|f| WorkoutFit {
            profile_yields: true,
            ..f
        })
    })
}

/// Places the workout panel of `full` size (UI px) in a window of
/// `window` UI px clear of every box of `avoid` (by [`GAP_PX`]): in the
/// left column, dropped from the top to the first free place (so under
/// the top panel), else compact there, else the same in the middle
/// column. `None`: no room (hidden).
pub fn fit_workout(window: Vec2, full: Vec2, avoid: &[Rect]) -> Option<WorkoutFit> {
    let compact = Vec2::new(full.x, (full.y - COMPACT_DROP_PX).max(1.0));
    let columns = [MARGIN_PX, ((window.x - full.x) / 2.0).floor()];
    for x in columns {
        for (size, is_compact) in [(full, false), (compact, true)] {
            if let Some(rect) = drop_down(window, x, size, avoid) {
                return Some(WorkoutFit {
                    rect,
                    compact: is_compact,
                    profile_yields: false,
                });
            }
        }
    }
    None
}

/// The highest place of a `size` box at `x` clear of `avoid`, from the
/// top margin down.
fn drop_down(window: Vec2, x: f32, size: Vec2, avoid: &[Rect]) -> Option<Rect> {
    if x < 0.0 || x + size.x > window.x - MARGIN_PX + 0.5 {
        return None;
    }
    let mut y = MARGIN_PX;
    loop {
        let r = Rect::new(x, y, x + size.x, y + size.y);
        if r.max.y > window.y - MARGIN_PX {
            return None;
        }
        let near = |b: &Rect| {
            b.min.x < r.max.x + GAP_PX
                && b.max.x + GAP_PX > r.min.x
                && b.min.y < r.max.y + GAP_PX
                && b.max.y + GAP_PX > r.min.y
        };
        match avoid
            .iter()
            .filter(|b| near(b))
            .map(|b| b.max.y + GAP_PX)
            .reduce(f32::max)
        {
            None => return Some(r),
            // Below the lowest box in the way (it always moves down).
            Some(next) => y = next.max(y + 1.0),
        }
    }
}

/// Size of the profile chart (logical pixels).
const PROFILE_W: f32 = 300.0;
const PROFILE_H: f32 = 56.0;

#[derive(Component)]
pub(crate) struct Panel;

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
                // Placed by `place_panel` (left column, under the top panel).
                position_type: PositionType::Absolute,
                top: px(MARGIN_PX),
                left: px(MARGIN_PX),
                padding: UiRect::all(px(PAD_PX)),
                flex_direction: FlexDirection::Column,
                row_gap: px(ROW_GAP_PX),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            // Hidden until placed; with no room hidden, but still laid out
            // so its size stays known.
            Visibility::Hidden,
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

/// Shows or hides the elevation profile's column.
#[allow(clippy::type_complexity)]
fn show_column(
    column: &mut Query<
        (&mut Node, &ComputedNode, &UiGlobalTransform),
        (With<YieldsToWorkout>, Without<Panel>, Without<WorkoutAvoid>),
    >,
    show: bool,
) {
    let want = if show { Display::Flex } else { Display::None };
    for (mut n, _, _) in column {
        if n.display != want {
            n.display = want;
        }
    }
}

/// Puts the panel at its place ([`place_workout`]) every frame, after the
/// navigation map took the lower right corner.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system
fn place_panel(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    view: Res<RideView>,
    map: Option<Res<crate::navi_map::NaviMapInfo>>,
    map_avoid: Option<Res<crate::navi_map::MapAvoidRects>>,
    avoid: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            &Node,
        ),
        (With<WorkoutAvoid>, Without<Panel>, Without<YieldsToWorkout>),
    >,
    mut column: Query<
        (&mut Node, &ComputedNode, &UiGlobalTransform),
        (With<YieldsToWorkout>, Without<Panel>, Without<WorkoutAvoid>),
    >,
    mut panel: Query<
        (&mut Node, &mut Visibility, &ComputedNode),
        (With<Panel>, Without<YieldsToWorkout>),
    >,
    mut parts: Query<
        &mut Node,
        (
            Or<(With<Title>, With<Profile>)>,
            Without<Panel>,
            Without<WorkoutAvoid>,
            Without<YieldsToWorkout>,
        ),
    >,
    mut info: ResMut<WorkoutPanelInfo>,
    (mut full, mut column_rect): (Local<Option<Vec2>>, Local<Option<Rect>>),
) {
    let Ok((mut node, mut vis, computed)) = panel.single_mut() else {
        return;
    };
    if view.workout.is_none() {
        if *info != WorkoutPanelInfo::default() {
            *info = WorkoutPanelInfo::default();
            show_column(&mut column, true);
        }
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = ui_scale.map_or(1.0, |s| s.0).max(0.1);
    // The UI's area (inside the safe area, spec §3.2.3).
    let win = crate::safe_area::ui_area(window, safe.as_deref()).1 / scale;
    // The full panel's size: measured while shown full, else the last.
    let measured = computed.size() * computed.inverse_scale_factor();
    if measured.x > 0.0 && node.display != Display::None && !info.compact {
        *full = Some(measured);
    }
    let size = full.unwrap_or(FULL_SIZE);
    let ui_rect = |c: &ComputedNode, t: &UiGlobalTransform| {
        let k = c.inverse_scale_factor();
        let centre = t.translation * k;
        let half = c.size() * k * 0.5;
        Rect::from_corners(centre - half, centre + half)
    };
    let mut boxes: Vec<Rect> = map_avoid.map(|m| m.0.clone()).unwrap_or_default();
    boxes.extend(
        avoid
            .iter()
            .filter(|(c, _, v, n)| v.get() && n.display != Display::None && c.size().x > 0.0)
            .map(|(c, t, _, _)| ui_rect(c, t)),
    );
    // The junction fan only while it is shown (a junction ahead): on a
    // small window with a large HUD its corner is the only room left.
    if view.junction.is_some() {
        boxes.push(crate::navi_map::fan_rect(win));
    }
    if let Some(r) = map.and_then(|m| m.rect) {
        let [x, y, w, h] = r.map(|v| v / scale);
        boxes.push(Rect::new(x, y, x + w, y + h));
    }
    // The elevation profile's box: measured while shown, else the last
    // one (so hiding it does not make room that brings it back).
    for (n, c, t) in &column {
        if n.display != Display::None && c.size().x > 0.0 {
            *column_rect = Some(ui_rect(c, t));
        }
    }
    let fit = place_workout(win, size, &boxes, *column_rect);
    show_column(&mut column, !fit.is_some_and(|f| f.profile_yields));
    let want = if fit.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *vis != want {
        *vis = want;
    }
    if let Some(f) = fit {
        let (l, t) = (px(f.rect.min.x), px(f.rect.min.y));
        if node.left != l || node.top != t {
            node.left = l;
            node.top = t;
        }
    }
    let compact = fit.is_some_and(|f| f.compact);
    let pad = UiRect::all(px(if compact { COMPACT_PAD_PX } else { PAD_PX }));
    if node.padding != pad {
        node.padding = pad;
    }
    let shown = if compact {
        Display::None
    } else {
        Display::Flex
    };
    for mut n in &mut parts {
        if n.display != shown {
            n.display = shown;
        }
    }
    let new = WorkoutPanelInfo {
        rect: fit.map(|f| f.rect),
        compact,
        profile_yields: fit.is_some_and(|f| f.profile_yields),
        screen: fit.map(|f| {
            let r = f.rect;
            [
                r.min.x * scale,
                r.min.y * scale,
                r.width() * scale,
                r.height() * scale,
            ]
        }),
    };
    if *info != new {
        *info = new;
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

    /// The other HUD boxes (UI px) of a window `w` × `h` UI px: the top
    /// panel (`top`: its width and bottom edge), the top-right column
    /// (time stamp, REC, FPS), the voice panel (if `voice`), the elevation
    /// profile with the GPS line, the junction fan (if `fan`: a junction
    /// ahead) and the navigation map where `fit_map` puts it (it keeps the
    /// fan's place free always).
    fn hud_boxes(
        w: f32,
        h: f32,
        top: (f32, f32),
        voice: bool,
        fan: bool,
    ) -> Vec<(&'static str, Rect)> {
        use crate::navi_map::{fan_rect, fit_map};
        let win = Vec2::new(w, h);
        let pw = (300.0f32 + 16.0).min(0.6 * w);
        let profile = Rect::new(12.0, h - 12.0 - 56.0 - 30.0 - 24.0, 12.0 + pw, h - 12.0);
        let top_panel = Rect::new(12.0, 12.0, 12.0 + top.0, top.1);
        let column = Rect::new(w - 12.0 - 230.0, 12.0, w - 12.0, 12.0 + 110.0);
        let mut boxes = vec![
            ("top panel", top_panel),
            ("top-right column", column),
            ("profile", profile),
        ];
        if fan {
            boxes.push(("fan", fan_rect(win)));
        }
        if voice {
            let v = Rect::new(
                12.0,
                h - 120.0 - 140.0,
                12.0 + 420.0f32.min(w - 24.0),
                h - 120.0,
            );
            boxes.push(("voice", v));
        }
        if let Some(m) = fit_map(win, fan_rect(win), Some(profile), &[top_panel, column]) {
            boxes.push(("map", m.rect));
        }
        boxes
    }

    /// Spec 3.2 Q1 (2026-11-19: "find a better location for the workout
    /// panel"): at every supported window (16:9 from 1280 × 720 to
    /// 3840 × 2160, 4:3) and HUD size (100 – 150 %) the panel never
    /// overlaps another HUD box and the map keeps its place and size. On
    /// an everyday ride it sits in full right under the top panel in the
    /// left column up to 125 %; without voice and junction fan it is
    /// always shown (compact, or with the elevation profile yielding on
    /// small windows with a large HUD); hidden only where nothing is free
    /// (voice or a junction fan on those).
    #[test]
    fn the_workout_panel_fits_every_window_beside_the_map() {
        use crate::hud::tests_support::{tall_top_panel, typical_top_panel};
        let sizes = [
            (1280.0f32, 720.0f32),
            (1366.0, 768.0),
            (1600.0, 900.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3840.0, 2160.0),
            (1024.0, 768.0),
            (1280.0, 960.0),
            (1600.0, 1200.0),
        ];
        let mut table = Vec::new();
        let mut bad = Vec::new();
        for (pw, ph) in sizes {
            for scale in [1.0f32, 1.25, 1.5] {
                let (w, h) = (pw / scale, ph / scale);
                for (case, top) in [
                    ("typical", typical_top_panel(w)),
                    ("tall", tall_top_panel(w)),
                ] {
                    for voice in [false, true] {
                        for fan in [false, true] {
                            let boxes = hud_boxes(w, h, top, voice, fan);
                            let what = format!(
                                "{pw}x{ph} at {scale}, {case} top panel (to {:.0}), voice {voice}, fan {fan}",
                                top.1
                            );
                            let profile = boxes.iter().find(|b| b.0 == "profile").map(|b| b.1);
                            let others: Vec<Rect> = boxes
                                .iter()
                                .filter(|b| b.0 != "profile")
                                .map(|b| b.1)
                                .collect();
                            let fit = place_workout(Vec2::new(w, h), FULL_SIZE, &others, profile);
                            table.push(format!(
                                "{what}: {}",
                                match fit {
                                    None => "hidden".to_string(),
                                    Some(f) => format!(
                                        "{} at {:.0},{:.0}{}",
                                        if f.compact { "compact" } else { "full" },
                                        f.rect.min.x,
                                        f.rect.min.y,
                                        if f.profile_yields {
                                            ", the elevation profile yields"
                                        } else {
                                            ""
                                        }
                                    ),
                                }
                            ));
                            if !boxes.iter().any(|b| b.0 == "map") {
                                bad.push(format!("{what}: no map"));
                            }
                            let everyday = case == "typical" && !voice && !fan;
                            let Some(f) = fit else {
                                // Without voice and a junction fan it is
                                // always shown (the elevation profile
                                // yields where needed).
                                // (1024 × 768 at 150 % with the longest
                                // lines: the top panel ends 24 px above
                                // the window's bottom.)
                                let full_top = case == "tall" && pw < 1280.0 && scale > 1.4;
                                if !fan && !voice && !full_top {
                                    bad.push(format!("{what}: hidden"));
                                }
                                continue;
                            };
                            let r = f.rect;
                            if !(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= w && r.max.y <= h) {
                                bad.push(format!("{what}: off screen {r:?}"));
                            }
                            for (name, b) in &boxes {
                                if *name == "profile" && f.profile_yields {
                                    continue;
                                }
                                if !r.intersect(*b).is_empty() {
                                    bad.push(format!("{what}: over the {name}"));
                                }
                            }
                            if everyday && scale < 1.4 {
                                if f.compact || f.profile_yields {
                                    bad.push(format!("{what}: compact"));
                                }
                                if r.min.x != MARGIN_PX || (r.min.y - (top.1 + GAP_PX)).abs() > 0.5
                                {
                                    bad.push(format!(
                                        "{what}: not right under the top panel {r:?}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        eprintln!("{}", table.join("\n"));
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// The panel goes compact, then to the middle column, then hides – it
    /// never covers a box.
    #[test]
    fn the_workout_panel_steps_aside() {
        let win = Vec2::new(1280.0, 720.0);
        let full = FULL_SIZE;
        // Nothing in the way: top left.
        let f = fit_workout(win, full, &[]).unwrap();
        assert_eq!(f.rect.min, Vec2::splat(MARGIN_PX));
        assert!(!f.compact);
        // Under a top panel 300 px high.
        let top = Rect::new(12.0, 12.0, 900.0, 300.0);
        let f = fit_workout(win, full, &[top]).unwrap();
        assert_eq!(f.rect.min, Vec2::new(MARGIN_PX, 308.0));
        // A box below leaves room only for the compact panel.
        let low = Rect::new(12.0, 308.0 + full.y - 40.0, 400.0, 720.0);
        let f = fit_workout(win, full, &[top, low]).unwrap();
        assert!(f.compact && f.rect.min.x == MARGIN_PX, "{f:?}");
        assert!(f.rect.max.y + GAP_PX <= low.min.y + 0.01);
        // The left column full: the middle one.
        let left = Rect::new(12.0, 300.0, 400.0, 720.0);
        let f = fit_workout(win, full, &[top, left]).unwrap();
        assert!(
            f.rect.min.x > 400.0 && f.rect.intersect(left).is_empty(),
            "{f:?}"
        );
        // No room at all: hidden.
        let all = Rect::new(0.0, 0.0, 1280.0, 720.0);
        assert!(fit_workout(win, full, &[all]).is_none());
        // The elevation profile yields only where nothing else is free.
        let profile = Rect::new(12.0, 600.0, 330.0, 708.0);
        let f = place_workout(win, full, &[top], Some(profile)).unwrap();
        assert!(!f.profile_yields && f.rect.min.x == MARGIN_PX);
        let right = Rect::new(400.0, 300.0, 1280.0, 720.0);
        let low = Rect::new(12.0, 380.0, 330.0, 708.0);
        let f = place_workout(win, full, &[top, right], Some(low)).unwrap();
        assert!(f.profile_yields, "{f:?}");
        assert!(f.rect.intersect(right).is_empty() && f.rect.intersect(top).is_empty());
        assert!(place_workout(win, full, &[all], Some(profile)).is_none());
    }

    /// The panel moves to its place and hides with no room or without a
    /// workout; compact leaves out the title and the chart.
    #[test]
    fn the_panel_is_placed_on_screen() {
        let mut app = app();
        app.world_mut().spawn((
            Window {
                resolution: bevy::window::WindowResolution::new(1280, 720),
                ..default()
            },
            bevy::window::PrimaryWindow,
        ));
        app.update();
        assert_eq!(app.world().resource::<WorkoutPanelInfo>().rect, None);
        app.world_mut().resource_mut::<RideView>().workout = Some(panel());
        app.update();
        let info = app.world().resource::<WorkoutPanelInfo>().clone();
        let r = info.rect.expect("placed");
        assert_eq!(r.min, Vec2::splat(MARGIN_PX), "nothing else on screen");
        let mut q = app
            .world_mut()
            .query_filtered::<(&Node, &Visibility), With<Panel>>();
        let (n, v) = q.single(app.world()).unwrap();
        assert_eq!((n.left, n.top, *v), (px(12), px(12), Visibility::Inherited));
        // A top panel and a map box below with room for the compact
        // panel only between them.
        app.insert_resource(crate::navi_map::MapAvoidRects(vec![Rect::new(
            12.0, 12.0, 1000.0, 400.0,
        )]));
        let compact_h = FULL_SIZE.y - COMPACT_DROP_PX;
        app.insert_resource(crate::navi_map::NaviMapInfo {
            rect: Some([12.0, 400.0 + 2.0 * GAP_PX + compact_h + 4.0, 300.0, 100.0]),
            ..default()
        });
        app.update();
        let info = app.world().resource::<WorkoutPanelInfo>().clone();
        assert!(info.compact, "{info:?}");
        let mut q = app
            .world_mut()
            .query_filtered::<&Node, (With<Profile>, Without<Panel>)>();
        assert_eq!(q.single(app.world()).unwrap().display, Display::None);
        // No workout: hidden again.
        app.world_mut().resource_mut::<RideView>().workout = None;
        app.update();
        let mut q = app.world_mut().query_filtered::<&Visibility, With<Panel>>();
        assert_eq!(*q.single(app.world()).unwrap(), Visibility::Hidden);
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
