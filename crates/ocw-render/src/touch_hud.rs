//! The touch controls' drawing (spec §3.2.3 *Visual feedback*, *The
//! on-screen controls*): the chevrons ‹ › at the lower left and right
//! (40 % opacity, 15 % after 8 s without a touch, a flash on a tap step),
//! a glow along the edge of the side being held, the first-ride tip; the
//! **menu button** (☰) and the **hide button** (an eye) beside it
//! ([`corner_buttons`] is their layout, shared with the hit test in
//! `ocw_ui::touch`), always drawn on a touch device; the marks of the
//! **faster / slower zones** (the top and bottom quarter of the screen,
//! which replaced the + / − / stop buttons) and their feedback ("▲ 220 W"
//! at the touched edge). With the controls hidden ([`TouchHud::hidden`])
//! only the two buttons and the feedback are drawn – every touch area
//! still works. Part of the HUD (hidden with `hud off`). The shared UI
//! (`ocw_ui::touch`) decides what shows and writes [`TouchHud`]; this
//! module only draws it.

use bevy::prelude::*;
use bevy::text::Justify;

pub struct TouchHudPlugin;

impl Plugin for TouchHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchHud>()
            .init_resource::<crate::hud_fit::HudLayout>()
            .init_resource::<TouchLayout>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (update, place_buttons, place_power, place_tip));
    }
}

/// What the touch controls show this frame.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct TouchHud {
    /// Touch is in use (a touch device, or after a touch): the HUD is in
    /// its touch form and the hints are drawn unless [`Self::hidden`].
    pub shown: bool,
    /// The rider hid the on-screen controls (the eye button): chevrons,
    /// zone marks and tips are not drawn; the touch areas still work.
    pub hidden: bool,
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
    /// Where the steering zones start and end (shares of the UI area's
    /// height from the top; the glow's span).
    pub zone_top: f32,
    pub zone_bottom: f32,
    /// The menu and hide buttons show (a touch device, also while the
    /// controls are hidden and over the open menu).
    pub buttons: bool,
    /// The menu is open: only the menu button stays, above the veil.
    pub menu_open: bool,
    /// The button a finger is on (drawn darker).
    pub button_held: Option<CornerButton>,
    /// The faster / slower zones are on (the power is the virtual
    /// trainer's): where the top zone ends and the bottom zone starts
    /// (shares of the UI area's height).
    pub power: Option<(f32, f32)>,
    /// The zone held now (the glow along that edge).
    pub power_held: Option<PowerEdge>,
    /// The feedback of a step or a hold: the edge, the text ("▲ 220 W")
    /// and where across the UI area (a share of its width).
    pub power_flash: Option<(PowerEdge, String, f32)>,
    /// A short note beside the buttons ("Controls hidden – they still
    /// work", the first time).
    pub toast: Option<String>,
}

impl TouchHud {
    /// The hints (chevrons, zone marks, tips) are drawn.
    pub fn hints(&self) -> bool {
        self.shown && !self.hidden
    }

    /// Anything of the hints' layer is drawn (the hints or the feedback
    /// of a touch, which shows with hidden controls too).
    fn layer(&self) -> bool {
        self.hints()
            || self.left_held
            || self.right_held
            || self.power_held.is_some()
            || self.power_flash.is_some()
    }
}

/// An edge of the screen with a power zone: the top is faster, the
/// bottom slower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEdge {
    Top,
    Bottom,
}

impl PowerEdge {
    pub const ALL: [PowerEdge; 2] = [PowerEdge::Top, PowerEdge::Bottom];

    /// The feedback's arrow.
    pub fn arrow(self) -> &'static str {
        match self {
            Self::Top => "\u{25b2}",
            Self::Bottom => "\u{25bc}",
        }
    }

    /// The zone's mark.
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "\u{25b2} faster",
            Self::Bottom => "\u{25bc} slower",
        }
    }
}

/// The two buttons that are always drawn on a touch device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerButton {
    /// ☰: opens (and closes) the menu, as Esc / gamepad Start.
    Menu,
    /// The eye: hides / shows the other on-screen controls.
    Hide,
}

impl CornerButton {
    pub const ALL: [CornerButton; 2] = [CornerButton::Menu, CornerButton::Hide];
}

/// A corner button's side (CSS px, the browser's logical px: above
/// Apple's 44 pt and Android's 48 dp touch targets, kept at any HUD
/// scale), the gap between the two and their distance from the UI area's
/// edges (the safe area: clear of the notch).
pub const CORNER_BUTTON_PX: f32 = 48.0;
pub const CORNER_GAP_PX: f32 = 8.0;
pub const CORNER_MARGIN_PX: f32 = 12.0;
/// The right chevron's distance from the bottom (share of the height) and
/// from the right edge (UI px).
const CHEV_BOTTOM: f32 = 0.36;
const CHEV_RIGHT: f32 = 36.0;

/// The menu and hide buttons' rectangles (min, max; UI px inside the UI
/// area `ui` UI px large at `ui_scale`): a row ☰, eye at the top left –
/// below every rectangle of `avoid` it would overlap (the HUD's top
/// panel; UI px), and right of the left chevron's column if that pushed
/// it down to the chevron (a small phone in landscape). The renderer
/// draws them there and the touch zones leave them out.
pub fn corner_buttons(ui: Vec2, ui_scale: f32, avoid: &[Rect]) -> [(CornerButton, Rect); 2] {
    let k = ui_scale.max(0.1);
    let (side, gap, margin) = (
        CORNER_BUTTON_PX / k,
        CORNER_GAP_PX / k,
        CORNER_MARGIN_PX / k,
    );
    let w = 2.0 * side + gap;
    let mut left = margin;
    let mut top = margin;
    let mut sorted: Vec<Rect> = avoid.to_vec();
    sorted.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
    for a in &sorted {
        let block = Rect::new(left, top, left + w, top + side);
        if !block.intersect(*a).is_empty() {
            top = a.max.y + gap;
        }
    }
    let chev_top = ui.y * (1.0 - CHEV_BOTTOM) - CHEV_H;
    if top + side + gap > chev_top && top < chev_top + CHEV_H {
        left = CHEV_LEFT + CHEV_W + gap;
    }
    // Never off the UI area (a tiny window).
    top = top.min((ui.y - side - margin).max(0.0));
    [
        (
            CornerButton::Menu,
            Rect::new(left, top, left + side, top + side),
        ),
        (
            CornerButton::Hide,
            Rect::new(left + side + gap, top, left + w, top + side),
        ),
    ]
}

/// The rectangles the corner buttons and the first-ride tip keep below
/// ([`corner_buttons`], [`tip_place`]; UI px): the top panel and the
/// top-right column of the last frame ([`crate::hud::HudBoxes`], window
/// px), less the UI's offset `off`.
pub fn top_boxes(boxes: &crate::hud::HudBoxes, off: Vec2, ui_scale: f32) -> Vec<Rect> {
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

/// The left chevron's distance from the left edge (UI px).
const CHEV_LEFT: f32 = 36.0;
/// The gap between the first-ride tip and its neighbours (CSS px).
pub const TIP_GAP_PX: f32 = 8.0;
/// The slot of the page's screen tip (full screen / Add to Home Screen,
/// a small banner of one or two lines; CSS px high) above the first-ride
/// tip in portrait.
pub const BANNER_PX: f32 = 44.0;

/// Where the first-ride tip goes (UI px): below the HUD's top boxes, in
/// a band of its own – in portrait over the width below the menu and
/// hide buttons (the page's screen-tip banner in a slot right above it),
/// in landscape between the chevrons and right of the two buttons (the
/// banner goes to the bottom there, the page places it) – above the
/// rider in the middle, clear of the chevrons, the buttons and the
/// panels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TipPlace {
    /// The tip's top edge and its horizontal span (it is centred in it).
    pub top: f32,
    pub left: f32,
    pub right: f32,
    /// The screen tip's slot (portrait).
    pub banner: Option<Rect>,
}

/// [`TipPlace`] in the UI area `ui` (UI px at `ui_scale`), below the
/// boxes of `avoid` (the top panel and the top-right column, UI px,
/// [`top_boxes`]) and clear of the corner buttons `buttons` (while
/// shown).
pub fn tip_place(
    ui: Vec2,
    ui_scale: f32,
    avoid: &[Rect],
    buttons: Option<&[(CornerButton, Rect)]>,
) -> TipPlace {
    let k = ui_scale.max(0.1);
    let gap = TIP_GAP_PX / k;
    let portrait = ui.y > ui.x;
    let mut top = avoid.iter().map(|r| r.max.y).fold(0.0, f32::max) + gap;
    let (mut left, right) = if portrait {
        (gap, ui.x - gap)
    } else {
        (CHEV_LEFT + CHEV_W + gap, ui.x - CHEV_RIGHT - CHEV_W - gap)
    };
    if let Some(b) = buttons.filter(|b| !b.is_empty()) {
        let max_x = b.iter().map(|(_, r)| r.max.x).fold(0.0, f32::max);
        let max_y = b.iter().map(|(_, r)| r.max.y).fold(0.0, f32::max);
        if max_y + gap > top {
            if portrait {
                top = max_y + gap;
            } else {
                left = left.max(max_x + gap);
            }
        }
    }
    let banner = portrait.then(|| {
        let r = Rect::new(left, top, right, top + BANNER_PX / k);
        top = r.max.y + gap;
        r
    });
    TipPlace {
        top,
        left,
        right: right.max(left + 1.0),
        banner,
    }
}

/// The touch layout for the page (window px, logical – CSS px in the
/// browser: `[left, top, width, height]`): the screen tip's slot
/// ([`TipPlace::banner`]), the chevrons and the menu and hide buttons,
/// for the page's banner and the layout checks
/// (`window.ocw.hudLayout()`).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct TouchLayout {
    pub banner: Option<[f32; 4]>,
    pub chevrons: Vec<[f32; 4]>,
    /// The menu button, then the hide button (while they show).
    pub buttons: Vec<[f32; 4]>,
}

/// The first-ride tip's box (measured by the HUD: `HudBoxes::touch_tip`).
#[derive(Component)]
pub(crate) struct TipPanel;

/// The root of the menu and hide buttons (above the menu's veil).
#[derive(Component)]
struct ButtonsRoot;

#[derive(Component)]
struct CornerKey(CornerButton);

/// The stroke through the eye while the controls are hidden.
#[derive(Component)]
struct EyeStroke;

#[derive(Component)]
struct Toast;

#[derive(Component)]
struct ToastText;

/// A tick of an edge's zone mark.
#[derive(Component)]
struct ZoneTick(PowerEdge);

#[derive(Component)]
struct ZoneLabel(PowerEdge);

/// The glow along the top / bottom edge while its zone is held or just
/// stepped.
#[derive(Component)]
struct EdgeGlow(PowerEdge);

#[derive(Component)]
struct FlashBox;

#[derive(Component)]
struct FlashText;

/// The zone marks' ticks (UI px), the marks' opacity and the feedback
/// box's size.
const TICK_W: f32 = 28.0;
const TICK_H: f32 = 3.0;
pub const ZONE_MARK_ALPHA: f32 = 0.35;
const FLASH_W: f32 = 180.0;
const FLASH_H: f32 = 40.0;

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
                    n.left = px(CHEV_LEFT);
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
            // The tip, centred in its band ([`tip_place`], placed by
            // [`place_tip`]).
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: percent(28),
                    left: px(0),
                    width: percent(100),
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
                        padding: UiRect::axes(px(12), px(6)),
                        border_radius: BorderRadius::all(px(8)),
                        max_width: percent(100),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                    TipPanel,
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
            // The faster / slower zones (placed by [`place_power`]): two
            // ticks at each zone's line and its label, the glow along
            // the edge and the feedback of a step.
            for edge in PowerEdge::ALL {
                for right in [false, true] {
                    let mut n = Node {
                        position_type: PositionType::Absolute,
                        width: px(TICK_W),
                        height: px(TICK_H),
                        display: Display::None,
                        ..default()
                    };
                    if right {
                        n.right = px(0);
                    } else {
                        n.left = px(0);
                    }
                    r.spawn((
                        n,
                        BackgroundColor(Color::WHITE.with_alpha(ZONE_MARK_ALPHA)),
                        ZoneTick(edge),
                        Pickable::IGNORE,
                    ));
                }
                r.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(TICK_W + 8.0),
                        display: Display::None,
                        ..default()
                    },
                    Text::new(edge.label()),
                    TextFont {
                        font_size: 16.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE.with_alpha(ZONE_MARK_ALPHA + 0.2)),
                    ZoneLabel(edge),
                    Pickable::IGNORE,
                ));
                for (i, alpha) in [GLOW_ALPHA, GLOW_ALPHA * 0.4].into_iter().enumerate() {
                    let off = px(i as f32 * GLOW_PX / 2.0);
                    let mut n = Node {
                        position_type: PositionType::Absolute,
                        left: px(0),
                        right: px(0),
                        height: px(GLOW_PX / 2.0),
                        display: Display::None,
                        ..default()
                    };
                    match edge {
                        PowerEdge::Top => n.top = off,
                        PowerEdge::Bottom => n.bottom = off,
                    }
                    r.spawn((
                        n,
                        BackgroundColor(ACCENT.with_alpha(alpha)),
                        EdgeGlow(edge),
                        Pickable::IGNORE,
                    ));
                }
            }
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: px(FLASH_W),
                    height: px(FLASH_H),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(FLASH_H / 2.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                FlashBox,
                Pickable::IGNORE,
            ))
            .with_child((
                Text::new(""),
                TextFont {
                    font_size: 22.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                FlashText,
                Pickable::IGNORE,
            ));
        });
    // The menu and hide buttons: a root of their own above the menu's
    // veil (the menu button stays to close it), placed by
    // [`place_buttons`].
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(101),
            ButtonsRoot,
            crate::hud::HudPanel,
            crate::access::AccessExempt,
            Pickable::IGNORE,
        ))
        .with_children(|r| {
            let button = |border: f32| {
                (
                    Node {
                        position_type: PositionType::Absolute,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(5),
                        border: UiRect::all(px(2)),
                        border_radius: BorderRadius::all(px(10)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                    BorderColor::all(Color::WHITE.with_alpha(border)),
                    crate::navi_map::MapAvoid,
                )
            };
            // ☰: three bars. The button takes the pointer (no
            // `Pickable::IGNORE`): over the open menu a tap on it never
            // reaches a row below.
            r.spawn((button(0.5), CornerKey(CornerButton::Menu)))
                .with_children(|b| {
                    for _ in 0..3 {
                        b.spawn((
                            Node {
                                width: px(24),
                                height: px(3.5),
                                border_radius: BorderRadius::all(px(2)),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE.with_alpha(0.9)),
                            Pickable::IGNORE,
                        ));
                    }
                });
            // The eye: a lid with a pupil, struck through while the
            // controls are hidden. Smaller and fainter than ☰.
            r.spawn((button(0.3), CornerKey(CornerButton::Hide), Pickable::IGNORE))
                .with_children(|b| {
                    b.spawn((
                        Node {
                            width: px(26),
                            height: px(16),
                            border: UiRect::all(px(2.5)),
                            border_radius: BorderRadius::all(px(8)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BorderColor::all(Color::WHITE.with_alpha(0.8)),
                        Pickable::IGNORE,
                    ))
                    .with_child((
                        Node {
                            width: px(6),
                            height: px(6),
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BackgroundColor(Color::WHITE.with_alpha(0.8)),
                        Pickable::IGNORE,
                    ));
                    b.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(32),
                            height: px(3),
                            border_radius: BorderRadius::all(px(1.5)),
                            display: Display::None,
                            ..default()
                        },
                        UiTransform::from_rotation(Rot2::degrees(-40.0)),
                        BackgroundColor(Color::WHITE.with_alpha(0.9)),
                        EyeStroke,
                        Pickable::IGNORE,
                    ));
                });
            // The note beside the buttons.
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(px(12), px(6)),
                    border_radius: BorderRadius::all(px(8)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                Toast,
                Pickable::IGNORE,
            ))
            .with_child((
                Text::new(""),
                TextFont {
                    font_size: 16.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                ToastText,
                Pickable::IGNORE,
            ));
        });
}

/// The corner buttons' rectangles of this frame (UI px) and the UI's
/// offset and scale.
fn buttons_now(
    layout: &crate::hud_fit::HudLayout,
    scale: Option<&UiScale>,
    boxes: Option<&crate::hud::HudBoxes>,
    window: Option<&Window>,
    safe: Option<&crate::safe_area::SafeArea>,
) -> ([(CornerButton, Rect); 2], Vec<Rect>, Vec2, f32) {
    let k = scale.map_or(1.0, |s| s.0).max(0.1);
    let off = crate::safe_area::ui_offset(window, safe);
    let avoid = boxes.map(|b| top_boxes(b, off, k)).unwrap_or_default();
    (corner_buttons(layout.ui_px, k, &avoid), avoid, off, k)
}

/// Shows and places the menu and hide buttons ([`corner_buttons`]) and
/// the note beside them every frame: the layout follows the window, the
/// HUD's scale and the top panel.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system
fn place_buttons(
    hud: Res<TouchHud>,
    layout: Res<crate::hud_fit::HudLayout>,
    scale: Option<Res<UiScale>>,
    boxes: Option<Res<crate::hud::HudBoxes>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    mut root: Query<
        &mut Node,
        (
            With<ButtonsRoot>,
            Without<CornerKey>,
            Without<Toast>,
            Without<EyeStroke>,
        ),
    >,
    mut keys: Query<
        (&mut Node, &mut BackgroundColor, &CornerKey),
        (Without<ButtonsRoot>, Without<Toast>, Without<EyeStroke>),
    >,
    mut stroke: Query<
        &mut Node,
        (
            With<EyeStroke>,
            Without<ButtonsRoot>,
            Without<CornerKey>,
            Without<Toast>,
        ),
    >,
    mut toast: Query<
        &mut Node,
        (
            With<Toast>,
            Without<ButtonsRoot>,
            Without<CornerKey>,
            Without<EyeStroke>,
        ),
    >,
    mut toast_text: Query<&mut Text, With<ToastText>>,
) {
    let show = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut root {
        if n.display != show(hud.buttons) {
            n.display = show(hud.buttons);
        }
    }
    if !hud.buttons {
        return;
    }
    let (rects, _, _, k) = buttons_now(
        &layout,
        scale.as_deref(),
        boxes.as_deref(),
        windows.single().ok(),
        safe.as_deref(),
    );
    for (mut n, mut bg, key) in &mut keys {
        let Some((_, r)) = rects.iter().find(|(b, _)| *b == key.0) else {
            continue;
        };
        // Over the open menu only ☰ stays.
        let display = show(key.0 == CornerButton::Menu || !hud.menu_open);
        let (left, top, w, h) = (px(r.min.x), px(r.min.y), px(r.width()), px(r.height()));
        if n.left != left || n.top != top || n.width != w || n.height != h || n.display != display {
            n.left = left;
            n.top = top;
            n.width = w;
            n.height = h;
            n.display = display;
        }
        let held = hud.button_held == Some(key.0);
        let open = key.0 == CornerButton::Menu && hud.menu_open;
        let want = Color::srgba(0.0, 0.0, 0.0, if held || open { 0.75 } else { 0.45 });
        if bg.0 != want {
            bg.0 = want;
        }
    }
    for mut n in &mut stroke {
        if n.display != show(hud.hidden) {
            n.display = show(hud.hidden);
        }
    }
    let hide = rects[1].1;
    for mut n in &mut toast {
        let display = show(hud.toast.is_some() && !hud.menu_open);
        let (left, top, max) = (
            px(hide.max.x + CORNER_GAP_PX / k),
            px(hide.min.y),
            px((layout.ui_px.x - hide.max.x - 2.0 * CORNER_GAP_PX / k).max(1.0)),
        );
        if n.display != display || n.left != left || n.top != top || n.max_width != max {
            n.display = display;
            n.left = left;
            n.top = top;
            n.max_width = max;
            n.min_height = px(hide.height());
        }
    }
    if let (Ok(mut t), Some(text)) = (toast_text.single_mut(), &hud.toast)
        && t.0 != *text
    {
        t.0 = text.clone();
    }
}

/// Where the feedback box goes (UI px: left, top) for a step at the share
/// `x` of the width: over the touched place, just inside the zone's line
/// – below the HUD's top panel, above the height profile.
pub fn flash_place(ui: Vec2, edge: PowerEdge, band: (f32, f32), x: f32) -> Vec2 {
    let left = (x * ui.x - FLASH_W / 2.0).clamp(8.0, (ui.x - FLASH_W - 8.0).max(8.0));
    let top = match edge {
        PowerEdge::Top => band.0 * ui.y - FLASH_H - 10.0,
        PowerEdge::Bottom => band.1 * ui.y + 10.0,
    };
    Vec2::new(left, top.max(0.0))
}

/// Shows and places the faster / slower zones' marks, glow and feedback.
#[allow(clippy::type_complexity)] // Bevy queries
fn place_power(
    hud: Res<TouchHud>,
    layout: Res<crate::hud_fit::HudLayout>,
    mut ticks: Query<(&mut Node, &ZoneTick), (Without<ZoneLabel>, Without<FlashBox>)>,
    mut labels: Query<(&mut Node, &ZoneLabel), (Without<ZoneTick>, Without<FlashBox>)>,
    mut glows: Query<
        (&mut Node, &EdgeGlow),
        (Without<ZoneTick>, Without<ZoneLabel>, Without<FlashBox>),
    >,
    mut flash: Query<&mut Node, (With<FlashBox>, Without<ZoneTick>, Without<ZoneLabel>)>,
    mut flash_text: Query<&mut Text, With<FlashText>>,
) {
    if !hud.is_changed() && !layout.is_changed() {
        return;
    }
    let show = |on: bool| if on { Display::Flex } else { Display::None };
    let ui = layout.ui_px;
    let marks = hud.hints().then_some(hud.power).flatten();
    let line = |edge: PowerEdge, band: (f32, f32)| match edge {
        PowerEdge::Top => band.0 * ui.y,
        PowerEdge::Bottom => band.1 * ui.y,
    };
    for (mut n, t) in &mut ticks {
        n.display = show(marks.is_some());
        if let Some(band) = marks {
            n.top = px(line(t.0, band) - TICK_H / 2.0);
        }
    }
    for (mut n, l) in &mut labels {
        n.display = show(marks.is_some());
        if let Some(band) = marks {
            // Inside its zone: above the top line, below the bottom one.
            n.top = px(match l.0 {
                PowerEdge::Top => line(l.0, band) - 24.0,
                PowerEdge::Bottom => line(l.0, band) + 6.0,
            });
        }
    }
    let lit = |edge| {
        hud.power_held == Some(edge) || hud.power_flash.as_ref().is_some_and(|f| f.0 == edge)
    };
    for (mut n, g) in &mut glows {
        n.display = show(lit(g.0));
    }
    for mut n in &mut flash {
        n.display = show(hud.power_flash.is_some());
        if let (Some((edge, _, x)), Some(band)) = (&hud.power_flash, hud.power) {
            let at = flash_place(ui, *edge, band, *x);
            n.left = px(at.x);
            n.top = px(at.y);
        }
    }
    if let (Ok(mut t), Some((_, text, _))) = (flash_text.single_mut(), &hud.power_flash)
        && t.0 != *text
    {
        t.0 = text.clone();
    }
}

/// Places the first-ride tip ([`tip_place`]) every frame and publishes
/// the touch layout ([`TouchLayout`]).
#[allow(clippy::too_many_arguments)] // a Bevy system
fn place_tip(
    hud: Res<TouchHud>,
    layout: Res<crate::hud_fit::HudLayout>,
    scale: Option<Res<UiScale>>,
    boxes: Option<Res<crate::hud::HudBoxes>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    mut tip_box: Query<&mut Node, With<TipBox>>,
    mut out: ResMut<TouchLayout>,
) {
    if !hud.shown && !hud.buttons {
        out.set_if_neq(TouchLayout::default());
        return;
    }
    let (rects, avoid, off, k) = buttons_now(
        &layout,
        scale.as_deref(),
        boxes.as_deref(),
        windows.single().ok(),
        safe.as_deref(),
    );
    let ui = layout.ui_px;
    let buttons = hud.buttons.then_some(&rects[..]);
    let place = tip_place(ui, k, &avoid, buttons);
    for mut n in &mut tip_box {
        let (top, left, width) = (px(place.top), px(place.left), px(place.right - place.left));
        if n.top != top || n.left != left || n.width != width {
            n.top = top;
            n.left = left;
            n.width = width;
        }
    }
    let window = |r: Rect| {
        let min = r.min * k + off;
        let size = r.size() * k;
        [min.x, min.y, size.x, size.y]
    };
    let chev_top = ui.y * (1.0 - CHEV_BOTTOM) - CHEV_H;
    let chevrons = if hud.hints() {
        vec![
            window(Rect::new(
                CHEV_LEFT,
                chev_top,
                CHEV_LEFT + CHEV_W,
                chev_top + CHEV_H,
            )),
            window(Rect::new(
                ui.x - CHEV_RIGHT - CHEV_W,
                chev_top,
                ui.x - CHEV_RIGHT,
                chev_top + CHEV_H,
            )),
        ]
    } else {
        Vec::new()
    };
    out.set_if_neq(TouchLayout {
        banner: place
            .banner
            .filter(|_| hud.hints() && hud.tip.is_some())
            .map(window),
        chevrons,
        buttons: buttons
            .map(|b| b.iter().map(|(_, r)| window(*r)).collect())
            .unwrap_or_default(),
    });
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy queries
fn update(
    hud: Res<TouchHud>,
    mut root: Query<&mut Node, (With<Root>, Without<Glow>, Without<TipBox>, Without<Chevron>)>,
    mut glows: Query<(&mut Node, &Glow), (Without<Root>, Without<TipBox>, Without<Chevron>)>,
    mut chevrons: Query<&mut Node, (With<Chevron>, Without<Root>, Without<Glow>, Without<TipBox>)>,
    mut bars: Query<(&mut BackgroundColor, &ChevronBar)>,
    mut tip_box: Query<&mut Node, (With<TipBox>, Without<Root>, Without<Glow>, Without<Chevron>)>,
    mut tip_text: Query<&mut Text, With<TipText>>,
    layout: Res<crate::hud_fit::HudLayout>,
) {
    if !hud.is_changed() && !layout.is_changed() {
        return;
    }
    let show = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut root {
        n.display = show(hud.layer());
    }
    if !hud.layer() {
        return;
    }
    for (mut n, g) in &mut glows {
        n.display = show(if g.0 { hud.right_held } else { hud.left_held });
        n.top = percent(hud.zone_top * 100.0);
        n.bottom = percent(((1.0 - hud.zone_bottom) * 100.0).clamp(0.0, 100.0));
    }
    // Hidden controls: no chevrons, no tip – the glow of a held side
    // stays as the touch's feedback.
    for mut n in &mut chevrons {
        n.display = show(hud.hints());
    }
    for (mut c, b) in &mut bars {
        c.0 = Color::WHITE.with_alpha(chevron_alpha(&hud, b.0));
    }
    for mut n in &mut tip_box {
        n.display = show(hud.hints() && hud.tip.is_some());
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

    /// The UI sizes of the cases below: (UI px, scale) of a desktop
    /// window, an iPhone 15 Pro Max (430 × 932 CSS px less the insets) in
    /// portrait and landscape, a small Android phone both ways and a
    /// tablet both ways.
    fn sizes() -> Vec<(Vec2, f32)> {
        let fit = |css: Vec2| {
            let k = crate::hud_fit::screen_fit(css);
            (css / k, k)
        };
        vec![
            (Vec2::new(1280.0, 720.0), 1.0),
            (Vec2::new(1920.0, 1080.0), 1.5),
            fit(Vec2::new(430.0, 932.0 - 93.0)),
            fit(Vec2::new(932.0 - 118.0, 430.0 - 21.0)),
            fit(Vec2::new(360.0, 640.0)),
            fit(Vec2::new(640.0, 360.0)),
            fit(Vec2::new(820.0, 1180.0)),
            fit(Vec2::new(1180.0, 820.0)),
        ]
    }

    fn chevron_rects(ui: Vec2) -> [Rect; 2] {
        let chev_top = ui.y * (1.0 - CHEV_BOTTOM) - CHEV_H;
        [
            Rect::new(CHEV_LEFT, chev_top, CHEV_LEFT + CHEV_W, chev_top + CHEV_H),
            Rect::new(
                ui.x - CHEV_RIGHT - CHEV_W,
                chev_top,
                ui.x - CHEV_RIGHT,
                chev_top + CHEV_H,
            ),
        ]
    }

    /// A top panel as the HUD lays it out: over the width on a narrow
    /// screen, else over the left 60 %; up to a third of the height.
    fn top_panel(ui: Vec2, share: f32) -> Rect {
        if ui.x < crate::hud_fit::NARROW_UI_PX {
            Rect::new(10.0, 10.0, ui.x - 10.0, ui.y * share)
        } else {
            Rect::new(10.0, 10.0, ui.x * 0.6, ui.y * share)
        }
    }

    /// The menu and hide buttons: a row ☰, eye at the top left, 48 CSS px
    /// at any HUD scale (above the 44 pt touch target), 12 px inside the
    /// UI area (the safe area), below the top panel and clear of the
    /// chevrons – at phone (portrait and landscape), tablet and desktop
    /// sizes.
    #[test]
    fn the_menu_and_hide_buttons_sit_at_the_top_left_below_the_panel() {
        for (ui, scale) in sizes() {
            // Without a HUD: the corner.
            let r = corner_buttons(ui, scale, &[]);
            assert_eq!([r[0].0, r[1].0], CornerButton::ALL, "the menu button first");
            for (_, b) in &r {
                assert!((b.width() * scale - CORNER_BUTTON_PX).abs() < 0.01, "{ui}");
                assert!((b.height() * scale - CORNER_BUTTON_PX).abs() < 0.01);
                assert!(b.width() * scale >= 44.0);
            }
            assert!((r[0].1.min.x * scale - CORNER_MARGIN_PX).abs() < 0.01);
            assert!((r[0].1.min.y * scale - CORNER_MARGIN_PX).abs() < 0.01);
            let gap = (r[1].1.min.x - r[0].1.max.x) * scale;
            assert!((gap - CORNER_GAP_PX).abs() < 0.01, "{ui}: {gap}");
            // With the top panel, at a quarter and at a third of the
            // height (the most the phone layout lets it take): below it,
            // on the screen, off the chevrons and the road's middle.
            for share in [0.2, 0.25, 0.36] {
                let panel = top_panel(ui, share);
                let column = Rect::new(
                    ui.x - 260.0,
                    panel.max.y + 4.0,
                    ui.x - 10.0,
                    panel.max.y + 24.0,
                );
                let r = corner_buttons(ui, scale, &[panel, column]);
                for (_, b) in &r {
                    assert!(b.min.y >= panel.max.y, "{ui} {share}: {b:?}");
                    assert!(b.intersect(column).is_empty(), "{ui} {share}");
                    assert!(
                        b.min.x >= 0.0 && b.max.x <= ui.x && b.max.y <= ui.y,
                        "{ui} {share}: {b:?}"
                    );
                    for c in chevron_rects(ui) {
                        assert!(b.intersect(c).is_empty(), "{ui} {share}: {b:?} {c:?}");
                    }
                    assert!(b.max.x < ui.x * 0.45, "{ui} {share}: off the middle");
                }
                // A panel beside them does not move them.
                let beside = Rect::new(ui.x - 50.0, 0.0, ui.x, ui.y);
                assert_eq!(
                    corner_buttons(ui, scale, &[beside]),
                    corner_buttons(ui, scale, &[])
                );
            }
        }
        // The boxes the HUD measured, in window px, to UI px.
        let boxes = crate::hud::HudBoxes {
            top_panel: Some([9.0, 68.0, 375.0, 89.0]),
            top_right: Some([200.0, 161.0, 184.0, 15.0]),
            ..Default::default()
        };
        let a = top_boxes(&boxes, Vec2::new(0.0, 59.0), 0.5);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], Rect::new(18.0, 18.0, 768.0, 196.0));
    }

    /// The first-ride tip's band (spec 3.2.3): below the top boxes – in
    /// portrait below the menu and hide buttons with the screen tip's
    /// slot above it, in landscape between the chevrons right of the
    /// buttons – clear of the buttons and the chevrons, in the upper half
    /// (above the rider), at iPhone and small Android sizes.
    #[test]
    fn the_first_ride_tip_keeps_clear_of_the_buttons_and_chevrons() {
        for css in [
            Vec2::new(390.0, 844.0 - 93.0),
            Vec2::new(844.0 - 118.0, 390.0 - 21.0),
            Vec2::new(430.0, 932.0 - 93.0),
            Vec2::new(932.0 - 118.0, 430.0 - 21.0),
            Vec2::new(360.0, 640.0),
            Vec2::new(640.0, 360.0),
        ] {
            let k = 0.75;
            let ui = css / k;
            // A top panel over the width (portrait) or the left half.
            let portrait = ui.y > ui.x;
            let panel = if portrait {
                Rect::new(10.0, 10.0, ui.x - 10.0, 150.0)
            } else {
                Rect::new(10.0, 10.0, ui.x * 0.6, 140.0)
            };
            let column = Rect::new(
                ui.x - 260.0,
                10.0,
                ui.x - 10.0,
                if portrait { 175.0 } else { 40.0 },
            );
            let avoid = [panel, column];
            let buttons = corner_buttons(ui, k, &avoid);
            let t = tip_place(ui, k, &avoid, Some(&buttons));
            let tip = Rect::new(t.left, t.top, t.right, t.top + 60.0 / k);
            assert!(t.right - t.left > 150.0 / k, "{css}: {t:?}");
            assert!(tip.max.y <= ui.y * 0.5, "{css}: above the rider {t:?}");
            for r in avoid.iter().chain(buttons.iter().map(|(_, r)| r)) {
                assert!(tip.intersect(*r).is_empty(), "{css}: {t:?} {r:?}");
            }
            for c in chevron_rects(ui) {
                assert!(tip.intersect(c).is_empty(), "{css}: {t:?} {c:?}");
            }
            match t.banner {
                Some(b) => {
                    assert!(portrait, "{css}");
                    assert!(b.max.y < t.top && b.min.y >= panel.max.y, "{css}: {b:?}");
                    for (_, r) in &buttons {
                        assert!(b.intersect(*r).is_empty(), "{css}: {b:?} {r:?}");
                    }
                    assert!((b.height() * k - BANNER_PX).abs() < 0.01);
                }
                None => assert!(!portrait, "{css}: the banner slot in portrait"),
            }
        }
        // Without the buttons the tip takes the width below the panel.
        let ui = Vec2::new(390.0, 751.0) / 0.75;
        let t = tip_place(ui, 0.75, &[], None);
        assert!((t.right - (ui.x - TIP_GAP_PX / 0.75)).abs() < 0.01, "{t:?}");
        assert!((t.left - TIP_GAP_PX / 0.75).abs() < 0.01, "{t:?}");
    }

    /// The feedback of a faster / slower step: over the touched place,
    /// inside its zone, on the screen.
    #[test]
    fn the_power_feedback_sits_inside_the_touched_zone() {
        assert!(crate::font::covers(
            "\u{25b2}\u{25bc} faster slower stop 220 W"
        ));
        for (ui, _) in sizes() {
            let band = (0.25, 0.75);
            for x in [0.0, 0.1, 0.5, 0.97, 1.0] {
                let top = flash_place(ui, PowerEdge::Top, band, x);
                let bottom = flash_place(ui, PowerEdge::Bottom, band, x);
                for p in [top, bottom] {
                    assert!(p.x >= 0.0 && p.x + FLASH_W <= ui.x, "{ui} {x}: {p}");
                }
                assert!(top.y >= 0.0 && top.y + FLASH_H < band.0 * ui.y, "{ui}");
                assert!(bottom.y > band.1 * ui.y && bottom.y + FLASH_H < ui.y);
            }
            let mid = flash_place(ui, PowerEdge::Top, band, 0.5);
            assert!((mid.x + FLASH_W / 2.0 - ui.x / 2.0).abs() < 0.01);
        }
    }

    fn display<C: Component>(app: &mut App) -> Vec<Display> {
        let mut q = app.world_mut().query_filtered::<&Node, With<C>>();
        q.iter(app.world()).map(|n| n.display).collect()
    }

    /// The hints draw headless and hide with the setting.
    #[test]
    fn the_hints_show_and_hide() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(TouchHudPlugin);
        app.update();
        assert_eq!(display::<Root>(&mut app), [Display::None]);
        app.world_mut().resource_mut::<TouchHud>().shown = true;
        app.world_mut().resource_mut::<TouchHud>().right_held = true;
        app.world_mut().resource_mut::<TouchHud>().tip = Some("Tap".into());
        app.update();
        assert_eq!(display::<Root>(&mut app), [Display::Flex]);
        let mut q = app.world_mut().query::<(&Node, &Glow)>();
        let lit: Vec<bool> = q
            .iter(app.world())
            .filter(|(n, _)| n.display == Display::Flex)
            .map(|(_, g)| g.0)
            .collect();
        assert_eq!(lit, [true, true], "both strips of the right glow");
        let mut q = app.world_mut().query::<&Text>();
        assert!(q.iter(app.world()).any(|t| t.0 == "Tap"));
        assert_eq!(
            display::<ZoneTick>(&mut app),
            [Display::None; 4],
            "no zone marks without the zones"
        );
        assert_eq!(display::<ButtonsRoot>(&mut app), [Display::None]);
        app.world_mut().resource_mut::<TouchHud>().shown = false;
        app.world_mut().resource_mut::<TouchHud>().right_held = false;
        app.update();
        assert_eq!(display::<Root>(&mut app), [Display::None]);
    }

    /// No stop, + or − button is left; the faster / slower zones show as
    /// two ticks and a label each, and a step's feedback at its edge.
    #[test]
    fn the_zones_are_marked_and_a_step_flashes_at_its_edge() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(TouchHudPlugin);
        {
            let mut h = app.world_mut().resource_mut::<TouchHud>();
            h.shown = true;
            h.buttons = true;
            h.power = Some((0.25, 0.75));
        }
        app.update();
        let mut q = app.world_mut().query::<&Text>();
        let texts: Vec<String> = q.iter(app.world()).map(|t| t.0.clone()).collect();
        for gone in ["stop", "+", "\u{2212}"] {
            assert!(!texts.iter().any(|t| t == gone), "{gone:?}: {texts:?}");
        }
        assert!(texts.iter().any(|t| t == "\u{25b2} faster"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "\u{25bc} slower"), "{texts:?}");
        assert_eq!(display::<ZoneTick>(&mut app), [Display::Flex; 4]);
        assert_eq!(display::<ZoneLabel>(&mut app), [Display::Flex; 2]);
        assert_eq!(display::<FlashBox>(&mut app), [Display::None]);
        assert_eq!(display::<EdgeGlow>(&mut app), [Display::None; 4]);
        // The marks sit at the zones' lines (720 px high by default).
        let mut q = app.world_mut().query::<(&Node, &ZoneTick)>();
        for (n, t) in q.iter(app.world()) {
            let y = if t.0 == PowerEdge::Top { 180.0 } else { 540.0 };
            assert_eq!(n.top, px(y - TICK_H / 2.0));
        }
        // A step at the bottom: its text and the glow of that edge.
        app.world_mut().resource_mut::<TouchHud>().power_flash =
            Some((PowerEdge::Bottom, "\u{25bc} 180 W".into(), 0.5));
        app.update();
        assert_eq!(display::<FlashBox>(&mut app), [Display::Flex]);
        let mut q = app.world_mut().query::<(&Node, &EdgeGlow)>();
        let lit: Vec<PowerEdge> = q
            .iter(app.world())
            .filter(|(n, _)| n.display == Display::Flex)
            .map(|(_, g)| g.0)
            .collect();
        assert_eq!(lit, [PowerEdge::Bottom; 2]);
        let mut q = app.world_mut().query_filtered::<&Text, With<FlashText>>();
        assert_eq!(q.single(app.world()).unwrap().0, "\u{25bc} 180 W");
        // With a power device: no zones, no marks.
        {
            let mut h = app.world_mut().resource_mut::<TouchHud>();
            h.power = None;
            h.power_flash = None;
        }
        app.update();
        assert_eq!(display::<ZoneTick>(&mut app), [Display::None; 4]);
        assert_eq!(display::<FlashBox>(&mut app), [Display::None]);
    }

    /// Hidden controls: the chevrons, the zone marks and the tip are not
    /// drawn; the menu and hide buttons stay (the eye struck through),
    /// the feedback of a touch still shows, and a note says so once.
    #[test]
    fn hidden_controls_draw_only_the_two_buttons_and_the_feedback() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(TouchHudPlugin);
        {
            let mut h = app.world_mut().resource_mut::<TouchHud>();
            h.shown = true;
            h.buttons = true;
            h.tip = Some("Tap".into());
            h.power = Some((0.25, 0.75));
        }
        app.update();
        assert_eq!(display::<Chevron>(&mut app), [Display::Flex; 2]);
        assert_eq!(display::<TipBox>(&mut app), [Display::Flex]);
        assert_eq!(display::<ButtonsRoot>(&mut app), [Display::Flex]);
        assert_eq!(display::<CornerKey>(&mut app), [Display::Flex; 2]);
        assert_eq!(display::<EyeStroke>(&mut app), [Display::None]);
        assert_eq!(display::<Toast>(&mut app), [Display::None]);
        // The buttons are 48 px (scale 1) at the top left.
        let mut q = app.world_mut().query::<(&Node, &CornerKey)>();
        for (n, k) in q.iter(app.world()) {
            assert_eq!((n.width, n.height), (px(48), px(48)));
            let left = if k.0 == CornerButton::Menu {
                12.0
            } else {
                68.0
            };
            assert_eq!((n.left, n.top), (px(left), px(12)));
        }
        {
            let mut h = app.world_mut().resource_mut::<TouchHud>();
            h.hidden = true;
            h.toast = Some("Controls hidden \u{2013} they still work".into());
        }
        app.update();
        assert_eq!(display::<Root>(&mut app), [Display::None]);
        assert_eq!(display::<ZoneTick>(&mut app), [Display::None; 4]);
        assert_eq!(display::<ZoneLabel>(&mut app), [Display::None; 2]);
        assert_eq!(display::<ButtonsRoot>(&mut app), [Display::Flex]);
        assert_eq!(display::<CornerKey>(&mut app), [Display::Flex; 2]);
        assert_eq!(display::<EyeStroke>(&mut app), [Display::Flex]);
        assert_eq!(display::<Toast>(&mut app), [Display::Flex]);
        let layout = app.world().resource::<TouchLayout>().clone();
        assert!(layout.chevrons.is_empty() && layout.banner.is_none());
        assert_eq!(layout.buttons.len(), 2);
        // A touch's feedback still shows: the held side's glow, a step.
        {
            let mut h = app.world_mut().resource_mut::<TouchHud>();
            h.left_held = true;
            h.power_flash = Some((PowerEdge::Top, "\u{25b2} 220 W".into(), 0.3));
        }
        app.update();
        assert_eq!(display::<Root>(&mut app), [Display::Flex]);
        assert_eq!(display::<Chevron>(&mut app), [Display::None; 2]);
        assert_eq!(display::<TipBox>(&mut app), [Display::None]);
        assert_eq!(display::<FlashBox>(&mut app), [Display::Flex]);
        let mut q = app.world_mut().query::<(&Node, &Glow)>();
        assert_eq!(
            q.iter(app.world())
                .filter(|(n, g)| n.display == Display::Flex && !g.0)
                .count(),
            2
        );
        // Over the open menu only ☰ stays (it closes the menu).
        app.world_mut().resource_mut::<TouchHud>().menu_open = true;
        app.update();
        let mut q = app.world_mut().query::<(&Node, &CornerKey)>();
        for (n, k) in q.iter(app.world()) {
            let want = if k.0 == CornerButton::Menu {
                Display::Flex
            } else {
                Display::None
            };
            assert_eq!(n.display, want, "{:?}", k.0);
        }
        assert_eq!(display::<Toast>(&mut app), [Display::None]);
    }
}
