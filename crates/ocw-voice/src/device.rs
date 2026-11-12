//! Microphone and speaker selection (spec §2.4, M8): the device is saved
//! by name (`voice.input`, `voice.output`; empty = the system default). A
//! saved device that is not there (unplugged headset) falls back to the
//! default without forgetting the choice, so it is used again once it is
//! back.

/// The device to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The saved device, present.
    Saved(String),
    /// The system default; `missing` names a saved device that is not
    /// present (shown in the menu and the log).
    Default { missing: Option<String> },
}

impl Choice {
    /// The device name to open (`None`: the default device).
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Saved(n) => Some(n),
            Self::Default { .. } => None,
        }
    }
}

/// Picks the device for the saved name among the present ones. Names
/// match exactly first, then ignoring case and surrounding blanks (some
/// systems change the case between releases).
pub fn choose(saved: Option<&str>, present: &[String]) -> Choice {
    let Some(saved) = saved
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "default")
    else {
        return Choice::Default { missing: None };
    };
    if let Some(n) = present.iter().find(|n| n.as_str() == saved) {
        return Choice::Saved(n.clone());
    }
    if let Some(n) = present
        .iter()
        .find(|n| n.trim().eq_ignore_ascii_case(saved))
    {
        return Choice::Saved(n.clone());
    }
    Choice::Default {
        missing: Some(saved.to_owned()),
    }
}

/// The device after `current` in the menu's ← / → cycle: the default
/// (shown as "default"), then the present devices in order.
pub fn step(current: Option<&str>, present: &[String], dir: i32) -> Option<String> {
    let mut all: Vec<Option<&str>> = vec![None];
    all.extend(present.iter().map(|s| Some(s.as_str())));
    let at = all
        .iter()
        .position(|d| *d == current.filter(|c| !c.is_empty() && *c != "default"))
        .unwrap_or(0) as i32;
    let n = all.len() as i32;
    all[(at + dir.signum()).rem_euclid(n) as usize].map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Vec<String> {
        vec!["Built-in Microphone".into(), "USB Headset".into()]
    }

    #[test]
    fn saved_device_present() {
        assert_eq!(
            choose(Some("USB Headset"), &devices()),
            Choice::Saved("USB Headset".into())
        );
        assert_eq!(
            choose(Some(" usb headset "), &devices()),
            Choice::Saved("USB Headset".into())
        );
    }

    #[test]
    fn missing_device_falls_back_to_the_default() {
        let c = choose(Some("Bluetooth Earbuds"), &devices());
        assert_eq!(
            c,
            Choice::Default {
                missing: Some("Bluetooth Earbuds".into())
            }
        );
        assert_eq!(c.name(), None);
        // Nothing present at all.
        assert_eq!(choose(Some("USB Headset"), &[]).name(), None);
        // No choice saved.
        for saved in [None, Some(""), Some("default")] {
            assert_eq!(choose(saved, &devices()), Choice::Default { missing: None });
        }
    }

    #[test]
    fn menu_steps_through_default_and_devices() {
        let d = devices();
        assert_eq!(step(None, &d, 1).as_deref(), Some("Built-in Microphone"));
        assert_eq!(
            step(Some("Built-in Microphone"), &d, 1).as_deref(),
            Some("USB Headset")
        );
        assert_eq!(
            step(Some("USB Headset"), &d, 1),
            None,
            "wraps to the default"
        );
        assert_eq!(step(None, &d, -1).as_deref(), Some("USB Headset"));
        // A saved device that is not present starts from the default.
        assert_eq!(
            step(Some("Gone"), &d, 1).as_deref(),
            Some("Built-in Microphone")
        );
        assert_eq!(step(None, &[], 1), None);
    }
}
