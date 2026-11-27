//! The road vehicles on the client (spec 14 Phase 18, 18E T5; Q3 (a)):
//! the server sends each vehicle near the rider once as its plan and then
//! only its changes (`TrafficUpdate`); [`ClientVehicles`] keeps one
//! `ocw_sim::traffic::PlanFollower` per vehicle – the same code the
//! server runs beside it – and extrapolates them on the server's clock,
//! so every client of a session sees the same vehicles at the same tick.
//!
//! A change can come after the client has driven the vehicle past the
//! change's tick: the follower takes the change's state and is driven on
//! to now. The step in the drawn position this makes is not shown at
//! once: it is blended out over [`BLEND_S`] (an offset that fades), so a
//! vehicle never jumps by more than a few centimetres a frame; only an
//! error above [`SNAP_M`] (a vehicle that was somewhere else entirely)
//! is taken at once.
//!
//! Road traffic is a desktop thing (Q9 (b)): the browser's client is made
//! with [`ClientVehicles::set_shown`] off and ignores the updates.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ocw_protocol::v1;
use ocw_protocol::vehicles::{change_from_wire, spawn_from_wire};
use ocw_render::traffic::VehicleView;
use ocw_sim::traffic::{Curves, EdgeRule, Motion, PlanFollower, Tick, lateral_offset, package};
use ocw_world::RoadNetwork;

/// A correction is blended out over about this long (s).
pub const BLEND_S: f64 = 0.3;
/// A correction larger than this is taken at once (m).
pub const SNAP_M: f64 = 25.0;

/// What the client has of one vehicle besides its follower.
#[derive(Debug, Clone, Copy, Default)]
struct Blend {
    /// The drawn position minus the follower's at `at_s` (x, z).
    offset: [f64; 2],
    at_s: f64,
}

impl Blend {
    fn at(&self, now_s: f64) -> [f64; 2] {
        let age = (now_s - self.at_s).max(0.0);
        // A smooth fade to zero: (1 − x)² over BLEND_S.
        let k = (1.0 - age / BLEND_S).max(0.0);
        let k = k * k;
        [self.offset[0] * k, self.offset[1] * k]
    }
}

/// Counters for tests and `net.stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VehicleStats {
    pub updates: u64,
    pub spawns: u64,
    pub changes: u64,
    pub despawns: u64,
    pub resets: u64,
    /// The largest correction seen (m) and how many were snapped.
    pub max_correction_m: f64,
    pub snapped: u64,
    /// Updates for another graph, ignored.
    pub foreign: u64,
}

struct Inner {
    net: Arc<RoadNetwork>,
    curves: Curves,
    rules: BTreeMap<usize, EdgeRule>,
    followers: Vec<PlanFollower>,
    blends: BTreeMap<u32, Blend>,
    dt: f64,
    /// Traffic tick minus the session's tick (the server's clock).
    offset: i64,
}

/// The road vehicles near the rider.
pub struct ClientVehicles {
    shown: bool,
    inner: Option<Mutex<Inner>>,
    stats: VehicleStats,
    /// A route course's road as the traffic's network (the server makes
    /// the same: `ocw_sim::traffic::package::route_network`).
    route_net: Option<(String, Arc<RoadNetwork>)>,
}

impl Default for ClientVehicles {
    fn default() -> Self {
        Self {
            shown: true,
            inner: None,
            stats: VehicleStats::default(),
            route_net: None,
        }
    }
}

impl Inner {
    fn new(net: Arc<RoadNetwork>) -> Self {
        Self {
            curves: Curves::new(&net),
            net,
            rules: BTreeMap::new(),
            followers: Vec::new(),
            blends: BTreeMap::new(),
            dt: 0.02,
            offset: 0,
        }
    }

    fn rule(&mut self, e: usize) -> EdgeRule {
        let net = &self.net;
        *self.rules.entry(e).or_insert_with(|| {
            let edge = net.edge(e);
            package::edge_rule(&net.road_rules().edge(edge), edge.class)
        })
    }

    /// Drives every follower to `tick`.
    fn advance(&mut self, tick: Tick) {
        let (net, curves, dt) = (&*self.net, &self.curves, self.dt);
        for f in &mut self.followers {
            if f.tick < tick {
                f.advance_to(net, curves, tick, dt);
            }
        }
    }

    /// The follower's drawn point (x, y, z), heading and lateral offset.
    fn place(&mut self, i: usize, frac: f64) -> Option<([f64; 3], [f64; 2], f64)> {
        let f = &self.followers[i];
        let (e, forward, s) = f.on_edge(&self.net)?;
        let lane = f.lane;
        let v = f.v;
        let ended = f.ended || f.motion == Motion::Stopped;
        let g = self.net.geometry(e);
        let len = g.length();
        // Within the tick: on along the heading at the speed.
        let ahead = if ended { 0.0 } else { v * frac * self.dt };
        let s = if forward { s + ahead } else { s - ahead }.clamp(0.0, len);
        let p = g.point_at(s);
        let (a, b) = (
            g.point_at((s - 1.0).max(0.0)),
            g.point_at((s + 1.0).min(len)),
        );
        let mut h = ocw_world::network::unit([b[0] - a[0], b[2] - a[2]]);
        if !forward {
            h = [-h[0], -h[1]];
        }
        let width = self.net.edge(e).width_m;
        let keep_left = self.net.keep_left;
        let rule = self.rule(e);
        let lateral = lateral_offset(&rule, width, lane) * if keep_left { -1.0 } else { 1.0 };
        let right = [-h[1], h[0]];
        Some((
            [p[0] + right[0] * lateral, p[1], p[2] + right[1] * lateral],
            h,
            lateral,
        ))
    }

    fn xz(&mut self, i: usize) -> Option<[f64; 2]> {
        self.place(i, 0.0).map(|(p, ..)| [p[0], p[2]])
    }

    fn index(&self, id: u32) -> Result<usize, usize> {
        self.followers.binary_search_by_key(&id, |f| f.id)
    }
}

impl ClientVehicles {
    /// Whether this client keeps and shows road vehicles (the browser:
    /// off, Q9 (b)).
    pub fn set_shown(&mut self, on: bool) {
        self.shown = on;
        if !on {
            self.inner = None;
        }
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn stats(&self) -> VehicleStats {
        self.stats
    }

    /// The vehicles kept now.
    pub fn len(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(0, |m| m.lock().map_or(0, |i| i.followers.len()))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The traffic's network of a route course without a road graph.
    pub fn route_network(&mut self, route: &ocw_world::Route) -> Arc<RoadNetwork> {
        match &self.route_net {
            Some((id, n)) if id == route.id() => n.clone(),
            _ => {
                let n = Arc::new(package::route_network(route));
                self.route_net = Some((route.id().to_owned(), n.clone()));
                n
            }
        }
    }

    /// Forgets every vehicle (a world change, a new session).
    pub fn clear(&mut self) {
        self.inner = None;
    }

    /// The traffic tick at the session tick `server_tick` (`None` before
    /// the first update).
    pub fn tick_at(&self, server_tick: f64) -> Option<f64> {
        let m = self.inner.as_ref()?.lock().ok()?;
        Some(server_tick + m.offset as f64)
    }

    /// One update from the server, received at client time `now_s`, with
    /// the client's estimate of the session's tick then. `net`: the graph
    /// the client rides (updates for another are ignored).
    pub fn on_update(
        &mut self,
        u: &v1::TrafficUpdate,
        net: Option<&Arc<RoadNetwork>>,
        server_tick_now: f64,
        now_s: f64,
    ) {
        if !self.shown {
            return;
        }
        self.stats.updates += 1;
        let Some(net) = net else {
            self.stats.foreign += 1;
            return;
        };
        if u.graph_hash != 0 && u.graph_hash != net.fingerprint() {
            self.stats.foreign += 1;
            return;
        }
        if u.reset {
            self.stats.resets += 1;
        }
        let fresh = u.reset
            || self
                .inner
                .as_ref()
                .is_none_or(|m| !m.lock().is_ok_and(|i| Arc::ptr_eq(&i.net, net)));
        if fresh {
            self.inner = Some(Mutex::new(Inner::new(net.clone())));
        }
        let Some(m) = self.inner.as_mut() else {
            return;
        };
        let Ok(inner) = m.get_mut() else {
            return;
        };
        if u.step_ms > 0 {
            inner.dt = f64::from(u.step_ms) / 1000.0;
        }
        inner.offset = u.tick as i64 - u.server_tick as i64;
        // Where the client is on the traffic's clock now (never behind the
        // update: it was made at `tick`).
        let now_tick =
            ((server_tick_now + inner.offset as f64).floor().max(0.0) as Tick).max(u.tick);
        for id in &u.despawns {
            if let Ok(i) = inner.index(*id) {
                inner.followers.remove(i);
                inner.blends.remove(id);
                self.stats.despawns += 1;
            }
        }
        for c in &u.changes {
            let Some(c) = change_from_wire(c, u.tick) else {
                continue;
            };
            let Ok(i) = inner.index(c.id) else {
                continue;
            };
            inner.followers[i].advance_to(&inner.net, &inner.curves, now_tick, inner.dt);
            let before = inner.xz(i);
            inner.followers[i].apply(&c);
            let net = inner.net.clone();
            inner.followers[i].advance_to(&net, &inner.curves, now_tick, inner.dt);
            let after = inner.xz(i);
            self.stats.changes += 1;
            if let (Some(b), Some(a)) = (before, after) {
                let blend = inner.blends.get(&c.id).map_or([0.0; 2], |x| x.at(now_s));
                let off = [b[0] + blend[0] - a[0], b[1] + blend[1] - a[1]];
                let d = (off[0] * off[0] + off[1] * off[1]).sqrt();
                self.stats.max_correction_m = self.stats.max_correction_m.max(d);
                if d > SNAP_M {
                    self.stats.snapped += 1;
                    inner.blends.remove(&c.id);
                } else if d > 1e-3 {
                    inner.blends.insert(
                        c.id,
                        Blend {
                            offset: off,
                            at_s: now_s,
                        },
                    );
                }
            }
        }
        for s in &u.spawns {
            let Some(p) = spawn_from_wire(s, u.tick) else {
                continue;
            };
            if p.route
                .iter()
                .any(|st| st.edge as usize >= net.graph().edges.len())
            {
                continue;
            }
            let mut f = PlanFollower::new(&p);
            f.advance_to(&inner.net, &inner.curves, now_tick, inner.dt);
            match inner.index(p.id) {
                Ok(i) => inner.followers[i] = f,
                Err(i) => inner.followers.insert(i, f),
            }
            inner.blends.remove(&p.id);
            self.stats.spawns += 1;
        }
    }

    /// The vehicles for the renderer at client time `now_s`, the session's
    /// tick then being `server_tick_now` (fractional: between ticks).
    pub fn view(&self, server_tick_now: f64, now_s: f64) -> Vec<VehicleView> {
        let Some(m) = self.inner.as_ref() else {
            return Vec::new();
        };
        let Ok(mut inner) = m.lock() else {
            return Vec::new();
        };
        let t = (server_tick_now + inner.offset as f64).max(0.0);
        let tick = t.floor() as Tick;
        inner.advance(tick);
        let mut out = Vec::with_capacity(inner.followers.len());
        for i in 0..inner.followers.len() {
            let f = &inner.followers[i];
            let frac = (t - f.tick as f64).clamp(0.0, 1.0);
            let (id, class, look, lane, v) = (f.id, f.class, f.look, f.lane, f.v);
            let braking = match f.motion {
                Motion::Stopped => true,
                Motion::Accel { a, .. } => a < -0.3,
                Motion::Free => false,
            };
            let Some((pos, heading, lateral)) = inner.place(i, frac) else {
                continue;
            };
            let b = inner.blends.get(&id).map_or([0.0; 2], |x| x.at(now_s));
            out.push(VehicleView {
                id,
                class,
                look,
                pos: [pos[0] + b[0], pos[1], pos[2] + b[1]],
                heading,
                lateral,
                lane,
                speed: v,
                braking,
            });
        }
        out
    }

    /// The followers' positions (x, z) at traffic tick `tick`, without
    /// blending (tests: two clients see the same vehicles).
    pub fn positions_at(&self, tick: Tick) -> BTreeMap<u32, [f64; 2]> {
        let Some(m) = self.inner.as_ref() else {
            return BTreeMap::new();
        };
        let Ok(mut inner) = m.lock() else {
            return BTreeMap::new();
        };
        inner.advance(tick);
        (0..inner.followers.len())
            .filter_map(|i| Some((inner.followers[i].id, inner.xz(i)?)))
            .collect()
    }

    /// The traffic tick the followers are at (the latest).
    pub fn followers_tick(&self) -> Option<Tick> {
        let m = self.inner.as_ref()?.lock().ok()?;
        m.followers.iter().map(|f| f.tick).max()
    }
}
