//! Exports the plants of `ocw_render::flora` into `artwork/3dmodels/flora/`
//! with their manifest entries (spec §14 Phase 15, M1; Q8):
//!
//! ```text
//! cargo run -p ocw-render --example export-flora [-- [--out DIR] [--check]]
//! ```
//!
//! `--check` only compares (exit 1 when a file or the manifest differs).

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out = PathBuf::from("artwork/3dmodels");
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--check" => check = true,
            "--out" => match args.next() {
                Some(d) => out = d.into(),
                None => {
                    eprintln!("--out needs a folder");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    let models = match ocw_render::flora_files::models() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut stale = 0;
    let mut total = 0;
    for (e, m) in &models {
        let bytes = ocw_assets::glb::write(m);
        total += bytes.len();
        let path = out.join(&e.file);
        if check {
            if !std::fs::read(&path).is_ok_and(|old| old == bytes) {
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
        if let Err(err) = std::fs::write(&path, &bytes) {
            eprintln!("{}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!(
            "{} ({} B, {} triangles)",
            path.display(),
            bytes.len(),
            m.triangles()
        );
    }
    let manifest = out.join(ocw_assets::manifest::MANIFEST);
    let old = std::fs::read_to_string(&manifest).unwrap_or_default();
    let entries: Vec<_> = models.into_iter().map(|(e, _)| e).collect();
    let text = match ocw_assets::export::write_manifest(&old, &["flora/"], &entries) {
        Ok(t) => t,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            return ExitCode::FAILURE;
        }
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
    println!(
        "{} plant files, {:.1} KiB",
        entries.len(),
        total as f64 / 1024.0
    );
    if stale > 0 {
        eprintln!("{stale} stale: cargo run -p ocw-render --example export-flora");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
