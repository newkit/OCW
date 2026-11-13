//! World packages on disk and on the wire (spec §4.7, §12): key files,
//! the trusted roots, the local package cache, loading with verification
//! and downloading over HTTP(S).
//!
//! The verification itself is pure and lives in [`ocw_world::manifest`];
//! this crate adds the file system, the clock and the network, so it is
//! used by the desktop app, the server and the tools, never by wasm code.
//!
//! - [`keys`]: key files (secret ones readable only by their owner)
//! - [`trust`]: which root keys a build trusts (development roots, files)
//! - [`cache`]: `<cache>/<id>/<version>/{manifest.json, files}`
//! - [`load`]: `--world PATH | ID[@VERSION]` with verify-on-load
//! - [`fetch`]: download a package into the cache, resumable, verified
//! - [`neighbours`]: installed neighbour packages joined to a world set
//! - [`discover`]: the worlds that can be ridden here (the World menu)

pub mod cache;
pub mod discover;
pub mod fetch;
pub mod keys;
pub mod load;
pub mod neighbours;
pub mod trust;

pub use cache::{CacheEntry, WorldCache};
pub use fetch::{FileTransport, Transport, UreqTransport, fetch};
pub use load::{LoadError, LoadOptions, LoadedWorld, load_world, manifest_path_for};
pub use neighbours::{Neighbourhood, neighbourhood};

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
