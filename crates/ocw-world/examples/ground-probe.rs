//! What a world package says about the ground at some points (diagnostics
//! for the terrain colours, spec 4.6): the altitude, the land class at the
//! point and around it, the measured forest, the region's measured palette
//! and the seasonal satellite colours, the point obstacles nearby.
//!
//! ```text
//! cargo run --release -p ocw-world --example ground-probe -- PKG X,Z [X,Z …] [--radius M]
//! ```

use std::collections::BTreeMap;
use std::process::ExitCode;

use ocw_world::landuse::LandClass;
use ocw_world::look::PaletteKey;
use ocw_world::package::{TileKey, WorldPackage};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: ground-probe PKG X,Z [X,Z …] [--radius M]");
        return ExitCode::from(2);
    };
    let mut radius = 200.0f64;
    let mut points: Vec<[f64; 2]> = Vec::new();
    while let Some(a) = args.next() {
        if a == "--radius" {
            radius = args.next().and_then(|r| r.parse().ok()).unwrap_or(radius);
            continue;
        }
        let v: Vec<f64> = a.split(',').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 2 {
            points.push([v[0], v[1]]);
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pkg = match WorldPackage::from_bytes(&bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let size = f64::from(pkg.tile_size_m);
    println!(
        "package {path}: {} tiles, {} samples ({:.1} m grid), tz {:?}",
        pkg.tiles.len(),
        pkg.samples,
        pkg.spacing_m(),
        pkg.time_zone
    );
    if let Some(p) = &pkg.look.palette {
        for e in &p.entries {
            println!("palette {:?}: {:?} ({} samples)", e.key, e.rgb, e.samples);
        }
    }
    if let Some(s) = &pkg.look.seasons {
        for (i, info) in s.seasons.iter().enumerate() {
            let means: Vec<String> = s.class_means[i]
                .iter()
                .map(|(c, rgb)| format!("{c:?} {rgb:?}"))
                .collect();
            println!(
                "season {i} {:?} doy {}: {}",
                info.season,
                info.centre_doy,
                means.join(", ")
            );
        }
    }
    if let Some(lu) = &pkg.landuse {
        let mut counts = [0u64; 17];
        let mut water_tiles = 0;
        for t in lu.tiles.values() {
            for c in &t.classes {
                counts[c.code() as usize] += 1;
            }
            water_tiles += usize::from(!t.water.is_empty());
        }
        let total: u64 = counts.iter().sum();
        let line: Vec<String> = LandClass::ALL
            .iter()
            .filter(|c| counts[c.code() as usize] > 0)
            .map(|c| {
                format!(
                    "{c:?} {:.1}%",
                    100.0 * counts[c.code() as usize] as f64 / total as f64
                )
            })
            .collect();
        println!(
            "classes: {} ; tiles with water: {water_tiles}",
            line.join(", ")
        );
    }
    for p in points {
        let key = TileKey {
            x: (p[0] / size).floor() as i32,
            z: (p[1] / size).floor() as i32,
        };
        let alt = pkg.height(p[0], p[1]);
        let class = |x: f64, z: f64| -> Option<LandClass> {
            let k = TileKey {
                x: (x / size).floor() as i32,
                z: (z / size).floor() as i32,
            };
            let (lu, cells) = ocw_world::trees::tile_landuse(&pkg, k)?;
            let o = k.origin(size);
            let cs = size / f64::from(cells);
            let c = (((x - o[0]) / cs) as u32).min(cells - 1);
            let r = (((z - o[1]) / cs) as u32).min(cells - 1);
            lu.classes.get((r * cells + c) as usize).copied()
        };
        let mut hist: BTreeMap<String, u32> = BTreeMap::new();
        let mut n = 0;
        let step = 10.0;
        let mut z = p[1] - radius;
        while z <= p[1] + radius {
            let mut x = p[0] - radius;
            while x <= p[0] + radius {
                if let Some(c) = class(x, z) {
                    *hist.entry(format!("{c:?}")).or_default() += 1;
                    n += 1;
                }
                x += step;
            }
            z += step;
        }
        let hist: Vec<String> = hist
            .iter()
            .map(|(c, k)| format!("{c} {:.0}%", 100.0 * f64::from(*k) / f64::from(n.max(1))))
            .collect();
        let forest = ocw_world::trees::tile_forest(&pkg, key)
            .and_then(|(f, cells)| f.at(key, size, cells, p[0], p[1]));
        println!(
            "\npoint {:.0},{:.0} tile {},{}: alt {:?} m, class {:?}, within {radius} m: {}",
            p[0],
            p[1],
            key.x,
            key.z,
            alt.map(|a| a.round()),
            class(p[0], p[1]),
            hist.join(", ")
        );
        println!("  forest {forest:?}");
        if let Some(s) = &pkg.look.seasons {
            for (i, info) in s.seasons.iter().enumerate() {
                let c = s.map(key, i).map(|m| {
                    let o = key.origin(size);
                    let cs = size / f64::from(s.cells);
                    let cc = (((p[0] - o[0]) / cs) as u32).min(s.cells - 1);
                    let rr = (((p[1] - o[1]) / cs) as u32).min(s.cells - 1);
                    m[(rr * s.cells + cc) as usize]
                });
                println!("  satellite {:?}: {c:?}", info.season);
            }
        }
        if let Some(c) = class(p[0], p[1])
            && let Some(pal) = &pkg.look.palette
        {
            println!(
                "  palette of its class: {:?}",
                pal.colour(PaletteKey::Land(c))
            );
        }
        if let Some(obst) = &pkg.obstacles {
            for t in obst.tiles.values() {
                for o in &t.points {
                    let d = (f64::from(o.pos[0]) - p[0]).hypot(f64::from(o.pos[1]) - p[1]);
                    if d <= radius * 3.0
                        && o.kind != ocw_world::obstacles::ObstacleKind::PlantedTree
                    {
                        println!(
                            "  obstacle {:?} at {:.0},{:.0} ({d:.0} m) height {:.1} m flags {}",
                            o.kind, o.pos[0], o.pos[1], o.height_m, o.flags
                        );
                    }
                }
            }
        }
    }
    ExitCode::SUCCESS
}
