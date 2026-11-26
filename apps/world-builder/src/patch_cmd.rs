//! World patches on the command line (spec §14 Phase 16, 16C, E5 step 1):
//! reading a patch stack for `package --patch`, and `patch check`, which
//! validates a patch against a built package (and its OSM sources) and
//! reports the operations whose target is gone.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use ocw_world::WorldPackage;
use ocw_world::geo::LocalProjection;
use ocw_world::patch::{LoadedPatch, Report, Targets, apply_height_points, apply_road_edits};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::patch::Patching;

use crate::dist;

#[derive(Subcommand)]
pub enum PatchCmd {
    /// Check a patch stack against a package: which operations find their
    /// target (OSM way, stable edge id, a road at a height point) and which
    /// do not. Exits 1 if any target is gone.
    Check {
        /// The built package (version 2, with a road graph).
        #[arg(long)]
        world: PathBuf,
        /// Patch files, applied in this order.
        #[arg(required = true, value_name = "FILE")]
        patch: Vec<PathBuf>,
        /// The OSM extract(s) to check way and node targets against
        /// (default: way targets against the package's road graph; node
        /// targets stay unchecked).
        #[arg(long)]
        pbf: Vec<PathBuf>,
    },
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

/// Reads a patch stack; the label of each is its file name.
pub fn load(paths: &[PathBuf]) -> Result<Vec<LoadedPatch>, String> {
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let label = p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            LoadedPatch::parse(label, &text).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

/// The id a package (being) built at `package` has: the one given, else its
/// manifest's, else the one made from its name (as `dist::write_manifest`
/// decides it).
pub fn package_id(package: &Path, given: Option<&str>, name: &str) -> String {
    if let Some(id) = given {
        return id.to_string();
    }
    std::fs::read_to_string(dist::sidecar(package))
        .ok()
        .and_then(|t| ocw_world::SignedManifest::parse(&t).ok())
        .map(|s| s.manifest.id)
        .unwrap_or_else(|| dist::id_from_name(name))
}

/// Every patch must be made for the package `id`.
pub fn check_ids(stack: &[LoadedPatch], id: &str) -> Result<(), String> {
    for p in stack {
        if p.patch.package != id {
            return Err(format!(
                "{}: the patch is for package `{}`, not `{id}`",
                p.label, p.patch.package
            ));
        }
    }
    Ok(())
}

/// The stack for `package --patch`: read and checked before the build
/// starts (exits 2 on a broken patch or one for another package).
pub fn load_for_build(
    paths: &[PathBuf],
    out: &Path,
    manifest: &crate::ManifestOpts,
    name: &str,
) -> Vec<LoadedPatch> {
    let stack = load(paths).unwrap_or_else(|e| fail(e));
    if !stack.is_empty() {
        let id = package_id(out, manifest.id.as_deref(), name);
        check_ids(&stack, &id).unwrap_or_else(|e| fail(e));
        for p in &stack {
            println!(
                "patch {}: {} operations for {}{}",
                p.label,
                p.patch.ops.len(),
                p.patch.package,
                p.patch
                    .title
                    .as_deref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default()
            );
        }
    }
    stack
}

pub fn print_report(report: &Report) {
    for line in report.lines() {
        println!("{line}");
    }
    if report.missing_count() > 0 {
        println!(
            "patch: WARNING – {} operation(s) not applied: their target is gone (see MISSING above)",
            report.missing_count()
        );
    }
}

/// `package --strict-patch` (16H Q7 (a), our official builds): the
/// failure message when an operation lost its target – which ones and
/// what to do; `None` when every target was found.
pub fn strict_failure(report: &Report) -> Option<String> {
    let n = report.missing_count();
    if n == 0 {
        return None;
    }
    let lost: Vec<String> = report
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("  MISSING "))
        .collect();
    Some(format!(
        "patch: FAILED (--strict-patch) – {n} operation(s) lost their target (a road or node gone or redrawn in this OSM data); the package was not written:
{}
Fix the patch (target the road anew in OCW Studio, or delete the operation) and build again; `world-builder patch check --world PKG --patch FILE --pbf PBF` lists them without a build.",
        lost.join("
")
    ))
}

/// Checks a stack against a built package: with `osm`, way and node
/// targets against the OSM data, else way targets against the package's
/// graph; edge targets and height points against the graph.
pub fn check_package(
    pkg: &WorldPackage,
    stack: &[LoadedPatch],
    osm: Option<&mut OsmData>,
) -> Result<Report, String> {
    let mut graph = pkg
        .graph
        .as_deref()
        .cloned()
        .ok_or("the package has no road graph (rebuild it with this world-builder)")?;
    let mut patching = Patching::new(stack);
    let targets = match osm {
        Some(osm) => {
            patching.on_osm(osm);
            Targets::EdgesOnly
        }
        None => Targets::All,
    };
    let Patching { plan, report, .. } = &mut patching;
    apply_road_edits(plan, &mut graph, targets, report);
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    apply_height_points(plan, &mut graph, &proj, report);
    Ok(patching.report)
}

fn check(world: &Path, paths: &[PathBuf], pbf: &[PathBuf]) -> i32 {
    let stack = load(paths).unwrap_or_else(|e| fail(e));
    let pkg = crate::read_package(world);
    let id = package_id(world, None, &pkg.name);
    if let Err(e) = check_ids(&stack, &id) {
        eprintln!("{e}");
        return 1;
    }
    let mut osm = (!pbf.is_empty()).then(|| OsmData::read_pbfs(pbf).unwrap_or_else(|e| fail(e)));
    let report = check_package(&pkg, &stack, osm.as_mut()).unwrap_or_else(|e| fail(e));
    println!("{} ({id}):", world.display());
    print_report(&report);
    i32::from(report.missing_count() > 0)
}

pub fn run(cmd: PatchCmd) {
    match cmd {
        PatchCmd::Check { world, patch, pbf } => {
            let code = check(&world, &patch, &pbf);
            if code != 0 {
                std::process::exit(code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ocw_world::graph::PROFILE_STEP_M;
    use ocw_world::patch::{OpRef, Outcome};

    use super::*;

    const MINI: &str = include_str!("../../../crates/ocw-worldgen/tests/fixtures/mini-v1.osm");

    /// The mini region of the world pipeline tests as a package, its
    /// roads flat at 500 m (no DEM needed).
    fn mini_package() -> (WorldPackage, OsmData) {
        let osm = OsmData::parse_osm_xml(MINI).unwrap();
        let region = ocw_worldgen::build::Region::bbox(46.490, 9.485, 46.510, 9.515);
        let (mut graph, _) = ocw_worldgen::build::build_road_graph(&osm, &region);
        for e in &mut graph.edges {
            e.profile = vec![500.0; (e.length_m() / PROFILE_STEP_M).ceil() as usize + 1];
        }
        let pkg = ocw_worldgen::package::build_package(
            "Mini",
            region.center(),
            &graph,
            |_, _| 500.0,
            1000.0,
            11,
            0,
        );
        (pkg, osm)
    }

    fn stack(body: &str) -> Vec<LoadedPatch> {
        vec![
            LoadedPatch::parse(
                "mini.ocwpatch",
                &format!("ocwpatch = 1\npackage = \"mini\"\n{body}"),
            )
            .unwrap(),
        ]
    }

    const BODY: &str = r#"
[[op]]
op = "road"
way = 105
surface = "gravel"

[[op]]
op = "remove_road"
way = 999

[[op]]
op = "height_point"
id = "at-node-2"
node = 2
raise_m = 2.0

[[op]]
op = "height_point"
id = "on-hauptstrasse"
at = [46.5000, 9.4925]
height_m = 498.0

[[op]]
op = "height_point"
id = "in-a-field"
at = [46.5060, 9.4925]
height_m = 498.0
"#;

    #[test]
    fn check_against_the_package_reports_gone_targets() {
        let (pkg, mut osm) = mini_package();
        let s = stack(BODY);
        let outcome = |r: &Report, op| r.get(OpRef { patch: 0, op }).unwrap().outcome.clone();

        // The package alone: way targets against its graph, node unchecked.
        let r = check_package(&pkg, &s, None).unwrap();
        assert!(matches!(outcome(&r, 0), Outcome::Applied(_)));
        assert!(matches!(outcome(&r, 1), Outcome::Missing(m) if m.contains("way 999")));
        assert!(matches!(outcome(&r, 2), Outcome::Unchecked(_)));
        assert!(matches!(outcome(&r, 3), Outcome::Applied(_)));
        assert!(matches!(outcome(&r, 4), Outcome::Missing(m) if m.contains("no road")));
        assert_eq!(r.missing_count(), 2);

        // With the OSM sources the node target is placed too.
        let r = check_package(&pkg, &s, Some(&mut osm)).unwrap();
        assert!(matches!(outcome(&r, 2), Outcome::Applied(_)));
        assert!(matches!(outcome(&r, 1), Outcome::Missing(m) if m.contains("way 999")));
        assert_eq!(r.unchecked_count(), 0);

        // `package --strict-patch` (16H Q7 (a)): the lost targets fail
        // the official build, named one by one; all found: no failure.
        let msg = strict_failure(&r).expect("fails");
        assert!(msg.contains("FAILED (--strict-patch) – 2 operation(s) lost their target"));
        assert!(
            msg.contains("way 999") && msg.contains("in-a-field"),
            "{msg}"
        );
        assert!(!msg.contains("on-hauptstrasse"), "{msg}");
        assert!(msg.contains("not written"));
        let fine = check_package(
            &pkg,
            &stack(&BODY[..BODY.find("[[op]]\nop = \"remove_road\"").unwrap()]),
            Some(&mut osm),
        )
        .unwrap();
        assert_eq!(fine.missing_count(), 0);
        assert_eq!(strict_failure(&fine), None);
    }

    #[test]
    fn patches_must_be_for_the_package() {
        let dir = tempfile::tempdir().unwrap();
        let s = stack("");
        let pkg = dir.path().join("mini.ocwpkg");
        assert_eq!(package_id(&pkg, None, "Mini"), "mini");
        assert_eq!(package_id(&pkg, Some("europe/mini"), "Mini"), "europe/mini");
        assert!(check_ids(&s, "mini").is_ok());
        let e = check_ids(&s, "europe/mini").unwrap_err();
        assert!(e.contains("`mini`") && e.contains("europe/mini"), "{e}");
    }

    #[test]
    fn load_names_the_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.ocwpatch");
        std::fs::write(&good, format!("ocwpatch = 1\npackage = \"mini\"\n{BODY}")).unwrap();
        let bad = dir.path().join("bad.ocwpatch");
        std::fs::write(
            &bad,
            "ocwpatch = 1\npackage = \"mini\"\n[[op]]\nop = \"road\"\nway = 1\nwidth_m = 99\n",
        )
        .unwrap();
        let s = load(std::slice::from_ref(&good)).unwrap();
        assert_eq!(s[0].label, "good.ocwpatch");
        assert_eq!(s[0].patch.ops.len(), 5);
        let e = load(&[good, bad]).unwrap_err();
        assert!(e.contains("bad.ocwpatch: line"), "{e}");
        assert!(e.contains("width_m"), "{e}");
    }
}
