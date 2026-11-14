//! Real weather (spec §14 Phase 9, S7; 9F, decisions Q6 and Q8): the
//! providers, their requests and responses, the mapping of their fields
//! onto a [`WeatherState`], and the server's feed ([`feed::RealFeed`]) that
//! picks the providers for the place of the session, caches per region and
//! falls back across them.
//!
//! Everything here is pure (no sockets, files or clock reads; instants are
//! passed in): the game server does the HTTP requests on its own thread
//! (native only, `ocw_server::realweather`) and hands the bodies back, and
//! the tests feed recorded responses (`tests/data/`). Clients never ask a
//! provider (privacy, 9A): the server asks for the centre of a 0.1° grid
//! cell and a 200 m altitude band, never for a rider's exact position.
//!
//! | Provider | Area | Data | Licence, attribution |
//! |---|---|---|---|
//! | MeteoSwiss open data | Switzerland, Liechtenstein | SwissMetNet stations, 10-minute measurements | CC BY 4.0, "Source: MeteoSwiss" |
//! | DWD open data | Germany | station observations (hourly, "POI" reports) | CC BY 4.0 (DWD legal notice: open spatial data, source named), "Source: Deutscher Wetterdienst" |
//! | MET Norway Locationforecast 2.0 | global | hourly forecast | CC BY 4.0 and NLOD 2.0, "Data from MET Norway" |
//! | Open-Meteo | global | forecast; history from ERA5 | CC BY 4.0, "Weather data by Open-Meteo.com"; ERA5: "Contains modified Copernicus Climate Change Service information" |

pub mod cache;
pub mod days;
pub mod dwd;
pub mod feed;
pub mod meteoswiss;
pub mod metno;
pub mod openmeteo;
pub mod stations;
pub mod time;

use crate::particles::extinction_per_km;
use crate::state::{CLEAR_VISIBILITY_M, CloudType, MIN_VISIBILITY_M, Precip, WeatherState};

/// A source of real weather (spec 9F, Q6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    MeteoSwiss,
    Dwd,
    MetNorway,
    OpenMeteo,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::MeteoSwiss,
        Provider::Dwd,
        Provider::MetNorway,
        Provider::OpenMeteo,
    ];

    /// The console's word.
    pub fn id(self) -> &'static str {
        match self {
            Self::MeteoSwiss => "meteoswiss",
            Self::Dwd => "dwd",
            Self::MetNorway => "metno",
            Self::OpenMeteo => "open-meteo",
        }
    }

    /// The menu's and the attribution's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::MeteoSwiss => "MeteoSwiss",
            Self::Dwd => "DWD",
            Self::MetNorway => "MET Norway",
            Self::OpenMeteo => "Open-Meteo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| {
            p.id() == s
                || p.name().to_ascii_lowercase() == s
                || matches!(
                    (p, s.as_str()),
                    (Self::MetNorway, "met" | "met-norway" | "yr")
                        | (Self::OpenMeteo, "openmeteo")
                        | (Self::Dwd, "deutscher-wetterdienst")
                )
        })
    }

    /// The data's licence (all four publish under CC BY 4.0; MET Norway
    /// also under NLOD 2.0, DWD's open data is CC BY 4.0 with the source
    /// named, checked 2026-11-14).
    pub fn licence(self) -> &'static str {
        "CC BY 4.0"
    }

    /// The credit the terms ask for (credits screen, spec §13).
    pub fn credit(self) -> &'static str {
        match self {
            Self::MeteoSwiss => "Source: MeteoSwiss",
            Self::Dwd => "Source: Deutscher Wetterdienst",
            Self::MetNorway => "Data from MET Norway",
            Self::OpenMeteo => "Weather data by Open-Meteo.com",
        }
    }

    /// Regional providers measure at stations in their country; the global
    /// ones forecast everywhere.
    pub fn is_regional(self) -> bool {
        matches!(self, Self::MeteoSwiss | Self::Dwd)
    }

    /// The host the requests go to (rate limits are per host).
    pub fn host(self) -> &'static str {
        match self {
            Self::MeteoSwiss => "data.geo.admin.ch",
            Self::Dwd => "opendata.dwd.de",
            Self::MetNorway => "api.met.no",
            Self::OpenMeteo => "api.open-meteo.com",
        }
    }
}

/// The weather machine's choice (World → Weather → Real weather → Data
/// source; Q6): automatic (regional first, then MET Norway, then
/// Open-Meteo) or one provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Choice {
    #[default]
    Automatic,
    Only(Provider),
}

impl Choice {
    /// Automatic and the four providers, in the menu's order.
    pub const ALL: [Choice; 5] = [
        Choice::Automatic,
        Choice::Only(Provider::MeteoSwiss),
        Choice::Only(Provider::Dwd),
        Choice::Only(Provider::MetNorway),
        Choice::Only(Provider::OpenMeteo),
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Only(p) => p.id(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Only(p) => p.name(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "automatic" | "auto" => Some(Self::Automatic),
            other => Provider::parse(other).map(Self::Only),
        }
    }

    /// The next (`dir` > 0) or previous choice, wrapping.
    pub fn step(self, dir: i32) -> Self {
        let n = Self::ALL.len() as i32;
        let i = Self::ALL.iter().position(|c| *c == self).unwrap_or(0) as i32;
        Self::ALL[(i + dir.signum()).rem_euclid(n) as usize]
    }
}

/// "Weather: MeteoSwiss, MET Norway – CC BY 4.0" (the menu and the HUD).
pub fn attribution(providers: &[Provider], historic: bool) -> String {
    if providers.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = providers.iter().map(|p| p.name()).collect();
    let mut licences: Vec<&str> = Vec::new();
    for p in providers {
        if !licences.contains(&p.licence()) {
            licences.push(p.licence());
        }
    }
    let era5 = if historic { " (ERA5, Copernicus)" } else { "" };
    format!(
        "Weather: {}{era5}, {}",
        names.join(", "),
        licences.join(", ")
    )
}

/// Where the session's weather is wanted: a position on the globe and its
/// height above the sea.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f32,
}

/// Grid cells are this many degrees wide (≈ 11 × 7 km in the Alps): the
/// server asks for the cell's centre, so riders nearby share a request and
/// no rider's exact position leaves the server.
pub const CELL_DEG: f64 = 0.1;
/// Altitude bands (m) of the requests.
pub const BAND_M: f32 = 200.0;

/// The request point of a place: its cell's centre and altitude band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cell {
    pub lat_i: i32,
    pub lon_i: i32,
    pub band: i32,
}

impl Cell {
    pub fn of(p: &Place) -> Self {
        Self {
            lat_i: (p.lat / CELL_DEG).floor() as i32,
            lon_i: (p.lon / CELL_DEG).floor() as i32,
            band: (p.alt_m.max(0.0) / BAND_M).round() as i32,
        }
    }

    /// The cell's centre (lat, lon), rounded to two decimals (MET Norway
    /// asks for at most four).
    pub fn centre(&self) -> (f64, f64) {
        let r = |v: f64| (v * 100.0).round() / 100.0;
        (
            r((f64::from(self.lat_i) + 0.5) * CELL_DEG),
            r((f64::from(self.lon_i) + 0.5) * CELL_DEG),
        )
    }

    /// The band's height (m).
    pub fn alt_m(&self) -> f32 {
        self.band as f32 * BAND_M
    }
}

/// Great-circle distance (km).
pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1) = (a.0.to_radians(), a.1.to_radians());
    let (la2, lo2) = (b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

/// A station of a regional network chosen for a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationPick {
    pub id: &'static str,
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub height_m: f32,
    pub distance_km: f64,
}

/// A station counts for a place within this distance (km).
pub const STATION_RANGE_KM: f64 = 25.0;

/// The best station of `table` for `at` (lat, lon) at `alt_m`: near and
/// at a similar height (100 m of height count like 1 km), within
/// [`STATION_RANGE_KM`].
pub fn nearest_station(
    table: &[stations::Station],
    at: (f64, f64),
    alt_m: f32,
) -> Option<StationPick> {
    table
        .iter()
        .map(|s| {
            let d = distance_km(at, (s.2, s.3));
            (s, d, d + f64::from((s.4 - alt_m).abs()) / 100.0)
        })
        .filter(|(_, d, _)| *d <= STATION_RANGE_KM)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(s, d, _)| StationPick {
            id: s.0,
            name: s.1,
            lat: s.2,
            lon: s.3,
            height_m: s.4,
            distance_km: d,
        })
}

/// The regional provider measuring at `at`, if any: the network whose
/// nearest station is closer (at a border both may be in range).
pub fn regional_for(at: (f64, f64), alt_m: f32) -> Option<(Provider, StationPick)> {
    let ch = nearest_station(stations::SWISS, at, alt_m).map(|s| (Provider::MeteoSwiss, s));
    let de = nearest_station(stations::GERMAN, at, alt_m).map(|s| (Provider::Dwd, s));
    match (ch, de) {
        (Some(a), Some(b)) => Some(if a.1.distance_km <= b.1.distance_km {
            a
        } else {
            b
        }),
        (a, b) => a.or(b),
    }
}

/// What a weather code or symbol says (WMO codes, MET Norway symbols, DWD
/// conditions).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sky {
    /// Cloud cover the code implies (0–1).
    pub cover: Option<f32>,
    pub precip: Option<Precip>,
    /// 0 light, 1 moderate, 2 heavy.
    pub strength: u8,
    pub showers: bool,
    pub thunder: bool,
    pub fog: bool,
}

impl Sky {
    /// A typical intensity (mm/h) for the code when no amount is given.
    pub fn typical_mm_h(&self) -> f32 {
        let k = [0.5, 1.0, 2.5][usize::from(self.strength.min(2))];
        match self.precip {
            None | Some(Precip::None) => 0.0,
            Some(Precip::Drizzle) => 0.3 * k,
            Some(Precip::Snow) => 1.0 * k,
            Some(Precip::Sleet) => 1.5 * k,
            Some(Precip::Hail) => 20.0,
            Some(Precip::Rain) => 3.0 * k * if self.thunder { 3.0 } else { 1.0 },
        }
    }
}

/// One moment of a provider's data, in its own units converted to SI
/// (absent: the provider has no such field).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Obs {
    /// The instant (Unix ms, UTC).
    pub at_ms: i64,
    pub temperature_c: Option<f32>,
    pub dew_point_c: Option<f32>,
    /// Relative humidity (0–1).
    pub humidity: Option<f32>,
    /// Air pressure at the data's elevation (hPa).
    pub pressure_hpa: Option<f32>,
    /// Air pressure reduced to the sea (hPa).
    pub pressure_msl_hpa: Option<f32>,
    /// Cloud cover (0–1): total and low / middle / high.
    pub cloud_cover: Option<f32>,
    pub cloud_low: Option<f32>,
    pub cloud_mid: Option<f32>,
    pub cloud_high: Option<f32>,
    /// Cloud base above the ground (m).
    pub cloud_base_m: Option<f32>,
    /// Precipitation (mm/h water equivalent).
    pub precip_mm_h: Option<f32>,
    /// The frozen share of it (snow; 0–1).
    pub snow_share: Option<f32>,
    /// Visibility including what falls (m).
    pub visibility_m: Option<f32>,
    /// Fog area fraction (0–1, MET Norway).
    pub fog: Option<f32>,
    pub wind_mps: Option<f32>,
    pub gust_mps: Option<f32>,
    /// Where the wind comes from (°).
    pub wind_from_deg: Option<f32>,
    pub sky: Option<Sky>,
}

/// A provider's answer: its data over time at one elevation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    /// The height (m) the values hold for (a station's, the model's grid
    /// point's or the requested altitude).
    pub elevation_m: Option<f32>,
    /// A station's name ("Vaduz (VAD)").
    pub station: Option<String>,
    /// Measured (stations) rather than modelled.
    pub measured: bool,
    /// Sorted by time.
    pub obs: Vec<Obs>,
}

/// How far a forecast hour may be used beyond the series' ends (ms).
pub const EDGE_MS: i64 = 90 * 60_000;
/// A measurement stands for this long after it was taken (ms): DWD's
/// hourly reports appear one to two hours late.
pub const MEASURED_MS: i64 = 150 * 60_000;

fn mix_opt(a: Option<f32>, b: Option<f32>, t: f32) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + (b - a) * t),
        (a, b) => {
            if t < 0.5 {
                a.or(b)
            } else {
                b.or(a)
            }
        }
    }
}

fn mix_angle_opt(a: Option<f32>, b: Option<f32>, t: f32) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(crate::state::mix_angle(a, b, t)),
        (a, b) => a.or(b),
    }
}

/// `a` and `b` blended at `t` (numbers linearly, the wind direction the
/// short way, codes from the nearer).
pub fn blend(a: &Obs, b: &Obs, t: f32) -> Obs {
    let m = |x, y| mix_opt(x, y, t);
    Obs {
        at_ms: a.at_ms + ((b.at_ms - a.at_ms) as f64 * f64::from(t)) as i64,
        temperature_c: m(a.temperature_c, b.temperature_c),
        dew_point_c: m(a.dew_point_c, b.dew_point_c),
        humidity: m(a.humidity, b.humidity),
        pressure_hpa: m(a.pressure_hpa, b.pressure_hpa),
        pressure_msl_hpa: m(a.pressure_msl_hpa, b.pressure_msl_hpa),
        cloud_cover: m(a.cloud_cover, b.cloud_cover),
        cloud_low: m(a.cloud_low, b.cloud_low),
        cloud_mid: m(a.cloud_mid, b.cloud_mid),
        cloud_high: m(a.cloud_high, b.cloud_high),
        cloud_base_m: m(a.cloud_base_m, b.cloud_base_m),
        precip_mm_h: m(a.precip_mm_h, b.precip_mm_h),
        snow_share: m(a.snow_share, b.snow_share),
        visibility_m: match (a.visibility_m, b.visibility_m) {
            // In log space, like the state's fog.
            (Some(x), Some(y)) => Some((x.max(1.0).ln() * (1.0 - t) + y.max(1.0).ln() * t).exp()),
            (x, y) => x.or(y),
        },
        fog: m(a.fog, b.fog),
        wind_mps: m(a.wind_mps, b.wind_mps),
        gust_mps: m(a.gust_mps, b.gust_mps),
        wind_from_deg: mix_angle_opt(a.wind_from_deg, b.wind_from_deg, t),
        sky: if t < 0.5 {
            a.sky.or(b.sky)
        } else {
            b.sky.or(a.sky)
        },
    }
}

impl Series {
    /// The first and last instant.
    pub fn span(&self) -> Option<(i64, i64)> {
        Some((self.obs.first()?.at_ms, self.obs.last()?.at_ms))
    }

    /// The data at `at_ms`: forecasts interpolated between their hours
    /// (usable up to [`EDGE_MS`] beyond the ends), measurements the last
    /// one taken at most [`MEASURED_MS`] before (or a few minutes after).
    pub fn at(&self, at_ms: i64) -> Option<Obs> {
        if self.measured {
            let o = self
                .obs
                .iter()
                .rev()
                .find(|o| o.at_ms <= at_ms + 10 * 60_000)?;
            return (at_ms - o.at_ms <= MEASURED_MS).then_some(*o);
        }
        let (first, last) = self.span()?;
        if at_ms < first - EDGE_MS || at_ms > last + EDGE_MS {
            return None;
        }
        let i = self.obs.partition_point(|o| o.at_ms <= at_ms);
        Some(match i {
            0 => self.obs[0],
            i if i == self.obs.len() => self.obs[i - 1],
            i => {
                let (a, b) = (&self.obs[i - 1], &self.obs[i]);
                let t = (at_ms - a.at_ms) as f32 / (b.at_ms - a.at_ms).max(1) as f32;
                blend(a, b, t)
            }
        })
    }

    /// The data at `at_ms`, or at the nearer end of the series (with
    /// whether it was clamped): beyond a forecast its last hour.
    pub fn at_or_edge(&self, at_ms: i64) -> Option<(Obs, bool)> {
        if let Some(o) = self.at(at_ms) {
            return Some((o, false));
        }
        if self.measured {
            return None;
        }
        let (first, _) = self.span()?;
        let o = if at_ms < first {
            self.obs.first()?
        } else {
            self.obs.last()?
        };
        Some((*o, true))
    }
}

/// Barometric factor of the pressure at height `h` (m) relative to the sea
/// (standard atmosphere).
fn baro(h: f32) -> f32 {
    (1.0 - 2.255_77e-5 * h).max(0.05).powf(5.255_88)
}

/// The standard atmosphere's pressure at height `h` (m; hPa): for data
/// without any pressure.
pub fn isa_pressure_hpa(h: f32) -> f32 {
    1013.25 * baro(h)
}

/// Dew point (°C) from temperature and relative humidity (Magnus).
pub fn dew_point_c(t: f32, rh: f32) -> f32 {
    let (a, b) = (17.62, 243.12);
    let g = (rh.clamp(0.01, 1.0)).ln() + a * t / (b + t);
    b * g / (a - g)
}

/// Relative humidity (0–1) from temperature and dew point (Magnus).
pub fn humidity_of(t: f32, td: f32) -> f32 {
    let (a, b) = (17.62, 243.12);
    ((a * td / (b + td)) - (a * t / (b + t)))
        .exp()
        .clamp(0.0, 1.0)
}

/// Temperature lapse rate (K/m) of the standard atmosphere.
pub const LAPSE_K_PER_M: f32 = 0.0065;

/// `o` (valid at `elevation`) moved to the height `alt_m`: temperature by
/// the lapse rate, the dew point by 1.8 K/km, the pressure by the
/// barometric formula (the sea-level pressure becomes the pressure there).
pub fn normalize(o: &Obs, elevation: Option<f32>, alt_m: f32) -> Obs {
    let dz = elevation.map_or(0.0, |e| alt_m - e);
    let mut n = *o;
    let td = o
        .dew_point_c
        .or_else(|| Some(dew_point_c(o.temperature_c?, o.humidity?)));
    n.temperature_c = o.temperature_c.map(|t| t - LAPSE_K_PER_M * dz);
    n.dew_point_c = td.map(|d| d - 0.0018 * dz);
    if let (Some(t), Some(d)) = (n.temperature_c, n.dew_point_c) {
        n.dew_point_c = Some(d.min(t));
        n.humidity = Some(humidity_of(t, d.min(t)));
    }
    n.pressure_hpa = match (o.pressure_hpa, o.pressure_msl_hpa, elevation) {
        (Some(p), _, Some(e)) => Some(p * baro(alt_m) / baro(e)),
        (Some(p), _, None) => Some(p),
        (None, Some(msl), _) => Some(msl * baro(alt_m)),
        (None, None, _) => None,
    };
    n.pressure_msl_hpa = None;
    n
}

/// `a` with the fields it lacks taken from `b` (a station's measurements
/// over a model's forecast).
pub fn merge(a: &Obs, b: &Obs) -> Obs {
    Obs {
        at_ms: a.at_ms,
        temperature_c: a.temperature_c.or(b.temperature_c),
        dew_point_c: a.dew_point_c.or(b.dew_point_c),
        humidity: a.humidity.or(b.humidity),
        pressure_hpa: a.pressure_hpa.or(b.pressure_hpa),
        pressure_msl_hpa: a.pressure_msl_hpa.or(b.pressure_msl_hpa),
        cloud_cover: a.cloud_cover.or(b.cloud_cover),
        cloud_low: a.cloud_low.or(b.cloud_low),
        cloud_mid: a.cloud_mid.or(b.cloud_mid),
        cloud_high: a.cloud_high.or(b.cloud_high),
        cloud_base_m: a.cloud_base_m.or(b.cloud_base_m),
        precip_mm_h: a.precip_mm_h.or(b.precip_mm_h),
        snow_share: a.snow_share.or(b.snow_share),
        visibility_m: a.visibility_m.or(b.visibility_m),
        fog: a.fog.or(b.fog),
        wind_mps: a.wind_mps.or(b.wind_mps),
        gust_mps: a.gust_mps.or(b.gust_mps),
        wind_from_deg: a.wind_from_deg.or(b.wind_from_deg),
        // A measured thunderstorm or snow shower beats the model's code;
        // the station's precipitation decides whether anything falls.
        sky: match (a.sky, b.sky) {
            (Some(s), _) => Some(s),
            (None, Some(mut s)) if a.precip_mm_h == Some(0.0) => {
                s.precip = None;
                s.thunder = false;
                Some(s)
            }
            (None, s) => s,
        },
    }
}

/// Precipitation below this (mm/h) is none.
pub const MIN_PRECIP_MM_H: f32 = 0.05;

/// The weather state of `o` (already at the rider's height, [`normalize`]):
/// cloud cover and type from the layers and codes, precipitation type from
/// the frozen share, the code or the temperature, the fog's visibility
/// without the precipitation's own extinction, wind with gusts,
/// temperature, humidity, pressure and lightning from thunder.
pub fn to_state(o: &Obs) -> WeatherState {
    let d = WeatherState::default();
    let sky = o.sky.unwrap_or_default();
    let t = o.temperature_c.unwrap_or(d.temperature_c);
    let humidity = o.humidity.unwrap_or(d.humidity);
    let td = o.dew_point_c.unwrap_or_else(|| dew_point_c(t, humidity));
    // What falls: the amount decides (a rain code with 0 mm is dry); only
    // without an amount the code's typical intensity.
    let coded = sky.precip.filter(|p| *p != Precip::None);
    let mut mm = o
        .precip_mm_h
        .map_or_else(|| sky.typical_mm_h(), |v| v.max(0.0));
    if mm < MIN_PRECIP_MM_H {
        mm = 0.0;
    }
    // Liquid: drizzle by the code, or a thin, steady amount without one.
    let liquid = if coded == Some(Precip::Drizzle) || (coded.is_none() && mm < 0.3 && !sky.showers)
    {
        Precip::Drizzle
    } else {
        Precip::Rain
    };
    // A model's frozen share decides; else the code; else the temperature.
    let precip = match (o.snow_share, coded) {
        _ if mm < MIN_PRECIP_MM_H => Precip::None,
        (_, Some(Precip::Hail)) => Precip::Hail,
        (Some(s), _) if s > 0.8 => Precip::Snow,
        (Some(s), _) if s > 0.2 => Precip::Sleet,
        (Some(_), _) => liquid,
        (None, Some(p @ (Precip::Snow | Precip::Sleet))) => p,
        (None, _) if t < 0.5 => Precip::Snow,
        (None, _) if t < 2.0 => Precip::Sleet,
        (None, _) => liquid,
    };
    if precip == Precip::None {
        mm = 0.0;
    }
    // Clouds.
    let mut cover = o.cloud_cover.or(sky.cover).unwrap_or(0.0).clamp(0.0, 1.0);
    if precip != Precip::None {
        cover = cover.max(if sky.showers { 0.7 } else { 0.85 });
    }
    let (low, mid, high) = (
        o.cloud_low.unwrap_or(0.0),
        o.cloud_mid.unwrap_or(0.0),
        o.cloud_high.unwrap_or(0.0),
    );
    let layered = o.cloud_low.is_some() || o.cloud_mid.is_some() || o.cloud_high.is_some();
    let thunder = sky.thunder;
    let cloud_type = if thunder {
        CloudType::Cumulonimbus
    } else if sky.showers {
        CloudType::Cumulus
    } else if layered && high >= 0.3 && low + mid < 0.2 {
        CloudType::Cirrus
    } else if (layered && (low >= 0.6 || mid >= 0.7) && cover >= 0.7)
        || (!layered && cover >= 0.85)
        || (precip != Precip::None && !sky.showers)
    {
        CloudType::Stratus
    } else {
        CloudType::Cumulus
    };
    // The base from the dew-point spread (125 m per K), else the code.
    let spread = (t - td).max(0.0);
    let mut base = o.cloud_base_m.unwrap_or(125.0 * spread).clamp(60.0, 4000.0);
    if cloud_type == CloudType::Cirrus {
        base = base.max(6000.0);
    }
    // Visibility: the provider's includes what falls; the state's fog
    // does not.
    let fog_m = if let Some(v) = o.visibility_m {
        let sigma = 3.912 / (v.max(MIN_VISIBILITY_M) / 1000.0) - extinction_per_km(precip, mm);
        if sigma <= 3.912 / (CLEAR_VISIBILITY_M / 1000.0) {
            CLEAR_VISIBILITY_M
        } else {
            3.912 / sigma * 1000.0
        }
    } else if let Some(f) = o.fog.filter(|f| *f > 0.05) {
        let k = crate::state::smooth(0.05, 0.9, f);
        (CLEAR_VISIBILITY_M.ln() * (1.0 - k) + 150.0f32.ln() * k).exp()
    } else if sky.fog {
        150.0
    } else {
        CLEAR_VISIBILITY_M
    };
    // Wind.
    let wind = o.wind_mps.unwrap_or(d.wind_mps).max(0.0);
    let gust = o.gust_mps.unwrap_or(wind * 1.6 + 1.0).max(wind);
    let lightning = if thunder {
        if sky.strength >= 2 || coded == Some(Precip::Hail) {
            5.0
        } else {
            2.0
        }
    } else {
        0.0
    };
    let pressure = o.pressure_hpa.unwrap_or(d.pressure_hpa);
    WeatherState {
        cloud_cover: cover,
        cloud_type,
        cloud_base_m: base,
        precip,
        precip_mm_h: mm,
        fog_visibility_m: fog_m,
        wind_mps: wind,
        gust_mps: gust,
        wind_from_deg: o.wind_from_deg.unwrap_or(d.wind_from_deg),
        temperature_c: t,
        humidity: humidity_of(t, td.min(t)),
        pressure_hpa: pressure,
        lightning_per_min: lightning,
    }
    .clamped()
}

/// WMO weather codes (Open-Meteo's `weather_code`).
pub fn wmo_sky(code: i64) -> Sky {
    let p = |precip, strength, showers| Sky {
        precip: Some(precip),
        strength,
        showers,
        ..Default::default()
    };
    match code {
        0 => Sky {
            cover: Some(0.0),
            ..Default::default()
        },
        1 => Sky {
            cover: Some(0.2),
            ..Default::default()
        },
        2 => Sky {
            cover: Some(0.5),
            ..Default::default()
        },
        3 => Sky {
            cover: Some(1.0),
            ..Default::default()
        },
        45 | 48 => Sky {
            fog: true,
            cover: Some(1.0),
            ..Default::default()
        },
        51 | 56 => p(Precip::Drizzle, 0, false),
        53 => p(Precip::Drizzle, 1, false),
        55 | 57 => p(Precip::Drizzle, 2, false),
        61 | 66 => p(Precip::Rain, 0, false),
        63 => p(Precip::Rain, 1, false),
        65 | 67 => p(Precip::Rain, 2, false),
        71 | 77 => p(Precip::Snow, 0, false),
        73 => p(Precip::Snow, 1, false),
        75 => p(Precip::Snow, 2, false),
        80 => p(Precip::Rain, 0, true),
        81 => p(Precip::Rain, 1, true),
        82 => p(Precip::Rain, 2, true),
        85 => p(Precip::Snow, 0, true),
        86 => p(Precip::Snow, 2, true),
        95 => Sky {
            thunder: true,
            ..p(Precip::Rain, 1, true)
        },
        96 | 99 => Sky {
            thunder: true,
            ..p(Precip::Hail, 2, true)
        },
        _ => Sky::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_and_choices_have_names_ids_and_credits() {
        for p in Provider::ALL {
            assert_eq!(Provider::parse(p.id()), Some(p));
            assert_eq!(Provider::parse(p.name()), Some(p));
            assert!(!p.credit().is_empty() && !p.licence().is_empty());
        }
        assert_eq!(Provider::parse("met"), Some(Provider::MetNorway));
        assert_eq!(Choice::parse("auto"), Some(Choice::Automatic));
        assert_eq!(
            Choice::parse("MeteoSwiss"),
            Some(Choice::Only(Provider::MeteoSwiss))
        );
        assert_eq!(Choice::parse("nope"), None);
        assert_eq!(
            Choice::Automatic.step(-1),
            Choice::Only(Provider::OpenMeteo)
        );
        assert_eq!(Choice::Only(Provider::OpenMeteo).step(1), Choice::Automatic);
        assert_eq!(
            attribution(&[Provider::MetNorway], false),
            "Weather: MET Norway, CC BY 4.0"
        );
        assert_eq!(
            attribution(&[Provider::MeteoSwiss, Provider::OpenMeteo], false),
            "Weather: MeteoSwiss, Open-Meteo, CC BY 4.0"
        );
        assert_eq!(
            attribution(&[Provider::OpenMeteo], true),
            "Weather: Open-Meteo (ERA5, Copernicus), CC BY 4.0"
        );
        assert_eq!(attribution(&[], false), "");
    }

    #[test]
    fn cells_hide_the_exact_position() {
        let vaduz = Place {
            lat: 47.1412,
            lon: 9.5215,
            alt_m: 455.0,
        };
        let c = Cell::of(&vaduz);
        assert_eq!(c.centre(), (47.15, 9.55));
        assert_eq!(c.alt_m(), 400.0);
        // A rider 300 m away asks for the same.
        let near = Place {
            lat: 47.143,
            lon: 9.525,
            alt_m: 470.0,
        };
        assert_eq!(Cell::of(&near), c);
        // Negative coordinates floor the same way.
        let west = Cell::of(&Place {
            lat: 28.27,
            lon: -16.64,
            alt_m: 2100.0,
        });
        assert_eq!(west.centre(), (28.25, -16.65));
    }

    #[test]
    fn regional_networks_cover_their_countries() {
        // Vaduz: the MeteoSwiss station Vaduz.
        let (p, s) = regional_for((47.15, 9.55), 400.0).unwrap();
        assert_eq!((p, s.id), (Provider::MeteoSwiss, "VAD"));
        // Feldberg (Black Forest) at 1400 m: the DWD summit station.
        let (p, s) = regional_for((47.87, 8.0), 1400.0).unwrap();
        assert_eq!((p, s.id), (Provider::Dwd, "10908"));
        // Freiburg in the valley: the Freiburg station.
        let (p, s) = regional_for((48.0, 7.85), 250.0).unwrap();
        assert_eq!((p, s.id), (Provider::Dwd, "10803"));
        // Tenerife and the open sea: none.
        assert!(regional_for((28.27, -16.64), 2000.0).is_none());
        assert!(regional_for((45.0, -30.0), 0.0).is_none());
        assert!((distance_km((47.0, 9.0), (48.0, 9.0)) - 111.2).abs() < 0.5);
    }

    #[test]
    fn heights_move_temperature_and_pressure() {
        let o = Obs {
            temperature_c: Some(10.0),
            humidity: Some(0.6),
            pressure_msl_hpa: Some(1013.25),
            ..Default::default()
        };
        // 1000 m higher: 6.5 K colder, ~899 hPa.
        let n = normalize(&o, Some(0.0), 1000.0);
        assert!((n.temperature_c.unwrap() - 3.5).abs() < 1e-4);
        assert!((n.pressure_hpa.unwrap() - 898.7).abs() < 1.0);
        assert!(n.humidity.unwrap() > 0.6, "colder air is moister");
        // A station's pressure moves by the difference of the heights.
        let st = Obs {
            pressure_hpa: Some(964.5),
            ..Default::default()
        };
        let n = normalize(&st, Some(457.0), 457.0);
        assert!((n.pressure_hpa.unwrap() - 964.5).abs() < 1e-3);
        let up = normalize(&st, Some(457.0), 1600.0).pressure_hpa.unwrap();
        assert!(up < 850.0 && up > 830.0, "{up}");
        assert!((dew_point_c(20.0, 0.5) - 9.3).abs() < 0.2);
        assert!((humidity_of(20.0, 9.27) - 0.5).abs() < 0.01);
    }

    #[test]
    fn codes_map_to_a_sky() {
        assert_eq!(wmo_sky(0).cover, Some(0.0));
        assert!(wmo_sky(45).fog);
        assert_eq!(wmo_sky(73).precip, Some(Precip::Snow));
        assert!(wmo_sky(82).showers && wmo_sky(82).strength == 2);
        assert!(wmo_sky(95).thunder && wmo_sky(99).precip == Some(Precip::Hail));
        assert_eq!(wmo_sky(1234), Sky::default());
    }

    fn obs(f: impl FnOnce(&mut Obs)) -> Obs {
        let mut o = Obs {
            temperature_c: Some(12.0),
            humidity: Some(0.7),
            pressure_hpa: Some(960.0),
            cloud_cover: Some(0.2),
            precip_mm_h: Some(0.0),
            wind_mps: Some(3.0),
            gust_mps: Some(7.0),
            wind_from_deg: Some(200.0),
            ..Default::default()
        };
        f(&mut o);
        o
    }

    /// The mapping onto the weather state (spec 9E fields).
    #[test]
    fn provider_fields_become_a_weather_state() {
        let fair = to_state(&obs(|_| {}));
        assert_eq!(fair.precip, Precip::None);
        assert_eq!(fair.cloud_type, CloudType::Cumulus);
        assert!((fair.cloud_cover - 0.2).abs() < 1e-6);
        assert_eq!((fair.wind_mps, fair.gust_mps), (3.0, 7.0));
        assert_eq!(fair.pressure_hpa, 960.0);
        assert!((fair.humidity - 0.7).abs() < 0.01);
        // Base from the spread: 12 °C at 70 % → dew point ~6.7 → ~660 m.
        assert!(
            (fair.cloud_base_m - 660.0).abs() < 40.0,
            "{}",
            fair.cloud_base_m
        );
        // A grey layer with rain.
        let rain = to_state(&obs(|o| {
            o.cloud_cover = Some(1.0);
            o.cloud_low = Some(0.9);
            o.precip_mm_h = Some(3.2);
        }));
        assert_eq!(
            (rain.precip, rain.cloud_type),
            (Precip::Rain, CloudType::Stratus)
        );
        assert_eq!(rain.precip_mm_h, 3.2);
        // Snow by the frozen share, sleet in between, snow by temperature.
        let snow = to_state(&obs(|o| {
            o.precip_mm_h = Some(1.0);
            o.snow_share = Some(1.0);
        }));
        assert_eq!(snow.precip, Precip::Snow);
        assert!(snow.cloud_cover >= 0.85);
        let sleet = to_state(&obs(|o| {
            o.precip_mm_h = Some(1.0);
            o.snow_share = Some(0.5);
        }));
        assert_eq!(sleet.precip, Precip::Sleet);
        let cold = to_state(&obs(|o| {
            o.temperature_c = Some(-3.0);
            o.precip_mm_h = Some(1.0);
        }));
        assert_eq!(cold.precip, Precip::Snow);
        // A trace is none; drizzle from a thin, steady amount.
        assert_eq!(
            to_state(&obs(|o| o.precip_mm_h = Some(0.02))).precip,
            Precip::None
        );
        assert_eq!(
            to_state(&obs(|o| o.precip_mm_h = Some(0.2))).precip,
            Precip::Drizzle
        );
        // A thunderstorm: cumulonimbus with lightning; hail.
        let storm = to_state(&obs(|o| {
            o.precip_mm_h = Some(12.0);
            o.sky = Some(wmo_sky(95));
        }));
        assert_eq!(storm.cloud_type, CloudType::Cumulonimbus);
        assert!(storm.lightning_per_min > 0.0 && storm.precip == Precip::Rain);
        let hail = to_state(&obs(|o| {
            o.precip_mm_h = Some(12.0);
            o.sky = Some(wmo_sky(99));
        }));
        assert_eq!(hail.precip, Precip::Hail);
        // Cirrus: only high cloud.
        let ci = to_state(&obs(|o| {
            o.cloud_cover = Some(0.5);
            o.cloud_high = Some(0.5);
            o.cloud_low = Some(0.0);
            o.cloud_mid = Some(0.05);
        }));
        assert_eq!(ci.cloud_type, CloudType::Cirrus);
        assert!(ci.cloud_base_m >= 6000.0);
        // Fog: the visibility without the rain's own extinction.
        let fog = to_state(&obs(|o| o.visibility_m = Some(200.0)));
        assert!((fog.fog_visibility_m - 200.0).abs() < 1.0);
        let rainy_vis = to_state(&obs(|o| {
            o.precip_mm_h = Some(25.0);
            o.visibility_m = Some(2200.0);
        }));
        assert!(
            rainy_vis.fog_visibility_m > 10_000.0,
            "the rain alone explains it"
        );
        assert!((rainy_vis.visibility_m() - 2200.0).abs() < 400.0);
        let metfog = to_state(&obs(|o| o.fog = Some(1.0)));
        assert!(metfog.fog_visibility_m < 200.0);
        // Missing fields fall back to the default; gusts at least the wind.
        let bare = to_state(&Obs {
            wind_mps: Some(10.0),
            ..Default::default()
        });
        assert!(bare.gust_mps >= 10.0);
        assert_eq!(bare, bare.clamped());
        // Only a code: its typical amount.
        let coded = to_state(&Obs {
            sky: Some(wmo_sky(63)),
            temperature_c: Some(8.0),
            ..Default::default()
        });
        assert_eq!(coded.precip, Precip::Rain);
        assert!(coded.precip_mm_h > 1.0);
        // An amount of 0 with a rain code: dry (the amount wins).
        let dry = to_state(&obs(|o| o.sky = Some(wmo_sky(61))));
        assert_eq!(dry.precip, Precip::None);
    }

    #[test]
    fn series_interpolate_and_measurements_stand_a_while() {
        let h = time::HOUR_MS;
        let s = Series {
            obs: vec![
                obs(|o| {
                    o.at_ms = 0;
                    o.temperature_c = Some(10.0);
                    o.wind_from_deg = Some(350.0);
                }),
                obs(|o| {
                    o.at_ms = h;
                    o.temperature_c = Some(14.0);
                    o.wind_from_deg = Some(30.0);
                }),
            ],
            ..Default::default()
        };
        let m = s.at(h / 2).unwrap();
        assert!((m.temperature_c.unwrap() - 12.0).abs() < 1e-4);
        assert!(
            (m.wind_from_deg.unwrap() - 10.0).abs() < 1e-3,
            "the short way"
        );
        assert_eq!(s.at(-h).unwrap().temperature_c, Some(10.0), "an edge");
        assert!(s.at(4 * h).is_none());
        let (late, clamped) = s.at_or_edge(30 * h).unwrap();
        assert!(clamped && late.temperature_c == Some(14.0));
        let measured = Series {
            measured: true,
            ..s.clone()
        };
        assert_eq!(
            measured.at(h + 30 * 60_000).unwrap().temperature_c,
            Some(14.0)
        );
        assert!(measured.at(h + 3 * h).is_none(), "too old");
        assert!(measured.at_or_edge(h + 3 * h).is_none());
        // Merging: the station's measurement over the model.
        let st = Obs {
            temperature_c: Some(5.0),
            precip_mm_h: Some(0.0),
            ..Default::default()
        };
        let model = obs(|o| {
            o.precip_mm_h = Some(2.0);
            o.sky = Some(wmo_sky(63));
        });
        let both = merge(&st, &model);
        assert_eq!(both.temperature_c, Some(5.0));
        assert_eq!(both.cloud_cover, Some(0.2));
        assert_eq!(both.precip_mm_h, Some(0.0), "measured dry");
        assert_eq!(to_state(&both).precip, Precip::None);
    }
}
