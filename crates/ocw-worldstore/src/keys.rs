//! Key files (spec §4.7, §12).
//!
//! - `root.key`: the root secret key. Generated once, then kept **offline**
//!   (it only signs signing keys).
//! - `root.pub.json`: the root public key, which clients trust.
//! - `<name>.key`: an online signing key with its root-signed certificate.
//!
//! Secret key files are created with mode 0600 and never overwritten. The
//! default directory is `~/.config/ocw/keys` (`$OCW_KEY_DIR` overrides),
//! outside any checkout; the repository's `.gitignore` also ignores `*.key`.

use std::io::Write;
use std::path::{Path, PathBuf};

use ocw_world::manifest::{RootKey, RootKeyPair, SigningKeyPair};

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
}
