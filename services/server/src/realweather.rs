//! Real weather on the game server (spec §14 Phase 9, S7; 9F): the server,
//! never a client, asks the providers for the weather of the session's
//! place (the host rider's position on the globe, as a 0.1° grid cell and
//! a 200 m altitude band) and the world clock's moment, and feeds the
//! session weather with it.
//!
//! - [`WeatherFetcher`] performs the requests without blocking the game
//!   loop. [`http::HttpFetcher`] (native only) does them with ureq on one
//!   worker thread per process, with a process-wide cache per request
//!   (answers fresh for 10 / 30 minutes, history for a week; many sessions
//!   and riders in a cell cause one request), a gap of a second between
//!   requests to the same host, and a pause after "429 Too Many Requests"
//!   or an unreachable host. Tests use their own fetcher (recorded
//!   answers, no network).
//! - [`RealSession`] is the session's state: the feed
//!   ([`ocw_weather::real::feed::RealFeed`]), when to ask and to apply,
//!   the first application (the request's transition, or at once with its
//!   ground), what the clients are told ([`RealInfo`]).

use ocw_weather::real::Choice;
use ocw_weather::real::feed::{Fetch, RealFeed, RealInfo, Status};

/// Performs the feed's requests (see the module docs).
pub trait WeatherFetcher: Send {
    /// Starts a request; never blocks.
    fn start(&mut self, fetch: &Fetch);
    /// The answers that arrived since the last call: the URL and the body
    /// (or why there is none).
    fn poll(&mut self) -> Vec<(String, Result<String, String>)>;
}

/// A fetcher answering from a function (tests: recorded answers, no
/// network): the answer comes with the next poll; every request is logged.
pub struct FnFetcher<F> {
    answer: F,
    queue: Vec<(String, Result<String, String>)>,
    /// The requests made so far.
    pub log: std::sync::Arc<std::sync::Mutex<Vec<Fetch>>>,
}

impl<F: FnMut(&Fetch) -> Result<String, String> + Send> FnFetcher<F> {
    pub fn new(answer: F) -> Self {
        Self {
            answer,
            queue: Vec::new(),
            log: Default::default(),
        }
    }
}

impl<F: FnMut(&Fetch) -> Result<String, String> + Send> WeatherFetcher for FnFetcher<F> {
    fn start(&mut self, fetch: &Fetch) {
        if let Ok(mut l) = self.log.lock() {
            l.push(fetch.clone());
        }
        let a = (self.answer)(fetch);
        self.queue.push((fetch.url.clone(), a));
    }

    fn poll(&mut self) -> Vec<(String, Result<String, String>)> {
        std::mem::take(&mut self.queue)
    }
}

/// The weather is re-evaluated this often (s of session time): the hourly
/// data interpolated to the moment, eased over [`EVAL_TRANSITION_S`].
pub const EVAL_S: f64 = 15.0;
/// Each re-evaluation eases over this (s).
pub const EVAL_TRANSITION_S: f64 = 20.0;
/// The place and the requests are checked this often (s).
pub const FETCH_S: f64 = 1.0;
/// A clock step longer than this (ms; the time machine) applies the new
/// moment's weather at once.
pub const JUMP_MS: i64 = 30 * 60_000;

/// The session's real weather (present while it is on).
#[derive(Debug, Clone)]
pub struct RealSession {
    pub feed: RealFeed,
    pub info: RealInfo,
    /// The transition of the first application (s; 0: at once with its
    /// ground).
    first_transition_s: f64,
    applied: bool,
    simulating: bool,
    next_fetch_t: f64,
    next_eval_t: f64,
    dirty: bool,
    last: Option<(f64, i64)>,
    /// The clock jumped and the new moment's weather is not applied yet
    /// (its data may still be on the way): apply it at once.
    jump_pending: bool,
}

/// What the session should do with its weather after a step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Apply {
    /// Nothing new.
    Keep,
    /// Ease to this state over the seconds; `settle`: set at once with
    /// its ground.
    Set {
        state: ocw_weather::WeatherState,
        transition_s: f64,
        settle: bool,
    },
    /// No data at all and the providers do not answer: simulate meanwhile.
    Simulate,
}

impl RealSession {
    pub fn new(choice: Choice, transition_s: f64) -> Self {
        Self {
            feed: RealFeed::new(choice),
            info: RealInfo {
                choice,
                ..Default::default()
            },
            first_transition_s: transition_s.max(0.0),
            applied: false,
            simulating: false,
            next_fetch_t: f64::NEG_INFINITY,
            next_eval_t: f64::NEG_INFINITY,
            dirty: true,
            last: None,
            jump_pending: false,
        }
    }

    /// Another data source (the weather follows when its data arrives).
    pub fn set_choice(&mut self, choice: Choice, transition_s: f64) {
        self.feed.set_choice(choice);
        self.info.choice = choice;
        self.first_transition_s = transition_s.max(0.0);
        self.applied = false;
        self.dirty = true;
    }

    /// Whether the weather was ever set from the data.
    pub fn applied(&self) -> bool {
        self.applied
    }

    /// One step at session time `t`: answers in, the place (`place`, the
    /// host's), requests out, and – every [`EVAL_S`], on new data or a
    /// clock jump – what to do with the session weather. `at_ms` is the
    /// world clock's moment, `now_ms` the real instant.
    pub fn step(
        &mut self,
        t: f64,
        at_ms: i64,
        now_ms: i64,
        place: Option<ocw_weather::real::Place>,
        fetcher: &mut dyn WeatherFetcher,
    ) -> Apply {
        for (url, result) in fetcher.poll() {
            if let Err(e) = &result {
                tracing::warn!(%url, error = %e, "weather request failed");
            }
            self.feed.on_result(&url, result, now_ms);
            self.dirty = true;
        }
        // A clock jump (the time machine): apply the new moment at once.
        let jumped = self.last.is_some_and(|(t0, at0)| {
            let expected = at0 + ((t - t0) * 1000.0) as i64;
            (at_ms - expected).abs() > JUMP_MS
        });
        self.last = Some((t, at_ms));
        self.jump_pending |= jumped;
        if t >= self.next_fetch_t || jumped {
            self.next_fetch_t = t + FETCH_S;
            if let Some(p) = place {
                self.feed.set_place(p);
            }
            for f in self.feed.wants(at_ms, now_ms) {
                tracing::info!(provider = f.provider.name(), url = %f.url, "weather request");
                fetcher.start(&f);
            }
        }
        if !(self.dirty || jumped || t >= self.next_eval_t) {
            return Apply::Keep;
        }
        self.dirty = false;
        self.next_eval_t = t + EVAL_S;
        let choice = self.feed.choice();
        match self.feed.resolve(at_ms, now_ms) {
            Some(r) => {
                let first = !self.applied;
                let jumped = std::mem::take(&mut self.jump_pending);
                let transition_s = if first {
                    self.first_transition_s
                } else if jumped {
                    0.0
                } else {
                    EVAL_TRANSITION_S
                };
                self.applied = true;
                self.simulating = false;
                self.info = RealInfo {
                    choice,
                    providers: r.providers,
                    attribution: r.attribution,
                    note: r.note,
                    status: r.status,
                    data_ms: r.data_ms,
                    historic: r.historic,
                    station: r.station,
                };
                Apply::Set {
                    state: r.state,
                    transition_s,
                    settle: transition_s == 0.0 && (first || jumped),
                }
            }
            None => {
                let status = self.feed.status(at_ms, now_ms);
                let error = self.feed.last_error(at_ms, now_ms);
                let note = match (status, self.applied) {
                    (_, _) if self.feed.place().is_none() => "no place for the weather yet".into(),
                    (Status::Offline, false) => format!(
                        "real weather unavailable ({}): simulated weather meanwhile",
                        error.as_deref().unwrap_or("no answer")
                    ),
                    (Status::Offline, true) => format!(
                        "no data for this place and time ({}): the last weather",
                        error.as_deref().unwrap_or("no answer")
                    ),
                    _ => "fetching the weather ...".into(),
                };
                self.info = RealInfo {
                    choice,
                    providers: Vec::new(),
                    attribution: String::new(),
                    note: Some(note),
                    status,
                    data_ms: 0,
                    historic: false,
                    station: None,
                };
                if status == Status::Offline && !self.applied && !self.simulating {
                    self.simulating = true;
                    return Apply::Simulate;
                }
                Apply::Keep
            }
        }
    }
}

/// The HTTP side (native only): one worker thread per process.
#[cfg(not(target_arch = "wasm32"))]
pub mod http {
    use std::collections::BTreeMap;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use ocw_weather::real::cache::Cache;
    use ocw_weather::real::feed::Fetch;

    use super::WeatherFetcher;

    /// The User-Agent of the requests: the product, never personal data
    /// (MET Norway's terms ask for an identifying one; an operator can add
    /// a contact with `OCW_WEATHER_USER_AGENT`).
    pub fn user_agent() -> String {
        std::env::var("OCW_WEATHER_USER_AGENT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                concat!(
                    "OpenCyclingWorld/",
                    env!("CARGO_PKG_VERSION"),
                    " (open-source cycling simulation; game server weather service)"
                )
                .into()
            })
    }

    /// `OCW_WEATHER_OFFLINE=1`: no requests (every one fails at once).
    pub fn offline() -> bool {
        std::env::var("OCW_WEATHER_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
    }

    /// Answers kept per process.
    pub const CACHE_ENTRIES: usize = 256;
    /// The gap between two requests to the same host.
    pub const HOST_GAP: Duration = Duration::from_secs(1);
    /// The pause after "429 Too Many Requests" (without a Retry-After)
    /// or an unreachable host.
    pub const HOST_PAUSE: Duration = Duration::from_secs(600);

    struct Job {
        fetch: Fetch,
        reply: Sender<(String, Result<String, String>)>,
    }

    struct Worker {
        jobs: Sender<Job>,
    }

    fn now_ms() -> i64 {
        crate::unix_now_ms()
    }

    fn cache() -> &'static Mutex<Cache<String>> {
        static CACHE: OnceLock<Mutex<Cache<String>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(Cache::new(CACHE_ENTRIES)))
    }

    fn worker() -> &'static Mutex<Worker> {
        static WORKER: OnceLock<Mutex<Worker>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("ocw-weather-http".into())
                .spawn(move || run(&rx))
                .expect("weather thread");
            Mutex::new(Worker { jobs: tx })
        })
    }

    /// A body: UTF-8, else Latin-1 (MeteoSwiss's CSV files).
    pub fn decode(bytes: Vec<u8>) -> String {
        match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => e.into_bytes().iter().map(|&b| char::from(b)).collect(),
        }
    }

    fn run(rx: &Receiver<Job>) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(user_agent())
            .build()
            .into();
        let mut last: BTreeMap<&'static str, Instant> = BTreeMap::new();
        let mut paused: BTreeMap<&'static str, Instant> = BTreeMap::new();
        while let Ok(Job { fetch, reply }) = rx.recv() {
            let url = fetch.url.clone();
            let result = get(&agent, &fetch, &mut last, &mut paused);
            let _ = reply.send((url, result));
        }
    }

    fn get(
        agent: &ureq::Agent,
        fetch: &Fetch,
        last: &mut BTreeMap<&'static str, Instant>,
        paused: &mut BTreeMap<&'static str, Instant>,
    ) -> Result<String, String> {
        let name = fetch.provider.name();
        if offline() {
            return Err(format!("{name}: offline (OCW_WEATHER_OFFLINE)"));
        }
        if let Some(body) = cache()
            .lock()
            .ok()
            .and_then(|c| c.fresh(&fetch.url, now_ms()))
        {
            return Ok(body);
        }
        let host = fetch.provider.host();
        if let Some(until) = paused.get(host)
            && Instant::now() < *until
        {
            return Err(format!(
                "{name}: paused after too many requests or no answer"
            ));
        }
        if let Some(t) = last.get(host) {
            let wait = HOST_GAP.saturating_sub(t.elapsed());
            std::thread::sleep(wait);
        }
        last.insert(host, Instant::now());
        let resp = agent.get(&fetch.url).call().map_err(|e| {
            paused.insert(host, Instant::now() + HOST_PAUSE / 5);
            format!("{name}: {e}")
        })?;
        let status = resp.status().as_u16();
        if status == 429 || status == 503 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map_or(HOST_PAUSE, Duration::from_secs);
            paused.insert(host, Instant::now() + retry.min(HOST_PAUSE * 6));
        }
        if status != 200 {
            return Err(format!("{name}: HTTP {status}"));
        }
        let mut body = resp.into_body();
        let bytes = body
            .with_config()
            .limit(8 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("{name}: {e}"))?;
        let text = decode(bytes);
        if let Ok(mut c) = cache().lock() {
            c.insert(&fetch.url, text.clone(), now_ms(), fetch.kind.ttl_ms());
        }
        Ok(text)
    }

    /// A session's handle on the worker: requests go to the shared thread,
    /// answers come back on its own channel.
    pub struct HttpFetcher {
        tx: Sender<(String, Result<String, String>)>,
        rx: Receiver<(String, Result<String, String>)>,
    }

    impl Default for HttpFetcher {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self { tx, rx }
        }
    }

    impl WeatherFetcher for HttpFetcher {
        fn start(&mut self, fetch: &Fetch) {
            let job = Job {
                fetch: fetch.clone(),
                reply: self.tx.clone(),
            };
            let sent = worker().lock().map(|w| w.jobs.send(job).is_ok());
            if !matches!(sent, Ok(true)) {
                let _ = self
                    .tx
                    .send((fetch.url.clone(), Err("the weather thread ended".into())));
            }
        }

        fn poll(&mut self) -> Vec<(String, Result<String, String>)> {
            self.rx.try_iter().collect()
        }
    }

    /// Fetches `fetch` now, blocking (the live smoke test,
    /// `OCW_LIVE_WEATHER=1`).
    pub fn fetch_blocking(fetch: &Fetch, timeout: Duration) -> Result<String, String> {
        let mut f = HttpFetcher::default();
        f.start(fetch);
        f.rx.recv_timeout(timeout)
            .map_err(|e| format!("no answer: {e}"))?
            .1
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bodies_decode_as_utf8_or_latin1_and_the_agent_is_generic() {
            assert_eq!(decode("Zürich".as_bytes().to_vec()), "Zürich");
            assert_eq!(decode(vec![b'Z', 0xFC, b'r']), "Zür");
            let ua = user_agent();
            assert!(ua.starts_with("OpenCyclingWorld/"), "{ua}");
            assert!(!ua.contains('@'), "no personal data");
        }
    }
}
