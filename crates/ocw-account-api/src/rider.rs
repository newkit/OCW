//! The rider of an account (spec 14 Phase 4, 4O W5b; 4N): one rider per
//! account – its name (unique on the backend, the name rules of
//! `ocw_core::name`), and the rider profile the apps keep (weight, height,
//! FTP, bike profile, the character and the kit's look), stored as the
//! apps' own document so that a newer app's keys survive an older one.
//! `GET /account/rider` reads it, `PUT /account/rider` writes it.

use serde::{Deserialize, Serialize};

/// The largest rider document the backend keeps (bytes of JSON).
pub const MAX_RIDER_BYTES: usize = 16 * 1024;

/// `GET` / `PUT /account/rider`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiderDoc {
    /// The rider name (checked and normalised by the backend; it becomes
    /// the account's display name, the name others see).
    pub name: String,
    /// The apps' rider profile (`ocw_client::riders::Rider` as JSON; its
    /// `name` is the one above).
    pub rider: serde_json::Value,
    /// When it was saved (Unix s; set by the backend).
    #[serde(default)]
    pub updated_at: i64,
}

impl RiderDoc {
    /// Checks the document's size and object shape; the name is the
    /// backend's to check (`ocw_core::name::check`).
    pub fn check_shape(&self) -> Result<(), String> {
        if !self.rider.is_object() {
            return Err("the rider is not an object".into());
        }
        let len = serde_json::to_string(&self.rider).map_or(usize::MAX, |s| s.len());
        if len > MAX_RIDER_BYTES {
            return Err(format!(
                "the rider takes {len} bytes, at most {MAX_RIDER_BYTES}"
            ));
        }
        Ok(())
    }

    /// The document with `name` in both places (what the backend keeps).
    pub fn named(mut self, name: &str, now: i64) -> Self {
        self.name = name.to_owned();
        if let Some(o) = self.rider.as_object_mut() {
            o.insert("name".into(), serde_json::Value::String(name.to_owned()));
        }
        self.updated_at = now;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_and_name() {
        let d = RiderDoc {
            name: "x".into(),
            rider: serde_json::json!({"name": "old", "weight_kg": 70.0}),
            updated_at: 0,
        };
        assert!(d.check_shape().is_ok());
        let d = d.named("Ida Uphill", 9);
        assert_eq!(d.rider["name"], "Ida Uphill");
        assert_eq!((d.name.as_str(), d.updated_at), ("Ida Uphill", 9));
        let big = RiderDoc {
            name: "x".into(),
            rider: serde_json::json!({"pad": "a".repeat(MAX_RIDER_BYTES)}),
            updated_at: 0,
        };
        assert!(big.check_shape().is_err());
        let not = RiderDoc {
            name: "x".into(),
            rider: serde_json::json!([1]),
            updated_at: 0,
        };
        assert!(not.check_shape().is_err());
    }
}
