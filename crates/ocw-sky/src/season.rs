//! Seasons by date and altitude (spec §14 Phase 9, 9C, milestone S8):
//! phenology, the crop calendar and the climatological snow cover.
//!
//! One climatology drives everything: the daily mean temperature of a
//! place is a cosine over the year around its annual mean, falling with
//! the altitude by a lapse rate ([`Climate::temp_c`]); the day-to-day
//! spread around it is a normal distribution, so a degree-day sum counts
//! the *expected* warm (or cold) part of each day ([`expected_above`]) and
//! a cold valley still gets the odd mild day.
//!
//! - **Spring** (leaf-out, grass green-up, crops) follows **growing degree
//!   days** above 5 °C, counted from the coldest day of the year
//!   ([`State::gdd`]): the higher (colder) a place, the later the sum is
//!   reached – Malbun (1 600 m) leafs out about six weeks after Vaduz
//!   (450 m), a place further north later than one further south.
//! - **Autumn** (leaf colouring and fall, grass dormancy, the frost that
//!   ends a field's season) follows **cold degree days** below 20 °C,
//!   counted from three weeks after the warmest day ([`State::cdd`]): the
//!   higher a place, the earlier.
//! - **Snow** lies where a daily snowpack balance (snowfall at the place's
//!   precipitation when it is cold, melt by degree days when it is mild),
//!   run over the climatological year, keeps snow on the ground
//!   ([`State::snow`]): the climatological snow line by date and altitude.
//!
//! The thresholds are [`Phenology`] and the crop calendars ([`CEREAL`],
//! [`MAIZE`]); the climate and the tree mix are tunable per region
//! ([`Climate::for_place`]: the Alpine Rhine valley of Liechtenstein, the
//! Black Forest, the Canary Islands, else a climate by latitude).
//!
//! The model is evaluated once per place into altitude bands
//! ([`SeasonModel`]), and per date into a [`SeasonDay`]; the renderer bakes
//! trees and colour maps from it and gives its shaders linear ramps over
//! the altitude ([`SeasonDay::ramps`]). Pure and deterministic, no clock
//! reads (the date comes from the world clock), wasm-safe.

/// Days of a mean year.
pub const YEAR_DAYS: f64 = 365.25;
/// The lowest altitude of the bands (m).
pub const MIN_ALT_M: f64 = -400.0;
/// Height of an altitude band (m).
pub const BAND_M: f64 = 50.0;
/// Number of altitude bands: −400 … 6 000 m.
pub const BANDS: usize = 129;
/// Base of the growing degree days (°C).
pub const GDD_BASE_C: f64 = 5.0;
/// Base of the cold degree days (°C).
pub const CDD_BASE_C: f64 = 20.0;
/// The autumn sum starts this many days after the warmest day.
pub const AUTUMN_START_DAYS: f64 = 20.0;

// --- Climate ----------------------------------------------------------------

/// Phenological thresholds (degree days; spec 9C: "simple degree-day
/// rules, tunable per region").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phenology {
    /// Deciduous trees: budburst and full leaf (GDD), fresh green until.
    pub budburst_gdd: f64,
    pub leaf_full_gdd: f64,
    pub fresh_until_gdd: f64,
    /// Larches flush earlier (GDD).
    pub larch_budburst_gdd: f64,
    pub larch_full_gdd: f64,
    /// Autumn colours start and are full (CDD), leaves fall (CDD).
    pub colour_start_cdd: f64,
    pub colour_full_cdd: f64,
    pub fall_start_cdd: f64,
    pub fall_end_cdd: f64,
    /// Larches turn and drop their needles this much later (CDD).
    pub larch_autumn_shift_cdd: f64,
    /// Grass greens up (GDD) and turns dormant (CDD).
    pub grass_up_gdd: (f64, f64),
    pub grass_dormant_cdd: (f64, f64),
    /// Meadows dry out (straw) in hot dry summers between these daily
    /// mean temperatures (°C), times [`Climate::summer_dry`].
    pub dry_c: (f64, f64),
}

/// Calibrated on the Alpine Rhine valley: beech budburst in Vaduz about
/// 13 April, full leaf early May; colours from early October, full in
/// late October, bare by mid-November; grass green from late March, dull
/// from late November.
pub const PHENOLOGY: Phenology = Phenology {
    budburst_gdd: 100.0,
    leaf_full_gdd: 220.0,
    fresh_until_gdd: 480.0,
    larch_budburst_gdd: 60.0,
    larch_full_gdd: 170.0,
    colour_start_cdd: 250.0,
    colour_full_cdd: 400.0,
    fall_start_cdd: 450.0,
    fall_end_cdd: 650.0,
    larch_autumn_shift_cdd: 60.0,
    grass_up_gdd: (25.0, 140.0),
    grass_dormant_cdd: (700.0, 1100.0),
    dry_c: (19.0, 23.5),
};

/// The share of trees that are deciduous broadleaves or larches, by
/// setting and altitude: the fallback until a package carries the forest
/// type (spec §14 Phase 8 T3, Copernicus HRL dominant leaf type).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafMix {
    /// Forest: (altitude m, deciduous share, larch share), ascending;
    /// linear in between, the ends held.
    pub forest: [(f64, f64, f64); 5],
    /// Deciduous share of orchards, town trees, lone trees in the open
    /// (meadows, fields), scrub and wetland trees below
    /// [`LeafMix::lowland_until_m`].
    pub orchard: f64,
    pub urban: f64,
    pub open: f64,
    pub scrub: f64,
    pub wetland: f64,
    /// Above this the shares outside forests fall to 20 % at
    /// [`LeafMix::highland_from_m`] (lone trees up there are spruces).
    pub lowland_until_m: f64,
    pub highland_from_m: f64,
    /// Measured broadleaves are evergreen here (laurel forest).
    pub evergreen_broadleaves: bool,
}

/// Central European mix (beech in the valleys, spruce, fir and larch
/// higher up).
pub const CENTRAL_EUROPE: LeafMix = LeafMix {
    forest: [
        (400.0, 0.6, 0.0),
        (800.0, 0.45, 0.02),
        (1200.0, 0.25, 0.05),
        (1600.0, 0.08, 0.12),
        (2000.0, 0.02, 0.2),
    ],
    orchard: 1.0,
    urban: 0.8,
    open: 0.85,
    scrub: 0.7,
    wetland: 0.9,
    lowland_until_m: 1000.0,
    highland_from_m: 1800.0,
    evergreen_broadleaves: false,
};

/// The climate of a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Climate {
    /// Region id ("alpine-rhine", "black-forest", "canaries",
    /// "latitude").
    pub region: &'static str,
    /// Annual mean temperature at sea level (°C).
    pub sea_mean_c: f64,
    /// Half the difference between the warmest and the coldest month
    /// (°C) at sea level, and its change per km of altitude.
    pub amplitude_c: f64,
    pub amplitude_gain_per_km: f64,
    /// Temperature lapse rate (K per km).
    pub lapse_k_per_km: f64,
    /// The warmest day of the year (1–366).
    pub warmest_day: f64,
    /// Day-to-day spread of the daily mean around the climatology (°C).
    pub spread_c: f64,
    /// Precipitation at sea level (mm/day) and its increase per km.
    pub precip_mm_day: f64,
    pub precip_gain_per_km: f64,
    /// How much meadows dry out in hot summers (0 – 1; Mediterranean and
    /// subtropical climates).
    pub summer_dry: f64,
    /// Share of maize among the fields (the rest cereals).
    pub maize_share: f64,
    pub phenology: Phenology,
    pub trees: LeafMix,
}

/// The Alpine Rhine valley (Liechtenstein, Werdenberg, Vorarlberg): Vaduz
/// (457 m) 10.3 °C annual mean, January 0.9 °C, July 19.6 °C; about 900
/// mm a year in the valley, 1 600 mm at Malbun.
pub const ALPINE_RHINE: Climate = Climate {
    region: "alpine-rhine",
    sea_mean_c: 12.8,
    amplitude_c: 9.4,
    amplitude_gain_per_km: 0.0,
    lapse_k_per_km: 5.5,
    warmest_day: 200.0,
    spread_c: 3.0,
    precip_mm_day: 2.5,
    precip_gain_per_km: 1.7,
    summer_dry: 0.0,
    maize_share: 0.4,
    phenology: PHENOLOGY,
    trees: CENTRAL_EUROPE,
};

/// The Black Forest: Freiburg (240 m) 11.4 °C, Feldberg (1 490 m) 3.9 °C;
/// mostly spruce and fir above the valleys.
pub const BLACK_FOREST: Climate = Climate {
    region: "black-forest",
    sea_mean_c: 12.7,
    amplitude_c: 9.0,
    amplitude_gain_per_km: 0.0,
    lapse_k_per_km: 5.8,
    warmest_day: 200.0,
    spread_c: 3.0,
    precip_mm_day: 2.6,
    precip_gain_per_km: 1.6,
    summer_dry: 0.0,
    maize_share: 0.3,
    phenology: PHENOLOGY,
    trees: LeafMix {
        forest: [
            (300.0, 0.55, 0.0),
            (600.0, 0.35, 0.02),
            (900.0, 0.2, 0.03),
            (1200.0, 0.12, 0.02),
            (1500.0, 0.08, 0.0),
        ],
        ..CENTRAL_EUROPE
    },
};

/// The Canary Islands (Tenerife): Puerto de la Cruz about 21 °C with a
/// small yearly swing that grows with the altitude (Izaña 2 370 m: 5.6 °C
/// in January, 18 °C in August), dry summers, snow on the Teide in some
/// winters; Canary pines and evergreen laurel forest.
pub const CANARIES: Climate = Climate {
    region: "canaries",
    sea_mean_c: 21.0,
    amplitude_c: 3.3,
    amplitude_gain_per_km: 1.0,
    lapse_k_per_km: 5.0,
    warmest_day: 228.0,
    spread_c: 2.0,
    precip_mm_day: 0.8,
    precip_gain_per_km: 0.5,
    summer_dry: 0.85,
    maize_share: 0.2,
    phenology: PHENOLOGY,
    trees: LeafMix {
        forest: [
            (0.0, 0.0, 0.0),
            (1000.0, 0.0, 0.0),
            (1500.0, 0.0, 0.0),
            (2000.0, 0.0, 0.0),
            (3000.0, 0.0, 0.0),
        ],
        orchard: 0.2,
        urban: 0.2,
        open: 0.1,
        scrub: 0.1,
        wetland: 0.3,
        evergreen_broadleaves: true,
        ..CENTRAL_EUROPE
    },
};

/// The eastern Pyrenees (Andorra): Andorra la Vella (1 075 m, Roc de Sant
/// Pere, 1971–2000) 9.8 °C annual mean, January 2.2 °C, July 18.8 °C, 812
/// mm a year; about 1 200 mm on the high valleys and passes, a slightly
/// smaller yearly swing up there (Envalira, 2 400 m: about −3 °C in
/// January, 11 °C in July), a short Mediterranean summer dryness on the
/// sunny slopes. The forests are pines (mountain pine 52 %, Scots pine 32 %
/// of the trees of the first national forest inventory, 2022), fir 5 %,
/// broadleaves 11 % (birch, oaks, rowan, ash, hazel; holm oak in the
/// south's lowest valleys); no spruce and no larch (spec 4.6.1, table 6).
pub const PYRENEES: Climate = Climate {
    region: "pyrenees",
    sea_mean_c: 15.7,
    amplitude_c: 8.8,
    amplitude_gain_per_km: -0.4,
    lapse_k_per_km: 5.5,
    warmest_day: 205.0,
    spread_c: 3.0,
    precip_mm_day: 1.3,
    precip_gain_per_km: 0.85,
    summer_dry: 0.15,
    maize_share: 0.05,
    phenology: PHENOLOGY,
    trees: LeafMix {
        forest: [
            (900.0, 0.35, 0.0),
            (1300.0, 0.15, 0.0),
            (1700.0, 0.06, 0.0),
            (2000.0, 0.03, 0.0),
            (2300.0, 0.01, 0.0),
        ],
        scrub: 0.6,
        lowland_until_m: 1300.0,
        highland_from_m: 2000.0,
        ..CENTRAL_EUROPE
    },
};

/// A region with its own climate: (latitude range, longitude range,
/// climate).
pub type Region = ((f64, f64), (f64, f64), Climate);

/// The regions with their own climate.
pub const REGIONS: [Region; 4] = [
    ((46.8, 47.7), (9.2, 10.0), ALPINE_RHINE),
    ((47.4, 48.9), (7.4, 8.9), BLACK_FOREST),
    ((27.4, 29.5), (-18.4, -13.2), CANARIES),
    ((42.3, 42.8), (1.2, 2.0), PYRENEES),
];

impl Climate {
    /// The climate of a place: its region's, else one by latitude
    /// ([`Climate::by_latitude`]).
    pub fn for_place(lat: f64, lon: f64) -> Self {
        REGIONS
            .iter()
            .find(|((a, b), (c, d), _)| (*a..=*b).contains(&lat) && (*c..=*d).contains(&lon))
            .map_or_else(|| Self::by_latitude(lat), |r| r.2)
    }

    /// A rough climate by latitude alone (places outside the regions):
    /// colder and with a larger yearly swing towards the poles, the
    /// seasons turned round in the south; no deciduous trees in the
    /// tropics.
    pub fn by_latitude(lat: f64) -> Self {
        let a = lat.abs();
        let tropics = a < 23.0;
        Self {
            region: "latitude",
            sea_mean_c: 27.5 - 0.42 * (a - 12.0).max(0.0),
            amplitude_c: 1.5 + 0.23 * (a - 10.0).max(0.0),
            amplitude_gain_per_km: 0.0,
            lapse_k_per_km: 6.0,
            warmest_day: if lat >= 0.0 {
                200.0
            } else {
                200.0 - YEAR_DAYS / 2.0
            },
            spread_c: 3.0,
            precip_mm_day: 2.5,
            precip_gain_per_km: 1.5,
            summer_dry: 0.0,
            maize_share: 0.35,
            phenology: PHENOLOGY,
            trees: if tropics {
                CANARIES.trees
            } else {
                CENTRAL_EUROPE
            },
        }
    }

    /// The yearly swing at `alt_m` (°C).
    pub fn amplitude_at(&self, alt_m: f64) -> f64 {
        (self.amplitude_c + self.amplitude_gain_per_km * alt_m.max(0.0) / 1000.0).max(0.0)
    }

    /// The climatological daily mean temperature on day `day` of the year
    /// (fractional, 1 = 1 January) at `alt_m` (°C).
    pub fn temp_c(&self, day: f64, alt_m: f64) -> f64 {
        let phase = std::f64::consts::TAU * (day - self.warmest_day) / YEAR_DAYS;
        self.sea_mean_c - self.lapse_k_per_km * alt_m / 1000.0
            + self.amplitude_at(alt_m) * phase.cos()
    }

    /// Precipitation at `alt_m` (mm of water a day).
    pub fn precip_mm_day(&self, alt_m: f64) -> f64 {
        self.precip_mm_day + self.precip_gain_per_km * alt_m.max(0.0) / 1000.0
    }

    /// The first day of the phenological cycle: the coldest day.
    pub fn cycle_start(&self) -> f64 {
        self.warmest_day - YEAR_DAYS / 2.0
    }

    /// Days since the coldest day on day `day` of the year (0 … 365.25).
    pub fn cycle_day(&self, day: f64) -> f64 {
        (day - self.cycle_start()).rem_euclid(YEAR_DAYS)
    }
}

// --- Maths ------------------------------------------------------------------

/// The standard normal distribution function (Abramowitz & Stegun 7.1.26,
/// error below 1.5 · 10⁻⁷).
pub fn normal_cdf(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

fn normal_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / (std::f64::consts::TAU).sqrt()
}

/// E[max(0, X)] for X normal with mean `mean` and spread `sd`: the
/// expected degree days of a day whose mean is `mean` above the base.
pub fn expected_above(mean: f64, sd: f64) -> f64 {
    if sd <= 0.0 {
        return mean.max(0.0);
    }
    let z = mean / sd;
    sd * normal_pdf(z) + mean * normal_cdf(z)
}

/// 0 below `lo`, 1 above `hi`, linear in between.
pub fn ramp(x: f64, (lo, hi): (f64, f64)) -> f64 {
    if hi <= lo {
        return if x >= lo { 1.0 } else { 0.0 };
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn smooth(x: f64, lohi: (f64, f64)) -> f64 {
    let t = ramp(x, lohi);
    t * t * (3.0 - 2.0 * t)
}

// --- The model ----------------------------------------------------------------

/// Snow: it falls below this daily mean (°C, spread like the temperature).
pub const SNOW_BELOW_C: f64 = 1.0;
/// Melt per degree day above 0 °C (mm of water).
pub const MELT_MM_PER_DEGREE_DAY: f64 = 3.5;
/// Snow water (mm) that covers the ground to 63 % (cover = 1 − e^(−w/x)).
pub const COVER_MM: f64 = 12.0;
/// The spread of the temperature for snowfall and melt (°C; larger than
/// the day-to-day spread: showers and nights).
pub const SNOW_SPREAD_C: f64 = 3.5;

/// The climate of a place evaluated into altitude bands: per band and day
/// the degree-day sums and the snow cover of the climatological year.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonModel {
    climate: Climate,
    /// Growing degree days since the coldest day, per band at whole days
    /// 0 … 366 of the cycle.
    gdd: Vec<f32>,
    /// Cold degree days since the autumn start, per band and cycle day.
    cdd: Vec<f32>,
    /// Snow cover (0 – 1) per band and day of the year 1 … 366.
    snow: Vec<f32>,
}

/// Altitude of band `b` (m).
pub fn band_alt(b: usize) -> f64 {
    MIN_ALT_M + b as f64 * BAND_M
}

impl SeasonModel {
    pub fn new(climate: Climate) -> Self {
        let mut gdd = Vec::with_capacity(BANDS * 367);
        let mut cdd = Vec::with_capacity(BANDS * 367);
        let mut snow = Vec::with_capacity(BANDS * 366);
        let start = climate.cycle_start();
        let autumn = YEAR_DAYS / 2.0 + AUTUMN_START_DAYS;
        for b in 0..BANDS {
            let alt = band_alt(b);
            let (mut g, mut c) = (0.0f64, 0.0f64);
            for i in 0..=366usize {
                gdd.push(g as f32);
                cdd.push(c as f32);
                let mid = i as f64 + 0.5;
                let t = climate.temp_c(start + mid, alt);
                g += expected_above(t - GDD_BASE_C, climate.spread_c);
                if mid >= autumn {
                    c += expected_above(CDD_BASE_C - t, climate.spread_c);
                }
            }
            // The snowpack: three climatological years from 1 January,
            // the last one kept (a steady state; glaciers stay capped).
            let mut swe = 0.0f64;
            let p = climate.precip_mm_day(alt);
            for year in 0..3 {
                for d in 1..=366usize {
                    let t = climate.temp_c(d as f64 - 0.5, alt);
                    let fall = p * normal_cdf((SNOW_BELOW_C - t) / SNOW_SPREAD_C);
                    let melt = MELT_MM_PER_DEGREE_DAY * expected_above(t, SNOW_SPREAD_C);
                    swe = (swe + fall - melt).clamp(0.0, 5000.0);
                    if year == 2 {
                        snow.push((1.0 - (-swe / COVER_MM).exp()) as f32);
                    }
                }
            }
        }
        Self {
            climate,
            gdd,
            cdd,
            snow,
        }
    }

    /// The model of a place's climate.
    pub fn for_place(lat: f64, lon: f64) -> Self {
        Self::new(Climate::for_place(lat, lon))
    }

    pub fn climate(&self) -> &Climate {
        &self.climate
    }

    fn cycle_value(table: &[f32], b: usize, t: f64) -> f32 {
        let i = (t.floor() as usize).min(365);
        let f = (t - i as f64).clamp(0.0, 1.0) as f32;
        let row = &table[b * 367..(b + 1) * 367];
        row[i] + (row[i + 1] - row[i]) * f
    }

    fn band_state(&self, b: usize, day: f64) -> State {
        let t = self.climate.cycle_day(day);
        let d = day.rem_euclid(YEAR_DAYS);
        // Snow per day of the year (1 … 366), interpolated.
        let x = (d - 0.5).rem_euclid(366.0);
        let i = (x.floor() as usize).min(365);
        let f = (x - i as f64) as f32;
        let row = &self.snow[b * 366..(b + 1) * 366];
        let snow = row[i] + (row[(i + 1) % 366] - row[i]) * f;
        State {
            gdd: Self::cycle_value(&self.gdd, b, t),
            cdd: Self::cycle_value(&self.cdd, b, t),
            snow,
            temp_c: self.climate.temp_c(day, band_alt(b)) as f32,
        }
    }

    /// The state at one altitude on day `day` (interpolated between the
    /// two bands around it, as [`SeasonDay::state`]; cheaper than a whole
    /// [`SeasonModel::day`] – the calendar's bands, spec §14 Phase 9 S9).
    pub fn state_at(&self, day: f64, alt_m: f64) -> State {
        let x = ((alt_m - MIN_ALT_M) / BAND_M).clamp(0.0, (BANDS - 1) as f64);
        let i = (x.floor() as usize).min(BANDS - 2);
        State::lerp(
            &self.band_state(i, day),
            &self.band_state(i + 1, day),
            (x - i as f64) as f32,
        )
    }

    /// The season on day `day` of the year (fractional, 1 = 1 January).
    pub fn day(&self, day: f64) -> SeasonDay {
        SeasonDay {
            day,
            climate: self.climate,
            states: (0..BANDS).map(|b| self.band_state(b, day)).collect(),
        }
    }
}

/// The phenological state at one altitude on one day.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct State {
    /// Growing degree days above 5 °C since the coldest day.
    pub gdd: f32,
    /// Cold degree days below 20 °C since three weeks after the warmest
    /// day (0 before).
    pub cdd: f32,
    /// Climatological snow cover (0 – 1).
    pub snow: f32,
    /// Climatological daily mean temperature (°C).
    pub temp_c: f32,
}

impl State {
    fn lerp(a: &State, b: &State, t: f32) -> State {
        let l = |x: f32, y: f32| x + (y - x) * t;
        State {
            gdd: l(a.gdd, b.gdd),
            cdd: l(a.cdd, b.cdd),
            snow: l(a.snow, b.snow),
            temp_c: l(a.temp_c, b.temp_c),
        }
    }
}

// --- Vegetation -----------------------------------------------------------------

/// How a tree carries its leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafType {
    /// Conifers (and evergreen broadleaves): green all year.
    Evergreen,
    /// Deciduous broadleaves: leaf out, green, colour, bare.
    Deciduous,
    /// Larches: needles that turn golden and fall.
    Larch,
}

/// Where a tree stands, for the leaf-type fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeSetting {
    Forest,
    Orchard,
    Urban,
    /// Lone trees in meadows and fields.
    Open,
    Scrub,
    Wetland,
}

/// The dominant leaf type a package measured (spec §14 Phase 8 T3: the
/// Copernicus HRL forest type; no package carries it yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasuredLeaf {
    Broadleaved,
    Coniferous,
}

impl LeafMix {
    /// (deciduous, larch) share of forest trees at `alt_m`.
    pub fn forest_shares(&self, alt_m: f64) -> (f64, f64) {
        let f = &self.forest;
        if alt_m <= f[0].0 {
            return (f[0].1, f[0].2);
        }
        for w in f.windows(2) {
            if alt_m <= w[1].0 {
                let t = (alt_m - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
                return (
                    w[0].1 + (w[1].1 - w[0].1) * t,
                    w[0].2 + (w[1].2 - w[0].2) * t,
                );
            }
        }
        let l = f[f.len() - 1];
        (l.1, l.2)
    }

    /// (deciduous, larch) share of trees in `setting` at `alt_m`.
    pub fn shares(&self, setting: TreeSetting, alt_m: f64) -> (f64, f64) {
        let base = match setting {
            TreeSetting::Forest => return self.forest_shares(alt_m),
            TreeSetting::Orchard => self.orchard,
            TreeSetting::Urban => self.urban,
            TreeSetting::Open => self.open,
            TreeSetting::Scrub => self.scrub,
            TreeSetting::Wetland => self.wetland,
        };
        let high = ramp(alt_m, (self.lowland_until_m, self.highland_from_m));
        let (fd, fl) = self.forest_shares(alt_m);
        let d = (base * (1.0 - 0.8 * high)).max(fd);
        (d, fl.min(1.0 - d))
    }

    /// (deciduous, larch) share of trees in `setting` at `alt_m` where the
    /// package measured a broadleaf share `broadleaf` (0 … 1, spec §14
    /// Phase 8 T3): the broadleaves deciduous (evergreen where
    /// [`LeafMix::evergreen_broadleaves`]), the conifers larches by the
    /// region's larch share among its conifers – what [`LeafMix::leaf_type`]
    /// gives a wood of measured trees on average (the far canopy).
    pub fn measured_shares(&self, setting: TreeSetting, alt_m: f64, broadleaf: f64) -> (f64, f64) {
        let (d, l) = self.shares(setting, alt_m);
        let larch_of_conifers = (l / (1.0 - d).max(1e-6)).clamp(0.0, 1.0);
        let b = broadleaf.clamp(0.0, 1.0);
        let dec = if self.evergreen_broadleaves { 0.0 } else { b };
        (dec, (1.0 - b) * larch_of_conifers)
    }

    /// The leaf type of a tree: the measured type if the package has it
    /// (a coniferous tree is a larch by the larch share among the
    /// conifers), else by the shares of its setting and altitude.
    /// `pick` (0 … 1) is the tree's own hash, so the choice is
    /// deterministic.
    pub fn leaf_type(
        &self,
        setting: TreeSetting,
        alt_m: f64,
        measured: Option<MeasuredLeaf>,
        pick: f32,
    ) -> LeafType {
        let pick = f64::from(pick);
        let (d, l) = self.shares(setting, alt_m);
        match measured {
            Some(MeasuredLeaf::Broadleaved) if self.evergreen_broadleaves => LeafType::Evergreen,
            Some(MeasuredLeaf::Broadleaved) => LeafType::Deciduous,
            Some(MeasuredLeaf::Coniferous) => {
                if pick < l / (1.0 - d).max(1e-6) {
                    LeafType::Larch
                } else {
                    LeafType::Evergreen
                }
            }
            None if pick < d => LeafType::Deciduous,
            None if pick < d + l => LeafType::Larch,
            None => LeafType::Evergreen,
        }
    }
}

/// The leaves of a tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Foliage {
    /// How much of the crown is in leaf (0 bare – 1 full).
    pub leaves: f32,
    /// Young spring green (0 – 1).
    pub fresh: f32,
    /// Autumn colour (0 green – 1 full colour).
    pub autumn: f32,
}

impl Foliage {
    pub const EVERGREEN: Foliage = Foliage {
        leaves: 1.0,
        fresh: 0.0,
        autumn: 0.0,
    };
}

/// The state of meadow grass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grass {
    /// 0 dormant (winter straw) – 1 lush.
    pub green: f32,
    /// Dried out by summer heat (0 – 1).
    pub dry: f32,
}

/// The kinds of field crops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crop {
    /// Winter cereals (wheat, barley): green in spring, golden in July,
    /// stubble, ploughed, sown again in autumn.
    Cereal,
    /// Maize: bare soil in spring, sown in May, tall and green all
    /// summer, harvested in autumn.
    Maize,
}

/// A crop calendar in growing degree days: each phase from `.0` to `.1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropCalendar {
    /// From the start (sown soil / bare soil) to full green.
    pub sprout: (f32, f32),
    /// From green to ripe (golden / dry).
    pub ripen: (f32, f32),
    /// Harvested: stubble.
    pub harvest: (f32, f32),
    /// Ploughed: bare soil.
    pub plough: (f32, f32),
    /// Sown again: soil with the first green (winter crops).
    pub sow: (f32, f32),
}

/// Never (a phase a crop does not have within its year).
pub const NEVER: (f32, f32) = (1.0e6, 1.0e6 + 1.0);

/// Winter cereals in the Rhine valley: green from mid-March, ripening
/// from early July, harvested late July, ploughed late August, sown again
/// in October.
pub const CEREAL: CropCalendar = CropCalendar {
    sprout: (30.0, 150.0),
    ripen: (900.0, 1150.0),
    harvest: (1250.0, 1320.0),
    plough: (1600.0, 1700.0),
    sow: (2150.0, 2280.0),
};

/// Maize: sown in early May (bare soil before), full green from mid-June,
/// drying from mid-September, harvested in October; the stubble stays
/// over winter.
pub const MAIZE: CropCalendar = CropCalendar {
    sprout: (250.0, 700.0),
    ripen: (1900.0, 2150.0),
    harvest: (2150.0, 2220.0),
    plough: NEVER,
    sow: NEVER,
};

/// Cold degree days after which fields still standing are cut (high
/// fields where the crop never ripens).
pub const FROST_CDD: (f32, f32) = (900.0, 1000.0);

/// A field's stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropStage {
    /// Sown soil with a little green (winter cereals over winter).
    Sown,
    /// Bare soil (maize before sowing).
    Bare,
    Sprouting,
    Green,
    Ripening,
    Ripe,
    Stubble,
    Ploughed,
}

impl CropStage {
    pub fn name(self) -> &'static str {
        match self {
            CropStage::Sown => "sown",
            CropStage::Bare => "bare soil",
            CropStage::Sprouting => "sprouting",
            CropStage::Green => "green",
            CropStage::Ripening => "ripening",
            CropStage::Ripe => "ripe",
            CropStage::Stubble => "stubble",
            CropStage::Ploughed => "ploughed",
        }
    }
}

impl Crop {
    pub fn calendar(self) -> &'static CropCalendar {
        match self {
            Crop::Cereal => &CEREAL,
            Crop::Maize => &MAIZE,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Crop::Cereal => "cereal",
            Crop::Maize => "maize",
        }
    }

    /// The stage of a field at `gdd` / `cdd` (see [`State`]).
    pub fn stage(self, gdd: f32, cdd: f32) -> CropStage {
        let c = self.calendar();
        let mid = |p: (f32, f32)| (p.0 + p.1) / 2.0;
        let stage = if gdd < c.sprout.0 {
            match self {
                Crop::Cereal => CropStage::Sown,
                Crop::Maize => CropStage::Bare,
            }
        } else if gdd < c.sprout.1 {
            CropStage::Sprouting
        } else if gdd < c.ripen.0 {
            CropStage::Green
        } else if gdd < c.ripen.1 {
            CropStage::Ripening
        } else if gdd < c.harvest.0 {
            CropStage::Ripe
        } else if gdd < mid(c.plough) {
            CropStage::Stubble
        } else if gdd < mid(c.sow) {
            CropStage::Ploughed
        } else {
            CropStage::Sown
        };
        let standing = matches!(
            stage,
            CropStage::Sprouting | CropStage::Green | CropStage::Ripening | CropStage::Ripe
        );
        if standing && cdd >= mid(FROST_CDD) {
            CropStage::Stubble
        } else {
            stage
        }
    }
}

// --- One day ----------------------------------------------------------------------

/// A linear ramp over the altitude: `clamp(a + b · alt, 0, 1)` – what the
/// shaders evaluate per fragment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ramp {
    pub a: f32,
    pub b: f32,
}

impl Ramp {
    /// The same value everywhere.
    pub fn constant(v: f32) -> Self {
        Self {
            a: v.clamp(0.0, 1.0),
            b: 0.0,
        }
    }

    pub fn at(&self, alt_m: f64) -> f32 {
        (f64::from(self.a) + f64::from(self.b) * alt_m).clamp(0.0, 1.0) as f32
    }

    /// The ramp through a process `f` (0 – 1, monotone in the altitude):
    /// through its 50 % altitude with the slope between its 10 % and
    /// 90 % altitudes; constant at its value at `around` if it does not
    /// cross 50 % between the lowest and the highest band.
    pub fn fit(f: impl Fn(f64) -> f64, around: f64) -> Self {
        let (lo, hi) = (MIN_ALT_M, band_alt(BANDS - 1));
        let cross = |level: f64| -> Option<f64> {
            let mut prev = (lo, f(lo) - level);
            let mut alt = lo + BAND_M;
            while alt <= hi + 1e-6 {
                let v = f(alt) - level;
                if (prev.1 <= 0.0) != (v <= 0.0) {
                    // Bisection within the band.
                    let (mut a, mut b) = (prev.0, alt);
                    let sa = prev.1 <= 0.0;
                    for _ in 0..30 {
                        let m = (a + b) / 2.0;
                        if (f(m) - level <= 0.0) == sa {
                            a = m;
                        } else {
                            b = m;
                        }
                    }
                    return Some((a + b) / 2.0);
                }
                prev = (alt, v);
                alt += BAND_M;
            }
            None
        };
        let Some(h50) = cross(0.5) else {
            return Self::constant(f(around) as f32);
        };
        let rising = f(h50 + BAND_M) >= f(h50 - BAND_M);
        let (h10, h90) = (cross(0.1), cross(0.9));
        let width = match (h10, h90) {
            (Some(a), Some(b)) => (b - a).abs() / 0.8,
            (Some(a), None) => (h50 - a).abs() / 0.4,
            (None, Some(b)) => (b - h50).abs() / 0.4,
            (None, None) => BAND_M,
        }
        .max(10.0);
        let b = if rising { 1.0 / width } else { -1.0 / width };
        Self {
            a: (0.5 - b * h50) as f32,
            b: b as f32,
        }
    }
}

/// A straight line over the altitude, `a + b · alt` (degree-day sums near
/// the rider).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Linear {
    pub a: f32,
    pub b: f32,
}

impl Linear {
    pub fn at(&self, alt_m: f64) -> f32 {
        (f64::from(self.a) + f64::from(self.b) * alt_m) as f32
    }

    /// Through `f` at `around` ± 300 m.
    pub fn fit(f: impl Fn(f64) -> f64, around: f64) -> Self {
        let d = 300.0;
        let b = (f(around + d) - f(around - d)) / (2.0 * d);
        Self {
            a: (f(around) - b * around) as f32,
            b: b as f32,
        }
    }
}

/// The season as the shaders take it (altitude ramps).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SeasonRamps {
    /// Climatological snow cover.
    pub snow: Ramp,
    /// Grass green-up and dormancy, summer dryness.
    pub grass_up: Ramp,
    pub grass_dormant: Ramp,
    pub dry: Ramp,
    /// Degree-day sums near the rider (crops).
    pub gdd: Linear,
    pub cdd: Linear,
}

/// A measured snow depth (real weather, spec §14 Phase 9 9F): at an
/// altitude, in metres of snow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnowObservation {
    pub alt_m: f64,
    pub depth_m: f64,
}

/// Snow depth (m) from which the ground counts as covered.
pub const COVERED_DEPTH_M: f64 = 0.05;

/// The snow lying at a place: the climatological (or observed) snowpack
/// of the date and the snow the session's weather let fall, whichever
/// covers more (spec 9C; decided in 9I-S8).
/// The season is looked at in buckets of this many days (the renderer
/// rebuilds its tiles when the bucket changes; the physics of wet leaves
/// uses the same day, spec 4.6.2 Q6).
pub const BUCKET_DAYS: u32 = 4;

/// The season bucket of a day of the year (1 …).
pub fn season_bucket(day_of_year: u32) -> u32 {
    day_of_year.saturating_sub(1) / BUCKET_DAYS
}

/// The day of the year a bucket stands for (its middle).
pub fn bucket_day(bucket: u32) -> f64 {
    f64::from(bucket * BUCKET_DAYS) + f64::from(BUCKET_DAYS) / 2.0 + 0.5
}

pub fn lying_snow(season: f32, weather: f32) -> f32 {
    season.max(weather).clamp(0.0, 1.0)
}

/// The season of one day at a place.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonDay {
    pub day: f64,
    pub climate: Climate,
    states: Vec<State>,
}

impl SeasonDay {
    /// The state at `alt_m` (interpolated between the bands, the ends
    /// held).
    pub fn state(&self, alt_m: f64) -> State {
        let x = ((alt_m - MIN_ALT_M) / BAND_M).clamp(0.0, (BANDS - 1) as f64);
        let i = (x.floor() as usize).min(BANDS - 2);
        State::lerp(&self.states[i], &self.states[i + 1], (x - i as f64) as f32)
    }

    pub fn phenology(&self) -> &Phenology {
        &self.climate.phenology
    }

    /// The leaves of a tree of `leaf` type at `alt_m`; `jitter` (−1 … 1,
    /// the tree's own) moves its thresholds by up to ±12 %: not every tree
    /// turns on the same day.
    pub fn foliage(&self, leaf: LeafType, alt_m: f64, jitter: f32) -> Foliage {
        let s = self.state(alt_m);
        foliage(self.phenology(), leaf, &s, jitter)
    }

    /// [`SeasonDay::foliage`] with a species' [`Timing`].
    pub fn foliage_timed(
        &self,
        leaf: LeafType,
        alt_m: f64,
        jitter: f32,
        timing: Timing,
    ) -> Foliage {
        let s = self.state(alt_m);
        foliage_timed(self.phenology(), leaf, &s, jitter, timing)
    }

    /// The meadow grass at `alt_m`.
    pub fn grass(&self, alt_m: f64) -> Grass {
        let s = self.state(alt_m);
        grass(&self.climate, &s)
    }

    /// The stage of a field of `crop` at `alt_m`.
    pub fn crop(&self, crop: Crop, alt_m: f64) -> CropStage {
        let s = self.state(alt_m);
        crop.stage(s.gdd, s.cdd)
    }

    /// The climatological snow cover at `alt_m` (0 – 1).
    pub fn snow(&self, alt_m: f64) -> f32 {
        self.state(alt_m).snow
    }

    /// The leaf fall at `alt_m` (0 in summer, 1 when the leaves are down;
    /// spec 4.6.2): the leaves come down with the autumn colour and lie
    /// through the winter, the fresh green of spring covers the last of
    /// them. The renderer's leaf patches and the leaves on the road, and
    /// the physics of wet leaves (4.6.2 Q6), count with it. Where the
    /// region's broadleaves are evergreen (laurel forest, the tropics) only
    /// its few deciduous lone trees drop leaves: the fall is scaled by
    /// [`SeasonDay::leaf_fall_share`] (spec §14 Phase 9 Q13).
    pub fn leaf_fall(&self, alt_m: f64) -> f32 {
        let f = self.foliage(LeafType::Deciduous, alt_m, 0.0);
        let fallen = (1.0 - f.leaves).clamp(0.0, 1.0);
        ((0.5 * f.autumn + fallen) * (1.0 - 0.6 * f.fresh)).clamp(0.0, 1.0) * self.leaf_fall_share()
    }

    /// How much of a deciduous place's leaf fall a place of this climate
    /// has: 1, or – where the broadleaves are evergreen – the share of
    /// deciduous lone trees ([`LeafMix::open`]: 0.1 on the Canary Islands).
    pub fn leaf_fall_share(&self) -> f32 {
        let t = &self.climate.trees;
        if t.evergreen_broadleaves {
            t.open.clamp(0.0, 1.0) as f32
        } else {
            1.0
        }
    }

    /// The climatological snow line: the lowest altitude where half the
    /// ground is covered (`None`: no snow below 6 000 m).
    pub fn snow_line_m(&self) -> Option<f64> {
        if self.states[0].snow >= 0.5 {
            return Some(MIN_ALT_M);
        }
        let r = Ramp::fit(|h| f64::from(self.snow(h)), 0.0);
        (r.b > 0.0).then(|| (0.5 - f64::from(r.a)) / f64::from(r.b))
    }

    /// The snow ramp: climatology, or – with a measured snow depth from
    /// real weather – the climatological ramp moved so that it matches
    /// the measurement at its altitude (covered from
    /// [`COVERED_DEPTH_M`]: the line at most there; no snow: the line
    /// above it).
    pub fn snow_ramp(&self, around: f64, observed: Option<SnowObservation>) -> Ramp {
        let clim = Ramp::fit(|h| f64::from(self.snow(h)), around);
        let Some(o) = observed else { return clim };
        let b = if clim.b > 0.0 { clim.b } else { 1.0 / 300.0 };
        let now = clim.at(o.alt_m);
        let want = if o.depth_m >= COVERED_DEPTH_M {
            ((0.6 + o.depth_m * 4.0) as f32).min(1.0).max(now)
        } else if o.depth_m > 0.0 {
            (o.depth_m / COVERED_DEPTH_M * 0.5) as f32
        } else {
            0.0
        };
        if (now - want).abs() < 1e-3 && clim.b > 0.0 {
            return clim;
        }
        // Through `want` at the observation, rising with the altitude.
        Ramp {
            a: (f64::from(want) - f64::from(b) * o.alt_m) as f32,
            b,
        }
    }

    /// The ramps for the shaders, fitted around the rider's altitude.
    pub fn ramps(&self, rider_alt_m: f64, observed: Option<SnowObservation>) -> SeasonRamps {
        let p = *self.phenology();
        let c = self.climate;
        SeasonRamps {
            snow: self.snow_ramp(rider_alt_m, observed),
            grass_up: Ramp::fit(
                |h| ramp(f64::from(self.state(h).gdd), p.grass_up_gdd),
                rider_alt_m,
            ),
            grass_dormant: Ramp::fit(
                |h| ramp(f64::from(self.state(h).cdd), p.grass_dormant_cdd),
                rider_alt_m,
            ),
            dry: Ramp::fit(
                |h| c.summer_dry * ramp(f64::from(self.state(h).temp_c), p.dry_c),
                rider_alt_m,
            ),
            gdd: Linear::fit(|h| f64::from(self.state(h).gdd), rider_alt_m),
            cdd: Linear::fit(|h| f64::from(self.state(h).cdd), rider_alt_m),
        }
    }

    /// One line for the console: the season at `alt_m`.
    pub fn describe(&self, alt_m: f64) -> String {
        let s = self.state(alt_m);
        let f = self.foliage(LeafType::Deciduous, alt_m, 0.0);
        let leaves = if f.leaves < 0.05 {
            "bare".to_string()
        } else if f.autumn > 0.2 {
            format!(
                "{:.0} % in leaf, {:.0} % coloured",
                f.leaves * 100.0,
                f.autumn * 100.0
            )
        } else if f.fresh > 0.3 {
            format!("{:.0} % in leaf, fresh green", f.leaves * 100.0)
        } else {
            format!("{:.0} % in leaf", f.leaves * 100.0)
        };
        let g = self.grass(alt_m);
        let line = match self.snow_line_m() {
            Some(h) if h <= MIN_ALT_M => "everywhere".to_string(),
            Some(h) => format!("{h:.0} m"),
            None => "none".to_string(),
        };
        format!(
            "day {:.0} at {:.0} m ({}): deciduous trees {leaves}; grass {:.0} % green{}; \
             cereal {}, maize {}; climatological snow {:.0} % (snow line {line}); \
             GDD {:.0}, CDD {:.0}, {:.1} °C",
            self.day,
            alt_m,
            self.climate.region,
            g.green * 100.0,
            if g.dry > 0.05 {
                format!(", {:.0} % dry", g.dry * 100.0)
            } else {
                String::new()
            },
            Crop::Cereal.stage(s.gdd, s.cdd).name(),
            Crop::Maize.stage(s.gdd, s.cdd).name(),
            s.snow * 100.0,
            s.gdd,
            s.cdd,
            s.temp_c,
        )
    }
}

/// The leaves of a tree of `leaf` type in state `s` (see
/// [`SeasonDay::foliage`]).
pub fn foliage(p: &Phenology, leaf: LeafType, s: &State, jitter: f32) -> Foliage {
    foliage_timed(p, leaf, s, jitter, Timing::DEFAULT)
}

/// When a species leafs out and turns compared with the beech the
/// thresholds are calibrated on (spec §4.6.1 table 5): factors on the
/// spring (growing degree days) and the autumn (cold degree days)
/// thresholds – birches and willows early in spring, ashes and oaks late;
/// ashes drop their leaves early, oaks and alders late.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub spring: f64,
    pub autumn: f64,
}

impl Timing {
    pub const DEFAULT: Timing = Timing {
        spring: 1.0,
        autumn: 1.0,
    };
}

/// [`foliage`] with a species' [`Timing`].
pub fn foliage_timed(
    p: &Phenology,
    leaf: LeafType,
    s: &State,
    jitter: f32,
    timing: Timing,
) -> Foliage {
    let j = 1.0 + 0.12 * f64::from(jitter.clamp(-1.0, 1.0));
    let k = j * timing.spring;
    let ka = j * timing.autumn;
    foliage_k(p, leaf, s, k, ka)
}

fn foliage_k(p: &Phenology, leaf: LeafType, s: &State, k: f64, ka: f64) -> Foliage {
    let (gdd, cdd) = (f64::from(s.gdd), f64::from(s.cdd));
    let (burst, full, shift) = match leaf {
        LeafType::Evergreen => return Foliage::EVERGREEN,
        LeafType::Deciduous => (p.budburst_gdd, p.leaf_full_gdd, 0.0),
        LeafType::Larch => (
            p.larch_budburst_gdd,
            p.larch_full_gdd,
            p.larch_autumn_shift_cdd,
        ),
    };
    let out = smooth(gdd, (burst * k, full * k));
    let fall = smooth(
        cdd,
        (
            (p.fall_start_cdd + shift) * ka,
            (p.fall_end_cdd + shift) * ka,
        ),
    );
    let fresh = out * (1.0 - smooth(gdd, (full * k, p.fresh_until_gdd * k)));
    let autumn = smooth(
        cdd,
        (
            (p.colour_start_cdd + shift) * ka,
            (p.colour_full_cdd + shift) * ka,
        ),
    );
    Foliage {
        leaves: (out * (1.0 - fall)) as f32,
        fresh: fresh as f32,
        autumn: autumn as f32,
    }
}

/// The meadow grass in state `s`.
pub fn grass(c: &Climate, s: &State) -> Grass {
    let p = &c.phenology;
    let up = ramp(f64::from(s.gdd), p.grass_up_gdd);
    let down = ramp(f64::from(s.cdd), p.grass_dormant_cdd);
    Grass {
        green: (up * (1.0 - down)) as f32,
        dry: (c.summer_dry * ramp(f64::from(s.temp_c), p.dry_c)) as f32,
    }
}

// --- How marked the seasons are -------------------------------------------------

/// The seasons are marked by the leaves where at least this share of the
/// trees a rider sees turns and drops its leaves (deciduous broadleaves
/// and larches; spec §14 Phase 9 Q13, *Worlds with weak or no seasons*).
/// Central Europe has 17 – 85 % up to the tree line, Tenerife's laurel
/// forest and Canary pines none (its lone fruit trees and chestnuts: 10 %).
pub const LEAVES_MIN: f64 = 0.15;
/// A yearly temperature range (the warmest minus the coldest day's
/// climatological mean) of at least this is a marked temperature year
/// (°C): Vaduz 18.8, Puerto de la Cruz 6.6, the Teide summit 14.
pub const MARKED_RANGE_C: f64 = 10.0;
/// A snow season: at least this many days of lying snow (climatological
/// cover ≥ 50 %) a year at the altitude.
pub const SNOW_SEASON_DAYS: u32 = 14;
/// The leaves turn at a place when the deciduous phenology runs through a
/// whole cycle there: some day at least this coloured (0 – 1) …
const CYCLE_AUTUMN: f32 = 0.15;
/// … and some day bare to at most this much leaf.
const CYCLE_BARE: f32 = 0.15;

/// How marked the seasons are at one place and altitude (spec §14 Phase 9
/// Q13): pure and deterministic, from the place's climate (the season
/// model) and the trees around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonStrength {
    /// The yearly temperature range at the altitude (°C, twice
    /// [`Climate::amplitude_at`]).
    pub range_c: f64,
    /// The share of the trees around that turn and drop their leaves
    /// (0 – 1): the larger of the forests' (by the measured broadleaf share
    /// where the package has one – deciduous only where the region's
    /// broadleaves are) and the lone trees' in the open – a rider sees
    /// both.
    pub deciduous: f64,
    /// Whether the deciduous phenology runs through a whole cycle at the
    /// altitude (colours some day, bare some day).
    pub leaf_cycle: bool,
    /// Days of lying snow a year (climatological cover ≥ 50 %).
    pub snow_days: u32,
}

/// The levels of [`SeasonStrength::level`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeasonLevel {
    /// The leaves turn: spring green-up, autumn colours and leaf fall
    /// (every season jump means something).
    Marked,
    /// The leaves do not turn, but there is a snow season at the
    /// altitude (Las Cañadas, the Teide).
    SnowOnly,
    /// The leaves do not turn and no snow lies (the Canary coast): only
    /// the sun, the day length and the meadows change over the year.
    Weak,
}

impl SeasonStrength {
    /// The strength at `alt_m` in `model`'s climate; `broadleaf` the
    /// measured broadleaf share of the forests around (0 – 1, the package's
    /// forest section), `None` for the region's mix.
    pub fn at(model: &SeasonModel, alt_m: f64, broadleaf: Option<f64>) -> Self {
        let c = model.climate();
        let mix = &c.trees;
        let (fd, fl) = match broadleaf {
            Some(b) => mix.measured_shares(TreeSetting::Forest, alt_m, b),
            None => mix.forest_shares(alt_m),
        };
        let (od, ol) = mix.shares(TreeSetting::Open, alt_m);
        let deciduous = (fd + fl).max(od + ol).clamp(0.0, 1.0);
        let (mut coloured, mut bare, mut snow_days) = (0.0f32, 1.0f32, 0u32);
        for d in 1..=365u32 {
            let s = model.state_at(f64::from(d), alt_m);
            if s.snow >= 0.5 {
                snow_days += 1;
            }
            let f = foliage(&c.phenology, LeafType::Deciduous, &s, 0.0);
            coloured = coloured.max(f.autumn);
            bare = bare.min(f.leaves);
        }
        Self {
            range_c: 2.0 * c.amplitude_at(alt_m),
            deciduous,
            leaf_cycle: coloured >= CYCLE_AUTUMN && bare <= CYCLE_BARE,
            snow_days,
        }
    }

    /// The leaves turn here (see [`LEAVES_MIN`]): spring green-up, autumn
    /// colours and leaf fall mean something.
    pub fn leaves_turn(&self) -> bool {
        self.deciduous >= LEAVES_MIN && self.leaf_cycle
    }

    /// A snow season at the altitude (see [`SNOW_SEASON_DAYS`]).
    pub fn snow_season(&self) -> bool {
        self.snow_days >= SNOW_SEASON_DAYS
    }

    /// A marked temperature year (see [`MARKED_RANGE_C`]).
    pub fn marked_temperature(&self) -> bool {
        self.range_c >= MARKED_RANGE_C
    }

    pub fn level(&self) -> SeasonLevel {
        if self.leaves_turn() {
            SeasonLevel::Marked
        } else if self.snow_season() {
            SeasonLevel::SnowOnly
        } else {
            SeasonLevel::Weak
        }
    }

    /// One line on the seasons where they are weak (`None` where the
    /// leaves turn): "Weak seasons here: evergreen trees, only 7 °C
    /// between the coolest and the warmest days, no lying snow."
    pub fn note(&self) -> Option<String> {
        let what = match self.level() {
            SeasonLevel::Marked => return None,
            SeasonLevel::SnowOnly => "Weak seasons here but for the snow",
            SeasonLevel::Weak => "Weak seasons here",
        };
        let trees = if self.deciduous < LEAVES_MIN {
            "evergreen trees"
        } else {
            "the leaves hardly turn"
        };
        let only = if self.marked_temperature() {
            ""
        } else {
            "only "
        };
        let snow = if self.snow_season() {
            format!("snow lies {} days a year", self.snow_days)
        } else {
            "no lying snow".to_owned()
        };
        Some(format!(
            "{what}: {trees}, {only}{:.0} °C between the coolest and the warmest days, {snow}.",
            self.range_c
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VADUZ: f64 = 455.0;

    #[test]
    fn species_timing_moves_spring_and_autumn_apart() {
        let m = liechtenstein();
        let late = Timing {
            spring: 1.35,
            autumn: 0.85,
        };
        // Early May: the beech is out, the ash still bare-ish.
        let may = m.day(125.0);
        let beech = may.foliage(LeafType::Deciduous, VADUZ, 0.0);
        let ash = may.foliage_timed(LeafType::Deciduous, VADUZ, 0.0, late);
        assert!(ash.leaves < beech.leaves, "{ash:?} {beech:?}");
        // Late October: the ash is already further on (colour / bare).
        let oct = m.day(295.0);
        let beech = oct.foliage(LeafType::Deciduous, VADUZ, 0.0);
        let ash = oct.foliage_timed(LeafType::Deciduous, VADUZ, 0.0, late);
        assert!(ash.autumn >= beech.autumn && ash.leaves <= beech.leaves);
        // The default timing is the plain foliage.
        assert_eq!(
            oct.foliage_timed(LeafType::Deciduous, VADUZ, 0.3, Timing::DEFAULT),
            oct.foliage(LeafType::Deciduous, VADUZ, 0.3)
        );
    }
    const MALBUN: f64 = 1600.0;

    fn liechtenstein() -> SeasonModel {
        SeasonModel::for_place(47.14, 9.52)
    }

    /// The day of the year when `f` first reaches `level` (scanning the
    /// year from 1 January).
    fn first_day(m: &SeasonModel, f: impl Fn(&SeasonDay) -> f64, level: f64) -> f64 {
        first_day_from(m, 1, f, level)
    }

    /// The same from day `from` (autumn: after the summer).
    fn first_day_from(
        m: &SeasonModel,
        from: u32,
        f: impl Fn(&SeasonDay) -> f64,
        level: f64,
    ) -> f64 {
        (from..=366)
            .map(f64::from)
            .find(|&d| f(&m.day(d)) >= level)
            .unwrap_or(f64::NAN)
    }

    fn leaf_out_day(m: &SeasonModel, alt: f64) -> f64 {
        first_day(
            m,
            |s| f64::from(s.foliage(LeafType::Deciduous, alt, 0.0).leaves),
            0.5,
        )
    }

    #[test]
    fn the_regions_have_their_climate() {
        assert_eq!(Climate::for_place(47.14, 9.52).region, "alpine-rhine");
        assert_eq!(Climate::for_place(47.9, 8.0).region, "black-forest");
        assert_eq!(Climate::for_place(28.3, -16.6).region, "canaries");
        assert_eq!(Climate::for_place(60.0, 10.7).region, "latitude");
        // Vaduz: about 0.9 °C in January, 19.6 °C in July, 10.3 °C a year.
        let c = ALPINE_RHINE;
        assert!((c.temp_c(17.0, 457.0) - 0.9).abs() < 0.6);
        assert!((c.temp_c(200.0, 457.0) - 19.6).abs() < 0.6);
        let mean: f64 = (1..=365)
            .map(|d| c.temp_c(f64::from(d), 457.0))
            .sum::<f64>()
            / 365.0;
        assert!((mean - 10.3).abs() < 0.1, "{mean}");
    }

    #[test]
    fn maths_helpers() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((normal_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((normal_cdf(-1.0) - 0.158_655).abs() < 1e-5);
        // A day far above the base counts in full, one far below not.
        assert!((expected_above(10.0, 3.0) - 10.0).abs() < 1e-3);
        assert!(expected_above(-10.0, 3.0) < 1e-3);
        // At the base the spread gives σ / √(2π).
        assert!((expected_above(0.0, 3.0) - 3.0 / (std::f64::consts::TAU).sqrt()).abs() < 1e-9);
        assert_eq!(ramp(5.0, (0.0, 10.0)), 0.5);
        assert_eq!(ramp(-1.0, (0.0, 10.0)), 0.0);
    }

    #[test]
    fn spring_comes_later_higher_up() {
        let m = liechtenstein();
        let vaduz = leaf_out_day(&m, VADUZ);
        let malbun = leaf_out_day(&m, MALBUN);
        // Beech in the Rhine valley: leaves out in the second half of
        // April; at 1 600 m about six weeks later (Hopkins: ~3–4 days per
        // 100 m).
        assert!((105.0..=125.0).contains(&vaduz), "Vaduz {vaduz}");
        assert!((145.0..=170.0).contains(&malbun), "Malbun {malbun}");
        let per_100m = (malbun - vaduz) / ((MALBUN - VADUZ) / 100.0);
        assert!((2.5..=4.5).contains(&per_100m), "{per_100m} days per 100 m");
        // Monotone with the altitude.
        let mut last = 0.0;
        for alt in [0.0, 300.0, 600.0, 900.0, 1200.0, 1500.0, 1800.0] {
            let d = leaf_out_day(&m, alt);
            assert!(d > last, "{alt} m: {d}");
            last = d;
        }
        // The grass greens up before the trees.
        let grass = first_day(&m, |s| f64::from(s.grass(VADUZ).green), 0.5);
        assert!(grass < vaduz, "grass {grass}, trees {vaduz}");
    }

    #[test]
    fn spring_comes_later_further_north() {
        let south = SeasonModel::new(Climate::by_latitude(42.0));
        let mid = SeasonModel::new(Climate::by_latitude(50.0));
        let north = SeasonModel::new(Climate::by_latitude(58.0));
        let (s, m, n) = (
            leaf_out_day(&south, 200.0),
            leaf_out_day(&mid, 200.0),
            leaf_out_day(&north, 200.0),
        );
        assert!(s < m && m < n, "{s} {m} {n}");
        // And autumn earlier.
        let colour = |model: &SeasonModel| {
            first_day_from(
                model,
                200,
                |d| f64::from(d.foliage(LeafType::Deciduous, 200.0, 0.0).autumn),
                0.5,
            )
        };
        assert!(colour(&north) < colour(&south));
    }

    #[test]
    fn the_seasons_turn_round_in_the_south() {
        let north = SeasonModel::new(Climate::by_latitude(45.0));
        let south = SeasonModel::new(Climate::by_latitude(-45.0));
        let leaves =
            |m: &SeasonModel, d: f64| m.day(d).foliage(LeafType::Deciduous, 100.0, 0.0).leaves;
        assert!(leaves(&north, 196.0) > 0.9 && leaves(&north, 15.0) < 0.05);
        assert!(leaves(&south, 15.0) > 0.9 && leaves(&south, 196.0) < 0.05);
    }

    #[test]
    fn autumn_comes_earlier_higher_up() {
        let m = liechtenstein();
        let colour = |alt| {
            first_day_from(
                &m,
                200,
                |s| f64::from(s.foliage(LeafType::Deciduous, alt, 0.0).autumn),
                0.5,
            )
        };
        let bare = |alt| {
            first_day(
                &m,
                |s| {
                    let f = s.foliage(LeafType::Deciduous, alt, 0.0);
                    if s.day > 200.0 && f.leaves < 0.05 {
                        1.0
                    } else {
                        0.0
                    }
                },
                0.5,
            )
        };
        let (v, mb) = (colour(VADUZ), colour(MALBUN));
        // Vaduz: colours in the second half of October, bare in November.
        assert!((285.0..=300.0).contains(&v), "Vaduz colours {v}");
        assert!(
            (305.0..=325.0).contains(&bare(VADUZ)),
            "Vaduz bare {}",
            bare(VADUZ)
        );
        assert!(mb < v - 15.0, "Malbun {mb}");
    }

    #[test]
    fn deciduous_leaves_through_the_year() {
        let m = liechtenstein();
        let at = |d: f64| m.day(d).foliage(LeafType::Deciduous, VADUZ, 0.0);
        // 15 January bare, 1 May fresh green, 1 July full green, 15
        // October colouring, 25 November bare.
        let jan = at(15.0);
        assert!(jan.leaves < 0.01, "{jan:?}");
        let may = at(121.0);
        assert!(
            may.leaves > 0.4 && may.fresh > 0.4 && may.autumn == 0.0,
            "{may:?}"
        );
        let jul = at(182.0);
        assert!(
            jul.leaves > 0.99 && jul.fresh < 0.05 && jul.autumn == 0.0,
            "{jul:?}"
        );
        let oct = at(288.0);
        assert!(oct.leaves > 0.8 && oct.autumn > 0.15, "{oct:?}");
        let late_oct = at(300.0);
        assert!(late_oct.autumn > 0.8, "{late_oct:?}");
        let nov = at(329.0);
        assert!(nov.leaves < 0.05, "{nov:?}");
        // Malbun on 1 May: still bare.
        assert!(
            m.day(121.0)
                .foliage(LeafType::Deciduous, MALBUN, 0.0)
                .leaves
                < 0.05
        );
        // Conifers stay green all year.
        for d in [15.0, 121.0, 182.0, 288.0, 329.0] {
            assert_eq!(
                m.day(d).foliage(LeafType::Evergreen, VADUZ, 0.3),
                Foliage::EVERGREEN
            );
        }
        // Larches flush before the beeches and turn later.
        let larch = m.day(108.0).foliage(LeafType::Larch, VADUZ, 0.0);
        assert!(larch.leaves > m.day(108.0).foliage(LeafType::Deciduous, VADUZ, 0.0).leaves);
        let l = m.day(295.0).foliage(LeafType::Larch, VADUZ, 0.0);
        let b = m.day(295.0).foliage(LeafType::Deciduous, VADUZ, 0.0);
        assert!(l.autumn < b.autumn, "larch {l:?} beech {b:?}");
        // The jitter moves single trees a few days.
        let early = m.day(112.0).foliage(LeafType::Deciduous, VADUZ, -1.0);
        let late = m.day(112.0).foliage(LeafType::Deciduous, VADUZ, 1.0);
        assert!(early.leaves > late.leaves);
    }

    #[test]
    fn grass_greens_and_rests() {
        let m = liechtenstein();
        let g = |d: f64, alt: f64| m.day(d).grass(alt);
        assert!(g(15.0, VADUZ).green < 0.05);
        assert!(g(182.0, VADUZ).green > 0.99);
        assert!(g(140.0, VADUZ).green > g(140.0, MALBUN).green);
        assert!(g(355.0, VADUZ).green < 0.3);
        // No summer drought in the Alps, but on the Canary coast.
        assert_eq!(g(200.0, VADUZ).dry, 0.0);
        let tf = SeasonModel::new(CANARIES);
        assert!(tf.day(230.0).grass(50.0).dry > 0.5);
        assert!(tf.day(60.0).grass(50.0).dry < 0.1);
        assert!(tf.day(230.0).grass(1800.0).dry < tf.day(230.0).grass(50.0).dry);
    }

    #[test]
    fn crop_calendar_stages() {
        let m = liechtenstein();
        let cereal = |d: f64| m.day(d).crop(Crop::Cereal, VADUZ);
        let maize = |d: f64| m.day(d).crop(Crop::Maize, VADUZ);
        assert_eq!(cereal(15.0), CropStage::Sown);
        assert_eq!(cereal(152.0), CropStage::Green);
        assert!(matches!(
            cereal(196.0),
            CropStage::Ripening | CropStage::Ripe
        ));
        assert_eq!(cereal(213.0), CropStage::Stubble);
        assert_eq!(cereal(258.0), CropStage::Ploughed);
        assert_eq!(cereal(320.0), CropStage::Sown);
        assert_eq!(maize(105.0), CropStage::Bare);
        assert_eq!(maize(140.0), CropStage::Sprouting);
        assert_eq!(maize(200.0), CropStage::Green);
        assert!(matches!(
            maize(270.0),
            CropStage::Ripening | CropStage::Ripe
        ));
        assert_eq!(maize(300.0), CropStage::Stubble);
        // The stages follow each other in order through the year.
        let order = |s: CropStage| match s {
            CropStage::Sown | CropStage::Bare => 0,
            CropStage::Sprouting => 1,
            CropStage::Green => 2,
            CropStage::Ripening => 3,
            CropStage::Ripe => 4,
            CropStage::Stubble => 5,
            CropStage::Ploughed => 6,
        };
        let mut last = 0;
        for d in 20..=250 {
            let o = order(cereal(f64::from(d)));
            assert!(o >= last, "day {d}");
            last = o;
        }
        // Higher fields ripen later; where the crop never ripens, the
        // autumn frost ends the season.
        assert!(m.day(196.0).state(1000.0).gdd < m.day(196.0).state(VADUZ).gdd);
        assert_eq!(m.day(320.0).crop(Crop::Cereal, 1700.0), CropStage::Stubble);
    }

    #[test]
    fn the_climatological_snow_line() {
        let m = liechtenstein();
        let line = |d: f64| m.day(d).snow_line_m().unwrap_or(f64::INFINITY);
        // Mid-January about 1 000 m, 1 May about 1 600 m, July 2 500 m
        // and higher, mid-October high, December down again.
        let (jan, may, jul, oct, dec) = (
            line(15.0),
            line(121.0),
            line(182.0),
            line(288.0),
            line(340.0),
        );
        assert!((800.0..=1150.0).contains(&jan), "January {jan}");
        assert!((1400.0..=1800.0).contains(&may), "May {may}");
        assert!(jul > 2300.0, "July {jul}");
        assert!(oct > 2200.0, "October {oct}");
        assert!(dec < 1700.0, "December {dec}");
        // Vaduz has no lying snow in the climatology; Malbun is white in
        // January and still partly covered on 1 May.
        assert!(m.day(15.0).snow(VADUZ) < 0.05);
        assert!(m.day(15.0).snow(MALBUN) > 0.95);
        let may_malbun = m.day(121.0).snow(MALBUN);
        assert!(may_malbun > 0.2, "{may_malbun}");
        assert!(m.day(121.0).snow(VADUZ) == 0.0);
        assert!(m.day(196.0).snow(MALBUN) == 0.0);
        // Snow cover grows with the altitude.
        let day = m.day(60.0);
        let mut last = -1.0;
        for alt in (0..4000).step_by(100) {
            let s = day.snow(f64::from(alt));
            assert!(s >= last - 1e-6);
            last = s;
        }
        // The Teide gets winter snow, the Canary coast never.
        let tf = SeasonModel::new(CANARIES);
        assert!(
            tf.day(40.0).snow(3700.0) > 0.5,
            "{}",
            tf.day(40.0).snow(3700.0)
        );
        assert_eq!(tf.day(40.0).snow(100.0), 0.0);
        assert!(tf.day(230.0).snow(3700.0) < 0.05);
    }

    #[test]
    fn ramps_follow_the_model() {
        let m = liechtenstein();
        for d in [15.0, 121.0, 182.0, 288.0, 340.0] {
            let s = m.day(d);
            let r = s.ramps(VADUZ, None);
            // At the snow line the ramp gives about half, well below none,
            // well above full cover.
            if let Some(h) = s.snow_line_m().filter(|h| *h > 0.0 && *h < 5500.0) {
                assert!((r.snow.at(h) - 0.5).abs() < 0.02, "day {d}");
                assert!(r.snow.at(h - 600.0) < 0.05);
                assert!(r.snow.at(h + 600.0) > 0.9);
            }
            // The grass ramps agree with the model within a tenth.
            for alt in [VADUZ, 800.0, 1200.0, MALBUN] {
                let g = s.grass(alt);
                let up = r.grass_up.at(alt);
                let down = r.grass_dormant.at(alt);
                let green = up * (1.0 - down);
                assert!(
                    (green - g.green).abs() < 0.12,
                    "day {d} {alt} m: {green} vs {}",
                    g.green
                );
            }
            // The degree days are linear near the rider.
            let near = s.state(VADUZ + 150.0).gdd;
            assert!((r.gdd.at(VADUZ + 150.0) - near).abs() < 0.05 * near.max(20.0));
        }
        // A process that does not change over the altitudes is constant.
        let flat = Ramp::fit(|_| 0.3, 500.0);
        assert_eq!(flat, Ramp::constant(0.3));
        // Falling processes fall.
        let f = Ramp::fit(
            |h| 1.0 - ((h - 1000.0) / 400.0 + 0.5).clamp(0.0, 1.0),
            500.0,
        );
        assert!(f.b < 0.0 && (f.at(1000.0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn snow_combines_climatology_observation_and_weather() {
        let m = liechtenstein();
        let jan = m.day(15.0);
        let clim = jan.snow_ramp(VADUZ, None);
        assert!(clim.at(VADUZ) < 0.05 && clim.at(MALBUN) > 0.9);
        // A measured 20 cm in Vaduz brings the snow down to the valley.
        let deep = jan.snow_ramp(
            VADUZ,
            Some(SnowObservation {
                alt_m: VADUZ,
                depth_m: 0.2,
            }),
        );
        assert!(deep.at(VADUZ) > 0.9, "{deep:?}");
        assert!(deep.at(MALBUN) >= clim.at(MALBUN));
        // A bare Malbun on 15 January (a snowless winter) moves it up.
        let bare = jan.snow_ramp(
            MALBUN,
            Some(SnowObservation {
                alt_m: MALBUN,
                depth_m: 0.0,
            }),
        );
        assert!(
            bare.at(MALBUN) < 0.01 && bare.at(MALBUN + 600.0) > 0.5,
            "{bare:?}"
        );
        // The weather's fresh snow adds to the climatology; neither takes
        // the other away.
        assert_eq!(lying_snow(0.0, 0.7), 0.7);
        assert_eq!(lying_snow(0.9, 0.2), 0.9);
        assert_eq!(lying_snow(0.0, 0.0), 0.0);
        assert_eq!(lying_snow(1.5, 0.0), 1.0);
    }

    #[test]
    fn leaf_types_by_setting_and_altitude() {
        let mix = CENTRAL_EUROPE;
        let share = |s, alt| {
            let n = 2000;
            (0..n)
                .filter(|i| {
                    mix.leaf_type(s, alt, None, (*i as f32 + 0.5) / n as f32) == LeafType::Deciduous
                })
                .count() as f64
                / n as f64
        };
        assert!((share(TreeSetting::Forest, 400.0) - 0.6).abs() < 0.01);
        assert!(share(TreeSetting::Forest, 1600.0) < 0.1);
        assert_eq!(share(TreeSetting::Orchard, 450.0), 1.0);
        assert!(share(TreeSetting::Open, 1900.0) < 0.3);
        // Larches up high.
        assert_eq!(
            mix.leaf_type(TreeSetting::Forest, 2000.0, None, 0.1),
            LeafType::Larch
        );
        // The measured type wins.
        assert_eq!(
            mix.leaf_type(
                TreeSetting::Forest,
                1900.0,
                Some(MeasuredLeaf::Broadleaved),
                0.99
            ),
            LeafType::Deciduous
        );
        assert_eq!(
            mix.leaf_type(
                TreeSetting::Orchard,
                400.0,
                Some(MeasuredLeaf::Coniferous),
                0.5
            ),
            LeafType::Evergreen
        );
        // Tenerife: evergreen.
        assert_eq!(
            CANARIES.trees.leaf_type(
                TreeSetting::Forest,
                1200.0,
                Some(MeasuredLeaf::Broadleaved),
                0.1
            ),
            LeafType::Evergreen
        );
        assert_eq!(
            CANARIES
                .trees
                .leaf_type(TreeSetting::Forest, 1200.0, None, 0.01),
            LeafType::Evergreen
        );
    }

    /// The far canopy's shares of a measured wood (spec §14 Phase 8 T3)
    /// are the average of the measured trees' leaf types.
    #[test]
    fn measured_shares_average_the_measured_trees() {
        let mix = CENTRAL_EUROPE;
        for (alt, b) in [(450.0, 0.7), (1300.0, 0.2), (1900.0, 0.0), (800.0, 1.0)] {
            let (d, l) = mix.measured_shares(TreeSetting::Forest, alt, b);
            let n = 4000;
            let (mut dd, mut ll) = (0usize, 0usize);
            for i in 0..n {
                let broad = (i as f64 + 0.5) / f64::from(n) < b;
                let m = if broad {
                    MeasuredLeaf::Broadleaved
                } else {
                    MeasuredLeaf::Coniferous
                };
                // The pick is independent of the broadleaf choice.
                let pick = ((i * 7919) % n) as f32 / n as f32;
                match mix.leaf_type(TreeSetting::Forest, alt, Some(m), pick) {
                    LeafType::Deciduous => dd += 1,
                    LeafType::Larch => ll += 1,
                    LeafType::Evergreen => {}
                }
            }
            assert!(
                (d - dd as f64 / f64::from(n)).abs() < 0.02,
                "{alt} {b}: {d} vs {dd}"
            );
            assert!(
                (l - ll as f64 / f64::from(n)).abs() < 0.02,
                "{alt} {b}: {l} vs {ll}"
            );
        }
        // Measured broadleaves on Tenerife stay evergreen.
        assert_eq!(
            CANARIES
                .trees
                .measured_shares(TreeSetting::Forest, 900.0, 0.8)
                .0,
            0.0
        );
    }

    #[test]
    fn deterministic() {
        let a = liechtenstein();
        let b = liechtenstein();
        assert_eq!(a, b);
        assert_eq!(a.day(121.5), b.day(121.5));
        assert_eq!(
            a.day(121.5).ramps(VADUZ, None),
            b.day(121.5).ramps(VADUZ, None)
        );
        assert_eq!(a.day(121.5).describe(MALBUN), b.day(121.5).describe(MALBUN));
        // The year wraps: day 0 is the last day of the year before.
        let x = a.day(0.5).state(VADUZ);
        let y = a.day(365.75).state(VADUZ);
        assert!(
            (x.gdd - y.gdd).abs() < 1.0 && (x.snow - y.snow).abs() < 0.01,
            "{x:?} {y:?}"
        );
    }

    /// Spec §14 Phase 9 Q13 (*Worlds with weak or no seasons*): the season
    /// strength of the three worlds and of Tenerife's altitude bands, by
    /// the documented thresholds.
    #[test]
    fn season_strength_of_the_worlds_and_tenerifes_heights() {
        let li = SeasonModel::new(ALPINE_RHINE);
        let bf = SeasonModel::new(BLACK_FOREST);
        let tf = SeasonModel::new(CANARIES);
        // Liechtenstein and the Black Forest: the leaves turn from the
        // valleys to the passes, measured broadleaf share or not.
        for (m, alts) in [
            (&li, &[455.0, 1000.0, 1600.0, 2000.0][..]),
            (&bf, &[250.0, 800.0, 1200.0, 1490.0][..]),
        ] {
            for &alt in alts {
                for measured in [None, Some(0.0), Some(0.3), Some(1.0)] {
                    let s = SeasonStrength::at(m, alt, measured);
                    assert_eq!(s.level(), SeasonLevel::Marked, "{alt} {measured:?} {s:?}");
                    assert!(s.leaves_turn() && s.marked_temperature());
                    assert!(s.deciduous >= LEAVES_MIN && s.range_c > 17.0, "{s:?}");
                    assert_eq!(s.note(), None);
                }
            }
        }
        // Snow seasons: none in Vaduz, months in Malbun.
        assert_eq!(SeasonStrength::at(&li, VADUZ, None).snow_days, 0);
        assert!(SeasonStrength::at(&li, MALBUN, None).snow_days > 120);
        // Tenerife: evergreen from the coast to the summit (its measured
        // broadleaves are laurel forest), no lying snow below the summit
        // cone.
        for alt in [
            0.0, 50.0, 500.0, 1000.0, 1500.0, 2000.0, 2370.0, 3000.0, 3700.0,
        ] {
            for measured in [None, Some(0.0), Some(0.7), Some(1.0)] {
                let s = SeasonStrength::at(&tf, alt, measured);
                assert!(!s.leaves_turn(), "{alt} {measured:?} {s:?}");
                assert!(s.deciduous < LEAVES_MIN, "{s:?}");
                assert_ne!(s.level(), SeasonLevel::Marked);
                assert!(s.note().is_some());
            }
        }
        // Puerto de la Cruz at sea level: weak, 6.6 °C, no snow.
        let coast = SeasonStrength::at(&tf, 20.0, None);
        assert_eq!(coast.level(), SeasonLevel::Weak);
        assert!(
            !coast.marked_temperature() && coast.range_c < 7.0,
            "{coast:?}"
        );
        assert_eq!(coast.snow_days, 0);
        assert_eq!(
            coast.note().unwrap(),
            "Weak seasons here: evergreen trees, only 7 °C between the coolest and the warmest days, no lying snow."
        );
        // Las Cañadas (2 000 – 2 400 m, the highest roads): a larger swing
        // but no lying snow in the climatology.
        let canadas = SeasonStrength::at(&tf, 2200.0, None);
        assert_eq!(canadas.level(), SeasonLevel::Weak, "{canadas:?}");
        assert!(canadas.marked_temperature(), "{canadas:?}");
        // The Teide's summit cone: a snow season.
        let teide = SeasonStrength::at(&tf, 3700.0, None);
        assert_eq!(teide.level(), SeasonLevel::SnowOnly, "{teide:?}");
        assert!(teide.snow_days >= SNOW_SEASON_DAYS && teide.snow_days < 200);
        assert!(
            teide
                .note()
                .unwrap()
                .starts_with("Weak seasons here but for the snow")
        );
        // Deterministic.
        assert_eq!(SeasonStrength::at(&tf, 3700.0, None), teide);
    }

    /// Andorra (spec 4.7 *Fifth world: Andorra*): marked seasons from the
    /// valley floor to the passes – the pine forests have few broadleaves,
    /// but the birches, ashes and fruit trees of the meadows turn – a
    /// short snow season at Andorra la Vella and a long one at Envalira.
    #[test]
    fn andorra_has_marked_seasons_and_long_snow_on_the_passes() {
        assert_eq!(Climate::for_place(42.507, 1.522).region, "pyrenees");
        assert_eq!(Climate::for_place(42.36, 1.46).region, "pyrenees");
        let m = SeasonModel::new(PYRENEES);
        // The climatology of Andorra la Vella (1 075 m): 9.8 °C a year,
        // about 2 °C in January and 18 – 19 °C in July.
        let c = m.climate();
        let year: f64 = (1..=365)
            .map(|d| c.temp_c(f64::from(d), 1075.0))
            .sum::<f64>()
            / 365.0;
        assert!((year - 9.8).abs() < 0.2, "{year}");
        assert!(
            (c.temp_c(15.0, 1075.0) - 2.2).abs() < 1.0,
            "{}",
            c.temp_c(15.0, 1075.0)
        );
        assert!(
            (c.temp_c(201.0, 1075.0) - 18.8).abs() < 1.0,
            "{}",
            c.temp_c(201.0, 1075.0)
        );
        for alt in [840.0, 1075.0, 1600.0, 2000.0, 2408.0] {
            let s = SeasonStrength::at(&m, alt, None);
            assert_eq!(s.level(), SeasonLevel::Marked, "{alt} {s:?}");
            assert!(s.marked_temperature(), "{alt} {s:?}");
            assert_eq!(s.note(), None);
        }
        let valley = SeasonStrength::at(&m, 1075.0, None).snow_days;
        let pass = SeasonStrength::at(&m, 2408.0, None).snow_days;
        assert!(valley < 60, "Andorra la Vella: {valley} days");
        assert!((130..220).contains(&pass), "Port d'Envalira: {pass} days");
    }

    /// Q13: where the broadleaves are evergreen (Tenerife) the leaf fall is
    /// that of the few deciduous lone trees – near zero, whatever the
    /// degree days do; unchanged in the Alps.
    #[test]
    fn evergreen_places_drop_almost_no_leaves() {
        let tf = SeasonModel::new(CANARIES);
        let li = SeasonModel::new(ALPINE_RHINE);
        for d in (1..=365).step_by(4) {
            let day = tf.day(f64::from(d));
            assert!((day.leaf_fall_share() - 0.1).abs() < 1e-6);
            for alt in [20.0, 600.0, 1200.0, 2000.0] {
                assert!(day.leaf_fall(alt) <= 0.1 + 1e-6, "{d} {alt}");
            }
            assert_eq!(li.day(f64::from(d)).leaf_fall_share(), 1.0);
        }
        assert!(li.day(320.0).leaf_fall(VADUZ) > 0.9);
    }
}
