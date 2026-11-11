//! OSM data in memory and PBF reading (spec §4.1).

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
}
