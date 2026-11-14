//! The loading screen of a streamed world (spec §14 Phase 10 B2): what is
//! done (manifest, road graph and sections, the first tiles), how much
//! came from the cache and how much over the network, the storage used.
//! Pure text, so the app only puts the lines on the screen.

use super::load::{LoadStats, Source};

/// Where a load is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    #[default]
    Manifest,
    /// Header and sections: road graph, routes, land use, buildings …
    Base,
    /// The tiles around the start.
    FirstTiles,
    Riding,
    Failed,
}

/// The browser's storage for the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Storage {
    /// Bytes in the chunk cache.
    pub used: u64,
    /// The cache's size budget.
    pub budget: u64,
    /// The browser's quota for the origin (all storage), if known.
    pub quota: Option<u64>,
    /// The browser keeps the storage under pressure (`persist()`).
    pub persisted: Option<bool>,
    /// No OPFS: chunks are not cached (a private window, an old browser).
    pub unavailable: bool,
}

/// Progress of loading one world.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadProgress {
    /// What `?world=` said.
    pub world: String,
    pub stage: Stage,
    /// "Liechtenstein (liechtenstein@21)", once the manifest is in.
    pub package: Option<String>,
    /// The trust line ("signed by …" or "NOT TRUSTED: …").
    pub trust: Option<String>,
    pub manifest_bytes: u64,
    /// Bytes of the base (header and sections): done / total.
    pub base_done: u64,
    pub base_total: u64,
    /// The first tiles: done / wanted.
    pub tiles_done: u32,
    pub tiles_wanted: u32,
    pub stats: LoadStats,
    pub storage: Option<Storage>,
    /// Time of each finished stage since the start (ms).
    pub manifest_ms: Option<f64>,
    pub base_ms: Option<f64>,
    pub first_tiles_ms: Option<f64>,
    pub error: Option<String>,
    /// Package file size (bytes), for "of 35.2 MB".
    pub package_bytes: u64,
}

/// `12.3 MB` (10^6 bytes, as browsers show storage).
pub fn mb(bytes: u64) -> String {
    let m = bytes as f64 / 1e6;
    if m >= 100.0 {
        format!("{m:.0} MB")
    } else if m >= 1.0 || bytes == 0 {
        format!("{m:.1} MB")
    } else {
        format!("{:.0} kB", bytes as f64 / 1e3)
    }
}

impl LoadProgress {
    pub fn new(world: &str) -> Self {
        Self {
            world: world.into(),
            ..Self::default()
        }
    }

    /// Counts a chunk (base or first tiles).
    pub fn chunk(&mut self, bytes: u64, source: Source) {
        match source {
            Source::Cache => {
                self.stats.cache_bytes += bytes;
                self.stats.cache_chunks += 1;
            }
            Source::Network => {
                self.stats.network_bytes += bytes;
                self.stats.network_chunks += 1;
            }
        }
    }

    fn done(at: Option<f64>) -> String {
        at.map_or(String::new(), |ms| format!(" ({:.1} s)", ms / 1000.0))
    }

    /// The loading screen, one line each.
    pub fn lines(&self) -> Vec<String> {
        let mark = |stage: Stage| {
            if self.stage == Stage::Failed {
                "  "
            } else if self.stage > stage {
                "✓ "
            } else if self.stage == stage {
                "… "
            } else {
                "  "
            }
        };
        let mut out = vec![format!(
            "Loading world {}",
            self.package.as_deref().unwrap_or(&self.world)
        )];
        out.push(format!(
            "{}manifest{}{}",
            mark(Stage::Manifest),
            if self.manifest_bytes > 0 {
                format!(" {}", mb(self.manifest_bytes))
            } else {
                String::new()
            },
            Self::done(self.manifest_ms)
        ));
        if let Some(t) = &self.trust {
            out.push(format!("  {t}"));
        }
        out.push(format!(
            "{}road graph, routes, land use, buildings: {} of {}{}",
            mark(Stage::Base),
            mb(self.base_done),
            mb(self.base_total),
            Self::done(self.base_ms)
        ));
        out.push(format!(
            "{}first tiles: {} of {}{}",
            mark(Stage::FirstTiles),
            self.tiles_done,
            self.tiles_wanted,
            Self::done(self.first_tiles_ms)
        ));
        let s = &self.stats;
        out.push(format!(
            "from cache {} ({} chunks), from network {} ({} chunks){}",
            mb(s.cache_bytes),
            s.cache_chunks,
            mb(s.network_bytes),
            s.network_chunks,
            if self.package_bytes > 0 {
                format!(", package {}", mb(self.package_bytes))
            } else {
                String::new()
            }
        ));
        if let Some(st) = &self.storage {
            out.push(storage_line(st));
        }
        if let Some(e) = &self.error {
            out.push(format!("Error: {e}"));
        }
        out
    }

    /// One line for the log when the ride starts.
    pub fn summary(&self) -> String {
        let s = &self.stats;
        format!(
            "world ready: {} in {:.2} s (manifest {:.2} s, base {:.2} s), from cache {} in {} chunks, from network {} in {} chunks, {} requests",
            self.package.as_deref().unwrap_or(&self.world),
            self.first_tiles_ms.unwrap_or(0.0) / 1000.0,
            self.manifest_ms.unwrap_or(0.0) / 1000.0,
            self.base_ms.unwrap_or(0.0) / 1000.0,
            mb(s.cache_bytes),
            s.cache_chunks,
            mb(s.network_bytes),
            s.network_chunks,
            s.requests
        )
    }
}

/// "storage: 34.1 MB of 1074 MB cache budget (browser quota 60 GB, persistent)".
pub fn storage_line(st: &Storage) -> String {
    if st.unavailable {
        return "storage: no Origin Private File System here: the world is not cached".into();
    }
    let quota = st
        .quota
        .map_or(String::new(), |q| format!(", browser quota {}", mb(q)));
    let persisted = match st.persisted {
        Some(true) => ", persistent",
        Some(false) => ", may be cleared by the browser",
        None => "",
    };
    format!(
        "storage: {} of {} cache budget in OPFS{quota}{persisted}",
        mb(st.used),
        mb(st.budget)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(mb(35_229_420), "35.2 MB");
        assert_eq!(mb(1 << 30), "1074 MB");
        assert_eq!(mb(53), "0 kB");
        assert_eq!(mb(0), "0.0 MB");
        assert_eq!(mb(98_000), "98 kB");
    }

    #[test]
    fn the_loading_screen_shows_each_stage() {
        let mut p = LoadProgress::new("liechtenstein");
        let l = p.lines();
        assert_eq!(l[0], "Loading world liechtenstein");
        assert!(l[1].starts_with("… manifest"), "{l:?}");
        p.package = Some("Liechtenstein (liechtenstein@21)".into());
        p.trust = Some("NOT TRUSTED: the package is not signed".into());
        p.manifest_bytes = 104_242;
        p.manifest_ms = Some(40.0);
        p.stage = Stage::Base;
        p.base_total = 11_223_000;
        p.base_done = 5_000_000;
        p.chunk(5_000_000, Source::Network);
        p.chunk(1_000, Source::Cache);
        p.package_bytes = 35_229_420;
        p.storage = Some(Storage {
            used: 5_001_000,
            budget: 1 << 30,
            quota: Some(60_000_000_000),
            persisted: Some(true),
            unavailable: false,
        });
        let l = p.lines();
        assert_eq!(l[0], "Loading world Liechtenstein (liechtenstein@21)");
        assert_eq!(l[1], "✓ manifest 104 kB (0.0 s)");
        assert!(l[2].contains("NOT TRUSTED"));
        assert_eq!(
            l[3],
            "… road graph, routes, land use, buildings: 5.0 MB of 11.2 MB"
        );
        assert_eq!(l[4], "  first tiles: 0 of 0");
        assert_eq!(
            l[5],
            "from cache 1 kB (1 chunks), from network 5.0 MB (1 chunks), package 35.2 MB"
        );
        assert_eq!(
            l[6],
            "storage: 5.0 MB of 1074 MB cache budget in OPFS, browser quota 60000 MB, persistent"
        );
        p.stage = Stage::Failed;
        p.error = Some("tile 3,4 is damaged".into());
        let l = p.lines();
        assert!(l.last().unwrap().starts_with("Error: tile 3,4"));
        assert!(!l[1].starts_with('✓'));
        assert!(
            storage_line(&Storage {
                unavailable: true,
                ..Storage::default()
            })
            .contains("not cached")
        );
        assert!(p.summary().starts_with("world ready: Liechtenstein"));
    }
}
