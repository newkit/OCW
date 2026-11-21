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
            .add_systems(Update, (update_overlay, update_notice, place_notice));
    }
}

/// Overlay size and the arrows' pivot (bottom centre) in pixels.
const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 160.0;
const PIVOT_Y: f32 = 108.0;
const ARROW_LEN: f32 = 82.0;

/// The overlay's distance from the window's bottom (px).
const BOTTOM: f32 = 6.0;

/// Where the fan is on a window of `window` logical px with the HUD scale
/// `ui_scale` (Bevy's `UiScale`): its rectangle, the arrows' pivot (all
/// in logical px) and the arrows' length. Touch taps on the arrows choose
/// the exit (spec §3.2.3 Q5); this is the layout that is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FanGeometry {
    pub min: Vec2,
    pub max: Vec2,
    pub pivot: Vec2,
    pub arrow_len: f32,
}

pub fn fan_geometry(window: Vec2, ui_scale: f32) -> FanGeometry {
    let s = ui_scale.max(0.1);
    let (w, h) = (window.x / s, window.y / s);
    let min = Vec2::new(w / 2.0 - WIDTH / 2.0, h - BOTTOM - HEIGHT);
    FanGeometry {
        min: min * s,
        max: Vec2::new(w / 2.0 + WIDTH / 2.0, h - BOTTOM) * s,
        pivot: Vec2::new(w / 2.0, min.y + PIVOT_Y) * s,
        arrow_len: ARROW_LEN * s,
    }
}

#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct Fan;

#[derive(Component)]
struct Label;

/// The exit keys above the fan (spec 3.2.5: "Del/Q  < exit >  PgDn/E").
#[derive(Component)]
struct KeyHint;

#[derive(Component)]
struct NoticeText;

/// The notice's box (hidden while there is no notice).
#[derive(Component)]
struct NoticeBox;

/// The notice's row (placed under the bike computer's panel).
#[derive(Component)]
struct NoticeRow;

/// The gap between the bike computer's panel and the notice (px).
const NOTICE_GAP_PX: f32 = 8.0;

/// Where the notice starts (UI px from the top): under the bike computer's
/// panel (`panel_bottom`, UI px; `None` while the HUD is hidden or not laid
/// out yet), never higher than [`NOTICE_TOP_PX`]. The panel grows with the
/// damage widget, always shown while damage is on (spec 14 Phase 12F): a
/// fixed place ran over its meters and labels.
pub fn notice_top(panel_bottom: Option<f32>) -> f32 {
    panel_bottom.map_or(NOTICE_TOP_PX, |b| (b + NOTICE_GAP_PX).max(NOTICE_TOP_PX))
}

/// Where the notice box starts (px from the top): below the bike
/// computer's panel with the damage widget (about 220 px), never over it.
pub const NOTICE_TOP_PX: f32 = 240.0;
/// The notice's shadow offset (px): small, so the text never reads as a
/// doubled, garbled line (it did at Bevy's default 4 px over a light sky).
pub const NOTICE_SHADOW_PX: f32 = 1.5;

fn spawn_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(BOTTOM),
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
            // The exit keys just above the fan, while the choice is open.
            o.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    width: px(WIDTH),
                    bottom: px(HEIGHT + 2.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                children![(
                    Text::new(""),
                    TextFont {
                        font_size: 14.0.into(),
                        ..default()
                    },
                    TextColor(Color::srgba(0.95, 0.95, 0.95, 0.85)),
                    TextShadow::default(),
                    TextLayout::justify(Justify::Center),
                    KeyHint,
                )],
            ));
        });
    // Notices ("Dead end in 800 m", "U-turn", "Crash! …") below the HUD
    // panel, on a dark box so they read over any sky.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(NOTICE_TOP_PX),
                left: px(0),
                right: px(0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            NoticeRow,
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    display: Display::None,
                    padding: UiRect::axes(px(14), px(6)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                NoticeBox,
            ))
            .with_child((
                Text::new(""),
                TextFont {
                    font_size: 22.0.into(),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.85, 0.3)),
                TextShadow {
                    offset: Vec2::splat(NOTICE_SHADOW_PX),
                    ..default()
                },
                NoticeText,
            ));
        });
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
    match j.route {
        Some(r) if r == j.selected => s.push_str("  [route]"),
        Some(_) => s.push_str("  [leaves the route]"),
        None => {}
    }
    s
}

/// Colour of the route's arrow when another exit is selected.
const ROUTE_COLOUR: Color = Color::srgb(0.3, 0.75, 1.0);

/// Colour and width of exit `i`'s arrow: selected (green when locked,
/// yellow before; on a route ride the route's arrow is blue, and so is the
/// selected one while it is the route's), others grey; dead ends reddish.
pub fn arrow_style(j: &JunctionView, i: usize) -> (Color, f32) {
    let e = &j.exits[i];
    let on_route = j.route == Some(i);
    if i == j.selected {
        let c = if j.locked {
            Color::srgb(0.2, 0.95, 0.35)
        } else if on_route {
            ROUTE_COLOUR
        } else {
            Color::srgb(1.0, 0.9, 0.25)
        };
        (c, 14.0)
    } else if on_route {
        (ROUTE_COLOUR.with_alpha(0.85), 11.0)
    } else if e.dead_end {
        (Color::srgba(0.95, 0.4, 0.35, 0.8), 9.0)
    } else {
        (Color::srgba(0.85, 0.85, 0.85, 0.7), 9.0)
    }
}

/// One arrow of the fan: (angle in degrees, selected, locked, dead end,
/// the route's).
type FanArrow = (i32, bool, bool, bool, bool);

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
                j.route == Some(i),
            )
        })
        .collect()
}

/// Moves the notice under the bike computer's panel ([`notice_top`]).
fn place_notice(
    show: Option<Res<crate::hud::ShowHud>>,
    panel: Query<(&ComputedNode, &UiGlobalTransform), With<crate::hud::HudTopPanel>>,
    mut row: Query<&mut Node, With<NoticeRow>>,
) {
    let bottom = panel
        .single()
        .ok()
        .filter(|_| show.is_none_or(|s| s.0))
        .filter(|(c, _)| c.size().y > 0.0)
        .map(|(c, t)| (t.translation.y + c.size().y * 0.5) * c.inverse_scale_factor());
    let want = px(notice_top(bottom));
    for mut n in &mut row {
        if n.top != want {
            n.top = want;
        }
    }
}

/// The notice line and its box (hidden without a notice).
fn update_notice(
    view: Res<RideView>,
    mut notice: Query<&mut Text, With<NoticeText>>,
    mut notice_box: Query<&mut Node, With<NoticeBox>>,
) {
    let want = view.notice.clone().unwrap_or_default();
    if let Ok(mut t) = notice.single_mut()
        && t.0 != want
    {
        t.0 = want.clone();
    }
    if let Ok(mut n) = notice_box.single_mut() {
        let d = if want.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if n.display != d {
            n.display = d;
        }
    }
}

/// The fan's street line and its key hint (two texts of one overlay).
type LabelText<'w, 's> =
    Query<'w, 's, &'static mut Text, (With<Label>, Without<NoticeText>, Without<KeyHint>)>;
type HintText<'w, 's> =
    Query<'w, 's, &'static mut Text, (With<KeyHint>, Without<NoticeText>, Without<Label>)>;

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn update_overlay(
    mut commands: Commands,
    view: Res<RideView>,
    mut overlay: Query<&mut Visibility, With<Overlay>>,
    fan: Query<Entity, With<Fan>>,
    mut label_text: LabelText,
    mut hint_text: HintText,
    mut shown: Local<Option<Vec<FanArrow>>>,
    hud: Option<Res<crate::hud::ShowHud>>,
    keys: Option<Res<crate::hud::RideKeys>>,
    touch: Option<Res<crate::touch_hud::TouchHud>>,
) {
    let (Ok(mut vis), Ok(fan)) = (overlay.single_mut(), fan.single()) else {
        return;
    };
    // `hud off` hides the fan too (clean pictures and videos).
    let hud_off = hud.is_some_and(|h| !h.0);
    let Some(j) = view.junction.as_ref().filter(|_| !hud_off) else {
        *vis = Visibility::Hidden;
        *shown = None;
        return;
    };
    *vis = Visibility::Inherited;
    if let Ok(mut t) = label_text.single_mut() {
        t.0 = label(j);
    }
    if let Ok(mut t) = hint_text.single_mut() {
        // Not once locked (nothing to choose), nor while the touch hints
        // show (a tap on an arrow chooses it).
        let touching = touch.is_some_and(|t| t.shown);
        let hint = match (&keys, j.locked || touching) {
            (_, true) => String::new(),
            (Some(k), false) => k.junction.clone(),
            (None, false) => crate::hud::JUNCTION_KEYS.to_string(),
        };
        if t.0 != hint {
            t.0 = hint;
        }
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

    /// Regression (K4 / K5 screenshots): the crash line looked doubled –
    /// Bevy's default 4 px shadow under yellow text on a light sky, partly
    /// under the HUD panel. Now one line on a dark box below the panel,
    /// a hair of shadow, and no box without a notice.
    #[test]
    fn the_notice_is_one_line_on_a_box_below_the_hud() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .add_plugins(JunctionPlugin);
        app.update();
        let state = |app: &mut App| {
            let w = app.world_mut();
            let shown = w
                .query_filtered::<&Node, With<NoticeBox>>()
                .single(w)
                .unwrap()
                .display;
            let (text, shadow) = w
                .query_filtered::<(&Text, &TextShadow), With<NoticeText>>()
                .single(w)
                .map(|(t, s)| (t.0.clone(), s.offset))
                .unwrap();
            (shown, text, shadow)
        };
        assert_eq!(state(&mut app).0, Display::None);
        app.world_mut().resource_mut::<RideView>().notice =
            Some("Crash! (wheel touch) - back on the bike in 3.4 s".into());
        app.update();
        let (shown, text, shadow) = state(&mut app);
        assert_eq!(shown, Display::Flex);
        assert_eq!(text, "Crash! (wheel touch) - back on the bike in 3.4 s");
        assert!(shadow.x <= 2.0 && shadow.y <= 2.0, "{shadow:?}");
        let w = app.world_mut();
        assert_eq!(
            w.query_filtered::<&Text, With<NoticeText>>()
                .iter(w)
                .count(),
            1,
            "one notice text"
        );
        app.world_mut().resource_mut::<RideView>().notice = None;
        app.update();
        assert_eq!(state(&mut app).0, Display::None);
    }

    /// Regression (spec 14 Phase 12F, 2026-11-19 screenshots): with the
    /// damage widget always shown the bike computer's panel reaches below
    /// 240 px, and the puncture notice ran over the widget's words. The
    /// notice now starts under the panel (never higher than before).
    #[test]
    fn the_notice_moves_under_a_taller_hud_panel() {
        assert_eq!(notice_top(None), NOTICE_TOP_PX);
        assert_eq!(notice_top(Some(150.0)), NOTICE_TOP_PX, "a short panel");
        assert_eq!(notice_top(Some(296.0)), 296.0 + NOTICE_GAP_PX);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .insert_resource(crate::hud::ShowHud(true))
            .add_plugins(JunctionPlugin);
        app.update();
        // A laid-out panel 284 px high whose centre is at 154 px (from 12
        // to 296 px), at a scale factor of 1.
        let computed = ComputedNode {
            size: Vec2::new(820.0, 284.0),
            inverse_scale_factor: 1.0,
            ..default()
        };
        app.world_mut().spawn((
            crate::hud::HudTopPanel,
            computed,
            UiGlobalTransform::from_translation(Vec2::new(422.0, 154.0)),
        ));
        app.update();
        let top = |app: &mut App| {
            let w = app.world_mut();
            w.query_filtered::<&Node, With<NoticeRow>>()
                .single(w)
                .unwrap()
                .top
        };
        assert_eq!(top(&mut app), px(296.0 + NOTICE_GAP_PX));
        // With the HUD off the notice goes back to its place.
        app.world_mut().resource_mut::<crate::hud::ShowHud>().0 = false;
        app.update();
        assert_eq!(top(&mut app), px(NOTICE_TOP_PX));
    }

    /// Regression (drone videos, 2026-11-17): the fan stayed on screen
    /// with `hud off`.
    #[test]
    fn hud_off_hides_the_fan() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .insert_resource(crate::hud::ShowHud(true))
            .add_plugins(JunctionPlugin);
        app.world_mut().resource_mut::<RideView>().junction = Some(view());
        app.update();
        app.update();
        let vis = |app: &mut App| {
            let w = app.world_mut();
            *w.query_filtered::<&Visibility, With<Overlay>>()
                .single(w)
                .unwrap()
        };
        assert_eq!(vis(&mut app), Visibility::Inherited);
        app.world_mut().resource_mut::<crate::hud::ShowHud>().0 = false;
        app.update();
        assert_eq!(vis(&mut app), Visibility::Hidden);
        app.world_mut().resource_mut::<crate::hud::ShowHud>().0 = true;
        app.update();
        assert_eq!(vis(&mut app), Visibility::Inherited);
    }

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
            route: None,
        }
    }

    #[test]
    fn route_rides_mark_the_route_exit_and_leaving_it() {
        let mut j = view();
        j.route = Some(2);
        j.default = 2;
        assert!(label(&j).ends_with("[leaves the route]"), "{}", label(&j));
        let (route_arrow, _) = arrow_style(&j, 2);
        let (plain, _) = arrow_style(&j, 0);
        assert_ne!(route_arrow, plain);
        j.selected = 2;
        assert!(label(&j).ends_with("[route]"));
        assert_eq!(arrow_style(&j, 2).0, ROUTE_COLOUR);
        assert_ne!(fan_key(&j), fan_key(&view()));
        // Locked: green as always.
        j.locked = true;
        assert_ne!(arrow_style(&j, 2).0, ROUTE_COLOUR);
    }

    /// The touch targets are where the fan is drawn (spec §3.2.3 Q5):
    /// bottom centre, scaled with the HUD.
    #[test]
    fn the_fan_geometry_follows_the_window_and_the_hud_scale() {
        let g = fan_geometry(Vec2::new(1280.0, 720.0), 1.0);
        assert_eq!(g.min, Vec2::new(490.0, 554.0));
        assert_eq!(g.max, Vec2::new(790.0, 714.0));
        assert_eq!(g.pivot, Vec2::new(640.0, 662.0));
        assert_eq!(g.arrow_len, ARROW_LEN);
        let big = fan_geometry(Vec2::new(1280.0, 720.0), 1.5);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(big.max.y, 720.0 - 1.5 * BOTTOM), "{big:?}");
        assert!(near(big.max.x - big.min.x, 1.5 * WIDTH), "{big:?}");
        assert!(near(big.pivot.x, 640.0), "{big:?}");
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
