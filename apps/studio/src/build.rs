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
//!
//! **The build recipe (16H Q17 (a)):** an official package ships
//! `<package>.recipe.toml` ([`ocw_world::recipe`], recorded in its
//! manifest): the steps of its official build (package, routes,
//! calibration, forest, vegetation), their arguments and the sources each
//! needs. When the sources it names are found – next to the package, in
//! a folder the user picks, or in `data/<region>/` – *Build package* runs
//! the whole chain with the patch ([`plan`]); else the basic step (the
//! recipe's package step, or the sources above) with a note of what is
//! missing. A package without a recipe (older ones, local builds without
//! the scripts) gets the basic step as before; a recipe changed since the
//! build (its SHA-256 no longer the manifest's) is not used.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use ocw_world::manifest::{PatchStatus, SignedManifest};
use ocw_world::recipe::{Recipe, recipe_path};

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

/// The build recipe next to `package` (16H Q17 (a)), if it can be used,
/// and a note when one is there but cannot: broken, not the one the
/// manifest records (changed since the build), or not recorded.
pub fn package_recipe(package: &Path, manifest: Option<&Path>) -> (Option<Recipe>, Option<String>) {
    let path = recipe_path(package);
    let Ok(bytes) = std::fs::read(&path) else {
        return (None, None);
    };
    let shown = crate::edit::shown_path(&path);
    let recipe = match Recipe::parse(&String::from_utf8_lossy(&bytes)) {
        Ok(r) => r,
        Err(e) => {
            return (
                None,
                Some(format!("the build recipe {shown} cannot be read: {e}")),
            );
        }
    };
    let Some(m) = manifest
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| SignedManifest::parse(&t).ok())
    else {
        // A local build without a manifest: nothing to check it against.
        return (Some(recipe), None);
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match m.manifest.recipe {
        Some(r) if r.path == name => match r.status(Some(&bytes)) {
            PatchStatus::Same => (Some(recipe), None),
            _ => (
                None,
                Some(format!(
                    "the build recipe {shown} changed since the package was built (not the one its manifest records): not used"
                )),
            ),
        },
        _ => (
            None,
            Some(format!(
                "the build recipe {shown} is not recorded in the package's manifest: not used"
            )),
        ),
    }
}

/// Where a recipe's sources are looked for, in order: next to the package,
/// the folder the user picked, `data/<region>/` (from the working folder).
pub fn source_dirs(package: &Path, region: Option<&str>, picked: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let beside = package
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let mut add = |p: PathBuf| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    add(beside);
    if let Some(p) = picked {
        add(p.to_path_buf());
    }
    if let Some(r) = region.filter(|r| !r.is_empty() && !r.contains(['/', '\\', '.'])) {
        add(Path::new("data").join(r));
    }
    out
}

/// What *Build package* runs.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildPlan {
    /// The `world-builder` command lines, in order.
    pub lines: Vec<Vec<String>>,
    /// The recipe's whole chain (else the basic step).
    pub full: bool,
    /// Why it is not the whole chain (what is missing), or a note on the
    /// recipe.
    pub note: Option<String>,
    /// Source files to check against the recipe's SHA-256 before the build
    /// (a difference is a warning: a newer download).
    pub checks: Vec<(PathBuf, String)>,
}

impl BuildPlan {
    /// The steps' names ("package, route, calibrate").
    pub fn steps(&self) -> String {
        self.lines
            .iter()
            .filter_map(|l| l.first().cloned())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A recipe source for people: "`imagery` (imagery for the calibration,
/// a folder; scripts/fetch-imagery.sh liechtenstein)".
fn describe_source(s: &ocw_world::recipe::Source) -> String {
    format!(
        "`{}` ({}{}{})",
        s.name,
        s.what,
        if s.dir { ", a folder" } else { "" },
        s.fetch
            .as_deref()
            .map(|f| format!("; {f}"))
            .unwrap_or_default()
    )
}

/// Plans the build of the package at `package` (its manifest beside it)
/// to `req.out` with `patch` (16H Q17 (a)): the recipe's whole chain when
/// its sources are found, else its package step when that step's are,
/// else the basic step from `req.sources`. `Err`: what is missing for any
/// build at all.
pub fn plan(
    package: &Path,
    manifest: Option<&Path>,
    facts: &PackageFacts,
    req: &crate::edit::BuildRequest,
    patch: &Path,
) -> Result<BuildPlan, Vec<String>> {
    let (recipe, recipe_note) = package_recipe(package, manifest);
    let patches = [patch.to_path_buf()];
    let mut note = recipe_note;
    if let Some(r) = recipe {
        let dirs = source_dirs(package, r.region.as_deref(), req.source_dir.as_deref());
        let found = r.find_sources(&dirs);
        let checks = |only: &Recipe| -> Vec<(PathBuf, String)> {
            only.sources
                .iter()
                .filter_map(|s| Some((found.get(&s.name)?.clone(), s.sha256.clone()?)))
                .collect()
        };
        let missing = r.missing(&found);
        if missing.is_empty() {
            let lines = r
                .commands(&found, &req.out, &patches)
                .map_err(|e| vec![e])?;
            return Ok(BuildPlan {
                lines,
                full: true,
                note: None,
                checks: checks(&r),
            });
        }
        let what: Vec<String> = missing.iter().map(|s| describe_source(s)).collect();
        let step = &r.steps[0];
        let package_ok = r.step_sources(step).all(|s| found.contains_key(&s.name));
        let left_out: Vec<&str> = r.steps[1..].iter().map(|s| s.step.as_str()).collect();
        let tail = if left_out.is_empty() {
            String::new()
        } else {
            format!(
                "; left out: {} (the steps of the official build that need them)",
                left_out.join(", ")
            )
        };
        note = Some(format!(
            "the full build of the recipe needs {} more source(s), not found next to the package, in the picked folder or in data/{}/: {}. Building the basic step only (the package with the patch){tail}.",
            missing.len(),
            r.region.as_deref().unwrap_or("<region>"),
            what.join("; ")
        ));
        if package_ok {
            let first = Recipe {
                steps: vec![step.clone()],
                ..r.clone()
            };
            let lines = first
                .commands(&found, &req.out, &patches)
                .map_err(|e| vec![e])?;
            return Ok(BuildPlan {
                lines,
                full: false,
                note,
                checks: checks(&first),
            });
        }
    }
    let missing = req.sources.missing();
    if !missing.is_empty() {
        let mut m = missing;
        if let Some(n) = note {
            m.push(n);
        }
        return Err(m);
    }
    Ok(BuildPlan {
        lines: vec![build_args(&req.sources, facts, patch, &req.out)],
        full: false,
        note,
        checks: Vec::new(),
    })
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
    /// The recipe's whole chain (16H Q17), else the basic step.
    pub full: bool,
    /// What the plan said (what is missing for the whole chain).
    pub note: Option<String>,
}

/// Runs `program args…` to its end, its output into `state`; the exit.
fn run_one(program: &Path, args: &[String], state: &Arc<Mutex<BuildState>>) -> Result<i32, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
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
    for h in handles {
        let _ = h.join();
    }
    child
        .wait()
        .map(|s| s.code().unwrap_or(-1))
        .map_err(|e| e.to_string())
}

/// The warning line of a source that is not the file the recipe names
/// (`None`: the same, or not readable – the build says so itself).
pub fn check_source(path: &Path, sha256: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let got = ocw_world::manifest::sha256(&bytes).to_string();
    (!got.eq_ignore_ascii_case(sha256)).then(|| {
        format!(
            "warning: {} is not the file the official build used (sha256 {got}, the recipe names {sha256}): a newer download? The result may differ from the official package.",
            crate::edit::shown_path(path)
        )
    })
}

impl BuildJob {
    /// Starts `program args…` (one step; the tests).
    #[cfg(test)]
    pub fn start(program: &Path, args: &[String], out: PathBuf) -> Self {
        Self::start_chain(program, &[args.to_vec()], &[], out)
    }

    /// Starts the steps `lines` one after the other (stopping at the first
    /// that fails), after checking the source files against `checks`.
    pub fn start_chain(
        program: &Path,
        lines: &[Vec<String>],
        checks: &[(PathBuf, String)],
        out: PathBuf,
    ) -> Self {
        let state = Arc::new(Mutex::new(BuildState::default()));
        let name = program.file_name().map_or_else(
            || program.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let command = lines
            .iter()
            .map(|a| format!("{name} {}", a.join(" ")))
            .collect::<Vec<_>>()
            .join(" && ");
        let job = Self {
            out,
            command,
            state: state.clone(),
            full: false,
            note: None,
        };
        let (program, lines, checks) = (program.to_path_buf(), lines.to_vec(), checks.to_vec());
        std::thread::spawn(move || {
            for (p, sha) in &checks {
                if let Some(w) = check_source(p, sha) {
                    state.lock().expect("build state").lines.push(w);
                }
            }
            let n = lines.len();
            let mut code = Ok(0);
            for (k, args) in lines.iter().enumerate() {
                if n > 1 {
                    state.lock().expect("build state").lines.push(format!(
                        "== step {}/{n}: {name} {} ==",
                        k + 1,
                        args.first().map_or("", String::as_str)
                    ));
                }
                code = run_one(&program, args, &state);
                if code != Ok(0) {
                    break;
                }
            }
            state.lock().expect("build state").done = Some(code);
        });
        job
    }

    /// " (full build from its recipe)" / " (basic step only)" (16H Q17).
    fn kind(&self) -> &'static str {
        if self.full {
            " (full build from its recipe)"
        } else if self.note.is_some() {
            " (basic step only)"
        } else {
            ""
        }
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
                "built {}{} – {missing} operation(s) MISSING (not applied)",
                crate::edit::shown_path(&self.out),
                self.kind()
            ),
            Some(Ok(0)) => format!(
                "built {}{}",
                crate::edit::shown_path(&self.out),
                self.kind()
            ),
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
        for _ in 0..500 {
            if !bad.running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(bad.summary().contains("could not start"));
    }

    fn wait(job: &BuildJob) {
        for _ in 0..1000 {
            if !job.running() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the build did not end");
    }

    /// The recipe's steps run one after the other and stop at the first
    /// that fails; a source that is not the recipe's file is a warning.
    #[test]
    fn a_chain_runs_its_steps_in_order_and_stops_at_a_failure() {
        let me = std::env::current_exe().unwrap();
        let d = dir();
        let src = d.join("li.osm.pbf");
        std::fs::write(&src, b"newer extract").unwrap();
        let other = ocw_world::manifest::sha256(b"official extract").to_string();
        let lines: Vec<Vec<String>> = vec![
            vec!["--list".into(), "the_report_is_picked".into()],
            vec!["--no-such-option-at-all".into()],
            vec!["--list".into(), "a_build_job_collects".into()],
        ];
        let job = BuildJob::start_chain(&me, &lines, &[(src.clone(), other)], "o.ocwpkg".into());
        assert_eq!(job.command.matches(" && ").count(), 2);
        wait(&job);
        let s = job.state.lock().unwrap();
        assert!(s.lines[0].starts_with("warning: ") && s.lines[0].contains("newer download"));
        assert!(s.lines.iter().any(|l| l.contains("step 1/3")));
        assert!(s.lines.iter().any(|l| l.contains("step 2/3")));
        assert!(
            !s.lines.iter().any(|l| l.contains("step 3/3")),
            "{:?}",
            s.lines
        );
        assert!(matches!(s.done, Some(Ok(c)) if c != 0));
        drop(s);
        // The same file: no warning.
        let same = ocw_world::manifest::sha256(b"newer extract").to_string();
        assert_eq!(check_source(&src, &same), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A package with its recipe and manifest (16H Q17 (a)), the recipe
    /// naming the OSM extract, the polygon, an SRTM tile, a DTM folder and
    /// the imagery; the package's facts.
    fn recipe_package(d: &Path) -> (PathBuf, PathBuf, PackageFacts) {
        use ocw_world::manifest::{Attribution, Manifest, PatchFile};
        let pkg = d.join("li.ocwpkg");
        std::fs::write(&pkg, b"pkg").unwrap();
        let p = |n: &str| d.join("official").join(n).display().to_string();
        let log: Vec<Vec<String>> = [
            vec![
                "package",
                "--pbf",
                &p("li.osm.pbf"),
                "--poly",
                &p("li.poly"),
                "--hgt",
                &p("N47E009.hgt"),
                "--dem",
                &p("swissalti3d"),
                "--name",
                "Liechtenstein",
                "--out",
                &pkg.display().to_string(),
            ],
            vec![
                "route",
                "--world",
                &pkg.display().to_string(),
                "--name",
                "Vaduz - Malbun",
                "--from",
                "47.1,9.5",
                "--to",
                "47.1,9.6",
            ],
            vec![
                "calibrate",
                "--world",
                &pkg.display().to_string(),
                "--imagery",
                &p("imagery"),
                "--pbf",
                &p("li.osm.pbf"),
            ],
        ]
        .into_iter()
        .map(|l| l.into_iter().map(String::from).collect())
        .collect();
        let mut r = Recipe::from_log("europe/liechtenstein", &pkg, &log, |path| {
            let n = path.file_name()?.to_string_lossy().into_owned();
            n.contains('.').then(|| {
                (
                    n.len() as u64,
                    ocw_world::manifest::sha256(n.as_bytes()).to_string(),
                )
            })
        });
        r.region = Some("no-such-region-here".into());
        let text = r.to_toml();
        std::fs::write(recipe_path(&pkg), &text).unwrap();
        let mut m = Manifest::new(
            "europe/liechtenstein",
            "Liechtenstein",
            1,
            0,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("li.ocwpkg", b"pkg").unwrap();
        m.recipe = Some(PatchFile::of("li.recipe.toml", text.as_bytes()).unwrap());
        let mp = d.join("li.ocwpkg.manifest.json");
        std::fs::write(&mp, SignedManifest::unsigned(&m).to_json()).unwrap();
        let facts = PackageFacts {
            name: "Liechtenstein".into(),
            id: "europe/liechtenstein".into(),
            origin: (47.1, 9.5),
            time_zone: None,
        };
        (pkg, mp, facts)
    }

    fn request(out: &Path, picked: Option<&Path>) -> crate::edit::BuildRequest {
        crate::edit::BuildRequest {
            sources: Sources::default(),
            out: out.to_path_buf(),
            source_dir: picked.map(Path::to_path_buf),
        }
    }

    /// Spec 16H Q17 (a): the whole chain when the recipe's sources are
    /// found (in a picked folder); with one missing the basic step from
    /// the recipe and a note naming it; without the package step's
    /// sources the old basic step or what is missing; an old package
    /// without a recipe as before; a changed recipe is not used.
    #[test]
    fn build_package_runs_the_recipe_when_its_sources_are_there() {
        let d = dir();
        let (pkg, mp, facts) = recipe_package(&d);
        let out = d.join("patched").join("li.ocwpkg");
        let patch = d.join("li.ocwpatch");
        let (r, note) = package_recipe(&pkg, Some(&mp));
        assert!(note.is_none(), "{note:?}");
        let r = r.expect("the recipe");
        assert_eq!(r.steps.len(), 3);
        // No sources anywhere: nothing to build (the basic sources are
        // named, and why the recipe could not run).
        let e = plan(&pkg, Some(&mp), &facts, &request(&out, None), &patch).unwrap_err();
        assert!(e.iter().any(|m| m.contains(".osm.pbf")), "{e:?}");
        assert!(e.iter().any(|m| m.contains("`imagery`")), "{e:?}");
        // All of them in a picked folder: the whole chain.
        let src = d.join("sources");
        std::fs::create_dir_all(src.join("swissalti3d")).unwrap();
        std::fs::create_dir_all(src.join("imagery")).unwrap();
        for f in ["li.osm.pbf", "li.poly", "N47E009.hgt"] {
            std::fs::write(src.join(f), f).unwrap();
        }
        let p = plan(&pkg, Some(&mp), &facts, &request(&out, Some(&src)), &patch).unwrap();
        assert!(p.full);
        assert_eq!(p.steps(), "package, route, calibrate");
        assert_eq!(p.note, None);
        let o = out.display().to_string();
        assert_eq!(
            p.lines[0].join(" "),
            format!(
                "package --pbf {s}/li.osm.pbf --poly {s}/li.poly --hgt {s}/N47E009.hgt --dem {s}/swissalti3d --name Liechtenstein --out {o} --patch {}",
                patch.display(),
                s = src.display()
            )
        );
        assert_eq!(
            p.lines[2][..3],
            ["calibrate".to_string(), "--world".into(), o.clone()]
        );
        // The files' checksums are checked (the folders have none).
        assert_eq!(p.checks.len(), 3);
        assert!(
            p.checks
                .iter()
                .all(|(f, sha)| check_source(f, sha).is_none())
        );
        // The imagery missing: the package step from the recipe, and why.
        std::fs::remove_dir(src.join("imagery")).unwrap();
        let p = plan(&pkg, Some(&mp), &facts, &request(&out, Some(&src)), &patch).unwrap();
        assert!(!p.full);
        assert_eq!(p.steps(), "package");
        let n = p.note.clone().unwrap();
        assert!(
            n.contains("`imagery` (imagery for the calibration, a folder"),
            "{n}"
        );
        assert!(n.contains("left out: route, calibrate"), "{n}");
        assert!(
            p.lines[0].iter().any(|a| a.ends_with("swissalti3d")),
            "the recipe's step"
        );
        // Next to the package is looked at first (no folder picked).
        for f in ["li.osm.pbf", "li.poly", "N47E009.hgt"] {
            std::fs::copy(src.join(f), d.join(f)).unwrap();
        }
        std::fs::create_dir_all(d.join("swissalti3d")).unwrap();
        std::fs::create_dir_all(d.join("imagery")).unwrap();
        let p = plan(&pkg, Some(&mp), &facts, &request(&out, None), &patch).unwrap();
        assert!(p.full, "{:?}", p.note);
        assert!(p.lines[0][2].starts_with(&d.display().to_string()));
        // Changed after the build: not used – the basic step with a note.
        std::fs::write(recipe_path(&pkg), r.to_toml() + "# edited\n").unwrap();
        let (none, note) = package_recipe(&pkg, Some(&mp));
        assert!(none.is_none() && note.unwrap().contains("changed since"));
        let basic = basic_request(&d);
        let p = plan(&pkg, Some(&mp), &facts, &basic, &patch).unwrap();
        assert!(!p.full && p.note.unwrap().contains("changed since"));
        assert_eq!(p.lines[0][0], "package");
        // An old package without a recipe: the basic step as before.
        std::fs::remove_file(recipe_path(&pkg)).unwrap();
        assert_eq!(package_recipe(&pkg, Some(&mp)), (None, None));
        let p = plan(&pkg, Some(&mp), &facts, &basic, &patch).unwrap();
        assert_eq!(
            p.lines,
            vec![build_args(&basic.sources, &facts, &patch, &out)]
        );
        assert!(!p.full && p.note.is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The basic step's request with the sources found next to the package.
    fn basic_request(d: &Path) -> crate::edit::BuildRequest {
        crate::edit::BuildRequest {
            sources: find_sources(&d.join("li.ocwpkg")),
            out: d.join("patched").join("li.ocwpkg"),
            source_dir: None,
        }
    }

    #[test]
    fn the_source_folders_and_their_order() {
        let d = dir();
        let pkg = d.join("w").join("li.ocwpkg");
        std::fs::create_dir_all(d.join("w")).unwrap();
        let picked = d.join("picked");
        std::fs::create_dir_all(&picked).unwrap();
        assert_eq!(
            source_dirs(&pkg, Some("../etc"), Some(&picked)),
            vec![d.join("w"), picked.clone()],
            "no folder outside data/ by a region name"
        );
        assert_eq!(
            source_dirs(&pkg, None, Some(&d.join("gone"))),
            vec![d.join("w")]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
