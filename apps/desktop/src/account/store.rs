//! Where the desktop keeps its sign-in (spec §9.1): the OS keyring
//! (Secret Service on Linux, the Keychain on macOS, the Credential Manager
//! on Windows; the `keyring` crate), else a private file
//! (`<config dir>/ocw/account.json`, mode 0600) when no keyring answers
//! (a headless kiosk without a Secret Service). The file is also what the
//! tests use.

use std::io::Write;
use std::path::PathBuf;

use ocw_auth::client::Session;

/// The keyring entry's service and user.
const SERVICE: &str = "open-cycling-world";

pub enum TokenStore {
    /// The OS keyring, with the file as the fallback.
    Keyring {
        user: String,
        file: PathBuf,
    },
    File(PathBuf),
    /// Nothing kept (no config directory).
    None,
}

impl TokenStore {
    /// The keyring entry for `issuer`'s sign-in, falling back to `file`.
    pub fn keyring(issuer: &str, file: PathBuf) -> Self {
        Self::Keyring {
            user: format!("account:{issuer}"),
            file,
        }
    }

    pub fn load(&self) -> Option<Session> {
        match self {
            Self::Keyring { user, file } => keyring_get(user).or_else(|| load_file(file)),
            Self::File(f) => load_file(f),
            Self::None => None,
        }
    }

    /// Keeps the session; returns where.
    pub fn save(&self, s: &Session) -> Result<&'static str, String> {
        let json = serde_json::to_string(s).map_err(|e| e.to_string())?;
        match self {
            Self::Keyring { user, file } => {
                if keyring_set(user, &json) {
                    // Not also in a file.
                    let _ = std::fs::remove_file(file);
                    Ok("the system keyring")
                } else {
                    save_file(file, &json).map(|()| "a private file")
                }
            }
            Self::File(f) => save_file(f, &json).map(|()| "a private file"),
            Self::None => Ok("memory only"),
        }
    }

    pub fn delete(&self) {
        match self {
            Self::Keyring { user, file } => {
                keyring_delete(user);
                let _ = std::fs::remove_file(file);
            }
            Self::File(f) => {
                let _ = std::fs::remove_file(f);
            }
            Self::None => {}
        }
    }
}

fn load_file(path: &PathBuf) -> Option<Session> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_file(path: &PathBuf, json: &str) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension("json.part");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(err)?;
    }
    f.write_all(json.as_bytes()).map_err(err)?;
    f.sync_all().map_err(err)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(err)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn entry(user: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, user).ok()
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn keyring_get(user: &str) -> Option<Session> {
    let text = entry(user)?.get_password().ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn keyring_set(user: &str, json: &str) -> bool {
    entry(user).is_some_and(|e| e.set_password(json).is_ok())
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn keyring_delete(user: &str) {
    if let Some(e) = entry(user) {
        let _ = e.delete_credential();
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn keyring_get(_: &str) -> Option<Session> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn keyring_set(_: &str, _: &str) -> bool {
    false
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn keyring_delete(_: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_store_is_private_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("ocw-account-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = TokenStore::File(dir.join("account.json"));
        assert!(store.load().is_none());
        let s = Session {
            issuer: "https://accounts.example.org".into(),
            client_id: "ocw-desktop".into(),
            access_token: "at".into(),
            access_expires_at: 10,
            refresh_token: Some("rt".into()),
            account_id: "acc_1".into(),
            name: "Eddy".into(),
            providers: vec!["strava".into()],
        };
        assert_eq!(store.save(&s).unwrap(), "a private file");
        assert_eq!(store.load(), Some(s));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("account.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete();
        assert!(store.load().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
