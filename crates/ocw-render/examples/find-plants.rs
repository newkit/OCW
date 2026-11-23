//! Finds where a world package's plants of some species stand, as the
//! renderer places them (for scenes and screenshots of a plant in its
//! world, spec 14 Phase 15 Q10):
//!
//! ```text
//! cargo run --release -p ocw-render --example find-plants -- PKG SPECIES… [--max N]
//! ```
//!
//! SPECIES are the names of `ocw_world::flora::Species::name` with `-` for
//! spaces (`teide-broom`, `prickly-pear`, `agave`, `green-alder`). Prints
//! `species x z ground_y scale` for up to N plants (default 5) of each, the
//! tiles in key order; stops when every species has N.

use std::collections::BTreeMap;
use std::process::ExitCode;

use ocw_world::flora::Species;
use ocw_world::package::WorldPackage;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: find-plants PKG SPECIES… [--max N]");
        return ExitCode::from(2);
    };
    let mut wanted: Vec<Species> = Vec::new();
    let mut max = 5usize;
    while let Some(a) = args.next() {
        if a == "--max" {
            max = args.next().and_then(|n| n.parse().ok()).unwrap_or(max);
            continue;
        }
        let name = a.replace('-', " ");
        match Species::ALL
            .iter()
            .find(|s| s.name().eq_ignore_ascii_case(&name))
        {
            Some(s) => wanted.push(*s),
            None => {
                eprintln!("unknown species {a}");
                return ExitCode::from(2);
            }
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pkg = match WorldPackage::from_bytes(&bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut found: BTreeMap<u8, usize> = BTreeMap::new();
    let keys: Vec<_> = pkg.tiles.keys().copied().collect();
    for key in keys {
        if wanted
            .iter()
            .all(|s| found.get(&s.code()).copied().unwrap_or(0) >= max)
        {
            break;
        }
        let Some(t) = ocw_render::package::build_tile_meshes(&pkg, key) else {
            continue;
        };
        for ch in t.plants.chunks.iter().chain(&t.plants.details) {
            for p in &ch.near.plants {
                if !wanted.contains(&p.sp) {
                    continue;
                }
                let n = found.entry(p.sp.code()).or_default();
                if *n >= max {
                    continue;
                }
                *n += 1;
                let b = p.at.base + bevy::math::Vec3::from(ch.origin);
                println!(
                    "{} {:.1} {:.1} {:.2} {:.2}",
                    p.sp.name().replace(' ', "-"),
                    b.x,
                    b.z,
                    b.y,
                    p.at.scale
                );
            }
        }
    }
    ExitCode::SUCCESS
}
