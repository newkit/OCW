//! The streaming sequence as small async steps (spec §14 Phase 10 B2),
//! independent of how it is run: the browser app loops over
//! [`StreamDriver::step`] in a `spawn_local` task (sleeping when idle),
//! tests call it until it is idle. The app and the driver share a
//! [`StreamShared`]: the progress for the loading screen, the decoded
//! base once it is there, the app's tile requests and the arrived tiles.
//!
//! 1. the cache is opened (its index read);
//! 2. `?world=ID`: `<base>/ID/latest` gives the version;
//! 3. the manifest is fetched and its signature chain checked
//!    ([`super::accept_manifest`]);
//! 4. the base (header and sections) is loaded, verified and decoded;
//! 5. then the tiles the app asks for, batch by batch.

use std::sync::Mutex;

use ocw_world::TileKey;
use ocw_world::manifest::{FileEntry, Manifest, TrustStatus, TrustStore, UnsignedPolicy};
use ocw_world::package::StreamedPackage;

use super::cache::{ChunkCache, ChunkStore};
use super::load::{LoadStats, RangeFetch, StreamError, load_chunks};
use super::plan::{RangeParams, base_chunks};
use super::progress::{LoadProgress, Stage, Storage};
use super::{MAX_MANIFEST_BYTES, WorldRef, accept_manifest, sibling_url};

/// What to stream and how.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// What the page asked for (`?world=`), for the loading screen.
    pub world: String,
    pub source: WorldRef,
    pub trust: TrustStore,
    pub policy: UnsignedPolicy,
    pub now_unix_s: i64,
    pub ranges: RangeParams,
    /// The chunk cache's size budget (bytes).
    pub cache_budget: u64,
    /// Most tiles in one batch of requests.
    pub batch_tiles: usize,
}

/// The decoded base of the package, handed to the app once.
#[derive(Debug, Clone)]
pub struct StreamBase {
    pub package: StreamedPackage,
    pub manifest: Manifest,
    pub trust: TrustStatus,
    /// The package file's entry (its chunks).
    pub file: FileEntry,
    pub file_url: String,
}

/// Between the app and the driver.
#[derive(Debug, Default)]
pub struct StreamShared {
    pub progress: LoadProgress,
    /// The base, until the app takes it.
    pub base: Option<StreamBase>,
    /// Tiles the app wants: key and chunk index.
    pub requests: Vec<(TileKey, usize)>,
    /// Fetched and verified tile chunks, or why not.
    pub arrived: Vec<(TileKey, Result<Vec<u8>, String>)>,
    /// What the tiles transferred while riding.
    pub tile_stats: LoadStats,
    /// The browser's storage quota and persistence (set by the app).
    pub quota: Option<u64>,
    pub persisted: Option<bool>,
}

enum State {
    Start,
    Resolve,
    Manifest(String),
    Base,
    Tiles,
    Failed,
}

/// Runs the sequence over `F` and an optional chunk store `S`.
pub struct StreamDriver<F, S> {
    fetch: F,
    store: Option<S>,
    cache: Option<ChunkCache<S>>,
    config: StreamConfig,
    state: State,
    manifest: Option<(Manifest, TrustStatus, FileEntry, String)>,
    clock: Box<dyn Fn() -> f64 + Send + Sync>,
    started_ms: f64,
}

impl<F: RangeFetch, S: ChunkStore> StreamDriver<F, S> {
    /// `clock` gives milliseconds (the browser's `performance.now()`).
    pub fn new(
        fetch: F,
        store: Option<S>,
        config: StreamConfig,
        clock: Box<dyn Fn() -> f64 + Send + Sync>,
    ) -> Self {
        let started_ms = clock();
        Self {
            fetch,
            store,
            cache: None,
            config,
            state: State::Start,
            manifest: None,
            clock,
            started_ms,
        }
    }

    pub fn cache(&self) -> Option<&ChunkCache<S>> {
        self.cache.as_ref()
    }

    pub fn failed(&self) -> bool {
        matches!(self.state, State::Failed)
    }

    fn elapsed(&self) -> f64 {
        (self.clock)() - self.started_ms
    }

    fn storage(&self, shared: &StreamShared) -> Storage {
        match &self.cache {
            Some(c) => Storage {
                used: c.used(),
                budget: c.budget(),
                quota: shared.quota,
                persisted: shared.persisted,
                unavailable: false,
            },
            None => Storage {
                unavailable: true,
                ..Storage::default()
            },
        }
    }

    fn fail(&mut self, shared: &Mutex<StreamShared>, e: impl std::fmt::Display) {
        let mut s = shared.lock().expect("not poisoned");
        s.progress.stage = Stage::Failed;
        s.progress.error = Some(e.to_string());
        self.state = State::Failed;
    }

    /// One step. Returns whether there was work (false: idle or failed;
    /// call again later, e.g. when the app asked for tiles).
    pub async fn step(&mut self, shared: &Mutex<StreamShared>) -> bool {
        match std::mem::replace(&mut self.state, State::Failed) {
            State::Start => {
                if let Some(store) = self.store.take() {
                    self.cache = Some(ChunkCache::open(store, self.config.cache_budget).await);
                }
                {
                    let mut s = shared.lock().expect("not poisoned");
                    if s.progress.world.is_empty() {
                        s.progress.world = self.config.world.clone();
                    }
                    let st = self.storage(&s);
                    s.progress.storage = Some(st);
                }
                self.state = State::Resolve;
                true
            }
            State::Resolve => {
                match self.config.source.clone() {
                    WorldRef::Manifest(url) => self.state = State::Manifest(url),
                    WorldRef::Latest { latest_url, .. } => {
                        match self.fetch.get(&latest_url, None).await {
                            Ok(r) if r.status == 200 => {
                                let text = String::from_utf8_lossy(&r.body);
                                match WorldRef::manifest_url_from_latest(&latest_url, &text) {
                                    Ok(url) => self.state = State::Manifest(url),
                                    Err(e) => self.fail(shared, e),
                                }
                            }
                            Ok(r) => self.fail(
                                shared,
                                StreamError::Status(
                                    format!("{latest_url} (no such world?)"),
                                    r.status,
                                ),
                            ),
                            Err(e) => self.fail(shared, StreamError::Transport(e)),
                        }
                    }
                }
                true
            }
            State::Manifest(url) => {
                if let Err(e) = self.manifest_step(&url, shared).await {
                    self.fail(shared, e);
                } else {
                    self.state = State::Base;
                }
                true
            }
            State::Base => {
                match self.base_step(shared).await {
                    Ok(()) => self.state = State::Tiles,
                    Err(e) => self.fail(shared, e),
                }
                true
            }
            State::Tiles => {
                self.state = State::Tiles;
                self.tiles_step(shared).await
            }
            State::Failed => false,
        }
    }

    async fn manifest_step(
        &mut self,
        url: &str,
        shared: &Mutex<StreamShared>,
    ) -> Result<(), StreamError> {
        let r = self
            .fetch
            .get(url, None)
            .await
            .map_err(StreamError::Transport)?;
        if r.status != 200 {
            return Err(StreamError::Status(url.into(), r.status));
        }
        if r.body.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(StreamError::Decode(format!("{url}: manifest too large")));
        }
        let text = String::from_utf8(r.body)
            .map_err(|_| StreamError::Decode(format!("{url}: the manifest is not UTF-8")))?;
        let c = &self.config;
        let (signed, trust) = accept_manifest(&text, &c.trust, c.policy, c.now_unix_s)?;
        let m = signed.manifest;
        let file = m
            .package_file()
            .filter(|f| !f.chunks.is_empty())
            .cloned()
            .ok_or_else(|| {
                StreamError::Decode(format!(
                    "{url}: the manifest lists no world package chunks (rebuild it with `world-builder sign`)"
                ))
            })?;
        let base_total = base_chunks(&file)
            .iter()
            .map(|&i| file.chunks[i].size)
            .sum();
        let elapsed = self.elapsed();
        {
            let mut s = shared.lock().expect("not poisoned");
            let p = &mut s.progress;
            p.package = Some(format!("{} ({}@{})", m.name, m.id, m.version));
            p.trust = Some(trust.to_string());
            p.manifest_bytes = text.len() as u64;
            p.manifest_ms = Some(elapsed);
            p.base_total = base_total;
            p.package_bytes = file.size;
            p.stage = Stage::Base;
            p.stats.requests += 1;
        }
        let file_url = sibling_url(url, &file.path);
        self.manifest = Some((m, trust, file, file_url));
        Ok(())
    }

    async fn base_step(&mut self, shared: &Mutex<StreamShared>) -> Result<(), StreamError> {
        let (m, trust, file, file_url) = self.manifest.clone().expect("after the manifest");
        let wanted = base_chunks(&file);
        let (chunks, stats) = load_chunks(
            &self.fetch,
            self.cache.as_ref(),
            &m,
            &file,
            &file_url,
            &wanted,
            self.config.ranges,
            &mut |bytes, _| {
                shared.lock().expect("not poisoned").progress.base_done += bytes;
            },
        )
        .await?;
        let header = chunks
            .iter()
            .find(|c| file.chunks[c.index].name == "header")
            .ok_or_else(|| StreamError::Decode("the package has no header chunk".into()))?;
        let mut sections = Vec::new();
        for c in chunks
            .iter()
            .filter(|c| file.chunks[c.index].name != "header")
        {
            sections.extend_from_slice(&c.bytes);
        }
        let package =
            StreamedPackage::from_parts(&header.bytes, &sections).map_err(StreamError::Decode)?;
        let elapsed = self.elapsed();
        let mut s = shared.lock().expect("not poisoned");
        let st = self.storage(&s);
        let p = &mut s.progress;
        p.stats.add(&stats);
        p.base_ms = Some(elapsed);
        p.stage = Stage::FirstTiles;
        p.storage = Some(st);
        s.base = Some(StreamBase {
            package,
            manifest: m,
            trust,
            file,
            file_url,
        });
        Ok(())
    }

    async fn tiles_step(&mut self, shared: &Mutex<StreamShared>) -> bool {
        let batch: Vec<(TileKey, usize)> = {
            let mut s = shared.lock().expect("not poisoned");
            let n = s.requests.len().min(self.config.batch_tiles.max(1));
            s.requests.drain(..n).collect()
        };
        if batch.is_empty() {
            return false;
        }
        let (m, _, file, file_url) = self.manifest.clone().expect("after the manifest");
        let wanted: Vec<usize> = batch.iter().map(|&(_, i)| i).collect();
        let result = load_chunks(
            &self.fetch,
            self.cache.as_ref(),
            &m,
            &file,
            &file_url,
            &wanted,
            self.config.ranges,
            &mut |_, _| {},
        )
        .await;
        let elapsed = self.elapsed();
        let mut s = shared.lock().expect("not poisoned");
        match result {
            Ok((chunks, stats)) => {
                let first = s.progress.stage == Stage::FirstTiles;
                if first {
                    s.progress.stats.add(&stats);
                }
                s.tile_stats.add(&stats);
                for (key, i) in batch {
                    match chunks.iter().find(|c| c.index == i) {
                        Some(c) => {
                            if first {
                                s.progress.tiles_done += 1;
                            }
                            s.arrived.push((key, Ok(c.bytes.clone())));
                        }
                        None => s.arrived.push((key, Err("not loaded".into()))),
                    }
                }
                let p = &mut s.progress;
                if first && p.tiles_wanted > 0 && p.tiles_done >= p.tiles_wanted {
                    p.first_tiles_ms = Some(elapsed);
                    p.stage = Stage::Riding;
                }
            }
            Err(e) => {
                let msg = e.to_string();
                for (key, _) in batch {
                    s.arrived.push((key, Err(msg.clone())));
                }
                if s.progress.stage == Stage::FirstTiles {
                    s.progress.stage = Stage::Failed;
                    s.progress.error = Some(msg);
                }
            }
        }
        let st = self.storage(&s);
        s.progress.storage = Some(st);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::cache::{MemoryStore, ready};
    use crate::stream::load::MemoryFetch;
    use crate::stream::load::tests::published;
    use crate::stream::plan::tile_chunks;
    use ocw_world::manifest::SignedManifest;

    const BASE: &str = "https://w.example/worlds/";

    fn config(policy: UnsignedPolicy) -> StreamConfig {
        StreamConfig {
            world: "test/mini".into(),
            source: WorldRef::parse("test/mini", BASE).unwrap(),
            trust: TrustStore::default(),
            policy,
            now_unix_s: 1000,
            // No gaps: the base is the header and the sections alone.
            ranges: RangeParams {
                max_gap: 0,
                max_len: 1 << 20,
            },
            cache_budget: 1 << 20,
            batch_tiles: 4,
        }
    }

    fn server() -> (MemoryFetch, Vec<u8>) {
        let (bytes, m) = published();
        let dir = format!("{BASE}test/mini/3/");
        let fetch = MemoryFetch::with([
            (format!("{BASE}test/mini/latest"), b"3\n".to_vec()),
            (
                format!("{dir}manifest.json"),
                SignedManifest::unsigned(&m).to_json().into_bytes(),
            ),
            (format!("{dir}mini.ocwpkg"), bytes.clone()),
        ]);
        (fetch, bytes)
    }

    /// Runs steps until the driver is idle; returns the steps taken.
    fn run<F: RangeFetch, S: ChunkStore>(
        d: &mut StreamDriver<F, S>,
        shared: &Mutex<StreamShared>,
    ) -> u32 {
        let mut n = 0;
        while ready(d.step(shared)) {
            n += 1;
            assert!(n < 100);
        }
        n
    }

    fn clock() -> Box<dyn Fn() -> f64 + Send + Sync> {
        let t = std::sync::atomic::AtomicU64::new(0);
        Box::new(move || (t.fetch_add(10, std::sync::atomic::Ordering::Relaxed) + 10) as f64)
    }

    #[test]
    fn a_world_streams_from_its_id_to_the_first_tiles() {
        let (fetch, _) = server();
        let store = MemoryStore::default();
        let shared = Mutex::new(StreamShared::default());
        let mut d = StreamDriver::new(&fetch, Some(&store), config(UnsignedPolicy::Warn), clock());
        run(&mut d, &shared);
        let base = shared
            .lock()
            .unwrap()
            .base
            .take()
            .expect("the base is there");
        assert_eq!(base.manifest.id, "test/mini");
        assert!(!base.trust.is_trusted());
        assert!(base.package.base().tiles.is_empty());
        {
            let s = shared.lock().unwrap();
            assert_eq!(s.progress.stage, Stage::FirstTiles);
            assert_eq!(s.progress.package.as_deref(), Some("Mini (test/mini@3)"));
            assert!(s.progress.trust.as_deref().unwrap().contains("NOT TRUSTED"));
            assert_eq!(s.progress.base_done, s.progress.base_total);
            assert!(s.progress.manifest_ms.is_some() && s.progress.base_ms.is_some());
        }
        // The app asks for the first tiles.
        let tiles = tile_chunks(&base.file);
        {
            let mut s = shared.lock().unwrap();
            s.requests = tiles.iter().take(6).map(|(k, i)| (*k, *i)).collect();
            s.progress.tiles_wanted = 6;
        }
        let steps = run(&mut d, &shared);
        assert_eq!(steps, 2, "two batches of at most 4");
        let s = shared.lock().unwrap();
        assert_eq!(s.arrived.len(), 6);
        assert!(s.arrived.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(s.progress.stage, Stage::Riding);
        assert_eq!(s.progress.tiles_done, 6);
        assert!(s.progress.first_tiles_ms.unwrap() > s.progress.base_ms.unwrap());
        let st = s.progress.storage.unwrap();
        assert!(st.used > 0 && !st.unavailable);
        assert!(
            s.progress
                .lines()
                .iter()
                .any(|l| l.starts_with("storage: "))
        );
    }

    /// The second visit loads the base and the tiles from the cache: only
    /// the small files (latest, manifest) come over the network.
    #[test]
    fn the_second_visit_uses_the_cache() {
        let (fetch, _) = server();
        let store = MemoryStore::default();
        let mut starts = Vec::new();
        for visit in 0..2 {
            starts.push(fetch.requests().len());
            let shared = Mutex::new(StreamShared::default());
            let mut d =
                StreamDriver::new(&fetch, Some(&store), config(UnsignedPolicy::Warn), clock());
            run(&mut d, &shared);
            let base = shared.lock().unwrap().base.take().unwrap();
            shared.lock().unwrap().requests = tile_chunks(&base.file)
                .iter()
                .map(|(k, i)| (*k, *i))
                .collect();
            run(&mut d, &shared);
            let s = shared.lock().unwrap();
            let p = &s.progress;
            let tiles = &s.tile_stats;
            if visit == 0 {
                assert_eq!(p.stats.cache_bytes, 0);
                // The first tiles count for the loading screen too.
                assert_eq!(p.stats.network_bytes, p.base_total + tiles.network_bytes);
                assert!(tiles.network_bytes > 0);
            } else {
                assert_eq!(p.stats.network_bytes, 0, "{p:?}");
                assert_eq!(p.stats.cache_bytes, p.base_total + tiles.cache_bytes);
                assert_eq!(tiles.network_bytes, 0);
                assert_eq!(tiles.cache_chunks, 12);
            }
        }
        // Visit 2 only asked for `latest` and the manifest.
        let second: Vec<String> = fetch.requests()[starts[1]..]
            .iter()
            .map(|(u, _)| u.clone())
            .collect();
        assert_eq!(second.len(), 2, "{second:?}");
        assert!(second.iter().all(|u| !u.ends_with(".ocwpkg")));
    }

    #[test]
    fn untrusted_or_missing_worlds_fail_with_a_message() {
        // Unsigned under Refuse: stops after the manifest.
        let (fetch, _) = server();
        let shared = Mutex::new(StreamShared::default());
        let mut d = StreamDriver::<_, MemoryStore>::new(
            &fetch,
            None,
            config(UnsignedPolicy::Refuse),
            clock(),
        );
        run(&mut d, &shared);
        assert!(d.failed());
        let s = shared.lock().unwrap();
        assert_eq!(s.progress.stage, Stage::Failed);
        assert!(s.progress.error.as_deref().unwrap().contains("not signed"));
        assert!(s.base.is_none());
        assert!(
            fetch
                .requests()
                .iter()
                .all(|(u, _)| !u.ends_with(".ocwpkg"))
        );
        // No OPFS: the storage line says so.
        assert!(s.progress.storage.unwrap().unavailable);
        drop(s);
        // An unknown id: the HTTP error of `latest`.
        let shared = Mutex::new(StreamShared::default());
        let mut c = config(UnsignedPolicy::Warn);
        c.source = WorldRef::parse("test/none", BASE).unwrap();
        let mut d = StreamDriver::<_, MemoryStore>::new(&fetch, None, c, clock());
        run(&mut d, &shared);
        let e = shared.lock().unwrap().progress.error.clone().unwrap();
        assert!(e.contains("404") && e.contains("no such world"), "{e}");
    }

    /// A tile damaged on the server: refetched, then reported as an
    /// error for its batch; the base and other tiles are unaffected.
    #[test]
    fn a_damaged_tile_on_the_server_is_reported() {
        let (fetch, _) = server();
        let shared = Mutex::new(StreamShared::default());
        let mut d = StreamDriver::<_, MemoryStore>::new(
            &fetch,
            None,
            config(UnsignedPolicy::Warn),
            clock(),
        );
        run(&mut d, &shared);
        let base = shared.lock().unwrap().base.take().unwrap();
        let tiles = tile_chunks(&base.file);
        let (k, i) = tiles.iter().next().map(|(k, i)| (*k, *i)).unwrap();
        let c = &base.file.chunks[i];
        fetch.corrupt_at(&base.file_url, c.offset + 3, 5);
        {
            let mut s = shared.lock().unwrap();
            s.requests = vec![(k, i)];
            s.progress.tiles_wanted = 1;
        }
        run(&mut d, &shared);
        let s = shared.lock().unwrap();
        let (key, r) = &s.arrived[0];
        assert_eq!(*key, k);
        let e = r.as_ref().unwrap_err();
        assert!(e.contains(&c.name) && e.contains("damaged"), "{e}");
        assert_eq!(s.progress.stage, Stage::Failed);
        assert!(s.progress.lines().last().unwrap().starts_with("Error: "));
    }
}
