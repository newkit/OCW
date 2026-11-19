//! Remembered Bluetooth devices (spec §3.4: "paired devices are remembered
//! and reconnected automatically"), kept in the local config file
//! (`ocw.toml [devices]`, spec 3.2.4; the old `devices.toml` is migrated
//! into it), or in the file `--devices-file` names.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Slot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevice {
    /// Platform Bluetooth identifier (from `devices.scan`).
    pub id: String,
    pub name: String,
    /// What it is (`trainer`, `power`, `cadence`, `hr`;
    /// `ocw_client::menu::devices::Role`), for the Devices page before it
    /// connects. Older files have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevices {
    #[serde(default, rename = "device")]
    pub devices: Vec<KnownDevice>,
}

/// Where the devices are kept: `--devices-file` if it exists, else the
/// default (`ocw.toml [devices]`). Regression: the test machine's kiosk
/// passed a file under /tmp, which a reboot deleted; the app then silently
/// rode the virtual trainer and the trainer on the bike had no effect.
pub fn choose_slot(given: Option<PathBuf>, default: Option<Slot>) -> Option<Slot> {
    match given {
        Some(p) if p.exists() => Some(Slot::File(p)),
        Some(p) => {
            tracing::warn!(missing = %p.display(), fallback = ?default, "--devices-file not found, using the default (ocw.toml [devices])");
            default.or(Some(Slot::File(p)))
        }
        None => default,
    }
}

impl KnownDevices {
    /// Loads the file; a missing file means no devices.
    #[cfg(test)]
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        Self::load_slot(&Slot::File(path.to_path_buf()))
    }

    #[cfg(test)]
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        self.save_slot(&Slot::File(path.to_path_buf()))
    }

    /// Loads the devices of `slot`; nothing saved means no devices.
    pub fn load_slot(slot: &Slot) -> Result<Self, String> {
        match slot.read()? {
            Some(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", slot.describe())),
            None => Ok(Self::default()),
        }
    }

    pub fn save_slot(&self, slot: &Slot) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        slot.write(&text)
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

    #[test]
    fn a_missing_devices_file_falls_back_to_the_default() {
        let dir = std::env::temp_dir().join(format!("ocw-known-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let given = dir.join("devices.toml");
        let default = Some(crate::config::ConfigFiles::in_dir(&dir).section("devices"));
        // Missing: the default.
        assert_eq!(choose_slot(Some(given.clone()), default.clone()), default);
        // Present: the given file.
        std::fs::write(&given, "").unwrap();
        assert_eq!(
            choose_slot(Some(given.clone()), default.clone()),
            Some(Slot::File(given.clone()))
        );
        // Nothing given: the default; no default at all: the given path.
        assert_eq!(choose_slot(None, default.clone()), default);
        std::fs::remove_file(&given).unwrap();
        assert_eq!(
            choose_slot(Some(given.clone()), None),
            Some(Slot::File(given))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
            kind: Some("trainer".into()),
        });
        k.remember(KnownDevice {
            id: "hci0/dev_CC".into(),
            name: "HRM".into(),
            kind: None,
        });
        k.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[device]]") && text.contains("KICKR CORE 1234"));
        assert!(text.contains("kind = \"trainer\""), "{text}");
        assert_eq!(KnownDevices::load(&path).unwrap(), k);
        // A file of an older version (no kinds) still reads.
        std::fs::write(&path, "[[device]]\nid = \"x\"\nname = \"KICKR\"\n").unwrap();
        assert_eq!(KnownDevices::load(&path).unwrap().devices[0].kind, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remember_updates_and_forget_removes() {
        let mut k = KnownDevices::default();
        let d = |name: &str| KnownDevice {
            id: "x".into(),
            name: name.into(),
            kind: None,
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
