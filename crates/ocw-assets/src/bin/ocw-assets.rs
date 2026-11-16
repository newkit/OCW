//! `ocw-assets`: export the generated models into `artwork/3dmodels/`
//! (spec §14 Phase 15, M1), validate a model folder (spec §8.1 step 5), or
//! list a model's nodes.
//!
//! ```text
//! ocw-assets model export-all [--out artwork/3dmodels] [--check]
//! ocw-assets generate [--out artwork/3dmodels] [--check]   (the same)
//! ocw-assets validate [DIR]
//! ocw-assets info FILE.glb|FILE.gltf
//! ocw-assets materials fetch [--manifest assets/materials.toml] [--out assets/materials]
//! ocw-assets materials validate [--manifest assets/materials.toml] [DIR]
//! ocw-assets materials credits [--manifest assets/materials.toml]
//! ocw-assets parts validate [--file assets/parts.toml] [--sources assets/parts-sources.toml]
//! ocw-assets parts reprice [--file assets/parts.toml] [--sources assets/parts-sources.toml]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Open Cycling World 3D assets: generator and validator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// The same as `model export-all` (the old name).
    Generate {
        #[arg(long, default_value = "artwork/3dmodels")]
        out: PathBuf,
        /// Only check that the files there are what the generators make.
        #[arg(long)]
        check: bool,
    },
    /// The model files (spec §14 Phase 15).
    #[command(subcommand)]
    Model(ModelCmd),
    /// Check the manifest and every model below a directory.
    Validate {
        #[arg(default_value = "artwork/3dmodels")]
        dir: PathBuf,
    },
    /// Print a model's nodes, materials and triangle counts.
    Info { file: PathBuf },
    /// The material library (spec §4.6, Phase 8 T1).
    #[command(subcommand)]
    Materials(MaterialsCmd),
    /// The parts catalogue of the workshop (spec §14 Phase 4, 4H, 4I).
    #[command(subcommand)]
    Parts(PartsCmd),
}

#[derive(Subcommand)]
enum ModelCmd {
    /// Export every model of this crate's generators (bikes, riders,
    /// helmets) with their manifest entries; the plants come from
    /// `cargo run -p ocw-render --example export-flora`.
    ExportAll {
        #[arg(long, default_value = "artwork/3dmodels")]
        out: PathBuf,
        /// Only check that the files and the manifest are what the
        /// generators make (byte for byte).
        #[arg(long)]
        check: bool,
    },
}

/// Writes (or with `check` compares) the exported files and the manifest.
fn export_all(out: &std::path::Path, check: bool) -> ExitCode {
    let files = ocw_assets::export::files();
    let mut stale = 0;
    for (e, bytes) in &files {
        let path = out.join(&e.file);
        if check {
            if !std::fs::read(&path).is_ok_and(|old| old == *bytes) {
                println!("{} differs", path.display());
                stale += 1;
            }
            continue;
        }
        if let Some(dir) = path.parent()
            && let Err(err) = std::fs::create_dir_all(dir)
        {
            eprintln!("{}: {err}", dir.display());
            return ExitCode::FAILURE;
        }
        if let Err(err) = std::fs::write(&path, bytes) {
            eprintln!("{}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("{} ({} KiB)", path.display(), bytes.len() / 1024);
    }
    let manifest = out.join(ocw_assets::manifest::MANIFEST);
    let old = std::fs::read_to_string(&manifest).unwrap_or_default();
    let entries: Vec<_> = files.into_iter().map(|(e, _)| e).collect();
    let text =
        match ocw_assets::export::write_manifest(&old, &ocw_assets::export::OWN_PREFIXES, &entries)
        {
            Ok(t) => t,
            Err(errors) => return report(&errors),
        };
    if check {
        if text != old {
            println!("{} differs", manifest.display());
            stale += 1;
        }
    } else if let Err(err) = std::fs::write(&manifest, text) {
        eprintln!("{}: {err}", manifest.display());
        return ExitCode::FAILURE;
    }
    if stale > 0 {
        eprintln!("{stale} stale: run `cargo run -p ocw-assets -- model export-all`");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Subcommand)]
enum PartsCmd {
    /// Check the catalogue: ids, prices = ceil(median) of the sources,
    /// base parts, brand names, model references, source age.
    Validate {
        #[arg(long, default_value = "assets/parts.toml")]
        file: PathBuf,
        #[arg(long, default_value = "assets/parts-sources.toml")]
        sources: PathBuf,
    },
    /// Recompute the hearts from the sources and rewrite them in the
    /// catalogue (comments and layout stay), printing the changes.
    Reprice {
        #[arg(long, default_value = "assets/parts.toml")]
        file: PathBuf,
        #[arg(long, default_value = "assets/parts-sources.toml")]
        sources: PathBuf,
    },
}

fn read_text(path: &std::path::Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("{}: {e}", path.display());
        ExitCode::FAILURE
    })
}

fn report(errors: &[String]) -> ExitCode {
    for e in errors {
        eprintln!("error: {e}");
    }
    ExitCode::FAILURE
}

fn parts(cmd: PartsCmd) -> ExitCode {
    use ocw_assets::parts::{self, Catalogue, Severity, Sources};
    let (PartsCmd::Validate { file, sources } | PartsCmd::Reprice { file, sources }) = &cmd;
    let (text, src) = match (read_text(file), read_text(sources)) {
        (Ok(t), Ok(s)) => (t, s),
        (Err(c), _) | (_, Err(c)) => return c,
    };
    let src = match Sources::parse(&src) {
        Ok(s) => s,
        Err(e) => return report(&e),
    };
    match cmd {
        PartsCmd::Validate { .. } => {
            let cat = match Catalogue::parse(&text) {
                Ok(c) => c,
                Err(e) => return report(&e),
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let (y, m, d, ..) = ocw_core::timestamp::civil(now);
            let problems = parts::validate(&cat, &src, (y as i32, m, d));
            for p in &problems {
                eprintln!("{p}");
            }
            let n = cat.parts.len();
            if problems.iter().any(|p| p.severity == Severity::Error) {
                ExitCode::FAILURE
            } else {
                println!("ok  {n} parts, {} price sources", src.sources.len());
                ExitCode::SUCCESS
            }
        }
        PartsCmd::Reprice { file, .. } => {
            let (new, changes) = parts::reprice(&text, &src);
            for c in &changes {
                println!("{:32} {:>6} -> {:>6} hearts", c.id, c.old, c.new);
            }
            if changes.is_empty() {
                println!("all prices are up to date");
                return ExitCode::SUCCESS;
            }
            if let Err(e) = std::fs::write(&file, new) {
                eprintln!("{}: {e}", file.display());
                return ExitCode::FAILURE;
            }
            println!("{} prices changed in {}", changes.len(), file.display());
            ExitCode::SUCCESS
        }
    }
}

#[derive(Subcommand)]
enum MaterialsCmd {
    /// Download the pinned sources (cached), check their SHA-256, convert
    /// them to KTX2 and write the library folder.
    Fetch {
        #[arg(long, default_value = "assets/materials.toml")]
        manifest: PathBuf,
        #[arg(long, default_value = "assets/materials")]
        out: PathBuf,
        /// Download cache (default: $OCW_MATERIAL_CACHE or the user cache).
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Only these material ids (comma-separated).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },
    /// Check the manifest (licences, pins) and, if given, a library folder.
    Validate {
        #[arg(long, default_value = "assets/materials.toml")]
        manifest: PathBuf,
        dir: Option<PathBuf>,
    },
    /// Print the credits (Markdown).
    Credits {
        #[arg(long, default_value = "assets/materials.toml")]
        manifest: PathBuf,
    },
}

fn read_manifest(
    path: &std::path::Path,
) -> Result<ocw_assets::materials::MaterialManifest, ExitCode> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("{}: {e}", path.display());
        ExitCode::FAILURE
    })?;
    ocw_assets::materials::MaterialManifest::parse(&text).map_err(|errors| {
        for e in &errors {
            eprintln!("error: {e}");
        }
        ExitCode::FAILURE
    })
}

fn materials(cmd: MaterialsCmd) -> ExitCode {
    use ocw_assets::materials;
    match cmd {
        MaterialsCmd::Fetch {
            manifest,
            out,
            cache,
            only,
        } => {
            let m = match read_manifest(&manifest) {
                Ok(m) => m,
                Err(c) => return c,
            };
            let cache = cache.unwrap_or_else(ocw_assets::fetch::default_cache);
            let only = (!only.is_empty()).then_some(only.as_slice());
            match ocw_assets::fetch::fetch(&m, &out, &cache, only) {
                Ok(r) => {
                    println!(
                        "materials: {} converted, {} up to date, {:.1} MB downloaded; library {}",
                        r.converted.len(),
                        r.up_to_date.len(),
                        r.downloaded_bytes as f64 / 1e6,
                        out.display()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        MaterialsCmd::Validate { manifest, dir } => {
            let m = match read_manifest(&manifest) {
                Ok(m) => m,
                Err(c) => return c,
            };
            let missing = m.missing_layers();
            if !missing.is_empty() {
                eprintln!("error: the renderer needs these materials: {missing:?}");
                return ExitCode::FAILURE;
            }
            for x in &m.materials {
                println!(
                    "ok  {:14} {:?} {} ({}, {})",
                    x.id, x.group, x.license, x.author, x.source
                );
            }
            if let Some(dir) = dir {
                match materials::load_library(&dir) {
                    Ok(lib) => {
                        if lib.manifest != m {
                            eprintln!(
                                "error: {} was fetched from another manifest: run scripts/fetch-materials.sh",
                                dir.display()
                            );
                            return ExitCode::FAILURE;
                        }
                        println!(
                            "library {}: {} materials loaded",
                            dir.display(),
                            lib.materials.len()
                        );
                    }
                    Err(errors) => {
                        for e in &errors {
                            eprintln!("error: {e}");
                        }
                        return ExitCode::FAILURE;
                    }
                }
            }
            ExitCode::SUCCESS
        }
        MaterialsCmd::Credits { manifest } => match read_manifest(&manifest) {
            Ok(m) => {
                print!("{}", materials::credits_markdown(&m));
                ExitCode::SUCCESS
            }
            Err(c) => c,
        },
    }
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Materials(m) => materials(m),
        Cmd::Parts(p) => parts(p),
        Cmd::Generate { out, check } | Cmd::Model(ModelCmd::ExportAll { out, check }) => {
            export_all(&out, check)
        }
        Cmd::Validate { dir } => match ocw_assets::manifest::validate_dir(&dir) {
            Ok(checked) => {
                for c in &checked {
                    println!(
                        "ok  {:44} {:9} {:6} triangles {:7} B, {} ({})",
                        c.entry.file,
                        c.entry.kind.name(),
                        c.triangles,
                        c.bytes,
                        c.entry.license,
                        c.entry.author
                    );
                    for w in c.warnings() {
                        eprintln!("warning: {w}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(errors) => {
                for e in &errors {
                    eprintln!("error: {e}");
                }
                ExitCode::FAILURE
            }
        },
        Cmd::Info { file } => match ocw_assets::glb::read_file(&file) {
            Ok(m) => {
                println!("copyright: {}", m.copyright);
                if !m.extras.is_null() {
                    println!("extras: {}", m.extras);
                }
                for (i, n) in m.nodes.iter().enumerate() {
                    let depth = std::iter::successors(n.parent, |p| m.nodes[*p].parent).count();
                    println!(
                        "{:indent$}{} {:?} – {} triangles",
                        "",
                        n.name,
                        m.position(&n.name).unwrap_or_default(),
                        m.triangles_below(Some(i)),
                        indent = depth * 2
                    );
                }
                let mats: Vec<&str> = m.materials.iter().map(|x| x.name.as_str()).collect();
                println!("materials: {}", mats.join(", "));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}: {e}", file.display());
                ExitCode::FAILURE
            }
        },
    }
}
