//! The sections `TRFC` and `AERO` (spec 14 Phase 18, 18B and 18C *Data
//! from the package*; milestone 18E T2): the traffic rules of every road
//! graph edge from its OSM way ([`ocw_world::traffic::EdgeRules::from_osm`]
//! with the edge's country and whether it is in town), the signals and
//! signs on the edges, bus stops and bus lines (`route=bus` relations),
//! car parks, and the aerodromes, runways and helipads with the nearby
//! airports outside the package.
//!
//! *Country:* an edge inside a complete country outline of the extract
//! (`boundary=administrative`, `admin_level=2` with `ISO3166-1`) takes
//! that country; elsewhere the package's country (its outline at the
//! origin, else its time zone).
//!
//! *In town:* within a radius of a `place` (city 1.5 km, town 900 m,
//! suburb 600 m, village 450 m, quarter / neighbourhood 300 m, hamlet
//! 150 m; areas at their centroid), or on residential or commercial land
//! use; residential and living streets always (and typed limits such as
//! `maxspeed:type=DE:rural` say otherwise).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ocw_world::aero::{Aero, Aerodrome, AerodromeKind, Helipad, NEARBY_RADIUS_KM, Runway};
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Edge, EdgeId, RoadGraph};
use ocw_world::landuse::LandClass;
use ocw_world::places::PlaceKind;
use ocw_world::traffic::{
    Applies, BusRoute, BusStop, ControlKind, Country, EdgeContext, EdgeRules, NodeControl,
    ParkingArea, ParkingKind, Side, TrafficData,
};
use ocw_world::{TileKey, WorldPackage};

use crate::areas::{assemble_rings, inside_rings};
use crate::osm::{OsmData, OsmError, RawOsm, RawRelation, Tags, Way, par_elements};
use crate::spatial::SegmentIndex;

/// A ring or line in package metres.
type Ring = Vec<[f64; 2]>;
/// A way's nodes and its kept tags (`None`: a relation member).
type WayEntry = (Vec<i64>, Option<Tags>);
/// Place centres and radii by 2 km cell.
type TownCells = HashMap<(i64, i64), Vec<([f64; 2], f64)>>;
/// An aeroway element: id, centre, outline, tags.
type Element<'a> = (i64, [f64; 2], Ring, &'a Tags);

/// Keys kept on the elements of [`TrafficOsm`].
fn kept(k: &str) -> bool {
    matches!(
        k,
        "highway"
            | "public_transport"
            | "bus"
            | "name"
            | "ref"
            | "route"
            | "type"
            | "boundary"
            | "admin_level"
            | "ISO3166-1"
            | "ISO3166-1:alpha2"
            | "aeroway"
            | "aerodrome:type"
            | "aerodrome"
            | "icao"
            | "iata"
            | "ele"
            | "sport"
            | "military"
            | "amenity"
            | "healthcare"
            | "emergency"
            | "capacity"
            | "parking"
            | "access"
            | "width"
            | "surface"
    )
}

fn keep(tags: impl Iterator<Item = (impl AsRef<str>, impl AsRef<str>)>) -> Tags {
    tags.filter(|(k, _)| kept(k.as_ref()))
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect()
}

fn tag<'a>(t: &'a Tags, k: &str) -> Option<&'a str> {
    t.get(k).map(String::as_str)
}

/// A bus line relation (`type=route`, `route=bus | trolleybus`).
fn is_bus_route(t: &Tags) -> bool {
    tag(t, "type") == Some("route") && matches!(tag(t, "route"), Some("bus" | "trolleybus"))
}

/// A country outline (`boundary=administrative`, `admin_level=2`, with an
/// ISO 3166-1 code).
fn country_of_boundary(t: &Tags) -> Option<String> {
    (tag(t, "boundary") == Some("administrative") && tag(t, "admin_level") == Some("2"))
        .then(|| tag(t, "ISO3166-1:alpha2").or_else(|| tag(t, "ISO3166-1")))
        .flatten()
        .map(str::to_string)
}

fn is_aerodrome_area(t: &Tags) -> bool {
    matches!(tag(t, "aeroway"), Some("aerodrome" | "heliport"))
}

/// Nodes and ways the traffic sections read.
fn interesting(t: &Tags) -> bool {
    matches!(
        tag(t, "aeroway"),
        Some("aerodrome" | "heliport" | "helipad" | "runway")
    ) || tag(t, "amenity") == Some("parking")
        || tag(t, "amenity") == Some("hospital")
        || tag(t, "healthcare") == Some("hospital")
        || is_bus_stop(t)
}

/// A bus stop node: `highway=bus_stop`, or a bus `public_transport=
/// platform | stop_position`.
fn is_bus_stop(t: &Tags) -> bool {
    tag(t, "highway") == Some("bus_stop")
        || (matches!(
            tag(t, "public_transport"),
            Some("platform" | "stop_position")
        ) && tag(t, "bus") == Some("yes"))
}

/// What of an extract the traffic sections need besides the road ways
/// ([`OsmData`]): bus line, country and aerodrome relations, the tagged
/// nodes and ways of [`interesting`], and the nodes they use.
#[derive(Debug, Clone, Default)]
pub struct TrafficOsm {
    pub nodes: HashMap<i64, (f64, f64)>,
    /// Tagged nodes of interest, by id order.
    pub points: Vec<(i64, Tags)>,
    /// Ways by id: tagged ways of interest, and relation members (no tags).
    pub ways: HashMap<i64, WayEntry>,
    pub relations: Vec<RawRelation>,
}

impl TrafficOsm {
    /// Three passes: relations, ways, nodes.
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        let relations = par_elements(path, |el, out| {
            if let osmpbf::Element::Relation(r) = el {
                let tags = keep(r.tags());
                if !(is_bus_route(&tags)
                    || country_of_boundary(&tags).is_some()
                    || (is_aerodrome_area(&tags) && tag(&tags, "type") == Some("multipolygon")))
                {
                    return;
                }
                let members = r
                    .members()
                    .map(|m| {
                        let t = match m.member_type {
                            osmpbf::RelMemberType::Node => "node",
                            osmpbf::RelMemberType::Way => "way",
                            osmpbf::RelMemberType::Relation => "relation",
                        };
                        (
                            t.to_string(),
                            m.member_id,
                            m.role().unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                out.push(RawRelation {
                    id: r.id(),
                    tags,
                    members,
                });
            }
        })?;
        // Bus lines' ways are road ways (in [`OsmData`]); only the outlines'
        // member ways are read here.
        let members: HashSet<i64> = relations
            .iter()
            .filter(|r| !is_bus_route(&r.tags))
            .flat_map(|r| r.members.iter().filter(|m| m.0 == "way").map(|m| m.1))
            .collect();
        let found = par_elements(path, |el, out| {
            if let osmpbf::Element::Way(w) = el {
                let tagged = w
                    .tags()
                    .any(|(k, _)| matches!(k, "aeroway" | "amenity" | "healthcare"))
                    .then(|| keep(w.tags()))
                    .filter(interesting);
                if tagged.is_some() || members.contains(&w.id()) {
                    out.push((w.id(), w.refs().collect::<Vec<_>>(), tagged));
                }
            }
        })?;
        let ways: HashMap<i64, WayEntry> =
            found.into_iter().map(|(id, n, t)| (id, (n, t))).collect();
        let stop_members: HashSet<i64> = relations
            .iter()
            .filter(|r| is_bus_route(&r.tags))
            .flat_map(|r| r.members.iter().filter(|m| m.0 == "node").map(|m| m.1))
            .collect();
        let needed: HashSet<i64> = ways.values().flat_map(|w| w.0.iter().copied()).collect();
        let found = par_elements(path, |el, out| {
            let (id, lat, lon, tags): (i64, f64, f64, Vec<(&str, &str)>) = match el {
                osmpbf::Element::Node(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                osmpbf::Element::DenseNode(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                _ => return,
            };
            let point = (!tags.is_empty())
                .then(|| keep(tags.iter().copied()))
                .filter(|t| interesting(t) || stop_members.contains(&id));
            if point.is_some() || needed.contains(&id) || stop_members.contains(&id) {
                out.push((id, (lat, lon), point));
            }
        })?;
        let mut d = Self {
            ways,
            relations,
            ..Default::default()
        };
        for (id, p, t) in found {
            d.nodes.insert(id, p);
            if let Some(t) = t {
                d.points.push((id, t));
            }
        }
        d.points.sort_by_key(|p| p.0);
        d.relations.sort_by_key(|r| r.id);
        Ok(d)
    }

    /// Reads several extracts and merges them (by id).
    pub fn read_pbfs(paths: &[PathBuf]) -> Result<Self, String> {
        let mut out = Self::default();
        let (mut points, mut relations) = (HashSet::new(), HashSet::new());
        for p in paths {
            let d = Self::read_pbf(p).map_err(|e| format!("{}: {e}", p.display()))?;
            out.nodes.extend(d.nodes);
            for (id, w) in d.ways {
                out.ways.entry(id).or_insert(w);
            }
            for n in d.points {
                if points.insert(n.0) {
                    out.points.push(n);
                }
            }
            for r in d.relations {
                if relations.insert(r.id) {
                    out.relations.push(r);
                }
            }
        }
        Ok(out)
    }

    /// The same from a parsed OSM XML file (fixtures).
    pub fn from_raw(raw: &RawOsm) -> Self {
        let relations: Vec<RawRelation> = raw
            .relations
            .iter()
            .filter(|r| {
                is_bus_route(&r.tags)
                    || country_of_boundary(&r.tags).is_some()
                    || is_aerodrome_area(&r.tags)
            })
            .map(|r| RawRelation {
                id: r.id,
                tags: keep(r.tags.iter()),
                members: r.members.clone(),
            })
            .collect();
        let members: HashSet<i64> = relations
            .iter()
            .filter(|r| !is_bus_route(&r.tags))
            .flat_map(|r| r.members.iter().filter(|m| m.0 == "way").map(|m| m.1))
            .collect();
        let stop_members: HashSet<i64> = relations
            .iter()
            .filter(|r| is_bus_route(&r.tags))
            .flat_map(|r| r.members.iter().filter(|m| m.0 == "node").map(|m| m.1))
            .collect();
        let ways: HashMap<i64, WayEntry> = raw
            .ways
            .iter()
            .filter_map(|w| {
                let t = Some(keep(w.tags.iter())).filter(interesting);
                (t.is_some() || members.contains(&w.id)).then(|| (w.id, (w.nodes.clone(), t)))
            })
            .collect();
        let mut points: Vec<(i64, Tags)> = raw
            .node_tags
            .iter()
            .map(|(id, t)| (*id, keep(t.iter())))
            .filter(|(id, t)| interesting(t) || stop_members.contains(id))
            .collect();
        points.sort_by_key(|p| p.0);
        Self {
            nodes: raw.nodes.clone(),
            points,
            ways,
            relations,
        }
    }

    /// Keeps what lies within (south, west, north, east) – relations
    /// whole.
    pub fn retain_near(&mut self, bounds: (f64, f64, f64, f64)) {
        let nodes = &self.nodes;
        let inside = |id: &i64| {
            nodes
                .get(id)
                .is_some_and(|p| crate::osm::way_box_overlaps(std::iter::once(p), bounds))
        };
        self.points.retain(|(id, _)| inside(id));
        let members: HashSet<i64> = self
            .relations
            .iter()
            .flat_map(|r| r.members.iter().map(|m| m.1))
            .collect();
        self.ways.retain(|id, w| {
            members.contains(id)
                || crate::osm::way_box_overlaps(w.0.iter().filter_map(|n| nodes.get(n)), bounds)
        });
    }

    fn coords(&self, nodes: &[i64]) -> Option<Vec<(f64, f64)>> {
        nodes.iter().map(|n| self.nodes.get(n).copied()).collect()
    }
}

/// What [`add_traffic`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficStats {
    pub edges: usize,
    pub motor_edges: usize,
    pub oneway: usize,
    pub roundabout: usize,
    pub no_hgv: usize,
    pub parking_sides: usize,
    pub urban: usize,
    pub signals: usize,
    pub stops_signs: usize,
    pub give_way: usize,
    pub bus_stops: usize,
    pub bus_routes: usize,
    pub car_parks: usize,
    pub countries: Vec<(Country, usize)>,
    pub aerodromes: usize,
    pub nearby: usize,
    pub runways: usize,
    pub helipads: usize,
}

impl TrafficStats {
    pub fn summary(&self) -> String {
        let countries: Vec<String> = self
            .countries
            .iter()
            .map(|(c, n)| format!("{} {n}", c.code()))
            .collect();
        format!(
            "{} edges ({} with motor traffic; {} one-way, {} roundabout, {} no lorries, {} with kerb parking, {} in town; countries {}), {} signals, {} stop and {} give-way signs, {} bus stops, {} bus lines, {} car parks",
            self.edges,
            self.motor_edges,
            self.oneway,
            self.roundabout,
            self.no_hgv,
            self.parking_sides,
            self.urban,
            countries.join(", "),
            self.signals,
            self.stops_signs,
            self.give_way,
            self.bus_stops,
            self.bus_routes,
            self.car_parks
        )
    }

    pub fn aero_summary(&self) -> String {
        format!(
            "{} aerodromes in the package, {} nearby outside it, {} runways, {} helipads",
            self.aerodromes, self.nearby, self.runways, self.helipads
        )
    }
}

/// Place radii for "in town" (m).
fn town_radius(kind: PlaceKind) -> f64 {
    match kind {
        PlaceKind::City => 1500.0,
        PlaceKind::Town => 900.0,
        PlaceKind::Suburb => 600.0,
        PlaceKind::Village => 450.0,
        PlaceKind::Neighbourhood => 300.0,
        PlaceKind::Hamlet => 150.0,
        _ => 0.0,
    }
}

/// The country outlines of the extract (complete rings only), in package
/// metres, with their boxes.
struct Countries {
    outlines: Vec<(Country, Vec<Ring>, [f64; 4])>,
}

impl Countries {
    fn new(osm: &TrafficOsm, proj: &LocalProjection) -> Self {
        let mut outlines = Vec::new();
        for r in &osm.relations {
            let Some(code) = country_of_boundary(&r.tags) else {
                continue;
            };
            let parts: Vec<&[i64]> = r
                .members
                .iter()
                .filter(|m| m.0 == "way" && matches!(m.2.as_str(), "" | "outer"))
                .filter_map(|m| osm.ways.get(&m.1).map(|w| w.0.as_slice()))
                .collect();
            let rings: Vec<Vec<[f64; 2]>> = assemble_rings(&parts)
                .iter()
                .filter_map(|ring| osm.coords(ring))
                .map(|c| c.iter().map(|&(la, lo)| proj.project(la, lo)).collect())
                .collect();
            if rings.is_empty() {
                continue;
            }
            let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for p in rings.iter().flatten() {
                b = [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ];
            }
            outlines.push((Country::from_code(&code), rings, b));
        }
        Self { outlines }
    }

    fn at(&self, p: [f64; 2]) -> Option<Country> {
        self.outlines
            .iter()
            .find(|(_, rings, b)| {
                p[0] >= b[0]
                    && p[0] <= b[2]
                    && p[1] >= b[1]
                    && p[1] <= b[3]
                    && inside_rings(p, rings)
            })
            .map(|o| o.0)
    }
}

/// The place centres and radii, in a grid of 2 km cells.
struct Towns {
    cells: TownCells,
}

const TOWN_CELL_M: f64 = 2000.0;

impl Towns {
    fn new(pkg: &WorldPackage) -> Self {
        let mut cells: TownCells = HashMap::new();
        for p in pkg.places.iter().flat_map(|p| p.places.iter()) {
            let r = town_radius(p.kind);
            if r <= 0.0 {
                continue;
            }
            let c = [f64::from(p.pos[0]), f64::from(p.pos[1])];
            let k = (
                (c[0] / TOWN_CELL_M).floor() as i64,
                (c[1] / TOWN_CELL_M).floor() as i64,
            );
            cells.entry(k).or_default().push((c, r));
        }
        Self { cells }
    }

    fn contains(&self, p: [f64; 2]) -> bool {
        let (kx, kz) = (
            (p[0] / TOWN_CELL_M).floor() as i64,
            (p[1] / TOWN_CELL_M).floor() as i64,
        );
        (-1..=1).any(|dx| {
            (-1..=1).any(|dz| {
                self.cells.get(&(kx + dx, kz + dz)).is_some_and(|v| {
                    v.iter()
                        .any(|(c, r)| (p[0] - c[0]).hypot(p[1] - c[1]) <= *r)
                })
            })
        })
    }
}

/// The point halfway along an edge.
fn midpoint(e: &Edge) -> [f64; 2] {
    let half = e.length_m() / 2.0;
    let mut walked = 0.0;
    for w in e.points.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if walked + l >= half && l > 0.0 {
            let t = (half - walked) / l;
            return [
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ];
        }
        walked += l;
    }
    e.points[0]
}

/// The way's node positions (package metres); `None` if a node is missing.
fn way_points(osm: &OsmData, w: &Way, proj: &LocalProjection) -> Option<Vec<[f64; 2]>> {
    w.nodes
        .iter()
        .map(|n| osm.nodes.get(n).map(|&(la, lo)| proj.project(la, lo)))
        .collect()
}

/// Where an edge lies on its way: whether it runs against the way, and
/// the lowest and highest index of the way nodes it passes. `None` when
/// the edge's points do not match the way's nodes (edited roads).
fn on_way(e: &Edge, pts: &[[f64; 2]]) -> Option<Placement> {
    const EPS: f64 = 0.05;
    let matches = |p: [f64; 2]| -> Vec<usize> {
        pts.iter()
            .enumerate()
            .filter(|(_, q)| (q[0] - p[0]).hypot(q[1] - p[1]) < EPS)
            .map(|(i, _)| i)
            .collect()
    };
    let closed = pts.len() > 2 && pts.first() == pts.last();
    let last = pts.len().saturating_sub(1);
    let mut reversed = None;
    'find: for w in e.points.windows(2) {
        let (a, b) = (matches(w[0]), matches(w[1]));
        for &i in &a {
            for &j in &b {
                if j == i + 1 || (closed && i == last && j == 1) {
                    reversed = Some(false);
                    break 'find;
                }
                if i == j + 1 || (closed && i == 0 && j + 1 == last) {
                    reversed = Some(true);
                    break 'find;
                }
            }
        }
    }
    let Some(reversed) = reversed else {
        return along_way(e, pts);
    };
    let idx: Vec<usize> = e.points.iter().flat_map(|&p| matches(p)).collect();
    Some(Placement {
        reversed,
        lo: idx.iter().copied().min()?,
        hi: idx.iter().copied().max()?,
    })
}

/// [`on_way`] for an edge whose points moved (cleaned, cut at the
/// border): its ends' feet on the way's line within 10 m, in way order.
fn along_way(e: &Edge, pts: &[[f64; 2]]) -> Option<Placement> {
    let at = |p: [f64; 2]| -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for (k, s) in pts.windows(2).enumerate() {
            let (d, t, _) = crate::spatial::foot_on_segment(p, s[0], s[1]);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, k as f64 + t));
            }
        }
        best.filter(|b| b.0 < 10.0).map(|b| b.1)
    };
    let (a, b) = (at(e.points[0])?, at(*e.points.last()?)?);
    if (a - b).abs() < 1e-9 {
        return None;
    }
    Some(Placement {
        reversed: a > b,
        lo: a.min(b).floor() as usize,
        hi: a.max(b).ceil() as usize,
    })
}

/// An edge's place on its OSM way ([`on_way`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placement {
    reversed: bool,
    lo: usize,
    hi: usize,
}

/// Builds `TRFC` and `AERO` into `pkg` (its graph, places and land use as
/// built) from the road ways of `osm` and the traffic elements of `data`.
pub fn add_traffic(
    pkg: &mut WorldPackage,
    osm: &OsmData,
    data: &TrafficOsm,
    proj: &LocalProjection,
) -> TrafficStats {
    let mut stats = TrafficStats::default();
    let Some(graph) = pkg.graph.clone() else {
        return stats;
    };
    let size = f64::from(pkg.tile_size_m);
    let countries = Countries::new(data, proj);
    let package_country = countries.at([0.0, 0.0]).unwrap_or_else(|| {
        pkg.time_zone
            .as_deref()
            .map_or(Country::Other, Country::from_time_zone)
    });
    let towns = Towns::new(pkg);
    let built_up = |p: [f64; 2]| {
        pkg.landuse.as_ref().is_some_and(|l| {
            matches!(
                l.class_at(size, p[0], p[1]),
                Some(LandClass::Residential | LandClass::Commercial)
            )
        })
    };
    let ways: HashMap<i64, &Way> = osm.ways.iter().map(|w| (w.id, w)).collect();
    // --- Edge rules ------------------------------------------------------
    let mut t = TrafficData {
        country: package_country,
        ..Default::default()
    };
    let mut placement: Vec<Option<Placement>> = Vec::with_capacity(graph.edges.len());
    let mut country_count: HashMap<Country, usize> = HashMap::new();
    let mut way_pts: HashMap<i64, Option<Vec<[f64; 2]>>> = HashMap::new();
    for e in &graph.edges {
        let mid = midpoint(e);
        let country = countries.at(mid).unwrap_or(package_country);
        let urban = towns.contains(mid) || built_up(mid);
        let way = ways.get(&e.osm_way).copied();
        let pts = way.and_then(|w| {
            way_pts
                .entry(w.id)
                .or_insert_with(|| way_points(osm, w, proj))
                .as_deref()
                .map(<[[f64; 2]]>::to_vec)
        });
        let place = pts.as_deref().and_then(|p| on_way(e, p));
        placement.push(place);
        let cx = EdgeContext {
            class: e.class,
            reversed: place.is_some_and(|p| p.reversed),
            country,
            urban,
            width_m: e.width_m,
        };
        let rules = match way {
            Some(w) => EdgeRules::from_osm(&cx, |k| w.tag(k)),
            None => EdgeRules::implied(e.class, country, urban),
        };
        *country_count.entry(rules.country).or_default() += 1;
        stats.edges += 1;
        if rules.has_traffic() {
            stats.motor_edges += 1;
            stats.oneway += usize::from(rules.oneway != ocw_world::graph::Oneway::No);
            stats.roundabout += usize::from(rules.roundabout);
            stats.no_hgv += usize::from(!rules.access.allows(ocw_world::traffic::Vehicle::Hgv));
            stats.parking_sides += rules.parking.iter().filter(|p| p.has_cars()).count();
            stats.urban += usize::from(rules.urban);
        }
        t.edges.insert(e.id, rules);
    }
    let mut cc: Vec<(Country, usize)> = country_count.into_iter().collect();
    cc.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    stats.countries = cc;
    let motor = |i: usize| {
        t.edges
            .get(&graph.edges[i].id)
            .is_some_and(EdgeRules::has_traffic)
    };
    let index = SegmentIndex::new(&graph, 100.0, motor);
    // --- Signals and signs ----------------------------------------------
    t.controls = controls(osm, &graph, &ways, &placement, &t, proj);
    for c in &t.controls {
        match c.kind {
            ControlKind::Signals | ControlKind::CrossingSignals => stats.signals += 1,
            ControlKind::Stop => stats.stops_signs += 1,
            ControlKind::GiveWay => stats.give_way += 1,
        }
    }
    t.sort();
    // --- Bus stops and lines ---------------------------------------------
    let on_tiles = |p: [f64; 2]| pkg.tiles.contains_key(&TileKey::containing(p, size));
    let near_edge = |p: [f64; 2], radius: f64| -> Option<(EdgeId, f32, Side)> {
        let h = index.near(&graph, p, radius).into_iter().next()?;
        let e = &graph.edges[h.edge];
        // The segment at the foot: the side of p seen driving forward.
        let k = (0..e.points.len() - 1)
            .min_by(|&a, &b| {
                let d =
                    |s: usize| crate::spatial::foot_on_segment(p, e.points[s], e.points[s + 1]).0;
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0);
        let (a, b) = (e.points[k], e.points[k + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        // Right of (dx, dz) in x east, z south is (−dz, dx).
        let right = (p[0] - h.foot[0]) * -dz + (p[1] - h.foot[1]) * dx > 0.0;
        Some((
            e.id,
            h.along as f32,
            if right { Side::Right } else { Side::Left },
        ))
    };
    let mut stop_index: HashMap<i64, u32> = HashMap::new();
    for (id, tags) in &data.points {
        if !is_bus_stop(tags) {
            continue;
        }
        let Some(&(la, lo)) = data.nodes.get(id) else {
            continue;
        };
        let p = proj.project(la, lo);
        if !on_tiles(p) {
            continue;
        }
        let (edge, along_m, side) =
            near_edge(p, 30.0).map_or((None, 0.0, Side::Right), |n| (Some(n.0), n.1, n.2));
        stop_index.insert(*id, t.stops.len() as u32);
        t.stops.push(BusStop {
            osm_node: *id,
            pos: [p[0] as f32, p[1] as f32],
            name: tag(tags, "name").unwrap_or("").to_string(),
            edge,
            along_m,
            side,
        });
    }
    t.bus_routes = bus_routes(data, &graph, &ways, &placement, &stop_index);
    // --- Car parks -------------------------------------------------------
    let parks = car_parks(data, proj);
    for (osm_id, p, area_m2, tags) in parks {
        if !on_tiles(p) {
            continue;
        }
        let capacity = tag(&tags, "capacity").and_then(|c| c.trim().parse::<u16>().ok());
        let estimated = capacity.is_none();
        let capacity = capacity.unwrap_or_else(|| {
            area_m2.map_or(20, |a| (a / 25.0).round().clamp(1.0, 5000.0) as u16)
        });
        let (edge, along_m) = near_edge(p, 150.0).map_or((None, 0.0), |n| (Some(n.0), n.1));
        t.parking.push(ParkingArea {
            osm_id,
            pos: [p[0] as f32, p[1] as f32],
            capacity,
            estimated,
            public: !matches!(tag(&tags, "access"), Some("private" | "customers" | "no")),
            kind: ParkingKind::from_osm(tag(&tags, "parking")),
            edge,
            along_m,
        });
    }
    stats.bus_stops = t.stops.len();
    stats.bus_routes = t.bus_routes.len();
    stats.car_parks = t.parking.len();
    // --- Aerodromes ------------------------------------------------------
    let mut aero = aerodromes(data, proj, &on_tiles);
    stats.aerodromes = aero.aerodromes.len();
    stats.runways = aero.runways.len();
    stats.helipads = aero.helipads.len();
    let centre = pkg.region.as_ref().map_or(pkg.origin, |r| {
        let [s, w, n, e] = r.bounds();
        ((s + n) / 2.0, (w + e) / 2.0)
    });
    stats.nearby = aero.add_nearby(centre, NEARBY_RADIUS_KM, proj);
    pkg.traffic = Some(Arc::new(t));
    pkg.aero = Some(aero);
    stats
}

/// The signals, stop and give-way signs of the road ways' nodes on the
/// motor edges.
fn controls(
    osm: &OsmData,
    graph: &RoadGraph,
    ways: &HashMap<i64, &Way>,
    placement: &[Option<Placement>],
    t: &TrafficData,
    proj: &LocalProjection,
) -> Vec<NodeControl> {
    let motor = |i: usize| {
        t.edges
            .get(&graph.edges[i].id)
            .is_some_and(EdgeRules::has_traffic)
    };
    let mut by_way: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if motor(i) {
            by_way.entry(e.osm_way).or_default().push(i);
        }
    }
    let adjacency = graph.adjacency();
    let mut nodes: Vec<(i64, ControlKind, &Tags)> = osm
        .node_tags
        .iter()
        .filter_map(|(id, tags)| {
            let h = tags.get("highway").map(String::as_str);
            let c = tags.get("crossing").map(String::as_str);
            let kind = match h {
                Some("traffic_signals") if c == Some("traffic_signals") => {
                    ControlKind::CrossingSignals
                }
                Some("traffic_signals") => ControlKind::Signals,
                Some("stop") => ControlKind::Stop,
                Some("give_way") => ControlKind::GiveWay,
                Some("crossing")
                    if c == Some("traffic_signals")
                        || tags.get("crossing:signals").map(String::as_str) == Some("yes") =>
                {
                    ControlKind::CrossingSignals
                }
                _ => return None,
            };
            Some((*id, kind, tags))
        })
        .collect();
    nodes.sort_by_key(|n| n.0);
    // The motor ways through each control node.
    let wanted: HashSet<i64> = nodes.iter().map(|n| n.0).collect();
    let mut ways_at: HashMap<i64, Vec<i64>> = HashMap::new();
    for wid in by_way.keys() {
        if let Some(w) = ways.get(wid) {
            for n in &w.nodes {
                if wanted.contains(n) {
                    ways_at.entry(*n).or_default().push(*wid);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(EdgeId, i64)> = HashSet::new();
    for (node, kind, tags) in nodes {
        let Some(wids) = ways_at.get(&node) else {
            continue;
        };
        let Some(&(la, lo)) = osm.nodes.get(&node) else {
            continue;
        };
        let p = proj.project(la, lo);
        let dir = tags
            .get("traffic_signals:direction")
            .or_else(|| tags.get("direction"))
            .map(String::as_str);
        // The edges of the ways through the node, and where on them.
        let mut hits: Vec<(usize, f64, f64)> = Vec::new(); // edge, along, length
        for wid in wids {
            for &ei in &by_way[wid] {
                let e = &graph.edges[ei];
                let (mut walked, mut best) = (0.0, (f64::INFINITY, 0.0));
                for s in e.points.windows(2) {
                    let (d, tt, _) = crate::spatial::foot_on_segment(p, s[0], s[1]);
                    let l = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                    if d < best.0 {
                        best = (d, walked + tt * l);
                    }
                    walked += l;
                }
                if best.0 <= e.width_m / 2.0 + 2.0 {
                    hits.push((ei, best.1, walked));
                }
            }
        }
        if hits.is_empty() {
            continue;
        }
        // At a junction vertex: every motor edge ending there.
        let vertex = hits.iter().find_map(|&(ei, along, len)| {
            let e = &graph.edges[ei];
            let v = if along < 0.5 {
                Some(e.from)
            } else if along > len - 0.5 {
                Some(e.to)
            } else {
                None
            }?;
            (adjacency[v.0 as usize].len() > 2).then_some(v)
        });
        let stop_like = matches!(kind, ControlKind::Stop | ControlKind::GiveWay);
        if let Some(v) = vertex {
            let mut arms: Vec<usize> = adjacency[v.0 as usize]
                .iter()
                .copied()
                .filter(|&i| motor(i))
                .collect();
            if stop_like {
                // A sign at the junction node is for the minor roads.
                let top = arms.iter().map(|&i| graph.edges[i].class).min();
                if arms.iter().any(|&i| Some(graph.edges[i].class) != top) {
                    arms.retain(|&i| Some(graph.edges[i].class) != top);
                }
            }
            for i in arms {
                let e = &graph.edges[i];
                let toward_to = e.to == v;
                if !seen.insert((e.id, node)) {
                    continue;
                }
                out.push(NodeControl {
                    edge: e.id,
                    along_m: if toward_to { e.length_m() as f32 } else { 0.0 },
                    kind,
                    applies: if toward_to {
                        Applies::Forward
                    } else {
                        Applies::Backward
                    },
                    at_junction: true,
                    osm_node: node,
                });
            }
            continue;
        }
        for (ei, along, len) in hits {
            let e = &graph.edges[ei];
            let reversed = placement[ei].is_some_and(|p| p.reversed);
            let by_tag = match dir {
                Some("forward") => Some(Applies::Forward),
                Some("backward") => Some(Applies::Backward),
                Some("both") => Some(Applies::Both),
                _ => None,
            }
            .map(|a| match (a, reversed) {
                (Applies::Forward, true) => Applies::Backward,
                (Applies::Backward, true) => Applies::Forward,
                (a, _) => a,
            });
            let applies = by_tag.unwrap_or(if stop_like {
                // Traffic meets the sign on its way to the nearer end.
                if along > len / 2.0 {
                    Applies::Forward
                } else {
                    Applies::Backward
                }
            } else {
                Applies::Both
            });
            if !seen.insert((e.id, node)) {
                continue;
            }
            out.push(NodeControl {
                edge: e.id,
                along_m: along as f32,
                kind,
                applies,
                at_junction: false,
                osm_node: node,
            });
        }
    }
    out
}

/// The bus lines on the graph: member ways chained into a driving order,
/// each way's edges in that order; stops (role `stop…`, else
/// `platform…`) by their index.
fn bus_routes(
    data: &TrafficOsm,
    graph: &RoadGraph,
    ways: &HashMap<i64, &Way>,
    placement: &[Option<Placement>],
    stop_index: &HashMap<i64, u32>,
) -> Vec<BusRoute> {
    let mut edges_of: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        edges_of.entry(e.osm_way).or_default().push(i);
    }
    let mut out = Vec::new();
    for r in data.relations.iter().filter(|r| is_bus_route(&r.tags)) {
        let members: Vec<(&Way, &str)> = r
            .members
            .iter()
            .filter(|m| m.0 == "way")
            .filter_map(|m| ways.get(&m.1).map(|w| (*w, m.2.as_str())))
            .collect();
        let mut legs: Vec<(EdgeId, bool)> = Vec::new();
        // The node the bus came in by (from the member before).
        let mut entry: Option<i64> = None;
        for (k, &(w, role)) in members.iter().enumerate() {
            if w.nodes.len() < 2 {
                entry = None;
                continue;
            }
            let pos = |n: i64| w.nodes.iter().position(|&m| m == n);
            let last = w.nodes.len() - 1;
            let closed = w.nodes[0] == w.nodes[last];
            // Where it leaves: a node shared with the next member (its
            // ends first).
            let shared = members.get(k + 1).and_then(|(next, _)| {
                let ends = [next.nodes[0], *next.nodes.last().expect("way has nodes")];
                ends.iter()
                    .copied()
                    .find(|n| pos(*n).is_some())
                    .or_else(|| next.nodes.iter().copied().find(|n| pos(*n).is_some()))
            });
            let (a, b) = (entry.and_then(pos), shared.and_then(pos));
            if closed {
                // Roundabouts: the core joins the legs around them.
                entry = shared;
                continue;
            }
            let forward = match (a, b) {
                (Some(a), Some(b)) if a != b => b > a,
                (None, Some(0)) => false,
                (None, Some(b)) if b == last => true,
                (Some(a), None) if a == last => false,
                (Some(0), None) => true,
                _ => role != "backward",
            };
            // The part of the way driven: from the entry to the way out.
            let from = a.unwrap_or(if forward { 0 } else { last });
            let to = b.unwrap_or(if forward { last } else { 0 });
            let (lo, hi) = (from.min(to), from.max(to));
            entry = Some(shared.unwrap_or(w.nodes[if forward { last } else { 0 }]));
            let Some(es) = edges_of.get(&w.id) else {
                continue;
            };
            let mut placed: Vec<(usize, bool, usize)> = es
                .iter()
                .filter_map(|&i| placement[i].map(|p| (p, i)))
                .filter(|(p, _)| p.lo >= lo && p.hi <= hi)
                .map(|(p, i)| (p.lo, p.reversed, i))
                .collect();
            placed.sort_unstable();
            if !forward {
                placed.reverse();
            }
            for (_, rev, i) in placed {
                legs.push((graph.edges[i].id, forward != rev));
            }
        }
        if legs.is_empty() {
            continue;
        }
        let roles = |prefix: &str| -> Vec<u32> {
            r.members
                .iter()
                .filter(|m| m.0 == "node" && m.2.starts_with(prefix))
                .filter_map(|m| stop_index.get(&m.1).copied())
                .collect()
        };
        let mut stops = roles("stop");
        if stops.is_empty() {
            stops = roles("platform");
        }
        stops.dedup();
        out.push(BusRoute {
            osm_relation: r.id,
            reference: tag(&r.tags, "ref").unwrap_or("").to_string(),
            name: tag(&r.tags, "name").unwrap_or("").to_string(),
            legs,
            stops,
        });
    }
    out
}

/// Area (m²) and centroid of a ring in package metres.
fn ring_area_centroid(ring: &[[f64; 2]]) -> (f64, [f64; 2]) {
    let (mut a, mut cx, mut cz) = (0.0, 0.0, 0.0);
    for i in 0..ring.len() {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        let cross = p[0] * q[1] - q[0] * p[1];
        a += cross;
        cx += (p[0] + q[0]) * cross;
        cz += (p[1] + q[1]) * cross;
    }
    if a.abs() < 1e-9 {
        let n = ring.len().max(1) as f64;
        let s = ring
            .iter()
            .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
        return (0.0, [s[0] / n, s[1] / n]);
    }
    ((a / 2.0).abs(), [cx / (3.0 * a), cz / (3.0 * a)])
}

/// The car parks: (OSM id, ways −id; centre; area if mapped as an area;
/// tags).
fn car_parks(data: &TrafficOsm, proj: &LocalProjection) -> Vec<(i64, [f64; 2], Option<f64>, Tags)> {
    let is_park = |t: &Tags| tag(t, "amenity") == Some("parking");
    let mut out = Vec::new();
    for (id, t) in &data.points {
        if is_park(t)
            && let Some(&(la, lo)) = data.nodes.get(id)
        {
            out.push((*id, proj.project(la, lo), None, t.clone()));
        }
    }
    let mut ways: Vec<(&i64, &WayEntry)> = data.ways.iter().collect();
    ways.sort_by_key(|w| *w.0);
    for (id, (nodes, t)) in ways {
        let Some(t) = t.as_ref().filter(|t| is_park(t)) else {
            continue;
        };
        let Some(c) = data.coords(nodes) else {
            continue;
        };
        let ring: Vec<[f64; 2]> = c.iter().map(|&(la, lo)| proj.project(la, lo)).collect();
        let (area, centre) = ring_area_centroid(&ring);
        out.push((-*id, centre, (area > 0.0).then_some(area), t.clone()));
    }
    out
}

/// The aerodromes, runways and helipads of `data` near the package.
fn aerodromes(
    data: &TrafficOsm,
    proj: &LocalProjection,
    on_tiles: &dyn Fn([f64; 2]) -> bool,
) -> Aero {
    let mut a = Aero::default();
    // (id, centre, ring, tags) of every aeroway element.
    let mut elements: Vec<Element<'_>> = Vec::new();
    for (id, t) in &data.points {
        if let Some(&(la, lo)) = data.nodes.get(id) {
            elements.push((*id, proj.project(la, lo), Vec::new(), t));
        }
    }
    let mut ways: Vec<(&i64, &WayEntry)> = data.ways.iter().collect();
    ways.sort_by_key(|w| *w.0);
    for (id, (nodes, t)) in ways {
        let (Some(t), Some(c)) = (t.as_ref(), data.coords(nodes)) else {
            continue;
        };
        let line: Vec<[f64; 2]> = c.iter().map(|&(la, lo)| proj.project(la, lo)).collect();
        let centre = ring_area_centroid(&line).1;
        elements.push((-*id, centre, line, t));
    }
    for r in &data.relations {
        if !is_aerodrome_area(&r.tags) {
            continue;
        }
        let parts: Vec<&[i64]> = r
            .members
            .iter()
            .filter(|m| m.0 == "way" && matches!(m.2.as_str(), "" | "outer"))
            .filter_map(|m| data.ways.get(&m.1).map(|w| w.0.as_slice()))
            .collect();
        let ring = assemble_rings(&parts)
            .into_iter()
            .filter_map(|ring| data.coords(&ring))
            .map(|c| {
                c.iter()
                    .map(|&(la, lo)| proj.project(la, lo))
                    .collect::<Vec<_>>()
            })
            .max_by(|x, y| ring_area_centroid(x).0.total_cmp(&ring_area_centroid(y).0));
        if let Some(ring) = ring {
            elements.push((-r.id, ring_area_centroid(&ring).1, ring, &r.tags));
        }
    }
    let hospitals: Vec<[f64; 2]> = elements
        .iter()
        .filter(|e| {
            tag(e.3, "amenity") == Some("hospital") || tag(e.3, "healthcare") == Some("hospital")
        })
        .map(|e| e.1)
        .collect();
    let ele = |t: &Tags| tag(t, "ele").and_then(ocw_world::landmarks::parse_ele);
    for (id, centre, _, t) in &elements {
        if tag(t, "aeroway") != Some("aerodrome") {
            continue;
        }
        let (lat, lon) = proj.unproject(*centre);
        a.aerodromes.push(Aerodrome {
            osm_id: *id,
            name: tag(t, "name").unwrap_or("").to_string(),
            icao: tag(t, "icao").unwrap_or("").to_string(),
            iata: tag(t, "iata").unwrap_or("").to_string(),
            kind: AerodromeKind::from_osm(|k| tag(t, k)),
            lat_lon: (lat, lon),
            pos: [centre[0] as f32, centre[1] as f32],
            ele_m: ele(t),
            outside: !on_tiles(*centre),
        });
    }
    for (_, _, line, t) in &elements {
        if tag(t, "aeroway") != Some("runway") || line.len() < 2 {
            continue;
        }
        // A line: its ends; an area: its two farthest corners.
        let closed = line.len() > 3 && line.first() == line.last();
        let (p, q) = if closed {
            let mut best = (0.0, line[0], line[1]);
            for (i, &u) in line.iter().enumerate() {
                for &v in &line[i + 1..] {
                    let d = (u[0] - v[0]).hypot(u[1] - v[1]);
                    if d > best.0 {
                        best = (d, u, v);
                    }
                }
            }
            (best.1, best.2)
        } else {
            (line[0], *line.last().expect("two points"))
        };
        let width = tag(t, "width")
            .and_then(|w| w.trim_end_matches('m').trim().parse::<f32>().ok())
            .unwrap_or_else(|| {
                if closed {
                    let len = (p[0] - q[0]).hypot(p[1] - q[1]);
                    (ring_area_centroid(line).0 / len.max(1.0)) as f32
                } else {
                    30.0
                }
            });
        let mut r = Runway::from_ends(
            tag(t, "ref").unwrap_or(""),
            proj.unproject(p),
            proj.unproject(q),
            width,
            proj,
        );
        r.surface = tag(t, "surface").unwrap_or("").to_string();
        // The aerodrome it belongs to: the nearest within 3 km.
        let c = r.centre();
        r.aerodrome = a
            .aerodromes
            .iter()
            .enumerate()
            .map(|(i, d)| (i, (d.pos[0] - c[0]).hypot(d.pos[1] - c[1])))
            .filter(|x| x.1 < 3000.0)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|x| x.0 as u16);
        a.runways.push(r);
    }
    for (id, centre, _, t) in &elements {
        let heliport = match tag(t, "aeroway") {
            Some("helipad") => false,
            Some("heliport") => true,
            _ => continue,
        };
        if !on_tiles(*centre) {
            continue;
        }
        let hospital = tag(t, "emergency") == Some("yes")
            || hospitals
                .iter()
                .any(|h| (h[0] - centre[0]).hypot(h[1] - centre[1]) < 300.0);
        a.helipads.push(Helipad {
            osm_id: *id,
            name: tag(t, "name").unwrap_or("").to_string(),
            pos: [centre[0] as f32, centre[1] as f32],
            ele_m: ele(t),
            hospital,
            heliport,
        });
    }
    a
}

#[cfg(test)]
mod tests;
