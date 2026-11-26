//! The workshop of an account (4O, W5c): the garage document, the hearts
//! ledger and the actions.
//!
//! The garage has the same field names as the desktop's local garage
//! (`apps/desktop/src/workshop/wallet.rs`: `bike`, `item`, `kit`, …), so
//! the app reads the account's garage into the same model and draws the
//! workshop unchanged; the backend is the truth and every change is an
//! action it checks against the parts catalogue's prices.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A ride of this length with a part on the bike makes it "ridden" (m):
/// selling it then refunds half the price paid (4D).
pub const RIDDEN_M: f64 = 1000.0;

/// The most bikes a garage holds (the workshop's wall has 24 hooks, 4A).
pub const MAX_BIKES: usize = 24;

/// A part or kit item the account owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    /// The catalogue id (`assets/parts.toml`).
    pub part: String,
    /// Hearts paid (refunds use it, never today's price).
    pub paid: u32,
    /// When it was bought (`YYYY-MM-DD`).
    pub bought: String,
    /// Metres ridden with it on the bike (or worn).
    #[serde(default)]
    pub ridden_m: f64,
    /// The groupset item this part came with (crankset, cassette, chain;
    /// 4B).
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

/// A bike on the wall: a frame of a physics profile with its parts. A slot
/// without an entry has the frame's free base part (Q18 (a)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bike {
    pub id: String,
    /// The frame's physics profile id (`road`, `gravel`, …).
    pub profile: String,
    /// Frame paint (`#RRGGBB`), free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<String>,
    /// Slot id → item id.
    #[serde(default)]
    pub parts: BTreeMap<String, u32>,
    /// The free starter bike (cannot be sold, Q16).
    #[serde(default)]
    pub starter: bool,
    /// Damage carried over: damage part id → condition per mille.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub condition: BTreeMap<String, u32>,
}

/// An account's garage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Garage {
    /// The rider's name (shown in the workshop).
    #[serde(default)]
    pub rider: String,
    /// The bike on the stand / ridden.
    pub active: String,
    #[serde(default, rename = "bike")]
    pub bikes: Vec<Bike>,
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
    #[serde(default = "first_item")]
    pub next_item: u32,
    /// Kit slot id → worn item id; a slot without one wears its free base
    /// item.
    #[serde(default)]
    pub kit: BTreeMap<String, u32>,
}

fn first_item() -> u32 {
    1
}

/// The id of the free starter bike every account gets.
pub const STARTER_BIKE: &str = "starter";

/// The profile of the starter bike. Q16 decided an old, rusty city bike;
/// until it has a model and a physics profile of its own, the starter is
/// the plain road bike with its base parts.
pub const STARTER_PROFILE: &str = "road";

impl Garage {
    /// A new account's garage (Q16–Q18, Q21 (a)): nothing bought – the free
    /// starter bike with its base parts on the stand, the base kit worn.
    pub fn starter(rider: &str) -> Self {
        Self {
            rider: rider.into(),
            active: STARTER_BIKE.into(),
            bikes: vec![Bike {
                id: STARTER_BIKE.into(),
                profile: STARTER_PROFILE.into(),
                paint: None,
                parts: BTreeMap::new(),
                starter: true,
                condition: BTreeMap::new(),
            }],
            items: Vec::new(),
            next_item: 1,
            kit: BTreeMap::new(),
        }
    }

    pub fn bike(&self, id: &str) -> Option<&Bike> {
        self.bikes.iter().find(|b| b.id == id)
    }

    pub fn bike_mut(&mut self, id: &str) -> Option<&mut Bike> {
        self.bikes.iter_mut().find(|b| b.id == id)
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

    pub fn worn(&self, item: u32) -> bool {
        self.kit.values().any(|i| *i == item)
    }

    /// The items that came with groupset item `groupset`.
    pub fn with_groupset(&self, groupset: u32) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.with == Some(groupset))
    }

    /// Metres ridden on `bike` (the active one if `None` or unknown): its
    /// parts and the worn kit count as ridden after [`RIDDEN_M`].
    pub fn ride(&mut self, bike: Option<&str>, metres: f64) {
        if !(metres.is_finite() && metres > 0.0) {
            return;
        }
        let b = bike
            .and_then(|id| self.bike(id))
            .or_else(|| self.bike(&self.active))
            .or(self.bikes.first());
        let Some(b) = b else { return };
        let on: Vec<u32> = b.parts.values().chain(self.kit.values()).copied().collect();
        for it in self.items.iter_mut().filter(|i| on.contains(&i.id)) {
            it.ridden_m += metres;
        }
    }
}

/// Why the balance changed (4O `hearts_ledger.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// The 5,000 ♥ of a new account.
    Start,
    Ride,
    Workout,
    Event,
    Achievement,
    Purchase,
    Sale,
    Repair,
    Grant,
}

impl Reason {
    pub const ALL: [Reason; 9] = [
        Self::Start,
        Self::Ride,
        Self::Workout,
        Self::Event,
        Self::Achievement,
        Self::Purchase,
        Self::Sale,
        Self::Repair,
        Self::Grant,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Ride => "ride",
            Self::Workout => "workout",
            Self::Event => "event",
            Self::Achievement => "achievement",
            Self::Purchase => "purchase",
            Self::Sale => "sale",
            Self::Repair => "repair",
            Self::Grant => "grant",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == s)
    }
}

/// One line of the account's hearts ledger (append-only; the balance is
/// the sum).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Unix seconds.
    pub at: i64,
    pub delta: i64,
    pub reason: Reason,
    /// The part bought or sold, the ride's id, `bike:<id>` …
    #[serde(default)]
    pub reference: String,
}

/// `GET /workshop`: the account's workshop as it is now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkshopState {
    pub account: String,
    pub hearts: i64,
    pub garage: Garage,
    /// The newest ledger lines, newest first.
    #[serde(default)]
    pub ledger: Vec<LedgerEntry>,
    /// The backend's time of this answer (Unix s).
    pub at: i64,
}

/// A workshop action (the request body of `POST /workshop/purchase`,
/// `/workshop/sell` and `/workshop/garage`, with an `Idempotency-Key`
/// header: a repeated request with the same key gets the first answer and
/// changes nothing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Buys `part` for `bike` and fits it (the part it replaces goes to
    /// the spares); a kit item is worn at once.
    BuyPart { bike: String, part: String },
    /// Buys a frameset as a new bike with its base parts (Q18 (a)) and
    /// `with` parts for it in the same purchase.
    BuyBike {
        frame: String,
        #[serde(default)]
        with: Vec<String>,
    },
    /// Sells an item (by the refund rule).
    Sell { item: u32 },
    /// Sells a whole bike with its parts.
    SellBike { bike: String },
    /// Puts an owned item on `bike` (or wears it).
    Equip { bike: String, item: u32 },
    /// Back to the base part on `slot`.
    Unequip { bike: String, slot: String },
    /// The bike on the stand.
    Select { bike: String },
    /// Frame paint (free).
    Paint {
        bike: String,
        #[serde(default)]
        colour: Option<String>,
    },
}

/// Which endpoint takes an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Purchase,
    Sell,
    Garage,
}

impl Endpoint {
    pub fn path(self) -> &'static str {
        match self {
            Self::Purchase => "/workshop/purchase",
            Self::Sell => "/workshop/sell",
            Self::Garage => "/workshop/garage",
        }
    }
}

impl Action {
    pub fn endpoint(&self) -> Endpoint {
        match self {
            Self::BuyPart { .. } | Self::BuyBike { .. } => Endpoint::Purchase,
            Self::Sell { .. } | Self::SellBike { .. } => Endpoint::Sell,
            _ => Endpoint::Garage,
        }
    }

    /// Whether it changes the balance (refused offline, Q20 (a): the
    /// synced garage is read-only).
    pub fn moves_hearts(&self) -> bool {
        self.endpoint() != Endpoint::Garage
    }
}

/// The answer to an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionAnswer {
    /// What happened, for the rider ("Bought the … for 1,390 ♥").
    pub message: String,
    pub hearts: i64,
    pub garage: Garage,
    /// The new item (a purchase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<u32>,
    /// The new bike (a bike purchase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bike: Option<String>,
    #[serde(default)]
    pub paid: u32,
    #[serde(default)]
    pub refund: u32,
    /// The parts fitted with it (a groupset's crankset, cassette, chain).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fitted: Vec<String>,
}

/// An error answer of the workshop and rides endpoints (the shape of the
/// OAuth error answers of the other endpoints): `error` a code
/// (`insufficient_hearts`, `refused`, `not_found`, `offline` …),
/// `error_description` the sentence for the rider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_garage_owns_nothing() {
        let g = Garage::starter("Ada Rider");
        assert_eq!(g.bikes.len(), 1);
        assert!(g.bikes[0].starter && g.bikes[0].parts.is_empty());
        assert!(g.items.is_empty() && g.kit.is_empty());
        assert_eq!(g.active, STARTER_BIKE);
    }

    #[test]
    fn refund_rule_and_riding() {
        let mut g = Garage::starter("Ada Rider");
        g.items.push(Item {
            id: 1,
            part: "wheels.road-alloy.entry".into(),
            paid: 299,
            bought: "2026-11-26".into(),
            ridden_m: 0.0,
            with: None,
        });
        g.bikes[0].parts.insert("wheels".into(), 1);
        assert_eq!(g.items[0].refund(), 299);
        g.ride(None, 999.0);
        assert_eq!(g.items[0].refund(), 299);
        g.ride(Some("no-such-bike"), 1.0);
        assert_eq!(g.items[0].refund(), 149);
        g.ride(None, f64::NAN);
        assert_eq!(g.items[0].ridden_m, 1000.0);
    }

    #[test]
    fn actions_on_the_wire() {
        let a: Action =
            serde_json::from_str(r#"{"action":"buy_bike","frame":"frame.road.mid"}"#).unwrap();
        assert_eq!(
            a,
            Action::BuyBike {
                frame: "frame.road.mid".into(),
                with: vec![]
            }
        );
        assert_eq!(a.endpoint(), Endpoint::Purchase);
        assert!(a.moves_hearts());
        let s = Action::Select { bike: "x".into() };
        assert!(!s.moves_hearts());
        let back: Action = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        for r in Reason::ALL {
            assert_eq!(Reason::parse(r.id()), Some(r));
            assert_eq!(
                serde_json::to_string(&r).unwrap(),
                format!("\"{}\"", r.id())
            );
        }
    }

    #[test]
    fn garage_uses_the_local_field_names() {
        let g = Garage::starter("Bea Climber");
        let v = serde_json::to_value(&g).unwrap();
        assert!(v.get("bike").is_some() && v.get("item").is_some());
        let back: Garage = serde_json::from_value(v).unwrap();
        assert_eq!(back, g);
    }
}
