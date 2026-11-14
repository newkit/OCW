//! The damage indicator (spec §14 Phase 12F, K5): a compact widget in the
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
//! widget only lays out [`DamageWidgetView`]. Hidden with `ride.damage`
//! off ([`crate::RideView::damage`] `None`) and with `hud.damage off`
//! ([`ShowDamage`]; the effects stay).

use bevy::prelude::*;
use ocw_sim::damage::{PARTS, Part, Status};

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

/// What the widget shows (spec §14 Phase 12F).
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

/// `hud.damage` (Settings → HUD): the widget shown (the effects stay).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowDamage(pub bool);

impl Default for ShowDamage {
    fn default() -> Self {
        Self(true)
    }
}

pub struct DamageHudPlugin;

impl Plugin for DamageHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowDamage>()
            .add_systems(Update, update_widget);
    }
}

/// The widget's root (spawned as the last child of the bike computer's
/// panel, so it moves and hides with it).
#[derive(Component)]
pub struct DamageWidget;

/// A coloured piece of an icon or a bar.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// The rider silhouette (its parts).
    Rider,
    /// A bar's fill; its background block (flashes).
    RiderFill,
    BikeFill,
    RiderBlock,
    BikeBlock,
    /// A part on the bike icon: wheels (border = the tyre too), the
    /// chain, the bars, the frame.
    Part(Part),
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

/// The silhouette's colour of a fine part (light grey).
const FINE: Color = Color::srgb(0.78, 0.80, 0.82);

pub fn status_color(s: Status) -> Color {
    let [r, g, b] = s.rgb();
    Color::srgb(r, g, b)
}

/// "▲ hurt" – the icon with the word.
pub fn level_text(m: &MeterView) -> String {
    format!("{} {}", m.status.icon(), m.word)
}

fn bar(parent: &mut ChildSpawnerCommands, fill: Piece) {
    parent
        .spawn((
            Node {
                width: px(150),
                height: px(11),
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

/// A box at (left, top) of `w` × `h` px in an icon.
fn piece(left: f32, top: f32, w: f32, h: f32, radius: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: px(left),
        top: px(top),
        width: px(w),
        height: px(h),
        border_radius: BorderRadius::all(px(radius)),
        ..default()
    }
}

/// Spawns the widget (hidden until the view has damage) under `parent`.
pub fn spawn_widget(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: px(16),
                margin: UiRect::top(px(6)),
                padding: UiRect::all(px(6)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            // Darker than the panel: the status colours read on it.
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            DamageWidget,
        ))
        .with_children(|row| {
            // The rider: silhouette, bar, words.
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(8),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                Piece::RiderBlock,
            ))
            .with_children(|b| {
                b.spawn(Node {
                    width: px(33),
                    height: px(48),
                    ..default()
                })
                .with_children(|icon| {
                    // Head, body, legs.
                    icon.spawn((
                        piece(9.0, 0.0, 15.0, 15.0, 7.5),
                        BackgroundColor(FINE),
                        Piece::Rider,
                    ));
                    icon.spawn((
                        piece(6.0, 16.5, 21.0, 18.0, 6.0),
                        BackgroundColor(FINE),
                        Piece::Rider,
                    ));
                    icon.spawn((
                        piece(7.5, 34.5, 7.5, 13.5, 3.0),
                        BackgroundColor(FINE),
                        Piece::Rider,
                    ));
                    icon.spawn((
                        piece(18.0, 34.5, 7.5, 13.5, 3.0),
                        BackgroundColor(FINE),
                        Piece::Rider,
                    ));
                });
                b.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                })
                .with_children(|c| {
                    c.spawn(small_text(Line::RiderHead, 20.0));
                    bar(c, Piece::RiderFill);
                    c.spawn(small_text(Line::RiderWord, 20.0));
                });
            });
            // The bike: silhouette with its parts, bar, words.
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(8),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                Piece::BikeBlock,
            ))
            .with_children(|b| {
                b.spawn(Node {
                    width: px(81),
                    height: px(48),
                    ..default()
                })
                .with_children(|icon| {
                    let wheel = |left: f32| Node {
                        border: UiRect::all(px(4.5)),
                        ..piece(left, 15.0, 30.0, 30.0, 15.0)
                    };
                    icon.spawn((
                        wheel(0.0),
                        BorderColor::all(FINE),
                        Piece::Part(Part::RearWheel),
                    ));
                    icon.spawn((
                        wheel(51.0),
                        BorderColor::all(FINE),
                        Piece::Part(Part::FrontWheel),
                    ));
                    // Frame: top tube and seat tube; chain; bars.
                    icon.spawn((
                        piece(15.0, 15.0, 51.0, 4.5, 1.5),
                        BackgroundColor(FINE),
                        Piece::Part(Part::Frame),
                    ));
                    icon.spawn((
                        piece(28.5, 9.0, 4.5, 22.5, 1.5),
                        BackgroundColor(FINE),
                        Piece::Part(Part::Frame),
                    ));
                    icon.spawn((
                        piece(15.0, 31.5, 21.0, 3.0, 1.5),
                        BackgroundColor(FINE),
                        Piece::Part(Part::Drivetrain),
                    ));
                    icon.spawn((
                        piece(60.0, 4.5, 15.0, 4.5, 1.5),
                        BackgroundColor(FINE),
                        Piece::Part(Part::Cockpit),
                    ));
                });
                b.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                })
                .with_children(|c| {
                    c.spawn(small_text(Line::BikeHead, 20.0));
                    bar(c, Piece::BikeFill);
                    c.spawn(small_text(Line::BikeWord, 20.0));
                });
            });
            // The part labels, the stop and the last hit's loss.
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(1),
                ..default()
            })
            .with_children(|c| {
                c.spawn(small_text(Line::Loss, 24.0));
                c.spawn(small_text(Line::Labels, 18.0));
            });
        });
}

/// The status a bike icon piece shows: wheels with their tyre.
fn part_light(view: &DamageWidgetView, p: Part) -> Option<Status> {
    let of = |q: Part| view.parts[q.index()];
    match p {
        Part::FrontWheel => of(Part::FrontWheel).max(of(Part::FrontTyre)),
        Part::RearWheel => of(Part::RearWheel).max(of(Part::RearTyre)),
        _ => of(p),
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

#[allow(clippy::type_complexity)]
fn update_widget(
    view: Res<crate::RideView>,
    show: Res<ShowDamage>,
    mut root: Query<&mut Node, (With<DamageWidget>, Without<Piece>)>,
    mut pieces: Query<
        (
            &Piece,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            &mut Node,
        ),
        Without<DamageWidget>,
    >,
    mut texts: Query<(&Line, &mut Text, &mut TextColor)>,
) {
    let w = view.damage.as_ref().filter(|_| show.0);
    for mut n in &mut root {
        let want = if w.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if n.display != want {
            n.display = want;
        }
    }
    let Some(w) = w else { return };
    for (p, bg, border, mut node) in &mut pieces {
        match *p {
            Piece::Rider => {
                if let Some(mut bg) = bg {
                    bg.0 = match w.rider.status {
                        Status::Good => FINE,
                        s => status_color(s),
                    };
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
                    bg.0 = if m.flash > 0.0 {
                        status_color(m.status.max(Status::Warning)).with_alpha(0.55 * m.flash)
                    } else {
                        Color::NONE
                    };
                }
            }
            Piece::Part(part) => {
                let c = part_light(w, part).map_or(FINE, status_color);
                if let Some(mut b) = border {
                    *b = BorderColor::all(c);
                }
                if let Some(mut bg) = bg
                    && !matches!(part, Part::FrontWheel | Part::RearWheel)
                {
                    bg.0 = c;
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
mod tests {
    use super::*;
    use crate::RideView;
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
        // The rear wheel lights up on the bike icon, the front stays grey.
        let w = app.world_mut();
        let mut q = w.query::<(&Piece, &BorderColor)>();
        let rear = q
            .iter(w)
            .find(|(p, _)| **p == Piece::Part(Part::RearWheel))
            .unwrap()
            .1;
        assert_eq!(rear.top, status_color(Status::Serious));
        let front = q
            .iter(w)
            .find(|(p, _)| **p == Piece::Part(Part::FrontWheel))
            .unwrap()
            .1;
        assert_eq!(front.top, FINE);
        // The bars follow the shown value.
        let mut q = w.query::<(&Piece, &Node)>();
        let fill = q.iter(w).find(|(p, _)| **p == Piece::RiderFill).unwrap().1;
        assert_eq!(fill.width, Val::Percent(62.0));
        // Hidden by the setting.
        app.world_mut().resource_mut::<ShowDamage>().0 = false;
        app.update();
        assert_eq!(display(&mut app), Display::None);
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
}
