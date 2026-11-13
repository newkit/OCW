//! What the Ride and Navi pages list (spec §3.2.1, M3), Bevy-free and read
//! from directories given by the caller (fixture directories in tests):
//! the user's routes (`<data>/ocw/routes/*.ocw-graph-route`,
//! e.g. imported GPX files), GPX files to import (`<data>/ocw/gpx/`, the
//! downloads folder) and the saved rides (`rides/*.fit` with their
//! sidecar and Strava sync state). The worlds of World → Change world come
//! from `ocw_worldstore::discover`.

use std::path::{Path, PathBuf};

use crate::strava::sync::{RideSync, SyncState};

/// A route of the user (an `ocw-graph-route` file).
#[derive(Debug, Clone, PartialEq)]
pub struct UserRoute {
    pub path: PathBuf,
    pub name: String,
    pub length_m: f64,
}

/// The route files in `dir` made for the package `package` (by the name
/// in the file; files without one fit any package), sorted by name.
pub fn user_routes(dir: &Path, package: &str) -> Vec<UserRoute> {
    let mut out: Vec<UserRoute> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ocw-graph-route"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let (route, made_for) = ocw_world::GraphRoute::parse(&text).ok()?;
            if let Some((name, _)) = made_for
                && name.replace('_', " ") != package.replace('_', " ")
            {
                return None;
            }
            Some(UserRoute {
                length_m: super::setup::graph_route_length_m(&route),
                name: route.name,
                path: p,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    out
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
    /// "uploaded", "processing", "failed", "rejected" or "not uploaded".
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
                None => "not uploaded",
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
                    strava: "not uploaded",
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
