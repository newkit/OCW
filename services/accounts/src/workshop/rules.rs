//! The workshop's rules on the backend (4O W5c): every action checked
//! against the parts catalogue (`assets/parts.toml`, its prices) and the
//! account's garage and balance, the same rules as the local garage of the
//! desktop (`apps/desktop/src/workshop/wallet.rs`, 4D):
//!
//! - a part is bought at the catalogue's price, only if it fits the bike
//!   (the bike type; a frameset: the frame's profile) and the balance
//!   covers it; it is fitted at once, the part it replaces goes to the
//!   spares; a kit item is worn at once;
//! - a groupset comes with the crankset, cassette and chain of its tier
//!   (free items that belong to it, sold with it, never alone);
//! - a frameset bought is a new bike with its **base parts in every slot**
//!   (Q18 (a)), plus the parts bought with it in the same purchase;
//! - selling refunds the price paid before the first ride (1 km), half of
//!   it after; base parts and the starter bike cannot be sold, nor the
//!   last bike.
//!
//! Pure: the caller loads the garage and balance and stores the result
//! with the [`Booking`]s in one transaction.

use std::collections::BTreeMap;

use ocw_account_api::workshop::{Action, Bike, Garage, Injury, Item, MAX_BIKES, Reason};
use ocw_assets::parts::{Catalogue, Part, Slot};
use ocw_core::ProfileId;
use ocw_core::hearts::format as hearts;
use ocw_sim::damage::{self, Part as DamagePart};

/// One line for the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booking {
    pub delta: i64,
    pub reason: Reason,
    pub reference: String,
}

impl Booking {
    pub fn new(delta: i64, reason: Reason, reference: impl Into<String>) -> Self {
        Self {
            delta,
            reason,
            reference: reference.into(),
        }
    }
}

/// What an action did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    pub message: String,
    pub bookings: Vec<Booking>,
    pub item: Option<u32>,
    pub bike: Option<String>,
    pub paid: u32,
    pub refund: u32,
    pub fitted: Vec<String>,
}

/// Why an action was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("{what} costs {}; the balance is {} ({} more needed)", hearts(*.price), hearts(*.balance), hearts(*.price - *.balance))]
    Insufficient {
        what: String,
        price: i64,
        balance: i64,
    },
}

fn profile(b: &Bike) -> ProfileId {
    ProfileId::from_wire(Some(&b.profile))
}

fn bike_name(b: &Bike) -> &'static str {
    profile(b).name()
}

/// Whether `part` fits `bike`.
pub fn fits(part: &Part, bike: &Bike) -> bool {
    if part.slot.is_kit() {
        return true;
    }
    let p = profile(bike);
    if part.slot == Slot::Frame {
        return part.profile == Some(p);
    }
    part.fits.contains(&p.bike())
}

/// The catalogue part on each bike slot: the fitted item's, else the base
/// part (Q18 (a): a bike is always complete).
pub fn resolved<'c>(cat: &'c Catalogue, g: &Garage, bike: &Bike) -> Vec<(Slot, Option<&'c Part>)> {
    Slot::ALL
        .iter()
        .copied()
        .filter(|s| !s.is_kit() && s.has_base())
        .map(|slot| {
            let own = bike
                .parts
                .get(slot.id())
                .and_then(|i| g.item(*i))
                .and_then(|i| cat.get(&i.part));
            let part = own.or_else(|| match slot {
                Slot::Frame => cat.base_frame(profile(bike)),
                _ => cat.base(slot, profile(bike).bike()),
            });
            (slot, part)
        })
        .collect()
}

fn get<'c>(cat: &'c Catalogue, id: &str) -> Result<&'c Part, RuleError> {
    cat.get(id)
        .ok_or_else(|| RuleError::NotFound(format!("no part {id} in the catalogue")))
}

fn bike_ref<'g>(g: &'g Garage, id: &str) -> Result<&'g Bike, RuleError> {
    g.bike(id)
        .ok_or_else(|| RuleError::NotFound(format!("no bike {id} in the garage")))
}

fn bike_mut<'g>(g: &'g mut Garage, id: &str) -> Result<&'g mut Bike, RuleError> {
    g.bike_mut(id)
        .ok_or_else(|| RuleError::NotFound(format!("no bike {id} in the garage")))
}

fn afford(what: &str, price: i64, balance: i64) -> Result<(), RuleError> {
    if balance < price {
        return Err(RuleError::Insufficient {
            what: what.into(),
            price,
            balance,
        });
    }
    Ok(())
}

fn new_item(g: &mut Garage, part: &str, paid: u32, date: &str, with: Option<u32>) -> u32 {
    let id = g.next_item;
    g.next_item += 1;
    g.items.push(Item {
        id,
        part: part.into(),
        paid,
        bought: date.into(),
        ridden_m: 0.0,
        with,
    });
    id
}

/// Fits the crankset, cassette and chain of groupset item `gid` on `bike`
/// (its own items, else new free ones).
fn fit_included(
    cat: &Catalogue,
    g: &mut Garage,
    bike: &str,
    gid: u32,
    groupset: &Part,
    date: &str,
) -> Result<Vec<String>, RuleError> {
    let mut names = Vec::new();
    for c in cat.included(groupset) {
        let own = g.with_groupset(gid).find(|i| i.part == c.id).map(|i| i.id);
        let id = match own {
            Some(id) => id,
            None => new_item(g, &c.id, 0, date, Some(gid)),
        };
        for b in &mut g.bikes {
            if b.id != bike {
                b.parts.retain(|_, i| *i != id);
            }
        }
        bike_mut(g, bike)?.parts.insert(c.slot.id().into(), id);
        names.push(c.name.clone());
    }
    Ok(names)
}

fn new_bike_id(g: &Garage, p: ProfileId) -> String {
    (1..)
        .map(|n| format!("{}-{n}", p.id()))
        .find(|id| g.bike(id).is_none())
        .expect("an unused id")
}

fn buy_part(
    cat: &Catalogue,
    g: &mut Garage,
    balance: i64,
    date: &str,
    bike: &str,
    part_id: &str,
) -> Result<Applied, RuleError> {
    let part = get(cat, part_id)?;
    let b = bike_ref(g, bike)?;
    if let Some(gs) = cat.parts.iter().find(|x| x.includes.contains(&part.id)) {
        return Err(RuleError::Refused(format!(
            "the {} comes with a groupset (e.g. the {}): buy the groupset",
            part.name, gs.name
        )));
    }
    if part.hearts == 0 {
        return Err(RuleError::Refused(format!(
            "{} is a base part: it comes with the frame",
            part.name
        )));
    }
    if part.slot == Slot::Frame {
        return Err(RuleError::Refused(format!(
            "{} is a frameset: buy it as a new bike",
            part.name
        )));
    }
    if !fits(part, b) {
        return Err(RuleError::Refused(format!(
            "{} does not fit the {}",
            part.name,
            bike_name(b)
        )));
    }
    afford(&part.name, i64::from(part.hearts), balance)?;
    let id = new_item(g, &part.id, part.hearts, date, None);
    let mut fitted = Vec::new();
    if part.slot.is_kit() {
        g.kit.insert(part.slot.id().into(), id);
    } else {
        bike_mut(g, bike)?.parts.insert(part.slot.id().into(), id);
        fitted = fit_included(cat, g, bike, id, part, date)?;
    }
    Ok(Applied {
        message: format!(
            "Bought the {} for {}",
            part.name,
            hearts(part.hearts.into())
        ),
        bookings: vec![Booking::new(
            -i64::from(part.hearts),
            Reason::Purchase,
            &part.id,
        )],
        item: Some(id),
        paid: part.hearts,
        fitted,
        ..Applied::default()
    })
}

fn buy_bike(
    cat: &Catalogue,
    g: &mut Garage,
    balance: i64,
    date: &str,
    frame: &str,
    with: &[String],
) -> Result<Applied, RuleError> {
    let f = get(cat, frame)?;
    let Some(p) = f.profile.filter(|_| f.slot == Slot::Frame) else {
        return Err(RuleError::Refused(format!("{} is not a frameset", f.name)));
    };
    if f.hearts == 0 {
        return Err(RuleError::Refused(format!(
            "{} is a base frame: it comes with the starter bike",
            f.name
        )));
    }
    if g.bikes.len() >= MAX_BIKES {
        return Err(RuleError::Refused(format!(
            "the wall is full ({MAX_BIKES} bikes): sell a bike first"
        )));
    }
    let probe = Bike {
        id: new_bike_id(g, p),
        profile: p.id().into(),
        paint: None,
        parts: BTreeMap::new(),
        starter: false,
        condition: BTreeMap::new(),
    };
    let mut parts: Vec<&Part> = Vec::new();
    for id in with {
        let x = get(cat, id)?;
        if x.slot == Slot::Frame || x.slot.is_kit() {
            return Err(RuleError::Refused(format!(
                "{} is not a part for the bike",
                x.name
            )));
        }
        if x.hearts == 0 || cat.is_included(&x.id) {
            return Err(RuleError::Refused(format!("{} is not sold alone", x.name)));
        }
        if !fits(x, &probe) {
            return Err(RuleError::Refused(format!(
                "{} does not fit the {}",
                x.name,
                p.name()
            )));
        }
        if parts.iter().any(|q| q.slot == x.slot) {
            return Err(RuleError::Refused(format!(
                "two parts for the {}",
                x.slot.name()
            )));
        }
        parts.push(x);
    }
    let total = i64::from(f.hearts) + parts.iter().map(|x| i64::from(x.hearts)).sum::<i64>();
    let what = if parts.is_empty() {
        f.name.clone()
    } else {
        format!("the {} with its parts", f.name)
    };
    afford(&what, total, balance)?;
    let bike_id = probe.id.clone();
    let item = new_item(g, &f.id, f.hearts, date, None);
    g.bikes.push(Bike {
        parts: BTreeMap::from([(Slot::Frame.id().to_string(), item)]),
        ..probe
    });
    let mut out = Applied {
        message: String::new(),
        bookings: vec![Booking::new(-i64::from(f.hearts), Reason::Purchase, &f.id)],
        item: Some(item),
        bike: Some(bike_id.clone()),
        paid: total as u32,
        ..Applied::default()
    };
    let mut left = balance - i64::from(f.hearts);
    for x in parts {
        let r = buy_part(cat, g, left, date, &bike_id, &x.id)?;
        left -= i64::from(r.paid);
        out.fitted.push(x.name.clone());
        out.fitted.extend(r.fitted);
        out.bookings.extend(r.bookings);
    }
    out.message = format!(
        "Bought the {} as a new bike ({}) for {}",
        f.name,
        bike_id,
        hearts(total)
    );
    Ok(out)
}

fn sell(g: &mut Garage, item: u32) -> Result<Applied, RuleError> {
    let at = g
        .items
        .iter()
        .position(|i| i.id == item)
        .ok_or_else(|| RuleError::NotFound(format!("no item {item}")))?;
    if let Some(gid) = g.items[at].with {
        return Err(RuleError::Refused(format!(
            "item {item} came with a groupset (item {gid}): it is sold with the groupset"
        )));
    }
    if g.bikes
        .iter()
        .any(|b| b.parts.get(Slot::Frame.id()) == Some(&item))
    {
        return Err(RuleError::Refused(format!(
            "item {item} is the frame of a bike: sell the bike"
        )));
    }
    let it = g.items.remove(at);
    let gone: Vec<u32> = std::iter::once(item)
        .chain(g.with_groupset(item).map(|i| i.id))
        .collect();
    g.items.retain(|i| !gone.contains(&i.id));
    for b in &mut g.bikes {
        b.parts.retain(|_, i| !gone.contains(i));
    }
    g.kit.retain(|_, i| *i != item);
    let refund = it.refund();
    Ok(Applied {
        message: format!("Sold item {item} for {}", hearts(refund.into())),
        bookings: vec![Booking::new(i64::from(refund), Reason::Sale, &it.part)],
        refund,
        ..Applied::default()
    })
}

fn sell_bike(g: &mut Garage, bike: &str) -> Result<Applied, RuleError> {
    let b = bike_ref(g, bike)?.clone();
    if b.starter {
        return Err(RuleError::Refused("the starter bike cannot be sold".into()));
    }
    if g.bikes.len() <= 1 {
        return Err(RuleError::Refused("your last bike cannot be sold".into()));
    }
    let refund: u32 = b
        .parts
        .values()
        .filter_map(|i| g.item(*i))
        .map(Item::refund)
        .sum();
    let gone: Vec<u32> = b.parts.values().copied().collect();
    let with: Vec<u32> = g
        .items
        .iter()
        .filter(|i| i.with.is_some_and(|w| gone.contains(&w)))
        .map(|i| i.id)
        .collect();
    g.items
        .retain(|i| !gone.contains(&i.id) && !with.contains(&i.id));
    for other in &mut g.bikes {
        other.parts.retain(|_, i| !with.contains(i));
    }
    g.bikes.retain(|x| x.id != bike);
    if g.active == bike {
        g.active = g.bikes[0].id.clone();
    }
    Ok(Applied {
        message: format!("Sold the bike {bike} for {}", hearts(refund.into())),
        bookings: vec![Booking::new(
            i64::from(refund),
            Reason::Sale,
            format!("bike:{bike}"),
        )],
        refund,
        ..Applied::default()
    })
}

fn equip(
    cat: &Catalogue,
    g: &mut Garage,
    date: &str,
    bike: &str,
    item: u32,
) -> Result<Applied, RuleError> {
    let it = g
        .item(item)
        .ok_or_else(|| RuleError::NotFound(format!("no item {item}")))?
        .clone();
    let part = get(cat, &it.part)?;
    if part.slot.is_kit() {
        g.kit.insert(part.slot.id().into(), item);
        return Ok(Applied {
            message: format!("Wearing the {}", part.name),
            ..Applied::default()
        });
    }
    if it.with.is_some() {
        return Err(RuleError::Refused(format!(
            "the {} goes with its groupset",
            part.name
        )));
    }
    if part.slot == Slot::Frame {
        return Err(RuleError::Refused("a frameset is a bike of its own".into()));
    }
    let b = bike_ref(g, bike)?;
    if !fits(part, b) {
        return Err(RuleError::Refused(format!(
            "{} does not fit the {}",
            part.name,
            bike_name(b)
        )));
    }
    let from = g.bike_of(item).map(|b| b.id.clone());
    if let Some(from) = from.filter(|f| f != bike) {
        bike_mut(g, &from)?.parts.remove(part.slot.id());
    }
    bike_mut(g, bike)?.parts.insert(part.slot.id().into(), item);
    let fitted = fit_included(cat, g, bike, item, part, date)?;
    Ok(Applied {
        message: format!("The {} is on {bike}", part.name),
        fitted,
        ..Applied::default()
    })
}

fn unequip(g: &mut Garage, bike: &str, slot: &str) -> Result<Applied, RuleError> {
    let s = Slot::parse(slot).ok_or_else(|| RuleError::NotFound(format!("no slot {slot}")))?;
    if s.is_kit() {
        g.kit.remove(s.id());
        return Ok(Applied {
            message: format!("Base {} worn", s.name()),
            ..Applied::default()
        });
    }
    if s == Slot::Frame {
        return Err(RuleError::Refused(
            "the frame stays on its bike: sell the bike".into(),
        ));
    }
    let off = bike_mut(g, bike)?.parts.remove(s.id());
    if let Some(gid) = off {
        let with: Vec<u32> = g.with_groupset(gid).map(|i| i.id).collect();
        bike_mut(g, bike)?.parts.retain(|_, i| !with.contains(i));
    }
    Ok(Applied {
        message: format!("Base {} on {bike}", s.name()),
        ..Applied::default()
    })
}

/// A damage part's condition on `bike` (per mille → 0 … 1; 1: as new).
fn condition(bike: &Bike, p: DamagePart) -> f64 {
    bike.condition
        .get(p.id())
        .map_or(1.0, |pm| f64::from((*pm).min(1000)) / 1000.0)
}

/// What repairing `p` on `bike` costs (0: as new) – the local garage's
/// rule (`ocw_sim::damage::slot_price`, `repair_price`).
pub fn repair_quote(cat: &Catalogue, g: &Garage, bike: &Bike, p: DamagePart) -> u32 {
    let c = condition(bike, p);
    if c >= 1.0 {
        return 0;
    }
    let slot = Slot::parse(p.slot()).expect("damage parts map to slots");
    let paid = bike
        .parts
        .get(slot.id())
        .and_then(|i| g.item(*i))
        .map(|i| i.paid);
    let options = if slot == Slot::Frame {
        cat.frames(profile(bike))
    } else {
        cat.for_slot(slot, profile(bike).bike())
    };
    let cheapest = options
        .iter()
        .filter(|q| q.hearts > 0)
        .map(|q| q.hearts)
        .min();
    damage::repair_price(damage::slot_price(paid, cheapest, p), c, p)
}

/// Repairs `part` of `bike` (`None`: every damaged part) for its price
/// (ledger reason `repair`).
fn repair(
    cat: &Catalogue,
    g: &mut Garage,
    balance: i64,
    bike: &str,
    part: Option<&str>,
) -> Result<Applied, RuleError> {
    let b = bike_ref(g, bike)?;
    let parts: Vec<DamagePart> = match part {
        Some(id) => {
            let p = DamagePart::from_id(id)
                .ok_or_else(|| RuleError::NotFound(format!("no damage part {id}")))?;
            if condition(b, p) >= 1.0 {
                return Err(RuleError::Refused(format!(
                    "the {} needs no repair",
                    p.label()
                )));
            }
            vec![p]
        }
        None => DamagePart::ALL
            .into_iter()
            .filter(|p| condition(b, *p) < 1.0)
            .collect(),
    };
    if parts.is_empty() {
        return Err(RuleError::Refused(format!("{bike} needs no repair")));
    }
    let price: u32 = parts.iter().map(|p| repair_quote(cat, g, b, *p)).sum();
    let what = match parts.as_slice() {
        [p] => format!("repairing the {}", p.label()),
        _ => format!("repairing {} parts", parts.len()),
    };
    afford(&what, i64::from(price), balance)?;
    let b = bike_mut(g, bike)?;
    for p in &parts {
        b.condition.remove(p.id());
    }
    Ok(Applied {
        message: format!(
            "{} of {bike} repaired for {}",
            match parts.as_slice() {
                [p] => format!("The {}", p.label()),
                _ => format!("{} parts", parts.len()),
            },
            hearts(i64::from(price))
        ),
        bookings: (price > 0)
            .then(|| Booking::new(-i64::from(price), Reason::Repair, format!("repair:{bike}")))
            .into_iter()
            .collect(),
        paid: price,
        ..Applied::default()
    })
}

/// Keeps the damage a ride left (spec 14 Phase 12 Q6): the parts'
/// condition of `bike` and the rider's injury, as the app reports them.
fn keep_damage(
    g: &mut Garage,
    bike: &str,
    condition: &BTreeMap<String, u32>,
    injury: Option<Injury>,
) -> Result<Applied, RuleError> {
    for (id, pm) in condition {
        if DamagePart::from_id(id).is_none() {
            return Err(RuleError::NotFound(format!("no damage part {id}")));
        }
        if *pm > 1000 {
            return Err(RuleError::Refused(format!(
                "{id}: {pm} ‰ is not a condition"
            )));
        }
    }
    if injury.is_some_and(|i| i.condition_pm > 1000) {
        return Err(RuleError::Refused("the injury is not a condition".into()));
    }
    bike_mut(g, bike)?.condition = condition
        .iter()
        .filter(|(_, pm)| **pm < 1000)
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    g.injury = injury.filter(|i| i.condition_pm < 1000);
    Ok(Applied {
        message: format!("damage of {bike} kept"),
        ..Applied::default()
    })
}

/// Applies `action` to the garage (`balance`: the account's hearts now;
/// `date`: today, `YYYY-MM-DD`). On an error nothing changed.
pub fn apply(
    cat: &Catalogue,
    g: &mut Garage,
    balance: i64,
    date: &str,
    action: &Action,
) -> Result<Applied, RuleError> {
    let mut work = g.clone();
    let r = match action {
        Action::BuyPart { bike, part } => buy_part(cat, &mut work, balance, date, bike, part),
        Action::BuyBike { frame, with } => buy_bike(cat, &mut work, balance, date, frame, with),
        Action::Sell { item } => sell(&mut work, *item),
        Action::SellBike { bike } => sell_bike(&mut work, bike),
        Action::Equip { bike, item } => equip(cat, &mut work, date, bike, *item),
        Action::Unequip { bike, slot } => unequip(&mut work, bike, slot),
        Action::Select { bike } => {
            bike_ref(&work, bike)?;
            work.active = bike.clone();
            Ok(Applied {
                message: format!("{bike} is on the stand"),
                ..Applied::default()
            })
        }
        Action::Repair { bike, part } => repair(cat, &mut work, balance, bike, part.as_deref()),
        Action::Damage {
            bike,
            condition,
            injury,
        } => keep_damage(&mut work, bike, condition, *injury),
        Action::Paint { bike, colour } => {
            if let Some(c) = colour
                && ocw_core::Rgb::parse(c).is_none()
            {
                return Err(RuleError::Refused(format!("{c} is not a colour (#RRGGBB)")));
            }
            bike_mut(&mut work, bike)?.paint = colour.clone();
            Ok(Applied {
                message: format!("{bike} painted"),
                ..Applied::default()
            })
        }
    }?;
    let spent: i64 = r.bookings.iter().map(|b| b.delta).sum();
    if balance + spent < 0 {
        // Never reached through the checks above; the store's constraint
        // is the last guard.
        return Err(RuleError::Insufficient {
            what: "this".into(),
            price: -spent,
            balance,
        });
    }
    *g = work;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_account_api::START_HEARTS;
    use ocw_account_api::workshop::STARTER_BIKE;

    fn cat() -> &'static Catalogue {
        ocw_assets::shipped::catalogue()
    }

    const DAY: &str = "2026-11-26";

    fn run(g: &mut Garage, bal: &mut i64, a: Action) -> Result<Applied, RuleError> {
        let r = apply(cat(), g, *bal, DAY, &a)?;
        *bal += r.bookings.iter().map(|b| b.delta).sum::<i64>();
        Ok(r)
    }

    #[test]
    fn the_start_buys_a_complete_bike_with_its_base_parts() {
        let mut g = Garage::starter("Ada Rider");
        let mut bal = START_HEARTS;
        // The starter bike is complete with base parts.
        let starter = g.bike(STARTER_BIKE).unwrap().clone();
        assert!(
            resolved(cat(), &g, &starter)
                .iter()
                .all(|(_, p)| p.is_some())
        );
        let r = run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.road.upper".into(),
                with: vec![],
            },
        )
        .unwrap();
        let id = r.bike.unwrap();
        let b = g.bike(&id).unwrap().clone();
        assert_eq!(b.parts.len(), 1, "only the frame is an item");
        assert!(resolved(cat(), &g, &b).iter().all(|(_, p)| p.is_some()));
        assert_eq!(bal, START_HEARTS - 2839);
        // A full mid-tier build with kit fits in 5,000 ♥.
        let mut g = Garage::starter("Bea Climber");
        let mut bal = START_HEARTS;
        run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.road.mid".into(),
                with: vec![
                    "groupset.road-mech.entry".into(),
                    "wheels.road-alloy.mid".into(),
                ],
            },
        )
        .unwrap();
        for kit in ["shoes.race.top", "helmet.light.top", "jersey.aero.upper"] {
            run(
                &mut g,
                &mut bal,
                Action::BuyPart {
                    bike: STARTER_BIKE.into(),
                    part: kit.into(),
                },
            )
            .unwrap();
        }
        assert!((0..START_HEARTS).contains(&bal), "{bal}");
        assert_eq!(g.kit.len(), 3);
    }

    #[test]
    fn never_below_zero() {
        let mut g = Garage::starter("Cleo Sprinter");
        let mut bal = START_HEARTS;
        let e = run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.tt.top".into(),
                with: vec![],
            },
        )
        .unwrap_err();
        assert!(matches!(e, RuleError::Insufficient { .. }), "{e}");
        assert_eq!(bal, START_HEARTS);
        assert_eq!(g, Garage::starter("Cleo Sprinter"), "nothing changed");
        // A complete build that is a little too much is refused whole.
        let e = run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.road.upper".into(),
                with: vec!["groupset.road-electronic.upper".into()],
            },
        )
        .unwrap_err();
        assert!(matches!(e, RuleError::Insufficient { .. }));
        assert_eq!(g.bikes.len(), 1);
    }

    #[test]
    fn groupsets_fit_their_parts_and_sell_with_them() {
        let mut g = Garage::starter("Dana Tempo");
        let mut bal = START_HEARTS;
        let r = run(
            &mut g,
            &mut bal,
            Action::BuyPart {
                bike: STARTER_BIKE.into(),
                part: "groupset.road-mech.entry".into(),
            },
        )
        .unwrap();
        assert_eq!(r.fitted.len(), 3);
        assert_eq!(g.items.len(), 4);
        let included = g.items.iter().find(|i| i.with.is_some()).unwrap().id;
        assert!(run(&mut g, &mut bal, Action::Sell { item: included }).is_err());
        // Buying an included part alone is refused.
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::BuyPart {
                    bike: STARTER_BIKE.into(),
                    part: "cassette.road-11.entry".into(),
                },
            )
            .is_err()
        );
        let r = run(
            &mut g,
            &mut bal,
            Action::Sell {
                item: r.item.unwrap(),
            },
        )
        .unwrap();
        assert_eq!(r.refund, 951);
        assert_eq!(bal, START_HEARTS);
        assert!(g.items.is_empty() && g.bikes[0].parts.is_empty());
    }

    #[test]
    fn selling_rules() {
        let mut g = Garage::starter("Eli Rouleur");
        let mut bal = START_HEARTS;
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::SellBike {
                    bike: STARTER_BIKE.into()
                }
            )
            .is_err()
        );
        let r = run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.gravel.upper".into(),
                with: vec!["tyres.gravel.fast".into()],
            },
        )
        .unwrap();
        let bike = r.bike.unwrap();
        assert_eq!(bal, START_HEARTS - 2799 - 140);
        // The frame is sold with its bike only.
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::Sell {
                    item: r.item.unwrap()
                }
            )
            .is_err()
        );
        // Ridden: half back.
        g.ride(Some(&bike), 1500.0);
        run(&mut g, &mut bal, Action::Select { bike: bike.clone() }).unwrap();
        let r = run(&mut g, &mut bal, Action::SellBike { bike: bike.clone() }).unwrap();
        assert_eq!(r.refund, 2799 / 2 + 140 / 2);
        assert_eq!(g.active, STARTER_BIKE);
        assert_eq!(g.bikes.len(), 1);
        // Base parts are not for sale; wrong bike types are refused.
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::BuyPart {
                    bike: STARTER_BIKE.into(),
                    part: "wheels.road.base".into()
                }
            )
            .is_err()
        );
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::BuyPart {
                    bike: STARTER_BIKE.into(),
                    part: "tyres.mtb.xc".into()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn equip_and_paint() {
        let mut g = Garage::starter("Fay Descender");
        let mut bal = START_HEARTS;
        let w = run(
            &mut g,
            &mut bal,
            Action::BuyPart {
                bike: STARTER_BIKE.into(),
                part: "wheels.road-alloy.entry".into(),
            },
        )
        .unwrap()
        .item
        .unwrap();
        let nb = run(
            &mut g,
            &mut bal,
            Action::BuyBike {
                frame: "frame.road.mid".into(),
                with: vec![],
            },
        )
        .unwrap()
        .bike
        .unwrap();
        run(
            &mut g,
            &mut bal,
            Action::Equip {
                bike: nb.clone(),
                item: w,
            },
        )
        .unwrap();
        assert!(!g.bike(STARTER_BIKE).unwrap().parts.contains_key("wheels"));
        assert_eq!(g.bike(&nb).unwrap().parts.get("wheels"), Some(&w));
        run(
            &mut g,
            &mut bal,
            Action::Unequip {
                bike: nb.clone(),
                slot: "wheels".into(),
            },
        )
        .unwrap();
        assert!(g.bike_of(w).is_none());
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::Paint {
                    bike: nb.clone(),
                    colour: Some("red".into())
                }
            )
            .is_err()
        );
        run(
            &mut g,
            &mut bal,
            Action::Paint {
                bike: nb.clone(),
                colour: Some("#C03020".into()),
            },
        )
        .unwrap();
        assert_eq!(g.bike(&nb).unwrap().paint.as_deref(), Some("#C03020"));
    }

    /// Repairs on the account (spec 14 Phase 4, W6; 4O part 4): the damage
    /// a ride left is kept with the account, a repair costs what the local
    /// garage asks (the shared rule) and leaves the part as new; nothing
    /// to repair, an unknown part or too few hearts are refused.
    #[test]
    fn damage_is_kept_and_repaired_at_the_local_price() {
        let mut g = Garage::starter("Gus Gravel");
        let mut bal = START_HEARTS;
        let bike = STARTER_BIKE.to_string();
        let nothing = run(
            &mut g,
            &mut bal,
            Action::Repair {
                bike: bike.clone(),
                part: None,
            },
        );
        assert!(matches!(nothing, Err(RuleError::Refused(_))), "{nothing:?}");
        let condition: BTreeMap<String, u32> = [
            ("front_wheel".to_string(), 600),
            ("frame".to_string(), 50),
            ("cockpit".to_string(), 1000),
        ]
        .into();
        run(
            &mut g,
            &mut bal,
            Action::Damage {
                bike: bike.clone(),
                condition: condition.clone(),
                injury: Some(Injury {
                    condition_pm: 700,
                    at_unix: 5,
                }),
            },
        )
        .unwrap();
        let b = g.bike(&bike).unwrap();
        assert_eq!(b.condition.len(), 2, "as new is not kept");
        assert_eq!(g.injury.map(|i| i.condition_pm), Some(700));
        assert_eq!(bal, START_HEARTS, "keeping damage is free");
        // Bad reports are refused.
        for c in [
            [("seatpost".to_string(), 10)].into(),
            [("frame".to_string(), 1001)].into(),
        ] {
            assert!(
                run(
                    &mut g,
                    &mut bal,
                    Action::Damage {
                        bike: bike.clone(),
                        condition: c,
                        injury: None,
                    }
                )
                .is_err()
            );
        }
        // The front wheel: the local rule's price.
        let quote = repair_quote(cat(), &g, g.bike(&bike).unwrap(), DamagePart::FrontWheel);
        assert!(quote > 0);
        let r = run(
            &mut g,
            &mut bal,
            Action::Repair {
                bike: bike.clone(),
                part: Some("front_wheel".into()),
            },
        )
        .unwrap();
        assert_eq!(r.paid, quote);
        assert_eq!(bal, START_HEARTS - i64::from(quote));
        assert_eq!(r.bookings[0].reason, Reason::Repair);
        assert!(!g.bike(&bike).unwrap().condition.contains_key("front_wheel"));
        // The frame below 10 %: replaced at the full price; too dear.
        let frame = repair_quote(cat(), &g, g.bike(&bike).unwrap(), DamagePart::Frame);
        let mut poor = 1;
        assert!(matches!(
            run(
                &mut g,
                &mut poor,
                Action::Repair {
                    bike: bike.clone(),
                    part: None
                }
            ),
            Err(RuleError::Insufficient { .. })
        ));
        run(
            &mut g,
            &mut bal,
            Action::Repair {
                bike: bike.clone(),
                part: None,
            },
        )
        .unwrap();
        assert_eq!(bal, START_HEARTS - i64::from(quote + frame));
        assert!(g.bike(&bike).unwrap().condition.is_empty());
        assert!(
            run(
                &mut g,
                &mut bal,
                Action::Repair {
                    bike: bike.clone(),
                    part: Some("tyre".into())
                }
            )
            .is_err()
        );
    }
}
