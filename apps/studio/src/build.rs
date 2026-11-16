//! *Build package* (spec §14 Phase 16, 16C *Preview and build*, E5 step
//! 2): runs `world-builder package … --patch FILE` as a child process on
//! this computer (as for users; our own world builds stay on the build
//! machines) – only when the package's source data are there, else the
//! studio says what is needed. Its output is shown as it comes, and the
//! per-operation report (`ok` / `MISSING` / `?`) is picked out of it.
//!
//! The sources are looked for next to the package, under the names the
//! build scripts use (`<name>.osm.pbf`, `<name>.poly`, the `.hgt` tiles,
//! a DTM folder); each can be given in the Build dialog instead.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// The source data of a package build (`world-builder package`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub pbf: Vec<PathBuf>,
    pub poly: Vec<PathBuf>,
    pub hgt: Vec<PathBuf>,
    /// DTM folders or GeoTIFFs (optional; best first).
    pub dem: Vec<PathBuf>,
}

/// DTM folders the build scripts use next to the package.
pub const DEM_DIRS: [&str; 4] = ["swissalti3d", "dgm1", "mdt05", "dem"];

/// Looks for the sources of `package` in its folder: `<stem>.osm.pbf`
/// and `<stem>.poly` (the package's file name without `.ocwpkg`), every
/// `.hgt` tile there, and a DTM folder of [`DEM_DIRS`].
pub fn find_sources(package: &Path) -> Sources {
    let dir = package.parent().unwrap_or(Path::new("."));
    let stem = package
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut s = Sources::default();
    let pbf = dir.join(format!("{stem}.osm.pbf"));
    if pbf.is_file() {
        s.pbf.push(pbf);
    }
    let poly = dir.join(format!("{stem}.poly"));
    if poly.is_file() {
        s.poly.push(poly);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut hgt: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("hgt")))
            .collect();
        hgt.sort();
        s.hgt = hgt;
    }
    for d in DEM_DIRS {
        let p = dir.join(d);
        if p.is_dir() {
            s.dem.push(p);
        }
    }
    s
}

impl Sources {
    /// What is missing for a build, for people (empty: it can start).
    pub fn missing(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.pbf.is_empty() {
            out.push(
                "the OSM extract the package was built from (`<name>.osm.pbf`, e.g. from Geofabrik; `scripts/fetch-pilot.sh` for the pilot)"
                    .into(),
            );
        }
        if self.poly.is_empty() {
            out.push("the region boundary (`<name>.poly`, an Osmosis / Geofabrik polygon)".into());
        }
        if self.hgt.is_empty() && self.dem.is_empty() {
            out.push(
                "elevation data: SRTM tiles (`.hgt`) and / or a DTM folder (GeoTIFF, `world-builder dem-import`)"
                    .into(),
            );
        }
        for p in self.pbf.iter().chain(&self.poly).chain(&self.hgt) {
            if !p.is_file() {
                out.push(format!("{} (not found)", crate::edit::shown_path(p)));
            }
        }
        out
    }

    /// Paths from a field of the Build dialog: one per line or separated
    /// by `;`.
    pub fn parse_list(text: &str) -> Vec<PathBuf> {
        text.split(['\n', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    /// A list for a Build dialog field.
    pub fn list_text(paths: &[PathBuf]) -> String {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// What a rebuild of the package keeps from it.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageFacts {
    pub name: String,
    pub id: String,
    /// The projection origin (lat, lon): the same metres and tiles.
    pub origin: (f64, f64),
    pub time_zone: Option<String>,
}

/// The `world-builder` arguments of a build of the patched package.
pub fn build_args(src: &Sources, facts: &PackageFacts, patch: &Path, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec!["package".into()];
    let mut push = |flag: &str, paths: &[PathBuf]| {
        for p in paths {
            a.push(flag.into());
            a.push(p.display().to_string());
        }
    };
    push("--pbf", &src.pbf);
    push("--poly", &src.poly);
    push("--hgt", &src.hgt);
    push("--dem", &src.dem);
    a.extend([
        "--name".into(),
        facts.name.clone(),
        "--id".into(),
        facts.id.clone(),
        "--origin".into(),
        format!("{},{}", facts.origin.0, facts.origin.1),
    ]);
    if let Some(tz) = &facts.time_zone {
        a.extend(["--time-zone".into(), tz.clone()]);
    }
    a.extend([
        "--out".into(),
        out.display().to_string(),
        "--patch".into(),
        patch.display().to_string(),
    ]);
    a
}

/// Where the patched package goes by default: `patched/<file>` next to
/// the package (the original, signed package is never overwritten).
pub fn default_output(package: &Path) -> PathBuf {
    let dir = package.parent().unwrap_or(Path::new("."));
    dir.join("patched").join(
        package
            .file_name()
            .unwrap_or_else(|| "world.ocwpkg".as_ref()),
    )
}

/// The `world-builder` program: `$OCW_WORLD_BUILDER`, else next to this
/// program, else on the `PATH`.
pub fn world_builder() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OCW_WORLD_BUILDER") {
        return Some(PathBuf::from(p));
    }
    let name = format!("world-builder{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(&name)))
        .filter(|p| p.is_file());
    beside.or_else(|| {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|d| d.join(&name))
                .find(|p| p.is_file())
        })
    })
}

/// The lines of a build's patch report (`ok`, `MISSING`, `?`, the
/// summary and the warning).
pub fn report_lines(output: &[String]) -> Vec<String> {
    output
        .iter()
        .filter(|l| {
            l.starts_with("  ok ")
                || l.starts_with("  MISSING ")
                || l.starts_with("  ? ")
                || l.starts_with("patch:")
                || l.starts_with("patch ")
        })
        .cloned()
        .collect()
}

/// A build running (or done).
#[derive(Debug, Default)]
pub struct BuildState {
    /// The output so far, stdout and stderr as they come.
    pub lines: Vec<String>,
    /// The exit: `Ok(code)`, or why it could not start.
    pub done: Option<Result<i32, String>>,
}

/// A build of the patched package, its output shared with its threads.
#[derive(Debug, Clone)]
pub struct BuildJob {
    pub out: PathBuf,
    pub command: String,
    pub state: Arc<Mutex<BuildState>>,
}

impl BuildJob {
    /// Starts `program args…`; the output is collected by two threads.
    pub fn start(program: &Path, args: &[String], out: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(BuildState::default()));
        let command = format!(
            "{} {}",
            program.file_name().map_or_else(
                || program.display().to_string(),
                |n| n.to_string_lossy().into_owned()
            ),
            args.join(" ")
        );
        let job = Self {
            out,
            command,
            state: state.clone(),
        };
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                state.lock().expect("build state").done =
                    Some(Err(format!("{}: {e}", program.display())));
                return job;
            }
        };
        let readers: Vec<Box<dyn std::io::Read + Send>> = vec![
            Box::new(child.stdout.take().expect("piped")),
            Box::new(child.stderr.take().expect("piped")),
        ];
        let handles: Vec<_> = readers
            .into_iter()
            .map(|r| {
                let state = state.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(r).lines().map_while(Result::ok) {
                        state.lock().expect("build state").lines.push(line);
                    }
                })
            })
            .collect();
        std::thread::spawn(move || {
            for h in handles {
                let _ = h.join();
            }
            let code = match child.wait() {
                Ok(s) => Ok(s.code().unwrap_or(-1)),
                Err(e) => Err(e.to_string()),
            };
            state.lock().expect("build state").done = Some(code);
        });
        job
    }

    pub fn running(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.done.is_none())
    }

    /// The status line of the build.
    pub fn summary(&self) -> String {
        let s = self.state.lock().expect("build state");
        let missing = report_lines(&s.lines)
            .iter()
            .filter(|l| l.starts_with("  MISSING "))
            .count();
        match &s.done {
            None => format!("building… ({} lines)", s.lines.len()),
            Some(Ok(0)) if missing > 0 => format!(
                "built {} – {missing} operation(s) MISSING (not applied)",
                crate::edit::shown_path(&self.out)
            ),
            Some(Ok(0)) => format!("built {}", crate::edit::shown_path(&self.out)),
            Some(Ok(c)) => format!("the build failed (exit {c})"),
            Some(Err(e)) => format!("the build could not start: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ocw-studio-build-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_sources_are_found_next_to_the_package_or_explained() {
        let d = dir();
        let pkg = d.join("testland.ocwpkg");
        let s = find_sources(&pkg);
        assert_eq!(s, Sources::default());
        let m = s.missing();
        assert_eq!(m.len(), 3, "{m:?}");
        assert!(m[0].contains(".osm.pbf") && m[1].contains(".poly") && m[2].contains(".hgt"));
        for f in [
            "testland.osm.pbf",
            "testland.poly",
            "N47E009.hgt",
            "N46E009.hgt",
            "other.poly",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        std::fs::create_dir(d.join("swissalti3d")).unwrap();
        let s = find_sources(&pkg);
        assert_eq!(s.pbf, vec![d.join("testland.osm.pbf")]);
        assert_eq!(
            s.poly,
            vec![d.join("testland.poly")],
            "only the package's own"
        );
        assert_eq!(s.hgt, vec![d.join("N46E009.hgt"), d.join("N47E009.hgt")]);
        assert_eq!(s.dem, vec![d.join("swissalti3d")]);
        assert!(s.missing().is_empty());
        let mut gone = s.clone();
        gone.pbf = vec![d.join("nope.osm.pbf")];
        assert!(gone.missing()[0].contains("not found"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_build_command_keeps_the_package_facts_and_adds_the_patch() {
        let src = Sources {
            pbf: vec!["d/t.osm.pbf".into()],
            poly: vec!["d/t.poly".into()],
            hgt: vec!["d/a.hgt".into(), "d/b.hgt".into()],
            dem: vec![],
        };
        let facts = PackageFacts {
            name: "Testland".into(),
            id: "europe/testland".into(),
            origin: (47.1, 9.5),
            time_zone: Some("Europe/Vaduz".into()),
        };
        let a = build_args(
            &src,
            &facts,
            Path::new("d/t.ocwpatch"),
            Path::new("d/patched/t.ocwpkg"),
        );
        assert_eq!(
            a.join(" "),
            "package --pbf d/t.osm.pbf --poly d/t.poly --hgt d/a.hgt --hgt d/b.hgt --name Testland --id europe/testland --origin 47.1,9.5 --time-zone Europe/Vaduz --out d/patched/t.ocwpkg --patch d/t.ocwpatch"
        );
        assert_eq!(
            default_output(Path::new("/w/t.ocwpkg")),
            PathBuf::from("/w/patched/t.ocwpkg")
        );
        assert_eq!(
            Sources::parse_list(" a.hgt ; b.hgt\nc.hgt;"),
            vec![PathBuf::from("a.hgt"), "b.hgt".into(), "c.hgt".into()]
        );
        assert_eq!(Sources::list_text(&src.hgt), "d/a.hgt; d/b.hgt");
    }

    #[test]
    fn the_report_is_picked_out_of_the_output() {
        let out: Vec<String> = [
            "reading OSM",
            "patch t.ocwpatch: 2 operations for europe/testland",
            "  ok       t.ocwpatch:5 road edge 1: 1 edge(s)",
            "  MISSING  t.ocwpatch:9 remove_road way 4: way 4 is not in the road graph",
            "meshing",
            "patch: 2 operations, 1 applied, 1 missing (target gone, not applied), 0 unchecked",
        ]
        .map(String::from)
        .to_vec();
        let r = report_lines(&out);
        assert_eq!(r.len(), 4);
        assert!(r[2].contains("MISSING"));
    }

    /// The child process's output and exit arrive (a program every test
    /// machine has: this test binary itself, asked for its test list).
    #[test]
    fn a_build_job_collects_the_output_of_its_process() {
        let me = std::env::current_exe().unwrap();
        let job = BuildJob::start(
            &me,
            &["--list".into(), "the_report_is_picked".into()],
            "out.ocwpkg".into(),
        );
        for _ in 0..500 {
            if !job.running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let s = job.state.lock().unwrap();
        assert_eq!(s.done, Some(Ok(0)));
        assert!(
            s.lines.iter().any(|l| l.contains("the_report_is_picked")),
            "{:?}",
            s.lines
        );
        drop(s);
        assert!(job.summary().starts_with("built"));
        let bad = BuildJob::start(Path::new("/nonexistent/world-builder"), &[], "x".into());
        assert!(bad.summary().contains("could not start"));
    }
}
