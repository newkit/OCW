//! The section `PLCE` (spec §4.7 *Places section*, §14 Phase 16 16A *Map
//! labels*): the named places of OSM – `place=city | town | village |
//! hamlet | suburb | neighbourhood | locality | isolated_dwelling` (and
//! `quarter`, read as a suburb) – as nodes, and as areas (closed ways,
//! multipolygon and boundary relations) where a place is mapped only as
//! an area: then at the area's centroid. An area whose place is also
//! mapped as a node of the same name inside it is left out (the node is
//! the label's position). Only places on the package's tiles are kept.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ocw_world::geo::LocalProjection;
use ocw_world::places::{
    NAME_LANGUAGES, OsmElement, Place, PlaceKind, Places, capital_level, parse_population,
    place_rank,
};
use ocw_world::{TileKey, WorldPackage};

use crate::areas::{assemble_rings, inside_rings};
use crate::osm::{OsmError, RawOsm, Tags, par_elements, way_box_overlaps};

/// Keys kept on place elements.
fn kept(k: &str) -> bool {
    matches!(
        k,
        "place" | "name" | "population" | "capital" | "admin_level" | "type"
    ) || k
        .strip_prefix("name:")
        .is_some_and(|l| NAME_LANGUAGES.contains(&l))
}

/// A named settlement label: `place` of a known kind and a `name`.
fn is_place(tags: &Tags) -> bool {
    tags.get("place")
        .is_some_and(|v| PlaceKind::from_osm(v).is_some())
        && tags.get("name").is_some_and(|n| !n.trim().is_empty())
}

fn keep(tags: impl Iterator<Item = (impl AsRef<str>, impl AsRef<str>)>) -> Tags {
    tags.filter(|(k, _)| kept(k.as_ref()))
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect()
}

/// The OSM places of an extract: place nodes, place ways, place
/// relations with their outer member ways, and the nodes those ways use.
#[derive(Debug, Clone, Default)]
pub struct PlaceData {
    /// Node id → (lat, lon): the place nodes' and the area ways' nodes.
    pub nodes: HashMap<i64, (f64, f64)>,
    /// Place nodes with their kept tags.
    pub points: Vec<(i64, Tags)>,
    /// Ways by id: place ways with their tags, relation members with none.
    pub ways: HashMap<i64, (Vec<i64>, Option<Tags>)>,
    /// Place relations (multipolygon, boundary): tags and outer way ids.
    pub relations: Vec<(i64, Tags, Vec<i64>)>,
}

impl PlaceData {
    /// Three passes: relations (their member ways), ways, nodes.
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        let relations = par_elements(path, |el, out| {
            if let osmpbf::Element::Relation(r) = el {
                let tags = keep(r.tags());
                if !is_place(&tags)
                    || !matches!(
                        tags.get("type").map(String::as_str),
                        Some("multipolygon" | "boundary")
                    )
                {
                    return;
                }
                let outer: Vec<i64> = r
                    .members()
                    .filter(|m| {
                        m.member_type == osmpbf::RelMemberType::Way
                            && matches!(m.role().unwrap_or(""), "" | "outer")
                    })
                    .map(|m| m.member_id)
                    .collect();
                out.push((r.id(), tags, outer));
            }
        })?;
        let members: HashSet<i64> = relations.iter().flat_map(|r| r.2.iter().copied()).collect();
        let found = par_elements(path, |el, out| {
            if let osmpbf::Element::Way(w) = el {
                let tags = keep(w.tags());
                let place = is_place(&tags);
                if place || members.contains(&w.id()) {
                    out.push((w.id(), w.refs().collect::<Vec<_>>(), place.then_some(tags)));
                }
            }
        })?;
        let ways: HashMap<i64, (Vec<i64>, Option<Tags>)> = found
            .into_iter()
            .map(|(id, nodes, tags)| (id, (nodes, tags)))
            .collect();
        let needed: HashSet<i64> = ways.values().flat_map(|w| w.0.iter().copied()).collect();
        let found = par_elements(path, |el, out| {
            let (id, lat, lon, tags): (i64, f64, f64, Vec<(&str, &str)>) = match el {
                osmpbf::Element::Node(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                osmpbf::Element::DenseNode(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                _ => return,
            };
            let place = tags
                .iter()
                .any(|(k, _)| *k == "place")
                .then(|| keep(tags.into_iter()));
            let place = place.filter(is_place);
            if place.is_some() || needed.contains(&id) {
                out.push((id, (lat, lon), place));
            }
        })?;
        let mut d = Self {
            ways,
            relations,
            ..Default::default()
        };
        for (id, p, tags) in found {
            d.nodes.insert(id, p);
            if let Some(t) = tags {
                d.points.push((id, t));
            }
        }
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
                if relations.insert(r.0) {
                    out.relations.push(r);
                }
            }
        }
        Ok(out)
    }

    /// The places of a parsed OSM XML file (fixtures).
    pub fn from_raw(raw: &RawOsm) -> Self {
        let relations: Vec<(i64, Tags, Vec<i64>)> = raw
            .relations
            .iter()
            .filter(|r| {
                is_place(&r.tags)
                    && matches!(
                        r.tags.get("type").map(String::as_str),
                        Some("multipolygon" | "boundary")
                    )
            })
            .map(|r| {
                let outer = r
                    .members
                    .iter()
                    .filter(|(t, _, role)| t == "way" && matches!(role.as_str(), "" | "outer"))
                    .map(|m| m.1)
                    .collect();
                (r.id, keep(r.tags.iter()), outer)
            })
            .collect();
        let members: HashSet<i64> = relations.iter().flat_map(|r| r.2.iter().copied()).collect();
        let ways: HashMap<i64, (Vec<i64>, Option<Tags>)> = raw
            .ways
            .iter()
            .filter(|w| is_place(&w.tags) || members.contains(&w.id))
            .map(|w| {
                let place = is_place(&w.tags).then(|| keep(w.tags.iter()));
                (w.id, (w.nodes.clone(), place))
            })
            .collect();
        let needed: HashSet<i64> = ways.values().flat_map(|w| w.0.iter().copied()).collect();
        let mut points: Vec<(i64, Tags)> = raw
            .node_tags
            .iter()
            .filter(|(_, t)| is_place(t))
            .map(|(id, t)| (*id, keep(t.iter())))
            .collect();
        points.sort_by_key(|p| p.0);
        let nodes = raw
            .nodes
            .iter()
            .filter(|(id, _)| needed.contains(id) || points.iter().any(|p| p.0 == **id))
            .map(|(id, p)| (*id, *p))
            .collect();
        Self {
            nodes,
            points,
            ways,
            relations,
        }
    }

    /// Keeps what overlaps (south, west, north, east).
    pub fn retain_near(&mut self, bounds: (f64, f64, f64, f64)) {
        let nodes = &self.nodes;
        self.points.retain(|(id, _)| {
            nodes
                .get(id)
                .is_some_and(|p| way_box_overlaps(std::iter::once(p), bounds))
        });
        let near = |w: &[i64]| way_box_overlaps(w.iter().filter_map(|n| nodes.get(n)), bounds);
        let ways = &self.ways;
        self.relations
            .retain(|r| r.2.iter().any(|w| ways.get(w).is_some_and(|w| near(&w.0))));
        let members: HashSet<i64> = self
            .relations
            .iter()
            .flat_map(|r| r.2.iter().copied())
            .collect();
        self.ways
            .retain(|id, w| members.contains(id) || (w.1.is_some() && near(&w.0)));
        let used: HashSet<i64> = self
            .ways
            .values()
            .flat_map(|w| w.0.iter().copied())
            .chain(self.points.iter().map(|p| p.0))
            .collect();
        self.nodes.retain(|id, _| used.contains(id));
    }
}

/// What [`add_places`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaceStats {
    pub nodes: usize,
    pub areas: usize,
    /// Areas left out because a node of the same place lies in them.
    pub areas_with_node: usize,
    /// Places off the package's tiles.
    pub outside: usize,
    pub summary: String,
}

/// The area-weighted centroid of rings (package metres), the vertices'
/// mean for degenerate ones.
fn centroid(rings: &[Vec<[f64; 2]>]) -> Option<[f64; 2]> {
    let (mut a, mut cx, mut cz) = (0.0, 0.0, 0.0);
    let (mut n, mut sx, mut sz) = (0usize, 0.0, 0.0);
    for ring in rings {
        for i in 0..ring.len() {
            let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
            let cross = p[0] * q[1] - q[0] * p[1];
            a += cross;
            cx += (p[0] + q[0]) * cross;
            cz += (p[1] + q[1]) * cross;
            n += 1;
            sx += p[0];
            sz += p[1];
        }
    }
    if n == 0 {
        return None;
    }
    if a.abs() < 1e-6 {
        return Some([sx / n as f64, sz / n as f64]);
    }
    Some([cx / (3.0 * a), cz / (3.0 * a)])
}

fn make_place(
    tags: &Tags,
    pos: [f64; 2],
    area: bool,
    element: OsmElement,
    id: i64,
) -> Option<Place> {
    let kind = PlaceKind::from_osm(tags.get("place")?)?;
    let name = tags.get("name")?.trim().to_string();
    let population = tags.get("population").and_then(|v| parse_population(v));
    let capital = capital_level(
        tags.get("capital").map(String::as_str),
        tags.get("admin_level").map(String::as_str),
    );
    let names = NAME_LANGUAGES
        .iter()
        .filter_map(|l| {
            let v = tags.get(&format!("name:{l}"))?.trim();
            (!v.is_empty() && v != name).then(|| ((*l).to_string(), v.to_string()))
        })
        .collect();
    Some(Place {
        kind,
        rank: place_rank(kind, capital, population),
        pos: [pos[0] as f32, pos[1] as f32],
        name,
        names,
        population,
        capital,
        area,
        element,
        osm_id: id,
    })
}

/// An area place: its tags, its rings (package metres), element and id.
type AreaPlace<'a> = (&'a Tags, Vec<Vec<[f64; 2]>>, OsmElement, i64);

/// The places of `data` on the package's tiles, most important first.
pub fn build_places(
    data: &PlaceData,
    pkg: &WorldPackage,
    proj: &LocalProjection,
) -> (Places, PlaceStats) {
    let size = f64::from(pkg.tile_size_m);
    let on_tiles = |p: [f64; 2]| pkg.tiles.contains_key(&TileKey::containing(p, size));
    let project = |id: &i64| data.nodes.get(id).map(|&(lat, lon)| proj.project(lat, lon));
    let mut st = PlaceStats::default();
    let mut places = Vec::new();
    let mut node_places: Vec<(&str, [f64; 2])> = Vec::new();
    for (id, tags) in &data.points {
        let Some(p) = project(id) else { continue };
        let Some(place) = make_place(tags, p, false, OsmElement::Node, *id) else {
            continue;
        };
        node_places.push((tags.get("name").map_or("", String::as_str), p));
        if on_tiles(p) {
            st.nodes += 1;
            places.push(place);
        } else {
            st.outside += 1;
        }
    }
    // Areas: closed place ways, then place relations.
    let ring_of = |nodes: &[i64]| -> Option<Vec<[f64; 2]>> {
        let open = if nodes.first() == nodes.last() {
            &nodes[..nodes.len().saturating_sub(1)]
        } else {
            nodes
        };
        open.iter().map(project).collect()
    };
    let mut areas: Vec<AreaPlace> = Vec::new();
    let mut way_ids: Vec<&i64> = data.ways.keys().collect();
    way_ids.sort();
    for id in way_ids {
        let (nodes, tags) = &data.ways[id];
        let Some(tags) = tags else { continue };
        if nodes.len() < 4 || nodes.first() != nodes.last() {
            continue;
        }
        if let Some(r) = ring_of(nodes) {
            areas.push((tags, vec![r], OsmElement::Way, *id));
        }
    }
    for (id, tags, outer) in &data.relations {
        let parts: Vec<&[i64]> = outer
            .iter()
            .filter_map(|w| data.ways.get(w))
            .map(|w| w.0.as_slice())
            .collect();
        let rings: Vec<Vec<[f64; 2]>> = assemble_rings(&parts)
            .iter()
            .filter_map(|r| ring_of(r))
            .collect();
        if !rings.is_empty() {
            areas.push((tags, rings, OsmElement::Relation, *id));
        }
    }
    for (tags, rings, element, id) in areas {
        let name = tags.get("name").map_or("", String::as_str);
        if node_places
            .iter()
            .any(|(n, p)| *n == name && inside_rings(*p, &rings))
        {
            st.areas_with_node += 1;
            continue;
        }
        let Some(c) = centroid(&rings) else { continue };
        // A concave area's centroid may lie outside it: then its first
        // vertex's ring mean is no better; keep the centroid (labels only).
        let Some(place) = make_place(tags, c, true, element, id) else {
            continue;
        };
        if on_tiles(c) {
            st.areas += 1;
            places.push(place);
        } else {
            st.outside += 1;
        }
    }
    let places = Places::new(places);
    st.summary = format!(
        "{}; {} areas left out for their node, {} off the tiles",
        places.summary(),
        st.areas_with_node,
        st.outside
    );
    (places, st)
}

/// Writes the places into the package (section `PLCE`).
pub fn add_places(pkg: &mut WorldPackage, data: &PlaceData, proj: &LocalProjection) -> PlaceStats {
    let (places, st) = build_places(data, pkg, proj);
    pkg.places = Some(places);
    st
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::osm::parse_osm_xml_raw;

    /// Made-up places around (47.10, 9.50): a capital town, a city with a
    /// population, a village mapped as node and area, a hamlet mapped only
    /// as an area, a locality, a place without a name, a shop node, and a
    /// village far away (off the tiles).
    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="hand">
  <node id="1" lat="47.1000" lon="9.5000">
    <tag k="place" v="town"/><tag k="name" v="Adastadt"/>
    <tag k="name:en" v="Ada Town"/><tag k="name:de" v="Adastadt"/>
    <tag k="capital" v="yes"/><tag k="population" v="5,700"/>
  </node>
  <node id="2" lat="47.1200" lon="9.5300">
    <tag k="place" v="city"/><tag k="name" v="Beaburg"/><tag k="population" v="48000"/>
  </node>
  <node id="3" lat="47.0900" lon="9.4900">
    <tag k="place" v="village"/><tag k="name" v="Coradorf"/>
  </node>
  <node id="4" lat="47.1050" lon="9.5200">
    <tag k="place" v="locality"/><tag k="name" v="Dora Ried"/>
  </node>
  <node id="5" lat="47.1010" lon="9.5010"><tag k="place" v="hamlet"/></node>
  <node id="6" lat="47.1020" lon="9.5020"><tag k="shop" v="bakery"/><tag k="name" v="Elia Brot"/></node>
  <node id="7" lat="48.5000" lon="11.0000">
    <tag k="place" v="village"/><tag k="name" v="Fernweiler"/>
  </node>
  <node id="10" lat="47.0890" lon="9.4890"/>
  <node id="11" lat="47.0890" lon="9.4910"/>
  <node id="12" lat="47.0910" lon="9.4910"/>
  <node id="13" lat="47.0910" lon="9.4890"/>
  <way id="100">
    <nd ref="10"/><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="10"/>
    <tag k="place" v="village"/><tag k="name" v="Coradorf"/>
  </way>
  <node id="20" lat="47.1100" lon="9.4800"/>
  <node id="21" lat="47.1100" lon="9.4900"/>
  <node id="22" lat="47.1200" lon="9.4900"/>
  <node id="23" lat="47.1200" lon="9.4800"/>
  <way id="200"><nd ref="20"/><nd ref="21"/><nd ref="22"/></way>
  <way id="201"><nd ref="22"/><nd ref="23"/><nd ref="20"/></way>
  <relation id="300">
    <member type="way" ref="200" role="outer"/>
    <member type="way" ref="201" role="outer"/>
    <tag k="type" v="multipolygon"/><tag k="place" v="hamlet"/><tag k="name" v="Gitahof"/>
  </relation>
  <way id="400"><nd ref="10"/><nd ref="11"/><tag k="highway" v="track"/></way>
</osm>
"#;

    /// A package with 6 × 6 tiles of 1 km around the origin (no terrain).
    fn package(origin: (f64, f64)) -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for x in -3..3 {
            for z in -3..3 {
                let key = TileKey { x, z };
                tiles.insert(
                    key,
                    ocw_world::Tile {
                        key,
                        heights: vec![0.0; 4],
                        roads: Vec::new(),
                    },
                );
            }
        }
        WorldPackage {
            name: "Fixture".into(),
            origin,
            tile_size_m: 1000.0,
            samples: 2,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
        }
    }

    fn built() -> (WorldPackage, PlaceStats) {
        let raw = parse_osm_xml_raw(FIXTURE).unwrap();
        let mut data = PlaceData::from_raw(&raw);
        data.retain_near((47.0, 9.4, 47.2, 9.6));
        let mut pkg = package((47.10, 9.50));
        let proj = LocalProjection::new(47.10, 9.50);
        let st = add_places(&mut pkg, &data, &proj);
        (pkg, st)
    }

    #[test]
    fn places_come_from_nodes_and_areas_with_rank_names_and_population() {
        let (pkg, st) = built();
        let places = &pkg.places.as_ref().unwrap().places;
        let names: Vec<&str> = places.iter().map(|p| p.name.as_str()).collect();
        // Most important first: the city (larger) and the capital town
        // (rank 0 as a capital), the village, the hamlet area, the locality.
        assert_eq!(
            names,
            ["Beaburg", "Adastadt", "Coradorf", "Gitahof", "Dora Ried"]
        );
        let ada = &places[1];
        assert_eq!(
            (ada.kind, ada.rank, ada.capital),
            (PlaceKind::Town, 0, Some(2))
        );
        assert_eq!(ada.population, Some(5_700));
        assert_eq!(ada.names, vec![("en".to_string(), "Ada Town".to_string())]);
        assert_eq!(ada.pos, [0.0, 0.0]);
        assert_eq!(
            (ada.element, ada.osm_id, ada.area),
            (OsmElement::Node, 1, false)
        );
        let bea = &places[0];
        assert_eq!(
            (bea.kind, bea.rank, bea.population),
            (PlaceKind::City, 0, Some(48_000))
        );
        // East and north of the origin: +x, -z.
        assert!(bea.pos[0] > 2000.0 && bea.pos[1] < -2000.0, "{:?}", bea.pos);
        // Coradorf is a node and an area: the node only.
        let cora: Vec<_> = places.iter().filter(|p| p.name == "Coradorf").collect();
        assert_eq!(cora.len(), 1);
        assert!(!cora[0].area);
        // Gitahof only as a multipolygon of two ways: at its centre.
        let gita = places.iter().find(|p| p.name == "Gitahof").unwrap();
        assert!(gita.area && gita.element == OsmElement::Relation);
        let c = LocalProjection::new(47.10, 9.50).project(47.115, 9.485);
        assert!(
            (f64::from(gita.pos[0]) - c[0]).abs() < 1.0
                && (f64::from(gita.pos[1]) - c[1]).abs() < 1.0,
            "{:?} vs {c:?}",
            gita.pos
        );
        assert_eq!(
            places.iter().find(|p| p.name == "Dora Ried").unwrap().rank,
            5
        );
        assert_eq!((st.nodes, st.areas, st.areas_with_node), (4, 1, 1));
        // The nameless hamlet, the bakery and the far village are not there.
        assert!(!names.contains(&"Elia Brot") && !names.contains(&"Fernweiler"));
    }

    #[test]
    fn places_off_the_tiles_are_left_out_and_the_build_is_deterministic() {
        let raw = parse_osm_xml_raw(FIXTURE).unwrap();
        let data = PlaceData::from_raw(&raw);
        let proj = LocalProjection::new(47.10, 9.50);
        let mut pkg = package((47.10, 9.50));
        // Only the tile with the origin.
        pkg.tiles
            .retain(|k, _| *k == TileKey { x: 0, z: 0 } || *k == TileKey { x: -1, z: -1 });
        let (p, st) = build_places(&data, &pkg, &proj);
        let names: Vec<&str> = p.places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Adastadt"]);
        assert_eq!(st.outside, 5);
        let (again, _) = build_places(&data, &pkg, &proj);
        assert_eq!(again, p);
    }

    #[test]
    fn the_section_survives_the_package_round_trip() {
        let (pkg, _) = built();
        let back = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
        assert_eq!(back.places, pkg.places);
    }

    #[test]
    fn the_raw_reader_keeps_relations_and_rejects_broken_files() {
        let raw = parse_osm_xml_raw(FIXTURE).unwrap();
        assert_eq!(raw.relations.len(), 1);
        assert_eq!(raw.relations[0].members.len(), 2);
        assert_eq!(raw.ways.len(), 4);
        assert_eq!(raw.node_tags[&6]["shop"], "bakery");
        for bad in [
            "<osm><node id=\"1\" lat=\"1\" lon=\"2\">",
            "<osm><way id=\"1\"><node id=\"2\" lat=\"1\" lon=\"1\"/></way></osm>",
            "<osm></way></osm>",
        ] {
            assert!(parse_osm_xml_raw(bad).is_err(), "{bad}");
        }
    }
}
