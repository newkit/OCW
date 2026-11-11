//! SVG map of a road graph, for checking a build by eye.

use std::fmt::Write;

use ocw_world::graph::{RoadClass, RoadGraph};

fn style(class: RoadClass) -> (&'static str, f64) {
    match class {
        RoadClass::Trunk | RoadClass::Primary => ("#d6453d", 3.0),
        RoadClass::Secondary => ("#e8902c", 2.6),
        RoadClass::Tertiary => ("#d8c03a", 2.2),
        RoadClass::Unclassified | RoadClass::Residential => ("#707070", 1.4),
        RoadClass::Service => ("#a8a8a8", 1.0),
        RoadClass::Track => ("#8a6a3a", 1.0),
        RoadClass::Cycleway => ("#2f7fd8", 1.4),
        RoadClass::Path => ("#3a9a4a", 0.9),
    }
}

/// Renders `graph` `width_px` wide: roads coloured by class, dead ends in
/// violet, boundary edges in cyan, with a small legend and `title`.
pub fn graph_svg(graph: &RoadGraph, width_px: f64, title: &str) -> String {
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for e in &graph.edges {
        for p in &e.points {
            min = [min[0].min(p[0]), min[1].min(p[1])];
            max = [max[0].max(p[0]), max[1].max(p[1])];
        }
    }
    if !min[0].is_finite() {
        min = [0.0, 0.0];
        max = [1.0, 1.0];
    }
    let scale = width_px / (max[0] - min[0]).max(1.0);
    let height = (max[1] - min[1]).max(1.0) * scale;
    let mut s = String::new();
    let _ = writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}"><rect width="100%" height="100%" fill="#f6f4ee"/>"##,
        w = width_px,
        h = height + 60.0
    );
    let draw = |s: &mut String, e: &ocw_world::graph::Edge, color: &str, width: f64| {
        let pts: Vec<String> = e
            .points
            .iter()
            .map(|p| {
                format!(
                    "{:.1},{:.1}",
                    (p[0] - min[0]) * scale,
                    (p[1] - min[1]) * scale + 60.0
                )
            })
            .collect();
        let _ = writeln!(
            s,
            r#"<polyline points="{}" fill="none" stroke="{color}" stroke-width="{width:.1}" stroke-linecap="round"/>"#,
            pts.join(" ")
        );
    };
    // Small roads first, big roads on top; flags on top of everything.
    let mut order: Vec<&ocw_world::graph::Edge> = graph.edges.iter().collect();
    order.sort_by_key(|e| std::cmp::Reverse(e.class));
    for e in &order {
        let (color, width) = style(e.class);
        draw(&mut s, e, color, width);
    }
    for e in &order {
        if e.boundary {
            draw(&mut s, e, "#19c3d6", 4.0);
        } else if e.dead_end_m.is_some() {
            draw(&mut s, e, "#9b3fd1", 1.6);
        }
    }
    let _ = writeln!(
        s,
        r##"<text x="10" y="22" font-family="sans-serif" font-size="16" fill="#222">{}</text>"##,
        title.replace('&', "&amp;").replace('<', "&lt;")
    );
    let legend = [
        ("#d6453d", "primary/trunk"),
        ("#e8902c", "secondary"),
        ("#d8c03a", "tertiary"),
        ("#707070", "residential"),
        ("#8a6a3a", "track"),
        ("#2f7fd8", "cycleway"),
        ("#9b3fd1", "dead end"),
        ("#19c3d6", "boundary"),
    ];
    for (i, (color, label)) in legend.iter().enumerate() {
        let x = 10.0 + i as f64 * (width_px - 20.0) / legend.len() as f64;
        let _ = writeln!(
            s,
            r##"<rect x="{x:.0}" y="34" width="14" height="6" fill="{color}"/><text x="{tx:.0}" y="41" font-family="sans-serif" font-size="11" fill="#333">{label}</text>"##,
            tx = x + 18.0
        );
    }
    s.push_str("</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, VertexId};

    #[test]
    fn one_polyline_per_edge_plus_flags() {
        let edge = |i: u64, boundary: bool, dead: bool| Edge {
            id: EdgeId(i),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 0.0], [100.0, 50.0]],
            class: RoadClass::Primary,
            name: None,
            width_m: 7.0,
            bridge: false,
            tunnel: false,
            dead_end_m: dead.then_some(100.0),
            boundary,
            osm_way: 1,
        };
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 50.0]],
            edges: vec![
                edge(1, false, false),
                edge(2, true, false),
                edge(3, false, true),
            ],
            boundary_vertices: Default::default(),
        };
        let svg = graph_svg(&g, 400.0, "Test <region>");
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<polyline").count(), 5); // 3 roads + 2 flags
        assert!(svg.contains("Test &lt;region>"));
        assert!(svg.contains(r#"width="400""#));
        // An empty graph still renders.
        assert!(graph_svg(&RoadGraph::default(), 100.0, "empty").contains("</svg>"));
    }
}
