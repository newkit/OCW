//! The chunk cache of streamed packages (spec §4.7, §14 Phase 10 B2):
//! verified chunks stored per package id, version and chunk name behind a
//! [`ChunkStore`] – the browser's Origin Private File System in the web
//! app, [`MemoryStore`] in tests – with an index of sizes and last use,
//! and least-recently-used eviction when the stored bytes exceed a budget.
//!
//! A cached chunk is checked against the manifest's SHA-256 again when it
//! is read: a damaged or stale file is removed and fetched anew. Store
//! errors (no OPFS, quota exceeded) never fail a ride: the chunk is then
//! simply not cached.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ocw_world::manifest::{Chunk, sha256};
use serde::{Deserialize, Serialize};

/// Where the cache keeps its index.
pub const INDEX_FILE: &str = "ocw-index.json";
/// Default size budget of the cache (bytes): a few regions.
pub const DEFAULT_BUDGET: u64 = 1 << 30;

/// Files by name in one flat directory (async: OPFS is).
#[allow(async_fn_in_trait)] // used single-threaded (wasm) and in tests
pub trait ChunkStore {
    /// The file's bytes, `None` if there is none.
    async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String>;
    /// Creates or replaces the file.
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// Removes the file (no error if there is none).
    async fn remove(&self, name: &str) -> Result<(), String>;
}

/// A [`ChunkStore`] in memory (tests; a browser without OPFS). Writes
/// can be made to fail (a full disk).
#[derive(Debug, Default)]
pub struct MemoryStore {
    pub files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub fail_writes: std::sync::atomic::AtomicBool,
}

impl MemoryStore {
    pub fn names(&self) -> Vec<String> {
        self.files
            .lock()
            .expect("not poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

impl ChunkStore for MemoryStore {
    async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.files.lock().expect("not poisoned").get(name).cloned())
    }
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if self.fail_writes.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(format!("{name}: quota exceeded"));
        }
        self.files
            .lock()
            .expect("not poisoned")
            .insert(name.into(), bytes.to_vec());
        Ok(())
    }
    async fn remove(&self, name: &str) -> Result<(), String> {
        self.files.lock().expect("not poisoned").remove(name);
        Ok(())
    }
}

impl<S: ChunkStore> ChunkStore for &S {
    async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        (**self).read(name).await
    }
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        (**self).write(name, bytes).await
    }
    async fn remove(&self, name: &str) -> Result<(), String> {
        (**self).remove(name).await
    }
}

impl<S: ChunkStore> ChunkStore for std::sync::Arc<S> {
    async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        (**self).read(name).await
    }
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        (**self).write(name, bytes).await
    }
    async fn remove(&self, name: &str) -> Result<(), String> {
        (**self).remove(name).await
    }
}

/// The cache's index: every stored chunk with its size and last use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheIndex {
    pub entries: BTreeMap<String, IndexEntry>,
    /// Counts uses (the order of last use; no wall clock needed).
    pub clock: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub size: u64,
    pub used: u64,
}

impl CacheIndex {
    pub fn total(&self) -> u64 {
        self.entries.values().map(|e| e.size).sum()
    }

    fn touch(&mut self, name: &str) {
        self.clock += 1;
        if let Some(e) = self.entries.get_mut(name) {
            e.used = self.clock;
        }
    }

    /// The least recently used entries to remove so that the total fits
    /// `budget` (never `keep`, the entry just written).
    pub fn evictions(&self, budget: u64, keep: &str) -> Vec<String> {
        let mut total = self.total();
        let mut by_age: Vec<(&String, &IndexEntry)> = self.entries.iter().collect();
        by_age.sort_by_key(|(n, e)| (e.used, n.as_str()));
        let mut out = Vec::new();
        for (name, e) in by_age {
            if total <= budget {
                break;
            }
            if name == keep {
                continue;
            }
            total -= e.size;
            out.push(name.clone());
        }
        out
    }
}

/// The file name of a chunk: `<id with / as ~>@<version>@<chunk>` with the
/// chunk name's blanks and commas as `_` (`liechtenstein@21@tile_3_-4`).
pub fn chunk_file_name(id: &str, version: u64, chunk: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '#' => c,
                '/' => '~',
                _ => '_',
            })
            .collect()
    };
    format!("{}@{version}@{}", clean(id), clean(chunk))
}

/// What the cache did (the loading screen's "from cache" line).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub hit_bytes: u64,
    pub misses: u64,
    /// Cached files that no longer matched the manifest (removed).
    pub damaged: u64,
    pub stored: u64,
    pub evicted: u64,
    pub write_errors: u64,
}

/// Verified chunks in a [`ChunkStore`] with an index and a size budget.
pub struct ChunkCache<S> {
    store: S,
    index: Mutex<CacheIndex>,
    budget: u64,
    stats: Mutex<CacheStats>,
}

impl<S: ChunkStore> ChunkCache<S> {
    /// Opens the cache in `store` (an unreadable index starts afresh; its
    /// files are then unknown and overwritten as chunks come again).
    pub async fn open(store: S, budget: u64) -> Self {
        let index = match store.read(INDEX_FILE).await {
            Ok(Some(b)) => serde_json::from_slice(&b).unwrap_or_default(),
            _ => CacheIndex::default(),
        };
        Self {
            store,
            index: Mutex::new(index),
            budget,
            stats: Mutex::new(CacheStats::default()),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Bytes stored now.
    pub fn used(&self) -> u64 {
        self.index.lock().expect("not poisoned").total()
    }

    pub fn stats(&self) -> CacheStats {
        *self.stats.lock().expect("not poisoned")
    }

    pub fn index(&self) -> CacheIndex {
        self.index.lock().expect("not poisoned").clone()
    }

    fn stat(&self, f: impl FnOnce(&mut CacheStats)) {
        f(&mut self.stats.lock().expect("not poisoned"));
    }

    /// The chunk's bytes if they are cached and still match `chunk`
    /// (size and SHA-256 of the manifest); a mismatch is removed.
    pub async fn get(&self, id: &str, version: u64, chunk: &Chunk) -> Option<Vec<u8>> {
        let name = chunk_file_name(id, version, &chunk.name);
        let known = self
            .index
            .lock()
            .expect("not poisoned")
            .entries
            .contains_key(&name);
        if !known {
            self.stat(|s| s.misses += 1);
            return None;
        }
        match self.store.read(&name).await {
            Ok(Some(b)) if b.len() as u64 == chunk.size && sha256(&b) == chunk.sha256 => {
                self.index.lock().expect("not poisoned").touch(&name);
                self.stat(|s| {
                    s.hits += 1;
                    s.hit_bytes += b.len() as u64;
                });
                Some(b)
            }
            Ok(Some(_)) => {
                let _ = self.store.remove(&name).await;
                self.index
                    .lock()
                    .expect("not poisoned")
                    .entries
                    .remove(&name);
                self.stat(|s| {
                    s.damaged += 1;
                    s.misses += 1;
                });
                None
            }
            _ => {
                self.index
                    .lock()
                    .expect("not poisoned")
                    .entries
                    .remove(&name);
                self.stat(|s| s.misses += 1);
                None
            }
        }
    }

    /// Stores a verified chunk, evicts the least recently used ones over
    /// the budget and saves the index. Returns the evicted file names.
    pub async fn put(
        &self,
        id: &str,
        version: u64,
        chunk: &Chunk,
        bytes: &[u8],
    ) -> Result<Vec<String>, String> {
        if bytes.len() as u64 > self.budget {
            return Ok(Vec::new());
        }
        let name = chunk_file_name(id, version, &chunk.name);
        if let Err(e) = self.store.write(&name, bytes).await {
            self.stat(|s| s.write_errors += 1);
            return Err(e);
        }
        let evict = {
            let mut index = self.index.lock().expect("not poisoned");
            index.clock += 1;
            let used = index.clock;
            index.entries.insert(
                name.clone(),
                IndexEntry {
                    size: bytes.len() as u64,
                    used,
                },
            );
            let evict = index.evictions(self.budget, &name);
            for n in &evict {
                index.entries.remove(n);
            }
            evict
        };
        for n in &evict {
            let _ = self.store.remove(n).await;
        }
        let n = evict.len() as u64;
        self.stat(|s| {
            s.stored += 1;
            s.evicted += n;
        });
        self.save_index().await?;
        Ok(evict)
    }

    /// Writes the index (also the last-use order of the hits).
    pub async fn save_index(&self) -> Result<(), String> {
        let json = serde_json::to_vec(&*self.index.lock().expect("not poisoned"))
            .map_err(|e| e.to_string())?;
        self.store.write(INDEX_FILE, &json).await
    }
}

/// Runs a future that never waits (the in-memory stores and fetches of
/// the tests) to completion.
#[cfg(test)]
pub(crate) fn ready<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match f.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("the test future waited"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(name: &str, bytes: &[u8]) -> Chunk {
        Chunk {
            name: name.into(),
            offset: 0,
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        }
    }

    #[test]
    fn file_names() {
        assert_eq!(
            chunk_file_name("europe/liechtenstein", 21, "tile 3,-4"),
            "europe~liechtenstein@21@tile_3_-4"
        );
        assert_eq!(
            chunk_file_name("li", 2, "section SEAS#2"),
            "li@2@section_SEAS#2"
        );
    }

    #[test]
    fn chunks_come_back_verified_and_are_reused() {
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 1000));
        let a = chunk("tile 0,0", b"aaaa");
        assert_eq!(ready(cache.get("li", 1, &a)), None);
        ready(cache.put("li", 1, &a, b"aaaa")).unwrap();
        assert_eq!(ready(cache.get("li", 1, &a)).as_deref(), Some(&b"aaaa"[..]));
        // Another version is another chunk.
        assert_eq!(ready(cache.get("li", 2, &a)), None);
        assert_eq!(cache.used(), 4);
        // Reopened (the next visit): the index is read back.
        drop(cache);
        let cache = ready(ChunkCache::open(&store, 1000));
        assert_eq!(cache.used(), 4);
        assert!(ready(cache.get("li", 1, &a)).is_some());
        let s = cache.stats();
        assert_eq!((s.hits, s.hit_bytes, s.misses), (1, 4, 0));
    }

    #[test]
    fn a_damaged_cached_chunk_is_removed_and_missed() {
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 1000));
        let a = chunk("tile 0,0", b"aaaa");
        ready(cache.put("li", 1, &a, b"aaaa")).unwrap();
        store
            .files
            .lock()
            .unwrap()
            .insert(chunk_file_name("li", 1, "tile 0,0"), b"aaab".to_vec());
        assert_eq!(ready(cache.get("li", 1, &a)), None);
        assert_eq!(cache.stats().damaged, 1);
        assert_eq!(cache.used(), 0);
        assert!(
            !store
                .names()
                .contains(&chunk_file_name("li", 1, "tile 0,0"))
        );
        // A file missing behind the index's back: a miss, the entry goes.
        ready(cache.put("li", 1, &a, b"aaaa")).unwrap();
        store.files.lock().unwrap().clear();
        assert_eq!(ready(cache.get("li", 1, &a)), None);
        assert_eq!(cache.used(), 0);
    }

    /// Eviction by the size budget: least recently used first, never the
    /// chunk just stored; a hit counts as a use.
    #[test]
    fn the_budget_evicts_the_least_recently_used() {
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 10));
        let (a, b, c) = (
            chunk("a", b"aaaa"),
            chunk("b", b"bbbb"),
            chunk("c", b"cccc"),
        );
        ready(cache.put("li", 1, &a, b"aaaa")).unwrap();
        ready(cache.put("li", 1, &b, b"bbbb")).unwrap();
        // a is used again: b is now the oldest.
        assert!(ready(cache.get("li", 1, &a)).is_some());
        let evicted = ready(cache.put("li", 1, &c, b"cccc")).unwrap();
        assert_eq!(evicted, [chunk_file_name("li", 1, "b")]);
        assert_eq!(cache.used(), 8);
        assert!(ready(cache.get("li", 1, &b)).is_none());
        assert!(ready(cache.get("li", 1, &a)).is_some());
        assert!(!store.names().contains(&chunk_file_name("li", 1, "b")));
        assert_eq!(cache.stats().evicted, 1);
        // A chunk larger than the whole budget is not stored.
        let big = chunk("big", &[0; 11]);
        assert!(
            ready(cache.put("li", 1, &big, &[0; 11]))
                .unwrap()
                .is_empty()
        );
        assert!(ready(cache.get("li", 1, &big)).is_none());
        assert_eq!(cache.used(), 8);
    }

    #[test]
    fn store_errors_leave_the_cache_consistent() {
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 1000));
        store
            .fail_writes
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let a = chunk("a", b"aaaa");
        assert!(ready(cache.put("li", 1, &a, b"aaaa")).is_err());
        assert_eq!(cache.used(), 0);
        assert_eq!(cache.stats().write_errors, 1);
        // A broken index file: an empty cache, not an error.
        store
            .fail_writes
            .store(false, std::sync::atomic::Ordering::Relaxed);
        ready(store.write(INDEX_FILE, b"{not json")).unwrap();
        let cache = ready(ChunkCache::open(&store, 1000));
        assert_eq!(cache.index(), CacheIndex::default());
    }
}
