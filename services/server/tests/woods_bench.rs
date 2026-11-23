//! The tick time with a rider deep in a wood (spec 14 Phase 13 Q10 (a)):
//! the game server's session on a real world package, a rider landed far
//! from any road in a dense wood, riding about among the trunks for a
//! minute – µs per tick with the trunks made on demand and without them.
//!
//! Not run by default (needs a package): `OCW_BENCH_WORLD=data/…/x.ocwpkg
//! cargo test --release -p ocw-server --test woods_bench -- --ignored
//! --nocapture`.

use std::time::Instant;

use ocw_core::RiderProfile;
use ocw_server::Course;
use ocw_sim::god::GodPose;
use ocw_sim::{GodLeave, RiderInput, Session};
use ocw_world::landuse::LandClass;
use ocw_world::woods::{Stored, WoodCell};

/// A spot in a wood at least `away` m from any road with the densest
/// trunks found on a 100 m grid of the package.
fn deep_in_a_wood(net: &ocw_world::RoadNetwork, away: f64) -> Option<([f64; 2], usize)> {
    let woods = net.woods()?.clone();
    let pkg = woods.package()?;
    let size = f64::from(pkg.tile_size_m);
    let lu = pkg.landuse.as_ref()?;
    let mut best: Option<([f64; 2], usize)> = None;
    for key in pkg.tiles.keys() {
        let o = key.origin(size);
        for k in 0..10 {
            for i in 0..10 {
                let p = [
                    o[0] + 50.0 + f64::from(i) * 100.0,
                    o[1] + 50.0 + f64::from(k) * 100.0,
                ];
                if lu.class_at(size, p[0], p[1]) != Some(LandClass::Forest)
                    || net.nearest_within(p, away).is_some()
                {
                    continue;
                }
                let cell = WoodCell::containing(p, pkg.tile_size_m);
                let n = woods
                    .grid_trunks(cell, &Stored::default())
                    .map_or(0, |t| t.len());
                if best.is_none_or(|(_, b)| n > b) {
                    best = Some((p, n));
                }
            }
        }
        if best.is_some_and(|(_, n)| n >= 22) {
            break;
        }
    }
    best
}

/// µs per tick (mean, the slowest) over `secs` of riding with lean wiggles.
fn ride(
    net: std::sync::Arc<ocw_world::RoadNetwork>,
    start: ocw_sim::FreeRideStart,
    at: [f64; 2],
    secs: f64,
) -> (f64, f64, usize, usize) {
    let mut s = Session::free_ride(net, start);
    s.set_collisions(true);
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    s.enter_god(me).unwrap();
    // A free spot near `at` (the landing refuses trunks).
    let mut landed = false;
    'find: for r in 0..20 {
        for k in 0..16 {
            let a = f64::from(k) * std::f64::consts::TAU / 16.0;
            let (x, z) = (
                at[0] + f64::from(r) * a.cos(),
                at[1] + f64::from(r) * a.sin(),
            );
            s.set_god_pose(
                me,
                GodPose {
                    pos: [x, 2000.0, z],
                    yaw: a,
                    pitch: 0.0,
                    speed: 0.0,
                },
                true,
            )
            .unwrap();
            if s.leave_god(me, GodLeave::Land { x, z }).is_ok() {
                landed = true;
                break 'find;
            }
        }
    }
    assert!(landed, "no free spot in the wood");
    assert!(s.off_road(me).is_some());
    s.apply_input(
        me,
        1,
        RiderInput {
            power_w: 150.0,
            ..Default::default()
        },
    );
    let ticks = (secs * 50.0) as usize;
    let (mut total, mut worst) = (0.0f64, 0.0f64);
    let mut seq = 2;
    for t in 0..ticks {
        if t % 100 == 0 {
            // Lean left and right in turn (around the trunks).
            let lean = if (t / 100) % 2 == 0 { 0.15 } else { -0.15 };
            s.set_lean(me, seq, lean);
            seq += 1;
        }
        let t0 = Instant::now();
        s.step();
        let dt = t0.elapsed().as_secs_f64() * 1e6;
        total += dt;
        worst = worst.max(dt);
    }
    let stats = s.collision_stats();
    (
        total / ticks as f64,
        worst,
        stats.crashes as usize,
        stats.contacts as usize,
    )
}

#[test]
#[ignore = "needs a world package: OCW_BENCH_WORLD"]
fn ticks_deep_in_a_wood() {
    let spec = std::env::var("OCW_BENCH_WORLD").expect("OCW_BENCH_WORLD=path to a package");
    let dir = tempfile::tempdir().unwrap();
    let trust = ocw_worldstore::trust::TrustConfig::for_build()
        .trust_store()
        .unwrap();
    let opts = ocw_worldstore::LoadOptions {
        allow_unsigned: true,
        ..ocw_worldstore::LoadOptions::new(
            trust,
            ocw_worldstore::WorldCache::new(dir.path().join("c")),
        )
    };
    let t0 = Instant::now();
    let (course, _) = ocw_server::worlds::load_course(&spec, None, &opts, false, 7).unwrap();
    println!("loaded in {:.1} s", t0.elapsed().as_secs_f64());
    let Course::FreeRide { net, start } = course else {
        panic!("a free ride")
    };
    let (at, n) = deep_in_a_wood(&net, 200.0).expect("a wood 200 m from the roads");
    println!("deep in a wood at {at:?}: {n} trunks in its 40 m cell");
    // Making one cell.
    let woods = net.woods().unwrap().clone();
    // What the server keeps for the trees (`tree_inputs`), roughly.
    let pkg = woods.package().unwrap();
    let heights: usize = pkg.tiles.values().map(|t| t.heights.len() * 4).sum();
    let roads: usize = pkg
        .tiles
        .values()
        .flat_map(|t| &t.roads)
        .map(|r| r.points.len() * 12 + 64)
        .sum();
    let lu: usize = pkg.landuse.as_ref().map_or(0, |l| {
        l.tiles
            .values()
            .map(|t| t.classes.len() + t.water.len() * 8)
            .sum()
    });
    let houses: usize = pkg.buildings.as_ref().map_or(0, |b| {
        b.values().flatten().map(|b| b.outline.len() * 8 + 96).sum()
    });
    let forest: usize = pkg.look.forest.as_ref().map_or(0, |f| {
        f.tiles
            .values()
            .map(|t| t.density.len() + t.broadleaf.len())
            .sum()
    });
    let fields: usize = pkg.look.vegetation.as_ref().map_or(0, |v| {
        v.tiles
            .values()
            .map(|t| t.crops.len() + t.rows.len() + t.trees.len() * 16)
            .sum()
    });
    println!(
        "the server's tree inputs, {} tiles: heights {:.1} MB, roads {:.1} MB, land use {:.1} MB, buildings {:.1} MB, forest {:.1} MB, fields {:.1} MB",
        pkg.tiles.len(),
        heights as f64 / 1e6,
        roads as f64 / 1e6,
        lu as f64 / 1e6,
        houses as f64 / 1e6,
        forest as f64 / 1e6,
        fields as f64 / 1e6
    );
    let t0 = Instant::now();
    let mut made = 0;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let c = WoodCell::containing(
                [at[0] + f64::from(dx) * 40.0, at[1] + f64::from(dz) * 40.0],
                woods.tile_size_m(),
            );
            made += woods
                .grid_trunks(c, &Stored::default())
                .map_or(0, |t| t.len());
        }
    }
    println!(
        "a cell made in {:.0} µs (25 cells, {made} trunks)",
        t0.elapsed().as_secs_f64() * 1e6 / 25.0
    );
    let key = ocw_world::TileKey::containing(at, f64::from(woods.tile_size_m()));
    let t0 = Instant::now();
    let planted = woods
        .planted_trunks(key, &Stored::default())
        .map_or(0, |t| t.len());
    println!(
        "the tile's planted trees ({planted}) in {:.0} µs",
        t0.elapsed().as_secs_f64() * 1e6
    );
    // Clear runs at a trunk (for the screenshots: `god.pose X Z 3 HEADING`,
    // `god off` lands there facing it).
    let trunks: Vec<ocw_world::woods::Trunk> = WoodCell::covering(
        [at[0] - 80.0, at[1] - 80.0],
        [at[0] + 80.0, at[1] + 80.0],
        woods.tile_size_m(),
    )
    .into_iter()
    .flat_map(|c| woods.grid_trunks(c, &Stored::default()).unwrap_or_default())
    .collect();
    for before in [3.0, 25.0] {
        'found: for t in &trunks {
            for k in 0..32 {
                let yaw = f64::from(k) * std::f64::consts::TAU / 32.0;
                let d = ocw_sim::god::forward(yaw);
                let p = [t.c[0] - d[0] * before, t.c[1] - d[1] * before];
                let clear = trunks.iter().filter(|u| u.id != t.id).all(|u| {
                    let rel = [u.c[0] - p[0], u.c[1] - p[1]];
                    let along = rel[0] * d[0] + rel[1] * d[1];
                    let beside = (rel[0] * d[1] - rel[1] * d[0]).abs();
                    along < -2.0 || along > before + 3.0 || beside > u.r + 1.2
                });
                if clear {
                    println!(
                        "a clear run of {before} m at the trunk at {:.1},{:.1} (r {:.2} m): god.pose {:.1} {:.1} 3 {:.1}",
                        t.c[0],
                        t.c[1],
                        t.r,
                        p[0],
                        p[1],
                        ocw_sim::god::heading_deg(yaw)
                    );
                    break 'found;
                }
            }
        }
    }
    let without = std::sync::Arc::new((*net).clone().with_woods(None));
    let (mean, worst, crashes, contacts) = ride(net, start, at, 60.0);
    println!(
        "with the trunks: {mean:.1} µs per tick (slowest {worst:.0} µs), {contacts} contacts, {crashes} crashes in 60 s"
    );
    let (mean0, worst0, _, contacts0) = ride(without, start, at, 60.0);
    println!("without them: {mean0:.1} µs per tick (slowest {worst0:.0} µs), {contacts0} contacts");
    assert!(mean < 1000.0, "a tick in a wood takes {mean} µs");
}
