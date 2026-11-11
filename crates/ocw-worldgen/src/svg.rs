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

/// How roads are coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colouring {
    /// By road class, with dead ends and boundary edges highlighted.
    Class,
    /// By the steepest 25 m gradient of the elevation profile.
    Grade,
}

/// Colour for a gradient: green (flat) → yellow (6 %) → red (12 %) → dark red.
pub fn grade_colour(grade: f64) -> String {
    let g = grade.abs();
    let (r, gr, b) = if g < 0.06 {
        let t = g / 0.06;
        (60.0 + 180.0 * t, 160.0 + 40.0 * t, 60.0)
    } else if g < 0.12 {
        let t = (g - 0.06) / 0.06;
        (240.0, 200.0 - 150.0 * t, 60.0 - 30.0 * t)
    } else {
        let t = ((g - 0.12) / 0.08).min(1.0);
        (240.0 - 110.0 * t, 50.0 - 40.0 * t, 30.0)
    };
    format!("#{:02x}{:02x}{:02x}", r as u8, gr as u8, b as u8)
}

/// Renders `graph` `width_px` wide: roads coloured by class, dead ends in
/// violet, boundary edges in cyan, with a small legend and `title`.
pub fn graph_svg(graph: &RoadGraph, width_px: f64, title: &str) -> String {
    graph_svg_with(graph, width_px, title, Colouring::Class)
}

/// [`graph_svg`] with a choice of colouring.
pub fn graph_svg_with(
    graph: &RoadGraph,
    width_px: f64,
    title: &str,
    colouring: Colouring,
) -> String {
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
        match colouring {
            Colouring::Class => draw(&mut s, e, color, width),
            Colouring::Grade => {
                let grade = crate::profile::steepest(&e.profile, 5);
                draw(&mut s, e, &grade_colour(grade), width.max(1.4));
            }
        }
    }
    if colouring == Colouring::Class {
        for e in &order {
            if e.boundary {
                draw(&mut s, e, "#19c3d6", 4.0);
            } else if e.dead_end_m.is_some() {
                draw(&mut s, e, "#9b3fd1", 1.6);
            }
        }
    }
    let _ = writeln!(
        s,
        r##"<text x="10" y="22" font-family="sans-serif" font-size="16" fill="#222">{}</text>"##,
        title.replace('&', "&amp;").replace('<', "&lt;")
    );
    let grade_legend: Vec<(String, &str)> = [
        (0.0, "flat"),
        (0.03, "3 %"),
        (0.06, "6 %"),
        (0.09, "9 %"),
        (0.12, "12 %"),
        (0.16, "16 %"),
        (0.20, "20 %+"),
    ]
    .iter()
    .map(|(g, l)| (grade_colour(*g), *l))
    .collect();
    let class_legend: Vec<(String, &str)> = [
        ("#d6453d", "primary/trunk"),
        ("#e8902c", "secondary"),
        ("#d8c03a", "tertiary"),
        ("#707070", "residential"),
        ("#8a6a3a", "track"),
        ("#2f7fd8", "cycleway"),
        ("#9b3fd1", "dead end"),
        ("#19c3d6", "boundary"),
    ]
    .iter()
    .map(|(c, l)| (c.to_string(), *l))
    .collect();
    let legend = if colouring == Colouring::Grade {
        grade_legend
    } else {
        class_legend
    };
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
    fn grade_colours_go_from_green_to_red() {
        assert_eq!(grade_colour(0.0), "#3ca03c");
        assert_eq!(grade_colour(-0.06), grade_colour(0.06));
        let red = |c: &str| u8::from_str_radix(&c[1..3], 16).unwrap();
        let green = |c: &str| u8::from_str_radix(&c[3..5], 16).unwrap();
        assert!(green(&grade_colour(0.12)) < green(&grade_colour(0.06)));
        assert!(red(&grade_colour(0.06)) > red(&grade_colour(0.0)));
        assert_eq!(grade_colour(0.5), grade_colour(0.25));
    }

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
            oneway: ocw_world::graph::Oneway::No,
            boundary,
            osm_way: 1,
            profile: Vec::new(),
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
        // Grade colouring: no flag overlays.
        let svg = graph_svg_with(&g, 400.0, "grade", Colouring::Grade);
        assert_eq!(svg.matches("<polyline").count(), 3);
    }
}
