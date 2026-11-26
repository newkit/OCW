//! What the Ride and Navi pages list (spec §3.2.1, M3), Bevy-free and read
//! from directories given by the caller (fixture directories in tests):
//! the user's routes (`routes/*.ocwroute` next to the package, where OCW
//! Studio saves them, and `<data>/ocw/routes/`: imported GPX files and the
//! studio's routes without a writable package folder; spec 16H Q19), GPX files to import (`<data>/ocw/gpx/`, the
//! downloads folder) and the saved rides (`rides/*.fit` with their
//! sidecar and Strava sync state). The worlds of World → Change world come
//! from `ocw_worldstore::discover`.

use std::path::{Path, PathBuf};

use crate::strava::sync::{RideSync, SyncState};

/// Where a route of the user was found (spec 16H Q19 (a)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    /// `<package folder>/routes/`: where OCW Studio's *Save route* writes.
    NextToPackage,
    /// `<data dir>/ocw/routes/`: imported GPX files, and the studio's
    /// routes when the package folder is not writable.
    DataDir,
}

/// A route of the user (an `ocw-graph-route` file: `.ocwroute` from OCW
/// Studio, `.ocw-graph-route` from a GPX import).
#[derive(Debug, Clone, PartialEq)]
pub struct UserRoute {
    pub path: PathBuf,
    pub name: String,
    pub length_m: f64,
    pub source: RouteSource,
}

/// A route file that could not be offered: unreadable, not a route, or
/// not on the loaded package's roads.
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedRoute {
    pub path: PathBuf,
    pub why: String,
}

/// The user's routes for the loaded package and the files skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteScan {
    pub routes: Vec<UserRoute>,
    pub skipped: Vec<SkippedRoute>,
}

/// Route files: `.ocwroute` (OCW Studio's *Save route*) and
/// `.ocw-graph-route` (the game's GPX import).
fn is_route_file(p: &Path) -> bool {
    p.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("ocwroute") || x == "ocw-graph-route")
}

/// The folders the game reads the user's routes from, in this order:
/// `routes/` next to the package file, then `<data dir>/ocw/routes/`.
pub fn route_dirs(
    package_file: Option<&Path>,
    data_routes: Option<&Path>,
) -> Vec<(PathBuf, RouteSource)> {
    let mut out = Vec::new();
    if let Some(dir) = package_file.and_then(Path::parent) {
        out.push((dir.join("routes"), RouteSource::NextToPackage));
    }
    if let Some(d) = data_routes {
        out.push((d.to_path_buf(), RouteSource::DataDir));
    }
    out
}

/// The route files in `dirs` made for the package `package` (by the name
/// in the file; files without one fit any package), sorted by name. A file
/// made for another package is left out quietly (it belongs to another
/// world, e.g. a sibling package in the same folder); an unreadable or
/// broken file, or one whose roads `net` does not have, is skipped with a
/// note. A folder listed twice (the same path) is read once.
pub fn scan_user_routes(
    dirs: &[(PathBuf, RouteSource)],
    package: &str,
    net: Option<&ocw_world::RoadNetwork>,
) -> RouteScan {
    let same = |a: &str, b: &str| a.replace('_', " ").trim() == b.replace('_', " ").trim();
    let mut scan = RouteScan::default();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (dir, source) in dirs {
        let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_route_file(p))
            .collect();
        files.sort();
        for p in files {
            let why = match read_user_route(&p, package, net, &same) {
                Ok(Some(route)) => {
                    scan.routes.push(UserRoute {
                        length_m: super::setup::graph_route_length_m(&route),
                        name: route.name,
                        path: p,
                        source: *source,
                    });
                    continue;
                }
                Ok(None) => continue,
                Err(why) => why,
            };
            eprintln!("route file {} skipped: {why}", p.display());
            scan.skipped.push(SkippedRoute { path: p, why });
        }
    }
    scan.routes
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    scan
}

/// One route file: the route (`None`: made for another package) or why it
/// is skipped.
fn read_user_route(
    p: &Path,
    package: &str,
    net: Option<&ocw_world::RoadNetwork>,
    same: &dyn Fn(&str, &str) -> bool,
) -> Result<Option<ocw_world::GraphRoute>, String> {
    let bytes = std::fs::read(p).map_err(|e| format!("cannot be read: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let (route, made_for) =
        ocw_world::GraphRoute::parse(&text).map_err(|e| format!("not a route: {e}"))?;
    if let Some((name, _)) = &made_for
        && !same(name, package)
    {
        return Ok(None);
    }
    if let Some(net) = net {
        route
            .resolve(net)
            .map_err(|e| format!("not on these roads: {e}"))?;
    }
    Ok(Some(route))
}

/// The route files in `dir` made for the package `package` (see
/// [`scan_user_routes`]; no road check).
#[cfg(test)]
pub fn user_routes(dir: &Path, package: &str) -> Vec<UserRoute> {
    scan_user_routes(&[(dir.to_path_buf(), RouteSource::DataDir)], package, None).routes
}

/// A file name for a route called `name` in `dir` that does not exist yet.
pub fn route_file_name(dir: &Path, name: &str) -> PathBuf {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let stem = if stem.is_empty() {
        "route".into()
    } else {
        stem
    };
    let mut path = dir.join(format!("{stem}.ocw-graph-route"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("{stem}-{n}.ocw-graph-route"));
    }
    path
}

/// The `.gpx` files in `dirs` (not recursive), newest first.
pub fn gpx_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gpx")))
        .map(|p| {
            let t = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (t, p)
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.dedup_by(|a, b| a.1 == b.1);
    out.into_iter().map(|(_, p)| p).collect()
}

/// A saved ride for the Rides list.
#[derive(Debug, Clone, PartialEq)]
pub struct RideEntry {
    pub file: String,
    /// "2026-11-11 12:00" (UTC, from the file name).
    pub date: String,
    /// The route or world, from the sidecar.
    pub route: String,
    /// "uploaded", "processing", "failed", "rejected", "not uploaded" or
    /// (a whole ride: never uploaded, spec 2.8.1 Q6) "not for Strava".
    pub strava: &'static str,
}

/// The saved rides in `dir`, newest first, at most `limit`.
pub fn recent_rides(dir: &Path, sync: &SyncState, limit: usize) -> Vec<RideEntry> {
    let mut files = crate::strava::sync::ride_files(dir);
    files.reverse();
    files
        .into_iter()
        .take(limit)
        .map(|file| {
            let meta = crate::recording::load_meta(&dir.join(&file));
            let stem = file.trim_end_matches(".fit");
            let date = match stem.split_once('_') {
                Some((d, t)) => format!("{d} {}", t.get(..5).unwrap_or(t).replace('-', ":")),
                None => stem.to_string(),
            };
            let strava = match sync.rides.get(&file) {
                // Whole rides never go to Strava (spec 2.8.1 Q6).
                None if !file.starts_with("strava/") => "not for Strava",
                None => "not uploaded",
                Some(RideSync::Waiting { .. }) => "waiting for set-up",
                Some(RideSync::Processing { .. }) => "processing",
                Some(RideSync::Uploaded { .. }) => "uploaded",
                Some(RideSync::Failed { .. }) => "failed",
                Some(RideSync::Rejected { .. }) => "rejected",
            };
            RideEntry {
                route: meta.map_or_else(|| "-".into(), |m| m.route),
                file,
                date,
                strava,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ocw-catalog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn user_routes_are_those_of_the_package() {
        let d = temp_dir("routes");
        let route = |name: &str| ocw_world::GraphRoute {
            name: name.into(),
            steps: vec![ocw_world::RouteStep::Edge {
                id: ocw_world::graph::EdgeId(4),
                from_m: 0.0,
                to_m: 1500.0,
            }],
            is_loop: false,
        };
        std::fs::write(
            d.join("a.ocw-graph-route"),
            route("Up the valley").to_text(Some(("Example Land", 7))),
        )
        .unwrap();
        std::fs::write(
            d.join("b.ocw-graph-route"),
            route("Elsewhere").to_text(Some(("Other", 7))),
        )
        .unwrap();
        std::fs::write(d.join("c.ocw-graph-route"), route("Anywhere").to_text(None)).unwrap();
        std::fs::write(d.join("broken.ocw-graph-route"), "nonsense").unwrap();
        let r = user_routes(&d, "Example Land");
        let names: Vec<&str> = r.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Anywhere", "Up the valley"]);
        assert_eq!(r[1].length_m, 1500.0);
        // New file names never overwrite.
        assert_eq!(
            route_file_name(&d, "a"),
            d.join("a-2.ocw-graph-route"),
            "a.ocw-graph-route exists"
        );
        assert_eq!(
            route_file_name(&d, "Tour: Rhein/Valley"),
            d.join("Tour__Rhein_Valley.ocw-graph-route")
        );
    }

    #[test]
    fn gpx_files_are_listed_newest_first() {
        let d = temp_dir("gpx");
        std::fs::write(d.join("old.gpx"), "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.join("new.GPX"), "x").unwrap();
        std::fs::write(d.join("x.fit"), "x").unwrap();
        let f = gpx_files(&[d.clone(), d.join("missing")]);
        assert_eq!(f, [d.join("new.GPX"), d.join("old.gpx")]);
    }

    #[test]
    fn rides_list_shows_the_sidecar_and_strava_state() {
        let d = temp_dir("rides");
        for stem in ["2026-11-10_08-00-00", "2026-11-11_17-30-05"] {
            std::fs::write(d.join(format!("{stem}.fit")), b"fit").unwrap();
        }
        std::fs::write(
            d.join("2026-11-11_17-30-05.toml"),
            "name = \"Evening\"\ndescription = \"\"\nroute = \"Hills\"\n",
        )
        .unwrap();
        let mut sync = SyncState::default();
        sync.rides.insert(
            "2026-11-10_08-00-00.fit".into(),
            RideSync::Uploaded {
                activity_id: Some(1),
                at: 0,
            },
        );
        let r = recent_rides(&d, &sync, 10);
        assert_eq!(
            r,
            [
                RideEntry {
                    file: "2026-11-11_17-30-05.fit".into(),
                    date: "2026-11-11 17:30".into(),
                    route: "Hills".into(),
                    strava: "not for Strava",
                },
                RideEntry {
                    file: "2026-11-10_08-00-00.fit".into(),
                    date: "2026-11-10 08:00".into(),
                    route: "-".into(),
                    strava: "uploaded",
                },
            ]
        );
        assert_eq!(recent_rides(&d, &sync, 1).len(), 1);
    }
}
