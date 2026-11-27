//! What a vehicle may do on an edge and at a junction (spec 14 Phase 18,
//! 18B *Data from the package*): the [`EdgeRules`] trait the traffic core
//! asks, and [`ClassRules`], its default derived from the road class of
//! the graph alone (old packages without `TRFC`: "traffic only on roads
//! by class with implied speeds"). The world data of T2 plugs in later by
//! implementing the trait over its per-edge records.

use ocw_world::RoadNetwork;
use ocw_world::graph::{Oneway, RoadClass, VertexId};

/// The kinds of road vehicle (spec 18B *Vehicles*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VehicleClass {
    Car,
    Van,
    Lorry,
    Bus,
    Motorbike,
    Tractor,
}

/// The fixed physical traits of a vehicle class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassTraits {
    /// Body length (m).
    pub length_m: f64,
    /// Highest acceleration (m/s², IDM `a`).
    pub accel: f64,
    /// Comfortable deceleration (m/s², IDM `b`).
    pub decel: f64,
    /// Top speed of the vehicle or its legal limit (m/s).
    pub top_speed: f64,
    /// Comfortable lateral acceleration in curves (m/s²).
    pub lateral: f64,
}

impl VehicleClass {
    pub const ALL: [Self; 6] = [
        Self::Car,
        Self::Van,
        Self::Lorry,
        Self::Bus,
        Self::Motorbike,
        Self::Tractor,
    ];

    /// Index 0 – 5 (in [`Self::ALL`]).
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Car => "car",
            Self::Van => "van",
            Self::Lorry => "lorry",
            Self::Bus => "bus",
            Self::Motorbike => "motorbike",
            Self::Tractor => "tractor",
        }
    }

    pub fn traits(self) -> ClassTraits {
        let t = |length_m, accel, decel, top_kmh: f64, lateral| ClassTraits {
            length_m,
            accel,
            decel,
            top_speed: top_kmh / 3.6,
            lateral,
        };
        match self {
            Self::Car => t(4.5, 1.5, 2.0, 180.0, 2.6),
            Self::Van => t(5.4, 1.2, 1.8, 150.0, 2.3),
            // Lorries and buses: 80 km/h outside towns (DE, CH, ES).
            Self::Lorry => t(12.0, 0.7, 1.5, 80.0, 1.8),
            Self::Bus => t(12.0, 0.9, 1.5, 80.0, 1.6),
            Self::Motorbike => t(2.2, 2.5, 3.0, 200.0, 3.4),
            Self::Tractor => t(6.0, 0.6, 1.5, 40.0, 1.5),
        }
    }
}

/// Which vehicle classes may use an edge (a bit per [`VehicleClass`];
/// OSM `access`, `motor_vehicle`, `hgv`, `psv`, `agricultural`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Access(pub u8);

impl Access {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b11_1111);

    pub fn only(classes: &[VehicleClass]) -> Self {
        Self(classes.iter().fold(0, |m, c| m | 1 << c.index()))
    }

    pub fn allows(self, class: VehicleClass) -> bool {
        self.0 & (1 << class.index()) != 0
    }

    pub fn without(self, class: VehicleClass) -> Self {
        Self(self.0 & !(1 << class.index()))
    }

    pub fn with(self, class: VehicleClass) -> Self {
        Self(self.0 | 1 << class.index())
    }

    /// No motor vehicle at all (cycleways, paths, `motor_vehicle=no`).
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// The traffic facts of one road-graph edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRule {
    /// One-way for motor vehicles (`oneway`; `oneway:bicycle=no` frees
    /// only riders, so this may be stricter than the graph's).
    pub oneway: Oneway,
    /// Lanes from `from` to `to` and back (`lanes:forward` / `:backward`).
    /// A direction against `oneway` has none.
    pub lanes: [u8; 2],
    /// The speed limit (m/s; `maxspeed` or the implied limit).
    pub speed: f64,
    /// Who may drive here.
    pub access: Access,
    /// Annual average daily traffic, both directions together (vehicles a
    /// day; the class's typical value, or T11's counts).
    pub aadt: f64,
    /// Part of a roundabout (`junction=roundabout`): the circle has right
    /// of way.
    pub roundabout: bool,
    /// The importance for right of way (higher wins; from the class).
    pub rank: u8,
}

impl EdgeRule {
    /// Whether `class` may drive the edge in this direction.
    pub fn allows(&self, class: VehicleClass, forward: bool) -> bool {
        self.access.allows(class) && self.oneway.allows(forward) && self.lanes_in(forward) > 0
    }

    /// The lanes in a direction.
    pub fn lanes_in(&self, forward: bool) -> u8 {
        self.lanes[usize::from(!forward)]
    }
}

/// How a junction is controlled for the vehicles arriving on one edge
/// (spec 18B *Behaviour*, 2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    /// Nothing to do (a bend, a road end).
    None,
    /// The arrival has right of way (the major road, the roundabout's circle).
    Priority,
    /// Give way to the major road (`give_way`, a minor road, entering a roundabout).
    GiveWay,
    /// Stop, then give way (`stop`).
    Stop,
    /// Equal roads: give way to the vehicle from the right (left where
    /// traffic keeps left).
    RightBeforeLeft,
    /// Traffic lights (`highway=traffic_signals`), cycles from the tick.
    Signals,
}

impl Control {
    /// Right-of-way level: a vehicle yields to arrivals of a higher level.
    pub fn level(self) -> u8 {
        match self {
            Self::None | Self::Priority | Self::Signals => 2,
            Self::RightBeforeLeft => 1,
            Self::GiveWay | Self::Stop => 0,
        }
    }

    /// Whether a vehicle must look for conflicting traffic here.
    pub fn yields(self) -> bool {
        matches!(self, Self::GiveWay | Self::Stop | Self::RightBeforeLeft)
    }
}

/// The rules the traffic core asks. [`ClassRules`] derives them from the
/// road class; T2's `TRFC` data implements this trait over its records.
pub trait EdgeRules {
    /// The traffic facts of edge `edge` (an index of `net`).
    fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule;

    /// An explicit control at `vertex` for vehicles arriving on `edge` in
    /// direction `forward` (signals, `stop`, `give_way`); `None` lets the
    /// core derive it from the classes of the roads that meet
    /// ([`derived_control`]).
    fn node_control(
        &self,
        net: &RoadNetwork,
        vertex: VertexId,
        edge: usize,
        forward: bool,
    ) -> Option<Control> {
        let _ = (net, vertex, edge, forward);
        None
    }
}

/// Typical values by road class (no `TRFC` data).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassDefaults {
    pub speed_kmh: f64,
    pub aadt: f64,
    pub rank: u8,
    pub access: Access,
}

/// The default facts of a road class: implied speed limits between a
/// town's 50 and the open road's 80 – 100 (spec 18B, refined by T2 per
/// country), daily traffic per class (tuned by T11's counts later).
pub fn class_defaults(class: RoadClass) -> ClassDefaults {
    use VehicleClass as V;
    let motor = Access::ALL;
    let d = |speed_kmh, aadt, rank, access| ClassDefaults {
        speed_kmh,
        aadt,
        rank,
        access,
    };
    match class {
        RoadClass::Trunk => d(100.0, 20_000.0, 7, motor.without(V::Tractor)),
        RoadClass::Primary => d(80.0, 9_000.0, 6, motor),
        RoadClass::Secondary => d(70.0, 4_500.0, 5, motor),
        RoadClass::Tertiary => d(60.0, 2_200.0, 4, motor),
        RoadClass::Unclassified => d(50.0, 700.0, 3, motor),
        RoadClass::Residential => d(30.0, 250.0, 2, motor),
        RoadClass::Service => d(20.0, 60.0, 1, motor.without(V::Lorry).without(V::Bus)),
        // Tracks: tractors only (`agricultural`, 18B).
        RoadClass::Track => d(25.0, 8.0, 0, Access::only(&[V::Tractor])),
        RoadClass::Cycleway | RoadClass::Path => d(0.0, 0.0, 0, Access::NONE),
    }
}

/// The rules of an edge from its class and geometry alone.
pub fn class_rule(net: &RoadNetwork, edge: usize) -> EdgeRule {
    let e = net.edge(edge);
    let c = class_defaults(e.class);
    // The graph's one-way is the riders'; where a road is one-way for
    // riders it is for cars too (the reverse needs `TRFC`).
    let oneway = e.oneway;
    let per_dir = match (e.class, oneway) {
        (RoadClass::Trunk, Oneway::Forward | Oneway::Backward) => 2,
        (_, Oneway::Forward | Oneway::Backward) if e.width_m >= 7.0 => 2,
        _ => 1,
    };
    let lanes = if c.access.is_none() {
        [0, 0]
    } else {
        [
            if oneway.allows(true) { per_dir } else { 0 },
            if oneway.allows(false) { per_dir } else { 0 },
        ]
    };
    EdgeRule {
        oneway,
        lanes,
        speed: c.speed_kmh / 3.6,
        access: c.access,
        aadt: c.aadt,
        roundabout: false,
        rank: c.rank,
    }
}

/// The default [`EdgeRules`]: everything from the road class
/// ([`class_rule`]); junctions by [`derived_control`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassRules;

impl EdgeRules for ClassRules {
    fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
        class_rule(net, edge)
    }
}

/// The control of a junction from the roads that meet (spec 18B
/// *Behaviour*): roundabouts yield to the circle; a lower class gives
/// way to a higher one; equal minor roads (unclassified and below) are
/// right before left; equal major roads have right of way.
/// `rule(e)` gives the rules of each edge at the vertex.
pub fn derived_control(
    net: &RoadNetwork,
    vertex: VertexId,
    edge: usize,
    rule: impl Fn(usize) -> EdgeRule,
) -> Control {
    let mine = rule(edge);
    let others: Vec<EdgeRule> = net
        .edges_at(vertex)
        .iter()
        .filter(|&&e| e != edge)
        .map(|&e| rule(e))
        .filter(|r| !r.access.is_none())
        .collect();
    // A bend or a road end is no junction.
    if net.edges_at(vertex).len() <= 2 || others.is_empty() {
        return Control::None;
    }
    if mine.roundabout {
        return Control::Priority;
    }
    if others.iter().any(|r| r.roundabout) {
        return Control::GiveWay;
    }
    let top = others.iter().map(|r| r.rank).max().unwrap_or(0);
    if mine.rank < top {
        Control::GiveWay
    } else if mine.rank > top {
        Control::Priority
    } else if mine.rank <= class_defaults(RoadClass::Unclassified).rank {
        Control::RightBeforeLeft
    } else {
        Control::Priority
    }
}
