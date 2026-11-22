//! Mini-roundabouts (spec §4.8.8 J3): the OSM nodes tagged
//! `highway=mini_roundabout` matched to the road graph's junction
//! vertices, for the section `JMRB` (the renderer paints a disc on their
//! junction surface).

use std::path::{Path, PathBuf};

use ocw_world::geo::LocalProjection;
use ocw_world::graph::RoadGraph;
use ocw_world::junction::section::MiniRoundabouts;

use crate::osm::{OsmError, par_elements};

/// A node lies at most this far from the vertex it marks (m).
pub const MATCH_M: f64 = 3.0;

/// The `highway=mini_roundabout` nodes of an extract: (id, lat, lon).
pub fn read_pbf(path: &Path) -> Result<Vec<(i64, f64, f64)>, OsmError> {
    par_elements(path, |el, out| {
        let (id, lat, lon, mini) = match el {
            osmpbf::Element::Node(n) => (
                n.id(),
                n.lat(),
                n.lon(),
                n.tags().any(|t| t == ("highway", "mini_roundabout")),
            ),
            osmpbf::Element::DenseNode(n) => (
                n.id(),
                n.lat(),
                n.lon(),
                n.tags().any(|t| t == ("highway", "mini_roundabout")),
            ),
            _ => return,
        };
        if mini {
            out.push((id, lat, lon));
        }
    })
}

/// [`read_pbf`] of several extracts, each node once (by id, sorted).
pub fn read_pbfs(paths: &[PathBuf]) -> Result<Vec<(i64, f64, f64)>, String> {
    let mut out = Vec::new();
    for p in paths {
        out.extend(read_pbf(p).map_err(|e| format!("{}: {e}", p.display()))?);
    }
    out.sort_by_key(|n| n.0);
    out.dedup_by_key(|n| n.0);
    Ok(out)
}

/// The junction vertices (three or more edges) within [`MATCH_M`] of the
/// nodes (lat, lon), the nearest for each node.
pub fn match_vertices(
    g: &RoadGraph,
    proj: &LocalProjection,
    nodes: &[(i64, f64, f64)],
) -> MiniRoundabouts {
    let adj = g.adjacency();
    let cell = |p: [f64; 2]| ((p[0] / 10.0).floor() as i64, (p[1] / 10.0).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (v, p) in g.vertices.iter().enumerate() {
        if adj[v].len() >= 3 {
            grid.entry(cell(*p)).or_default().push(v);
        }
    }
    let mut vertices = Vec::new();
    for &(_, lat, lon) in nodes {
        let p = proj.project(lat, lon);
        let (cx, cz) = cell(p);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &v in grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    let q = g.vertices[v];
                    let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                    if d <= MATCH_M && best.is_none_or(|b| d < b.0) {
                        best = Some((d, v));
                    }
                }
            }
        }
        if let Some((_, v)) = best {
            vertices.push(v as u32);
        }
    }
    vertices.sort_unstable();
    vertices.dedup();
    MiniRoundabouts {
        vertex_count: g.vertices.len() as u32,
        vertices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::RoadClass;
    use ocw_world::junction::fixtures::t_junction;

    #[test]
    fn nodes_mark_the_junction_vertex_next_to_them() {
        let g = t_junction(RoadClass::Residential, 0.0, 0.0);
        let proj = LocalProjection::new(47.0, 9.0);
        // A node 1.5 m from the T junction's vertex (0, 0), one 2 m from
        // a dead end (not a junction), one 10 m out on the main road.
        let at = |x: f64, z: f64| {
            let (lat, lon) = proj.unproject([x, z]);
            (lat, lon)
        };
        let nodes: Vec<(i64, f64, f64)> = [(1, at(1.5, 0.0)), (2, at(-118.0, 0.0)), (3, at(10.0, 0.0))]
            .into_iter()
            .map(|(id, (lat, lon))| (id, lat, lon))
            .collect();
        let m = match_vertices(&g, &proj, &nodes);
        assert_eq!(m.vertices, vec![0]);
        assert!(m.fits(&g) && m.contains(0) && !m.contains(1));
    }
}
