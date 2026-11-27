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
//!
//! In an immersive WebXR session on the **Oculus Quest 1** (spec §14 Phase
//! 14, 14I W3) `auto` is the **`quest1`** level ([`resolve_xr`]), with the
//! XR layer's render scale and fixed foveation of [`XrLayer`]; another
//! headset keeps the rules above (the WebXR build is WebGL 2: Low). A PC
//! headset through the desktop app's OpenXR (spec §14 Phase 11, `--vr`)
//! makes `auto` the **`vr`** level.

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
    pub const CHOICES: &'static [&'static str] = &["auto", "low", "medium", "high", "quest1", "vr"];

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
    /// iOS or iPadOS: every browser there is WebKit ([`is_ios_webkit`] of
    /// the page's user agent, platform and touch points; spec 14 Phase 10,
    /// 10G – the shadows are off there on WebGL 2).
    pub ios: bool,
}

/// iOS or iPadOS, whatever the browser (Safari, Firefox, Chrome, Edge –
/// all WebKit there): `iPhone`, `iPad` or `iPod` in the user agent, or an
/// iPad asking for desktop pages (a `Macintosh` user agent or the
/// `MacIntel` platform with more than one touch point; a Mac has none).
pub fn is_ios_webkit(user_agent: &str, platform: &str, max_touch_points: u32) -> bool {
    ["iPhone", "iPad", "iPod"]
        .iter()
        .any(|d| user_agent.contains(d))
        || ((user_agent.contains("Macintosh") || platform == "MacIntel") && max_touch_points > 1)
}

impl DeviceInfo {
    /// From the page's JSON (`{"mobile": false, "coarse_pointer": false,
    /// "cores": 12, "memory_gb": 8, …}`); missing or broken fields are
    /// "not known".
    pub fn from_json(text: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let touch = v["max_touch_points"]
            .as_u64()
            .map_or(0, |n| n.min(1024) as u32);
        Self {
            ios: is_ios_webkit(
                v["user_agent"].as_str().unwrap_or(""),
                v["platform"].as_str().unwrap_or(""),
                touch,
            ),
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

/// The headset of an immersive session, as far as the quality cares
/// (14I W3; `ocw_xr::headset::from_user_agent` finds it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrHeadset {
    /// The Oculus Quest (first generation): Snapdragon 835, 72 Hz.
    Quest1,
    /// A PC headset through OpenXR in the desktop app (spec §14 Phase 11,
    /// `--vr`): the PC's GPU draws both eyes at the headset's rate.
    Pc,
    /// Any other (the later Quests are postponed; a PC headset in a
    /// browser).
    Other,
}

/// The level in an immersive session (`xr`: its headset; `None`: the flat
/// view): `auto` on a Quest 1 is [`QualityLevel::Quest1`], on a PC headset
/// (the desktop's OpenXR) [`QualityLevel::Vr`], everything else as
/// [`resolve`] (a level chosen by name always wins).
pub fn resolve_xr(
    setting: QualitySetting,
    api: GpuApi,
    device: &DeviceInfo,
    xr: Option<XrHeadset>,
) -> (QualityLevel, &'static str) {
    match (setting, xr) {
        (QualitySetting::Auto, Some(XrHeadset::Quest1)) => {
            (QualityLevel::Quest1, "auto: Quest 1 in VR")
        }
        (QualitySetting::Auto, Some(XrHeadset::Pc)) => (QualityLevel::Vr, "auto: VR headset"),
        _ => resolve(setting, api, device),
    }
}

/// The XR layer's options for a level (14I W3): the render scale – of the
/// eyes in the canvas and of the layer's framebuffer
/// (`framebufferScaleFactor`, 1.0 = the browser's recommended size) – and
/// the fixed foveation (`XRWebGLLayer.fixedFoveation`, 0 … 1; `None`: the
/// browser's own). The page's `xrscale=` / `xrfoveation=` still win.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XrLayer {
    pub scale: f32,
    pub foveation: Option<f32>,
}

/// The `quest1` level's render scale: the Quest 1's standalone scale
/// (`ocw_xr::headset::QUEST_1.render_scale_standalone`, 0.8 of the
/// recommended 1216 × 1344 per eye: about 973 × 1075).
pub const QUEST1_RENDER_SCALE: f32 = 0.8;
/// … and its fixed foveation: the highest (Meta's level "high"; the Quest
/// 1's lenses blur the edges anyway).
pub const QUEST1_FOVEATION: f32 = 1.0;

impl XrLayer {
    pub fn of(level: QualityLevel) -> Self {
        match level {
            QualityLevel::Quest1 => Self {
                scale: QUEST1_RENDER_SCALE,
                foveation: Some(QUEST1_FOVEATION),
            },
            _ => Self {
                scale: 1.0,
                foveation: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use QualityLevel::*;

    /// 14I W3: `auto` in a session on a Quest 1 is `quest1` with its
    /// layer options; another headset or the flat view keeps the rules; a
    /// level by name wins in the headset too.
    #[test]
    fn auto_on_a_quest_1_in_vr_is_the_quest1_level() {
        let quest = DeviceInfo::from_json(r#"{"mobile": false, "cores": 8, "memory_gb": 4}"#);
        let auto = QualitySetting::Auto;
        assert_eq!(
            resolve_xr(auto, GpuApi::WebGl2, &quest, Some(XrHeadset::Quest1)),
            (Quest1, "auto: Quest 1 in VR")
        );
        assert_eq!(
            resolve_xr(auto, GpuApi::WebGl2, &quest, Some(XrHeadset::Other)),
            (Low, "auto: WebGL 2")
        );
        assert_eq!(
            resolve_xr(auto, GpuApi::WebGl2, &quest, None),
            resolve(auto, GpuApi::WebGl2, &quest)
        );
        assert_eq!(
            resolve_xr(
                QualitySetting::Fixed(Medium),
                GpuApi::WebGl2,
                &quest,
                Some(XrHeadset::Quest1)
            ),
            (Medium, "chosen")
        );
        assert_eq!(
            QualitySetting::parse("quest1"),
            Some(QualitySetting::Fixed(Quest1))
        );
        let layer = XrLayer::of(Quest1);
        assert_eq!(layer.scale, 0.8);
        assert_eq!(layer.foveation, Some(1.0));
        for l in QualityLevel::ALL {
            assert_eq!(
                XrLayer::of(l),
                XrLayer {
                    scale: 1.0,
                    foveation: None
                }
            );
        }
    }

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
    fn auto_in_vr_is_the_vr_preset() {
        let pc = desktop_browser();
        let auto = QualitySetting::Auto;
        assert_eq!(
            resolve_xr(auto, GpuApi::Native, &pc, Some(XrHeadset::Pc)),
            (Vr, "auto: VR headset")
        );
        assert_eq!(resolve_xr(auto, GpuApi::Native, &pc, None).0, High);
        // Chosen by name: as chosen, also in VR.
        assert_eq!(
            resolve_xr(
                QualitySetting::Fixed(Medium),
                GpuApi::Native,
                &pc,
                Some(XrHeadset::Pc)
            )
            .0,
            Medium
        );
        assert_eq!(QualitySetting::parse("vr"), Some(QualitySetting::Fixed(Vr)));
    }

    /// User agents of the browsers on iOS / iPadOS (all WebKit there; spec
    /// 14 Phase 10, 10G: the shadows are off on them with WebGL 2) and of
    /// those that are not.
    const IPHONE_SAFARI: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Mobile/15E148 Safari/604.1";
    const IPHONE_FIREFOX: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/145.0 Mobile/15E148 Safari/605.1.15";
    const IPHONE_CHROME: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0.6778.73 Mobile/15E148 Safari/604.1";
    const IPAD_SAFARI: &str = "Mozilla/5.0 (iPad; CPU OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Mobile/15E148 Safari/604.1";
    const IPAD_CHROME: &str = "Mozilla/5.0 (iPad; CPU OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0.6778.73 Mobile/15E148 Safari/604.1";
    /// iPadOS asks for desktop pages: a Mac's user agent (also a Mac's).
    const MAC_SAFARI: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Safari/605.1.15";
    const ANDROID_CHROME: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";
    const ANDROID_FIREFOX: &str =
        "Mozilla/5.0 (Android 14; Mobile; rv:133.0) Gecko/133.0 Firefox/133.0";
    const WINDOWS_CHROME: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
    const LINUX_FIREFOX: &str =
        "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0";

    #[test]
    fn ios_webkit_is_every_browser_on_iphone_and_ipad() {
        for ua in [
            IPHONE_SAFARI,
            IPHONE_FIREFOX,
            IPHONE_CHROME,
            IPAD_SAFARI,
            IPAD_CHROME,
        ] {
            assert!(is_ios_webkit(ua, "iPhone", 5), "{ua}");
            // Whatever the platform says.
            assert!(is_ios_webkit(ua, "", 0), "{ua}");
        }
        // An iPad asking for desktop pages: a Mac's agent with touch.
        assert!(is_ios_webkit(MAC_SAFARI, "MacIntel", 5));
        assert!(is_ios_webkit(MAC_SAFARI, "", 5));
        assert!(is_ios_webkit("", "MacIntel", 5));
        // A Mac has no touch points; Android, Windows and Linux are not iOS,
        // touch screens or not.
        assert!(!is_ios_webkit(MAC_SAFARI, "MacIntel", 0));
        assert!(!is_ios_webkit(MAC_SAFARI, "MacIntel", 1));
        for (ua, p) in [
            (ANDROID_CHROME, "Linux armv81"),
            (ANDROID_FIREFOX, "Linux aarch64"),
            (WINDOWS_CHROME, "Win32"),
            (LINUX_FIREFOX, "Linux x86_64"),
        ] {
            assert!(!is_ios_webkit(ua, p, 5), "{ua}");
            assert!(!is_ios_webkit(ua, p, 0), "{ua}");
        }
        // From the page's JSON.
        let info = |json: &str| DeviceInfo::from_json(json);
        let firefox_ios = info(&format!(
            r#"{{"mobile": true, "max_touch_points": 5, "user_agent": "{IPHONE_FIREFOX}", "platform": "iPhone"}}"#
        ));
        assert!(firefox_ios.ios && firefox_ios.handheld());
        let ipad_desktop = info(&format!(
            r#"{{"coarse_pointer": true, "max_touch_points": 5, "ipad_as_mac": true, "user_agent": "{MAC_SAFARI}", "platform": "MacIntel"}}"#
        ));
        assert!(ipad_desktop.ios);
        let mac = info(&format!(
            r#"{{"max_touch_points": 0, "user_agent": "{MAC_SAFARI}", "platform": "MacIntel"}}"#
        ));
        assert!(!mac.ios);
        let android = info(&format!(
            r#"{{"mobile": true, "max_touch_points": 5, "user_agent": "{ANDROID_CHROME}"}}"#
        ));
        assert!(!android.ios && android.handheld());
        assert!(!DeviceInfo::default().ios);
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
