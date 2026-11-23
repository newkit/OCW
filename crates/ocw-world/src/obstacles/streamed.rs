//! Street lamp poles of a streamed package (spec 14 Phase 12, K3; the
//! browser): its base has no tiles, so the poles the apps place from the
//! tiles' strips ([`crate::lamps::tile_lamps`]) come as the tiles stream in
//! and go with them. The server's session and the client's prediction
//! share the [`super::WorldObstacles`] (in an `Arc`), so both have a pole
//! from the same tick on; each tile the poles stand in has a version, and
//! a cached spatial index of an older version is built again.

use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::lamps::Lamp;
use crate::package::TileKey;

/// A pole: foot (x, z) and the yaw of its model.
pub type Pole = ([f32; 2], f32);

#[derive(Debug, Clone, Default, PartialEq)]
struct Inner {
    /// Per tile a pole stands in: per tile whose strips placed it, its
    /// poles (in placement order; a strip's lamps can stand a few metres
    /// into the next tile).
    by_foot: BTreeMap<TileKey, BTreeMap<TileKey, Vec<Pole>>>,
    /// Per placing tile the tiles its poles stand in.
    sources: BTreeMap<TileKey, Vec<TileKey>>,
    /// Per tile poles stand (or stood) in: its version, bumped at every
    /// change.
    versions: BTreeMap<TileKey, u64>,
    next: u64,
}

/// The streamed poles (see the module).
#[derive(Default)]
pub struct StreamedLamps {
    inner: RwLock<Inner>,
}

impl StreamedLamps {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets the poles placed by tile `source` (replacing its earlier ones);
    /// `yaw` turns a lamp into its model's yaw.
    pub fn set(&self, source: TileKey, lamps: &[Lamp], size_m: f64, yaw: impl Fn(&Lamp) -> f32) {
        let mut w = self.write();
        drop_source(&mut w, source);
        let mut feet: Vec<TileKey> = Vec::new();
        for l in lamps {
            let at = [f64::from(l.foot[0]), f64::from(l.foot[2])];
            let key = TileKey::containing(at, size_m);
            w.by_foot
                .entry(key)
                .or_default()
                .entry(source)
                .or_default()
                .push(([l.foot[0], l.foot[2]], yaw(l)));
            if !feet.contains(&key) {
                feet.push(key);
            }
        }
        for k in &feet {
            bump(&mut w, *k);
        }
        // A tile without poles is remembered too: it is not placed again.
        w.sources.insert(source, feet);
    }

    /// Drops the poles placed by tile `source` (it left the memory).
    pub fn drop_source(&self, source: TileKey) {
        drop_source(&mut self.write(), source);
    }

    /// The tiles whose poles were placed.
    pub fn sources(&self) -> Vec<TileKey> {
        self.read().sources.keys().copied().collect()
    }

    pub fn has_source(&self, source: TileKey) -> bool {
        self.read().sources.contains_key(&source)
    }

    /// The poles standing in tile `key`, by placing tile (the order of a
    /// whole package's [`crate::lamps::package_lamps`]).
    pub fn poles_in(&self, key: TileKey) -> Vec<Pole> {
        self.read()
            .by_foot
            .get(&key)
            .map(|m| m.values().flatten().copied().collect())
            .unwrap_or_default()
    }

    /// The tiles poles stand in.
    pub fn foot_tiles(&self) -> Vec<TileKey> {
        self.read().by_foot.keys().copied().collect()
    }

    /// The version of tile `key`'s poles (0: never had any).
    pub fn version(&self, key: TileKey) -> u64 {
        self.read().versions.get(&key).copied().unwrap_or(0)
    }

    pub fn count(&self) -> usize {
        self.read()
            .by_foot
            .values()
            .flat_map(|m| m.values())
            .map(Vec::len)
            .sum()
    }

    /// Approximate memory (bytes).
    pub fn approx_bytes(&self) -> usize {
        let r = self.read();
        r.by_foot
            .values()
            .flat_map(|m| m.values())
            .map(|v| v.capacity() * std::mem::size_of::<Pole>() + 48)
            .sum::<usize>()
            + r.sources.values().map(|v| v.len() * 8 + 48).sum::<usize>()
            + r.versions.len() * 24
    }
}

fn bump(w: &mut Inner, key: TileKey) {
    w.next += 1;
    let v = w.next;
    w.versions.insert(key, v);
}

fn drop_source(w: &mut Inner, source: TileKey) {
    let Some(feet) = w.sources.remove(&source) else {
        return;
    };
    for k in feet {
        if let Some(m) = w.by_foot.get_mut(&k) {
            m.remove(&source);
            if m.is_empty() {
                w.by_foot.remove(&k);
            }
        }
        bump(w, k);
    }
}

impl Clone for StreamedLamps {
    fn clone(&self) -> Self {
        Self {
            inner: RwLock::new(self.read().clone()),
        }
    }
}

impl PartialEq for StreamedLamps {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || self.read().by_foot == other.read().by_foot
    }
}

impl std::fmt::Debug for StreamedLamps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamedLamps")
            .field("sources", &self.read().sources.len())
            .field("poles", &self.count())
            .finish()
    }
}
