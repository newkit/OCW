//! The Routes workspace's document (spec §14 Phase 16, 16D): a route on a
//! package's road graph, made of **via-points** and the **legs** between
//! them (edge steps with stable edge ids, §2.7), plus what it was imported
//! from (the raw track, its problem sections).
//!
//! **Commands.** Every edit replaces the route's geometry (and, for an
//! import, the raw track and its problems) as one [`RouteOp`] whose
//! inverse is the geometry before – exact, so undo / redo, dirty flags and
//! autosave work unchanged. The [`Editor`] turns what the user does into
//! those operations on the road graph: add, insert, move and remove
//! via-points, **drag the route** (a via-point is inserted where it was
//! grabbed and the leg between its neighbours re-planned), reverse, loop,
//! out-and-back, cut the start or end, split and join, re-plan with
//! another profile. Legs whose two via-points did not change are kept as
//! they were (an imported track stays as matched until it is edited
//! there); the others are planned with the built-in planner
//! (`ocw_world::plan`).
//!
//! **Validators** ([`route_validators`]): continuity on the graph (every
//! edge id in the graph, the legs joined, one-way roads their way, a loop
//! that closes), the via-points on the road, and the import's problem
//! sections (off the graph, no road nearby, a recording gap, outside the
//! package).

pub mod formats;

use std::sync::Arc;

use ocw_world::geo::LocalProjection;
use ocw_world::graph_route::{GraphRoute, RoutePlan};
use ocw_world::matching::{MatchError, MatchParams, match_track};
use ocw_world::network::RoadNetwork;
use ocw_world::path::Segment;
use ocw_world::plan::{EdgeCosts, Snap, join_legs, plan_leg, snap, steps_points};
use ocw_world::road_surface::RoadSurface;
use serde::{Deserialize, Serialize};

use crate::document::{DocumentKind, EditError};
use crate::validate::{Finding, Validator};

/// The package (and its graph build) a route is on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutePackage {
    /// Manifest id (`local/<name>`).
    pub id: String,
    /// The package's name (graph route files name it).
    pub name: String,
    /// `RoadGraph::fingerprint` of the graph the edge ids are from.
    pub fingerprint: u64,
    /// The package's origin (lat, lon): local metres ↔ degrees.
    pub origin: (f64, f64),
}

/// A via-point: where it was put (x, z) and where that is on the road.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Via {
    pub at: [f64; 2],
    /// Stable edge id and metres from its first vertex.
    pub edge: u64,
    pub s: f64,
}

/// One step of a leg (a stored `RouteStep`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Step {
    Edge { id: u64, from_m: f64, to_m: f64 },
    Turn,
}

/// The route's shape: via-points and the legs between them (one fewer
/// than via-points, one more when `closed`: the leg back to the start).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub vias: Vec<Via>,
    pub legs: Vec<Vec<Step>>,
    /// A loop: the last leg leads back to the first via-point (laps).
    pub closed: bool,
    /// Who planned it ("Routing: BRouter, …"), shown and exported with it.
    pub credits: Vec<String>,
}

impl Geometry {
    /// How many legs the via-points need.
    pub fn legs_needed(&self) -> usize {
        match self.vias.len() {
            0 | 1 => 0,
            n => n - 1 + usize::from(self.closed),
        }
    }

    /// The via-points leg `k` runs between.
    pub fn leg_ends(&self, k: usize) -> Option<(Via, Via)> {
        let n = self.vias.len();
        (k < self.legs_needed()).then(|| (self.vias[k], self.vias[(k + 1) % n]))
    }

    /// The same route the other way round.
    pub fn reversed(&self) -> Geometry {
        let mut vias = self.vias.clone();
        vias.reverse();
        let n = self.legs.len();
        let open = self.legs_needed().saturating_sub(usize::from(self.closed));
        let mut legs: Vec<Vec<Step>> = Vec::with_capacity(n);
        for k in (0..open).rev() {
            legs.push(reverse_steps(&self.legs[k]));
        }
        if self.closed && n > open {
            legs.push(reverse_steps(&self.legs[open]));
        }
        Geometry {
            vias,
            legs,
            closed: self.closed,
            credits: self.credits.clone(),
        }
    }
}

fn reverse_steps(steps: &[Step]) -> Vec<Step> {
    steps
        .iter()
        .rev()
        .map(|s| match *s {
            Step::Edge { id, from_m, to_m } => Step::Edge {
                id,
                from_m: to_m,
                to_m: from_m,
            },
            Step::Turn => Step::Turn,
        })
        .collect()
}

/// A point of an imported track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackPoint {
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
}

/// The track a route was imported or planned from (drawn grey beside it,
/// its elevation the profile's comparison line).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: Option<String>,
    /// "GPX", "TCX", "GeoJSON", "FIT", "BRouter".
    pub format: String,
    pub points: Vec<TrackPoint>,
}

impl Track {
    pub fn of_gpx(g: &ocw_world::gpx::Gpx, format: &str) -> Self {
        Self {
            name: g.name.clone(),
            format: format.into(),
            points: g
                .points
                .iter()
                .map(|p| TrackPoint {
                    lat: p.lat,
                    lon: p.lon,
                    ele: p.ele,
                })
                .collect(),
        }
    }

    pub fn to_gpx(&self) -> ocw_world::gpx::Gpx {
        ocw_world::gpx::Gpx {
            name: self.name.clone(),
            points: self
                .points
                .iter()
                .map(|p| ocw_world::gpx::GpxPoint {
                    lat: p.lat,
                    lon: p.lon,
                    ele: p.ele,
                })
                .collect(),
            segments: Vec::new(),
        }
    }

    /// Distance along the track (m) and its elevation, where it has one.
    pub fn elevation(&self) -> Vec<[f64; 2]> {
        let gpx = self.to_gpx();
        ocw_world::formats::distances(&gpx.points)
            .into_iter()
            .zip(&self.points)
            .filter_map(|(d, p)| Some([d, p.ele?]))
            .collect()
    }
}

/// What kind of problem a section of the imported track has (16D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProblemKind {
    /// Matched, but further from the road than GPS noise explains.
    OffGraph,
    /// No road of the package nearby: skipped by the matcher.
    Unmatched,
    /// A jump between two recorded points (the recording paused).
    Gap,
    /// Outside the package: another package is needed (*Which world?*).
    MissingPackage,
    /// The roads do not connect the track here: not matched at all.
    NotMatched,
}

/// A problem section of the imported track (point indices, inclusive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub kind: ProblemKind,
    pub first: usize,
    pub last: usize,
    /// How far from the road (m), or the gap's length.
    pub metres: Option<f64>,
}

impl Problem {
    pub fn message(&self) -> String {
        let pts = if self.first == self.last {
            format!("point {}", self.first + 1)
        } else {
            format!("points {}-{}", self.first + 1, self.last + 1)
        };
        match self.kind {
            ProblemKind::OffGraph => format!(
                "off the road graph: {pts} (up to {:.0} m away)",
                self.metres.unwrap_or(0.0)
            ),
            ProblemKind::Unmatched => format!("no road nearby: {pts} (not matched)"),
            ProblemKind::Gap => format!(
                "a gap in the recording: {pts} ({:.0} m without points)",
                self.metres.unwrap_or(0.0)
            ),
            ProblemKind::MissingPackage => {
                format!("outside this package: {pts} (another world package is needed)")
            }
            ProblemKind::NotMatched => {
                format!("the package's roads do not connect the track near {pts}")
            }
        }
    }
}

/// The Routes workspace's document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RouteDoc {
    pub package: RoutePackage,
    pub name: String,
    pub description: String,
    /// The built-in planner's profile (`ocw_world::plan::Profile` id).
    pub profile: String,
    pub geometry: Geometry,
    /// The imported (or externally planned) track, if any.
    pub track: Option<Track>,
    pub problems: Vec<Problem>,
}

impl RouteDoc {
    pub fn new(package: RoutePackage, name: impl Into<String>) -> Self {
        Self {
            package,
            name: name.into(),
            profile: ocw_world::plan::Profile::default().id().into(),
            ..Default::default()
        }
    }

    pub fn projection(&self) -> LocalProjection {
        LocalProjection::new(self.package.origin.0, self.package.origin.1)
    }

    /// The route resolved on `net` (a loop if it is closed and closes).
    pub fn plan(&self, net: &RoadNetwork) -> Result<RoutePlan, String> {
        let legs = self
            .geometry
            .legs
            .iter()
            .enumerate()
            .map(|(k, l)| {
                to_segments(net, l)
                    .map_err(|id| format!("leg {}: edge {id} is not in the road graph", k + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut steps = join_legs(&legs);
        if self.geometry.closed {
            close_at_vertex(net, &mut steps);
        }
        let plan = RoutePlan::new(net, self.name.clone(), steps, false)
            .map_err(|e| format!("the route is not continuous: {e}"))?;
        if self.geometry.closed && plan.closes(net) {
            RoutePlan::new(net, self.name.clone(), plan.steps.clone(), true)
                .map_err(|e| e.to_string())
        } else {
            Ok(plan)
        }
    }

    /// The stored graph route.
    pub fn graph_route(&self, net: &RoadNetwork) -> Result<GraphRoute, String> {
        Ok(GraphRoute::from_plan(&self.plan(net)?, net))
    }
}

/// A loop is ridden in laps from a junction (`RoutePlan::closes`): when
/// the way back reaches the first via-point's road through its junction,
/// in the riding direction, the start moves back to that junction (the
/// last step joins the first).
fn close_at_vertex(net: &RoadNetwork, steps: &mut Vec<Segment>) {
    if steps.len() < 2 {
        return;
    }
    let (
        Segment::Edge {
            edge: fe,
            from_m: ff,
            to_m: ft,
        },
        Segment::Edge {
            edge: le,
            from_m: lf,
            to_m: lt,
        },
    ) = (steps[0], steps[steps.len() - 1])
    else {
        return;
    };
    let len = net.geometry(fe).length();
    if fe == le
        && (lt - ff).abs() < 0.5
        && (lt >= lf) == (ft >= ff)
        && (lf <= 0.5 || lf >= len - 0.5)
    {
        steps.pop();
        steps[0] = Segment::Edge {
            edge: fe,
            from_m: lf,
            to_m: ft,
        };
    }
}

/// The document's commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RouteOp {
    SetName(String),
    SetDescription(String),
    /// A new shape (and maybe profile, track and problems) of the route.
    Set(Box<RouteSet>),
}

/// What [`RouteOp::Set`] sets; `None`: unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteSet {
    pub geometry: Geometry,
    pub profile: Option<String>,
    pub imported: Option<Imported>,
    pub edit: Edit,
}

/// An import's track and problems.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub track: Option<Track>,
    pub problems: Vec<Problem>,
}

/// Which user action a [`RouteSet`] is (its undo name; drags merge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    AddVia,
    InsertVia(usize),
    MoveVia(usize),
    RemoveVia(usize),
    /// The route grabbed and a via-point inserted here.
    Drag(usize),
    Reverse,
    Loop(bool),
    OutAndBack,
    CutStart(usize),
    CutEnd(usize),
    Split(usize),
    Join,
    Replan,
    Import,
    Plan,
    Clear,
}

impl DocumentKind for RouteDoc {
    type Op = RouteOp;
    /// A via-point's index.
    type Item = usize;
    const KIND: &'static str = "route";

    fn apply(&mut self, op: &RouteOp) -> Result<RouteOp, EditError> {
        Ok(match op {
            RouteOp::SetName(n) => RouteOp::SetName(std::mem::replace(&mut self.name, n.clone())),
            RouteOp::SetDescription(d) => {
                RouteOp::SetDescription(std::mem::replace(&mut self.description, d.clone()))
            }
            RouteOp::Set(set) => {
                let g = &set.geometry;
                if g.vias.len() >= 2 && g.legs.len() != g.legs_needed() {
                    return Err(EditError::new(format!(
                        "{} via-points need {} legs, not {}",
                        g.vias.len(),
                        g.legs_needed(),
                        g.legs.len()
                    )));
                }
                let geometry = std::mem::replace(&mut self.geometry, set.geometry.clone());
                let profile = set
                    .profile
                    .as_ref()
                    .map(|p| std::mem::replace(&mut self.profile, p.clone()));
                let imported = set.imported.as_ref().map(|i| Imported {
                    track: std::mem::replace(&mut self.track, i.track.clone()),
                    problems: std::mem::replace(&mut self.problems, i.problems.clone()),
                });
                RouteOp::Set(Box::new(RouteSet {
                    geometry,
                    profile,
                    imported,
                    edit: set.edit,
                }))
            }
        })
    }

    fn label(op: &RouteOp) -> String {
        match op {
            RouteOp::SetName(_) => "Rename route".into(),
            RouteOp::SetDescription(_) => "Edit description".into(),
            RouteOp::Set(s) => match s.edit {
                Edit::AddVia => "Add via-point".into(),
                Edit::InsertVia(i) => format!("Insert via-point {}", i + 1),
                Edit::MoveVia(i) => format!("Move via-point {}", i + 1),
                Edit::RemoveVia(i) => format!("Remove via-point {}", i + 1),
                Edit::Drag(_) => "Drag route".into(),
                Edit::Reverse => "Reverse route".into(),
                Edit::Loop(true) => "Make a loop".into(),
                Edit::Loop(false) => "Open the loop".into(),
                Edit::OutAndBack => "Out and back".into(),
                Edit::CutStart(_) => "Cut the start".into(),
                Edit::CutEnd(_) => "Cut the end".into(),
                Edit::Split(_) => "Split route".into(),
                Edit::Join => "Join routes".into(),
                Edit::Replan => "Re-plan route".into(),
                Edit::Import => "Import track".into(),
                Edit::Plan => "Plan route".into(),
                Edit::Clear => "Clear route".into(),
            },
        }
    }

    fn merges(prev: &RouteOp, next: &RouteOp) -> bool {
        match (prev, next) {
            (RouteOp::SetName(_), RouteOp::SetName(_))
            | (RouteOp::SetDescription(_), RouteOp::SetDescription(_)) => true,
            (RouteOp::Set(a), RouteOp::Set(b)) => matches!(
                (a.edit, b.edit),
                (Edit::MoveVia(i) | Edit::Drag(i), Edit::MoveVia(j)) if i == j
            ),
            _ => false,
        }
    }

    fn contains(&self, item: &usize) -> bool {
        *item < self.geometry.vias.len()
    }
}

/// Steps of a leg on `net` (edge indices); the id of a missing edge.
pub fn to_segments(net: &RoadNetwork, steps: &[Step]) -> Result<Vec<Segment>, u64> {
    steps
        .iter()
        .map(|s| match *s {
            Step::Edge { id, from_m, to_m } => {
                let edge = net.edge_index(ocw_world::graph::EdgeId(id)).ok_or(id)?;
                let len = net.geometry(edge).length();
                Ok(Segment::Edge {
                    edge,
                    from_m: from_m.clamp(0.0, len),
                    to_m: to_m.clamp(0.0, len),
                })
            }
            Step::Turn => Ok(Segment::Turnaround(ocw_world::TurnReason::Manual)),
        })
        .collect()
}

/// The stored steps of segments on `net`.
pub fn to_steps(net: &RoadNetwork, segs: &[Segment]) -> Vec<Step> {
    segs.iter()
        .map(|s| match *s {
            Segment::Edge { edge, from_m, to_m } => Step::Edge {
                id: net.edge(edge).id.0,
                from_m,
                to_m,
            },
            Segment::Turnaround(_) => Step::Turn,
        })
        .collect()
}

/// Splits steps at route distances `cuts` (ascending, within the route):
/// `cuts.len() + 1` pieces, and the (edge, metres) of every cut.
pub fn split_steps(steps: &[Segment], cuts: &[f64]) -> (Vec<Vec<Segment>>, Vec<(usize, f64)>) {
    let mut pieces: Vec<Vec<Segment>> = vec![Vec::new()];
    let mut at_cut: Vec<(usize, f64)> = Vec::new();
    let mut cuts = cuts.iter().copied().peekable();
    let mut d = 0.0;
    let mut last_edge: Option<(usize, f64)> = None;
    for s in steps {
        let Segment::Edge { edge, from_m, to_m } = *s else {
            pieces.last_mut().expect("one").push(*s);
            continue;
        };
        let len = (to_m - from_m).abs();
        let dir = if to_m >= from_m { 1.0 } else { -1.0 };
        let mut start = from_m;
        while let Some(&c) = cuts.peek() {
            if c > d + len + 1e-9 {
                break;
            }
            cuts.next();
            let at = from_m + dir * (c - d).clamp(0.0, len);
            if (at - start).abs() > 1e-9 {
                pieces.last_mut().expect("one").push(Segment::Edge {
                    edge,
                    from_m: start,
                    to_m: at,
                });
            }
            at_cut.push((edge, at));
            pieces.push(Vec::new());
            start = at;
        }
        if (to_m - start).abs() > 1e-9 {
            pieces.last_mut().expect("one").push(Segment::Edge {
                edge,
                from_m: start,
                to_m,
            });
        }
        d += len;
        last_edge = Some((edge, to_m));
    }
    // Cuts past the end land at the end.
    for _ in cuts {
        if let Some(e) = last_edge {
            at_cut.push(e);
        }
        pieces.push(Vec::new());
    }
    (pieces, at_cut)
}

/// Where via-points go along a matched or stored route: its start, every
/// `every_m` metres, its end.
pub fn geometry_along(net: &RoadNetwork, steps: &[Segment], every_m: f64) -> Geometry {
    let length: f64 = steps
        .iter()
        .map(|s| match *s {
            Segment::Edge { from_m, to_m, .. } => (to_m - from_m).abs(),
            Segment::Turnaround(_) => 0.0,
        })
        .sum();
    let n = (length / every_m.max(1.0)).ceil().max(1.0) as usize;
    let cuts: Vec<f64> = (0..=n).map(|k| length * k as f64 / n as f64).collect();
    geometry_at(net, steps, &cuts)
}

/// Via-points at route distances `cuts` (the first 0, the last the
/// length), legs between them.
pub fn geometry_at(net: &RoadNetwork, steps: &[Segment], cuts: &[f64]) -> Geometry {
    let (mut pieces, at) = split_steps(steps, cuts);
    // The piece before the first cut (0 m) is empty, so is the one after
    // the last (the length).
    pieces.remove(0);
    pieces.pop();
    let vias = at
        .iter()
        .map(|&(edge, s)| {
            let p = net.geometry(edge).point_at(s);
            Via {
                at: [p[0], p[2]],
                edge: net.edge(edge).id.0,
                s,
            }
        })
        .collect();
    Geometry {
        vias,
        legs: pieces.iter().map(|p| to_steps(net, p)).collect(),
        closed: false,
        credits: Vec::new(),
    }
}

/// The road point of a via-point on `net`.
pub fn via_snap(net: &RoadNetwork, v: &Via) -> Option<Snap> {
    let edge = net.edge_index(ocw_world::graph::EdgeId(v.edge))?;
    let s = v.s.clamp(0.0, net.geometry(edge).length());
    let p = net.geometry(edge).point_at(s);
    Some(Snap {
        edge,
        s,
        distance_m: (p[0] - v.at[0]).hypot(p[2] - v.at[1]),
    })
}

/// Turns edits into [`RouteOp`]s on a package's road graph.
pub struct Editor<'a> {
    pub net: &'a RoadNetwork,
    pub costs: &'a EdgeCosts,
}

impl Editor<'_> {
    /// A via-point at `at` (x, z), on the nearest road.
    pub fn via(&self, at: [f64; 2]) -> Result<Via, EditError> {
        let s = snap(self.net, at).ok_or_else(|| EditError::new("the package has no roads"))?;
        Ok(Via {
            at,
            edge: self.net.edge(s.edge).id.0,
            s: s.s,
        })
    }

    /// The leg from `a` to `b` with the built-in planner.
    pub fn leg(&self, a: &Via, b: &Via, k: usize) -> Result<Vec<Step>, EditError> {
        let (Some(sa), Some(sb)) = (via_snap(self.net, a), via_snap(self.net, b)) else {
            return Err(EditError::new("a via-point's road is not in this package"));
        };
        let segs = plan_leg(self.net, self.costs, sa, sb).ok_or_else(|| {
            EditError::new(format!(
                "no ridable way from via-point {} to via-point {}",
                k + 1,
                k + 2
            ))
        })?;
        Ok(to_steps(self.net, &segs))
    }

    /// A geometry with `vias` (and `closed`): the legs of `old` whose two
    /// via-points are unchanged are kept, the others planned.
    pub fn relink(
        &self,
        old: &Geometry,
        vias: Vec<Via>,
        closed: bool,
    ) -> Result<Geometry, EditError> {
        let mut g = Geometry {
            vias,
            legs: Vec::new(),
            closed,
            credits: old.credits.clone(),
        };
        if g.vias.len() < 2 {
            return Ok(g);
        }
        let need = g.legs_needed();
        for k in 0..need {
            let (a, b) = g.leg_ends(k).expect("k < need");
            let kept = (0..old.legs.len()).find_map(|j| {
                let (oa, ob) = old.leg_ends(j)?;
                (oa == a && ob == b).then(|| old.legs[j].clone())
            });
            let leg = match kept {
                Some(l) => l,
                None => self.leg(&a, &b, k)?,
            };
            g.legs.push(leg);
        }
        Ok(g)
    }

    fn set(&self, geometry: Geometry, edit: Edit) -> RouteOp {
        RouteOp::Set(Box::new(RouteSet {
            geometry,
            profile: None,
            imported: None,
            edit,
        }))
    }

    /// A via-point at the end (a loop: before the way back).
    pub fn add_via(&self, doc: &RouteDoc, at: [f64; 2]) -> Result<RouteOp, EditError> {
        let mut vias = doc.geometry.vias.clone();
        vias.push(self.via(at)?);
        let g = self.relink(&doc.geometry, vias, doc.geometry.closed)?;
        Ok(self.set(g, Edit::AddVia))
    }

    pub fn insert_via(&self, doc: &RouteDoc, i: usize, at: [f64; 2]) -> Result<RouteOp, EditError> {
        let mut vias = doc.geometry.vias.clone();
        let i = i.min(vias.len());
        vias.insert(i, self.via(at)?);
        let g = self.relink(&doc.geometry, vias, doc.geometry.closed)?;
        Ok(self.set(g, Edit::InsertVia(i)))
    }

    pub fn move_via(&self, doc: &RouteDoc, i: usize, at: [f64; 2]) -> Result<RouteOp, EditError> {
        let mut vias = doc.geometry.vias.clone();
        let v = vias
            .get_mut(i)
            .ok_or_else(|| EditError::new(format!("no via-point {}", i + 1)))?;
        *v = self.via(at)?;
        let g = self.relink(&doc.geometry, vias, doc.geometry.closed)?;
        Ok(self.set(g, Edit::MoveVia(i)))
    }

    pub fn remove_via(&self, doc: &RouteDoc, i: usize) -> Result<RouteOp, EditError> {
        let mut vias = doc.geometry.vias.clone();
        if i >= vias.len() {
            return Err(EditError::new(format!("no via-point {}", i + 1)));
        }
        vias.remove(i);
        let closed = doc.geometry.closed && vias.len() >= 2;
        let g = self.relink(&doc.geometry, vias, closed)?;
        Ok(self.set(g, Edit::RemoveVia(i)))
    }

    /// The leg whose route passes nearest to `p`, and how far it is (m).
    pub fn leg_near(&self, doc: &RouteDoc, p: [f64; 2]) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (k, leg) in doc.geometry.legs.iter().enumerate() {
            let Ok(segs) = to_segments(self.net, leg) else {
                continue;
            };
            for q in steps_points(self.net, &segs, 5.0) {
                let d = (q[0] - p[0]).hypot(q[2] - p[1]);
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((k, d));
                }
            }
        }
        best
    }

    /// Drags the route: grabbed at `grab` (x, z), dropped at `to`. A
    /// via-point is inserted into the leg grabbed and the leg re-planned
    /// through it; the new via-point's index (further moves of the same
    /// drag are [`Editor::move_via`] and merge into one undo step).
    pub fn drag(
        &self,
        doc: &RouteDoc,
        grab: [f64; 2],
        to: [f64; 2],
    ) -> Result<(RouteOp, usize), EditError> {
        let (k, _) = self
            .leg_near(doc, grab)
            .ok_or_else(|| EditError::new("there is no route to drag"))?;
        let mut vias = doc.geometry.vias.clone();
        let i = k + 1;
        vias.insert(i, self.via(to)?);
        let g = self.relink(&doc.geometry, vias, doc.geometry.closed)?;
        Ok((self.set(g, Edit::Drag(i)), i))
    }

    /// The route the other way round; legs over a one-way road are
    /// re-planned.
    pub fn reverse(&self, doc: &RouteDoc) -> Result<RouteOp, EditError> {
        let mut g = doc.geometry.reversed();
        for k in 0..g.legs.len() {
            let wrong = match to_segments(self.net, &g.legs[k]) {
                Err(_) => true,
                Ok(segs) => segs.iter().any(|s| match *s {
                    Segment::Edge { edge, from_m, to_m } => {
                        !self.costs.allows(edge, to_m >= from_m)
                    }
                    Segment::Turnaround(_) => false,
                }),
            };
            if wrong {
                let (a, b) = g.leg_ends(k).expect("k < legs");
                g.legs[k] = self.leg(&a, &b, k)?;
            }
        }
        Ok(self.set(g, Edit::Reverse))
    }

    /// Makes the route a loop (a leg back to the start) or opens it.
    pub fn set_loop(&self, doc: &RouteDoc, on: bool) -> Result<RouteOp, EditError> {
        if on && doc.geometry.vias.len() < 2 {
            return Err(EditError::new("a loop needs at least two via-points"));
        }
        let g = self.relink(&doc.geometry, doc.geometry.vias.clone(), on)?;
        Ok(self.set(g, Edit::Loop(on)))
    }

    /// Out and back: the route, a U-turn at its end, and the way back.
    pub fn out_and_back(&self, doc: &RouteDoc) -> Result<RouteOp, EditError> {
        let g = &doc.geometry;
        if g.closed || g.vias.len() < 2 {
            return Err(EditError::new(
                "out and back needs an open route with two via-points",
            ));
        }
        let back = g.reversed();
        let mut vias = g.vias.clone();
        vias.extend(back.vias.iter().skip(1));
        let mut legs = g.legs.clone();
        legs.extend(back.legs);
        Ok(self.set(
            Geometry {
                vias,
                legs,
                closed: false,
                credits: g.credits.clone(),
            },
            Edit::OutAndBack,
        ))
    }

    /// Cuts the route before via-point `i` (it becomes the start).
    pub fn cut_start(&self, doc: &RouteDoc, i: usize) -> Result<RouteOp, EditError> {
        let g = &doc.geometry;
        if i == 0 || i >= g.vias.len() {
            return Err(EditError::new(format!("cannot cut at via-point {}", i + 1)));
        }
        let vias = g.vias[i..].to_vec();
        let ng = self.relink(g, vias, false)?;
        Ok(self.set(ng, Edit::CutStart(i)))
    }

    /// Cuts the route after via-point `i` (it becomes the end).
    pub fn cut_end(&self, doc: &RouteDoc, i: usize) -> Result<RouteOp, EditError> {
        let g = &doc.geometry;
        if i + 1 >= g.vias.len() && !g.closed {
            return Err(EditError::new(format!("cannot cut at via-point {}", i + 1)));
        }
        let vias = g.vias[..=i.min(g.vias.len() - 1)].to_vec();
        let ng = self.relink(g, vias, false)?;
        Ok(self.set(ng, Edit::CutEnd(i)))
    }

    /// Splits at via-point `i`: the operation keeps the first part in this
    /// document, the second part's geometry goes into a new one.
    pub fn split(&self, doc: &RouteDoc, i: usize) -> Result<(RouteOp, Geometry), EditError> {
        let g = &doc.geometry;
        if g.closed || i == 0 || i + 1 >= g.vias.len() {
            return Err(EditError::new(format!(
                "an open route can be split at an inner via-point, not {}",
                i + 1
            )));
        }
        let first = Geometry {
            vias: g.vias[..=i].to_vec(),
            legs: g.legs[..i].to_vec(),
            closed: false,
            credits: g.credits.clone(),
        };
        let second = Geometry {
            vias: g.vias[i..].to_vec(),
            legs: g.legs[i..].to_vec(),
            closed: false,
            credits: g.credits.clone(),
        };
        Ok((self.set(first, Edit::Split(i)), second))
    }

    /// Appends `other` to the route, with a planned leg between them.
    pub fn join(&self, doc: &RouteDoc, other: &Geometry) -> Result<RouteOp, EditError> {
        let g = &doc.geometry;
        if g.closed || other.closed {
            return Err(EditError::new("loops cannot be joined; open them first"));
        }
        if g.vias.is_empty() {
            return Ok(self.set(other.clone(), Edit::Join));
        }
        let mut legs = g.legs.clone();
        let mut vias = g.vias.clone();
        if let (Some(a), Some(b)) = (g.vias.last(), other.vias.first()) {
            legs.push(self.leg(a, b, vias.len() - 1)?);
        }
        vias.extend(other.vias.iter().copied());
        legs.extend(other.legs.iter().cloned());
        let mut credits = g.credits.clone();
        for c in &other.credits {
            if !credits.contains(c) {
                credits.push(c.clone());
            }
        }
        Ok(self.set(
            Geometry {
                vias,
                legs,
                closed: false,
                credits,
            },
            Edit::Join,
        ))
    }

    /// Every leg planned again (another profile: `costs` is its costs).
    pub fn replan(&self, doc: &RouteDoc, profile: &str) -> Result<RouteOp, EditError> {
        let g = self.relink(
            &Geometry::default(),
            doc.geometry.vias.clone(),
            doc.geometry.closed,
        )?;
        Ok(RouteOp::Set(Box::new(RouteSet {
            geometry: Geometry {
                credits: Vec::new(),
                ..g
            },
            profile: Some(profile.into()),
            imported: None,
            edit: Edit::Replan,
        })))
    }

    /// A route through these points (x, z), planned from scratch.
    pub fn plan_through(&self, points: &[[f64; 2]]) -> Result<RouteOp, EditError> {
        let vias = points
            .iter()
            .map(|p| self.via(*p))
            .collect::<Result<Vec<_>, _>>()?;
        let g = self.relink(&Geometry::default(), vias, false)?;
        Ok(self.set(g, Edit::Plan))
    }
}

/// What an import gives: the geometry on the graph, the problems.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub geometry: Geometry,
    pub problems: Vec<Problem>,
    /// The matcher's summary (or why it could not match).
    pub summary: String,
}

/// Points further apart than this (and than three times the track's
/// median spacing) are a gap in the recording (m).
pub const GAP_M: f64 = 150.0;
/// Via-points of an imported route every this many metres.
pub const IMPORT_VIA_SPACING_M: f64 = 1000.0;

/// Runs of indices where `pred` holds: (first, last).
fn runs(n: usize, pred: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..n {
        match (pred(i), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, n - 1));
    }
    out
}

/// The road graph's bounding box (x, z).
fn graph_bounds(net: &RoadNetwork) -> ([f64; 2], [f64; 2]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for v in &net.graph().vertices {
        for k in 0..2 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    (lo, hi)
}

/// One part of a track file – a GPX track segment or route, a TCX track,
/// a GeoJSON line – as the import's picker lists it (16D: choosing one of
/// several segments instead of joining them).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPart {
    /// Its track's or route's name.
    pub name: Option<String>,
    pub points: usize,
    /// Its length (great circle, m).
    pub length_m: f64,
    /// Its first and last point (lat, lon).
    pub start: (f64, f64),
    pub end: (f64, f64),
    /// Its points in package metres (x, z), for the map.
    pub line: Vec<[f64; 2]>,
    /// Share of its points within the package's roads (0 – 1: the graph's
    /// bounds plus the matcher's search radius).
    pub inside: f64,
}

/// The parts of `gpx` (in file order) on `net` of `package`.
pub fn track_parts(
    gpx: &ocw_world::gpx::Gpx,
    net: &RoadNetwork,
    package: &RoutePackage,
    params: &MatchParams,
) -> Vec<TrackPart> {
    let proj = LocalProjection::new(package.origin.0, package.origin.1);
    let (lo, hi) = graph_bounds(net);
    let m = params.search_radius_m;
    gpx.part_ranges()
        .into_iter()
        .map(|(range, name)| {
            let pts = &gpx.points[range];
            let line: Vec<[f64; 2]> = pts.iter().map(|p| proj.project(p.lat, p.lon)).collect();
            let inside = line
                .iter()
                .filter(|p| (0..2).all(|k| p[k] >= lo[k] - m && p[k] <= hi[k] + m))
                .count();
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            TrackPart {
                name,
                points: pts.len(),
                length_m: pts
                    .windows(2)
                    .map(|w| ocw_world::gpx::haversine_m(w[0].lat, w[0].lon, w[1].lat, w[1].lon))
                    .sum(),
                start: (a.lat, a.lon),
                end: (b.lat, b.lon),
                inside: inside as f64 / line.len() as f64,
                line,
            }
        })
        .collect()
}

/// Which part of a track file is imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartChoice {
    /// All parts joined in file order (what an import did before the
    /// picker).
    All,
    /// Part `i` of [`track_parts`] alone.
    One(usize),
}

/// The track to import from `gpx` (its `format` name): all parts joined,
/// or one of them; `None` for a part that does not exist.
pub fn chosen_track(gpx: &ocw_world::gpx::Gpx, choice: PartChoice, format: &str) -> Option<Track> {
    match choice {
        PartChoice::All => Some(Track::of_gpx(gpx, format)),
        PartChoice::One(i) => gpx.part(i).map(|g| Track::of_gpx(&g, format)),
    }
}

/// Matches `track` onto `net` (spec §2.7 matching, 16D): the matched route
/// with via-points every [`IMPORT_VIA_SPACING_M`], and the problem
/// sections – outside the package, recording gaps, off the graph, no road
/// nearby. A track the roads cannot connect gives no route and a
/// [`ProblemKind::NotMatched`] problem (the raw track is still shown).
pub fn import_track(
    net: &RoadNetwork,
    package: &RoutePackage,
    track: &Track,
    params: &MatchParams,
) -> Result<ImportResult, String> {
    let proj = LocalProjection::new(package.origin.0, package.origin.1);
    let pts: Vec<[f64; 2]> = track
        .points
        .iter()
        .map(|p| proj.project(p.lat, p.lon))
        .collect();
    if pts.len() < 2 {
        return Err("the track needs at least two points".into());
    }
    let (lo, hi) = graph_bounds(net);
    let margin = params.search_radius_m;
    let outside = |i: usize| {
        let p = pts[i];
        (0..2).any(|k| p[k] < lo[k] - margin || p[k] > hi[k] + margin)
    };
    let mut problems: Vec<Problem> = runs(pts.len(), outside)
        .into_iter()
        .map(|(first, last)| Problem {
            kind: ProblemKind::MissingPackage,
            first,
            last,
            metres: None,
        })
        .collect();
    if problems.len() == 1 && problems[0].first == 0 && problems[0].last == pts.len() - 1 {
        return Err(format!(
            "the track is not in this package ({}): open the world it is in",
            package.name
        ));
    }
    // A gap: a jump well beyond the track's usual spacing (planners'
    // exports have sparse points on long straight roads).
    let mut spacing: Vec<f64> = pts
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .collect();
    spacing.sort_by(f64::total_cmp);
    let gap_m = GAP_M.max(3.0 * spacing[spacing.len() / 2]);
    for i in 1..pts.len() {
        let d = (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
        if d > gap_m && !outside(i) && !outside(i - 1) {
            problems.push(Problem {
                kind: ProblemKind::Gap,
                first: i - 1,
                last: i,
                metres: Some(d),
            });
        }
    }
    let name = track
        .name
        .clone()
        .unwrap_or_else(|| "Imported route".into());
    // Only the points inside the package are matched (indices mapped back).
    let inside: Vec<usize> = (0..pts.len()).filter(|&i| !outside(i)).collect();
    let inside_pts: Vec<[f64; 2]> = inside.iter().map(|&i| pts[i]).collect();
    match match_track(net, &inside_pts, &name, params) {
        Ok(report) => {
            for o in &report.off_road {
                problems.push(Problem {
                    kind: if o.max_distance_m.is_finite() {
                        ProblemKind::OffGraph
                    } else {
                        ProblemKind::Unmatched
                    },
                    first: inside[o.first],
                    last: inside[o.last],
                    metres: o.max_distance_m.is_finite().then_some(o.max_distance_m),
                });
            }
            problems.sort_by_key(|p| (p.first, p.last));
            let geometry = geometry_along(net, &report.plan.steps, IMPORT_VIA_SPACING_M);
            Ok(ImportResult {
                geometry,
                problems,
                summary: report.summary(),
            })
        }
        Err(MatchError::Broken { point }) => {
            let point = inside[point];
            problems.push(Problem {
                kind: ProblemKind::NotMatched,
                first: point,
                last: point,
                metres: None,
            });
            problems.sort_by_key(|p| (p.first, p.last));
            Ok(ImportResult {
                geometry: Geometry::default(),
                problems,
                summary: MatchError::Broken { point }.to_string(),
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The geometry of an externally planned polyline matched onto the
/// graph, split at the points nearest to the user's via-points (in order).
pub fn import_planned(
    net: &RoadNetwork,
    package: &RoutePackage,
    track: &Track,
    vias: &[[f64; 2]],
    credit: &str,
) -> Result<ImportResult, String> {
    let mut r = import_track(
        net,
        package,
        track,
        &MatchParams {
            // A planner's line follows roads: tighter than GPS.
            sigma_m: 5.0,
            ..MatchParams::default()
        },
    )?;
    if r.geometry.vias.len() >= 2 && vias.len() >= 2 {
        let doc = RouteDoc {
            geometry: r.geometry.clone(),
            ..RouteDoc::default()
        };
        let plan = doc.plan(net)?;
        // Route distance of the point nearest to each via-point, in order.
        let pts = route_samples(net, &plan.steps, 5.0);
        let mut cuts = vec![0.0];
        let mut from = 0usize;
        for v in &vias[1..vias.len() - 1] {
            let best = (from..pts.len())
                .min_by(|&a, &b| {
                    let da = (pts[a].1[0] - v[0]).hypot(pts[a].1[1] - v[1]);
                    let db = (pts[b].1[0] - v[0]).hypot(pts[b].1[1] - v[1]);
                    da.total_cmp(&db)
                })
                .unwrap_or(from);
            cuts.push(pts[best].0.max(*cuts.last().expect("one")));
            from = best;
        }
        cuts.push(plan.length_m());
        r.geometry = geometry_at(net, &plan.steps, &cuts);
    }
    r.geometry.credits = vec![credit.to_string()];
    Ok(r)
}

/// (route distance, (x, z)) every `step_m` metres along steps.
fn route_samples(net: &RoadNetwork, steps: &[Segment], step_m: f64) -> Vec<(f64, [f64; 2])> {
    let mut out = Vec::new();
    let mut d0 = 0.0;
    for s in steps {
        let Segment::Edge { edge, from_m, to_m } = *s else {
            continue;
        };
        let g = net.geometry(edge);
        let len = (to_m - from_m).abs();
        let n = (len / step_m).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = g.point_at(from_m + (to_m - from_m) * t);
            out.push((d0 + len * t, [p[0], p[2]]));
        }
        d0 += len;
    }
    out
}

/// A stored route of the package (or a graph-route file) as a geometry.
pub fn geometry_of_graph_route(net: &RoadNetwork, route: &GraphRoute) -> Result<Geometry, String> {
    let plan = route.resolve(net).map_err(|e| e.to_string())?;
    let mut g = geometry_along(net, &plan.steps, IMPORT_VIA_SPACING_M);
    if route.is_loop && plan.closes(net) && g.vias.len() >= 3 {
        // The last via-point is the start again: the closing leg instead.
        g.vias.pop();
        g.closed = true;
    }
    Ok(g)
}

/// The route's elevation profile and figures (16D *Views*).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteProfile {
    /// (distance m, road height m) every few metres.
    pub samples: Vec<[f64; 2]>,
    pub length_m: f64,
    pub gain_m: f64,
    pub loss_m: f64,
    /// The steepest 100 m up and down (fractions).
    pub max_grade: f64,
    pub min_grade: f64,
    /// Metres on each surface, largest first.
    pub surfaces: Vec<(RoadSurface, f64)>,
    /// The surface from each sample to the next (as long as `samples`;
    /// the riding time's rolling resistance).
    pub sample_surfaces: Vec<RoadSurface>,
}

/// The profile of `plan` from the road heights; `surface` is the package's
/// surface of an edge (index).
pub fn route_profile(
    net: &RoadNetwork,
    plan: &RoutePlan,
    surface: &dyn Fn(usize) -> RoadSurface,
) -> RouteProfile {
    let mut samples: Vec<[f64; 2]> = Vec::new();
    let mut d0 = 0.0;
    let mut by_surface: Vec<(RoadSurface, f64)> = Vec::new();
    let mut sample_surfaces: Vec<RoadSurface> = Vec::new();
    for s in &plan.steps {
        let Segment::Edge { edge, from_m, to_m } = *s else {
            continue;
        };
        let g = net.geometry(edge);
        let len = (to_m - from_m).abs();
        let n = (len / 10.0).ceil().max(1.0) as usize;
        let sf = surface(edge);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = g.point_at(from_m + (to_m - from_m) * t);
            let d = d0 + len * t;
            if samples.last().is_none_or(|l| d - l[0] > 0.01) {
                samples.push([d, p[1]]);
                sample_surfaces.push(sf);
            } else if k == 0
                && let Some(last) = sample_surfaces.last_mut()
            {
                // The previous edge's end: this edge's surface from here.
                *last = sf;
            }
        }
        match by_surface.iter_mut().find(|(x, _)| *x == sf) {
            Some(e) => e.1 += len,
            None => by_surface.push((sf, len)),
        }
        d0 += len;
    }
    by_surface.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (mut gain, mut loss) = (0.0, 0.0);
    for w in samples.windows(2) {
        let dh = w[1][1] - w[0][1];
        if dh > 0.0 {
            gain += dh;
        } else {
            loss -= dh;
        }
    }
    let (mut max_grade, mut min_grade) = (0.0f64, 0.0f64);
    let mut j = 0;
    for i in 0..samples.len() {
        while j < samples.len() && samples[j][0] - samples[i][0] < 100.0 {
            j += 1;
        }
        if j >= samples.len() {
            break;
        }
        let g = (samples[j][1] - samples[i][1]) / (samples[j][0] - samples[i][0]);
        max_grade = max_grade.max(g);
        min_grade = min_grade.min(g);
    }
    RouteProfile {
        samples,
        length_m: d0,
        gain_m: gain,
        loss_m: loss,
        max_grade,
        min_grade,
        surfaces: by_surface,
        sample_surfaces,
    }
}

/// Via-points further than this from their road are flagged (m).
pub const VIA_OFF_ROAD_M: f64 = 40.0;

/// The route document's validators on the package's graph: continuity,
/// via-points, the import's problem sections.
pub fn route_validators(net: Arc<RoadNetwork>) -> Vec<Validator<RouteDoc>> {
    let n1 = net.clone();
    vec![
        Box::new(move |d: &RouteDoc| check_continuity(&n1, d)),
        Box::new(move |d: &RouteDoc| check_vias(&net, d)),
        Box::new(check_problems),
    ]
}

/// The route is one continuous ride on the graph (§2.7: every step joins
/// the one before; one-way roads only their way; a loop closes).
pub fn check_continuity(net: &RoadNetwork, d: &RouteDoc) -> Vec<Finding> {
    let mut out = Vec::new();
    let g = &d.geometry;
    if g.vias.len() < 2 {
        return out;
    }
    if d.package.fingerprint != 0 && d.package.fingerprint != net.fingerprint() {
        out.push(Finding::warning(
            "the route was made on another build of this package's roads (its edge ids are looked up again)",
        ));
    }
    if g.legs.len() != g.legs_needed() {
        out.push(Finding::error(format!(
            "{} via-points need {} legs, the route has {}",
            g.vias.len(),
            g.legs_needed(),
            g.legs.len()
        )));
        return out;
    }
    let mut ok = true;
    for (k, leg) in g.legs.iter().enumerate() {
        match to_segments(net, leg) {
            Err(id) => {
                ok = false;
                out.push(
                    Finding::error(format!("leg {}: edge {id} is not in the road graph", k + 1))
                        .about(format!("leg:{k}")),
                );
            }
            Ok(segs) => {
                for s in &segs {
                    if let Segment::Edge { edge, from_m, to_m } = *s
                        && !net.edge(edge).oneway.allows(to_m >= from_m)
                    {
                        let e = net.edge(edge);
                        out.push(
                            Finding::error(format!(
                                "leg {}: rides the one-way road {} against its direction",
                                k + 1,
                                e.name.clone().unwrap_or_else(|| format!("edge {}", e.id.0))
                            ))
                            .about(format!("leg:{k}")),
                        );
                    }
                }
                // The leg starts and ends at its via-points.
                let (a, b) = g.leg_ends(k).expect("k < legs");
                let ends = |segs: &[Segment]| {
                    let first = segs.iter().find_map(|s| match *s {
                        Segment::Edge { edge, from_m, .. } => Some((edge, from_m)),
                        Segment::Turnaround(_) => None,
                    });
                    let last = segs.iter().rev().find_map(|s| match *s {
                        Segment::Edge { edge, to_m, .. } => Some((edge, to_m)),
                        Segment::Turnaround(_) => None,
                    });
                    (first, last)
                };
                let at = |v: &Via, e: Option<(usize, f64)>| -> bool {
                    let Some(sv) = via_snap(net, v) else {
                        return false;
                    };
                    let p = net.geometry(sv.edge).point_at(sv.s);
                    match e {
                        None => true,
                        Some((edge, s)) => {
                            let q = net.geometry(edge).point_at(s);
                            (p[0] - q[0]).hypot(p[2] - q[2]) < 1.0
                        }
                    }
                };
                let (first, last) = ends(&segs);
                if !(at(&a, first) && at(&b, last)) {
                    ok = false;
                    out.push(
                        Finding::error(format!(
                            "leg {} does not run from via-point {} to via-point {}",
                            k + 1,
                            k + 1,
                            (k + 1) % g.vias.len() + 1
                        ))
                        .about(format!("leg:{k}")),
                    );
                }
            }
        }
    }
    if ok {
        let empty = g
            .legs
            .iter()
            .all(|l| !l.iter().any(|s| matches!(s, Step::Edge { .. })));
        match d.plan(net) {
            Err(_) if empty => out.push(Finding::info("the via-points are all at one spot")),
            Err(e) => out.push(Finding::error(e)),
            Ok(plan) => {
                if g.closed && !plan.is_loop {
                    out.push(Finding::warning(
                        "the loop turns back at its start: it is ridden once, not in laps",
                    ));
                }
            }
        }
    }
    out
}

/// Via-points on the road.
pub fn check_vias(net: &RoadNetwork, d: &RouteDoc) -> Vec<Finding> {
    let mut out = Vec::new();
    if d.geometry.vias.len() == 1 {
        out.push(Finding::info("add a second via-point to plan a route"));
    }
    for (i, v) in d.geometry.vias.iter().enumerate() {
        match via_snap(net, v) {
            None => out.push(
                Finding::error(format!(
                    "via-point {}: its road is not in this package",
                    i + 1
                ))
                .about(format!("via:{i}")),
            ),
            Some(s) if s.distance_m > VIA_OFF_ROAD_M => out.push(
                Finding::warning(format!(
                    "via-point {} is {:.0} m from the road",
                    i + 1,
                    s.distance_m
                ))
                .about(format!("via:{i}")),
            ),
            _ => {}
        }
    }
    out
}

/// The import's problem sections, as warnings (errors where nothing was
/// matched).
pub fn check_problems(d: &RouteDoc) -> Vec<Finding> {
    d.problems
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let f = if p.kind == ProblemKind::NotMatched {
                Finding::error(p.message())
            } else {
                Finding::warning(p.message())
            };
            f.about(format!("problem:{i}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocId, Document};
    use ocw_world::plan::{Profile, test_grid};

    pub const ORIGIN: (f64, f64) = (47.14, 9.52);

    pub fn package(net: &RoadNetwork) -> RoutePackage {
        RoutePackage {
            id: "local/grid".into(),
            name: "Grid".into(),
            fingerprint: net.fingerprint(),
            origin: ORIGIN,
        }
    }

    fn setup() -> (Arc<RoadNetwork>, EdgeCosts) {
        let net = Arc::new(test_grid());
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        (net, costs)
    }

    fn doc(net: &Arc<RoadNetwork>) -> Document<RouteDoc> {
        let mut d = Document::new(DocId(1), "r", RouteDoc::new(package(net), "Ada's ride"));
        for v in route_validators(net.clone()) {
            d = d.with_validator(v);
        }
        d
    }

    fn errors(d: &Document<RouteDoc>) -> Vec<String> {
        d.findings()
            .iter()
            .filter(|f| f.severity == crate::Severity::Error)
            .map(|f| f.message.clone())
            .collect()
    }

    fn length(d: &Document<RouteDoc>, net: &RoadNetwork) -> f64 {
        d.content().plan(net).map_or(0.0, |p| p.length_m())
    }

    #[test]
    fn via_points_are_added_moved_removed_and_undone() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = doc(&net);
        d.execute(ed.add_via(d.content(), [0.0, 200.0]).unwrap(), 0.0)
            .unwrap();
        assert!(
            d.findings()
                .iter()
                .any(|f| f.message.contains("second via-point"))
        );
        d.execute(ed.add_via(d.content(), [400.0, 200.0]).unwrap(), 2.0)
            .unwrap();
        assert!((length(&d, &net) - 400.0).abs() < 1e-6);
        d.execute(ed.add_via(d.content(), [400.0, 600.0]).unwrap(), 4.0)
            .unwrap();
        assert!((length(&d, &net) - 800.0).abs() < 1e-6);
        assert!(errors(&d).is_empty(), "{:?}", errors(&d));
        // Moving the middle one re-plans its two legs only.
        let first_leg = d.content().geometry.legs[0].clone();
        d.execute(ed.move_via(d.content(), 1, [200.0, 400.0]).unwrap(), 6.0)
            .unwrap();
        assert_ne!(d.content().geometry.legs[0], first_leg);
        assert!((length(&d, &net) - 800.0).abs() < 1e-6);
        // ... and further moves of it merge into one undo step.
        d.execute(ed.move_via(d.content(), 1, [200.0, 300.0]).unwrap(), 6.5)
            .unwrap();
        assert_eq!(d.history().undo.len(), 4);
        assert_eq!(d.undo_label(), Some("Move via-point 2"));
        d.execute(ed.remove_via(d.content(), 1).unwrap(), 9.0)
            .unwrap();
        assert_eq!(d.content().geometry.vias.len(), 2);
        assert!((length(&d, &net) - 800.0).abs() < 1e-6);
        // Undo walks back to the empty route.
        while d.undo(10.0).unwrap() {}
        assert_eq!(d.content().geometry, Geometry::default());
        assert!(!d.is_dirty());
        while d.redo(11.0).unwrap() {}
        assert_eq!(d.content().geometry.vias.len(), 2);
    }

    #[test]
    fn dragging_the_route_re_plans_only_the_leg_grabbed() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = doc(&net);
        d.execute(
            ed.plan_through(&[[0.0, 200.0], [600.0, 200.0], [600.0, 600.0]])
                .unwrap(),
            0.0,
        )
        .unwrap();
        let second = d.content().geometry.legs[1].clone();
        // Grab row 1 at x = 300 and drop it on row 2.
        let (op, i) = ed
            .drag(d.content(), [300.0, 205.0], [300.0, 400.0])
            .unwrap();
        assert_eq!(i, 1);
        d.execute(op, 2.0).unwrap();
        assert_eq!(d.content().geometry.vias.len(), 4);
        assert_eq!(d.content().geometry.legs[2], second);
        assert!(errors(&d).is_empty(), "{:?}", errors(&d));
        let plan = d.content().plan(&net).unwrap();
        // 0,200 → 0,400? no: down to row 2 at x ≤ 300, along it, back up.
        assert!(plan.length_m() > 1000.0 + 1.0);
        // The rest of the drag moves the new via-point: one undo step.
        d.execute(ed.move_via(d.content(), 1, [300.0, 600.0]).unwrap(), 2.2)
            .unwrap();
        assert_eq!(d.undo_label(), Some("Drag route"));
        d.undo(3.0).unwrap();
        assert_eq!(d.content().geometry.vias.len(), 3);
    }

    #[test]
    fn reverse_loop_out_and_back_cut_split_and_join_keep_the_route_continuous() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = doc(&net);
        d.execute(
            ed.plan_through(&[[0.0, 200.0], [400.0, 200.0], [400.0, 400.0]])
                .unwrap(),
            0.0,
        )
        .unwrap();
        let ok = |d: &Document<RouteDoc>| assert!(errors(d).is_empty(), "{:?}", errors(d));
        let len = length(&d, &net);
        d.execute(ed.reverse(d.content()).unwrap(), 2.0).unwrap();
        ok(&d);
        assert!((length(&d, &net) - len).abs() < 1e-6);
        assert_eq!(d.content().geometry.vias[0].at, [400.0, 400.0]);
        d.execute(ed.reverse(d.content()).unwrap(), 4.0).unwrap();
        d.execute(ed.set_loop(d.content(), true).unwrap(), 6.0)
            .unwrap();
        ok(&d);
        let plan = d.content().plan(&net).unwrap();
        assert!(plan.is_loop);
        assert!(
            (plan.length_m() - 1200.0).abs() < 1e-6,
            "{}",
            plan.length_m()
        );
        // Reversing a loop keeps it a loop.
        d.execute(ed.reverse(d.content()).unwrap(), 7.0).unwrap();
        ok(&d);
        assert!(d.content().plan(&net).unwrap().is_loop);
        d.execute(ed.set_loop(d.content(), false).unwrap(), 8.0)
            .unwrap();
        d.execute(ed.out_and_back(d.content()).unwrap(), 10.0)
            .unwrap();
        ok(&d);
        let plan = d.content().plan(&net).unwrap();
        assert!((plan.length_m() - 2.0 * len).abs() < 1e-6);
        assert!(
            plan.steps
                .iter()
                .any(|s| matches!(s, Segment::Turnaround(_)))
        );
        d.execute(ed.cut_end(d.content(), 2).unwrap(), 12.0)
            .unwrap();
        ok(&d);
        assert!((length(&d, &net) - len).abs() < 1e-6);
        d.execute(ed.cut_start(d.content(), 1).unwrap(), 14.0)
            .unwrap();
        ok(&d);
        assert_eq!(d.content().geometry.vias.len(), 2);
        // Split at an inner via-point and join again.
        d.execute(ed.add_via(d.content(), [600.0, 400.0]).unwrap(), 16.0)
            .unwrap();
        let before = d.content().geometry.clone();
        let (op, second) = ed.split(d.content(), 1).unwrap();
        d.execute(op, 18.0).unwrap();
        ok(&d);
        assert_eq!(second.vias.len(), 2);
        d.execute(ed.join(d.content(), &second).unwrap(), 20.0)
            .unwrap();
        ok(&d);
        // The join plans a leg between the two equal points: no distance.
        assert!(
            (length(&d, &net) - {
                let mut x = doc(&net);
                x.execute(ed.set(before, Edit::Plan), 0.0).unwrap();
                length(&x, &net)
            })
            .abs()
                < 1e-6
        );
        assert!(ed.split(d.content(), 0).is_err());
    }

    #[test]
    fn a_broken_route_is_found_by_the_validators() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = doc(&net);
        d.execute(
            ed.plan_through(&[[0.0, 200.0], [400.0, 200.0]]).unwrap(),
            0.0,
        )
        .unwrap();
        let mut g = d.content().geometry.clone();
        // An edge that is not there.
        g.legs[0].push(Step::Edge {
            id: 99,
            from_m: 0.0,
            to_m: 10.0,
        });
        d.execute(ed.set(g, Edit::Plan), 1.0).unwrap();
        assert!(errors(&d).iter().any(|e| e.contains("edge 99")));
        d.undo(2.0).unwrap();
        // A gap: the second half of the leg dropped.
        let mut g = d.content().geometry.clone();
        g.legs[0].truncate(1);
        d.execute(ed.set(g, Edit::Plan), 3.0).unwrap();
        assert!(errors(&d).iter().any(|e| e.contains("does not run from")));
        d.undo(4.0).unwrap();
        // A one-way road the wrong way.
        let mut graph = net.graph().clone();
        let mut g2 = d.content().geometry.clone();
        let Step::Edge { id, .. } = g2.legs[0][0] else {
            panic!()
        };
        for e in &mut graph.edges {
            if e.id.0 == id {
                e.oneway = ocw_world::graph::Oneway::Backward;
            }
        }
        let net2 = RoadNetwork::new(graph);
        assert!(
            check_continuity(&net2, d.content())
                .iter()
                .any(|f| f.message.contains("one-way"))
        );
        g2.closed = true;
        assert!(
            d.execute(ed.set(g2, Edit::Plan), 5.0).is_err(),
            "legs must fit the via-points"
        );
        // Legs are set with the via-points.
        assert!(errors(&d).is_empty());
    }

    fn fixture_track(text: &str, name: &str) -> Track {
        let (t, _) = formats::read_track(name, text.as_bytes()).unwrap();
        t
    }

    #[test]
    fn a_gpx_ride_is_matched_and_its_problem_sections_are_found() {
        let (net, costs) = setup();
        let track = fixture_track(
            include_str!("../tests/fixtures/grid-problems.gpx"),
            "grid-problems.gpx",
        );
        let r = import_track(&net, &package(&net), &track, &MatchParams::default()).unwrap();
        let kinds: Vec<ProblemKind> = r.problems.iter().map(|p| p.kind).collect();
        assert!(
            kinds.contains(&ProblemKind::OffGraph) || kinds.contains(&ProblemKind::Unmatched),
            "{kinds:?}"
        );
        assert!(kinds.contains(&ProblemKind::Gap), "{kinds:?}");
        // Via-points every kilometre; the route continuous.
        assert!(r.geometry.vias.len() >= 3);
        let mut d = doc(&net);
        d.execute(
            RouteOp::Set(Box::new(RouteSet {
                geometry: r.geometry.clone(),
                profile: None,
                imported: Some(Imported {
                    track: Some(track.clone()),
                    problems: r.problems.clone(),
                }),
                edit: Edit::Import,
            })),
            0.0,
        )
        .unwrap();
        assert!(errors(&d).is_empty(), "{:?}", errors(&d));
        let warnings = d
            .findings()
            .iter()
            .filter(|f| f.severity == crate::Severity::Warning)
            .count();
        assert_eq!(warnings, r.problems.len());
        let plan = d.content().plan(&net).unwrap();
        // 0,0 → 400,0 → 400,600 → 100,600 (ends at the last point near 0,600).
        assert!(plan.length_m() > 1300.0, "{}", plan.length_m());
        // Editing a leg away from the problem keeps the other legs.
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let legs = d.content().geometry.legs.clone();
        let last = d.content().geometry.vias.len() - 1;
        d.execute(ed.move_via(d.content(), last, [0.0, 600.0]).unwrap(), 2.0)
            .unwrap();
        assert_eq!(d.content().geometry.legs[0], legs[0]);
        assert!(errors(&d).is_empty());
        // Undo of the import gives the empty document.
        while d.undo(3.0).unwrap() {}
        assert_eq!(d.content(), &RouteDoc::new(package(&net), "Ada's ride"));
    }

    #[test]
    fn a_track_outside_the_package_is_refused_and_a_part_outside_is_a_problem() {
        let (net, _) = setup();
        let mut track = fixture_track(
            include_str!("../tests/fixtures/grid-row.tcx"),
            "grid-row.tcx",
        );
        let far = Track {
            points: track
                .points
                .iter()
                .map(|p| TrackPoint {
                    lat: p.lat + 1.0,
                    ..*p
                })
                .collect(),
            ..track.clone()
        };
        assert!(
            import_track(&net, &package(&net), &far, &MatchParams::default())
                .unwrap_err()
                .contains("not in this package")
        );
        // The track runs on 2 km east, out of the package.
        let last = *track.points.last().unwrap();
        for k in 1..40 {
            track.points.push(TrackPoint {
                lat: last.lat,
                lon: last.lon + f64::from(k) * 0.0007,
                ele: None,
            });
        }
        let r = import_track(&net, &package(&net), &track, &MatchParams::default()).unwrap();
        assert!(
            r.problems
                .iter()
                .any(|p| p.kind == ProblemKind::MissingPackage)
        );
        assert!(!r.problems.iter().any(|p| p.kind == ProblemKind::Unmatched));
    }

    #[test]
    fn tcx_and_geojson_imports_match_onto_their_roads() {
        let (net, _) = setup();
        let tcx = fixture_track(include_str!("../tests/fixtures/grid-row.tcx"), "x.tcx");
        assert_eq!(tcx.format, "TCX");
        assert_eq!(tcx.name.as_deref(), Some("Bea Climber row"));
        let r = import_track(&net, &package(&net), &tcx, &MatchParams::default()).unwrap();
        let d = RouteDoc {
            geometry: r.geometry,
            ..RouteDoc::new(package(&net), "t")
        };
        assert!((d.plan(&net).unwrap().length_m() - 800.0).abs() < 5.0);
        let gj = fixture_track(
            include_str!("../tests/fixtures/grid-column.geojson"),
            "x.geojson",
        );
        let r = import_track(&net, &package(&net), &gj, &MatchParams::default()).unwrap();
        let d = RouteDoc {
            geometry: r.geometry,
            ..RouteDoc::new(package(&net), "t")
        };
        assert!((d.plan(&net).unwrap().length_m() - 600.0).abs() < 5.0);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn a_brouter_answer_is_matched_and_split_at_the_via_points() {
        let (net, _) = setup();
        let answer = ocw_world::plan::brouter::parse_geojson(include_str!(
            "../../ocw-world/tests/fixtures/brouter-grid.geojson"
        ))
        .unwrap();
        let track = Track {
            name: None,
            format: "BRouter".into(),
            points: answer
                .points
                .iter()
                .map(|p| TrackPoint {
                    lat: p.lat,
                    lon: p.lon,
                    ele: p.ele,
                })
                .collect(),
        };
        let vias = [[0.0, 0.0], [200.0, 400.0], [600.0, 600.0]];
        let r = import_planned(&net, &package(&net), &track, &vias, &answer.credit).unwrap();
        assert_eq!(r.geometry.vias.len(), 3);
        let mid = r.geometry.vias[1].at;
        assert!((mid[0] - 200.0).hypot(mid[1] - 400.0) < 10.0, "{mid:?}");
        assert_eq!(r.geometry.credits, vec![answer.credit.clone()]);
        let d = RouteDoc {
            geometry: r.geometry,
            ..RouteDoc::new(package(&net), "b")
        };
        assert!(
            check_continuity(&net, &d).is_empty(),
            "{:?}",
            check_continuity(&net, &d)
        );
        // 1200 m on the grid; the last 40 m are past the package's roads.
        assert!((d.plan(&net).unwrap().length_m() - 1200.0).abs() < 5.0);
    }

    #[test]
    fn a_package_route_becomes_via_points_and_legs() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let op = ed
            .plan_through(&[[0.0, 200.0], [400.0, 200.0], [400.0, 400.0], [0.0, 400.0]])
            .unwrap();
        let mut d = doc(&net);
        d.execute(op, 0.0).unwrap();
        d.execute(ed.set_loop(d.content(), true).unwrap(), 1.0)
            .unwrap();
        let gr = d.content().graph_route(&net).unwrap();
        assert!(gr.is_loop);
        let g = geometry_of_graph_route(&net, &gr).unwrap();
        assert!(g.closed);
        let back = RouteDoc {
            geometry: g,
            ..RouteDoc::new(package(&net), "x")
        };
        assert!(check_continuity(&net, &back).is_empty());
        assert!((back.plan(&net).unwrap().length_m() - 1200.0).abs() < 1e-6);
        assert_eq!(
            GraphRoute::from_plan(&back.plan(&net).unwrap(), &net)
                .steps
                .len(),
            gr.steps.len()
        );
    }

    #[test]
    fn split_steps_cuts_where_asked() {
        let net = test_grid();
        let steps = vec![Segment::whole(&net, 0, true), Segment::whole(&net, 2, true)];
        let (pieces, at) = split_steps(&steps, &[0.0, 150.0, 200.0, 400.0]);
        assert_eq!(pieces.len(), 5);
        assert!(pieces[0].is_empty() && pieces[4].is_empty());
        assert_eq!(pieces[1].len(), 1);
        assert_eq!(at.len(), 4);
        assert_eq!(at[1], (0, 150.0));
    }

    #[test]
    fn the_profile_counts_climbs_grades_and_surfaces() {
        let net = test_grid();
        let mut g = net.graph().clone();
        g.edges[0].profile = (0..=20).map(|k| 450.0 + f64::from(k)).collect();
        let net = RoadNetwork::new(g);
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = RouteDoc::new(package(&net), "p");
        let RouteOp::Set(s) = ed
            .plan_through(&[[0.0, 0.0], [200.0, 0.0], [200.0, 200.0]])
            .unwrap()
        else {
            panic!()
        };
        d.geometry = s.geometry;
        let plan = d.plan(&net).unwrap();
        let p = route_profile(&net, &plan, &|e| RoadSurface::of_class(net.edge(e).class));
        assert!((p.length_m - 400.0).abs() < 1e-6);
        assert!(p.gain_m > 1.0);
        assert!(p.max_grade > 0.0);
        assert_eq!(p.surfaces.len(), 1);
        assert!(p.samples.len() > 30);
        // Per sample: the first 200 m (edge 0) on gravel, the rest asphalt;
        // the corner at 200 m starts the second edge's surface.
        let p = route_profile(&net, &plan, &|e| {
            if e == 0 {
                RoadSurface::Gravel
            } else {
                RoadSurface::Asphalt
            }
        });
        assert_eq!(p.sample_surfaces.len(), p.samples.len());
        for (s, sf) in p.samples.iter().zip(&p.sample_surfaces) {
            let want = if s[0] < 200.0 - 1e-6 {
                RoadSurface::Gravel
            } else {
                RoadSurface::Asphalt
            };
            assert_eq!(*sf, want, "at {} m", s[0]);
        }
    }

    #[test]
    fn one_part_of_a_track_file_is_chosen_or_all_are_joined() {
        let (net, _) = setup();
        let proj = LocalProjection::new(ORIGIN.0, ORIGIN.1);
        // Two segments on the grid: row 1 east (600 m), then column 0 of
        // rows 2 – 3 far from where the first ends.
        let seg = |pts: &[[f64; 2]]| -> String {
            let mut s = String::from("<trkseg>");
            for p in pts {
                let (lat, lon) = proj.unproject(*p);
                s.push_str(&format!(r#"<trkpt lat="{lat:.7}" lon="{lon:.7}"/>"#));
            }
            s + "</trkseg>"
        };
        let row: Vec<[f64; 2]> = (0..=30).map(|k| [f64::from(k) * 20.0, 200.0]).collect();
        let col: Vec<[f64; 2]> = (0..=20)
            .map(|k| [0.0, 400.0 + f64::from(k) * 10.0])
            .collect();
        let text = format!(
            "<gpx><trk><name>Ada's two rides</name>{}{}</trk></gpx>",
            seg(&row),
            seg(&col)
        );
        let (gpx, format) = formats::read_parts("two.gpx", text.as_bytes()).unwrap();
        let parts = track_parts(&gpx, &net, &package(&net), &MatchParams::default());
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].points, parts[1].points), (31, 21));
        assert!(
            (parts[0].length_m - 600.0).abs() < 3.0,
            "{}",
            parts[0].length_m
        );
        assert!(
            (parts[1].length_m - 200.0).abs() < 2.0,
            "{}",
            parts[1].length_m
        );
        let start = proj.project(parts[1].start.0, parts[1].start.1);
        assert!((start[0] - 0.0).abs() < 0.5 && (start[1] - 400.0).abs() < 0.5);
        assert_eq!(parts[0].inside, 1.0);
        assert_eq!(parts[1].name.as_deref(), Some("Ada's two rides"));
        assert_eq!(parts[1].line.len(), 21);
        // The second part alone: only its 200 m are matched.
        let one = chosen_track(&gpx, PartChoice::One(1), format.name()).unwrap();
        assert_eq!(one.points.len(), 21);
        let r = import_track(&net, &package(&net), &one, &MatchParams::default()).unwrap();
        let mut d = doc(&net);
        d.execute(
            RouteOp::Set(Box::new(RouteSet {
                geometry: r.geometry,
                profile: None,
                imported: Some(Imported {
                    track: Some(one),
                    problems: r.problems,
                }),
                edit: Edit::Import,
            })),
            0.0,
        )
        .unwrap();
        assert!(
            (length(&d, &net) - 200.0).abs() < 2.0,
            "{}",
            length(&d, &net)
        );
        // All joined: both segments and the jump between them (a gap).
        let all = chosen_track(&gpx, PartChoice::All, format.name()).unwrap();
        assert_eq!(all.points.len(), 52);
        let r = import_track(&net, &package(&net), &all, &MatchParams::default()).unwrap();
        assert!(
            r.problems.iter().any(|p| p.kind == ProblemKind::Gap),
            "{:?}",
            r.problems
        );
        assert!(chosen_track(&gpx, PartChoice::One(2), "GPX").is_none());
        // A part outside the package says so.
        let far: Vec<[f64; 2]> = (0..5)
            .map(|k| [5000.0 + f64::from(k) * 20.0, 0.0])
            .collect();
        let text = format!("<gpx><trk>{}{}</trk></gpx>", seg(&row), seg(&far));
        let (gpx, _) = formats::read_parts("far.gpx", text.as_bytes()).unwrap();
        let parts = track_parts(&gpx, &net, &package(&net), &MatchParams::default());
        assert_eq!(parts[1].inside, 0.0);
    }

    #[test]
    fn random_edits_undo_to_the_empty_route() {
        let (net, costs) = setup();
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let mut d = doc(&net);
        let mut seed = 7u64;
        let mut rnd = |n: u64| {
            seed = ocw_world::network::splitmix(seed);
            seed % n
        };
        let mut t = 0.0;
        for _ in 0..200 {
            t += 2.0;
            let p = [rnd(601) as f64, rnd(601) as f64];
            let n = d.content().geometry.vias.len();
            let op = match rnd(8) {
                0 | 1 => ed.add_via(d.content(), p),
                2 if n > 0 => ed.move_via(d.content(), rnd(n as u64) as usize, p),
                3 if n > 0 => ed.remove_via(d.content(), rnd(n as u64) as usize),
                4 => ed.reverse(d.content()),
                5 => ed.set_loop(d.content(), !d.content().geometry.closed),
                6 if n > 0 => ed.insert_via(d.content(), rnd(n as u64) as usize, p),
                7 => {
                    if d.can_undo() {
                        d.undo(t).unwrap();
                    }
                    continue;
                }
                _ => continue,
            };
            if let Ok(op) = op {
                d.execute(op, t).unwrap();
                assert!(errors(&d).is_empty(), "{:?}", errors(&d));
            }
        }
        while d.undo(t).unwrap() {}
        assert_eq!(d.content(), &RouteDoc::new(package(&net), "Ada's ride"));
    }
}
