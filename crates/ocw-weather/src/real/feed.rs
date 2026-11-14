//! The server's real-weather feed (spec 9F): which providers to ask for the
//! session's place and moment, what to fetch now (fresh answers are not
//! fetched again; failures wait, doubling up to an hour), the fallback
//! across providers, and the weather state with its attribution.
//!
//! The moment is the session clock's (the time machine's date: a past day
//! comes from the history, ERA5; spec 9D "historic weather"), the real
//! instant decides what is history, measurement or forecast.
//!
//! Pure: the game server performs the [`Fetch`]es (HTTP on its own thread,
//! native only) and hands the bodies to [`RealFeed::on_result`].

use std::collections::BTreeMap;

use super::cache::ttl;
use super::time::{DAY_MS, HOUR_MS, day_of, hh_mm_utc};
use super::{
    Cell, Choice, Place, Provider, Series, StationPick, attribution, dwd, merge, meteoswiss, metno,
    normalize, openmeteo, regional_for, to_state,
};
use crate::state::WeatherState;

/// Moments older than this before the real now come from the history
/// (ERA5; the forecast request covers the past week).
pub const HISTORIC_AFTER_MS: i64 = 6 * DAY_MS + 12 * HOUR_MS;
/// Stations count for moments this close to the real now.
pub const MEASURED_WITHIN_MS: i64 = 2 * HOUR_MS;
/// A failed request is asked again after this, doubling (ms) …
pub const RETRY_MS: i64 = 60_000;
/// … up to this.
pub const MAX_RETRY_MS: i64 = HOUR_MS;
/// Data this much older than its time to live is stale (shown with a
/// note: the server is offline or the provider down).
pub const STALE_FACTOR: i64 = 3;
/// Answers kept per session (older cells and days go).
pub const MAX_ENTRIES: usize = 24;

/// What a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A regional station's measurements.
    Measured,
    /// A global forecast (with the past week).
    Forecast,
    /// ERA5 history of one day.
    Historic,
}

impl Kind {
    pub fn ttl_ms(self) -> i64 {
        match self {
            Self::Measured => ttl::MEASURED_MS,
            Self::Forecast => ttl::FORECAST_MS,
            Self::Historic => ttl::HISTORIC_MS,
        }
    }
}

/// One request for the server to perform (a GET of `url`).
#[derive(Debug, Clone, PartialEq)]
pub struct Fetch {
    pub provider: Provider,
    pub kind: Kind,
    pub url: String,
    pub station: Option<StationPick>,
}

impl Fetch {
    /// The body parsed by the provider's format.
    pub fn parse(&self, body: &str) -> Result<Series, String> {
        match (self.provider, &self.station) {
            (Provider::OpenMeteo, _) => openmeteo::parse(body),
            (Provider::MetNorway, _) => metno::parse(body),
            (Provider::MeteoSwiss, Some(s)) => meteoswiss::parse(body, s),
            (Provider::Dwd, Some(s)) => dwd::parse(body, s),
            (p, None) => Err(format!("{}: no station", p.name())),
        }
    }
}

/// How the real weather stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Asked, no answer yet.
    #[default]
    Waiting,
    /// Current data.
    Live,
    /// Only old data (the providers do not answer): the last weather.
    Stale,
    /// No data at all and the providers do not answer.
    Offline,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Live => "live",
            Self::Stale => "stale",
            Self::Offline => "offline",
        }
    }
}

/// The real weather at a moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub state: WeatherState,
    /// Whose data it is (in the attribution's order).
    pub providers: Vec<Provider>,
    /// "Weather: MeteoSwiss, MET Norway, CC BY 4.0 / NLOD 2.0".
    pub attribution: String,
    /// Fallbacks, a provider that does not cover the place, the end of the
    /// forecast, stale data.
    pub note: Option<String>,
    /// The station measured at ("Vaduz (VAD)").
    pub station: Option<String>,
    /// The instant of the data (the measurement's, else the moment).
    pub data_ms: i64,
    pub historic: bool,
    pub status: Status,
}

/// Which sources a moment wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub regional: Option<Fetch>,
    /// The global ones, the first preferred, the rest its fallbacks.
    pub globals: Vec<Fetch>,
    pub historic: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    series: Series,
    fetched_ms: i64,
    kind: Kind,
}

#[derive(Debug, Clone)]
struct Failure {
    count: u32,
    retry_ms: i64,
    error: String,
}

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct RealFeed {
    choice: Choice,
    place: Option<Place>,
    data: BTreeMap<String, Entry>,
    pending: BTreeMap<String, Fetch>,
    failures: BTreeMap<String, Failure>,
}

fn global_fetch(p: Provider, cell: &Cell) -> Fetch {
    Fetch {
        provider: p,
        kind: Kind::Forecast,
        url: match p {
            Provider::MetNorway => metno::url(cell),
            _ => openmeteo::forecast_url(cell),
        },
        station: None,
    }
}

impl RealFeed {
    pub fn new(choice: Choice) -> Self {
        Self {
            choice,
            ..Default::default()
        }
    }

    pub fn choice(&self) -> Choice {
        self.choice
    }

    pub fn set_choice(&mut self, choice: Choice) {
        self.choice = choice;
    }

    pub fn place(&self) -> Option<Place> {
        self.place
    }

    /// Where the weather is wanted (the server's host rider; its cell and
    /// band decide the requests).
    pub fn set_place(&mut self, place: Place) {
        self.place = Some(place);
    }

    /// The sources for moment `at_ms` at the real instant `now_ms`.
    pub fn plan(&self, at_ms: i64, now_ms: i64) -> Option<Plan> {
        let place = self.place?;
        let cell = Cell::of(&place);
        if at_ms < now_ms - HISTORIC_AFTER_MS {
            let note = match self.choice {
                Choice::Only(p) if p != Provider::OpenMeteo => Some(format!(
                    "history comes from ERA5 via Open-Meteo ({} has none here)",
                    p.name()
                )),
                _ => None,
            };
            return Some(Plan {
                regional: None,
                globals: vec![Fetch {
                    provider: Provider::OpenMeteo,
                    kind: Kind::Historic,
                    url: openmeteo::archive_url(&cell, day_of(at_ms)),
                    station: None,
                }],
                historic: true,
                note,
            });
        }
        let near_now = (at_ms - now_ms).abs() <= MEASURED_WITHIN_MS;
        let regional = regional_for(cell.centre(), cell.alt_m());
        let station = |(p, s): (Provider, StationPick)| Fetch {
            provider: p,
            kind: Kind::Measured,
            url: match p {
                Provider::MeteoSwiss => meteoswiss::url(&s),
                _ => dwd::url(&s),
            },
            station: Some(s),
        };
        let mut note = None;
        let globals = |first: Provider| {
            let second = if first == Provider::MetNorway {
                Provider::OpenMeteo
            } else {
                Provider::MetNorway
            };
            vec![global_fetch(first, &cell), global_fetch(second, &cell)]
        };
        let (reg, glob) = match self.choice {
            Choice::Only(p) if p.is_regional() => match regional {
                Some(r) if r.0 == p => {
                    if !near_now {
                        note = Some(format!(
                            "{} measures the weather now; other times from MET Norway's forecast",
                            p.name()
                        ));
                    }
                    (Some(r), globals(Provider::MetNorway))
                }
                _ => {
                    note = Some(format!("{} does not cover this place: Automatic", p.name()));
                    (regional, globals(Provider::MetNorway))
                }
            },
            Choice::Only(p) => (None, globals(p)),
            Choice::Automatic => (regional, globals(Provider::MetNorway)),
        };
        Some(Plan {
            regional: reg.filter(|_| near_now).map(station),
            globals: glob,
            historic: false,
            note,
        })
    }

    fn usable(&self, url: &str, now_ms: i64) -> bool {
        self.data
            .get(url)
            .is_some_and(|e| now_ms - e.fetched_ms < e.kind.ttl_ms() && now_ms >= e.fetched_ms)
    }

    fn may_ask(&self, url: &str, now_ms: i64) -> bool {
        !self.pending.contains_key(url)
            && !self.usable(url, now_ms)
            && self.failures.get(url).is_none_or(|f| now_ms >= f.retry_ms)
    }

    fn failed(&self, url: &str) -> bool {
        self.failures.contains_key(url)
    }

    /// The requests to make now for moment `at_ms` (marked as pending):
    /// the regional station, the preferred global source, and its fallback
    /// while the preferred one fails and has no data.
    pub fn wants(&mut self, at_ms: i64, now_ms: i64) -> Vec<Fetch> {
        let Some(plan) = self.plan(at_ms, now_ms) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(r) = plan.regional {
            out.push(r);
        }
        for g in &plan.globals {
            out.push(g.clone());
            // The next one only while this one fails without data.
            if !(self.failed(&g.url) && !self.data.contains_key(&g.url)) {
                break;
            }
        }
        out.retain(|f| self.may_ask(&f.url, now_ms));
        for f in &out {
            self.pending.insert(f.url.clone(), f.clone());
        }
        out
    }

    /// Whether a request is out.
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The answer to a request (`Err`: the network, the provider's status
    /// or a body that does not parse).
    pub fn on_result(&mut self, url: &str, result: Result<String, String>, now_ms: i64) {
        let Some(fetch) = self.pending.remove(url) else {
            return;
        };
        match result.and_then(|body| fetch.parse(&body)) {
            Ok(series) => {
                self.failures.remove(url);
                if !self.data.contains_key(url)
                    && self.data.len() >= MAX_ENTRIES
                    && let Some(k) = self
                        .data
                        .iter()
                        .min_by_key(|(_, e)| e.fetched_ms)
                        .map(|(k, _)| k.clone())
                {
                    self.data.remove(&k);
                }
                self.data.insert(
                    url.to_owned(),
                    Entry {
                        series,
                        fetched_ms: now_ms,
                        kind: fetch.kind,
                    },
                );
            }
            Err(error) => {
                let count = self.failures.get(url).map_or(1, |f| f.count + 1);
                let wait = (RETRY_MS << (count - 1).min(10)).min(MAX_RETRY_MS);
                self.failures.insert(
                    url.to_owned(),
                    Failure {
                        count,
                        retry_ms: now_ms + wait,
                        error,
                    },
                );
            }
        }
    }

    /// The last error of a source of the plan, if any ("MET Norway: HTTP
    /// 503").
    pub fn last_error(&self, at_ms: i64, now_ms: i64) -> Option<String> {
        let plan = self.plan(at_ms, now_ms)?;
        plan.regional
            .iter()
            .chain(&plan.globals)
            .find_map(|f| self.failures.get(&f.url).map(|e| e.error.clone()))
    }

    /// How it stands without a resolved weather.
    pub fn status(&self, at_ms: i64, now_ms: i64) -> Status {
        if let Some(r) = self.resolve(at_ms, now_ms) {
            return r.status;
        }
        let Some(plan) = self.plan(at_ms, now_ms) else {
            return Status::Offline;
        };
        let all_failed = plan
            .regional
            .iter()
            .chain(&plan.globals)
            .all(|f| self.failed(&f.url));
        if all_failed {
            Status::Offline
        } else {
            Status::Waiting
        }
    }

    /// The real weather at moment `at_ms` (`None`: no data yet or none at
    /// all; the server keeps its weather then).
    pub fn resolve(&self, at_ms: i64, now_ms: i64) -> Option<Resolved> {
        let place = self.place?;
        let plan = self.plan(at_ms, now_ms)?;
        let mut notes: Vec<String> = plan.note.iter().cloned().collect();
        let mut providers = Vec::new();
        let mut stale_since: Option<i64> = None;
        let mut mark = |e: &Entry| {
            if now_ms - e.fetched_ms > STALE_FACTOR * e.kind.ttl_ms() {
                stale_since = Some(stale_since.map_or(e.fetched_ms, |s: i64| s.min(e.fetched_ms)));
            }
        };
        // The global source: the first with data for the moment.
        let mut global = None;
        for (i, g) in plan.globals.iter().enumerate() {
            let Some(e) = self.data.get(&g.url) else {
                continue;
            };
            let Some((o, clamped)) = e.series.at_or_edge(at_ms) else {
                continue;
            };
            mark(e);
            if i > 0 && self.failed(&plan.globals[0].url) {
                notes.push(format!(
                    "{} unavailable: {}",
                    plan.globals[0].provider.name(),
                    g.provider.name()
                ));
            }
            if clamped {
                notes.push(if at_ms > now_ms {
                    "beyond the forecast: its last hour".into()
                } else {
                    "before the data: its first hour".into()
                });
            }
            global = Some((g.provider, normalize(&o, e.series.elevation_m, place.alt_m)));
            break;
        }
        // The regional station's measurement.
        let mut station = None;
        let mut data_ms = at_ms;
        let regional = plan.regional.as_ref().and_then(|r| {
            let e = self.data.get(&r.url)?;
            let o = e.series.at(at_ms.min(now_ms))?;
            mark(e);
            station.clone_from(&e.series.station);
            data_ms = o.at_ms;
            Some((r.provider, normalize(&o, e.series.elevation_m, place.alt_m)))
        });
        if let Some(r) = &plan.regional
            && regional.is_none()
            && self.failed(&r.url)
        {
            notes.push(format!("{} unavailable", r.provider.name()));
        }
        let mut obs = match (&regional, &global) {
            (Some((p, r)), Some((q, g))) => {
                providers.extend([*p, *q]);
                merge(r, g)
            }
            (Some((p, r)), None) => {
                providers.push(*p);
                *r
            }
            (None, Some((q, g))) => {
                providers.push(*q);
                *g
            }
            (None, None) => return None,
        };
        if obs.pressure_hpa.is_none() {
            obs.pressure_hpa = Some(super::isa_pressure_hpa(place.alt_m));
        }
        let status = match stale_since {
            Some(t) => {
                notes.push(format!("no new data: the weather of {} UTC", hh_mm_utc(t)));
                Status::Stale
            }
            None => Status::Live,
        };
        Some(Resolved {
            state: to_state(&obs),
            attribution: attribution(&providers, plan.historic),
            providers,
            note: (!notes.is_empty()).then(|| notes.join("; ")),
            station,
            data_ms,
            historic: plan.historic,
            status,
        })
    }
}

/// What the clients learn about the real weather (the menu, the HUD's
/// attribution, `weather.status`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RealInfo {
    /// The session's data source.
    pub choice: Choice,
    /// Whose data shows.
    pub providers: Vec<Provider>,
    /// "Weather: MET Norway, CC BY 4.0" (empty while there is no data).
    pub attribution: String,
    pub note: Option<String>,
    pub status: Status,
    /// The instant of the data (Unix ms; 0: none).
    pub data_ms: i64,
    pub historic: bool,
    pub station: Option<String>,
}

impl RealInfo {
    /// "MeteoSwiss, MET Norway" / "waiting for the data" / "offline".
    pub fn source_label(&self) -> String {
        if self.providers.is_empty() {
            return match self.status {
                Status::Offline => "offline".into(),
                _ => "waiting for the data".into(),
            };
        }
        self.providers
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
