//! The trust dialog and the trusted world keys (spec §4.7 *User keys*;
//! §14 Phase 16, 16C *Signing and trust*, Q2), Bevy-free and shared by
//! both apps.
//!
//! - The dialog ([`super::model::Page::ConfirmTrust`]) shows the world
//!   (name, id, version), the signer's name **as the key claims it**, the
//!   key's full fingerprint large ([`dialog_emphasis`]), a warning and the
//!   package's data sources with their licences ([`dialog_text`]). Its
//!   items: *Trust this key*, *Load once*, *Cancel* (focused).
//! - Settings → Account → *Trusted world keys* ([`rows`]): the trusted
//!   user keys (name, fingerprint, date added); a key's page removes it
//!   after "Remove this key?".

use ocw_worldstore::userkeys::{TrustRequest, TrustedKey};

use super::pages::{Activate, Page, Row, info_row, open_row, short_label};

/// The dialog's warning (always shown).
pub const WARNING: &str = "This world was not made by Open Cycling World. Only trust keys whose fingerprint you checked with their owner, for example in person or over a channel you trust.";

/// The Trusted world keys page's note.
pub const KEYS_NOTE: &str = "Keys of other riders whose own (unofficial) worlds you trust. Official worlds need none. Remove a key to be asked again.";

/// A trusted key's page note.
pub const KEY_NOTE: &str =
    "The name is the one the key gave itself: only the fingerprint identifies it.";

/// Groups of the fingerprint per line in the dialog (two lines of eight).
pub const GROUPS_PER_LINE: usize = 8;

/// "2026-11-14" (UTC) of a Unix time.
pub fn iso_date(unix_s: i64) -> String {
    let (y, m, d) = ocw_sky::time::utc_date(unix_s * 1000);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The line over the fingerprint: "Signed by 'Ada Rider' (unverified
/// name):" – the name as the key claims it; only the fingerprint under it
/// identifies the key.
pub fn dialog_status(r: &TrustRequest) -> String {
    let c = ocw_worldstore::userkeys::claimed(&short_label(&r.claimed_signer, 20));
    let mut chars = c.chars();
    let first = chars
        .next()
        .map(|f| f.to_uppercase().to_string())
        .unwrap_or_default();
    format!("{first}{}:", chars.as_str())
}

/// The dialog's paragraphs, the most important first (a small window
/// drops them from the end): the warning, the world (in a shared ride:
/// that Cancel leaves it), the data sources with their licences.
pub fn dialog_text(r: &TrustRequest, shared: bool) -> Vec<String> {
    let mut out = vec![
        WARNING.to_owned(),
        format!(
            "World: {} ({} v{}, built {}){}",
            short_label(&r.world, 40),
            short_label(&r.id, 40),
            r.version,
            iso_date(r.created_unix_s),
            if shared {
                ", the host's choice: Cancel leaves the shared ride."
            } else {
                "."
            }
        ),
    ];
    if let Some(d) = data_line(r) {
        out.push(d);
    }
    out
}

/// Longest data line in the dialog (characters); more sources are counted.
pub const DATA_MAX_CHARS: usize = 170;

/// "Data: OpenStreetMap (ODbL-1.0); SRTM elevation data (public-domain);
/// … and 3 more sources": the sources with their licence's short name
/// (up to its first parenthesis), sources under the same licence grouped,
/// OpenStreetMap first, at most [`DATA_MAX_CHARS`] (the full notices are
/// on World -> About -> Data sources once it is loaded).
pub fn data_line(r: &TrustRequest) -> Option<String> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for a in &r.attributions {
        let licence = a
            .licence
            .split(" (")
            .next()
            .unwrap_or(&a.licence)
            .trim()
            .to_owned();
        let source = short_label(&a.source, 40);
        match groups.iter_mut().find(|(l, _)| *l == licence) {
            Some((_, s)) => s.push(source),
            None => groups.push((licence, vec![source])),
        }
    }
    if groups.is_empty() {
        return None;
    }
    let total: usize = groups.iter().map(|(_, s)| s.len()).sum();
    let mut line = String::from("Data:");
    let mut shown = 0;
    for (i, (licence, sources)) in groups.iter().enumerate() {
        let sep = if i == 0 { "" } else { ";" };
        let part = format!("{sep} {} ({licence})", sources.join(", "));
        if i > 0 && line.chars().count() + part.chars().count() > DATA_MAX_CHARS {
            break;
        }
        line.push_str(&part);
        shown += sources.len();
    }
    Some(if shown < total {
        format!("{line}; and {} more sources.", total - shown)
    } else {
        format!("{line}.")
    })
}

/// The full fingerprint, two lines of eight groups (drawn large).
pub fn dialog_emphasis(r: &TrustRequest) -> Vec<String> {
    r.fingerprint().lines(GROUPS_PER_LINE)
}

/// The console's text of a request (`world trust show`).
pub fn request_text(r: &TrustRequest) -> String {
    format!(
        "{}\n{}\nfingerprint: {}\nkey id: {}\n{}",
        dialog_text(r, false).join("\n"),
        r.claimed(),
        r.fingerprint(),
        r.root.key_id,
        "choose: world trust accept | world trust once | world trust decline"
    )
}

/// The `world trust remove` line of a key.
pub fn remove_line(k: &TrustedKey) -> String {
    format!("world trust remove {}", k.fingerprint().hex())
}

/// The rows of the Trusted world keys pages.
pub fn rows(page: Page, keys: &[TrustedKey], warning: Option<&str>) -> Vec<Row> {
    match page {
        Page::TrustedKeys => {
            let mut out: Vec<Row> = Vec::new();
            if let Some(w) = warning {
                out.push(info_row("Store", Some(short_label(w, 34))));
            }
            out.extend(keys.iter().enumerate().map(|(i, k)| {
                open_row(
                    &short_label(&k.name, 22),
                    Some(k.fingerprint().groups()[..2].join(" ") + " ..."),
                    Page::TrustedKey(i),
                )
            }));
            if keys.is_empty() {
                out.push(info_row("No trusted keys", Some("see README".into())));
            }
            out
        }
        Page::TrustedKey(i) => {
            let Some(k) = keys.get(i) else {
                return Vec::new();
            };
            let mut out = vec![
                info_row("Name (claimed)", Some(short_label(&k.name, 26))),
                info_row("Added", Some(iso_date(k.added_unix_s))),
            ];
            if let Some(n) = &k.note {
                out.push(info_row("Note", Some(short_label(n, 30))));
            }
            out.push(Row {
                label: "Remove this key".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::AskForget(remove_line(k))),
            });
            out
        }
        _ => Vec::new(),
    }
}

/// The full fingerprint of the key a page shows (drawn large): the key's
/// own page, or the key a "Remove this key?" asks about (`pending`).
pub fn key_emphasis(page: Page, keys: &[TrustedKey]) -> Vec<String> {
    match page {
        Page::TrustedKey(i) => keys
            .get(i)
            .map(|k| k.fingerprint().lines(GROUPS_PER_LINE))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The key a pending `world trust remove HEX` line names.
pub fn pending_key<'a>(pending: Option<&str>, keys: &'a [TrustedKey]) -> Option<&'a TrustedKey> {
    let fp = pending?.strip_prefix("world trust remove ")?;
    keys.iter().find(|k| k.fingerprint().matches(fp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, MAX_ITEMS, Menu, Page as MenuPage};
    use ocw_world::manifest::{Attribution, RootKeyPair, TrustStore};
    use ocw_worldstore::userkeys::UserKeys;

    fn request() -> TrustRequest {
        TrustRequest {
            world: "Ada's Alps".into(),
            id: "user/ada/alps".into(),
            version: 3,
            created_unix_s: 1_800_000_000,
            claimed_signer: "Ada Rider".into(),
            root: RootKeyPair::from_seed("Ada Rider", [77; 32]).public(),
            attributions: vec![Attribution::openstreetmap(), Attribution::srtm()],
        }
    }

    #[test]
    fn the_dialog_shows_world_claimed_signer_fingerprint_and_sources() {
        let r = request();
        let text = dialog_text(&r, false);
        assert_eq!(text[0], WARNING);
        assert!(text[1].contains("Ada's Alps") && text[1].contains("user/ada/alps v3"));
        assert_eq!(
            dialog_status(&r),
            "Signed by 'Ada Rider' (unverified name):"
        );
        assert!(
            text.last()
                .unwrap()
                .starts_with("Data: OpenStreetMap (ODbL"),
            "{text:?}"
        );
        // Many sources: grouped by licence, cut with a count.
        let mut many = r.clone();
        for i in 0..8 {
            let mut a = Attribution::srtm();
            a.source = format!("Copernicus layer {i} with a long name");
            a.licence = format!("Licence {i} (free, full and open; attribution)");
            many.attributions.push(a);
        }
        let d = data_line(&many).unwrap();
        assert!(d.starts_with("Data: OpenStreetMap (ODbL-1.0); SRTM"), "{d}");
        assert!(
            d.chars().count() <= DATA_MAX_CHARS + 30 && d.contains("more sources"),
            "{d}"
        );
        assert!(!text.join(" ").contains("shared ride"));
        assert!(
            dialog_text(&r, true)
                .iter()
                .any(|t| t.contains("leaves the shared ride"))
        );
        let big = dialog_emphasis(&r);
        assert_eq!(big.len(), 2);
        assert_eq!(big.join(" "), r.fingerprint().to_string());
        assert!(request_text(&r).contains(&r.fingerprint().to_string()));
    }

    #[test]
    fn the_dialog_focuses_cancel_and_offers_trust_and_load_once() {
        let mut m = Menu::default();
        // Opened over a closed menu: the menu opens at the dialog.
        assert_eq!(m.apply(Action::TrustDialog), Effect::Opened);
        assert_eq!(m.page(), Some(MenuPage::ConfirmTrust));
        assert_eq!(m.labels(), ["Trust this key", "Load once", "Cancel"]);
        assert_eq!(m.focus(), 2, "Cancel is focused");
        // Enter right away cancels: back to the top page, nothing run.
        assert_eq!(m.apply(Action::Confirm), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::Top));
        // Trusting takes a deliberate move: up twice, then Enter.
        m.apply(Action::TrustDialog);
        m.apply(Action::Up);
        m.apply(Action::Up);
        assert_eq!(
            m.apply(Action::Confirm),
            Effect::Command("world trust accept".into())
        );
        // The command closes the menu (`ocw_ui::trust`); until it ran the
        // dialog stays.
        assert_eq!(m.page(), Some(MenuPage::ConfirmTrust));
        // Load once; the mouse and touch click the items directly.
        m.apply(Action::Close);
        m.apply(Action::TrustDialog);
        assert_eq!(
            m.apply(Action::Click(1)),
            Effect::Command("world trust once".into())
        );
        // Esc / B cancel; opening it twice does not stack it.
        m.apply(Action::Open);
        m.apply(Action::TrustDialog);
        m.apply(Action::TrustDialog);
        assert_eq!(m.depth(), 2);
        m.apply(Action::Back);
        assert_eq!(m.page(), Some(MenuPage::Top));
    }

    #[test]
    fn trusted_keys_are_listed_and_removed_after_asking() {
        let mut keys = UserKeys::default();
        for (name, seed) in [("Ada Rider", 81u8), ("Bo Builder", 82)] {
            keys.add(
                &RootKeyPair::from_seed(name, [seed; 32]).public(),
                None,
                Some("club"),
                1_800_000_000,
                &TrustStore::default(),
            )
            .unwrap();
        }
        let k = &keys.keys;
        let list = rows(Page::TrustedKeys, k, None);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].label, "Ada Rider");
        assert!(
            list[0]
                .value
                .as_ref()
                .unwrap()
                .starts_with(&k[0].fingerprint().groups()[0])
        );
        assert!(
            rows(Page::TrustedKeys, &[], None)[0]
                .label
                .contains("No trusted")
        );
        assert!(rows(Page::TrustedKeys, k, Some("broken")).len() == 3);
        let page = rows(Page::TrustedKey(1), k, None);
        assert!(page.len() < MAX_ITEMS);
        assert_eq!(page[0].value.as_deref(), Some("Bo Builder"));
        assert_eq!(page[1].value.as_deref(), Some("2027-01-15"));
        assert_eq!(
            key_emphasis(Page::TrustedKey(1), k).join(" "),
            k[1].fingerprint().to_string()
        );
        assert!(rows(Page::TrustedKey(5), k, None).is_empty());
        // Remove: asks first (Cancel focused), then runs and goes back.
        let rows_of = |p| rows(p, k, None);
        let line = remove_line(&k[1]);
        assert_eq!(pending_key(Some(&line), k), Some(&k[1]));
        let mut m = Menu::default();
        open_at(&mut m, Page::TrustedKey(1), &rows_of);
        let at = m
            .labels()
            .iter()
            .position(|l| l == "Remove this key")
            .unwrap();
        m.apply_with(Action::Click(at), rows_of);
        assert_eq!(m.page(), Some(MenuPage::ConfirmForget));
        assert_eq!(m.focus(), 1, "Cancel is focused");
        assert_eq!(m.pending(), Some(line.as_str()));
        let e = m.apply_with(Action::Click(0), rows_of);
        assert_eq!(e, Effect::Command(line));
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::TrustedKey(1))));
    }

    /// Opens the menu at `page` as the apps do: Settings → Account → its
    /// "Trusted world keys" row (here a row that opens `page`).
    fn open_at(m: &mut Menu, page: Page, rows: &impl Fn(Page) -> Vec<Row>) {
        m.apply(Action::Open);
        let to = |p: Page| {
            move |q: Page| {
                if q == Page::Account {
                    vec![open_row("Trusted world keys", None, p)]
                } else {
                    rows(q)
                }
            }
        };
        let settings = m.labels().iter().position(|l| l == "Settings").unwrap();
        m.apply_with(Action::Click(settings), to(page));
        m.apply_with(Action::Click(0), to(page));
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Account)));
        m.apply_with(Action::Click(0), to(page));
        assert_eq!(m.page(), Some(MenuPage::Sub(page)));
    }
}
