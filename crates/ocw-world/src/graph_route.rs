//! Routes on the road graph (spec §2.7, §4.7): an ordered list of edges
//! with their riding direction, referenced by **stable edge ID** (§4.4),
//! so a route survives a package rebuild; explicit U-turns for
//! out-and-back routes (§2.2.2); a loop flag for laps.
//!
//! - [`GraphRoute`] is the stored form (package section `ROUT`, or a
//!   standalone `ocw-graph-route 1` text file naming its package).
//! - [`RoutePlan`] is the same route resolved against a [`RoadNetwork`]
//!   (edge indices), checked for continuity; riders follow it with the
//!   free-ride path machinery ([`crate::RidePath::with_plan`]).
//!
//! Text format:
//!
//! ```text
//! ocw-graph-route 1
//! name Vaduz loop
//! package Liechtenstein 9f3c2a01b2c3d4e5   # package name and graph fingerprint (hex)
//! loop yes                                 # optional: the end joins the start (laps)
//! edge 1234567890 0.0 312.5                # stable ID, from / to metres from its first vertex
//! turn                                     # a U-turn where the previous edge step ends
//! edge 1234567890 312.5 0.0
//! ```

use crate::graph::EdgeId;
use crate::network::RoadNetwork;
use crate::path::{Segment, TurnReason};

/// One step of a stored route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RouteStep {
    /// Along the edge with stable ID `id` from `from_m` to `to_m` (metres
    /// from its first vertex; `from_m > to_m` = against its direction).
    Edge { id: EdgeId, from_m: f64, to_m: f64 },
    /// A U-turn where the previous step ends.
    Turnaround,
}

/// A route as stored in a package or a route file.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphRoute {
    pub name: String,
    pub steps: Vec<RouteStep>,
    /// The last step ends where the first starts: ridden in laps.
    pub is_loop: bool,
}

/// Why a stored route does not fit a road graph (e.g. after a rebuild from
/// newer OSM data, spec §4.4: "the world builder flags any that break").
#[derive(Debug, Clone, PartialEq)]
pub enum RouteBreak {
    /// The route has no edge step.
    Empty,
    /// Step `step` names an edge the graph does not have.
    MissingEdge { step: usize, id: EdgeId },
    /// Step `step` does not start where the previous one ends.
    Gap { step: usize },
    /// A U-turn that is not followed by the same edge the other way.
    BadTurn { step: usize },
}

impl std::fmt::Display for RouteBreak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "the route has no edges"),
            Self::MissingEdge { step, id } => {
                write!(f, "step {step}: edge {} is not in the road graph", id.0)
            }
            Self::Gap { step } => {
                write!(f, "step {step} does not start where the previous step ends")
            }
            Self::BadTurn { step } => write!(
                f,
                "step {step}: a U-turn must be followed by the same edge the other way"
            ),
        }
    }
}

/// Metres two route positions may differ and still join (rounding).
const JOIN_TOLERANCE_M: f64 = 0.5;

impl GraphRoute {
    /// Resolves the route against `net`: edge indices, continuity checked.
    pub fn resolve(&self, net: &RoadNetwork) -> Result<RoutePlan, RouteBreak> {
        let mut steps = Vec::with_capacity(self.steps.len());
        for (i, s) in self.steps.iter().enumerate() {
            steps.push(match *s {
                RouteStep::Edge { id, from_m, to_m } => {
                    let edge = net
                        .edge_index(id)
                        .ok_or(RouteBreak::MissingEdge { step: i, id })?;
                    let len = net.geometry(edge).length();
                    Segment::Edge {
                        edge,
                        from_m: from_m.clamp(0.0, len),
                        to_m: to_m.clamp(0.0, len),
                    }
                }
                RouteStep::Turnaround => Segment::Turnaround(TurnReason::Manual),
            });
        }
        RoutePlan::new(net, self.name.clone(), steps, self.is_loop)
    }

    /// The stored form of a plan on `net`.
    pub fn from_plan(plan: &RoutePlan, net: &RoadNetwork) -> Self {
        Self {
            name: plan.name.clone(),
            steps: plan
                .steps
                .iter()
                .map(|s| match *s {
                    Segment::Edge { edge, from_m, to_m } => RouteStep::Edge {
                        id: net.edge(edge).id,
                        from_m,
                        to_m,
                    },
                    Segment::Turnaround(_) => RouteStep::Turnaround,
                })
                .collect(),
            is_loop: plan.is_loop,
        }
    }

    /// The text form; `package` names the package and its graph
    /// fingerprint ([`crate::graph::RoadGraph::fingerprint`]).
    pub fn to_text(&self, package: Option<(&str, u64)>) -> String {
        let mut s = format!("ocw-graph-route 1\nname {}\n", self.name.replace('\n', " "));
        if let Some((name, fp)) = package {
            s.push_str(&format!("package {} {fp:016x}\n", name.replace(' ', "_")));
        }
        if self.is_loop {
            s.push_str("loop yes\n");
        }
        for step in &self.steps {
            match step {
                RouteStep::Edge { id, from_m, to_m } => {
                    s.push_str(&format!("edge {} {from_m:.2} {to_m:.2}\n", id.0));
                }
                RouteStep::Turnaround => s.push_str("turn\n"),
            }
        }
        s
    }

    /// Parses the text form; returns the route and the package line (name,
    /// fingerprint) if present.
    pub fn parse(text: &str) -> Result<(Self, Option<(String, u64)>), String> {
        let mut lines = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty());
        if lines.next() != Some("ocw-graph-route 1") {
            return Err("not an ocw-graph-route 1 file".into());
        }
        let mut route = GraphRoute {
            name: String::new(),
            steps: Vec::new(),
            is_loop: false,
        };
        let mut package = None;
        for line in lines {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            match key {
                "name" => route.name = rest.to_string(),
                "package" => {
                    let (n, fp) = rest
                        .rsplit_once(' ')
                        .ok_or_else(|| format!("expected `package NAME HASH`: {line}"))?;
                    let fp = u64::from_str_radix(fp.trim(), 16)
                        .map_err(|e| format!("package hash {fp}: {e}"))?;
                    package = Some((n.trim().replace('_', " "), fp));
                }
                "loop" => route.is_loop = matches!(rest, "yes" | "true" | "1"),
                "turn" => route.steps.push(RouteStep::Turnaround),
                "edge" => {
                    let v: Vec<&str> = rest.split_whitespace().collect();
                    let [id, a, b] = v[..] else {
                        return Err(format!("expected `edge ID FROM TO`: {line}"));
                    };
                    let num = |t: &str| t.parse::<f64>().map_err(|e| format!("{t}: {e}"));
                    route.steps.push(RouteStep::Edge {
                        id: EdgeId(id.parse().map_err(|e| format!("{id}: {e}"))?),
                        from_m: num(a)?,
                        to_m: num(b)?,
                    });
                }
                _ => return Err(format!("unknown line: {line}")),
            }
        }
        if route.steps.is_empty() {
            return Err("the route has no edges".into());
        }
        Ok((route, package))
    }
}

/// A route resolved against a road network: what riders follow.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutePlan {
    pub name: String,
    /// Edge steps (edge indices) and U-turns, continuous; the first may
    /// start and the last end part-way along an edge.
    pub steps: Vec<Segment>,
    pub is_loop: bool,
    /// Route distance where each step starts, and the total length.
    starts_m: Vec<f64>,
    length_m: f64,
}

impl RoutePlan {
    /// A plan from resolved steps; checks that each step starts where the
    /// previous one ends (and U-turns reverse the same edge).
    pub fn new(
        net: &RoadNetwork,
        name: impl Into<String>,
        steps: Vec<Segment>,
        is_loop: bool,
    ) -> Result<Self, RouteBreak> {
        if !steps.iter().any(|s| matches!(s, Segment::Edge { .. })) {
            return Err(RouteBreak::Empty);
        }
        if matches!(steps[0], Segment::Turnaround(_)) {
            return Err(RouteBreak::BadTurn { step: 0 });
        }
        for i in 1..steps.len() {
            let ok = match (steps[i - 1], steps[i]) {
                (Segment::Edge { .. }, Segment::Turnaround(_)) => {
                    matches!(steps.get(i + 1), Some(Segment::Edge { .. }))
                }
                (Segment::Turnaround(_), Segment::Edge { edge, from_m, to_m }) => {
                    let Some(Segment::Edge {
                        edge: pe,
                        from_m: pf,
                        to_m: pt,
                    }) = i.checked_sub(2).map(|k| steps[k])
                    else {
                        return Err(RouteBreak::BadTurn { step: i });
                    };
                    let reversed = (pt >= pf) != (to_m >= from_m);
                    if pe != edge || !reversed || (from_m - pt).abs() > JOIN_TOLERANCE_M {
                        return Err(RouteBreak::BadTurn { step: i });
                    }
                    true
                }
                (Segment::Turnaround(_), Segment::Turnaround(_)) => false,
                (a @ Segment::Edge { .. }, b @ Segment::Edge { .. }) => joins(net, a, b),
            };
            if !ok {
                return Err(match steps[i] {
                    Segment::Turnaround(_) => RouteBreak::BadTurn { step: i },
                    Segment::Edge { .. } => RouteBreak::Gap { step: i },
                });
            }
        }
        if matches!(steps.last(), Some(Segment::Turnaround(_))) {
            return Err(RouteBreak::BadTurn {
                step: steps.len() - 1,
            });
        }
        let mut starts_m = Vec::with_capacity(steps.len());
        let mut at = 0.0;
        for s in &steps {
            starts_m.push(at);
            if let Segment::Edge { from_m, to_m, .. } = s {
                at += (to_m - from_m).abs();
            }
        }
        Ok(Self {
            name: name.into(),
            steps,
            is_loop,
            starts_m,
            length_m: at,
        })
    }

    /// Length of one lap (metres).
    pub fn length_m(&self) -> f64 {
        self.length_m
    }

    /// Total climbing of one lap (m): the sum of the rises of the road
    /// profile along the steps, sampled every [`crate::graph::PROFILE_STEP_M`];
    /// 0 for edges without a profile.
    pub fn elevation_gain_m(&self, net: &RoadNetwork) -> f64 {
        let mut gain = 0.0;
        for s in &self.steps {
            let Segment::Edge { edge, from_m, to_m } = *s else {
                continue;
            };
            let e = net.edge(edge);
            let len = (to_m - from_m).abs();
            let n = (len / crate::graph::PROFILE_STEP_M).ceil().max(1.0) as usize;
            let mut last: Option<f64> = None;
            for k in 0..=n {
                let d = from_m + (to_m - from_m) * k as f64 / n as f64;
                let Some(h) = e.height_at(d) else { break };
                if let Some(l) = last
                    && h > l
                {
                    gain += h - l;
                }
                last = Some(h);
            }
        }
        gain
    }

    /// Route distance where step `i` starts.
    pub fn start_of(&self, i: usize) -> f64 {
        self.starts_m.get(i).copied().unwrap_or(self.length_m)
    }

    /// Step `i` as ridden: on later laps the first step is the whole edge
    /// (the lap starts at its vertex).
    pub fn step(&self, net: &RoadNetwork, i: usize, lap: u32) -> Option<Segment> {
        let s = *self.steps.get(i)?;
        Some(match s {
            Segment::Edge { edge, from_m, to_m } if i == 0 && lap > 0 => {
                Segment::whole(net, edge, to_m >= from_m)
            }
            s => s,
        })
    }

    /// The first step and its direction (where riders start).
    pub fn start(&self) -> (usize, f64, bool) {
        match self.steps[0] {
            Segment::Edge { edge, from_m, to_m } => (edge, from_m, to_m >= from_m),
            Segment::Turnaround(_) => unreachable!("checked in new"),
        }
    }

    /// Points (x, z) along the route every `step_m` metres (and at every
    /// step's ends), for re-matching it onto a rebuilt graph or drawing it.
    pub fn points(&self, net: &RoadNetwork, step_m: f64) -> Vec<[f64; 2]> {
        let mut out: Vec<[f64; 2]> = Vec::new();
        for s in &self.steps {
            let Segment::Edge { edge, from_m, to_m } = *s else {
                continue;
            };
            let g = net.geometry(edge);
            let len = (to_m - from_m).abs();
            let n = (len / step_m.max(0.5)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let p = g.point_at(from_m + (to_m - from_m) * t);
                let q = [p[0], p[2]];
                if out
                    .last()
                    .is_none_or(|l| (l[0] - q[0]).hypot(l[1] - q[1]) > 0.01)
                {
                    out.push(q);
                }
            }
        }
        out
    }

    /// Whether the end joins the start (a loop can be ridden in laps).
    pub fn closes(&self, net: &RoadNetwork) -> bool {
        let first = self.steps[0];
        let last = *self.steps.last().expect("not empty");
        let Segment::Edge {
            edge, from_m, to_m, ..
        } = first
        else {
            return false;
        };
        let start_v = net.start_vertex(edge, to_m >= from_m);
        match last {
            Segment::Edge {
                edge: le,
                from_m: lf,
                to_m: lt,
            } => {
                let len = net.geometry(le).length();
                let at_end = if lt >= lf {
                    lt >= len - JOIN_TOLERANCE_M
                } else {
                    lt <= JOIN_TOLERANCE_M
                };
                at_end && net.end_vertex(le, lt >= lf) == start_v
            }
            Segment::Turnaround(_) => false,
        }
    }
}

/// Whether edge step `b` starts where edge step `a` ends: at a shared
/// vertex (both at their edge's ends) or at the same point of one edge.
fn joins(net: &RoadNetwork, a: Segment, b: Segment) -> bool {
    let (
        Segment::Edge {
            edge: ea,
            from_m: fa,
            to_m: ta,
        },
        Segment::Edge {
            edge: eb,
            from_m: fb,
            to_m: tb,
        },
    ) = (a, b)
    else {
        return false;
    };
    if ea == eb && (tb >= fb) == (ta >= fa) && (fb - ta).abs() <= JOIN_TOLERANCE_M {
        return true;
    }
    let (la, lb) = (net.geometry(ea).length(), net.geometry(eb).length());
    let a_end = if ta >= fa {
        (ta >= la - JOIN_TOLERANCE_M).then(|| net.edge(ea).to)
    } else {
        (ta <= JOIN_TOLERANCE_M).then(|| net.edge(ea).from)
    };
    let b_start = if tb >= fb {
        (fb <= JOIN_TOLERANCE_M).then(|| net.edge(eb).from)
    } else {
        (fb >= lb - JOIN_TOLERANCE_M).then(|| net.edge(eb).to)
    };
    matches!((a_end, b_start), (Some(x), Some(y)) if x == y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::tests::crossroads;

    fn net() -> RoadNetwork {
        RoadNetwork::new(crossroads())
    }

    /// West arm inwards, then east: 150 m + 200 m.
    fn west_to_east() -> GraphRoute {
        GraphRoute {
            name: "W-E".into(),
            steps: vec![
                RouteStep::Edge {
                    id: EdgeId(1000),
                    from_m: 150.0,
                    to_m: 0.0,
                },
                RouteStep::Edge {
                    id: EdgeId(1001),
                    from_m: 0.0,
                    to_m: 200.0,
                },
            ],
            is_loop: false,
        }
    }

    #[test]
    fn climbing_sums_the_rises_of_the_profile() {
        let mut g = crossroads();
        // The west arm rises 10 m outwards from the centre, so riding it
        // inwards (150 m to 0) descends; the east arm (200 m) rises 6 m,
        // then falls 2 m.
        for e in &mut g.edges {
            let n = (e.length_m() / crate::graph::PROFILE_STEP_M).ceil() as usize + 1;
            e.profile = match e.id.0 {
                1000 => (0..n)
                    .map(|i| 100.0 + 10.0 * i as f64 / (n - 1) as f64)
                    .collect(),
                1001 => (0..n)
                    .map(|i| {
                        let t = i as f64 / (n - 1) as f64;
                        100.0
                            + if t < 0.75 {
                                8.0 * t
                            } else {
                                6.0 - 8.0 * (t - 0.75)
                            }
                    })
                    .collect(),
                _ => Vec::new(),
            };
        }
        let n = RoadNetwork::new(g);
        let plan = west_to_east().resolve(&n).unwrap();
        let gain = plan.elevation_gain_m(&n);
        assert!((gain - 6.0).abs() < 0.2, "{gain}");
        // Without profiles: no climbing known.
        assert_eq!(
            west_to_east()
                .resolve(&net())
                .unwrap()
                .elevation_gain_m(&net()),
            0.0
        );
    }

    #[test]
    fn routes_resolve_by_stable_id() {
        let n = net();
        let plan = west_to_east().resolve(&n).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.length_m(), 350.0);
        assert_eq!(plan.start_of(1), 150.0);
        assert_eq!(plan.start(), (0, 150.0, false));
        assert!(!plan.closes(&n));
        // Back to the stored form.
        assert_eq!(GraphRoute::from_plan(&plan, &n), west_to_east());
    }

    #[test]
    fn broken_routes_are_reported() {
        let n = net();
        let mut r = west_to_east();
        r.steps[1] = RouteStep::Edge {
            id: EdgeId(77),
            from_m: 0.0,
            to_m: 1.0,
        };
        assert_eq!(
            r.resolve(&n),
            Err(RouteBreak::MissingEdge {
                step: 1,
                id: EdgeId(77)
            })
        );
        // East arm ridden inwards does not start at the centre.
        let mut gap = west_to_east();
        gap.steps[1] = RouteStep::Edge {
            id: EdgeId(1001),
            from_m: 200.0,
            to_m: 0.0,
        };
        assert_eq!(gap.resolve(&n), Err(RouteBreak::Gap { step: 1 }));
        assert!(
            RouteBreak::Gap { step: 1 }
                .to_string()
                .contains("does not start")
        );
        let empty = GraphRoute {
            name: "e".into(),
            steps: vec![],
            is_loop: false,
        };
        assert_eq!(empty.resolve(&n), Err(RouteBreak::Empty));
    }

    #[test]
    fn out_and_back_needs_an_explicit_turn() {
        let n = net();
        let out_back = |turn: bool| {
            let mut steps = vec![RouteStep::Edge {
                id: EdgeId(1001),
                from_m: 0.0,
                to_m: 120.0,
            }];
            if turn {
                steps.push(RouteStep::Turnaround);
            }
            steps.push(RouteStep::Edge {
                id: EdgeId(1001),
                from_m: 120.0,
                to_m: 0.0,
            });
            GraphRoute {
                name: "ob".into(),
                steps,
                is_loop: false,
            }
        };
        let plan = out_back(true).resolve(&n).unwrap();
        assert_eq!(plan.length_m(), 240.0);
        assert!(plan.closes(&n), "back at the centre");
        assert_eq!(
            out_back(false).resolve(&n),
            Err(RouteBreak::Gap { step: 1 })
        );
        // A turn followed by another edge is refused.
        let mut bad = out_back(true);
        bad.steps[2] = RouteStep::Edge {
            id: EdgeId(1002),
            from_m: 0.0,
            to_m: 50.0,
        };
        assert_eq!(bad.resolve(&n), Err(RouteBreak::BadTurn { step: 2 }));
    }

    #[test]
    fn text_round_trip() {
        let mut r = west_to_east();
        r.steps.insert(1, RouteStep::Turnaround);
        r.steps.insert(
            2,
            RouteStep::Edge {
                id: EdgeId(1000),
                from_m: 0.0,
                to_m: 10.5,
            },
        );
        r.is_loop = true;
        let text = r.to_text(Some(("Liechtenstein test", 0xabc)));
        assert!(text.contains("package Liechtenstein_test 0000000000000abc"));
        let (back, pkg) = GraphRoute::parse(&text).unwrap();
        assert_eq!(back, r);
        assert_eq!(pkg, Some(("Liechtenstein test".into(), 0xabc)));
        assert!(GraphRoute::parse("ocw-route 1\n").is_err());
        assert!(GraphRoute::parse("ocw-graph-route 1\nname x\n").is_err());
        assert!(GraphRoute::parse("ocw-graph-route 1\nedge 1 2\n").is_err());
        assert!(GraphRoute::parse("ocw-graph-route 1\nfoo\n").is_err());
    }
}
