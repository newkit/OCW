//! What the World workspace shows of the open package (spec §14 Phase 16,
//! 16A, 16C): id, version, signing and trust, bounds, tiles, sections,
//! roads, buildings, routes – pure, from the loaded package.

use ocw_world::package::WorldPackage;
use ocw_worldstore::details::WorldDetails;

/// The package facts as (label, value) rows.
pub fn package_rows(
    details: &WorldDetails,
    pkg: &WorldPackage,
    sections: &[([u8; 4], u64)],
) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut row = |label: &str, value: String| rows.push((label.into(), value));
    row("Region", details.region.clone());
    row("Id", details.id.clone());
    row(
        "Version",
        if details.version == 0 {
            "0 (no manifest: a local build)".into()
        } else {
            details.version.to_string()
        },
    );
    row("Signing", trust_text(details));
    row(
        "Origin",
        format!("{:.5} N, {:.5} E", pkg.origin.0, pkg.origin.1),
    );
    if let Some(r) = &pkg.region {
        row(
            "Bounds",
            format!(
                "{:.4}-{:.4} N, {:.4}-{:.4} E",
                r.south, r.north, r.west, r.east
            ),
        );
    } else if let Some((lo, hi)) = tile_extent(pkg) {
        let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
        // z grows southwards: the north edge is the smallest z.
        let (n, w) = proj.unproject([lo[0], lo[1]]);
        let (s, e) = proj.unproject([hi[0], hi[1]]);
        row(
            "Bounds",
            format!("{s:.4}-{n:.4} N, {w:.4}-{e:.4} E (tiles)"),
        );
    }
    if let Some((lo, hi)) = tile_extent(pkg) {
        row(
            "Extent",
            format!(
                "{:.1} x {:.1} km (x {:.0}..{:.0} m, z {:.0}..{:.0} m)",
                (hi[0] - lo[0]) / 1000.0,
                (hi[1] - lo[1]) / 1000.0,
                lo[0],
                hi[0],
                lo[1],
                hi[1]
            ),
        );
    }
    row(
        "Tiles",
        format!(
            "{} of {:.0} m, {} samples a side",
            pkg.tiles.len(),
            pkg.tile_size_m,
            pkg.samples
        ),
    );
    if let Some(a) = details.area_km2 {
        row("Area", format!("{a:.1} km2"));
    }
    if let Some((lo, hi)) = details.elevation_m {
        row("Elevation", format!("{lo:.0} - {hi:.0} m"));
    }
    row("Roads", roads_text(pkg));
    let buildings: usize = pkg
        .buildings
        .iter()
        .flat_map(|b| b.values())
        .map(Vec::len)
        .sum();
    row("Buildings", buildings.to_string());
    row(
        "Routes",
        if pkg.routes.is_empty() {
            "none".into()
        } else {
            format!(
                "{}: {}",
                pkg.routes.len(),
                pkg.routes
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
    );
    row(
        "Sections",
        if sections.is_empty() {
            "none (format 1)".into()
        } else {
            sections
                .iter()
                .map(|(tag, len)| format!("{} {}", String::from_utf8_lossy(tag), size_text(*len)))
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    if let Some(tz) = &details.time_zone {
        row("Time zone", tz.clone());
    }
    if !details.parts.is_empty() {
        row("Joined", details.parts.join(", "));
    }
    if details.size_bytes > 0 {
        row("File", size_text(details.size_bytes));
    }
    let credits: Vec<String> = details
        .attributions
        .iter()
        .map(|a| format!("{} ({})", a.source, a.licence))
        .collect();
    if !credits.is_empty() {
        row("Credits", credits.join("; "));
    }
    rows
}

/// The signing line: "verified", "unofficial: signed by 'Ada Rider'
/// (unverified name)", "unsigned", "untrusted".
pub fn trust_text(d: &WorldDetails) -> String {
    match (d.trust, &d.signer) {
        ("unofficial", Some(name)) => {
            format!("unofficial: signed by '{name}' (unverified name)")
        }
        ("verified", _) => "verified (signed by a trusted root)".into(),
        ("unsigned", _) => "unsigned".into(),
        (other, _) => other.into(),
    }
}

/// The tiles' extent in package metres: (min, max) of (x, z).
pub fn tile_extent(pkg: &WorldPackage) -> Option<([f64; 2], [f64; 2])> {
    let s = f64::from(pkg.tile_size_m);
    let keys = pkg.tiles.keys();
    let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
    for k in keys {
        lo = [lo[0].min(k.x), lo[1].min(k.z)];
        hi = [hi[0].max(k.x), hi[1].max(k.z)];
    }
    (lo[0] <= hi[0]).then(|| {
        (
            [f64::from(lo[0]) * s, f64::from(lo[1]) * s],
            [f64::from(hi[0] + 1) * s, f64::from(hi[1] + 1) * s],
        )
    })
}

/// "12 345 edges, 1 234.5 km, 9 876 junctions" from the road graph, or
/// the strips of a format 1 package.
pub fn roads_text(pkg: &WorldPackage) -> String {
    match &pkg.graph {
        Some(g) => {
            let km: f64 = g
                .edges
                .iter()
                .map(ocw_world::graph::Edge::length_m)
                .sum::<f64>()
                / 1000.0;
            format!(
                "{} edges, {km:.1} km, {} vertices (road graph)",
                g.edges.len(),
                g.vertices.len()
            )
        }
        None => {
            let strips: usize = pkg.tiles.values().map(|t| t.roads.len()).sum();
            format!("{strips} strips (no road graph)")
        }
    }
}

/// "1.2 MB", "340 kB", "12 B".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} kB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details() -> WorldDetails {
        WorldDetails {
            id: "local/testland".into(),
            version: 0,
            region: "Testland".into(),
            trust: "unsigned",
            area_km2: Some(0.02),
            elevation_m: Some((400.0, 400.0)),
            ..Default::default()
        }
    }

    fn value<'a>(rows: &'a [(String, String)], label: &str) -> &'a str {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("no row {label}: {rows:?}"))
    }

    #[test]
    fn the_rows_tell_id_version_trust_bounds_tiles_and_sections() {
        let pkg = crate::map::tests::package();
        let rows = package_rows(&details(), &pkg, &[(*b"LUSE", 1234), (*b"BLDG", 56)]);
        assert_eq!(value(&rows, "Id"), "local/testland");
        assert!(value(&rows, "Version").starts_with("0 (no manifest"));
        assert_eq!(value(&rows, "Signing"), "unsigned");
        assert!(value(&rows, "Bounds").ends_with("(tiles)"));
        assert!(
            value(&rows, "Extent").starts_with("0.2 x 0.1 km"),
            "{rows:?}"
        );
        assert!(value(&rows, "Tiles").starts_with("2 of 100 m"));
        assert_eq!(value(&rows, "Roads"), "2 strips (no road graph)");
        assert_eq!(value(&rows, "Buildings"), "1");
        assert_eq!(value(&rows, "Routes"), "none");
        assert_eq!(value(&rows, "Sections"), "LUSE 1 kB, BLDG 56 B");
        assert_eq!(value(&rows, "Elevation"), "400 - 400 m");
    }

    #[test]
    fn signing_names_the_claimed_signer() {
        let mut d = details();
        d.trust = "unofficial";
        d.signer = Some("Ada Rider".into());
        assert_eq!(
            trust_text(&d),
            "unofficial: signed by 'Ada Rider' (unverified name)"
        );
        d.trust = "verified";
        assert!(trust_text(&d).starts_with("verified"));
        d.trust = "untrusted";
        assert_eq!(trust_text(&d), "untrusted");
        assert_eq!(size_text(2_500_000), "2.5 MB");
    }

    #[test]
    fn a_road_graph_is_counted_in_edges_and_kilometres() {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let mut pkg = crate::map::tests::package();
        pkg.graph = Some(
            RoadGraph {
                vertices: vec![[0.0, 0.0], [3000.0, 4000.0]],
                edges: vec![Edge {
                    id: EdgeId(1),
                    from: VertexId(0),
                    to: VertexId(1),
                    points: vec![[0.0, 0.0], [3000.0, 4000.0]],
                    class: RoadClass::Primary,
                    name: None,
                    width_m: 7.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: 1,
                    profile: Vec::new(),
                }],
                boundary_vertices: Default::default(),
            }
            .into(),
        );
        assert_eq!(roads_text(&pkg), "1 edges, 5.0 km, 2 vertices (road graph)");
    }
}
