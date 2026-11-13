//! The Steg tunnel on the Vaduz-Malbun route file in the Liechtenstein
//! package (spec §4.3, §3.2 tunnels), from the real pilot data.
//!
//! Regression (user, 2026-11-13, kiosk at 08f72b9): the kiosk rode a route
//! file built from swissALTI3D in a package rebuilt from SRTM by the
//! deploy. The route ran up to 15 m below the drawn road on the climb to
//! the tunnel, so the rider sank into the road, and the tunnel finder saw
//! one 1.7 km tunnel from km 8.2 (squares of terrain and a broken ceiling
//! where the hillside cut through the tube) instead of km 8.75-9.4. The
//! desktop fits a route file's heights onto the package roads
//! (`ocw_world::route_fit`, as `ride::setup::route_file_on_package`); this
//! checks the result: the avatar stands on the road surface before and at
//! the portal, and the tunnel is found where it is.
//!
//! Needs `data/pilot/liechtenstein.ocwpkg` and `vaduz-malbun.route`
//! (`OCW_PILOT_DIR` overrides the folder); skipped with a message without.

use std::path::PathBuf;

use ocw_render::riders::avatar_transform;
use ocw_render::tunnel::covered_stretches;
use ocw_world::package::ROAD_LIFT_M;
use ocw_world::route_fit::{FIT_TOLERANCE_M, fit_heights};
use ocw_world::{Route, RouteFile, WorldPackage};

fn data() -> Option<(WorldPackage, RouteFile)> {
    let dir = std::env::var("OCW_PILOT_DIR").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/pilot"),
        PathBuf::from,
    );
    let pkg = std::fs::read(dir.join("liechtenstein.ocwpkg")).ok()?;
    let route = std::fs::read_to_string(dir.join("vaduz-malbun.route")).ok()?;
    eprintln!("pilot data from {}", dir.display());
    Some((
        WorldPackage::from_bytes(&pkg).expect("readable package"),
        RouteFile::parse(&route).expect("readable route file"),
    ))
}

/// The route as the desktop rides it in the package.
fn ridden(pkg: &WorldPackage, mut file: RouteFile) -> Route {
    let fit = fit_heights(&mut file.points, |x, z, d| pkg.road_heights_along(x, z, d));
    eprintln!(
        "route file vs package roads: worst {:.2} m, mean {:+.2} m over {} points, fitted: {}",
        fit.worst_m, fit.mean_m, fit.on_roads, fit.fitted
    );
    file.to_route().expect("route")
}

/// Height of the drawn road surface under `p` nearest to `y`.
fn road_surface(pkg: &WorldPackage, p: [f64; 3]) -> Option<f64> {
    pkg.road_heights(p[0], p[2])
        .into_iter()
        .map(|h| h + ROAD_LIFT_M)
        .min_by(|a, b| (a - p[1]).abs().total_cmp(&(b - p[1]).abs()))
}

#[test]
fn the_rider_stands_on_the_road_before_the_steg_tunnel() {
    let Some((pkg, file)) = data() else {
        eprintln!("skipped: no pilot package and route file in data/pilot");
        return;
    };
    let route = ridden(&pkg, file);
    let (mut samples, mut worst, mut at) = (0, 0.0f64, 0.0);
    let mut d = 8400.0;
    while d <= 8770.0 {
        let state = ocw_sim::RiderState {
            distance: d,
            ..Default::default()
        };
        let y = f64::from(
            avatar_transform(
                &route,
                &state,
                None,
                &ocw_assets::shipped::bike_rig(ocw_core::ProfileId::Road),
            )
            .translation
            .y,
        );
        let p = route.sample(d).position;
        if let Some(road) = road_surface(&pkg, p) {
            samples += 1;
            // Below the drawn surface: the wheels are in the road.
            if road - y > worst {
                (worst, at) = (road - y, d);
            }
        }
        d += 1.0;
    }
    if std::env::var("OCW_PILOT_DEBUG").is_ok() {
        for k in -8..=8 {
            let s = route.sample(at + f64::from(k));
            let q = s.position;
            let dir = [s.direction[0], s.direction[1]];
            eprintln!(
                "  d {:+} route {:.2} at ({:.1}, {:.1}) strips {:?} along {:?}",
                k,
                q[1],
                q[0],
                q[2],
                pkg.road_heights(q[0], q[2]),
                pkg.road_heights_along(q[0], q[2], Some(dir))
            );
        }
    }
    let p = route.sample(at).position;
    eprintln!(
        "km 8.40-8.77: {samples} samples on roads, deepest {worst:.3} m below the surface at km {:.3} (route {:.2} m, strips {:?})",
        at / 1000.0,
        p[1],
        pkg.road_heights(p[0], p[2])
    );
    assert!(samples > 300, "only {samples} samples on package roads");
    assert!(
        worst <= FIT_TOLERANCE_M,
        "the rider is {worst:.2} m below the road surface before the tunnel"
    );
}

#[test]
fn the_steg_tunnel_is_found_where_it_is() {
    let Some((pkg, file)) = data() else {
        eprintln!("skipped: no pilot package and route file in data/pilot");
        return;
    };
    let route = ridden(&pkg, file);
    let ground = |x: f64, z: f64| pkg.surface_height(x, z);
    let tunnels = covered_stretches(&route, &ground, 8000.0, 10_500.0);
    eprintln!(
        "tunnels km 8-10.5: {:?}",
        tunnels
            .iter()
            .map(|(a, b)| format!("{:.3}-{:.3}", a / 1000.0, b / 1000.0))
            .collect::<Vec<_>>()
    );
    // The Steg tunnel: about km 8.80-9.52 with its portals (consistent
    // data, 2026-11-13), nothing else; the mismatched data gave 8.21-9.92.
    assert_eq!(tunnels.len(), 1, "{tunnels:?}");
    let (a, b) = tunnels[0];
    assert!(
        (8700.0..=8850.0).contains(&a),
        "entry at km {:.3}",
        a / 1000.0
    );
    assert!(
        (9400.0..=9600.0).contains(&b),
        "exit at km {:.3}",
        b / 1000.0
    );
}
