//! Parked cars (spec 14 Phase 18, 18B *Vehicles*: "parked cars (static,
//! at the kerb outside the riding lane)").
//!
//! Static cars derived from the package alone – the same on the server
//! and on every client, with no network traffic: [`ParkedCars`] places
//! them per 250 m cell when first asked, from the network's `TRFC`
//! (`parking:left|right|both`, the old `parking:lane:*`; T2's
//! [`ocw_world::traffic::Parking`]) and its surface car parks
//! ([`ocw_world::traffic::ParkingArea`]), and a seed from the road
//! graph's fingerprint.
//!
//! - **Along the kerb:** in slots of the orientation (parallel 6 m,
//!   diagonal 3.4 m, perpendicular 2.6 m), each taken by the street's
//!   occupancy ([`occupancy`]: residential streets fuller than main
//!   roads), the gaps between them random from the seed; none within
//!   [`JUNCTION_GAP_M`] of a junction, [`STOP_GAP_M`] of a bus stop or
//!   10 m of a crossing or a signal, none on bridges and in tunnels. A
//!   residential or unclassified street in town without parking data gets
//!   cars at half the occupancy (the usual case: parking is mapped on few
//!   streets).
//! - **Never in a lane or on the riders' line:** each car stands so its
//!   inner side keeps [`LANE_CLEAR_M`] from the outermost traffic lane's
//!   widest vehicle (a bus) and at least [`RIDERS_CLEAR_M`] from the
//!   centreline riders ride on; where the kerb leaves no such room (a
//!   narrow street whose lanes fill it), the car moves onto the kerb, and
//!   where even that is too far from the road, the side gets none. It
//!   stays clear of every solid of the world (street lamps, bollards,
//!   trees, walls, buildings) and of every other road's carriageway.
//! - **In car parks:** surface car parks (not multi-storey or
//!   underground ones) get rows of perpendicular spaces round their
//!   centre for their capacity, aligned with the road at their entrance,
//!   about half of them taken.
//! - **Obstacles:** cars never stand in a lane, so the traffic never
//!   meets them; for the riders they are solids ([`ParkedCars::solids`],
//!   12B's walls) where a rider leaves the carriageway (wild mode).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ocw_world::RoadNetwork;
use ocw_world::graph::RoadClass;
use ocw_world::obstacles::{ObstacleKind, Shape, Solid};
use ocw_world::traffic::{ParkingKind, ParkingOrientation, ParkingPlace};

use super::routing::{key, unit_f};
use super::rules::VehicleClass;
use crate::collide::WorldCache;

/// The cells cars are placed in (m).
pub const CELL_M: f64 = 250.0;
/// No car this close to the end of an edge (m: the junction, its corner).
pub const JUNCTION_GAP_M: f64 = 10.0;
/// No car this close to a bus stop on the edge (m).
pub const STOP_GAP_M: f64 = 20.0;
/// A car's inner side keeps this from the outermost lane's widest vehicle (m).
pub const LANE_CLEAR_M: f64 = 0.25;
/// … and at least this from the road's centreline (m): the riders' line.
pub const RIDERS_CLEAR_M: f64 = 1.5;
/// The widest vehicle in a lane (m; half of a bus).
const LANE_HALF_W: f64 = 1.28;
/// A car's half length and width (m; the T6 cars).
pub const CAR_HALF: [f64; 2] = [2.25, 0.9];
/// A van's (m).
pub const VAN_HALF: [f64; 2] = [2.7, 1.0];
/// The kerb's height a car on it stands on (m).
const KERB_Y_M: f64 = 0.12;

/// A parked car.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParkedCar {
    /// Unique among parked cars (the top bit set: never a moving vehicle's).
    pub id: u32,
    pub class: VehicleClass,
    pub look: u32,
    /// The ground under its centre (x, y, z).
    pub pos: [f64; 3],
    /// The way its front looks (unit x, z).
    pub heading: [f64; 2],
    /// Half its length and width (m).
    pub half: [f64; 2],
    /// The edge it parks at (`None`: in a car park), the side (+1 right of
    /// the edge's direction) and its centre's metres from the centreline.
    pub edge: Option<u32>,
    pub lateral: f64,
}

impl ParkedCar {
    /// The car's four corners (x, z).
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let (h, r) = (self.heading, [-self.heading[1], self.heading[0]]);
        let [hl, hw] = self.half;
        let c = [self.pos[0], self.pos[2]];
        [(1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (-1.0, 1.0)].map(|(a, b)| {
            [
                c[0] + h[0] * hl * a + r[0] * hw * b,
                c[1] + h[1] * hl * a + r[1] * hw * b,
            ]
        })
    }

    /// The car as a solid (a wall along its length, its half width thick).
    pub fn solid(&self) -> Solid {
        let (h, [hl, hw]) = (self.heading, self.half);
        let c = [self.pos[0], self.pos[2]];
        Solid {
            shape: Shape::Segment {
                a: [c[0] - h[0] * (hl - hw), c[1] - h[1] * (hl - hw)],
                b: [c[0] + h[0] * (hl - hw), c[1] + h[1] * (hl - hw)],
                half: hw,
            },
            kind: ObstacleKind::ParkedCar,
            y: None,
            id: 0xCA20_0000_0000_0000 | u64::from(self.id),
        }
    }

    /// Whether point `p` (x, z) is within `margin` of the car.
    pub fn touches(&self, p: [f64; 2], margin: f64) -> bool {
        let d = [p[0] - self.pos[0], p[1] - self.pos[2]];
        let along = d[0] * self.heading[0] + d[1] * self.heading[1];
        let across = -d[0] * self.heading[1] + d[1] * self.heading[0];
        along.abs() <= self.half[0] + margin && across.abs() <= self.half[1] + margin
    }
}

/// The share of kerb slots taken by street class (tagged parking; half of
/// it where a town street's parking is not mapped).
pub fn occupancy(class: RoadClass) -> f64 {
    match class {
        RoadClass::Residential => 0.65,
        RoadClass::Unclassified | RoadClass::Service => 0.45,
        RoadClass::Tertiary => 0.5,
        RoadClass::Secondary => 0.4,
        RoadClass::Primary => 0.3,
        _ => 0.0,
    }
}

/// A cell of [`CELL_M`] (x, z index).
pub type Cell = (i32, i32);

/// The parked cars of a world's network.
pub struct ParkedCars {
    net: Arc<RoadNetwork>,
    seed: u64,
    world: Mutex<WorldCache>,
    cells: Mutex<BTreeMap<Cell, Arc<[ParkedCar]>>>,
    /// The car parks by the cell of their centre.
    parks: BTreeMap<(i32, i32), Vec<usize>>,
}

impl std::fmt::Debug for ParkedCars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParkedCars")
            .field("seed", &self.seed)
            .field("cells", &self.cells.lock().map_or(0, |c| c.len()))
            .finish()
    }
}

fn cell_of(p: [f64; 2]) -> (i32, i32) {
    (
        (p[0] / CELL_M).floor() as i32,
        (p[1] / CELL_M).floor() as i32,
    )
}

impl ParkedCars {
    /// The parked cars of `net` (`None`: its package has no traffic data,
    /// as route rides and old packages).
    pub fn new(net: Arc<RoadNetwork>) -> Option<Self> {
        let data = net.road_rules().data?;
        let mut parks: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
        for (i, p) in data.parking.iter().enumerate() {
            if p.kind == ParkingKind::Surface && p.capacity > 0 {
                parks
                    .entry(cell_of([f64::from(p.pos[0]), f64::from(p.pos[1])]))
                    .or_default()
                    .push(i);
            }
        }
        Some(Self {
            seed: net.fingerprint() ^ 0x0BAD_CA25_0F00_0001,
            world: Mutex::new(WorldCache::new(net.obstacles().cloned())),
            cells: Mutex::new(BTreeMap::new()),
            parks,
            net,
        })
    }

    pub fn network(&self) -> &Arc<RoadNetwork> {
        &self.net
    }

    /// The cars of cell `cell`, placed when first asked.
    pub fn cell(&self, cell: (i32, i32)) -> Arc<[ParkedCar]> {
        if let Some(c) = self.cells.lock().ok().and_then(|m| m.get(&cell).cloned()) {
            return c;
        }
        let cars: Arc<[ParkedCar]> = self.place(cell).into();
        if let Ok(mut m) = self.cells.lock() {
            m.insert(cell, cars.clone());
        }
        cars
    }

    /// The cars whose centre is within `radius` of `p` (x, z), by id.
    pub fn near(&self, p: [f64; 2], radius: f64) -> Vec<ParkedCar> {
        // Car parks reach a little out of their centre's cell.
        let r = radius + 100.0;
        let (a, b) = (cell_of([p[0] - r, p[1] - r]), cell_of([p[0] + r, p[1] + r]));
        let mut out: Vec<ParkedCar> = Vec::new();
        for cx in a.0..=b.0 {
            for cz in a.1..=b.1 {
                out.extend(
                    self.cell((cx, cz))
                        .iter()
                        .filter(|c| (c.pos[0] - p[0]).hypot(c.pos[2] - p[1]) <= radius),
                );
            }
        }
        out.sort_by_key(|c| c.id);
        out
    }

    /// The cars as solids within the box (lo, hi) (12B: riders in wild mode).
    pub fn solids(&self, lo: [f64; 2], hi: [f64; 2], out: &mut Vec<Solid>) {
        let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let r = (hi[0] - lo[0]).hypot(hi[1] - lo[1]) / 2.0 + 3.0;
        for c in self.near(mid, r) {
            let s = c.solid();
            let (a, b) = s.bounds();
            if a[0] <= hi[0] && b[0] >= lo[0] && a[1] <= hi[1] && b[1] >= lo[1] {
                out.push(s);
            }
        }
    }

    /// Whether the box of `car` grown by 0.15 m touches a solid of the world.
    fn blocked(&self, car: &ParkedCar) -> bool {
        let Ok(mut w) = self.world.lock() else {
            return false;
        };
        if w.is_empty() {
            return false;
        }
        let r = car.half[0] + 0.5;
        let c = [car.pos[0], car.pos[2]];
        let mut solids = Vec::new();
        w.solids_near([c[0] - r, c[1] - r], [c[0] + r, c[1] + r], 0, &mut solids);
        solids.iter().any(|s| match s.shape {
            Shape::Circle { c, r } => car.touches(c, r + 0.15),
            Shape::Segment { a, b, half } => {
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                let n = (len / 0.25).ceil().max(1.0) as usize;
                (0..=n).any(|k| {
                    let t = k as f64 / n as f64;
                    car.touches(
                        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                        half + 0.15,
                    )
                })
            }
        })
    }

    /// Whether `car` reaches onto the carriageway of another road than
    /// `own` (a side street at a corner, a parallel road).
    fn on_other_road(&self, car: &ParkedCar, own: Option<usize>) -> bool {
        let net = &*self.net;
        let c = [car.pos[0], car.pos[2]];
        let pts = car.corners();
        net.edges_near(c, 20.0).into_iter().any(|e| {
            if Some(e) == own {
                return false;
            }
            let edge = net.edge(e);
            let half = edge.width_m / 2.0 + 0.3;
            let g = net.geometry(e);
            pts.iter().chain(std::iter::once(&c)).any(|p| {
                g.points.windows(2).any(|w| {
                    ocw_world::obstacles::segment_distance(
                        *p,
                        [w[0][0], w[0][2]],
                        [w[1][0], w[1][2]],
                    ) < half
                })
            })
        })
    }

    /// Places the cars of a cell (see the module).
    fn place(&self, cell: (i32, i32)) -> Vec<ParkedCar> {
        let net = &*self.net;
        let Some(data) = net.road_rules().data else {
            return Vec::new();
        };
        let rules = net.road_rules();
        let mut out: Vec<ParkedCar> = Vec::new();
        let centre = [
            (f64::from(cell.0) + 0.5) * CELL_M,
            (f64::from(cell.1) + 0.5) * CELL_M,
        ];
        let id_base = 0x8000_0000u32
            | ((cell.0.rem_euclid(256) as u32) << 23)
            | ((cell.1.rem_euclid(256) as u32) << 15);
        let mut next = 0u32;
        let mut take_id = || {
            next += 1;
            id_base | (next & 0x7FFF)
        };
        let flip = if net.keep_left { -1.0 } else { 1.0 };
        for e in net.edges_near(centre, CELL_M * 0.75) {
            let edge = net.edge(e);
            if edge.bridge || edge.tunnel {
                continue;
            }
            let base_occ = occupancy(edge.class);
            if base_occ <= 0.0 {
                continue;
            }
            let wr = rules.edge(edge);
            if !wr.has_traffic() {
                continue;
            }
            let core = super::world::edge_rule(&wr, edge.class);
            let total = core.lanes[0] + core.lanes[1];
            if total == 0 {
                continue;
            }
            let g = net.geometry(e);
            let len = g.length();
            let w = edge.width_m;
            // The lanes' centres, right of the centreline along the edge.
            let mut centres: Vec<f64> = Vec::new();
            for k in 0..core.lanes[0] {
                centres.push(flip * super::lateral_offset(&core, w, k));
            }
            for k in 0..core.lanes[1] {
                centres.push(-flip * super::lateral_offset(&core, w, k));
            }
            let stops: Vec<f64> = data
                .stops
                .iter()
                .filter(|s| s.edge == Some(edge.id))
                .map(|s| f64::from(s.along_m))
                .collect();
            let controls: Vec<f64> = data
                .controls_on(edge.id)
                .iter()
                .map(|c| f64::from(c.along_m))
                .collect();
            for side in [-1.0f64, 1.0] {
                let p = wr.parking[usize::from(side > 0.0)];
                let (occ, place) = if p.has_cars() {
                    (base_occ, p.place)
                } else if p.place == ParkingPlace::Unknown
                    && wr.urban
                    && matches!(edge.class, RoadClass::Residential | RoadClass::Unclassified)
                {
                    (base_occ * 0.5, ParkingPlace::Lane)
                } else {
                    continue;
                };
                let (slot, yaw, across) = match p.orientation {
                    ParkingOrientation::Parallel => (6.0, 0.0, VAN_HALF[1]),
                    ParkingOrientation::Diagonal => (
                        3.4,
                        std::f64::consts::FRAC_PI_4,
                        (VAN_HALF[0] + VAN_HALF[1]) * std::f64::consts::FRAC_1_SQRT_2,
                    ),
                    ParkingOrientation::Perpendicular => {
                        (2.6, std::f64::consts::FRAC_PI_2, VAN_HALF[0])
                    }
                };
                let lane_out = centres
                    .iter()
                    .map(|c| c * side)
                    .fold(f64::NEG_INFINITY, f64::max)
                    + LANE_HALF_W;
                let inner = (lane_out + LANE_CLEAR_M).max(RIDERS_CLEAR_M);
                let at_kerb = match place {
                    ParkingPlace::Lane => w / 2.0 - across - 0.15,
                    ParkingPlace::HalfOnKerb => w / 2.0,
                    _ => w / 2.0 + across + 0.2,
                };
                let lateral = at_kerb.max(inner + across);
                if lateral > w / 2.0 + across + 0.8 {
                    continue;
                }
                let skey = key(self.seed, edge.id.0, u64::from(side > 0.0), 0x5107);
                let phase = unit_f(skey) * slot;
                let mut s = JUNCTION_GAP_M + phase + slot / 2.0;
                let mut k = 0u64;
                while s < len - JUNCTION_GAP_M {
                    let here = s;
                    s += slot;
                    k += 1;
                    let h = key(skey, k, 1, 2);
                    if unit_f(h) >= occ {
                        continue;
                    }
                    if stops.iter().any(|&x| (x - here).abs() < STOP_GAP_M)
                        || controls.iter().any(|&x| (x - here).abs() < 10.0)
                    {
                        continue;
                    }
                    let at = (here + (unit_f(key(h, 3, 4, 5)) - 0.5) * 0.4).clamp(0.0, len);
                    let pt = g.point_at(at);
                    let (a, b) = (
                        g.point_at((at - 1.0).max(0.0)),
                        g.point_at((at + 1.0).min(len)),
                    );
                    let dir = ocw_world::network::unit([b[0] - a[0], b[2] - a[2]]);
                    let right = [-dir[1], dir[0]];
                    let c = [
                        pt[0] + right[0] * lateral * side,
                        pt[2] + right[1] * lateral * side,
                    ];
                    // Parallel cars face the way of their side's traffic.
                    let forward = if core.lanes[1] == 0 || core.lanes[0] == 0 {
                        core.lanes[0] > 0
                    } else {
                        (side > 0.0) != net.keep_left
                    };
                    let mut hd = if forward { dir } else { [-dir[0], -dir[1]] };
                    if yaw != 0.0 {
                        // Nose towards the kerb (perpendicular: either way).
                        let turn = if yaw > 1.0 && unit_f(key(h, 6, 7, 8)) < 0.5 {
                            -yaw
                        } else {
                            yaw
                        } * if forward { side } else { -side };
                        let (sn, cs) = turn.sin_cos();
                        hd = [hd[0] * cs - hd[1] * sn, hd[0] * sn + hd[1] * cs];
                    }
                    let van = unit_f(key(h, 9, 10, 11)) < 0.08;
                    let half = if van { VAN_HALF } else { CAR_HALF };
                    let car = ParkedCar {
                        id: 0,
                        class: if van {
                            VehicleClass::Van
                        } else {
                            VehicleClass::Car
                        },
                        look: key(h, 12, 13, 14) as u32,
                        pos: [
                            c[0],
                            pt[1] + if lateral > w / 2.0 { KERB_Y_M } else { 0.0 },
                            c[1],
                        ],
                        heading: hd,
                        half,
                        edge: Some(e as u32),
                        lateral: lateral * side,
                    };
                    if cell_of(c) != cell || self.on_other_road(&car, Some(e)) || self.blocked(&car)
                    {
                        continue;
                    }
                    out.push(ParkedCar {
                        id: take_id(),
                        ..car
                    });
                }
            }
        }
        // Surface car parks with their centre in this cell.
        for &pi in self.parks.get(&cell).map_or(&[][..], Vec::as_slice) {
            let p = &data.parking[pi];
            let c0 = [f64::from(p.pos[0]), f64::from(p.pos[1])];
            let dir = p
                .edge
                .and_then(|id| net.edge_index(id))
                .map_or([1.0, 0.0], |e| {
                    let g = net.geometry(e);
                    let len = g.length();
                    let at = f64::from(p.along_m).clamp(0.0, len);
                    let (a, b) = (
                        g.point_at((at - 2.0).max(0.0)),
                        g.point_at((at + 2.0).min(len)),
                    );
                    ocw_world::network::unit([b[0] - a[0], b[2] - a[2]])
                });
            let right = [-dir[1], dir[0]];
            let cap = usize::from(p.capacity.min(400));
            // Double rows of perpendicular spaces 2.5 × 5 m, 6 m aisles.
            let side_m = (cap as f64 * 25.0).sqrt();
            let rows = ((side_m / 8.0).round() as usize).max(1);
            let per_row = cap.div_ceil(rows);
            let road_y = net.nearest_within(c0, 80.0).map_or(0.0, |(_, _, _, y)| y);
            let ground = net.ground();
            let pkey = key(self.seed, p.osm_id as u64, 0xCA2, 0);
            for r in 0..rows {
                let across = (r as f64 - (rows as f64 - 1.0) / 2.0) * 8.0;
                for k in 0..per_row {
                    let along = (k as f64 - (per_row as f64 - 1.0) / 2.0) * 2.5;
                    let h = key(pkey, r as u64, k as u64, 1);
                    if unit_f(h) >= 0.55 {
                        continue;
                    }
                    let c = [
                        c0[0] + dir[0] * along + right[0] * across,
                        c0[1] + dir[1] * along + right[1] * across,
                    ];
                    // Nose to the row's middle line (rows alternate).
                    let s = if r % 2 == 0 { 1.0 } else { -1.0 };
                    let van = unit_f(key(h, 9, 10, 11)) < 0.06;
                    let car = ParkedCar {
                        id: 0,
                        class: if van {
                            VehicleClass::Van
                        } else {
                            VehicleClass::Car
                        },
                        look: key(h, 12, 13, 14) as u32,
                        pos: [
                            c[0],
                            ground.and_then(|g| g.height(c[0], c[1])).unwrap_or(road_y),
                            c[1],
                        ],
                        heading: [right[0] * s, right[1] * s],
                        half: if van { VAN_HALF } else { CAR_HALF },
                        edge: None,
                        lateral: 0.0,
                    };
                    if self.on_other_road(&car, None) || self.blocked(&car) {
                        continue;
                    }
                    out.push(ParkedCar {
                        id: take_id(),
                        ..car
                    });
                }
            }
        }
        // Two placements that touch (a corner where two streets' kerbs
        // meet): the later goes.
        let mut kept: Vec<ParkedCar> = Vec::with_capacity(out.len());
        for c in out {
            let clash = kept.iter().any(|k| {
                (k.pos[0] - c.pos[0]).hypot(k.pos[2] - c.pos[2]) < 6.0
                    && (c.corners().iter().any(|p| k.touches(*p, 0.1))
                        || k.corners().iter().any(|p| c.touches(*p, 0.1)))
            });
            if !clash {
                kept.push(c);
            }
        }
        kept
    }
}
