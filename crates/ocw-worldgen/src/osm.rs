//! OSM data in memory, PBF reading (spec §4.1) and a minimal OSM XML
//! reader for small hand-made test fixtures.

use std::collections::HashMap;
use std::path::Path;

/// Tags of a way, as key → value.
pub type Tags = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct Way {
    pub id: i64,
    pub nodes: Vec<i64>,
    pub tags: Tags,
}

impl Way {
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }
}

/// The parts of an OSM extract the road graph needs.
#[derive(Debug, Clone, Default)]
pub struct OsmData {
    /// Node id → (lat, lon) in degrees.
    pub nodes: HashMap<i64, (f64, f64)>,
    pub ways: Vec<Way>,
}

/// The box of the points overlaps (south, west, north, east).
pub fn way_box_overlaps<'a>(
    pts: impl Iterator<Item = &'a (f64, f64)>,
    (s, w, n, e): (f64, f64, f64, f64),
) -> bool {
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    let mut any = false;
    for &(lat, lon) in pts {
        any = true;
        lo = (lo.0.min(lat), lo.1.min(lon));
        hi = (hi.0.max(lat), hi.1.max(lon));
    }
    any && lo.0 <= n && hi.0 >= s && lo.1 <= e && hi.1 >= w
}

#[derive(Debug, thiserror::Error)]
pub enum OsmError {
    #[error("{0}")]
    Pbf(#[from] osmpbf::Error),
}

impl OsmData {
    /// Reads all ways with a `highway` tag and the nodes they reference.
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        // Two passes: ways first (to know which nodes are needed), then nodes.
        let mut ways = Vec::new();
        osmpbf::ElementReader::from_path(path)?.for_each(|el| {
            if let osmpbf::Element::Way(w) = el
                && w.tags().any(|(k, _)| k == "highway")
            {
                ways.push(Way {
                    id: w.id(),
                    nodes: w.refs().collect(),
                    tags: w
                        .tags()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                });
            }
        })?;
        let needed: std::collections::HashSet<i64> =
            ways.iter().flat_map(|w| w.nodes.iter().copied()).collect();
        let mut nodes = HashMap::with_capacity(needed.len());
        osmpbf::ElementReader::from_path(path)?.for_each(|el| match el {
            osmpbf::Element::Node(n) if needed.contains(&n.id()) => {
                nodes.insert(n.id(), (n.lat(), n.lon()));
            }
            osmpbf::Element::DenseNode(n) if needed.contains(&n.id()) => {
                nodes.insert(n.id(), (n.lat(), n.lon()));
            }
            _ => {}
        })?;
        Ok(Self { nodes, ways })
    }

    /// Reads several extracts and merges them (ways by id, nodes by id):
    /// neighbouring Geofabrik extracts of one day overlap at their border
    /// and hold the same objects there.
    pub fn read_pbfs(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        let mut out = Self::default();
        for p in paths {
            let d = Self::read_pbf(p).map_err(|e| format!("{}: {e}", p.display()))?;
            out.merge(d);
        }
        Ok(out)
    }

    /// Adds another extract's ways and nodes (ways already here are kept).
    pub fn merge(&mut self, other: Self) {
        let have: std::collections::HashSet<i64> = self.ways.iter().map(|w| w.id).collect();
        self.ways
            .extend(other.ways.into_iter().filter(|w| !have.contains(&w.id)));
        self.nodes.extend(other.nodes);
    }

    /// Keeps only the ways whose node box overlaps (south, west, north,
    /// east) and the nodes they use: a big extract cut down to the part
    /// being built (memory, spec §15).
    pub fn retain_near(&mut self, bounds: (f64, f64, f64, f64)) {
        let nodes = &self.nodes;
        self.ways
            .retain(|w| way_box_overlaps(w.nodes.iter().filter_map(|n| nodes.get(n)), bounds));
        let used: std::collections::HashSet<i64> = self
            .ways
            .iter()
            .flat_map(|w| w.nodes.iter().copied())
            .collect();
        self.nodes.retain(|id, _| used.contains(id));
    }

    /// Reads OSM XML (`.osm`, as written by JOSM or the OSM API): nodes
    /// with `lat`/`lon`, ways with `nd ref` and `tag k v`. Like
    /// [`OsmData::read_pbf`] it keeps only ways with a `highway` tag and
    /// the nodes they use. Meant for small checked-in fixtures; relations,
    /// comments and processing instructions are skipped.
    pub fn parse_osm_xml(text: &str) -> Result<Self, String> {
        let mut nodes = HashMap::new();
        let mut ways = Vec::new();
        let mut way: Option<Way> = None;
        let mut rest = text;
        while let Some(start) = rest.find('<') {
            rest = &rest[start + 1..];
            if let Some(after) = rest.strip_prefix("!--") {
                let end = after.find("-->").ok_or("unterminated comment")?;
                rest = &after[end + 3..];
                continue;
            }
            let end = rest.find('>').ok_or("unterminated tag")?;
            let tag = &rest[..end];
            rest = &rest[end + 1..];
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                if name.trim() == "way" {
                    let w = way.take().ok_or("</way> without <way>")?;
                    if w.tags.contains_key("highway") {
                        ways.push(w);
                    }
                }
                continue;
            }
            let self_closing = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let (name, attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
            let attrs = xml_attributes(attrs)?;
            let get = |k: &str| {
                attrs
                    .iter()
                    .find(|(a, _)| a == k)
                    .map(|(_, v)| v.as_str())
                    .ok_or_else(|| format!("<{name}> without {k}"))
            };
            let num = |k: &str| -> Result<f64, String> {
                get(k)?
                    .parse::<f64>()
                    .map_err(|e| format!("<{name} {k}>: {e}"))
            };
            let int = |k: &str| -> Result<i64, String> {
                get(k)?
                    .parse::<i64>()
                    .map_err(|e| format!("<{name} {k}>: {e}"))
            };
            match name {
                "node" => {
                    nodes.insert(int("id")?, (num("lat")?, num("lon")?));
                }
                "way" => {
                    if way.is_some() {
                        return Err("nested <way>".into());
                    }
                    let w = Way {
                        id: int("id")?,
                        nodes: Vec::new(),
                        tags: Tags::new(),
                    };
                    // A self-closing way has no tags, so it is no highway.
                    if !self_closing {
                        way = Some(w);
                    }
                }
                "nd" => {
                    if let Some(w) = way.as_mut() {
                        w.nodes.push(int("ref")?);
                    }
                }
                "tag" => {
                    if let Some(w) = way.as_mut() {
                        w.tags.insert(get("k")?.to_string(), get("v")?.to_string());
                    }
                }
                _ => {}
            }
        }
        if way.is_some() {
            return Err("unterminated <way>".into());
        }
        let needed: std::collections::HashSet<i64> =
            ways.iter().flat_map(|w| w.nodes.iter().copied()).collect();
        nodes.retain(|id, _| needed.contains(id));
        Ok(Self { nodes, ways })
    }
}

/// `k="v" k2='v2'` → pairs, with the five predefined XML entities and
/// numeric character references decoded.
fn xml_attributes(s: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("bad attributes: {s}"))?;
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let quote = after
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| format!("unquoted attribute {key}"))?;
        let body = &after[1..];
        let end = body
            .find(quote)
            .ok_or_else(|| format!("unterminated attribute {key}"))?;
        out.push((key, xml_unescape(&body[..end])?));
        rest = body[end + 1..].trim_start();
    }
    Ok(out)
}

fn xml_unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or("unterminated entity")? + i;
        let entity = &rest[i + 1..end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| format!("bad entity &{e};"))?,
            e if e.starts_with('#') => e[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| format!("bad entity &{e};"))?,
            e => return Err(format!("unknown entity &{e};")),
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(id: i64, nodes: &[i64]) -> Way {
        Way {
            id,
            nodes: nodes.to_vec(),
            tags: [("highway".to_string(), "track".to_string())].into(),
        }
    }

    #[test]
    fn extracts_merge_and_shrink_to_the_part_being_built() {
        let mut a = OsmData {
            nodes: [(1, (48.0, 8.0)), (2, (48.1, 8.0)), (3, (49.0, 9.0))].into(),
            ways: vec![way(10, &[1, 2]), way(11, &[2, 3])],
        };
        // The neighbouring extract repeats way 11 at the border and adds 12.
        let b = OsmData {
            nodes: [(2, (48.1, 8.0)), (3, (49.0, 9.0)), (4, (49.1, 9.1))].into(),
            ways: vec![way(11, &[2, 3]), way(12, &[3, 4])],
        };
        a.merge(b);
        assert_eq!(a.ways.len(), 3);
        assert_eq!(a.nodes.len(), 4);
        // Box around 48 N 8 E: way 12 goes, way 11 (reaching in) stays.
        a.retain_near((47.9, 7.9, 48.2, 8.2));
        let ids: Vec<i64> = a.ways.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![10, 11]);
        assert!(!a.nodes.contains_key(&4) && a.nodes.contains_key(&3));
        // A way crossing the box without a node inside it is kept too.
        let mut c = OsmData {
            nodes: [(1, (47.0, 8.0)), (2, (49.0, 8.0))].into(),
            ways: vec![way(1, &[1, 2])],
        };
        c.retain_near((47.9, 7.9, 48.2, 8.2));
        assert_eq!(c.ways.len(), 1);
        assert!(OsmData::read_pbfs(&["no/such.pbf".into()]).is_err());
    }

    #[test]
    fn osm_xml_fixture_reading() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="hand">
  <!-- a comment with <way> inside -->
  <node id="1" lat="47.1" lon="9.5"/>
  <node id="2" lat='47.2' lon="9.6" version="1">
    <tag k="highway" v="crossing"/>
  </node>
  <node id="3" lat="47.3" lon="9.7"/>
  <node id="-4" lat="47.4" lon="9.8"/>
  <way id="10">
    <nd ref="1"/><nd ref="2"/>
    <tag k="highway" v="residential"/>
    <tag k="name" v="Haupt &amp; Neben &quot;Weg&quot; &#246;&#x00e4;"/>
  </way>
  <way id="11">
    <nd ref="3"/><nd ref="1"/>
    <tag k="building" v="yes"/>
  </way>
  <way id="-12">
    <nd ref="-4"/><nd ref="2"/>
    <tag k="highway" v="track"/>
  </way>
  <relation id="5"><member type="way" ref="10" role=""/></relation>
</osm>"#;
        let osm = OsmData::parse_osm_xml(xml).unwrap();
        assert_eq!(osm.ways.len(), 2);
        assert_eq!(osm.ways[0].id, 10);
        assert_eq!(osm.ways[0].nodes, [1, 2]);
        assert_eq!(osm.ways[0].tag("name"), Some("Haupt & Neben \"Weg\" öä"));
        assert_eq!(osm.ways[1].id, -12);
        // Only nodes of highway ways are kept.
        let mut ids: Vec<i64> = osm.nodes.keys().copied().collect();
        ids.sort();
        assert_eq!(ids, [-4, 1, 2]);
        assert_eq!(osm.nodes[&2], (47.2, 9.6));
        for bad in [
            r#"<way id="1"><nd ref="1"/>"#,
            r#"<node id="1" lat="x" lon="1"/>"#,
            r#"<node id=1 lat="1" lon="1"/>"#,
            r#"<way id="1"><tag k="a" v="&bogus;"/></way>"#,
            r#"<node id="1""#,
            "</way>",
        ] {
            assert!(OsmData::parse_osm_xml(bad).is_err(), "{bad}");
        }
    }
}
