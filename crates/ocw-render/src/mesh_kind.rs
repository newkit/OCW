//! What each mesh on the GPU is (spec 14 Phase 10, 10G *The first tiles'
//! GPU memory*): the world's meshes are added with a kind ("terrain",
//! "houses near", "plants far", …) so the browser's GPU sum can say which
//! part of a tile takes how many bytes (`window.ocw.gpuMemory()`, the page
//! log's "gpu meshes by kind" line). Meshes added without one count as
//! "other". A small table of asset ids: the sum walks the render world's
//! meshes and looks their kind up here.

use std::collections::HashMap;
use std::sync::Mutex;

use bevy::prelude::*;

static KINDS: Mutex<Option<HashMap<AssetId<Mesh>, &'static str>>> = Mutex::new(None);

/// Adds `mesh` to `meshes` as a mesh of `kind`.
pub fn add(meshes: &mut Assets<Mesh>, mesh: Mesh, kind: &'static str) -> Handle<Mesh> {
    let h = meshes.add(mesh);
    tag(h.id(), kind);
    h
}

/// Marks the mesh `id` as one of `kind`.
pub fn tag(id: AssetId<Mesh>, kind: &'static str) {
    if let Ok(mut k) = KINDS.lock() {
        k.get_or_insert_with(HashMap::new).insert(id, kind);
    }
}

/// The kind of the mesh `id` ("other" when it was added without one).
pub fn kind_of(id: AssetId<Mesh>) -> &'static str {
    KINDS
        .lock()
        .ok()
        .and_then(|k| k.as_ref().and_then(|k| k.get(&id).copied()))
        .unwrap_or("other")
}

/// The kinds of `ids` in one lock, and the table cut to them (the meshes
/// dropped since the last sum leave it).
pub fn kinds_of(ids: &[AssetId<Mesh>]) -> Vec<&'static str> {
    let Ok(mut k) = KINDS.lock() else {
        return vec!["other"; ids.len()];
    };
    let table = k.get_or_insert_with(HashMap::new);
    let out: Vec<&'static str> = ids
        .iter()
        .map(|id| table.get(id).copied().unwrap_or("other"))
        .collect();
    if table.len() > ids.len() + 256 {
        let live: std::collections::HashSet<_> = ids.iter().collect();
        table.retain(|id, _| live.contains(id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (Ids of their own: the table is shared with the other tests, whose
    /// meshes get the same asset indices.)
    #[test]
    fn meshes_keep_their_kind_and_others_are_other() {
        let id = |n: u128| AssetId::<Mesh>::Uuid {
            uuid: bevy::asset::uuid::Uuid::from_u128(0x0c3a_6b1d_0000_0000_0000_0000_0000_0000 + n),
        };
        tag(id(1), "terrain");
        assert_eq!(kind_of(id(1)), "terrain");
        assert_eq!(kind_of(id(2)), "other");
        assert_eq!(kinds_of(&[id(2), id(1)]), ["other", "terrain"]);
        // `add` tags the mesh it adds.
        let mut meshes = Assets::<Mesh>::default();
        let h = add(&mut meshes, Mesh::from(Cuboid::default()), "roads");
        assert!(meshes.get(&h).is_some());
    }
}
