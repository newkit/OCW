//! Range planning (spec §4.7, §14 Phase 10 B2): which chunks of a package
//! file a client needs, and which `Range` requests fetch them.
//!
//! A package file is `header`, the tiles (`tile X,Z`, ordered by key),
//! then `sections` and the `section TAG` chunks (road graph, routes, land
//! use, buildings …; [`ocw_world::manifest::package_chunks`]). The base –
//! header and sections – is needed before the ride; the tiles around the
//! rider come on demand. Neighbouring chunks are merged into one request
//! when the gap between them is small.

use std::collections::BTreeMap;
use std::ops::Range;

use ocw_world::TileKey;
use ocw_world::manifest::FileEntry;

/// How chunks are merged into range requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeParams {
    /// Chunks this many bytes apart (or closer) go into one request; the
    /// gap is fetched and thrown away.
    pub max_gap: u64,
    /// A merged request stops growing at this size (one chunk may be
    /// larger on its own).
    pub max_len: u64,
}

impl Default for RangeParams {
    fn default() -> Self {
        Self {
            // About one tile: fetching it beats another round trip.
            max_gap: 64 << 10,
            max_len: 4 << 20,
        }
    }
}

/// One `Range: bytes=start-(end-1)` request and the chunks in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRequest {
    pub range: Range<u64>,
    /// Indices into the file's `chunks`, by offset.
    pub chunks: Vec<usize>,
}

impl RangeRequest {
    pub fn len(&self) -> u64 {
        self.range.end - self.range.start
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

/// The tile key of a `tile X,Z` chunk.
pub fn tile_key(chunk_name: &str) -> Option<TileKey> {
    let (x, z) = chunk_name.strip_prefix("tile ")?.split_once(',')?;
    Some(TileKey {
        x: x.trim().parse().ok()?,
        z: z.trim().parse().ok()?,
    })
}

/// The chunks every client needs first: `header`, `sections` and every
/// `section …` (graph, routes, land use, buildings, …), by offset.
pub fn base_chunks(file: &FileEntry) -> Vec<usize> {
    let mut v: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| {
            let n = &file.chunks[i].name;
            n == "header" || n == "sections" || n.starts_with("section ")
        })
        .collect();
    v.sort_by_key(|&i| file.chunks[i].offset);
    v
}

/// The tile chunks by key.
pub fn tile_chunks(file: &FileEntry) -> BTreeMap<TileKey, usize> {
    file.chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| tile_key(&c.name).map(|k| (k, i)))
        .collect()
}

/// The tiles of `tiles` within `radius` rings (Chebyshev) of `centre`,
/// nearest first (by ring, then by distance from the centre tile).
pub fn tiles_around<V>(tiles: &BTreeMap<TileKey, V>, centre: TileKey, radius: i32) -> Vec<TileKey> {
    let mut out: Vec<TileKey> = Vec::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let k = TileKey {
                x: centre.x + dx,
                z: centre.z + dz,
            };
            if tiles.contains_key(&k) {
                out.push(k);
            }
        }
    }
    out.sort_by_key(|k| {
        let (dx, dz) = (k.x - centre.x, k.z - centre.z);
        (k.ring(centre), dx * dx + dz * dz, k.x, k.z)
    });
    out
}

/// The tile chunks a client needs at `p` (package metres): those within
/// `radius` of the tile under `p`, nearest first (indices into `chunks`).
pub fn chunks_for_position(
    file: &FileEntry,
    tile_size_m: f64,
    p: [f64; 2],
    radius: i32,
) -> Vec<usize> {
    let tiles = tile_chunks(file);
    let centre = TileKey::containing(p, tile_size_m);
    tiles_around(&tiles, centre, radius)
        .into_iter()
        .map(|k| tiles[&k])
        .collect()
}

/// Merges the `wanted` chunks of `file` into range requests (sorted by
/// offset; duplicates ignored).
pub fn plan_ranges(file: &FileEntry, wanted: &[usize], params: RangeParams) -> Vec<RangeRequest> {
    let mut idx: Vec<usize> = wanted
        .iter()
        .copied()
        .filter(|&i| i < file.chunks.len())
        .collect();
    idx.sort_by_key(|&i| file.chunks[i].offset);
    idx.dedup();
    let mut out: Vec<RangeRequest> = Vec::new();
    for i in idx {
        let c = &file.chunks[i];
        let (start, end) = (c.offset, c.offset + c.size);
        if let Some(last) = out.last_mut()
            && start >= last.range.end
            && start - last.range.end <= params.max_gap
            && end - last.range.start <= params.max_len
        {
            last.range.end = end;
            last.chunks.push(i);
            continue;
        }
        out.push(RangeRequest {
            range: start..end,
            chunks: vec![i],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::manifest::Chunk;

    /// header (0..50), tiles x 0..3 × z 0..3 of 1000 bytes each, sections.
    pub(crate) fn file() -> FileEntry {
        let mut chunks = vec![chunk("header", 0, 50)];
        let mut at = 50;
        for x in 0..4 {
            for z in 0..4 {
                chunks.push(chunk(&format!("tile {x},{z}"), at, 1000));
                at += 1000;
            }
        }
        chunks.push(chunk("sections", at, 4));
        chunks.push(chunk("section GRPH", at + 4, 5000));
        chunks.push(chunk("section LUSE", at + 5004, 3000));
        FileEntry {
            path: "w.ocwpkg".into(),
            size: at + 8004,
            sha256: ocw_world::manifest::sha256(b""),
            chunks,
        }
    }

    fn chunk(name: &str, offset: u64, size: u64) -> Chunk {
        Chunk {
            name: name.into(),
            offset,
            size,
            sha256: ocw_world::manifest::sha256(name.as_bytes()),
        }
    }

    #[test]
    fn chunk_names() {
        assert_eq!(tile_key("tile 3,-4"), Some(TileKey { x: 3, z: -4 }));
        assert_eq!(tile_key("tile x,1"), None);
        assert_eq!(tile_key("section GRPH"), None);
        let f = file();
        let names: Vec<&str> = base_chunks(&f)
            .iter()
            .map(|&i| f.chunks[i].name.as_str())
            .collect();
        assert_eq!(
            names,
            ["header", "sections", "section GRPH", "section LUSE"]
        );
        assert_eq!(tile_chunks(&f).len(), 16);
    }

    /// Which chunks for a position: the 3 × 3 tiles around it (fewer at
    /// the package's edge), the tile under the rider first.
    #[test]
    fn the_tiles_around_a_position() {
        let f = file();
        let near = chunks_for_position(&f, 1000.0, [1500.0, 2500.0], 1);
        let names: Vec<&str> = near.iter().map(|&i| f.chunks[i].name.as_str()).collect();
        assert_eq!(names.len(), 9);
        assert_eq!(names[0], "tile 1,2");
        assert!(
            names[1..5]
                .iter()
                .all(|n| { ["tile 0,2", "tile 2,2", "tile 1,1", "tile 1,3"].contains(n) })
        );
        // In the corner: only 4 of them exist; far away: none.
        assert_eq!(chunks_for_position(&f, 1000.0, [10.0, 10.0], 1).len(), 4);
        assert!(chunks_for_position(&f, 1000.0, [-9000.0, 0.0], 2).is_empty());
        // Negative coordinates floor into the right tile.
        assert_eq!(
            TileKey::containing([-0.5, 0.0], 1000.0),
            TileKey { x: -1, z: 0 }
        );
    }

    #[test]
    fn neighbouring_chunks_share_a_request() {
        let f = file();
        let params = RangeParams {
            max_gap: 1000,
            max_len: 3500,
        };
        // Tiles 0,0 0,1 0,2 are adjacent, 0,3 too but past max_len; 2,0 is
        // three tiles further (gap 3000 > 1000).
        let t = tile_chunks(&f);
        let want: Vec<usize> = ["0,0", "0,1", "0,2", "0,3", "1,1", "3,0"]
            .iter()
            .map(|k| {
                let (x, z) = k.split_once(',').unwrap();
                t[&TileKey {
                    x: x.parse().unwrap(),
                    z: z.parse().unwrap(),
                }]
            })
            .collect();
        let plan = plan_ranges(&f, &want, params);
        let ranges: Vec<Range<u64>> = plan.iter().map(|r| r.range.clone()).collect();
        assert_eq!(ranges, [50..3050, 3050..6050, 12050..13050], "{plan:?}");
        assert_eq!(plan[0].chunks.len(), 3);
        // 0,3 and 1,1 (gap of one tile) share the second request.
        assert_eq!(plan[1].chunks.len(), 2);
        // The base: header alone, the sections in one request.
        let base = plan_ranges(
            &f,
            &base_chunks(&f),
            RangeParams {
                max_len: 1 << 20,
                ..params
            },
        );
        assert_eq!(base.len(), 2, "{base:?}");
        assert_eq!(base[0].range, 0..50);
        assert_eq!(base[1].range, 16050..24054);
        assert_eq!(base[1].len(), 8004);
        // Duplicates and bad indices are ignored.
        assert_eq!(plan_ranges(&f, &[1, 1, 999], params).len(), 1);
        assert!(plan_ranges(&f, &[], params).is_empty());
    }
}
