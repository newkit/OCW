//! World packages on disk and on the wire (spec §4.7, §12): key files,
//! the trusted roots, the local package cache, loading with verification
//! and downloading over HTTP(S).
//!
//! The verification itself is pure and lives in [`ocw_world::manifest`];
//! this crate adds the file system, the clock and the network, so most of
//! it is used by the desktop app, the server and the tools. The browser
//! app (wasm32, spec §14 Phase 10 B2) uses only [`stream`] and [`trust`].
//!
//! - [`keys`]: key files (secret ones readable only by their owner)
//! - [`trust`]: which root keys a build trusts (development roots, files)
//! - [`cache`]: `<cache>/<id>/<version>/{manifest.json, files}`
//! - [`load`]: `--world PATH | ID[@VERSION]` with verify-on-load
//! - [`fetch`]: download a package into the cache, resumable, verified
//! - [`neighbours`]: installed neighbour packages joined to a world set
//! - [`discover`]: the worlds that can be ridden here (the World menu)
//! - [`stream`]: a package streamed by chunk over HTTP range requests,
//!   verified chunk by chunk, cached behind a [`stream::ChunkStore`]
//!   (the browser's OPFS), with the tiles kept resident around the rider

#[cfg(not(target_arch = "wasm32"))]
pub mod cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod discover;
#[cfg(not(target_arch = "wasm32"))]
pub mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub mod keys;
#[cfg(not(target_arch = "wasm32"))]
pub mod load;
#[cfg(not(target_arch = "wasm32"))]
pub mod neighbours;
pub mod stream;
pub mod trust;

#[cfg(not(target_arch = "wasm32"))]
pub use cache::{CacheEntry, WorldCache};
#[cfg(not(target_arch = "wasm32"))]
pub use fetch::{FileTransport, Transport, UreqTransport, fetch};
#[cfg(not(target_arch = "wasm32"))]
pub use load::{
    LoadError, LoadOptions, LoadedWorld, load_world, load_world_skeleton, manifest_path_for,
};
#[cfg(not(target_arch = "wasm32"))]
pub use neighbours::{Neighbourhood, neighbourhood};

/// Seconds since the Unix epoch (0 if the clock is before it).
#[cfg(not(target_arch = "wasm32"))]
pub fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
