//! The user's trusted keys (spec §4.7 *User keys*, §12; §14 Phase 16, 16C
//! *Signing and trust*, Q2).
//!
//! A world edited by a user is signed with the user's **own key**: a root
//! of their own (`world-builder keys init-user`) whose signing key embeds
//! the root's public key in every manifest it signs. Anyone can therefore
//! check such a package's signature chain, but nobody trusts the key until
//! they add it here – after comparing its [`Fingerprint`] with its owner.
//!
//! - [`UserKeys`]: the list of trusted user keys – name (as the key
//!   claims it, or as the user typed it), public key, key id, fingerprint,
//!   the date added and a note. Kept per user: on the desktop in
//!   `<data dir>/ocw/trusted-keys.json` ([`default_path`]), in the browser
//!   in `localStorage` ([`STORAGE_KEY`]). A corrupt store is reported and
//!   read as empty; it is never overwritten until the user changes the
//!   list.
//! - **Built-in roots** (production, development, preview, `--trust-root`,
//!   `trusted-roots/`) are never in this list: [`UserKeys::add`] refuses
//!   them and [`with_user_keys`] keeps the built-in entry when both name
//!   the same key, so the list can neither remove nor override them.
//! - [`decide`]: what an app does with a manifest – load it (a built-in
//!   root: official; a trusted user key: *unofficial*), ask the user (an
//!   intact package signed by a user key nobody here trusts: the trust
//!   dialog with a [`TrustRequest`]), leave it to the unsigned-package
//!   policy, or refuse it (a bad signature, a revoked key: never asked
//!   about).
//! - **Load once** ([`accept_once`]): the dialog's choice that loads
//!   without remembering; the key is accepted until the app quits.
//!
//! Pure except for the file helpers (not on wasm32).

use std::sync::Mutex;

use ocw_world::manifest::{
    Attribution, Fingerprint, Hex, KeyId, RootKey, SignedManifest, Signer, TrustStore, VerifyError,
};
use serde::{Deserialize, Serialize};

/// The store's file name in `<data dir>/ocw/` (the desktop).
pub const FILE_NAME: &str = "trusted-keys.json";
/// The browser's `localStorage` key.
pub const STORAGE_KEY: &str = "ocw-trusted-keys";
const KIND: &str = "ocw-trusted-keys";
const FORMAT: u32 = 1;
/// The longest name or note kept (they are shown in the menu).
pub const MAX_TEXT: usize = 80;

/// One trusted user key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKey {
    /// The name the key claims (or the user gave it). Not verified: only
    /// the fingerprint identifies the key.
    pub name: String,
    pub public_key: Hex<32>,
    pub key_id: KeyId,
    /// The fingerprint as shown (16 groups), for people reading the file;
    /// checked against the key when the file is read.
    pub fingerprint: String,
    /// When the user trusted it (Unix seconds).
    pub added_unix_s: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TrustedKey {
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.public_key.0)
    }

    /// The key as a root of the trust store, marked as a user key.
    pub fn root(&self) -> RootKey {
        let mut r = RootKey::from_public(&self.name, self.public_key.0).expect("checked on add");
        r.user = true;
        r
    }
}

#[derive(Serialize, Deserialize)]
struct StoreFile {
    kind: String,
    format: u32,
    keys: Vec<TrustedKey>,
}

/// The trusted user keys, in the order they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserKeys {
    pub keys: Vec<TrustedKey>,
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_TEXT {
        s.to_owned()
    } else {
        s.chars().take(MAX_TEXT).collect()
    }
}

impl UserKeys {
    /// Reads a store's text; an error names what is wrong.
    pub fn parse(text: &str) -> Result<Self, String> {
        let f: StoreFile = serde_json::from_str(text).map_err(|e| format!("not readable: {e}"))?;
        if f.kind != KIND {
            return Err(format!("not a trusted-keys file ({:?})", f.kind));
        }
        if f.format != FORMAT {
            return Err(format!("format {} (this build reads {FORMAT})", f.format));
        }
        let mut out = Self::default();
        for k in f.keys {
            let root = RootKey::from_public(&k.name, k.public_key.0)
                .map_err(|e| format!("{:?}: {e}", k.name))?;
            if root.key_id != k.key_id {
                return Err(format!("{:?}: the key id does not match the key", k.name));
            }
            if root.fingerprint().to_string() != k.fingerprint {
                return Err(format!(
                    "{:?}: the fingerprint does not match the key",
                    k.name
                ));
            }
            if out.find(k.key_id).is_none() {
                out.keys.push(k);
            }
        }
        Ok(out)
    }

    /// The store as loaded from `text` (a file, `localStorage`): nothing
    /// saved yet is empty; a corrupt or unreadable store is empty with a
    /// warning (the app shows it and does not overwrite the store until
    /// the user changes the list).
    pub fn load(text: Result<Option<String>, String>) -> (Self, Option<String>) {
        match text {
            Ok(None) => (Self::default(), None),
            Ok(Some(t)) if t.trim().is_empty() => (Self::default(), None),
            Ok(Some(t)) => match Self::parse(&t) {
                Ok(k) => (k, None),
                Err(e) => (
                    Self::default(),
                    Some(format!(
                        "the trusted keys could not be read ({e}); none are trusted"
                    )),
                ),
            },
            Err(e) => (
                Self::default(),
                Some(format!(
                    "the trusted keys could not be read ({e}); none are trusted"
                )),
            ),
        }
    }

    pub fn to_json(&self) -> String {
        let f = StoreFile {
            kind: KIND.into(),
            format: FORMAT,
            keys: self.keys.clone(),
        };
        serde_json::to_string_pretty(&f).expect("serializable") + "\n"
    }

    pub fn find(&self, id: KeyId) -> Option<&TrustedKey> {
        self.keys.iter().find(|k| k.key_id == id)
    }

    pub fn contains(&self, id: KeyId) -> bool {
        self.find(id).is_some()
    }

    /// Trusts `key` (its claimed name unless `name` is given). Refuses a
    /// built-in root of `builtin` (it is trusted already and stays
    /// official) and a key that is trusted already.
    pub fn add(
        &mut self,
        key: &RootKey,
        name: Option<&str>,
        note: Option<&str>,
        now_unix_s: i64,
        builtin: &TrustStore,
    ) -> Result<TrustedKey, String> {
        key.check()?;
        if builtin.root(key.key_id).is_some() {
            return Err(format!(
                "{} is a built-in key of this app (trusted already; the list cannot change it)",
                key.fingerprint().short()
            ));
        }
        if let Some(k) = self.find(key.key_id) {
            return Err(format!("trusted already, as {:?}", k.name));
        }
        let name = clip(name.unwrap_or(&key.name));
        let k = TrustedKey {
            name: if name.is_empty() {
                "unnamed key".into()
            } else {
                name
            },
            public_key: key.public_key,
            key_id: key.key_id,
            fingerprint: key.fingerprint().to_string(),
            added_unix_s: now_unix_s,
            note: note.map(clip).filter(|n| !n.is_empty()),
        };
        self.keys.push(k.clone());
        Ok(k)
    }

    /// Removes the key whose fingerprint `text` names (all of it, or an
    /// unambiguous prefix of at least 8 hex digits).
    pub fn remove(&mut self, text: &str) -> Result<TrustedKey, String> {
        if ocw_world::manifest::normalize_fingerprint(text).is_none() {
            return Err(format!(
                "{text:?} is not a fingerprint (at least 8 hex digits)"
            ));
        }
        let hits: Vec<usize> = (0..self.keys.len())
            .filter(|&i| self.keys[i].fingerprint().matches(text))
            .collect();
        match hits.as_slice() {
            [i] => Ok(self.keys.remove(*i)),
            [] => Err(format!("no trusted key with the fingerprint {text}")),
            _ => Err(format!(
                "{text} names {} keys; give more of the fingerprint",
                hits.len()
            )),
        }
    }

    /// The keys as roots of a trust store (marked as user keys).
    pub fn roots(&self) -> Vec<RootKey> {
        self.keys.iter().map(TrustedKey::root).collect()
    }
}

/// `builtin` with the user keys `user` appended (marked as user keys). A
/// user key that is also a built-in root is dropped: the built-in entry
/// wins, so the list never overrides an official root.
pub fn with_user_keys(mut builtin: TrustStore, user: &[RootKey]) -> TrustStore {
    for k in user {
        if builtin.root(k.key_id).is_none() && k.check().is_ok() {
            let mut k = k.clone();
            k.user = true;
            builtin.roots.push(k);
        }
    }
    builtin
}

/// The keys accepted with *Load once* until the app quits.
static ONCE: Mutex<Vec<RootKey>> = Mutex::new(Vec::new());

/// *Load once*: accepts `root` until the app quits, without remembering it.
pub fn accept_once(root: &RootKey) {
    let mut once = ONCE.lock().unwrap_or_else(|e| e.into_inner());
    if !once.iter().any(|k| k.key_id == root.key_id) {
        let mut k = root.clone();
        k.user = true;
        once.push(k);
    }
}

/// The keys accepted with *Load once* so far.
pub fn once_roots() -> Vec<RootKey> {
    ONCE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Forgets a *Load once* key (it was removed from the trusted list, or a
/// test is done with it).
pub fn forget_once(id: KeyId) {
    ONCE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|k| k.key_id != id);
}

/// What the trust dialog shows and decides about.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustRequest {
    /// The region's name, its id and version (from the manifest).
    pub world: String,
    pub id: String,
    pub version: u64,
    pub created_unix_s: i64,
    /// The name the key claims – not verified.
    pub claimed_signer: String,
    /// The key to trust (the root embedded in the manifest).
    pub root: RootKey,
    pub attributions: Vec<Attribution>,
}

impl TrustRequest {
    /// The request for a manifest signed by a user key (with its root).
    pub fn of(signed: &SignedManifest) -> Option<Self> {
        let root = signed.embedded_root()?.clone();
        let m = &signed.manifest;
        Some(Self {
            world: m.name.clone(),
            id: m.id.clone(),
            version: m.version,
            created_unix_s: m.created_unix_s,
            claimed_signer: root.name.clone(),
            root,
            attributions: m.attribution.clone(),
        })
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.root.fingerprint()
    }

    /// "signed by 'Ada Rider' (unverified name)".
    pub fn claimed(&self) -> String {
        claimed(&self.claimed_signer)
    }
}

/// "signed by 'NAME' (unverified name)": how a key's own name is shown.
pub fn claimed(name: &str) -> String {
    format!("signed by '{}' (unverified name)", clip(name))
}

/// What to do with a package's manifest.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Signed by a trusted key: load it (`signer.root_user`: a trusted
    /// user key, an *unofficial* world).
    Load(Signer),
    /// Intact and signed by a user key nobody here trusts: ask the user
    /// (the trust dialog).
    Ask(Box<TrustRequest>),
    /// Not vouched for in another way (unsigned, an unknown root without
    /// its key, outside the key's validity): the unsigned-package policy
    /// decides, as before.
    Policy(VerifyError),
    /// Damaged or tampered with (a bad signature, a revoked key): refused,
    /// never asked about.
    Refuse(VerifyError),
}

/// The decision for `signed` under `trust` (built-in roots plus the
/// user's keys, [`with_user_keys`]) at `now_unix_s`.
pub fn decide(signed: &SignedManifest, trust: &TrustStore, now_unix_s: i64) -> Decision {
    match signed.verify_signature(trust, now_unix_s) {
        Ok(s) => Decision::Load(s),
        Err(e) if e.needs_consent() => match TrustRequest::of(signed) {
            Some(r) => Decision::Ask(Box::new(r)),
            None => Decision::Refuse(e),
        },
        Err(e) if e.is_trust_failure() => Decision::Policy(e),
        Err(e) => Decision::Refuse(e),
    }
}

/// A key typed or pasted by the user (`world trust add`): 64 hex digits of
/// the public key, a root public key file's JSON, or a manifest's JSON
/// with the key embedded.
pub fn parse_key(text: &str) -> Result<RootKey, String> {
    let t = text.trim();
    if let Ok(h) = Hex::<32>::parse(&t.replace([' ', ':'], "")) {
        return RootKey::from_public("unnamed key", h.0);
    }
    if let Ok(k) = RootKey::parse(t) {
        return Ok(k);
    }
    if let Ok(m) = SignedManifest::parse(t) {
        return m
            .embedded_root()
            .cloned()
            .ok_or_else(|| "this manifest carries no user key".to_string());
    }
    Err("not a key: give 64 hex digits of the public key, a .pub.json key file or a package's manifest".into())
}

/// `$OCW_TRUSTED_KEYS`, else `<data dir>/ocw/trusted-keys.json` (the
/// desktop and the server: per user account).
#[cfg(not(target_arch = "wasm32"))]
pub fn default_path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("OCW_TRUSTED_KEYS") {
        return Some(p.into());
    }
    dirs::data_dir().map(|d| d.join("ocw").join(FILE_NAME))
}

/// A key named on a command line (`world trust add`, `ocw-server
/// --trust-key`): a file – a `.pub.json` key file, a manifest, or a
/// package (its manifest) – or the key's text ([`parse_key`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_key(arg: &str) -> Result<RootKey, String> {
    let p = std::path::Path::new(arg);
    if !p.is_file() {
        return parse_key(arg);
    }
    let path = if arg.ends_with(".ocwpkg") {
        crate::manifest_path_for(p)
            .ok_or_else(|| format!("{arg}: no manifest next to the package"))?
    } else {
        p.to_path_buf()
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_key(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The store's text: `Ok(None)` when there is no file yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes the store atomically (a temporary file, then a rename).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file(path: &std::path::Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocw_world::manifest::{Manifest, RootKeyPair, SigningKeyPair};

    pub(crate) const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    /// A throwaway user key (test only; fictional names).
    pub(crate) fn user_key(name: &str, seed: u8) -> (RootKeyPair, SigningKeyPair) {
        let root = RootKeyPair::from_seed(name, [seed; 32]);
        let s = [seed ^ 0x5a; 32];
        let cert = root.certify(
            name,
            SigningKeyPair::public_key_of(s),
            NOW - DAY,
            NOW + 3650 * DAY,
        );
        let key = SigningKeyPair::new(s, cert)
            .unwrap()
            .with_root(root.public())
            .unwrap();
        (root, key)
    }

    fn signed(key: &SigningKeyPair) -> SignedManifest {
        let mut m = Manifest::new(
            "user/ada/alps",
            "Ada's Alps",
            3,
            NOW,
            "test",
            vec![Attribution::openstreetmap(), Attribution::srtm()],
        )
        .unwrap();
        m.add_file("alps.ocwpkg", b"not a real package").unwrap();
        SignedManifest::signed(&m, key)
    }

    #[test]
    fn the_store_round_trips_and_refuses_built_in_roots() {
        let (ada, _) = user_key("Ada Rider", 11);
        let (bo, _) = user_key("Bo Builder", 12);
        let official = RootKeyPair::from_seed("OCW test root", [13; 32]).public();
        let builtin = TrustStore::new([official.clone()]);
        let mut keys = UserKeys::default();
        let k = keys
            .add(&ada.public(), None, Some("met at the club"), NOW, &builtin)
            .unwrap();
        assert_eq!(k.name, "Ada Rider");
        assert_eq!(k.fingerprint, ada.public().fingerprint().to_string());
        assert_eq!(k.added_unix_s, NOW);
        keys.add(&bo.public(), Some("Bo (club)"), None, NOW + 1, &builtin)
            .unwrap();
        // Twice, or a built-in root: refused.
        assert!(keys.add(&ada.public(), None, None, NOW, &builtin).is_err());
        let e = keys.add(&official, None, None, NOW, &builtin).unwrap_err();
        assert!(e.contains("built-in"), "{e}");
        // Saved and read again: the same list.
        let text = keys.to_json();
        assert_eq!(UserKeys::parse(&text).unwrap(), keys);
        let (again, warn) = UserKeys::load(Ok(Some(text)));
        assert_eq!((again, warn), (keys.clone(), None));
        // The roots are user keys.
        assert!(keys.roots().iter().all(|r| r.user));
        // Removing by fingerprint (a prefix, any case and spacing).
        let fp = bo.public().fingerprint();
        assert!(keys.remove("zz").is_err());
        let gone = keys.remove(&fp.lines(4)[0].to_lowercase()).unwrap();
        assert_eq!(gone.name, "Bo (club)");
        assert!(keys.remove(&fp.to_string()).is_err(), "gone");
        assert_eq!(keys.keys.len(), 1);
    }

    #[test]
    fn a_corrupt_store_is_empty_with_a_warning() {
        for bad in [
            "{ not json",
            r#"{"kind":"something else","format":1,"keys":[]}"#,
            r#"{"kind":"ocw-trusted-keys","format":9,"keys":[]}"#,
        ] {
            let (k, warn) = UserKeys::load(Ok(Some(bad.into())));
            assert!(k.keys.is_empty());
            assert!(
                warn.is_some_and(|w| w.contains("could not be read")),
                "{bad}"
            );
        }
        // A fingerprint that does not belong to the key: refused (someone
        // edited the file by hand).
        let (ada, _) = user_key("Ada Rider", 21);
        let mut keys = UserKeys::default();
        keys.add(&ada.public(), None, None, NOW, &TrustStore::default())
            .unwrap();
        let text = keys
            .to_json()
            .replace(&keys.keys[0].fingerprint[..4], "0000");
        assert!(UserKeys::parse(&text).unwrap_err().contains("fingerprint"));
        // Nothing saved yet, an unreadable store.
        assert_eq!(UserKeys::load(Ok(None)), (UserKeys::default(), None));
        assert!(UserKeys::load(Err("blocked".into())).1.is_some());
    }

    #[test]
    fn user_keys_never_override_built_in_roots() {
        let official = RootKeyPair::from_seed("OCW test root", [31; 32]).public();
        let (ada, _) = user_key("Ada Rider", 32);
        // The same key as a user key: the built-in entry stays (official).
        let mut fake = official.clone();
        fake.name = "Ada Rider".into();
        let t = with_user_keys(TrustStore::new([official.clone()]), &[fake, ada.public()]);
        assert_eq!(t.roots.len(), 2);
        assert_eq!(t.roots[0], official);
        assert!(!t.roots[0].user);
        assert!(t.roots[1].user);
        // A store cannot hold a built-in key at all.
        let mut keys = UserKeys::default();
        assert!(
            keys.add(
                &official,
                None,
                None,
                NOW,
                &TrustStore::new([official.clone()])
            )
            .is_err()
        );
    }

    #[test]
    fn the_decision_follows_the_key() {
        let (ada, key) = user_key("Ada Rider", 41);
        let m = signed(&key);
        let official_root = RootKeyPair::from_seed("OCW test root", [42; 32]);
        let builtin = TrustStore::new([official_root.public()]);
        // Unknown user key: ask, with what the dialog shows.
        let Decision::Ask(r) = decide(&m, &builtin, NOW) else {
            panic!("{:?}", decide(&m, &builtin, NOW));
        };
        assert_eq!(r.world, "Ada's Alps");
        assert_eq!((r.id.as_str(), r.version), ("user/ada/alps", 3));
        assert_eq!(r.claimed(), "signed by 'Ada Rider' (unverified name)");
        assert_eq!(r.fingerprint(), ada.public().fingerprint());
        assert_eq!(r.attributions.len(), 2);
        // Trusted: load, unofficial.
        let mut keys = UserKeys::default();
        keys.add(&ada.public(), None, None, NOW, &builtin).unwrap();
        let trusted = with_user_keys(builtin.clone(), &keys.roots());
        match decide(&m, &trusted, NOW) {
            Decision::Load(s) => assert!(s.root_user),
            d => panic!("{d:?}"),
        }
        // Trusted, then removed: asked again.
        keys.remove(&ada.public().fingerprint().to_string())
            .unwrap();
        let after = with_user_keys(builtin.clone(), &keys.roots());
        assert!(matches!(decide(&m, &after, NOW), Decision::Ask(_)));
        // A built-in root: load, official.
        let seed = [43; 32];
        let cert = official_root.certify(
            "builds",
            SigningKeyPair::public_key_of(seed),
            NOW - DAY,
            NOW + DAY,
        );
        let off = SignedManifest::signed(&m.manifest, &SigningKeyPair::new(seed, cert).unwrap());
        match decide(&off, &builtin, NOW) {
            Decision::Load(s) => assert!(!s.root_user),
            d => panic!("{d:?}"),
        }
        // A bad signature: refused, no dialog.
        let tampered =
            SignedManifest::parse(&m.to_json().replacen("Ada's Alps", "Ada's Alpz", 1)).unwrap();
        assert!(matches!(
            decide(&tampered, &builtin, NOW),
            Decision::Refuse(VerifyError::BadSignature(_))
        ));
        // Unsigned: the policy decides, as before.
        let unsigned = SignedManifest::unsigned(&m.manifest);
        assert!(matches!(
            decide(&unsigned, &builtin, NOW),
            Decision::Policy(VerifyError::Unsigned)
        ));
        // Load once: accepted until the app quits, never saved.
        accept_once(&ada.public());
        let once = with_user_keys(builtin.clone(), &once_roots());
        assert!(matches!(decide(&m, &once, NOW), Decision::Load(_)));
        forget_once(ada.public().key_id);
        assert!(!once_roots().iter().any(|k| k.key_id == ada.public().key_id));
    }

    #[test]
    fn keys_are_read_from_hex_key_files_and_manifests() {
        let (ada, key) = user_key("Ada Rider", 51);
        let p = ada.public();
        assert_eq!(
            parse_key(&p.public_key.to_string()).unwrap().key_id,
            p.key_id
        );
        assert_eq!(parse_key(&p.to_json()).unwrap(), p);
        assert_eq!(parse_key(&signed(&key).to_json()).unwrap(), p);
        assert!(parse_key("hello").is_err());
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("sub").join(FILE_NAME);
        assert_eq!(read_file(&path).unwrap(), None);
        write_file(&path, "x").unwrap();
        assert_eq!(read_file(&path).unwrap().as_deref(), Some("x"));
    }
}
