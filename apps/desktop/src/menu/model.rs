//! The in-game menu as a Bevy-free model (spec §3.2.1): pages, the page
//! stack, the focused item. Input arrives as abstract [`Action`]s; what the
//! app has to do comes back as an [`Effect`]. Menu functions are console
//! command lines, so menu, console and scripts share one code path.

/// Abstract menu input (keyboard, mouse and gamepad map onto these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the menu at its top page (Esc / Start while it is closed).
    Open,
    /// Close the menu from any page (Start while it is open).
    Close,
    Up,
    Down,
    /// Change a value (no value rows in M1: ignored).
    Left,
    Right,
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
    Workshop,
    Navi,
    Settings,
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
    item("Workshop", ItemKind::Open(Page::Workshop)),
    item("Navi", ItemKind::Open(Page::Navi)),
    item("Settings", ItemKind::Open(Page::Settings)),
    item("Restart", ItemKind::Open(Page::ConfirmRestart)),
    item("Quit", ItemKind::Open(Page::ConfirmQuit)),
];
const SETTINGS_ITEMS: &[Item] = &[
    item(
        "FPS counter on / off",
        ItemKind::Command {
            line: "fps",
            close: false,
        },
    ),
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
            Page::Workshop => PageDef {
                title: "Workshop",
                text: &[
                    "Coming with Phase 4 (M6): the bike (frame, wheels, paint) and the character (clothing, helmet), with a 3D preview. Cosmetic choices never change the physics.",
                    "Before that (M2): choose the active rider profile here.",
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
                    "Coming soon (M2): Controls, Trainer (devices, SIM / ERG, difficulty), Strava, Display and Riders (name, weight, FTP), saved in settings.toml.",
                    "Audio comes with game audio (M5).",
                ],
                items: SETTINGS_ITEMS,
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

    fn push(&mut self, page: Page) {
        self.stack.push((page, page.def().initial_focus));
    }

    fn close(&mut self) -> Effect {
        if self.stack.is_empty() {
            return Effect::None;
        }
        self.stack.clear();
        Effect::Closed
    }

    fn set_focus(&mut self, i: usize) {
        if let Some((_, f)) = self.stack.last_mut() {
            *f = i;
        }
    }

    /// Moves the focus by `step`, wrapping around at both ends.
    fn move_focus(&mut self, step: isize) {
        let n = self.items().len();
        if n == 0 {
            return;
        }
        let f = (self.focus() as isize + step).rem_euclid(n as isize);
        self.set_focus(f as usize);
    }

    fn select(&mut self, i: usize) -> Effect {
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

    fn back(&mut self) -> Effect {
        if self.stack.len() > 1 {
            self.stack.pop();
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
            Action::Open | Action::Left | Action::Right => Effect::None,
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
                if i < self.items().len() {
                    self.set_focus(i);
                }
                Effect::None
            }
            Action::Click(i) => self.select(i),
        }
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

    #[test]
    fn settings_runs_a_command_and_stays_open() {
        let mut m = Menu::default();
        m.apply(Action::Open);
        m.apply(Action::Click(4));
        assert_eq!(m.apply(Action::Confirm), Effect::Command("fps".into()));
        assert_eq!(m.page(), Some(Page::Settings));
    }

    #[test]
    fn every_page_fits_the_ten_foot_limits() {
        for page in [
            Page::Top,
            Page::Ride,
            Page::Workshop,
            Page::Navi,
            Page::Settings,
            Page::ConfirmRestart,
            Page::ConfirmQuit,
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
}
