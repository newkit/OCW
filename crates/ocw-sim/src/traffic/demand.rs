//! How much traffic and of which kind (spec 18B *Demand – time of day,
//! day of week, season*): hourly profiles per day type, the day's volume,
//! the class mix by road class and season, public holidays per region,
//! and the session's level (18A `traffic.road`, Q4's lunatic mode).

use super::rules::VehicleClass;
use ocw_world::graph::RoadClass;

/// The road-traffic level of a session (18A `traffic.road`; Q4 adds
/// *lunatic*: drivers speed, cut corners and take risks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TrafficLevel {
    Off,
    Light,
    #[default]
    Normal,
    Busy,
    /// Normal volume, reckless drivers (Q4).
    Lunatic,
}

impl TrafficLevel {
    /// The factor on the demand: light a quarter, busy 1.5 × (18A).
    pub fn factor(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Light => 0.25,
            Self::Normal | Self::Lunatic => 1.0,
            Self::Busy => 1.5,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Normal => "normal",
            Self::Busy => "busy",
            Self::Lunatic => "lunatic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => Self::Off,
            "light" => Self::Light,
            "normal" => Self::Normal,
            "busy" => Self::Busy,
            "lunatic" => Self::Lunatic,
            _ => return None,
        })
    }
}

/// How drivers behave around riders (Q8, Settings → Traffic → *Driver
/// manners*; the host's choice in group rides). Used by the overtaking
/// of T4; T3 carries it in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DriverManners {
    /// (a) Always polite and legal; no horns.
    #[default]
    Polite,
    /// (b) Mostly polite, an occasional honk at a rider in the middle of
    /// the road for a long time.
    Honking,
    /// (c) Some realism: now and then a close pass that never touches.
    Realistic,
}

impl DriverManners {
    pub fn name(self) -> &'static str {
        match self {
            Self::Polite => "polite",
            Self::Honking => "honking",
            Self::Realistic => "realistic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "polite" => Self::Polite,
            "honking" => Self::Honking,
            "realistic" => Self::Realistic,
            _ => return None,
        })
    }

    /// The share of drivers who pass closer than the legal distance
    /// (never under [`super::around::PASS_FLOOR_M`]), and the same share
    /// again who squeeze past in marginal sight (T4, Q8 (c)).
    pub fn close_pass_share(self) -> f64 {
        match self {
            Self::Polite | Self::Honking => 0.0,
            Self::Realistic => 0.15,
        }
    }

    /// Whether drivers honk at a rider they waited behind for long (T4,
    /// Q8 (b) and (c)).
    pub fn honks(self) -> bool {
        self != Self::Polite
    }
}

impl From<ocw_core::traffic::DriverManners> for DriverManners {
    fn from(m: ocw_core::traffic::DriverManners) -> Self {
        use ocw_core::traffic::DriverManners as M;
        match m {
            M::Polite => Self::Polite,
            M::MostlyPolite => Self::Honking,
            M::SomeRealism => Self::Realistic,
        }
    }
}

impl From<ocw_core::traffic::RoadTraffic> for TrafficLevel {
    fn from(t: ocw_core::traffic::RoadTraffic) -> Self {
        use ocw_core::traffic::RoadTraffic as R;
        match t {
            R::Off => Self::Off,
            R::Light => Self::Light,
            R::Normal => Self::Normal,
            R::Busy => Self::Busy,
            R::Lunatic => Self::Lunatic,
        }
    }
}

/// The region whose public holidays apply (spec 18B: a small table per region).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Region {
    /// Germany (Baden-Württemberg's holidays).
    De,
    /// Switzerland (the canton St. Gallen's main holidays).
    Ch,
    /// Liechtenstein.
    Li,
    /// Spain (the Canary Islands' holidays).
    Es,
    /// Anywhere else: only New Year, Easter and Christmas.
    #[default]
    Other,
}

/// Easter Sunday (Gregorian; the anonymous algorithm): (month, day).
pub fn easter(year: i32) -> (u32, u32) {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    (month as u32, day as u32)
}

/// Days since 1970-01-01 of a civil date.
fn day_number(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether a date is a public holiday in `region`.
pub fn is_holiday(region: Region, year: i32, month: u32, day: u32) -> bool {
    let fixed: &[(u32, u32)] = match region {
        Region::De => &[(1, 1), (1, 6), (5, 1), (10, 3), (11, 1), (12, 25), (12, 26)],
        Region::Ch => &[(1, 1), (1, 2), (8, 1), (12, 25), (12, 26)],
        Region::Li => &[
            (1, 1),
            (1, 6),
            (5, 1),
            (8, 15),
            (9, 8),
            (11, 1),
            (12, 8),
            (12, 25),
            (12, 26),
        ],
        Region::Es => &[
            (1, 1),
            (1, 6),
            (5, 1),
            (5, 30),
            (8, 15),
            (10, 12),
            (11, 1),
            (12, 6),
            (12, 8),
            (12, 25),
        ],
        Region::Other => &[(1, 1), (12, 25)],
    };
    if fixed.contains(&(month, day)) {
        return true;
    }
    // Movable feasts, days from Easter Sunday.
    let movable: &[i64] = match region {
        // Good Friday, Easter Monday, Ascension, Whit Monday, Corpus Christi.
        Region::De | Region::Li => &[-2, 1, 39, 50, 60],
        Region::Ch => &[-2, 1, 39, 50],
        // Maundy Thursday, Good Friday.
        Region::Es => &[-3, -2],
        Region::Other => &[1],
    };
    let (em, ed) = easter(year);
    let off = day_number(year, month, day) - day_number(year, em, ed);
    movable.contains(&off)
}

/// The local time that picks the demand (from `ocw_sky`'s world clock and
/// the zone of the place).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DemandTime {
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u8,
    /// Seconds since local midnight.
    pub seconds: u32,
    /// 1 – 12.
    pub month: u8,
    /// A public holiday (counts as a Sunday).
    pub holiday: bool,
}

impl DemandTime {
    /// From the local wall-clock time of the place.
    pub fn from_local(t: &ocw_sky::LocalTime, region: Region) -> Self {
        Self {
            weekday: t.weekday() as u8,
            seconds: t.seconds_of_day(),
            month: t.month as u8,
            holiday: is_holiday(region, t.year, t.month, t.day),
        }
    }

    /// A weekday in June at `hour` (tests and tools).
    pub fn weekday_at(hour: f64) -> Self {
        Self {
            weekday: 2,
            seconds: (hour * 3600.0) as u32,
            month: 6,
            holiday: false,
        }
    }

    /// The same day, `dt` seconds later (wraps at midnight; the day is
    /// kept so a simulated day keeps its day type).
    pub fn plus_seconds(self, dt: u32) -> Self {
        Self {
            seconds: (self.seconds + dt) % 86_400,
            ..self
        }
    }

    pub fn day_type(&self) -> DayType {
        if self.holiday || self.weekday == 6 {
            DayType::Sunday
        } else if self.weekday == 5 {
            DayType::Saturday
        } else {
            DayType::Weekday
        }
    }

    /// Summer half (May – September): more motorbikes, tractors at hay
    /// and harvest.
    pub fn summer(&self) -> bool {
        (5..=9).contains(&self.month)
    }
}

/// The three day profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayType {
    Weekday,
    Saturday,
    /// Sundays and public holidays.
    Sunday,
}

/// Share of a day's traffic per hour (per cent; each sums to 100):
/// weekday double peak 07 – 09 and 16 – 19, a flatter Saturday, a quiet
/// Sunday morning, little at night (spec 18B).
const WEEKDAY: [f64; 24] = [
    0.8, 0.5, 0.4, 0.4, 0.7, 1.8, 4.5, 7.4, 7.6, 5.6, 5.0, 5.2, 5.6, 5.6, 5.8, 6.6, 7.6, 8.0, 6.6,
    4.6, 3.2, 2.4, 1.9, 1.2,
];
const SATURDAY: [f64; 24] = [
    1.4, 0.9, 0.6, 0.5, 0.5, 0.8, 1.5, 2.6, 4.0, 5.6, 6.8, 7.4, 7.4, 7.0, 6.6, 6.4, 6.4, 6.4, 6.0,
    5.0, 3.8, 3.0, 2.4, 1.6,
];
const SUNDAY: [f64; 24] = [
    1.8, 1.2, 0.8, 0.6, 0.5, 0.6, 0.9, 1.5, 2.6, 4.2, 5.8, 6.8, 7.0, 6.8, 7.0, 7.4, 7.6, 7.4, 6.8,
    5.6, 4.2, 3.0, 2.2, 1.7,
];

/// The hourly share of the day's traffic at `t` (a fraction; linear
/// between the hours' middles, so the flow has no steps).
pub fn hour_share(t: &DemandTime) -> f64 {
    let table = match t.day_type() {
        DayType::Weekday => &WEEKDAY,
        DayType::Saturday => &SATURDAY,
        DayType::Sunday => &SUNDAY,
    };
    let sum: f64 = table.iter().sum();
    let h = f64::from(t.seconds) / 3600.0 - 0.5;
    let i0 = h.floor();
    let f = h - i0;
    let i0 = (i0 as i64).rem_euclid(24) as usize;
    let i1 = (i0 + 1) % 24;
    (table[i0] * (1.0 - f) + table[i1] * f) / sum
}

/// The day's volume against an average weekday.
pub fn day_factor(t: &DemandTime) -> f64 {
    let season = match t.month {
        12 | 1 | 2 => 0.93,
        6..=9 => 1.05,
        _ => 1.0,
    };
    let day = match t.day_type() {
        DayType::Weekday if t.weekday == 4 => 1.06,
        DayType::Weekday => 1.0,
        DayType::Saturday => 0.85,
        DayType::Sunday => 0.7,
    };
    season * day
}

/// Vehicles per hour in one direction of an edge with `aadt` (both
/// directions), `two_way` or one-way, at `t` and `level`.
pub fn flow_per_hour(aadt: f64, two_way: bool, t: &DemandTime, level: TrafficLevel) -> f64 {
    let per_dir = if two_way { 0.5 } else { 1.0 };
    aadt * per_dir * day_factor(t) * hour_share(t) * level.factor()
}

/// The class mix on a road class at `t` (shares summing to 1, in
/// [`VehicleClass::ALL`] order): more motorbikes on summer weekends, few
/// lorries on Sundays (the Sunday ban of DE / CH / LI), tractors on tracks
/// and country roads, more in summer.
pub fn class_mix(road: RoadClass, t: &DemandTime) -> [f64; 6] {
    // car, van, lorry, bus, motorbike, tractor
    let mut m: [f64; 6] = match road {
        RoadClass::Trunk => [0.78, 0.09, 0.10, 0.01, 0.02, 0.0],
        RoadClass::Primary => [0.80, 0.09, 0.07, 0.015, 0.02, 0.005],
        RoadClass::Secondary => [0.83, 0.08, 0.05, 0.01, 0.02, 0.01],
        RoadClass::Tertiary => [0.84, 0.08, 0.03, 0.01, 0.02, 0.02],
        RoadClass::Unclassified => [0.85, 0.08, 0.02, 0.0, 0.02, 0.03],
        RoadClass::Residential => [0.88, 0.09, 0.01, 0.0, 0.02, 0.0],
        RoadClass::Service => [0.85, 0.12, 0.0, 0.0, 0.03, 0.0],
        RoadClass::Track => [0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        RoadClass::Cycleway | RoadClass::Path => [0.0; 6],
    };
    let weekend = t.day_type() != DayType::Weekday;
    let moto = match (t.summer(), weekend) {
        (true, true) => 5.0,
        (true, false) => 1.5,
        (false, true) => 0.8,
        (false, false) => 0.3,
    };
    m[4] *= moto;
    if t.day_type() == DayType::Sunday {
        m[2] *= 0.1;
        m[1] *= 0.4;
    }
    if !t.summer() && road != RoadClass::Track {
        m[5] *= 0.4;
    }
    let sum: f64 = m.iter().sum();
    if sum > 0.0 {
        for v in &mut m {
            *v /= sum;
        }
    }
    m
}

/// Picks a class from a mix with `u` in [0, 1).
pub fn pick_class(mix: &[f64; 6], u: f64) -> Option<VehicleClass> {
    let mut acc = 0.0;
    for (i, &w) in mix.iter().enumerate() {
        acc += w;
        if u < acc && w > 0.0 {
            return VehicleClass::from_index(i);
        }
    }
    mix.iter()
        .rposition(|&w| w > 0.0)
        .and_then(VehicleClass::from_index)
}
