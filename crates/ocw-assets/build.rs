//! Generates the embedded model table from `artwork/3dmodels/manifest.toml`
//! (spec §14 Phase 15, 15D): one `include_bytes!` per listed file, so
//! release builds, the AppImage and the browser need no asset paths.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("../../artwork/3dmodels");
    let manifest = dir.join("manifest.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    // The `file = "…"` lines of the [[asset]] tables (the full parse and
    // its checks are the library's and the validator's).
    let mut files: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("file")?.trim_start().strip_prefix('=')?;
            let v = rest.trim();
            let v = v.strip_prefix('"')?.strip_suffix('"')?;
            Some(v.to_owned())
        })
        .collect();
    files.sort();
    files.dedup();
    let mut out = String::from(
        "/// Every model file of the manifest: (path in artwork/3dmodels, bytes).\n\
         pub static FILES: &[(&str, &[u8])] = &[\n",
    );
    for f in &files {
        assert!(
            !f.contains("..") && !f.starts_with('/') && !f.contains('\\'),
            "manifest file `{f}`: a plain path below artwork/3dmodels"
        );
        let path = dir.join(f);
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            // Not embedded: the library reports it missing (and the export
            // can write it: `ocw-assets model export-all`).
            println!("cargo:warning={f}: listed in the manifest, but missing");
            continue;
        }
        let full = path;
        if std::fs::read(&full).is_ok_and(|b| b.starts_with(b"version https://git-lfs")) {
            println!(
                "cargo:warning={f} is a Git LFS pointer, not the model: run `git lfs pull` (spec §14 Phase 15, Q2)"
            );
        }
        writeln!(
            out,
            "    ({f:?}, include_bytes!({:?})),",
            full.display().to_string()
        )
        .expect("write");
    }
    out.push_str("];\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("models.rs");
    std::fs::write(&dest, out).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
}
