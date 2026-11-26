//! `world-builder recipe …` (spec 14 Phase 16, 16H Q17 (a)): the build
//! recipe that ships with an official package (`<package>.recipe.toml`,
//! [`ocw_world::recipe`]).
//!
//! The build scripts set `OCW_RECIPE_LOG=FILE` while they build: every
//! `world-builder` run then appends its command line to the file
//! ([`log_invocation`]); `recipe write --world PKG --log FILE` keeps the
//! steps that made or changed that package, names their source data
//! (with size and SHA-256 of each file), writes the recipe next to the
//! package and records it in the manifest (rewritten unsigned, as every
//! step after the package does: the scripts sign afterwards).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use ocw_world::manifest::{Manifest, PatchFile, PatchStatus, SignedManifest};
use ocw_world::recipe::{Recipe, recipe_path};

use crate::dist::sidecar;

/// The environment variable naming the log of a build's command lines.
pub const LOG_ENV: &str = "OCW_RECIPE_LOG";

/// Separates the arguments of one logged command line.
const SEP: char = '\u{1f}';

#[derive(Subcommand, Debug)]
pub enum RecipeCmd {
    /// Write `<package>.recipe.toml` from a build's log of command lines
    /// (`OCW_RECIPE_LOG`) and record it in the package's manifest.
    Write {
        #[arg(long)]
        world: PathBuf,
        /// The log (one command line per line).
        #[arg(long)]
        log: PathBuf,
        /// The data folder of the build, `data/<region>/`.
        #[arg(long)]
        region: Option<String>,
        /// The build script.
        #[arg(long)]
        script: Option<String>,
        /// The region name of the fetch scripts (`liechtenstein`): each
        /// source names the script that downloads it
        /// (`scripts/fetch-pilot.sh liechtenstein`, `fetch-imagery.sh`, …).
        #[arg(long)]
        fetch: Option<String>,
    },
    /// Print a package's recipe and whether it is the one its manifest records.
    Show {
        #[arg(long)]
        world: PathBuf,
    },
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

/// Appends this run's command line to `$OCW_RECIPE_LOG` (if set; a
/// `recipe` command itself is not logged).
pub fn log_invocation() {
    let Some(path) = std::env::var_os(LOG_ENV).filter(|p| !p.is_empty()) else {
        return;
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_none_or(|a| a == "recipe") {
        return;
    }
    use std::io::Write;
    let line = log_line(&args);
    let r = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = r {
        eprintln!(
            "warning: {}: {e} (build recipe log)",
            Path::new(&path).display()
        );
    }
}

/// One logged command line.
pub fn log_line(args: &[String]) -> String {
    args.iter()
        .map(|a| a.replace(['\n', SEP], " "))
        .collect::<Vec<_>>()
        .join(&SEP.to_string())
}

/// The command lines of a log.
pub fn parse_log(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(SEP).map(String::from).collect())
        .collect()
}

/// The script that downloads a source of this kind for `region`.
pub fn fetch_hint(what: &str, region: &str) -> String {
    let script = if what.starts_with("imagery") {
        "fetch-imagery.sh"
    } else if what.starts_with("forest") {
        "fetch-forest.sh"
    } else if what.starts_with("crop") {
        "fetch-crops.sh"
    } else {
        "fetch-pilot.sh"
    };
    format!("scripts/{script} {region}")
}

/// Size and SHA-256 of a source file (`None`: a folder or not readable).
fn file_info(p: &Path) -> Option<(u64, String)> {
    if !p.is_file() {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    Some((
        bytes.len() as u64,
        ocw_world::manifest::sha256(&bytes).to_string(),
    ))
}

fn read_manifest(package: &Path) -> Option<SignedManifest> {
    std::fs::read_to_string(sidecar(package))
        .ok()
        .and_then(|t| SignedManifest::parse(&t).ok())
}

/// Writes the recipe of `package` from the build log and records it in
/// the manifest; the recipe.
pub fn write(
    package: &Path,
    log: &Path,
    region: Option<String>,
    script: Option<String>,
    fetch: Option<String>,
) -> Recipe {
    let text =
        std::fs::read_to_string(log).unwrap_or_else(|e| fail(format!("{}: {e}", log.display())));
    let signed = read_manifest(package).unwrap_or_else(|| {
        fail(format!(
            "{}: no manifest next to the package (the package step writes one)",
            package.display()
        ))
    });
    let was_signed = signed.is_signed();
    let mut m: Manifest = signed.manifest;
    let mut r = Recipe::from_log(&m.id, package, &parse_log(&text), file_info);
    r.region = region;
    r.script = script;
    if let Some(region) = &fetch {
        for s in &mut r.sources {
            s.fetch = Some(fetch_hint(&s.what, region));
        }
    }
    if let Some(p) = r.problem() {
        fail(format!(
            "no recipe for {}: {p} (was OCW_RECIPE_LOG set while it was built?)",
            package.display()
        ));
    }
    let path = recipe_path(package);
    let toml = r.to_toml();
    std::fs::write(&path, &toml).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let record = PatchFile::of(&name, toml.as_bytes()).unwrap_or_else(|e| fail(e));
    m.recipe = Some(record.clone());
    let side = sidecar(package);
    std::fs::write(&side, SignedManifest::unsigned(&m).to_json())
        .unwrap_or_else(|e| fail(format!("{}: {e}", side.display())));
    println!(
        "recipe: {} ({} steps: {}; {} sources), sha256 {} recorded in {}{}",
        path.display(),
        r.steps.len(),
        r.steps
            .iter()
            .map(|s| s.step.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        r.sources.len(),
        record.sha256,
        side.display(),
        if was_signed {
            " (now unsigned: sign it again)"
        } else {
            ""
        }
    );
    r
}

/// The recorded recipe against the file next to the package.
pub fn status(package: &Path, m: &Manifest) -> Option<(PatchFile, PatchStatus)> {
    let r = m.recipe.as_ref()?;
    let dir = package.parent().unwrap_or(Path::new("."));
    let found = std::fs::read(dir.join(&r.path)).ok();
    Some((r.clone(), r.status(found.as_deref())))
}

/// A report line of the recipe's status (build, verify, publish).
pub fn describe(r: &PatchFile, s: &PatchStatus) -> String {
    match s {
        PatchStatus::Same => format!("recipe {}: OK (sha256 {})", r.path, r.sha256),
        PatchStatus::Missing => format!(
            "recipe {}: MISSING next to the package (the manifest records sha256 {}): users' builds run the basic step only",
            r.path, r.sha256
        ),
        PatchStatus::Changed { sha256, .. } => format!(
            "recipe {}: CHANGED since the build (sha256 {sha256}, recorded {}): not shipped; write it again with the build",
            r.path, r.sha256
        ),
    }
}

/// Prints the recipe's status line (a warning unless it matches); the
/// number of warnings.
pub fn report(package: &Path, m: &Manifest) -> usize {
    match status(package, m) {
        Some((r, PatchStatus::Same)) => {
            println!("  {}", describe(&r, &PatchStatus::Same));
            0
        }
        Some((r, s)) => {
            eprintln!("warning: {}", describe(&r, &s));
            1
        }
        None => 0,
    }
}

pub fn run(cmd: RecipeCmd) {
    match cmd {
        RecipeCmd::Write {
            world,
            log,
            region,
            script,
            fetch,
        } => {
            write(&world, &log, region, script, fetch);
        }
        RecipeCmd::Show { world } => {
            let path = recipe_path(&world);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
            let r =
                Recipe::parse(&text).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
            println!("recipe of {} ({} steps)", r.package, r.steps.len());
            for s in &r.steps {
                println!("  {} {}", s.step, s.args.join(" "));
            }
            for s in &r.sources {
                println!(
                    "  source {}: {}{}",
                    s.name,
                    s.what,
                    s.sha256
                        .as_ref()
                        .map_or(String::new(), |h| format!(", sha256 {h}"))
                );
            }
            match read_manifest(&world) {
                Some(m) if m.manifest.recipe.is_some() => {
                    report(&world, &m.manifest);
                }
                _ => println!("  (not recorded in a manifest)"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::manifest::Attribution;

    #[test]
    fn log_lines_round_trip() {
        let a: Vec<String> = ["route", "--world", "d/x.ocwpkg", "--name", "Vaduz - Malbun"]
            .map(String::from)
            .to_vec();
        let b: Vec<String> = ["package", "--name", "two\nlines"]
            .map(String::from)
            .to_vec();
        let text = format!("{}\n{}\n\n", log_line(&a), log_line(&b));
        let back = parse_log(&text);
        assert_eq!(back[0], a);
        assert_eq!(back[1][2], "two lines");
    }

    /// `recipe write`: the recipe next to the package, its sources'
    /// checksums, its record in the manifest; verify / publish see it.
    #[test]
    fn write_records_the_recipe_in_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        let pkg = dir.join("li.ocwpkg");
        std::fs::write(&pkg, b"pkg").unwrap();
        std::fs::write(dir.join("li.osm.pbf"), b"osm data").unwrap();
        std::fs::create_dir(dir.join("swissalti3d")).unwrap();
        let mut m = Manifest::new(
            "europe/liechtenstein",
            "Liechtenstein",
            3,
            0,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("li.ocwpkg", b"pkg").unwrap();
        std::fs::write(sidecar(&pkg), SignedManifest::unsigned(&m).to_json()).unwrap();
        let p = |s: &Path| s.display().to_string();
        let log = dir.join("build.log");
        let lines = [
            vec![
                "package".into(),
                "--pbf".into(),
                p(&dir.join("li.osm.pbf")),
                "--dem".into(),
                p(&dir.join("swissalti3d")),
                "--name".into(),
                "Liechtenstein".into(),
                "--out".into(),
                p(&pkg),
            ],
            vec![
                "route".into(),
                "--world".into(),
                p(&pkg),
                "--name".into(),
                "Vaduz - Malbun".into(),
            ],
            vec!["routes".into(), "--world".into(), p(&pkg)],
        ];
        let text: String = lines.iter().map(|l| log_line(l) + "\n").collect();
        std::fs::write(&log, text).unwrap();
        let r = write(
            &pkg,
            &log,
            Some("pilot".into()),
            Some("scripts/build-liechtenstein.sh".into()),
            Some("liechtenstein".into()),
        );
        assert_eq!(r.steps.len(), 2);
        let pbf = r.source("li.osm.pbf").unwrap();
        assert_eq!(pbf.size, Some(8));
        assert_eq!(
            pbf.sha256.as_deref(),
            Some(
                ocw_world::manifest::sha256(b"osm data")
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(
            pbf.fetch.as_deref(),
            Some("scripts/fetch-pilot.sh liechtenstein")
        );
        assert!(r.source("swissalti3d").unwrap().dir);
        let file = dir.join("li.recipe.toml");
        let back = Recipe::parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.region.as_deref(), Some("pilot"));
        let m = read_manifest(&pkg).unwrap().manifest;
        let rec = m.recipe.clone().unwrap();
        assert_eq!(rec.path, "li.recipe.toml");
        assert_eq!(status(&pkg, &m).unwrap().1, PatchStatus::Same);
        assert_eq!(report(&pkg, &m), 0);
        assert_eq!(m.version, 3, "the version stays");
        // Changed after the build: reported.
        std::fs::write(&file, "edited").unwrap();
        assert!(matches!(
            status(&pkg, &m).unwrap().1,
            PatchStatus::Changed { .. }
        ));
        assert_eq!(report(&pkg, &m), 1);
    }
}
