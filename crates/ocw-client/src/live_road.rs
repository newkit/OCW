//! The client's live road events (spec 14 Phase 18, 18A, 18D *The* Live
//! traffic data *switch*, 18E T13; Q5): the rider's *Live traffic data*
//! goes to the server (`LiveRoadWish`, after every Welcome, on or off),
//! the server's `LiveRoadUpdate`s come back – whether the world has a
//! source and its credit, and while live the events on the package's
//! edges. The client does not change the traffic itself (the server's
//! simulation already follows the events); it shows the *Live traffic
//! data* row ([`LiveRoadRow`]), the credit while the events apply, and a
//! note when a closure lies on the rider's way ahead.

use ocw_protocol::live_road::{event_from_wire, status, wish};
use ocw_protocol::{encode, v1};
use ocw_sim::traffic::live::{EdgeEvent, LiveKind};
use ocw_world::RidePath;

/// The rider is told of a closure on their way this far ahead (m).
pub const CLOSURE_AHEAD_M: f64 = 1500.0;

/// What the *Live traffic data* row shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LiveRoadRow {
    /// No ride yet, or an older server: nothing known.
    #[default]
    Unknown,
    /// The world has no source: greyed with why.
    NoSource(String),
    /// This server fetches none (the browser's own): greyed with why.
    Unavailable(String),
    /// The world has a source (its credit and what it gives); the rider
    /// may switch it.
    Available { attribution: String, what: String },
    /// On and applied: the source's credit.
    Live { attribution: String, events: usize },
    /// On, nothing applied now: why (down, the host decides, …).
    Waiting { attribution: String, note: String },
}

impl LiveRoadRow {
    /// The world has a source the rider can switch on.
    pub fn switchable(&self) -> bool {
        matches!(
            self,
            Self::Available { .. } | Self::Live { .. } | Self::Waiting { .. }
        )
    }
}

/// The last update as received.
#[derive(Debug, Clone, PartialEq)]
struct Received {
    status: v1::LiveRoadStatus,
    note: String,
    attribution: String,
    what: String,
    graph_hash: u64,
    events: Vec<(EdgeEvent, String)>,
}

/// The live road events of the client.
#[derive(Debug, Clone, Default)]
pub struct LiveRoadClient {
    last: Option<Received>,
    /// What the server was told (`None`: nothing in this session yet).
    told: Option<bool>,
}

impl LiveRoadClient {
    /// A new session: tell the server again, forget the old world's.
    pub fn on_welcome(&mut self) {
        self.told = None;
        self.last = None;
    }

    /// The `LiveRoadWish` to send when the rider's wish changed or the
    /// session is new (also *off*: the answer says whether the world has a
    /// source, for the menu row).
    pub fn wish_message(&mut self, wanted: bool) -> Option<Vec<u8>> {
        if self.told == Some(wanted) {
            return None;
        }
        self.told = Some(wanted);
        Some(encode(&wish(wanted)))
    }

    /// A server's update.
    pub fn on_update(&mut self, u: &v1::LiveRoadUpdate) {
        self.last = Some(Received {
            status: status(u),
            note: u.note.clone(),
            attribution: u.attribution.clone(),
            what: u.what.clone(),
            graph_hash: u.graph_hash,
            events: u
                .events
                .iter()
                .filter_map(|e| Some((event_from_wire(e)?, e.road.clone())))
                .collect(),
        });
    }

    /// The row's state (`wanted`: the rider's setting).
    pub fn row(&self, wanted: bool) -> LiveRoadRow {
        let Some(r) = &self.last else {
            return LiveRoadRow::Unknown;
        };
        let attribution = r.attribution.clone();
        match r.status {
            v1::LiveRoadStatus::NoSource => LiveRoadRow::NoSource(r.note.clone()),
            v1::LiveRoadStatus::Unavailable => LiveRoadRow::Unavailable(r.note.clone()),
            v1::LiveRoadStatus::Live if wanted => LiveRoadRow::Live {
                attribution,
                events: r.events.len(),
            },
            v1::LiveRoadStatus::Down | v1::LiveRoadStatus::Available
                if wanted && !r.note.is_empty() =>
            {
                LiveRoadRow::Waiting {
                    attribution,
                    note: r.note.clone(),
                }
            }
            _ => LiveRoadRow::Available {
                attribution,
                what: r.what.clone(),
            },
        }
    }

    /// The credit to show while live events apply (`None`: none apply).
    pub fn attribution(&self) -> Option<&str> {
        self.last
            .as_ref()
            .filter(|r| r.status == v1::LiveRoadStatus::Live && self.told == Some(true))
            .map(|r| r.attribution.as_str())
            .filter(|a| !a.is_empty())
    }

    /// The events in force (edge events with their road's name), for the
    /// graph `graph_hash` (another graph: none).
    pub fn events(&self, graph_hash: u64) -> Vec<(EdgeEvent, &str)> {
        self.last
            .as_ref()
            .filter(|r| {
                r.status == v1::LiveRoadStatus::Live
                    && (r.graph_hash == graph_hash || r.graph_hash == 0)
            })
            .map(|r| r.events.iter().map(|(e, n)| (*e, n.as_str())).collect())
            .unwrap_or_default()
    }

    /// The note for a closure on the rider's way ahead: the path from
    /// distance `d` on, within [`CLOSURE_AHEAD_M`] (riders may still ride
    /// it – only the cars are kept off).
    pub fn closure_ahead(&self, path: &RidePath, graph_hash: u64, d: f64) -> Option<String> {
        let events = self.events(graph_hash);
        if !events.iter().any(|(e, _)| e.kind == LiveKind::Closed) {
            return None;
        }
        let mut at = d;
        while at <= d + CLOSURE_AHEAD_M {
            let (edge, forward, _) = path.locate(at)?;
            if let Some((_, road)) = events.iter().find(|(e, _)| {
                e.kind == LiveKind::Closed && e.edge as usize == edge && e.dir.holds(forward)
            }) {
                let m = ((at - d) / 50.0).round() * 50.0;
                let road = if road.is_empty() { "the road" } else { road };
                return Some(if m < 50.0 {
                    format!("Road closed: {road} - no cars here (live traffic data)")
                } else {
                    format!(
                        "Road closed ahead: {road} in {m:.0} m - no cars there (live traffic data)"
                    )
                });
            }
            at += 20.0;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::live_road::event_to_wire;
    use ocw_sim::traffic::live::Dir;
    use ocw_world::RoadNetwork;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use std::sync::Arc;

    fn update(status: v1::LiveRoadStatus, events: Vec<v1::LiveRoadEvent>) -> v1::LiveRoadUpdate {
        v1::LiveRoadUpdate {
            status: status as i32,
            attribution: "Fuente: DGT".into(),
            what: "closures, roadworks and reported jams".into(),
            events,
            graph_hash: 7,
            ..Default::default()
        }
    }

    #[test]
    fn the_wish_goes_after_every_welcome_and_the_row_follows_the_server() {
        let mut c = LiveRoadClient::default();
        assert_eq!(c.row(false), LiveRoadRow::Unknown);
        assert!(c.wish_message(false).is_some(), "off too: the row's answer");
        assert!(c.wish_message(false).is_none());
        assert!(c.wish_message(true).is_some());
        c.on_welcome();
        assert!(c.wish_message(true).is_some());
        // Another world: no source.
        c.on_update(&v1::LiveRoadUpdate {
            status: v1::LiveRoadStatus::NoSource as i32,
            note: "no live data for this world yet".into(),
            ..Default::default()
        });
        assert_eq!(
            c.row(true),
            LiveRoadRow::NoSource("no live data for this world yet".into())
        );
        assert!(!c.row(true).switchable());
        // Tenerife: available, then live with its credit.
        c.on_update(&update(v1::LiveRoadStatus::Available, vec![]));
        assert!(c.row(false).switchable());
        let ev = EdgeEvent {
            edge: 3,
            dir: Dir::Forward,
            kind: LiveKind::Closed,
            speed_mps: None,
        };
        c.on_update(&update(
            v1::LiveRoadStatus::Live,
            vec![event_to_wire(&ev, "TF-82", 0)],
        ));
        assert_eq!(
            c.row(true),
            LiveRoadRow::Live {
                attribution: "Fuente: DGT".into(),
                events: 1
            }
        );
        assert_eq!(c.attribution(), Some("Fuente: DGT"));
        assert_eq!(c.events(7).len(), 1);
        assert!(c.events(8).is_empty(), "another graph");
        // Down: why, and no credit.
        c.on_update(&v1::LiveRoadUpdate {
            note: "live data not reachable – using statistics".into(),
            ..update(v1::LiveRoadStatus::Down, vec![])
        });
        assert!(
            matches!(c.row(true), LiveRoadRow::Waiting { ref note, .. } if note.contains("statistics"))
        );
        assert_eq!(c.attribution(), None);
    }

    #[test]
    fn a_closure_on_the_way_ahead_is_told() {
        // Three edges west to east, 500 m each.
        let v = [[0.0, 0.0], [500.0, 0.0], [1000.0, 0.0], [1500.0, 0.0]];
        let edges = (0..3)
            .map(|i| Edge {
                id: EdgeId(i as u64 + 1),
                from: VertexId(i),
                to: VertexId(i + 1),
                points: vec![v[i as usize], v[i as usize + 1]],
                class: RoadClass::Primary,
                name: None,
                width_m: 7.0,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: i64::from(i),
                profile: Vec::new(),
            })
            .collect();
        let net = Arc::new(RoadNetwork::new(RoadGraph {
            vertices: v.to_vec(),
            edges,
            boundary_vertices: Default::default(),
        }));
        let segs: Vec<ocw_world::Segment> = (0..3)
            .map(|edge| ocw_world::Segment::Edge {
                edge,
                from_m: 0.0,
                to_m: 500.0,
            })
            .collect();
        let path = RidePath::from_segments(net.clone(), &segs, 0.0, 0).expect("path");
        let mut c = LiveRoadClient::default();
        c.wish_message(true);
        let closed = |dir| {
            update(
                v1::LiveRoadStatus::Live,
                vec![event_to_wire(
                    &EdgeEvent {
                        edge: 2,
                        dir,
                        kind: LiveKind::Closed,
                        speed_mps: None,
                    },
                    "TF-82",
                    0,
                )],
            )
        };
        c.on_update(&closed(Dir::Both));
        let note = c.closure_ahead(&path, 7, 0.0).expect("told");
        assert!(note.contains("TF-82") && note.contains("ahead"), "{note}");
        assert!(note.contains("1000 m"), "{note}");
        // On it.
        let on = c.closure_ahead(&path, 7, 1100.0).expect("on it");
        assert!(on.starts_with("Road closed: TF-82"), "{on}");
        // The other direction, or another graph: nothing.
        c.on_update(&closed(Dir::Backward));
        assert!(c.closure_ahead(&path, 7, 0.0).is_none());
        c.on_update(&closed(Dir::Forward));
        assert!(c.closure_ahead(&path, 8, 0.0).is_none());
    }
}
