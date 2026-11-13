//! Bridge meshes (spec §4.8.6): the structures of a package's `BRDG`
//! section, built by `ocw_world::bridges::bridge_mesh` and coloured here
//! by material and part. [`bridge_colour`] is the one place that decides a
//! bridge's look: a material library can replace it (textures per
//! material and part) without touching the geometry.

use ocw_world::bridges::{Bridge, BridgeMaterial, BridgePart, bridge_mesh};
use ocw_world::package::{TileKey, WorldPackage};

use crate::package::MeshArrays;

fn lin(r: f32, g: f32, b: f32) -> [f32; 4] {
    let c = bevy::color::Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Plain colour (linear RGBA) of a bridge part of a material: the hook for
/// the material library.
pub fn bridge_colour(material: BridgeMaterial, part: BridgePart) -> [f32; 4] {
    let base = match material {
        BridgeMaterial::Concrete => (0.66, 0.65, 0.62),
        BridgeMaterial::Stone => (0.58, 0.54, 0.47),
        BridgeMaterial::Steel => (0.36, 0.40, 0.43),
        BridgeMaterial::Wood => (0.47, 0.33, 0.21),
    };
    let (r, g, b) = match part {
        // Railings of concrete bridges are steel; the rest is the material.
        BridgePart::Railing if material == BridgeMaterial::Concrete => (0.55, 0.57, 0.58),
        BridgePart::Frame if material == BridgeMaterial::Concrete => (0.36, 0.40, 0.43),
        BridgePart::Roof => (0.40, 0.26, 0.18),
        // Supports a little darker (weathered, damp).
        BridgePart::Pier | BridgePart::Abutment => (base.0 * 0.88, base.1 * 0.88, base.2 * 0.88),
        BridgePart::Arch => (base.0 * 0.92, base.1 * 0.92, base.2 * 0.92),
        _ => base,
    };
    lin(r, g, b)
}

/// The meshes of `bridges`, in package metres.
pub fn bridge_arrays(bridges: &[&Bridge]) -> MeshArrays {
    let mut m = MeshArrays::default();
    for b in bridges {
        let g = bridge_mesh(b);
        let base = m.positions.len() as u32;
        m.positions.extend_from_slice(&g.positions);
        m.normals.extend_from_slice(&g.normals);
        m.colors
            .extend(g.parts.iter().map(|&p| bridge_colour(b.material, p)));
        m.indices.extend(g.indices.iter().map(|i| i + base));
    }
    m
}

/// The bridges whose decks may reach into tile `key`: those of its tile and
/// the eight around it (a bridge belongs to the tile of its middle).
pub fn bridges_near(pkg: &WorldPackage, key: TileKey) -> Vec<&Bridge> {
    let Some(t) = &pkg.bridges else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            if let Some(v) = t.get(&TileKey {
                x: key.x + dx,
                z: key.z + dz,
            }) {
                out.extend(v.iter());
            }
        }
    }
    out
}

/// Whether a road strip point lies on a bridge deck (horizontally within
/// its half width plus its kerbs and within 0.3 m of its height): such
/// points get no edge faces into the terrain, the deck has its own sides.
pub fn on_deck(bridges: &[&Bridge], p: [f32; 3]) -> bool {
    bridges.iter().any(|b| {
        b.deck_near(p[0], p[2]).is_some_and(|(d, y)| {
            d <= b.width_m / 2.0 + ocw_world::bridges::KERB_M && (y - p[1]).abs() < 0.3
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::bridges::{BridgeSource, BridgeStructure, Pier, Railing};
    use ocw_world::graph::EdgeId;

    fn bridge(material: BridgeMaterial) -> Bridge {
        Bridge {
            edge: EdgeId(1),
            osm_way: 1,
            structure: BridgeStructure::Beam,
            material,
            source: BridgeSource::Tagged,
            railing: Railing::Parapet,
            width_m: 6.0,
            deck_m: 0.8,
            points: vec![[0.0, 10.0, 0.0], [20.0, 10.0, 0.0], [40.0, 10.0, 0.0]],
            abutment_foot_y: [5.0, 5.0],
            piers: vec![Pier {
                along_m: 20.0,
                foot_y: 0.0,
            }],
            arches: vec![],
        }
    }

    #[test]
    fn bridges_are_coloured_by_material_and_part() {
        let (a, b) = (bridge(BridgeMaterial::Stone), bridge(BridgeMaterial::Steel));
        let m = bridge_arrays(&[&a, &b]);
        assert_eq!(m.positions.len(), m.colors.len());
        assert_eq!(m.positions.len(), m.normals.len());
        assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
        // The second bridge's indices are offset past the first's vertices.
        let one = bridge_arrays(&[&a]);
        assert_eq!(m.indices.len(), 2 * one.indices.len());
        assert_ne!(
            bridge_colour(BridgeMaterial::Stone, BridgePart::Deck),
            bridge_colour(BridgeMaterial::Wood, BridgePart::Deck)
        );
        assert_ne!(
            bridge_colour(BridgeMaterial::Concrete, BridgePart::Deck),
            bridge_colour(BridgeMaterial::Concrete, BridgePart::Railing)
        );
    }

    #[test]
    fn strip_points_on_the_deck_are_found() {
        let a = bridge(BridgeMaterial::Concrete);
        assert!(on_deck(&[&a], [10.0, 10.0, 2.0]));
        assert!(!on_deck(&[&a], [10.0, 10.0, 6.0]), "beside the deck");
        assert!(!on_deck(&[&a], [10.0, 4.0, 0.0]), "a road under the bridge");
        assert!(!on_deck(&[], [10.0, 10.0, 0.0]));
    }
}
