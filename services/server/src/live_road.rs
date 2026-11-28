//! Live road events on the game server (spec 14 Phase 18, 18D *The* Live
//! traffic data *switch*, 18E T13; Q5): the server, never a client, pulls
//! its world's live road source while at least one rider who decides the
//! road traffic has *Live traffic data* on, keeps the records of the
//! world's box from the last fetch only, maps them onto the package's
//! road edges (`ocw_sim::traffic::live`) and lays them over the simulated
//! traffic – closures, roadworks, reported jams, obstructions; the riders
//! who want it get the events as compact edge events (`LiveRoadUpdate`).
//!
//! - [`LiveRoadSource`] is a source: how to ask ([`RoadRequest`]), what it
//!   gives ([`SourceKind`]: events now, density later), its credit, and
//!   how to read its answer **as a stream** into the [`RoadRecord`]s
//!   inside a box. [`DgtDatex2`] is the Spanish traffic authority's
//!   incident feed (DATEX II v3.7, CC BY, no key, every minute; about 3 –
//!   7 MB of XML for all of Spain, re-checked 2026-11-28).
//! - [`SOURCES`] is the per-world source table (by the package's place):
//!   Tenerife → the DGT; Liechtenstein and the Black Forest → none yet
//!   (their live counters and reports need the registrations of Q11);
//!   every other world → none, "no live data for this world yet".
//! - [`RoadFetcher`] performs the requests without blocking the game loop:
//!   [`http::HttpRoadFetcher`] (native; ureq on a worker thread that reads
//!   the gzip'd answer straight into the parser, a process-wide gap
//!   between requests), [`FileRoadFetcher`] (a recorded feed from a file:
//!   `OCW_LIVE_ROAD_FILE`, for runs without the network), and in tests
//!   [`FnRoadFetcher`] with recorded answers.
//! - [`LiveRoadFeed`] is the session's state: when to ask (every
//!   [`FETCH_GAP_MS`] while wanted, the back-off after failures), the
//!   records of the last fetch, what the riders are told; when the source
//!   is down or its last fetch older than [`STALE_MS`], the traffic goes
//!   back to the statistics ("live data not reachable – using
//!   statistics").
//!
//! **Kept and sent (18D):** only the current fetch's records of the box,
//! in memory; the clients get edge numbers, kinds, limits and road names,
//! never a raw record. Logs carry counts only.

use std::io::BufRead;
use std::sync::Arc;

use ocw_sim::traffic::live::{GeoEvent, LiveKind};

/// The DGT's credit (CC BY: attribution), shown while its events apply.
pub const DGT_ATTRIBUTION: &str = "Fuente: DGT";
/// The DGT's incident feed (DATEX II v3.7, spec 18D).
pub const DGT_URL: &str = "https://nap.dgt.es/datex2/v3/dgt/SituationPublication/datex2_v37.xml";
/// What a world without a source says (Q5).
pub const NOTE_NO_SOURCE: &str = "no live data for this world yet";
/// What a rider is told while the source is down (18D *Fallback*).
pub const NOTE_DOWN: &str = "live data not reachable – using statistics";
/// While a session's riders want it: a fetch at most this often (ms; 18D:
/// "events every 2 minutes").
pub const FETCH_GAP_MS: i64 = 120_000;
/// A request without an answer after this long counts as failed (ms; the
/// whole of Spain's feed, read as it comes).
pub const ANSWER_TIMEOUT_MS: i64 = 30_000;
/// The first wait after a failure (ms), doubled up to [`BACKOFF_MAX_MS`].
pub const BACKOFF_MS: i64 = 60_000;
pub const BACKOFF_MAX_MS: i64 = 30 * 60_000;
/// Events older than this are dropped: the statistics again (ms; 18D: 30
/// minutes for events).
pub const STALE_MS: i64 = 30 * 60_000;
/// Never more requests than this in 24 hours to one source (one every
/// two minutes all day).
pub const DAY_MAX: usize = 720;
/// At most this much of an answer is read (bytes, uncompressed).
pub const MAX_BODY: u64 = 64 * 1024 * 1024;

/// A box on the globe (degrees).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoBox {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
}

impl GeoBox {
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        (self.lat_min..=self.lat_max).contains(&lat) && (self.lon_min..=self.lon_max).contains(&lon)
    }
}

/// What a source gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Closures, roadworks, reported jams (T13).
    Events,
    /// Counters: flow and speed (T14, after Q11).
    Density,
}

/// One live record of a source, as it lies on the globe.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadRecord {
    /// The source's record id (in memory only: duplicates in one answer).
    pub id: String,
    pub event: GeoEvent,
    /// When it holds from / until (Unix ms; `None`: open).
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

impl RoadRecord {
    /// Whether it holds at `now_ms`.
    pub fn holds(&self, now_ms: i64) -> bool {
        self.start_ms.is_none_or(|s| s <= now_ms) && self.end_ms.is_none_or(|e| now_ms < e)
    }
}

/// One request to a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadRequest {
    pub seq: u64,
    pub source: &'static str,
    pub url: String,
}

/// A request's result as the fetcher got it: the records of the box (the
/// fetcher parses while it reads), or why there are none.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadAnswer {
    pub seq: u64,
    pub result: Result<Vec<RoadRecord>, String>,
    /// `Retry-After` (s), with a 429 or 503.
    pub retry_after_s: Option<u64>,
}

/// A live road source (spec 18D).
pub trait LiveRoadSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// The credit to show while its data applies.
    fn attribution(&self) -> &'static str;
    /// What it gives, for the menu: "closures, roadworks and reported jams".
    fn what(&self) -> &'static str;
    fn kind(&self) -> SourceKind;
    /// Whether it can be used on this server (`Err`: why not – a key
    /// missing in the server's secrets).
    fn configured(&self) -> Result<(), &'static str>;
    fn request(&self, seq: u64) -> RoadRequest;
    /// Reads an answer as a stream: the records with a point in `area`.
    fn parse(&self, body: &mut dyn BufRead, area: &GeoBox) -> Result<Vec<RoadRecord>, String>;
}

/// One row of the per-world source table.
#[derive(Clone, Copy)]
pub struct SourceEntry {
    /// The world, as Settings and the logs name it.
    pub world: &'static str,
    /// The package's place lies in here; the records are kept inside it.
    pub area: GeoBox,
    /// Its source (`None`: none yet, with `note` why).
    pub source: Option<fn() -> Arc<dyn LiveRoadSource>>,
    pub note: &'static str,
}

impl std::fmt::Debug for SourceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceEntry")
            .field("world", &self.world)
            .field("source", &self.source.is_some())
            .finish_non_exhaustive()
    }
}

fn dgt() -> Arc<dyn LiveRoadSource> {
    Arc::new(DgtDatex2)
}

/// The live road sources by world (spec 18D *Per world*).
pub const SOURCES: &[SourceEntry] = &[
    SourceEntry {
        world: "Tenerife",
        area: GeoBox {
            lat_min: 27.98,
            lat_max: 28.62,
            lon_min: -16.95,
            lon_max: -16.10,
        },
        source: Some(dgt),
        note: "",
    },
    SourceEntry {
        world: "Liechtenstein",
        area: GeoBox {
            lat_min: 46.95,
            lat_max: 47.30,
            lon_min: 9.40,
            lon_max: 9.70,
        },
        source: None,
        note: NOTE_NO_SOURCE,
    },
    SourceEntry {
        world: "Black Forest",
        area: GeoBox {
            lat_min: 47.50,
            lat_max: 48.95,
            lon_min: 7.50,
            lon_max: 8.90,
        },
        source: None,
        note: NOTE_NO_SOURCE,
    },
];

/// The table's row for a package at `place` (lat, lon), if any.
pub fn source_for(place: (f64, f64)) -> Option<&'static SourceEntry> {
    SOURCES
        .iter()
        .find(|e| e.area.contains(place.0, place.1) && e.source.is_some())
}

// ------------------------------------------------------------------ DGT

/// The DGT's incidents in DATEX II v3.7 (spec 18D: Spain without Catalonia
/// and the Basque Country, the Canary Islands included; CC BY, "Fuente:
/// DGT"; no key; updated every minute).
#[derive(Debug, Clone, Copy, Default)]
pub struct DgtDatex2;

/// A situation record being read.
#[derive(Debug, Default)]
struct Rec {
    kind: String,
    id: String,
    validity: String,
    probability: String,
    start: String,
    end: String,
    cause: String,
    road: String,
    direction: String,
    direction_road: String,
    lanes: Vec<String>,
    management: Vec<String>,
    abnormal: String,
    speed_limit: String,
    /// (lat, lon, km) of `from`, `to`, or a point.
    from: (Option<f64>, Option<f64>, Option<f64>),
    to: (Option<f64>, Option<f64>, Option<f64>),
    point: (Option<f64>, Option<f64>, Option<f64>),
}

/// "2027-03-11T00:00:00.000+01:00" → Unix ms.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = &s[19..];
    let mut ms = 0;
    if let Some(r) = rest.strip_prefix('.') {
        let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
        rest = &r[digits.len()..];
        ms = format!("{digits:0<3}").get(0..3)?.parse().ok()?;
    }
    let offset_min = match rest {
        "" | "Z" => 0,
        r => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let h: i64 = r.get(1..3)?.parse().ok()?;
            let m: i64 = r.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0);
            sign * (h * 60 + m)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + se - offset_min * 60) * 1000) + ms)
}

/// The compass bearing of a TPEG direction (`None`: no compass direction).
fn bearing(d: &str) -> Option<f64> {
    Some(match d {
        "northBound" => 0.0,
        "northEastBound" => 45.0,
        "eastBound" => 90.0,
        "southEastBound" => 135.0,
        "southBound" => 180.0,
        "southWestBound" => 225.0,
        "westBound" => 270.0,
        "northWestBound" => 315.0,
        _ => return None,
    })
}

impl Rec {
    /// What the record does to the traffic, and the limit it gives (km/h).
    fn effect(&self) -> Option<(LiveKind, Option<f32>)> {
        let has = |list: &[String], v: &str| list.iter().any(|x| x == v);
        let all_lanes = self.lanes.is_empty() || has(&self.lanes, "allLanesCompleteCarriageway");
        let limit = self
            .speed_limit
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| *v >= ocw_sim::traffic::live::MIN_LIMIT_KMH);
        let maintenance = self.cause == "roadMaintenance";
        let obstruction = matches!(
            self.cause.as_str(),
            "accident" | "vehicleObstruction" | "obstruction" | "environmentalObstruction"
        );
        Some(match self.kind.as_str() {
            "RoadOrCarriagewayOrLaneManagement" => {
                let m = &self.management;
                if has(m, "roadClosed") || has(m, "carriagewayClosures") {
                    (LiveKind::Closed, None)
                } else if has(m, "doNotUseSpecifiedLanesOrCarriageways") {
                    if all_lanes {
                        (LiveKind::Closed, None)
                    } else {
                        (LiveKind::LaneClosed, None)
                    }
                } else if [
                    "laneClosures",
                    "narrowLanes",
                    "singleAlternateLineTraffic",
                    "contraflow",
                    "lanesDeviated",
                    "intermittentShortTermClosures",
                ]
                .iter()
                .any(|t| has(m, t))
                {
                    (LiveKind::LaneClosed, None)
                } else if maintenance || has(m, "newRoadworksLayout") {
                    (LiveKind::Roadworks, None)
                } else {
                    return None;
                }
            }
            "MaintenanceWorks" | "Roadworks" | "ConstructionWorks" => (LiveKind::Roadworks, limit),
            "SpeedManagement" => match limit {
                Some(v) => (LiveKind::SpeedLimit, Some(v)),
                None if maintenance => (LiveKind::Roadworks, None),
                None => return None,
            },
            "AbnormalTraffic" => {
                use ocw_sim::traffic::live::{JAM_QUEUING_KMH, JAM_SLOW_KMH, JAM_STATIONARY_KMH};
                let v = match self.abnormal.as_str() {
                    "stationaryTraffic" => JAM_STATIONARY_KMH,
                    "slowTraffic" | "heavyTraffic" => JAM_SLOW_KMH,
                    _ => JAM_QUEUING_KMH,
                };
                (LiveKind::Jam, Some(v))
            }
            "Accident"
            | "VehicleObstruction"
            | "GeneralObstruction"
            | "AnimalPresenceObstruction"
            | "EnvironmentalObstruction"
            | "InfrastructureDamageObstruction" => (LiveKind::Obstruction, None),
            "GenericSituationRecord" if obstruction => (LiveKind::Obstruction, None),
            "GenericSituationRecord" if maintenance => (LiveKind::Roadworks, None),
            "GenericSituationRecord" if self.cause == "abnormalTraffic" => {
                (LiveKind::Jam, Some(ocw_sim::traffic::live::JAM_QUEUING_KMH))
            }
            _ => return None,
        })
    }

    fn record(&self, area: &GeoBox) -> Option<RoadRecord> {
        if matches!(self.validity.as_str(), "suspended" | "inactive")
            || matches!(self.probability.as_str(), "riskOf" | "improbable")
        {
            return None;
        }
        let ll = |p: (Option<f64>, Option<f64>, Option<f64>)| Some((p.0?, p.1?));
        let (from, to) = match (ll(self.from), ll(self.to), ll(self.point)) {
            (Some(f), Some(t), _) => (f, Some(t)),
            (Some(f), None, _) | (None, Some(f), _) | (None, None, Some(f)) => (f, None),
            _ => return None,
        };
        if !area.contains(from.0, from.1) && !to.is_some_and(|t| area.contains(t.0, t.1)) {
            return None;
        }
        let (kind, speed_kmh) = self.effect()?;
        let length_m = match (self.from.2, self.to.2) {
            (Some(a), Some(b)) => Some((a - b).abs() * 1000.0),
            _ => None,
        };
        let unknown = matches!(self.direction.as_str(), "" | "unknown" | "bothWays")
            || self.direction_road == "both";
        Some(RoadRecord {
            id: self.id.clone(),
            event: GeoEvent {
                kind,
                speed_kmh,
                road: self.road.trim().to_owned(),
                from,
                to,
                length_m,
                one_direction: !unknown,
                bearing_deg: bearing(&self.direction),
            },
            start_ms: parse_time(&self.start),
            end_ms: parse_time(&self.end),
        })
    }
}

/// Where in a record the reader is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    None,
    From,
    To,
    Point,
}

/// Reads a DATEX II v3 SituationPublication as a stream: the records with
/// a point in `area`.
pub fn parse_datex2(body: &mut dyn BufRead, area: &GeoBox) -> Result<Vec<RoadRecord>, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(body);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut out: Vec<RoadRecord> = Vec::new();
    let mut rec: Option<Rec> = None;
    let mut place = Place::None;
    let mut tag = String::new();
    let mut payload = false;
    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("DATEX II: {e}"))?;
        match ev {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match name.as_str() {
                    "payload" => payload = true,
                    "situationRecord" => {
                        let attr = |k: &str| {
                            e.try_get_attribute(k)
                                .ok()
                                .flatten()
                                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                                .unwrap_or_default()
                        };
                        let kind = attr("xsi:type");
                        let kind = kind.rsplit(':').next().unwrap_or_default().to_owned();
                        rec = Some(Rec {
                            kind,
                            id: attr("id"),
                            ..Default::default()
                        });
                        place = Place::None;
                    }
                    "from" if rec.is_some() => place = Place::From,
                    "to" if rec.is_some() => place = Place::To,
                    "point" | "tpegPointLocation" if rec.is_some() => place = Place::Point,
                    _ => {}
                }
                tag = name;
            }
            Event::Text(t) => {
                let Some(r) = rec.as_mut() else {
                    continue;
                };
                let text = t.decode().map_err(|e| format!("DATEX II: {e}"))?;
                let text = text.trim();
                let num = text.parse::<f64>().ok();
                let at = match place {
                    Place::From => Some(&mut r.from),
                    Place::To => Some(&mut r.to),
                    Place::Point => Some(&mut r.point),
                    Place::None => None,
                };
                match tag.as_str() {
                    "latitude" => {
                        if let Some(p) = at {
                            p.0 = num;
                        }
                    }
                    "longitude" => {
                        if let Some(p) = at {
                            p.1 = num;
                        }
                    }
                    "kilometerPoint" => {
                        if let Some(p) = at {
                            p.2 = num;
                        }
                    }
                    "validityStatus" => r.validity = text.into(),
                    "probabilityOfOccurrence" => r.probability = text.into(),
                    "overallStartTime" => r.start = text.into(),
                    "overallEndTime" => r.end = text.into(),
                    "causeType" => r.cause = text.into(),
                    "roadName" => r.road = text.into(),
                    "tpegDirection" => r.direction = text.into(),
                    "tpegDirectionRoad" => r.direction_road = text.into(),
                    "laneUsage" => r.lanes.push(text.into()),
                    "roadOrCarriagewayOrLaneManagementType" => r.management.push(text.into()),
                    "abnormalTrafficType" => r.abnormal = text.into(),
                    "temporarySpeedLimit" => r.speed_limit = text.into(),
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"situationRecord" => {
                        if let Some(r) = rec.take()
                            && let Some(x) = r.record(area)
                            && !out.iter().any(|o| o.id == x.id && !x.id.is_empty())
                        {
                            out.push(x);
                        }
                    }
                    b"from" | b"to" | b"point" | b"tpegPointLocation" => place = Place::None,
                    _ => {}
                }
                tag.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !payload {
        return Err("DATEX II: no payload".into());
    }
    Ok(out)
}

impl LiveRoadSource for DgtDatex2 {
    fn name(&self) -> &'static str {
        "DGT DATEX II"
    }

    fn attribution(&self) -> &'static str {
        DGT_ATTRIBUTION
    }

    fn what(&self) -> &'static str {
        "closures, roadworks and reported jams"
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Events
    }

    fn configured(&self) -> Result<(), &'static str> {
        Ok(())
    }

    fn request(&self, seq: u64) -> RoadRequest {
        RoadRequest {
            seq,
            source: self.name(),
            url: DGT_URL.into(),
        }
    }

    fn parse(&self, body: &mut dyn BufRead, area: &GeoBox) -> Result<Vec<RoadRecord>, String> {
        parse_datex2(body, area)
    }
}

// ------------------------------------------------------------ fetchers

/// Performs a feed's requests (see the module docs).
pub trait RoadFetcher: Send {
    /// Starts a request; the fetcher reads the answer with `source` and
    /// keeps only `area`. Never blocks.
    fn start(&mut self, req: &RoadRequest, source: Arc<dyn LiveRoadSource>, area: GeoBox);
    /// The answers that arrived since the last call.
    fn poll(&mut self) -> Vec<RoadAnswer>;
}

/// A fetcher answering from a function (tests: recorded bodies, no
/// network): `Some(Ok(body))` is parsed by the source, `Some(Err(..))`
/// fails, `None` never answers (a source that hangs); every request is
/// counted.
pub struct FnRoadFetcher<F> {
    answer: F,
    queue: Vec<RoadAnswer>,
    pub log: Arc<std::sync::Mutex<Vec<RoadRequest>>>,
}

impl<F: FnMut(&RoadRequest) -> Option<Result<String, String>> + Send> FnRoadFetcher<F> {
    pub fn new(answer: F) -> Self {
        Self {
            answer,
            queue: Vec::new(),
            log: Default::default(),
        }
    }
}

impl<F: FnMut(&RoadRequest) -> Option<Result<String, String>> + Send> RoadFetcher
    for FnRoadFetcher<F>
{
    fn start(&mut self, req: &RoadRequest, source: Arc<dyn LiveRoadSource>, area: GeoBox) {
        if let Ok(mut l) = self.log.lock() {
            l.push(req.clone());
        }
        if let Some(r) = (self.answer)(req) {
            let result = r.and_then(|body| source.parse(&mut body.as_bytes(), &area));
            self.queue.push(RoadAnswer {
                seq: req.seq,
                result,
                retry_after_s: None,
            });
        }
    }

    fn poll(&mut self) -> Vec<RoadAnswer> {
        std::mem::take(&mut self.queue)
    }
}

/// A recorded feed from a file (`OCW_LIVE_ROAD_FILE`): every request reads
/// it again, so a run without the network shows the recorded events.
#[derive(Debug, Clone)]
pub struct FileRoadFetcher {
    pub path: std::path::PathBuf,
    queue: Vec<RoadAnswer>,
}

impl FileRoadFetcher {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            queue: Vec::new(),
        }
    }

    /// The file `OCW_LIVE_ROAD_FILE` names, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var_os("OCW_LIVE_ROAD_FILE")
            .filter(|p| !p.is_empty())
            .map(Self::new)
    }
}

impl RoadFetcher for FileRoadFetcher {
    fn start(&mut self, req: &RoadRequest, source: Arc<dyn LiveRoadSource>, area: GeoBox) {
        let result = std::fs::File::open(&self.path)
            .map_err(|e| format!("recorded feed: {e}"))
            .and_then(|f| source.parse(&mut std::io::BufReader::new(f), &area));
        self.queue.push(RoadAnswer {
            seq: req.seq,
            result,
            retry_after_s: None,
        });
    }

    fn poll(&mut self) -> Vec<RoadAnswer> {
        std::mem::take(&mut self.queue)
    }
}

// ---------------------------------------------------------------- feed

/// What the feed tells the riders who want live road data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedState {
    /// The world has no source.
    NoSource,
    /// A source, nothing fetched (nobody wants it, or the first fetch is
    /// on its way).
    Idle,
    Live,
    /// Failed, paused, or the last fetch too old: the statistics.
    Down,
}

/// Counters for the `traffic` command and the logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedStats {
    pub requests: u64,
    pub answers: u64,
    pub failures: u64,
    /// Records of the box in the last answer.
    pub records: usize,
}

/// The live road data of a session (present on every server; it fetches
/// only with a fetcher, on a world with a source, and while wanted).
pub struct LiveRoadFeed {
    entry: Option<SourceEntry>,
    source: Option<Arc<dyn LiveRoadSource>>,
    seq: u64,
    in_flight: Option<(u64, i64)>,
    /// The requests of the last 24 hours (Unix ms).
    times: std::collections::VecDeque<i64>,
    records: Vec<RoadRecord>,
    fetched_ms: Option<i64>,
    failures: u32,
    retry_at_ms: i64,
    state: FeedState,
    note: String,
    /// The records or the state changed since [`LiveRoadFeed::take_changed`].
    changed: bool,
    pub stats: FeedStats,
}

impl std::fmt::Debug for LiveRoadFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRoadFeed")
            .field("world", &self.entry.map(|e| e.world))
            .field("state", &self.state)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Default for LiveRoadFeed {
    fn default() -> Self {
        Self::new(None)
    }
}

impl LiveRoadFeed {
    /// The feed of the world at `place` (lat, lon; `None`: no place – a
    /// test route).
    pub fn new(place: Option<(f64, f64)>) -> Self {
        let mut f = Self {
            entry: None,
            source: None,
            seq: 0,
            in_flight: None,
            times: Default::default(),
            records: Vec::new(),
            fetched_ms: None,
            failures: 0,
            retry_at_ms: i64::MIN,
            state: FeedState::NoSource,
            note: NOTE_NO_SOURCE.into(),
            changed: false,
            stats: FeedStats::default(),
        };
        f.set_place(place);
        f
    }

    /// With another source for the world (tests).
    pub fn with_source(mut self, source: Arc<dyn LiveRoadSource>, area: GeoBox) -> Self {
        self.entry = Some(SourceEntry {
            world: "test",
            area,
            source: None,
            note: "",
        });
        self.source = Some(source);
        self.state = FeedState::Idle;
        self.note.clear();
        self
    }

    /// The world changed: its row of the table; the last records were for
    /// the old one.
    pub fn set_place(&mut self, place: Option<(f64, f64)>) {
        let entry = place.and_then(source_for).copied();
        let same = match (entry, self.entry) {
            (Some(a), Some(b)) => a.world == b.world,
            (None, None) => true,
            _ => false,
        };
        if same && place.is_some() {
            return;
        }
        self.entry = entry;
        self.source = entry.and_then(|e| e.source).map(|f| f());
        self.records.clear();
        self.fetched_ms = None;
        self.in_flight = None;
        self.failures = 0;
        self.retry_at_ms = i64::MIN;
        match (&self.source, place) {
            (Some(s), _) => {
                self.state = FeedState::Idle;
                self.note = match s.configured() {
                    Ok(()) => String::new(),
                    Err(why) => why.into(),
                };
            }
            (None, p) => {
                self.state = FeedState::NoSource;
                self.note = p
                    .and_then(|p| {
                        SOURCES
                            .iter()
                            .find(|e| e.area.contains(p.0, p.1))
                            .map(|e| e.note)
                    })
                    .filter(|n| !n.is_empty())
                    .unwrap_or(NOTE_NO_SOURCE)
                    .into();
            }
        }
        self.changed = true;
    }

    pub fn state(&self) -> FeedState {
        self.state
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    /// The world's source (`None`: none).
    pub fn source(&self) -> Option<&dyn LiveRoadSource> {
        self.source.as_deref()
    }

    pub fn world(&self) -> Option<&'static str> {
        self.entry.map(|e| e.world)
    }

    pub fn fetched_ms(&self) -> Option<i64> {
        self.fetched_ms
    }

    /// The records in force at `now_ms` (empty unless live).
    pub fn records(&self, now_ms: i64) -> Vec<&RoadRecord> {
        if self.state != FeedState::Live {
            return Vec::new();
        }
        self.records.iter().filter(|r| r.holds(now_ms)).collect()
    }

    /// Requests in the last 24 hours.
    pub fn used_today(&mut self, now_ms: i64) -> usize {
        while self
            .times
            .front()
            .is_some_and(|t| *t <= now_ms - 86_400_000)
        {
            self.times.pop_front();
        }
        self.times.len()
    }

    /// Whether the records or the state changed (once).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn set_state(&mut self, state: FeedState, note: &str) {
        if self.state != state || self.note != note {
            self.state = state;
            self.note = note.into();
            self.changed = true;
        }
    }

    fn fail(&mut self, now_ms: i64, why: &str, retry_after_s: Option<u64>) {
        self.failures += 1;
        self.stats.failures += 1;
        let mut wait = (BACKOFF_MS << (self.failures - 1).min(10)).min(BACKOFF_MAX_MS);
        if let Some(s) = retry_after_s {
            wait = wait.max((s as i64).saturating_mul(1000).min(86_400_000));
        }
        self.retry_at_ms = now_ms + wait;
        let source = self.source.as_ref().map_or("live roads", |s| s.name());
        tracing::warn!(
            source,
            failures = self.failures,
            retry_s = wait / 1000,
            "live road data unavailable ({why}): the statistics meanwhile"
        );
        if !self.records.is_empty() {
            self.records.clear();
            self.changed = true;
        }
        self.fetched_ms = None;
        self.set_state(FeedState::Down, NOTE_DOWN);
    }

    /// One step at `now_ms` (the server's real clock): `wanted` while a
    /// rider who decides the road traffic wants live data. Returns whether
    /// the riders should be told (records or state changed).
    pub fn step(
        &mut self,
        now_ms: i64,
        wanted: bool,
        fetcher: Option<&mut (dyn RoadFetcher + '_)>,
    ) -> bool {
        let (Some(source), Some(area)) = (self.source.clone(), self.entry.map(|e| e.area)) else {
            return self.take_changed();
        };
        let Some(fetcher) = fetcher else {
            return self.take_changed();
        };
        if source.configured().is_err() {
            return self.take_changed();
        }
        for a in fetcher.poll() {
            if self.in_flight.map(|f| f.0) != Some(a.seq) {
                continue;
            }
            self.in_flight = None;
            self.answer(now_ms, a);
        }
        if let Some((_, started)) = self.in_flight
            && now_ms - started > ANSWER_TIMEOUT_MS
        {
            self.in_flight = None;
            self.fail(now_ms, "no answer", None);
        }
        // Too old (paused, the budget): the statistics.
        if let Some(t) = self.fetched_ms
            && now_ms - t >= STALE_MS
            && self.state == FeedState::Live
        {
            self.records.clear();
            self.fetched_ms = None;
            self.changed = true;
            self.set_state(FeedState::Down, NOTE_DOWN);
        }
        if !wanted {
            // Nobody wants it: nothing applies; the next rider starts anew.
            if self.state == FeedState::Live || !self.records.is_empty() {
                self.records.clear();
                self.fetched_ms = None;
                self.changed = true;
                self.set_state(FeedState::Idle, "");
            }
            return self.take_changed();
        }
        let due = self.fetched_ms.is_none_or(|t| now_ms - t >= FETCH_GAP_MS);
        let last_gap_ok = self
            .times
            .back()
            .is_none_or(|t| now_ms - t >= FETCH_GAP_MS / 2);
        if self.in_flight.is_none()
            && due
            && last_gap_ok
            && now_ms >= self.retry_at_ms
            && self.used_today(now_ms) < DAY_MAX
        {
            self.seq += 1;
            self.times.push_back(now_ms);
            self.stats.requests += 1;
            let req = source.request(self.seq);
            self.in_flight = Some((self.seq, now_ms));
            fetcher.start(&req, source.clone(), area);
            for a in fetcher.poll() {
                if a.seq == self.seq {
                    self.in_flight = None;
                    self.answer(now_ms, a);
                }
            }
        }
        self.take_changed()
    }

    fn answer(&mut self, now_ms: i64, a: RoadAnswer) {
        let records = match a.result {
            Ok(r) => r,
            Err(e) => return self.fail(now_ms, &e, a.retry_after_s),
        };
        self.failures = 0;
        self.retry_at_ms = i64::MIN;
        self.stats.answers += 1;
        self.stats.records = records.len();
        tracing::info!(
            source = self.source.as_ref().map_or("", |s| s.name()),
            records = records.len(),
            "live road data fetched"
        );
        if records != self.records {
            self.records = records;
            self.changed = true;
        }
        self.fetched_ms = Some(now_ms);
        self.set_state(FeedState::Live, "");
    }
}

/// The HTTP fetcher (native only): ureq on one worker thread per process,
/// reading the (gzip'd) answer straight into the source's parser; never two
/// requests to the source within a minute from this process.
#[cfg(not(target_arch = "wasm32"))]
pub mod http {
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use super::{GeoBox, LiveRoadSource, RoadAnswer, RoadFetcher, RoadRequest};

    /// `OCW_LIVE_ROAD_OFFLINE=1`: no requests (every one fails at once).
    pub fn offline() -> bool {
        std::env::var("OCW_LIVE_ROAD_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
    }

    /// Our User-Agent: the program, no personal data.
    pub fn user_agent() -> String {
        concat!(
            "OpenCyclingWorld/",
            env!("CARGO_PKG_VERSION"),
            " (open-source cycling simulation; game server live road events)"
        )
        .into()
    }

    struct Job {
        req: RoadRequest,
        source: Arc<dyn LiveRoadSource>,
        area: GeoBox,
        reply: Sender<RoadAnswer>,
    }

    fn worker() -> &'static Mutex<Sender<Job>> {
        static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("ocw-live-road-http".into())
                .spawn(move || run(&rx))
                .expect("live road thread");
            Mutex::new(tx)
        })
    }

    /// The least time between two requests of this process (ms).
    const PROCESS_GAP_MS: i64 = 60_000;

    fn run(rx: &Receiver<Job>) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_millis(
                super::ANSWER_TIMEOUT_MS as u64 - 2_000,
            )))
            .http_status_as_error(false)
            .user_agent(user_agent())
            .build()
            .into();
        let mut last: Option<i64> = None;
        while let Ok(job) = rx.recv() {
            let now = crate::unix_now_ms();
            let answer = |result| RoadAnswer {
                seq: job.req.seq,
                result,
                retry_after_s: None,
            };
            let a = if offline() {
                answer(Err("offline (OCW_LIVE_ROAD_OFFLINE)".into()))
            } else if last.is_some_and(|t| now - t < PROCESS_GAP_MS) {
                answer(Err("over this process's request gap".into()))
            } else {
                last = Some(now);
                get(&agent, &job)
            };
            let _ = job.reply.send(a);
        }
    }

    fn get(agent: &ureq::Agent, job: &Job) -> RoadAnswer {
        let fail = |e: String, retry_after_s| RoadAnswer {
            seq: job.req.seq,
            result: Err(e),
            retry_after_s,
        };
        let resp = match agent.get(&job.req.url).call() {
            Ok(r) => r,
            Err(e) => return fail(short(&e.to_string()), None),
        };
        let status = resp.status().as_u16();
        if status != 200 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok());
            return fail(format!("HTTP {status}"), retry);
        }
        let reader = resp.into_body().into_reader().take(super::MAX_BODY);
        let mut reader = std::io::BufReader::with_capacity(64 * 1024, reader);
        RoadAnswer {
            seq: job.req.seq,
            result: job.source.parse(&mut reader, &job.area),
            retry_after_s: None,
        }
    }

    /// An error without the URL.
    fn short(e: &str) -> String {
        let s: String = e
            .split_whitespace()
            .filter(|w| !w.contains("://"))
            .collect::<Vec<_>>()
            .join(" ");
        s.chars().take(120).collect()
    }

    /// A session's handle on the worker.
    pub struct HttpRoadFetcher {
        tx: Sender<RoadAnswer>,
        rx: Receiver<RoadAnswer>,
    }

    impl Default for HttpRoadFetcher {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self { tx, rx }
        }
    }

    impl RoadFetcher for HttpRoadFetcher {
        fn start(&mut self, req: &RoadRequest, source: Arc<dyn LiveRoadSource>, area: GeoBox) {
            let job = Job {
                req: req.clone(),
                source,
                area,
                reply: self.tx.clone(),
            };
            let sent = worker().lock().map(|w| w.send(job).is_ok());
            if !matches!(sent, Ok(true)) {
                let _ = self.tx.send(RoadAnswer {
                    seq: req.seq,
                    result: Err("the live road thread ended".into()),
                    retry_after_s: None,
                });
            }
        }

        fn poll(&mut self) -> Vec<RoadAnswer> {
            self.rx.try_iter().collect()
        }
    }

    /// The fetcher for a native server: a recorded feed when
    /// `OCW_LIVE_ROAD_FILE` names one, else HTTP.
    pub fn default_fetcher() -> Box<dyn RoadFetcher> {
        match super::FileRoadFetcher::from_env() {
            Some(f) => Box::new(f),
            None => Box::new(HttpRoadFetcher::default()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn errors_lose_the_url_and_the_agent_is_generic() {
            assert_eq!(
                short("io: timed out reading https://nap.dgt.es/datex2/v3/x.xml"),
                "io: timed out reading"
            );
            let ua = user_agent();
            assert!(ua.starts_with("OpenCyclingWorld/") && !ua.contains('@'));
        }
    }
}

#[cfg(test)]
mod tests;
