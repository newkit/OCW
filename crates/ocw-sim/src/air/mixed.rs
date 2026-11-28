//! Live and simulated aircraft in one sky (spec 14 Phase 18, 18C, 18J;
//! Q13 (c)): with *Aircraft: live* the sky shows the live aircraft of the
//! feed **plus** the simulated flights of the kinds ADS-B rarely shows –
//!
//! - **always:** gliders, balloons and small planes in circuits;
//! - **while the feed shows none of their kind within [`QUIET_M`] of the
//!   session's place:** the other small planes (cross-country) and the
//!   helicopters (rescue, tours);
//! - **never:** the simulated airliners (the live ones are the real
//!   picture).
//!
//! **The same mix for every rider** (18A, Q2). The simulated flights are
//! each client's own, from the session's air seed; which of them fly
//! beside the live aircraft is decided from what the server shares with
//! every rider who wants live aircraft:
//!
//! - the **[`Ledger`]** (in every `LiveAirUpdate`): for small planes and
//!   helicopters the times during which the feed showed one of the kind
//!   within 30 km of the place the server asks for. A simulated flight of
//!   such a kind flies if its **start** lies outside these times – so one
//!   that already flies when a live one turns up finishes its flight (no
//!   pop-out), and none starts while a live one is there or appears in
//!   mid-air after it left (no pop-in: flights begin on their field, pad
//!   or far out). The ledger holds kinds and times only – no position and
//!   nothing of an aircraft (18D *Privacy*) – and a rider who joins later
//!   gets it whole, so the decision is a function of the update alone.
//! - the **[`Separation`]** from the live aircraft of the update in force
//!   when a simulated flight starts: within [`SEPARATION_M`] and
//!   [`SEPARATION_ALT_M`] of where it starts, it does not fly. The server
//!   keeps no positions beyond its last fetch (18D), so this is decided
//!   by each client when the flight starts; riders who have live aircraft
//!   on then hold the same update and decide alike.
//!
//! Ids: a live aircraft's id is [`ID_BIT`] with the server's 32-bit
//! number; [`is_live`] tells them from the simulated flights' hashes (a
//! simulated flight whose hash happens to look like one is left out of
//! the mixed sky).
//!
//! Pure and wasm-safe; callers pass the instant.

use std::collections::HashMap;
use std::sync::Arc;

use ocw_world::geo::LocalProjection;

use super::live::{ID_BIT, LiveAircraft, distance_m, position};
use super::relief::Relief;
use super::upper::UpperAir;
use super::{AirInputs, AirKind, AirSky, AirSpace, AircraftNow, Flight, Heard, Mission};

/// The feed shows "none of their kind" when no live aircraft of the kind
/// is within this (m) of the place the server asks for (Q13 (c)).
pub const QUIET_M: f64 = 30_000.0;
/// A kind seen at a fetch counts as there for this long after it (ms), or
/// until the next fetch says otherwise: as long as a client extrapolates
/// a fix (`live::MAX_AGE_S`).
pub const HOLD_MS: i64 = 420_000;
/// Times that ended longer ago than this are forgotten (ms): no simulated
/// small plane or helicopter flies for longer.
pub const KEEP_MS: i64 = 2 * 3_600_000;
/// No simulated flight starts within this (m, horizontally) …
pub const SEPARATION_M: f64 = 5000.0;
/// … and this (m, vertically) of a live aircraft.
pub const SEPARATION_ALT_M: f64 = 600.0;
/// A client decides the separation of a flight it first sees within this
/// (s) of its start; one it first sees later was not near a live aircraft
/// it knew of.
pub const SPAWN_WINDOW_S: f64 = 90.0;

/// Whether `id` is a live aircraft's ([`ID_BIT`] and a 32-bit number).
pub fn is_live(id: u64) -> bool {
    id >> 32 == ID_BIT >> 32
}

/// When a simulated flight flies beside live aircraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Share {
    /// Kinds ADS-B rarely shows: always.
    Always,
    /// Only while the feed shows none of its kind near.
    Quiet,
    /// Not beside live aircraft.
    Never,
}

/// The rule of a kind and mission (Q13 (c)).
pub fn share(kind: AirKind, mission: Mission) -> Share {
    match (kind, mission) {
        (AirKind::Airliner, _) => Share::Never,
        (AirKind::Glider | AirKind::Balloon, _) => Share::Always,
        (AirKind::SmallPlane, Mission::Circuit) => Share::Always,
        (AirKind::SmallPlane | AirKind::Helicopter, _) => Share::Quiet,
    }
}

/// The kinds the ledger keeps times for.
pub const QUIET_KINDS: [AirKind; 2] = [AirKind::SmallPlane, AirKind::Helicopter];

/// A time during which the feed showed a kind near: `[from_ms, to_ms)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub kind: AirKind,
    pub from_ms: i64,
    pub to_ms: i64,
}

/// When the feed showed small planes and helicopters near the session's
/// place (see the module docs): kept by the server, sent whole with every
/// update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    busy: Vec<Busy>,
}

impl Ledger {
    /// A received ledger (times of other kinds and empty ones dropped).
    pub fn from_times(times: impl IntoIterator<Item = Busy>) -> Self {
        Self {
            busy: times
                .into_iter()
                .filter(|b| QUIET_KINDS.contains(&b.kind) && b.to_ms > b.from_ms)
                .collect(),
        }
    }

    pub fn times(&self) -> &[Busy] {
        &self.busy
    }

    /// A fetch at `now_ms`: whether a live aircraft of `kind` was near.
    pub fn note(&mut self, now_ms: i64, kind: AirKind, seen: bool) {
        if !QUIET_KINDS.contains(&kind) {
            return;
        }
        // The time still running (its hold not over), if any.
        let open = self
            .busy
            .iter_mut()
            .rev()
            .find(|b| b.kind == kind && b.from_ms <= now_ms && b.to_ms >= now_ms);
        match (open, seen) {
            (Some(b), true) => b.to_ms = now_ms + HOLD_MS,
            // Gone at this fetch: the time ends now, never earlier – what
            // clients decided before stays true.
            (Some(b), false) => b.to_ms = now_ms,
            (None, true) => self.busy.push(Busy {
                kind,
                from_ms: now_ms,
                to_ms: now_ms + HOLD_MS,
            }),
            (None, false) => {}
        }
    }

    /// A fetch at `now_ms` around `centre` (lat, lon) with these live
    /// aircraft (kind, lat, lon).
    pub fn fetch(
        &mut self,
        now_ms: i64,
        centre: (f64, f64),
        aircraft: impl IntoIterator<Item = (AirKind, f64, f64)>,
    ) {
        let mut seen = [false; QUIET_KINDS.len()];
        for (kind, lat, lon) in aircraft {
            if let Some(i) = QUIET_KINDS.iter().position(|k| *k == kind)
                && distance_m(centre, (lat, lon)) <= QUIET_M
            {
                seen[i] = true;
            }
        }
        for (kind, seen) in QUIET_KINDS.into_iter().zip(seen) {
            self.note(now_ms, kind, seen);
        }
        self.busy
            .retain(|b| b.to_ms > b.from_ms && b.to_ms > now_ms - KEEP_MS);
    }

    /// The feed is lost at `now_ms` (it failed, the last fetch grew too
    /// old): the clients draw the whole simulated sky, and what starts in
    /// it stays when the feed is back.
    pub fn lost(&mut self, now_ms: i64) {
        for b in &mut self.busy {
            if b.from_ms <= now_ms && b.to_ms > now_ms {
                b.to_ms = now_ms;
            }
        }
        self.busy.retain(|b| b.to_ms > b.from_ms);
    }

    /// Whether the feed showed `kind` near at `t_ms`.
    pub fn busy_at(&self, kind: AirKind, t_ms: i64) -> bool {
        self.busy
            .iter()
            .any(|b| b.kind == kind && (b.from_ms..b.to_ms).contains(&t_ms))
    }

    /// Whether simulated flight `f` flies beside the live aircraft (the
    /// rule of its kind; the separation is the client's [`Separation`]).
    pub fn admits(&self, f: &Flight) -> bool {
        if is_live(f.id) {
            return false;
        }
        match share(f.kind, f.mission) {
            Share::Always => true,
            Share::Never => false,
            Share::Quiet => !self.busy_at(f.kind, (f.start_s * 1000.0).floor() as i64),
        }
    }
}

/// A client's memory of which simulated flights started clear of the live
/// aircraft (see the module docs). Every flight drawn passes through
/// [`Separation::keeps`] – also while the sky is all simulated (no live
/// aircraft given), so that a flight already drawn is never taken away
/// when the live aircraft arrive.
#[derive(Debug, Clone, Default)]
pub struct Separation {
    /// By flight id: kept, and when the flight ends (s).
    decided: HashMap<u64, (bool, f64)>,
}

impl Separation {
    pub fn clear(&mut self) {
        self.decided.clear();
    }

    /// Whether `f` flies: decided once, when the client first sees it, from
    /// the `live` aircraft where they were when it started.
    pub fn keeps(
        &mut self,
        f: &Flight,
        live: &[LiveAircraft],
        proj: &LocalProjection,
        relief: &Relief,
        now_s: f64,
    ) -> bool {
        if let Some((kept, _)) = self.decided.get(&f.id) {
            return *kept;
        }
        if self.decided.len() > 2048 {
            self.decided.retain(|_, (_, end)| *end + 600.0 > now_s);
        }
        let kept = now_s - f.start_s > SPAWN_WINDOW_S || {
            let p = f.state(f.start_s).pos;
            let t_ms = (f.start_s * 1000.0).round() as i64;
            !live.iter().any(|a| {
                position(a, proj, relief, t_ms).is_some_and(|q| {
                    (q[0] - p[0]).hypot(q[2] - p[2]) < SEPARATION_M
                        && (q[1] - p[1]).abs() < SEPARATION_ALT_M
                })
            })
        };
        self.decided.insert(f.id, (kept, f.end_s()));
        kept
    }
}

impl AirSky {
    /// [`AirSky::flights`] with a filter.
    pub fn flights_where(
        &mut self,
        space: &AirSpace,
        inp: &AirInputs,
        mut keep: impl FnMut(&Flight) -> bool,
    ) -> Vec<Arc<Flight>> {
        let mut f = self.flights(space, inp);
        f.retain(|f| keep(f));
        f
    }
}

fn flat(p: [f64; 3], v: [f64; 3]) -> f64 {
    (p[0] - v[0]).hypot(p[2] - v[2])
}

/// The frame's aircraft from the `live` ones (already within the radius)
/// and the simulated `flights`: the nearest within the budget, whichever
/// they are.
pub fn now(live: Vec<AircraftNow>, flights: &[Arc<Flight>], inp: &AirInputs) -> Vec<AircraftNow> {
    enum One {
        Live(AircraftNow),
        Sim(Arc<Flight>),
    }
    let t = inp.now_s;
    let v = inp.viewer;
    let mut all: Vec<(f64, u64, One)> = live
        .into_iter()
        .map(|a| (flat(a.pos, v), a.id, One::Live(a)))
        .chain(
            flights
                .iter()
                .map(|f| (flat(f.state(t).pos, v), f.id, One::Sim(f.clone())))
                .filter(|(d, ..)| *d <= inp.radius_m),
        )
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    all.truncate(inp.max_aircraft);
    let upper = UpperAir::from_weather(inp.weather, inp.ground_m);
    all.into_iter()
        .map(|(_, _, a)| match a {
            One::Live(a) => a,
            One::Sim(f) => super::aircraft_now(&f, t, &upper, inp.contrails),
        })
        .collect()
}

/// What is heard at `listener`: the `live` aircraft's sounds and the
/// simulated `flights`', the nearest first.
pub fn heard(
    live: Vec<Heard>,
    flights: &[Arc<Flight>],
    now_s: f64,
    listener: [f64; 3],
) -> Vec<Heard> {
    let mut out = live;
    out.extend(
        flights
            .iter()
            .filter_map(|f| super::heard(f, now_s, listener)),
    );
    out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    out
}

/// How many aircraft of each kind (in the order of [`AirKind::ALL`]).
pub type Counts = [usize; AirKind::ALL.len()];

pub fn count(kinds: impl IntoIterator<Item = AirKind>) -> Counts {
    let mut c = Counts::default();
    for k in kinds {
        if let Some(i) = AirKind::ALL.iter().position(|a| *a == k) {
            c[i] += 1;
        }
    }
    c
}

/// "12 airliners, 1 helicopter" (`none` without any).
pub fn describe(c: &Counts) -> String {
    let parts: Vec<String> = AirKind::ALL
        .iter()
        .zip(c)
        .filter(|(_, n)| **n > 0)
        .map(|(k, n)| format!("{n} {}{}", k.key(), if *n == 1 { "" } else { "s" }))
        .collect();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::super::path::{PathBuilder, Waypoint};
    use super::*;

    const T0: i64 = 1_784_120_400_000; // 2026-07-15 13:00 UTC
    const CENTRE: (f64, f64) = (48.0, 8.0);

    /// A point `km` north of the centre.
    fn north(km: f64) -> (f64, f64) {
        let (lat, lon) = super::super::live::destination(CENTRE.0, CENTRE.1, 0.0, km * 1000.0);
        (lat, lon)
    }

    fn flight(id: u64, kind: AirKind, mission: Mission, start_ms: i64) -> Flight {
        let mut b = PathBuilder::new(2000.0);
        b.to(Waypoint::air([0.0, 900.0, 0.0], 50.0));
        b.to(Waypoint::air([20_000.0, 900.0, 0.0], 50.0));
        Flight {
            id,
            kind,
            mission,
            start_s: start_ms as f64 / 1000.0,
            path: b.build(None, 0.0, 0.05),
            look: 0,
            runway: None,
            field_m: 300.0,
        }
    }

    #[test]
    fn the_rule_per_kind_and_mission() {
        use AirKind::*;
        assert_eq!(share(Glider, Mission::Soaring), Share::Always);
        assert_eq!(share(Balloon, Mission::Drift), Share::Always);
        assert_eq!(share(SmallPlane, Mission::Circuit), Share::Always);
        assert_eq!(share(SmallPlane, Mission::CrossCountry), Share::Quiet);
        assert_eq!(share(Helicopter, Mission::Rescue), Share::Quiet);
        assert_eq!(share(Helicopter, Mission::Tour), Share::Quiet);
        for m in [Mission::Cruise, Mission::Arrival, Mission::Departure] {
            assert_eq!(share(Airliner, m), Share::Never);
        }
    }

    /// 29 km: the kind is there; 31 km: it is not. Other kinds near do
    /// not count for it.
    #[test]
    fn a_live_aircraft_counts_within_30_km() {
        let (near, far) = (north(29.0), north(31.0));
        let mut l = Ledger::default();
        l.fetch(T0, CENTRE, [(AirKind::Helicopter, far.0, far.1)]);
        assert!(!l.busy_at(AirKind::Helicopter, T0));
        let heli = flight(1, AirKind::Helicopter, Mission::Tour, T0 + 10_000);
        let cross = flight(2, AirKind::SmallPlane, Mission::CrossCountry, T0 + 10_000);
        assert!(l.admits(&heli) && l.admits(&cross));
        let mut l = Ledger::default();
        l.fetch(
            T0,
            CENTRE,
            [
                (AirKind::Helicopter, near.0, near.1),
                (AirKind::Airliner, CENTRE.0, CENTRE.1),
                (AirKind::Glider, CENTRE.0, CENTRE.1),
            ],
        );
        assert!(l.busy_at(AirKind::Helicopter, T0));
        assert!(!l.admits(&heli), "a live helicopter 29 km away");
        assert!(l.admits(&cross), "no live small plane: simulated ones fly");
        // The kinds always there, and the airliners never.
        let mut l = Ledger::default();
        l.fetch(
            T0,
            CENTRE,
            [
                (AirKind::SmallPlane, near.0, near.1),
                (AirKind::Helicopter, near.0, near.1),
            ],
        );
        let t = T0 + 10_000;
        assert!(l.admits(&flight(3, AirKind::Glider, Mission::Soaring, t)));
        assert!(l.admits(&flight(4, AirKind::Balloon, Mission::Drift, t)));
        assert!(l.admits(&flight(5, AirKind::SmallPlane, Mission::Circuit, t)));
        assert!(!l.admits(&cross) && !l.admits(&heli));
        assert!(!l.admits(&flight(6, AirKind::Airliner, Mission::Cruise, t)));
        assert!(
            !Ledger::default().admits(&flight(6, AirKind::Airliner, Mission::Cruise, t)),
            "also with nothing live near"
        );
    }

    /// A simulated flight already flying when a live one of its kind
    /// turns up finishes its flight; none starts while it is there; one
    /// that would have started then does not appear after it left; the
    /// next one after it left flies.
    #[test]
    fn hysteresis_by_the_start_of_the_flight() {
        let near = north(12.0);
        let heli = |id, start| flight(id, AirKind::Helicopter, Mission::Rescue, start);
        let before = heli(1, T0 - 300_000);
        let during = heli(2, T0 + 90_000);
        let after = heli(3, T0 + 200_000);
        let mut l = Ledger::default();
        l.fetch(T0 - 60_000, CENTRE, []);
        assert!(l.admits(&before) && l.admits(&during));
        // Seen at T0 and a minute later, gone at T0 + 120 s.
        l.fetch(T0, CENTRE, [(AirKind::Helicopter, near.0, near.1)]);
        l.fetch(T0 + 60_000, CENTRE, [(AirKind::Helicopter, near.0, near.1)]);
        assert!(l.admits(&before), "already flying: it finishes its flight");
        assert!(!l.admits(&during), "none starts while a live one is there");
        assert!(
            !l.admits(&after),
            "held until the next fetch says otherwise"
        );
        l.fetch(T0 + 120_000, CENTRE, []);
        assert!(l.admits(&before));
        assert!(!l.admits(&during), "no pop-in in mid-air after it left");
        assert!(l.admits(&after), "the next one flies");
        assert_eq!(
            l.times(),
            [Busy {
                kind: AirKind::Helicopter,
                from_ms: T0,
                to_ms: T0 + 120_000
            }]
        );
        // A later fetch never changes what was decided.
        l.fetch(
            T0 + 240_000,
            CENTRE,
            [(AirKind::Helicopter, near.0, near.1)],
        );
        assert!(l.admits(&after) && !l.admits(&during) && l.admits(&before));
        assert_eq!(l.times().len(), 2);
        // No fetch for long (the budget): the hold ends by itself.
        assert!(l.busy_at(AirKind::Helicopter, T0 + 240_000 + HOLD_MS - 1));
        assert!(!l.busy_at(AirKind::Helicopter, T0 + 240_000 + HOLD_MS));
        // The feed lost: what starts in the simulated sky stays.
        l.lost(T0 + 260_000);
        assert!(!l.busy_at(AirKind::Helicopter, T0 + 260_000));
        assert!(l.busy_at(AirKind::Helicopter, T0 + 259_999));
        // Old times are forgotten.
        l.fetch(T0 + KEEP_MS + 300_000, CENTRE, []);
        assert!(l.times().is_empty());
    }

    /// A rider who joins later gets the ledger whole and decides alike.
    #[test]
    fn the_ledger_alone_decides() {
        let near = north(5.0);
        let mut l = Ledger::default();
        l.fetch(T0, CENTRE, [(AirKind::SmallPlane, near.0, near.1)]);
        l.fetch(T0 + 60_000, CENTRE, []);
        let late = Ledger::from_times(l.times().iter().copied());
        assert_eq!(late, l);
        for (i, start) in (T0 - 120_000..T0 + 240_000).step_by(7000).enumerate() {
            let f = flight(i as u64, AirKind::SmallPlane, Mission::CrossCountry, start);
            assert_eq!(l.admits(&f), late.admits(&f));
            assert_eq!(l.admits(&f), !(T0..T0 + 60_000).contains(&start));
        }
        // Rubbish from the wire is dropped.
        let odd = Ledger::from_times([
            Busy {
                kind: AirKind::Airliner,
                from_ms: 0,
                to_ms: 10,
            },
            Busy {
                kind: AirKind::Helicopter,
                from_ms: 10,
                to_ms: 10,
            },
        ]);
        assert!(odd.times().is_empty());
    }

    fn live(id: u32, x_m: f64, alt_m: f64, at_ms: i64) -> LiveAircraft {
        let (lat, lon) = super::super::live::destination(CENTRE.0, CENTRE.1, 90.0, x_m);
        LiveAircraft {
            id,
            kind: AirKind::SmallPlane,
            lat,
            lon,
            alt_m,
            speed_mps: 0.0,
            track_deg: 0.0,
            climb_mps: 0.0,
            at_ms,
        }
    }

    /// No simulated flight starts within 5 km and 600 m of a live
    /// aircraft; farther or in another height band it does; a flight
    /// already seen stays; the decision is kept.
    #[test]
    fn simulated_flights_start_clear_of_live_aircraft() {
        let proj = LocalProjection::new(CENTRE.0, CENTRE.1);
        let relief = Relief::default();
        let now = T0 as f64 / 1000.0 + 1.0;
        // The flight starts at (0, 900, 0).
        let f = |id| flight(id, AirKind::Glider, Mission::Soaring, T0);
        let mut s = Separation::default();
        assert!(!s.keeps(&f(1), &[live(1, 4000.0, 1200.0, T0)], &proj, &relief, now));
        assert!(
            !s.keeps(&f(1), &[], &proj, &relief, now + 600.0),
            "decided once"
        );
        assert!(s.keeps(&f(2), &[live(1, 6000.0, 1200.0, T0)], &proj, &relief, now));
        assert!(
            s.keeps(&f(2), &[live(1, 100.0, 900.0, T0)], &proj, &relief, now),
            "already flying: it stays"
        );
        assert!(
            s.keeps(&f(3), &[live(1, 4000.0, 1600.0, T0)], &proj, &relief, now),
            "700 m above: another height band"
        );
        // Drawn in the simulated sky first: never taken away.
        assert!(s.keeps(&f(4), &[], &proj, &relief, now));
        assert!(s.keeps(&f(4), &[live(1, 0.0, 900.0, T0)], &proj, &relief, now));
        // First seen long after its start: it flies.
        assert!(s.keeps(
            &f(5),
            &[live(1, 0.0, 900.0, T0)],
            &proj,
            &relief,
            now + SPAWN_WINDOW_S + 1.0
        ));
        // Two clients with the same live aircraft decide alike, whenever
        // in the window they look.
        let set = [live(1, 3000.0, 700.0, T0 - 20_000)];
        let (mut a, mut b) = (Separation::default(), Separation::default());
        assert_eq!(
            a.keeps(&f(6), &set, &proj, &relief, now),
            b.keeps(&f(6), &set, &proj, &relief, now + 40.0)
        );
    }

    #[test]
    fn ids_stay_apart() {
        assert!(is_live(ID_BIT | 7));
        assert!(is_live(ID_BIT | u64::from(u32::MAX)));
        assert!(!is_live(7));
        assert!(!is_live(ID_BIT | (1 << 40)), "a hash, not a session number");
        // A simulated flight whose hash looks live is left out.
        let f = flight(ID_BIT | 9, AirKind::Glider, Mission::Soaring, T0);
        assert!(!Ledger::default().admits(&f));
    }

    #[test]
    fn counts_read_well() {
        use AirKind::*;
        let c = count([Airliner, Glider, Airliner, Balloon, SmallPlane, Glider]);
        assert_eq!(
            describe(&c),
            "2 airliners, 1 small plane, 2 gliders, 1 balloon"
        );
        assert_eq!(describe(&count([])), "none");
    }
}
