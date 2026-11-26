//! The build recipe of a world package (spec 14 Phase 16, 16C *Preview
//! and build*, 16H Q17 (a)): a small TOML file next to the package,
//! `<package>.recipe.toml`, that says how the official build made it –
//! the `world-builder` steps in order (the package, its routes, the
//! calibration, the forest and the vegetation), their arguments, and the
//! source data each needs (the OSM extract, the region polygon, the
//! elevation tiles and models, the imagery, the forest and crop maps),
//! named by their file or folder name, with the size and SHA-256 of each
//! file. The build scripts write it (`world-builder recipe write`), its
//! SHA-256 goes into the manifest like the patches' (16H Q9:
//! `Manifest::recipe`), `world-builder publish` and the release script
//! ship it, and OCW Studio's *Build package* runs the whole chain with
//! the user's patch when it finds the named sources ([`Recipe::commands`]).
//!
//! Arguments name the package as `{package}` and a source as
//! `{src:NAME}`; the package step's `--out`, `--patch`, `--sign-key` and
//! `--profile` and the switch `--strict-patch` are not recorded (the one
//! who builds chooses them).
//!
//! ```toml
//! ocwrecipe = 1
//! package = "europe/liechtenstein"
//! region = "pilot"
//! script = "scripts/build-liechtenstein.sh"
//!
//! [[source]]
//! name = "liechtenstein.osm.pbf"
//! what = "OSM extract"
//! size = 3456789
//! sha256 = "…"
//! fetch = "scripts/fetch-pilot.sh liechtenstein"
//!
//! [[source]]
//! name = "swissalti3d"
//! what = "elevation model (DTM folder)"
//! dir = true
//!
//! [[step]]
//! step = "package"
//! args = ["--pbf", "{src:liechtenstein.osm.pbf}", "--dem", "{src:swissalti3d}", "--name", "Liechtenstein"]
//!
//! [[step]]
//! step = "route"
//! args = ["--world", "{package}", "--name", "Vaduz - Malbun", "--from", "47.1395,9.5220", "--to", "47.1013,9.6094"]
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The format version.
pub const FORMAT_VERSION: u32 = 1;

/// The file name's ending: `<package stem>.recipe.toml`.
pub const SUFFIX: &str = "recipe.toml";

/// The `world-builder` commands a recipe runs (the ones that make or
/// change the package; listing, checking and signing are not steps).
pub const STEPS: [&str; 5] = ["package", "route", "calibrate", "forest", "vegetation"];

/// The flags whose value is source data, and what it is.
pub const SOURCE_FLAGS: [(&str, &str); 7] = [
    ("--pbf", "OSM extract"),
    ("--poly", "region boundary (polygon)"),
    ("--hgt", "SRTM elevation tile"),
    ("--dem", "elevation model (DTM folder)"),
    ("--imagery", "imagery for the calibration"),
    ("--forest", "forest type and density maps"),
    ("--crops", "crop map"),
];

/// Flags of the package step the one who builds chooses (not recorded).
const OWN_FLAGS: [&str; 4] = ["--out", "--patch", "--sign-key", "--profile"];
/// Switches of the package step that are ours alone (not recorded): an
/// official build fails on a patch target that is gone (16H Q7 (a)), a
/// user's build from the recipe only warns.
const OWN_SWITCHES: [&str; 1] = ["--strict-patch"];

/// The recipe of `package`: `<stem>.recipe.toml` next to it.
pub fn recipe_path(package: &Path) -> PathBuf {
    package.with_extension(SUFFIX)
}

/// A build recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// [`FORMAT_VERSION`].
    pub ocwrecipe: u32,
    /// The package id (the manifest's).
    pub package: String,
    /// The data folder of the official build, `data/<region>/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The build script that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, rename = "source", skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    #[serde(default, rename = "step", skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// Source data a step needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Its file or folder name (looked for under this name).
    pub name: String,
    /// What it is ("OSM extract").
    pub what: String,
    /// A folder (no checksum).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// SHA-256 (lower-case hex) of the file the official build used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// How to get it ("scripts/fetch-pilot.sh liechtenstein").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch: Option<String>,
}

/// One `world-builder` command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The command ([`STEPS`]).
    pub step: String,
    pub args: Vec<String>,
}

/// The placeholder of a source in the arguments.
pub fn source_ref(name: &str) -> String {
    format!("{{src:{name}}}")
}

/// The placeholder of the package.
pub const PACKAGE_REF: &str = "{package}";

impl Recipe {
    pub fn new(package: impl Into<String>) -> Self {
        Self {
            ocwrecipe: FORMAT_VERSION,
            package: package.into(),
            region: None,
            script: None,
            sources: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// Reads and checks a recipe.
    pub fn parse(text: &str) -> Result<Self, String> {
        let head: toml::Table = toml::from_str(text).map_err(|e| e.message().to_string())?;
        match head.get("ocwrecipe") {
            None => return Err("not a build recipe: the `ocwrecipe = 1` line is missing".into()),
            Some(toml::Value::Integer(v)) if *v > i64::from(FORMAT_VERSION) => {
                return Err(format!(
                    "recipe format {v} is newer than this program reads ({FORMAT_VERSION}); update OCW"
                ));
            }
            Some(toml::Value::Integer(v)) if *v >= 1 => {}
            Some(v) => return Err(format!("`ocwrecipe` must be a format version, not {v}")),
        }
        let r: Self = toml::from_str(text).map_err(|e| e.message().to_string())?;
        if let Some(p) = r.problem() {
            return Err(p);
        }
        Ok(r)
    }

    /// The TOML text (reads back to the same recipe).
    pub fn to_toml(&self) -> String {
        format!(
            "# Build recipe of a world package (OCW, spec 16H Q17): the world-builder steps of its\n# official build and the source data they need. OCW Studio's Build package runs them.\n{}",
            toml::to_string(self).expect("a recipe serialises")
        )
    }

    /// What is wrong with it beyond the types: the first step builds the
    /// package, the others act on it, every source named is listed.
    pub fn problem(&self) -> Option<String> {
        if self.package.trim().is_empty() {
            return Some("no package id".into());
        }
        match self.steps.first() {
            None => return Some("no steps".into()),
            Some(s) if s.step != "package" => {
                return Some(format!("the first step is `{}`, not `package`", s.step));
            }
            _ => {}
        }
        for (i, s) in self.steps.iter().enumerate() {
            if !STEPS.contains(&s.step.as_str()) {
                return Some(format!("step {}: unknown step `{}`", i + 1, s.step));
            }
            if i > 0 && !s.args.iter().any(|a| a == PACKAGE_REF) {
                return Some(format!(
                    "step {} ({}) does not act on the package",
                    i + 1,
                    s.step
                ));
            }
            for a in &s.args {
                if let Some(name) = a.strip_prefix("{src:").and_then(|r| r.strip_suffix('}'))
                    && self.source(name).is_none()
                {
                    return Some(format!(
                        "step {} ({}): source `{name}` is not listed",
                        i + 1,
                        s.step
                    ));
                }
            }
        }
        None
    }

    pub fn source(&self, name: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.name == name)
    }

    /// The sources a step needs.
    pub fn step_sources<'a>(&'a self, step: &'a Step) -> impl Iterator<Item = &'a Source> + 'a {
        step.args.iter().filter_map(|a| {
            let name = a.strip_prefix("{src:")?.strip_suffix('}')?;
            self.source(name)
        })
    }

    /// Makes a recipe from the `world-builder` command lines a build ran
    /// (`log`: each line's arguments without the program) for the package
    /// at `package`: the steps that made or changed it, in order (a new
    /// `package` build of it starts over), their source paths as
    /// `{src:NAME}` and the package as `{package}`. `source_info` gives a
    /// source file's size and SHA-256 (`None` for a folder).
    pub fn from_log(
        id: &str,
        package: &Path,
        log: &[Vec<String>],
        mut source_info: impl FnMut(&Path) -> Option<(u64, String)>,
    ) -> Self {
        let same = |a: &str| paths_equal(Path::new(a), package);
        let mut r = Self::new(id);
        let mut paths: BTreeMap<String, String> = BTreeMap::new();
        for line in log {
            let Some((cmd, args)) = line.split_first() else {
                continue;
            };
            if !STEPS.contains(&cmd.as_str()) {
                continue;
            }
            let value = |flag: &str| args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
            let is_build = cmd == "package" && value("--out").is_some_and(|o| same(&o));
            let on_it = cmd != "package" && value("--world").is_some_and(|w| same(&w));
            // A step writing elsewhere (`--out`) does not change this one.
            let elsewhere = cmd != "package" && value("--out").is_some_and(|o| !same(&o));
            if !(is_build || on_it) || elsewhere {
                continue;
            }
            if is_build {
                r.steps.clear();
            }
            let mut out = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let a = &args[i];
                let next = args.get(i + 1);
                if is_build && OWN_FLAGS.contains(&a.as_str()) && next.is_some() {
                    i += 2;
                    continue;
                }
                if is_build && OWN_SWITCHES.contains(&a.as_str()) {
                    i += 1;
                    continue;
                }
                out.push(a.clone());
                if let Some(v) = next {
                    if a == "--world" && same(v) {
                        out.push(PACKAGE_REF.into());
                        i += 2;
                        continue;
                    }
                    if let Some((_, what)) = SOURCE_FLAGS.iter().find(|(f, _)| f == a) {
                        let name = r.add_source(&mut paths, v, what, &mut source_info);
                        out.push(source_ref(&name));
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }
            r.steps.push(Step {
                step: cmd.clone(),
                args: out,
            });
        }
        // Only the sources the kept steps use.
        let used: Vec<String> = r
            .steps
            .iter()
            .flat_map(|s| s.args.iter())
            .filter_map(|a| Some(a.strip_prefix("{src:")?.strip_suffix('}')?.to_owned()))
            .collect();
        r.sources.retain(|s| used.contains(&s.name));
        r
    }

    /// Adds the source at `path` (once per path; a second source of the
    /// same name gets a number); its name.
    fn add_source(
        &mut self,
        paths: &mut BTreeMap<String, String>,
        path: &str,
        what: &str,
        info: &mut impl FnMut(&Path) -> Option<(u64, String)>,
    ) -> String {
        if let Some(n) = paths.get(path) {
            return n.clone();
        }
        let p = Path::new(path);
        let base = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned());
        let mut name = base.clone();
        let mut k = 2;
        while self.source(&name).is_some() {
            name = format!("{base}#{k}");
            k += 1;
        }
        let file = info(p);
        self.sources.push(Source {
            name: name.clone(),
            what: what.into(),
            dir: file.is_none(),
            size: file.as_ref().map(|f| f.0),
            sha256: file.map(|f| f.1),
            fetch: None,
        });
        paths.insert(path.to_owned(), name.clone());
        name
    }

    /// The `world-builder` command lines that build the package to `out`
    /// with `patches`, the sources at `found` (by name): the package step
    /// gets `--out OUT` and a `--patch` per patch, every `{package}` is
    /// `out`. A source not in `found` is an error (see [`Recipe::missing`]).
    pub fn commands(
        &self,
        found: &BTreeMap<String, PathBuf>,
        out: &Path,
        patches: &[PathBuf],
    ) -> Result<Vec<Vec<String>>, String> {
        let mut lines = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            let mut line = vec![s.step.clone()];
            for a in &s.args {
                if a == PACKAGE_REF {
                    line.push(out.display().to_string());
                } else if let Some(name) = a.strip_prefix("{src:").and_then(|r| r.strip_suffix('}'))
                {
                    let p = found
                        .get(name)
                        .ok_or_else(|| format!("source `{name}` not found"))?;
                    line.push(p.display().to_string());
                } else {
                    line.push(a.clone());
                }
            }
            if i == 0 {
                line.push("--out".into());
                line.push(out.display().to_string());
                for p in patches {
                    line.push("--patch".into());
                    line.push(p.display().to_string());
                }
            }
            lines.push(line);
        }
        Ok(lines)
    }

    /// The sources not in `found`, in order.
    pub fn missing<'a>(&'a self, found: &BTreeMap<String, PathBuf>) -> Vec<&'a Source> {
        self.sources
            .iter()
            .filter(|s| !found.contains_key(&s.name))
            .collect()
    }

    /// Looks for every source by its name in `dirs` (in order: the first
    /// folder that has it wins; a folder source must be a folder, a file a
    /// file).
    pub fn find_sources(&self, dirs: &[PathBuf]) -> BTreeMap<String, PathBuf> {
        let mut out = BTreeMap::new();
        for s in &self.sources {
            // A numbered duplicate (`x#2`) is looked for under its name.
            let file = s.name.split('#').next().unwrap_or(&s.name);
            for d in dirs {
                let p = d.join(file);
                if (s.dir && p.is_dir()) || (!s.dir && p.is_file()) {
                    out.insert(s.name.clone(), p);
                    break;
                }
            }
        }
        out
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The same file (both canonical where they exist, else as written).
fn paths_equal(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Vec<String> {
        s.split('|').map(String::from).collect()
    }

    /// The Liechtenstein build as `scripts/build-liechtenstein.sh` runs it
    /// (route file, listing and check steps in between).
    fn log() -> Vec<Vec<String>> {
        [
            "package|--pbf|d/li.osm.pbf|--poly|d/li.poly|--hgt|d/N47E009.hgt|--dem|d/swissalti3d|--name|Liechtenstein|--dem-attribution|swissalti3d|--sign-key|k.key|--patch|w/li.ocwpatch|--strict-patch|--out|d/li.ocwpkg",
            "route|--pbf|d/li.osm.pbf|--poly|d/li.poly|--hgt|d/N47E009.hgt|--name|Vaduz - Malbun|--out|d/vaduz-malbun.route",
            "route|--world|d/li.ocwpkg|--name|Vaduz - Malbun|--from|47.1395,9.5220|--to|47.1013,9.6094",
            "calibrate|--world|d/li.ocwpkg|--imagery|d/imagery|--pbf|d/li.osm.pbf|--season-cells|50|--forest|d/forest",
            "vegetation|--world|d/li.ocwpkg|--pbf|d/li.osm.pbf|--crops|d/crops",
            "sign|--key|k.key|d/li.ocwpkg",
            "routes|--world|d/li.ocwpkg",
            "check|--world|d/li.ocwpkg",
            "route|--world|d/other.ocwpkg|--name|X|--from|1,2|--to|3,4",
        ]
        .map(line)
        .to_vec()
    }

    fn info(p: &Path) -> Option<(u64, String)> {
        let n = p.file_name()?.to_string_lossy();
        n.contains('.')
            .then(|| (n.len() as u64, format!("{:0>64}", n.len())))
    }

    #[test]
    fn a_build_log_becomes_its_recipe() {
        let r = Recipe::from_log(
            "europe/liechtenstein",
            Path::new("d/li.ocwpkg"),
            &log(),
            info,
        );
        let steps: Vec<&str> = r.steps.iter().map(|s| s.step.as_str()).collect();
        assert_eq!(steps, ["package", "route", "calibrate", "vegetation"]);
        assert_eq!(
            r.steps[0].args.join(" "),
            "--pbf {src:li.osm.pbf} --poly {src:li.poly} --hgt {src:N47E009.hgt} --dem {src:swissalti3d} --name Liechtenstein --dem-attribution swissalti3d",
            "no --out, --patch, --strict-patch, --sign-key"
        );
        assert_eq!(
            r.steps[2].args.join(" "),
            "--world {package} --imagery {src:imagery} --pbf {src:li.osm.pbf} --season-cells 50 --forest {src:forest}"
        );
        let names: Vec<&str> = r.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "li.osm.pbf",
                "li.poly",
                "N47E009.hgt",
                "swissalti3d",
                "imagery",
                "forest",
                "crops"
            ]
        );
        let pbf = r.source("li.osm.pbf").unwrap();
        assert_eq!((pbf.what.as_str(), pbf.dir), ("OSM extract", false));
        assert_eq!(pbf.size, Some(10));
        assert!(pbf.sha256.is_some());
        let dem = r.source("swissalti3d").unwrap();
        assert!(dem.dir && dem.sha256.is_none());
        assert_eq!(r.problem(), None);
        // Round trip through the file.
        let text = r.to_toml();
        assert!(text.starts_with("# Build recipe"), "{text}");
        assert_eq!(Recipe::parse(&text).unwrap(), r);
        // A second package build of it starts over.
        let mut again = log();
        again.push(line(
            "package|--pbf|d/li.osm.pbf|--name|Liechtenstein|--out|d/li.ocwpkg",
        ));
        let r2 = Recipe::from_log(
            "europe/liechtenstein",
            Path::new("d/li.ocwpkg"),
            &again,
            info,
        );
        assert_eq!(r2.steps.len(), 1);
        assert_eq!(r2.sources.len(), 1, "only what is used");
    }

    #[test]
    fn the_commands_put_in_the_sources_the_output_and_the_patches() {
        let r = Recipe::from_log(
            "europe/liechtenstein",
            Path::new("d/li.ocwpkg"),
            &log(),
            info,
        );
        let mut found = BTreeMap::new();
        for s in &r.sources {
            found.insert(s.name.clone(), PathBuf::from("/src").join(&s.name));
        }
        let c = r
            .commands(
                &found,
                Path::new("/w/patched/li.ocwpkg"),
                &[PathBuf::from("/w/li.ocwpatch")],
            )
            .unwrap();
        assert_eq!(c.len(), 4);
        assert_eq!(
            c[0].join(" "),
            "package --pbf /src/li.osm.pbf --poly /src/li.poly --hgt /src/N47E009.hgt --dem /src/swissalti3d --name Liechtenstein --dem-attribution swissalti3d --out /w/patched/li.ocwpkg --patch /w/li.ocwpatch"
        );
        assert_eq!(
            c[3].join(" "),
            "vegetation --world /w/patched/li.ocwpkg --pbf /src/li.osm.pbf --crops /src/crops"
        );
        // Missing sources are named; the commands refuse without them.
        found.remove("imagery");
        found.remove("crops");
        let m: Vec<&str> = r.missing(&found).iter().map(|s| s.name.as_str()).collect();
        assert_eq!(m, ["imagery", "crops"]);
        assert!(r.commands(&found, Path::new("o"), &[]).is_err());
        // The package step alone has its sources.
        assert!(
            r.step_sources(&r.steps[0])
                .all(|s| found.contains_key(&s.name))
        );
    }

    #[test]
    fn sources_are_found_by_name_in_the_folders_in_order() {
        let d = std::env::temp_dir().join(format!("ocw-recipe-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(a.join("forest")).unwrap();
        std::fs::create_dir_all(b.join("swissalti3d")).unwrap();
        std::fs::write(a.join("li.osm.pbf"), "x").unwrap();
        std::fs::write(b.join("li.osm.pbf"), "y").unwrap();
        // A file where a folder is named does not count.
        std::fs::write(a.join("crops"), "not a folder").unwrap();
        let r = Recipe::from_log(
            "europe/liechtenstein",
            Path::new("d/li.ocwpkg"),
            &log(),
            info,
        );
        let found = r.find_sources(&[a.clone(), b.clone()]);
        assert_eq!(
            found["li.osm.pbf"],
            a.join("li.osm.pbf"),
            "the first folder"
        );
        assert_eq!(found["swissalti3d"], b.join("swissalti3d"));
        assert_eq!(found["forest"], a.join("forest"));
        assert!(!found.contains_key("crops"));
        assert!(!found.contains_key("li.poly"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn broken_recipes_are_refused_with_a_reason() {
        assert!(
            Recipe::parse("package = \"x\"")
                .unwrap_err()
                .contains("not a build recipe")
        );
        assert!(
            Recipe::parse("ocwrecipe = 9\npackage = \"x\"")
                .unwrap_err()
                .contains("newer")
        );
        let e = Recipe::parse("ocwrecipe = 1\npackage = \"x\"\n[[step]]\nstep = \"route\"\nargs = [\"--world\", \"{package}\"]\n").unwrap_err();
        assert!(e.contains("first step"), "{e}");
        let e = Recipe::parse("ocwrecipe = 1\npackage = \"x\"\n[[step]]\nstep = \"package\"\nargs = [\"--pbf\", \"{src:gone.pbf}\"]\n").unwrap_err();
        assert!(e.contains("gone.pbf"), "{e}");
        let e = Recipe::parse("ocwrecipe = 1\npackage = \"x\"\n[[step]]\nstep = \"package\"\nargs = []\n[[step]]\nstep = \"vegetation\"\nargs = []\n").unwrap_err();
        assert!(e.contains("does not act on the package"), "{e}");
        let e = Recipe::parse("ocwrecipe = 1\npackage = \"x\"\nlava = 1\n").unwrap_err();
        assert!(e.contains("lava"), "{e}");
        assert_eq!(
            recipe_path(Path::new("/w/li.ocwpkg")),
            PathBuf::from("/w/li.recipe.toml")
        );
    }
}
