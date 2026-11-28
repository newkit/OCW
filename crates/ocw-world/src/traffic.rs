//! Road traffic rules from the package (spec 14 Phase 18, 18B *Data from
//! the package*): what motor vehicles may do on each road-graph edge –
//! which vehicles may drive there, one-way, lanes, the speed limit (tagged,
//! or the country's implied limit), weight and height limits, parking at
//! the kerb, roundabouts, overtaking – plus the signals, stop and give-way
//! signs along the edges, the bus stops and bus lines, and the car parks.
//! Pure data, wasm-safe; written by `world-builder` (`ocw-worldgen::traffic`),
//! simulated by `ocw-sim::traffic` (T3).
//!
//! **Section `TRFC`** (optional, additive; older readers skip it; a package
//! without it reads as "no traffic data": [`EdgeRules::implied`] gives the
//! rules by road class and the implied limits of the package's country,
//! see [`RoadRules`]):
//!
//! ```text
//! layout version: u8 (1)
//! package country: [u8; 2] (ISO 3166-1 alpha-2, "ZZ" unknown)
//! edge count: u32, per edge (28 bytes, sorted by edge id):
//!   edge id: u64   access: u8 (Access bits)   flags: u16 (EdgeFlags)
//!   lanes forward, backward: u8 u8
//!   maxspeed forward, backward: u16 u16 (SpeedLimit::code)
//!   speed source: u8   country: [u8; 2]
//!   maxweight: u16 (0.1 t, 0 none)   maxheight: u16 (cm, 0 none)
//!   parking left, right: u8 u8 (Parking::code)
//!   flow: u16 (1/100; 0 = no factor, read as 1 – packages before T11
//!   wrote 100 everywhere)
//! control count: u32, per control (sorted by edge, along):
//!   edge id: u64   along: f32 (m from the edge's `from`)   kind: u8
//!   applies: u8   at junction: u8   OSM node: i64
//! stop count: u32, per bus stop:
//!   OSM node: i64   x, z: f32 f32   name: str   edge: u64 (0 none)
//!   along: f32   side: u8
//! route count: u32, per bus line:
//!   OSM relation: i64   ref: str   name: str
//!   leg count: u32, per leg: edge id: u64, forward: u8
//!   stop count: u32, per stop: index into the stops: u32
//! parking count: u32, per car park:
//!   OSM id: i64   x, z: f32 f32   capacity: u16   flags: u8
//!   kind: u8   edge: u64 (0 none)   along: f32
//! str = u16 length + UTF-8
//! ```
//!
//! Directions: *forward* is along the edge from `from` to `to` (the graph's
//! orientation, not the OSM way's – the builder turns the way's tags
//! round for an edge that runs against its way); *left* and *right* are
//! seen when driving forward.

use std::collections::BTreeMap;

use crate::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph};
use crate::package::Bytes;

/// Section tag.
pub const TRAFFIC_TAG: &[u8; 4] = b"TRFC";
const TRAFFIC_VERSION: u8 = 1;

/// Walking pace for `maxspeed=walk` and the German living street (km/h;
/// "Schrittgeschwindigkeit", courts read it as 4 – 7 km/h).
pub const WALK_KMH: f32 = 7.0;

// ---------------------------------------------------------------------------
// Countries and their implied limits
// ---------------------------------------------------------------------------

/// The country whose traffic law applies (implied speed limits, spec 18B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Country {
    /// Germany.
    De,
    /// Switzerland.
    Ch,
    /// Liechtenstein (the Swiss limits).
    Li,
    /// Austria.
    At,
    /// Spain.
    Es,
    /// Andorra.
    Ad,
    /// France.
    Fr,
    /// Anywhere else (generic 50 / 90 / 120).
    #[default]
    Other,
}

impl Country {
    pub const ALL: [Country; 8] = [
        Self::De,
        Self::Ch,
        Self::Li,
        Self::At,
        Self::Es,
        Self::Ad,
        Self::Fr,
        Self::Other,
    ];

    /// ISO 3166-1 alpha-2 code ("ZZ" for [`Country::Other`]).
    pub fn code(self) -> &'static str {
        match self {
            Self::De => "DE",
            Self::Ch => "CH",
            Self::Li => "LI",
            Self::At => "AT",
            Self::Es => "ES",
            Self::Ad => "AD",
            Self::Fr => "FR",
            Self::Other => "ZZ",
        }
    }

    /// From an ISO 3166-1 alpha-2 code (any case; also the country part of
    /// an ISO 3166-2 code such as "DE-BW" or "ES-CN").
    pub fn from_code(code: &str) -> Self {
        let c = code.trim().get(..2).unwrap_or("").to_ascii_uppercase();
        Self::ALL
            .into_iter()
            .find(|k| k.code() == c)
            .unwrap_or(Self::Other)
    }

    /// The country of an IANA time zone (the package's `TZID`): its main
    /// country, for packages without country outlines.
    pub fn from_time_zone(zone: &str) -> Self {
        match zone {
            "Europe/Berlin" | "Europe/Busingen" => Self::De,
            "Europe/Zurich" => Self::Ch,
            "Europe/Vaduz" => Self::Li,
            "Europe/Vienna" => Self::At,
            "Europe/Madrid" | "Atlantic/Canary" | "Africa/Ceuta" => Self::Es,
            "Europe/Andorra" => Self::Ad,
            "Europe/Paris" => Self::Fr,
            _ => Self::Other,
        }
    }

    /// The implied limit (no `maxspeed` tag) in `setting`;
    /// `lanes_per_direction` matters in Spain's towns only (RD 970/2020:
    /// 30 km/h with one lane per direction, 50 with two or more).
    ///
    /// | | urban | rural | motorway | living street |
    /// |---|---|---|---|---|
    /// | DE | 50 | 100 | none (130 advised) | walking pace |
    /// | CH, LI | 50 | 80 | 120 | 20 |
    /// | AT | 50 | 100 | 130 | walking pace |
    /// | ES | 30 / 50 | 90 | 120 | 20 |
    /// | AD | 50 | 90 | 90 | 20 |
    /// | FR | 50 | 80 | 130 | 20 |
    /// | other | 50 | 90 | 120 | 20 |
    pub fn implied(self, setting: Setting, lanes_per_direction: u8) -> SpeedLimit {
        use SpeedLimit::{Kmh, Unlimited, Walk};
        match (self, setting) {
            (Self::De, Setting::Urban) => Kmh(50),
            (Self::De, Setting::Rural) => Kmh(100),
            (Self::De, Setting::Motorway) => Unlimited { advisory_kmh: 130 },
            (Self::De | Self::At, Setting::LivingStreet) => Walk,
            (Self::Ch | Self::Li, Setting::Urban) => Kmh(50),
            (Self::Ch | Self::Li, Setting::Rural) => Kmh(80),
            (Self::Ch | Self::Li, Setting::Motorway) => Kmh(120),
            (Self::At, Setting::Urban) => Kmh(50),
            (Self::At, Setting::Rural) => Kmh(100),
            (Self::At, Setting::Motorway) => Kmh(130),
            (Self::Es, Setting::Urban) if lanes_per_direction >= 2 => Kmh(50),
            (Self::Es, Setting::Urban) => Kmh(30),
            (Self::Es, Setting::Rural) => Kmh(90),
            (Self::Es, Setting::Motorway) => Kmh(120),
            (Self::Ad, Setting::Urban) => Kmh(50),
            (Self::Ad, Setting::Rural | Setting::Motorway) => Kmh(90),
            (Self::Fr, Setting::Urban) => Kmh(50),
            (Self::Fr, Setting::Rural) => Kmh(80),
            (Self::Fr, Setting::Motorway) => Kmh(130),
            (Self::Other, Setting::Urban) => Kmh(50),
            (Self::Other, Setting::Rural) => Kmh(90),
            (Self::Other, Setting::Motorway) => Kmh(120),
            (_, Setting::LivingStreet) => Kmh(20),
        }
    }
}

/// Where a road is, for its implied limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    /// Inside a built-up area (in town).
    Urban,
    /// Outside built-up areas.
    Rural,
    /// A motorway (`highway=motorway`; not in the riders' graph today).
    Motorway,
    /// A living street (`highway=living_street`; DE "verkehrsberuhigter
    /// Bereich", CH "Begegnungszone").
    LivingStreet,
}

// ---------------------------------------------------------------------------
// Speed limits
// ---------------------------------------------------------------------------

/// A speed limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeedLimit {
    /// A limit in km/h.
    Kmh(u16),
    /// Walking pace (`maxspeed=walk`), [`WALK_KMH`].
    Walk,
    /// No limit (`maxspeed=none`, German motorways), with the advised speed.
    Unlimited { advisory_kmh: u16 },
}

impl SpeedLimit {
    /// The speed to drive at most (km/h): the limit, walking pace, or the
    /// advised speed where there is no limit.
    pub fn kmh(self) -> f32 {
        match self {
            Self::Kmh(v) => f32::from(v),
            Self::Walk => WALK_KMH,
            Self::Unlimited { advisory_kmh } => f32::from(advisory_kmh),
        }
    }

    /// [`Self::kmh`] in m/s.
    pub fn ms(self) -> f32 {
        self.kmh() / 3.6
    }

    /// The stored code: km/h 1 – 32,767; 0xFFFF walk; 0x8000 | advised
    /// km/h for no limit.
    pub fn code(self) -> u16 {
        match self {
            Self::Kmh(v) => v.clamp(1, 0x7FFF),
            Self::Walk => 0xFFFF,
            Self::Unlimited { advisory_kmh } => 0x8000 | advisory_kmh.min(0x7FFE),
        }
    }

    pub fn from_code(c: u16) -> Self {
        match c {
            0xFFFF => Self::Walk,
            c if c & 0x8000 != 0 => Self::Unlimited {
                advisory_kmh: c & 0x7FFF,
            },
            c => Self::Kmh(c.max(1)),
        }
    }

    /// An OSM `maxspeed` value: "50", "50 km/h", "30 mph", "10 knots",
    /// "walk", "none"; several values ("50;30") give the lowest. `None`
    /// for "signals", "variable", country codes ("DE:urban", see
    /// [`parse_typed`]) and anything else.
    pub fn parse(value: &str) -> Option<Self> {
        let mut best: Option<Self> = None;
        for v in value.split(';') {
            let v = v.trim();
            let s = match v {
                "walk" => Some(Self::Walk),
                "none" => Some(Self::Unlimited { advisory_kmh: 130 }),
                _ => {
                    let (num, factor) = if let Some(n) = v.strip_suffix("mph") {
                        (n, 1.609_344)
                    } else if let Some(n) = v.strip_suffix("knots") {
                        (n, 1.852)
                    } else {
                        (v.trim_end_matches("km/h").trim_end_matches("kmh"), 1.0)
                    };
                    num.trim()
                        .parse::<f32>()
                        .ok()
                        .filter(|k| (1.0..=400.0).contains(k))
                        .map(|k| Self::Kmh((k * factor).round() as u16))
                }
            };
            if let Some(s) = s
                && best.is_none_or(|b| s.kmh() < b.kmh())
            {
                best = Some(s);
            }
        }
        best
    }
}

/// What a typed limit says: "DE:urban", "CH:rural", "ES:motorway",
/// "DE:living_street", "DE:zone30" / "DE:zone:30", "DE:bicycle_road",
/// and `zone:maxspeed` values such as "DE:30". Returns the setting (if
/// it names one) and the limit (if it names one directly: zones, bicycle
/// roads, "DE:30").
pub fn parse_typed(value: &str) -> Option<(Country, Option<Setting>, Option<SpeedLimit>)> {
    let (cc, rest) = value.trim().split_once(':')?;
    if cc.len() != 2 || !cc.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let country = Country::from_code(cc);
    let rest = rest.trim().to_ascii_lowercase();
    let setting = match rest.as_str() {
        "urban" => Some(Setting::Urban),
        "rural" => Some(Setting::Rural),
        "motorway" => Some(Setting::Motorway),
        "living_street" => Some(Setting::LivingStreet),
        _ => None,
    };
    if setting.is_some() {
        return Some((country, setting, None));
    }
    // Swiss and German motorways for cars only ("Autostrasse",
    // "Kraftfahrstraße"), Spanish dual carriageways.
    if matches!(rest.as_str(), "trunk" | "motorroad" | "expressway") {
        let kmh = match country {
            Country::Ch | Country::Li | Country::De | Country::Es => 100,
            Country::At => 100,
            Country::Fr => 110,
            _ => 100,
        };
        return Some((country, Some(Setting::Rural), Some(SpeedLimit::Kmh(kmh))));
    }
    if rest == "bicycle_road" || rest == "cyclestreet" {
        return Some((country, Some(Setting::Urban), Some(SpeedLimit::Kmh(30))));
    }
    let digits = rest
        .trim_start_matches("zone")
        .trim_start_matches(':')
        .trim_start_matches('-');
    if let Ok(v) = digits.parse::<u16>()
        && (5..=130).contains(&v)
    {
        let zone = rest.starts_with("zone") || v <= 30;
        return Some((
            country,
            zone.then_some(Setting::Urban),
            Some(SpeedLimit::Kmh(v)),
        ));
    }
    None
}

/// Where an edge's limit comes from (`world-builder lint` reports the
/// roads without speed data: [`SpeedSource::Implied`] and
/// [`SpeedSource::Default`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SpeedSource {
    /// A `maxspeed` (or `maxspeed:forward` / `:backward`) value.
    Tagged = 0,
    /// A typed limit: `maxspeed:type`, `source:maxspeed`, `zone:maxspeed`
    /// or a `maxspeed` like "DE:urban".
    Typed = 1,
    /// Untagged: the country's implied limit for the setting.
    #[default]
    Implied = 2,
    /// Untagged, in no known country: the generic limits.
    Default = 3,
}

impl SpeedSource {
    fn from_code(c: u8) -> Self {
        match c {
            0 => Self::Tagged,
            1 => Self::Typed,
            3 => Self::Default,
            _ => Self::Implied,
        }
    }

    /// Whether a mapper said what the limit is.
    pub fn is_mapped(self) -> bool {
        matches!(self, Self::Tagged | Self::Typed)
    }
}

// ---------------------------------------------------------------------------
// Vehicles and access
// ---------------------------------------------------------------------------

/// The kinds of road vehicles of spec 18B, by the access they need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vehicle {
    /// Cars and vans (`motorcar`).
    Car,
    /// Lorries (`hgv`).
    Hgv,
    /// Buses (`bus`, `psv`).
    Bus,
    /// Motorbikes (`motorcycle`).
    Motorcycle,
    /// Tractors (`agricultural`, `forestry`).
    Tractor,
}

/// Which motor vehicles may use an edge (bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Access(pub u8);

impl Access {
    pub const CAR: u8 = 1;
    pub const HGV: u8 = 2;
    pub const BUS: u8 = 4;
    pub const MOTORCYCLE: u8 = 8;
    pub const TRACTOR: u8 = 16;
    /// No motor vehicles (cycleways, paths, pedestrian streets).
    pub const NONE: Access = Access(0);
    /// Every motor vehicle.
    pub const ALL: Access = Access(31);

    fn bit(v: Vehicle) -> u8 {
        match v {
            Vehicle::Car => Self::CAR,
            Vehicle::Hgv => Self::HGV,
            Vehicle::Bus => Self::BUS,
            Vehicle::Motorcycle => Self::MOTORCYCLE,
            Vehicle::Tractor => Self::TRACTOR,
        }
    }

    pub fn allows(self, v: Vehicle) -> bool {
        self.0 & Self::bit(v) != 0
    }

    pub fn with(self, v: Vehicle, on: bool) -> Self {
        if on {
            Self(self.0 | Self::bit(v))
        } else {
            Self(self.0 & !Self::bit(v))
        }
    }

    /// Any motor vehicle at all.
    pub fn any(self) -> bool {
        self.0 != 0
    }
}

/// An OSM access value: `Some((allowed, destination only))`, `None` if it
/// says nothing for this vehicle (unknown values). Lists ("agricultural;
/// forestry") allow tractors only.
fn access_value(value: &str, v: Vehicle) -> Option<(bool, bool)> {
    let mut out: Option<(bool, bool)> = None;
    for part in value.split(';') {
        let r = match part.trim() {
            "yes" | "designated" | "permissive" | "official" | "permit" => Some((true, false)),
            "destination" | "delivery" | "customers" => Some((true, true)),
            "agricultural" | "forestry" => Some((v == Vehicle::Tractor, false)),
            "no" | "private" | "use_sidepath" | "military" => Some((false, false)),
            "discouraged" => Some((true, false)),
            _ => None,
        };
        // The most permissive part wins ("agricultural;forestry" →
        // tractors yes, cars no).
        if let Some(r) = r {
            out = Some(match out {
                Some(o) if o.0 && !r.0 => o,
                Some(o) if o.0 && r.0 => (true, o.1 && r.1),
                _ => r,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parking at the kerb
// ---------------------------------------------------------------------------

/// Where cars park on one side of a road (`parking:left|right|both`,
/// the older `parking:lane:*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParkingPlace {
    /// Not tagged (the core may guess by the road: parked cars in town).
    #[default]
    Unknown = 0,
    /// No parking (`no`, `no_parking`, `no_stopping`, `fire_lane`).
    No = 1,
    /// On the carriageway (`lane`, `on_street`, `parallel`, …).
    Lane = 2,
    /// In bays beside the carriageway (`street_side`, `lay_by`).
    StreetSide = 3,
    /// On the kerb (`on_kerb`).
    OnKerb = 4,
    /// Half on the kerb (`half_on_kerb`).
    HalfOnKerb = 5,
    /// On the shoulder (`shoulder`).
    Shoulder = 6,
    /// Mapped as its own area (`separate`).
    Separate = 7,
}

/// How parked cars stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParkingOrientation {
    #[default]
    Parallel = 0,
    Diagonal = 1,
    Perpendicular = 2,
}

/// Parking on one side of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Parking {
    pub place: ParkingPlace,
    pub orientation: ParkingOrientation,
}

impl Parking {
    pub const UNKNOWN: Parking = Parking {
        place: ParkingPlace::Unknown,
        orientation: ParkingOrientation::Parallel,
    };

    /// Whether parked cars stand along the road here (for the static
    /// parked cars of 18B).
    pub fn has_cars(self) -> bool {
        matches!(
            self.place,
            ParkingPlace::Lane
                | ParkingPlace::StreetSide
                | ParkingPlace::OnKerb
                | ParkingPlace::HalfOnKerb
                | ParkingPlace::Shoulder
        )
    }

    /// Whether parked cars take room on the carriageway.
    pub fn on_carriageway(self) -> bool {
        matches!(self.place, ParkingPlace::Lane | ParkingPlace::HalfOnKerb)
    }

    pub fn code(self) -> u8 {
        (self.place as u8) | ((self.orientation as u8) << 4)
    }

    pub fn from_code(c: u8) -> Self {
        let place = match c & 0x0F {
            1 => ParkingPlace::No,
            2 => ParkingPlace::Lane,
            3 => ParkingPlace::StreetSide,
            4 => ParkingPlace::OnKerb,
            5 => ParkingPlace::HalfOnKerb,
            6 => ParkingPlace::Shoulder,
            7 => ParkingPlace::Separate,
            _ => ParkingPlace::Unknown,
        };
        let orientation = match c >> 4 {
            1 => ParkingOrientation::Diagonal,
            2 => ParkingOrientation::Perpendicular,
            _ => ParkingOrientation::Parallel,
        };
        Self { place, orientation }
    }

    fn orientation_of(v: &str) -> Option<ParkingOrientation> {
        match v.trim() {
            "parallel" => Some(ParkingOrientation::Parallel),
            "diagonal" => Some(ParkingOrientation::Diagonal),
            "perpendicular" => Some(ParkingOrientation::Perpendicular),
            _ => None,
        }
    }

    /// The parking on side `side` ("left" | "right") of an OSM way: the
    /// current scheme (`parking:SIDE`, `parking:both`, with
    /// `:orientation`) wins over the old one (`parking:lane:SIDE`,
    /// `parking:lane:both`, with `:parallel` etc. for the position).
    pub fn from_osm<'a>(side: &str, tag: impl Fn(&str) -> Option<&'a str>) -> Self {
        let pick = |pre: &str, suffix: &str| {
            tag(&format!("{pre}:{side}{suffix}")).or_else(|| tag(&format!("{pre}:both{suffix}")))
        };
        if let Some(v) = pick("parking", "") {
            let place = match v.trim() {
                "lane" | "yes" => ParkingPlace::Lane,
                "street_side" => ParkingPlace::StreetSide,
                "on_kerb" => ParkingPlace::OnKerb,
                "half_on_kerb" => ParkingPlace::HalfOnKerb,
                "shoulder" => ParkingPlace::Shoulder,
                "no" => ParkingPlace::No,
                "separate" => ParkingPlace::Separate,
                _ => ParkingPlace::Unknown,
            };
            let orientation = pick("parking", ":orientation")
                .and_then(Self::orientation_of)
                .unwrap_or_default();
            return Self { place, orientation };
        }
        if let Some(v) = pick("parking:lane", "") {
            let v = v.trim();
            if let Some(orientation) = Self::orientation_of(v) {
                let pos = pick("parking:lane", &format!(":{v}"));
                let place = match pos.map(str::trim) {
                    Some("half_on_kerb") => ParkingPlace::HalfOnKerb,
                    Some("on_kerb") => ParkingPlace::OnKerb,
                    Some("street_side" | "lay_by") => ParkingPlace::StreetSide,
                    Some("shoulder") => ParkingPlace::Shoulder,
                    _ => ParkingPlace::Lane,
                };
                return Self { place, orientation };
            }
            let place = match v {
                "marked" | "yes" => ParkingPlace::Lane,
                "no_parking" | "no_stopping" | "fire_lane" | "no" => ParkingPlace::No,
                "separate" => ParkingPlace::Separate,
                _ => ParkingPlace::Unknown,
            };
            return Self {
                place,
                orientation: ParkingOrientation::Parallel,
            };
        }
        Self::UNKNOWN
    }
}

// ---------------------------------------------------------------------------
// The rules of an edge
// ---------------------------------------------------------------------------

/// What motor traffic may do on one road-graph edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRules {
    /// Which motor vehicles may drive here ([`Access::NONE`]: no traffic –
    /// cycleways, paths, pedestrian streets, `motor_vehicle=no`).
    pub access: Access,
    /// Only for access to places along it (`destination`, `delivery`).
    pub destination_only: bool,
    /// One-way for motor vehicles, along the edge (`oneway`, roundabouts;
    /// `oneway:bicycle=no` does not lift it – that only frees riders).
    pub oneway: Oneway,
    /// Motor lanes forward / backward (0 against a one-way).
    pub lanes: [u8; 2],
    /// One lane for both directions (`lanes=1` on a two-way road, or a
    /// narrow untagged road): oncoming vehicles must wait or squeeze by.
    pub single_track: bool,
    /// The limit forward / backward.
    pub maxspeed: [SpeedLimit; 2],
    pub speed_source: SpeedSource,
    /// The country whose law applies.
    pub country: Country,
    /// In a built-up area (from `place` areas, land use and typed limits).
    pub urban: bool,
    /// `maxweight` (t) and `maxheight` (m): lorries above them stay out.
    pub maxweight_t: Option<f32>,
    pub maxheight_m: Option<f32>,
    /// Parking at the kerb on the left / right (driving forward).
    pub parking: [Parking; 2],
    /// Part of a roundabout (`junction=roundabout | circular`).
    pub roundabout: bool,
    /// Overtaking forbidden forward / backward (`overtaking=no`).
    pub no_overtaking: [bool; 2],
    /// Demand factor of the road: its annual average daily traffic against
    /// the class's default (open counts, `world-builder traffic-counts`,
    /// spec 18E T11; 1 without counts), 0.01 – 655.
    pub flow: f32,
}

/// A measure such as "7.5", "7.5 t", "3500 kg", "3.8 m" in the unit of
/// the first factor-1 entry of `units`.
fn measure(value: Option<&str>, units: &[(&str, f64)]) -> Option<f32> {
    let v = value?.trim();
    let (num, scale) = units
        .iter()
        .find_map(|(u, s)| v.strip_suffix(u).map(|n| (n.trim(), *s)))
        .unwrap_or((v, 1.0));
    num.parse::<f64>()
        .ok()
        .map(|n| (n * scale) as f32)
        .filter(|n| *n > 0.0 && *n < 1000.0)
}

/// The flag bits of a stored edge.
mod flags {
    pub const ONEWAY_FWD: u16 = 1;
    pub const ONEWAY_BWD: u16 = 2;
    pub const ROUNDABOUT: u16 = 4;
    pub const URBAN: u16 = 8;
    pub const DESTINATION: u16 = 16;
    pub const NO_OVERTAKING_FWD: u16 = 32;
    pub const NO_OVERTAKING_BWD: u16 = 64;
    pub const SINGLE_TRACK: u16 = 128;
}

/// The OSM tags [`EdgeRules::from_osm`] reads (the builder keeps only
/// these of the road ways for it).
pub const RULE_KEYS: &[&str] = &[
    "highway",
    "junction",
    "oneway",
    "oneway:motor_vehicle",
    "oneway:vehicle",
    "lanes",
    "lanes:forward",
    "lanes:backward",
    "maxspeed",
    "maxspeed:forward",
    "maxspeed:backward",
    "maxspeed:type",
    "source:maxspeed",
    "zone:maxspeed",
    "access",
    "vehicle",
    "motor_vehicle",
    "motorcar",
    "hgv",
    "bus",
    "psv",
    "motorcycle",
    "agricultural",
    "forestry",
    "maxweight",
    "maxheight",
    "overtaking",
    "overtaking:forward",
    "overtaking:backward",
];

/// Where an edge lies and how it runs, for [`EdgeRules::from_osm`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeContext {
    pub class: RoadClass,
    /// The edge runs against its OSM way (its `from` is the way's later
    /// node): forward / backward and left / right tags are turned round.
    pub reversed: bool,
    /// The country at the edge.
    pub country: Country,
    /// In a built-up area by the builder's test (place areas, land use);
    /// a typed limit can say otherwise.
    pub urban: bool,
    /// Carriageway width (m).
    pub width_m: f64,
}

impl EdgeRules {
    /// The rules of an edge of `class` without any tags (packages without
    /// `TRFC`, spec 18B: "traffic only on roads by class with implied
    /// speeds"): cars on roads, tractors only on tracks, nothing on
    /// cycleways and paths; residential streets count as in town.
    pub fn implied(class: RoadClass, country: Country, urban: bool) -> Self {
        let tag = |_: &str| -> Option<&str> { None };
        Self::from_osm(
            &EdgeContext {
                class,
                reversed: false,
                country,
                urban,
                width_m: class.default_width_m(),
            },
            tag,
        )
    }

    /// The rules from an OSM way's tags (see [`RULE_KEYS`]).
    pub fn from_osm<'a>(cx: &EdgeContext, tag: impl Fn(&str) -> Option<&'a str>) -> Self {
        let highway = tag("highway").map(str::trim);
        // --- Access -------------------------------------------------------
        let base = match cx.class {
            RoadClass::Cycleway | RoadClass::Path => Access::NONE,
            RoadClass::Track => Access(Access::TRACTOR),
            _ if highway == Some("pedestrian") => Access::NONE,
            _ => Access::ALL,
        };
        let mut access = base;
        let mut destination_only = false;
        let vehicles = [
            (Vehicle::Car, &["motorcar"][..]),
            (Vehicle::Hgv, &["hgv"][..]),
            (Vehicle::Bus, &["psv", "bus"][..]),
            (Vehicle::Motorcycle, &["motorcycle"][..]),
            (Vehicle::Tractor, &["agricultural", "forestry"][..]),
        ];
        for (v, own) in vehicles {
            let mut allowed = base.allows(v);
            let mut dest = false;
            for key in ["access", "vehicle", "motor_vehicle"]
                .iter()
                .chain(own.iter())
            {
                if let Some((a, d)) = tag(key).and_then(|val| access_value(val, v)) {
                    allowed = a;
                    dest = d;
                }
            }
            access = access.with(v, allowed);
            if v == Vehicle::Car && allowed {
                destination_only = dest;
            }
        }
        // --- One-way ------------------------------------------------------
        let roundabout = matches!(
            tag("junction").map(str::trim),
            Some("roundabout" | "circular")
        );
        let ow = tag("oneway:motor_vehicle")
            .or_else(|| tag("oneway:vehicle"))
            .or_else(|| tag("oneway"))
            .map(str::trim);
        let mut oneway = match ow {
            Some("yes" | "true" | "1") => Oneway::Forward,
            Some("-1" | "reverse") => Oneway::Backward,
            Some(_) => Oneway::No,
            None if roundabout => Oneway::Forward,
            None if highway.is_some_and(|h| h == "motorway" || h == "motorway_link") => {
                Oneway::Forward
            }
            None => Oneway::No,
        };
        // --- Lanes --------------------------------------------------------
        let num = |k: &str| {
            tag(k)
                .and_then(|v| v.split(';').next()?.trim().parse::<u8>().ok())
                .filter(|&n| n <= 16)
        };
        let (total, mut fwd, mut bwd) = (num("lanes"), num("lanes:forward"), num("lanes:backward"));
        // --- Speed --------------------------------------------------------
        let mut sp_fwd = tag("maxspeed:forward").and_then(SpeedLimit::parse);
        let mut sp_bwd = tag("maxspeed:backward").and_then(SpeedLimit::parse);
        let mut no_ot_fwd = tag("overtaking:forward").map(str::trim) == Some("no");
        let mut no_ot_bwd = tag("overtaking:backward").map(str::trim) == Some("no");
        // Turn the way's directions round onto the edge.
        if cx.reversed {
            oneway = oneway.reversed();
            std::mem::swap(&mut fwd, &mut bwd);
            std::mem::swap(&mut sp_fwd, &mut sp_bwd);
            std::mem::swap(&mut no_ot_fwd, &mut no_ot_bwd);
        }
        let lanes = match oneway {
            Oneway::Forward => [fwd.or(total).unwrap_or(1).max(1), 0],
            Oneway::Backward => [0, bwd.or(total).unwrap_or(1).max(1)],
            Oneway::No => match (fwd, bwd, total) {
                (Some(f), Some(b), _) => [f, b],
                (Some(f), None, Some(t)) => [f, t.saturating_sub(f).max(1)],
                (None, Some(b), Some(t)) => [t.saturating_sub(b).max(1), b],
                (Some(f), None, None) => [f, 1],
                (None, Some(b), None) => [1, b],
                (None, None, Some(t)) if t >= 2 => [t / 2, t / 2],
                _ => [1, 1],
            },
        };
        let single_track = oneway == Oneway::No
            && (total == Some(1) || (total.is_none() && fwd.is_none() && cx.width_m < 4.0));
        let typed = [
            "maxspeed:type",
            "source:maxspeed",
            "zone:maxspeed",
            "maxspeed",
        ]
        .iter()
        .find_map(|k| tag(k).and_then(parse_typed));
        let mut urban = cx.urban
            || matches!(cx.class, RoadClass::Residential)
            || matches!(highway, Some("living_street" | "pedestrian"));
        let mut setting = if highway == Some("living_street") {
            Setting::LivingStreet
        } else if matches!(highway, Some("motorway" | "motorway_link")) {
            Setting::Motorway
        } else if urban {
            Setting::Urban
        } else {
            Setting::Rural
        };
        let mut typed_limit = None;
        let mut country = cx.country;
        if let Some((c, s, l)) = typed {
            if c != Country::Other {
                country = c;
            }
            if let Some(s) = s {
                setting = s;
                urban = matches!(s, Setting::Urban | Setting::LivingStreet);
            }
            typed_limit = l;
        }
        let per_dir = lanes[0].max(lanes[1]);
        let implied = typed_limit.unwrap_or_else(|| country.implied(setting, per_dir));
        let tagged = tag("maxspeed").and_then(SpeedLimit::parse);
        let source = if tagged.is_some() || sp_fwd.is_some() || sp_bwd.is_some() {
            SpeedSource::Tagged
        } else if typed.is_some() {
            SpeedSource::Typed
        } else if country == Country::Other {
            SpeedSource::Default
        } else {
            SpeedSource::Implied
        };
        let both = tagged.unwrap_or(implied);
        // `maxspeed=none` keeps the country's advised speed.
        let advised = |s: SpeedLimit| match s {
            SpeedLimit::Unlimited { .. } => match country.implied(Setting::Motorway, 2) {
                u @ SpeedLimit::Unlimited { .. } => u,
                SpeedLimit::Kmh(v) => SpeedLimit::Unlimited { advisory_kmh: v },
                SpeedLimit::Walk => s,
            },
            s => s,
        };
        let maxspeed = [
            advised(sp_fwd.unwrap_or(both)),
            advised(sp_bwd.unwrap_or(both)),
        ];
        // --- Limits for lorries ------------------------------------------
        let maxweight_t = measure(
            tag("maxweight"),
            &[("kg", 0.001), ("st", 0.907), ("t", 1.0)],
        );
        let maxheight_m = measure(tag("maxheight"), &[("m", 1.0)]);
        let ot = tag("overtaking").map(str::trim) == Some("no");
        let parking = if cx.reversed {
            [
                Parking::from_osm("right", &tag),
                Parking::from_osm("left", &tag),
            ]
        } else {
            [
                Parking::from_osm("left", &tag),
                Parking::from_osm("right", &tag),
            ]
        };
        Self {
            access,
            destination_only,
            oneway,
            lanes,
            single_track,
            maxspeed,
            speed_source: source,
            country,
            urban,
            maxweight_t,
            maxheight_m,
            parking,
            roundabout,
            no_overtaking: [ot || no_ot_fwd, ot || no_ot_bwd],
            flow: 1.0,
        }
    }

    /// Whether `v` may drive along the edge in this direction (`forward` =
    /// from `from` to `to`).
    pub fn allows(&self, v: Vehicle, forward: bool) -> bool {
        self.access.allows(v) && self.oneway.allows(forward)
    }

    /// Whether any motor vehicle may use the edge.
    pub fn has_traffic(&self) -> bool {
        self.access.any()
    }

    /// The limit in this direction.
    pub fn limit(&self, forward: bool) -> SpeedLimit {
        self.maxspeed[usize::from(!forward)]
    }

    /// Motor lanes in this direction.
    pub fn lanes_towards(&self, forward: bool) -> u8 {
        self.lanes[usize::from(!forward)]
    }

    /// The parking on the side to the right of a vehicle going this way
    /// (where it would park in right-hand traffic).
    pub fn kerb_parking(&self, forward: bool) -> Parking {
        if forward {
            self.parking[1]
        } else {
            self.parking[0]
        }
    }

    /// A driving guess, not a law: at most the limit, and less on small
    /// roads where no one drives the rural limit (service roads 30 km/h,
    /// tracks 25, unclassified 70, tertiary 90, secondary 100). The core
    /// lowers it further in curves and in the wet (18B *Behaviour*).
    pub fn target_kmh(&self, class: RoadClass, forward: bool) -> f32 {
        let cap = match class {
            RoadClass::Service => 30.0,
            RoadClass::Track => 25.0,
            RoadClass::Residential => 50.0,
            RoadClass::Unclassified => 70.0,
            RoadClass::Tertiary => 90.0,
            RoadClass::Secondary => 100.0,
            _ => f32::INFINITY,
        };
        self.limit(forward).kmh().min(cap)
    }

    fn flag_bits(&self) -> u16 {
        let mut f = 0;
        match self.oneway {
            Oneway::Forward => f |= flags::ONEWAY_FWD,
            Oneway::Backward => f |= flags::ONEWAY_BWD,
            Oneway::No => {}
        }
        for (on, bit) in [
            (self.roundabout, flags::ROUNDABOUT),
            (self.urban, flags::URBAN),
            (self.destination_only, flags::DESTINATION),
            (self.no_overtaking[0], flags::NO_OVERTAKING_FWD),
            (self.no_overtaking[1], flags::NO_OVERTAKING_BWD),
            (self.single_track, flags::SINGLE_TRACK),
        ] {
            if on {
                f |= bit;
            }
        }
        f
    }
}

// ---------------------------------------------------------------------------
// Controls at nodes, bus stops and lines, car parks
// ---------------------------------------------------------------------------

/// What stops or slows traffic at a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlKind {
    /// Traffic lights (`highway=traffic_signals`).
    Signals = 0,
    /// Lights at a pedestrian crossing (`crossing=traffic_signals`).
    CrossingSignals = 1,
    /// A stop sign (`highway=stop`).
    Stop = 2,
    /// A give-way sign (`highway=give_way`).
    GiveWay = 3,
}

impl ControlKind {
    fn from_code(c: u8) -> Self {
        match c {
            1 => Self::CrossingSignals,
            2 => Self::Stop,
            3 => Self::GiveWay,
            _ => Self::Signals,
        }
    }
}

/// Which travel direction along the edge a control applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Applies {
    Forward = 0,
    Backward = 1,
    Both = 2,
}

impl Applies {
    fn from_code(c: u8) -> Self {
        match c {
            0 => Self::Forward,
            1 => Self::Backward,
            _ => Self::Both,
        }
    }

    pub fn includes(self, forward: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Forward => forward,
            Self::Backward => !forward,
        }
    }
}

/// A signal or sign on an edge (OSM nodes and their `direction`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeControl {
    pub edge: EdgeId,
    /// Metres from the edge's `from` vertex.
    pub along_m: f32,
    pub kind: ControlKind,
    pub applies: Applies,
    /// At a junction vertex (the end of the edge): traffic arriving there
    /// meets it; otherwise along the edge (crossing lights, signs before
    /// the junction).
    pub at_junction: bool,
    pub osm_node: i64,
}

/// A side of the road, driving forward along the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left = 0,
    Right = 1,
}

/// A bus stop (`highway=bus_stop`, a bus `public_transport=platform` or
/// `stop_position`), on its nearest motor edge.
#[derive(Debug, Clone, PartialEq)]
pub struct BusStop {
    pub osm_node: i64,
    /// Package metres (x east, z south).
    pub pos: [f32; 2],
    pub name: String,
    /// The edge it serves (`None`: no motor road near).
    pub edge: Option<EdgeId>,
    pub along_m: f32,
    pub side: Side,
}

/// A bus line (`type=route`, `route=bus`): its edges in driving order and
/// its stops in order. Edges off the package (or not in the graph) are
/// left out, so legs may jump: the core joins them by the shortest path.
#[derive(Debug, Clone, PartialEq)]
pub struct BusRoute {
    pub osm_relation: i64,
    /// The line's number (`ref`).
    pub reference: String,
    pub name: String,
    /// (edge, driven forward).
    pub legs: Vec<(EdgeId, bool)>,
    /// Indices into [`TrafficData::stops`].
    pub stops: Vec<u32>,
}

/// What a car park is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParkingKind {
    Surface = 0,
    MultiStorey = 1,
    Underground = 2,
    Other = 3,
}

impl ParkingKind {
    pub fn from_osm(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("surface") | None => Self::Surface,
            Some("multi-storey") => Self::MultiStorey,
            Some("underground") => Self::Underground,
            Some(_) => Self::Other,
        }
    }

    fn from_code(c: u8) -> Self {
        match c {
            0 => Self::Surface,
            1 => Self::MultiStorey,
            2 => Self::Underground,
            _ => Self::Other,
        }
    }
}

/// A car park (`amenity=parking`): cars arrive and leave (18B).
#[derive(Debug, Clone, PartialEq)]
pub struct ParkingArea {
    /// OSM id (ways and relations negative: −id).
    pub osm_id: i64,
    /// Package metres (the area's centroid).
    pub pos: [f32; 2],
    /// Spaces (`capacity`, else estimated from the area: 25 m² a car).
    pub capacity: u16,
    pub estimated: bool,
    /// Open to the public (not `access=private | customers`).
    pub public: bool,
    pub kind: ParkingKind,
    /// The nearest motor edge (its entrance).
    pub edge: Option<EdgeId>,
    pub along_m: f32,
}

// ---------------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------------

/// The traffic data of a package (section `TRFC`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrafficData {
    /// The package's main country (edges have their own).
    pub country: Country,
    /// The rules of each graph edge, by stable id.
    pub edges: BTreeMap<EdgeId, EdgeRules>,
    /// Signals and signs, sorted by edge and position.
    pub controls: Vec<NodeControl>,
    pub stops: Vec<BusStop>,
    pub bus_routes: Vec<BusRoute>,
    pub parking: Vec<ParkingArea>,
}

impl TrafficData {
    /// The stored rules of an edge.
    pub fn get(&self, id: EdgeId) -> Option<&EdgeRules> {
        self.edges.get(&id)
    }

    /// The signals and signs on an edge, by position.
    pub fn controls_on(&self, id: EdgeId) -> &[NodeControl] {
        let lo = self.controls.partition_point(|c| c.edge < id);
        let hi = self.controls.partition_point(|c| c.edge <= id);
        &self.controls[lo..hi]
    }

    /// Sorts the controls (by edge, then position) as [`Self::controls_on`]
    /// needs.
    pub fn sort(&mut self) {
        self.controls.sort_by(|a, b| {
            a.edge
                .cmp(&b.edge)
                .then(a.along_m.total_cmp(&b.along_m))
                .then(a.osm_node.cmp(&b.osm_node))
        });
    }

    /// How much of the motor roads has a mapped limit (for `world-builder
    /// lint`).
    pub fn speed_coverage(&self, graph: &RoadGraph) -> SpeedCoverage {
        let mut c = SpeedCoverage::default();
        for e in &graph.edges {
            let Some(r) = self.get(e.id) else {
                c.missing_edges += 1;
                continue;
            };
            if !r.has_traffic() || r.access == Access(Access::TRACTOR) {
                continue;
            }
            let km = e.length_m() / 1000.0;
            let row = c.by_class.entry(e.class).or_default();
            row.edges += 1;
            row.km += km;
            if !r.speed_source.is_mapped() {
                row.unmapped_edges += 1;
                row.unmapped_km += km;
            }
        }
        c
    }
}

/// Mapped and unmapped speed limits by road class.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpeedCoverage {
    pub by_class: BTreeMap<RoadClass, CoverageRow>,
    /// Graph edges without a `TRFC` record.
    pub missing_edges: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CoverageRow {
    pub edges: usize,
    pub km: f64,
    /// Without a `maxspeed` or typed limit (the implied limit is used).
    pub unmapped_edges: usize,
    pub unmapped_km: f64,
}

impl SpeedCoverage {
    pub fn total(&self) -> CoverageRow {
        self.by_class
            .values()
            .fold(CoverageRow::default(), |a, r| CoverageRow {
                edges: a.edges + r.edges,
                km: a.km + r.km,
                unmapped_edges: a.unmapped_edges + r.unmapped_edges,
                unmapped_km: a.unmapped_km + r.unmapped_km,
            })
    }
}

/// The rules the traffic core reads per edge: the package's `TRFC` where
/// it has it, else [`EdgeRules::implied`] by class and the country of the
/// package (old packages: "no traffic data", spec 18B).
#[derive(Debug, Clone, Copy)]
pub struct RoadRules<'a> {
    pub data: Option<&'a TrafficData>,
    /// For edges without data (from `TRFC`, else the package's time zone).
    pub country: Country,
}

impl<'a> RoadRules<'a> {
    /// The rules of `package` (its `TRFC`, or the implied ones of the
    /// country of its time zone).
    pub fn of(package: &'a crate::WorldPackage) -> Self {
        let data = package.traffic.as_deref();
        let country = data.map(|d| d.country).unwrap_or_else(|| {
            package
                .time_zone
                .as_deref()
                .map_or(Country::Other, Country::from_time_zone)
        });
        Self { data, country }
    }

    /// The rules of an edge.
    pub fn edge(&self, e: &Edge) -> EdgeRules {
        self.data
            .and_then(|d| d.get(e.id))
            .copied()
            .unwrap_or_else(|| EdgeRules::implied(e.class, self.country, false))
    }

    /// The rules of every edge of `graph`, by edge index (as
    /// `RoadNetwork` numbers them).
    pub fn aligned(&self, graph: &RoadGraph) -> Vec<EdgeRules> {
        graph.edges.iter().map(|e| self.edge(e)).collect()
    }

    /// The signals and signs on an edge (none without data).
    pub fn controls_on(&self, id: EdgeId) -> &'a [NodeControl] {
        self.data.map_or(&[], |d| d.controls_on(id))
    }
}

// --- Encoding -------------------------------------------------------------

fn put_str(b: &mut Vec<u8>, s: &str) {
    let bytes = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
    b.extend((bytes.len() as u16).to_le_bytes());
    b.extend(bytes);
}

pub(crate) fn get_str(r: &mut Bytes) -> Result<String, String> {
    let n = u16::from_le_bytes(r.arr::<2>()?) as usize;
    String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "string is not UTF-8".to_string())
}

fn put_country(b: &mut Vec<u8>, c: Country) {
    b.extend(c.code().as_bytes());
}

fn get_country(r: &mut Bytes) -> Result<Country, String> {
    let a = r.arr::<2>()?;
    Ok(Country::from_code(std::str::from_utf8(&a).unwrap_or("ZZ")))
}

fn u16_of(r: &mut Bytes) -> Result<u16, String> {
    Ok(u16::from_le_bytes(r.arr::<2>()?))
}

fn put_opt_edge(b: &mut Vec<u8>, e: Option<EdgeId>) {
    b.extend(e.map_or(0, |e| e.0).to_le_bytes());
}

fn get_opt_edge(r: &mut Bytes) -> Result<Option<EdgeId>, String> {
    let v = r.u64()?;
    Ok((v != 0).then_some(EdgeId(v)))
}

/// The stored demand factor (1/100): never 0, which means "no factor".
fn flow_code(flow: f32) -> u16 {
    if flow.is_finite() {
        (flow * 100.0).round().clamp(1.0, 65535.0) as u16
    } else {
        100
    }
}

/// The demand factor of a stored code (0: none, 1).
fn flow_of(code: u16) -> f32 {
    if code == 0 {
        1.0
    } else {
        f32::from(code) / 100.0
    }
}

/// Writes the `TRFC` payload.
pub fn write_traffic(t: &TrafficData, b: &mut Vec<u8>) {
    b.push(TRAFFIC_VERSION);
    put_country(b, t.country);
    b.extend((t.edges.len() as u32).to_le_bytes());
    for (id, r) in &t.edges {
        b.extend(id.0.to_le_bytes());
        b.push(r.access.0);
        b.extend(r.flag_bits().to_le_bytes());
        b.extend(r.lanes);
        b.extend(r.maxspeed[0].code().to_le_bytes());
        b.extend(r.maxspeed[1].code().to_le_bytes());
        b.push(r.speed_source as u8);
        put_country(b, r.country);
        let w = r
            .maxweight_t
            .map_or(0, |t| (t * 10.0).round().clamp(1.0, 65535.0) as u16);
        let h = r
            .maxheight_m
            .map_or(0, |m| (m * 100.0).round().clamp(1.0, 65535.0) as u16);
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        b.push(r.parking[0].code());
        b.push(r.parking[1].code());
        b.extend(flow_code(r.flow).to_le_bytes());
    }
    b.extend((t.controls.len() as u32).to_le_bytes());
    for c in &t.controls {
        b.extend(c.edge.0.to_le_bytes());
        b.extend(c.along_m.to_le_bytes());
        b.push(c.kind as u8);
        b.push(c.applies as u8);
        b.push(u8::from(c.at_junction));
        b.extend(c.osm_node.to_le_bytes());
    }
    b.extend((t.stops.len() as u32).to_le_bytes());
    for s in &t.stops {
        b.extend(s.osm_node.to_le_bytes());
        b.extend(s.pos[0].to_le_bytes());
        b.extend(s.pos[1].to_le_bytes());
        put_str(b, &s.name);
        put_opt_edge(b, s.edge);
        b.extend(s.along_m.to_le_bytes());
        b.push(s.side as u8);
    }
    b.extend((t.bus_routes.len() as u32).to_le_bytes());
    for r in &t.bus_routes {
        b.extend(r.osm_relation.to_le_bytes());
        put_str(b, &r.reference);
        put_str(b, &r.name);
        b.extend((r.legs.len() as u32).to_le_bytes());
        for (e, fwd) in &r.legs {
            b.extend(e.0.to_le_bytes());
            b.push(u8::from(*fwd));
        }
        b.extend((r.stops.len() as u32).to_le_bytes());
        for s in &r.stops {
            b.extend(s.to_le_bytes());
        }
    }
    b.extend((t.parking.len() as u32).to_le_bytes());
    for p in &t.parking {
        b.extend(p.osm_id.to_le_bytes());
        b.extend(p.pos[0].to_le_bytes());
        b.extend(p.pos[1].to_le_bytes());
        b.extend(p.capacity.to_le_bytes());
        b.push(u8::from(p.estimated) | (u8::from(p.public) << 1));
        b.push(p.kind as u8);
        put_opt_edge(b, p.edge);
        b.extend(p.along_m.to_le_bytes());
    }
}

/// Reads a `TRFC` payload; a newer layout gives an error containing "not
/// supported" (the package reader then skips it).
pub fn read_traffic(payload: &[u8]) -> Result<TrafficData, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != TRAFFIC_VERSION {
        return Err(format!("traffic layout {version} not supported"));
    }
    let country = get_country(&mut r)?;
    let n = r.u32()? as usize;
    let mut edges = BTreeMap::new();
    for _ in 0..n {
        let id = EdgeId(r.u64()?);
        let access = Access(r.u8()?);
        let f = u16_of(&mut r)?;
        let lanes = [r.u8()?, r.u8()?];
        let maxspeed = [
            SpeedLimit::from_code(u16_of(&mut r)?),
            SpeedLimit::from_code(u16_of(&mut r)?),
        ];
        let speed_source = SpeedSource::from_code(r.u8()?);
        let edge_country = get_country(&mut r)?;
        let w = u16_of(&mut r)?;
        let h = u16_of(&mut r)?;
        let parking = [Parking::from_code(r.u8()?), Parking::from_code(r.u8()?)];
        let flow = flow_of(u16_of(&mut r)?);
        let oneway = if f & flags::ONEWAY_FWD != 0 {
            Oneway::Forward
        } else if f & flags::ONEWAY_BWD != 0 {
            Oneway::Backward
        } else {
            Oneway::No
        };
        edges.insert(
            id,
            EdgeRules {
                access,
                destination_only: f & flags::DESTINATION != 0,
                oneway,
                lanes,
                single_track: f & flags::SINGLE_TRACK != 0,
                maxspeed,
                speed_source,
                country: edge_country,
                urban: f & flags::URBAN != 0,
                maxweight_t: (w != 0).then(|| f32::from(w) / 10.0),
                maxheight_m: (h != 0).then(|| f32::from(h) / 100.0),
                parking,
                roundabout: f & flags::ROUNDABOUT != 0,
                no_overtaking: [
                    f & flags::NO_OVERTAKING_FWD != 0,
                    f & flags::NO_OVERTAKING_BWD != 0,
                ],
                flow,
            },
        );
    }
    let n = r.u32()? as usize;
    let mut controls = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        controls.push(NodeControl {
            edge: EdgeId(r.u64()?),
            along_m: r.f32()?,
            kind: ControlKind::from_code(r.u8()?),
            applies: Applies::from_code(r.u8()?),
            at_junction: r.u8()? != 0,
            osm_node: r.i64()?,
        });
    }
    let n = r.u32()? as usize;
    let mut stops = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        stops.push(BusStop {
            osm_node: r.i64()?,
            pos: [r.f32()?, r.f32()?],
            name: get_str(&mut r)?,
            edge: get_opt_edge(&mut r)?,
            along_m: r.f32()?,
            side: if r.u8()? == 0 {
                Side::Left
            } else {
                Side::Right
            },
        });
    }
    let n = r.u32()? as usize;
    let mut bus_routes = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let osm_relation = r.i64()?;
        let reference = get_str(&mut r)?;
        let name = get_str(&mut r)?;
        let legs_n = r.u32()? as usize;
        let mut legs = Vec::with_capacity(legs_n.min(1 << 20));
        for _ in 0..legs_n {
            legs.push((EdgeId(r.u64()?), r.u8()? != 0));
        }
        let stops_n = r.u32()? as usize;
        let mut route_stops = Vec::with_capacity(stops_n.min(1 << 20));
        for _ in 0..stops_n {
            route_stops.push(r.u32()?);
        }
        bus_routes.push(BusRoute {
            osm_relation,
            reference,
            name,
            legs,
            stops: route_stops,
        });
    }
    let n = r.u32()? as usize;
    let mut parking = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let osm_id = r.i64()?;
        let pos = [r.f32()?, r.f32()?];
        let capacity = u16_of(&mut r)?;
        let fl = r.u8()?;
        let kind = ParkingKind::from_code(r.u8()?);
        parking.push(ParkingArea {
            osm_id,
            pos,
            capacity,
            estimated: fl & 1 != 0,
            public: fl & 2 != 0,
            kind,
            edge: get_opt_edge(&mut r)?,
            along_m: r.f32()?,
        });
    }
    r.finish("traffic")?;
    Ok(TrafficData {
        country,
        edges,
        controls,
        stops,
        bus_routes,
        parking,
    })
}

#[cfg(test)]
mod tests;
