//! Lists every vertex of a package's road graph that a junction surface
//! covers (spec 4.8.8): `VERTEX patch` for a patch drawn there, `VERTEX
//! member FIRST` for a vertex covered by another vertex's patch (a
//! cluster's member, a way split an arm runs on past), for comparing two
//! builds (`diff`: no junction loses its surface).
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example surface_list -- PKG.ocwpkg > OUT.txt
//! ```

use ocw_world::WorldPackage;
use ocw_world::junction::JunctionSet;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: surface_list PKG.ocwpkg");
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).expect("package")).expect("pkg");
    let g = pkg.graph.clone().expect("road graph");
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    for v in 0..g.vertices.len() {
        if let Some(p) = js.patch(&g, v) {
            if p.is_at(v) {
                println!("{v} patch");
            } else {
                println!("{v} member {}", p.vertex);
            }
        }
    }
}
