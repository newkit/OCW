//! The parts catalogue of the workshop (spec §14 Phase 4, 4B, 4H, 4I):
//! `assets/parts.toml` (generic, unbranded parts with real weights and
//! prices in hearts) and `assets/parts-sources.toml` (the real reference
//! products the prices come from), the validator (4J) and the repricing of
//! the catalogue from its sources. Bevy-free and without file access (the
//! command-line tool reads the files).
//!
//! The rider's kit (spec 4C, W4: jersey, bib shorts / tights, socks,
//! shoes, gloves, helmet, cap – category *apparel*) is in the same
//! catalogue: [`Slot::KIT`], one free base item per slot (the base kit
//! every rider starts with), models are the rider generator's variants
//! (`crate::rider::VARIANTS`), small effect classes (spec 4C, v3.04).
//!
//! Prices: **1 heart = 1 EUR**, a part's hearts are `ceil(median)` of its
//! references (two references: their mean), computed exactly in half
//! cents; groups of references (a bar and a stem) add their medians, pairs
//! priced per piece (tyres) multiply by 2.

use std::collections::BTreeSet;

use ocw_core::{BikeType, ProfileId};
use serde::Deserialize;

/// A part slot of a bike (spec §14 Phase 4, 4B): one hotspot each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    Frame,
    Wheels,
    Tyres,
    Groupset,
    Crankset,
    Cassette,
    Chain,
    Cockpit,
    BarTape,
    Saddle,
    Seatpost,
    Pedals,
    BottleCages,
    Lights,
    Computer,
    Bag,
    Paint,
    // The rider's kit (category apparel, spec 4C).
    Jersey,
    Shorts,
    Socks,
    Shoes,
    Gloves,
    Helmet,
    Cap,
}

impl Slot {
    /// The kit slots (the character corner's paid slots), in list order.
    pub const KIT: [Slot; 7] = [
        Slot::Jersey,
        Slot::Shorts,
        Slot::Socks,
        Slot::Shoes,
        Slot::Gloves,
        Slot::Helmet,
        Slot::Cap,
    ];

    /// Every slot, in list order (LB / RB step through them in this order).
    pub const ALL: [Slot; 17] = [
        Slot::Frame,
        Slot::Wheels,
        Slot::Tyres,
        Slot::Groupset,
        Slot::Crankset,
        Slot::Cassette,
        Slot::Chain,
        Slot::Cockpit,
        Slot::BarTape,
        Slot::Saddle,
        Slot::Seatpost,
        Slot::Pedals,
        Slot::BottleCages,
        Slot::Lights,
        Slot::Computer,
        Slot::Bag,
        Slot::Paint,
    ];

    /// Id in files, part ids, console commands and the `hs_<slot>` nodes.
    pub fn id(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Wheels => "wheels",
            Self::Tyres => "tyres",
            Self::Groupset => "groupset",
            Self::Crankset => "crankset",
            Self::Cassette => "cassette",
            Self::Chain => "chain",
            Self::Cockpit => "cockpit",
            Self::BarTape => "bar_tape",
            Self::Saddle => "saddle",
            Self::Seatpost => "seatpost",
            Self::Pedals => "pedals",
            Self::BottleCages => "bottle_cages",
            Self::Lights => "lights",
            Self::Computer => "computer",
            Self::Bag => "bag",
            Self::Paint => "paint",
            Self::Jersey => "jersey",
            Self::Shorts => "shorts",
            Self::Socks => "socks",
            Self::Shoes => "shoes",
            Self::Gloves => "gloves",
            Self::Helmet => "helmet",
            Self::Cap => "cap",
        }
    }

    /// Name for the UI (hotspot labels, picker titles).
    pub fn name(self) -> &'static str {
        match self {
            Self::Frame => "Frameset",
            Self::Wheels => "Wheels",
            Self::Tyres => "Tyres",
            Self::Groupset => "Groupset",
            Self::Crankset => "Crankset",
            Self::Cassette => "Cassette",
            Self::Chain => "Chain",
            Self::Cockpit => "Cockpit",
            Self::BarTape => "Bar tape",
            Self::Saddle => "Saddle",
            Self::Seatpost => "Seatpost",
            Self::Pedals => "Pedals",
            Self::BottleCages => "Bottle cages",
            Self::Lights => "Lights",
            Self::Computer => "Bike computer",
            Self::Bag => "Bag",
            Self::Paint => "Paint",
            Self::Jersey => "Jersey",
            Self::Shorts => "Bib shorts / tights",
            Self::Socks => "Socks",
            Self::Shoes => "Shoes",
            Self::Gloves => "Gloves",
            Self::Helmet => "Helmet",
            Self::Cap => "Cap",
        }
    }

    /// A slot of the bike ([`Slot::ALL`]) or of the kit ([`Slot::KIT`]).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .chain(Self::KIT)
            .find(|x| x.id().eq_ignore_ascii_case(s))
    }

    /// An apparel slot of the rider's kit (not a bike part).
    pub fn is_kit(self) -> bool {
        Self::KIT.contains(&self)
    }

    /// Whether the slot holds catalogue parts with a base part per bike
    /// type (all but paint: paint colours are free and not catalogue parts).
    pub fn has_base(self) -> bool {
        self != Self::Paint
    }
}

/// A part's tier (base = the free stock part a frame comes with).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Base,
    Entry,
    Mid,
    Upper,
    Top,
}

impl Tier {
    pub const ALL: [Tier; 5] = [Tier::Base, Tier::Entry, Tier::Mid, Tier::Upper, Tier::Top];

    pub fn id(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Entry => "entry",
            Self::Mid => "mid",
            Self::Upper => "upper",
            Self::Top => "top",
        }
    }

    /// Badge text in the picker.
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Entry => "Entry",
            Self::Mid => "Mid",
            Self::Upper => "Upper",
            Self::Top => "Top",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|x| x.id().eq_ignore_ascii_case(s))
    }
}

/// Optional standards of a part (spec 4F; informational in W2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Standard {
    pub speeds: Option<u32>,
    /// `mech` or `electronic`.
    pub shifting: Option<String>,
    /// `rim` or `disc`.
    pub brake: Option<String>,
    /// `700c` or `29`.
    pub wheel: Option<String>,
    pub width_mm: Option<u32>,
    /// Drivetrain family: `road`, `gravel`, `mtb`.
    pub family: Option<String>,
}

/// One catalogue entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Stable id, `<slot>.<kind>[.<tier>]`, never reused.
    pub id: String,
    pub slot: Slot,
    pub tier: Tier,
    /// Generic English name, no brand.
    pub name: String,
    /// `ceil(median EUR)` of the sources; 0 for base parts.
    pub hearts: u32,
    /// Typical weight (pairs for wheels, tyres, pedals).
    pub weight_g: u32,
    /// The bike types it fits.
    pub fits: Vec<BikeType>,
    pub standard: Standard,
    /// Frames: the physics profile (spec §2.5).
    pub profile: Option<ProfileId>,
    /// Class steps (4G): the build physics of `ocw_sim::build` (W3).
    pub rolling: i32,
    pub aero: i32,
    pub efficiency: i32,
    /// Kit: the kit slots this item covers (a skinsuit: the bib shorts).
    pub covers: Vec<Slot>,
    /// Groupsets: the crankset, cassette and chain fitted with it (spec
    /// 4B *Groupset and its parts*; free parts that are never sold alone).
    pub includes: Vec<String>,
    /// Frames: `bike-<profile>`; base parts: `stock`; else a part variant
    /// (kit: a rider variant).
    pub model: String,
    /// Key into the sources file.
    pub sources: Option<String>,
    /// Date of the price research (`YYYY-MM-DD`).
    pub priced: Option<String>,
}

impl Part {
    pub fn fits(&self, bike: BikeType) -> bool {
        self.fits.contains(&bike)
    }

    pub fn is_base(&self) -> bool {
        self.tier == Tier::Base
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPart {
    id: String,
    slot: String,
    tier: String,
    name: String,
    hearts: u32,
    weight_g: u32,
    fits: Vec<String>,
    #[serde(default)]
    standard: Standard,
    profile: Option<String>,
    #[serde(default)]
    rolling: i32,
    #[serde(default)]
    aero: i32,
    #[serde(default)]
    efficiency: i32,
    #[serde(default)]
    covers: Vec<String>,
    #[serde(default)]
    includes: Vec<String>,
    model: String,
    sources: Option<String>,
    priced: Option<toml::value::Datetime>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogue {
    #[serde(default)]
    part: Vec<RawPart>,
}

fn date_string(d: &toml::value::Datetime) -> Option<String> {
    d.date
        .map(|d| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day))
}

/// `(year, month, day)` of a `YYYY-MM-DD` string.
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.trim().splitn(3, '-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Whole months from `from` to `to` (a month counts once its day is reached).
pub fn months_between(from: (i32, u32, u32), to: (i32, u32, u32)) -> i32 {
    let mut m = (to.0 - from.0) * 12 + to.1 as i32 - from.1 as i32;
    if to.2 < from.2 {
        m -= 1;
    }
    m
}

/// The parts catalogue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    pub parts: Vec<Part>,
}

impl Catalogue {
    /// Parses `assets/parts.toml`; every malformed entry is reported.
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let raw: RawCatalogue = toml::from_str(text).map_err(|e| vec![format!("parts: {e}")])?;
        let mut errors = Vec::new();
        let mut parts = Vec::new();
        for r in raw.part {
            let at = r.id.clone();
            let slot = Slot::parse(&r.slot);
            let tier = Tier::parse(&r.tier);
            if slot.is_none() {
                errors.push(format!("{at}: unknown slot {:?}", r.slot));
            }
            if tier.is_none() {
                errors.push(format!("{at}: unknown tier {:?}", r.tier));
            }
            let mut fits = Vec::new();
            for f in &r.fits {
                match BikeType::parse(f) {
                    Some(b) if !fits.contains(&b) => fits.push(b),
                    Some(_) => errors.push(format!("{at}: bike type {f:?} twice in fits")),
                    None => errors.push(format!("{at}: unknown bike type {f:?}")),
                }
            }
            let profile = match r.profile.as_deref() {
                None => None,
                Some(p) => match ProfileId::parse(p) {
                    Some(p) => Some(p),
                    None => {
                        errors.push(format!("{at}: unknown profile {p:?}"));
                        None
                    }
                },
            };
            let mut covers = Vec::new();
            for c in &r.covers {
                match Slot::parse(c) {
                    Some(c) if c.is_kit() => covers.push(c),
                    _ => errors.push(format!("{at}: covers {c:?}, not a kit slot")),
                }
            }
            let (Some(slot), Some(tier)) = (slot, tier) else {
                continue;
            };
            parts.push(Part {
                id: r.id,
                slot,
                tier,
                name: r.name,
                hearts: r.hearts,
                weight_g: r.weight_g,
                fits,
                standard: r.standard,
                profile,
                rolling: r.rolling,
                aero: r.aero,
                efficiency: r.efficiency,
                covers,
                includes: r.includes,
                model: r.model,
                sources: r.sources,
                priced: r.priced.as_ref().and_then(date_string),
            });
        }
        if errors.is_empty() {
            Ok(Self { parts })
        } else {
            Err(errors)
        }
    }

    pub fn get(&self, id: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }

    /// The parts fitted with groupset `groupset` (spec 4B: its crankset,
    /// cassette and chain); unknown ids are skipped.
    pub fn included(&self, groupset: &Part) -> Vec<&Part> {
        groupset
            .includes
            .iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// Whether part `id` comes only with a groupset (free, never sold
    /// alone).
    pub fn is_included(&self, id: &str) -> bool {
        self.parts
            .iter()
            .any(|g| g.includes.iter().any(|i| i == id))
    }

    /// The parts of `slot` that fit `bike`: base first, then by tier and
    /// price (cheapest first). Frames: use [`Catalogue::frames`].
    pub fn for_slot(&self, slot: Slot, bike: BikeType) -> Vec<&Part> {
        let mut v: Vec<&Part> = self
            .parts
            .iter()
            .filter(|p| p.slot == slot && p.fits(bike))
            .collect();
        v.sort_by_key(|p| (p.tier, p.hearts, p.id.clone()));
        v
    }

    /// The framesets of a physics profile (a frame change keeps the
    /// profile, spec 4G), base first.
    pub fn frames(&self, profile: ProfileId) -> Vec<&Part> {
        let mut v: Vec<&Part> = self
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Frame && p.profile == Some(profile))
            .collect();
        v.sort_by_key(|p| (p.tier, p.hearts, p.id.clone()));
        v
    }

    /// The free base part of `slot` for `bike` (frames: the base frame of
    /// the bike type's default profile; see [`Catalogue::base_frame`]).
    pub fn base(&self, slot: Slot, bike: BikeType) -> Option<&Part> {
        if slot == Slot::Frame {
            return self.base_frame(bike.default_profile());
        }
        self.parts
            .iter()
            .find(|p| p.slot == slot && p.is_base() && p.fits(bike))
    }

    /// The kit items of `slot` (spec 4C): base first, then by tier and
    /// price.
    pub fn kit(&self, slot: Slot) -> Vec<&Part> {
        let mut v: Vec<&Part> = self.parts.iter().filter(|p| p.slot == slot).collect();
        v.sort_by_key(|p| (p.tier, p.hearts, p.id.clone()));
        v
    }

    /// The free base item of kit `slot` (the base kit).
    pub fn base_kit(&self, slot: Slot) -> Option<&Part> {
        self.parts.iter().find(|p| p.slot == slot && p.is_base())
    }

    /// The base frameset of a profile (a starter bike's frame).
    pub fn base_frame(&self, profile: ProfileId) -> Option<&Part> {
        self.parts
            .iter()
            .find(|p| p.slot == Slot::Frame && p.is_base() && p.profile == Some(profile))
    }
}

/// One real reference product (research data, spec 4H).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub product: String,
    pub brand: String,
    /// Price in EUR incl. VAT.
    pub eur: f64,
    /// `R`, `R*` or `S` (spec 4H).
    pub kind: String,
    /// Where the price was found (abbreviation of the 4H source list).
    pub via: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl Reference {
    /// The price in whole cents.
    pub fn cents(&self) -> i64 {
        (self.eur * 100.0).round() as i64
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    #[serde(default)]
    pub name: String,
    pub refs: Vec<Reference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    key: String,
    checked: toml::value::Datetime,
    #[serde(default = "one")]
    multiply: u32,
    #[serde(default)]
    refs: Vec<Reference>,
    #[serde(default)]
    group: Vec<Group>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSources {
    #[serde(default)]
    source: Vec<RawSource>,
}

/// The references of one price (one key of `parts-sources.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub key: String,
    /// Date of the research (`YYYY-MM-DD`).
    pub checked: String,
    /// 2 for pairs priced per piece.
    pub multiply: u32,
    /// Groups whose medians add up (one group for most parts).
    pub groups: Vec<Group>,
}

/// Median of `cents` in half cents (exact: odd count 2 × middle, even
/// count the sum of the two middle values). `None` for no values.
pub fn median_half_cents(cents: &[i64]) -> Option<i64> {
    if cents.is_empty() {
        return None;
    }
    let mut v = cents.to_vec();
    v.sort_unstable();
    let n = v.len();
    Some(if n % 2 == 1 {
        2 * v[n / 2]
    } else {
        v[n / 2 - 1] + v[n / 2]
    })
}

impl Source {
    /// The price in hearts: the groups' medians summed, × `multiply`,
    /// rounded up to whole hearts (1 heart = 1 EUR).
    pub fn hearts(&self) -> u32 {
        let half_cents: i64 = self
            .groups
            .iter()
            .filter_map(|g| {
                median_half_cents(&g.refs.iter().map(Reference::cents).collect::<Vec<_>>())
            })
            .sum::<i64>()
            * i64::from(self.multiply);
        // 200 half cents = 1 EUR = 1 heart, rounded up.
        (half_cents.max(0) as u64).div_ceil(200) as u32
    }
}

/// The price references (`assets/parts-sources.toml`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub sources: Vec<Source>,
}

impl Sources {
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let raw: RawSources =
            toml::from_str(text).map_err(|e| vec![format!("parts sources: {e}")])?;
        let mut errors = Vec::new();
        let mut sources = Vec::new();
        for r in raw.source {
            let Some(checked) = date_string(&r.checked) else {
                errors.push(format!("{}: `checked` must be a date", r.key));
                continue;
            };
            let mut groups = r.group;
            if !r.refs.is_empty() {
                groups.insert(
                    0,
                    Group {
                        name: String::new(),
                        refs: r.refs,
                    },
                );
            }
            sources.push(Source {
                key: r.key,
                checked,
                multiply: r.multiply,
                groups,
            });
        }
        if errors.is_empty() {
            Ok(Self { sources })
        } else {
            Err(errors)
        }
    }

    pub fn get(&self, key: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.key == key)
    }

    /// The hearts of source `key`.
    pub fn hearts_of(&self, key: &str) -> Option<u32> {
        self.get(key).map(Source::hearts)
    }
}

/// The model of a base part: the frame's own (stock) part.
pub const STOCK_MODEL: &str = "stock";

/// Brands and product names of the price research (spec 4H) that must
/// never appear in the game: checked (case-insensitive, whole words)
/// against every part's name, id and model. Brands that are also plain
/// English or bike words ([`GENERIC_BRAND_WORDS`]: Time, Look, Force, Red,
/// Record, Open, Elite, Race, PRO, Factor, Giant, Apex, Rival, Rally, Edge,
/// Comp, Zero …) are not on the list: generic names such as "Time-trial
/// tyres" or "Race 12-speed groupset" use them; the distinctive product
/// names around them are listed instead.
pub const BRANDS: &[&str] = &[
    // Groupsets, cranks, cassettes, chains.
    "Shimano",
    "SRAM",
    "Campagnolo",
    "Tiagra",
    "Ultegra",
    "Dura-Ace",
    "Di2",
    "AXS",
    "XPLR",
    "GRX",
    "Deore",
    "XTR",
    "Chorus",
    "Super Record",
    "Eagle",
    "T-Type",
    "Flattop",
    "4iiii",
    "Favero",
    "Assioma",
    "Garmin",
    // Wheels and tyres.
    "Fulcrum",
    "Mavic",
    "Aksium",
    "Ksyrium",
    "DT Swiss",
    "Zipp",
    "Continental",
    "Schwalbe",
    "Pirelli",
    "P Zero",
    "Cinturato",
    "Vittoria",
    "Corsa",
    "Maxxis",
    "Rekon",
    "GP 5000",
    "Lugano",
    "G-One",
    // Saddles, cockpits, seat posts, pedals.
    "Selle Italia",
    "Specialized",
    "S-Works",
    "Fizik",
    "Prologo",
    "Deda",
    "Ritchey",
    "Renthal",
    "Race Face",
    "Profile Design",
    "Black Inc",
    "Roval",
    "Speedplay",
    "Crankbrothers",
    "Keo",
    "Kéo",
    // Framesets.
    "Cannondale",
    "CAAD14",
    "Blackheart",
    "Cervélo",
    "Cervelo",
    "BMC",
    "Trek",
    "Madone",
    "Tarmac",
    "Scott",
    "Mondraker",
    "Canyon",
    // Accessories and computers.
    "Supacaz",
    "Tacx",
    "SKS",
    "Lezyne",
    "busch+müller",
    "Knog",
    "K-Edge",
    "Topeak",
    "Ortlieb",
    "Apidura",
    "Wahoo",
    "Elemnt",
    // Apparel (spec 4C, W4: helmets, shoes, clothing, gloves, caps).
    "Castelli",
    "Perfetto",
    "Sorpasso",
    "Quindici",
    "Sanremo",
    "Entrata",
    "Arenberg",
    "Rapha",
    "Assos",
    "Equipe",
    "RSR",
    "Ultraz",
    "Schtradivari",
    "Le Houdini",
    "Gore",
    "Gorewear",
    "Gore Wear",
    "Gore-Tex",
    "Infinium",
    "Windstopper",
    "Sportful",
    "Bodyfit",
    "Fiandre",
    "Hyperepic",
    "MAAP",
    "S-Phyre",
    "RC903",
    "Vento",
    "Infinito",
    "DMT",
    "KR0",
    "KRSL",
    "Sidi",
    "GripGrab",
    "RaceAero",
    "Endura",
    "Xtract",
    "Roeckl",
    "Belluno",
    "Bioxcell",
    "Giro",
    "Aries",
    "Aerohead",
    "Supernatural",
    "Trenta",
    "Manta",
    "Prevail",
    "Evade",
    "POC",
    "Ventral",
    "Tempor",
    "Procen",
    "Kask",
    "Valegro",
    "Utopia",
    "Bambino",
    "Rudy Project",
    "Café du Cycliste",
    "Cafe du Cycliste",
    "Gapette",
    "Pas Normal",
    "Pedal Ed",
    "PEdALED",
    "Isadore",
    "Le Col",
    "Gobik",
    "Santini",
    "EKOI",
    "Velotoze",
    "Uyn",
    "Ryzon",
    "MIPS",
    "Lycra",
    "Technogel",
    "Poron",
    "Chiba",
    "MET",
];

/// Researched brands that are plain words and so not on [`BRANDS`]
/// (they appear in generic names).
pub const GENERIC_BRAND_WORDS: &[&str] = &[
    "Time", "Look", "Force", "Red", "Record", "Open", "Elite", "Race", "PRO", "Factor", "Giant",
    "Apex", "Rival", "Rally", "Edge", "Comp", "Zero", "Rose",
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// The first brand of [`BRANDS`] in `text` (case-insensitive, whole
/// words: "DT Swiss wheels" yes, "Treklike" no).
pub fn brand_in(text: &str) -> Option<&'static str> {
    let hay = text.to_lowercase();
    BRANDS.iter().copied().find(|b| {
        let needle = b.to_lowercase();
        hay.match_indices(&needle).any(|(i, m)| {
            let before = hay[..i].chars().next_back();
            let after = hay[i + m.len()..].chars().next();
            !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
        })
    })
}

/// How bad a [`Problem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A finding of [`validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s}: {}", self.message)
    }
}

/// Kit `aero` class range (the server's `ocw_sim::build::KIT_AERO_RANGE`).
pub const KIT_AERO_RANGE: i32 = 4;

/// Sources older than this are an error (re-research the prices, 4H).
pub const STALE_MONTHS: i32 = 18;
/// … and older than this a warning.
pub const AGING_MONTHS: i32 = 12;

fn valid_id(part: &Part) -> bool {
    let segs: Vec<&str> = part.id.split('.').collect();
    (2..=3).contains(&segs.len())
        && segs[0] == part.slot.id()
        && segs.iter().all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        })
}

/// Checks the catalogue against its sources (spec 4J) on `today`
/// (`(year, month, day)`): unique, well-formed ids; prices =
/// `ceil(median)` of the sources; base parts free and present for every
/// slot and bike type (frames: every profile); frames name their profile
/// and model; model references resolve to a generator variant of the
/// slot that fits; no brand names; sources not older than 18 months.
pub fn validate(cat: &Catalogue, sources: &Sources, today: (i32, u32, u32)) -> Vec<Problem> {
    let mut errs: Vec<String> = Vec::new();
    let mut warns: Vec<String> = Vec::new();
    let mut err = |m: String| errs.push(m);
    let mut seen = BTreeSet::new();
    for p in &cat.parts {
        let at = &p.id;
        if !seen.insert(p.id.as_str()) {
            err(format!("{at}: duplicate id"));
        }
        if !valid_id(p) {
            err(format!(
                "{at}: id must be `{}.<kind>[.<tier>]` (lower case, digits, - and _)",
                p.slot.id()
            ));
        }
        if p.fits.is_empty() {
            err(format!("{at}: fits no bike type"));
        }
        if p.slot.is_kit() {
            // Kit is worn by the rider, on any bike (spec 4C).
            if BikeType::ALL.iter().any(|b| !p.fits.contains(b)) {
                err(format!(
                    "{at}: kit fits every bike type (road, tt, gravel, mtb)"
                ));
            }
            if p.rolling != 0 {
                err(format!("{at}: kit has no rolling class"));
            }
            if !(-KIT_AERO_RANGE..=KIT_AERO_RANGE).contains(&p.aero)
                || !(-1..=1).contains(&p.efficiency)
            {
                err(format!(
                    "{at}: kit classes are aero −{KIT_AERO_RANGE} … +{KIT_AERO_RANGE}, efficiency −1 … +1"
                ));
            }
            // Comfort items have no effect (spec 4C).
            if matches!(p.slot, Slot::Gloves | Slot::Cap) && (p.aero != 0 || p.efficiency != 0) {
                err(format!(
                    "{at}: gloves and caps are comfort items without effect classes"
                ));
            }
            for c in &p.covers {
                if *c == p.slot {
                    err(format!("{at}: an item cannot cover its own slot"));
                }
            }
        } else if !p.covers.is_empty() {
            err(format!("{at}: only kit items cover other slots"));
        }
        if p.name.trim().is_empty() {
            err(format!("{at}: no name"));
        }
        for (what, text) in [("name", &p.name), ("id", &p.id), ("model", &p.model)] {
            if let Some(b) = brand_in(text) {
                err(format!(
                    "{at}: brand name {b:?} in the {what} (no brands, spec Q11)"
                ));
            }
        }
        if p.slot == Slot::Paint {
            err(format!("{at}: paint colours are not catalogue parts"));
        }
        // Prices.
        let included = cat.is_included(&p.id);
        if p.is_base() {
            if p.hearts != 0 {
                err(format!(
                    "{at}: a base part costs 0 hearts, not {}",
                    p.hearts
                ));
            }
            if included {
                err(format!("{at}: a base part is not part of a groupset"));
            }
        } else if included {
            // Fitted with its groupset (spec 4B): its price covers it.
            if p.hearts != 0 || p.sources.is_some() {
                err(format!(
                    "{at}: comes with a groupset: free and without sources of its own"
                ));
            }
        } else {
            match p.sources.as_deref() {
                None => err(format!("{at}: no sources for its price")),
                Some(key) => match sources.hearts_of(key) {
                    None => err(format!("{at}: unknown sources {key:?}")),
                    Some(h) if h != p.hearts => err(format!(
                        "{at}: {} hearts, but ceil(median) of {key:?} is {h} (run `ocw-assets parts reprice`)",
                        p.hearts
                    )),
                    Some(_) => {}
                },
            }
            if p.hearts == 0 {
                err(format!("{at}: only base parts are free"));
            }
            if p.priced.is_none() {
                err(format!("{at}: no `priced` date"));
            }
        }
        // Frames and models.
        if p.slot.is_kit() {
            if p.is_base() {
                if p.model != STOCK_MODEL {
                    err(format!("{at}: a base kit item's model is {STOCK_MODEL:?}"));
                }
            } else {
                match crate::rider::variant(&p.model) {
                    None => err(format!("{at}: unknown rider variant {:?}", p.model)),
                    Some(v) if v.slot != p.slot.id() => err(format!(
                        "{at}: model {:?} is a {} variant, not {}",
                        p.model,
                        v.slot,
                        p.slot.id()
                    )),
                    Some(_) => {}
                }
            }
            if p.profile.is_some() {
                err(format!("{at}: only framesets have a profile"));
            }
        } else if p.slot == Slot::Frame {
            match p.profile {
                None => err(format!("{at}: a frameset names its physics profile")),
                Some(prof) => {
                    if p.fits != [prof.bike()] {
                        err(format!(
                            "{at}: a {} frameset fits only {}",
                            prof.id(),
                            prof.bike().id()
                        ));
                    }
                    let want = format!("bike-{}", prof.id());
                    if p.model != want {
                        err(format!("{at}: model {:?}, expected {want:?}", p.model));
                    }
                }
            }
        } else {
            if p.profile.is_some() {
                err(format!("{at}: only framesets have a profile"));
            }
            if p.is_base() || (included && p.model == STOCK_MODEL) {
                if p.model != STOCK_MODEL {
                    err(format!("{at}: a base part's model is {STOCK_MODEL:?}"));
                }
            } else {
                match crate::bikes::variant(&p.model) {
                    None => err(format!("{at}: unknown model {:?}", p.model)),
                    Some(v) => {
                        if v.slot != p.slot.id() {
                            err(format!(
                                "{at}: model {:?} is a {} variant, not {}",
                                p.model,
                                v.slot,
                                p.slot.id()
                            ));
                        }
                        for b in &p.fits {
                            if !v.fits.contains(b) {
                                err(format!(
                                    "{at}: model {:?} has no {} variant",
                                    p.model,
                                    b.id()
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    // A free base part for every slot and bike type, a base frame per profile.
    for slot in Slot::ALL
        .into_iter()
        .filter(|s| s.has_base() && *s != Slot::Frame)
    {
        for b in BikeType::ALL {
            let n = cat
                .parts
                .iter()
                .filter(|p| p.slot == slot && p.is_base() && p.fits(b))
                .count();
            if n != 1 {
                err(format!(
                    "{} base parts of slot {} for {} (need exactly 1)",
                    n,
                    slot.id(),
                    b.id()
                ));
            }
        }
    }
    // The free base kit: one base item per kit slot.
    for slot in Slot::KIT {
        let n = cat
            .parts
            .iter()
            .filter(|p| p.slot == slot && p.is_base())
            .count();
        if n != 1 {
            err(format!(
                "{n} base items of kit slot {} (need exactly 1: the free base kit)",
                slot.id()
            ));
        }
    }
    // Groupsets and their crankset, cassette and chain (spec 4B).
    for g in &cat.parts {
        let at = &g.id;
        if g.includes.is_empty() {
            if g.slot == Slot::Groupset && !g.is_base() {
                err(format!(
                    "{at}: a groupset includes its crankset, cassette and chain"
                ));
            }
            continue;
        }
        if g.slot != Slot::Groupset || g.is_base() {
            err(format!("{at}: only bought groupsets include parts"));
            continue;
        }
        let mut slots = Vec::new();
        for id in &g.includes {
            let Some(c) = cat.get(id) else {
                err(format!("{at}: includes unknown part {id:?}"));
                continue;
            };
            if !matches!(c.slot, Slot::Crankset | Slot::Cassette | Slot::Chain) {
                err(format!(
                    "{at}: includes {id}, a {} (crankset, cassette or chain only)",
                    c.slot.id()
                ));
            }
            slots.push(c.slot);
            for b in &g.fits {
                if !c.fits.contains(b) {
                    err(format!("{at}: {id} does not fit {}", b.id()));
                }
            }
        }
        for s in [Slot::Crankset, Slot::Cassette, Slot::Chain] {
            if slots.iter().filter(|x| **x == s).count() != 1 {
                err(format!("{at}: includes exactly one {}", s.id()));
            }
        }
    }
    // An included part weighs what the base part does: the groupset's
    // researched weight is the whole groupset's (no double counting).
    for p in cat.parts.iter().filter(|p| cat.is_included(&p.id)) {
        for b in &p.fits {
            if let Some(base) = cat.base(p.slot, *b)
                && base.weight_g != p.weight_g
            {
                err(format!(
                    "{}: weighs {} g, the base part for {} {} g (its weight is in the groupset's)",
                    p.id,
                    p.weight_g,
                    b.id(),
                    base.weight_g
                ));
            }
        }
    }
    for prof in ProfileId::ALL {
        if cat.base_frame(prof).is_none() {
            err(format!("no base frameset for profile {}", prof.id()));
        }
    }
    // The sources: well-formed and not too old.
    let mut keys = BTreeSet::new();
    for s in &sources.sources {
        if !keys.insert(s.key.as_str()) {
            err(format!("sources {:?}: duplicate key", s.key));
        }
        if s.multiply == 0 {
            err(format!("sources {:?}: multiply must be at least 1", s.key));
        }
        if s.groups.is_empty() || s.groups.iter().any(|g| g.refs.is_empty()) {
            err(format!("sources {:?}: a group without references", s.key));
        }
        for r in s.groups.iter().flat_map(|g| &g.refs) {
            if r.eur.is_nan() || r.eur <= 0.0 {
                err(format!("sources {:?}: {} has no price", s.key, r.product));
            }
            if !matches!(r.kind.as_str(), "R" | "R*" | "S") {
                err(format!(
                    "sources {:?}: {}: kind {:?} (R, R* or S)",
                    s.key, r.product, r.kind
                ));
            }
        }
        match parse_date(&s.checked) {
            None => err(format!("sources {:?}: bad date {}", s.key, s.checked)),
            Some(d) => {
                let age = months_between(d, today);
                if age >= STALE_MONTHS {
                    err(format!(
                        "sources {:?}: checked {} ({age} months ago): re-research the prices (4H)",
                        s.key, s.checked
                    ));
                } else if age >= AGING_MONTHS {
                    warns.push(format!(
                        "sources {:?}: checked {} ({age} months ago): due for a re-research soon",
                        s.key, s.checked
                    ));
                }
            }
        }
    }
    let problem = |severity: Severity| move |message: String| Problem { severity, message };
    errs.into_iter()
        .map(problem(Severity::Error))
        .chain(warns.into_iter().map(problem(Severity::Warning)))
        .collect()
}

/// Only the errors of [`validate`].
pub fn errors(problems: &[Problem]) -> Vec<&Problem> {
    problems
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .collect()
}

/// A price change of [`reprice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub old: u32,
    pub new: u32,
}

/// Rewrites the `hearts = N` lines of `parts.toml` text from `sources`
/// (spec 4H, `ocw-assets parts reprice`): everything else – comments,
/// order, layout – stays. Parts without sources (base parts) and unknown
/// source keys are left alone (the validator reports the latter).
pub fn reprice(text: &str, sources: &Sources) -> (String, Vec<Change>) {
    struct Block {
        id: Option<String>,
        hearts_line: Option<usize>,
        key: Option<String>,
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut changes = Vec::new();
    let value = |l: &str, key: &str| -> Option<String> {
        let t = l.trim_start();
        let rest = t.strip_prefix(key)?.trim_start().strip_prefix('=')?;
        let rest = rest.split('#').next()?.trim();
        Some(rest.trim_matches('"').to_string())
    };
    let mut finish = |b: &Block, out: &mut Vec<String>| {
        let (Some(i), Some(key)) = (b.hearts_line, &b.key) else {
            return;
        };
        let Some(new) = sources.hearts_of(key) else {
            return;
        };
        let line = &lines[i];
        let Some(old) = value(line, "hearts").and_then(|v| v.parse::<u32>().ok()) else {
            return;
        };
        if old == new {
            return;
        }
        let eq = line.find('=').expect("a hearts line");
        let after = &line[eq + 1..];
        let digits_at = after.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
        let digits_len = after[digits_at..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len() - digits_at);
        out[i] = format!(
            "{}{}{}{}",
            &line[..eq + 1],
            &after[..digits_at],
            new,
            &after[digits_at + digits_len..]
        );
        changes.push(Change {
            id: b.id.clone().unwrap_or_default(),
            old,
            new,
        });
    };
    let mut block: Option<Block> = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            if let Some(b) = block.take() {
                finish(&b, &mut out);
            }
            if t == "[[part]]" {
                block = Some(Block {
                    id: None,
                    hearts_line: None,
                    key: None,
                });
            }
            continue;
        }
        if let Some(b) = block.as_mut() {
            if let Some(v) = value(l, "id") {
                b.id = Some(v);
            } else if value(l, "hearts").is_some() {
                b.hearts_line = Some(i);
            } else if let Some(v) = value(l, "sources") {
                b.key = Some(v);
            }
        }
    }
    if let Some(b) = block.take() {
        finish(&b, &mut out);
    }
    (out.concat(), changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> (Catalogue, Sources) {
        (
            Catalogue::parse(crate::shipped::PARTS).expect("parts.toml"),
            Sources::parse(crate::shipped::PARTS_SOURCES).expect("parts-sources.toml"),
        )
    }

    const TODAY: (i32, u32, u32) = (2026, 11, 14);

    #[test]
    fn the_shipped_catalogue_is_valid() {
        let (c, s) = shipped();
        let problems = validate(&c, &s, TODAY);
        assert!(problems.is_empty(), "{problems:#?}");
        // The first catalogue of 4I (about 40 parts) plus the base parts.
        let priced = c.parts.iter().filter(|p| !p.is_base()).count();
        assert!(priced >= 40, "{priced}");
        assert!(std::ptr::eq(
            crate::shipped::catalogue(),
            crate::shipped::catalogue()
        ));
    }

    /// The validator with today's date: the sources must be re-researched
    /// within 18 months (spec 4H); this test turns red when they are not.
    #[test]
    fn the_price_research_is_not_older_than_18_months() {
        let (c, s) = shipped();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let (y, m, d, ..) = ocw_core::timestamp::civil(now);
        let problems = validate(&c, &s, (y as i32, m, d));
        let errs = errors(&problems);
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn prices_from_the_spec_examples() {
        let (c, _) = shipped();
        for (id, hearts) in [
            ("groupset.road-electronic.top", 4135),
            ("groupset.gravel.mid", 1415),
            ("tyres.road.training", 61),
            ("tyres.road.performance", 164),
            ("tyres.road.race", 190),
            ("frame.road-aero.top", 5279),
            ("frame.road.mid", 1390),
            ("cockpit.road.carbon", 405),
            ("saddle.road.top", 310),
            ("lights.set", 80),
            ("computer.top", 660),
            ("bottle_cages.carbon", 31),
        ] {
            assert_eq!(c.get(id).map(|p| p.hearts), Some(hearts), "{id}");
        }
        let p = c.get("tyres.road.performance").unwrap();
        assert_eq!((p.slot, p.tier, p.rolling), (Slot::Tyres, Tier::Upper, -1));
        assert_eq!(p.standard.width_mm, Some(28));
        assert_eq!(
            c.get("frame.road-aero.top").unwrap().profile,
            Some(ProfileId::RoadAero)
        );
    }

    /// Ids are stable (§6.2 rule): ids that existed must still exist.
    #[test]
    fn earlier_ids_still_exist() {
        let (c, _) = shipped();
        const FIXTURE: &[&str] = &[
            "frame.road.base",
            "frame.road-aero.base",
            "frame.road-climbing.base",
            "frame.tt.base",
            "frame.gravel.base",
            "frame.mtb.base",
            "frame.road.mid",
            "frame.road.upper",
            "frame.road-climbing.top",
            "frame.road-aero.top",
            "frame.tt.top",
            "frame.gravel.upper",
            "frame.mtb.upper",
            "groupset.road-mech.entry",
            "groupset.road-mech.mid",
            "groupset.road-electronic.mid",
            "groupset.road-electronic.upper",
            "groupset.road-electronic.top",
            "groupset.gravel.mid",
            "groupset.gravel.top",
            "groupset.mtb.mid",
            "groupset.mtb.top",
            "crankset.road.power",
            "wheels.road-alloy.entry",
            "wheels.road-alloy.mid",
            "wheels.road-carbon.mid",
            "wheels.road-carbon.top",
            "wheels.tt.disc",
            "wheels.gravel.carbon",
            "tyres.road.training",
            "tyres.road.performance",
            "tyres.road.race",
            "tyres.gravel.fast",
            "tyres.mtb.xc",
            "cockpit.road.carbon",
            "cockpit.road.integrated",
            "cockpit.tt.extensions",
            "saddle.road.top",
            "seatpost.carbon",
            "pedals.road.top",
            "pedals.spd.mid",
            "bottle_cages.carbon",
            "lights.set",
            "computer.top",
            "bar_tape.mid",
            "bag.top_tube",
        ];
        for id in FIXTURE {
            assert!(
                c.get(id).is_some(),
                "{id} disappeared: ids are never removed or reused"
            );
        }
    }

    #[test]
    fn every_slot_and_bike_type_has_a_free_base_part() {
        let (c, _) = shipped();
        for slot in Slot::ALL.into_iter().filter(|s| s.has_base()) {
            for b in BikeType::ALL {
                let p = c.base(slot, b).unwrap_or_else(|| panic!("{slot:?} {b:?}"));
                assert_eq!((p.hearts, p.tier), (0, Tier::Base), "{}", p.id);
                assert!(p.fits(b) && p.slot == slot);
            }
        }
        for prof in ProfileId::ALL {
            let f = c.base_frame(prof).unwrap();
            assert_eq!(f.model, format!("bike-{}", prof.id()));
            assert_eq!(c.frames(prof)[0].id, f.id, "base first");
        }
        assert!(c.base(Slot::Paint, BikeType::Road).is_none());
    }

    #[test]
    fn for_slot_filters_by_bike_type_base_first_cheapest_first() {
        let (c, _) = shipped();
        let road: Vec<&str> = c
            .for_slot(Slot::Tyres, BikeType::Road)
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(
            road,
            [
                "tyres.road.base",
                "tyres.road.training",
                "tyres.road.performance",
                "tyres.road.race"
            ]
        );
        let mtb = c.for_slot(Slot::Tyres, BikeType::Mountain);
        assert!(mtb.iter().all(|p| p.fits(BikeType::Mountain)));
        assert!(mtb.iter().all(|p| !p.id.starts_with("tyres.road")));
        // A TT disc does not fit a road bike.
        assert!(
            !c.for_slot(Slot::Wheels, BikeType::Road)
                .iter()
                .any(|p| p.id == "wheels.tt.disc")
        );
    }

    #[test]
    fn hearts_are_the_rounded_up_median() {
        let refs = |e: &[f64]| Group {
            name: String::new(),
            refs: e
                .iter()
                .map(|&eur| Reference {
                    product: "x".into(),
                    brand: "y".into(),
                    eur,
                    kind: "R".into(),
                    via: "bc".into(),
                    url: None,
                    note: None,
                })
                .collect(),
        };
        let src = |groups: Vec<Group>, multiply| Source {
            key: "k".into(),
            checked: "2026-11-13".into(),
            multiply,
            groups,
        };
        // Odd count: the middle one; exact euros stay.
        assert_eq!(src(vec![refs(&[3999.0, 4135.0, 4300.0])], 1).hearts(), 4135);
        // Two references: the mean, rounded up (1414.5 → 1415).
        assert_eq!(src(vec![refs(&[1269.0, 1560.0])], 1).hearts(), 1415);
        // A pair priced per tyre: 2 × 81.925 = 163.85 → 164.
        assert_eq!(
            src(vec![refs(&[94.95, 78.90, 84.95, 69.95])], 2).hearts(),
            164
        );
        // Bar + stem: 349.95 + 54.95 = 404.90 → 405.
        assert_eq!(
            src(
                vec![refs(&[339.0, 360.0, 349.95]), refs(&[54.40, 54.95, 110.0])],
                1
            )
            .hearts(),
            405
        );
        // A cent above a whole euro rounds up; exactly whole does not.
        assert_eq!(src(vec![refs(&[10.01])], 1).hearts(), 11);
        assert_eq!(src(vec![refs(&[10.0])], 1).hearts(), 10);
        assert_eq!(median_half_cents(&[]), None);
    }

    #[test]
    fn the_deny_list_finds_brands_but_not_generic_words() {
        assert_eq!(brand_in("Shimano 105 groupset"), Some("Shimano"));
        assert_eq!(brand_in("carbon wheels by DT Swiss"), Some("DT Swiss"));
        assert_eq!(brand_in("a cervélo frame"), Some("Cervélo"));
        assert_eq!(brand_in("wheels.zipp-404.top"), Some("Zipp"));
        assert_eq!(brand_in("Dura-Ace crankset"), Some("Dura-Ace"));
        for generic in [
            "Time-trial tyres 25 mm (pair)",
            "Race 12-speed electronic groupset",
            "Aero race frameset, carbon",
            "Front and rear light set",
            "Trekking pedals",
            "Force-fit bar",
        ] {
            assert_eq!(brand_in(generic), None, "{generic}");
        }
        // Every researched brand is either on the deny-list or a plain word.
        let (c, s) = shipped();
        for r in s
            .sources
            .iter()
            .flat_map(|s| &s.groups)
            .flat_map(|g| &g.refs)
        {
            assert!(
                brand_in(&r.brand).is_some()
                    || GENERIC_BRAND_WORDS
                        .iter()
                        .any(|g| g.eq_ignore_ascii_case(&r.brand)),
                "brand {:?} is neither denied nor listed as generic",
                r.brand
            );
        }
        // And none of them is in the catalogue.
        for p in &c.parts {
            for t in [&p.name, &p.id, &p.model] {
                assert_eq!(brand_in(t), None, "{t}");
            }
        }
    }

    fn broken(edit: impl Fn(&mut Catalogue)) -> Vec<String> {
        let (mut c, s) = shipped();
        edit(&mut c);
        errors(&validate(&c, &s, TODAY))
            .iter()
            .map(|p| p.message.clone())
            .collect()
    }

    fn has(errors: &[String], needle: &str) -> bool {
        errors.iter().any(|e| e.contains(needle))
    }

    #[test]
    fn the_validator_catches_broken_entries() {
        let e = broken(|c| {
            let p = c.get("lights.set").unwrap().clone();
            c.parts.push(p);
        });
        assert!(has(&e, "lights.set: duplicate id"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .hearts = 79
        });
        assert!(has(&e, "ceil(median) of \"lights.set\" is 80"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .name = "Knog light set".into()
        });
        assert!(has(&e, "brand name \"Knog\""), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .model = "lamp".into()
        });
        assert!(has(&e, "unknown model \"lamp\""), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .model = "tyre-road-28".into()
        });
        assert!(has(&e, "is a tyres variant"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .fits
                .push(BikeType::TimeTrial)
        });
        assert!(has(&e, "has no tt variant"), "{e:?}");
        let e = broken(|c| c.parts.retain(|p| p.id != "bag.none.base"));
        assert!(has(&e, "0 base parts of slot bag for road"), "{e:?}");
        let e = broken(|c| c.parts.retain(|p| p.id != "frame.tt.base"));
        assert!(has(&e, "no base frameset for profile tt"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "bag.none.base")
                .unwrap()
                .hearts = 5
        });
        assert!(has(&e, "a base part costs 0 hearts"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .id = "light.set".into()
        });
        assert!(has(&e, "id must be"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "frame.tt.top")
                .unwrap()
                .model = "bike-road".into()
        });
        assert!(has(&e, "expected \"bike-tt\""), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "lights.set")
                .unwrap()
                .sources = Some("nope".into())
        });
        assert!(has(&e, "unknown sources \"nope\""), "{e:?}");
    }

    /// Spec 4B *Groupset and its parts*: every bought groupset includes
    /// one crankset, cassette and chain of its tier that fit it, free and
    /// at the base weight.
    #[test]
    fn groupsets_include_their_crankset_cassette_and_chain() {
        let (c, _) = shipped();
        let groupsets: Vec<&Part> = c
            .parts
            .iter()
            .filter(|p| p.slot == Slot::Groupset && !p.is_base())
            .collect();
        assert_eq!(groupsets.len(), 9);
        for g in groupsets {
            let inc = c.included(g);
            let slots: Vec<Slot> = inc.iter().map(|p| p.slot).collect();
            assert_eq!(
                slots,
                vec![Slot::Crankset, Slot::Cassette, Slot::Chain],
                "{}",
                g.id
            );
            for p in inc {
                assert!(c.is_included(&p.id));
                assert_eq!(p.hearts, 0, "{}", p.id);
                assert!(!p.is_base(), "{}", p.id);
                assert!(g.fits.iter().all(|b| p.fits(*b)), "{} on {}", p.id, g.id);
            }
        }
        let top = c.get("groupset.road-electronic.top").unwrap();
        assert_eq!(
            top.includes,
            [
                "crankset.road-race.top",
                "cassette.road-race.top",
                "chain.12-speed-race.top"
            ]
        );
        // The power-meter crankset is sold alone.
        assert!(!c.is_included("crankset.road.power"));
        assert!(c.included(c.get("groupset.road.base").unwrap()).is_empty());
        // Broken: a groupset without a chain, a heavier included part, a
        // priced one, a groupset including a tyre.
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "groupset.mtb.top")
                .unwrap()
                .includes
                .pop();
        });
        assert!(
            has(&e, "groupset.mtb.top: includes exactly one chain"),
            "{e:?}"
        );
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "cassette.mtb-12.top")
                .unwrap()
                .weight_g = 380
        });
        assert!(has(&e, "its weight is in the groupset's"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "chain.11-speed.entry")
                .unwrap()
                .hearts = 30
        });
        assert!(has(&e, "comes with a groupset: free"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "groupset.gravel.mid")
                .unwrap()
                .includes
                .push("tyres.gravel.fast".into());
        });
        assert!(has(&e, "crankset, cassette or chain only"), "{e:?}");
        let e = broken(|c| {
            c.parts
                .iter_mut()
                .find(|p| p.id == "groupset.gravel.mid")
                .unwrap()
                .includes
                .clear();
        });
        assert!(has(&e, "a groupset includes its crankset"), "{e:?}");
    }

    #[test]
    fn old_sources_warn_at_12_months_and_fail_at_18() {
        let (c, s) = shipped();
        let p = validate(&c, &s, (2027, 12, 1));
        assert!(errors(&p).is_empty(), "{p:#?}");
        assert!(p.iter().any(|p| p.severity == Severity::Warning));
        let p = validate(&c, &s, (2028, 5, 13));
        assert!(!errors(&p).is_empty(), "18 months");
        assert!(errors(&p)[0].message.contains("re-research"));
        assert_eq!(months_between((2026, 11, 13), (2028, 5, 12)), 17);
        assert_eq!(months_between((2026, 11, 13), (2028, 5, 13)), 18);
    }

    #[test]
    fn reprice_rewrites_only_stale_hearts_and_keeps_the_rest() {
        let (_, s) = shipped();
        // The shipped file is a fixed point.
        let (same, changes) = reprice(crate::shipped::PARTS, &s);
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(same, crate::shipped::PARTS);
        let text = "# top comment\n[[part]]\nid = \"lights.set\"\nhearts = 70   # old price\nsources = \"lights.set\"\n\n[[part]]\nid = \"bag.none.base\"\nhearts = 0\n";
        let (new, changes) = reprice(text, &s);
        assert_eq!(
            changes,
            [Change {
                id: "lights.set".into(),
                old: 70,
                new: 80
            }]
        );
        assert_eq!(new, text.replace("hearts = 70   #", "hearts = 80   #"));
    }

    #[test]
    fn malformed_files_are_reported() {
        let e = Catalogue::parse(
            "[[part]]\nid = \"x.y\"\nslot = \"wings\"\ntier = \"gold\"\nname = \"n\"\nhearts = 1\nweight_g = 1\nfits = [\"bmx\"]\nmodel = \"m\"\n",
        )
        .unwrap_err();
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(Catalogue::parse("[[part]]\nid = 1\n").is_err());
        assert!(Sources::parse("[[source]]\nkey = \"k\"\nchecked = \"soon\"\n").is_err());
        assert_eq!(Slot::parse("BAR_TAPE"), Some(Slot::BarTape));
        assert_eq!(Tier::parse("upper"), Some(Tier::Upper));
        assert_eq!(parse_date("2026-11-13"), Some((2026, 11, 13)));
        assert_eq!(parse_date("2026-13-01"), None);
    }

    /// The kit (spec 4C, W4): every apparel price is ceil(median) of its
    /// researched references; the base kit is free and complete; kit fits
    /// every bike; no brand in names, ids or models; the researched price
    /// table of the spec.
    #[test]
    fn the_kit_is_priced_from_its_research_and_the_base_kit_is_free() {
        let (c, s) = shipped();
        for slot in Slot::KIT {
            let base = c.base_kit(slot).expect("a free base item");
            assert_eq!(base.hearts, 0, "{}", base.id);
            assert_eq!(base.model, STOCK_MODEL);
            let items = c.kit(slot);
            assert_eq!(items[0].id, base.id, "base first");
            assert!(items.len() >= 2, "{slot:?} has something to buy");
            for p in items {
                assert!(BikeType::ALL.iter().all(|b| p.fits(*b)), "{}", p.id);
                assert!(brand_in(&p.name).is_none() && brand_in(&p.id).is_none());
                if !p.is_base() {
                    let key = p.sources.as_deref().unwrap();
                    let src = s.get(key).unwrap();
                    assert!(src.groups[0].refs.len() >= 2, "{key}: a few references");
                    assert_eq!(Some(p.hearts), s.hearts_of(key), "{}", p.id);
                    assert!(crate::rider::variant(&p.model).is_some(), "{}", p.model);
                }
            }
        }
        let price = |id: &str| c.get(id).map(|p| p.hearts);
        for (id, h) in [
            ("jersey.thermal.mid", 130),
            ("jersey.aero.upper", 210),
            ("jersey.skinsuit.top", 430),
            ("shorts.tights.mid", 180),
            ("shorts.aero.upper", 250),
            ("socks.tall.entry", 20),
            ("socks.aero.upper", 34),
            ("shoes.race.top", 400),
            ("shoes.overshoes.mid", 70),
            ("gloves.gel.entry", 30),
            ("gloves.full.mid", 60),
            ("gloves.winter.upper", 108),
            ("helmet.light.top", 320),
            ("helmet.aero-road.top", 268),
            ("helmet.tt.top", 500),
            ("cap.cotton.entry", 30),
        ] {
            assert_eq!(price(id), Some(h), "{id}");
        }
        // The skinsuit covers the bib shorts.
        assert_eq!(
            c.get("jersey.skinsuit.top").unwrap().covers,
            vec![Slot::Shorts]
        );
        // Kit slots parse, and are not bike slots.
        for slot in Slot::KIT {
            assert_eq!(Slot::parse(slot.id()), Some(slot));
            assert!(slot.is_kit() && !Slot::ALL.contains(&slot));
        }
        assert!(brand_in("Kask Bambino Pro").is_some());
        assert!(brand_in("Time-trial helmet").is_none());
    }

    #[test]
    fn the_validator_catches_broken_kit() {
        let edit = |id: &'static str, f: fn(&mut Part)| {
            broken(move |c| f(c.parts.iter_mut().find(|p| p.id == id).unwrap()))
        };
        let e = edit("helmet.tt.top", |p| {
            p.fits.retain(|b| *b != BikeType::Mountain)
        });
        assert!(has(&e, "kit fits every bike type"), "{e:?}");
        let e = edit("gloves.full.mid", |p| p.aero = 1);
        assert!(has(&e, "comfort items"), "{e:?}");
        let e = edit("helmet.tt.top", |p| p.aero = 5);
        assert!(has(&e, "kit classes"), "{e:?}");
        let e = edit("socks.aero.upper", |p| p.rolling = -1);
        assert!(has(&e, "no rolling class"), "{e:?}");
        let e = edit("helmet.tt.top", |p| p.model = "tyre-road-28".into());
        assert!(has(&e, "unknown rider variant"), "{e:?}");
        let e = edit("helmet.tt.top", |p| p.model = "shoes-race".into());
        assert!(has(&e, "is a shoes variant"), "{e:?}");
        let e = edit("cap.none.base", |p| p.hearts = 5);
        assert!(has(&e, "a base part costs 0 hearts"), "{e:?}");
        let e = edit("jersey.skinsuit.top", |p| p.covers = vec![Slot::Jersey]);
        assert!(has(&e, "cannot cover its own slot"), "{e:?}");
        let e = broken(|c| c.parts.retain(|p| p.id != "helmet.road.base"));
        assert!(has(&e, "base items of kit slot helmet"), "{e:?}");
        let e = edit("helmet.tt.top", |p| {
            p.name = "Kask time-trial helmet".into()
        });
        assert!(has(&e, "brand name \"Kask\""), "{e:?}");
    }
}
