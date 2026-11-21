//! Touch steering's hints (spec §3.2.3 *Visual feedback*): the chevrons
//! ‹ › at the lower left and right (40 % opacity, 15 % after 8 s without
//! a touch, a flash on a tap step), a glow along the edge of the side
//! being held, the first-ride tip and the touch power buttons (stop, −, +
//! above the right chevron while no power device is connected, TS5;
//! [`power_buttons`] is their layout, shared with the hit test in
//! `ocw_ui::touch`). Part of the HUD (hidden with `hud off`). The shared UI (`ocw_ui::touch`) decides what shows and
//! writes [`TouchHud`]; this module only draws it.

use bevy::prelude::*;
use bevy::text::Justify;

pub struct TouchHudPlugin;

impl Plugin for TouchHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchHud>()
            .init_resource::<crate::hud_fit::HudLayout>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (update, place_power_buttons));
    }
}

/// What the touch hints show this frame.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct TouchHud {
    /// The hints are shown (a touch device, or after a touch).
    pub shown: bool,
    /// No touch for a while: the chevrons are fainter.
    pub dimmed: bool,
    /// The side held now (the edge glow).
    pub left_held: bool,
    pub right_held: bool,
    /// A tap step: the chevron flashes.
    pub flash_left: bool,
    pub flash_right: bool,
    /// The first-ride tip, if it still shows.
    pub tip: Option<String>,
    /// Where the zones start (share of the window's height from the top).
    pub zone_top: f32,
    /// The touch power buttons show (TS5: hints shown and the power is
    /// the virtual trainer's).
    pub power: bool,
    /// The power button held now (drawn brighter).
    pub power_held: Option<PowerButton>,
}

/// A touch power button (spec 3.2.3 *Power without a trainer*, TS5): the
/// keyboard's ↓ (stop: brake, power 0), Page Down (−10 W) and ↑ (a tap a
/// step, a hold ramps), fed into the shared `KeyThrottle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerButton {
    Stop,
    Down,
    Up,
}

impl PowerButton {
    pub const ALL: [PowerButton; 3] = [PowerButton::Stop, PowerButton::Down, PowerButton::Up];

    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Down => "\u{2212}",
            Self::Up => "+",
        }
    }
}

/// A power button's side (CSS px, the browser's logical px: a touch
/// target of about 9 mm, spec 3.2.2 – kept at any HUD scale) and the gap
/// between them.
pub const POWER_BUTTON_PX: f32 = 56.0;
pub const POWER_GAP_PX: f32 = 12.0;
/// The smallest a power button gets (CSS px; Apple's 44 pt touch target)
/// when a portrait column lacks room.
pub const POWER_BUTTON_MIN_PX: f32 = 44.0;
/// The right chevron's distance from the bottom (share of the height) and
/// from the right edge (UI px).
const CHEV_BOTTOM: f32 = 0.36;
const CHEV_RIGHT: f32 = 36.0;

/// The power buttons' rectangles (min, max; UI px inside the UI area
/// `ui` UI px large at `ui_scale`), right-aligned with the right chevron's
/// outer edge, above the right steering zone: in landscape a row stop,
/// −, + (+ nearest the thumb) at the top, under the top-right column; in
/// portrait a column (+ at the top) along the right edge just above the
/// chevron – off the road in the middle either way. They keep below every rectangle of `avoid`
/// they would overlap (the top panel, the top-right column; UI px). The
/// renderer draws them there and the touch zones leave them out.
pub fn power_buttons(ui: Vec2, ui_scale: f32, avoid: &[Rect]) -> [(PowerButton, Rect); 3] {
    let k = ui_scale.max(0.1);
    let side = POWER_BUTTON_PX / k;
    let gap = POWER_GAP_PX / k;
    let right = ui.x - CHEV_RIGHT;
    let bottom = ui.y * (1.0 - CHEV_BOTTOM) - CHEV_H - gap;
    let portrait = ui.y > ui.x;
    let (w, h) = if portrait {
        (side, 3.0 * side + 2.0 * gap)
    } else {
        (3.0 * side + 2.0 * gap, side)
    };
    let mut top = if portrait {
        (bottom - h).max(0.0)
    } else {
        12.0
    };
    let mut sorted: Vec<Rect> = avoid.to_vec();
    sorted.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
    for a in sorted {
        let block = Rect::new(right - w, top, right, top + h);
        if !block.intersect(a).is_empty() {
            top = a.max.y + gap;
        }
    }
    // A portrait column pushed down by a panel shrinks to keep above the
    // chevron, down to the smallest touch target.
    let mut side = side;
    if portrait && top + h > bottom {
        side = ((bottom - top - 2.0 * gap) / 3.0).clamp(POWER_BUTTON_MIN_PX / k, side);
    }
    let mut out = [(PowerButton::Stop, Rect::default()); 3];
    for (i, b) in PowerButton::ALL.into_iter().enumerate() {
        let r = if portrait {
            // + at the top, stop at the bottom.
            let y0 = top + (2 - i) as f32 * (side + gap);
            Rect::new(right - side, y0, right, y0 + side)
        } else {
            let x1 = right - (2 - i) as f32 * (side + gap);
            Rect::new(x1 - side, top, x1, top + side)
        };
        out[i] = (b, r);
    }
    out
}

/// The rectangles the power buttons keep below ([`power_buttons`], UI
/// px): the top panel and the top-right column of the last frame
/// ([`crate::hud::HudBoxes`], window px), less the UI's offset `off`.
pub fn power_avoid(boxes: &crate::hud::HudBoxes, off: Vec2, ui_scale: f32) -> Vec<Rect> {
    let k = ui_scale.max(0.1);
    [boxes.top_panel, boxes.top_right]
        .into_iter()
        .flatten()
        .map(|[x, y, w, h]| {
            let min = (Vec2::new(x, y) - off) / k;
            Rect::from_corners(min, min + Vec2::new(w, h) / k)
        })
        .collect()
}

/// Where the first-ride tip starts (% of the HUD's height): in the upper
/// part of the zones, and on a compact HUD (a phone) lower, below the
/// bike computer's panel.
pub fn tip_top(compact: bool) -> f32 {
    if compact { 40.0 } else { 28.0 }
}

#[derive(Component)]
struct PowerRow;

#[derive(Component)]
struct PowerKey(PowerButton);

/// The chevrons' opacity: shown, dimmed, flashing.
pub const CHEVRON_ALPHA: f32 = 0.4;
pub const CHEVRON_DIM_ALPHA: f32 = 0.15;
pub const CHEVRON_FLASH_ALPHA: f32 = 0.9;
/// The edge glow: width (px) and opacity.
pub const GLOW_PX: f32 = 24.0;
pub const GLOW_ALPHA: f32 = 0.25;
/// The HUD's accent (the colour-blind-safe blue of the status colours).
const ACCENT: Color = Color::srgb(0.35, 0.7, 1.0);

#[derive(Component)]
struct Root;

/// A chevron; its bars know their side (`true`: the right one).
#[derive(Component)]
struct Chevron;

#[derive(Component)]
struct ChevronBar(bool);

/// The glow of a side (`true`: right) – two strips, the outer brighter.
#[derive(Component)]
struct Glow(bool);

#[derive(Component)]
struct TipBox;

#[derive(Component)]
struct TipText;

/// The chevron's size (px) and its bars.
const CHEV_W: f32 = 48.0;
const CHEV_H: f32 = 72.0;
const BAR: f32 = 8.0;

/// A chevron's two bars: (centre x, centre y, length, angle in degrees,
/// clockwise) in its box; pointing left (‹) or right (›).
pub fn chevron_bars(right: bool) -> [(f32, f32, f32, f32); 2] {
    let (dx, dy) = (CHEV_W / 2.0, CHEV_H / 2.0 - 8.0);
    let len = (dx * dx + dy * dy).sqrt();
    let angle = dy.atan2(dx).to_degrees();
    let (upper, lower) = if right {
        (angle, -angle)
    } else {
        (-angle, angle)
    };
    [
        (CHEV_W / 2.0, CHEV_H / 2.0 - dy / 2.0, len, upper),
        (CHEV_W / 2.0, CHEV_H / 2.0 + dy / 2.0, len, lower),
    ]
}

fn spawn(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
            Root,
            crate::hud::HudPanel,
            Pickable::IGNORE,
        ))
        .with_children(|r| {
            for right in [false, true] {
                // The glow: an outer strip and a fainter inner one.
                for (i, alpha) in [GLOW_ALPHA, GLOW_ALPHA * 0.4].into_iter().enumerate() {
                    let off = px(i as f32 * GLOW_PX / 2.0);
                    let mut n = Node {
                        position_type: PositionType::Absolute,
                        width: px(GLOW_PX / 2.0),
                        top: percent(15),
                        bottom: px(0),
                        display: Display::None,
                        ..default()
                    };
                    if right {
                        n.right = off;
                    } else {
                        n.left = off;
                    }
                    r.spawn((
                        n,
                        BackgroundColor(ACCENT.with_alpha(alpha)),
                        Glow(right),
                        Pickable::IGNORE,
                    ));
                }
                let mut n = Node {
                    position_type: PositionType::Absolute,
                    width: px(CHEV_W),
                    height: px(CHEV_H),
                    // Above the browser's Devices panel (bottom right).
                    bottom: percent(CHEV_BOTTOM * 100.0),
                    ..default()
                };
                if right {
                    n.right = px(CHEV_RIGHT);
                } else {
                    n.left = px(36);
                }
                r.spawn((n, Chevron, crate::navi_map::MapAvoid, Pickable::IGNORE))
                    .with_children(|c| {
                        for (x, y, len, deg) in chevron_bars(right) {
                            c.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(x - len / 2.0),
                                    top: px(y - BAR / 2.0),
                                    width: px(len),
                                    height: px(BAR),
                                    border_radius: BorderRadius::all(px(BAR / 2.0)),
                                    ..default()
                                },
                                UiTransform::from_rotation(Rot2::degrees(deg)),
                                BackgroundColor(Color::WHITE.with_alpha(CHEVRON_ALPHA)),
                                ChevronBar(right),
                                Pickable::IGNORE,
                            ));
                        }
                    });
            }
            // The tip, centred in the upper part of the zones.
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: percent(tip_top(false)),
                    left: px(0),
                    right: px(0),
                    justify_content: JustifyContent::Center,
                    display: Display::None,
                    ..default()
                },
                TipBox,
                Pickable::IGNORE,
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        padding: UiRect::axes(px(16), px(8)),
                        border_radius: BorderRadius::all(px(8)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                    Pickable::IGNORE,
                ))
                .with_child((
                    Text::new(""),
                    TextFont {
                        font_size: 20.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::justify(Justify::Center),
                    TipText,
                ));
            });
            // The touch power buttons (TS5), placed by
            // [`place_power_buttons`].
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    display: Display::None,
                    ..default()
                },
                PowerRow,
                Pickable::IGNORE,
            ))
            .with_children(|row| {
                for b in PowerButton::ALL {
                    row.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(px(2)),
                            border_radius: BorderRadius::all(px(10)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                        BorderColor::all(Color::WHITE.with_alpha(0.5)),
                        PowerKey(b),
                        Pickable::IGNORE,
                    ))
                    .with_child((
                        Text::new(b.label()),
                        TextFont {
                            font_size: if b == PowerButton::Stop { 16.0 } else { 28.0 }.into(),
                            ..default()
                        },
                        TextColor(Color::WHITE.with_alpha(0.9)),
                        Pickable::IGNORE,
                    ));
                }
            });
        });
}

/// Shows and places the power buttons ([`power_buttons`]) every frame:
/// the layout follows the window and the HUD's scale.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system
fn place_power_buttons(
    hud: Res<TouchHud>,
    layout: Res<crate::hud_fit::HudLayout>,
    scale: Option<Res<UiScale>>,
    boxes: Option<Res<crate::hud::HudBoxes>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    mut row: Query<&mut Node, (With<PowerRow>, Without<PowerKey>)>,
    mut keys: Query<(&mut Node, &mut BackgroundColor, &PowerKey), Without<PowerRow>>,
) {
    let on = hud.shown && hud.power;
    let display = if on { Display::Flex } else { Display::None };
    for mut n in &mut row {
        if n.display != display {
            n.display = display;
        }
    }
    if !on {
        return;
    }
    let k = scale.map_or(1.0, |s| s.0);
    let off = crate::safe_area::ui_offset(windows.single().ok(), safe.as_deref());
    let avoid = boxes.map(|b| power_avoid(&b, off, k)).unwrap_or_default();
    let rects = power_buttons(layout.ui_px, k, &avoid);
    for (mut n, mut bg, k) in &mut keys {
        let Some((_, r)) = rects.iter().find(|(b, _)| *b == k.0) else {
            continue;
        };
        let (left, top, w, h) = (px(r.min.x), px(r.min.y), px(r.width()), px(r.height()));
        if n.left != left || n.top != top || n.width != w || n.height != h {
            n.left = left;
            n.top = top;
            n.width = w;
            n.height = h;
        }
        let held = hud.power_held == Some(k.0);
        let want = Color::srgba(0.0, 0.0, 0.0, if held { 0.75 } else { 0.45 });
        if bg.0 != want {
            bg.0 = want;
        }
    }
}

/// A chevron's opacity.
pub fn chevron_alpha(hud: &TouchHud, right: bool) -> f32 {
    let (flash, held) = if right {
        (hud.flash_right, hud.right_held)
    } else {
        (hud.flash_left, hud.left_held)
    };
    if flash || held {
        CHEVRON_FLASH_ALPHA
    } else if hud.dimmed {
        CHEVRON_DIM_ALPHA
    } else {
        CHEVRON_ALPHA
    }
}

#[allow(clippy::type_complexity)] // Bevy queries
fn update(
    hud: Res<TouchHud>,
    mut root: Query<&mut Node, (With<Root>, Without<Glow>, Without<TipBox>)>,
    mut glows: Query<(&mut Node, &Glow), (Without<Root>, Without<TipBox>)>,
    mut bars: Query<(&mut BackgroundColor, &ChevronBar)>,
    mut tip_box: Query<&mut Node, (With<TipBox>, Without<Root>, Without<Glow>)>,
    mut tip_text: Query<&mut Text, With<TipText>>,
    layout: Res<crate::hud_fit::HudLayout>,
) {
    if !hud.is_changed() && !layout.is_changed() {
        return;
    }
    let show = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut root {
        n.display = show(hud.shown);
    }
    if !hud.shown {
        return;
    }
    for (mut n, g) in &mut glows {
        n.display = show(if g.0 { hud.right_held } else { hud.left_held });
        n.top = percent(hud.zone_top * 100.0);
    }
    for (mut c, b) in &mut bars {
        c.0 = Color::WHITE.with_alpha(chevron_alpha(&hud, b.0));
    }
    for mut n in &mut tip_box {
        n.display = show(hud.tip.is_some());
        n.top = percent(tip_top(layout.compact));
        // In portrait on a phone the power buttons' column is at the
        // right edge: the tip keeps left of it.
        n.right = percent(if layout.narrow && hud.power {
            22.0
        } else {
            0.0
        });
    }
    if let (Ok(mut t), Some(tip)) = (tip_text.single_mut(), &hud.tip)
        && t.0 != *tip
    {
        t.0 = tip.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chevrons_point_out_and_fade_without_touches() {
        // ‹: the upper bar rises to the right (negative angle), › mirrors.
        let [up, low] = chevron_bars(false);
        assert!(up.3 < 0.0 && low.3 > 0.0 && up.1 < low.1);
        let [up_r, low_r] = chevron_bars(true);
        assert_eq!((up_r.3, low_r.3), (-up.3, -low.3));
        let mut hud = TouchHud {
            shown: true,
            ..TouchHud::default()
        };
        assert_eq!(chevron_alpha(&hud, false), CHEVRON_ALPHA);
        hud.dimmed = true;
        assert_eq!(chevron_alpha(&hud, true), CHEVRON_DIM_ALPHA);
        hud.flash_right = true;
        assert_eq!(chevron_alpha(&hud, true), CHEVRON_FLASH_ALPHA);
        assert_eq!(chevron_alpha(&hud, false), CHEVRON_DIM_ALPHA);
        hud.left_held = true;
        assert_eq!(chevron_alpha(&hud, false), CHEVRON_FLASH_ALPHA);
    }

    /// The power buttons (TS5) sit right above the right chevron – a row
    /// stop, −, + in landscape, a column along the right edge in portrait
    /// –, inside the window, 56 CSS px at any HUD scale, at phone and
    /// desktop sizes; they keep below a panel in their way.
    #[test]
    fn the_power_buttons_sit_above_the_right_chevron() {
        for (ui, scale) in [
            (Vec2::new(1280.0, 720.0), 1.0),
            (Vec2::new(1920.0, 1080.0), 1.5),
            (Vec2::new(393.0, 566.0) / 0.7278, 0.7278),
            (Vec2::new(393.0, 759.0) / 0.7278, 0.7278),
            (Vec2::new(734.0, 372.0) / 0.75, 0.75),
        ] {
            let r = power_buttons(ui, scale, &[]);
            let kinds: Vec<_> = r.iter().map(|(b, _)| *b).collect();
            assert_eq!(kinds, PowerButton::ALL);
            let chev_top = ui.y * (1.0 - CHEV_BOTTOM) - CHEV_H;
            let portrait = ui.y > ui.x;
            for (i, (_, b)) in r.iter().enumerate() {
                assert!((b.width() * scale - POWER_BUTTON_PX).abs() < 0.01, "{ui}");
                assert!((b.height() * scale - POWER_BUTTON_PX).abs() < 0.01, "{ui}");
                assert!(
                    b.min.x >= 0.0 && b.max.x <= ui.x && b.min.y >= 0.0,
                    "{ui}: {b:?}"
                );
                assert!(b.max.y < chev_top, "{ui}: above the chevron");
                if i > 0 {
                    let prev = r[i - 1].1;
                    let gap = if portrait {
                        prev.min.y - b.max.y
                    } else {
                        b.min.x - prev.max.x
                    } * scale;
                    assert!((gap - POWER_GAP_PX).abs() < 0.01, "{ui}: {gap}");
                }
            }
            // + nearest the thumb, in line with the chevron's outer edge.
            assert!((r[2].1.max.x - (ui.x - CHEV_RIGHT)).abs() < 0.01);
            // Off the middle of the window (the road): in portrait along
            // the right edge.
            let left = r.iter().map(|(_, b)| b.min.x).fold(f32::MAX, f32::min);
            assert!(
                left > ui.x * if portrait { 0.75 } else { 0.4 },
                "{ui}: {left}"
            );
            // A panel in the way pushes them below it (a portrait column
            // shrinks to stay above the chevron, not below 44 CSS px) …
            let panel = Rect::new(0.0, 0.0, ui.x, r[2].1.min.y + 10.0);
            let below = power_buttons(ui, scale, &[panel]);
            assert!(below.iter().all(|(_, b)| b.min.y > panel.max.y), "{ui}");
            if portrait {
                let side = below[0].1.width() * scale;
                assert!(
                    (POWER_BUTTON_MIN_PX - 0.01..POWER_BUTTON_PX).contains(&side),
                    "{ui}: {side}"
                );
                assert!(below[0].1.max.y < chev_top, "{ui}");
            }
            // … one beside them does not.
            let beside = Rect::new(0.0, 0.0, 10.0, ui.y);
            assert_eq!(power_buttons(ui, scale, &[beside]), r);
        }
        // The boxes the HUD measured, in window px, to UI px.
        let boxes = crate::hud::HudBoxes {
            top_panel: Some([9.0, 68.0, 375.0, 89.0]),
            top_right: Some([200.0, 161.0, 184.0, 15.0]),
            ..Default::default()
        };
        let a = power_avoid(&boxes, Vec2::new(0.0, 59.0), 0.5);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], Rect::new(18.0, 18.0, 768.0, 196.0));
    }

    /// The hints draw headless and hide with the setting.
    #[test]
    fn the_hints_show_and_hide() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(TouchHudPlugin);
        app.update();
        let root_display = |app: &mut App| {
            let mut q = app.world_mut().query_filtered::<&Node, With<Root>>();
            q.single(app.world()).unwrap().display
        };
        assert_eq!(root_display(&mut app), Display::None);
        app.world_mut().resource_mut::<TouchHud>().shown = true;
        app.world_mut().resource_mut::<TouchHud>().right_held = true;
        app.world_mut().resource_mut::<TouchHud>().tip = Some("Tap".into());
        app.update();
        assert_eq!(root_display(&mut app), Display::Flex);
        let mut q = app.world_mut().query::<(&Node, &Glow)>();
        let lit: Vec<bool> = q
            .iter(app.world())
            .filter(|(n, _)| n.display == Display::Flex)
            .map(|(_, g)| g.0)
            .collect();
        assert_eq!(lit, [true, true], "both strips of the right glow");
        let mut q = app.world_mut().query::<&Text>();
        assert!(q.iter(app.world()).any(|t| t.0 == "Tap"));
        let row_display = |app: &mut App| {
            let mut q = app.world_mut().query_filtered::<&Node, With<PowerRow>>();
            q.single(app.world()).unwrap().display
        };
        assert_eq!(
            row_display(&mut app),
            Display::None,
            "no power buttons by default"
        );
        app.world_mut().resource_mut::<TouchHud>().power = true;
        app.update();
        assert_eq!(row_display(&mut app), Display::Flex);
        let mut q = app.world_mut().query::<(&Node, &PowerKey)>();
        let placed: Vec<_> = q.iter(app.world()).map(|(n, k)| (k.0, n.width)).collect();
        assert_eq!(placed.len(), 3);
        assert!(
            placed.iter().all(|(_, w)| *w == px(POWER_BUTTON_PX)),
            "{placed:?}"
        );
        app.world_mut().resource_mut::<TouchHud>().shown = false;
        app.update();
        assert_eq!(
            row_display(&mut app),
            Display::None,
            "hidden with the hints"
        );
    }
}
