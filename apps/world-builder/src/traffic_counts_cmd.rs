//! `world-builder traffic-counts` (spec 14 Phase 18, 18E T11): the open
//! traffic counts into `TRFC`'s per-edge `flow` of a built package. Only
//! the `TRFC` section changes (spliced into the package's bytes, every
//! other byte as it was) and the manifest (new hash, the sources'
//! attribution). See `ocw_worldgen::traffic_counts` for the matching and
//! the spread along the roads.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ocw_sim::traffic::demand::annual_day_factor;
use ocw_sim::traffic::world::{region_of, unit_aadt};
use ocw_world::geo::LocalProjection;
use ocw_world::manifest::SigningKeyPair;
use ocw_world::traffic::{TRAFFIC_TAG, write_traffic};
use ocw_worldgen::traffic_counts::{self as tc, CountSource, Issue, Params, Station, WayNames};

use crate::dist;

/// The options of the command.
pub struct Args<'a> {
    pub world: &'a Path,
    pub pbf: &'a [PathBuf],
    pub counts: &'a [PathBuf],
    pub out: Option<PathBuf>,
    pub key: Option<&'a SigningKeyPair>,
    pub tolerance_m: f64,
    pub svg: Option<PathBuf>,
    pub report: Option<PathBuf>,
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

pub fn run(a: &Args<'_>) {
    let t0 = std::time::Instant::now();
    let bytes =
        std::fs::read(a.world).unwrap_or_else(|e| fail(format!("{}: {e}", a.world.display())));
    let pkg = ocw_world::WorldPackage::skeleton_from_bytes(&bytes)
        .unwrap_or_else(|e| fail(format!("{}: {e}", a.world.display())));
    let graph = pkg
        .graph
        .clone()
        .unwrap_or_else(|| fail("the package has no road graph"));
    let mut traffic = pkg.traffic.as_deref().cloned().unwrap_or_else(|| {
        fail("the package has no TRFC section: run `world-builder traffic` first")
    });
    let names = WayNames::read_pbfs(a.pbf).unwrap_or_else(|e| fail(e));
    println!(
        "roads: {} edges, {} OSM ways with a ref or name",
        graph.edges.len(),
        names.by_way.len()
    );
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    // Inside: the box of the road graph.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for v in &graph.vertices {
        lo = [lo[0].min(v[0]), lo[1].min(v[1])];
        hi = [hi[0].max(v[0]), hi[1].max(v[1])];
    }
    let inside = |p: [f64; 2]| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1];
    // All sources' stations in one list (source index per station).
    let mut sources: Vec<CountSource> = Vec::new();
    let mut stations: Vec<Station> = Vec::new();
    let mut source_of: Vec<usize> = Vec::new();
    for path in a.counts {
        let (src, st) = tc::read_counts(path).unwrap_or_else(|e| fail(e));
        println!("{}: {} stations ({})", src.id, st.len(), src.title);
        source_of.extend(std::iter::repeat_n(sources.len(), st.len()));
        stations.extend(st);
        sources.push(src);
    }
    let params = Params {
        tolerance_m: a.tolerance_m,
    };
    let matching = tc::match_stations(&graph, &traffic, &names, &proj, &stations, &params, inside);
    // The flow-1 volume per edge: the sim's class default and the year's
    // mean day factor of the edge's country.
    let mut annual: BTreeMap<ocw_world::traffic::Country, f64> = BTreeMap::new();
    let units: Vec<f64> = graph
        .edges
        .iter()
        .map(|e| {
            traffic.edges.get(&e.id).map_or(0.0, |r| {
                let f = *annual
                    .entry(r.country)
                    .or_insert_with(|| annual_day_factor(region_of(r.country)));
                unit_aadt(r, e.class) * f
            })
        })
        .collect();
    for (c, f) in &annual {
        println!("annual day factor {}: {f:.4}", c.code());
    }
    let flows = tc::flow_factors(&graph, &traffic, &names, &matching, |e| units[e]);
    let with = tc::apply_flows(&graph, &mut traffic, &flows);
    // Per source: what matched (only matched sources are credited).
    let mut used = Vec::new();
    let mut report = String::new();
    let _ = writeln!(
        report,
        "source\tstation\tname\tref\taadt\tresult\tedge\tdist_m\tflow"
    );
    for (si, src) in sources.iter().enumerate() {
        let of = |i: &usize| source_of[*i] == si;
        let total = source_of.iter().filter(|&&s| s == si).count();
        let outside = matching.outside.iter().filter(|i| of(i)).count();
        let matched = matching.anchors.iter().filter(|x| of(&x.station)).count();
        let count = |f: &dyn Fn(&Issue) -> bool| matching.issues.iter().filter(|i| f(i)).count();
        let unmatched = count(&|i| matches!(i, Issue::Unmatched { station, .. } if of(station)));
        let mismatch = count(&|i| matches!(i, Issue::RefMismatch { station, .. } if of(station)));
        let ambiguous = count(&|i| matches!(i, Issue::Ambiguous { station, .. } if of(station)));
        println!(
            "{}: {total} stations, {outside} outside the package, {matched} matched, \
             {unmatched} without a road within {:.0} m, {mismatch} with another road number near, \
             {ambiguous} ambiguous (the best used)",
            src.id, a.tolerance_m
        );
        if matched > 0 {
            used.push(src.attribution());
        }
    }
    let label = |i: usize| {
        let s = &stations[i];
        format!("{} {} {:?}", sources[source_of[i]].id, s.id, s.name)
    };
    for issue in &matching.issues {
        let line = match issue {
            Issue::Unmatched { station, nearest } => format!(
                "  unmatched: {} (nearest motor road {})",
                label(*station),
                nearest
                    .as_ref()
                    .map_or("none near".into(), |(d, r)| format!("{r}, {d:.0} m"))
            ),
            Issue::RefMismatch {
                station,
                wanted,
                found,
                dist_m,
            } => format!(
                "  road number: {} wants {wanted}, {dist_m:.0} m from {} – not used",
                label(*station),
                found.join(", ")
            ),
            Issue::Ambiguous { station, edges } => format!(
                "  ambiguous: {} between edges {} and {} (the first used)",
                label(*station),
                graph.edges[edges[0]].id.0,
                graph.edges[edges[1]].id.0
            ),
            Issue::SharedEdge { edge, stations: s } => format!(
                "  shared edge {}: {} (their mean used)",
                graph.edges[*edge].id.0,
                s.iter().map(|&i| label(i)).collect::<Vec<_>>().join(", ")
            ),
        };
        println!("{line}");
    }
    let flow_of = |e: usize| flows[e].map_or(1.0, f64::from);
    for (i, s) in stations.iter().enumerate() {
        let src = &sources[source_of[i]].id;
        let (result, edge, dist, flow) = if matching.outside.contains(&i) {
            (
                "outside".to_string(),
                String::new(),
                String::new(),
                String::new(),
            )
        } else if let Some(an) = matching.anchors.iter().find(|x| x.station == i) {
            (
                "matched".into(),
                graph.edges[an.edge].id.0.to_string(),
                format!("{:.1}", an.dist_m),
                format!("{:.2}", flow_of(an.edge)),
            )
        } else {
            (
                "not matched".into(),
                String::new(),
                String::new(),
                String::new(),
            )
        };
        let _ = writeln!(
            report,
            "{src}\t{}\t{}\t{}\t{:.0}\t{result}\t{edge}\t{dist}\t{flow}",
            s.id,
            s.name,
            s.reference.as_deref().unwrap_or(""),
            s.aadt
        );
    }
    // The factors' spread.
    let (mut km_with, mut km_motor) = (0.0, 0.0);
    let mut hist = [0usize; 6];
    for (e, edge) in graph.edges.iter().enumerate() {
        if units[e] <= 0.0 || !traffic.edges.get(&edge.id).is_some_and(|r| r.has_traffic()) {
            continue;
        }
        let len: f64 = edge
            .points
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum();
        km_motor += len / 1000.0;
        if let Some(f) = flows[e] {
            km_with += len / 1000.0;
            let f = f64::from(f);
            let b = [0.5, 0.8, 1.25, 2.0, 4.0]
                .iter()
                .take_while(|&&x| f >= x)
                .count();
            hist[b] += 1;
        }
    }
    println!(
        "flow factors: {with} edges ({km_with:.0} of {km_motor:.0} km of motor roads); \
         edges < 0.5: {}, 0.5 – 0.8: {}, 0.8 – 1.25: {}, 1.25 – 2: {}, 2 – 4: {}, ≥ 4: {}",
        hist[0], hist[1], hist[2], hist[3], hist[4], hist[5]
    );
    // The package: only TRFC replaced.
    let mut payload = Vec::new();
    write_traffic(&traffic, &mut payload);
    let new = ocw_world::package::replace_section(&bytes, TRAFFIC_TAG, &payload)
        .unwrap_or_else(|e| fail(e));
    let path = a.out.clone().unwrap_or_else(|| a.world.to_path_buf());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A new file (never written through a hard link to the original).
    let tmp = path.with_extension("ocwpkg.part");
    std::fs::write(&tmp, &new).unwrap_or_else(|e| fail(format!("{}: {e}", tmp.display())));
    std::fs::rename(&tmp, &path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    println!(
        "{}: {:.1} MB, TRFC {:.3} MB, {:.1} s",
        path.display(),
        new.len() as f64 / 1e6,
        payload.len() as f64 / 1e6,
        t0.elapsed().as_secs_f64()
    );
    if path != a.world {
        dist::carry_manifest(a.world, &path);
    }
    dist::write_manifest_with(
        &path,
        &dist::ManifestArgs {
            id: None,
            name: None,
            version: None,
            dem_attribution: Vec::new(),
            patches: None,
        },
        a.key,
        Some(&used),
    );
    if let Some(r) = &a.report {
        std::fs::write(r, &report).unwrap_or_else(|e| fail(format!("{}: {e}", r.display())));
        println!("report: {}", r.display());
    }
    if let Some(svg) = &a.svg {
        let title = format!("{} – traffic flow factors from open counts", pkg.name);
        let doc = tc::flow_svg(&title, &graph, &traffic, &proj, &stations, &matching);
        std::fs::write(svg, doc).unwrap_or_else(|e| fail(format!("{}: {e}", svg.display())));
        println!("map: {}", svg.display());
    }
}
