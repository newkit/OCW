//! World packages streamed over HTTP (spec §4.5, §4.7; the browser app,
//! spec §14 Phase 10 B2). Pure and transport-independent, so it builds for
//! wasm32 and is unit-tested natively:
//!
//! 1. **Manifest first** ([`accept_manifest`], shared with
//!    `--fetch-world`): parsed, its signature chain checked against the
//!    trusted roots before anything large is fetched.
//! 2. **Range planning** ([`plan`]): which chunks of the package file a
//!    position needs (the `header`, the sections – road graph, routes,
//!    land use, buildings … – and the tiles around the rider), merged into
//!    few `Range: bytes=a-b` requests.
//! 3. **Chunk verification** ([`load::load_chunks`]): every chunk is
//!    checked against its SHA-256 in the manifest before it is used or
//!    cached; a damaged one is fetched once more on its own, then the load
//!    fails with a message naming it.
//! 4. **The chunk cache** ([`cache::ChunkCache`]): verified chunks per
//!    package id / version / chunk behind a [`ChunkStore`] (the browser's
//!    Origin Private File System; [`cache::MemoryStore`] in tests), with
//!    an index and least-recently-used eviction by a size budget.
//! 5. **Residency** ([`resident::ResidentTiles`]): the decoded tiles kept
//!    in memory around the rider, fetched on demand within a radius,
//!    dropped beyond a larger one or when over the memory budget.
//! 6. **The driver** ([`driver::StreamDriver`]): the whole sequence as
//!    small async steps over a [`RangeFetch`] and a [`ChunkStore`]; the
//!    app exchanges requests, arrived tiles and the [`progress`] (the
//!    loading screen) through [`driver::StreamShared`].
//!
//! Package URLs follow `world-builder publish`:
//! `<base>/<id>/<version>/manifest.json` next to the package file, and
//! `<base>/<id>/latest` with the newest version number.

pub mod cache;
pub mod driver;
pub mod load;
pub mod plan;
pub mod progress;
pub mod resident;

pub use cache::{ChunkCache, ChunkStore, MemoryStore};
pub use load::{HttpResponse, MemoryFetch, RangeFetch, Source, StreamError};

use ocw_world::manifest::{
    SignedManifest, TrustStatus, TrustStore, UnsignedPolicy, VerifyError, apply_policy,
};

/// The manifest's file name next to the package (`world-builder publish`).
pub const MANIFEST_FILE: &str = "manifest.json";
/// The file with the newest published version of a package id.
pub const LATEST_FILE: &str = "latest";
/// Largest manifest accepted (a 500 MB region has ~1000 tile chunks).
pub const MAX_MANIFEST_BYTES: u64 = 16 << 20;

/// Parses a manifest and checks its signature chain (spec §4.7) under
/// `policy`: what both the download (`--fetch-world`) and the streaming
/// client do before they fetch any package bytes.
pub fn accept_manifest(
    text: &str,
    trust: &TrustStore,
    policy: UnsignedPolicy,
    now_unix_s: i64,
) -> Result<(SignedManifest, TrustStatus), VerifyError> {
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(VerifyError::Malformed("manifest too large".into()));
    }
    let signed = SignedManifest::parse(text)?;
    let status = apply_policy(signed.verify_signature(trust, now_unix_s), policy)?;
    Ok((signed, status))
}

/// URL of `file` next to the manifest at `manifest_url`.
pub fn sibling_url(manifest_url: &str, file: &str) -> String {
    // Windows paths (`C:\w\manifest.json`) also separate with `\`; on Unix
    // a backslash is an ordinary file name character.
    #[cfg(windows)]
    let sep = if manifest_url.contains("://") && !manifest_url.starts_with("file://") {
        manifest_url.rfind('/')
    } else {
        manifest_url.rfind(['/', '\\'])
    };
    #[cfg(not(windows))]
    let sep = manifest_url.rfind('/');
    match sep {
        Some(i) => format!("{}{file}", &manifest_url[..=i]),
        None => file.to_string(),
    }
}

/// What a page's `?world=` names (spec §14 Phase 10 B2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldRef {
    /// A manifest URL (absolute, or relative to the page).
    Manifest(String),
    /// A package id: its newest version, from `<base><id>/latest`.
    Latest { id: String, latest_url: String },
}

impl WorldRef {
    /// `?world=` → what to fetch: a URL (with `://`, ending in `.json`,
    /// or starting with `/` or `.`) as it is; `ID@VERSION` as
    /// `<base>ID/VERSION/manifest.json`; `ID` via `<base>ID/latest`.
    /// `base` is where `world-builder publish` output is served (e.g.
    /// `worlds/`, relative to the page).
    pub fn parse(spec: &str, base: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err("empty world".into());
        }
        if spec.contains("://")
            || spec.ends_with(".json")
            || spec.starts_with('/')
            || spec.starts_with('.')
        {
            return Ok(Self::Manifest(spec.into()));
        }
        let base = if base.is_empty() || base.ends_with('/') {
            base.to_string()
        } else {
            format!("{base}/")
        };
        let (id, version) = match spec.split_once('@') {
            Some((id, v)) => (
                id,
                Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("{spec}: the version after @ is not a number"))?,
                ),
            ),
            None => (spec, None),
        };
        ocw_world::manifest::validate_id(id)?;
        Ok(match version {
            Some(v) => Self::Manifest(format!("{base}{id}/{v}/{MANIFEST_FILE}")),
            None => Self::Latest {
                id: id.into(),
                latest_url: format!("{base}{id}/{LATEST_FILE}"),
            },
        })
    }

    /// The manifest URL once `<id>/latest` said `latest`.
    pub fn manifest_url_from_latest(latest_url: &str, latest: &str) -> Result<String, String> {
        let v: u64 = latest
            .trim()
            .parse()
            .map_err(|_| format!("{latest_url}: not a version number"))?;
        Ok(sibling_url(latest_url, &format!("{v}/{MANIFEST_FILE}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::manifest::{Manifest, RootKeyPair, SigningKeyPair};

    #[test]
    fn world_options() {
        assert_eq!(
            WorldRef::parse("liechtenstein", "worlds/").unwrap(),
            WorldRef::Latest {
                id: "liechtenstein".into(),
                latest_url: "worlds/liechtenstein/latest".into()
            }
        );
        assert_eq!(
            WorldRef::parse("europe/liechtenstein@21", "/w").unwrap(),
            WorldRef::Manifest("/w/europe/liechtenstein/21/manifest.json".into())
        );
        for url in [
            "https://worlds.example/li/3/manifest.json",
            "/worlds/li/3/manifest.json",
            "./m.json",
        ] {
            assert_eq!(
                WorldRef::parse(url, "worlds/").unwrap(),
                WorldRef::Manifest(url.into())
            );
        }
        assert!(WorldRef::parse("Bad Id", "worlds/").is_err());
        assert!(WorldRef::parse("li@x", "worlds/").is_err());
        assert!(WorldRef::parse(" ", "worlds/").is_err());
        assert_eq!(
            WorldRef::manifest_url_from_latest("worlds/li/latest", "21\n").unwrap(),
            "worlds/li/21/manifest.json"
        );
        assert!(WorldRef::manifest_url_from_latest("worlds/li/latest", "<html>").is_err());
    }

    #[test]
    fn manifests_are_accepted_by_signature_and_policy() {
        let root = RootKeyPair::from_seed("root", [7; 32]);
        let cert = root.certify("signer", SigningKeyPair::public_key_of([8; 32]), 0, 100_000);
        let key = SigningKeyPair::new([8; 32], cert).unwrap();
        let mut m = Manifest::new("test/mini", "Mini", 1, 500, "test", vec![]).unwrap();
        m.add_file("mini.ocwpkg", b"not a package").unwrap();
        let trust = TrustStore::new([root.public()]);
        let signed = SignedManifest::signed(&m, &key).to_json();
        let (s, status) = accept_manifest(&signed, &trust, UnsignedPolicy::Refuse, 1000).unwrap();
        assert!(status.is_trusted());
        assert_eq!(s.manifest.id, "test/mini");
        // Another root: refused (with Warn: loaded, but not trusted).
        let other = TrustStore::new([RootKeyPair::from_seed("x", [9; 32]).public()]);
        assert!(matches!(
            accept_manifest(&signed, &other, UnsignedPolicy::Refuse, 1000),
            Err(VerifyError::UnknownRoot(_))
        ));
        // Unsigned: refused by default, accepted with a warning status.
        let unsigned = SignedManifest::unsigned(&m).to_json();
        assert!(matches!(
            accept_manifest(&unsigned, &trust, UnsignedPolicy::Refuse, 1000),
            Err(VerifyError::Unsigned)
        ));
        let (_, st) = accept_manifest(&unsigned, &trust, UnsignedPolicy::Warn, 1000).unwrap();
        assert!(!st.is_trusted());
        // Tampered signed bytes break the signature: refused even with Warn.
        let tampered = signed.replace("\"Mini\"", "\"Maxi\"");
        assert!(matches!(
            accept_manifest(&tampered, &trust, UnsignedPolicy::Warn, 1000),
            Err(VerifyError::BadSignature(_))
        ));
        assert!(accept_manifest("{", &trust, UnsignedPolicy::Warn, 1000).is_err());
    }

    #[test]
    fn sibling_urls() {
        assert_eq!(
            sibling_url("https://h/w/a/3/manifest.json", "a.ocwpkg"),
            "https://h/w/a/3/a.ocwpkg"
        );
        assert_eq!(sibling_url("manifest.json", "a"), "a");
    }
}
