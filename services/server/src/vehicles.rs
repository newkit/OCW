//! The road vehicles of a session on the game server (spec 14 Phase 18,
//! 18A, 18E T5): the server runs `ocw_sim::traffic::TrafficSim` at the
//! physics rate (50 Hz) while the session's road traffic is on and the
//! ride is on a world's road graph, and sends each rider the vehicles
//! near them (Q3 (a): plans, then only changes).
//!
//! - **The simulation** starts when the road traffic in force
//!   ([`GameServer::road_traffic`]: the session's, or a running event's)
//!   is on, with its level and manners; it is dropped when the traffic
//!   goes off (no vehicles, no cost) or the world changes (a new one on
//!   the new graph). The rules are the package's (`TRFC`, else implied:
//!   `ocw_sim::traffic::package`), the demand's hour is the local time of
//!   the place from the world clock (`ocw_sky`), the bubbles are 1.5 km
//!   around every connected rider, and every rider on the road is a
//!   `RoadUser` the vehicles must not drive through.
//! - **Interest management:** each connection has its set of vehicles.
//!   At every broadcast (10 Hz) it gets one `TrafficUpdate` with the plans
//!   of vehicles that came within [`INTEREST_M`] of its rider (as they
//!   drive now: `TrafficSim::current_plans`), the changes of the vehicles
//!   it has, and despawns for those that left the simulation or went
//!   beyond [`INTEREST_M`] + [`INTEREST_HYSTERESIS_M`]; nothing when
//!   nothing changed, and a bare update about once a second for the
//!   client's traffic clock. Its first update (joining, reconnecting, a
//!   world switch) has `reset` and the current plans. A rider whose road
//!   traffic went off gets one `reset` with nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ocw_core::RiderId;
use ocw_core::traffic::{DriverManners as CoreManners, RoadTraffic};
use ocw_protocol::v1;
use ocw_protocol::vehicles::{change_to_wire, spawn_to_wire};
use ocw_protocol::{encode, server_message};
use ocw_sim::traffic::{
    Bubble, DemandTime, DriverManners, Gust, Plan, Region, RiderEvent, RoadUser, TrafficConfig,
    TrafficEvent, TrafficLevel, TrafficSim, world,
};
use ocw_world::RoadNetwork;

use crate::{ConnectionId, GameServer};

/// A rider gets the vehicles within this distance (m; spec 18A: 1.5 km).
pub const INTEREST_M: f64 = 1500.0;
/// … and keeps them until they are this much farther (m).
pub const INTEREST_HYSTERESIS_M: f64 = 100.0;
/// The simulated area around each rider (m; spec 18A).
pub const BUBBLE_M: f64 = 1500.0;
/// A bare update at least this often while a rider has vehicles (ticks of
/// the traffic: 1 s), for the client's traffic clock.
const HEARTBEAT_TICKS: u64 = 50;

/// The sim's level of the session's road traffic.
pub fn level_of(l: RoadTraffic) -> TrafficLevel {
    match l {
        RoadTraffic::Off => TrafficLevel::Off,
        RoadTraffic::Light => TrafficLevel::Light,
        RoadTraffic::Normal => TrafficLevel::Normal,
        RoadTraffic::Busy => TrafficLevel::Busy,
        RoadTraffic::Lunatic => TrafficLevel::Lunatic,
    }
}

/// The sim's manners of the session's (Q8).
pub fn manners_of(m: CoreManners) -> DriverManners {
    match m {
        CoreManners::Polite => DriverManners::Polite,
        CoreManners::MostlyPolite => DriverManners::Honking,
        CoreManners::SomeRealism => DriverManners::Realistic,
    }
}

/// What one connection has.
#[derive(Debug, Clone, Default)]
struct Interest {
    known: BTreeSet<u32>,
    /// The next update forgets everything first.
    reset: bool,
    /// The traffic tick of its last update.
    last_tick: Option<u64>,
}

/// The vehicles' encounters with riders (T4's events), counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficRiderStats {
    pub passes: u64,
    pub contacts: u64,
    pub honks: u64,
    pub approaching: u64,
    /// Rider-ticks with a vehicle's air on them.
    pub gusts: u64,
}

/// Bytes and messages sent, for the tests and `ocw-server`'s log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficSent {
    pub messages: u64,
    pub bytes: u64,
    pub spawns: u64,
    pub changes: u64,
    pub despawns: u64,
}

/// The running road traffic of a session.
pub(crate) struct RoadRun {
    sim: TrafficSim,
    net: Arc<RoadNetwork>,
    region: Region,
    zone: Option<ocw_sky::Zone>,
    /// The demand's time, recomputed once a second.
    time: Option<(u64, DemandTime)>,
    /// The events since the last broadcast.
    pending: Vec<TrafficEvent>,
}

/// The server's road vehicles: the run while it is on, and what each
/// connection has.
#[derive(Default)]
pub(crate) struct ServerVehicles {
    run: Option<RoadRun>,
    /// A route course's road as a network (no road graph: the test
    /// routes, route files), made once per route.
    route_net: Option<(u64, Arc<RoadNetwork>)>,
    interest: BTreeMap<ConnectionId, Interest>,
    sent: BTreeMap<ConnectionId, TrafficSent>,
    rider_stats: TrafficRiderStats,
    /// Each rider's encounters since their last update (spec 18E T7: the
    /// client's honks, pass-by whooshes and proximity cue).
    cues: BTreeMap<RiderId, Vec<v1::TrafficCue>>,
}

/// At most this many cues wait for a rider's next update.
const MAX_CUES: usize = 16;

impl ServerVehicles {
    fn cue(&mut self, rider: u32, vehicle: u32, kind: v1::TrafficCueKind, seconds: f32) {
        let q = self.cues.entry(RiderId(rider)).or_default();
        if q.len() < MAX_CUES {
            q.push(v1::TrafficCue {
                vehicle,
                kind: kind as i32,
                seconds,
            });
        }
    }

    /// A world switch: the vehicles of the old graph are gone; every
    /// connection starts again with a reset.
    pub(crate) fn world_changed(&mut self) {
        self.run = None;
        for i in self.interest.values_mut() {
            *i = Interest {
                reset: true,
                ..Default::default()
            };
        }
    }

    pub(crate) fn disconnect(&mut self, conn: ConnectionId) {
        self.interest.remove(&conn);
    }
}

impl GameServer {
    /// The road vehicles simulated now (0 while the road traffic is off).
    pub fn vehicle_count(&self) -> usize {
        self.vehicles.run.as_ref().map_or(0, |r| r.sim.len())
    }

    /// The traffic simulation, while it runs (tests and tools).
    pub fn traffic_sim(&self) -> Option<&TrafficSim> {
        self.vehicles.run.as_ref().map(|r| &r.sim)
    }

    /// What connection `conn` was sent of the road traffic.
    pub fn traffic_sent(&self, conn: ConnectionId) -> TrafficSent {
        self.vehicles.sent.get(&conn).copied().unwrap_or_default()
    }

    /// The vehicles connection `conn` has now (ids).
    pub fn traffic_known(&self, conn: ConnectionId) -> Vec<u32> {
        self.vehicles
            .interest
            .get(&conn)
            .map(|i| i.known.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Where rider `id` is (x, z) and as a road user (on an edge of the
    /// session's graph; `None` off the road or in a U-turn loop).
    fn rider_on_road(&self, id: RiderId) -> Option<([f64; 2], Option<RoadUser>)> {
        let s = self.session.rider_state(id)?;
        let p = self.session.track(id).sample(s.distance).position;
        let user = if self.session.off_road(id).is_some() {
            None
        } else if self.session.network().is_none() {
            // A route course: its road ([`world::route_network`]).
            let (_, net) = self.vehicles.route_net.as_ref()?;
            let (edge, along) = world::on_route_network(net, self.session.route(), s.distance);
            Some(RoadUser {
                id: id.0,
                edge,
                s: along,
                forward: true,
                speed: s.speed,
                lateral: s.lateral_offset,
                ..Default::default()
            })
        } else {
            self.session
                .path(id)
                .and_then(|path| path.locate(s.distance))
                .map(|(edge, forward, along)| RoadUser {
                    id: id.0,
                    edge,
                    s: along,
                    forward,
                    speed: s.speed,
                    lateral: s.lateral_offset,
                    ..Default::default()
                })
        };
        Some(([p[0], p[2]], user))
    }

    /// Starts, stops or steps the road traffic (every physics tick, after
    /// the riders' step), and at broadcasts queues each connection's
    /// [`v1::TrafficUpdate`] as a directed message.
    pub(crate) fn vehicles_tick(&mut self) {
        let rules = self.road_traffic();
        let on = rules.level.is_on();
        let net = if !on {
            None
        } else if let Some(n) = self.session.network() {
            Some(n.clone())
        } else {
            // A route course: the traffic drives the route's own road.
            let route = self.session.route();
            let key = ocw_world::graph::fnv1a(route.id().as_bytes())
                ^ route.length().to_bits()
                ^ (route.points().len() as u64).rotate_left(32);
            if self
                .vehicles
                .route_net
                .as_ref()
                .is_none_or(|(k, _)| *k != key)
            {
                self.vehicles.route_net = Some((key, Arc::new(world::route_network(route))));
            }
            self.vehicles.route_net.as_ref().map(|(_, n)| n.clone())
        };
        let on = on && net.is_some();
        if !on {
            if self.vehicles.run.take().is_some() {
                tracing::info!("road traffic off: the vehicles are gone");
            }
            self.vehicles_off_updates();
            return;
        }
        let net = net.expect("checked");
        if self
            .vehicles
            .run
            .as_ref()
            .is_some_and(|r| r.net.fingerprint() != net.fingerprint())
        {
            self.vehicles.world_changed();
        }
        if self.vehicles.run.is_none() {
            let rules_src = world::WorldRules::new(&net);
            let region = world::region_of(rules_src.country());
            tracing::info!(
                level = %rules.summary(),
                trfc = rules_src.from_data(),
                signals = rules_src.signalled(),
                controls = rules_src.controls(),
                "road traffic on (spec 18E T5)"
            );
            let cfg = TrafficConfig {
                seed: self.traffic.air_seed ^ 0x0007_2AFF_1C00_5EED,
                level: level_of(rules.level),
                manners: manners_of(rules.manners),
                ..Default::default()
            };
            let place = self.geo_origin.unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
            let zone = ocw_sky::Zone::get(&ocw_sky::zone::zone_for(place.0, place.1)).ok();
            self.vehicles.run = Some(RoadRun {
                sim: TrafficSim::new(&net, Box::new(rules_src), cfg),
                net: net.clone(),
                region,
                zone,
                time: None,
                pending: Vec::new(),
            });
        }
        // The riders: bubbles around the connected ones, every rider on
        // the road as a road user.
        let mut bubbles = Vec::new();
        let mut users = Vec::new();
        let humans: BTreeSet<RiderId> = self.riders.values().copied().collect();
        let mut centres: BTreeMap<RiderId, [f64; 2]> = BTreeMap::new();
        for r in self.session.snapshot() {
            let Some((xz, user)) = self.rider_on_road(r.id) else {
                continue;
            };
            if humans.contains(&r.id) {
                bubbles.push(Bubble {
                    center: xz,
                    radius: BUBBLE_M,
                });
                centres.insert(r.id, xz);
            }
            users.extend(user);
        }
        let t = self.session_time_s();
        let now_ms = self.clock.now(t);
        let run = self.vehicles.run.as_mut().expect("started above");
        let (level, manners) = (level_of(rules.level), manners_of(rules.manners));
        if run.sim.config().level != level {
            run.sim.set_level(level);
        }
        if run.sim.config().manners != manners {
            run.sim.set_manners(manners);
        }
        let tick = run.sim.tick();
        let time = match run.time {
            Some((at, d)) if tick < at + 50 => d.plus_seconds(((tick - at) / 50) as u32),
            _ => {
                let d = match &run.zone {
                    Some(z) => DemandTime::from_local(&z.local(now_ms), run.region),
                    None => DemandTime::weekday_at(12.0),
                };
                run.time = Some((tick, d));
                d
            }
        };
        // Vehicles and riders touch only with collisions on (12A, Q4).
        let collisions = self.session.collision_mode() != ocw_sim::collide::Mode::Off;
        if run.sim.config().collisions != collisions {
            run.sim.set_collisions(collisions);
        }
        run.sim.step(&net, &bubbles, &users, &time);
        let events = run.sim.drain_events();
        run.pending.extend(events);
        let rider_events = run.sim.drain_rider_events();
        let gusts = run.sim.gusts().to_vec();
        self.vehicle_rider_events(rider_events, &gusts);
        if self
            .session
            .tick()
            .is_multiple_of(u64::from(ocw_core::TICKS_PER_BROADCAST))
        {
            self.vehicles_updates(&centres);
        }
    }

    /// What the vehicles did to the riders this tick (T4's events): a
    /// contact is 12B's response with the collision rules in force (a
    /// crash or a bump, the damage by 12E); a lorry's air a wobble; passes,
    /// honks and vehicles closing in from behind are counted
    /// ([`GameServer::traffic_rider_stats`]) for the client's sound and HUD
    /// (T7).
    fn vehicle_rider_events(&mut self, events: Vec<RiderEvent>, gusts: &[Gust]) {
        for ev in events {
            let st = &mut self.vehicles.rider_stats;
            match ev {
                RiderEvent::Contact(c) => {
                    st.contacts += 1;
                    if self.session.vehicle_contact(
                        RiderId(c.rider),
                        c.closing_mps,
                        c.relative,
                        c.push_side,
                    ) {
                        tracing::info!(
                            rider = c.rider,
                            vehicle = c.vehicle,
                            class = c.class.name(),
                            closing = c.closing_mps,
                            "a vehicle touched a rider"
                        );
                    }
                }
                RiderEvent::Passed(p) => {
                    st.passes += 1;
                    self.vehicles
                        .cue(p.rider, p.vehicle, v1::TrafficCueKind::Passed, 0.0);
                }
                RiderEvent::Honk { vehicle, rider, .. } => {
                    st.honks += 1;
                    self.vehicles
                        .cue(rider, vehicle, v1::TrafficCueKind::Honk, 0.0);
                }
                RiderEvent::Approaching {
                    vehicle,
                    rider,
                    seconds,
                    ..
                } => {
                    st.approaching += 1;
                    self.vehicles.cue(
                        rider,
                        vehicle,
                        v1::TrafficCueKind::Approaching,
                        seconds as f32,
                    );
                }
            }
        }
        for g in gusts {
            self.vehicles.rider_stats.gusts += 1;
            self.session.vehicle_gust(RiderId(g.rider), g);
        }
    }

    /// What the vehicles did to the riders so far (counts).
    pub fn traffic_rider_stats(&self) -> TrafficRiderStats {
        self.vehicles.rider_stats
    }

    /// One reset to every connection that still has vehicles (the road
    /// traffic went off or the ride left the road graph).
    fn vehicles_off_updates(&mut self) {
        let conns: Vec<ConnectionId> = self
            .vehicles
            .interest
            .iter()
            .filter(|(_, i)| !i.known.is_empty())
            .map(|(c, _)| *c)
            .collect();
        for conn in conns {
            let u = v1::TrafficUpdate {
                reset: true,
                step_ms: 20,
                ..Default::default()
            };
            self.vehicles.interest.insert(conn, Interest::default());
            self.send_traffic(conn, u);
        }
    }

    fn send_traffic(&mut self, conn: ConnectionId, u: v1::TrafficUpdate) {
        let sent = self.vehicles.sent.entry(conn).or_default();
        sent.spawns += u.spawns.len() as u64;
        sent.changes += u.changes.len() as u64;
        sent.despawns += u.despawns.len() as u64;
        let bytes = encode(&server_message(v1::server_message::Msg::Traffic(u)));
        sent.messages += 1;
        sent.bytes += bytes.len() as u64;
        self.directed.push((conn, bytes));
    }

    /// Each connection's update at a broadcast (interest management).
    fn vehicles_updates(&mut self, centres: &BTreeMap<RiderId, [f64; 2]>) {
        let Some(run) = self.vehicles.run.as_mut() else {
            return;
        };
        let pending = std::mem::take(&mut run.pending);
        let tick = run.sim.tick();
        let step_ms = run.sim.config().step_ms;
        let graph_hash = run.net.fingerprint();
        let server_tick = self.session.tick();
        let positions: BTreeMap<u32, [f64; 2]> = run
            .sim
            .views(&run.net)
            .iter()
            .map(|v| (v.id, v.xz))
            .collect();
        let mut plans: Option<BTreeMap<u32, Plan>> = None;
        let conns: Vec<(ConnectionId, RiderId)> =
            self.riders.iter().map(|(c, r)| (*c, *r)).collect();
        self.vehicles
            .interest
            .retain(|c, _| self.riders.contains_key(c));
        let riding: BTreeSet<RiderId> = self.riders.values().copied().collect();
        self.vehicles.cues.retain(|r, _| riding.contains(r));
        for (conn, rider) in conns {
            let Some(&centre) = centres.get(&rider) else {
                continue;
            };
            let run = self.vehicles.run.as_ref().expect("checked");
            let interest = self.vehicles.interest.entry(conn).or_insert(Interest {
                reset: true,
                ..Default::default()
            });
            let dist = |id: &u32| {
                positions.get(id).map(|p| {
                    let (dx, dz) = (p[0] - centre[0], p[1] - centre[1]);
                    (dx * dx + dz * dz).sqrt()
                })
            };
            let mut u = v1::TrafficUpdate {
                tick,
                reset: interest.reset,
                step_ms,
                graph_hash,
                server_tick,
                ..Default::default()
            };
            if interest.reset {
                interest.known.clear();
            }
            // Gone or out of range.
            let leave = INTEREST_M + INTEREST_HYSTERESIS_M;
            let gone: Vec<u32> = interest
                .known
                .iter()
                .copied()
                .filter(|id| dist(id).is_none_or(|d| d > leave))
                .collect();
            for id in &gone {
                interest.known.remove(id);
            }
            u.despawns = gone;
            u.cues = self.vehicles.cues.remove(&rider).unwrap_or_default();
            // The changes of the vehicles it keeps, in order.
            for ev in &pending {
                if let TrafficEvent::Change(c) = ev
                    && interest.known.contains(&c.id)
                {
                    u.changes.push(change_to_wire(c, tick));
                }
            }
            // New in range: their plans now.
            let new: Vec<u32> = positions
                .keys()
                .copied()
                .filter(|id| {
                    !interest.known.contains(id) && dist(id).is_some_and(|d| d <= INTEREST_M)
                })
                .collect();
            if !new.is_empty() {
                let plans = plans.get_or_insert_with(|| {
                    run.sim
                        .current_plans()
                        .into_iter()
                        .map(|p| (p.id, p))
                        .collect()
                });
                for id in new {
                    if let Some(p) = plans.get(&id) {
                        u.spawns.push(spawn_to_wire(p, tick));
                        interest.known.insert(id);
                    }
                }
            }
            let quiet = u.spawns.is_empty()
                && u.changes.is_empty()
                && u.despawns.is_empty()
                && u.cues.is_empty();
            let heartbeat = !interest.known.is_empty()
                && interest
                    .last_tick
                    .is_none_or(|t| tick >= t + HEARTBEAT_TICKS);
            if quiet && !u.reset && !heartbeat {
                continue;
            }
            interest.reset = false;
            interest.last_tick = Some(tick);
            self.send_traffic(conn, u);
        }
    }
}
