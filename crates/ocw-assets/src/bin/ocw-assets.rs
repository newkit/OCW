//! `ocw-assets`: generate the shipped bike and rider models, validate an
//! asset directory (spec §8.1 step 5), or list a model's nodes.
//!
//! ```text
//! ocw-assets generate [--out assets/models] [--check]
//! ocw-assets validate [DIR]
//! ocw-assets info FILE.glb
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
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
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
