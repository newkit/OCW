//! Rider names (spec §14 Phase 4, 4N N1; decisions Q8, Q9, Q11, Q15,
//! Q22): one pure check used by the editor, the `riders.toml` loader and
//! the game server, and the uniqueness rule behind one interface for a
//! server's session and (later) the accounts backend.
//!
//! - **Rules** ([`check`]): 2–20 characters as seen (grapheme clusters),
//!   at most 64 bytes of UTF-8, Unicode NFC; letters and marks of **every
//!   script** (Q11), digits, the space and `- ' . _`; spaces trimmed and
//!   collapsed; no control, format or invisible characters (a zero-width
//!   joiner / non-joiner only inside a grapheme, as Indic and Persian
//!   names need it), so no bidi overrides; at least one letter. Reserved
//!   names are refused ([`is_reserved`]: the echo bot, host, server, OCW,
//!   the fallback `Rider NNNN` and the server bots' `Bot N`).
//! - **No filter** (Q9): no deny-list, no word is refused for its meaning.
//! - **Unique** (Q8, Q22): no "(2)" suffixes. A name is held by one
//!   holder in a [`NameRegistry`] – on a game server by a connection for
//!   the session, in the backend by an account worldwide. Names compare by
//!   [`key`] (NFC, case folded), so "Ada Rider" and "ada rider" are the
//!   same name.
//! - **Renaming** (Q15): only between rides – [`rename_allowed`].

use std::collections::BTreeMap;

use icu_normalizer::ComposingNormalizer;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};
use unicode_segmentation::UnicodeSegmentation;

/// Fewest characters (grapheme clusters) in a name.
pub const MIN_CHARS: usize = 2;
/// Most characters (grapheme clusters) in a name.
pub const MAX_CHARS: usize = 20;
/// Most bytes of UTF-8 in a name (the wire's limit, N12).
pub const MAX_BYTES: usize = 64;
/// The punctuation a name may hold besides letters, digits and spaces.
pub const PUNCTUATION: [char; 4] = ['-', '\'', '.', '_'];

/// Why a name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    TooShort,
    TooLong,
    TooManyBytes,
    /// A character that is not a letter, mark, digit, space or one of
    /// [`PUNCTUATION`] (an emoji, a symbol, a control character).
    Character(char),
    /// An invisible or format character (bidi override, zero width
    /// outside a grapheme).
    Invisible(char),
    /// Only digits and punctuation.
    NoLetter,
    /// One of the reserved names ([`is_reserved`]).
    Reserved,
    /// Another holder has it (Q8 / Q22).
    Taken,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "a name needs at least {MIN_CHARS} characters"),
            Self::TooLong => write!(f, "a name has at most {MAX_CHARS} characters"),
            Self::TooManyBytes => write!(f, "a name has at most {MAX_BYTES} bytes"),
            Self::Character(c) => write!(
                f,
                "a name may hold letters, digits, spaces and - ' . _ (not {c:?})"
            ),
            Self::Invisible(c) => write!(
                f,
                "a name may not hold invisible characters (U+{:04X})",
                *c as u32
            ),
            Self::NoLetter => write!(f, "a name needs a letter"),
            Self::Reserved => write!(f, "this name is reserved"),
            Self::Taken => write!(f, "this name is taken"),
        }
    }
}

impl std::error::Error for NameError {}

/// Unicode NFC.
pub fn nfc(s: &str) -> String {
    ComposingNormalizer::new_nfc().normalize(s).into_owned()
}

/// Spaces trimmed and runs of whitespace collapsed to one space.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The name as it is checked and shown: NFC, spaces trimmed and
/// collapsed (no other change).
pub fn normalize(raw: &str) -> String {
    collapse(&nfc(raw))
}

/// The name's identity for uniqueness: [`normalize`]d and case folded.
pub fn key(name: &str) -> String {
    normalize(name).to_lowercase()
}

/// Number of characters as seen (extended grapheme clusters).
pub fn chars(name: &str) -> usize {
    name.graphemes(true).count()
}

/// The reserved names (refused for riders): "Echo" (the echo bot),
/// "Host", "Server", "OCW", "Rider" followed only by digits (the
/// server's fallback [`fallback`], "Rider" alone too) and the server
/// bots' "Bot N". Compared by [`key`].
pub fn is_reserved(name: &str) -> bool {
    let k = key(name);
    if ["echo", "host", "server", "ocw"].contains(&k.as_str()) {
        return true;
    }
    let numbered = |prefix: &str, bare: bool| {
        k.strip_prefix(prefix).is_some_and(|rest| {
            let rest = rest.trim_start();
            (bare && rest.is_empty())
                || (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
    };
    numbered("rider", true) || numbered("bot", false)
}

/// The name a server gives a rider whose name it refuses: "Rider 0042"
/// from the rider id (unique in the session, reserved for riders).
pub fn fallback(rider_id: u32) -> String {
    format!("Rider {rider_id:04}")
}

/// Whether `c` is invisible: control and format characters, the line and
/// paragraph separators, the unassigned and private-use code points.
fn invisible(gc: GeneralCategory) -> bool {
    matches!(
        gc,
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
    )
}

/// The rules of N1 (see the module): `Ok` with the normalized name.
pub fn check(raw: &str) -> Result<String, NameError> {
    let name = normalize(raw);
    let gc = CodePointMapData::<GeneralCategory>::new();
    let mut letter = false;
    for g in name.graphemes(true) {
        for (i, c) in g.chars().enumerate() {
            let cat = gc.get(c);
            // A joiner inside a grapheme (Indic, Persian) is part of it;
            // anywhere else it is an invisible character.
            if matches!(c, '\u{200C}' | '\u{200D}') && i > 0 {
                continue;
            }
            if invisible(cat) {
                return Err(NameError::Invisible(c));
            }
            if GeneralCategoryGroup::Letter.contains(cat) {
                letter = true;
            } else if GeneralCategoryGroup::Mark.contains(cat) && i > 0 {
                // A combining mark on a base character.
            } else if !(GeneralCategoryGroup::DecimalNumber.contains(cat)
                || c == ' '
                || PUNCTUATION.contains(&c))
            {
                return Err(NameError::Character(c));
            }
        }
    }
    let n = chars(&name);
    if n < MIN_CHARS {
        return Err(NameError::TooShort);
    }
    if n > MAX_CHARS {
        return Err(NameError::TooLong);
    }
    if name.len() > MAX_BYTES {
        return Err(NameError::TooManyBytes);
    }
    if !letter {
        return Err(NameError::NoLetter);
    }
    if is_reserved(&name) {
        return Err(NameError::Reserved);
    }
    Ok(name)
}

/// Who holds the names of a [`NameRegistry`] (Q22): a game server's
/// connection for its session, an account in the backend.
pub trait NameRegistry {
    type Holder: Clone + PartialEq;

    /// Who holds the name with this [`key`].
    fn holder_of_key(&self, key: &str) -> Option<Self::Holder>;
    /// Stores `holder` as the holder of `name` (checked, free or theirs)
    /// and releases what they held before.
    fn store(&mut self, holder: Self::Holder, key: String, name: String);
    /// Releases the name `holder` holds.
    fn release(&mut self, holder: &Self::Holder);

    /// Who holds `name`.
    fn holder(&self, name: &str) -> Option<Self::Holder> {
        self.holder_of_key(&key(name))
    }

    /// Claims `name` for `holder` (Q8, Q22): checked by [`check`], refused
    /// with [`NameError::Taken`] when someone else holds it; `Ok` with the
    /// normalized name (the holder's earlier name is released).
    fn claim(&mut self, holder: Self::Holder, name: &str) -> Result<String, NameError> {
        let name = check(name)?;
        let k = key(&name);
        match self.holder_of_key(&k) {
            Some(h) if h != holder => Err(NameError::Taken),
            _ => {
                self.store(holder, k, name.clone());
                Ok(name)
            }
        }
    }
}

/// An in-memory [`NameRegistry`]: a game server's session names (holder:
/// the connection), and the account names until the backend keeps them.
///
/// TODO(spec §14 Phase 4 4O / W5b): the backend's account names live in
/// PostgreSQL (the rider table, the name unique by its [`key`]) behind
/// this same trait; `NameBook<AccountId>` is its test double until then.
#[derive(Debug, Clone, Default)]
pub struct NameBook<H> {
    by_key: BTreeMap<String, (H, String)>,
}

impl<H: Clone + PartialEq> NameBook<H> {
    pub fn new() -> Self {
        Self {
            by_key: BTreeMap::new(),
        }
    }

    /// The name `holder` holds.
    pub fn name_of(&self, holder: &H) -> Option<&str> {
        self.by_key
            .values()
            .find(|(h, _)| h == holder)
            .map(|(_, n)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

impl<H: Clone + PartialEq> NameRegistry for NameBook<H> {
    type Holder = H;

    fn holder_of_key(&self, key: &str) -> Option<H> {
        self.by_key.get(key).map(|(h, _)| h.clone())
    }

    fn store(&mut self, holder: H, key: String, name: String) {
        self.release(&holder);
        self.by_key.insert(key, (holder, name));
    }

    fn release(&mut self, holder: &H) {
        self.by_key.retain(|_, (h, _)| h != holder);
    }
}

/// An account's id in the backend (the token's `sub`, spec §9.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(pub String);

/// The account names (Q22: reserved for one account worldwide), in
/// memory until the backend's store exists (TODO W5b, see [`NameBook`]).
pub type AccountNames = NameBook<AccountId>;

/// The dice's first words (N1: made-up names, never a real person's).
pub const DICE_ADJECTIVES: [&str; 16] = [
    "Swift", "Calm", "Brave", "Bright", "Quiet", "Steady", "Lucky", "Merry", "Nimble", "Bold",
    "Gentle", "Keen", "Sunny", "Wild", "Clever", "Happy",
];
/// The dice's riding words.
pub const DICE_RIDERS: [&str; 16] = [
    "Climber",
    "Descender",
    "Sprinter",
    "Rouleur",
    "Pedaller",
    "Cruiser",
    "Spinner",
    "Breakaway",
    "Wheelie",
    "Tourer",
    "Domestique",
    "Puncheur",
    "Grimpeur",
    "Freewheeler",
    "Cadence",
    "Tailwind",
];

/// A made-up name from the dice (N1): an adjective and a riding word,
/// "Swift Climber"; the same for the same `seed`. Always passes [`check`].
pub fn dice(seed: u64) -> String {
    // SplitMix64: a well spread value from any seed.
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let a = DICE_ADJECTIVES[(z % 16) as usize];
    let r = DICE_RIDERS[((z >> 8) % 16) as usize];
    format!("{a} {r}")
}

/// Whether the rider may rename now (Q15: "the name can only be changed
/// between rides, never during a ride or a running event").
pub fn rename_allowed(in_ride: bool, in_running_event: bool) -> Result<(), &'static str> {
    if in_running_event {
        Err("the name cannot change while you ride in a running event")
    } else if in_ride {
        Err("the name cannot change during a ride: rename between rides")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_every_script_pass() {
        for n in [
            "Ada Rider",
            "Bea Climber",
            "Zoë Fahrer",
            "Jürgen-Ölm",
            "O'Neill",
            "Dara_Descender",
            "J. Doe",
            "Ηλέκτρα",
            "Ёлка Велосипед",
            "יעל רוכבת",
            "ريم الدراجة",
            "मीरा साइकिल",
            "สมชาย",
            "李小龍",
            "さくら",
            "김민지",
            "Ọlá",
            "ده‌کده", // Persian with a zero-width non-joiner in a word
            "Rider Ada",
            "Ann2",
        ] {
            assert_eq!(check(n).as_deref(), Ok(n), "{n}");
        }
    }

    #[test]
    fn names_are_normalized() {
        // NFD "e" + combining acute becomes NFC "é".
        assert_eq!(check("Ame\u{301}lie").unwrap(), "Amélie");
        assert_eq!(check("  Ada   Rider ").unwrap(), "Ada Rider");
        assert_eq!(check("Ada\tRider").unwrap(), "Ada Rider");
    }

    #[test]
    fn lengths_count_graphemes_and_bytes() {
        assert_eq!(check("A"), Err(NameError::TooShort));
        assert_eq!(check(""), Err(NameError::TooShort));
        assert_eq!(check("   "), Err(NameError::TooShort));
        assert!(check("Ab").is_ok());
        let twenty = "Abcdefghij Klmnopqrs";
        assert_eq!(chars(twenty), 20);
        assert!(check(twenty).is_ok());
        assert_eq!(check("Abcdefghij Klmnopqrst"), Err(NameError::TooLong));
        // Hindi: few graphemes, many bytes each.
        let hindi = "क्षत्रियक्षत्रियक्षत्रियक्षत्रि";
        assert!(chars(hindi) <= MAX_CHARS, "{}", chars(hindi));
        assert!(hindi.len() > MAX_BYTES);
        assert_eq!(check(hindi), Err(NameError::TooManyBytes));
        // A combining mark counts with its base.
        assert_eq!(chars("Ame\u{301}lie"), 6);
    }

    #[test]
    fn invisible_control_and_symbol_characters_are_refused() {
        assert_eq!(
            check("Ada\u{202E}Rider"),
            Err(NameError::Invisible('\u{202E}'))
        );
        assert_eq!(
            check("Ada\u{200B}Rider"),
            Err(NameError::Invisible('\u{200B}'))
        );
        assert_eq!(check("\u{200D}Ada"), Err(NameError::Invisible('\u{200D}')));
        assert_eq!(
            check("Ada\u{0007}Rider"),
            Err(NameError::Invisible('\u{7}'))
        );
        assert_eq!(
            check("Ada \u{1F6B4}"),
            Err(NameError::Character('\u{1F6B4}'))
        );
        assert_eq!(check("Ada <3"), Err(NameError::Character('<')));
        assert_eq!(check("Ada\"Rider"), Err(NameError::Character('"')));
        assert_eq!(check("\u{301}Ada"), Err(NameError::Character('\u{301}')));
        assert_eq!(check("1234"), Err(NameError::NoLetter));
        assert_eq!(check("--.."), Err(NameError::NoLetter));
    }

    #[test]
    fn reserved_names_are_refused_but_no_word_filter() {
        for n in [
            "Echo",
            "ECHO",
            "Host",
            "server",
            "OCW",
            "Rider",
            "Rider 7",
            "rider 0042",
            "Rider0042",
            "Bot 3",
            "bot12",
        ] {
            assert!(is_reserved(n), "{n}");
            assert_eq!(check(n), Err(NameError::Reserved), "{n}");
        }
        for n in ["Riders", "Rider Ada", "Bot", "Botanist", "Echo Valley"] {
            assert!(!is_reserved(n), "{n}");
            assert!(check(n).is_ok(), "{n}");
        }
        // Q9: no deny-list – a rude word is a name like any other.
        assert!(check("Stinky Pants").is_ok());
        assert_eq!(fallback(42), "Rider 0042");
        assert!(is_reserved(&fallback(42)));
    }

    #[test]
    fn a_session_keeps_names_unique() {
        let mut book: NameBook<u32> = NameBook::new();
        assert_eq!(book.claim(1, "Ada Rider").as_deref(), Ok("Ada Rider"));
        // Q8: no "(2)": the same name in any case is taken.
        assert_eq!(book.claim(2, "ada  rider"), Err(NameError::Taken));
        assert_eq!(book.claim(2, "Ada Rider"), Err(NameError::Taken));
        assert_eq!(book.holder("ADA RIDER"), Some(1));
        // The holder may claim it again, or another name (the old one is
        // freed).
        assert!(book.claim(1, "Ada Rider").is_ok());
        assert!(book.claim(1, "Ada Climber").is_ok());
        assert_eq!(book.len(), 1);
        assert!(book.claim(2, "Ada Rider").is_ok());
        book.release(&1);
        assert_eq!(book.holder("Ada Climber"), None);
        assert_eq!(book.name_of(&2), Some("Ada Rider"));
        // Invalid names never enter the book.
        assert_eq!(book.claim(3, "Echo"), Err(NameError::Reserved));
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn account_names_are_reserved_worldwide() {
        // The backend's check behind the same interface (TODO W5b: the
        // PostgreSQL store), here on its in-memory double.
        let mut accounts = AccountNames::new();
        let a = AccountId("acct-ada".into());
        let b = AccountId("acct-bea".into());
        assert!(accounts.claim(a.clone(), "Ada Rider").is_ok());
        assert_eq!(
            accounts.claim(b.clone(), "Ada Rider"),
            Err(NameError::Taken)
        );
        assert!(accounts.claim(b.clone(), "Bea Climber").is_ok());
        assert_eq!(accounts.holder("ada rider"), Some(a.clone()));
        // Renaming frees the old name for others.
        assert!(accounts.claim(a, "Ada Sprinter").is_ok());
        assert!(accounts.claim(b, "Ada Rider").is_ok());
    }

    #[test]
    fn the_dice_makes_valid_made_up_names() {
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..200 {
            let n = dice(seed);
            assert_eq!(check(&n).as_deref(), Ok(n.as_str()), "{n}");
            assert_eq!(dice(seed), n, "deterministic");
            seen.insert(n);
        }
        assert!(seen.len() > 100, "{} distinct names", seen.len());
    }

    #[test]
    fn renaming_only_between_rides() {
        assert!(rename_allowed(false, false).is_ok());
        assert!(rename_allowed(true, false).is_err());
        assert!(rename_allowed(false, true).is_err());
        assert!(rename_allowed(true, true).unwrap_err().contains("event"));
    }
}
