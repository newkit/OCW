//! Remembered Bluetooth devices (spec §3.4: "paired devices are remembered
//! and reconnected automatically"), stored as TOML in the user's config
//! directory (`~/.config/ocw/devices.toml` on Linux).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevice {
    /// Platform Bluetooth identifier (from `devices.scan`).
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevices {
    #[serde(default, rename = "device")]
    pub devices: Vec<KnownDevice>,
}

/// Default location; `OCW_CONFIG_DIR` overrides the directory.
pub fn default_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("OCW_CONFIG_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir()?.join("ocw"),
    };
    Some(dir.join("devices.toml"))
}

impl KnownDevices {
    /// Loads the file; a missing file means no devices.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Adds a device or updates its name.
    pub fn remember(&mut self, device: KnownDevice) {
        match self.devices.iter_mut().find(|d| d.id == device.id) {
            Some(d) => d.name = device.name,
            None => self.devices.push(device),
        }
    }

    /// Removes a device by id; returns whether it was known.
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-known-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_is_empty_and_round_trip_works() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("nested/devices.toml");
        assert_eq!(KnownDevices::load(&path).unwrap(), KnownDevices::default());
        let mut k = KnownDevices::default();
        k.remember(KnownDevice {
            id: "hci0/dev_AA_BB".into(),
            name: "KICKR CORE 1234".into(),
        });
        k.remember(KnownDevice {
            id: "hci0/dev_CC".into(),
            name: "HRM".into(),
        });
        k.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[device]]") && text.contains("KICKR CORE 1234"));
        assert_eq!(KnownDevices::load(&path).unwrap(), k);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remember_updates_and_forget_removes() {
        let mut k = KnownDevices::default();
        let d = |name: &str| KnownDevice {
            id: "x".into(),
            name: name.into(),
        };
        k.remember(d("old"));
        k.remember(d("new"));
        assert_eq!(k.devices, vec![d("new")]);
        assert!(k.forget("x"));
        assert!(!k.forget("x"));
        assert!(k.devices.is_empty());
    }

    #[test]
    fn broken_file_is_an_error() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.toml");
        std::fs::write(&path, "[[device]]\nid = 3").unwrap();
        assert!(KnownDevices::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
