//! The game server's side of the live road events (spec 14 Phase 18, 18D
//! *The* Live traffic data *switch*, 18E T13): who wants them, when the
//! feed steps, the records mapped onto the running traffic's road graph
//! and laid over it, and what the riders are told.
//!
//! - **Who decides (Q2, 18D):** the road traffic's own rule – solo the
//!   rider, in a shared ride the host (a guest's wish only gets them the
//!   events the host's choice brings), nobody during an organiser's event
//!   (no live data in races and events). It only matters while the road
//!   traffic is on.
//! - **Applied:** the feed's records in force, mapped onto the edges of
//!   the traffic's graph (`ocw_sim::traffic::live::map_events` with the
//!   statistics' rules), set on the traffic simulation
//!   (`TrafficSim::set_live`); none – the statistics – when the feed is
//!   down, stale, unwanted, or the world has no source.
//! - **Told:** each rider's `LiveRoadWish` is answered with the state now
//!   (the world's source and credit, or why there is none); the riders
//!   who want it get a new `LiveRoadUpdate` whenever the events or the
//!   state change.

use ocw_protocol::live_road::event_to_wire;
use ocw_protocol::{encode, server_message, v1};
use ocw_sim::traffic::live::{EdgeEvent, map_events};

use crate::live_road::{self, FeedState};
use crate::{ConnectionId, GameServer, SIM_TICK_HZ};

type S = v1::server_message::Msg;

/// The events in force on the running traffic.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RoadEvents {
    /// What they were made from: (graph hash, fetch time, the ids of the
    /// records in force).
    made_of: Option<(u64, i64, Vec<String>)>,
    /// Each edge event with its road's name and until when (Unix s).
    events: Vec<(EdgeEvent, String, i64)>,
    graph_hash: u64,
    /// Records of the box that matched no road of the package.
    pub(crate) unmapped: usize,
}

impl RoadEvents {
    fn edge_events(&self) -> Vec<EdgeEvent> {
        self.events.iter().map(|(e, ..)| *e).collect()
    }
}

impl GameServer {
    /// Lets this server fetch live road data (spec 18E T13): the HTTP
    /// fetcher (or a recorded feed) on the native servers, recorded answers
    /// in tests.
    pub fn set_road_fetcher(&mut self, fetcher: Box<dyn live_road::RoadFetcher>) {
        self.road_fetcher = Some(std::sync::Mutex::new(fetcher));
    }

    /// [`GameServer::set_road_fetcher`] as a builder.
    pub fn with_road_fetcher(mut self, fetcher: Box<dyn live_road::RoadFetcher>) -> Self {
        self.set_road_fetcher(fetcher);
        self
    }

    /// Uses `feed` for live road data (tests: another source).
    pub fn set_live_road_feed(&mut self, feed: live_road::LiveRoadFeed) {
        self.live_road = feed;
        self.road_events = RoadEvents::default();
    }

    /// The live road feed (the `traffic` command's server side, tests).
    pub fn live_road(&self) -> &live_road::LiveRoadFeed {
        &self.live_road
    }

    /// The live road events in force on the traffic now (edge events).
    pub fn live_road_events(&self) -> Vec<EdgeEvent> {
        self.road_events.edge_events()
    }

    /// Records of the last fetch that matched no road of the package.
    pub fn live_road_unmapped(&self) -> usize {
        self.road_events.unmapped
    }

    /// Whether the riders' wishes make the server fetch: a rider who
    /// decides the road traffic wants it, the road traffic is on, and no
    /// organiser's event runs.
    fn live_road_wanted(&self) -> bool {
        if self.event_traffic().is_some() || !self.road_traffic().level.is_on() {
            return false;
        }
        let host = self.host();
        let access = self.traffic_access();
        self.road_wish.iter().any(|c| {
            self.riders
                .get(c)
                .is_some_and(|r| access.allows(Some(*r), host))
        })
    }

    /// A rider's *Live traffic data* (spec 18E T13): kept per connection;
    /// every wish is answered with the state now.
    pub(crate) fn live_road_wish(&mut self, conn: ConnectionId, on: bool) {
        if on {
            self.road_wish.insert(conn);
        } else {
            self.road_wish.remove(&conn);
        }
        let u = self.live_road_update(on);
        self.directed
            .push((conn, encode(&server_message(S::LiveRoad(u)))));
    }

    /// What a rider is told: the world's source, its state and – for a
    /// rider who wants them while they are live – the events.
    pub(crate) fn live_road_update(&self, wants: bool) -> v1::LiveRoadUpdate {
        let feed = &self.live_road;
        let mut u = v1::LiveRoadUpdate {
            note: feed.note().to_owned(),
            graph_hash: self.road_events.graph_hash,
            ..Default::default()
        };
        let Some(source) = feed.source() else {
            u.status = v1::LiveRoadStatus::NoSource as i32;
            return u;
        };
        u.attribution = source.attribution().into();
        u.what = source.what().into();
        if self.road_fetcher.is_none() {
            u.status = v1::LiveRoadStatus::Unavailable as i32;
            u.note = "this server does not fetch live road data".into();
            return u;
        }
        let status = match feed.state() {
            FeedState::NoSource => v1::LiveRoadStatus::NoSource,
            FeedState::Live if wants => v1::LiveRoadStatus::Live,
            FeedState::Down if wants => v1::LiveRoadStatus::Down,
            _ => v1::LiveRoadStatus::Available,
        };
        u.status = status as i32;
        if status == v1::LiveRoadStatus::Available && wants {
            u.note = if self.event_traffic().is_some() {
                "not during events".into()
            } else if !self.road_traffic().level.is_on() {
                "turn on road traffic first".into()
            } else if !self.live_road_wanted() {
                "the host decides".into()
            } else {
                "fetching the first live data".into()
            };
        }
        if status == v1::LiveRoadStatus::Live {
            u.fetched_ms = feed.fetched_ms().unwrap_or_default();
            u.events = self
                .road_events
                .events
                .iter()
                .map(|(e, road, until)| event_to_wire(e, road, *until))
                .collect();
        }
        u
    }

    /// Once a second: the feed's step, the events laid over the traffic,
    /// and an update to every rider who wants them when anything changed.
    pub(crate) fn live_road_tick(&mut self, t: f64) {
        if !self.session.tick().is_multiple_of(u64::from(SIM_TICK_HZ)) {
            return;
        }
        if self.live_road.source().is_none() && self.road_events.events.is_empty() {
            // No source for this world: nothing to fetch or apply.
            if self.live_road.take_changed() {
                self.live_road_broadcast();
            }
            return;
        }
        let now = self.clock.real(t);
        let wanted = self.live_road_wanted();
        let fetcher = self
            .road_fetcher
            .as_mut()
            .and_then(|m| m.get_mut().ok())
            .map(|b| b.as_mut() as &mut dyn live_road::RoadFetcher);
        let mut changed = self.live_road.step(now, wanted, fetcher);
        changed |= self.apply_live_road(now);
        if changed {
            self.live_road_broadcast();
        }
    }

    fn live_road_broadcast(&mut self) {
        if self.road_wish.is_empty() {
            return;
        }
        let bytes = encode(&server_message(S::LiveRoad(self.live_road_update(true))));
        for c in &self.road_wish {
            self.directed.push((*c, bytes.clone()));
        }
    }

    /// Maps the feed's records in force onto the running traffic's graph
    /// and sets them on it; whether the events changed.
    pub(crate) fn apply_live_road(&mut self, now_ms: i64) -> bool {
        let Some(run) = self.vehicles.run.as_mut() else {
            // No traffic runs: nothing to lay them on.
            let had = !self.road_events.events.is_empty();
            self.road_events = RoadEvents::default();
            return had;
        };
        let records = self.live_road.records(now_ms);
        let hash = run.net.fingerprint();
        let made_of = (
            hash,
            self.live_road.fetched_ms().unwrap_or_default(),
            records.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        );
        let mut changed = false;
        if self.road_events.made_of.as_ref() != Some(&made_of) {
            let mut events = Vec::new();
            let mut unmapped = 0;
            if let Some(origin) = self.geo_origin
                && !records.is_empty()
            {
                let geo: Vec<_> = records.iter().map(|r| r.event.clone()).collect();
                let mapped = map_events(&run.net, run.sim.base_rules(), origin, &geo);
                for (r, m) in records.iter().zip(mapped) {
                    match m {
                        Ok(list) => {
                            let until = r.end_ms.map_or(0, |e| e / 1000);
                            events
                                .extend(list.into_iter().map(|e| (e, r.event.road.clone(), until)));
                        }
                        Err(_) => unmapped += 1,
                    }
                }
            }
            changed = events != self.road_events.events;
            if changed || unmapped != self.road_events.unmapped {
                tracing::info!(
                    records = records.len(),
                    edges = events.len(),
                    unmapped,
                    "live road events mapped onto the road graph"
                );
            }
            self.road_events = RoadEvents {
                made_of: Some(made_of),
                events,
                graph_hash: hash,
                unmapped,
            };
        }
        // A traffic started again (switched off and on) gets them too; the
        // simulation ignores the same set.
        let gone = run.sim.set_live(&self.road_events.edge_events());
        if gone > 0 {
            tracing::info!(vehicles = gone, "vehicles left a closed road");
        }
        changed
    }
}
