//! The damage indicator (spec 14 Phase 12F, K5): a compact widget in the
//! bike computer's panel – a rider silhouette and a bike silhouette, each
//! with its bar, its percentage and its level word ("hurt", "scratched"),
//! the status colours (good / warning / serious / critical: green, amber,
//! orange, red) always with an icon and a word, never colour alone; the
//! damaged parts light up on the bike icon (wheels, tyres, chain, bars,
//! frame) with short labels ("rear wheel buckled", "puncture: stop to
//! repair"); on a hit the icon flashes, the bar drops with an animation and
//! the loss shows for 2 s ("−12 % bike").
//!
//! The app computes everything (Bevy-free, `ocw_client::damage`): the
//! widget only lays out [`DamageWidgetView`]. **Always shown while damage
//! is on** (the user, 2026-11-19), also at 100 % before any hit; hidden
//! only with `ride.damage` off ([`crate::RideView::damage`] `None`) and
//! with `hud off` (it is part of the bike computer's panel). The icons are
//! drawn in code ([`crate::damage_icon`]): a side-view bicycle whose parts
//! are tinted layers, and a small rider.

use bevy::prelude::*;
use ocw_sim::damage::{PARTS, Status};

use crate::damage_icon::{
    self, BIKE_ICON, BIKE_LAYERS, BikeLayer, RIDER_ICON, bike_shapes, rider_shapes,
};

/// One meter of the widget.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterView {
    /// The bar as shown (0–1; it drops with an animation after a hit).
    pub shown: f32,
    /// The condition (whole per cent).
    pub pct: u32,
    /// The level word ("hurt", "scratched").
    pub word: String,
    pub status: Status,
    /// The icon's flash after a hit (0–1).
    pub flash: f32,
}

/// What the widget shows (spec 14 Phase 12F).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageWidgetView {
    pub rider: MeterView,
    pub bike: MeterView,
    /// Each part's light on the bike icon (`None`: fine), by
    /// [`Part::index`].
    pub parts: [Option<Status>; PARTS],
    /// The damaged parts' labels.
    pub labels: Vec<String>,
    /// The last hit's loss for 2 s ("−12 % bike").
    pub loss: Option<String>,
    /// A stop under way ("Medical stop: 42 s").
    pub stop: Option<String>,
}

pub struct DamageHudPlugin;

impl Plugin for DamageHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, make_icons)
            .add_systems(Update, (show_icons, update_widget));
    }
}

/// The widget's root (spawned as the last child of the bike computer's
/// panel, so it moves and hides with it).
#[derive(Component)]
pub struct DamageWidget;

/// The row of the two meters (stacked on a very narrow panel).
#[derive(Component)]
struct Meters;

/// A coloured piece of an icon or a bar.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// The rider silhouette.
    Rider,
    /// A bar's fill; its background block (flashes).
    RiderFill,
    BikeFill,
    RiderBlock,
    BikeBlock,
    /// A layer of the bike icon: a part (wheels, tyres, the chain, the
    /// bars, the frame) or the saddle.
    Bike(BikeLayer),
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    RiderHead,
    RiderWord,
    BikeHead,
    BikeWord,
    Labels,
    Loss,
}

/// The icons' images (made at startup; absent without an asset server, as
/// in headless tests).
#[derive(Resource, Debug, Clone)]
struct DamageIcons {
    rider: Handle<Image>,
    bike: Vec<(BikeLayer, Handle<Image>)>,
}

/// The silhouette's colour of a fine part (light grey).
const FINE: Color = Color::srgb(0.78, 0.80, 0.82);

/// The widget's measures (UI px), for the layout and the fit test.
pub const BAR_W: f32 = 150.0;
const BAR_H: f32 = 11.0;
const TEXT_FONT: f32 = 20.0;
const LABEL_FONT: f32 = 18.0;
const LOSS_FONT: f32 = 24.0;
/// Between the widget's blocks side by side, and one above the other.
const BLOCK_GAP: f32 = 16.0;
const ROW_GAP: f32 = 4.0;
const ICON_GAP: f32 = 8.0;
/// The widget's padding, a block's padding, the margin above it.
const WIDGET_PAD: f32 = 6.0;
const BLOCK_PAD: f32 = 4.0;
const WIDGET_MARGIN: f32 = 6.0;

pub fn status_color(s: Status) -> Color {
    let [r, g, b] = s.rgb();
    Color::srgb(r, g, b)
}

/// "▲ hurt" – the icon with the word.
pub fn level_text(m: &MeterView) -> String {
    format!("{} {}", m.status.icon(), m.word)
}

fn make_icons(mut commands: Commands, images: Option<ResMut<Assets<Image>>>) {
    let Some(mut images) = images else { return };
    let rider = images.add(damage_icon::mask_image(&rider_shapes(), RIDER_ICON));
    let bike = BIKE_LAYERS
        .into_iter()
        .map(|l| {
            (
                l,
                images.add(damage_icon::mask_image(&bike_shapes(l), BIKE_ICON)),
            )
        })
        .collect();
    commands.insert_resource(DamageIcons { rider, bike });
}

/// Puts the icons' images on their layers (once they exist).
fn show_icons(icons: Option<Res<DamageIcons>>, mut layers: Query<(&Piece, &mut ImageNode)>) {
    let Some(icons) = icons else { return };
    for (p, mut img) in &mut layers {
        let want = match p {
            Piece::Rider => &icons.rider,
            Piece::Bike(l) => match icons.bike.iter().find(|(k, _)| k == l) {
                Some((_, h)) => h,
                None => continue,
            },
            _ => continue,
        };
        if img.image != *want {
            img.image = want.clone();
        }
    }
}

fn bar(parent: &mut ChildSpawnerCommands, fill: Piece) {
    parent
        .spawn((
            Node {
                width: px(BAR_W),
                height: px(BAR_H),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.22)),
        ))
        .with_child((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            BackgroundColor(FINE),
            fill,
        ));
}

fn small_text(line: Line, size: f32) -> impl Bundle {
    (
        Text::new(""),
        TextFont {
            font_size: size.into(),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 0.9)),
        line,
    )
}

/// An image layer filling its icon (the image comes from [`show_icons`]).
fn layer(piece: Piece) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        ImageNode::default().with_color(FINE),
        piece,
    )
}

/// A meter's block: the icon (its layers), then the head, the bar and the
/// word.
fn block(
    row: &mut ChildSpawnerCommands,
    block: Piece,
    icon: Vec2,
    layers: &[Piece],
    lines: (Line, Piece, Line),
) {
    row.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(ICON_GAP),
            padding: UiRect::all(px(BLOCK_PAD)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        block,
    ))
    .with_children(|b| {
        b.spawn(Node {
            width: px(icon.x),
            height: px(icon.y),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|i| {
            for l in layers {
                i.spawn(layer(*l));
            }
        });
        b.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
            min_width: px(BAR_W),
            ..default()
        })
        .with_children(|c| {
            c.spawn(small_text(lines.0, TEXT_FONT));
            bar(c, lines.1);
            c.spawn(small_text(lines.2, TEXT_FONT));
        });
    });
}

/// Spawns the widget (hidden until the view has damage) under `parent`.
pub fn spawn_widget(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::None,
                // Labels beside the meters, or under them on a narrow
                // panel ([`widget_layout`]; no flex wrap: the bike
                // computer's panel did not grow with a wrapped line).
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: px(BLOCK_GAP),
                row_gap: px(ROW_GAP),
                margin: UiRect::top(px(WIDGET_MARGIN)),
                padding: UiRect::all(px(WIDGET_PAD)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            // Darker than the panel: the status colours read on it.
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            DamageWidget,
        ))
        .with_children(|row| {
            // The two meters: side by side, or stacked on a very narrow
            // panel.
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::FlexStart,
                    column_gap: px(BLOCK_GAP),
                    row_gap: px(ROW_GAP),
                    ..default()
                },
                Meters,
            ))
            .with_children(|m| {
                block(
                    m,
                    Piece::RiderBlock,
                    RIDER_ICON,
                    &[Piece::Rider],
                    (Line::RiderHead, Piece::RiderFill, Line::RiderWord),
                );
                let layers: Vec<Piece> = BIKE_LAYERS.into_iter().map(Piece::Bike).collect();
                block(
                    m,
                    Piece::BikeBlock,
                    BIKE_ICON,
                    &layers,
                    (Line::BikeHead, Piece::BikeFill, Line::BikeWord),
                );
            });
            // The part labels, the stop and the last hit's loss.
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(1),
                ..default()
            })
            .with_children(|c| {
                c.spawn(small_text(Line::Loss, LOSS_FONT));
                c.spawn(small_text(Line::Labels, LABEL_FONT));
            });
        });
}

/// The colour of a bike icon layer: a damaged part in its status colour,
/// a fine part and the saddle grey.
fn layer_color(view: &DamageWidgetView, l: BikeLayer) -> Color {
    match l {
        BikeLayer::Part(p) => view.parts[p.index()].map_or(FINE, status_color),
        BikeLayer::Saddle => FINE,
    }
}

/// The text lines of the widget (tests, screen readers later).
pub fn lines(view: &DamageWidgetView) -> [String; 6] {
    let mut labels = view.labels.join("\n");
    if let Some(s) = &view.stop {
        labels = if labels.is_empty() {
            s.clone()
        } else {
            format!("{s}\n{labels}")
        };
    }
    [
        format!("Rider {} %", view.rider.pct),
        level_text(&view.rider),
        format!("Bike {} %", view.bike.pct),
        level_text(&view.bike),
        labels,
        view.loss.clone().unwrap_or_default(),
    ]
}

/// How the widget is laid out in a panel whose content may be `room` px
/// wide (UI px; 0 or less: not known yet, as wide as needed): the meters
/// side by side unless they do not fit, the labels beside them unless they
/// do not fit; and the widget's size then (UI px, with its padding and the
/// margin above it). Text is measured as Bevy's monospaced estimate (0.6 em
/// a character, lines of 1.2 em).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidgetLayout {
    /// The bike's meter under the rider's.
    pub meters_stacked: bool,
    /// The labels (and the loss) under the meters.
    pub notes_below: bool,
    pub size: Vec2,
}

pub fn widget_layout(view: &DamageWidgetView, room: f32) -> WidgetLayout {
    let chars = |s: &str| s.chars().count() as f32;
    let text_w =
        |lines: &[&str], font: f32| lines.iter().map(|l| chars(l)).fold(0.0, f32::max) * font * 0.6;
    let [rider_head, rider_word, bike_head, bike_word, labels, loss] = lines(view);
    let meter_h = 2.0 * BLOCK_PAD + 2.0 * TEXT_FONT * 1.2 + BAR_H + 2.0 * 2.0;
    let meter = |icon: Vec2, head: &str, word: &str| {
        let text = text_w(&[head, word], TEXT_FONT).max(BAR_W);
        Vec2::new(
            2.0 * BLOCK_PAD + icon.x + ICON_GAP + text,
            meter_h.max(2.0 * BLOCK_PAD + icon.y),
        )
    };
    let label_lines: Vec<&str> = labels.lines().filter(|l| !l.is_empty()).collect();
    let notes = Vec2::new(
        text_w(&label_lines, LABEL_FONT).max(text_w(&[&loss], LOSS_FONT)),
        if loss.is_empty() {
            0.0
        } else {
            LOSS_FONT * 1.2
        } + label_lines.len() as f32 * LABEL_FONT * 1.2,
    );
    let room = if room > 0.0 {
        room - 2.0 * WIDGET_PAD
    } else {
        f32::INFINITY
    };
    let (rider, bike) = (
        meter(RIDER_ICON, &rider_head, &rider_word),
        meter(BIKE_ICON, &bike_head, &bike_word),
    );
    let beside = |a: Vec2, b: Vec2| Vec2::new(a.x + BLOCK_GAP + b.x, a.y.max(b.y));
    let below = |a: Vec2, b: Vec2| Vec2::new(a.x.max(b.x), a.y + ROW_GAP + b.y);
    let meters_stacked = beside(rider, bike).x > room;
    let meters = if meters_stacked {
        below(rider, bike)
    } else {
        beside(rider, bike)
    };
    let notes_below = notes.x > 0.0 && beside(meters, notes).x > room;
    let inner = if notes.x <= 0.0 {
        meters
    } else if notes_below {
        below(meters, notes)
    } else {
        beside(meters, notes)
    };
    WidgetLayout {
        meters_stacked,
        notes_below,
        size: inner + Vec2::splat(2.0 * WIDGET_PAD) + Vec2::new(0.0, WIDGET_MARGIN),
    }
}

/// The widget's size (UI px) for `view` in a panel whose content may be
/// `room` px wide ([`widget_layout`]).
pub fn widget_size(view: &DamageWidgetView, room: f32) -> Vec2 {
    widget_layout(view, room).size
}

#[allow(clippy::type_complexity)]
fn update_widget(
    view: Res<crate::RideView>,
    mut root: Query<&mut Node, (With<DamageWidget>, Without<Piece>, Without<Meters>)>,
    mut meters: Query<&mut Node, (With<Meters>, Without<DamageWidget>, Without<Piece>)>,
    mut pieces: Query<
        (
            &Piece,
            Option<&mut BackgroundColor>,
            Option<&mut ImageNode>,
            &mut Node,
        ),
        (Without<DamageWidget>, Without<Meters>),
    >,
    mut texts: Query<(&Line, &mut Text, &mut TextColor)>,
    access: Option<Res<crate::access::Accessibility>>,
    panel: Option<Res<crate::hud::TopPanelWidth>>,
) {
    // Reduced motion (spec 3.2.1 M9): no flashing after a hit.
    let still = access.is_some_and(|a| a.reduced_motion);
    let w = view.damage.as_ref();
    // The room the bike computer's panel leaves (0 before its first
    // layout: as wide as needed).
    let room = panel.map_or(0.0, |p| p.0 - 2.0 * crate::hud::PANEL_PAD);
    let layout = w.map(|w| widget_layout(w, room));
    let direction = |below: bool| {
        if below {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        }
    };
    for mut n in &mut root {
        let want = if w.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if n.display != want {
            n.display = want;
        }
        if let Some(l) = layout
            && n.flex_direction != direction(l.notes_below)
        {
            n.flex_direction = direction(l.notes_below);
        }
    }
    if let Some(l) = layout {
        for mut n in &mut meters {
            if n.flex_direction != direction(l.meters_stacked) {
                n.flex_direction = direction(l.meters_stacked);
            }
        }
    }
    let Some(w) = w else { return };
    for (p, bg, image, mut node) in &mut pieces {
        match *p {
            Piece::Rider => {
                if let Some(mut img) = image {
                    let c = match w.rider.status {
                        Status::Good => FINE,
                        s => status_color(s),
                    };
                    if img.color != c {
                        img.color = c;
                    }
                }
            }
            Piece::RiderFill | Piece::BikeFill => {
                let m = if *p == Piece::RiderFill {
                    &w.rider
                } else {
                    &w.bike
                };
                node.width = Val::Percent((m.shown.clamp(0.0, 1.0) * 100.0).max(2.0));
                if let Some(mut bg) = bg {
                    bg.0 = status_color(m.status);
                }
            }
            Piece::RiderBlock | Piece::BikeBlock => {
                let m = if *p == Piece::RiderBlock {
                    &w.rider
                } else {
                    &w.bike
                };
                if let Some(mut bg) = bg {
                    bg.0 = if m.flash > 0.0 && !still {
                        status_color(m.status.max(Status::Warning)).with_alpha(0.55 * m.flash)
                    } else {
                        Color::NONE
                    };
                }
            }
            Piece::Bike(l) => {
                let c = layer_color(w, l);
                if let Some(mut img) = image
                    && img.color != c
                {
                    img.color = c;
                }
            }
        }
    }
    let [rider_head, rider_word, bike_head, bike_word, labels, loss] = lines(w);
    for (line, mut text, mut colour) in &mut texts {
        let (want, c) = match line {
            Line::RiderHead => (&rider_head, Color::srgb(0.92, 0.92, 0.92)),
            Line::RiderWord => (&rider_word, status_color(w.rider.status)),
            Line::BikeHead => (&bike_head, Color::srgb(0.92, 0.92, 0.92)),
            Line::BikeWord => (&bike_word, status_color(w.bike.status)),
            Line::Labels => (&labels, Color::srgb(0.95, 0.85, 0.6)),
            Line::Loss => (&loss, status_color(Status::Serious)),
        };
        if text.0 != *want {
            text.0 = want.clone();
        }
        if colour.0 != c {
            colour.0 = c;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::RideView;
    use ocw_sim::damage::Part;
    use ocw_world::Route;

    fn meter(pct: u32, word: &str, status: Status) -> MeterView {
        MeterView {
            shown: pct as f32 / 100.0,
            pct,
            word: word.into(),
            status,
            flash: 0.0,
        }
    }

    fn widget() -> DamageWidgetView {
        let mut parts = [None; PARTS];
        parts[Part::RearWheel.index()] = Some(Status::Serious);
        DamageWidgetView {
            rider: meter(62, "hurt", Status::Warning),
            bike: meter(71, "scratched", Status::Good),
            parts,
            labels: vec!["rear wheel buckled".into()],
            loss: Some("−12 % bike".into()),
            stop: None,
        }
    }

    /// A fresh rider on a fresh bike: what the widget shows with damage on
    /// before any hit.
    pub(crate) fn fresh() -> DamageWidgetView {
        DamageWidgetView {
            rider: meter(100, "fine", Status::Good),
            bike: meter(100, "fine", Status::Good),
            parts: [None; PARTS],
            labels: Vec::new(),
            loss: None,
            stop: None,
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(DamageHudPlugin)
            .add_systems(Startup, |mut commands: Commands| {
                commands.spawn(Node::default()).with_children(spawn_widget);
            });
        app.update();
        app
    }

    fn display(app: &mut App) -> Display {
        let w = app.world_mut();
        w.query_filtered::<&Node, With<DamageWidget>>()
            .single(w)
            .unwrap()
            .display
    }

    fn text(app: &mut App, line: Line) -> (String, Color) {
        let w = app.world_mut();
        let mut q = w.query::<(&Line, &Text, &TextColor)>();
        let (_, t, c) = q.iter(w).find(|(l, _, _)| **l == line).unwrap();
        (t.0.clone(), c.0)
    }

    fn tint(app: &mut App, piece: Piece) -> Color {
        let w = app.world_mut();
        let mut q = w.query::<(&Piece, &ImageNode)>();
        q.iter(w).find(|(p, _)| **p == piece).unwrap().1.color
    }

    #[test]
    fn the_widget_shows_the_meters_words_icons_and_parts() {
        let mut app = app();
        assert_eq!(display(&mut app), Display::None, "damage off: hidden");
        app.world_mut().resource_mut::<RideView>().damage = Some(widget());
        app.update();
        assert_eq!(display(&mut app), Display::Flex);
        assert_eq!(text(&mut app, Line::RiderHead).0, "Rider 62 %");
        let (word, colour) = text(&mut app, Line::RiderWord);
        assert_eq!(word, "▲ hurt", "an icon and a word, never colour alone");
        assert_eq!(colour, status_color(Status::Warning));
        assert_eq!(text(&mut app, Line::BikeWord).0, "● scratched");
        assert_eq!(text(&mut app, Line::Labels).0, "rear wheel buckled");
        assert_eq!(text(&mut app, Line::Loss).0, "−12 % bike");
        // The rear wheel lights up on the bike icon, the rest stays grey.
        assert_eq!(
            tint(&mut app, Piece::Bike(BikeLayer::Part(Part::RearWheel))),
            status_color(Status::Serious)
        );
        for p in [Part::FrontWheel, Part::RearTyre, Part::Frame] {
            assert_eq!(
                tint(&mut app, Piece::Bike(BikeLayer::Part(p))),
                FINE,
                "{p:?}"
            );
        }
        assert_eq!(tint(&mut app, Piece::Rider), status_color(Status::Warning));
        // The bars follow the shown value.
        let w = app.world_mut();
        let mut q = w.query::<(&Piece, &Node)>();
        let fill = q.iter(w).find(|(p, _)| **p == Piece::RiderFill).unwrap().1;
        assert_eq!(fill.width, Val::Percent(62.0));
        // Damage off again: hidden.
        app.world_mut().resource_mut::<RideView>().damage = None;
        app.update();
        assert_eq!(display(&mut app), Display::None);
    }

    /// The user (2026-11-19): with damage on the widget is always there,
    /// also at 100 % before any hit – both meters, the words, a grey bike.
    #[test]
    fn the_widget_shows_at_100_percent() {
        let mut app = app();
        app.world_mut().resource_mut::<RideView>().damage = Some(fresh());
        app.update();
        assert_eq!(display(&mut app), Display::Flex);
        assert_eq!(text(&mut app, Line::RiderHead).0, "Rider 100 %");
        assert_eq!(text(&mut app, Line::BikeHead).0, "Bike 100 %");
        assert_eq!(text(&mut app, Line::BikeWord).0, "● fine");
        assert_eq!(text(&mut app, Line::Labels).0, "");
        for l in BIKE_LAYERS {
            assert_eq!(tint(&mut app, Piece::Bike(l)), FINE, "{l:?}");
        }
    }

    /// Every part's light goes to its own layer of the new bike icon: a
    /// puncture lights the tyre (not the rim), a broken chain the
    /// drivetrain, a bent bar the cockpit, frame damage the frame – and
    /// the saddle never lights up.
    #[test]
    fn part_highlights_land_on_their_layers() {
        let mut app = app();
        for part in Part::ALL {
            let mut v = fresh();
            v.parts[part.index()] = Some(Status::Critical);
            app.world_mut().resource_mut::<RideView>().damage = Some(v);
            app.update();
            for l in BIKE_LAYERS {
                let want = if l == BikeLayer::Part(part) {
                    status_color(Status::Critical)
                } else {
                    FINE
                };
                assert_eq!(tint(&mut app, Piece::Bike(l)), want, "{part:?} → {l:?}");
            }
        }
        // Each part has exactly one layer.
        for part in Part::ALL {
            let w = app.world_mut();
            let mut q = w.query::<&Piece>();
            let n = q
                .iter(w)
                .filter(|p| **p == Piece::Bike(BikeLayer::Part(part)))
                .count();
            assert_eq!(n, 1, "{part:?}");
        }
    }

    #[test]
    fn a_hit_flashes_the_block_and_a_stop_is_listed_first() {
        let mut app = app();
        let mut v = widget();
        v.bike.flash = 1.0;
        v.stop = Some("Mending the puncture: 28 s".into());
        app.world_mut().resource_mut::<RideView>().damage = Some(v);
        app.update();
        let w = app.world_mut();
        let mut q = w.query::<(&Piece, &BackgroundColor)>();
        let block = q.iter(w).find(|(p, _)| **p == Piece::BikeBlock).unwrap().1;
        assert!(block.0.alpha() > 0.4, "the bike icon flashes");
        let calm = q.iter(w).find(|(p, _)| **p == Piece::RiderBlock).unwrap().1;
        assert_eq!(calm.0, Color::NONE);
        assert_eq!(
            text(&mut app, Line::Labels).0,
            "Mending the puncture: 28 s\nrear wheel buckled"
        );
    }

    /// With an asset store the icons get their images: one for the rider,
    /// one per bike layer.
    #[test]
    fn the_icons_get_their_images() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(DamageHudPlugin)
            .add_systems(Startup, |mut commands: Commands| {
                commands.spawn(Node::default()).with_children(spawn_widget);
            });
        app.update();
        let w = app.world_mut();
        let mut q = w.query::<(&Piece, &ImageNode)>();
        let handles: Vec<Handle<Image>> = q.iter(w).map(|(_, i)| i.image.clone()).collect();
        assert_eq!(handles.len(), 1 + BIKE_LAYERS.len());
        let images = w.resource::<Assets<Image>>();
        for h in &handles {
            assert!(images.get(h).is_some());
        }
        let size = images.get(&handles[0]).unwrap().size();
        assert_eq!(
            size,
            (RIDER_ICON * damage_icon::ICON_SCALE as f32).as_uvec2()
        );
    }

    #[test]
    fn the_widget_texts_are_in_the_ui_font() {
        for s in [
            Status::Good,
            Status::Warning,
            Status::Serious,
            Status::Critical,
        ] {
            assert!(crate::font::covers(s.icon()), "{s:?}");
        }
        assert!(crate::font::covers("−12 % bike, −8 % rider"));
    }

    /// The widget's layout: everything side by side in a wide panel; the
    /// labels under the meters when they do not fit beside them; the
    /// meters stacked in a very narrow panel.
    #[test]
    fn the_widget_stacks_in_a_narrow_panel() {
        let wide = widget_layout(&fresh(), 1000.0);
        assert!(!wide.meters_stacked && !wide.notes_below && wide.size.x < 1000.0);
        let narrow = widget_layout(&fresh(), 400.0);
        assert!(
            narrow.meters_stacked && narrow.size.x <= 400.0,
            "{narrow:?}"
        );
        assert!(narrow.size.y > wide.size.y * 1.5, "{narrow:?}");
        let hurt = widget_layout(&widget(), 1000.0);
        assert!(!hurt.notes_below && hurt.size.x > wide.size.x, "{hurt:?}");
        let middle = widget_layout(&widget(), 600.0);
        assert!(!middle.meters_stacked && middle.notes_below, "{middle:?}");
        assert!(middle.size.x <= 600.0 && middle.size.y > hurt.size.y);
        assert_eq!(
            widget_layout(&widget(), 0.0),
            hurt_unbounded(),
            "unknown room"
        );
    }

    fn hurt_unbounded() -> WidgetLayout {
        widget_layout(&widget(), f32::MAX)
    }

    /// The nodes follow the layout: the root's direction puts the labels
    /// under the meters, the meters' row stacks them.
    #[test]
    fn the_nodes_follow_the_layout() {
        let mut app = app();
        app.world_mut()
            .insert_resource(crate::hud::TopPanelWidth(620.0));
        app.world_mut().resource_mut::<RideView>().damage = Some(widget());
        app.update();
        let dirs = |app: &mut App| {
            let w = app.world_mut();
            let root = w
                .query_filtered::<&Node, With<DamageWidget>>()
                .single(w)
                .unwrap()
                .flex_direction;
            let meters = w
                .query_filtered::<&Node, With<Meters>>()
                .single(w)
                .unwrap()
                .flex_direction;
            (root, meters)
        };
        assert_eq!(dirs(&mut app), (FlexDirection::Column, FlexDirection::Row));
        app.world_mut()
            .resource_mut::<crate::hud::TopPanelWidth>()
            .0 = 380.0;
        app.update();
        assert_eq!(
            dirs(&mut app),
            (FlexDirection::Column, FlexDirection::Column)
        );
        app.world_mut()
            .resource_mut::<crate::hud::TopPanelWidth>()
            .0 = 1200.0;
        app.update();
        assert_eq!(dirs(&mut app), (FlexDirection::Row, FlexDirection::Row));
    }
}
