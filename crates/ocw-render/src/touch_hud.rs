//! Touch steering's hints (spec §3.2.3 *Visual feedback*): the chevrons
//! ‹ › at the lower left and right (40 % opacity, 15 % after 8 s without
//! a touch, a flash on a tap step), a glow along the edge of the side
//! being held, and the first-ride tip. Part of the HUD (hidden with
//! `hud off`). The shared UI (`ocw_ui::touch`) decides what shows and
//! writes [`TouchHud`]; this module only draws it.

use bevy::prelude::*;
use bevy::text::Justify;

pub struct TouchHudPlugin;

impl Plugin for TouchHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchHud>()
            .add_systems(Startup, spawn)
            .add_systems(Update, update);
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
}

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
                    bottom: percent(36),
                    ..default()
                };
                if right {
                    n.right = px(36);
                } else {
                    n.left = px(36);
                }
                r.spawn((n, Chevron, Pickable::IGNORE)).with_children(|c| {
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
                    top: percent(28),
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

#[allow(clippy::type_complexity)] // Bevy queries
fn update(
    hud: Res<TouchHud>,
    mut root: Query<&mut Node, (With<Root>, Without<Glow>, Without<TipBox>)>,
    mut glows: Query<(&mut Node, &Glow), (Without<Root>, Without<TipBox>)>,
    mut bars: Query<(&mut BackgroundColor, &ChevronBar)>,
    mut tip_box: Query<&mut Node, (With<TipBox>, Without<Root>, Without<Glow>)>,
    mut tip_text: Query<&mut Text, With<TipText>>,
) {
    if !hud.is_changed() {
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
    }
}
