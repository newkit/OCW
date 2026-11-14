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
//! The rider's kit (spec 4C, W4) lives in the same garage: `kit` maps a
//! kit slot to the worn item (a slot without one wears its free base
//! item); kit items are bought, worn, taken off and sold by the same rules
//! (they count as ridden after 1 km in the saddle), on any bike. The files are trivially editable – accepted, they only
//! matter on the local server until the Phase 5 backend.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ocw_assets::parts::{Catalogue, Part, Slot};
use ocw_core::hearts::START_HEARTS;
use ocw_core::{ProfileId, Rgb};
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
}

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
                })
                .collect(),
            items: Vec::new(),
            next_item: 1,
            kit: BTreeMap::new(),
        }
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
    #[cfg(test)]
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

    /// The spare parts (owned, on no bike; the bench's shelf, 4D).
    #[cfg(test)]
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
}

/// The wallet: every rider's garage and the ledger.
#[derive(Debug, Default)]
pub struct Wallet {
    garages: BTreeMap<String, Garage>,
    ledger: Vec<Entry>,
    /// Ledger lines not yet appended to the file.
    unsaved: usize,
    dir: Option<PathBuf>,
    /// The clock (UTC unix seconds) for ledger lines and purchase dates.
    pub now_unix: Option<i64>,
}

pub const WORKSHOP_FILE: &str = "workshop.toml";
pub const LEDGER_FILE: &str = "hearts-ledger.jsonl";

/// `<data dir>/ocw` (`OCW_DATA_DIR` overrides it), where the wallet lives.
pub fn default_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::data_dir()?.join("ocw")),
    }
}

impl Wallet {
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

    /// The rider's balance: the sum of the ledger.
    pub fn balance(&self, rider: &str) -> i64 {
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
            bought: date,
            ridden_m: 0.0,
        });
        if part.slot.is_kit() {
            g.kit.insert(part.slot.id().into(), id);
        } else {
            g.bike_mut(bike)?.parts.insert(part.slot.id().into(), id);
        }
        self.book(rider, -price, "purchase", part.id.clone());
        Ok(Receipt {
            item: id,
            paid: part.hearts,
            balance: self.balance(rider),
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
        Ok(g.bike_mut(bike)?.parts.remove(slot.id()))
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
        let it = g.items.remove(at);
        for b in &mut g.bikes {
            b.parts.retain(|_, i| *i != item);
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

    /// Writes `workshop.toml` (replaced whole) and appends the new ledger
    /// lines. Without a directory: nothing.
    pub fn save(&mut self) -> Result<(), String> {
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
}
