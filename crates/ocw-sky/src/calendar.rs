//! The calendar of World → Time → Calendar (spec §14 Phase 9, S9, 9J):
//! civil dates and the month grid, the look of each day at a place and
//! altitude (from the season model of S8, [`crate::season`]), the season
//! starts and the representative days of the seasons of a place (decision
//! Q11: choosing a season jumps the date to its representative day), the
//! equinoxes and solstices, and the moon's main phases per day.
//!
//! Pure and deterministic like the rest of the crate (no clock reads,
//! wasm-safe); the menu's model (`ocw_client::menu::calendar`) builds the
//! page from it.

use crate::moon::{self, Phase};
use crate::season::{self, Climate, LeafType, SeasonModel, State};
use crate::time::{MS_PER_DAY, UnixMs, civil_from_days, days_from_civil, utc_midnight};

/// A civil date (year, month 1–12, day 1–31).
pub type Date = (i32, u32, u32);

/// The calendar's range: the time machine's (spec 9I).
pub const FIRST: Date = (1900, 1, 1);
pub const LAST: Date = (2100, 12, 31);

/// Weeks start on Monday (ISO 8601; from the locale later, spec 9J).
pub const WEEK_START_MONDAY: bool = true;

/// Cells of a month grid: six weeks of seven days.
pub const GRID_DAYS: usize = 42;

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days of year `y`.
pub fn days_in_year(y: i32) -> u32 {
    if is_leap(y) { 366 } else { 365 }
}

/// Days since 1970-01-01.
pub fn day_number(d: Date) -> i64 {
    days_from_civil(d.0, d.1, d.2)
}

pub fn from_day_number(n: i64) -> Date {
    civil_from_days(n)
}

/// `d` limited to the calendar's range.
pub fn clamp(d: Date) -> Date {
    from_day_number(day_number(d).clamp(day_number(FIRST), day_number(LAST)))
}

/// The weekday of `d`: 0 Monday … 6 Sunday (1970-01-01 was a Thursday).
pub fn weekday(d: Date) -> u32 {
    (day_number(d) + 3).rem_euclid(7) as u32
}

/// `d` moved by `n` days, within the range.
pub fn add_days(d: Date, n: i64) -> Date {
    clamp(from_day_number(day_number(d) + n))
}

/// `d` moved by `n` months, the day kept where the month has it (31 Jan
/// + 1 month = 28 / 29 Feb), within the range.
pub fn add_months(d: Date, n: i32) -> Date {
    let i = d.0 * 12 + d.1 as i32 - 1 + n;
    let (y, m) = (i.div_euclid(12), i.rem_euclid(12) as u32 + 1);
    clamp((y, m, d.2.min(days_in_month(y, m))))
}

/// The 42 days of the month view of `y`-`m`: six weeks from the Monday on
/// or before the 1st (the days of the neighbouring months included).
pub fn month_grid(y: i32, m: u32) -> [Date; GRID_DAYS] {
    let first = day_number((y, m, 1));
    let start = first - i64::from(weekday((y, m, 1)));
    std::array::from_fn(|i| from_day_number(start + i as i64))
}

/// The cell of `d` in the month view of `y`-`m`, if shown there.
pub fn grid_index(y: i32, m: u32, d: Date) -> Option<usize> {
    let first = day_number((y, m, 1)) - i64::from(weekday((y, m, 1)));
    let i = day_number(d) - first;
    (0..GRID_DAYS as i64).contains(&i).then_some(i as usize)
}

/// The day of the year (1 = 1 January; 366 = 31 December of a leap year).
pub fn day_of_year(d: Date) -> u32 {
    (day_number(d) - day_number((d.0, 1, 1)) + 1) as u32
}

/// The date of day `doy` of year `y` (366 in a common year: 31 December).
pub fn date_of_doy(y: i32, doy: u32) -> Date {
    let doy = doy.clamp(1, days_in_year(y));
    from_day_number(day_number((y, 1, 1)) + i64::from(doy) - 1)
}

/// The date with day of the year `doy` nearest to `around` (this year's,
/// last year's or next year's), within the range.
pub fn nearest(doy: u32, around: Date) -> Date {
    let at = day_number(around);
    let best = (around.0 - 1..=around.0 + 1)
        .map(|y| date_of_doy(y, doy))
        .min_by_key(|d| ((day_number(*d) - at).abs(), day_number(*d)))
        .expect("three candidates");
    clamp(best)
}

/// The first date after `after` with day of the year `doy`.
pub fn next_after(doy: u32, after: Date) -> Date {
    let at = day_number(after);
    let d = (after.0..=after.0 + 1)
        .map(|y| date_of_doy(y, doy))
        .find(|d| day_number(*d) > at)
        .expect("next year's");
    clamp(d)
}

/// The last date before `before` with day of the year `doy`.
pub fn prev_before(doy: u32, before: Date) -> Date {
    let at = day_number(before);
    let d = (before.0 - 1..=before.0)
        .rev()
        .map(|y| date_of_doy(y, doy))
        .find(|d| day_number(*d) < at)
        .expect("last year's");
    clamp(d)
}

pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Monday first (see [`WEEK_START_MONDAY`]).
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "16 Nov 2026".
pub fn short_date(d: Date) -> String {
    format!("{} {} {}", d.2, &MONTHS[(d.1 - 1) as usize][..3], d.0)
}

/// "2026-11-16" (the console's form).
pub fn iso(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

// --- The look of a day --------------------------------------------------------

/// What the landscape looks like on a day at a place and altitude: the
/// calendar's coloured band (spec 9J).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Look {
    /// Deciduous trees from budburst to full leaf.
    Fresh,
    /// Summer green (evergreen places: green meadows).
    Summer,
    /// Autumn colours.
    Autumn,
    /// Deciduous trees bare.
    Bare,
    /// Lying snow (climatological, or the measured snow of real weather).
    Snow,
    /// Places without deciduous trees (the Canary Islands, the tropics):
    /// meadows dried out by the summer.
    Dry,
}

impl Look {
    pub const ALL: [Look; 6] = [
        Self::Fresh,
        Self::Summer,
        Self::Autumn,
        Self::Bare,
        Self::Snow,
        Self::Dry,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "fresh green",
            Self::Summer => "summer",
            Self::Autumn => "autumn colours",
            Self::Bare => "bare",
            Self::Snow => "snow",
            Self::Dry => "dry",
        }
    }

    /// The band's colour (sRGB 0–1), told apart by brightness too.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Self::Fresh => [0.62, 0.86, 0.30],
            Self::Summer => [0.14, 0.52, 0.18],
            Self::Autumn => [0.93, 0.50, 0.12],
            Self::Bare => [0.52, 0.40, 0.30],
            Self::Snow => [0.96, 0.97, 1.0],
            Self::Dry => [0.86, 0.74, 0.40],
        }
    }
}

/// A place without deciduous trees: its look follows the meadows (dry or
/// green) and the snow.
pub fn evergreen(c: &Climate) -> bool {
    c.trees.evergreen_broadleaves
}

/// The look of state `s` in climate `c`, with `snow` the lying snow (the
/// climatological `s.snow`, or a measured one).
pub fn look_of(c: &Climate, s: &State, snow: f32) -> Look {
    if snow >= 0.5 {
        return Look::Snow;
    }
    if evergreen(c) {
        return if season::grass(c, s).dry > 0.4 {
            Look::Dry
        } else {
            Look::Summer
        };
    }
    let f = season::foliage(&c.phenology, LeafType::Deciduous, s, 0.0);
    if f.leaves < 0.15 {
        Look::Bare
    } else if f.autumn > 0.15 {
        Look::Autumn
    } else if f.fresh > 0.3 || f.leaves < 0.8 {
        Look::Fresh
    } else {
        Look::Summer
    }
}

/// The seasons a rider chooses on World → Time → Season (spec 9J, S9b):
/// each a representative day of the place (decision Q11: a date jump).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Season {
    Spring,
    EarlySummer,
    Summer,
    Autumn,
    LateAutumn,
    Winter,
}

impl Season {
    pub const ALL: [Season; 6] = [
        Self::Spring,
        Self::EarlySummer,
        Self::Summer,
        Self::Autumn,
        Self::LateAutumn,
        Self::Winter,
    ];

    /// The console name ("early-summer").
    pub fn id(self) -> &'static str {
        match self {
            Self::Spring => "spring",
            Self::EarlySummer => "early-summer",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
            Self::LateAutumn => "late-autumn",
            Self::Winter => "winter",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spring => "Spring",
            Self::EarlySummer => "Early summer",
            Self::Summer => "Summer",
            Self::Autumn => "Autumn",
            Self::LateAutumn => "Late autumn",
            Self::Winter => "Winter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|p| p.id() == s)
    }
}

/// The season starts of a place (days of the year; `None` where the
/// place does not have it: no deciduous trees, no lying snow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeasonStarts {
    pub first_green: Option<u32>,
    pub full_leaf: Option<u32>,
    pub peak_autumn: Option<u32>,
    pub leaves_gone: Option<u32>,
    pub first_snow: Option<u32>,
}

impl SeasonStarts {
    /// The shortcuts in the order of the year's cycle: (label, day).
    pub fn list(&self) -> [(&'static str, Option<u32>); 5] {
        [
            ("First green", self.first_green),
            ("Full leaf", self.full_leaf),
            ("Peak autumn colours", self.peak_autumn),
            ("Leaves gone", self.leaves_gone),
            ("First lying snow", self.first_snow),
        ]
    }
}

/// A place's year at one altitude: the look of every day of the year,
/// the season starts and the representative days of the seasons.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonYear {
    /// The look per day of the year (index 0 = 1 January, 366 entries).
    pub looks: Vec<Look>,
    /// The climatological snow per day (index as `looks`).
    pub snow: Vec<f32>,
    pub starts: SeasonStarts,
    /// The representative day of each of [`Season::ALL`].
    pub seasons: [u32; 6],
    pub climate: Climate,
    pub alt_m: f64,
}

/// Day of the year (1 … 365) of a fractional day.
fn wrap_doy(d: f64) -> u32 {
    (d.round() as i64 - 1).rem_euclid(365) as u32 + 1
}

impl SeasonYear {
    /// The year at `alt_m`; `day_of` maps a day of the year to the day the
    /// model is evaluated on (the renderer's season bucket, so that the
    /// band shows what the world shows that day).
    pub fn new(model: &SeasonModel, alt_m: f64, day_of: impl Fn(u32) -> f64) -> Self {
        let c = *model.climate();
        let states: Vec<State> = (1..=366)
            .map(|d| model.state_at(day_of(d), alt_m))
            .collect();
        let looks: Vec<Look> = states.iter().map(|s| look_of(&c, s, s.snow)).collect();
        let snow: Vec<f32> = states.iter().map(|s| s.snow).collect();
        let at = |doy: u32| &states[(doy - 1) as usize];
        let cold = wrap_doy(c.cycle_start());
        let warm = wrap_doy(c.warmest_day);
        // Days of the cycle from the coldest day: k → day of the year.
        let doy = |k: u32| (cold - 1 + k) % 365 + 1;
        let k_of = |d: u32| (d + 365 - cold) % 365;
        let leaf = |k: u32| season::foliage(&c.phenology, LeafType::Deciduous, at(doy(k)), 0.0);
        let mut starts = SeasonStarts::default();
        if !evergreen(&c) {
            let green = (0..365).find(|&k| leaf(k).leaves >= 0.05);
            let full = green.and_then(|g| (g..365).find(|&k| leaf(k).leaves >= 0.95));
            let peak = full.and_then(|f| {
                (f..365)
                    .map(|k| (k, leaf(k).autumn * leaf(k).leaves))
                    .filter(|(_, v)| *v > 0.2)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(k, _)| k)
            });
            let gone = peak.and_then(|p| (p..365).find(|&k| leaf(k).leaves < 0.05));
            starts.first_green = green.map(doy);
            starts.full_leaf = full.map(doy);
            starts.peak_autumn = peak.map(doy);
            starts.leaves_gone = gone.map(doy);
        }
        // Snow: from the warmest day on, the first day it lies.
        let lies = |d: u32| at(d).snow >= 0.5;
        let after_warm = |i: u32| (warm - 1 + i) % 365 + 1;
        let always = (1..=365).all(lies);
        if !always {
            starts.first_snow = (1..365)
                .map(after_warm)
                .find(|&d| lies(d) && !lies(if d == 1 { 365 } else { d - 1 }));
        }
        // Winter: the middle of the snow season (the longest run of
        // snow days), else the coldest day.
        let winter = if always {
            cold
        } else {
            let mut best: Option<(u32, u32)> = None; // (start i, length)
            let mut i = 0;
            while i < 365 {
                if lies(after_warm(i)) {
                    let start = i;
                    while i < 365 && lies(after_warm(i)) {
                        i += 1;
                    }
                    if best.is_none_or(|b| i - start > b.1) {
                        best = Some((start, i - start));
                    }
                }
                i += 1;
            }
            best.map_or(cold, |(s, n)| after_warm(s + n / 2))
        };
        let wk = k_of(warm);
        let seasons = match (
            starts.first_green,
            starts.full_leaf,
            starts.peak_autumn,
            starts.leaves_gone,
        ) {
            (Some(g), Some(f), Some(p), Some(l)) => {
                let (g, f, p, l) = (k_of(g), k_of(f), k_of(p), k_of(l));
                let summer = wk.clamp(f + 10, p.saturating_sub(10).max(f + 10));
                [
                    doy((g + f) / 2),
                    doy((f + summer) / 2),
                    doy(summer),
                    doy(p),
                    doy((p + 2 * l) / 3),
                    winter,
                ]
            }
            // No deciduous trees here: the seasons by the temperature's
            // year, from the coldest and the warmest day.
            _ => [
                doy(91),
                doy((91 + wk) / 2),
                warm,
                doy((wk + 76) % 365),
                doy((wk + 122) % 365),
                winter,
            ],
        };
        Self {
            looks,
            snow,
            starts,
            seasons,
            climate: c,
            alt_m,
        }
    }

    /// The look on `d` (the day of the year's).
    pub fn look(&self, d: Date) -> Look {
        self.looks[(day_of_year(d) - 1) as usize]
    }

    /// The representative day of `s`.
    pub fn season_doy(&self, s: Season) -> u32 {
        self.seasons[Season::ALL.iter().position(|&x| x == s).expect("listed")]
    }

    /// The season of a day of the year: the one whose representative day
    /// is nearest (over the year's turn).
    pub fn season_of(&self, doy: u32) -> Season {
        let dist = |a: u32| {
            let d = (i64::from(a) - i64::from(doy)).rem_euclid(365);
            d.min(365 - d)
        };
        Season::ALL
            .into_iter()
            .min_by_key(|&s| dist(self.season_doy(s)))
            .expect("six seasons")
    }

    /// The date of season `s` nearest to `around` (decision Q11: the
    /// season's representative day of the place; the menu and `time
    /// season` jump there, keeping the time of day).
    pub fn season_date(&self, s: Season, around: Date) -> Date {
        nearest(self.season_doy(s), around)
    }

    /// The next (`dir` > 0) or previous representative day of any season,
    /// strictly after / before `from` (the Season row's ← / →), and its
    /// season.
    pub fn step_season(&self, from: Date, dir: i32) -> (Season, Date) {
        let at = day_number(from);
        Season::ALL
            .into_iter()
            .map(|s| {
                let d = if dir > 0 {
                    next_after(self.season_doy(s), from)
                } else {
                    prev_before(self.season_doy(s), from)
                };
                (s, d)
            })
            .min_by_key(|(_, d)| (day_number(*d) - at).abs())
            .expect("six seasons")
    }
}

/// The clock change of `time season PICK` at the world instant `now`: the
/// date of the season's representative day (or of the day of the year)
/// nearest to the world date, **keeping the time of day** (decision Q11),
/// and that date.
pub fn season_change(
    pick: crate::machine::SeasonPick,
    now: UnixMs,
    zone: &crate::zone::Zone,
    year: &SeasonYear,
) -> Result<(crate::machine::ClockChange, Date), String> {
    use crate::machine::SeasonPick;
    let l = zone.local(now);
    let today = (l.year, l.month, l.day);
    let d = match pick {
        SeasonPick::Season(s) => year.season_date(s, today),
        SeasonPick::Day(n) => nearest(n, today),
    };
    let (ms, _) = zone.instant(d, (l.hour, l.minute, l.second))?;
    Ok((
        crate::machine::ClockChange::Fixed(crate::time::clamp_range(ms)),
        d,
    ))
}

// --- Sun and moon ---------------------------------------------------------------

/// The March equinox, June solstice, September equinox and December
/// solstice of year `y` (UTC instants, to about a minute: the sun's
/// apparent longitude 0°, 90°, 180°, 270°).
pub fn solar_events(y: i32) -> [UnixMs; 4] {
    let guesses = [(3, 20), (6, 21), (9, 22), (12, 21)];
    std::array::from_fn(|i| {
        let target = 90.0 * i as f64;
        let (m, d) = guesses[i];
        let mut t = utc_midnight(y, m, d) + MS_PER_DAY / 2;
        for _ in 0..6 {
            let lon = crate::sun::geocentric(crate::time::julian_day(t)).longitude_deg;
            let off = crate::astro::wrap180(target - lon);
            t += (off / 0.985_647 * MS_PER_DAY as f64) as i64;
            if off.abs() < 1e-5 {
                break;
            }
        }
        t
    })
}

/// The names of [`solar_events`].
pub const SOLAR_EVENTS: [&str; 4] = [
    "March equinox",
    "June solstice",
    "September equinox",
    "December solstice",
];

/// The moon's main phases in `[from, to)`, in time order.
pub fn moon_phases(from: UnixMs, to: UnixMs) -> Vec<(UnixMs, Phase)> {
    let mut out = Vec::new();
    for p in [
        Phase::New,
        Phase::FirstQuarter,
        Phase::Full,
        Phase::LastQuarter,
    ] {
        let mut t = moon::next_phase(from - 1, p);
        while t < to {
            out.push((t, p));
            t = moon::next_phase(t, p);
        }
    }
    out.sort_by_key(|e| e.0);
    out
}

/// The glyph of a main phase as the UI font draws it in light ink on the
/// dark menu (the ink is the lit part): full ●, new ○, a quarter lit on
/// the right (first quarter in the north) ◑ or on the left ◐ – turned
/// round south of the equator.
pub fn phase_glyph(p: Phase, southern: bool) -> &'static str {
    match (p, southern) {
        (Phase::Full, _) => "●",
        (Phase::New, _) => "○",
        (Phase::FirstQuarter, false) | (Phase::LastQuarter, true) => "◑",
        (Phase::FirstQuarter, true) | (Phase::LastQuarter, false) => "◐",
    }
}

pub fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::New => "new moon",
        Phase::FirstQuarter => "first quarter",
        Phase::Full => "full moon",
        Phase::LastQuarter => "last quarter",
    }
}

/// The moon's illuminated fraction (0–1) and whether it waxes, at `ms`
/// (from the elongation; geocentric).
pub fn moon_light(ms: UnixMs) -> (f64, bool) {
    let e = moon::elongation_lon(ms);
    ((1.0 - e.to_radians().cos()) / 2.0, e < 180.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::SeasonModel;
    use crate::zone::Zone;

    #[test]
    fn weekdays_and_leap_years() {
        assert_eq!(weekday((2026, 11, 16)), 0, "a Monday");
        assert_eq!(weekday((2000, 1, 1)), 5, "a Saturday");
        assert_eq!(weekday((1900, 1, 1)), 0, "a Monday");
        assert_eq!(weekday((2100, 12, 31)), 4, "a Friday");
        assert!(is_leap(2024) && is_leap(2000) && !is_leap(2100) && !is_leap(1900));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(day_of_year((2024, 12, 31)), 366);
        assert_eq!(day_of_year((2026, 3, 1)), 60);
        assert_eq!(date_of_doy(2026, 366), (2026, 12, 31));
        assert_eq!(date_of_doy(2024, 60), (2024, 2, 29));
    }

    /// Six Monday-first weeks; weeks across the year's end; February 2100
    /// (not a leap year) fits four weeks from a Monday.
    #[test]
    fn month_grids() {
        let g = month_grid(2026, 11);
        assert_eq!(g[0], (2026, 10, 26), "the Monday before Sunday 1 Nov");
        assert_eq!(g[6], (2026, 11, 1));
        assert_eq!(g[41], (2026, 12, 6));
        assert_eq!(grid_index(2026, 11, (2026, 11, 16)), Some(21));
        let g = month_grid(2027, 1);
        assert_eq!(g[0], (2026, 12, 28), "the week across the new year");
        assert_eq!(g[4], (2027, 1, 1));
        let g = month_grid(2100, 2);
        assert_eq!(g[0], (2100, 2, 1), "1 Feb 2100 is a Monday");
        assert_eq!(g[27], (2100, 2, 28));
        assert_eq!(g[28], (2100, 3, 1), "no 29 February 2100");
        let g = month_grid(2024, 2);
        assert!(g.contains(&(2024, 2, 29)));
        for (y, m) in [(1900, 1), (2026, 11), (2100, 12), (2024, 2)] {
            let g = month_grid(y, m);
            assert_eq!(weekday(g[0]), 0);
            for w in g.windows(2) {
                assert_eq!(day_number(w[1]) - day_number(w[0]), 1);
            }
            for d in 1..=days_in_month(y, m) {
                assert!(g.contains(&(y, m, d)));
            }
        }
        // The first grid shows days before 1900 (not selectable: clamped).
        assert_eq!(month_grid(1900, 1)[0], (1900, 1, 1));
        assert!(month_grid(2100, 12)[41] > LAST);
    }

    #[test]
    fn steps_clamp_to_the_range() {
        assert_eq!(add_days((2026, 12, 31), 1), (2027, 1, 1));
        assert_eq!(add_days((2100, 12, 31), 1), LAST);
        assert_eq!(add_days((1900, 1, 1), -7), FIRST);
        assert_eq!(add_months((2026, 1, 31), 1), (2026, 2, 28));
        assert_eq!(add_months((2024, 1, 31), 1), (2024, 2, 29));
        assert_eq!(add_months((2026, 11, 16), -12), (2025, 11, 16));
        assert_eq!(add_months((2026, 12, 5), 1), (2027, 1, 5));
        assert_eq!(add_months((2100, 12, 5), 1), LAST);
        assert_eq!(add_months((1900, 1, 5), -1), FIRST);
        assert_eq!(nearest(15, (2026, 11, 16)), (2027, 1, 15));
        assert_eq!(nearest(196, (2026, 11, 16)), (2026, 7, 15));
        assert_eq!(nearest(15, (2100, 11, 16)), (2100, 12, 31), "clamped");
        assert_eq!(next_after(320, (2026, 11, 16)), (2027, 11, 16));
        assert_eq!(prev_before(320, (2026, 11, 16)), (2025, 11, 16));
        assert_eq!(short_date((2026, 11, 16)), "16 Nov 2026");
        assert_eq!(iso((2026, 3, 5)), "2026-03-05");
    }

    /// The equinoxes and solstices of 2026 (USNO: 20 Mar 14:46, 21 Jun
    /// 08:24, 23 Sep 00:05, 21 Dec 20:50 UTC) within 10 minutes.
    #[test]
    fn equinoxes_and_solstices() {
        let at = |m, d, h: i64, mi: i64| utc_midnight(2026, m, d) + (h * 60 + mi) * 60_000;
        let want = [
            at(3, 20, 14, 46),
            at(6, 21, 8, 24),
            at(9, 23, 0, 5),
            at(12, 21, 20, 50),
        ];
        for (got, want) in solar_events(2026).into_iter().zip(want) {
            assert!(
                (got - want).abs() < 10 * 60_000,
                "{} min off",
                (got - want) / 60_000
            );
        }
    }

    /// The main phases of 2026 against the known dates (the full moons of
    /// `moon`'s tests): 13 full moons, the 24 November one on that day in
    /// Vaduz; the glyphs exist and turn round in the south.
    #[test]
    fn moon_phases_of_a_year() {
        let p = moon_phases(utc_midnight(2026, 1, 1), utc_midnight(2027, 1, 1));
        let full: Vec<_> = p.iter().filter(|e| e.1 == Phase::Full).collect();
        assert_eq!(full.len(), 13);
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let l = zone.local(full[11].0);
        assert_eq!((l.month, l.day), (11, 24));
        let new: Vec<_> = p.iter().filter(|e| e.1 == Phase::New).collect();
        let l = zone.local(new[10].0);
        assert_eq!((l.month, l.day), (11, 9));
        // Phases a week apart, in order.
        for w in p.windows(2) {
            let days = (w[1].0 - w[0].0) as f64 / MS_PER_DAY as f64;
            assert!((5.5..9.0).contains(&days), "{days}");
        }
        assert_eq!(phase_glyph(Phase::FirstQuarter, false), "◑");
        assert_eq!(phase_glyph(Phase::FirstQuarter, true), "◐");
        let (lit, _) = moon_light(full[11].0);
        assert!(lit > 0.99, "{lit}");
        let (_, waxing) = moon_light(full[11].0 - MS_PER_DAY);
        assert!(waxing);
        let (lit, _) = moon_light(new[10].0);
        assert!(lit < 0.01, "{lit}");
    }

    fn year(lat: f64, lon: f64, alt: f64) -> SeasonYear {
        SeasonYear::new(&SeasonModel::for_place(lat, lon), alt, f64::from)
    }

    fn date(doy: Option<u32>) -> Date {
        date_of_doy(2026, doy.expect("a day"))
    }

    const VADUZ: (f64, f64, f64) = (47.14, 9.52, 455.0);
    const MALBUN: (f64, f64, f64) = (47.10, 9.61, 1600.0);

    /// The bands of S8's test days in Vaduz and Malbun.
    #[test]
    fn looks_in_vaduz_and_malbun() {
        let v = year(VADUZ.0, VADUZ.1, VADUZ.2);
        assert_eq!(v.look((2026, 1, 15)), Look::Bare);
        assert_eq!(v.look((2026, 5, 1)), Look::Fresh);
        assert_eq!(v.look((2026, 7, 1)), Look::Summer);
        assert_eq!(v.look((2026, 10, 25)), Look::Autumn);
        assert_eq!(v.look((2026, 12, 1)), Look::Bare);
        assert!(!v.looks.contains(&Look::Snow), "no lying snow in Vaduz");
        let m = year(MALBUN.0, MALBUN.1, MALBUN.2);
        assert_eq!(m.look((2026, 1, 15)), Look::Snow);
        assert_eq!(m.look((2026, 7, 15)), Look::Summer);
        assert_ne!(
            m.look((2026, 5, 1)),
            Look::Summer,
            "spring comes later up there"
        );
    }

    /// The computed season starts lie in plausible windows (spec 9J).
    #[test]
    fn season_starts_vaduz_malbun_tenerife() {
        let v = year(VADUZ.0, VADUZ.1, VADUZ.2).starts;
        let within = |d: Option<u32>, a: Date, b: Date| {
            let d = date(d);
            assert!(d >= a && d <= b, "{d:?} not in {a:?}..{b:?}");
        };
        within(v.first_green, (2026, 4, 1), (2026, 4, 25));
        within(v.full_leaf, (2026, 4, 20), (2026, 5, 20));
        within(v.peak_autumn, (2026, 10, 10), (2026, 11, 10));
        within(v.leaves_gone, (2026, 11, 1), (2026, 12, 5));
        assert_eq!(v.first_snow, None);
        let m = year(MALBUN.0, MALBUN.1, MALBUN.2).starts;
        within(m.first_green, (2026, 5, 10), (2026, 6, 20));
        within(m.peak_autumn, (2026, 9, 15), (2026, 10, 20));
        within(m.first_snow, (2026, 10, 15), (2026, 12, 20));
        assert!(date(m.first_green) > date(v.first_green));
        // Tenerife: evergreen, no lying snow at the coast; the Teide white
        // in winter.
        let coast = year(28.41, -16.55, 50.0);
        assert_eq!(coast.starts.first_green, None);
        assert_eq!(coast.starts.first_snow, None);
        assert!(!coast.looks.contains(&Look::Bare));
        assert_eq!(coast.look((2026, 8, 15)), Look::Dry);
        assert_eq!(coast.look((2026, 2, 1)), Look::Summer);
        let teide = year(28.27, -16.64, 3700.0);
        assert_eq!(teide.look((2026, 2, 9)), Look::Snow);
        assert_ne!(teide.look((2026, 7, 20)), Look::Snow);
        let first = date(teide.starts.first_snow);
        assert!(matches!(first.1, 11 | 12 | 1), "{first:?}");
    }

    /// The southern hemisphere: spring in September – November, summer
    /// around January (a made-up place at 43.5° S).
    #[test]
    fn seasons_turn_round_in_the_south() {
        let s = year(-43.5, 172.6, 20.0);
        let first = date_of_doy(2026, s.starts.first_green.unwrap());
        assert!(first.1 >= 9 && first.1 <= 11, "{first:?}");
        let summer = s.season_date(Season::Summer, (2026, 11, 16));
        assert!(
            summer.1 == 1 || summer.1 == 2 || summer.1 == 12,
            "{summer:?}"
        );
        assert_eq!(s.look((2026, 7, 15)), Look::Bare);
        assert_eq!(s.look((2026, 1, 15)), Look::Summer);
    }

    /// The representative days: in the cycle's order, each of its look
    /// (spring fresh, summer, autumn colours, winter snow in Malbun);
    /// the date jump goes to the nearest one, the season of a day is the
    /// nearest representative day's.
    #[test]
    fn representative_days_and_jumps() {
        for (lat, lon, alt) in [VADUZ, MALBUN] {
            let y = year(lat, lon, alt);
            let look = |s| y.looks[(y.season_doy(s) - 1) as usize];
            assert_eq!(look(Season::Spring), Look::Fresh, "{alt}");
            assert_eq!(look(Season::Summer), Look::Summer, "{alt}");
            assert_eq!(look(Season::Autumn), Look::Autumn, "{alt}");
            for s in Season::ALL {
                assert_eq!(y.season_of(y.season_doy(s)), s, "{alt} {s:?}");
                assert_eq!(Season::parse(s.id()), Some(s));
            }
        }
        let m = year(MALBUN.0, MALBUN.1, MALBUN.2);
        assert_eq!(
            m.looks[(m.season_doy(Season::Winter) - 1) as usize],
            Look::Snow
        );
        let v = year(VADUZ.0, VADUZ.1, VADUZ.2);
        // From 16 November 2026: summer was in July, winter comes in January.
        let s = v.season_date(Season::Summer, (2026, 11, 16));
        assert_eq!((s.0, s.1), (2026, 7));
        let w = v.season_date(Season::Winter, (2026, 11, 16));
        assert_eq!(w.0, 2027);
        // Stepping: the next representative day after and before a date.
        let (s, d) = v.step_season((2026, 7, 1), 1);
        assert!(d > (2026, 7, 1));
        assert!(matches!(s, Season::Summer | Season::Autumn), "{s:?}");
        let (_, back) = v.step_season(d, -1);
        assert!(back < d);
        assert_eq!(Season::parse("Late_Autumn"), Some(Season::LateAutumn));
    }

    /// Decision Q11: a season jumps the date to its representative day of
    /// the place, the time of day kept (across a DST change too).
    #[test]
    fn the_season_jump_keeps_the_time_of_day() {
        use crate::machine::{ClockChange, SeasonPick};
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let (now, _) = zone.instant((2026, 11, 16), (14, 20, 0)).unwrap();
        let m = year(MALBUN.0, MALBUN.1, MALBUN.2);
        let (change, d) =
            season_change(SeasonPick::Season(Season::Summer), now, &zone, &m).unwrap();
        assert_eq!(d, m.season_date(Season::Summer, (2026, 11, 16)));
        let ClockChange::Fixed(at) = change else {
            panic!("{change:?}")
        };
        let l = zone.local(at);
        assert_eq!((l.year, l.month, l.day), d);
        assert_eq!(
            (l.hour, l.minute),
            (14, 20),
            "CEST in summer, the same wall clock"
        );
        assert!(l.dst);
        assert_eq!(m.look(d), Look::Summer);
        let (_, d) = season_change(SeasonPick::Season(Season::Winter), now, &zone, &m).unwrap();
        assert_eq!(m.look(d), Look::Snow);
        assert!(d > (2026, 11, 16), "the coming winter is nearer: {d:?}");
        let (_, d) = season_change(SeasonPick::Day(1), now, &zone, &m).unwrap();
        assert_eq!(d, (2027, 1, 1));
    }
}
