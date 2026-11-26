//! The local wallet (spec §14 Phase 4, 4E *Storage – before the
//! backend*): per rider profile the garage (bikes, the parts on them,
//! spare parts) in `<data dir>/ocw/workshop.toml` and the hearts in an
//! append-only ledger, `hearts-ledger.jsonl`. Bevy-free.
//!
//! Rules: every rider starts with the six starter bikes (one per physics
//! profile, base parts) and 500 ♥; parts are bought at the catalogue's
//! price, only if they fit the bike (bike type, or the frame's profile
//! for framesets) and the balance covers them; a bought part is equipped
//! at once, the one it replaces goes to the spare parts; selling refunds
//! the price paid in full while the part was never ridden (no ride of
//! ≥ 1 km with it on the bike), afterwards half of it, rounded down;
//! base parts and starter bikes cannot be sold. The balance is the sum of
//! the ledger.
//!
//! A groupset (spec 4B *Groupset and its parts*) is fitted with the
//! crankset, cassette and chain of its tier (the catalogue's `includes`):
//! buying or equipping it puts them on the bike too (free items that
//! belong to the groupset's item, [`Item::with`]); the parts they replace
//! go to the spare parts. They can be changed alone afterwards (a power
//! meter crankset), go with the groupset to another bike, back to the
//! spares when it is taken off, and with it when it is sold (its refund
//! rule is unchanged; they are never bought or sold alone).
//!
//! The rider's kit (spec 4C, W4) lives in the same garage: `kit` maps a
//! kit slot to the worn item (a slot without one wears its free base
//! item); kit items are bought, worn, taken off and sold by the same rules
//! (they count as ridden after 1 km in the saddle), on any bike. The files are trivially editable – accepted, they only
//! matter on the local server until the Phase 5 backend.
//!
//! Damage (spec §14 Phase 12 Q6, Phase 4 W6): a bike's parts keep their
//! condition between rides (`condition`: damage part → per mille; a
//! missing part is as new) and the rider's injury with the time it was
//! saved (`injury`; it heals over real time, `ocw_sim::damage::heal`).
//! Repairs are paid in hearts: `ceil(price × (1 − condition) × 0.3)` ♥
//! ([`ocw_sim::damage::repair_price`]); the price is the part's on the
//! slot (the price paid), a base part's that of the cheapest paid part of
//! the slot that fits the bike (a new entry part, [`slot_price`]), or
//! [`BASE_REPAIR_HEARTS`] if the catalogue has none; wheels and tyres are
//! pairs (half each); a frame or wheel below 10 % is replaced at the full
//! price.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ocw_assets::parts::{Catalogue, Part, Slot};
use ocw_client::damage::Carried;
use ocw_core::hearts::START_HEARTS;
use ocw_core::{ProfileId, Rgb};
use ocw_sim::damage::{self, PARTS, Part as DamagePart};
use serde::{Deserialize, Serialize};

/// A ride of this length with a part on the bike makes it "ridden" (m).
pub const RIDDEN_M: f64 = 1000.0;

/// A part the rider owns (bought).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    /// The catalogue id.
    pub part: String,
    /// Hearts paid (refunds use it, never today's price).
    pub paid: u32,
    /// When it was bought (`YYYY-MM-DD`).
    pub bought: String,
    /// Metres ridden with it on the bike.
    #[serde(default)]
    pub ridden_m: f64,
    /// The groupset item this part came with (its crankset, cassette,
    /// chain; spec 4B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<u32>,
}

impl Item {
    pub fn ridden(&self) -> bool {
        self.ridden_m >= RIDDEN_M
    }

    /// What selling it gives back: the price paid before the first ride,
    /// half of it (rounded down) after.
    pub fn refund(&self) -> u32 {
        if self.ridden() {
            self.paid / 2
        } else {
            self.paid
        }
    }
}

/// A bike in the garage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bike {
    /// Garage id: the profile id for the starter bikes.
    pub id: String,
    /// The frame's physics profile.
    pub profile: String,
    /// Frame paint (`#RRGGBB`; free, spec §14 Phase 4, 4B).
    #[serde(default)]
    pub paint: Option<String>,
    /// Slot id → item id; a slot without an entry has its base part.
    #[serde(default)]
    pub parts: BTreeMap<String, u32>,
    /// A starter bike (cannot be sold).
    #[serde(default)]
    pub starter: bool,
    /// Damage carried over (spec §14 Phase 12 Q6): damage part id
    /// (`ocw_sim::damage::Part::id`) → condition in per mille; missing:
    /// as new.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub condition: BTreeMap<String, u32>,
}

impl Bike {
    pub fn profile(&self) -> ProfileId {
        ProfileId::from_wire(Some(&self.profile))
    }

    pub fn paint(&self) -> Option<Rgb> {
        self.paint.as_deref().and_then(Rgb::parse)
    }

    /// The display name: "Road – aero".
    pub fn name(&self) -> &'static str {
        self.profile().name()
    }

    /// The name on cards and in lists: a starter bike's profile name, a
    /// bought one's with its number ("Road - standard 2", spec 4D).
    pub fn label(&self) -> String {
        match self.id.rsplit_once('-').map(|(_, n)| n) {
            Some(n) if !self.starter && n.chars().all(|c| c.is_ascii_digit()) => {
                format!("{} {n}", self.name())
            }
            _ => self.name().to_string(),
        }
    }

    /// A damage part's condition (0–1).
    pub fn condition(&self, p: DamagePart) -> f64 {
        self.condition
            .get(p.id())
            .map_or(1.0, |pm| f64::from((*pm).min(1000)) / 1000.0)
    }

    /// Every damage part's condition.
    pub fn conditions(&self) -> [f64; PARTS] {
        DamagePart::ALL.map(|p| self.condition(p))
    }

    /// The parts with damage, worst first.
    pub fn damaged(&self) -> Vec<(DamagePart, f64)> {
        let mut v: Vec<(DamagePart, f64)> = DamagePart::ALL
            .into_iter()
            .map(|p| (p, self.condition(p)))
            .filter(|(_, c)| *c < 1.0)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    fn set_conditions(&mut self, parts: &[f64; PARTS]) {
        self.condition = DamagePart::ALL
            .into_iter()
            .zip(parts)
            .map(|(p, c)| (p.id().to_string(), damage::per_mille(*c)))
            .filter(|(_, pm)| *pm < 1000)
            .collect();
    }
}

/// The rider's injury carried over (spec §14 Phase 12 Q6): the condition
/// (per mille) when it was saved, and when (UTC unix s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Injury {
    pub condition_pm: u32,
    pub at_unix: i64,
}

impl Injury {
    /// The condition at `now_unix`: healed over the real time since.
    pub fn at(&self, now_unix: i64) -> f64 {
        let c = f64::from(self.condition_pm.min(1000)) / 1000.0;
        damage::heal(c, (now_unix - self.at_unix) as f64 / 3600.0)
    }
}

/// A base part's repair price when the catalogue has no paid part of the
/// slot for the bike (hearts, W6).
pub const BASE_REPAIR_HEARTS: u32 = 20;

/// The price a damage part's repair is priced from (W6): the price paid
/// for the item on its slot; a base part: the cheapest paid part of the
/// slot that fits the bike (frames: of its profile), or
/// [`BASE_REPAIR_HEARTS`]; wheels and tyres: half the pair's (rounded up).
pub fn slot_price(cat: &Catalogue, g: &Garage, bike: &Bike, p: DamagePart) -> u32 {
    let slot = Slot::parse(p.slot()).expect("damage parts map to slots");
    let paid = bike
        .parts
        .get(slot.id())
        .and_then(|i| g.item(*i))
        .map(|i| i.paid)
        .filter(|h| *h > 0);
    let price = paid.unwrap_or_else(|| {
        let options = if slot == Slot::Frame {
            cat.frames(bike.profile())
        } else {
            cat.for_slot(slot, bike.profile().bike())
        };
        options
            .iter()
            .filter(|q| q.hearts > 0)
            .map(|q| q.hearts)
            .min()
            .unwrap_or(BASE_REPAIR_HEARTS)
    });
    (f64::from(price) * p.price_share()).ceil() as u32
}

/// The repair price of damage part `p` on `bike` (0: as new).
pub fn repair_quote(cat: &Catalogue, g: &Garage, bike: &Bike, p: DamagePart) -> u32 {
    let c = bike.condition(p);
    if c >= 1.0 {
        return 0;
    }
    damage::repair_price(slot_price(cat, g, bike, p), c, p)
}

/// One rider's garage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Garage {
    pub rider: String,
    /// The bike on the stand / ridden.
    pub active: String,
    #[serde(default, rename = "bike")]
    pub bikes: Vec<Bike>,
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
    #[serde(default = "first_item")]
    pub next_item: u32,
    /// The worn kit (spec 4C): kit slot id → item id; a slot without an
    /// entry wears its free base item.
    #[serde(default)]
    pub kit: BTreeMap<String, u32>,
    /// The rider's injury carried over (spec §14 Phase 12 Q6; `None`:
    /// fine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injury: Option<Injury>,
    /// The kit's own colours (shorts, socks, shoes, gloves; spec 4C, Phase
    /// 8 T6): looks only, never physics. `None`: never set here (the rider
    /// profile's, if any, stay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_colours: Option<KitColours>,
}

/// The colours of the kit (`#RRGGBB`; `None`: the item's own colour).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitColours {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shorts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gloves: Option<String>,
}

impl KitColours {
    /// The kit colours of a design.
    pub fn of(d: &ocw_core::KitDesign) -> Self {
        let s = |c: Option<Rgb>| c.map(|c| c.to_string());
        Self {
            shorts: s(d.shorts),
            socks: s(d.socks),
            shoes: s(d.shoes),
            gloves: s(d.gloves),
        }
    }

    /// `d` with these colours (unparsable ones: the item's own).
    pub fn apply(&self, d: &mut ocw_core::KitDesign) {
        let p = |c: &Option<String>| c.as_deref().and_then(Rgb::parse);
        d.shorts = p(&self.shorts);
        d.socks = p(&self.socks);
        d.shoes = p(&self.shoes);
        d.gloves = p(&self.gloves);
    }
}

/// The most bikes a garage holds (the workshop's wall has 24 hooks, 4A).
pub const MAX_BIKES: usize = 24;

fn first_item() -> u32 {
    1
}

impl Garage {
    /// The six starter bikes, base parts, the default road bike active.
    pub fn starter(rider: &str, active: ProfileId) -> Self {
        Self {
            rider: rider.into(),
            active: active.id().into(),
            bikes: ProfileId::ALL
                .iter()
                .map(|p| Bike {
                    id: p.id().into(),
                    profile: p.id().into(),
                    paint: None,
                    parts: BTreeMap::new(),
                    starter: true,
                    condition: BTreeMap::new(),
                })
                .collect(),
            items: Vec::new(),
            next_item: 1,
            kit: BTreeMap::new(),
            injury: None,
            kit_colours: None,
        }
    }

    /// A new bike id for a frame of `profile`: `<profile>-2`, `-3` … (the
    /// starter bikes have the profile's id).
    pub fn new_bike_id(&self, profile: ProfileId) -> String {
        (2..)
            .map(|n| format!("{}-{n}", profile.id()))
            .find(|id| self.bike(id).is_none())
            .expect("an unused id")
    }

    /// Whether `bike` can be sold (spec 4D): not a starter bike, not the
    /// last bike; the reason otherwise.
    pub fn can_sell_bike(&self, bike: &Bike) -> Result<(), String> {
        if bike.starter {
            return Err(format!(
                "the {} is a starter bike: it cannot be sold",
                bike.name()
            ));
        }
        if self.bikes.len() <= 1 {
            return Err("your last bike cannot be sold".into());
        }
        Ok(())
    }

    /// What selling `bike` gives back: the frameset's and its non-base
    /// parts' refunds (spec 4D; a groupset's own parts are free and go
    /// with it).
    pub fn bike_refund(&self, bike: &Bike) -> u32 {
        bike.parts
            .values()
            .filter_map(|i| self.item(*i))
            .map(Item::refund)
            .sum()
    }

    /// The spare framesets (owned, on no bike): each can hang on a new
    /// hook as a bike (spec 4D).
    pub fn spare_frames<'c>(&self, cat: &'c Catalogue) -> Vec<(u32, &'c Part)> {
        self.items
            .iter()
            .filter(|i| self.bike_of(i.id).is_none())
            .filter_map(|i| cat.get(&i.part).map(|p| (i.id, p)))
            .filter(|(_, p)| p.slot == Slot::Frame && p.profile.is_some())
            .collect()
    }

    /// The rider's condition at `now_unix` (healed over real time).
    pub fn rider_condition(&self, now_unix: i64) -> f64 {
        self.injury.map_or(1.0, |i| i.at(now_unix))
    }

    /// What the next ride on `bike` carries over (spec §14 Phase 12 Q6).
    pub fn carried(&self, bike: &Bike) -> Option<Carried> {
        let rider = self
            .injury
            .map_or(1.0, |i| f64::from(i.condition_pm.min(1000)) / 1000.0);
        let parts = bike.conditions();
        (rider < 1.0 || parts.iter().any(|c| *c < 1.0)).then(|| Carried {
            rider,
            parts,
            at_unix: self.injury.map_or(0, |i| i.at_unix),
        })
    }

    pub fn bike(&self, id: &str) -> Option<&Bike> {
        self.bikes.iter().find(|b| b.id == id)
    }

    fn bike_mut(&mut self, id: &str) -> Result<&mut Bike, String> {
        self.bikes
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("no bike {id} in the garage"))
    }

    pub fn active_bike(&self) -> &Bike {
        self.bike(&self.active).unwrap_or(&self.bikes[0])
    }

    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The bike an item is on, if any.
    pub fn bike_of(&self, item: u32) -> Option<&Bike> {
        self.bikes
            .iter()
            .find(|b| b.parts.values().any(|i| *i == item))
    }

    /// The item on `bike`'s `slot` (`None`: the base part).
    #[cfg(test)]
    pub fn equipped(&self, bike: &str, slot: Slot) -> Option<&Item> {
        let id = *self.bike(bike)?.parts.get(slot.id())?;
        self.item(id)
    }

    /// The catalogue part on `bike`'s `slot`: the equipped item's, else
    /// the base part.
    pub fn part_on<'c>(&self, cat: &'c Catalogue, bike: &Bike, slot: Slot) -> Option<&'c Part> {
        if let Some(p) = bike
            .parts
            .get(slot.id())
            .and_then(|i| self.item(*i))
            .and_then(|i| cat.get(&i.part))
        {
            return Some(p);
        }
        match slot {
            Slot::Frame => cat.base_frame(bike.profile()),
            _ => cat.base(slot, bike.profile().bike()),
        }
    }

    /// The catalogue ids of everything on `bike` that is not a base part.
    pub fn part_ids(&self, bike: &Bike) -> Vec<String> {
        bike.parts
            .values()
            .filter_map(|i| self.item(*i))
            .map(|i| i.part.clone())
            .collect()
    }

    /// Whether item `item` is worn (kit).
    pub fn worn(&self, item: u32) -> bool {
        self.kit.values().any(|i| *i == item)
    }

    /// The kit item worn in `slot`: the worn item's, else the free base
    /// item (spec 4C).
    pub fn kit_on<'c>(&self, cat: &'c Catalogue, slot: Slot) -> Option<&'c Part> {
        self.kit
            .get(slot.id())
            .and_then(|i| self.item(*i))
            .and_then(|i| cat.get(&i.part))
            .or_else(|| cat.base_kit(slot))
    }

    /// The catalogue ids of the worn kit that is not the base kit.
    pub fn kit_ids(&self) -> Vec<String> {
        self.kit
            .values()
            .filter_map(|i| self.item(*i))
            .map(|i| i.part.clone())
            .collect()
    }

    /// Everything the server counts for riding `bike`: its parts and the
    /// worn kit (spec 4C: bike and kit share the build caps).
    pub fn riding_ids(&self, bike: &Bike) -> Vec<String> {
        let mut v = self.part_ids(bike);
        v.extend(self.kit_ids());
        v
    }

    /// The items that came with groupset item `groupset` (spec 4B).
    pub fn with_groupset(&self, groupset: u32) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.with == Some(groupset))
    }

    /// Fits the crankset, cassette and chain of groupset item `gid`
    /// (catalogue part `groupset`) on `bike`: its own items where it has
    /// them (taken from another bike, which gets its base parts back),
    /// new free items otherwise (a groupset bought before W2's fitting, or
    /// one whose parts were lost). The parts they replace go to the spare
    /// parts. Returns the fitted parts' names.
    fn fit_included(
        &mut self,
        cat: &Catalogue,
        bike: &str,
        gid: u32,
        groupset: &Part,
        date: &str,
    ) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for c in cat.included(groupset) {
            let own = self
                .with_groupset(gid)
                .find(|i| i.part == c.id)
                .map(|i| i.id);
            let id = match own {
                Some(id) => id,
                None => {
                    let id = self.next_item;
                    self.next_item += 1;
                    self.items.push(Item {
                        id,
                        part: c.id.clone(),
                        paid: 0,
                        bought: date.into(),
                        ridden_m: 0.0,
                        with: Some(gid),
                    });
                    id
                }
            };
            for b in &mut self.bikes {
                if b.id != bike {
                    b.parts.retain(|_, i| *i != id);
                }
            }
            self.bike_mut(bike)?.parts.insert(c.slot.id().into(), id);
            names.push(c.name.clone());
        }
        Ok(names)
    }

    /// The spare parts (owned, on no bike, not worn; the bench's shelf,
    /// 4D).
    pub fn spares(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .filter(|i| self.bike_of(i.id).is_none() && !self.worn(i.id))
    }
}

/// Whether `part` fits `bike`: the bike type, and a frameset's profile.
pub fn fits(part: &Part, bike: &Bike) -> bool {
    if part.slot.is_kit() {
        // Worn by the rider, on any bike.
        return true;
    }
    let profile = bike.profile();
    if part.slot == Slot::Frame {
        return part.profile == Some(profile);
    }
    part.fits.contains(&profile.bike())
}

/// One line of the hearts ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// UTC time, `YYYY-MM-DDTHH:MM:SSZ`.
    pub t: String,
    pub rider: String,
    pub delta: i64,
    /// start, ride, purchase, sale, grant.
    pub reason: String,
    #[serde(default, rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkshopFile {
    #[serde(default, rename = "rider")]
    riders: Vec<Garage>,
}

/// What a purchase did.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    pub item: u32,
    pub paid: u32,
    pub balance: i64,
    /// A groupset: the names of the crankset, cassette and chain fitted
    /// with it (spec 4B).
    pub fitted: Vec<String>,
}

/// What buying a bike did (spec 4D).
#[derive(Debug, Clone, PartialEq)]
pub struct BikeReceipt {
    /// The new bike's garage id.
    pub bike: String,
    /// The sum paid (frameset and parts).
    pub paid: u32,
    pub balance: i64,
    /// The parts fitted with it (a complete build; with a groupset's
    /// crankset, cassette and chain).
    pub fitted: Vec<String>,
}

/// The wallet: every rider's garage and the ledger.
#[derive(Debug, Default)]
pub struct Wallet {
    /// Training mode (spec 3.2.4 Q4, *Ride locally*): every bike and all
    /// gear are free, no hearts are booked or earned ([`UNLIMITED`]).
    pub training: bool,
    garages: BTreeMap<String, Garage>,
    ledger: Vec<Entry>,
    /// Ledger lines not yet appended to the file.
    unsaved: usize,
    dir: Option<PathBuf>,
    /// The clock (UTC unix seconds) for ledger lines and purchase dates.
    pub now_unix: Option<i64>,
    /// Signed in (spec 14 Phase 4, 4O): the account's workshop. The
    /// balance, the ledger and the garage are the backend's as last
    /// synced; nothing is booked or saved here – every change is an
    /// action the backend checks ([`Wallet::is_account`]).
    pub account: Option<AccountView>,
}

/// The account behind an account wallet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountView {
    pub account: String,
    pub hearts: i64,
    /// The backend answered the last call; offline the workshop is
    /// read-only (Q20 (a)).
    pub online: bool,
    /// When the garage was synced (Unix s).
    pub synced_at: i64,
    /// Ride summaries waiting for the backend.
    pub queued_rides: usize,
    /// Actions sent, not answered yet.
    pub pending: usize,
}

/// The account's garage as the desktop's model (the same fields).
pub fn garage_of(g: &ocw_account_api::workshop::Garage, rider: &str) -> Garage {
    let mut out: Garage = serde_json::to_value(g)
        .and_then(serde_json::from_value)
        .unwrap_or_else(|_| Garage::starter(rider, ProfileId::default()));
    out.rider = rider.into();
    if out.bikes.is_empty() {
        out = Garage::starter(rider, ProfileId::default());
    }
    out
}

/// The account's ledger lines as the local ledger's.
pub fn entries_of(rider: &str, ledger: &[ocw_account_api::workshop::LedgerEntry]) -> Vec<Entry> {
    ledger
        .iter()
        .rev()
        .map(|e| {
            let (y, mo, d, h, mi, s) = ocw_core::timestamp::civil(e.at);
            Entry {
                t: format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
                rider: rider.into(),
                delta: e.delta,
                reason: e.reason.id().into(),
                reference: e.reference.clone(),
            }
        })
        .collect()
}

pub const WORKSHOP_FILE: &str = "workshop.toml";

/// The balance in Training mode: everything is affordable (far from
/// overflowing when prices are added or taken off).
pub const UNLIMITED: i64 = i64::MAX / 4;

/// Training mode's own garage, beside the wallet (`<data dir>/ocw/training/`):
/// what is tried there never mixes with a wallet of earned hearts (Campaign
/// mode, postponed, will ride the wallet itself).
pub const TRAINING_DIR: &str = "training";

/// A balance or price for the rider: hearts, or "free" in Training mode.
pub fn shown(hearts: i64) -> String {
    if hearts >= UNLIMITED / 2 {
        "free (Training mode)".into()
    } else {
        ocw_core::hearts::format(hearts)
    }
}
pub const LEDGER_FILE: &str = "hearts-ledger.jsonl";

/// `<data dir>/ocw` (`OCW_DATA_DIR` overrides it), where the wallet lives.
pub fn default_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::data_dir()?.join("ocw")),
    }
}

impl Wallet {
    /// The account's workshop as synced (spec 14 Phase 4, 4O): `rider`'s
    /// garage is the account's, the balance and ledger the backend's.
    pub fn for_account(
        view: AccountView,
        rider: &str,
        state: &ocw_account_api::workshop::WorkshopState,
    ) -> Self {
        let mut w = Self {
            account: Some(view),
            ..Self::default()
        };
        w.adopt(rider, state);
        w
    }

    /// Takes the backend's workshop (a sync or an action's answer).
    pub fn adopt(&mut self, rider: &str, state: &ocw_account_api::workshop::WorkshopState) {
        self.garages
            .insert(rider.into(), garage_of(&state.garage, rider));
        self.ledger = entries_of(rider, &state.ledger);
        if let Some(a) = &mut self.account {
            a.hearts = state.hearts;
            a.account.clone_from(&state.account);
        }
    }

    /// An action's answer: the garage and the balance it left.
    pub fn adopt_answer(&mut self, rider: &str, a: &ocw_account_api::workshop::ActionAnswer) {
        self.garages
            .insert(rider.into(), garage_of(&a.garage, rider));
        if let Some(v) = &mut self.account {
            v.hearts = a.hearts;
        }
    }

    /// Signed in: the account's workshop (no local booking or saving).
    pub fn is_account(&self) -> bool {
        self.account.is_some()
    }

    /// A wallet that is never saved (tests, no data directory).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Loads the wallet in `dir` (missing files: a new wallet); problems
    /// come back as warnings, never as errors (a broken file must not stop
    /// the app: it is kept aside as `.broken`).
    pub fn load(dir: &Path) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut w = Self {
            dir: Some(dir.to_path_buf()),
            ..Default::default()
        };
        let path = dir.join(WORKSHOP_FILE);
        if let Ok(text) = std::fs::read_to_string(&path) {
            match toml::from_str::<WorkshopFile>(&text) {
                Ok(f) => {
                    for g in f.riders {
                        w.garages.insert(g.rider.clone(), g);
                    }
                }
                Err(e) => {
                    warnings.push(format!("{}: {e} (kept as .broken)", path.display()));
                    let _ = std::fs::rename(&path, path.with_extension("toml.broken"));
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join(LEDGER_FILE)) {
            for (n, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Entry>(line) {
                    Ok(e) => w.ledger.push(e),
                    Err(e) => warnings.push(format!("{LEDGER_FILE} line {}: {e}", n + 1)),
                }
            }
        }
        (w, warnings)
    }

    /// The wallet's clock (UTC unix s): the system's, or the fixed one of
    /// tests and scenes.
    pub fn clock(&self) -> i64 {
        self.now()
    }

    fn now(&self) -> i64 {
        self.now_unix.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        })
    }

    fn stamp(&self) -> String {
        let (y, mo, d, h, mi, s) = ocw_core::timestamp::civil(self.now());
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    fn book(&mut self, rider: &str, delta: i64, reason: &str, reference: String) {
        // Training mode: nothing costs or earns hearts; an account books on
        // the backend.
        if self.training || self.account.is_some() {
            return;
        }
        let t = self.stamp();
        self.ledger.push(Entry {
            t,
            rider: rider.into(),
            delta,
            reason: reason.into(),
            reference,
        });
        self.unsaved += 1;
    }

    /// The rider's garage, created with the starter bikes and the start
    /// hearts the first time (`active`: the bike the rider rides now).
    pub fn garage(&mut self, rider: &str, active: ProfileId) -> &mut Garage {
        if !self.garages.contains_key(rider) {
            self.garages
                .insert(rider.into(), Garage::starter(rider, active));
            if !self.ledger.iter().any(|e| e.rider == rider) {
                self.book(rider, i64::from(START_HEARTS), "start", String::new());
            }
        }
        self.garages.get_mut(rider).expect("just inserted")
    }

    /// The garage, if the rider has one.
    pub fn get(&self, rider: &str) -> Option<&Garage> {
        self.garages.get(rider)
    }

    /// The rider's balance: the sum of the ledger ([`UNLIMITED`] in
    /// Training mode).
    pub fn balance(&self, rider: &str) -> i64 {
        if let Some(a) = &self.account {
            return a.hearts;
        }
        if self.training {
            return UNLIMITED;
        }
        self.ledger
            .iter()
            .filter(|e| e.rider == rider)
            .map(|e| e.delta)
            .sum()
    }

    /// The rider's last `n` ledger lines, newest first.
    pub fn history(&self, rider: &str, n: usize) -> Vec<&Entry> {
        self.ledger
            .iter()
            .rev()
            .filter(|e| e.rider == rider)
            .take(n)
            .collect()
    }

    /// Hearts the server credited for riding (spec §14 Phase 4, 4E).
    pub fn credit(&mut self, rider: &str, hearts: u32, reference: &str) {
        if hearts > 0 {
            self.book(rider, i64::from(hearts), "ride", reference.into());
        }
    }

    /// `hearts.grant` (dev, local server only).
    pub fn grant(&mut self, rider: &str, hearts: i64) {
        self.book(rider, hearts, "grant", String::new());
    }

    /// Buys `part_id` for `bike` and equips it there (the part it replaces
    /// goes to the spare parts).
    pub fn buy(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        bike: &str,
        part_id: &str,
    ) -> Result<Receipt, String> {
        let part = cat
            .get(part_id)
            .ok_or_else(|| format!("no part {part_id} in the catalogue"))?;
        let garage = self
            .garages
            .get(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let b = garage
            .bike(bike)
            .ok_or_else(|| format!("no bike {bike} in the garage"))?;
        if let Some(g) = cat.parts.iter().find(|g| g.includes.contains(&part.id)) {
            return Err(format!(
                "the {} comes with a groupset (e.g. the {}): buy the groupset",
                part.name, g.name
            ));
        }
        if part.hearts == 0 {
            return Err(format!(
                "{} is a base part: it comes with the frame",
                part.name
            ));
        }
        if !fits(part, b) {
            return Err(format!("{} does not fit the {}", part.name, b.name()));
        }
        let balance = self.balance(rider);
        let price = i64::from(part.hearts);
        if balance < price {
            return Err(format!(
                "{} costs {}; the balance is {} ({} more needed)",
                part.name,
                ocw_core::hearts::format(price),
                ocw_core::hearts::format(balance),
                ocw_core::hearts::format(price - balance)
            ));
        }
        let date = self.stamp()[..10].to_string();
        let g = self.garages.get_mut(rider).expect("checked");
        let id = g.next_item;
        g.next_item += 1;
        g.items.push(Item {
            id,
            part: part.id.clone(),
            paid: part.hearts,
            bought: date.clone(),
            ridden_m: 0.0,
            with: None,
        });
        let mut fitted = Vec::new();
        if part.slot.is_kit() {
            g.kit.insert(part.slot.id().into(), id);
        } else {
            g.bike_mut(bike)?.parts.insert(part.slot.id().into(), id);
            fitted = g.fit_included(cat, bike, id, part, &date)?;
        }
        self.book(rider, -price, "purchase", part.id.clone());
        Ok(Receipt {
            item: id,
            paid: part.hearts,
            balance: self.balance(rider),
            fitted,
        })
    }

    /// Puts an owned part on `bike` (from the spares or another bike, which
    /// gets its base part back).
    pub fn equip(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        bike: &str,
        item: u32,
    ) -> Result<String, String> {
        let date = self.stamp()[..10].to_string();
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let it = g
            .item(item)
            .ok_or_else(|| format!("no item {item}"))?
            .clone();
        let part = cat
            .get(&it.part)
            .ok_or_else(|| format!("{} is not in the catalogue", it.part))?;
        if part.slot.is_kit() {
            g.kit.insert(part.slot.id().into(), item);
            return Ok(format!("wearing the {}", part.name));
        }
        let b = g.bike(bike).ok_or_else(|| format!("no bike {bike}"))?;
        if !fits(part, b) {
            return Err(format!("{} does not fit the {}", part.name, b.name()));
        }
        let from = g.bike_of(item).map(|b| b.id.clone());
        let mut said = format!("{} on the {}", part.name, b.name());
        if let Some(from) = from.filter(|f| f != bike) {
            g.bike_mut(&from)?.parts.remove(part.slot.id());
            let name = g.bike(&from).map_or("", |b| b.name());
            said.push_str(&format!("; the {name} gets its base part back"));
        }
        g.bike_mut(bike)?.parts.insert(part.slot.id().into(), item);
        let fitted = g.fit_included(cat, bike, item, part, &date)?;
        if !fitted.is_empty() {
            said.push_str(&format!(", with its {}", fitted.join(", ")));
        }
        Ok(said)
    }

    /// Back to the base part on `slot`; the part goes to the spares.
    pub fn unequip(&mut self, rider: &str, bike: &str, slot: Slot) -> Result<Option<u32>, String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        if slot.is_kit() {
            return Ok(g.kit.remove(slot.id()));
        }
        let off = g.bike_mut(bike)?.parts.remove(slot.id());
        // A groupset takes its crankset, cassette and chain along (those
        // still on the bike; spec 4B).
        if let Some(gid) = off {
            let with: Vec<u32> = g.with_groupset(gid).map(|i| i.id).collect();
            g.bike_mut(bike)?.parts.retain(|_, i| !with.contains(i));
        }
        Ok(off)
    }

    /// Sells an owned part (taken off its bike first): the refund rule.
    pub fn sell(&mut self, rider: &str, item: u32) -> Result<u32, String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let at = g
            .items
            .iter()
            .position(|i| i.id == item)
            .ok_or_else(|| format!("no item {item}"))?;
        if let Some(gid) = g.items[at].with {
            let name = g.item(gid).map_or("groupset".to_string(), |i| {
                format!("groupset (item {})", i.id)
            });
            return Err(format!(
                "item {item} came with a {name}: it is sold with the groupset"
            ));
        }
        let it = g.items.remove(at);
        // A groupset goes with its crankset, cassette and chain (free
        // parts, its price covered them; the refund rule is the
        // groupset's).
        let gone: Vec<u32> = std::iter::once(item)
            .chain(g.with_groupset(item).map(|i| i.id))
            .collect();
        g.items.retain(|i| !gone.contains(&i.id));
        for b in &mut g.bikes {
            b.parts.retain(|_, i| !gone.contains(i));
        }
        g.kit.retain(|_, i| *i != item);
        let refund = it.refund();
        self.book(rider, i64::from(refund), "sale", it.part.clone());
        Ok(refund)
    }

    /// Makes `bike` the one on the stand / ridden.
    pub fn select(&mut self, rider: &str, bike: &str) -> Result<(), String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        g.bike_mut(bike)?;
        g.active = bike.into();
        Ok(())
    }

    /// Paints a bike's frame (free).
    pub fn paint(&mut self, rider: &str, bike: &str, colour: Option<Rgb>) -> Result<(), String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        g.bike_mut(bike)?.paint = colour.map(|c| c.to_string());
        Ok(())
    }

    /// Buys a frameset as a new bike (spec 4D): it comes with the base
    /// parts (entry tier) and hangs on a new hook; `with` are parts for it
    /// bought in the same purchase (a *complete build*, at the sum of the
    /// prices, no discount). Everything is checked before anything is
    /// booked: the frameset must be a paid one, every part must fit the
    /// new bike and be sold alone, at most one per slot, the balance must
    /// cover the sum, and the wall has at most [`MAX_BIKES`] hooks. The
    /// active bike stays on the stand.
    pub fn buy_bike(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        frame: &str,
        with: &[String],
    ) -> Result<BikeReceipt, String> {
        let f = cat
            .get(frame)
            .ok_or_else(|| format!("no part {frame} in the catalogue"))?;
        let Some(profile) = f.profile.filter(|_| f.slot == Slot::Frame) else {
            return Err(format!("{} is not a frameset", f.name));
        };
        if f.hearts == 0 {
            return Err(format!(
                "{} is a base frame: it comes with the starter bike",
                f.name
            ));
        }
        let g = self
            .garages
            .get(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        if g.bikes.len() >= MAX_BIKES {
            return Err(format!(
                "the wall is full ({MAX_BIKES} bikes): sell a bike first"
            ));
        }
        let probe = Bike {
            id: String::new(),
            profile: profile.id().into(),
            paint: None,
            parts: BTreeMap::new(),
            starter: false,
            condition: BTreeMap::new(),
        };
        let mut parts: Vec<&Part> = Vec::new();
        for id in with {
            let p = cat
                .get(id)
                .ok_or_else(|| format!("no part {id} in the catalogue"))?;
            if p.slot == Slot::Frame || p.slot.is_kit() {
                return Err(format!("{} is not a part for the bike", p.name));
            }
            if p.hearts == 0 || cat.is_included(&p.id) {
                return Err(format!("{} is not sold alone", p.name));
            }
            if !fits(p, &probe) {
                return Err(format!("{} does not fit the {}", p.name, profile.name()));
            }
            if parts.iter().any(|q| q.slot == p.slot) {
                return Err(format!("two parts for the {}", p.slot.name()));
            }
            parts.push(p);
        }
        let total: i64 =
            i64::from(f.hearts) + parts.iter().map(|p| i64::from(p.hearts)).sum::<i64>();
        let balance = self.balance(rider);
        if balance < total {
            return Err(format!(
                "the {} costs {}; the balance is {} ({} more needed)",
                if parts.is_empty() {
                    f.name.clone()
                } else {
                    format!("{} with its parts", f.name)
                },
                ocw_core::hearts::format(total),
                ocw_core::hearts::format(balance),
                ocw_core::hearts::format(total - balance)
            ));
        }
        let date = self.stamp()[..10].to_string();
        let g = self.garages.get_mut(rider).expect("checked");
        let id = g.new_bike_id(profile);
        let item = g.next_item;
        g.next_item += 1;
        g.items.push(Item {
            id: item,
            part: f.id.clone(),
            paid: f.hearts,
            bought: date.clone(),
            ridden_m: 0.0,
            with: None,
        });
        g.bikes.push(Bike {
            id: id.clone(),
            parts: BTreeMap::from([(Slot::Frame.id().to_string(), item)]),
            ..probe
        });
        self.book(rider, -i64::from(f.hearts), "purchase", f.id.clone());
        let mut fitted = Vec::new();
        for p in parts {
            let r = self.buy(cat, rider, &id, &p.id)?;
            fitted.push(p.name.clone());
            fitted.extend(r.fitted);
        }
        Ok(BikeReceipt {
            bike: id,
            paid: total as u32,
            balance: self.balance(rider),
            fitted,
        })
    }

    /// Hangs a spare frameset on a new hook as a bike with the base parts
    /// (spec 4D: a frameset taken off a bike is not lost). Returns the new
    /// bike's id.
    pub fn hang_frame(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        item: u32,
    ) -> Result<String, String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let (_, part) = g
            .spare_frames(cat)
            .into_iter()
            .find(|(i, _)| *i == item)
            .ok_or_else(|| format!("item {item} is not a spare frameset"))?;
        if g.bikes.len() >= MAX_BIKES {
            return Err(format!(
                "the wall is full ({MAX_BIKES} bikes): sell a bike first"
            ));
        }
        let profile = part.profile.expect("a frameset");
        let id = g.new_bike_id(profile);
        g.bikes.push(Bike {
            id: id.clone(),
            profile: profile.id().into(),
            paint: None,
            parts: BTreeMap::from([(Slot::Frame.id().to_string(), item)]),
            starter: false,
            condition: BTreeMap::new(),
        });
        Ok(id)
    }

    /// Sells a whole bike (spec 4D): the frameset's and its parts' refunds
    /// (each by its own rule), the hook freed. Not a starter bike, not the
    /// last one. The active bike sold: the first bike goes on the stand.
    pub fn sell_bike(&mut self, rider: &str, bike: &str) -> Result<u32, String> {
        let g = self
            .garages
            .get_mut(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let b = g
            .bike(bike)
            .ok_or_else(|| format!("no bike {bike} in the garage"))?
            .clone();
        g.can_sell_bike(&b)?;
        let refund = g.bike_refund(&b);
        let gone: Vec<u32> = b.parts.values().copied().collect();
        // A groupset's own parts go with it, wherever they are.
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
        self.book(rider, i64::from(refund), "sale", format!("bike:{bike}"));
        Ok(refund)
    }

    /// Keeps the kit's colours in the garage (spec 4C; looks only).
    pub fn set_kit_colours(&mut self, rider: &str, design: &ocw_core::KitDesign) -> bool {
        let Some(g) = self.garages.get_mut(rider) else {
            return false;
        };
        let c = Some(KitColours::of(design));
        let changed = g.kit_colours != c;
        g.kit_colours = c;
        changed
    }

    /// Metres ridden on the rider's active bike: its parts count as
    /// ridden after [`RIDDEN_M`]. True when a part just became ridden
    /// (worth saving).
    pub fn ride(&mut self, rider: &str, metres: f64) -> bool {
        let Some(g) = self.garages.get_mut(rider) else {
            return false;
        };
        if !(metres.is_finite() && metres > 0.0) {
            return false;
        }
        let on: Vec<u32> = g
            .active_bike()
            .parts
            .values()
            .chain(g.kit.values())
            .copied()
            .collect();
        let mut changed = false;
        for it in g.items.iter_mut().filter(|i| on.contains(&i.id)) {
            let was = it.ridden();
            it.ridden_m += metres;
            changed |= !was && it.ridden();
        }
        changed
    }

    /// Keeps the server's damage of a ride on `bike` (spec §14 Phase 12
    /// Q6): the bike's parts and the rider's injury (stamped as `kept`
    /// says). True when something changed (worth saving).
    pub fn keep_damage(&mut self, rider: &str, bike: &str, kept: &Carried) -> bool {
        let Some(g) = self.garages.get_mut(rider) else {
            return false;
        };
        let mut changed = false;
        if let Ok(b) = g.bike_mut(bike) {
            let before = b.condition.clone();
            b.set_conditions(&kept.parts);
            changed |= b.condition != before;
        }
        let pm = damage::per_mille(kept.rider);
        let injury = (pm < 1000).then_some(Injury {
            condition_pm: pm,
            at_unix: kept.at_unix,
        });
        if injury.map(|i| i.condition_pm) != g.injury.map(|i| i.condition_pm) {
            g.injury = injury;
            changed = true;
        }
        changed
    }

    /// Repairs damage part `part` of `bike` (spec §14 Phase 4, W6): pays
    /// its [`repair_quote`] in hearts (ledger reason "repair"); the part is
    /// as new. Returns the hearts paid.
    pub fn repair(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        bike: &str,
        part: DamagePart,
    ) -> Result<u32, String> {
        let g = self
            .garages
            .get(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let b = g
            .bike(bike)
            .ok_or_else(|| format!("no bike {bike} in the garage"))?;
        if b.condition(part) >= 1.0 {
            return Err(format!("the {} needs no repair", part.label()));
        }
        let price = repair_quote(cat, g, b, part);
        let balance = self.balance(rider);
        if balance < i64::from(price) {
            return Err(format!(
                "repairing the {} costs {}; the balance is {} ({} more needed)",
                part.label(),
                ocw_core::hearts::format(i64::from(price)),
                ocw_core::hearts::format(balance),
                ocw_core::hearts::format(i64::from(price) - balance)
            ));
        }
        let g = self.garages.get_mut(rider).expect("checked");
        g.bike_mut(bike)?.condition.remove(part.id());
        self.book(
            rider,
            -i64::from(price),
            "repair",
            format!("{bike}:{}", part.id()),
        );
        Ok(price)
    }

    /// Repairs every damaged part of `bike` at once, if the balance covers
    /// all of them (W6 *Repair all*). Returns (hearts paid, parts).
    pub fn repair_all(
        &mut self,
        cat: &Catalogue,
        rider: &str,
        bike: &str,
    ) -> Result<(u32, usize), String> {
        let g = self
            .garages
            .get(rider)
            .ok_or_else(|| format!("{rider} has no garage"))?;
        let b = g
            .bike(bike)
            .ok_or_else(|| format!("no bike {bike} in the garage"))?;
        let damaged = b.damaged();
        if damaged.is_empty() {
            return Err(format!("the {} needs no repair", b.name()));
        }
        let total: u32 = damaged
            .iter()
            .map(|(p, _)| repair_quote(cat, g, b, *p))
            .sum();
        let balance = self.balance(rider);
        if balance < i64::from(total) {
            return Err(format!(
                "repairing everything costs {}; the balance is {} ({} more needed)",
                ocw_core::hearts::format(i64::from(total)),
                ocw_core::hearts::format(balance),
                ocw_core::hearts::format(i64::from(total) - balance)
            ));
        }
        for (p, _) in &damaged {
            self.repair(cat, rider, bike, *p)?;
        }
        Ok((total, damaged.len()))
    }

    /// Writes `workshop.toml` (replaced whole) and appends the new ledger
    /// lines. Without a directory: nothing.
    pub fn save(&mut self) -> Result<(), String> {
        // The account's workshop lives on the backend.
        if self.account.is_some() {
            return Ok(());
        }
        let Some(dir) = self.dir.clone() else {
            return Ok(());
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = WorkshopFile {
            riders: self.garages.values().cloned().collect(),
        };
        let text = toml::to_string(&file).map_err(|e| e.to_string())?;
        let path = dir.join(WORKSHOP_FILE);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        if self.unsaved > 0 {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(LEDGER_FILE))
                .map_err(|e| format!("{LEDGER_FILE}: {e}"))?;
            let start = self.ledger.len() - self.unsaved;
            for e in &self.ledger[start..] {
                let line = serde_json::to_string(e).map_err(|e| e.to_string())?;
                writeln!(f, "{line}").map_err(|e| format!("{LEDGER_FILE}: {e}"))?;
            }
            self.unsaved = 0;
        }
        Ok(())
    }
}

/// Whether the hearts the server credits go into the local wallet (spec
/// §14 Phase 4, 4E): only from the local server (a remote one has no say
/// in the local wallet before the backend), never in captures (scripted
/// runs) or with `--no-hearts` (the kiosk / show mode). Virtual power,
/// god mode, crashes and bots are left out by the server itself.
pub fn earns(local_server: bool, capture: bool, no_hearts: bool) -> bool {
    local_server && !capture && !no_hearts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> &'static Catalogue {
        ocw_assets::shipped::catalogue()
    }

    fn wallet() -> Wallet {
        let mut w = Wallet::in_memory();
        w.now_unix = Some(1_794_000_000);
        w.garage("Ann", ProfileId::Road);
        w
    }

    /// A priced part that fits the road bike, and its price.
    fn road_part(slot: Slot) -> &'static Part {
        let road = Garage::starter("x", ProfileId::Road);
        cat()
            .parts
            .iter()
            .filter(|p| p.slot == slot && p.hearts > 0)
            .filter(|p| fits(p, road.bike("road").unwrap()))
            .min_by_key(|p| p.hearts)
            .expect("a priced part")
    }

    /// Training mode (spec 3.2.4 Q4, *Ride locally*): every bike and all
    /// gear are available – the dearest part and the dearest frameset of
    /// every type are bought with no hearts, nothing is booked or earned.
    #[test]
    fn training_mode_unlocks_everything() {
        let mut w = Wallet::in_memory();
        w.training = true;
        w.garage("Ada Rider", ProfileId::Road);
        assert_eq!(w.balance("Ada Rider"), UNLIMITED);
        assert_eq!(shown(w.balance("Ada Rider")), "free (Training mode)");
        let dear = cat()
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Wheels)
            .max_by_key(|p| p.hearts)
            .unwrap();
        let road = Garage::starter("x", ProfileId::Road);
        let fitting = |p: &&Part| fits(p, road.bike("road").unwrap());
        let dear_road = cat()
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Wheels && p.hearts > 0)
            .filter(fitting)
            .max_by_key(|p| p.hearts)
            .unwrap();
        assert!(dear.hearts > 500, "a part beyond the start hearts");
        w.buy(cat(), "Ada Rider", "road", &dear_road.id).unwrap();
        // Every frameset (every bike) can be bought, up to the wall's hooks.
        let frames: Vec<&Part> = cat()
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Frame && p.hearts > 0)
            .collect();
        assert!(!frames.is_empty());
        for f in frames.iter().take(MAX_BIKES - 6) {
            w.buy_bike(cat(), "Ada Rider", &f.id, &[])
                .unwrap_or_else(|e| panic!("{}: {e}", f.id));
        }
        // Kit too.
        let kit = cat()
            .parts
            .iter()
            .filter(|p| p.slot.is_kit() && p.hearts > 0)
            .max_by_key(|p| p.hearts)
            .unwrap();
        w.buy(cat(), "Ada Rider", "road", &kit.id).unwrap();
        // No hearts move: nothing booked, riding earns none.
        w.credit("Ada Rider", 40, "ride");
        assert!(w.history("Ada Rider", 10).is_empty());
        assert_eq!(w.balance("Ada Rider"), UNLIMITED);
        // The same wallet outside Training mode keeps its rules.
        let mut h = Wallet::in_memory();
        h.garage("Ada Rider", ProfileId::Road);
        assert!(h.buy(cat(), "Ada Rider", "tt", &dear.id).is_err());
    }

    #[test]
    fn a_new_rider_has_six_starter_bikes_and_500_hearts() {
        let mut w = wallet();
        let g = w.garage("Ann", ProfileId::Road);
        assert_eq!(g.bikes.len(), 6);
        assert!(g.bikes.iter().all(|b| b.starter && b.parts.is_empty()));
        assert_eq!(g.active, "road");
        assert_eq!(w.balance("Ann"), 500);
        // Asking again changes nothing (no second start grant).
        w.garage("Ann", ProfileId::Road);
        assert_eq!(w.balance("Ann"), 500);
        assert_eq!(w.balance("Bob"), 0);
        // Every slot has a part on every starter bike: the base parts.
        let g = w.get("Ann").unwrap();
        for b in &g.bikes {
            for s in Slot::ALL.into_iter().filter(|s| *s != Slot::Paint) {
                let p = g
                    .part_on(cat(), b, s)
                    .unwrap_or_else(|| panic!("{} {s:?}", b.id));
                assert_eq!(p.hearts, 0, "{}", p.id);
            }
        }
    }

    #[test]
    fn buying_needs_the_balance_and_a_fitting_part() {
        let mut w = wallet();
        let tyres = road_part(Slot::Tyres);
        let r = w.buy(cat(), "Ann", "road", &tyres.id).unwrap();
        assert_eq!(r.paid, tyres.hearts);
        assert_eq!(r.balance, 500 - i64::from(tyres.hearts));
        let g = w.get("Ann").unwrap();
        assert_eq!(g.equipped("road", Slot::Tyres).unwrap().part, tyres.id);
        assert_eq!(g.part_ids(g.bike("road").unwrap()), vec![tyres.id.clone()]);
        // Too expensive: refused with the missing amount, nothing booked.
        let dear = cat()
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Wheels)
            .max_by_key(|p| p.hearts)
            .unwrap();
        let before = w.balance("Ann");
        let e = w.buy(cat(), "Ann", "tt", &dear.id).unwrap_err();
        assert!(e.contains("more needed"), "{e}");
        assert_eq!(w.balance("Ann"), before);
        // Not fitting: an MTB tyre on the road bike.
        let mtb = cat()
            .parts
            .iter()
            .find(|p| {
                p.slot == Slot::Tyres
                    && p.hearts > 0
                    && p.fits == vec![ocw_core::BikeType::Mountain]
            })
            .unwrap();
        assert!(
            w.buy(cat(), "Ann", "road", &mtb.id)
                .unwrap_err()
                .contains("does not fit")
        );
        // Base parts are not sold.
        let base = cat().base(Slot::Saddle, ocw_core::BikeType::Road).unwrap();
        assert!(w.buy(cat(), "Ann", "road", &base.id).is_err());
        assert!(w.buy(cat(), "Ann", "road", "nope.nope.top").is_err());
        assert!(w.buy(cat(), "Ann", "unicycle", &tyres.id).is_err());
    }

    #[test]
    fn selling_refunds_in_full_before_the_first_ride_and_half_after() {
        let mut w = wallet();
        w.grant("Ann", 10_000);
        let p = road_part(Slot::Saddle);
        let a = w.buy(cat(), "Ann", "road", &p.id).unwrap().item;
        let before = w.balance("Ann");
        assert_eq!(w.sell("Ann", a).unwrap(), p.hearts);
        assert_eq!(w.balance("Ann"), before + i64::from(p.hearts));
        assert!(
            w.get("Ann")
                .unwrap()
                .equipped("road", Slot::Saddle)
                .is_none(),
            "base again"
        );
        // Ridden: 999 m is not yet a ride, 1 km is.
        let b = w.buy(cat(), "Ann", "road", &p.id).unwrap().item;
        assert!(!w.ride("Ann", 999.0));
        assert_eq!(w.get("Ann").unwrap().item(b).unwrap().refund(), p.hearts);
        assert!(w.ride("Ann", 1.0), "just became ridden");
        assert!(!w.ride("Ann", 5000.0));
        let before = w.balance("Ann");
        assert_eq!(w.sell("Ann", b).unwrap(), p.hearts / 2);
        assert_eq!(w.balance("Ann"), before + i64::from(p.hearts / 2));
        // A spare part on another (not active) bike is not ridden by riding.
        let c = w.buy(cat(), "Ann", "road-aero", &p.id).unwrap().item;
        w.ride("Ann", 10_000.0);
        assert!(!w.get("Ann").unwrap().item(c).unwrap().ridden());
        assert!(w.sell("Ann", 999).is_err());
    }

    #[test]
    fn equip_moves_parts_between_bikes_and_unequip_restores_the_base() {
        let mut w = wallet();
        w.grant("Ann", 10_000);
        let p = road_part(Slot::Pedals);
        let id = w.buy(cat(), "Ann", "road", &p.id).unwrap().item;
        let said = w.equip(cat(), "Ann", "road-aero", id).unwrap();
        assert!(said.contains("gets its base part back"), "{said}");
        let g = w.get("Ann").unwrap();
        assert!(g.equipped("road", Slot::Pedals).is_none());
        assert_eq!(g.equipped("road-aero", Slot::Pedals).unwrap().id, id);
        assert_eq!(
            w.unequip("Ann", "road-aero", Slot::Pedals).unwrap(),
            Some(id)
        );
        let g = w.get("Ann").unwrap();
        assert_eq!(g.spares().map(|i| i.id).collect::<Vec<_>>(), vec![id]);
        // An MTB does not take road pedals.
        assert!(
            w.equip(cat(), "Ann", "mtb", id)
                .unwrap_err()
                .contains("does not fit")
        );
        assert_eq!(w.unequip("Ann", "road", Slot::Pedals).unwrap(), None);
    }

    /// Spec 4B *Groupset and its parts* (open since W2): buying or
    /// equipping a groupset fits its crankset, cassette and chain, the old
    /// ones go to the spares; they go along with it and are sold with it.
    #[test]
    fn a_groupset_is_fitted_with_its_crankset_cassette_and_chain() {
        let mut w = wallet();
        w.grant("Ann", 100_000);
        let slots = [Slot::Crankset, Slot::Cassette, Slot::Chain];
        let power = w
            .buy(cat(), "Ann", "road", "crankset.road.power")
            .unwrap()
            .item;
        let before = w.balance("Ann");
        let top = cat().get("groupset.road-electronic.top").unwrap();
        let r = w
            .buy(cat(), "Ann", "road", "groupset.road-electronic.top")
            .unwrap();
        let gid = r.item;
        assert_eq!(r.fitted.len(), 3, "{:?}", r.fitted);
        assert_eq!(
            w.balance("Ann"),
            before - i64::from(top.hearts),
            "one price"
        );
        let g = w.get("Ann").unwrap();
        let fitted: Vec<String> = slots
            .iter()
            .map(|s| g.equipped("road", *s).unwrap().part.clone())
            .collect();
        assert_eq!(fitted, top.includes);
        for s in slots {
            let it = g.equipped("road", s).unwrap();
            assert_eq!((it.with, it.paid), (Some(gid), 0));
        }
        // The power crankset it replaced is a spare part now.
        assert!(g.spares().any(|i| i.id == power));
        // The physics: the groupset's weight covers its parts.
        let table = ocw_sim::build::PartTable::shipped();
        let b = g.bike("road").unwrap();
        let with = ocw_sim::build::build(table, ProfileId::Road, &g.part_ids(b));
        let alone = ocw_sim::build::build(table, ProfileId::Road, std::slice::from_ref(&top.id));
        assert_eq!(with.params.bike_mass_kg, alone.params.bike_mass_kg);
        // Never bought or sold alone.
        let e = w
            .buy(cat(), "Ann", "road", "crankset.road-race.top")
            .unwrap_err();
        assert!(e.contains("comes with a groupset"), "{e}");
        let crank = w
            .get("Ann")
            .unwrap()
            .equipped("road", Slot::Crankset)
            .unwrap()
            .id;
        let e = w.sell("Ann", crank).unwrap_err();
        assert!(e.contains("sold with the groupset"), "{e}");
        // Changed alone: the power crankset back on, the race one a spare.
        w.equip(cat(), "Ann", "road", power).unwrap();
        let g = w.get("Ann").unwrap();
        assert!(g.spares().any(|i| i.id == crank));
        // To another bike: the groupset takes its parts along (the road
        // bike keeps the power crankset, gets its base cassette and chain).
        let n = w.get("Ann").unwrap().items.len();
        let said = w.equip(cat(), "Ann", "road-aero", gid).unwrap();
        assert!(said.contains("with its"), "{said}");
        let g = w.get("Ann").unwrap();
        assert_eq!(g.items.len(), n, "its own parts, no new ones");
        assert_eq!(g.equipped("road", Slot::Crankset).unwrap().id, power);
        for s in [Slot::Groupset, Slot::Cassette, Slot::Chain] {
            assert!(g.equipped("road", s).is_none(), "{s:?}");
        }
        for s in slots {
            assert_eq!(g.equipped("road-aero", s).unwrap().with, Some(gid));
        }
        // Taken off: its parts come off with it.
        assert_eq!(
            w.unequip("Ann", "road-aero", Slot::Groupset).unwrap(),
            Some(gid)
        );
        let g = w.get("Ann").unwrap();
        assert!(g.bike("road-aero").unwrap().parts.is_empty());
        assert_eq!(g.spares().count(), 4);
        // A groupset bought before the fitting (no parts of its own) gets
        // them when it is equipped.
        let g = w.garages.get_mut("Ann").unwrap();
        g.items.retain(|i| i.with != Some(gid));
        w.equip(cat(), "Ann", "road", gid).unwrap();
        let g = w.get("Ann").unwrap();
        assert_eq!(g.with_groupset(gid).count(), 3);
        assert_eq!(g.equipped("road", Slot::Chain).unwrap().with, Some(gid));
        // Sold: the refund rule is the groupset's (in full, not ridden),
        // its parts go with it; the power crankset stays a spare.
        let before = w.balance("Ann");
        assert_eq!(w.sell("Ann", gid).unwrap(), top.hearts);
        assert_eq!(w.balance("Ann"), before + i64::from(top.hearts));
        let g = w.get("Ann").unwrap();
        assert_eq!(g.with_groupset(gid).count(), 0);
        assert!(g.bike("road").unwrap().parts.is_empty());
        assert_eq!(g.spares().map(|i| i.id).collect::<Vec<_>>(), vec![power]);
        // Other parts and kit are not affected: a tyre buy fits nothing else.
        let r = w.buy(cat(), "Ann", "road", "tyres.road.race").unwrap();
        assert!(r.fitted.is_empty());
    }

    #[test]
    fn a_groupset_parts_owner_survives_the_file() {
        let it = Item {
            id: 7,
            part: "chain.12-speed.mid".into(),
            paid: 0,
            bought: "2026-11-14".into(),
            ridden_m: 0.0,
            with: Some(3),
        };
        let text = toml::to_string(&it).unwrap();
        assert!(text.contains("with = 3"), "{text}");
        assert_eq!(toml::from_str::<Item>(&text).unwrap(), it);
        // Older files have no `with`.
        let old: Item =
            toml::from_str("id = 1\npart = \"lights.set\"\npaid = 80\nbought = \"2026-11-13\"\n")
                .unwrap();
        assert_eq!(old.with, None);
        let plain = Item { with: None, ..it };
        assert!(!toml::to_string(&plain).unwrap().contains("with"));
    }

    #[test]
    fn framesets_fit_only_their_profile() {
        let mut w = wallet();
        w.grant("Ann", 100_000);
        let aero = cat()
            .parts
            .iter()
            .find(|p| {
                p.slot == Slot::Frame && p.hearts > 0 && p.profile == Some(ProfileId::RoadAero)
            })
            .unwrap();
        assert!(w.buy(cat(), "Ann", "road", &aero.id).is_err());
        assert!(w.buy(cat(), "Ann", "road-aero", &aero.id).is_ok());
    }

    #[test]
    fn credits_select_paint_and_history() {
        let mut w = wallet();
        w.credit("Ann", 7, "session 1");
        w.credit("Ann", 0, "nothing");
        assert_eq!(w.balance("Ann"), 507);
        let h = w.history("Ann", 5);
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].reason.as_str(), h[0].delta), ("ride", 7));
        assert_eq!(h[0].t, "2026-11-06T21:20:00Z");
        w.select("Ann", "gravel").unwrap();
        assert_eq!(
            w.get("Ann").unwrap().active_bike().profile(),
            ProfileId::Gravel
        );
        assert!(w.select("Ann", "penny-farthing").is_err());
        w.paint("Ann", "gravel", Some(Rgb(0x43A047))).unwrap();
        assert_eq!(
            w.get("Ann").unwrap().bike("gravel").unwrap().paint(),
            Some(Rgb(0x43A047))
        );
    }

    #[test]
    fn the_wallet_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("ocw-wallet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut w, warn) = Wallet::load(&dir);
        assert!(warn.is_empty());
        w.now_unix = Some(1_794_000_000);
        w.garage("Ann", ProfileId::TimeTrial);
        let p = road_part(Slot::Tyres);
        let item = w.buy(cat(), "Ann", "tt", &p.id).unwrap().item;
        w.paint("Ann", "tt", Some(Rgb(0xFF8F00))).unwrap();
        w.save().unwrap();
        w.credit("Ann", 3, "r");
        w.save().unwrap();
        let (back, warn) = Wallet::load(&dir);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.balance("Ann"), 500 - i64::from(p.hearts) + 3);
        let g = back.get("Ann").unwrap();
        assert_eq!(g, w.get("Ann").unwrap());
        assert_eq!(g.active, "tt");
        assert_eq!(g.equipped("tt", Slot::Tyres).unwrap().id, item);
        // The ledger is append-only: one line per booking.
        let lines = std::fs::read_to_string(dir.join(LEDGER_FILE)).unwrap();
        assert_eq!(lines.lines().count(), 3, "{lines}");
        // A broken file is kept aside, the wallet starts fresh.
        std::fs::write(dir.join(WORKSHOP_FILE), "rider = [[[").unwrap();
        let (fresh, warn) = Wallet::load(&dir);
        assert_eq!(warn.len(), 1);
        assert!(fresh.get("Ann").is_none());
        assert!(dir.join("workshop.toml.broken").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn hurt_ride() -> Carried {
        let mut parts = [1.0; PARTS];
        parts[DamagePart::RearWheel.index()] = 0.6;
        parts[DamagePart::Cockpit.index()] = 0.5;
        Carried {
            rider: 0.45,
            parts,
            at_unix: 1_794_000_000,
        }
    }

    /// Phase 12 Q6: the bike's parts and the rider's injury survive a
    /// restart of the app (the file round trip); the injury heals over
    /// real time.
    #[test]
    fn damage_persists_across_a_restart_and_heals_over_real_time() {
        let dir = std::env::temp_dir().join(format!("ocw-wallet-damage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut w, _) = Wallet::load(&dir);
        w.now_unix = Some(1_794_000_000);
        w.garage("Ann", ProfileId::Road);
        assert!(
            w.get("Ann")
                .unwrap()
                .carried(w.get("Ann").unwrap().active_bike())
                .is_none()
        );
        assert!(w.keep_damage("Ann", "road", &hurt_ride()));
        assert!(!w.keep_damage("Ann", "road", &hurt_ride()), "nothing new");
        w.save().unwrap();
        let (back, warn) = Wallet::load(&dir);
        assert!(warn.is_empty(), "{warn:?}");
        let g = back.get("Ann").unwrap();
        assert_eq!(g, w.get("Ann").unwrap());
        let bike = g.bike("road").unwrap();
        assert_eq!(bike.condition(DamagePart::RearWheel), 0.6);
        assert_eq!(bike.condition(DamagePart::Frame), 1.0);
        let c = g.carried(bike).unwrap();
        assert_eq!(c.rider, 0.45);
        // 3 h later the injury is 30 % better; the bike is not.
        let (rider, parts) = c.at(1_794_000_000 + 3 * 3600);
        assert!((rider - 0.75).abs() < 1e-9);
        assert_eq!(parts[DamagePart::Cockpit.index()], 0.5);
        assert!((g.rider_condition(1_794_000_000 + 3 * 3600) - 0.75).abs() < 1e-9);
        // The other bikes keep no damage.
        assert!(g.bike("tt").unwrap().damaged().is_empty());
        let text = std::fs::read_to_string(dir.join(WORKSHOP_FILE)).unwrap();
        assert!(text.contains("rear_wheel = 600"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// W6: a repair costs `ceil(price × (1 − condition) × 0.3)` hearts; base
    /// parts are priced at the cheapest paid part of the slot; Repair all
    /// needs the whole price.
    #[test]
    fn repairs_are_paid_in_hearts() {
        let mut w = wallet();
        w.keep_damage("Ann", "road", &hurt_ride());
        let g = w.get("Ann").unwrap();
        let bike = g.bike("road").unwrap();
        // The cockpit is a base part: priced at the cheapest paid cockpit.
        let cheapest = road_part(Slot::Cockpit).hearts;
        assert_eq!(slot_price(cat(), g, bike, DamagePart::Cockpit), cheapest);
        let want = damage::repair_price(cheapest, 0.5, DamagePart::Cockpit);
        assert_eq!(repair_quote(cat(), g, bike, DamagePart::Cockpit), want);
        assert_eq!(want, (u64::from(cheapest) * 15).div_ceil(100) as u32);
        // Wheels are a pair: half the wheelset's price.
        let wheels = road_part(Slot::Wheels).hearts;
        assert_eq!(
            slot_price(cat(), g, bike, DamagePart::RearWheel),
            wheels.div_ceil(2)
        );
        assert_eq!(repair_quote(cat(), g, bike, DamagePart::Frame), 0);
        let before = w.balance("Ann");
        let paid = w.repair(cat(), "Ann", "road", DamagePart::Cockpit).unwrap();
        assert_eq!(paid, want);
        assert_eq!(w.balance("Ann"), before - i64::from(paid));
        assert_eq!(w.history("Ann", 1)[0].reason, "repair");
        assert!(
            w.repair(cat(), "Ann", "road", DamagePart::Cockpit).is_err(),
            "as new"
        );
        // A bought part is priced at what was paid for it.
        w.grant("Ann", 5000);
        let item = w
            .buy(cat(), "Ann", "road", &road_part(Slot::Groupset).id)
            .unwrap();
        let mut ride = hurt_ride();
        ride.parts[DamagePart::Drivetrain.index()] = 0.4;
        w.keep_damage("Ann", "road", &ride);
        let g = w.get("Ann").unwrap();
        let bike = g.bike("road").unwrap();
        assert_eq!(
            slot_price(cat(), g, bike, DamagePart::Drivetrain),
            item.paid
        );
        // Repair all: everything or nothing.
        w.grant("Ann", -(w.balance("Ann") - 1));
        assert!(w.repair_all(cat(), "Ann", "road").is_err());
        assert_eq!(
            w.get("Ann").unwrap().bike("road").unwrap().damaged().len(),
            3
        );
        w.grant("Ann", 5000);
        let (total, n) = w.repair_all(cat(), "Ann", "road").unwrap();
        assert_eq!(n, 3);
        assert!(total > 0);
        assert!(
            w.get("Ann")
                .unwrap()
                .bike("road")
                .unwrap()
                .damaged()
                .is_empty()
        );
        // Beyond repair: a frame below 10 % is replaced at the full price.
        let mut wreck = hurt_ride();
        wreck.parts[DamagePart::Frame.index()] = 0.05;
        w.keep_damage("Ann", "road", &wreck);
        let g = w.get("Ann").unwrap();
        let bike = g.bike("road").unwrap();
        assert_eq!(
            repair_quote(cat(), g, bike, DamagePart::Frame),
            slot_price(cat(), g, bike, DamagePart::Frame)
        );
    }

    #[test]
    fn hearts_go_to_the_wallet_only_from_the_local_server_and_real_rides() {
        assert!(earns(true, false, false));
        assert!(!earns(false, false, false), "a remote server");
        assert!(!earns(true, true, false), "a capture");
        assert!(!earns(true, false, true), "kiosk / show mode (--no-hearts)");
    }

    /// The kit (spec 4C, W4): bought with hearts and worn at once (on
    /// every bike), taken off back to the free base item, sold by the same
    /// refund rules; the worn kit goes to the server with the bike.
    #[test]
    fn kit_is_bought_worn_taken_off_and_sold_like_parts() {
        let mut w = wallet();
        let g = w.get("Ann").unwrap();
        for slot in Slot::KIT {
            assert!(
                g.kit_on(cat(), slot).unwrap().is_base(),
                "{slot:?}: the base kit"
            );
        }
        assert!(g.kit_ids().is_empty());
        // Too dear: the TT helmet costs 500 ♥, the start balance is 500 ♥.
        w.grant("Ann", -1);
        let e = w.buy(cat(), "Ann", "road", "helmet.tt.top").unwrap_err();
        assert!(
            e.contains("1 more needed") || e.contains("more needed"),
            "{e}"
        );
        w.grant("Ann", 1001);
        let r = w.buy(cat(), "Ann", "road", "helmet.tt.top").unwrap();
        assert_eq!(r.paid, 500);
        assert_eq!(w.balance("Ann"), 1000);
        let g = w.get("Ann").unwrap();
        assert_eq!(g.kit_on(cat(), Slot::Helmet).unwrap().id, "helmet.tt.top");
        assert!(g.worn(r.item) && g.bike_of(r.item).is_none());
        // Worn on every bike: the gravel bike rides with it too.
        let gravel = g.bike("gravel").unwrap();
        assert!(g.riding_ids(gravel).contains(&"helmet.tt.top".to_string()));
        assert!(g.part_ids(gravel).is_empty(), "not a bike part");
        // A base item is free and cannot be bought.
        assert!(w.buy(cat(), "Ann", "road", "helmet.road.base").is_err());
        // Taken off: the base helmet again, the TT helmet a spare.
        assert_eq!(
            w.unequip("Ann", "road", Slot::Helmet).unwrap(),
            Some(r.item)
        );
        let g = w.get("Ann").unwrap();
        assert!(g.kit_on(cat(), Slot::Helmet).unwrap().is_base());
        assert!(g.spares().any(|i| i.id == r.item));
        w.equip(cat(), "Ann", "mtb", r.item).unwrap();
        assert!(w.get("Ann").unwrap().worn(r.item));
        // Sold before the first ride: in full; ridden: half (floor).
        let cap = w.buy(cat(), "Ann", "road", "cap.cotton.entry").unwrap();
        assert!(w.ride("Ann", 1500.0), "worn kit gets ridden");
        assert_eq!(w.sell("Ann", cap.item).unwrap(), 15);
        let shoes = w.buy(cat(), "Ann", "road", "shoes.overshoes.mid").unwrap();
        assert_eq!(w.sell("Ann", shoes.item).unwrap(), 70);
        let g = w.get("Ann").unwrap();
        assert!(g.kit_on(cat(), Slot::Cap).unwrap().is_base());
        assert!(g.kit_on(cat(), Slot::Shoes).unwrap().is_base());
        // The kit survives a save and a load.
        let dir = std::env::temp_dir().join(format!("ocw-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut w2, _) = Wallet::load(&dir);
        w2.now_unix = Some(1_794_000_000);
        w2.garage("Bo", ProfileId::Road);
        let it = w2.buy(cat(), "Bo", "road", "cap.cotton.entry").unwrap();
        w2.save().unwrap();
        let (w3, warnings) = Wallet::load(&dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(w3.get("Bo").unwrap().worn(it.item));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The bench (spec 4D): a frameset is bought as a new bike with the
    /// base parts on a new hook; it needs the balance, a paid frameset and
    /// room on the wall; the stand keeps its bike.
    #[test]
    fn a_frameset_is_bought_as_a_new_bike_with_the_base_parts() {
        let mut w = wallet();
        // 500 ♥ do not buy a frameset: nothing is booked.
        let e = w
            .buy_bike(cat(), "Ann", "frame.road.upper", &[])
            .unwrap_err();
        assert!(e.contains("more needed"), "{e}");
        assert_eq!(w.balance("Ann"), 500);
        assert_eq!(w.get("Ann").unwrap().bikes.len(), 6);
        // Base frames and other parts are not framesets for sale.
        assert!(w.buy_bike(cat(), "Ann", "frame.road.base", &[]).is_err());
        assert!(w.buy_bike(cat(), "Ann", "tyres.road.race", &[]).is_err());
        w.grant("Ann", 3_000);
        let r = w.buy_bike(cat(), "Ann", "frame.road.upper", &[]).unwrap();
        assert_eq!((r.bike.as_str(), r.paid), ("road-2", 2_839));
        assert_eq!(w.balance("Ann"), 3_500 - 2_839);
        let g = w.get("Ann").unwrap();
        assert_eq!(g.bikes.len(), 7);
        assert_eq!(g.active, "road", "the stand keeps its bike");
        let b = g.bike("road-2").unwrap();
        assert!(!b.starter);
        assert_eq!(b.profile(), ProfileId::Road);
        assert_eq!(
            g.part_on(cat(), b, Slot::Frame).unwrap().id,
            "frame.road.upper"
        );
        // Everything else is the base part (entry tier).
        for slot in [Slot::Wheels, Slot::Tyres, Slot::Groupset, Slot::Saddle] {
            assert!(g.part_on(cat(), b, slot).unwrap().is_base(), "{slot:?}");
        }
        assert_eq!(g.part_ids(b), vec!["frame.road.upper".to_string()]);
        // It rides the frame's build (the server's physics, 4G).
        assert_eq!(g.riding_ids(b), vec!["frame.road.upper".to_string()]);
        // One ledger line per part bought.
        assert_eq!(w.history("Ann", 1)[0].reference, "frame.road.upper");
        // Another of the same profile: the next id.
        w.grant("Ann", 1_390);
        let r = w.buy_bike(cat(), "Ann", "frame.road.mid", &[]).unwrap();
        assert_eq!(r.bike, "road-3");
        // The wall holds 24 bikes.
        w.grant("Ann", 1_390 * 20);
        for _ in 0..(MAX_BIKES - 8) {
            w.buy_bike(cat(), "Ann", "frame.road.mid", &[]).unwrap();
        }
        assert_eq!(w.get("Ann").unwrap().bikes.len(), MAX_BIKES);
        let before = w.balance("Ann");
        let e = w.buy_bike(cat(), "Ann", "frame.road.mid", &[]).unwrap_err();
        assert!(e.contains("full"), "{e}");
        assert_eq!(w.balance("Ann"), before);
    }

    /// A complete build (spec 4D): the frameset and its parts in one
    /// purchase at the sum of the prices; a part that does not fit, is not
    /// sold alone or doubles a slot stops it before anything is booked.
    #[test]
    fn a_complete_build_buys_the_frameset_and_its_parts_at_their_sum() {
        let mut w = wallet();
        w.grant("Ann", 10_000);
        let with = vec![
            "groupset.road-mech.mid".to_string(),
            "wheels.road-alloy.mid".to_string(),
        ];
        let before = w.balance("Ann");
        for bad in [
            vec!["groupset.mtb.mid".to_string()],
            vec!["chain.12-speed.mid".to_string()],
            vec![
                "wheels.road-alloy.mid".into(),
                "wheels.road-carbon.mid".into(),
            ],
            vec!["helmet.tt.top".to_string()],
        ] {
            assert!(
                w.buy_bike(cat(), "Ann", "frame.road.upper", &bad).is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(w.balance("Ann"), before);
        assert_eq!(w.get("Ann").unwrap().bikes.len(), 6);
        let r = w.buy_bike(cat(), "Ann", "frame.road.upper", &with).unwrap();
        let sum = 2_839 + 1_441 + 600;
        assert_eq!(r.paid, sum, "no discount");
        assert_eq!(w.balance("Ann"), before - i64::from(sum));
        let g = w.get("Ann").unwrap();
        let b = g.bike(&r.bike).unwrap();
        assert_eq!(
            g.part_on(cat(), b, Slot::Groupset).unwrap().id,
            "groupset.road-mech.mid"
        );
        assert_eq!(
            g.part_on(cat(), b, Slot::Wheels).unwrap().id,
            "wheels.road-alloy.mid"
        );
        // The groupset came with its crankset, cassette and chain (4B).
        assert!(r.fitted.len() > 2, "{:?}", r.fitted);
        assert!(!g.part_on(cat(), b, Slot::Chain).unwrap().is_base());
    }

    /// Selling a whole bike (spec 4D): the frameset's and its non-base
    /// parts' refunds by their rules, the hook freed; never a starter bike
    /// or the last one; the stand gets another bike when its own is sold.
    #[test]
    fn a_bike_is_sold_whole_but_never_a_starter_or_the_last() {
        let mut w = wallet();
        w.grant("Ann", 10_000);
        assert!(
            w.sell_bike("Ann", "road")
                .unwrap_err()
                .contains("starter bike")
        );
        let r = w
            .buy_bike(
                cat(),
                "Ann",
                "frame.road.upper",
                &["groupset.road-mech.mid".to_string()],
            )
            .unwrap();
        w.select("Ann", &r.bike).unwrap();
        // Ridden: the frameset and the groupset give half back.
        assert!(w.ride("Ann", 2_000.0));
        let tyres = w.buy(cat(), "Ann", &r.bike, "tyres.road.race").unwrap();
        let g = w.get("Ann").unwrap();
        let b = g.bike(&r.bike).unwrap();
        let refund = g.bike_refund(b);
        assert_eq!(refund, 2_839 / 2 + 1_441 / 2 + 190, "{refund}");
        let before = w.balance("Ann");
        assert_eq!(w.sell_bike("Ann", &r.bike).unwrap(), refund);
        assert_eq!(w.balance("Ann"), before + i64::from(refund));
        let g = w.get("Ann").unwrap();
        assert!(g.bike(&r.bike).is_none());
        assert_eq!(g.bikes.len(), 6);
        assert_eq!(g.active, "road", "the first bike on the stand");
        assert!(g.item(tyres.item).is_none(), "its parts went with it");
        assert!(
            g.items.iter().all(|i| i.part != "chain.12-speed.mid"),
            "the groupset's own parts too"
        );
        assert_eq!(w.history("Ann", 1)[0].reference, format!("bike:{}", r.bike));
        // The last bike cannot be sold.
        let mut g = Garage::starter("Cy", ProfileId::Road);
        g.bikes.truncate(1);
        g.bikes[0].starter = false;
        assert!(g.can_sell_bike(&g.bikes[0]).unwrap_err().contains("last"));
    }

    /// A spare frameset (taken off a bike) hangs on a new hook as a bike.
    #[test]
    fn a_spare_frameset_hangs_on_a_new_hook() {
        let mut w = wallet();
        w.grant("Ann", 10_000);
        let r = w.buy(cat(), "Ann", "road", "frame.road.mid").unwrap();
        w.unequip("Ann", "road", Slot::Frame).unwrap();
        let g = w.get("Ann").unwrap();
        assert_eq!(g.spare_frames(cat()).len(), 1);
        let id = w.hang_frame(cat(), "Ann", r.item).unwrap();
        let g = w.get("Ann").unwrap();
        assert_eq!(id, "road-2");
        assert_eq!(g.bike_of(r.item).unwrap().id, id);
        assert!(g.spare_frames(cat()).is_empty());
        assert!(w.hang_frame(cat(), "Ann", r.item).is_err(), "not spare now");
    }

    /// The kit's colours (spec 4C): kept in `workshop.toml` with the
    /// garage, across a restart; looks only – the bike's and kit's
    /// physics ids stay the same.
    #[test]
    fn kit_colours_persist_in_the_garage_and_never_change_the_physics() {
        let dir = std::env::temp_dir().join(format!("ocw-kit-colours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut w, _) = Wallet::load(&dir);
        w.now_unix = Some(1_794_000_000);
        w.garage("Ann", ProfileId::Road);
        let g = w.get("Ann").unwrap();
        let ids = g.riding_ids(g.active_bike());
        let build = ocw_sim::build::build(ocw_sim::PartTable::shipped(), ProfileId::Road, &ids);
        assert!(g.kit_colours.is_none());
        let d = ocw_core::KitDesign {
            shorts: Some(Rgb(0x3949AB)),
            shoes: Some(Rgb(0xF2F2EE)),
            gloves: Some(Rgb(0xE53935)),
            ..Default::default()
        };
        assert!(w.set_kit_colours("Ann", &d));
        assert!(!w.set_kit_colours("Ann", &d), "no change the second time");
        w.save().unwrap();
        let text = std::fs::read_to_string(dir.join(WORKSHOP_FILE)).unwrap();
        assert!(text.contains("gloves = \"#E53935\""), "{text}");
        let (w2, warnings) = Wallet::load(&dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        let g = w2.get("Ann").unwrap();
        let mut back = ocw_core::KitDesign::default();
        g.kit_colours.as_ref().unwrap().apply(&mut back);
        assert_eq!(
            (back.shorts, back.socks, back.shoes, back.gloves),
            (d.shorts, None, d.shoes, d.gloves)
        );
        // Looks only: the same ids, the same build.
        assert_eq!(g.riding_ids(g.active_bike()), ids);
        assert_eq!(
            ocw_sim::build::build(ocw_sim::PartTable::shipped(), ProfileId::Road, &ids),
            build
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
