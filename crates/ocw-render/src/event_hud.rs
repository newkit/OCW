//! The event overlay (spec §2.3): the big countdown in the middle
//! ("Get ready", 3-2-1, GO!), the event panel on the right (title,
//! position and gaps, distance to go, the ghost's gap) and the results
//! panel in the middle after the finish. It only lays out
//! [`RideView::event`]; the texts come from the app (Bevy-free, tested
//! there). Sizes are shares of the window height (ten-foot UI, §3.2.1).

use bevy::prelude::*;
use bevy::text::FontSize;

use crate::{EventView, ResultRow, RideView};

pub struct EventHudPlugin;

impl Plugin for EventHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_event_hud)
            .add_systems(Update, update_event_hud);
    }
}

/// Rows of the results panel (the app sends at most 10 + the local rider).
pub const MAX_ROWS: usize = 11;
/// Longest name in the results table (characters).
pub const NAME_CHARS: usize = 16;

const PANEL: Color = Color::srgba(0.02, 0.03, 0.05, 0.82);
const GOLD: Color = Color::srgb(1.0, 0.78, 0.24);
const DIM: Color = Color::srgb(0.8, 0.82, 0.86);
/// The local rider's row (the jersey's pink, lighter).
const LOCAL: Color = Color::srgb(1.0, 0.55, 0.75);

#[derive(Component)]
struct CountdownText;
#[derive(Component)]
struct EventPanel;
#[derive(Component)]
struct EventTitle;
#[derive(Component)]
struct EventLines;
#[derive(Component)]
struct ResultsPanel;
#[derive(Component)]
struct ResultsTitle;
#[derive(Component)]
struct ResultsHeader;
#[derive(Component)]
struct ResultsRow(usize);
#[derive(Component)]
struct ResultsFooter;

fn font(vh: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    }
}

/// One results row as monospaced columns (Bevy's default font is
/// monospaced): position, name (cut to [`NAME_CHARS`]), time, gap.
pub fn result_line(row: &ResultRow) -> String {
    let name: String = if row.name.chars().count() > NAME_CHARS {
        let cut: String = row.name.chars().take(NAME_CHARS - 1).collect();
        format!("{}.", cut.trim_end())
    } else {
        row.name.clone()
    };
    format!(
        "{:>3}  {:<w$}  {:>9}  {}",
        row.position,
        name,
        row.time,
        row.gap,
        w = NAME_CHARS
    )
    .trim_end()
    .to_string()
}

/// The header line over the rows.
pub fn header_line() -> String {
    format!(
        "{:>3}  {:<w$}  {:>9}  {}",
        "",
        "Rider",
        "Time",
        "Gap",
        w = NAME_CHARS
    )
}

fn spawn_event_hud(mut commands: Commands) {
    // The countdown, centred a little above the middle.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: percent(22),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_child((
            Text::new(""),
            font(16.0),
            TextColor(GOLD),
            TextShadow::default(),
            Visibility::Hidden,
            CountdownText,
        ));
    // The event panel: right, below the REC indicator's place.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: percent(21),
                right: px(12),
                padding: UiRect::axes(px(12), px(8)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(4),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            EventPanel,
        ))
        .with_children(|p| {
            p.spawn((Text::new(""), font(3.2), TextColor(GOLD), EventTitle));
            p.spawn((
                Text::new(""),
                font(3.0),
                TextColor(Color::WHITE),
                EventLines,
            ));
        });
    // The results panel, centred.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .insert(Pickable::IGNORE)
        .with_children(|full| {
            full.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Vh(2.5)),
                    row_gap: Val::Vh(0.6),
                    border_radius: BorderRadius::all(px(10)),
                    border: UiRect::all(px(2)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(GOLD),
                ResultsPanel,
            ))
            .with_children(|panel| {
                panel.spawn((Text::new(""), font(4.6), TextColor(GOLD), ResultsTitle));
                panel.spawn((
                    Text::new(header_line()),
                    font(3.3),
                    TextColor(DIM),
                    ResultsHeader,
                ));
                for i in 0..MAX_ROWS {
                    panel.spawn((
                        Text::new(""),
                        font(3.3),
                        TextColor(Color::WHITE),
                        Node {
                            display: Display::None,
                            ..default()
                        },
                        ResultsRow(i),
                    ));
                }
                panel.spawn((Text::new(""), font(3.0), TextColor(DIM), ResultsFooter));
            });
        });
}

type Texts<'w, 's, M> = Query<'w, 's, &'static mut Text, M>;

fn set_text(text: &mut Text, s: &str) {
    if text.0 != s {
        text.0 = s.to_owned();
    }
}

fn set_display(node: &mut Node, show: bool) {
    let d = if show { Display::Flex } else { Display::None };
    if node.display != d {
        node.display = d;
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system
fn update_event_hud(
    view: Res<RideView>,
    mut countdown: Query<(&mut Text, &mut Visibility), With<CountdownText>>,
    mut panel: Query<&mut Node, (With<EventPanel>, Without<ResultsPanel>, Without<ResultsRow>)>,
    mut title: Texts<(
        With<EventTitle>,
        Without<EventLines>,
        Without<CountdownText>,
    )>,
    mut lines: Texts<(
        With<EventLines>,
        Without<EventTitle>,
        Without<CountdownText>,
    )>,
    mut results: Query<&mut Node, (With<ResultsPanel>, Without<EventPanel>, Without<ResultsRow>)>,
    mut results_title: Texts<(
        With<ResultsTitle>,
        Without<ResultsFooter>,
        Without<ResultsRow>,
        Without<EventTitle>,
        Without<EventLines>,
        Without<CountdownText>,
    )>,
    mut rows: Query<
        (&ResultsRow, &mut Text, &mut TextColor, &mut Node),
        (
            Without<ResultsPanel>,
            Without<EventPanel>,
            Without<ResultsTitle>,
            Without<ResultsFooter>,
            Without<EventTitle>,
            Without<EventLines>,
            Without<CountdownText>,
        ),
    >,
    mut footer: Texts<(
        With<ResultsFooter>,
        Without<ResultsTitle>,
        Without<ResultsRow>,
        Without<EventTitle>,
        Without<EventLines>,
        Without<CountdownText>,
    )>,
) {
    let empty = EventView::default();
    let ev = view.event.as_ref().unwrap_or(&empty);
    if let Ok((mut text, mut vis)) = countdown.single_mut() {
        let c = ev.countdown.as_deref().unwrap_or("");
        set_text(&mut text, c);
        vis.set_if_neq(if c.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    if let Ok(mut node) = panel.single_mut() {
        // The results panel says it all; the side panel steps aside.
        set_display(&mut node, view.event.is_some() && ev.results.is_none());
    }
    if let Ok(mut t) = title.single_mut() {
        set_text(&mut t, &ev.title);
    }
    if let Ok(mut t) = lines.single_mut() {
        set_text(&mut t, &ev.lines.join("\n"));
    }
    let res = ev.results.as_ref();
    if let Ok(mut node) = results.single_mut() {
        set_display(&mut node, res.is_some());
    }
    let Some(res) = res else { return };
    if let Ok(mut t) = results_title.single_mut() {
        set_text(&mut t, &res.title);
    }
    for (row, mut text, mut color, mut node) in &mut rows {
        match res.rows.get(row.0) {
            Some(r) => {
                set_display(&mut node, true);
                set_text(&mut text, &result_line(r));
                let c = if r.is_local { LOCAL } else { Color::WHITE };
                if color.0 != c {
                    color.0 = c;
                }
            }
            None => set_display(&mut node, false),
        }
    }
    if let Ok(mut t) = footer.single_mut() {
        set_text(&mut t, &res.footer.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResultsView;
    use ocw_world::Route;

    fn row(pos: &str, name: &str, time: &str, gap: &str) -> ResultRow {
        ResultRow {
            position: pos.into(),
            name: name.into(),
            time: time.into(),
            gap: gap.into(),
            is_local: false,
        }
    }

    #[test]
    fn result_rows_line_up_in_columns() {
        let a = result_line(&row("1", "Bot 3", "10:00.0", ""));
        let b = result_line(&row(
            "12",
            "A rider with a very long name",
            "1:02:03.4",
            "+4.2 s",
        ));
        // Columns: 3 + 2, name 16 + 2, time 9 (right-aligned) + 2, gap.
        assert!(a.starts_with("  1  Bot 3 "), "{a:?}");
        assert_eq!(a.find("10:00.0"), Some(25), "{a:?}");
        assert!(a.ends_with("10:00.0"), "no trailing blanks");
        assert!(b.starts_with(" 12  A rider with a. "), "{b:?}");
        assert_eq!(b.find("1:02:03.4"), Some(23), "{b:?}");
        assert!(b.ends_with("  +4.2 s"));
        assert_eq!(header_line().find("Time"), Some(28));
        assert!(header_line().starts_with("     Rider"));
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(EventHudPlugin);
        app.update();
        app
    }

    fn shown<C: Component>(app: &mut App) -> bool {
        let mut q = app.world_mut().query_filtered::<&Node, With<C>>();
        q.single(app.world()).unwrap().display != Display::None
    }

    #[test]
    fn overlay_follows_the_event_view() {
        let mut app = app();
        assert!(!shown::<EventPanel>(&mut app));
        assert!(!shown::<ResultsPanel>(&mut app));
        app.world_mut().resource_mut::<RideView>().event = Some(EventView {
            countdown: Some("3".into()),
            title: "Race - 0.3 km".into(),
            lines: vec!["P1/4".into()],
            results: None,
        });
        app.update();
        assert!(shown::<EventPanel>(&mut app));
        assert!(!shown::<ResultsPanel>(&mut app));
        let mut q = app
            .world_mut()
            .query_filtered::<(&Text, &Visibility), With<CountdownText>>();
        let (t, v) = q.single(app.world()).unwrap();
        assert_eq!((t.0.as_str(), *v), ("3", Visibility::Inherited));
        // Results: the rows given, the rest hidden.
        app.world_mut().resource_mut::<RideView>().event = Some(EventView {
            results: Some(ResultsView {
                title: "Race - results".into(),
                rows: vec![
                    row("1", "a", "1:00.0", ""),
                    row("2", "b", "1:01.0", "+1.0 s"),
                ],
                footer: vec!["Esc / A: close".into()],
            }),
            ..Default::default()
        });
        app.update();
        assert!(shown::<ResultsPanel>(&mut app));
        assert!(
            !shown::<EventPanel>(&mut app),
            "steps aside for the results"
        );
        let mut q = app.world_mut().query::<(&ResultsRow, &Node)>();
        let visible = q
            .iter(app.world())
            .filter(|(_, n)| n.display != Display::None)
            .count();
        assert_eq!(visible, 2);
        app.world_mut().resource_mut::<RideView>().event = None;
        app.update();
        assert!(!shown::<EventPanel>(&mut app) && !shown::<ResultsPanel>(&mut app));
    }
}
