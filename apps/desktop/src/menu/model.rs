//! The in-game menu as a Bevy-free model (spec §3.2.1): pages, the page
//! stack, the focused item. Input arrives as abstract [`Action`]s; what the
//! app has to do comes back as an [`Effect`]. Menu functions are console
//! command lines, so menu, console and scripts share one code path.
//!
//! The Settings sub pages and the Workshop (M2) are [`Page::Sub`]: their
//! rows come from `settings::pages` (current values, left / right change
//! them), passed in with [`Menu::apply_with`].

use crate::settings::pages::{self as sub, Activate, Row};

/// Abstract menu input (keyboard, mouse and gamepad map onto these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the menu at its top page (Esc / Start while it is closed).
    Open,
    /// Close the menu from any page (Start while it is open).
    Close,
    Up,
    Down,
    /// Change the focused row's value (value rows of [`Page::Sub`]).
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
    /// A Settings sub page or the Workshop, with rows from `settings::pages`.
    Sub(sub::Page),
    ConfirmRestart,
    ConfirmQuit,
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

const TOP_ITEMS: &[Item] = &[
    item("Resume", ItemKind::Resume),
    item("Ride", ItemKind::Open(Page::Ride)),
    item("Workshop", ItemKind::Open(Page::Sub(sub::Page::Workshop))),
    item("Navi", ItemKind::Open(Page::Navi)),
    item("Settings", ItemKind::Open(Page::Settings)),
    item("Restart", ItemKind::Open(Page::ConfirmRestart)),
    item("Quit", ItemKind::Open(Page::ConfirmQuit)),
];
const SETTINGS_ITEMS: &[Item] = &[
    item("Riders", ItemKind::Open(Page::Sub(sub::Page::Riders))),
    item("Controls", ItemKind::Open(Page::Sub(sub::Page::Controls))),
    item("Trainer", ItemKind::Open(Page::Sub(sub::Page::Trainer))),
    item("Display", ItemKind::Open(Page::Sub(sub::Page::Display))),
    item("Strava", ItemKind::Open(Page::Sub(sub::Page::Strava))),
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
                    "Coming soon (M3): pick the world and the mode - free ride from a random or named place, a route of the world package or a test route - and the number of bots, then start the ride.",
                    "Later: structured workouts in ERG mode, and events (group rides, races) once accounts exist (Phase 5).",
                ],
                items: &[BACK],
                initial_focus: 0,
            },
            Page::Navi => PageDef {
                title: "Navi",
                text: &[
                    "Coming soon (M3): the route library, your saved rides with FIT export and their Strava sync state, Sync now / Retry failed.",
                    "Later (M4): GPX import with map matching, GPX export.",
                ],
                items: &[BACK],
                initial_focus: 0,
            },
            Page::Settings => PageDef {
                title: "Settings",
                text: &[
                    "Saved in settings.toml and riders.toml (console: set, get, riders.*). Audio comes with game audio (M5).",
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
        }
    }
}

/// The menu: closed, or a stack of (page, focused item).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menu {
    stack: Vec<(Page, usize)>,
    /// The rows of the current [`Page::Sub`] (refreshed by `apply_with`).
    rows: Vec<Row>,
}

impl Menu {
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// The current page, if open.
    pub fn page(&self) -> Option<Page> {
        self.stack.last().map(|(p, _)| *p)
    }

    /// The focused item index on the current page.
    pub fn focus(&self) -> usize {
        self.stack.last().map_or(0, |(_, f)| *f)
    }

    /// Pages from the top page to the current one.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn items(&self) -> &'static [Item] {
        self.page().map_or(&[], |p| p.def().items)
    }

    /// Number of focusable items on the current page.
    pub fn item_count(&self) -> usize {
        match self.page() {
            Some(Page::Sub(_)) => self.rows.len() + 1,
            _ => self.items().len(),
        }
    }

    /// The item labels of the current page, as the menu shows them: value
    /// rows as "Label:  < value >".
    pub fn labels(&self) -> Vec<String> {
        match self.page() {
            Some(Page::Sub(_)) => self
                .rows
                .iter()
                .map(row_label)
                .chain(["Back".to_string()])
                .collect(),
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
        self.stack.push((page, page.def().initial_focus));
    }

    fn close(&mut self) -> Effect {
        if self.stack.is_empty() {
            return Effect::None;
        }
        self.stack.clear();
        self.rows.clear();
        Effect::Closed
    }

    fn set_focus(&mut self, i: usize) {
        if let Some((_, f)) = self.stack.last_mut() {
            *f = i;
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
        }
    }

    fn select_row(&mut self, i: usize) -> Effect {
        if i >= self.item_count() {
            return Effect::None;
        }
        self.set_focus(i);
        match self.rows.get(i).map(|r| r.confirm.clone()) {
            None => self.back(),
            Some(Some(Activate::Command(line))) => Effect::Command(line),
            Some(Some(Activate::Open(p))) => {
                self.push(Page::Sub(p));
                self.rows.clear();
                Effect::None
            }
            Some(None) => Effect::None,
        }
    }

    /// Left / right on a value row: its command line.
    fn change(&self, right: bool) -> Effect {
        let Some(row) = self.rows.get(self.focus()) else {
            return Effect::None;
        };
        match if right { &row.right } else { &row.left } {
            Some(line) => Effect::Command(line.clone()),
            None => Effect::None,
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
    /// gone (deleted) goes back to the list.
    pub fn refresh(&mut self, rows: &impl Fn(sub::Page) -> Vec<Row>) {
        match self.page() {
            Some(Page::Sub(p)) => {
                self.rows = rows(p);
                if matches!(p, sub::Page::Rider(_)) && self.rows.is_empty() {
                    self.stack.pop();
                    self.refresh(rows);
                    return;
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
            Action::Open => Effect::None,
            Action::Left | Action::Right => self.change(action == Action::Right),
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
                "Resume", "Ride", "Workshop", "Navi", "Settings", "Restart", "Quit"
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
        m.apply(Action::Hover(4));
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(Page::Settings));
        assert_eq!(m.depth(), 2);
        // Back returns to the top page with the focus where it was.
        assert_eq!(m.apply(Action::Back), Effect::None);
        assert_eq!((m.page(), m.focus()), (Some(Page::Top), 4));
        // Placeholder pages: their Back item goes up too.
        m.apply(Action::Click(1));
        assert_eq!(m.page(), Some(Page::Ride));
        assert_eq!(labels(&m), ["Back"]);
        m.apply(Action::Confirm);
        assert_eq!((m.page(), m.focus()), (Some(Page::Top), 1));
        // Back at the top closes.
        assert_eq!(m.apply(Action::Back), Effect::Closed);
        assert!(!m.is_open());
        // Close from a sub page closes everything; reopening starts at the top.
        m.apply(Action::Open);
        m.apply(Action::Click(3));
        assert_eq!(m.page(), Some(Page::Navi));
        assert_eq!(m.apply(Action::Close), Effect::Closed);
        m.apply(Action::Open);
        assert_eq!((m.page(), m.focus(), m.depth()), (Some(Page::Top), 0, 1));
    }

    #[test]
    fn restart_and_quit_ask_first_with_cancel_focused() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Click(5));
        assert_eq!(m.page(), Some(Page::ConfirmRestart));
        assert_eq!(labels(&m)[m.focus()], "Cancel");
        // Confirming Cancel goes back, nothing is run.
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(Page::Top));
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
    fn settings_pages_change_values_with_left_and_right() {
        let (s, r) = (Settings::default(), riders());
        let mut m = Menu::default();
        act(&mut m, Action::Open, &s, &r);
        act(&mut m, Action::Click(4), &s, &r);
        assert_eq!(m.page(), Some(Page::Settings));
        // Display: value rows.
        act(&mut m, Action::Click(3), &s, &r);
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

    #[test]
    fn tabs_switch_between_the_settings_pages_and_the_workshop() {
        let (s, r) = (Settings::default(), riders());
        let mut m = Menu::default();
        act(&mut m, Action::Open, &s, &r);
        act(&mut m, Action::Click(2), &s, &r);
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
        act(&mut m, Action::Open, &s, &r);
        act(&mut m, Action::Click(2), &s, &r);
        assert_eq!(m.labels()[0], "Alex:  78 kg - riding");
        assert_eq!(
            act(&mut m, Action::Click(1), &s, &r),
            Effect::Command("riders.select \"Anna\"".into())
        );
        // "Edit riders" opens the list, a rider opens the editor.
        act(&mut m, Action::Click(2), &s, &r);
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
        for p in sub::Page::TABS.into_iter().chain([sub::Page::Rider(0)]) {
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
        ] {
            let d = page.def();
            assert!(!d.items.is_empty() && d.items.len() <= 7, "{page:?}");
            assert!(d.initial_focus < d.items.len(), "{page:?}");
            // Bevy's default font: ASCII only.
            for s in d.text.iter().chain(d.items.iter().map(|i| &i.label)) {
                assert!(s.is_ascii(), "{page:?}: {s}");
            }
            // Every sub page can be left with an item, not only with Back.
            if page != Page::Top {
                assert!(d.items.iter().any(|i| i.kind == ItemKind::Back), "{page:?}");
            }
        }
    }
}
