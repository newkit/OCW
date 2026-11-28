//! Terrain shadows and shading of a world package for one sun (diagnostics
//! for spec 4.5 *Terrain shadows*): the shadow-line grid's size and build
//! time, the share of the terrain in the shadow of the mountains, a map of
//! it, and how the far terrain's 100 m normals shade the slopes compared
//! with the near tiles' 10 m normals (N·L, the same sun).
//!
//! ```text
//! cargo run --release -p ocw-world --example terrain-shadow-probe -- PKG \
//!     --utc 2026-10-08T15:45 [--at X,Z ...] [--map COLS] [--pick N]
//! ```

use std::process::ExitCode;
use std::time::Instant;

use ocw_world::package::WorldPackage;
use ocw_world::terrain_shadow::{HeightGrid, MAX_SIDE, MIN_CELL_M, shadow_lines};

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_utc(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let d: Vec<i64> = date.split('-').filter_map(|v| v.parse().ok()).collect();
    let t: Vec<i64> = time.split(':').filter_map(|v| v.parse().ok()).collect();
    if d.len() != 3 || t.len() < 2 {
        return None;
    }
    Some((days_from_civil(d[0], d[1], d[2]) * 1440 + t[0] * 60 + t[1]) * 60_000)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: terrain-shadow-probe PKG --utc Y-M-DTH:M [--at X,Z ...] [--map COLS]");
        return ExitCode::from(2);
    };
    let mut when = None;
    let mut points = Vec::new();
    let mut cols = 100usize;
    let mut pick = 0usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--utc" => when = args.next().and_then(|s| parse_utc(&s)),
            "--pick" => pick = args.next().and_then(|s| s.parse().ok()).unwrap_or(1),
            "--map" => cols = args.next().and_then(|s| s.parse().ok()).unwrap_or(cols),
            "--at" => {
                if let Some(v) = args.next() {
                    let v: Vec<f64> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                    if v.len() == 2 {
                        points.push([v[0], v[1]]);
                    }
                }
            }
            _ => {}
        }
    }
    let Some(ms) = when else {
        eprintln!("--utc Y-M-DTH:M needed");
        return ExitCode::from(2);
    };
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
    let sun = ocw_sky::sun::position(ms, pkg.origin.0, pkg.origin.1);
    let (az, el) = (sun.azimuth_deg.to_radians(), sun.elevation_deg.to_radians());
    let towards = [
        (el.cos() * az.sin()) as f32,
        el.sin() as f32,
        -(el.cos() * az.cos()) as f32,
    ];
    println!(
        "sun azimuth {:.1}°, elevation {:.1}°",
        sun.azimuth_deg, sun.elevation_deg
    );
    let t0 = Instant::now();
    let Some(grid) = HeightGrid::of_package(&pkg, MIN_CELL_M, MAX_SIDE) else {
        eprintln!("no terrain");
        return ExitCode::FAILURE;
    };
    let t1 = Instant::now();
    let lines = shadow_lines(&grid, towards);
    let t2 = Instant::now();
    println!(
        "grid {} x {} nodes, {:.0} m cells, heights {:.0} – {:.0} m: sampled in {:.0} ms, shadow lines in {:.1} ms; {:.1} MB heights, {:.1} MB lines (GPU: RG16F {:.1} MB)",
        grid.nx,
        grid.nz,
        grid.cell_m,
        grid.min,
        grid.max,
        (t1 - t0).as_secs_f64() * 1000.0,
        (t2 - t1).as_secs_f64() * 1000.0,
        grid.bytes() as f64 / 1e6,
        lines.bytes() as f64 / 1e6,
        (grid.nx * grid.nz * 4) as f64 / 1e6,
    );
    let mut shaded = 0usize;
    for (h, l) in grid.heights.iter().zip(&lines.line) {
        if ocw_world::terrain_shadow::visibility(*h, l[0], l[1]) < 0.5 {
            shaded += 1;
        }
    }
    println!(
        "terrain in the shadow of the mountains: {:.1} % of the nodes",
        100.0 * shaded as f64 / grid.heights.len() as f64
    );
    // The map: '#' shadow, '.' sun, ' ' no terrain; north up.
    let rows = cols * grid.nz / grid.nx / 2;
    for r in 0..rows.max(1) {
        let mut s = String::new();
        for c in 0..cols {
            let x = grid.origin[0]
                + (c as f64 + 0.5) / cols as f64 * (grid.nx - 1) as f64 * grid.cell_m;
            let z = grid.origin[1]
                + (r as f64 + 0.5) / rows as f64 * (grid.nz - 1) as f64 * grid.cell_m;
            let ch = match pkg.height(x, z) {
                None => ' ',
                Some(h) => {
                    if lines.sun([x, z], h as f32) < 0.5 {
                        '#'
                    } else {
                        '.'
                    }
                }
            };
            s.push(ch);
        }
        println!("{s}");
    }
    println!(
        "map: x {:.0} … {:.0}, z {:.0} … {:.0} (north up)",
        grid.origin[0],
        grid.origin[0] + (grid.nx - 1) as f64 * grid.cell_m,
        grid.origin[1],
        grid.origin[1] + (grid.nz - 1) as f64 * grid.cell_m
    );
    for p in &points {
        let h = pkg.height(p[0], p[1]);
        let l = lines.at(*p);
        println!(
            "at {:.0},{:.0}: height {:?}, shadow line {:?}, sun {:.2}",
            p[0],
            p[1],
            h.map(|h| h.round()),
            l.map(|l| [l[0].round(), l[1].round()]),
            h.map_or(1.0, |h| lines.sun(*p, h as f32))
        );
    }
    shading(&pkg, towards);
    if pick > 0 {
        pick_paths(&grid, &lines, towards, pick);
    }
    ExitCode::SUCCESS
}

/// Camera paths for the approach pictures: targets deep in the
/// mountains' shadow with sunlit ground within 1.5 km (an edge to see),
/// each with a bearing from which the target stays in sight from 10 km to
/// 1 km at one altitude, the sun to the side. Prints
/// `PATH n target_x target_z target_h bearing altitude` and one
/// `POSE n distance x z height_above_ground yaw pitch` per 500 m.
fn pick_paths(
    grid: &HeightGrid,
    lines: &ocw_world::terrain_shadow::ShadowLines,
    sun: [f32; 3],
    count: usize,
) {
    let c = grid.cell_m;
    let mut cands = Vec::new();
    let step = (500.0 / c).round().max(1.0) as usize;
    for k in (0..grid.nz).step_by(step) {
        for i in (0..grid.nx).step_by(step) {
            let p = [grid.origin[0] + i as f64 * c, grid.origin[1] + k as f64 * c];
            let h = grid.heights[k * grid.nx + i];
            let Some([s, _]) = lines.at(p) else { continue };
            let depth = s - h;
            if depth < 120.0 {
                continue;
            }
            // Sunlit ground within 1.5 km.
            let lit = (0..16).any(|a| {
                let t = f64::from(a) / 16.0 * std::f64::consts::TAU;
                let q = [p[0] + 1500.0 * t.cos(), p[1] + 1500.0 * t.sin()];
                grid.height(q).is_some_and(|hq| lines.sun(q, hq) > 0.9)
            });
            if lit {
                cands.push((depth, p, h));
            }
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sun_bearing = f64::from(sun[0]).atan2(-f64::from(sun[2])).to_degrees();
    let mut found = 0;
    let mut used: Vec<[f64; 2]> = Vec::new();
    for (_, t, ht) in cands {
        if found >= count {
            break;
        }
        if used
            .iter()
            .any(|u| (u[0] - t[0]).hypot(u[1] - t[1]) < 4000.0)
        {
            continue;
        }
        // Bearings from the target to the camera, the sun to the side
        // first (90° off), then others.
        for off in [90.0, -90.0, 60.0, -60.0, 120.0, -120.0] {
            let b = (sun_bearing + off).to_radians();
            let dir = [b.sin(), -b.cos()];
            // The highest ground under the path, the camera above it.
            let mut top = f32::MIN;
            let mut inside = true;
            for d in (1000..=10_000).step_by(100) {
                let q = [t[0] + dir[0] * f64::from(d), t[1] + dir[1] * f64::from(d)];
                match grid.height(q) {
                    Some(h) => top = top.max(h),
                    None => inside = false,
                }
            }
            if !inside {
                continue;
            }
            let alt = f64::from(top) + 150.0;
            // In sight from every pose: no ground above the line of sight.
            let clear = (1000..=10_000).step_by(500).all(|d| {
                let d = f64::from(d);
                (1..100).all(|j| {
                    let f = f64::from(j) / 100.0;
                    let q = [t[0] + dir[0] * d * f, t[1] + dir[1] * d * f];
                    let y = f64::from(ht) + (alt - f64::from(ht)) * f;
                    grid.height(q).is_none_or(|h| f64::from(h) < y - 5.0)
                })
            });
            if !clear {
                continue;
            }
            found += 1;
            used.push(t);
            let bearing = (b.to_degrees() + 360.0) % 360.0;
            println!(
                "PATH {found} {:.0} {:.0} {:.0} {:.0} {:.0}",
                t[0], t[1], ht, bearing, alt
            );
            // Every 500 m from 10 km to 1 km, then into the cascades' reach.
            let near = [500, 250];
            for d in (1000..=10_000).rev().step_by(500).chain(near) {
                let d = f64::from(d);
                let q = [t[0] + dir[0] * d, t[1] + dir[1] * d];
                let g = f64::from(grid.height(q).unwrap_or(0.0));
                // Close in the camera comes down towards the target.
                let alt = if d < 1000.0 {
                    f64::from(ht) + (alt - f64::from(ht)) * d / 1000.0
                } else {
                    alt
                };
                // Looking back at the target.
                let yaw = (bearing + 180.0) % 360.0;
                let pitch = ((f64::from(ht) - alt) / d).atan().to_degrees();
                println!(
                    "POSE {found} {d:.0} {:.0} {:.0} {:.0} {yaw:.1} {pitch:.1}",
                    q[0],
                    q[1],
                    (alt - g).max(30.0)
                );
            }
            break;
        }
    }
}

/// N·L of the far terrain's normals (central differences over the far
/// stride, as `far_terrain::far_tile_arrays`) against the mean of the near
/// tiles' (over the 10 m samples of the far cell, as
/// `package::terrain_arrays`), both clamped at 0 as the lighting is.
fn shading(pkg: &WorldPackage, l: [f32; 3]) {
    let n = pkg.samples as usize;
    let cells = n - 1;
    let want = ((100.0 / pkg.spacing_m()).round() as usize).clamp(1, cells);
    let stride = (1..=want)
        .rev()
        .find(|d| cells.is_multiple_of(*d))
        .unwrap_or(1);
    let s = pkg.spacing_m() as f32;
    let ndl = |dx: f32, dz: f32| {
        let len = (dx * dx + 1.0 + dz * dz).sqrt();
        ((-dx * l[0] + l[1] - dz * l[2]) / len).max(0.0)
    };
    let mut diffs = Vec::new();
    let (mut far_sum, mut near_sum) = (0.0f64, 0.0f64);
    for tile in pkg.tiles.values().filter(|t| !t.heights.is_empty()) {
        let h = |i: usize, k: usize| tile.heights[k.min(n - 1) * n + i.min(n - 1)];
        let m = cells / stride + 1;
        for kf in 1..m - 1 {
            for if_ in 1..m - 1 {
                let (i, k) = (if_ * stride, kf * stride);
                let dx = (h(i + stride, k) - h(i - stride, k)) / (2.0 * stride as f32 * s);
                let dz = (h(i, k + stride) - h(i, k - stride)) / (2.0 * stride as f32 * s);
                let far = ndl(dx, dz);
                let half = stride / 2;
                let (mut sum, mut cnt) = (0.0f32, 0);
                for kk in k - half..=k + half {
                    for ii in i - half..=i + half {
                        let dx = (h(ii + 1, kk) - h(ii - 1, kk)) / (2.0 * s);
                        let dz = (h(ii, kk + 1) - h(ii, kk - 1)) / (2.0 * s);
                        sum += ndl(dx, dz);
                        cnt += 1;
                    }
                }
                let near = sum / cnt as f32;
                far_sum += f64::from(far);
                near_sum += f64::from(near);
                diffs.push(far - near);
            }
        }
    }
    if diffs.is_empty() {
        return;
    }
    diffs.sort_by(f32::total_cmp);
    let q = |f: f64| diffs[((diffs.len() - 1) as f64 * f) as usize];
    let mean = diffs.iter().map(|d| f64::from(*d)).sum::<f64>() / diffs.len() as f64;
    let abs = diffs.iter().map(|d| f64::from(d.abs())).sum::<f64>() / diffs.len() as f64;
    println!(
        "shading N·L, far (100 m normals) − near (mean of 10 m normals) over {} far cells: mean {:+.3}, mean |Δ| {:.3}, 5 % {:+.3}, 50 % {:+.3}, 95 % {:+.3}; mean N·L far {:.3}, near {:.3}",
        diffs.len(),
        mean,
        abs,
        q(0.05),
        q(0.5),
        q(0.95),
        far_sum / diffs.len() as f64,
        near_sum / diffs.len() as f64
    );
}
