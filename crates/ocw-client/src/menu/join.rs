//! Ride → Join (spec §3.2.1 *Join*): the page's rows and its text fields,
//! Bevy-free and shared by both apps.
//!
//! A [`TextField`] is a row: Enter starts typing (the menu's keys go to
//! the field until Enter or Esc ends it), typed characters go in at the
//! cursor, ← / → / Home / End move it, Backspace / Delete remove, Ctrl+V
//! (⌘V, the browser's paste event) and the *Paste link* row paste. On a
//! gamepad the field is a character wheel: ↑ / ↓ change the character at
//! the cursor ([`WHEEL`]), ← / → move, A ends, B deletes; touch uses
//! *Paste link*. The rows show the text with a bar at the cursor.

use super::pages::{Activate, Page, Row};
use crate::join::{JoinFlow, RecentJoins};
use ocw_protocol::join::JoinLink;

/// Longest text a field takes (a link with world and route fits).
pub const MAX_LEN: usize = 400;
/// Characters shown of a long text (the row's width at 16:9).
pub const SHOWN: usize = 34;
/// The gamepad's character wheel: what links are made of.
pub const WHEEL: &str = "abcdefghijklmnopqrstuvwxyz0123456789.:/?=&-_[]@%";

/// Which field is being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Link,
    Fingerprint,
}

/// One line of text with a cursor (in characters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextField {
    pub text: String,
    pub cursor: usize,
}

impl TextField {
    pub fn new(text: &str) -> Self {
        let mut f = Self::default();
        f.insert(text);
        f
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(b, _)| b)
    }

    /// Inserts typed or pasted text at the cursor: line breaks and tabs
    /// become nothing (a pasted link is one line), other control
    /// characters are dropped; at most [`MAX_LEN`] characters.
    pub fn insert(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            if self.len() >= MAX_LEN {
                break;
            }
            let at = self.byte(self.cursor);
            self.text.insert(at, c);
            self.cursor += 1;
        }
    }

    /// Pastes: the clipboard's text, trimmed (spaces around a copied
    /// link).
    pub fn paste(&mut self, s: &str) {
        self.insert(s.trim());
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The gamepad's wheel: the character before the cursor steps through
    /// [`WHEEL`] (`dir` ±1); at the start or after a step past the end a
    /// new character is added.
    pub fn wheel(&mut self, dir: i32) {
        let wheel: Vec<char> = WHEEL.chars().collect();
        let n = wheel.len() as i32;
        if self.cursor == 0 {
            self.insert(&wheel[if dir > 0 { 0 } else { (n - 1) as usize }].to_string());
            return;
        }
        let at = self.byte(self.cursor - 1);
        let c = self.text[at..].chars().next().expect("a character");
        let i = wheel
            .iter()
            .position(|w| *w == c.to_ascii_lowercase())
            .map_or(-1, |i| i as i32);
        let next = wheel[(i + dir).rem_euclid(n) as usize];
        self.text
            .replace_range(at..at + c.len_utf8(), &next.to_string());
    }

    /// The text as the row shows it: with `|` at the cursor while typing,
    /// cut to [`SHOWN`] characters around the cursor ("…" where cut).
    pub fn shown(&self, typing: bool, empty: &str) -> String {
        if self.text.is_empty() && !typing {
            return empty.to_owned();
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut shown: Vec<char> = chars.clone();
        let mut cur = self.cursor;
        if typing {
            shown.insert(cur, '|');
            cur += 1;
        }
        if shown.len() <= SHOWN {
            return shown.into_iter().collect();
        }
        // A window ending a little after the cursor.
        let end = (cur + 4).clamp(SHOWN - 1, shown.len());
        let start = end.saturating_sub(SHOWN - 1);
        let mut out = String::new();
        if start > 0 {
            out.push('…');
        }
        out.extend(&shown[start..end]);
        if end < shown.len() {
            out.push('…');
        }
        out
    }
}

/// What the Join page shows.
#[derive(Debug, Clone, Default)]
pub struct JoinPageInfo {
    pub link: TextField,
    pub fingerprint: TextField,
    /// The field being typed into.
    pub typing: Option<Field>,
    pub flow: JoinFlow,
    pub recent: RecentJoins,
    /// This app joins only WebTransport servers (the browser).
    pub web_only: bool,
}

impl JoinPageInfo {
    /// The link needs the fingerprint field: it parses, has none and does
    /// not name this machine.
    pub fn needs_fingerprint(&self) -> bool {
        JoinLink::parse(&self.link.text).is_ok_and(|l| l.cert.is_none() && !l.is_loopback())
    }
}

/// The rows of Ride → Join.
pub fn rows(page: Page, info: &JoinPageInfo) -> Vec<Row> {
    if page != Page::Join {
        return Vec::new();
    }
    let field = |label: &str, f: &TextField, which: Field, empty: &str, edit: &str| Row {
        label: label.into(),
        value: Some(f.shown(info.typing == Some(which), empty)),
        left: None,
        right: None,
        confirm: Some(Activate::Command(format!("join.edit {edit}"))),
    };
    let mut out = vec![field(
        "Link",
        &info.link,
        Field::Link,
        "(Enter to type, or paste)",
        "link",
    )];
    if info.needs_fingerprint() || !info.fingerprint.text.is_empty() {
        out.push(field(
            "Fingerprint",
            &info.fingerprint,
            Field::Fingerprint,
            "(64 hex digits)",
            "fingerprint",
        ));
    }
    out.push(Row::action("Paste link", "join.paste"));
    if info.flow.busy() {
        out.push(Row::action("Cancel joining", "join.cancel"));
    } else {
        out.push(Row {
            label: "Join".into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(if info.link.text.trim().is_empty() {
                Activate::Disabled
            } else {
                Activate::Command("join.go".into())
            }),
        });
    }
    for (i, e) in info.recent.entries.iter().enumerate() {
        out.push(Row::action(&e.label(), format!("join {}", i + 1)));
    }
    out
}

/// The text over the rows: the status of the join, or why a link needs a
/// fingerprint.
pub fn text(info: &JoinPageInfo, now_s: f64) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = info.flow.status_line(now_s) {
        out.push(s);
    } else if info.needs_fingerprint() {
        out.push(
            "This server is not on this machine: enter its certificate fingerprint (from its host) so nobody can pretend to be it.".into(),
        );
    }
    if info.web_only {
        out.push("The browser joins servers with WebTransport (https:// links).".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join::RecentJoins;

    const FP: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    #[test]
    fn typing_moving_and_deleting() {
        let mut f = TextField::default();
        f.insert("ocw://hst");
        f.left();
        f.left();
        f.insert("o");
        assert_eq!(f.text, "ocw://host");
        f.end();
        f.backspace();
        f.home();
        f.delete();
        assert_eq!(f.text, "cw://hos");
        assert_eq!(f.cursor, 0);
        f.backspace();
        f.left();
        assert_eq!(f.cursor, 0);
        f.end();
        f.right();
        assert_eq!(f.cursor, 8);
        // Unicode keeps whole characters.
        let mut f = TextField::new("Zürich");
        f.backspace();
        f.left();
        f.left();
        f.left();
        f.backspace();
        assert_eq!(f.text, "Zric");
    }

    #[test]
    fn paste_trims_and_drops_line_breaks() {
        let mut f = TextField::new("x");
        f.home();
        f.paste(&format!("  ocw://10.0.0.1?cert={FP}\n"));
        assert_eq!(f.text, format!("ocw://10.0.0.1?cert={FP}x"));
        let mut f = TextField::default();
        f.paste(&"a".repeat(MAX_LEN + 50));
        assert_eq!(f.text.len(), MAX_LEN);
        let mut f = TextField::default();
        f.insert("a\tb\r\nc");
        assert_eq!(f.text, "abc");
    }

    #[test]
    fn the_gamepad_wheel_steps_characters() {
        let mut f = TextField::default();
        f.wheel(1);
        assert_eq!(f.text, "a");
        f.wheel(1);
        f.wheel(1);
        assert_eq!(f.text, "c");
        f.wheel(-1);
        f.wheel(-1);
        f.wheel(-1);
        assert_eq!(f.text, "%");
        f.end();
        f.insert(":");
        f.wheel(1);
        assert_eq!(f.text, "%/");
    }

    #[test]
    fn long_text_is_shown_around_the_cursor() {
        let f = TextField::new(&format!("ocw://10.0.0.1?cert={FP}"));
        let s = f.shown(true, "");
        assert!(s.starts_with('…') && s.ends_with('|'), "{s}");
        assert!(s.chars().count() <= SHOWN + 1);
        let mut g = f.clone();
        g.home();
        let s = g.shown(true, "");
        assert!(s.starts_with("|ocw://") && s.ends_with('…'), "{s}");
        assert_eq!(TextField::default().shown(false, "(empty)"), "(empty)");
        assert_eq!(TextField::new("ab").shown(false, "-"), "ab");
    }

    fn info(link: &str) -> JoinPageInfo {
        JoinPageInfo {
            link: TextField::new(link),
            ..Default::default()
        }
    }

    #[test]
    fn the_page_rows_follow_the_link_and_the_flow() {
        // Empty: Join is dimmed.
        let r = rows(Page::Join, &info(""));
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Link", "Paste link", "Join"]);
        assert_eq!(r[2].confirm, Some(Activate::Disabled));
        assert_eq!(
            r[0].confirm,
            Some(Activate::Command("join.edit link".into()))
        );
        // A remote host without a fingerprint: the field, and the text says why.
        let i = info("ride.example.org");
        let r = rows(Page::Join, &i);
        assert_eq!(r[1].label, "Fingerprint");
        assert!(text(&i, 0.0)[0].contains("fingerprint"));
        assert_eq!(r[3].confirm, Some(Activate::Command("join.go".into())));
        // This machine: no fingerprint needed.
        assert_eq!(rows(Page::Join, &info("localhost")).len(), 3);
        assert_eq!(
            rows(Page::Join, &info(&format!("ocw://h?cert={FP}"))).len(),
            3
        );
        // Busy: Cancel instead of Join; recent servers after it.
        let mut i = info("localhost");
        i.flow.start("localhost", "", 0.0).unwrap();
        let mut recent = RecentJoins::default();
        recent.push(&JoinLink::new("localhost", 47800), "route hills");
        i.recent = recent;
        let r = rows(Page::Join, &i);
        assert_eq!(r[2].label, "Cancel joining");
        assert_eq!(r[3].label, "localhost:47800 – route hills");
        assert_eq!(r[3].confirm, Some(Activate::Command("join 1".into())));
        assert!(text(&i, 2.0)[0].starts_with("Connecting to localhost:47800"));
        // While typing the row shows the cursor.
        let mut i = info("ab");
        i.typing = Some(Field::Link);
        assert_eq!(rows(Page::Join, &i)[0].value.as_deref(), Some("ab|"));
        assert!(rows(Page::Routes, &i).is_empty());
    }
}
