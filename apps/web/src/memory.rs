//! The browser app's memory on phones (spec 14 Phase 10 B2; the iPhone's
//! restarts of 2026-11-22). iOS gives a tab's WebContent process roughly
//! 1.5 – 2 GB on an iPhone 15 (less on older phones), and the WebGL
//! memory counts against the same memory; a page over it is killed and
//! Safari loads it again. A phone or tablet (the page's `handheld`)
//! therefore rides with fewer world tiles in memory and on the GPU than a
//! desktop: the renderer meshes the 3 × 3 tiles around the rider (desktop:
//! 5 × 5; beyond, the far terrain stands in), and the streaming fetches
//! one ring and keeps two rings of tiles (desktop: 3 / 5) with half the
//! tile budget. `memory=saver` (the page after a page died in a ride,
//! `ocw-gpu.js`) keeps only the 3 × 3 tiles; `memory=normal` is the
//! desktop's. Desktops are unchanged.

use ocw_client::quality::DeviceInfo;
use ocw_worldstore::stream::resident::Residency;

/// How much memory the world may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMode {
    /// The desktop's (as before).
    Normal,
    /// A phone or tablet.
    Phone,
    /// The least: after the page died in a ride.
    Saver,
}

impl MemoryMode {
    /// The page option `memory=normal|phone|saver|auto` (`auto`: `None`,
    /// by the device).
    pub fn parse(s: &str) -> Option<Option<Self>> {
        match s {
            "auto" => Some(None),
            "normal" => Some(Some(Self::Normal)),
            "phone" => Some(Some(Self::Phone)),
            "saver" => Some(Some(Self::Saver)),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Phone => "phone",
            Self::Saver => "saver",
        }
    }

    /// The page's choice, else Phone on a phone or tablet, else Normal.
    pub fn resolve(page: Option<Self>, device: &DeviceInfo) -> Self {
        page.unwrap_or(if device.handheld() {
            Self::Phone
        } else {
            Self::Normal
        })
    }

    /// The ring of near tiles the renderer meshes at most (`None`: the
    /// render distance decides, up to 5 × 5).
    pub fn ring_cap(self) -> Option<i32> {
        match self {
            Self::Normal => None,
            Self::Phone | Self::Saver => Some(1),
        }
    }

    /// The streaming's policy: the page's, made smaller on phones (never
    /// larger than the page asked for).
    pub fn residency(self, page: Residency) -> Residency {
        let (fetch, keep, budget) = match self {
            Self::Normal => return page,
            Self::Phone => (2, 3, 48 << 20),
            Self::Saver => (1, 2, 32 << 20),
        };
        Residency {
            fetch_radius: page.fetch_radius.min(fetch),
            keep_radius: page.keep_radius.min(keep).max(page.fetch_radius.min(fetch)),
            budget_bytes: page.budget_bytes.min(budget),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(json: &str) -> DeviceInfo {
        DeviceInfo::from_json(json)
    }

    #[test]
    fn phones_take_less_and_desktops_stay() {
        let iphone = device(r#"{"mobile": true, "coarse_pointer": true, "max_touch_points": 5}"#);
        let ipad =
            device(r#"{"coarse_pointer": true, "max_touch_points": 5, "ipad_as_mac": true}"#);
        let pc =
            device(r#"{"mobile": false, "coarse_pointer": false, "cores": 12, "memory_gb": 8}"#);
        assert_eq!(MemoryMode::resolve(None, &iphone), MemoryMode::Phone);
        assert_eq!(MemoryMode::resolve(None, &ipad), MemoryMode::Phone);
        assert_eq!(MemoryMode::resolve(None, &pc), MemoryMode::Normal);
        assert_eq!(
            MemoryMode::resolve(None, &DeviceInfo::default()),
            MemoryMode::Normal
        );
        // The page's choice wins.
        assert_eq!(
            MemoryMode::resolve(Some(MemoryMode::Normal), &iphone),
            MemoryMode::Normal
        );
        assert_eq!(
            MemoryMode::resolve(Some(MemoryMode::Saver), &pc),
            MemoryMode::Saver
        );
        // A desktop: as before (5 × 5 meshed, fetch 3, keep 5, 96 MB).
        let page = Residency::default();
        assert_eq!(MemoryMode::Normal.residency(page), page);
        assert_eq!(MemoryMode::Normal.ring_cap(), None);
        // A phone: 3 × 3 meshed, fetch 2, keep 3, 48 MB.
        let phone = MemoryMode::Phone.residency(page);
        assert_eq!(
            (phone.fetch_radius, phone.keep_radius, phone.budget_bytes),
            (2, 3, 48 << 20)
        );
        assert_eq!(MemoryMode::Phone.ring_cap(), Some(1));
        let saver = MemoryMode::Saver.residency(page);
        assert_eq!(
            (saver.fetch_radius, saver.keep_radius, saver.budget_bytes),
            (1, 2, 32 << 20)
        );
        // Never more than the page asked for (`tiles=1&keep=2&memory-mb=16`).
        let small = Residency {
            fetch_radius: 1,
            keep_radius: 2,
            budget_bytes: 16 << 20,
        };
        assert_eq!(MemoryMode::Phone.residency(small), small);
    }

    #[test]
    fn the_page_option_parses() {
        assert_eq!(MemoryMode::parse("auto"), Some(None));
        assert_eq!(MemoryMode::parse("saver"), Some(Some(MemoryMode::Saver)));
        assert_eq!(MemoryMode::parse("phone"), Some(Some(MemoryMode::Phone)));
        assert_eq!(MemoryMode::parse("normal"), Some(Some(MemoryMode::Normal)));
        assert_eq!(MemoryMode::parse("lots"), None);
        for m in [MemoryMode::Normal, MemoryMode::Phone, MemoryMode::Saver] {
            assert_eq!(MemoryMode::parse(m.name()), Some(Some(m)));
        }
    }
}
