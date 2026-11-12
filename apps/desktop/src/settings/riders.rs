//! Rider profiles (spec §3.2.1, Settings → Riders; chosen in the
//! Workshop): name, weight, optional height and FTP. Stored in
//! `riders.toml` next to `settings.toml`:
//!
//! ```toml
//! active = "Alex"
//!
//! [[rider]]
//! name = "Alex"
//! weight_kg = 78.0
//! height_cm = 182.0
//! ftp_w = 250
//! ```
//!
//! The active rider's weight is the rider mass of the physics (sent to the
//! server in the hello, §6.1) and, with the bike, the trainer's SIM mass.
//! Bevy-free.

use std::path::Path;

use ocw_core::{MAX_RIDER_WEIGHT_KG, MIN_RIDER_WEIGHT_KG, RiderProfile};
use serde::{Deserialize, Serialize};

/// Accepted body heights in cm.
pub const HEIGHT_RANGE_CM: (f64, f64) = (100.0, 250.0);
/// Accepted FTP values in W.
pub const FTP_RANGE_W: (u32, u32) = (30, 700);
/// Longest rider name in characters.
pub const MAX_NAME_CHARS: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rider {
    pub name: String,
    pub weight_kg: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_cm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_w: Option<u32>,
}

impl Default for Rider {
    /// The default rider: 75 kg, as the physics default (§3.2.1).
    fn default() -> Self {
        Self {
            name: "Rider".into(),
            weight_kg: RiderProfile::default().rider_mass_kg,
            height_cm: None,
            ftp_w: None,
        }
    }
}

impl Rider {
    /// The physics profile this rider rides with (spec §2.5): the curated
    /// default with the rider's (clamped) weight.
    pub fn physics(&self) -> RiderProfile {
        RiderProfile::for_rider_weight(self.weight_kg)
    }

    /// "78 kg, 182 cm, FTP 250 W".
    pub fn summary(&self) -> String {
        let mut s = format!("{} kg", fmt_num(self.weight_kg));
        if let Some(h) = self.height_cm {
            s.push_str(&format!(", {} cm", fmt_num(h)));
        }
        if let Some(f) = self.ftp_w {
            s.push_str(&format!(", FTP {f} W"));
        }
        s
    }

    /// Checks a profile entered by the user.
    pub fn validate(&self) -> Result<(), String> {
        check_name(&self.name)?;
        check_weight(self.weight_kg)?;
        if let Some(h) = self.height_cm
            && !(HEIGHT_RANGE_CM.0..=HEIGHT_RANGE_CM.1).contains(&h)
        {
            return Err(format!(
                "height must be {} to {} cm",
                HEIGHT_RANGE_CM.0, HEIGHT_RANGE_CM.1
            ));
        }
        if let Some(f) = self.ftp_w
            && !(FTP_RANGE_W.0..=FTP_RANGE_W.1).contains(&f)
        {
            return Err(format!(
                "FTP must be {} to {} W",
                FTP_RANGE_W.0, FTP_RANGE_W.1
            ));
        }
        Ok(())
    }
}

/// A number without a trailing ".0".
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.trim() != name {
        return Err("a rider name must not be empty or start / end with spaces".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "a rider name has at most {MAX_NAME_CHARS} characters"
        ));
    }
    // Names are quoted in console command lines.
    if name.contains('"') || name.chars().any(char::is_control) {
        return Err("a rider name must not contain quotes".into());
    }
    Ok(())
}

pub fn check_weight(kg: f64) -> Result<(), String> {
    if (MIN_RIDER_WEIGHT_KG..=MAX_RIDER_WEIGHT_KG).contains(&kg) {
        Ok(())
    } else {
        Err(format!(
            "weight must be {MIN_RIDER_WEIGHT_KG} to {MAX_RIDER_WEIGHT_KG} kg"
        ))
    }
}

/// Changes to a rider (`riders.edit`); `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiderEdit {
    pub name: Option<String>,
    pub weight_kg: Option<f64>,
    /// `Some(None)` clears the height.
    pub height_cm: Option<Option<f64>>,
    pub ftp_w: Option<Option<u32>>,
}

/// The rider profiles and which one is active.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Riders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default, rename = "rider")]
    pub riders: Vec<Rider>,
}

impl Riders {
    /// Reads the profiles; returns them and warnings for entries that had to
    /// be repaired (weight clamped, nameless or duplicate riders dropped).
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let mut r: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        let warnings = r.repair();
        Ok((r, warnings))
    }

    /// Reads the file; a missing file means no profiles (the default rider).
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("riders serialize")
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_toml()).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn repair(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut kept: Vec<Rider> = Vec::new();
        for mut r in std::mem::take(&mut self.riders) {
            if let Err(e) = check_name(&r.name) {
                warnings.push(format!("rider {:?} dropped: {e}", r.name));
                continue;
            }
            if kept.iter().any(|k| k.name.eq_ignore_ascii_case(&r.name)) {
                warnings.push(format!(
                    "rider {:?} dropped: the name is used twice",
                    r.name
                ));
                continue;
            }
            if check_weight(r.weight_kg).is_err() {
                let w = ocw_core::clamp_rider_weight(r.weight_kg);
                warnings.push(format!(
                    "rider {:?}: weight {} kg out of range, using {w} kg",
                    r.name, r.weight_kg
                ));
                r.weight_kg = w;
            }
            if r.height_cm
                .is_some_and(|h| !(HEIGHT_RANGE_CM.0..=HEIGHT_RANGE_CM.1).contains(&h))
            {
                warnings.push(format!("rider {:?}: height ignored (out of range)", r.name));
                r.height_cm = None;
            }
            if r.ftp_w
                .is_some_and(|f| !(FTP_RANGE_W.0..=FTP_RANGE_W.1).contains(&f))
            {
                warnings.push(format!("rider {:?}: FTP ignored (out of range)", r.name));
                r.ftp_w = None;
            }
            kept.push(r);
        }
        self.riders = kept;
        if let Some(a) = &self.active
            && self.find(a).is_none()
        {
            warnings.push(format!("active rider {a:?} not found, using the first"));
            self.active = None;
        }
        warnings
    }

    /// Index of the rider named `name` (case does not matter).
    pub fn find(&self, name: &str) -> Option<usize> {
        self.riders
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// The active rider: the selected one, else the first, else the
    /// default rider (75 kg).
    pub fn active(&self) -> Rider {
        self.active
            .as_deref()
            .and_then(|a| self.find(a))
            .or(if self.riders.is_empty() {
                None
            } else {
                Some(0)
            })
            .map(|i| self.riders[i].clone())
            .unwrap_or_default()
    }

    /// Whether `name` is the active rider.
    pub fn is_active(&self, name: &str) -> bool {
        !self.riders.is_empty() && self.active().name.eq_ignore_ascii_case(name)
    }

    /// Adds a rider; the first one added becomes the active one.
    pub fn add(&mut self, rider: Rider) -> Result<(), String> {
        rider.validate()?;
        if self.find(&rider.name).is_some() {
            return Err(format!("there is already a rider {:?}", rider.name));
        }
        if self.riders.is_empty() {
            self.active = Some(rider.name.clone());
        }
        self.riders.push(rider);
        Ok(())
    }

    /// Changes a rider; returns the changed profile.
    pub fn edit(&mut self, name: &str, edit: RiderEdit) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        let mut r = self.riders[i].clone();
        if let Some(n) = &edit.name {
            if self.find(n).is_some_and(|j| j != i) {
                return Err(format!("there is already a rider {n:?}"));
            }
            r.name = n.clone();
        }
        if let Some(w) = edit.weight_kg {
            r.weight_kg = w;
        }
        if let Some(h) = edit.height_cm {
            r.height_cm = h;
        }
        if let Some(f) = edit.ftp_w {
            r.ftp_w = f;
        }
        r.validate()?;
        if self.is_active(name) {
            self.active = Some(r.name.clone());
        }
        self.riders[i] = r.clone();
        Ok(r)
    }

    /// Deletes a rider. Deleting the active rider makes the first one
    /// active.
    pub fn remove(&mut self, name: &str) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        let was_active = self.is_active(name);
        let r = self.riders.remove(i);
        if was_active {
            self.active = self.riders.first().map(|r| r.name.clone());
        }
        Ok(r)
    }

    /// Makes `name` the active rider.
    pub fn select(&mut self, name: &str) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        self.active = Some(self.riders[i].name.clone());
        Ok(self.riders[i].clone())
    }

    /// A name for a new rider that is not taken: "Rider 2", "Rider 3", ...
    pub fn new_name(&self) -> String {
        (1..)
            .map(|n| {
                if n == 1 {
                    "Rider".to_string()
                } else {
                    format!("Rider {n}")
                }
            })
            .find(|n| self.find(n).is_none())
            .expect("a free name")
    }
}

fn no_rider(name: &str) -> String {
    format!("no rider {name:?} (riders.list shows them)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider(name: &str, kg: f64) -> Rider {
        Rider {
            name: name.into(),
            weight_kg: kg,
            ..Rider::default()
        }
    }

    #[test]
    fn no_profiles_means_the_default_rider() {
        let r = Riders::default();
        assert_eq!(r.active(), Rider::default());
        assert_eq!(r.active().physics(), RiderProfile::default());
        assert!(!r.is_active("Rider"));
    }

    #[test]
    fn add_select_edit_remove() {
        let mut r = Riders::default();
        r.add(rider("Alex", 78.0)).unwrap();
        // The first rider becomes active.
        assert_eq!(r.active().name, "Alex");
        r.add(rider("Anna", 61.5)).unwrap();
        assert!(r.add(rider("anna", 60.0)).unwrap_err().contains("already"));
        assert!(r.add(rider("Heavy", 250.0)).unwrap_err().contains("weight"));
        assert!(r.add(rider("", 70.0)).is_err());
        assert!(r.add(rider("Say \"hi\"", 70.0)).is_err());
        assert_eq!(r.select("ANNA").unwrap().name, "Anna");
        assert_eq!(r.active().physics().rider_mass_kg, 61.5);
        assert_eq!(r.active().physics().total_mass_kg(), 69.5);
        // Renaming the active rider keeps it active.
        let e = r
            .edit(
                "anna",
                RiderEdit {
                    name: Some("Anna B".into()),
                    weight_kg: Some(62.0),
                    ftp_w: Some(Some(230)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(e.summary(), "62 kg, FTP 230 W");
        assert_eq!(r.active().name, "Anna B");
        let bad = RiderEdit {
            weight_kg: Some(10.0),
            ..Default::default()
        };
        assert!(r.edit("Anna B", bad).is_err());
        assert_eq!(r.active().weight_kg, 62.0);
        let clash = RiderEdit {
            name: Some("alex".into()),
            ..Default::default()
        };
        assert!(r.edit("Anna B", clash).is_err());
        assert!(r.edit("Nobody", RiderEdit::default()).is_err());
        // Removing the active rider activates the first.
        r.remove("Anna B").unwrap();
        assert_eq!(r.active().name, "Alex");
        r.remove("Alex").unwrap();
        assert_eq!(r.active(), Rider::default());
        assert!(r.remove("Alex").is_err());
    }

    #[test]
    fn toml_round_trip() {
        let mut r = Riders::default();
        r.add(Rider {
            name: "Alex".into(),
            weight_kg: 78.0,
            height_cm: Some(182.0),
            ftp_w: Some(250),
        })
        .unwrap();
        r.add(rider("Guest", 70.0)).unwrap();
        let text = r.to_toml();
        assert!(text.contains("active = \"Alex\""), "{text}");
        assert!(
            text.contains("[[rider]]") && text.contains("ftp_w = 250"),
            "{text}"
        );
        // No height / FTP keys for the guest.
        assert_eq!(text.matches("height_cm").count(), 1);
        let (again, warnings) = Riders::parse(&text).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(again, r);
    }

    #[test]
    fn broken_entries_are_repaired_with_warnings() {
        let text = "\
active = \"Ghost\"
[[rider]]
name = \"Big\"
weight_kg = 500.0
height_cm = 20.0
[[rider]]
name = \"big\"
weight_kg = 70.0
[[rider]]
name = \"\"
weight_kg = 70.0
[[rider]]
name = \"Fit\"
weight_kg = 65.0
ftp_w = 5000
";
        let (r, warnings) = Riders::parse(text).unwrap();
        assert_eq!(warnings.len(), 6, "{warnings:?}");
        assert_eq!(r.riders.len(), 2);
        assert_eq!(r.riders[0].weight_kg, MAX_RIDER_WEIGHT_KG);
        assert_eq!(r.riders[0].height_cm, None);
        assert_eq!(r.riders[1].ftp_w, None);
        // The missing active rider: the first.
        assert_eq!(r.active().name, "Big");
        assert!(Riders::parse("[[rider]]\nname = 3").is_err());
    }

    #[test]
    fn new_names_are_free() {
        let mut r = Riders::default();
        assert_eq!(r.new_name(), "Rider");
        r.add(rider("Rider", 70.0)).unwrap();
        r.add(rider("Rider 2", 70.0)).unwrap();
        assert_eq!(r.new_name(), "Rider 3");
    }

    #[test]
    fn files_load_and_save() {
        let dir = std::env::temp_dir().join(format!("ocw-riders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("riders.toml");
        assert_eq!(Riders::load(&path).unwrap().0, Riders::default());
        let mut r = Riders::default();
        r.add(rider("Alex", 80.0)).unwrap();
        r.save(&path).unwrap();
        assert_eq!(Riders::load(&path).unwrap().0, r);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
