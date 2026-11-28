//! `world-builder plants-check`: no plant on a road (the world-building
//! rule of 2026-11-28, `ocw_world::road_clear`). Every plant the package
//! draws – the tree grid, the mapped trees and tree rows, the plantations'
//! rows, the fields' stands – is tested against the drawn road surfaces
//! (strips, graph edges, junction surfaces, bridges at their height); a
//! package with any plant on a road fails. `check`, `verify` and `publish`
//! run the same test and refuse such a package.

use std::path::Path;

use ocw_world::WorldPackage;
use ocw_world::geo::LocalProjection;
use ocw_world::road_clear::{Report, check_package};

/// The report of `pkg` (named `name`): the counts by source, part and
/// surface, then the first `list` cases with their coordinates.
pub fn report_text(name: &str, pkg: &WorldPackage, r: &Report, list: usize) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    macro_rules! println {
        ($($t:tt)*) => {{
            let _ = writeln!(out, $($t)*);
        }};
    }
    println!(
        "plants on roads: {} of {} plants in {} ({} tiles)",
        r.violations.len(),
        r.plants,
        name,
        pkg.tiles.len()
    );
    if r.violations.is_empty() {
        return out;
    }
    for ((source, part), n) in r.by_source() {
        println!("  {:<22} {:<5} {n}", source.label(), part.label());
    }
    let surfaces: Vec<String> = r
        .by_surface()
        .into_iter()
        .map(|(s, n)| format!("{} {n}", s.label()))
        .collect();
    println!("  on: {}", surfaces.join(", "));
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    for v in r.violations.iter().take(list) {
        let f = v.plant.foot;
        let (lat, lon) = proj.unproject([f[0], f[2]]);
        println!(
            "  x {:.1} z {:.1} ({lat:.6},{lon:.6}) tile {},{}: {} {} on {}, {:.2} m too near (foot {:.2} m from the edge){}",
            f[0],
            f[2],
            v.tile.x,
            v.tile.z,
            v.plant.source.label(),
            v.conflict.part.label(),
            v.conflict.surface.label(),
            v.conflict.by_m,
            v.conflict.edge_m,
            v.plant.species.map_or(String::new(), |s| format!(
                ", {} {:.1} m",
                s.name(),
                v.plant.shape.height_m
            )),
        );
    }
    if r.violations.len() > list {
        println!("  … and {} more", r.violations.len() - list);
    }
    out
}

/// `plants-check`: exits 1 when a plant stands on a road (`report_only`:
/// always 0).
pub fn run(world: &Path, list: usize, report_only: bool) {
    let pkg = crate::read_package(world);
    let t = std::time::Instant::now();
    let r = check_package(&pkg);
    print!(
        "{}",
        report_text(&world.display().to_string(), &pkg, &r, list)
    );
    println!("  checked in {:.1} s", t.elapsed().as_secs_f64());
    if !r.violations.is_empty() && !report_only {
        refuse(world, &r);
    }
}

fn refuse(world: &Path, r: &Report) -> ! {
    eprintln!(
        "{}: {} plants stand on a road – no tree or plant may stand on a road surface in any world (world-builder plants-check, ocw_world::road_clear); rebuild the package with this world-builder",
        world.display(),
        r.violations.len()
    );
    std::process::exit(1);
}

/// The test of `check`, `verify` and `publish`: a package with a plant
/// on a road is refused (exit 1) with its first cases.
pub fn enforce(world: &Path) {
    let pkg = crate::read_package(world);
    let r = check_package(&pkg);
    if r.violations.is_empty() {
        println!("  plants on roads: none ({} plants)", r.plants);
        return;
    }
    // On stderr: the build scripts filter `check`'s output.
    eprint!(
        "{}",
        report_text(&world.display().to_string(), &pkg, &r, 10)
    );
    refuse(world, &r);
}

/// The other objects on the drawn road surfaces (report only, `--others`):
/// street-lamp feet (`ocw_world::lamps::package_lamps`) and buildings
/// (outline corners and edge middles, 1 m inside the walls, at least 1 m
/// on the surface).
pub fn others(world: &Path) {
    use ocw_world::road_clear::{PlantShape, RoadSpace};
    let pkg = crate::read_package(world);
    let point = PlantShape::hedge(1.0, 0.0);
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let mut lamps = (0usize, 0usize, Vec::new());
    for (key, list) in ocw_world::lamps::package_lamps(&pkg) {
        let space = RoadSpace::for_tile(&pkg, key);
        for l in list {
            lamps.0 += 1;
            let f = l.foot.map(f64::from);
            if let Some(c) = space.conflict(f, &point) {
                lamps.1 += 1;
                if lamps.2.len() < 5 {
                    lamps.2.push((f, c));
                }
            }
        }
    }
    println!("street lamps on a road surface: {} of {}", lamps.1, lamps.0);
    for (f, c) in &lamps.2 {
        let (lat, lon) = proj.unproject([f[0], f[2]]);
        println!(
            "  x {:.1} z {:.1} ({lat:.6},{lon:.6}) on {}, {:.2} m in",
            f[0],
            f[2],
            c.surface.label(),
            -c.edge_m
        );
    }
    let mut houses = (0usize, 0usize, Vec::new());
    if let Some(all) = &pkg.buildings {
        for (key, list) in all {
            let space = RoadSpace::for_tile(&pkg, *key);
            for b in list {
                houses.0 += 1;
                let n = b.outline.len();
                if n < 3 {
                    continue;
                }
                let c = b
                    .outline
                    .iter()
                    .fold([0.0, 0.0], |s, p| {
                        [s[0] + f64::from(p[0]), s[1] + f64::from(p[1])]
                    })
                    .map(|v| v / n as f64);
                let mut hit = None;
                'edges: for i in 0..n {
                    let a = b.outline[i].map(f64::from);
                    let e = b.outline[(i + 1) % n].map(f64::from);
                    for q in [a, [(a[0] + e[0]) / 2.0, (a[1] + e[1]) / 2.0]] {
                        // 1 m inside the walls.
                        let d = ((c[0] - q[0]).powi(2) + (c[1] - q[1]).powi(2)).sqrt();
                        let t = if d > 1.0 { 1.0 / d } else { 0.0 };
                        let p = [q[0] + (c[0] - q[0]) * t, q[1] + (c[1] - q[1]) * t];
                        let Some(y) = pkg.height(p[0], p[1]) else {
                            continue;
                        };
                        if let Some(cf) = space
                            .conflict([p[0], y, p[1]], &point)
                            .filter(|cf| cf.edge_m < -1.0)
                        {
                            hit = Some((p, cf));
                            break 'edges;
                        }
                    }
                }
                if let Some(h) = hit {
                    houses.1 += 1;
                    if houses.2.len() < 5 {
                        houses.2.push(h);
                    }
                }
            }
        }
    }
    println!(
        "buildings standing on a road surface: {} of {}",
        houses.1, houses.0
    );
    for (p, c) in &houses.2 {
        let (lat, lon) = proj.unproject(*p);
        println!(
            "  x {:.1} z {:.1} ({lat:.6},{lon:.6}) on {}, {:.2} m in",
            p[0],
            p[1],
            c.surface.label(),
            -c.edge_m
        );
    }
}
