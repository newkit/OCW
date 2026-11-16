//! Which quality preset runs (spec §14 Phase 10 B7): the setting
//! `display.quality` (`auto`, `low`, `medium`, `high`) and, for `auto`,
//! the GPU API and the device. Bevy-free; the presets' numbers are the
//! renderer's (`ocw_render::quality`).
//!
//! `auto`:
//! - the desktop app (native GPU APIs): **High** – the look it always had;
//! - WebGL 2 (the browser's fallback build): **Low** – one shadow
//!   cascade is all WebGL 2 has, and it runs on the weakest GPUs;
//! - a phone or tablet (the browser says mobile, or a touch screen as
//!   the only pointer): **Low**;
//! - WebGPU on a small device (≤ 4 cores or ≤ 4 GB as the browser reports
//!   them): **Medium**;
//! - WebGPU otherwise: **High**.
//!
//! A level chosen by name always wins (also High on WebGL 2: the renderer
//! then draws what WebGL 2 can – one cascade).

use ocw_render::quality::QualityLevel;

/// The setting's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QualitySetting {
    /// By GPU API and device ([`resolve`]).
    #[default]
    Auto,
    Fixed(QualityLevel),
}

impl QualitySetting {
    /// The setting's words (`display.quality`'s choices).
    pub const CHOICES: &'static [&'static str] = &["auto", "low", "medium", "high"];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "auto" => Some(Self::Auto),
            other => QualityLevel::parse(other).map(Self::Fixed),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fixed(l) => l.name(),
        }
    }
}

/// The GPU API the app renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuApi {
    /// The desktop app: Vulkan, Metal, DirectX 12 (or GL) through wgpu.
    #[default]
    Native,
    /// The browser's WebGPU build.
    WebGpu,
    /// The browser's WebGL 2 build (the fallback).
    WebGl2,
}

impl GpuApi {
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::WebGpu => "WebGPU",
            Self::WebGl2 => "WebGL2",
        }
    }
}

/// What the browser tells about the device (the page's
/// `window.ocwDeviceInfo`; the desktop: the default, nothing known).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    /// The browser says it is a mobile device (`navigator.userAgentData
    /// .mobile`, else the user agent).
    pub mobile: bool,
    /// The main pointer is a finger (`(pointer: coarse)`).
    pub coarse_pointer: bool,
    /// Logical cores (`navigator.hardwareConcurrency`; 0: not known).
    pub cores: u32,
    /// Memory in GB as the browser rounds it (`navigator.deviceMemory`,
    /// at most 8 in Chromium; 0: not known).
    pub memory_gb: f32,
    /// `navigator.maxTouchPoints` (touch steering's hints, spec §3.2.3).
    pub max_touch_points: u32,
    /// An iPad that reports a Mac (`MacIntel` with `maxTouchPoints > 1`).
    pub ipad_as_mac: bool,
}

impl DeviceInfo {
    /// From the page's JSON (`{"mobile": false, "coarse_pointer": false,
    /// "cores": 12, "memory_gb": 8, …}`); missing or broken fields are
    /// "not known".
    pub fn from_json(text: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        Self {
            mobile: v["mobile"].as_bool().unwrap_or(false),
            coarse_pointer: v["coarse_pointer"].as_bool().unwrap_or(false),
            cores: v["cores"].as_u64().map_or(0, |c| c.min(1024) as u32),
            memory_gb: v["memory_gb"].as_f64().map_or(0.0, |m| m as f32),
            max_touch_points: v["max_touch_points"]
                .as_u64()
                .map_or(0, |n| n.min(1024) as u32),
            ipad_as_mac: v["ipad_as_mac"].as_bool().unwrap_or(false),
        }
    }

    /// What touch steering's hints decide by (spec §3.2.3 *Detection*).
    pub fn touch_facts(&self) -> crate::input::touch::TouchFacts {
        crate::input::touch::TouchFacts {
            max_touch_points: self.max_touch_points,
            coarse_pointer: self.coarse_pointer,
            ipad_as_mac: self.ipad_as_mac,
        }
    }

    /// A phone or tablet.
    pub fn handheld(&self) -> bool {
        self.mobile || self.coarse_pointer
    }

    /// A small device: few cores or little memory (as far as known).
    pub fn small(&self) -> bool {
        (self.cores > 0 && self.cores <= 4) || (self.memory_gb > 0.0 && self.memory_gb <= 4.0)
    }
}

/// The level that runs, and why (for the console and the menu).
pub fn resolve(
    setting: QualitySetting,
    api: GpuApi,
    device: &DeviceInfo,
) -> (QualityLevel, &'static str) {
    match setting {
        QualitySetting::Fixed(level) => (level, "chosen"),
        QualitySetting::Auto => match api {
            GpuApi::Native => (QualityLevel::High, "auto: desktop"),
            GpuApi::WebGl2 => (QualityLevel::Low, "auto: WebGL 2"),
            GpuApi::WebGpu if device.handheld() => (QualityLevel::Low, "auto: phone or tablet"),
            GpuApi::WebGpu if device.small() => (QualityLevel::Medium, "auto: small device"),
            GpuApi::WebGpu => (QualityLevel::High, "auto: WebGPU"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use QualityLevel::*;

    fn desktop_browser() -> DeviceInfo {
        DeviceInfo::from_json(
            r#"{"mobile": false, "coarse_pointer": false, "cores": 12, "memory_gb": 8}"#,
        )
    }

    #[test]
    fn auto_follows_the_gpu_api_and_the_device() {
        let pc = desktop_browser();
        let auto = QualitySetting::Auto;
        // The desktop app keeps its look, whatever the device says.
        assert_eq!(
            resolve(auto, GpuApi::Native, &DeviceInfo::default()).0,
            High
        );
        assert_eq!(resolve(auto, GpuApi::Native, &pc).0, High);
        // The browser: WebGPU on a PC High, WebGL 2 Low.
        assert_eq!(resolve(auto, GpuApi::WebGpu, &pc), (High, "auto: WebGPU"));
        assert_eq!(resolve(auto, GpuApi::WebGl2, &pc), (Low, "auto: WebGL 2"));
        // A phone: Low on either API.
        let phone = DeviceInfo::from_json(
            r#"{"mobile": true, "coarse_pointer": true, "cores": 8, "memory_gb": 4, "screen_px": [1170, 2532]}"#,
        );
        assert_eq!(resolve(auto, GpuApi::WebGpu, &phone).0, Low);
        assert_eq!(resolve(auto, GpuApi::WebGl2, &phone).0, Low);
        // An iPad says it is a Mac, but its pointer is a finger.
        let ipad =
            DeviceInfo::from_json(r#"{"mobile": false, "coarse_pointer": true, "cores": 8}"#);
        assert_eq!(resolve(auto, GpuApi::WebGpu, &ipad).0, Low);
        // A small laptop on WebGPU: Medium (cores or memory).
        let small = DeviceInfo::from_json(r#"{"cores": 4, "memory_gb": 8}"#);
        assert_eq!(
            resolve(auto, GpuApi::WebGpu, &small),
            (Medium, "auto: small device")
        );
        let little_memory = DeviceInfo::from_json(r#"{"cores": 8, "memory_gb": 2}"#);
        assert_eq!(resolve(auto, GpuApi::WebGpu, &little_memory).0, Medium);
        // Nothing known (a browser that tells nothing): High on WebGPU.
        assert_eq!(
            resolve(auto, GpuApi::WebGpu, &DeviceInfo::default()).0,
            High
        );
    }

    /// Touch steering's hints (spec §3.2.3): the page's touch facts.
    #[test]
    fn the_touch_facts_come_from_the_page() {
        let phone = DeviceInfo::from_json(
            r#"{"mobile": true, "coarse_pointer": true, "max_touch_points": 5}"#,
        );
        assert!(phone.touch_facts().touch_first());
        let ipad = DeviceInfo::from_json(
            r#"{"mobile": false, "coarse_pointer": false, "max_touch_points": 5, "ipad_as_mac": true}"#,
        );
        assert!(ipad.touch_facts().touch_first());
        let laptop = DeviceInfo::from_json(r#"{"coarse_pointer": false, "max_touch_points": 10}"#);
        assert!(!laptop.touch_facts().touch_first());
        assert!(!DeviceInfo::default().touch_facts().touch_first());
    }

    #[test]
    fn a_level_by_name_always_wins() {
        let phone = DeviceInfo {
            mobile: true,
            ..Default::default()
        };
        for api in [GpuApi::Native, GpuApi::WebGpu, GpuApi::WebGl2] {
            for level in QualityLevel::ALL {
                assert_eq!(
                    resolve(QualitySetting::Fixed(level), api, &phone),
                    (level, "chosen")
                );
            }
        }
    }

    #[test]
    fn setting_words() {
        for w in QualitySetting::CHOICES {
            assert_eq!(QualitySetting::parse(w).unwrap().name(), *w);
        }
        assert_eq!(QualitySetting::parse("ultra"), None);
        assert_eq!(QualitySetting::default(), QualitySetting::Auto);
        // The registry's choices are these words, `auto` the default.
        let d = crate::settings::def("display.quality").unwrap();
        assert_eq!(
            d.kind,
            crate::settings::Kind::Choice(QualitySetting::CHOICES)
        );
        assert_eq!(d.default, "auto");
    }

    #[test]
    fn device_info_from_the_page() {
        assert_eq!(desktop_browser().cores, 12);
        assert!(!desktop_browser().handheld() && !desktop_browser().small());
        // Broken or missing: nothing known.
        assert_eq!(DeviceInfo::from_json("not json"), DeviceInfo::default());
        assert_eq!(DeviceInfo::from_json(r#"{"cores": "many"}"#).cores, 0);
        assert!(!DeviceInfo::default().small());
    }
}
