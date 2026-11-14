//! Fetching chunks with range requests and verifying them (spec §4.7,
//! §14 Phase 10 B2): from the cache when it has them, else over a
//! [`RangeFetch`] (the browser's `fetch()`, [`MemoryFetch`] in tests).
//! Every chunk from the network is checked against its SHA-256 in the
//! manifest before it is used or cached. A chunk that does not match is
//! fetched once more on its own (a transfer error, a proxy that mangled a
//! merged range); if it still does not match, the load fails with an error
//! that names the chunk.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Mutex;

use ocw_world::manifest::{FileEntry, Manifest, VerifyError};

use super::cache::{ChunkCache, ChunkStore};
use super::plan::{RangeParams, plan_ranges};

/// An HTTP response: status and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// 200 (the whole file) or 206 (the requested range); others are
    /// errors.
    pub status: u16,
    pub body: Vec<u8>,
}

/// How bytes are fetched.
#[allow(async_fn_in_trait)] // used single-threaded (wasm) and in tests
pub trait RangeFetch {
    /// GET `url`; with a range, `Range: bytes=start-(end - 1)`.
    async fn get(&self, url: &str, range: Option<Range<u64>>) -> Result<HttpResponse, String>;
}

impl<F: RangeFetch> RangeFetch for &F {
    async fn get(&self, url: &str, range: Option<Range<u64>>) -> Result<HttpResponse, String> {
        (**self).get(url, range).await
    }
}

/// Files served from memory with range support (tests; the web app's
/// native tests). Can damage the bytes of a range a number of times, or
/// ignore ranges like a server without them.
#[derive(Debug, Default)]
pub struct MemoryFetch {
    pub files: BTreeMap<String, Vec<u8>>,
    /// Every request: URL and range.
    pub requests: Mutex<Vec<(String, Option<Range<u64>>)>>,
    /// Flip a byte at this absolute file offset in the next N responses
    /// that contain it: (url, offset) → N.
    pub corrupt: Mutex<BTreeMap<(String, u64), u32>>,
    /// Answer every request with the whole file (200).
    pub ignore_ranges: bool,
}

impl MemoryFetch {
    pub fn with(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        Self {
            files: files.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn requests(&self) -> Vec<(String, Option<Range<u64>>)> {
        self.requests.lock().expect("not poisoned").clone()
    }

    /// Bytes served so far (bodies of ranges or whole files).
    pub fn served_bytes(&self) -> u64 {
        self.requests()
            .iter()
            .map(|(url, r)| match (r, self.ignore_ranges) {
                (Some(r), false) => r.end - r.start,
                _ => self.files.get(url).map_or(0, |f| f.len() as u64),
            })
            .sum()
    }

    /// Damages the byte at `offset` of `url` in the next `times` responses.
    pub fn corrupt_at(&self, url: &str, offset: u64, times: u32) {
        self.corrupt
            .lock()
            .expect("not poisoned")
            .insert((url.into(), offset), times);
    }
}

impl RangeFetch for MemoryFetch {
    async fn get(&self, url: &str, range: Option<Range<u64>>) -> Result<HttpResponse, String> {
        self.requests
            .lock()
            .expect("not poisoned")
            .push((url.into(), range.clone()));
        let Some(data) = self.files.get(url) else {
            return Ok(HttpResponse {
                status: 404,
                body: Vec::new(),
            });
        };
        let (status, start, mut body) = match range {
            Some(r) if !self.ignore_ranges => {
                let end = (r.end as usize).min(data.len());
                let start = (r.start as usize).min(end);
                (206, start as u64, data[start..end].to_vec())
            }
            _ => (200, 0, data.clone()),
        };
        let mut corrupt = self.corrupt.lock().expect("not poisoned");
        for ((u, at), n) in corrupt.iter_mut() {
            if u == url && *n > 0 && *at >= start && *at < start + body.len() as u64 {
                body[(*at - start) as usize] ^= 0xff;
                *n -= 1;
            }
        }
        Ok(HttpResponse { status, body })
    }
}

/// Why streaming a package failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("{0}")]
    Transport(String),
    #[error("{0}: HTTP status {1}")]
    Status(String, u16),
    #[error("{0}")]
    Verify(#[from] VerifyError),
    #[error(
        "{file}: chunk \"{chunk}\" does not match its SHA-256 in the manifest, also when fetched again: the copy on the server is damaged or was changed (refusing to use it)"
    )]
    Corrupt { file: String, chunk: String },
    #[error("{0}")]
    Decode(String),
}

/// Where a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cache,
    Network,
}

/// A verified chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedChunk {
    /// Index into the file's `chunks`.
    pub index: usize,
    pub bytes: Vec<u8>,
    pub source: Source,
}

/// What a load transferred.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadStats {
    pub requests: u64,
    /// Bytes received (including the gaps of merged ranges).
    pub network_bytes: u64,
    pub network_chunks: u64,
    pub cache_bytes: u64,
    pub cache_chunks: u64,
    /// Chunks that failed their hash and were fetched again.
    pub refetched: u64,
}

impl LoadStats {
    pub fn add(&mut self, o: &LoadStats) {
        self.requests += o.requests;
        self.network_bytes += o.network_bytes;
        self.network_chunks += o.network_chunks;
        self.cache_bytes += o.cache_bytes;
        self.cache_chunks += o.cache_chunks;
        self.refetched += o.refetched;
    }
}

/// Fetches `range` of `url` and returns exactly its bytes (a server that
/// ignored the range sends the whole file: the range is cut out of it).
async fn fetch_range<F: RangeFetch>(
    fetch: &F,
    url: &str,
    range: Range<u64>,
    stats: &mut LoadStats,
) -> Result<Vec<u8>, StreamError> {
    let resp = fetch
        .get(url, Some(range.clone()))
        .await
        .map_err(StreamError::Transport)?;
    stats.requests += 1;
    stats.network_bytes += resp.body.len() as u64;
    let want = (range.end - range.start) as usize;
    let body = match resp.status {
        206 => resp.body,
        200 => resp
            .body
            .get(range.start as usize..range.end as usize)
            .map(<[u8]>::to_vec)
            .unwrap_or_default(),
        s => return Err(StreamError::Status(url.into(), s)),
    };
    if body.len() != want {
        // Short: the server's file is not the manifest's. The hash check
        // reports it per chunk.
        let mut b = body;
        b.resize(want, 0);
        return Ok(b);
    }
    Ok(body)
}

/// Loads the `wanted` chunks of `file` (served at `file_url`) of the
/// package `manifest`: from `cache` where it has them, the rest with
/// merged range requests, each verified before use; verified network
/// chunks are stored in the cache. `progress(bytes, source)` is called per
/// chunk. The chunks come back in the order of `wanted`.
#[allow(clippy::too_many_arguments)]
pub async fn load_chunks<F: RangeFetch, S: ChunkStore>(
    fetch: &F,
    cache: Option<&ChunkCache<S>>,
    manifest: &Manifest,
    file: &FileEntry,
    file_url: &str,
    wanted: &[usize],
    params: RangeParams,
    progress: &mut dyn FnMut(u64, Source),
) -> Result<(Vec<LoadedChunk>, LoadStats), StreamError> {
    let mut stats = LoadStats::default();
    let mut got: BTreeMap<usize, LoadedChunk> = BTreeMap::new();
    let mut missing = Vec::new();
    for &i in wanted {
        let Some(c) = file.chunks.get(i) else {
            return Err(VerifyError::MissingFile(format!("{}: chunk {i}", file.path)).into());
        };
        if got.contains_key(&i) || missing.contains(&i) {
            continue;
        }
        let hit = match cache {
            Some(cache) => cache.get(&manifest.id, manifest.version, c).await,
            None => None,
        };
        match hit {
            Some(bytes) => {
                stats.cache_chunks += 1;
                stats.cache_bytes += bytes.len() as u64;
                progress(bytes.len() as u64, Source::Cache);
                got.insert(
                    i,
                    LoadedChunk {
                        index: i,
                        bytes,
                        source: Source::Cache,
                    },
                );
            }
            None => missing.push(i),
        }
    }
    let mut retry = Vec::new();
    for req in plan_ranges(file, &missing, params) {
        let body = fetch_range(fetch, file_url, req.range.clone(), &mut stats).await?;
        for i in req.chunks {
            let c = &file.chunks[i];
            let at = (c.offset - req.range.start) as usize;
            let bytes = body[at..at + c.size as usize].to_vec();
            if manifest.verify_chunk(&file.path, &c.name, &bytes).is_ok() {
                got.insert(
                    i,
                    LoadedChunk {
                        index: i,
                        bytes,
                        source: Source::Network,
                    },
                );
            } else {
                retry.push(i);
            }
        }
    }
    for i in retry {
        let c = &file.chunks[i];
        stats.refetched += 1;
        let bytes = fetch_range(fetch, file_url, c.offset..c.offset + c.size, &mut stats).await?;
        if manifest.verify_chunk(&file.path, &c.name, &bytes).is_err() {
            return Err(StreamError::Corrupt {
                file: file.path.clone(),
                chunk: c.name.clone(),
            });
        }
        got.insert(
            i,
            LoadedChunk {
                index: i,
                bytes,
                source: Source::Network,
            },
        );
    }
    for chunk in got.values().filter(|c| c.source == Source::Network) {
        let c = &file.chunks[chunk.index];
        stats.network_chunks += 1;
        progress(c.size, Source::Network);
        if let Some(cache) = cache {
            // Not cached (quota, no OPFS): the ride goes on without.
            let _ = cache
                .put(&manifest.id, manifest.version, c, &chunk.bytes)
                .await;
        }
    }
    let mut out = Vec::with_capacity(got.len());
    let mut seen = std::collections::BTreeSet::new();
    for &i in wanted {
        if seen.insert(i)
            && let Some(c) = got.remove(&i)
        {
            out.push(c);
        }
    }
    Ok((out, stats))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::stream::cache::{MemoryStore, ready};
    use crate::stream::plan::{base_chunks, tile_chunks};
    use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};
    use ocw_world::{RoadDress, RoadSurface};

    pub(crate) const URL: &str = "https://w.example/worlds/test/mini/3/mini.ocwpkg";

    /// A 4 × 3 tile package with a time zone section (version 2).
    pub(crate) fn package() -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for x in 0..4 {
            for z in 0..3 {
                let key = TileKey { x, z };
                let class = ocw_world::graph::RoadClass::Secondary;
                tiles.insert(
                    key,
                    Tile {
                        key,
                        heights: (0..9).map(|i| (i + 10 * x + 100 * z) as f32).collect(),
                        roads: vec![RoadStrip {
                            class,
                            width_m: 6.0,
                            points: vec![[1.0, 2.0, 3.0], [400.0, 2.0, 3.0]],
                            surface: RoadSurface::of_class(class),
                            dress: RoadDress::of_class(class, 6.0),
                        }],
                    },
                );
            }
        }
        WorldPackage {
            name: "Mini".into(),
            origin: (47.15, 9.52),
            tile_size_m: 1000.0,
            samples: 3,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: Some("Europe/Vaduz".into()),
            look: Default::default(),
        }
    }

    /// The package's bytes and its (unsigned) manifest.
    pub(crate) fn published() -> (Vec<u8>, Manifest) {
        let bytes = package().to_bytes();
        let mut m = Manifest::new("test/mini", "Mini", 3, 0, "test", vec![]).unwrap();
        m.add_file("mini.ocwpkg", &bytes).unwrap();
        (bytes, m)
    }

    fn load(
        fetch: &MemoryFetch,
        cache: Option<&ChunkCache<&MemoryStore>>,
        m: &Manifest,
        wanted: &[usize],
    ) -> Result<(Vec<LoadedChunk>, LoadStats), StreamError> {
        ready(load_chunks(
            fetch,
            cache,
            m,
            m.package_file().unwrap(),
            URL,
            wanted,
            RangeParams::default(),
            &mut |_, _| {},
        ))
    }

    #[test]
    fn the_base_comes_in_two_ranges_and_verifies() {
        let (bytes, m) = published();
        let fetch = MemoryFetch::with([(URL.to_string(), bytes.clone())]);
        let file = m.package_file().unwrap();
        let base = base_chunks(file);
        let (chunks, stats) = load(&fetch, None, &m, &base).unwrap();
        assert_eq!(chunks.len(), base.len());
        assert!(chunks.iter().all(|c| c.source == Source::Network));
        // The header alone, the sections together (the tiles in between
        // are larger than the gap that is worth fetching? 12 small tiles
        // are not: one request).
        assert!(stats.requests <= 2, "{stats:?}");
        let ranges: Vec<_> = fetch
            .requests()
            .into_iter()
            .map(|(_, r)| r.unwrap())
            .collect();
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, bytes.len() as u64);
        let s = ocw_world::package::StreamedPackage::from_parts(
            &chunks[0].bytes,
            &chunks[1..]
                .iter()
                .flat_map(|c| c.bytes.clone())
                .collect::<Vec<u8>>(),
        )
        .unwrap();
        assert_eq!(s.base().time_zone.as_deref(), Some("Europe/Vaduz"));
    }

    /// Verification failure handling: a corrupt chunk is fetched again on
    /// its own; corrupt again, the load fails naming the chunk; nothing
    /// damaged reaches the cache.
    #[test]
    fn a_corrupt_chunk_is_refetched_then_refused() {
        let (bytes, m) = published();
        let file = m.package_file().unwrap();
        let tiles = tile_chunks(file);
        let t = tiles[&TileKey { x: 1, z: 1 }];
        let c = &file.chunks[t];
        let fetch = MemoryFetch::with([(URL.to_string(), bytes.clone())]);
        // Damaged once (a bad transfer): the refetch is good.
        fetch.corrupt_at(URL, c.offset + 5, 1);
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 1 << 20));
        let wanted: Vec<usize> = tiles.values().copied().collect();
        let (chunks, stats) = load(&fetch, Some(&cache), &m, &wanted).unwrap();
        assert_eq!(chunks.len(), 12);
        assert_eq!(stats.refetched, 1);
        let again = fetch.requests().last().unwrap().1.clone().unwrap();
        assert_eq!(again, c.offset..c.offset + c.size);
        let good = &bytes[c.offset as usize..(c.offset + c.size) as usize];
        assert_eq!(chunks.iter().find(|x| x.index == t).unwrap().bytes, good);
        // Damaged for good (the server's copy): an error naming the chunk.
        let fetch = MemoryFetch::with([(URL.to_string(), bytes.clone())]);
        fetch.corrupt_at(URL, c.offset + 5, 10);
        let store = MemoryStore::default();
        let cache = ready(ChunkCache::open(&store, 1 << 20));
        let e = load(&fetch, Some(&cache), &m, &[t]).unwrap_err();
        assert!(
            matches!(&e, StreamError::Corrupt { chunk, .. } if chunk == "tile 1,1"),
            "{e}"
        );
        assert!(e.to_string().contains("damaged"), "{e}");
        assert!(ready(cache.get("test/mini", 3, c)).is_none());
        assert_eq!(cache.used(), 0);
        // A missing file is an HTTP error, not a panic.
        let fetch = MemoryFetch::default();
        assert!(matches!(
            load(&fetch, None, &m, &[t]),
            Err(StreamError::Status(_, 404))
        ));
    }

    /// The second visit: everything from the cache, no request at all.
    #[test]
    fn the_second_load_comes_from_the_cache() {
        let (bytes, m) = published();
        let file = m.package_file().unwrap();
        let all: Vec<usize> = (0..file.chunks.len()).collect();
        let store = MemoryStore::default();
        let fetch = MemoryFetch::with([(URL.to_string(), bytes.clone())]);
        {
            let cache = ready(ChunkCache::open(&store, 1 << 20));
            let (_, s) = load(&fetch, Some(&cache), &m, &all).unwrap();
            assert_eq!(s.network_chunks, file.chunks.len() as u64);
            assert_eq!(s.network_bytes, bytes.len() as u64);
        }
        let n = fetch.requests().len();
        let cache = ready(ChunkCache::open(&store, 1 << 20));
        let mut from_cache = 0;
        let (chunks, s) = ready(load_chunks(
            &fetch,
            Some(&cache),
            &m,
            file,
            URL,
            &all,
            RangeParams::default(),
            &mut |b, src| {
                assert_eq!(src, Source::Cache);
                from_cache += b;
            },
        ))
        .unwrap();
        assert_eq!(fetch.requests().len(), n);
        assert_eq!(
            (s.cache_chunks, s.cache_bytes),
            (all.len() as u64, bytes.len() as u64)
        );
        assert_eq!(from_cache, bytes.len() as u64);
        let joined: Vec<u8> = chunks.into_iter().flat_map(|c| c.bytes).collect();
        assert_eq!(joined, bytes);
    }

    /// A server without range support: the whole file comes, the chunks
    /// are cut out of it and verified the same way.
    #[test]
    fn a_server_without_ranges_still_works() {
        let (bytes, m) = published();
        let file = m.package_file().unwrap();
        let mut fetch = MemoryFetch::with([(URL.to_string(), bytes.clone())]);
        fetch.ignore_ranges = true;
        let t = tile_chunks(file)[&TileKey { x: 3, z: 2 }];
        let (chunks, _) = load(&fetch, None, &m, &[t]).unwrap();
        let c = &file.chunks[t];
        assert_eq!(
            chunks[0].bytes,
            &bytes[c.offset as usize..(c.offset + c.size) as usize]
        );
    }
}
