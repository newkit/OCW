//! Key files (spec §4.7, §12).
//!
//! - `root.key`: the root secret key. Generated once, then kept **offline**
//!   (it only signs signing keys).
//! - `root.pub.json`: the root public key, which clients trust.
//! - `<name>.key`: an online signing key with its root-signed certificate.
//! - A user's own key (spec 4.7 *User keys*): `user-root.key`,
//!   `user-root.pub.json` and `user.key`, the signing key that embeds its
//!   root ([`init_user`], made by `world-builder keys init-user` or by OCW
//!   Studio on its first *Build package*; read with [`user_signing_key`]).
//!
//! Secret key files are created with mode 0600 and never overwritten. The
//! default directory is `~/.config/ocw/keys` (`$OCW_KEY_DIR` overrides),
//! outside any checkout; the repository's `.gitignore` also ignores `*.key`.

use std::io::Write;
use std::path::{Path, PathBuf};

use ocw_world::manifest::{Fingerprint, RootKey, RootKeyPair, SigningKeyPair};

/// `$OCW_KEY_DIR`, else `<config dir>/ocw/keys`.
pub fn default_key_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OCW_KEY_DIR") {
        return PathBuf::from(d);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ocw/keys")
}

/// 32 bytes from the operating system's random number generator.
pub fn random_seed() -> Result<[u8; 32], String> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no OS randomness: {e}"))?;
    Ok(seed)
}

/// Writes a new file (never replaces one); `secret` files get mode 0600.
pub fn write_new(path: &Path, contents: &str, secret: bool) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = secret;
    let mut f = opts.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => {
            format!("{}: exists; not overwriting a key file", path.display())
        }
        _ => format!("{}: {e}", path.display()),
    })?;
    f.write_all(contents.as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A user's own root secret, its public key and the signing key, in the
/// key directory (spec 4.7 *User keys*).
pub const USER_ROOT_SECRET: &str = "user-root.key";
pub const USER_ROOT_PUBLIC: &str = "user-root.pub.json";
pub const USER_SIGNING: &str = "user.key";
/// The validity of a user's signing key (10 years).
pub const USER_KEY_DAYS: u32 = 3650;
const DAY_S: i64 = 86_400;

/// A user's key just made by [`init_user`].
pub struct UserKeyFiles {
    pub root_secret: PathBuf,
    pub root_public: PathBuf,
    pub signing: PathBuf,
    /// The root's public key (what others trust).
    pub root: RootKey,
    pub key: SigningKeyPair,
}

impl UserKeyFiles {
    pub fn fingerprint(&self) -> Fingerprint {
        self.root.fingerprint()
    }
}

/// Makes a user's own key in `dir` (spec 4.7 *User keys*): a root of
/// their own named `name` and a signing key for `days` that carries the
/// root's public key. Never overwrites a key file: if any of the three is
/// there already, nothing is written.
pub fn init_user(
    dir: &Path,
    name: &str,
    days: u32,
    now_unix_s: i64,
) -> Result<UserKeyFiles, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a key needs a name".into());
    }
    let (root_secret, root_public, signing) = (
        dir.join(USER_ROOT_SECRET),
        dir.join(USER_ROOT_PUBLIC),
        dir.join(USER_SIGNING),
    );
    for p in [&root_secret, &root_public, &signing] {
        if p.exists() {
            return Err(format!(
                "{}: exists; not overwriting a key file",
                p.display()
            ));
        }
    }
    let root = RootKeyPair::from_seed(name, random_seed()?);
    let seed = random_seed()?;
    let cert = root.certify(
        name,
        SigningKeyPair::public_key_of(seed),
        now_unix_s - 3600,
        now_unix_s + i64::from(days) * DAY_S,
    );
    let key = SigningKeyPair::new(seed, cert)?.with_root(root.public())?;
    write_new(&root_secret, &root.to_json(), true)?;
    write_new(&root_public, &root.public().to_json(), false)?;
    write_new(&signing, &key.to_json(), true)?;
    Ok(UserKeyFiles {
        root_secret,
        root_public,
        signing,
        root: root.public(),
        key,
    })
}

/// The user's own signing key in `dir` (`user.key`): `Ok(None)` when no
/// user key was made there yet; an error when it cannot be used – the file
/// is damaged, not a user key (no root of its own), not valid now, or the
/// root is there without its signing key. Nothing is ever replaced: the
/// caller says what is wrong and makes no new key.
pub fn user_signing_key(dir: &Path, now_unix_s: i64) -> Result<Option<SigningKeyPair>, String> {
    let path = dir.join(USER_SIGNING);
    if !path.exists() {
        let root = dir.join(USER_ROOT_SECRET);
        if root.exists() {
            return Err(format!(
                "{} is there but its signing key {} is missing",
                root.display(),
                path.display()
            ));
        }
        return Ok(None);
    }
    let key = read_signing_key(&path)?;
    if key.root.is_none() {
        return Err(format!(
            "{}: not a user key (it has no root of its own; made with `keys new-signing`?)",
            path.display()
        ));
    }
    let c = key.claims();
    if now_unix_s < c.valid_from_unix_s || now_unix_s > c.valid_until_unix_s {
        return Err(format!("{}: the key is not valid now", path.display()));
    }
    Ok(Some(key))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_root_secret(path: &Path) -> Result<RootKeyPair, String> {
    RootKeyPair::from_json(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_signing_key(path: &Path) -> Result<SigningKeyPair, String> {
    SigningKeyPair::from_json(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_root_public(path: &Path) -> Result<RootKey, String> {
    RootKey::parse(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_files_are_private_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let root = RootKeyPair::from_seed("r", random_seed().unwrap());
        let path = dir.path().join("sub/root.key");
        write_new(&path, &root.to_json(), true).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(
            write_new(&path, "x", true)
                .unwrap_err()
                .contains("not overwriting")
        );
        assert_eq!(read_root_secret(&path).unwrap().public(), root.public());
        let pubf = dir.path().join("root.pub.json");
        write_new(&pubf, &root.public().to_json(), false).unwrap();
        assert_eq!(read_root_public(&pubf).unwrap(), root.public());
        assert!(read_signing_key(&path).is_err());
        // Two seeds differ (the OS generator works).
        assert_ne!(random_seed().unwrap(), random_seed().unwrap());
    }

    #[test]
    fn a_user_key_is_made_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("keys");
        let now = 1_800_000_000;
        assert!(user_signing_key(&d, now).unwrap().is_none());
        let made = init_user(&d, "  Ada Rider ", USER_KEY_DAYS, now).unwrap();
        assert_eq!(made.root.name, "Ada Rider");
        #[cfg(unix)]
        for p in [&made.root_secret, &made.signing] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", p.display());
        }
        let k = user_signing_key(&d, now).unwrap().expect("made");
        assert_eq!(k.root.as_ref(), Some(&made.root));
        assert_eq!(k.claims().name, "Ada Rider");
        // Never made twice: the files stay as they are.
        let before = std::fs::read(&made.signing).unwrap();
        assert!(
            init_user(&d, "Bea Climber", USER_KEY_DAYS, now)
                .err()
                .expect("an error")
                .contains("not overwriting")
        );
        assert_eq!(std::fs::read(&made.signing).unwrap(), before);
        assert!(init_user(&dir.path().join("k2"), " ", USER_KEY_DAYS, now).is_err());
        // Not valid ten years and a day later.
        assert!(user_signing_key(&d, now + 3651 * DAY_S).is_err());
        // A damaged file is an error, never "no key".
        std::fs::write(&made.signing, "{ broken").unwrap();
        assert!(user_signing_key(&d, now).is_err());
        // The root without its signing key: an error too.
        std::fs::remove_file(&made.signing).unwrap();
        assert!(
            user_signing_key(&d, now)
                .err()
                .expect("an error")
                .contains("missing")
        );
        // A signing key without a root of its own is not a user key.
        let d2 = dir.path().join("official");
        let root = RootKeyPair::from_seed("r", random_seed().unwrap());
        let seed = random_seed().unwrap();
        let cert = root.certify("s", SigningKeyPair::public_key_of(seed), now - 1, now + 100);
        let plain = SigningKeyPair::new(seed, cert).unwrap();
        write_new(&d2.join(USER_SIGNING), &plain.to_json(), true).unwrap();
        assert!(
            user_signing_key(&d2, now)
                .err()
                .expect("an error")
                .contains("not a user key")
        );
    }
}
