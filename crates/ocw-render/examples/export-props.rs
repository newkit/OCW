//! Exports the point obstacles and street lamps into
//! `artwork/3dmodels/props/` with their manifest entries, once (spec §14
//! Phase 15, 15H MX; the files are the source afterwards, Q6):
//!
//! ```text
//! cargo run -p ocw-render --example export-props [-- --out DIR]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out = PathBuf::from("artwork/3dmodels");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--out", Some(d)) => out = d.into(),
            _ => {
                eprintln!("usage: export-props [--out DIR]");
                return ExitCode::from(2);
            }
        }
    }
    let models = ocw_render::prop_export::models();
    for (e, m) in &models {
        let bytes = ocw_assets::glb::write(m);
        let path = out.join(&e.file);
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
    let mut all = match ocw_assets::manifest::Manifest::parse(&old) {
        Ok(m) => m.assets,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            return ExitCode::FAILURE;
        }
    };
    all.retain(|e| !models.iter().any(|(n, _)| n.file == e.file));
    all.extend(models.into_iter().map(|(e, _)| e));
    if let Err(err) = std::fs::write(&manifest, ocw_assets::export::manifest_text(&all)) {
        eprintln!("{}: {err}", manifest.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
