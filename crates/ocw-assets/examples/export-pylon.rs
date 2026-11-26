//! Writes the lattice pylon (`ocw_assets::pylon`) into the model folder:
//!
//! ```text
//! cargo run -p ocw-assets --example export-pylon [-- artwork/3dmodels]
//! ```
//!
//! The manifest entry is `ocw_assets::pylon::entry()` (added by hand next to
//! the other props: the props' files are the source, their folder is not
//! the exporter's).

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "artwork/3dmodels".into());
    let path = std::path::Path::new(&dir).join(ocw_assets::pylon::FILE);
    let bytes = ocw_assets::glb::write(&ocw_assets::pylon::lattice());
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("{}: {} bytes", path.display(), bytes.len());
}
