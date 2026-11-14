//! The part picker (spec §14 Phase 4, 4B *Part picker*, *Compare*,
//! *Actions*), pure: a slot's options in the groups *equipped*, *owned*
//! (spare parts, or on another bike) and *shop* (by tier, cheapest first;
//! only parts that fit), the detail card of the focused row with its
//! effect line, compare with a pinned part, and the action dialogs (Cancel
//! focused). Every action is a console command line, as in the menu.
//!
//! Effects (W3, 4G): the card shows the weight difference, the part's
//! class steps and what it does to the bike's speed at the reference ride
//! (250 W, 75 kg; flat and 6 %), from the build physics the server uses
//! (`ocw_sim::build`, the same pure code and catalogue), capped like the
//! server caps it; the compare card shows the speed difference of the two
//! parts on this bike. Parts count in free ride and *Open* races, not in
//! *Profile* or *Equal* races (the default for races and group rides).

use ocw_assets::parts::{Catalogue, Part, Slot};
use ocw_core::Rgb;
use ocw_core::bike::PALETTE;
use ocw_core::hearts::format as hearts;
use ocw_sim::build::{self as bike_build, Build, PartTable};

use super::wallet::{Bike, Garage, fits};

/// What a picker row offers.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// On the bike now (an item, or the base part).
    Equipped { item: Option<u32> },
    /// Owned: a spare part, or on another bike (its name).
    Owned { item: u32, on: Option<String> },
    /// In the shop.
    Shop,
    /// A free paint colour (`None`: the rider's look's frame colour).
    Paint(Option<Rgb>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The catalogue id (paint rows: none).
    pub part: Option<String>,
    pub kind: Kind,
}

/// The rows for `bike`'s `slot`.
pub fn rows(cat: &Catalogue, garage: &Garage, bike: &Bike, slot: Slot) -> Vec<Row> {
    if slot == Slot::Paint {
        let now = bike.paint();
        let mut out = vec![Row {
            part: None,
            kind: Kind::Paint(None),
        }];
        out.extend(PALETTE.iter().map(|c| Row {
            part: None,
            kind: Kind::Paint(Some(*c)),
        }));
        // The current colour first.
        if let Some(i) = out.iter().position(|r| r.kind == Kind::Paint(now)) {
            let r = out.remove(i);
            out.insert(0, r);
        }
        return out;
    }
    let mut out = Vec::new();
    let equipped = bike.parts.get(slot.id()).copied();
    if let Some(p) = garage.part_on(cat, bike, slot) {
        out.push(Row {
            part: Some(p.id.clone()),
            kind: Kind::Equipped { item: equipped },
        });
    }
    // Owned parts of this slot that fit, not on this bike.
    for it in &garage.items {
        let Some(p) = cat.get(&it.part) else { continue };
        if p.slot != slot || !fits(p, bike) || Some(it.id) == equipped {
            continue;
        }
        let on = garage.bike_of(it.id).map(|b| b.name().to_string());
        out.push(Row {
            part: Some(p.id.clone()),
            kind: Kind::Owned { item: it.id, on },
        });
    }
    // The base part when something else is on the bike (unequip).
    if equipped.is_some()
        && let Some(base) = match slot {
            Slot::Frame => cat.base_frame(bike.profile()),
            _ => cat.base(slot, bike.profile().bike()),
        }
    {
        out.push(Row {
            part: Some(base.id.clone()),
            kind: Kind::Owned { item: 0, on: None },
        });
    }
    let mut shop: Vec<&Part> = match slot {
        Slot::Frame => cat.frames(bike.profile()),
        _ => cat.for_slot(slot, bike.profile().bike()),
    }
    .into_iter()
    .filter(|p| p.hearts > 0 && fits(p, bike))
    .collect();
    shop.sort_by_key(|p| (p.tier, p.hearts));
    out.extend(shop.into_iter().map(|p| Row {
        part: Some(p.id.clone()),
        kind: Kind::Shop,
    }));
    out
}

/// The group a row is shown under.
pub fn group(kind: &Kind) -> &'static str {
    match kind {
        Kind::Equipped { .. } => "Equipped",
        Kind::Owned { .. } => "Owned",
        Kind::Shop => "Shop",
        Kind::Paint(_) => "Paint (free)",
    }
}

/// A colour's name for the paint rows.
pub fn colour_name(c: Option<Rgb>) -> String {
    match c {
        None => "Rider's frame colour".into(),
        Some(c) => format!("Paint {c}"),
    }
}

/// The row as shown: (left text, right text, locked).
pub fn row_text(cat: &Catalogue, row: &Row, balance: i64) -> (String, String, bool) {
    if let Kind::Paint(c) = row.kind {
        return (colour_name(c), "free".into(), false);
    }
    let Some(p) = row.part.as_deref().and_then(|id| cat.get(id)) else {
        return ("?".into(), String::new(), false);
    };
    let left = format!("{}  [{}]  {}", p.name, p.tier.name(), grams(p.weight_g));
    match &row.kind {
        Kind::Equipped { .. } => (left, "equipped".into(), false),
        Kind::Owned { item: 0, .. } => (left, "base part".into(), false),
        Kind::Owned { on: Some(b), .. } => (left, format!("owned, on the {b}"), false),
        Kind::Owned { .. } => (left, "owned".into(), false),
        Kind::Shop => {
            let price = i64::from(p.hearts);
            if balance < price {
                (
                    left,
                    format!(
                        "{}  (needs {} more)",
                        hearts(price),
                        hearts(price - balance)
                    ),
                    true,
                )
            } else {
                (left, hearts(price), false)
            }
        }
        Kind::Paint(_) => unreachable!("handled above"),
    }
}

fn grams(g: u32) -> String {
    if g >= 1000 {
        format!("{},{:03} g", g / 1000, g % 1000)
    } else {
        format!("{g} g")
    }
}

fn signed_grams(d: i64) -> String {
    match d.cmp(&0) {
        std::cmp::Ordering::Less => format!("−{}", grams(d.unsigned_abs() as u32)),
        std::cmp::Ordering::Equal => "same weight".into(),
        std::cmp::Ordering::Greater => format!("+{}", grams(d as u32)),
    }
}

/// The class line of a part (4G): rolling ±4 % Crr, aero ±0.004 m² CdA,
/// efficiency ±0.3 % per step.
fn classes(p: &Part) -> Vec<String> {
    let mut out = Vec::new();
    if p.rolling != 0 {
        out.push(format!("rolling resistance {:+} %", p.rolling * 4));
    }
    if p.aero != 0 {
        out.push(format!("drag {:+.3} m² CdA", -f64::from(p.aero) * 0.004));
    }
    if p.efficiency != 0 {
        out.push(format!(
            "drivetrain {:+.1} %",
            f64::from(p.efficiency) * 0.3
        ));
    }
    out
}

/// The part ids that count on `bike` with `part` in its slot (`None`: as
/// it is).
pub fn ids_with(cat: &Catalogue, garage: &Garage, bike: &Bike, part: Option<&Part>) -> Vec<String> {
    let mut ids: Vec<String> = garage
        .part_ids(bike)
        .into_iter()
        .filter(|id| part.is_none_or(|p| cat.get(id).is_none_or(|q| q.slot != p.slot)))
        .collect();
    if let Some(p) = part
        && !p.is_base()
    {
        ids.push(p.id.clone());
    }
    ids
}

/// The build of `bike` (with `part` in its slot), as the server makes it.
/// The cards are drawn every frame: each build is computed once (the caps
/// take a few hundred steady-state solutions).
pub fn build_with(cat: &Catalogue, garage: &Garage, bike: &Bike, part: Option<&Part>) -> Build {
    type Key = (ocw_core::ProfileId, Vec<String>);
    static BUILDS: std::sync::Mutex<Option<std::collections::HashMap<Key, Build>>> =
        std::sync::Mutex::new(None);
    let mut ids = ids_with(cat, garage, bike, part);
    ids.sort();
    let key = (bike.profile(), ids);
    let mut cache = BUILDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = cache.get_or_insert_with(Default::default);
    if map.len() > 512 {
        map.clear();
    }
    map.entry(key)
        .or_insert_with_key(|(profile, ids)| bike_build::build(PartTable::shipped(), *profile, ids))
        .clone()
}

/// The physics of a card: the bike as it is and with the part fitted.
#[derive(Debug, Clone)]
pub struct CardPhysics {
    pub now: Build,
    pub with: Build,
}

impl CardPhysics {
    pub fn of(cat: &Catalogue, garage: &Garage, bike: &Bike, part: &Part) -> Self {
        Self {
            now: build_with(cat, garage, bike, None),
            with: build_with(cat, garage, bike, Some(part)),
        }
    }
}

/// Where parts count (the last line of a card).
pub const WHERE_PARTS_COUNT: &str =
    "Counts in free ride and Open races; cosmetic in Profile / Equal races (the default)";

/// "Bike: 7.42 kg · +1.2 % flat, +1.9 % at 6 % vs stock, capped".
pub fn bike_line(b: &Build) -> String {
    let e = b.effect_from(&Build::stock(b.profile));
    format!(
        "Bike: {:.2} kg · {} vs stock",
        b.params.bike_mass_kg,
        e.line()
    )
}

/// The detail card of `part` against the `equipped` one: the main
/// figures and the effect lines (`physics`: the bike's build now and with
/// the part).
pub fn detail(part: &Part, equipped: Option<&Part>, physics: Option<&CardPhysics>) -> Vec<String> {
    let mut out = vec![
        part.name.clone(),
        format!(
            "{} tier · {} · {}",
            part.tier.name(),
            grams(part.weight_g),
            if part.hearts == 0 {
                "free".to_string()
            } else {
                hearts(i64::from(part.hearts))
            }
        ),
    ];
    let mut effect = Vec::new();
    if let Some(e) = equipped
        && e.id != part.id
    {
        effect.push(signed_grams(
            i64::from(part.weight_g) - i64::from(e.weight_g),
        ));
    }
    effect.extend(classes(part));
    if !effect.is_empty() {
        out.push(format!("Effect: {}", effect.join(" · ")));
    }
    if let Some(ph) = physics {
        if ph.with.parts == ph.now.parts {
            out.push(format!(
                "Speed: on the bike now ({})",
                ph.with.effect_from(&Build::stock(ph.with.profile)).line()
            ));
        } else {
            out.push(format!("Speed: {}", ph.with.effect_from(&ph.now).line()));
        }
        out.push(bike_line(&ph.with));
    }
    out.push(WHERE_PARTS_COUNT.into());
    out
}

/// Better / worse in words (never colour alone).
fn better(lower_is_better: bool, a: i64, b: i64) -> &'static str {
    match (a.cmp(&b), lower_is_better) {
        (std::cmp::Ordering::Equal, _) => "same",
        (std::cmp::Ordering::Less, true) | (std::cmp::Ordering::Greater, false) => "better",
        _ => "worse",
    }
}

/// The comparison of the focused part `b` with the pinned `a`
/// (`physics`: the bike's build with `a` and with `b`).
pub fn compare(a: &Part, b: &Part, physics: Option<(&Build, &Build)>) -> Vec<String> {
    let mut out = vec![
        format!("A: {}", a.name),
        format!("B: {}", b.name),
        format!(
            "Weight: {} vs {} – B {}",
            grams(a.weight_g),
            grams(b.weight_g),
            better(true, i64::from(b.weight_g), i64::from(a.weight_g))
        ),
        format!(
            "Price: {} vs {} – B {}",
            hearts(i64::from(a.hearts)),
            hearts(i64::from(b.hearts)),
            better(true, i64::from(b.hearts), i64::from(a.hearts))
        ),
        format!(
            "Rolling: {:+} vs {:+} – B {}",
            a.rolling,
            b.rolling,
            better(true, i64::from(b.rolling), i64::from(a.rolling))
        ),
        format!(
            "Aero: {:+} vs {:+} – B {}",
            a.aero,
            b.aero,
            better(false, i64::from(b.aero), i64::from(a.aero))
        ),
    ];
    if let Some((with_a, with_b)) = physics {
        let e = with_b.effect_from(with_a);
        // Better by the speed on the flat and the climb together (0.01 %).
        let score = ((e.flat + e.climb) * 1e4).round() as i64;
        out.push(format!(
            "Speed: B {} – B {}",
            e.line(),
            better(false, score, 0)
        ));
    }
    out
}

/// A dialog button: its label and the command line it runs (none: Cancel).
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    pub line: Option<String>,
}

/// An action dialog: Cancel is focused (as for Restart).
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: String,
    pub text: String,
    pub buttons: Vec<Button>,
    pub focus: usize,
}

impl Dialog {
    fn new(title: String, text: String, mut buttons: Vec<Button>) -> Self {
        buttons.push(Button {
            label: "Cancel".into(),
            line: None,
        });
        let focus = buttons.len() - 1;
        Self {
            title,
            text,
            buttons,
            focus,
        }
    }
}

/// The dialog for `row` on `bike` (or `None`: nothing to do, e.g. the
/// equipped base part, or a shop part the balance does not cover).
pub fn dialog(
    cat: &Catalogue,
    garage: &Garage,
    bike: &Bike,
    slot: Slot,
    row: &Row,
    balance: i64,
) -> Option<Dialog> {
    let b = &bike.id;
    if let Kind::Paint(c) = row.kind {
        let arg = c.map_or("default".to_string(), |c| c.to_string());
        return Some(Dialog::new(
            format!("Paint the {}?", bike.name()),
            format!("{} – free.", colour_name(c)),
            vec![Button {
                label: "Paint".into(),
                line: Some(format!("workshop.paint {arg} --bike {b}")),
            }],
        ));
    }
    let p = cat.get(row.part.as_deref()?)?;
    match &row.kind {
        Kind::Shop => {
            let price = i64::from(p.hearts);
            if balance < price {
                return None;
            }
            Some(Dialog::new(
                format!("Buy {}?", p.name),
                format!(
                    "For {}. Balance {} → {}",
                    hearts(price),
                    hearts(balance),
                    hearts(balance - price)
                ),
                vec![Button {
                    label: "Buy".into(),
                    line: Some(format!("workshop.buy {} --bike {b}", p.id)),
                }],
            ))
        }
        Kind::Owned { item: 0, .. } => Some(Dialog::new(
            format!("Back to the {}?", p.name),
            "The part on the bike goes to your spare parts.".into(),
            vec![Button {
                label: "Fit base part".into(),
                line: Some(format!("workshop.unequip {} --bike {b}", slot.id())),
            }],
        )),
        Kind::Owned { item, on } => {
            let it = garage.item(*item)?;
            let mut text = match on {
                Some(other) => format!("Now on the {other}: it gets its base part back."),
                None => "From your spare parts.".into(),
            };
            text.push_str(&format!(
                " Selling gives {}.",
                hearts(i64::from(it.refund()))
            ));
            Some(Dialog::new(
                p.name.clone(),
                text,
                vec![
                    Button {
                        label: "Equip".into(),
                        line: Some(format!("workshop.equip {item} --bike {b}")),
                    },
                    Button {
                        label: format!("Sell for {}", hearts(i64::from(it.refund()))),
                        line: Some(format!("workshop.sell {item}")),
                    },
                ],
            ))
        }
        Kind::Equipped { item: Some(item) } => {
            let it = garage.item(*item)?;
            Some(Dialog::new(
                p.name.clone(),
                format!(
                    "On the bike{}. Selling gives {}.",
                    if it.ridden() {
                        ", ridden"
                    } else {
                        ", not ridden yet"
                    },
                    hearts(i64::from(it.refund()))
                ),
                vec![
                    Button {
                        label: "Unequip".into(),
                        line: Some(format!("workshop.unequip {} --bike {b}", slot.id())),
                    },
                    Button {
                        label: format!("Sell for {}", hearts(i64::from(it.refund()))),
                        line: Some(format!("workshop.sell {item}")),
                    },
                ],
            ))
        }
        Kind::Equipped { item: None } => None,
        Kind::Paint(_) => None,
    }
}

/// The picker of one slot: its rows, the focus, the pinned part, the
/// open dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub slot: Slot,
    pub rows: Vec<Row>,
    pub focus: usize,
    /// The row pinned as *A* for comparing.
    pub pinned: Option<Row>,
    pub dialog: Option<Dialog>,
}

impl Picker {
    pub fn new(slot: Slot, rows: Vec<Row>) -> Self {
        Self {
            slot,
            rows,
            focus: 0,
            pinned: None,
            dialog: None,
        }
    }

    pub fn focused(&self) -> Option<&Row> {
        self.rows.get(self.focus)
    }

    /// New rows (after a purchase …): the focus stays on the same part.
    pub fn refresh(&mut self, rows: Vec<Row>) {
        let was = self.focused().cloned();
        self.rows = rows;
        self.focus = was
            .and_then(|w| {
                self.rows
                    .iter()
                    .position(|r| r.part == w.part && r.kind == w.kind)
                    .or_else(|| self.rows.iter().position(|r| r.part == w.part))
            })
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
    }

    pub fn step(&mut self, d: isize) {
        let n = self.rows.len() as isize;
        if n > 0 {
            self.focus = (self.focus as isize + d).rem_euclid(n) as usize;
        }
    }

    /// Pins the focused row as *A*, or unpins.
    pub fn toggle_pin(&mut self) {
        self.pinned = match self.pinned {
            Some(_) => None,
            None => self.focused().cloned(),
        };
    }

    /// The catalogue part to preview on the bike (paint: none).
    pub fn preview(&self) -> Option<&str> {
        self.focused()?.part.as_deref()
    }

    /// The paint colour to preview (paint rows only).
    pub fn preview_paint(&self) -> Option<Option<Rgb>> {
        match self.focused()?.kind {
            Kind::Paint(c) => Some(c),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workshop::wallet::Wallet;
    use ocw_core::ProfileId;

    fn cat() -> &'static Catalogue {
        ocw_assets::shipped::catalogue()
    }

    fn wallet() -> Wallet {
        let mut w = Wallet::in_memory();
        w.now_unix = Some(1_794_000_000);
        w.garage("Ann", ProfileId::Road);
        w
    }

    fn picker(w: &Wallet, bike: &str, slot: Slot) -> Vec<Row> {
        let g = w.get("Ann").unwrap();
        rows(cat(), g, g.bike(bike).unwrap(), slot)
    }

    #[test]
    fn groups_equipped_owned_shop_and_only_fitting_parts() {
        let mut w = wallet();
        let r = picker(&w, "road", Slot::Tyres);
        assert!(matches!(r[0].kind, Kind::Equipped { item: None }));
        assert!(cat().get(r[0].part.as_deref().unwrap()).unwrap().is_base());
        let shop: Vec<&Part> = r[1..]
            .iter()
            .map(|r| {
                assert_eq!(r.kind, Kind::Shop);
                cat().get(r.part.as_deref().unwrap()).unwrap()
            })
            .collect();
        assert!(!shop.is_empty());
        assert!(shop.iter().all(|p| p.fits(ocw_core::BikeType::Road)));
        // By tier, then the cheapest first.
        assert!(
            shop.windows(2)
                .all(|w| (w[0].tier, w[0].hearts) <= (w[1].tier, w[1].hearts))
        );
        // MTB tyres are not offered on the road bike.
        assert!(
            r.iter()
                .all(|r| !r.part.as_deref().unwrap().contains("mtb"))
        );
        // After buying: equipped is the item, the base part can come back.
        let id = shop[0].id.clone();
        let item = w.buy(cat(), "Ann", "road", &id).unwrap().item;
        let r = picker(&w, "road", Slot::Tyres);
        assert_eq!(r[0].kind, Kind::Equipped { item: Some(item) });
        assert_eq!(r[1].kind, Kind::Owned { item: 0, on: None });
        // On another bike it is "owned, on the Road – standard".
        let r = picker(&w, "road-aero", Slot::Tyres);
        assert!(
            r.iter()
                .any(|r| matches!(&r.kind, Kind::Owned { item: i, on: Some(_) } if *i == item)),
            "{r:?}"
        );
    }

    #[test]
    fn rows_show_prices_locks_and_states() {
        let w = wallet();
        let r = picker(&w, "road", Slot::Wheels);
        let dear = r.iter().rfind(|r| r.kind == Kind::Shop).unwrap();
        let (_, right, locked) = row_text(cat(), dear, 500);
        assert!(
            locked && right.contains("needs") && right.contains('♥'),
            "{right}"
        );
        let (_, right, locked) = row_text(cat(), &r[0], 500);
        assert_eq!((right.as_str(), locked), ("equipped", false));
        // A locked part has no buy dialog; an affordable one does.
        let g = w.get("Ann").unwrap();
        let b = g.bike("road").unwrap();
        assert!(dialog(cat(), g, b, Slot::Wheels, dear, 500).is_none());
        let d = dialog(cat(), g, b, Slot::Wheels, dear, 1_000_000).unwrap();
        assert_eq!(d.buttons[d.focus].label, "Cancel", "Cancel is focused");
        assert_eq!(
            d.buttons[0].line.as_deref(),
            Some(format!("workshop.buy {} --bike road", dear.part.as_deref().unwrap()).as_str())
        );
        assert!(d.text.contains("Balance 1,000,000 ♥ →"), "{}", d.text);
        // The equipped base part: nothing to do.
        assert!(dialog(cat(), g, b, Slot::Wheels, &r[0], 500).is_none());
    }

    #[test]
    fn every_action_is_a_console_line() {
        let mut w = wallet();
        w.grant("Ann", 100_000);
        let r = picker(&w, "road", Slot::Saddle);
        let shop = r.iter().find(|r| r.kind == Kind::Shop).unwrap().clone();
        let item = w
            .buy(cat(), "Ann", "road", shop.part.as_deref().unwrap())
            .unwrap()
            .item;
        let g = w.get("Ann").unwrap();
        let b = g.bike("road").unwrap();
        let r = picker(&w, "road", Slot::Saddle);
        let lines =
            |d: Dialog| -> Vec<String> { d.buttons.into_iter().filter_map(|b| b.line).collect() };
        assert_eq!(
            lines(dialog(cat(), g, b, Slot::Saddle, &r[0], 0).unwrap()),
            vec![
                "workshop.unequip saddle --bike road".to_string(),
                format!("workshop.sell {item}")
            ]
        );
        assert_eq!(
            lines(dialog(cat(), g, b, Slot::Saddle, &r[1], 0).unwrap()),
            vec!["workshop.unequip saddle --bike road".to_string()]
        );
        let aero = g.bike("road-aero").unwrap();
        let r = rows(cat(), g, aero, Slot::Saddle);
        let owned = r
            .iter()
            .find(|r| matches!(r.kind, Kind::Owned { item: i, .. } if i == item))
            .unwrap();
        assert_eq!(
            lines(dialog(cat(), g, aero, Slot::Saddle, owned, 0).unwrap()),
            vec![
                format!("workshop.equip {item} --bike road-aero"),
                format!("workshop.sell {item}")
            ]
        );
        let paint = rows(cat(), g, b, Slot::Paint);
        assert_eq!(paint.len(), 13);
        assert_eq!(paint[0].kind, Kind::Paint(None), "the current colour first");
        assert_eq!(
            lines(dialog(cat(), g, b, Slot::Paint, &paint[1], 0).unwrap()),
            vec![format!("workshop.paint {} --bike road", PALETTE[0])]
        );
    }

    /// The server's catalogue copy (`ocw_sim::build::PartTable`) and the
    /// workshop's catalogue read the same file the same way: the cards
    /// show what the server applies.
    #[test]
    fn the_server_table_matches_the_catalogue() {
        let table = PartTable::shipped();
        assert_eq!(table.parts().len(), cat().parts.len());
        for p in &cat().parts {
            let s = table.get(&p.id).expect("in the server's table");
            assert_eq!(s.slot, p.slot.id(), "{}", p.id);
            assert_eq!(s.base, p.is_base(), "{}", p.id);
            assert_eq!(s.weight_g, p.weight_g, "{}", p.id);
            assert_eq!(s.fits, p.fits, "{}", p.id);
            assert_eq!(s.profile, p.profile, "{}", p.id);
            assert_eq!(
                (s.rolling, s.aero, s.efficiency),
                (p.rolling, p.aero, p.efficiency),
                "{}",
                p.id
            );
        }
    }

    #[test]
    fn detail_compare_and_preview() {
        let w = wallet();
        let r = picker(&w, "road", Slot::Tyres);
        let base = cat().get(r[0].part.as_deref().unwrap()).unwrap();
        let fast = r
            .iter()
            .filter_map(|r| cat().get(r.part.as_deref()?))
            .find(|p| p.rolling < 0)
            .unwrap();
        let g = w.get("Ann").unwrap();
        let b = g.bike("road").unwrap();
        let ph = CardPhysics::of(cat(), g, b, fast);
        let d = detail(fast, Some(base), Some(&ph));
        assert!(
            d.iter()
                .any(|l| l.starts_with("Effect:") && l.contains("rolling resistance -")),
            "{d:?}"
        );
        // The speed from the build physics (W3): race tyres are faster.
        let speed = d
            .iter()
            .find(|l| l.starts_with("Speed: +"))
            .expect("a speed line");
        assert!(
            speed.contains("% flat") && speed.contains("at 6 %"),
            "{speed}"
        );
        assert!(
            d.iter()
                .any(|l| l.starts_with("Bike: ") && l.contains("kg")),
            "{d:?}"
        );
        assert_eq!(d.last().unwrap(), WHERE_PARTS_COUNT);
        assert!(!d.iter().any(|l| l.contains("W3")), "{d:?}");
        // The equipped base part: what the bike does now.
        let now = CardPhysics::of(cat(), g, b, base);
        assert!(
            detail(base, Some(base), Some(&now))
                .iter()
                .any(|l| l.starts_with("Speed: on the bike now"))
        );
        let c = compare(base, fast, Some((&now.with, &ph.with)));
        assert!(
            c.iter()
                .any(|l| l.starts_with("Speed: B +") && l.ends_with("B better")),
            "{c:?}"
        );
        let back = compare(fast, base, Some((&ph.with, &now.with)));
        assert!(
            back.iter()
                .any(|l| l.starts_with("Speed: B −") && l.ends_with("B worse")),
            "{back:?}"
        );
        assert!(
            c.iter()
                .any(|l| l.starts_with("Rolling") && l.ends_with("B better")),
            "{c:?}"
        );
        assert!(
            c.iter()
                .any(|l| l.starts_with("Price") && l.ends_with("B worse")),
            "{c:?}"
        );
        // Pin, move, unpin; the preview follows the focus.
        let mut p = Picker::new(Slot::Tyres, r.clone());
        assert_eq!(p.preview(), Some(base.id.as_str()));
        p.toggle_pin();
        p.step(1);
        assert_eq!(p.pinned.as_ref(), Some(&r[0]));
        assert_eq!(p.preview(), r[1].part.as_deref());
        p.toggle_pin();
        assert!(p.pinned.is_none());
        p.step(-2);
        assert_eq!(p.focus, r.len() - 1, "wraps");
        // A refresh keeps the focused part.
        let at = p.focused().cloned();
        p.refresh(r.clone());
        assert_eq!(p.focused().cloned(), at);
    }
}
