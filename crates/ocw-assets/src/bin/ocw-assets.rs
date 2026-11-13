//! `ocw-assets`: generate the shipped bike and rider models, validate an
//! asset directory (spec §8.1 step 5), or list a model's nodes.
//!
//! ```text
//! ocw-assets generate [--out assets/models] [--check]
//! ocw-assets validate [DIR]
//! ocw-assets info FILE.glb
//! ocw-assets materials fetch [--manifest assets/materials.toml] [--out assets/materials]
//! ocw-assets materials validate [--manifest assets/materials.toml] [DIR]
//! ocw-assets materials credits [--manifest assets/materials.toml]
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
    /// Write the generated bikes and the rider as .glb files.
    Generate {
        #[arg(long, default_value = "assets/models")]
        out: PathBuf,
        /// Only check that the files there are what the generator makes.
        #[arg(long)]
        check: bool,
    },
    /// Check the manifest and every model in a directory.
    Validate {
        #[arg(default_value = "assets/models")]
        dir: PathBuf,
    },
    /// Print a model's nodes, materials and triangle counts.
    Info { file: PathBuf },
    /// The material library (spec §4.6, Phase 8 T1).
    #[command(subcommand)]
    Materials(MaterialsCmd),
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
        Cmd::Generate { out, check } => {
            let mut stale = 0;
            for (name, bytes) in ocw_assets::generate() {
                let path = out.join(&name);
                if check {
                    let same = std::fs::read(&path).is_ok_and(|old| old == bytes);
                    if !same {
                        println!("{} differs", path.display());
                        stale += 1;
                    }
                    continue;
                }
                if let Err(e) = std::fs::write(&path, &bytes) {
                    eprintln!("{}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
                println!("{} ({} KiB)", path.display(), bytes.len() / 1024);
            }
            if stale > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Cmd::Validate { dir } => match ocw_assets::manifest::validate_dir(&dir) {
            Ok(checked) => {
                for c in &checked {
                    println!(
                        "ok  {:24} {:?} {} triangles, {} ({})",
                        c.entry.file, c.entry.kind, c.triangles, c.entry.license, c.entry.author
                    );
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
        Cmd::Info { file } => {
            let bytes = match std::fs::read(&file) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
            match ocw_assets::glb::read(&bytes) {
                Ok(m) => {
                    println!("copyright: {}", m.copyright);
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
            }
        }
    }
}
