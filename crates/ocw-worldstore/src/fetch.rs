//! Downloading a package into the cache (spec §4.7): manifest first (its
//! signature is checked before any large download), then each file,
//! resumable with HTTP range requests, verified against the manifest
//! before the entry is moved into the cache.
//!
//! The manifest also lists every tile-sized chunk with offset, size and
//! hash, so a later client can fetch single tiles with `Range: bytes=a-b`
//! and verify each on its own (streaming, spec §4.5); today whole files
//! are downloaded once, before the ride.
//!
//! `URL` is the manifest's URL; files are fetched next to it
//! (`https://host/worlds/europe/liechtenstein/3/manifest.json` →
//! `.../3/liechtenstein.ocwpkg`), the layout `world-builder publish` writes.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ocw_world::manifest::{
    SignedManifest, TrustStatus, TrustStore, UnsignedPolicy, VerifyError, apply_policy,
};

use crate::cache::{CacheEntry, MANIFEST_FILE, WorldCache};

/// Largest manifest accepted (a 500 MB region has ~1000 tile chunks).
const MAX_MANIFEST_BYTES: u64 = 16 << 20;

/// A response to a GET.
pub struct Response {
    /// 200 (whole file) or 206 (from the requested byte); others are errors.
    pub status: u16,
    /// For 206: the first byte of the range (`Content-Range`).
    pub range_start: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// How bytes are fetched (HTTP, local files, or a mock in tests).
pub trait Transport: Send + Sync {
    /// GET `url`; from byte `from` on if `from > 0` (a range request).
    fn get(&self, url: &str, from: u64) -> Result<Response, String>;
}

/// HTTP(S) with `ureq` (rustls).
pub struct UreqTransport(ureq::Agent);

impl Default for UreqTransport {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(concat!("OpenCyclingWorld/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self(agent)
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &str, from: u64) -> Result<Response, String> {
        let mut req = self.0.get(url);
        if from > 0 {
            req = req.header("Range", format!("bytes={from}-"));
        }
        let resp = req.call().map_err(|e| format!("{url}: {e}"))?;
        let status = resp.status().as_u16();
        let range_start = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_start);
        Ok(Response {
            status,
            range_start,
            body: Box::new(resp.into_body().into_reader()),
        })
    }
}

/// `bytes 100-199/200` → 100.
fn parse_content_range_start(v: &str) -> Option<u64> {
    v.trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Local files: `file:///path` or a plain path (installing from a
/// directory that `world-builder publish` wrote, or a USB stick).
pub struct FileTransport;

impl Transport for FileTransport {
    fn get(&self, url: &str, from: u64) -> Result<Response, String> {
        use std::io::Seek;
        let path = url.strip_prefix("file://").unwrap_or(url);
        let mut f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        if from > 0 {
            f.seek(std::io::SeekFrom::Start(from))
                .map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(Response {
            status: if from > 0 { 206 } else { 200 },
            range_start: (from > 0).then_some(from),
            body: Box::new(f),
        })
    }
}

/// HTTP(S) for `http://` and `https://`, else local files.
pub fn transport_for(url: &str) -> Box<dyn Transport> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Box::new(UreqTransport::default())
    } else {
        Box::new(FileTransport)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("{0}")]
    Transport(String),
    #[error("{0}: HTTP status {1}")]
    Status(String, u16),
    #[error("{0}")]
    Verify(#[from] VerifyError),
    #[error("{0}")]
    Io(String),
}

/// A finished download.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub entry: CacheEntry,
    pub trust: TrustStatus,
    /// Bytes transferred (0 when it was already installed).
    pub downloaded: u64,
}

/// URL of `file` next to the manifest at `manifest_url`.
pub fn sibling_url(manifest_url: &str, file: &str) -> String {
    match manifest_url.rfind('/') {
        Some(i) => format!("{}{file}", &manifest_url[..=i]),
        None => file.to_string(),
    }
}

fn io(path: &Path, e: impl std::fmt::Display) -> FetchError {
    FetchError::Io(format!("{}: {e}", path.display()))
}

/// Downloads the package whose manifest is at `manifest_url` into `cache`.
/// `progress(file, bytes so far, file size)` is called while downloading.
pub fn fetch(
    manifest_url: &str,
    cache: &WorldCache,
    trust: &TrustStore,
    policy: UnsignedPolicy,
    now_unix_s: i64,
    transport: &dyn Transport,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<Fetched, FetchError> {
    let resp = transport
        .get(manifest_url, 0)
        .map_err(FetchError::Transport)?;
    if resp.status != 200 {
        return Err(FetchError::Status(manifest_url.into(), resp.status));
    }
    let mut text = String::new();
    resp.body
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| FetchError::Transport(format!("{manifest_url}: {e}")))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(FetchError::Verify(VerifyError::Malformed(
            "manifest too large".into(),
        )));
    }
    let signed = SignedManifest::parse(&text)?;
    // Refuse untrusted packages before downloading hundreds of MB.
    let status = apply_policy(signed.verify_signature(trust, now_unix_s), policy)?;
    let m = &signed.manifest;

    if let Some(entry) = cache.resolve(&format!("{}@{}", m.id, m.version))
        && std::fs::read_to_string(entry.manifest_path()).is_ok_and(|t| t == text)
        && m.files.iter().all(|f| {
            std::fs::read(entry.dir.join(&f.path)).is_ok_and(|b| m.verify_file(&f.path, &b).is_ok())
        })
    {
        return Ok(Fetched {
            entry,
            trust: status,
            downloaded: 0,
        });
    }

    let staging = cache.partial_dir(&m.id, m.version);
    let staged_manifest = staging.join(MANIFEST_FILE);
    // A different build under the same version: start afresh.
    if std::fs::read_to_string(&staged_manifest).is_ok_and(|t| t != text) {
        std::fs::remove_dir_all(&staging).map_err(|e| io(&staging, e))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| io(&staging, e))?;
    std::fs::write(&staged_manifest, &text).map_err(|e| io(&staged_manifest, e))?;

    let mut downloaded = 0;
    for f in &m.files {
        let done = staging.join(&f.path);
        if std::fs::read(&done).is_ok_and(|b| m.verify_file(&f.path, &b).is_ok()) {
            continue;
        }
        let part = staging.join(format!("{}.part", f.path));
        downloaded += download(
            transport,
            &sibling_url(manifest_url, &f.path),
            &part,
            f.size,
            &mut |n| progress(&f.path, n, f.size),
        )?;
        let bytes = std::fs::read(&part).map_err(|e| io(&part, e))?;
        if let Err(e) = m.verify_file(&f.path, &bytes) {
            // Damaged or tampered: never resume from these bytes.
            let _ = std::fs::remove_file(&part);
            return Err(e.into());
        }
        std::fs::rename(&part, &done).map_err(|e| io(&done, e))?;
    }
    let entry = cache
        .commit(&staging, &m.id, m.version)
        .map_err(FetchError::Io)?;
    Ok(Fetched {
        entry,
        trust: status,
        downloaded,
    })
}

/// Downloads `url` into `part`, resuming after the bytes already there.
/// Returns the bytes transferred.
fn download(
    transport: &dyn Transport,
    url: &str,
    part: &PathBuf,
    size: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, FetchError> {
    let mut have = std::fs::metadata(part).map_or(0, |m| m.len());
    if have > size {
        have = 0;
    }
    if have == size && size > 0 {
        return Ok(0);
    }
    let resp = transport.get(url, have).map_err(FetchError::Transport)?;
    let append = match resp.status {
        206 if have > 0 && resp.range_start == Some(have) => true,
        200 => false,
        s => return Err(FetchError::Status(url.into(), s)),
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part)
        .map_err(|e| io(part, e))?;
    let mut at = if append { have } else { 0 };
    let mut body = resp.body;
    let mut buf = vec![0u8; 256 << 10];
    let mut transferred = 0;
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                // Keep what arrived: the next fetch resumes from here.
                let _ = file.sync_all();
                return Err(FetchError::Transport(format!(
                    "{url}: {e} (after {at} of {size} bytes; run again to resume)"
                )));
            }
        };
        if at + n as u64 > size {
            drop(file);
            let _ = std::fs::remove_file(part);
            return Err(VerifyError::SizeMismatch {
                path: url.into(),
                expected: size,
                actual: at + n as u64,
            }
            .into());
        }
        file.write_all(&buf[..n]).map_err(|e| io(part, e))?;
        at += n as u64;
        transferred += n as u64;
        progress(at);
    }
    file.sync_all().map_err(|e| io(part, e))?;
    Ok(transferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::tests::{NOW, keys, package_bytes, write_package};
    use crate::load::{LoadOptions, load_world};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Serves files from memory; can cut one download short once.
    #[derive(Default)]
    struct Mock {
        files: HashMap<String, Vec<u8>>,
        requests: Mutex<Vec<(String, u64)>>,
        cut_after: Mutex<Option<(String, usize)>>,
        ignore_ranges: bool,
    }

    struct Failing(std::io::Cursor<Vec<u8>>);
    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.read(buf)? {
                0 => Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset",
                )),
                n => Ok(n),
            }
        }
    }

    impl Transport for Mock {
        fn get(&self, url: &str, from: u64) -> Result<Response, String> {
            self.requests.lock().unwrap().push((url.into(), from));
            let data = self.files.get(url).ok_or("not found")?;
            let from = if self.ignore_ranges { 0 } else { from };
            let body = data[from as usize..].to_vec();
            let cut = self.cut_after.lock().unwrap().take_if(|(u, _)| u == url);
            Ok(Response {
                status: if from > 0 { 206 } else { 200 },
                range_start: (from > 0).then_some(from),
                body: match cut {
                    Some((_, n)) => Box::new(Failing(std::io::Cursor::new(body[..n].to_vec()))),
                    None => Box::new(std::io::Cursor::new(body)),
                },
            })
        }
    }

    const BASE: &str = "https://example.invalid/worlds/test/mini/2/";

    fn mock(signed: bool) -> Mock {
        let tmp = tempfile::tempdir().unwrap();
        write_package(tmp.path(), signed, false, 2);
        let mut files = HashMap::new();
        for name in [MANIFEST_FILE, "mini.ocwpkg"] {
            files.insert(
                format!("{BASE}{name}"),
                std::fs::read(tmp.path().join(name)).unwrap(),
            );
        }
        Mock {
            files,
            ..Mock::default()
        }
    }

    fn trust() -> TrustStore {
        TrustStore::new([keys().0.public()])
    }

    fn run(m: &Mock, cache: &WorldCache, policy: UnsignedPolicy) -> Result<Fetched, FetchError> {
        fetch(
            &format!("{BASE}{MANIFEST_FILE}"),
            cache,
            &trust(),
            policy,
            NOW,
            m,
            &mut |_, _, _| {},
        )
    }

    #[test]
    fn interrupted_download_resumes_and_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        let m = mock(true);
        let pkg_url = format!("{BASE}mini.ocwpkg");
        *m.cut_after.lock().unwrap() = Some((pkg_url.clone(), 20));
        let e = run(&m, &cache, UnsignedPolicy::Refuse).unwrap_err();
        assert!(e.to_string().contains("resume"), "{e}");
        assert!(
            cache.resolve("test/mini").is_none(),
            "nothing installed yet"
        );
        let f = run(&m, &cache, UnsignedPolicy::Refuse).unwrap();
        assert!(f.trust.is_trusted());
        assert_eq!(f.downloaded, package_bytes().len() as u64 - 20);
        // The second request resumed after the 20 bytes.
        assert_eq!(m.requests.lock().unwrap().last(), Some(&(pkg_url, 20)));
        // It loads from the cache by id, verified, as the client does.
        let opts = LoadOptions {
            now_unix_s: NOW,
            ..LoadOptions::new(trust(), cache.clone())
        };
        let w = load_world("test/mini", &opts).unwrap();
        assert!(w.trust.is_trusted());
        assert_eq!(w.package.name, "Mini");
        // Fetching again downloads nothing.
        assert_eq!(
            run(&m, &cache, UnsignedPolicy::Refuse).unwrap().downloaded,
            0
        );
        assert!(!cache.partial_dir("test/mini", 2).exists());
    }

    #[test]
    fn server_without_ranges_restarts_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        let mut m = mock(true);
        m.ignore_ranges = true;
        *m.cut_after.lock().unwrap() = Some((format!("{BASE}mini.ocwpkg"), 20));
        assert!(run(&m, &cache, UnsignedPolicy::Refuse).is_err());
        let f = run(&m, &cache, UnsignedPolicy::Refuse).unwrap();
        assert_eq!(f.downloaded, package_bytes().len() as u64);
    }

    #[test]
    fn untrusted_or_damaged_packages_are_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        // Unsigned: refused before the package is requested.
        let m = mock(false);
        let e = run(&m, &cache, UnsignedPolicy::Refuse).unwrap_err();
        assert!(
            matches!(e, FetchError::Verify(VerifyError::Unsigned)),
            "{e}"
        );
        assert_eq!(m.requests.lock().unwrap().len(), 1);
        // ... but installed with --allow-unsigned.
        assert!(
            !run(&m, &cache, UnsignedPolicy::Warn)
                .unwrap()
                .trust
                .is_trusted()
        );
        // A tampered package file: refused, partial bytes deleted.
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        let mut m = mock(true);
        m.files.get_mut(&format!("{BASE}mini.ocwpkg")).unwrap()[40] ^= 1;
        let e = run(&m, &cache, UnsignedPolicy::Warn).unwrap_err();
        assert!(
            matches!(e, FetchError::Verify(VerifyError::HashMismatch(_))),
            "{e}"
        );
        assert!(
            !cache
                .partial_dir("test/mini", 2)
                .join("mini.ocwpkg.part")
                .exists()
        );
        assert!(cache.resolve("test/mini").is_none());
        // A missing file is an error, not a crash.
        let mut m = mock(true);
        m.files.remove(&format!("{BASE}mini.ocwpkg"));
        assert!(matches!(
            run(&m, &cache, UnsignedPolicy::Refuse),
            Err(FetchError::Transport(_))
        ));
    }

    #[test]
    fn urls_and_ranges() {
        assert_eq!(
            sibling_url("https://h/w/a/3/manifest.json", "a.ocwpkg"),
            "https://h/w/a/3/a.ocwpkg"
        );
        assert_eq!(sibling_url("manifest.json", "a"), "a");
        assert_eq!(sibling_url("/srv/w/manifest.json", "a"), "/srv/w/a");
        assert_eq!(parse_content_range_start("bytes 100-199/200"), Some(100));
        assert_eq!(parse_content_range_start("bytes */200"), None);
    }

    /// One-request-per-connection HTTP/1.1 server on localhost with Range
    /// support (no network needed); records the Range headers it got.
    fn serve(files: HashMap<String, Vec<u8>>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut from = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap_or(0);
                        log2.lock()
                            .unwrap()
                            .push(format!("{path} {}", lower.trim()));
                    }
                }
                let reply = match files.get(&path) {
                    Some(data) if from > 0 && from < data.len() => {
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nConnection: close\r\n\r\n",
                            data.len() - from,
                            data.len() - 1,
                            data.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&data[from..]);
                        r
                    }
                    Some(data) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(data);
                        r
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&reply);
            }
        });
        (format!("http://{addr}"), log)
    }

    #[test]
    fn http_download_with_range_resume() {
        let src = tempfile::tempdir().unwrap();
        write_package(src.path(), true, false, 2);
        let mut files = HashMap::new();
        for name in [MANIFEST_FILE, "mini.ocwpkg"] {
            files.insert(
                format!("/w/test/mini/2/{name}"),
                std::fs::read(src.path().join(name)).unwrap(),
            );
        }
        let (base, log) = serve(files);
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        // A previous, interrupted download left the first 25 bytes.
        let staging = cache.partial_dir("test/mini", 2);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::copy(src.path().join(MANIFEST_FILE), staging.join(MANIFEST_FILE)).unwrap();
        std::fs::write(staging.join("mini.ocwpkg.part"), &package_bytes()[..25]).unwrap();
        let url = format!("{base}/w/test/mini/2/{MANIFEST_FILE}");
        let mut calls = 0;
        let f = fetch(
            &url,
            &cache,
            &trust(),
            UnsignedPolicy::Refuse,
            NOW,
            &UreqTransport::default(),
            &mut |_, _, _| calls += 1,
        )
        .unwrap();
        assert!(f.trust.is_trusted());
        assert_eq!(f.downloaded, package_bytes().len() as u64 - 25);
        assert!(calls > 0);
        assert_eq!(
            *log.lock().unwrap(),
            ["/w/test/mini/2/mini.ocwpkg range: bytes=25-"]
        );
        // A missing manifest is a clean HTTP error.
        let e = fetch(
            &format!("{base}/w/nothing/manifest.json"),
            &cache,
            &trust(),
            UnsignedPolicy::Refuse,
            NOW,
            &UreqTransport::default(),
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert!(matches!(e, FetchError::Status(_, 404)), "{e}");
    }

    #[test]
    fn install_from_a_local_directory() {
        let src = tempfile::tempdir().unwrap();
        write_package(src.path(), true, false, 2);
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        let url = src.path().join(MANIFEST_FILE).display().to_string();
        let t = transport_for(&url);
        let f = fetch(
            &url,
            &cache,
            &trust(),
            UnsignedPolicy::Refuse,
            NOW,
            &*t,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!((f.entry.id.as_str(), f.entry.version), ("test/mini", 2));
        let file_url = format!("file://{url}");
        let f = fetch(
            &file_url,
            &cache,
            &trust(),
            UnsignedPolicy::Refuse,
            NOW,
            &FileTransport,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(f.downloaded, 0);
    }
}
