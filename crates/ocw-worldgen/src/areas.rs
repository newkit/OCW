//! OSM areas for land use and buildings (spec §4.1, §4.6): closed ways and
//! multipolygon relations (outer and inner rings), plus open waterway lines.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::osm::{OsmError, Tags, Way};

/// Keys whose ways or relations are read as areas or water lines.
const AREA_KEYS: [&str; 6] = [
    "landuse", "natural", "leisure", "building", "waterway", "water",
];

fn wanted(tags: &Tags) -> bool {
    AREA_KEYS.iter().any(|k| tags.contains_key(*k))
}

/// Role of a multipolygon member way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Outer,
    Inner,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub id: i64,
    pub tags: Tags,
    pub members: Vec<(i64, Role)>,
}

/// An area in lat/lon: rings closed implicitly (first point not repeated).
#[derive(Debug, Clone, PartialEq)]
pub struct OsmArea<'a> {
    /// OSM way id (positive) or relation id (negative).
    pub id: i64,
    pub tags: &'a Tags,
    pub outer: Vec<Vec<(f64, f64)>>,
    pub inner: Vec<Vec<(f64, f64)>>,
}

/// Tagged areas, multipolygons and waterways of an extract.
#[derive(Debug, Clone, Default)]
pub struct AreaData {
    pub nodes: HashMap<i64, (f64, f64)>,
    /// Tagged ways and untagged multipolygon members, by id.
    pub ways: HashMap<i64, Way>,
    /// Multipolygon relations with a wanted tag.
    pub relations: Vec<Relation>,
}

impl AreaData {
    /// Reads the areas from a PBF in three passes: relations (to know
    /// their member ways), ways, nodes.
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        let mut relations = Vec::new();
        osmpbf::ElementReader::from_path(path)?.for_each(|el| {
            if let osmpbf::Element::Relation(r) = el {
                let tags: Tags = r
                    .tags()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                if tags.get("type").map(String::as_str) != Some("multipolygon") || !wanted(&tags) {
                    return;
                }
                let members = r
                    .members()
                    .filter(|m| m.member_type == osmpbf::RelMemberType::Way)
                    .map(|m| {
                        let role = if m.role().unwrap_or("") == "inner" {
                            Role::Inner
                        } else {
                            Role::Outer
                        };
                        (m.member_id, role)
                    })
                    .collect();
                relations.push(Relation {
                    id: r.id(),
                    tags,
                    members,
                });
            }
        })?;
        let members: HashSet<i64> = relations
            .iter()
            .flat_map(|r| r.members.iter().map(|m| m.0))
            .collect();
        let mut ways = HashMap::new();
        osmpbf::ElementReader::from_path(path)?.for_each(|el| {
            if let osmpbf::Element::Way(w) = el {
                let tags: Tags = w
                    .tags()
                    .filter(|(k, _)| {
                        AREA_KEYS.contains(k)
                            || k.starts_with("building")
                            || k.starts_with("roof")
                            || *k == "height"
                            || *k == "width"
                            || *k == "area"
                    })
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                if wanted(&tags) || members.contains(&w.id()) {
                    ways.insert(
                        w.id(),
                        Way {
                            id: w.id(),
                            nodes: w.refs().collect(),
                            tags,
                        },
                    );
                }
            }
        })?;
        let needed: HashSet<i64> = ways
            .values()
            .flat_map(|w| w.nodes.iter().copied())
            .collect();
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
        Ok(Self {
            nodes,
            ways,
            relations,
        })
    }

    fn coords(&self, ring: &[i64]) -> Option<Vec<(f64, f64)>> {
        ring.iter().map(|n| self.nodes.get(n).copied()).collect()
    }

    /// All areas: closed tagged ways and multipolygon relations. Rings with
    /// missing nodes (cut by the extract) are left out.
    pub fn areas(&self) -> Vec<OsmArea<'_>> {
        let mut out = Vec::new();
        let mut ids: Vec<&i64> = self.ways.keys().collect();
        ids.sort();
        for id in ids {
            let w = &self.ways[id];
            if !wanted(&w.tags) || w.nodes.len() < 4 || w.nodes.first() != w.nodes.last() {
                continue;
            }
            if let Some(ring) = self.coords(&w.nodes[..w.nodes.len() - 1]) {
                out.push(OsmArea {
                    id: w.id,
                    tags: &w.tags,
                    outer: vec![ring],
                    inner: vec![],
                });
            }
        }
        for r in &self.relations {
            let rings = |role: Role| -> Vec<Vec<(f64, f64)>> {
                let parts: Vec<&[i64]> = r
                    .members
                    .iter()
                    .filter(|m| m.1 == role)
                    .filter_map(|m| self.ways.get(&m.0))
                    .map(|w| w.nodes.as_slice())
                    .collect();
                assemble_rings(&parts)
                    .iter()
                    .filter_map(|ring| self.coords(ring))
                    .collect()
            };
            let outer = rings(Role::Outer);
            if outer.is_empty() {
                continue;
            }
            out.push(OsmArea {
                id: -r.id,
                tags: &r.tags,
                outer,
                inner: rings(Role::Inner),
            });
        }
        out
    }

    /// Open ways with a `waterway` tag (rivers, streams, canals).
    pub fn waterways(&self) -> Vec<(&Way, Vec<(f64, f64)>)> {
        let mut out: Vec<_> = self
            .ways
            .values()
            .filter(|w| w.tags.contains_key("waterway") && w.nodes.first() != w.nodes.last())
            .filter_map(|w| Some((w, self.coords(&w.nodes)?)))
            .collect();
        out.sort_by_key(|(w, _)| w.id);
        out
    }
}

/// Joins way node sequences into closed rings (multipolygon members are
/// often split into several ways; they may run in either direction).
/// Returns rings without the repeated closing node; pieces that do not
/// close are dropped.
pub fn assemble_rings(parts: &[&[i64]]) -> Vec<Vec<i64>> {
    let mut open: Vec<Vec<i64>> = Vec::new();
    let mut rings = Vec::new();
    for p in parts.iter().filter(|p| p.len() >= 2) {
        if p.first() == p.last() {
            if p.len() >= 4 {
                rings.push(p[..p.len() - 1].to_vec());
            }
        } else {
            open.push(p.to_vec());
        }
    }
    while let Some(mut cur) = open.pop() {
        loop {
            if cur.first() == cur.last() {
                if cur.len() >= 4 {
                    cur.pop();
                    rings.push(cur);
                }
                break;
            }
            let end = *cur.last().expect("non-empty");
            let Some(j) = open
                .iter()
                .position(|p| p.first() == Some(&end) || p.last() == Some(&end))
            else {
                break; // does not close
            };
            let mut next = open.swap_remove(j);
            if next.first() != Some(&end) {
                next.reverse();
            }
            cur.extend_from_slice(&next[1..]);
        }
    }
    rings
}

/// Even-odd point-in-polygon over any number of rings (holes included).
pub fn inside_rings(p: [f64; 2], rings: &[Vec<[f64; 2]>]) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if p[0] < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_are_assembled_from_split_ways_in_any_direction() {
        // Square 1-2-3-4 split into three ways, the middle one reversed,
        // plus a closed triangle and a piece that never closes.
        let a = [1, 2];
        let b = [4, 3, 2];
        let c = [4, 1];
        let tri = [10, 11, 12, 10];
        let dangling = [20, 21];
        let rings = assemble_rings(&[&a, &b, &c, &tri, &dangling]);
        assert_eq!(rings.len(), 2);
        assert!(rings.contains(&vec![10, 11, 12]));
        let sq = rings.iter().find(|r| r.len() == 4).unwrap();
        let mut sorted = sq.clone();
        sorted.sort();
        assert_eq!(sorted, vec![1, 2, 3, 4]);
        // Consecutive nodes of the ring are neighbours on the square.
        for i in 0..4 {
            let (p, q) = (sq[i], sq[(i + 1) % 4]);
            assert!(matches!(
                (p, q),
                (1, 2) | (2, 1) | (2, 3) | (3, 2) | (3, 4) | (4, 3) | (4, 1) | (1, 4)
            ));
        }
    }

    #[test]
    fn point_in_rings_respects_holes() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]];
        let rings = vec![outer, hole];
        assert!(inside_rings([1.0, 1.0], &rings));
        assert!(!inside_rings([5.0, 5.0], &rings));
        assert!(!inside_rings([11.0, 5.0], &rings));
    }

    #[test]
    fn areas_come_from_closed_ways_and_multipolygons() {
        let mut d = AreaData::default();
        for (id, lat, lon) in [(1, 0.0, 0.0), (2, 0.0, 1.0), (3, 1.0, 1.0), (4, 1.0, 0.0)] {
            d.nodes.insert(id, (lat, lon));
        }
        let tags = |k: &str, v: &str| Tags::from([(k.to_string(), v.to_string())]);
        d.ways.insert(
            1,
            Way {
                id: 1,
                nodes: vec![1, 2, 3, 4, 1],
                tags: tags("landuse", "forest"),
            },
        );
        // Open untagged members of a relation.
        d.ways.insert(
            2,
            Way {
                id: 2,
                nodes: vec![1, 2, 3],
                tags: Tags::new(),
            },
        );
        d.ways.insert(
            3,
            Way {
                id: 3,
                nodes: vec![3, 4, 1],
                tags: Tags::new(),
            },
        );
        d.ways.insert(
            4,
            Way {
                id: 4,
                nodes: vec![1, 3],
                tags: tags("waterway", "stream"),
            },
        );
        // A ring with a node outside the extract is dropped.
        d.ways.insert(
            5,
            Way {
                id: 5,
                nodes: vec![1, 2, 99, 1],
                tags: tags("natural", "water"),
            },
        );
        d.relations.push(Relation {
            id: 7,
            tags: tags("natural", "water"),
            members: vec![(2, Role::Outer), (3, Role::Outer), (42, Role::Inner)],
        });
        let areas = d.areas();
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].id, 1);
        assert_eq!(areas[0].outer[0].len(), 4);
        assert_eq!(areas[1].id, -7);
        assert_eq!(areas[1].outer.len(), 1);
        assert!(areas[1].inner.is_empty());
        let ww = d.waterways();
        assert_eq!(ww.len(), 1);
        assert_eq!(ww[0].1, vec![(0.0, 0.0), (1.0, 1.0)]);
    }
}
