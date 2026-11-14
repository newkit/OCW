//! The in-game menu as a Bevy-free model (spec §3.2.1): pages, the page
//! stack, the focused item. Input arrives as abstract [`Action`]s; what the
//! app has to do comes back as an [`Effect`]. Menu functions are console
//! command lines, so menu, console and scripts share one code path.
//!
//! The Settings sub pages, the Workshop (M2) and the lists of the Ride and
//! Navi pages (M3) are [`Page::Sub`]: their rows come from
//! `settings::pages` / `ride::pages` (current values, left / right change
//! them), passed in with [`Menu::apply_with`]. Longer lists are paged
//! ([`PAGE_ROWS`] rows, then Previous / Next), so no page has more than
//! [`MAX_ITEMS`] items (ten-foot rule). Rows that would end the ride
//! (another ride) ask first ([`Page::ConfirmSwitch`]), as do event starts
//! ([`Page::ConfirmEvent`]); Cancel is focused on both.

use crate::settings::pages::{self as sub, Activate, Row};

/// At most this many items on a page (ten-foot UI, spec §3.2.1).
pub const MAX_ITEMS: usize = 7;
/// Rows per page of a paged list (+ Previous, Next and Back).
pub const PAGE_ROWS: usize = 4;

/// Abstract menu input (keyboard, mouse and gamepad map onto these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the menu at its top page (Esc / Start while it is closed).
    Open,
    /// Close the menu from any page (Start while it is open).
    Close,
    Up,
    Down,
    /// Change the focused row's value (value rows of [`Page::Sub`]); on a
    /// row without a value of a paged list: the previous / next page.
    Left,
    Right,
    /// The previous / next tab of the Settings and Workshop pages (Q / E,
    /// PgUp / PgDn, LB / RB).
    TabPrev,
    TabNext,
    Confirm,
    /// One page up; closes the menu at the top page.
    Back,
    /// The mouse moved onto item `i`.
    Hover(usize),
    /// Item `i` was clicked.
    Click(usize),
    /// Open the menu at the rider list of the Workshop page (the scene's
    /// changing corner until the character editor, W4).
    RiderPage,
}

/// What the app does after an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    /// The menu opened: hold the steering in autodrive.
    Opened,
    /// The menu closed: restore the steering.
    Closed,
    /// Run this console command line (e.g. `reset`, `quit`).
    Command(String),
}

/// The menu's pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Top,
    Ride,
    Navi,
    Settings,
    /// A Settings sub page, the Workshop or a list of the Ride / Navi
    /// pages, with rows from `settings::pages` / `ride::pages`.
    Sub(sub::Page),
    ConfirmRestart,
    ConfirmQuit,
    /// "Change the ride?" before a `ride.switch` line ([`Menu::pending`]).
    ConfirmSwitch,
    /// "Start the event?" before an `event …` line ([`Menu::pending`]).
    ConfirmEvent,
}

/// What an item does when selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// Close the menu, continue the ride.
    Resume,
    /// Open a sub page.
    Open(Page),
    /// One page up.
    Back,
    /// Run a console command line; `close` = close the menu afterwards.
    Command { line: &'static str, close: bool },
    /// Ask first ([`Page::ConfirmEvent`]), then start the event and close.
    AskEvent(&'static str),
    /// Run the asked line ([`Menu::pending`]) and close the menu.
    RunPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub kind: ItemKind,
}

const fn item(label: &'static str, kind: ItemKind) -> Item {
    Item { label, kind }
}

const BACK: Item = item("Back", ItemKind::Back);

/// The top page (spec §3.2.1): seven items, the ten-foot maximum, so
/// World took Restart's place (Restart is on the Ride page now).
const TOP_ITEMS: &[Item] = &[
    item("Resume", ItemKind::Resume),
    item("Ride", ItemKind::Open(Page::Ride)),
    // The world, its data and its conditions (time, weather).
    item("World", ItemKind::Open(Page::Sub(sub::Page::WorldHome))),
    // The workshop scene (spec §14 Phase 4, 4A): the menu closes, the
    // scene opens; leaving it comes back to this page.
    item(
        "Workshop",
        ItemKind::Command {
            line: "workshop open",
            close: true,
        },
    ),
    item("Navi", ItemKind::Open(Page::Navi)),
    item("Settings", ItemKind::Open(Page::Settings)),
    item("Quit", ItemKind::Open(Page::ConfirmQuit)),
];
/// Ride page: local events (spec §2.3), started as console commands after
/// a confirmation (the menu closes so the countdown is visible), workouts
/// and Restart. Another ride: World → Change world.
const RIDE_ITEMS: &[Item] = &[
    item(
        "Race: 1 lap / 5 km, 5 bots",
        ItemKind::AskEvent("event race --bots 5"),
    ),
    item(
        "Group ride at 150 W",
        ItemKind::AskEvent("event group --watts 150"),
    ),
    item("Time trial vs your ghost", ItemKind::AskEvent("event tt")),
    item(
        "End the event",
        ItemKind::Command {
            line: "event cancel",
            close: true,
        },
    ),
    item("Workouts", ItemKind::Open(Page::Sub(sub::Page::Workouts))),
    item("Restart", ItemKind::Open(Page::ConfirmRestart)),
    BACK,
];
const NAVI_ITEMS: &[Item] = &[
    item("Routes", ItemKind::Open(Page::Sub(sub::Page::Routes))),
    item("Rides", ItemKind::Open(Page::Sub(sub::Page::Rides))),
    item(
        "GPX import",
        ItemKind::Open(Page::Sub(sub::Page::GpxImport)),
    ),
    // The Strava account page lives here (Settings is full, ten-foot rule).
    item("Strava", ItemKind::Open(Page::Sub(sub::Page::Strava))),
    // The time machine moved to World → Time (a condition of the world).
    BACK,
];
const SETTINGS_ITEMS: &[Item] = &[
    item("Riders", ItemKind::Open(Page::Sub(sub::Page::Riders))),
    item("Controls", ItemKind::Open(Page::Sub(sub::Page::Controls))),
    item("Trainer", ItemKind::Open(Page::Sub(sub::Page::Trainer))),
    item("Audio", ItemKind::Open(Page::Sub(sub::Page::Audio))),
    item("Display", ItemKind::Open(Page::Sub(sub::Page::Display))),
    item("Voice", ItemKind::Open(Page::Sub(sub::Page::Voice))),
    BACK,
];
const RESTART_ITEMS: &[Item] = &[
    item(
        "Restart now",
        ItemKind::Command {
            line: "reset",
            close: false,
        },
    ),
    item("Cancel", ItemKind::Back),
];
const QUIT_ITEMS: &[Item] = &[
    item(
        "Quit now",
        ItemKind::Command {
            line: "quit",
            close: false,
        },
    ),
    item("Cancel", ItemKind::Back),
];
const SWITCH_ITEMS: &[Item] = &[
    item("Ride now", ItemKind::RunPending),
    item("Cancel", ItemKind::Back),
];
const EVENT_ITEMS: &[Item] = &[
    item("Start now", ItemKind::RunPending),
    item("Cancel", ItemKind::Back),
];

/// Title, description paragraphs (wrapped to the panel), items and the initially focused item of a page.
pub struct PageDef {
    pub title: &'static str,
    pub text: &'static [&'static str],
    pub items: &'static [Item],
    pub initial_focus: usize,
}

impl Page {
    pub fn def(self) -> PageDef {
        match self {
            Page::Top => PageDef {
                title: "Open Cycling World",
                text: &[],
                items: TOP_ITEMS,
                initial_focus: 0,
            },
            Page::Ride => PageDef {
                title: "Ride",
                text: &[
                    "Local events start on the line with 3-2-1-GO and end with results; a time trial races your ghost.",
                    "Workouts: ERG mode. Another world or route: World -> Change world, Navi -> Routes.",
                ],
                items: RIDE_ITEMS,
                initial_focus: 0,
            },
            Page::Navi => PageDef {
                title: "Navi",
                text: &[
                    "Routes of this world, your saved rides, GPX import onto the roads of the loaded world package. The time machine: World -> Time.",
                ],
                items: NAVI_ITEMS,
                initial_focus: 0,
            },
            Page::Settings => PageDef {
                title: "Settings",
                text: &[
                    "Saved in settings.toml and riders.toml (console: set, get, riders.*, audio.*).",
                ],
                items: SETTINGS_ITEMS,
                initial_focus: 0,
            },
            // The rows are dynamic (`Menu::refresh`); Back is added after them.
            Page::Sub(p) => PageDef {
                title: p.title(),
                text: p.notes(),
                items: &[],
                initial_focus: 0,
            },
            Page::ConfirmRestart => PageDef {
                title: "Restart?",
                text: &["Start everything again as at program start. The ride is saved first."],
                items: RESTART_ITEMS,
                // Cancel: a double press does not restart by accident.
                initial_focus: 1,
            },
            Page::ConfirmQuit => PageDef {
                title: "Quit?",
                text: &["The ride is saved, then the app exits."],
                items: QUIT_ITEMS,
                initial_focus: 1,
            },
            Page::ConfirmSwitch => PageDef {
                title: "Change the ride?",
                text: &[
                    "The ride so far is saved and the new one starts at once. Devices stay connected.",
                ],
                items: SWITCH_ITEMS,
                initial_focus: 1,
            },
            Page::ConfirmEvent => PageDef {
                title: "Start the event?",
                text: &[
                    "You are moved to the start line for the countdown; a running event ends. The ride recording goes on.",
                ],
                items: EVENT_ITEMS,
                initial_focus: 1,
            },
        }
    }
}

/// One open page: which, the focused item, the shown part of a paged list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Level {
    page: Page,
    focus: usize,
    /// Page of a paged list (0 = the first [`PAGE_ROWS`] rows).
    scroll: usize,
}

/// An item of a [`Page::Sub`] page as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Row(usize),
    Prev,
    Next,
    Back,
}

/// The menu: closed, or a stack of open pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menu {
    stack: Vec<Level>,
    /// The rows of the current [`Page::Sub`] (refreshed by `apply_with`).
    rows: Vec<Row>,
    /// The command line [`Page::ConfirmSwitch`] runs when confirmed.
    pending: Option<String>,
}

impl Menu {
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// The current page, if open.
    pub fn page(&self) -> Option<Page> {
        self.stack.last().map(|l| l.page)
    }

    /// The focused item index on the current page.
    pub fn focus(&self) -> usize {
        self.stack.last().map_or(0, |l| l.focus)
    }

    /// Pages from the top page to the current one.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The line a confirmation page would run.
    #[cfg(test)]
    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    pub fn items(&self) -> &'static [Item] {
        self.page().map_or(&[], |p| p.def().items)
    }

    fn scroll(&self) -> usize {
        self.stack.last().map_or(0, |l| l.scroll)
    }

    /// Pages of the current list (1 if it is not paged).
    pub fn list_pages(&self) -> usize {
        if self.rows.len() < MAX_ITEMS {
            1
        } else {
            self.rows.len().div_ceil(PAGE_ROWS)
        }
    }

    /// The items of the current [`Page::Sub`] page: all rows and Back, or
    /// one page of rows, Previous / Next and Back.
    fn slots(&self) -> Vec<Slot> {
        let n = self.rows.len();
        let pages = self.list_pages();
        if pages == 1 {
            return (0..n).map(Slot::Row).chain([Slot::Back]).collect();
        }
        let p = self.scroll().min(pages - 1);
        let mut out: Vec<Slot> = (p * PAGE_ROWS..(p * PAGE_ROWS + PAGE_ROWS).min(n))
            .map(Slot::Row)
            .collect();
        if p > 0 {
            out.push(Slot::Prev);
        }
        if p + 1 < pages {
            out.push(Slot::Next);
        }
        out.push(Slot::Back);
        out
    }

    /// Number of focusable items on the current page.
    pub fn item_count(&self) -> usize {
        match self.page() {
            Some(Page::Sub(_)) => self.slots().len(),
            _ => self.items().len(),
        }
    }

    /// The item labels of the current page, as the menu shows them: value
    /// rows as "Label:  < value >".
    pub fn labels(&self) -> Vec<String> {
        match self.page() {
            Some(Page::Sub(_)) => {
                let (p, pages) = (self.scroll() + 1, self.list_pages());
                self.slots()
                    .into_iter()
                    .map(|s| match s {
                        Slot::Row(i) => row_label(&self.rows[i]),
                        Slot::Prev => format!("Previous ({p}/{pages})"),
                        Slot::Next => format!("Next ({p}/{pages})"),
                        Slot::Back => "Back".to_string(),
                    })
                    .collect()
            }
            _ => self.items().iter().map(|i| i.label.to_string()).collect(),
        }
    }

    /// Replaces the current tab page (Settings sub pages, Workshop) with
    /// the previous / next one.
    fn switch_tab(&mut self, dir: isize) {
        let Some(Page::Sub(p)) = self.page() else {
            return;
        };
        let p = p.parent().unwrap_or(p);
        let Some(at) = sub::Page::TABS.iter().position(|t| *t == p) else {
            return;
        };
        let n = sub::Page::TABS.len() as isize;
        let next = sub::Page::TABS[(at as isize + dir).rem_euclid(n) as usize];
        self.stack.pop();
        self.push(Page::Sub(next));
        self.rows.clear();
    }

    fn push(&mut self, page: Page) {
        self.stack.push(Level {
            page,
            focus: page.def().initial_focus,
            scroll: 0,
        });
    }

    fn close(&mut self) -> Effect {
        if self.stack.is_empty() {
            return Effect::None;
        }
        self.stack.clear();
        self.rows.clear();
        self.pending = None;
        Effect::Closed
    }

    fn set_focus(&mut self, i: usize) {
        if let Some(l) = self.stack.last_mut() {
            l.focus = i;
        }
    }

    /// Moves the focus by `step`, wrapping around at both ends.
    fn move_focus(&mut self, step: isize) {
        let n = self.item_count();
        if n == 0 {
            return;
        }
        let f = (self.focus() as isize + step).rem_euclid(n as isize);
        self.set_focus(f as usize);
    }

    /// Shows the previous / next page of a paged list; the focus stays on
    /// the same kind of item (Next stays on Next while there is one).
    fn turn_page(&mut self, dir: isize) -> bool {
        let pages = self.list_pages() as isize;
        let at = self.scroll() as isize;
        let to = at + dir;
        if pages < 2 || !(0..pages).contains(&to) {
            return false;
        }
        let was = self.slots().get(self.focus()).copied();
        if let Some(l) = self.stack.last_mut() {
            l.scroll = to as usize;
        }
        let slots = self.slots();
        let f = was
            .and_then(|w| slots.iter().position(|s| *s == w))
            .or_else(|| {
                slots
                    .iter()
                    .position(|s| matches!(s, Slot::Prev | Slot::Next))
            })
            .unwrap_or(0);
        self.set_focus(f);
        true
    }

    fn ask(&mut self, line: String) -> Effect {
        self.ask_on(Page::ConfirmSwitch, line)
    }

    fn ask_on(&mut self, page: Page, line: String) -> Effect {
        self.pending = Some(line);
        self.push(page);
        Effect::None
    }

    /// Closes the menu and runs `line`.
    fn run_and_close(&mut self, line: String) -> Effect {
        self.stack.clear();
        self.rows.clear();
        self.pending = None;
        Effect::Command(line)
    }

    fn select(&mut self, i: usize) -> Effect {
        if let Some(Page::Sub(_)) = self.page() {
            return self.select_row(i);
        }
        let Some(item) = self.items().get(i).copied() else {
            return Effect::None;
        };
        self.set_focus(i);
        match item.kind {
            ItemKind::Resume => self.close(),
            ItemKind::Open(page) => {
                self.push(page);
                Effect::None
            }
            ItemKind::Back => self.back(),
            ItemKind::Command { line, close } => {
                if close {
                    self.stack.clear();
                }
                Effect::Command(line.to_owned())
            }
            ItemKind::AskEvent(line) => self.ask_on(Page::ConfirmEvent, line.to_owned()),
            ItemKind::RunPending => match self.pending.take() {
                Some(line) => self.run_and_close(line),
                None => self.back(),
            },
        }
    }

    fn select_row(&mut self, i: usize) -> Effect {
        let Some(slot) = self.slots().get(i).copied() else {
            return Effect::None;
        };
        self.set_focus(i);
        let row = match slot {
            Slot::Back => return self.back(),
            Slot::Prev => {
                self.turn_page(-1);
                return Effect::None;
            }
            Slot::Next => {
                self.turn_page(1);
                return Effect::None;
            }
            Slot::Row(r) => r,
        };
        match self.rows[row].confirm.clone() {
            Some(Activate::Command(line)) => Effect::Command(line),
            Some(Activate::Ask(line)) => self.ask(line),
            Some(Activate::Run(line)) => self.run_and_close(line),
            Some(Activate::Open(p)) => {
                self.push(Page::Sub(p));
                self.rows.clear();
                Effect::None
            }
            None => Effect::None,
        }
    }

    /// Left / right on a value row: its command line; on another row of a
    /// paged list: the previous / next page.
    fn change(&mut self, right: bool) -> Effect {
        let row = match self.slots().get(self.focus()) {
            Some(Slot::Row(r)) => self.rows.get(*r),
            _ => None,
        };
        match row.and_then(|row| if right { &row.right } else { &row.left }.clone()) {
            Some(line) => Effect::Command(line),
            None => {
                self.turn_page(if right { 1 } else { -1 });
                Effect::None
            }
        }
    }

    /// Applies one action, with the rows of the [`Page::Sub`] pages from
    /// `rows` (read again before and after the action, so they show the
    /// current values).
    pub fn apply_with(&mut self, action: Action, rows: impl Fn(sub::Page) -> Vec<Row>) -> Effect {
        self.refresh(&rows);
        let effect = self.apply(action);
        self.refresh(&rows);
        effect
    }

    /// Reads the current page's rows again. A rider editor whose rider is
    /// gone (deleted), or a world that is no longer listed, goes back to
    /// the list.
    pub fn refresh(&mut self, rows: &impl Fn(sub::Page) -> Vec<Row>) {
        match self.page() {
            Some(Page::Sub(p)) => {
                self.rows = rows(p);
                if matches!(
                    p,
                    sub::Page::Rider(_)
                        | sub::Page::Bike(_)
                        | sub::Page::World(_)
                        | sub::Page::WorldRoutes(_)
                        | sub::Page::WorldParts(_)
                        | sub::Page::WorldSource(_)
                ) && self.rows.is_empty()
                {
                    self.stack.pop();
                    self.refresh(rows);
                    return;
                }
                let pages = self.list_pages();
                if let Some(l) = self.stack.last_mut() {
                    l.scroll = l.scroll.min(pages - 1);
                }
                let n = self.item_count();
                if self.focus() >= n {
                    self.set_focus(n - 1);
                }
            }
            _ => self.rows.clear(),
        }
    }

    fn back(&mut self) -> Effect {
        if self.stack.len() > 1 {
            if matches!(self.page(), Some(Page::ConfirmSwitch | Page::ConfirmEvent)) {
                self.pending = None;
            }
            self.stack.pop();
            self.rows.clear();
            Effect::None
        } else {
            self.close()
        }
    }

    /// Applies one action. A command item that closes the menu reports
    /// only its command; check [`Self::is_open`] for the close.
    pub fn apply(&mut self, action: Action) -> Effect {
        if action == Action::RiderPage {
            let opened = !self.is_open();
            self.stack.clear();
            self.rows.clear();
            self.push(Page::Top);
            self.push(Page::Sub(sub::Page::Workshop));
            return if opened { Effect::Opened } else { Effect::None };
        }
        if !self.is_open() {
            return match action {
                Action::Open => {
                    self.push(Page::Top);
                    Effect::Opened
                }
                _ => Effect::None,
            };
        }
        match action {
            Action::Open | Action::RiderPage => Effect::None,
            Action::Left | Action::Right => {
                if matches!(self.page(), Some(Page::Sub(_))) {
                    self.change(action == Action::Right)
                } else {
                    Effect::None
                }
            }
            Action::TabPrev | Action::TabNext => {
                self.switch_tab(if action == Action::TabNext { 1 } else { -1 });
                Effect::None
            }
            Action::Close => self.close(),
            Action::Up => {
                self.move_focus(-1);
                Effect::None
            }
            Action::Down => {
                self.move_focus(1);
                Effect::None
            }
            Action::Confirm => self.select(self.focus()),
            Action::Back => self.back(),
            Action::Hover(i) => {
                if i < self.item_count() {
                    self.set_focus(i);
                }
                Effect::None
            }
            Action::Click(i) => self.select(i),
        }
    }
}

/// "Label:  < value >" for a value row, "Label:  value" for a shown value.
pub fn row_label(r: &Row) -> String {
    match &r.value {
        Some(v) if r.left.is_some() || r.right.is_some() => format!("{}:  < {v} >", r.label),
        Some(v) => format!("{}:  {v}", r.label),
        None => r.label.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(m: &Menu) -> Vec<&'static str> {
        m.items().iter().map(|i| i.label).collect()
    }

    #[test]
    fn opens_at_the_top_page_with_resume_focused() {
        let mut m = Menu::default();
        assert!(!m.is_open());
        // Closed: only Open does anything.
        for a in [Action::Confirm, Action::Back, Action::Down, Action::Close] {
            assert_eq!(m.apply(a), Effect::None);
            assert!(!m.is_open());
        }
        assert_eq!(m.apply(Action::Open), Effect::Opened);
        assert_eq!(m.page(), Some(Page::Top));
        assert_eq!(
            labels(&m),
            [
                "Resume", "Ride", "World", "Workshop", "Navi", "Settings", "Quit"
            ]
        );
        assert_eq!(m.focus(), 0);
        // Open again while open: nothing.
        assert_eq!(m.apply(Action::Open), Effect::None);
        assert_eq!(m.apply(Action::Confirm), Effect::Closed);
        assert!(!m.is_open());
    }

    #[test]
    fn focus_moves_and_wraps() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Down);
        m.apply(Action::Down);
        assert_eq!(m.focus(), 2);
        m.apply(Action::Up);
        assert_eq!(m.focus(), 1);
        m.apply(Action::Up);
        m.apply(Action::Up);
        assert_eq!(m.focus(), 6, "wraps to Quit");
        m.apply(Action::Down);
        assert_eq!(m.focus(), 0, "wraps to Resume");
        // Left / right: no value rows yet.
        m.apply(Action::Right);
        assert_eq!((m.page(), m.focus()), (Some(Page::Top), 0));
        // Hover focuses, out of range is ignored.
        m.apply(Action::Hover(4));
        assert_eq!(m.focus(), 4);
        m.apply(Action::Hover(40));
        assert_eq!(m.focus(), 4);
    }

    #[test]
    fn page_stack_back_and_back_at_the_top_closes() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Hover(5));
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(Page::Settings));
        assert_eq!(m.depth(), 2);
        // Back returns to the top page with the focus where it was.
        assert_eq!(m.apply(Action::Back), Effect::None);
        assert_eq!((m.page(), m.focus()), (Some(Page::Top), 5));
        // The Back item of a page goes up too.
        m.apply(Action::Click(1));
        assert_eq!(m.page(), Some(Page::Ride));
        assert_eq!(labels(&m)[4..], ["Workouts", "Restart", "Back"]);
        m.apply(Action::Click(6));
        assert_eq!((m.page(), m.focus()), (Some(Page::Top), 1));
        // World: its own page (rows from the ride catalog).
        m.apply(Action::Click(2));
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::WorldHome)));
        m.apply(Action::Back);
        // Back at the top closes.
        assert_eq!(m.apply(Action::Back), Effect::Closed);
        assert!(!m.is_open());
        // Close from a sub page closes everything; reopening starts at the top.
        m.apply(Action::Open);
        m.apply(Action::Click(4));
        assert_eq!(m.page(), Some(Page::Navi));
        assert_eq!(
            labels(&m),
            ["Routes", "Rides", "GPX import", "Strava", "Back"]
        );
        assert_eq!(m.apply(Action::Close), Effect::Closed);
        m.apply(Action::Open);
        assert_eq!((m.page(), m.focus(), m.depth()), (Some(Page::Top), 0, 1));
    }

    #[test]
    fn restart_and_quit_ask_first_with_cancel_focused() {
        // Restart: on the Ride page.
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Click(1));
        m.apply(Action::Click(5));
        assert_eq!(m.page(), Some(Page::ConfirmRestart));
        assert_eq!(labels(&m)[m.focus()], "Cancel");
        // Confirming Cancel goes back, nothing is run.
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(Page::Ride));
        m.apply(Action::Click(5));
        m.apply(Action::Up);
        assert_eq!(m.apply(Action::Confirm), Effect::Command("reset".into()));
        // Quit.
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Click(6));
        assert_eq!(m.page(), Some(Page::ConfirmQuit));
        assert_eq!(m.apply(Action::Back), Effect::None);
        m.apply(Action::Click(6));
        assert_eq!(m.apply(Action::Click(0)), Effect::Command("quit".into()));
    }

    use crate::settings::registry::Settings;
    use crate::settings::riders::{Rider, Riders};

    fn riders() -> Riders {
        let mut r = Riders::default();
        for (name, kg) in [("Alex", 78.0), ("Anna", 61.0)] {
            r.add(Rider {
                name: name.into(),
                weight_kg: kg,
                ..Rider::default()
            })
            .unwrap();
        }
        r
    }

    /// Applies `a` with the rows of the given settings and riders.
    fn act(m: &mut Menu, a: Action, s: &Settings, r: &Riders) -> Effect {
        m.apply_with(a, |p| sub::rows(p, s, r))
    }

    #[test]
    fn ride_page_starts_events_and_closes() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Click(1));
        assert_eq!(m.page(), Some(Page::Ride));
        assert_eq!(
            labels(&m),
            [
                "Race: 1 lap / 5 km, 5 bots",
                "Group ride at 150 W",
                "Time trial vs your ghost",
                "End the event",
                "Workouts",
                "Restart",
                "Back"
            ]
        );
        // An event start asks first, with Cancel focused.
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(Page::ConfirmEvent));
        assert_eq!(m.labels(), ["Start now", "Cancel"]);
        assert_eq!(m.focus(), 1);
        assert_eq!(m.pending(), Some("event race --bots 5"));
        // Cancel: back on the Ride page, nothing run.
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!((m.page(), m.focus()), (Some(Page::Ride), 0));
        assert_eq!(m.pending(), None);
        // Start now: runs the line and closes for the countdown.
        m.apply(Action::Confirm);
        assert_eq!(
            m.apply(Action::Click(0)),
            Effect::Command("event race --bots 5".into())
        );
        assert!(!m.is_open(), "closes for the countdown");
        m.apply(Action::Open);
        m.apply(Action::Click(1));
        m.apply(Action::Click(2));
        assert_eq!(m.pending(), Some("event tt"));
        // Back (Esc) cancels too.
        m.apply(Action::Back);
        assert_eq!((m.page(), m.pending()), (Some(Page::Ride), None));
        m.apply(Action::Click(2));
        assert_eq!(
            m.apply(Action::Click(0)),
            Effect::Command("event tt".into())
        );
        // Ending an event is not asked.
        m.apply(Action::Open);
        m.apply(Action::Click(1));
        assert_eq!(
            m.apply(Action::Click(3)),
            Effect::Command("event cancel".into())
        );
        m.apply(Action::Open);
        m.apply(Action::Click(1));
        m.apply(Action::Click(6));
        assert_eq!(m.page(), Some(Page::Top));
    }

    #[test]
    fn settings_pages_change_values_with_left_and_right() {
        let (s, r) = (Settings::default(), riders());
        let mut m = Menu::default();
        act(&mut m, Action::Open, &s, &r);
        act(&mut m, Action::Click(5), &s, &r);
        assert_eq!(m.page(), Some(Page::Settings));
        // Display: value rows.
        act(&mut m, Action::Click(4), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Display)));
        assert_eq!(m.labels()[0], "HUD:  < on >");
        assert_eq!(m.labels().last().unwrap(), "Back");
        assert_eq!(
            act(&mut m, Action::Right, &s, &r),
            Effect::Command("set display.hud off".into())
        );
        act(&mut m, Action::Down, &s, &r);
        assert_eq!(
            act(&mut m, Action::Left, &s, &r),
            Effect::Command("set display.units imperial".into())
        );
        assert_eq!(
            act(&mut m, Action::Confirm, &s, &r),
            Effect::Command("set display.units imperial".into())
        );
        // A row without left / right (the Back item): nothing.
        act(&mut m, Action::Up, &s, &r);
        act(&mut m, Action::Up, &s, &r);
        assert_eq!(m.focus(), m.item_count() - 1);
        assert_eq!(act(&mut m, Action::Right, &s, &r), Effect::None);
        // Back (the item) returns to Settings.
        assert_eq!(act(&mut m, Action::Confirm, &s, &r), Effect::None);
        assert_eq!(m.page(), Some(Page::Settings));
        // Trainer: an action row runs its command.
        act(&mut m, Action::Click(2), &s, &r);
        let scan = m
            .labels()
            .iter()
            .position(|l| l == "Scan for devices")
            .unwrap();
        assert_eq!(
            act(&mut m, Action::Click(scan), &s, &r),
            Effect::Command("devices.scan".into())
        );
        // Left / right do nothing on pages without value rows.
        let mut top = Menu::default();
        top.apply(Action::Open);
        assert_eq!(top.apply(Action::Right), Effect::None);
    }

    /// *Workshop* opens the workshop scene (spec §14 Phase 4, 4A): the
    /// menu closes and runs `workshop open`; the rider list stays
    /// reachable (`menu workshop-page`, the scene's changing corner).
    #[test]
    fn workshop_opens_the_scene() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        assert_eq!(m.items()[3].label, "Workshop");
        assert_eq!(
            m.apply(Action::Click(3)),
            Effect::Command("workshop open".into())
        );
        assert!(!m.is_open());
        assert_eq!(m.apply(Action::RiderPage), Effect::Opened);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Workshop)));
        assert_eq!(m.depth(), 2);
        assert_eq!(m.apply(Action::RiderPage), Effect::None, "already open");
    }

    #[test]
    fn tabs_switch_between_the_settings_pages_and_the_workshop() {
        let (s, r) = (Settings::default(), riders());
        let mut m = Menu::default();
        act(&mut m, Action::RiderPage, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Workshop)));
        assert_eq!(m.depth(), 2);
        act(&mut m, Action::TabNext, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Riders)));
        act(&mut m, Action::TabPrev, &s, &r);
        act(&mut m, Action::TabPrev, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Strava)), "wraps");
        assert_eq!(m.depth(), 2, "tabs replace the page");
        // Not on the top page.
        let mut top = Menu::default();
        top.apply(Action::Open);
        top.apply(Action::TabNext);
        assert_eq!(top.page(), Some(Page::Top));
    }

    #[test]
    fn workshop_chooses_the_rider_and_opens_the_editor() {
        let s = Settings::default();
        let mut r = riders();
        let mut m = Menu::default();
        act(&mut m, Action::RiderPage, &s, &r);
        assert_eq!(m.labels()[0], "Alex:  78 kg - riding");
        assert_eq!(
            act(&mut m, Action::Click(1), &s, &r),
            Effect::Command("riders.select \"Anna\"".into())
        );
        // "Bike" opens the active rider's bike page; Back returns.
        assert_eq!(
            m.labels()[2],
            "Bike:  Road - standard, frame #1E1E22, jersey #FF1472"
        );
        act(&mut m, Action::Click(2), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Bike(0))));
        assert_eq!(m.labels()[0], "Bike:  < Road - standard >");
        assert_eq!(
            act(&mut m, Action::Right, &s, &r),
            Effect::Command("riders.edit \"Alex\" --bike road-aero".into())
        );
        let mut tabbed = m.clone();
        act(&mut tabbed, Action::TabNext, &s, &r);
        assert_eq!(tabbed.page(), Some(Page::Sub(sub::Page::Riders)));
        act(&mut m, Action::Back, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Workshop)));
        // "Edit riders" opens the list, a rider opens the editor.
        act(&mut m, Action::Click(3), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Riders)));
        act(&mut m, Action::Click(0), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Rider(0))));
        act(&mut m, Action::Down, &s, &r);
        assert_eq!(m.labels()[1], "Weight:  < 78 kg >");
        assert_eq!(
            act(&mut m, Action::Left, &s, &r),
            Effect::Command("riders.edit \"Alex\" --weight 77".into())
        );
        // Q / E from the editor: the tab after Riders.
        let mut tabbed = m.clone();
        act(&mut tabbed, Action::TabNext, &s, &r);
        assert_eq!(tabbed.page(), Some(Page::Sub(sub::Page::Controls)));
        // The rider was deleted: the editor goes back to the list.
        r.remove("Alex").unwrap();
        r.remove("Anna").unwrap();
        act(&mut m, Action::Hover(0), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Riders)));
        assert_eq!(m.labels(), ["Add a rider", "Back"]);
        // Back at the depth of the Workshop page.
        act(&mut m, Action::Back, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Workshop)));
    }

    #[test]
    fn sub_pages_fit_the_ten_foot_limits() {
        let (s, r) = (Settings::default(), riders());
        for p in sub::Page::TABS
            .into_iter()
            .chain([sub::Page::Rider(0), sub::Page::Bike(0)])
        {
            let rows = sub::rows(p, &s, &r);
            assert!(!rows.is_empty() && rows.len() < 7, "{p:?}");
            for row in &rows {
                assert!(row_label(row).is_ascii(), "{p:?}: {row:?}");
            }
            for t in p.notes() {
                assert!(t.is_ascii(), "{p:?}: {t}");
            }
            assert!(p.title().is_ascii());
        }
    }

    #[test]
    fn every_page_fits_the_ten_foot_limits() {
        for page in [
            Page::Top,
            Page::Ride,
            Page::Navi,
            Page::Settings,
            Page::ConfirmRestart,
            Page::ConfirmQuit,
            Page::ConfirmSwitch,
            Page::ConfirmEvent,
        ] {
            let d = page.def();
            assert!(!d.items.is_empty() && d.items.len() <= 7, "{page:?}");
            assert!(d.initial_focus < d.items.len(), "{page:?}");
            // Every character has a glyph in the bundled UI font.
            for s in d.text.iter().chain(d.items.iter().map(|i| &i.label)) {
                let missing = ocw_render::font::missing(s);
                assert!(missing.is_empty(), "{page:?}: {s}: {missing:?}");
            }
            // Every sub page can be left with an item, not only with Back.
            if page != Page::Top {
                assert!(d.items.iter().any(|i| i.kind == ItemKind::Back), "{page:?}");
            }
        }
    }

    #[test]
    fn many_riders_are_paged_in_the_workshop() {
        let s = Settings::default();
        let mut r = Riders::default();
        for i in 0..8 {
            r.add(Rider {
                name: format!("Rider {i}"),
                ..Rider::default()
            })
            .unwrap();
        }
        let mut m = Menu::default();
        act(&mut m, Action::RiderPage, &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Workshop)));
        // 8 riders, Bike and Edit riders: three pages of four rows.
        let l = m.labels();
        assert!(l.len() <= MAX_ITEMS, "{l:?}");
        assert_eq!(&l[4..], ["Next (1/3)", "Back"]);
        act(&mut m, Action::Click(4), &s, &r);
        act(&mut m, Action::Click(5), &s, &r);
        let l = m.labels();
        assert_eq!(l[l.len() - 2], "Previous (3/3)");
        assert!(l.iter().any(|x| x == "Edit riders"), "{l:?}");
        // A row's command still runs from a later page.
        let at = l.iter().position(|x| x == "Edit riders").unwrap();
        act(&mut m, Action::Click(at), &s, &r);
        assert_eq!(m.page(), Some(Page::Sub(sub::Page::Riders)));
        // Back: the Workshop keeps its page.
        act(&mut m, Action::Back, &s, &r);
        assert_eq!(m.labels()[m.labels().len() - 2], "Previous (3/3)");
    }
}
