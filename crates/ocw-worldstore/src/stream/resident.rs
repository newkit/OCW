//! The tiles kept in memory around the rider (spec §4.5, §14 Phase 10
//! B2: the memory budget of the browser app). Tiles within
//! [`Residency::fetch_radius`] of the rider's tile are fetched; tiles
//! beyond [`Residency::keep_radius`] are dropped (they stay in the chunk
//! cache), and while the decoded tiles exceed [`Residency::budget_bytes`]
//! the farthest ones beyond the fetch radius go too. The renderer streams
//! its meshes from the resident tiles (`ocw-render`: 5 × 5 around the
//! rider), so the fetch radius must cover its radius plus one.

use std::collections::{BTreeMap, BTreeSet};

use ocw_world::manifest::FileEntry;
use ocw_world::package::{StreamedPackage, Tile, TileKey, WorldPackage};

use super::plan::{tile_chunks, tiles_around};

/// Which tiles stay in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Residency {
    /// Tiles up to this ring around the rider's tile are fetched.
    pub fetch_radius: i32,
    /// Tiles beyond this ring are dropped.
    pub keep_radius: i32,
    /// Decoded tiles over this many bytes: the farthest beyond the fetch
    /// radius are dropped (the fetch ring itself is always kept).
    pub budget_bytes: u64,
}

impl Default for Residency {
    fn default() -> Self {
        Self {
            // The renderer meshes 5 × 5 (radius 2) and keeps radius 3.
            fetch_radius: 3,
            keep_radius: 5,
            budget_bytes: 96 << 20,
        }
    }
}

/// What to change around a position.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidencyPlan {
    /// Tiles to fetch, nearest first.
    pub fetch: Vec<TileKey>,
    /// Tiles to drop.
    pub drop: Vec<TileKey>,
}

impl Residency {
    /// The plan at `centre` for `available` tiles, of which `resident`
    /// (with their sizes in bytes) are loaded and `pending` are on their
    /// way.
    pub fn plan<V>(
        &self,
        available: &BTreeMap<TileKey, V>,
        resident: &BTreeMap<TileKey, u64>,
        pending: &BTreeSet<TileKey>,
        centre: TileKey,
    ) -> ResidencyPlan {
        let fetch = tiles_around(available, centre, self.fetch_radius)
            .into_iter()
            .filter(|k| !resident.contains_key(k) && !pending.contains(k))
            .collect();
        let mut drop: Vec<TileKey> = resident
            .keys()
            .filter(|k| k.ring(centre) > self.keep_radius)
            .copied()
            .collect();
        let mut total: u64 = resident
            .iter()
            .filter(|(k, _)| !drop.contains(k))
            .map(|(_, b)| b)
            .sum();
        if total > self.budget_bytes {
            let mut far: Vec<(&TileKey, &u64)> = resident
                .iter()
                .filter(|(k, _)| k.ring(centre) > self.fetch_radius && !drop.contains(k))
                .collect();
            // Farthest first (then by key, for a stable order).
            far.sort_by_key(|(k, _)| {
                let (dx, dz) = (k.x - centre.x, k.z - centre.z);
                (std::cmp::Reverse(dx * dx + dz * dz), **k)
            });
            for (k, b) in far {
                if total <= self.budget_bytes {
                    break;
                }
                total -= b;
                drop.push(*k);
            }
        }
        ResidencyPlan { fetch, drop }
    }
}

/// A streamed package's resident tiles.
#[derive(Debug, Clone)]
pub struct ResidentTiles {
    package: StreamedPackage,
    /// Chunk index of every tile of the package.
    chunks: BTreeMap<TileKey, usize>,
    tiles: BTreeMap<TileKey, Tile>,
    sizes: BTreeMap<TileKey, u64>,
    pending: BTreeSet<TileKey>,
    pub policy: Residency,
    peak_bytes: u64,
    /// Tiles came or went since the last [`ResidentTiles::take_package`].
    dirty: bool,
    pub loaded_total: u64,
    pub dropped_total: u64,
}

impl ResidentTiles {
    pub fn new(package: StreamedPackage, file: &FileEntry, policy: Residency) -> Self {
        Self {
            package,
            chunks: tile_chunks(file),
            tiles: BTreeMap::new(),
            sizes: BTreeMap::new(),
            pending: BTreeSet::new(),
            policy,
            peak_bytes: 0,
            dirty: true,
            loaded_total: 0,
            dropped_total: 0,
        }
    }

    pub fn streamed(&self) -> &StreamedPackage {
        &self.package
    }

    /// The tile under `p` (package metres).
    pub fn tile_at(&self, p: [f64; 2]) -> TileKey {
        TileKey::containing(p, f64::from(self.package.head.tile_size_m))
    }

    /// Plans around `p`: drops what is too far (at once) and returns the
    /// tiles to fetch with their chunk indices, now marked pending.
    pub fn update(&mut self, p: [f64; 2]) -> Vec<(TileKey, usize)> {
        let plan = self
            .policy
            .plan(&self.chunks, &self.sizes, &self.pending, self.tile_at(p));
        for k in &plan.drop {
            self.tiles.remove(k);
            self.sizes.remove(k);
            self.dropped_total += 1;
            self.dirty = true;
        }
        plan.fetch
            .into_iter()
            .map(|k| {
                self.pending.insert(k);
                (k, self.chunks[&k])
            })
            .collect()
    }

    /// A fetched (verified) tile chunk: decoded and kept, unless the
    /// rider moved on so far that it would be dropped at once.
    pub fn arrive(&mut self, key: TileKey, bytes: &[u8]) -> Result<(), String> {
        self.pending.remove(&key);
        let tile = self.package.read_tile(bytes)?;
        if tile.key != key {
            return Err(format!(
                "tile chunk {},{} holds tile {},{}",
                key.x, key.z, tile.key.x, tile.key.z
            ));
        }
        self.sizes.insert(key, tile.approx_bytes());
        self.tiles.insert(key, tile);
        self.loaded_total += 1;
        self.dirty = true;
        self.peak_bytes = self.peak_bytes.max(self.resident_bytes());
        Ok(())
    }

    /// A tile that could not be fetched (it may be asked for again).
    pub fn failed(&mut self, key: TileKey) {
        self.pending.remove(&key);
    }

    pub fn is_resident(&self, key: TileKey) -> bool {
        self.tiles.contains_key(&key)
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Bytes of the decoded resident tiles.
    pub fn resident_bytes(&self) -> u64 {
        self.sizes.values().sum()
    }

    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// The tiles of the package (resident or not).
    pub fn tile_count(&self) -> usize {
        self.chunks.len()
    }

    /// All tiles up to `radius` around `p` are resident (the start of a
    /// ride waits for them).
    pub fn ready_around(&self, p: [f64; 2], radius: i32) -> bool {
        tiles_around(&self.chunks, self.tile_at(p), radius)
            .iter()
            .all(|k| self.tiles.contains_key(k))
    }

    /// The package with the resident tiles, if they changed.
    pub fn take_package(&mut self) -> Option<WorldPackage> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        Some(self.package.package(self.tiles.values().cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::load::tests::published;
    use crate::stream::plan::base_chunks;

    fn key(x: i32, z: i32) -> TileKey {
        TileKey { x, z }
    }

    #[test]
    fn the_plan_fetches_the_ring_and_drops_beyond_the_keep_radius() {
        let available: BTreeMap<TileKey, ()> = (-6..=6)
            .flat_map(|x| (-6..=6).map(move |z| (key(x, z), ())))
            .collect();
        let policy = Residency {
            fetch_radius: 1,
            keep_radius: 2,
            budget_bytes: u64::MAX,
        };
        let empty = BTreeMap::new();
        let p = policy.plan(&available, &empty, &BTreeSet::new(), key(0, 0));
        assert_eq!(p.fetch.len(), 9);
        assert_eq!(p.fetch[0], key(0, 0));
        assert!(p.drop.is_empty());
        // Moved three tiles east: the columns beyond ring 2 go, the
        // new ring comes; pending tiles are not asked for twice.
        let resident: BTreeMap<TileKey, u64> = p.fetch.iter().map(|k| (*k, 100)).collect();
        let pending: BTreeSet<TileKey> = [key(3, 0)].into();
        let p = policy.plan(&available, &resident, &pending, key(3, 0));
        assert_eq!(p.fetch.len(), 8, "{p:?}");
        assert!(!p.fetch.contains(&key(3, 0)));
        let mut dropped = p.drop.clone();
        dropped.sort();
        assert_eq!(
            dropped,
            [
                key(-1, -1),
                key(-1, 0),
                key(-1, 1),
                key(0, -1),
                key(0, 0),
                key(0, 1)
            ]
        );
    }

    /// Memory budget eviction: over the budget, the farthest tiles beyond
    /// the fetch ring go first; the fetch ring stays even over budget.
    #[test]
    fn the_memory_budget_drops_the_farthest_tiles() {
        let available: BTreeMap<TileKey, ()> = (-4..=4)
            .flat_map(|x| (-4..=4).map(move |z| (key(x, z), ())))
            .collect();
        let resident: BTreeMap<TileKey, u64> = available.keys().map(|k| (*k, 10)).collect();
        let policy = Residency {
            fetch_radius: 1,
            keep_radius: 4,
            budget_bytes: 300,
        };
        let p = policy.plan(&available, &resident, &BTreeSet::new(), key(0, 0));
        // 81 tiles of 10 bytes, 300 allowed: 51 go, the farthest first
        // (all 4 corners of ring 4 among them), never ring ≤ 1.
        assert_eq!(p.drop.len(), 51);
        for c in [key(4, 4), key(-4, 4), key(4, -4), key(-4, -4)] {
            assert!(p.drop.contains(&c));
        }
        assert!(p.drop.iter().all(|k| k.ring(key(0, 0)) > 1));
        // A budget below the fetch ring: only what is beyond it goes.
        let tiny = Residency {
            budget_bytes: 1,
            ..policy
        };
        let p = tiny.plan(&available, &resident, &BTreeSet::new(), key(0, 0));
        assert_eq!(p.drop.len(), 81 - 9);
    }

    #[test]
    fn resident_tiles_follow_the_rider() {
        let (bytes, m) = published();
        let file = m.package_file().unwrap();
        let base: Vec<u8> = base_chunks(file)[1..]
            .iter()
            .flat_map(|&i| {
                let c = &file.chunks[i];
                bytes[c.offset as usize..(c.offset + c.size) as usize].to_vec()
            })
            .collect();
        let header = &bytes[..file.chunks[0].size as usize];
        let s = StreamedPackage::from_parts(header, &base).unwrap();
        let mut r = ResidentTiles::new(
            s,
            file,
            Residency {
                fetch_radius: 1,
                keep_radius: 1,
                budget_bytes: u64::MAX,
            },
        );
        // The package is 4 × 3 tiles (x 0..3, z 0..2); at tile 0,0.
        let want = r.update([100.0, 100.0]);
        assert_eq!(want.len(), 4);
        assert_eq!(r.pending(), 4);
        assert!(r.update([100.0, 100.0]).is_empty(), "asked once");
        for (k, i) in &want {
            let c = &file.chunks[*i];
            r.arrive(*k, &bytes[c.offset as usize..(c.offset + c.size) as usize])
                .unwrap();
        }
        assert!(r.ready_around([100.0, 100.0], 1));
        assert_eq!(r.len(), 4);
        let pkg = r.take_package().unwrap();
        assert_eq!(pkg.tiles.len(), 4);
        assert!(r.take_package().is_none(), "unchanged");
        let full = WorldPackage::from_bytes(&bytes).unwrap();
        assert_eq!(pkg.tiles[&key(1, 1)], full.tiles[&key(1, 1)]);
        let peak = r.peak_bytes();
        assert_eq!(peak, r.resident_bytes());
        // Three tiles east: x 0..1 are dropped, x 2..3 come.
        let want = r.update([3100.0, 100.0]);
        assert_eq!(r.len(), 0, "ring 2 and 3 are beyond keep 1");
        assert!(r.take_package().unwrap().tiles.is_empty());
        assert_eq!(want.len(), 4);
        assert_eq!(r.dropped_total, 4);
        assert_eq!(r.peak_bytes(), peak);
        // A chunk that is not the tile it should be is refused.
        let (k, _) = want[0];
        let other = want[1].1;
        let c = &file.chunks[other];
        assert!(
            r.arrive(k, &bytes[c.offset as usize..(c.offset + c.size) as usize])
                .is_err()
        );
        r.failed(k);
        assert_eq!(r.tile_count(), 12);
    }
}
