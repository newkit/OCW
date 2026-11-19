//! The bench catalogue (spec §14 Phase 4, 4D), pure: the workbench (and
//! the "+" hook on the wall) opens the catalogue of **framesets** by bike
//! type and tier – a bought frameset comes with the base parts (entry
//! tier) and hangs on a new hook, to be built up on the stand –, the
//! **complete builds** (the frameset with the mid-tier groupset and wheels
//! in one confirmation, at the sum of the prices, no discount), the
//! **spare parts** on the shelf (fit on the stand bike, sell, compare; a
//! spare frameset hangs on a new hook) and **selling a whole bike** (the
//! frameset's and its parts' refunds; never a starter bike or the last).
//!
//! The pages are pickers ([`Picker::bench`]) of at most six rows (the
//! ten-foot rule of the menu pages, §3.2.1), except the shelf, which
//! lists the inventory; every action is a console line (`bikes.buy`,
//! `bikes.sell`, `bikes.hang`, `workshop.equip`, `workshop.sell`).

use ocw_assets::parts::{Catalogue, Part, Slot, Tier};
use ocw_core::BikeType;
use super::wallet::shown as hearts;
use ocw_sim::build::{self as bike_build, Build, PartTable};

use super::picker::{Button, Dialog, Kind, Picker, Row, WHERE_PARTS_COUNT, detail, included_line};
use super::wallet::{Bike, Garage, MAX_BIKES, fits};

/// A page of the bench catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchPage {
    /// Bike types, the shelf, selling a bike.
    Top,
    /// The framesets of a bike type, by tier.
    Type(BikeType),
    /// The complete builds of a bike type.
    Builds(BikeType),
    /// The spare parts on the shelf.
    Spares,
    /// The bikes that can be sold.
    Sell,
}

impl BenchPage {
    /// The page Back goes to (`None`: the catalogue closes).
    pub fn parent(self) -> Option<Self> {
        match self {
            Self::Top => None,
            Self::Type(_) | Self::Spares | Self::Sell => Some(Self::Top),
            Self::Builds(t) => Some(Self::Type(t)),
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Top => "The bench – bikes and spare parts".into(),
            Self::Type(t) => format!("{} framesets", t.name()),
            Self::Builds(t) => format!("{} complete builds", t.name()),
            Self::Spares => "Spare parts – the shelf".into(),
            Self::Sell => "Sell a bike".into(),
        }
    }
}

/// The parts of a complete build: the groupset and the wheels of the mid
/// tier (or the tier nearest to it the shop has: upper, top, entry).
pub const BUILD_SLOTS: [Slot; 2] = [Slot::Groupset, Slot::Wheels];

/// The tiers a complete build takes its parts from, in order.
const BUILD_TIERS: [Tier; 4] = [Tier::Mid, Tier::Upper, Tier::Top, Tier::Entry];

/// The framesets of `bike` for sale, by tier, cheapest first.
pub fn framesets(cat: &Catalogue, bike: BikeType) -> Vec<&Part> {
    let mut v: Vec<&Part> = cat
        .parts
        .iter()
        .filter(|p| p.slot == Slot::Frame && p.hearts > 0)
        .filter(|p| p.profile.is_some_and(|q| q.bike() == bike))
        .collect();
    v.sort_by_key(|p| (p.tier, p.hearts));
    v
}

/// The parts of `frame`'s complete build ([`BUILD_SLOTS`] at the mid tier
/// or the nearest; a slot the shop has nothing for is left out).
pub fn complete_build<'c>(cat: &'c Catalogue, frame: &Part) -> Vec<&'c Part> {
    let Some(profile) = frame.profile else {
        return Vec::new();
    };
    let bike = profile.bike();
    BUILD_SLOTS
        .iter()
        .filter_map(|slot| {
            let options: Vec<&Part> = cat
                .for_slot(*slot, bike)
                .into_iter()
                .filter(|p| p.hearts > 0 && !cat.is_included(&p.id))
                .collect();
            BUILD_TIERS.iter().find_map(|t| {
                options
                    .iter()
                    .filter(|p| p.tier == *t)
                    .min_by_key(|p| p.hearts)
                    .copied()
            })
        })
        .collect()
}

/// The price of a frameset with parts (no discount).
pub fn price(frame: &Part, parts: &[&Part]) -> i64 {
    i64::from(frame.hearts) + parts.iter().map(|p| i64::from(p.hearts)).sum::<i64>()
}

/// The spare parts the shelf lists: owned, on no bike, not worn; a
/// groupset's own parts are listed with it while it is a spare too.
pub fn shelf(g: &Garage) -> Vec<&super::wallet::Item> {
    let spare: Vec<u32> = g.spares().map(|i| i.id).collect();
    g.spares()
        .filter(|i| i.with.is_none_or(|w| !spare.contains(&w)))
        .collect()
}

/// The rows of `page`.
pub fn rows(cat: &Catalogue, g: &Garage, page: BenchPage) -> Vec<Row> {
    let row = |part: Option<String>, kind: Kind| Row { part, kind };
    match page {
        BenchPage::Top => BikeType::ALL
            .into_iter()
            .filter(|t| !framesets(cat, *t).is_empty())
            .map(|t| row(None, Kind::Page(BenchPage::Type(t))))
            .chain([
                row(None, Kind::Page(BenchPage::Spares)),
                row(None, Kind::Page(BenchPage::Sell)),
            ])
            .collect(),
        BenchPage::Type(t) => {
            let frames = framesets(cat, t);
            let builds = frames.iter().any(|f| !complete_build(cat, f).is_empty());
            frames
                .into_iter()
                .map(|f| row(Some(f.id.clone()), Kind::Frameset))
                .chain(builds.then(|| row(None, Kind::Page(BenchPage::Builds(t)))))
                .collect()
        }
        BenchPage::Builds(t) => framesets(cat, t)
            .into_iter()
            .filter_map(|f| {
                let parts = complete_build(cat, f);
                (!parts.is_empty()).then(|| {
                    row(
                        Some(f.id.clone()),
                        Kind::Build(parts.iter().map(|p| p.id.clone()).collect()),
                    )
                })
            })
            .collect(),
        BenchPage::Spares => shelf(g)
            .into_iter()
            .map(|i| row(Some(i.part.clone()), Kind::Spare { item: i.id }))
            .collect(),
        BenchPage::Sell => g
            .bikes
            .iter()
            .filter(|b| g.can_sell_bike(b).is_ok())
            .map(|b| row(None, Kind::SellBike(b.id.clone())))
            .collect(),
    }
}

fn grams(g: u32) -> String {
    if g >= 1000 {
        format!("{},{:03} g", g / 1000, g % 1000)
    } else {
        format!("{g} g")
    }
}

/// The price on the right of a row, with the lock when the balance is too
/// low.
fn priced(price: i64, balance: i64) -> (String, bool) {
    if balance >= super::wallet::UNLIMITED / 2 {
        return ("free".into(), false);
    }
    if balance < price {
        (
            format!(
                "{}  (needs {} more)",
                hearts(price),
                hearts(price - balance)
            ),
            true,
        )
    } else {
        (hearts(price), false)
    }
}

/// The row as shown (left, right, locked); `None`: not a bench row.
pub fn row_text(
    cat: &Catalogue,
    g: &Garage,
    row: &Row,
    balance: i64,
) -> Option<(String, String, bool)> {
    let part = row.part.as_deref().and_then(|id| cat.get(id));
    Some(match (&row.kind, part) {
        (Kind::Page(p), _) => match p {
            BenchPage::Type(t) => {
                let n = framesets(cat, *t).len();
                (
                    format!("{} bikes", t.name()),
                    format!("{n} frameset{}", if n == 1 { "" } else { "s" }),
                    false,
                )
            }
            BenchPage::Builds(_) => (
                "Complete builds".into(),
                "frameset + groupset + wheels".into(),
                false,
            ),
            BenchPage::Spares => (
                "Spare parts".into(),
                format!("{} on the shelf", shelf(g).len()),
                false,
            ),
            BenchPage::Sell => {
                let n = g
                    .bikes
                    .iter()
                    .filter(|b| g.can_sell_bike(b).is_ok())
                    .count();
                ("Sell a bike".into(), format!("{n} to sell"), false)
            }
            BenchPage::Top => ("The bench".into(), String::new(), false),
        },
        (Kind::Frameset, Some(f)) => {
            let (right, locked) = priced(i64::from(f.hearts), balance);
            (
                format!("{}  [{}]  {}", f.name, f.tier.name(), grams(f.weight_g)),
                right,
                locked,
            )
        }
        (Kind::Build(ids), Some(f)) => {
            let parts: Vec<&Part> = ids.iter().filter_map(|id| cat.get(id)).collect();
            let (right, locked) = priced(price(f, &parts), balance);
            let with: Vec<&str> = parts.iter().map(|p| p.slot.name()).collect();
            (
                format!("{} + {}", f.name, with.join(" + ").to_lowercase()),
                right,
                locked,
            )
        }
        (Kind::Spare { item }, Some(p)) => {
            let refund = g.item(*item).map_or(0, |i| i.refund());
            let right = if g.item(*item).is_some_and(|i| i.with.is_some()) {
                "spare, with its groupset".to_string()
            } else {
                format!("spare · sells for {}", hearts(i64::from(refund)))
            };
            (
                format!("{}  [{}]  {}", p.name, p.tier.name(), grams(p.weight_g)),
                right,
                false,
            )
        }
        (Kind::SellBike(id), _) => {
            let b = g.bike(id)?;
            (
                b.label(),
                format!("sells for {}", hearts(i64::from(g.bike_refund(b)))),
                false,
            )
        }
        _ => return None,
    })
}

/// A frameset's (or a build's) effect on the bike it makes: against the
/// stock bike of its profile, at the reference rider (4G).
fn effect_line(frame: &Part, parts: &[&Part]) -> Option<String> {
    let profile = frame.profile?;
    let ids: Vec<String> = std::iter::once(frame)
        .chain(parts.iter().copied())
        .map(|p| p.id.clone())
        .collect();
    let b = bike_build::build(PartTable::shipped(), profile, &ids);
    Some(format!(
        "{} – {}",
        profile.name(),
        b.effect_from(&Build::stock(profile)).line()
    ))
}

/// The detail card of the focused row.
pub fn card(
    cat: &Catalogue,
    g: &Garage,
    stand: Option<&Bike>,
    page: BenchPage,
    row: Option<&Row>,
) -> Vec<String> {
    let Some(row) = row else {
        return match page {
            BenchPage::Spares => vec![
                "The shelf is empty".into(),
                "Parts you take off a bike, or replace, land here.".into(),
            ],
            BenchPage::Sell => vec![
                "No bike to sell".into(),
                "The six starter bikes and your last bike are never sold; bikes bought at the bench are."
                    .into(),
            ],
            _ => Vec::new(),
        };
    };
    let part = row.part.as_deref().and_then(|id| cat.get(id));
    match (&row.kind, part) {
        (Kind::Page(p), _) => vec![
            p.title(),
            match p {
                BenchPage::Type(_) => "Framesets by tier: a new bike with the base parts (entry tier) on a new hook – build it up on the stand.".into(),
                BenchPage::Builds(_) => "The frameset with the mid-tier groupset and wheels in one purchase, at the sum of their prices (no discount).".into(),
                BenchPage::Spares => "Parts on no bike: fit one on the stand bike, sell it, compare (Y / C).".into(),
                BenchPage::Sell => "A bike bought at the bench: the frameset's and its parts' refunds (in full before their first ride, half after); the hook is freed.".into(),
                BenchPage::Top => String::new(),
            },
        ],
        (Kind::Frameset, Some(f)) => {
            let mut out = vec![
                f.name.clone(),
                format!("{} tier · {} · {}", f.tier.name(), grams(f.weight_g), hearts(i64::from(f.hearts))),
                "A new bike: frame and fork with the base parts (entry tier), on a new hook; build it up on the stand.".into(),
            ];
            out.extend(effect_line(f, &[]));
            out.push(WHERE_PARTS_COUNT.into());
            out
        }
        (Kind::Build(ids), Some(f)) => {
            let parts: Vec<&Part> = ids.iter().filter_map(|id| cat.get(id)).collect();
            let mut out = vec![format!("{} – complete build", f.name)];
            out.push(format!("Frameset: {}", hearts(i64::from(f.hearts))));
            for p in &parts {
                out.push(format!("{}: {} – {}", p.slot.name(), p.name, hearts(i64::from(p.hearts))));
                let inc = included_line(cat, p);
                if !inc.is_empty() {
                    out.push(inc);
                }
            }
            out.push(format!("Sum: {} (no discount)", hearts(price(f, &parts))));
            out.extend(effect_line(f, &parts));
            out
        }
        (Kind::Spare { item }, Some(p)) => {
            let mut out = detail(cat, p, None, None);
            if let Some(b) = stand {
                out.insert(
                    1,
                    if p.slot == Slot::Frame {
                        "A frameset: hangs on a new hook as a bike (or fits a bike of its profile)."
                            .into()
                    } else if p.slot.is_kit() {
                        "Kit: worn at the mirror (the changing corner).".into()
                    } else if fits(p, b) {
                        format!("Fits the {} on the stand.", b.label())
                    } else {
                        format!("Does not fit the {} on the stand.", b.label())
                    },
                );
            }
            if let Some(i) = g.item(*item).filter(|i| i.with.is_none()) {
                out.insert(1, format!("Sells for {} ({})", hearts(i64::from(i.refund())), if i.ridden() { "ridden: half" } else { "not ridden yet: in full" }));
            }
            out
        }
        (Kind::SellBike(id), _) => {
            let Some(b) = g.bike(id) else { return Vec::new() };
            let mut out = vec![b.label(), format!("{} · sells for {}", b.profile().bike().name(), hearts(i64::from(g.bike_refund(b))))];
            for i in b.parts.values().filter_map(|i| g.item(*i)).filter(|i| i.paid > 0) {
                let name = cat.get(&i.part).map_or(i.part.as_str(), |p| p.name.as_str());
                out.push(format!("{name}: {} of {}", hearts(i64::from(i.refund())), hearts(i64::from(i.paid))));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The dialog for `row` (Cancel focused; `None`: nothing to do – a
/// locked row, or a page row, which opens the page).
pub fn dialog(
    cat: &Catalogue,
    g: &Garage,
    stand: Option<&Bike>,
    row: &Row,
    balance: i64,
) -> Option<Dialog> {
    let part = row.part.as_deref().and_then(|id| cat.get(id));
    let full = g.bikes.len() >= MAX_BIKES;
    let wall = if full {
        format!(" The wall is full ({MAX_BIKES} bikes): sell a bike first.")
    } else {
        String::new()
    };
    match (&row.kind, part) {
        (Kind::Frameset | Kind::Build(_), Some(f)) => {
            let parts: Vec<&Part> = match &row.kind {
                Kind::Build(ids) => ids.iter().filter_map(|id| cat.get(id)).collect(),
                _ => Vec::new(),
            };
            let total = price(f, &parts);
            if balance < total {
                return None;
            }
            let mut line = format!("bikes.buy {}", f.id);
            for p in &parts {
                line.push_str(&format!(" --with {}", p.id));
            }
            let what = if parts.is_empty() {
                "It comes with the base parts (entry tier)".to_string()
            } else {
                let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
                format!(
                    "With the {} (the sum of the prices)",
                    names.join(" and the ")
                )
            };
            Some(Dialog::new(
                format!("Buy the {}?", f.name),
                format!(
                    "For {}. Balance {} → {}. {what} and hangs on a new hook.{wall}",
                    hearts(total),
                    hearts(balance),
                    hearts(balance - total)
                ),
                if full {
                    Vec::new()
                } else {
                    vec![Button {
                        label: "Buy".into(),
                        line: Some(line),
                    }]
                },
            ))
        }
        (Kind::Spare { item }, Some(p)) => {
            let it = g.item(*item)?;
            let mut buttons = Vec::new();
            if p.slot == Slot::Frame && !full {
                buttons.push(Button {
                    label: "Hang on a new hook".into(),
                    line: Some(format!("bikes.hang {item}")),
                });
            }
            if let Some(b) = stand.filter(|b| !p.slot.is_kit() && fits(p, b)) {
                buttons.push(Button {
                    label: format!("Fit on the {}", b.label()),
                    line: Some(format!("workshop.equip {item} --bike {}", b.id)),
                });
            }
            let text = if it.with.is_some() {
                "It came with a groupset and is sold with it.".to_string()
            } else {
                buttons.push(Button {
                    label: format!("Sell for {}", hearts(i64::from(it.refund()))),
                    line: Some(format!("workshop.sell {item}")),
                });
                format!(
                    "A spare part. Selling gives {}.",
                    hearts(i64::from(it.refund()))
                )
            };
            Some(Dialog::new(
                p.name.clone(),
                format!("{text}{wall}"),
                buttons,
            ))
        }
        (Kind::SellBike(id), _) => {
            let b = g.bike(id)?;
            g.can_sell_bike(b).ok()?;
            let refund = g.bike_refund(b);
            Some(Dialog::new(
                format!("Sell the {}?", b.label()),
                format!(
                    "The frameset and its parts for {}. Balance {} → {}. Its hook is freed.",
                    hearts(i64::from(refund)),
                    hearts(balance),
                    hearts(balance + i64::from(refund))
                ),
                vec![Button {
                    label: format!("Sell for {}", hearts(i64::from(refund))),
                    line: Some(format!("bikes.sell {id}")),
                }],
            ))
        }
        _ => None,
    }
}

impl Picker {
    /// A page of the bench catalogue.
    pub fn bench(page: BenchPage, rows: Vec<Row>) -> Self {
        Self {
            bench: Some(page),
            ..Self::new(Slot::Frame, rows)
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

    /// The pages follow the ten-foot rule (at most six rows) and list the
    /// framesets by bike type and tier, cheapest first, with their builds.
    #[test]
    fn pages_list_framesets_by_type_and_tier_in_six_rows() {
        let w = wallet();
        let g = w.get("Ann").unwrap();
        let top = rows(cat(), g, BenchPage::Top);
        assert!(top.len() <= 6, "{top:?}");
        assert_eq!(top.len(), 6, "four bike types, the shelf, selling");
        for t in BikeType::ALL {
            let page = rows(cat(), g, BenchPage::Type(t));
            assert!(page.len() <= 6, "{t:?}: {}", page.len());
            let frames: Vec<&Part> = page
                .iter()
                .filter(|r| r.kind == Kind::Frameset)
                .map(|r| cat().get(r.part.as_deref().unwrap()).unwrap())
                .collect();
            assert!(!frames.is_empty(), "{t:?}");
            assert!(
                frames
                    .iter()
                    .all(|f| f.hearts > 0 && f.profile.unwrap().bike() == t)
            );
            assert!(
                frames
                    .windows(2)
                    .all(|w| (w[0].tier, w[0].hearts) <= (w[1].tier, w[1].hearts))
            );
            // The builds page: one row per frameset with a build.
            assert_eq!(page.last().unwrap().kind, Kind::Page(BenchPage::Builds(t)));
            let builds = rows(cat(), g, BenchPage::Builds(t));
            assert!(builds.len() <= 6 && !builds.is_empty());
        }
        // Road: the alloy, carbon, climbing and aero framesets.
        assert_eq!(framesets(cat(), BikeType::Road).len(), 4);
        // Nothing to sell and nothing on the shelf at the start.
        assert!(rows(cat(), g, BenchPage::Sell).is_empty());
        assert!(rows(cat(), g, BenchPage::Spares).is_empty());
        assert!(!card(cat(), g, None, BenchPage::Sell, None).is_empty());
    }

    /// A complete build takes the mid-tier groupset and wheels (the
    /// nearest tier the shop has), all fitting, none a groupset's own part.
    #[test]
    fn complete_builds_take_the_mid_tier_groupset_and_wheels() {
        let road = cat().get("frame.road.upper").unwrap();
        let ids: Vec<&str> = complete_build(cat(), road)
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(ids, ["groupset.road-mech.mid", "wheels.road-alloy.mid"]);
        assert_eq!(
            price(road, &complete_build(cat(), road)),
            2_839 + 1_441 + 600
        );
        // Gravel has no mid wheels: the upper ones.
        let gravel = cat().get("frame.gravel.upper").unwrap();
        let ids: Vec<&str> = complete_build(cat(), gravel)
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(ids, ["groupset.gravel.mid", "wheels.gravel.carbon"]);
        for t in BikeType::ALL {
            for f in framesets(cat(), t) {
                for p in complete_build(cat(), f) {
                    assert!(
                        p.fits(t) && p.hearts > 0 && !cat().is_included(&p.id),
                        "{}",
                        p.id
                    );
                }
            }
        }
    }

    /// Rows show prices and locks; the dialogs (Cancel focused) are the
    /// console lines of the wallet's bench operations.
    #[test]
    fn dialogs_buy_frames_and_builds_and_sell_bikes() {
        let mut w = wallet();
        let g = w.get("Ann").unwrap();
        let page = rows(cat(), g, BenchPage::Type(BikeType::Road));
        let carbon = page
            .iter()
            .find(|r| r.part.as_deref() == Some("frame.road.upper"))
            .unwrap();
        let (_, right, locked) = row_text(cat(), g, carbon, 500).unwrap();
        assert!(locked && right.contains("more"), "{right}");
        assert!(dialog(cat(), g, None, carbon, 500).is_none(), "locked");
        let d = dialog(cat(), g, None, carbon, 5_000).unwrap();
        assert_eq!(d.buttons[d.focus].label, "Cancel");
        assert_eq!(
            d.buttons[0].line.as_deref(),
            Some("bikes.buy frame.road.upper")
        );
        assert!(d.text.contains("new hook"), "{}", d.text);
        let builds = rows(cat(), g, BenchPage::Builds(BikeType::Road));
        let b = builds
            .iter()
            .find(|r| r.part.as_deref() == Some("frame.road.upper"))
            .unwrap();
        let d = dialog(cat(), g, None, b, 10_000).unwrap();
        assert_eq!(
            d.buttons[0].line.as_deref(),
            Some(
                "bikes.buy frame.road.upper --with groupset.road-mech.mid --with wheels.road-alloy.mid"
            )
        );
        let (_, right, _) = row_text(cat(), g, b, 10_000).unwrap();
        assert_eq!(right, hearts(2_839 + 1_441 + 600));
        // The cards: the frameset's effect on its profile, the build's sum.
        let c = card(cat(), g, None, BenchPage::Builds(BikeType::Road), Some(b));
        assert!(c.iter().any(|l| l.starts_with("Sum:")), "{c:?}");
        // Bought: the bike is listed for selling, with its refund.
        w.grant("Ann", 5_000);
        let r = w.buy_bike(cat(), "Ann", "frame.road.upper", &[]).unwrap();
        let g = w.get("Ann").unwrap();
        let sell = rows(cat(), g, BenchPage::Sell);
        assert_eq!(
            sell,
            vec![Row {
                part: None,
                kind: Kind::SellBike(r.bike.clone())
            }]
        );
        let d = dialog(cat(), g, None, &sell[0], 0).unwrap();
        assert_eq!(d.buttons[0].line, Some(format!("bikes.sell {}", r.bike)));
        assert!(
            d.buttons[0].label.contains("2,839"),
            "{}",
            d.buttons[0].label
        );
    }

    /// The shelf: spare parts with their refund; fit on the stand bike when
    /// it fits; a spare frameset hangs on a new hook.
    #[test]
    fn the_shelf_lists_spares_to_fit_sell_or_hang() {
        let mut w = wallet();
        w.grant("Ann", 5_000);
        let tyres = w.buy(cat(), "Ann", "road", "tyres.road.race").unwrap();
        w.unequip("Ann", "road", Slot::Tyres).unwrap();
        let frame = w.buy(cat(), "Ann", "road", "frame.road.mid").unwrap();
        w.unequip("Ann", "road", Slot::Frame).unwrap();
        let g = w.get("Ann").unwrap();
        let shelf = rows(cat(), g, BenchPage::Spares);
        assert_eq!(shelf.len(), 2);
        let stand = g.bike("road");
        let t = shelf
            .iter()
            .find(|r| r.kind == Kind::Spare { item: tyres.item })
            .unwrap();
        let d = dialog(cat(), g, stand, t, 0).unwrap();
        let lines: Vec<_> = d.buttons.iter().filter_map(|b| b.line.clone()).collect();
        assert_eq!(
            lines,
            vec![
                format!("workshop.equip {} --bike road", tyres.item),
                format!("workshop.sell {}", tyres.item)
            ]
        );
        // On the MTB the road tyres do not fit: only selling.
        let d = dialog(cat(), g, g.bike("mtb"), t, 0).unwrap();
        assert_eq!(d.buttons.len(), 2, "sell and Cancel");
        let f = shelf
            .iter()
            .find(|r| r.kind == Kind::Spare { item: frame.item })
            .unwrap();
        let d = dialog(cat(), g, stand, f, 0).unwrap();
        assert_eq!(
            d.buttons[0].line,
            Some(format!("bikes.hang {}", frame.item))
        );
        let (_, right, _) = row_text(cat(), g, t, 0).unwrap();
        assert!(right.contains("190"), "{right}");
    }
}
