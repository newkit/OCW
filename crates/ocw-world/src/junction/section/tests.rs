use super::*;
use crate::graph::RoadClass;
use crate::junction::fixtures::{
    crossing, dual_carriageway_t, fork, split, t_junction, t_junction_at,
};

fn fixtures() -> Vec<RoadGraph> {
    vec![
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        t_junction(RoadClass::Residential, -0.03, 0.12),
        crossing([0.05, -0.04, 0.10, 0.0]),
        fork(40.0),
        fork(10.0),
        split(12.0, 6.0, 6.0, 0.04),
        split(45.0, 6.0, 6.0, 0.04),
        split(5.0, 7.5, 5.0, 0.02),
        dual_carriageway_t(),
        t_junction_at(0.10, -0.15, 30.0),
    ]
}

/// A stored index gives the very same rules as computing them: every
/// patch to the bit (arms, outline, mesh), every mitre, no rule where
/// none is computed.
#[test]
fn stored_rules_are_the_computed_ones() {
    for g in fixtures() {
        let computed = JunctionSet::new(&g);
        let index = JunctionIndex::of_set(&g, &computed, FLAG_SETTLED);
        let mut bytes = Vec::new();
        write_junctions(&index, &mut bytes);
        let back = read_junctions(&bytes).unwrap();
        assert_eq!(back, index);
        let stored = JunctionSet::with_index(&g, Some(Arc::new(back)));
        assert!(stored.is_stored());
        for v in 0..g.vertices.len() {
            assert_eq!(stored.rule(&g, v), computed.rule(&g, v), "vertex {v}");
        }
        assert!(index.settled());
    }
}

/// An index made for another graph (other heights: another fingerprint)
/// is not used: the junctions are computed.
#[test]
fn an_index_of_another_graph_is_ignored() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let index = Arc::new(JunctionIndex::of_set(&g, &JunctionSet::new(&g), 0));
    let mut other = g.clone();
    other.edges[2].profile[3] += 0.5;
    let set = JunctionSet::with_index(&other, Some(index.clone()));
    assert!(!set.is_stored());
    assert!(JunctionSet::with_index(&g, Some(index)).is_stored());
    let data = JunctionData {
        index: None,
        strip_edges: Some(StripEdges {
            fingerprint: g.fingerprint(),
            tiles: BTreeMap::new(),
        }),
        mini_roundabouts: None,
    };
    assert!(data.strip_edges_for(&g).is_some());
    assert!(data.strip_edges_for(&other).is_none());
}

/// Newer layouts are refused as "not supported" (the package reader skips
/// them); records of unknown kinds and bytes after a record's fields are
/// skipped.
#[test]
fn newer_layouts_kinds_and_fields_are_skipped() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let index = JunctionIndex::of_set(&g, &JunctionSet::new(&g), 0);
    assert_eq!(index.patches(), 1);
    let mut bytes = Vec::new();
    write_junctions(&index, &mut bytes);
    let mut newer = bytes.clone();
    newer[0] = 4;
    assert!(
        read_junctions(&newer)
            .unwrap_err()
            .contains("not supported")
    );
    // The patch record with two extra bytes, then a record of kind 7 (from
    // a newer builder) at a later vertex.
    let mut b = bytes[..14].to_vec();
    b.extend(2u32.to_le_bytes());
    let rec = &bytes[18..];
    let len = u16::from_le_bytes([rec[0], rec[1]]) as usize;
    let mut body = rec[2..2 + len].to_vec();
    let v = u32::from_le_bytes(body[0..4].try_into().unwrap());
    body.extend([9, 9]);
    b.extend(((len + 2) as u16).to_le_bytes());
    b.extend(body);
    b.extend(5u16.to_le_bytes());
    b.extend((v + 1).to_le_bytes());
    b.push(7);
    let read = read_junctions(&b).unwrap();
    assert_eq!(read.rules.len(), 1);
    assert_eq!(read.rules[0], index.rules[0]);
    let mut s = Vec::new();
    write_strip_edges(&StripEdges::default(), &mut s);
    s[0] = 9;
    assert!(read_strip_edges(&s).unwrap_err().contains("not supported"));
}

#[test]
fn strip_edges_round_trip_and_check_the_count() {
    let mut tiles = BTreeMap::new();
    tiles.insert(TileKey { x: -1, z: 2 }, vec![3, NO_EDGE, 0]);
    tiles.insert(TileKey { x: 0, z: 0 }, vec![]);
    let s = StripEdges {
        fingerprint: 0x1234,
        tiles,
    };
    let mut b = Vec::new();
    write_strip_edges(&s, &mut b);
    let back = read_strip_edges(&b).unwrap();
    assert_eq!(back, s);
    assert_eq!(
        back.tile(TileKey { x: -1, z: 2 }, 3),
        Some(&[3, NO_EDGE, 0][..])
    );
    assert_eq!(back.tile(TileKey { x: -1, z: 2 }, 4), None);
    assert_eq!(back.tile(TileKey { x: 5, z: 5 }, 0), None);
}
