//! Package manifests and signatures (spec §4.7, §12).
//!
//! A world package travels with a **manifest**: who built it (id, region
//! name, version, creation time, generator, format), where its data comes
//! from (attributions: © OpenStreetMap contributors / ODbL, the DEM), and
//! the size and SHA-256 of each file and of each tile-sized chunk of it
//! (header, every tile, every section), so a client can fetch single tiles
//! with HTTP range requests and still verify them.
//!
//! **Key hierarchy:** an offline **root key** signs **signing keys** with
//! a validity period ([`SigningKeyCert`]); a signing key signs manifests.
//! The client ships only the root public keys ([`TrustStore`]). A manifest
//! is accepted when
//!
//! 1. its signing key certificate is signed by a trusted root,
//! 2. neither the signing key nor the root is revoked,
//! 3. the manifest signature matches the signing key,
//! 4. the manifest was created within the signing key's validity period
//!    (so a cached package stays valid offline after the key expires, and
//!    an expired key cannot sign new packages), and not in the future
//!    (beyond [`MAX_CLOCK_SKEW_S`]),
//! 5. the files match their size and SHA-256.
//!
//! **File format:** JSON. The signed part is kept as raw bytes
//! (`"signed": {...}`) and signatures cover exactly those bytes, prefixed
//! with a context string per kind ([`MANIFEST_CONTEXT`],
//! [`SIGNING_KEY_CONTEXT`]), so no canonical JSON is needed and fields
//! added later never break old signatures.
//!
//! ```text
//! {
//!   "signed": { "format": 1, "id": "europe/liechtenstein", "version": 3, ... },
//!   "signature": {
//!     "key": { "signed": { "kind": "ocw-signing-key", "public_key": ..., "root_key_id": ...,
//!                          "valid_from_unix_s": ..., "valid_until_unix_s": ... },
//!              "signature": "<root signature, hex>" },
//!     "signature": "<signing key signature, hex>"
//!   }
//! }
//! ```
//!
//! Everything here is pure (no clock, files or randomness: callers pass
//! the time and the key seeds), so it builds for `wasm32-unknown-unknown`.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

/// Layout version of the manifest (`"format"`).
pub const MANIFEST_FORMAT: u32 = 1;
/// Prefix of the bytes a manifest signature covers.
pub const MANIFEST_CONTEXT: &[u8] = b"OCW world package manifest v1\0";
/// Prefix of the bytes a signing key certificate signature covers.
pub const SIGNING_KEY_CONTEXT: &[u8] = b"OCW signing key v1\0";
/// How far a manifest's creation time may lie in the future (clock skew).
pub const MAX_CLOCK_SKEW_S: i64 = 86_400;

const KIND_SIGNING_KEY: &str = "ocw-signing-key";
const KIND_ROOT_PUBLIC: &str = "ocw-root-public-key";
const KIND_ROOT_SECRET: &str = "ocw-root-secret-key";
const KIND_SIGNING_SECRET: &str = "ocw-signing-secret-key";

/// Fixed-size bytes, written as lower-case hex in JSON and `Display`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hex<const N: usize>(pub [u8; N]);

/// SHA-256 of a file or chunk.
pub type Sha256Hash = Hex<32>;
/// Short key id: the first 8 bytes of the SHA-256 of the public key.
pub type KeyId = Hex<8>;

impl<const N: usize> Hex<N> {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.len() != 2 * N || !s.is_ascii() {
            return Err(format!("expected {} hex digits", 2 * N));
        }
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|_| format!("not hex: {s}"))?;
        }
        Ok(Self(out))
    }
}

impl<const N: usize> fmt::Display for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl<const N: usize> fmt::Debug for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<const N: usize> Serialize for Hex<N> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de, const N: usize> Deserialize<'de> for Hex<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// SHA-256 of `bytes` (the content hash of a package file).
pub fn sha256(bytes: &[u8]) -> Sha256Hash {
    Hex(Sha256::digest(bytes).into())
}

/// Key id of an Ed25519 public key.
pub fn key_id(public_key: &[u8; 32]) -> KeyId {
    let h = sha256(public_key);
    let mut id = [0; 8];
    id.copy_from_slice(&h.0[..8]);
    Hex(id)
}

/// A data source and the notice it requires (spec §13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// What the data is, e.g. "OpenStreetMap".
    pub source: String,
    /// SPDX-like licence name, e.g. "ODbL-1.0".
    pub licence: String,
    /// Text to show on the credits screen.
    pub notice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Attribution {
    /// OpenStreetMap: the road graph is a Derivative Database under ODbL,
    /// the rendered world a Produced Work that needs the notice (spec §13).
    pub fn openstreetmap() -> Self {
        Self {
            source: "OpenStreetMap".into(),
            licence: "ODbL-1.0".into(),
            notice: "© OpenStreetMap contributors. The road graph in this package (section GRPH) \
                     is a Derivative Database under the Open Database License 1.0 and is offered \
                     for download alongside the package."
                .into(),
            url: Some("https://www.openstreetmap.org/copyright".into()),
        }
    }

    /// SRTM elevation data (NASA / USGS, public domain).
    pub fn srtm() -> Self {
        Self {
            source: "SRTM elevation data".into(),
            licence: "public-domain".into(),
            notice: "Elevation: SRTM, courtesy of NASA / USGS".into(),
            url: Some("https://www.usgs.gov/centers/eros".into()),
        }
    }
}

/// A byte range of a file with its own hash (one tile, the header, one
/// section), for range requests (spec §4.7: streaming in tile chunks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// `header`, `tile X,Z`, `sections` (the section count) or `section TAG`.
    pub name: String,
    pub offset: u64,
    pub size: u64,
    pub sha256: Sha256Hash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// File name next to the manifest (no directories).
    pub path: String,
    pub size: u64,
    pub sha256: Sha256Hash,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<Chunk>,
}

/// What a manifest says about a package (the signed part).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`MANIFEST_FORMAT`].
    pub format: u32,
    /// Package id: lower-case path segments, e.g. `europe/liechtenstein`
    /// ([`validate_id`]); with `version` the key of the cache.
    pub id: String,
    /// Region name for people.
    pub name: String,
    /// Build number; a rebuild of the same id gets a higher one.
    pub version: u64,
    /// Seconds since the Unix epoch.
    pub created_unix_s: i64,
    /// Tool and version that built the package, e.g. `world-builder 0.1.0`.
    pub generator: String,
    /// Binary format of the package file (its magic, e.g. `OCWPKG02`).
    pub package_format: String,
    pub attribution: Vec<Attribution>,
    pub files: Vec<FileEntry>,
}

/// Id, version and content hash of a package: what client and server
/// compare to know they ride the same world (and its stable edge IDs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageIdentity {
    pub id: String,
    pub version: u64,
    /// SHA-256 of the package file.
    pub content_hash: Sha256Hash,
}

impl PackageIdentity {
    /// A package without a manifest (a local build): id `local/<name>`,
    /// version 0; the content hash still tells builds apart.
    pub fn local(name: &str, bytes: &[u8]) -> Self {
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let slug = slug.trim_matches('-');
        Self {
            id: format!("local/{}", if slug.is_empty() { "world" } else { slug }),
            version: 0,
            content_hash: sha256(bytes),
        }
    }
}

impl fmt::Display for PackageIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{} ({})",
            self.id,
            self.version,
            &self.content_hash.to_string()[..16]
        )
    }
}

/// Package ids: 1–8 segments of `[a-z0-9_-]` separated by `/`, at most
/// 128 bytes (they become cache directories, so no `.`, `..` or `\`).
pub fn validate_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id.split('/').count() <= 8
        && id.split('/').all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        });
    if ok {
        Ok(())
    } else {
        Err(format!(
            "bad package id {id:?}: use lower-case segments of a-z, 0-9, - and _ separated by / (e.g. europe/liechtenstein)"
        ))
    }
}

/// File names in a manifest: one plain name (no directories, not hidden).
fn validate_file_name(path: &str) -> Result<(), String> {
    let ok = !path.is_empty()
        && path.len() <= 255
        && !path.starts_with('.')
        && !path.contains(['/', '\\', ':', '\0']);
    if ok {
        Ok(())
    } else {
        Err(format!("bad file name in manifest: {path:?}"))
    }
}

impl Manifest {
    /// A manifest without files yet; add them with [`Manifest::add_file`].
    pub fn new(
        id: &str,
        name: &str,
        version: u64,
        created_unix_s: i64,
        generator: &str,
        attribution: Vec<Attribution>,
    ) -> Result<Self, String> {
        validate_id(id)?;
        Ok(Self {
            format: MANIFEST_FORMAT,
            id: id.into(),
            name: name.into(),
            version,
            created_unix_s,
            generator: generator.into(),
            package_format: String::new(),
            attribution,
            files: Vec::new(),
        })
    }

    /// Adds (or replaces) a file. A world package file (`OCWPKG..`) gets
    /// its chunks listed and sets [`Manifest::package_format`].
    pub fn add_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        validate_file_name(path)?;
        let mut chunks = Vec::new();
        if bytes.starts_with(b"OCWPKG") {
            for (name, range) in package_chunks(bytes)? {
                chunks.push(Chunk {
                    name,
                    offset: range.start as u64,
                    size: (range.end - range.start) as u64,
                    sha256: sha256(&bytes[range]),
                });
            }
            if self.package_format.is_empty() {
                self.package_format = String::from_utf8_lossy(&bytes[..8]).into_owned();
            }
        }
        let entry = FileEntry {
            path: path.into(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
            chunks,
        };
        match self.files.iter_mut().find(|f| f.path == path) {
            Some(f) => *f = entry,
            None => self.files.push(entry),
        }
        Ok(())
    }

    pub fn package_id(&self) -> &str {
        &self.id
    }

    /// The world package file (the first `OCWPKG` file, else the first).
    pub fn package_file(&self) -> Option<&FileEntry> {
        self.files
            .iter()
            .find(|f| !f.chunks.is_empty())
            .or(self.files.first())
    }

    /// SHA-256 of the package file.
    pub fn content_hash(&self) -> Option<Sha256Hash> {
        self.package_file().map(|f| f.sha256)
    }

    pub fn identity(&self) -> Option<PackageIdentity> {
        Some(PackageIdentity {
            id: self.id.clone(),
            version: self.version,
            content_hash: self.content_hash()?,
        })
    }

    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Checks a file's size and hash.
    pub fn verify_file(&self, path: &str, bytes: &[u8]) -> Result<(), VerifyError> {
        let f = self
            .file(path)
            .ok_or_else(|| VerifyError::MissingFile(path.into()))?;
        if f.size != bytes.len() as u64 {
            return Err(VerifyError::SizeMismatch {
                path: path.into(),
                expected: f.size,
                actual: bytes.len() as u64,
            });
        }
        if sha256(bytes) != f.sha256 {
            return Err(VerifyError::HashMismatch(path.into()));
        }
        Ok(())
    }

    /// Checks one chunk fetched on its own (a range request).
    pub fn verify_chunk(&self, path: &str, chunk: &str, bytes: &[u8]) -> Result<(), VerifyError> {
        let f = self
            .file(path)
            .ok_or_else(|| VerifyError::MissingFile(path.into()))?;
        let c = f
            .chunks
            .iter()
            .find(|c| c.name == chunk)
            .ok_or_else(|| VerifyError::MissingFile(format!("{path}: {chunk}")))?;
        if c.size != bytes.len() as u64 || sha256(bytes) != c.sha256 {
            return Err(VerifyError::HashMismatch(format!("{path}: {chunk}")));
        }
        Ok(())
    }

    fn check(&self) -> Result<(), VerifyError> {
        if self.format != MANIFEST_FORMAT {
            return Err(VerifyError::Malformed(format!(
                "manifest format {} is not supported (this build reads {MANIFEST_FORMAT}); please update",
                self.format
            )));
        }
        validate_id(&self.id).map_err(VerifyError::Malformed)?;
        for f in &self.files {
            validate_file_name(&f.path).map_err(VerifyError::Malformed)?;
        }
        if self.files.is_empty() {
            return Err(VerifyError::Malformed("manifest lists no files".into()));
        }
        Ok(())
    }
}

/// Splits a world package file into its chunks: `header` (up to the tile
/// count), `tile X,Z` per tile, then for version 2 `sections` (the section
/// count) and `section TAG` per section. Together they cover the file.
pub fn package_chunks(bytes: &[u8]) -> Result<Vec<(String, std::ops::Range<usize>)>, String> {
    let mut r = Cursor { b: bytes, pos: 0 };
    let version = match r.take(8)? {
        b"OCWPKG01" => 1,
        b"OCWPKG02" => 2,
        _ => return Err("not an OCW world package (OCWPKG01/02)".into()),
    };
    let name_len = r.u32()?;
    r.skip(name_len)?;
    r.skip(16 + 4)?; // origin, tile size
    let samples = r.u32()?;
    let tiles = r.u32()?;
    let mut out = vec![("header".to_string(), 0..r.pos)];
    let heights = samples
        .checked_mul(samples)
        .and_then(|n| n.checked_mul(4))
        .ok_or("tile grid too large")?;
    for _ in 0..tiles {
        let start = r.pos;
        let x = r.u32()? as i32;
        let z = r.u32()? as i32;
        r.skip(heights)?;
        for _ in 0..r.u32()? {
            r.skip(1 + 4)?; // class, width
            let pts = r.u32()?;
            r.skip(pts.checked_mul(12).ok_or("road too long")?)?;
        }
        out.push((format!("tile {x},{z}"), start..r.pos));
    }
    if version >= 2 {
        let start = r.pos;
        let sections = r.u32()?;
        out.push(("sections".into(), start..r.pos));
        for _ in 0..sections {
            let start = r.pos;
            r.skip(4)?;
            let tag = String::from_utf8_lossy(&bytes[start..start + 4]).into_owned();
            let len = u64::from_le_bytes(r.take(8)?.try_into().expect("8 bytes"));
            r.skip(usize::try_from(len).map_err(|_| "section too large")?)?;
            let mut name = format!("section {tag}");
            let mut n = 2;
            while out.iter().any(|(c, _)| *c == name) {
                name = format!("section {tag}#{n}");
                n += 1;
            }
            out.push((name, start..r.pos));
        }
    }
    if r.pos != bytes.len() {
        return Err("trailing bytes after the last tile or section".into());
    }
    Ok(out)
}

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("package truncated")?;
        let s = self.b.get(self.pos..end).ok_or("package truncated")?;
        self.pos = end;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")) as usize)
    }
}

/// Why a manifest or package was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("malformed manifest: {0}")]
    Malformed(String),
    #[error("the package is not signed")]
    Unsigned,
    #[error("signed by an unknown root key {0}")]
    UnknownRoot(KeyId),
    #[error("key {0} is revoked")]
    Revoked(KeyId),
    #[error(
        "signing key {key} was not valid when the package was created ({created_unix_s}; valid {from_unix_s}..{until_unix_s})"
    )]
    KeyNotValid {
        key: KeyId,
        created_unix_s: i64,
        from_unix_s: i64,
        until_unix_s: i64,
    },
    #[error("the package claims to be created in the future ({created_unix_s} > now {now_unix_s})")]
    FromTheFuture {
        created_unix_s: i64,
        now_unix_s: i64,
    },
    #[error("invalid signature on the {0}")]
    BadSignature(&'static str),
    #[error("{0}: listed in the manifest but missing")]
    MissingFile(String),
    #[error("{path}: {actual} bytes, the manifest says {expected}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("{0}: SHA-256 does not match the manifest")]
    HashMismatch(String),
}

impl VerifyError {
    /// The data was changed or damaged (or a key revoked): always refused,
    /// whatever the unsigned-package policy says.
    pub fn is_integrity_failure(&self) -> bool {
        !self.is_trust_failure()
    }

    /// Intact, but not vouched for by a trusted key (unsigned, unknown
    /// root, outside the key's validity): refused or warned about by policy.
    pub fn is_trust_failure(&self) -> bool {
        matches!(
            self,
            Self::Unsigned
                | Self::UnknownRoot(_)
                | Self::KeyNotValid { .. }
                | Self::FromTheFuture { .. }
        )
    }
}

/// A root public key the client trusts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootKey {
    /// `ocw-root-public-key`.
    pub kind: String,
    pub name: String,
    pub key_id: KeyId,
    pub public_key: Hex<32>,
    /// Development root: trusted only by development builds (spec §12).
    #[serde(default)]
    pub dev_only: bool,
}

impl RootKey {
    /// Parses a root public key file and checks its key id.
    pub fn parse(json: &str) -> Result<Self, String> {
        let k: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if k.kind != KIND_ROOT_PUBLIC {
            return Err(format!("not a root public key ({:?})", k.kind));
        }
        if key_id(&k.public_key.0) != k.key_id {
            return Err("key id does not match the public key".into());
        }
        VerifyingKey::from_bytes(&k.public_key.0).map_err(|e| e.to_string())?;
        Ok(k)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serializable") + "\n"
    }
}

/// The root keys a client trusts, and revoked keys (roots or signing keys).
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    pub roots: Vec<RootKey>,
    pub revoked: Vec<KeyId>,
}

impl TrustStore {
    pub fn new(roots: impl IntoIterator<Item = RootKey>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
            revoked: Vec::new(),
        }
    }

    pub fn root(&self, id: KeyId) -> Option<&RootKey> {
        self.roots.iter().find(|r| r.key_id == id)
    }
}

/// What a signing key certificate states (signed by a root key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningKeyClaims {
    /// `ocw-signing-key`.
    pub kind: String,
    pub name: String,
    pub key_id: KeyId,
    pub public_key: Hex<32>,
    pub root_key_id: KeyId,
    pub valid_from_unix_s: i64,
    pub valid_until_unix_s: i64,
}

/// A signing key's public part and validity, signed by a root key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningKeyCert {
    signed: Box<RawValue>,
    signature: Hex<64>,
}

impl SigningKeyCert {
    pub fn claims(&self) -> Result<SigningKeyClaims, VerifyError> {
        let c: SigningKeyClaims = serde_json::from_str(self.signed.get())
            .map_err(|e| VerifyError::Malformed(format!("signing key: {e}")))?;
        if c.kind != KIND_SIGNING_KEY || key_id(&c.public_key.0) != c.key_id {
            return Err(VerifyError::Malformed("signing key certificate".into()));
        }
        Ok(c)
    }

    /// Checks the root's signature; returns the claims and the root.
    fn verify<'a>(
        &self,
        trust: &'a TrustStore,
    ) -> Result<(SigningKeyClaims, &'a RootKey), VerifyError> {
        let c = self.claims()?;
        let root = trust
            .root(c.root_key_id)
            .ok_or(VerifyError::UnknownRoot(c.root_key_id))?;
        verify(
            &root.public_key,
            SIGNING_KEY_CONTEXT,
            self.signed.get(),
            &self.signature,
        )
        .map_err(|_| VerifyError::BadSignature("signing key certificate"))?;
        Ok((c, root))
    }
}

fn verify(public: &Hex<32>, context: &[u8], raw: &str, sig: &Hex<64>) -> Result<(), ()> {
    let key = VerifyingKey::from_bytes(&public.0).map_err(|_| ())?;
    let msg = [context, raw.as_bytes()].concat();
    key.verify_strict(&msg, &Signature::from_bytes(&sig.0))
        .map_err(|_| ())
}

fn sign(key: &SigningKey, context: &[u8], raw: &str) -> Hex<64> {
    Hex(key.sign(&[context, raw.as_bytes()].concat()).to_bytes())
}

fn raw_json(value: &impl Serialize) -> Box<RawValue> {
    RawValue::from_string(serde_json::to_string_pretty(value).expect("serializable"))
        .expect("valid JSON")
}

#[derive(Serialize, Deserialize)]
struct RootSecretFile {
    kind: String,
    name: String,
    key_id: KeyId,
    public_key: Hex<32>,
    secret_key: Hex<32>,
}

/// The offline root key pair.
pub struct RootKeyPair {
    pub name: String,
    key: SigningKey,
}

impl RootKeyPair {
    /// From 32 random bytes (the caller provides the randomness).
    pub fn from_seed(name: &str, seed: [u8; 32]) -> Self {
        Self {
            name: name.into(),
            key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public(&self) -> RootKey {
        let public_key = Hex(self.key.verifying_key().to_bytes());
        RootKey {
            kind: KIND_ROOT_PUBLIC.into(),
            name: self.name.clone(),
            key_id: key_id(&public_key.0),
            public_key,
            dev_only: false,
        }
    }

    /// Signs a signing key's public key for the given validity period.
    pub fn certify(
        &self,
        name: &str,
        public_key: [u8; 32],
        valid_from_unix_s: i64,
        valid_until_unix_s: i64,
    ) -> SigningKeyCert {
        let claims = SigningKeyClaims {
            kind: KIND_SIGNING_KEY.into(),
            name: name.into(),
            key_id: key_id(&public_key),
            public_key: Hex(public_key),
            root_key_id: self.public().key_id,
            valid_from_unix_s,
            valid_until_unix_s,
        };
        let signed = raw_json(&claims);
        let signature = sign(&self.key, SIGNING_KEY_CONTEXT, signed.get());
        SigningKeyCert { signed, signature }
    }

    /// The secret key file (keep it offline, readable only by its owner).
    pub fn to_json(&self) -> String {
        let p = self.public();
        let f = RootSecretFile {
            kind: KIND_ROOT_SECRET.into(),
            name: self.name.clone(),
            key_id: p.key_id,
            public_key: p.public_key,
            secret_key: Hex(self.key.to_bytes()),
        };
        serde_json::to_string_pretty(&f).expect("serializable") + "\n"
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let f: RootSecretFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if f.kind != KIND_ROOT_SECRET {
            return Err(format!("not a root secret key ({:?})", f.kind));
        }
        let k = Self::from_seed(&f.name, f.secret_key.0);
        if k.public().public_key != f.public_key {
            return Err("secret and public key do not match".into());
        }
        Ok(k)
    }
}

#[derive(Serialize, Deserialize)]
struct SigningSecretFile {
    kind: String,
    secret_key: Hex<32>,
    certificate: SigningKeyCert,
}

/// An online signing key with its root-signed certificate.
pub struct SigningKeyPair {
    key: SigningKey,
    pub cert: SigningKeyCert,
}

impl SigningKeyPair {
    /// The public key of the signing key from `seed` (to certify it).
    pub fn public_key_of(seed: [u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(&seed).verifying_key().to_bytes()
    }

    pub fn new(seed: [u8; 32], cert: SigningKeyCert) -> Result<Self, String> {
        let key = SigningKey::from_bytes(&seed);
        let claims = cert.claims().map_err(|e| e.to_string())?;
        if claims.public_key.0 != key.verifying_key().to_bytes() {
            return Err("the certificate is for another key".into());
        }
        Ok(Self { key, cert })
    }

    pub fn claims(&self) -> SigningKeyClaims {
        self.cert.claims().expect("checked in new")
    }

    pub fn to_json(&self) -> String {
        let f = SigningSecretFile {
            kind: KIND_SIGNING_SECRET.into(),
            secret_key: Hex(self.key.to_bytes()),
            certificate: self.cert.clone(),
        };
        serde_json::to_string_pretty(&f).expect("serializable") + "\n"
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let f: SigningSecretFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if f.kind != KIND_SIGNING_SECRET {
            return Err(format!("not a signing secret key ({:?})", f.kind));
        }
        Self::new(f.secret_key.0, f.certificate)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestSignature {
    key: SigningKeyCert,
    signature: Hex<64>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    signed: Box<RawValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<ManifestSignature>,
}

/// Who vouched for a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub root_name: String,
    pub root_key_id: KeyId,
    pub root_dev_only: bool,
    pub signing_key: SigningKeyClaims,
}

/// A manifest as stored in its file: the signed bytes, their parsed
/// content and the signature (if any).
#[derive(Debug, Clone)]
pub struct SignedManifest {
    raw: Box<RawValue>,
    pub manifest: Manifest,
    signature: Option<ManifestSignature>,
}

impl SignedManifest {
    pub fn unsigned(manifest: &Manifest) -> Self {
        Self {
            raw: raw_json(manifest),
            manifest: manifest.clone(),
            signature: None,
        }
    }

    pub fn signed(manifest: &Manifest, key: &SigningKeyPair) -> Self {
        let raw = raw_json(manifest);
        let signature = Some(ManifestSignature {
            key: key.cert.clone(),
            signature: sign(&key.key, MANIFEST_CONTEXT, raw.get()),
        });
        Self {
            raw,
            manifest: manifest.clone(),
            signature,
        }
    }

    pub fn is_signed(&self) -> bool {
        self.signature.is_some()
    }

    /// The manifest file.
    pub fn to_json(&self) -> String {
        let e = Envelope {
            signed: self.raw.clone(),
            signature: self.signature.clone(),
        };
        serde_json::to_string_pretty(&e).expect("serializable") + "\n"
    }

    pub fn parse(json: &str) -> Result<Self, VerifyError> {
        let e: Envelope =
            serde_json::from_str(json).map_err(|e| VerifyError::Malformed(e.to_string()))?;
        let manifest: Manifest = serde_json::from_str(e.signed.get())
            .map_err(|e| VerifyError::Malformed(e.to_string()))?;
        manifest.check()?;
        Ok(Self {
            raw: e.signed,
            manifest,
            signature: e.signature,
        })
    }

    /// The signing key certificate's claims, without checking anything.
    pub fn signing_key(&self) -> Option<SigningKeyClaims> {
        self.signature.as_ref().and_then(|s| s.key.claims().ok())
    }

    /// Checks the signature chain (module docs, steps 1–4) at `now_unix_s`.
    pub fn verify_signature(
        &self,
        trust: &TrustStore,
        now_unix_s: i64,
    ) -> Result<Signer, VerifyError> {
        let sig = self.signature.as_ref().ok_or(VerifyError::Unsigned)?;
        let (claims, root) = sig.key.verify(trust)?;
        for id in [root.key_id, claims.key_id] {
            if trust.revoked.contains(&id) {
                return Err(VerifyError::Revoked(id));
            }
        }
        verify(
            &claims.public_key,
            MANIFEST_CONTEXT,
            self.raw.get(),
            &sig.signature,
        )
        .map_err(|_| VerifyError::BadSignature("manifest"))?;
        let created = self.manifest.created_unix_s;
        if created < claims.valid_from_unix_s || created > claims.valid_until_unix_s {
            return Err(VerifyError::KeyNotValid {
                key: claims.key_id,
                created_unix_s: created,
                from_unix_s: claims.valid_from_unix_s,
                until_unix_s: claims.valid_until_unix_s,
            });
        }
        if created > now_unix_s.saturating_add(MAX_CLOCK_SKEW_S) {
            return Err(VerifyError::FromTheFuture {
                created_unix_s: created,
                now_unix_s,
            });
        }
        Ok(Signer {
            root_name: root.name.clone(),
            root_key_id: root.key_id,
            root_dev_only: root.dev_only,
            signing_key: claims,
        })
    }
}

/// What to do with packages that are intact but not signed by a trusted
/// key (spec §4.7, §12). Integrity failures are always refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsignedPolicy {
    /// Refuse them (downloads, the cache, `--require-signed`).
    Refuse,
    /// Load them with a warning (local builds, `--allow-unsigned`).
    Warn,
}

/// Outcome of the trust check under a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStatus {
    /// Signed by a trusted root.
    Trusted(Signer),
    /// Accepted by policy despite the reason (unsigned, unknown root, ...).
    Untrusted(String),
}

impl TrustStatus {
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Trusted(_))
    }
}

impl fmt::Display for TrustStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trusted(s) => write!(
                f,
                "signed by {} ({}), root {}{} ({})",
                s.signing_key.name,
                s.signing_key.key_id,
                s.root_name,
                if s.root_dev_only {
                    " [development]"
                } else {
                    ""
                },
                s.root_key_id
            ),
            Self::Untrusted(why) => write!(f, "NOT TRUSTED: {why}"),
        }
    }
}

/// Applies `policy` to a signature check.
pub fn apply_policy(
    result: Result<Signer, VerifyError>,
    policy: UnsignedPolicy,
) -> Result<TrustStatus, VerifyError> {
    match result {
        Ok(s) => Ok(TrustStatus::Trusted(s)),
        Err(e) if e.is_trust_failure() && policy == UnsignedPolicy::Warn => {
            Ok(TrustStatus::Untrusted(e.to_string()))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{RoadStrip, Tile, TileKey, WorldPackage};

    const DAY: i64 = 86_400;
    const T0: i64 = 1_800_000_000;

    fn package_bytes(graph: bool) -> Vec<u8> {
        let mut tiles = std::collections::BTreeMap::new();
        for (x, z) in [(0, 0), (-1, 3)] {
            let key = TileKey { x, z };
            tiles.insert(
                key,
                Tile {
                    key,
                    heights: vec![1.0; 9],
                    roads: vec![RoadStrip {
                        class: crate::graph::RoadClass::Residential,
                        width_m: 4.0,
                        points: vec![[0.0; 3], [1.0, 2.0, 3.0]],
                    }],
                },
            );
        }
        WorldPackage {
            name: "Test".into(),
            origin: (47.0, 9.5),
            tile_size_m: 1000.0,
            samples: 3,
            tiles,
            graph: graph.then(crate::graph::RoadGraph::default),
            routes: Vec::new(),
            landuse: None,
            buildings: None,
        }
        .to_bytes()
    }

    fn keys() -> (RootKeyPair, SigningKeyPair) {
        let root = RootKeyPair::from_seed("Test root", [7; 32]);
        let seed = [9; 32];
        let cert = root.certify(
            "test signing",
            SigningKeyPair::public_key_of(seed),
            T0 - DAY,
            T0 + 30 * DAY,
        );
        (root, SigningKeyPair::new(seed, cert).unwrap())
    }

    fn manifest(bytes: &[u8]) -> Manifest {
        let mut m = Manifest::new(
            "test/region",
            "Test Region",
            3,
            T0,
            "test 1.0",
            vec![Attribution::openstreetmap(), Attribution::srtm()],
        )
        .unwrap();
        m.add_file("region.ocwpkg", bytes).unwrap();
        m
    }

    #[test]
    fn land_use_and_buildings_sections_are_chunks() {
        let mut p = WorldPackage::from_bytes(&package_bytes(true)).unwrap();
        p.landuse = Some(crate::landuse::LandUse {
            cells: 2,
            tiles: [(
                crate::package::TileKey { x: 0, z: 0 },
                crate::landuse::LandUseTile {
                    classes: vec![crate::landuse::LandClass::Forest; 4],
                    water: vec![],
                },
            )]
            .into_iter()
            .collect(),
        });
        p.buildings = Some(Default::default());
        let bytes = p.to_bytes();
        let chunks = package_chunks(&bytes).unwrap();
        let names: Vec<&str> = chunks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names[3..],
            ["sections", "section GRPH", "section LUSE", "section BLDG"]
        );
        let m = manifest(&bytes);
        for (name, r) in &chunks {
            m.verify_chunk("region.ocwpkg", name, &bytes[r.clone()])
                .unwrap();
        }
    }

    #[test]
    fn chunks_cover_the_package_and_verify_alone() {
        for graph in [false, true] {
            let bytes = package_bytes(graph);
            let chunks = package_chunks(&bytes).unwrap();
            let names: Vec<&str> = chunks.iter().map(|(n, _)| n.as_str()).collect();
            if graph {
                assert_eq!(
                    names,
                    [
                        "header",
                        "tile -1,3",
                        "tile 0,0",
                        "sections",
                        "section GRPH"
                    ]
                );
            } else {
                assert_eq!(names, ["header", "tile -1,3", "tile 0,0"]);
            }
            // Contiguous and complete.
            let mut at = 0;
            for (_, r) in &chunks {
                assert_eq!(r.start, at);
                at = r.end;
            }
            assert_eq!(at, bytes.len());
            let m = manifest(&bytes);
            assert_eq!(
                m.package_format,
                if graph { "OCWPKG02" } else { "OCWPKG01" }
            );
            for (name, r) in &chunks {
                m.verify_chunk("region.ocwpkg", name, &bytes[r.clone()])
                    .unwrap();
            }
            let (name, r) = &chunks[1];
            let mut tile = bytes[r.clone()].to_vec();
            tile[9] ^= 1;
            assert!(m.verify_chunk("region.ocwpkg", name, &tile).is_err());
        }
        assert!(package_chunks(b"OCWPKG02\x05\0\0\0ab").is_err());
        let mut long = package_bytes(true);
        long.push(0);
        assert!(package_chunks(&long).is_err());
    }

    #[test]
    fn signed_manifest_round_trip_and_verify() {
        let (root, signing) = keys();
        let bytes = package_bytes(true);
        let m = manifest(&bytes);
        let file = SignedManifest::signed(&m, &signing).to_json();
        let parsed = SignedManifest::parse(&file).unwrap();
        assert_eq!(parsed.manifest, m);
        let trust = TrustStore::new([root.public()]);
        let signer = parsed.verify_signature(&trust, T0 + 100 * DAY).unwrap();
        assert_eq!(signer.root_name, "Test root");
        assert_eq!(signer.signing_key.name, "test signing");
        parsed
            .manifest
            .verify_file("region.ocwpkg", &bytes)
            .unwrap();
        // Identity for the protocol (client and server ride the same world).
        let id = m.identity().unwrap();
        assert_eq!((id.id.as_str(), id.version), ("test/region", 3));
        assert_eq!(id.content_hash, sha256(&bytes));
        assert_eq!(m.content_hash(), Some(sha256(&bytes)));
        assert_eq!(m.package_id(), "test/region");
        // Attribution survives, incl. the ODbL notice.
        assert!(
            parsed.manifest.attribution[0]
                .notice
                .contains("© OpenStreetMap contributors")
        );
    }

    #[test]
    fn tampering_is_detected() {
        let (root, signing) = keys();
        let trust = TrustStore::new([root.public()]);
        let bytes = package_bytes(true);
        let file = SignedManifest::signed(&manifest(&bytes), &signing).to_json();
        // A changed manifest field (here the version) breaks the signature.
        let edited = file.replacen("\"version\": 3", "\"version\": 4", 1);
        assert_ne!(edited, file);
        assert_eq!(
            SignedManifest::parse(&edited)
                .unwrap()
                .verify_signature(&trust, T0),
            Err(VerifyError::BadSignature("manifest"))
        );
        // A changed validity period breaks the certificate.
        let until = (T0 + 30 * DAY).to_string();
        let edited = file.replacen(&until, &(T0 + 3000 * DAY).to_string(), 1);
        assert_eq!(
            SignedManifest::parse(&edited)
                .unwrap()
                .verify_signature(&trust, T0),
            Err(VerifyError::BadSignature("signing key certificate"))
        );
        // A changed package byte or length.
        let m = SignedManifest::parse(&file).unwrap().manifest;
        let mut b = bytes.clone();
        b[20] ^= 0x40;
        let e = m.verify_file("region.ocwpkg", &b).unwrap_err();
        assert!(matches!(e, VerifyError::HashMismatch(_)) && e.is_integrity_failure());
        assert!(matches!(
            m.verify_file("region.ocwpkg", &bytes[1..]),
            Err(VerifyError::SizeMismatch { .. })
        ));
        assert!(matches!(
            m.verify_file("other.ocwpkg", &bytes),
            Err(VerifyError::MissingFile(_))
        ));
        // A self-made key chain (attacker's root) is not trusted.
        let evil = RootKeyPair::from_seed("Evil", [1; 32]);
        let cert = evil.certify("evil", SigningKeyPair::public_key_of([2; 32]), 0, i64::MAX);
        let evil_signed = SignedManifest::signed(
            &manifest(&bytes),
            &SigningKeyPair::new([2; 32], cert).unwrap(),
        );
        let e = evil_signed.verify_signature(&trust, T0).unwrap_err();
        assert_eq!(e, VerifyError::UnknownRoot(evil.public().key_id));
        assert!(e.is_trust_failure());
        // A certificate swapped from another root, re-pointed at ours.
        let swapped = evil_signed.to_json().replace(
            &evil.public().key_id.to_string(),
            &root.public().key_id.to_string(),
        );
        assert_eq!(
            SignedManifest::parse(&swapped)
                .unwrap()
                .verify_signature(&trust, T0),
            Err(VerifyError::BadSignature("signing key certificate"))
        );
    }

    #[test]
    fn validity_revocation_and_policy() {
        let (root, signing) = keys();
        let bytes = package_bytes(false);
        let mut trust = TrustStore::new([root.public()]);
        let mut m = manifest(&bytes);
        // Created after the signing key expired.
        m.created_unix_s = T0 + 31 * DAY;
        let late = SignedManifest::signed(&m, &signing);
        let e = late.verify_signature(&trust, T0 + 31 * DAY).unwrap_err();
        assert!(matches!(e, VerifyError::KeyNotValid { .. }) && e.is_trust_failure());
        // Created within the period: still fine long after expiry (offline cache).
        m.created_unix_s = T0;
        let ok = SignedManifest::signed(&m, &signing);
        assert!(ok.verify_signature(&trust, T0 + 1000 * DAY).is_ok());
        // But not if the clock says it is from the future.
        assert!(matches!(
            ok.verify_signature(&trust, T0 - 2 * DAY),
            Err(VerifyError::FromTheFuture { .. })
        ));
        // Revoked signing key or root.
        trust.revoked.push(signing.claims().key_id);
        let e = ok.verify_signature(&trust, T0).unwrap_err();
        assert!(matches!(e, VerifyError::Revoked(_)) && e.is_integrity_failure());
        trust.revoked = vec![root.public().key_id];
        assert!(matches!(
            ok.verify_signature(&trust, T0),
            Err(VerifyError::Revoked(_))
        ));
        trust.revoked.clear();
        // Unsigned: refused or warned about by policy.
        let unsigned = SignedManifest::parse(&SignedManifest::unsigned(&m).to_json()).unwrap();
        assert!(!unsigned.is_signed());
        let r = unsigned.verify_signature(&trust, T0);
        assert_eq!(r, Err(VerifyError::Unsigned));
        assert_eq!(
            apply_policy(r.clone(), UnsignedPolicy::Refuse),
            Err(VerifyError::Unsigned)
        );
        let warned = apply_policy(r, UnsignedPolicy::Warn).unwrap();
        assert!(!warned.is_trusted() && warned.to_string().contains("not signed"));
        // Integrity failures are refused even under Warn.
        assert!(
            apply_policy(
                Err(VerifyError::BadSignature("manifest")),
                UnsignedPolicy::Warn
            )
            .is_err()
        );
        let trusted = apply_policy(ok.verify_signature(&trust, T0), UnsignedPolicy::Refuse);
        assert!(trusted.unwrap().is_trusted());
    }

    #[test]
    fn key_files_round_trip() {
        let (root, signing) = keys();
        let r = RootKeyPair::from_json(&root.to_json()).unwrap();
        assert_eq!(r.public(), root.public());
        assert_eq!(
            RootKey::parse(&root.public().to_json()).unwrap(),
            root.public()
        );
        let s = SigningKeyPair::from_json(&signing.to_json()).unwrap();
        assert_eq!(s.claims(), signing.claims());
        assert_eq!(s.claims().root_key_id, root.public().key_id);
        // Mismatched or wrong-kind files are rejected.
        assert!(RootKeyPair::from_json(&signing.to_json()).is_err());
        assert!(RootKey::parse(&root.to_json()).is_err());
        let bad_id = root
            .public()
            .to_json()
            .replace(&root.public().key_id.to_string(), "0011223344556677");
        assert!(RootKey::parse(&bad_id).is_err());
        assert!(SigningKeyPair::new([3; 32], signing.cert.clone()).is_err());
    }

    #[test]
    fn ids_and_hex() {
        for ok in ["europe/liechtenstein", "test", "a-b_c/0/x"] {
            validate_id(ok).unwrap();
        }
        for bad in ["", "Europe", "a//b", "../x", "a/.b", "a b", "a\\b", "/a"] {
            assert!(validate_id(bad).is_err(), "{bad}");
        }
        assert!(Manifest::new("../evil", "", 1, 0, "", vec![]).is_err());
        let mut m = manifest(&package_bytes(false));
        assert!(m.add_file("../x", b"").is_err());
        assert!(m.add_file(".hidden", b"").is_err());
        // A manifest with a bad file name or newer format is rejected on parse.
        m.files[0].path = "a/b".into();
        assert!(SignedManifest::parse(&SignedManifest::unsigned(&m).to_json()).is_err());
        let mut m = manifest(&package_bytes(false));
        m.format = 2;
        let e = SignedManifest::parse(&SignedManifest::unsigned(&m).to_json()).unwrap_err();
        assert!(e.to_string().contains("please update"));
        let h: Hex<4> = Hex::parse("00ff10Ab").unwrap();
        assert_eq!(h.0, [0, 255, 16, 171]);
        assert_eq!(h.to_string(), "00ff10ab");
        assert!(Hex::<4>::parse("00ff10").is_err());
        assert!(Hex::<2>::parse("zzzz").is_err());
        assert!(Hex::<2>::parse("éé").is_err());
        let local = PackageIdentity::local("Vaduz – Malbun", b"x");
        assert_eq!(local.id, "local/vaduz---malbun");
        validate_id(&local.id).unwrap();
        assert_eq!(PackageIdentity::local("", b"x").id, "local/world");
    }
}
