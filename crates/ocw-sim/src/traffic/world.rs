//! The package's traffic rules (`TRFC`, T2's [`ocw_world::traffic`]) as
//! the core's [`EdgeRules`] (T4: the country and *in town* give the legal
//! passing distance, `overtaking=no` the places where riders may not be
//! passed). A package without `TRFC` gets T2's implied rules by class and
//! the country of its time zone.
//!
//! ```ignore
//! let rules = WorldRules::of(&package, &net);
//! let sim = TrafficSim::new(&net, Box::new(rules), cfg);
//! ```

use ocw_world::RoadNetwork;
use ocw_world::graph::VertexId;
use ocw_world::traffic::{self as wt, ControlKind, RoadRules};

use super::around::PassRule;
use super::rules::{Access, Control, EdgeRule, EdgeRules, VehicleClass, class_defaults};

/// [`EdgeRules`] over a package's per-edge rules, owned (by edge index of
/// the network).
#[derive(Debug, Clone)]
pub struct WorldRules {
    edges: Vec<wt::EdgeRules>,
    /// Junction controls per edge index: (at the `to` end, kind, applies).
    controls: Vec<Vec<(bool, ControlKind, wt::Applies)>>,
}

impl WorldRules {
    /// The rules of `package` for its network `net`.
    pub fn of(package: &ocw_world::WorldPackage, net: &RoadNetwork) -> Self {
        Self::from_rules(&RoadRules::of(package), net)
    }

    pub fn from_rules(rules: &RoadRules<'_>, net: &RoadNetwork) -> Self {
        let edges = rules.aligned(net.graph());
        let controls = net
            .graph()
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let len = net.geometry(i).length();
                rules
                    .controls_on(e.id)
                    .iter()
                    .filter(|c| c.at_junction)
                    .map(|c| (f64::from(c.along_m) > len * 0.5, c.kind, c.applies))
                    .collect()
            })
            .collect();
        Self { edges, controls }
    }

    /// The package's rules of edge `e`.
    pub fn world_rule(&self, e: usize) -> &wt::EdgeRules {
        &self.edges[e]
    }
}

fn access(a: wt::Access) -> Access {
    use wt::Vehicle as W;
    let mut out = Access::NONE;
    for (w, cls) in [
        (W::Car, &[VehicleClass::Car, VehicleClass::Van][..]),
        (W::Hgv, &[VehicleClass::Lorry]),
        (W::Bus, &[VehicleClass::Bus]),
        (W::Motorcycle, &[VehicleClass::Motorbike]),
        (W::Tractor, &[VehicleClass::Tractor]),
    ] {
        if a.allows(w) {
            for &c in cls {
                out = out.with(c);
            }
        }
    }
    out
}

impl EdgeRules for WorldRules {
    fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
        let w = &self.edges[edge];
        let class = net.edge(edge).class;
        let d = class_defaults(class);
        let acc = access(w.access);
        let lanes = if acc.is_none() {
            [0, 0]
        } else {
            [
                if w.oneway.allows(true) {
                    w.lanes[0].max(1)
                } else {
                    0
                },
                if w.oneway.allows(false) {
                    w.lanes[1].max(1)
                } else {
                    0
                },
            ]
        };
        let kmh = w.target_kmh(class, true).max(w.target_kmh(class, false));
        EdgeRule {
            oneway: w.oneway,
            lanes,
            speed: f64::from(kmh) / 3.6,
            access: acc,
            aadt: d.aadt * f64::from(w.flow),
            roundabout: w.roundabout,
            rank: d.rank,
        }
    }

    fn node_control(
        &self,
        _net: &RoadNetwork,
        _vertex: VertexId,
        edge: usize,
        forward: bool,
    ) -> Option<Control> {
        self.controls[edge]
            .iter()
            .find(|(at_to, _, applies)| *at_to == forward && applies.includes(forward))
            .map(|(_, kind, _)| match kind {
                ControlKind::Signals | ControlKind::CrossingSignals => Control::Signals,
                ControlKind::Stop => Control::Stop,
                ControlKind::GiveWay => Control::GiveWay,
            })
    }

    fn passing(&self, _net: &RoadNetwork, edge: usize) -> PassRule {
        let w = &self.edges[edge];
        PassRule::new(w.country, w.urban, w.no_overtaking)
    }
}
