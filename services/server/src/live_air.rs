//! Live aircraft on the game server (spec 14 Phase 18, 18C, 18D, 18E T10;
//! Q6 (b)): the server, never a client, asks an ADS-B network for the
//! aircraft around the session's place while at least one rider has
//! *Aircraft: live*, keeps the last answer only, and sends those riders
//! the aircraft as compact states (`LiveAirUpdate`) – one fetch for all
//! riders of a session, the same live picture for all.
//!
//! - [`LiveAirSource`] is a network: how to ask ([`Request`]) and how to
//!   read its answer into [`Sighting`]s. [`AdsbLol`] is the source we use
//!   (ODbL, 500 requests a day per address without a key, re-checked
//!   2026-11-27); [`OpenSky`] is the documented fallback, **disabled**:
//!   its licence allows non-profit research and education only, any other
//!   use needs OpenSky's written agreement, which we do not have.
//! - [`AirFetcher`] performs the requests without blocking the game loop:
//!   [`http::HttpAirFetcher`] (native only; ureq on a worker thread, a
//!   process-wide [`Budget`] because the limit is per IP address); tests
//!   use [`FnAirFetcher`] with recorded answers (no network).
//! - [`LiveAirFeed`] is the session's state: when to ask (the rate limit,
//!   the back-off after failures), the session numbers of the aircraft
//!   (carried on from fetch to fetch by a salted hash of the address held
//!   for the last fetch only), what the riders are told.
//!
//! **Privacy (18D):** a sighting keeps no ICAO address, callsign,
//! registration, operator or type code – only a salted 64-bit hash of the
//! address (in memory, for the next fetch's numbering; never sent, never
//! logged, the salt new for every server), the class, the position and
//! the motion. Logs carry counts only.

use std::collections::{HashMap, VecDeque};

use ocw_protocol::live_air::aircraft_to_wire;
use ocw_protocol::v1;
use ocw_sim::air::AirKind;
use ocw_sim::air::live::{LiveAircraft, classify};

/// The adsb.lol credit (ODbL: attribution), shown in the app while live
/// aircraft are drawn.
pub const ADSB_LOL_ATTRIBUTION: &str = "Aircraft data: adsb.lol (ODbL)";
/// The radius of one area query (nautical miles; about 65 km – the
/// riders see aircraft out to the horizon of the haze).
pub const QUERY_RADIUS_NM: u32 = 35;
/// The query's centre moves with the riders in steps of this (degrees):
/// a rider moving within a cell keeps the same query.
pub const CENTRE_STEP_DEG: f64 = 0.05;
/// A request without an answer after this long counts as failed (ms): the
/// clients get the simulated sky within 5 s (18E T10).
pub const ANSWER_TIMEOUT_MS: i64 = 3_500;
/// The first wait after a failure (ms); doubled on each further failure
/// up to [`BACKOFF_MAX_MS`].
pub const BACKOFF_MS: i64 = 60_000;
pub const BACKOFF_MAX_MS: i64 = 30 * 60_000;
/// The last answer is dropped this long after its fetch (ms): the clients
/// extrapolate for at most `ocw_sim::air::live::MAX_AGE_S`.
pub const STALE_MS: i64 = (ocw_sim::air::live::MAX_AGE_S as i64 - 20) * 1000;
/// Sightings whose position is older than this in the answer (s) are left
/// out.
pub const MAX_SEEN_POS_S: f64 = 60.0;

/// One request to a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The request's number in its feed (its answer carries it back).
    pub seq: u64,
    pub source: &'static str,
    pub url: String,
    /// Extra headers (an API key); never logged.
    pub headers: Vec<(String, String)>,
}

/// A source's answer as the fetcher got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub seq: u64,
    /// The HTTP status and body, or why there is none.
    pub result: Result<(u16, String), String>,
    /// `Retry-After` (s), with a 429 or 503.
    pub retry_after_s: Option<u64>,
    /// `x-ratelimit-remaining` and `x-ratelimit-reset` (s): what the
    /// source says is left of its window.
    pub remaining: Option<u32>,
    pub reset_s: Option<u64>,
}

impl Default for Answer {
    fn default() -> Self {
        Self {
            seq: 0,
            result: Err("no answer".into()),
            retry_after_s: None,
            remaining: None,
            reset_s: None,
        }
    }
}

/// An aircraft as a source saw it, without identities.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sighting {
    /// A salted hash of its address: which aircraft of the last fetch it
    /// is. In memory only.
    pub key: u64,
    pub kind: AirKind,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub speed_mps: f64,
    pub track_deg: f64,
    pub climb_mps: f64,
    /// How long before the answer's time the position was seen (s).
    pub seen_s: f64,
}

/// A network of live aircraft positions (18D *Flights*).
pub trait LiveAirSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// The credit to show while its aircraft are drawn.
    fn attribution(&self) -> &'static str;
    /// Whether we may use it (`Err`: why not).
    fn usable(&self) -> Result<(), &'static str>;
    /// The area query around (lat, lon).
    fn request(&self, seq: u64, lat: f64, lon: f64, radius_nm: u32) -> Request;
    /// Its answer's aircraft; `salt` hashes the addresses.
    fn parse(&self, body: &str, salt: u64) -> Result<Vec<Sighting>, String>;
}

/// FNV-1a of a string, mixed with `salt` (SplitMix64).
pub fn salted_key(salt: u64, s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.trim().to_ascii_lowercase().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    ocw_sim::air::mix(h ^ salt)
}

const FT_M: f64 = 0.3048;
const KT_MPS: f64 = 1852.0 / 3600.0;
const FPM_MPS: f64 = FT_M / 60.0;

/// adsb.lol (spec 18D: the first choice; ODbL; `/v2/point/{lat}/{lon}/{nm}`,
/// up to 250 nm; 500 requests a day per address without a key, more with a
/// key in `x-api-key`).
#[derive(Debug, Clone, Default)]
pub struct AdsbLol {
    /// A key (`OCW_ADSB_LOL_KEY` on the server; a secret, never in the
    /// repository or the logs).
    pub api_key: Option<String>,
}

impl AdsbLol {
    pub const HOST: &'static str = "https://api.adsb.lol";

    /// The source with the server's key, if it has one.
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("OCW_ADSB_LOL_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty()),
        }
    }
}

fn num(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(serde_json::Value::as_f64)
}

impl LiveAirSource for AdsbLol {
    fn name(&self) -> &'static str {
        "adsb.lol"
    }

    fn attribution(&self) -> &'static str {
        ADSB_LOL_ATTRIBUTION
    }

    fn usable(&self) -> Result<(), &'static str> {
        Ok(())
    }

    fn request(&self, seq: u64, lat: f64, lon: f64, radius_nm: u32) -> Request {
        Request {
            seq,
            source: self.name(),
            url: format!(
                "{}/v2/point/{lat:.4}/{lon:.4}/{}",
                Self::HOST,
                radius_nm.min(250)
            ),
            headers: self
                .api_key
                .iter()
                .map(|k| ("x-api-key".to_owned(), k.clone()))
                .collect(),
        }
    }

    fn parse(&self, body: &str, salt: u64) -> Result<Vec<Sighting>, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("adsb.lol: not JSON ({e})"))?;
        let list = v
            .get("ac")
            .and_then(serde_json::Value::as_array)
            .ok_or("adsb.lol: no aircraft list")?;
        let mut out = Vec::with_capacity(list.len());
        for a in list {
            let (Some(lat), Some(lon)) = (num(a, "lat"), num(a, "lon")) else {
                continue;
            };
            // On the ground: left out (we draw no taxiing).
            let baro = a.get("alt_baro");
            if baro.and_then(serde_json::Value::as_str) == Some("ground") {
                continue;
            }
            let Some(alt_ft) = num(a, "alt_geom").or_else(|| baro.and_then(|b| b.as_f64())) else {
                continue;
            };
            let seen_s = num(a, "seen_pos").unwrap_or(0.0).max(0.0);
            if seen_s > MAX_SEEN_POS_S {
                continue;
            }
            let alt_m = alt_ft * FT_M;
            let speed_mps = num(a, "gs").unwrap_or(0.0).max(0.0) * KT_MPS;
            let category = a.get("category").and_then(serde_json::Value::as_str);
            let type_code = a.get("t").and_then(serde_json::Value::as_str);
            let Some(kind) = classify(category, type_code, alt_m, speed_mps) else {
                continue;
            };
            let address = a
                .get("hex")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            out.push(Sighting {
                key: salted_key(salt, address),
                kind,
                lat,
                lon,
                alt_m,
                speed_mps,
                track_deg: num(a, "track")
                    .or_else(|| num(a, "true_heading"))
                    .unwrap_or(0.0),
                climb_mps: num(a, "geom_rate")
                    .or_else(|| num(a, "baro_rate"))
                    .unwrap_or(0.0)
                    * FPM_MPS,
                seen_s,
            });
        }
        Ok(out)
    }
}

/// The OpenSky Network (spec 18D: the fallback "if OpenSky agrees"). Its
/// data licence grants use "solely for the purpose of non-profit research,
/// non-profit education … or for government purposes" and no
/// redistribution (checked 2026-11-27); a game showing the aircraft to its
/// riders needs OpenSky's written agreement, and the REST API an OAuth2
/// client secret on our server. Until both exist the source is disabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenSky;

impl LiveAirSource for OpenSky {
    fn name(&self) -> &'static str {
        "OpenSky Network"
    }

    fn attribution(&self) -> &'static str {
        "Aircraft data: The OpenSky Network, https://opensky-network.org"
    }

    fn usable(&self) -> Result<(), &'static str> {
        Err("disabled: OpenSky's licence needs their written agreement for a game")
    }

    fn request(&self, seq: u64, lat: f64, lon: f64, radius_nm: u32) -> Request {
        // A bounding box of the same size (1 credit up to 25 square
        // degrees).
        let dlat = f64::from(radius_nm) / 60.0;
        let dlon = dlat / lat.to_radians().cos().max(0.1);
        Request {
            seq,
            source: self.name(),
            url: format!(
                "https://opensky-network.org/api/states/all?lamin={:.3}&lomin={:.3}&lamax={:.3}&lomax={:.3}",
                lat - dlat,
                lon - dlon,
                lat + dlat,
                lon + dlon
            ),
            headers: Vec::new(),
        }
    }

    fn parse(&self, _body: &str, _salt: u64) -> Result<Vec<Sighting>, String> {
        Err(self.usable().unwrap_err().into())
    }
}

/// The request budget (spec 18D, 18E T10): adsb.lol allows 500 requests a
/// day per address without a key (a fixed 24-hour window). We stay well
/// inside it, also when other programs on the same address ask too:
///
/// - never two requests within [`Budget::MIN_GAP_MS`] (a minute);
/// - a bucket of [`Budget::BURST`] requests that refills with
///   [`Budget::REFILL_PER_DAY`] a day: the first 100 minutes of riding get
///   a fetch a minute, then one every [`Budget::refill_ms`] (4.8 min);
/// - never more than [`Budget::DAY_MAX`] in any 24 hours (burst + refill);
/// - the source's own word: below [`Budget::RESERVE`] requests left in
///   its window, a pause until it resets; after a 429, its `Retry-After`.
#[derive(Debug, Clone)]
pub struct Budget {
    /// The requests of the last 24 hours (Unix ms).
    times: VecDeque<i64>,
    tokens: f64,
    refilled_ms: Option<i64>,
    paused_until_ms: i64,
    /// Requests per day of the bucket's refill and its size (a key's
    /// higher tier may raise them).
    burst: f64,
    per_day: f64,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(Self::BURST, Self::REFILL_PER_DAY)
    }
}

impl Budget {
    pub const DAY_MS: i64 = 86_400_000;
    pub const MIN_GAP_MS: i64 = 60_000;
    pub const BURST: f64 = 100.0;
    pub const REFILL_PER_DAY: f64 = 300.0;
    /// Burst + refill: the most in any 24 hours.
    pub const DAY_MAX: usize = 400;
    /// The source's requests left at which we pause until its reset.
    pub const RESERVE: u32 = 50;
    /// A 429 without `Retry-After`: this pause (ms).
    pub const PAUSE_429_MS: i64 = 15 * 60_000;

    pub fn new(burst: f64, per_day: f64) -> Self {
        Self {
            times: VecDeque::new(),
            tokens: burst,
            refilled_ms: None,
            paused_until_ms: i64::MIN,
            burst,
            per_day,
        }
    }

    /// The refill interval (ms).
    pub fn refill_ms(&self) -> i64 {
        (Self::DAY_MS as f64 / self.per_day) as i64
    }

    fn refill(&mut self, now_ms: i64) {
        let last = self.refilled_ms.unwrap_or(now_ms);
        let dt = (now_ms - last).max(0) as f64;
        self.tokens = (self.tokens + dt * self.per_day / Self::DAY_MS as f64).min(self.burst);
        self.refilled_ms = Some(now_ms.max(last));
        while self
            .times
            .front()
            .is_some_and(|t| *t <= now_ms - Self::DAY_MS)
        {
            self.times.pop_front();
        }
    }

    /// Whether a request may go out now.
    pub fn allows(&mut self, now_ms: i64) -> bool {
        self.refill(now_ms);
        now_ms >= self.paused_until_ms
            && self.tokens >= 1.0
            && self.times.len() < Self::DAY_MAX.min((self.burst + self.per_day) as usize)
            && self
                .times
                .back()
                .is_none_or(|t| now_ms - t >= Self::MIN_GAP_MS)
    }

    /// Takes one request now (call only after [`Budget::allows`]).
    pub fn take(&mut self, now_ms: i64) {
        self.refill(now_ms);
        self.tokens -= 1.0;
        self.times.push_back(now_ms);
    }

    /// Learns from an answer: a 429's `Retry-After`, the window's rest.
    pub fn note(&mut self, now_ms: i64, a: &Answer) {
        if let Ok((429 | 503, _)) = a.result {
            let pause = a
                .retry_after_s
                .map_or(Self::PAUSE_429_MS, |s| (s as i64).saturating_mul(1000));
            self.paused_until_ms = self.paused_until_ms.max(now_ms + pause.min(Self::DAY_MS));
        }
        if let (Some(left), Some(reset)) = (a.remaining, a.reset_s)
            && left < Self::RESERVE
        {
            self.paused_until_ms = self
                .paused_until_ms
                .max(now_ms + (reset as i64).saturating_mul(1000).min(Self::DAY_MS));
        }
    }

    /// How long until the next request may go out after one now (ms; not
    /// counting a pause).
    pub fn next_gap_ms(&self) -> i64 {
        let wait = ((1.0 - self.tokens).max(0.0) * Self::DAY_MS as f64 / self.per_day) as i64;
        wait.max(Self::MIN_GAP_MS)
    }

    /// Paused by the source until then (Unix ms), if in the future.
    pub fn paused_until(&self, now_ms: i64) -> Option<i64> {
        (self.paused_until_ms > now_ms).then_some(self.paused_until_ms)
    }

    /// Requests in the last 24 hours.
    pub fn used_today(&mut self, now_ms: i64) -> usize {
        self.refill(now_ms);
        self.times.len()
    }
}

/// Performs a feed's requests (see the module docs).
pub trait AirFetcher: Send {
    /// Starts a request; never blocks.
    fn start(&mut self, req: &Request);
    /// The answers that arrived since the last call.
    fn poll(&mut self) -> Vec<Answer>;
}

/// A fetcher answering from a function (tests: recorded answers, no
/// network): the answer comes with the next poll (`None`: never – a
/// source that hangs); every request is counted.
pub struct FnAirFetcher<F> {
    answer: F,
    queue: Vec<Answer>,
    /// The requests made so far.
    pub log: std::sync::Arc<std::sync::Mutex<Vec<Request>>>,
}

impl<F: FnMut(&Request) -> Option<Answer> + Send> FnAirFetcher<F> {
    pub fn new(answer: F) -> Self {
        Self {
            answer,
            queue: Vec::new(),
            log: Default::default(),
        }
    }
}

impl<F: FnMut(&Request) -> Option<Answer> + Send> AirFetcher for FnAirFetcher<F> {
    fn start(&mut self, req: &Request) {
        if let Ok(mut l) = self.log.lock() {
            l.push(req.clone());
        }
        if let Some(mut a) = (self.answer)(req) {
            a.seq = req.seq;
            self.queue.push(a);
        }
    }

    fn poll(&mut self) -> Vec<Answer> {
        std::mem::take(&mut self.queue)
    }
}

/// What a feed tells the riders who want live aircraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedState {
    /// Nobody asked yet, or nothing fetched.
    Idle,
    Live,
    Down,
    Unavailable,
}

/// Counters for the `air` command and the logs (no identities).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedStats {
    pub requests: u64,
    pub answers: u64,
    pub failures: u64,
    /// Aircraft in the last answer.
    pub aircraft: usize,
}

/// The live aircraft of a session (present on every server; it fetches
/// only with a fetcher and while someone wants them).
pub struct LiveAirFeed {
    source: Box<dyn LiveAirSource>,
    budget: Budget,
    salt: u64,
    seq: u64,
    /// The request on its way: its number and when it left.
    in_flight: Option<(u64, i64)>,
    /// The last fetch (the only one kept) and when.
    last: Option<v1::LiveAirUpdate>,
    last_fetch_ms: Option<i64>,
    centre: Option<(f64, f64)>,
    /// The session numbers of the last fetch's aircraft by their salted
    /// key (only the last fetch's).
    ids: HashMap<u64, u32>,
    next_id: u32,
    failures: u32,
    retry_at_ms: i64,
    state: FeedState,
    note: String,
    pub stats: FeedStats,
}

impl std::fmt::Debug for LiveAirFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveAirFeed")
            .field("source", &self.source.name())
            .field("state", &self.state)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Default for LiveAirFeed {
    fn default() -> Self {
        Self::new(Box::new(AdsbLol::default()), 0)
    }
}

/// Snaps a place to the query grid.
pub fn query_centre(lat: f64, lon: f64) -> (f64, f64) {
    let s = |v: f64| (v / CENTRE_STEP_DEG).round() * CENTRE_STEP_DEG;
    (s(lat), s(lon))
}

impl LiveAirFeed {
    /// A feed of `source`; `salt` hashes the addresses (random per server).
    pub fn new(source: Box<dyn LiveAirSource>, salt: u64) -> Self {
        Self {
            source,
            budget: Budget::default(),
            salt: ocw_sim::air::mix(salt ^ 0x006c_6976_6561_6972),
            seq: 0,
            in_flight: None,
            last: None,
            last_fetch_ms: None,
            centre: None,
            ids: HashMap::new(),
            next_id: 1,
            failures: 0,
            retry_at_ms: i64::MIN,
            state: FeedState::Idle,
            note: String::new(),
            stats: FeedStats::default(),
        }
    }

    /// With its own request budget (tests, a key's tier).
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    pub fn source_name(&self) -> &'static str {
        self.source.name()
    }

    pub fn state(&self) -> FeedState {
        self.state
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    /// Requests in the last 24 hours.
    pub fn used_today(&mut self, now_ms: i64) -> usize {
        self.budget.used_today(now_ms)
    }

    /// The world changed: the last answer was for another place.
    pub fn world_changed(&mut self) {
        self.last = None;
        self.last_fetch_ms = None;
        self.centre = None;
        self.ids.clear();
        self.in_flight = None;
        if self.state == FeedState::Live {
            self.state = FeedState::Idle;
        }
    }

    fn status_update(&self, status: v1::LiveAirStatus) -> v1::LiveAirUpdate {
        v1::LiveAirUpdate {
            status: status as i32,
            note: self.note.clone(),
            attribution: self.source.attribution().into(),
            ..Default::default()
        }
    }

    /// What a rider who starts wanting live aircraft is told now: the last
    /// answer while it is fresh, else why there is none (`None`: nothing
    /// yet – the first fetch is on its way).
    pub fn current(&self, now_ms: i64, can_fetch: bool) -> Option<v1::LiveAirUpdate> {
        if !can_fetch {
            return Some(self.unavailable());
        }
        if let (Some(u), Some(t)) = (&self.last, self.last_fetch_ms)
            && now_ms - t < STALE_MS
            && self.state == FeedState::Live
        {
            return Some(u.clone());
        }
        match self.state {
            FeedState::Down => Some(self.status_update(v1::LiveAirStatus::Down)),
            _ => None,
        }
    }

    fn unavailable(&self) -> v1::LiveAirUpdate {
        v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Unavailable as i32,
            note: "this server does not fetch live aircraft".into(),
            attribution: self.source.attribution().into(),
            ..Default::default()
        }
    }

    fn fail(&mut self, now_ms: i64, why: String) -> v1::LiveAirUpdate {
        self.failures += 1;
        self.stats.failures += 1;
        let wait = (BACKOFF_MS << (self.failures - 1).min(10)).min(BACKOFF_MAX_MS);
        self.retry_at_ms = now_ms + wait;
        self.state = FeedState::Down;
        self.last = None;
        self.last_fetch_ms = None;
        tracing::warn!(
            source = self.source.name(),
            failures = self.failures,
            retry_s = wait / 1000,
            "live aircraft unavailable ({why}): the simulated sky meanwhile"
        );
        self.note = format!("{}: {why}", self.source.name());
        self.status_update(v1::LiveAirStatus::Down)
    }

    /// One step at `now_ms` (the server's real clock): `wanted` while at
    /// least one rider wants live aircraft, `place` (lat, lon) where the
    /// riders are. Returns the update to send to those riders, if there is
    /// a new one.
    pub fn step(
        &mut self,
        now_ms: i64,
        wanted: bool,
        place: (f64, f64),
        fetcher: Option<&mut (dyn AirFetcher + '_)>,
    ) -> Option<v1::LiveAirUpdate> {
        let fetcher = fetcher?;
        if let Err(why) = self.source.usable() {
            if self.state != FeedState::Unavailable {
                self.state = FeedState::Unavailable;
                self.note = format!("{}: {why}", self.source.name());
                return wanted.then(|| self.status_update(v1::LiveAirStatus::Unavailable));
            }
            return None;
        }
        // Answers.
        let mut out = None;
        for a in fetcher.poll() {
            if self.in_flight.map(|f| f.0) != Some(a.seq) {
                continue; // too late: its request already counted as failed
            }
            self.in_flight = None;
            self.budget.note(now_ms, &a);
            out = Some(self.answer(now_ms, a));
        }
        if let Some((_, started)) = self.in_flight
            && now_ms - started > ANSWER_TIMEOUT_MS
        {
            self.in_flight = None;
            out = Some(self.fail(now_ms, "no answer".into()));
        }
        // The last answer grows too old (the budget is spent, the source
        // paused us): the simulated sky.
        if let Some(t) = self.last_fetch_ms
            && now_ms - t >= STALE_MS
        {
            self.last = None;
            self.last_fetch_ms = None;
            if self.state == FeedState::Live {
                self.state = FeedState::Down;
                self.note = match self.budget.paused_until(now_ms) {
                    Some(_) => format!("{}: paused by its request limit", self.source.name()),
                    None => format!("{}: within our daily request budget", self.source.name()),
                };
                out = Some(self.status_update(v1::LiveAirStatus::Down));
            }
        }
        if !wanted {
            return out;
        }
        let centre = query_centre(place.0, place.1);
        let moved = self.centre != Some(centre);
        let due = self
            .last_fetch_ms
            .is_none_or(|t| now_ms - t >= Budget::MIN_GAP_MS)
            || moved;
        if self.in_flight.is_none()
            && due
            && now_ms >= self.retry_at_ms
            && self.budget.allows(now_ms)
        {
            self.budget.take(now_ms);
            self.seq += 1;
            self.centre = Some(centre);
            let req = self
                .source
                .request(self.seq, centre.0, centre.1, QUERY_RADIUS_NM);
            self.in_flight = Some((self.seq, now_ms));
            self.stats.requests += 1;
            fetcher.start(&req);
            // A fetcher may answer at once (tests).
            for a in fetcher.poll() {
                if a.seq == self.seq {
                    self.in_flight = None;
                    self.budget.note(now_ms, &a);
                    out = Some(self.answer(now_ms, a));
                }
            }
        }
        out
    }

    fn answer(&mut self, now_ms: i64, a: Answer) -> v1::LiveAirUpdate {
        let body = match a.result {
            Ok((200, body)) => body,
            Ok((status, _)) => return self.fail(now_ms, format!("HTTP {status}")),
            Err(e) => return self.fail(now_ms, e),
        };
        let sightings = match self.source.parse(&body, self.salt) {
            Ok(s) => s,
            Err(e) => return self.fail(now_ms, e),
        };
        self.failures = 0;
        self.retry_at_ms = i64::MIN;
        self.stats.answers += 1;
        self.stats.aircraft = sightings.len();
        // Numbers carry on for the aircraft of the last fetch; the others
        // are forgotten with it.
        let mut ids = HashMap::with_capacity(sightings.len());
        let mut aircraft = Vec::with_capacity(sightings.len());
        for s in &sightings {
            let id = match self.ids.get(&s.key).or_else(|| ids.get(&s.key)) {
                Some(id) => *id,
                None => {
                    let id = self.next_id;
                    self.next_id = self.next_id.wrapping_add(1).max(1);
                    id
                }
            };
            ids.insert(s.key, id);
            aircraft.push(aircraft_to_wire(
                &LiveAircraft {
                    id,
                    kind: s.kind,
                    lat: s.lat,
                    lon: s.lon,
                    alt_m: s.alt_m,
                    speed_mps: s.speed_mps,
                    track_deg: s.track_deg,
                    climb_mps: s.climb_mps,
                    at_ms: now_ms - (s.seen_s * 1000.0).round() as i64,
                },
                now_ms,
            ));
        }
        self.ids = ids;
        self.state = FeedState::Live;
        self.note.clear();
        let next_s = self.budget.next_gap_ms() / 1000;
        tracing::info!(
            source = self.source.name(),
            aircraft = aircraft.len(),
            "live aircraft fetched"
        );
        let u = v1::LiveAirUpdate {
            status: v1::LiveAirStatus::Live as i32,
            note: String::new(),
            attribution: self.source.attribution().into(),
            fetched_ms: now_ms,
            aircraft,
            next_s: next_s as u32,
        };
        self.last = Some(u.clone());
        self.last_fetch_ms = Some(now_ms);
        u
    }
}

/// The HTTP fetcher (native only): ureq on one worker thread per process,
/// with the process-wide [`Budget`] (the source's limit is per address).
#[cfg(not(target_arch = "wasm32"))]
pub mod http {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use super::{AirFetcher, Answer, Budget, Request};

    /// `OCW_LIVE_AIR_OFFLINE=1`: no requests (every one fails at once).
    pub fn offline() -> bool {
        std::env::var("OCW_LIVE_AIR_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
    }

    /// Our User-Agent: the program, no personal data.
    pub fn user_agent() -> String {
        concat!(
            "OpenCyclingWorld/",
            env!("CARGO_PKG_VERSION"),
            " (open-source cycling simulation; game server live aircraft)"
        )
        .into()
    }

    struct Job {
        req: Request,
        reply: Sender<Answer>,
    }

    fn worker() -> &'static Mutex<Sender<Job>> {
        static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("ocw-live-air-http".into())
                .spawn(move || run(&rx))
                .expect("live air thread");
            Mutex::new(tx)
        })
    }

    fn run(rx: &Receiver<Job>) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(Some(Duration::from_millis(
                super::ANSWER_TIMEOUT_MS as u64 - 300,
            )))
            .http_status_as_error(false)
            .user_agent(user_agent())
            .build()
            .into();
        // The process's budget: all sessions of this process share the
        // address.
        let mut budget = Budget::default();
        while let Ok(Job { req, reply }) = rx.recv() {
            let now = crate::unix_now_ms();
            let a = if offline() {
                Answer {
                    seq: req.seq,
                    result: Err("offline (OCW_LIVE_AIR_OFFLINE)".into()),
                    ..Default::default()
                }
            } else if !budget.allows(now) {
                Answer {
                    seq: req.seq,
                    result: Err("over this process's request budget".into()),
                    ..Default::default()
                }
            } else {
                budget.take(now);
                let a = get(&agent, &req);
                budget.note(crate::unix_now_ms(), &a);
                a
            };
            let _ = reply.send(a);
        }
    }

    fn header<T: std::str::FromStr>(r: &ureq::http::Response<ureq::Body>, name: &str) -> Option<T> {
        r.headers()
            .get(name)?
            .to_str()
            .ok()?
            // "500, 500;w=86400": the first number.
            .split([',', ';'])
            .next()?
            .trim()
            .parse()
            .ok()
    }

    fn get(agent: &ureq::Agent, req: &Request) -> Answer {
        let mut call = agent.get(&req.url);
        for (k, v) in &req.headers {
            call = call.header(k, v);
        }
        let resp = match call.call() {
            Ok(r) => r,
            Err(e) => {
                return Answer {
                    seq: req.seq,
                    result: Err(short(&e.to_string())),
                    ..Default::default()
                };
            }
        };
        let status = resp.status().as_u16();
        let retry_after_s = header(&resp, "retry-after");
        let remaining = header(&resp, "x-ratelimit-remaining");
        let reset_s = header(&resp, "x-ratelimit-reset");
        let body = resp
            .into_body()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| short(&e.to_string()));
        Answer {
            seq: req.seq,
            result: body.map(|b| (status, b)),
            retry_after_s,
            remaining,
            reset_s,
        }
    }

    /// An error without the URL (it holds the riders' place).
    fn short(e: &str) -> String {
        let s: String = e
            .split_whitespace()
            .filter(|w| !w.contains("://"))
            .collect::<Vec<_>>()
            .join(" ");
        s.chars().take(120).collect()
    }

    /// A session's handle on the worker.
    pub struct HttpAirFetcher {
        tx: Sender<Answer>,
        rx: Receiver<Answer>,
    }

    impl Default for HttpAirFetcher {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self { tx, rx }
        }
    }

    impl AirFetcher for HttpAirFetcher {
        fn start(&mut self, req: &Request) {
            let job = Job {
                req: req.clone(),
                reply: self.tx.clone(),
            };
            let sent = worker().lock().map(|w| w.send(job).is_ok());
            if !matches!(sent, Ok(true)) {
                let _ = self.tx.send(Answer {
                    seq: req.seq,
                    result: Err("the live air thread ended".into()),
                    ..Default::default()
                });
            }
        }

        fn poll(&mut self) -> Vec<Answer> {
            self.rx.try_iter().collect()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn errors_lose_the_url_and_the_agent_is_generic() {
            assert_eq!(
                short("io: timed out reading https://api.adsb.lol/v2/point/47.9/7.9/35"),
                "io: timed out reading"
            );
            let ua = user_agent();
            assert!(ua.starts_with("OpenCyclingWorld/") && !ua.contains('@'));
        }
    }
}

#[cfg(test)]
mod tests;
