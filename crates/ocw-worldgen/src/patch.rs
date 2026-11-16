//! World patches on the OSM sources (spec §14 Phase 16, 16C): the way
//! targets of a [`Plan`] change the OSM tags before the road graph is
//! built, so everything that reads the tags (rideability, class, width,
//! one-way, surface, names, the road dress) sees the patched road. The
//! format, the edge edits and the height points are in
//! [`ocw_world::patch`].

use std::collections::{BTreeMap, BTreeSet};

use ocw_world::RoadSurface;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Edge, EdgeId, RoadGraph};
use ocw_world::patch::{
    BikeAccess, LoadedPatch, PatchClass, PatchOneway, Plan, Report, Target, Targets,
    apply_height_points, apply_road_edits,
};

use crate::filter::rideable;
use crate::osm::{OsmData, Way};

/// Applies the plan's way targets (road attributes, closures, removals)
/// to `osm`, in plan order. A way that is not in the data is reported
/// missing; edge targets are left for [`ocw_world::patch::apply_road_edits`]
/// on the raw graph.
pub fn apply_to_osm(plan: &Plan, osm: &mut OsmData, report: &mut Report) {
    let index: BTreeMap<i64, usize> = osm
        .ways
        .iter()
        .enumerate()
        .map(|(i, w)| (w.id, i))
        .collect();
    let gone = |w: i64| format!("way {w} is not among the OSM roads near the region");
    for r in &plan.roads {
        let Target::Way(id) = r.target else {
            continue;
        };
        let Some(&i) = index.get(&id) else {
            report.missing(r.at, gone(id));
            continue;
        };
        let way = &mut osm.ways[i];
        edit_tags(way, &r.edit);
        let what = if r.edit.bike == Some(BikeAccess::Closed) {
            "OSM tags changed; closed for bikes".to_string()
        } else if rideable(way).is_some() {
            "OSM tags changed".to_string()
        } else {
            "OSM tags changed; the way is not ridable with them".to_string()
        };
        report.applied(r.at, what);
    }
    let mut remove = BTreeSet::new();
    for &(at, t) in &plan.removals {
        let Target::Way(id) = t else {
            continue;
        };
        if index.contains_key(&id) {
            remove.insert(id);
            report.applied(at, "way left out of the road graph");
        } else {
            report.missing(at, gone(id));
        }
    }
    if !remove.is_empty() {
        osm.ways.retain(|w| !remove.contains(&w.id));
    }
}

fn set(way: &mut Way, k: &str, v: &str) {
    way.tags.insert(k.to_string(), v.to_string());
}

/// The OSM tags that give the asked attributes (as `filter` reads them).
fn edit_tags(way: &mut Way, e: &ocw_world::patch::RoadEdit) {
    if let Some(s) = &e.surface {
        set(way, "surface", s);
        // `smoothness` would turn asphalt into worn asphalt.
        way.tags.remove("smoothness");
    }
    if let Some(w) = e.width_m {
        set(way, "width", &format!("{w}"));
    }
    if let Some(c) = e.class {
        set(way, "highway", c.osm());
        if c == PatchClass::Path
            && !matches!(
                way.tag("bicycle"),
                Some("yes" | "designated" | "permissive")
            )
        {
            // A path is ridable only with bike access (filter::rideable).
            set(way, "bicycle", "yes");
        }
        if c == PatchClass::Service {
            way.tags.remove("service");
        }
    }
    if let Some(o) = e.oneway {
        way.tags.remove("oneway:bicycle");
        set(
            way,
            "oneway",
            match o {
                PatchOneway::No => "no",
                PatchOneway::Forward => "yes",
                PatchOneway::Backward => "-1",
            },
        );
    }
    if e.bike == Some(BikeAccess::Closed) {
        set(way, "bicycle", "no");
    }
    if let Some(n) = &e.name {
        if n.is_empty() {
            way.tags.remove("name");
        } else {
            set(way, "name", n);
        }
    }
}

/// Positions of the plan's OSM node targets from `osm`.
pub fn resolve_nodes(plan: &mut Plan, osm: &OsmData, report: &mut Report) {
    plan.resolve_nodes(|n| osm.nodes.get(&n).copied(), report);
}

/// A patch stack on its way through a build, stage by stage (the order
/// of spec §14 Phase 16 16C: after reading OSM, before meshing):
/// 1. [`Patching::on_osm`] right after reading the OSM roads;
/// 2. [`Patching::on_raw_graph`] on the raw graph, before its cleanup
///    (`build::build_road_graph_with`; for the region and any context
///    graph);
/// 3. [`Patching::on_profiles`] after the road profiles and their
///    cleaning, before the terrain is conformed to the roads;
/// 4. [`Patching::surface`] for the road surface section.
///
/// [`Patching::report`] then says what each operation did.
#[derive(Debug, Clone, Default)]
pub struct Patching {
    pub plan: Plan,
    pub report: Report,
    surfaces: BTreeMap<EdgeId, RoadSurface>,
}

impl Patching {
    pub fn new(stack: &[LoadedPatch]) -> Self {
        let mut report = Report::new(stack);
        let plan = Plan::new(stack, &mut report);
        Self {
            plan,
            report,
            surfaces: BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.plan.roads.is_empty() && self.plan.removals.is_empty() && self.plan.points.is_empty()
    }

    pub fn on_osm(&mut self, osm: &mut OsmData) {
        apply_to_osm(&self.plan, osm, &mut self.report);
        resolve_nodes(&mut self.plan, osm, &mut self.report);
    }

    pub fn on_raw_graph(&mut self, graph: &mut RoadGraph) {
        let s = apply_road_edits(&self.plan, graph, Targets::EdgesOnly, &mut self.report);
        self.surfaces.extend(s);
    }

    pub fn on_profiles(&mut self, graph: &mut RoadGraph, proj: &LocalProjection) {
        apply_height_points(&self.plan, graph, proj, &mut self.report);
    }

    /// The surface an edge target asked for (way targets changed the OSM
    /// `surface` tag instead).
    pub fn surface(&self, edge: &Edge) -> Option<RoadSurface> {
        self.surfaces.get(&edge.id).copied()
    }
}

#[cfg(test)]
mod tests {
    use ocw_world::patch::{LoadedPatch, OpRef, Outcome};

    use super::*;
    use crate::build::{Region, build_road_graph};
    use crate::filter;

    const FIXTURE: &str = include_str!("../tests/fixtures/mini-v1.osm");

    fn patch(text: &str) -> (Plan, Report) {
        let s = vec![
            LoadedPatch::parse(
                "t.ocwpatch",
                &format!("ocwpatch = 1\npackage = \"mini\"\n{text}"),
            )
            .unwrap(),
        ];
        let mut report = Report::new(&s);
        (Plan::new(&s, &mut report), report)
    }

    fn way(osm: &OsmData, id: i64) -> &Way {
        osm.ways.iter().find(|w| w.id == id).unwrap()
    }

    #[test]
    fn way_targets_change_the_tags() {
        let mut osm = OsmData::parse_osm_xml(FIXTURE).unwrap();
        let (plan, mut report) = patch(
            r#"
[[op]]
op = "road"
way = 100
surface = "gravel"
width_m = 4.5
class = "tertiary"
name = "Neue Strasse"

[[op]]
op = "road"
way = 105
oneway = "no"

[[op]]
op = "road"
way = 102
class = "path"

[[op]]
op = "road"
way = 101
bike = "closed"

[[op]]
op = "road"
way = 9999
name = "Nirgendwo"

[[op]]
op = "remove_road"
way = 103
"#,
        );
        apply_to_osm(&plan, &mut osm, &mut report);
        let w = way(&osm, 100);
        assert_eq!(
            filter::rideable(w),
            Some(ocw_world::graph::RoadClass::Tertiary)
        );
        assert_eq!(
            filter::width_m(w, ocw_world::graph::RoadClass::Tertiary),
            4.5
        );
        assert_eq!(
            filter::surface(w, ocw_world::graph::RoadClass::Tertiary),
            ocw_world::RoadSurface::Gravel
        );
        assert_eq!(w.tag("name"), Some("Neue Strasse"));
        assert_eq!(filter::oneway(way(&osm, 105)), ocw_world::graph::Oneway::No);
        assert_eq!(
            filter::rideable(way(&osm, 102)),
            Some(ocw_world::graph::RoadClass::Path)
        );
        assert_eq!(filter::rideable(way(&osm, 101)), None);
        assert!(osm.ways.iter().all(|w| w.id != 103));
        assert_eq!(report.missing_count(), 1);
        assert!(matches!(
            &report.get(OpRef { patch: 0, op: 4 }).unwrap().outcome,
            Outcome::Missing(m) if m.contains("way 9999")
        ));
        // The graph follows: no dead end (101) and no bridge (103) left;
        // without the bridge the loop is a dead end at 9 and 10 now.
        let (g, _) = build_road_graph(&osm, &Region::bbox(46.490, 9.485, 46.510, 9.515));
        assert!(g.edges.iter().all(|e| e.osm_way != 101 && e.osm_way != 103));
        assert!(g.edges.iter().any(|e| e.osm_way == 100 && e.width_m == 4.5));
    }
}
