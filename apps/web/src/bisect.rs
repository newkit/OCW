//! The bisect switches in the page's address (spec 14 Phase 10, 10G
//! *Bisecting a phone's crash*): each turns off one part that could make a
//! phone's browser close the page, so a rider can find the one by tapping
//! a few links (the README's "Something doesn't work on my phone").
//!
//! | Switch | Turns off |
//! |---|---|
//! | `nosee=1` | the trees' see-through shader (`see_through.wesl`): Bevy's standard one |
//! | `noarrays=1` | the texture-array shaders of road, ground, houses, gear: the standard ones |
//! | `plain=1` | every material shader of ours (with water and the far terrain) |
//! | `noshadow=1` | the shadow maps |
//! | `nomsaa=1` | multisampling |
//! | `nosky=1` | the cloud dome and the night sky (the simplest sky) |
//! | `noegui=1` | egui (the console's overlay; the commands still run) |
//! | `novoice=1` | voice (LiveKit's SDK is not loaded) |
//! | `noaudio=1` | the game sound (no Web Audio) |
//! | `pipelines=serial` | one render pipeline compiled per frame (`=N`: N per frame, `=all`: all at once) |
//! | `frames=N` | the cameras stop drawing after N frames (`0`: nothing is drawn) |
//! | `ios=safe` | all of the above but `frames` (`pipelines=serial`) |
//! | `shadowmap=N` | the sun's shadow map at most N² (256 – 4096; shadows on) |
//! | `shadowfix=…` | a change to the shadow pass: `hw2x2`, `near` (comma-separated) |
//! | `photos=0` | the own ride photographer's photos (`photos=1` turns them on) |
//! | `photofix=…` | a change to the photo's capture: `small`, `once`, `screen`, `nosave`, `bgra`, `all` (comma-separated) |
//!
//! On iOS / iPadOS WebKit with WebGL 2 the shadows are off by default
//! ([`Bisect::platform_defaults`]; spec 10G: the shadow pass closed the
//! page on an iPhone, iOS 18.7): `noshadow=0` turns them on again,
//! `shadowmap=N` turns them on at that size. The own ride photos are off
//! there too ([`Bisect::photos_off`]; spec 10G: the first photo closed the
//! page on that iPhone): `photos=1` turns them on, `photofix=…` changes how
//! they are captured.
//!
//! `=0` turns a switch off again (`ios=safe&nosee=0`: everything but the
//! trees' shader). On WebGL 2 at most [`WEBGL2_BUDGET`] pipelines compile
//! per frame anyway (it compiles on the page's thread). Unknown values are
//! ignored with a warning, like every option of the page.

use ocw_render::switches::{RenderSwitches, ShadowsOffWhy};

/// Pipelines that start compiling per frame on WebGL 2 without
/// `pipelines=` (spec 10G: the first frames compiled all at once before).
pub const WEBGL2_BUDGET: usize = 4;

/// The address's switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bisect {
    pub render: RenderSwitches,
    /// No voice: the backend says why, the page loads no SDK.
    pub no_voice: bool,
    /// No game sound.
    pub no_audio: bool,
    /// `pipelines=`: `None` the default of the GPU API, `Some(None)` all
    /// at once, `Some(Some(n))` n per frame.
    pub pipelines: Option<Option<usize>>,
    /// The address named `noshadow` (or `ios=safe`): the platform's
    /// default does not change it.
    pub shadow_named: bool,
    /// `photos=`: the own ride photos on (`Some(true)`) or off
    /// (`Some(false)`, also `ios=safe`); `None` the platform's default.
    pub photos: Option<bool>,
}

/// The keys [`Bisect::set`] reads (index.html reads `novoice` itself).
pub const KEYS: &[&str] = &[
    "nosee",
    "noarrays",
    "plain",
    "noshadow",
    "nomsaa",
    "nosky",
    "noegui",
    "novoice",
    "noaudio",
    "pipelines",
    "frames",
    "ios",
    "shadowmap",
    "shadowfix",
    "photos",
    "photofix",
];

fn flag(value: &str) -> Option<bool> {
    match value.trim() {
        "" | "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

impl Bisect {
    /// Reads the switches of `pairs` (the address's keys and values, in
    /// order; `ios=safe` first, so a later `nosee=0` wins); the warnings
    /// for values it does not know.
    pub fn from_pairs(pairs: &[(String, String)], warnings: &mut Vec<String>) -> Self {
        let mut b = Self::default();
        let ios = pairs.iter().filter(|(k, _)| k == "ios");
        let rest = pairs.iter().filter(|(k, _)| k != "ios");
        for (k, v) in ios.chain(rest) {
            if KEYS.contains(&k.as_str()) && !b.set(k, v) {
                warnings.push(format!("{k}={v}: ignored"));
            }
        }
        b
    }

    /// One switch; `false`: a value it does not know (nothing changed).
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        if key == "ios" {
            if value.trim() != "safe" {
                return false;
            }
            let frames = self.render.frames;
            *self = Self {
                render: RenderSwitches {
                    no_see: true,
                    no_arrays: true,
                    plain: true,
                    no_shadow: true,
                    no_msaa: true,
                    no_sky: true,
                    no_egui: true,
                    frames,
                    ..self.render
                },
                no_voice: true,
                no_audio: true,
                pipelines: Some(Some(1)),
                shadow_named: true,
                photos: Some(false),
            };
            return true;
        }
        if key == "shadowmap" {
            return match value.trim().parse::<usize>() {
                Ok(n) if ocw_render::switches::SHADOW_MAP_SIZES.contains(&n) => {
                    self.render.shadow_map = Some(n);
                    true
                }
                _ => false,
            };
        }
        if key == "photos" {
            return match flag(value) {
                Some(on) => {
                    self.photos = Some(on);
                    true
                }
                None => false,
            };
        }
        if key == "photofix" {
            return match ocw_render::switches::PhotoFix::parse(value) {
                Some(f) => {
                    self.render.photo_fix = f;
                    true
                }
                None => false,
            };
        }
        if key == "shadowfix" {
            return match ocw_render::switches::ShadowFix::parse(value) {
                Some(f) => {
                    self.render.shadow_fix = f;
                    true
                }
                None => false,
            };
        }
        let r = &mut self.render;
        let slot = match key {
            "nosee" => &mut r.no_see,
            "noarrays" => &mut r.no_arrays,
            "plain" => &mut r.plain,
            "noshadow" => &mut r.no_shadow,
            "nomsaa" => &mut r.no_msaa,
            "nosky" => &mut r.no_sky,
            "noegui" => &mut r.no_egui,
            "novoice" => &mut self.no_voice,
            "noaudio" => &mut self.no_audio,
            "pipelines" => {
                self.pipelines = match value.trim() {
                    "serial" | "1" => Some(Some(1)),
                    "all" | "0" => Some(None),
                    n => match n.parse::<usize>() {
                        Ok(n @ 2..=64) => Some(Some(n)),
                        _ => return false,
                    },
                };
                return true;
            }
            "frames" => {
                return match value.trim().parse::<u32>() {
                    Ok(n) if n <= 1_000_000 => {
                        r.frames = Some(n);
                        true
                    }
                    _ => false,
                };
            }
            _ => return false,
        };
        match flag(value) {
            Some(on) => {
                *slot = on;
                if key == "noshadow" {
                    self.shadow_named = true;
                    self.render.shadows_off_why = ShadowsOffWhy::Switch;
                }
                true
            }
            None => false,
        }
    }

    /// The defaults of the platform under the address's switches: on iOS /
    /// iPadOS WebKit with WebGL 2 the shadows are off (spec 10G: the shadow
    /// pass closed the page on an iPhone, iOS 18.7, 2026-11-27) unless the
    /// address names `noshadow` or asks for a `shadowmap=N`. `true`: the
    /// default turned them off.
    pub fn platform_defaults(&mut self, ios_webkit: bool, webgl2: bool) -> bool {
        if ios_webkit && webgl2 && !self.shadow_named && self.render.shadow_map.is_none() {
            self.render.no_shadow = true;
            self.render.shadows_off_why = ShadowsOffWhy::IosWebKit;
            return true;
        }
        false
    }

    /// Whether the own ride photos are off, and why (`None`: on): on iOS /
    /// iPadOS WebKit with WebGL 2 by default (spec 10G: taking the first
    /// photo closed the page on an iPhone, iOS 18.7, 2026-11-27) unless
    /// the address says `photos=1`; anywhere with `photos=0` (or
    /// `ios=safe`).
    pub fn photos_off(
        &self,
        ios_webkit: bool,
        webgl2: bool,
    ) -> Option<ocw_ui::photographer::PhotosOff> {
        use ocw_ui::photographer::PhotosOff;
        match self.photos {
            Some(true) => None,
            Some(false) => Some(PhotosOff::Switch),
            None => (ios_webkit && webgl2).then_some(PhotosOff::IosWebKit),
        }
    }

    /// The renderer's switches for this GPU API: the pipelines' budget
    /// ([`WEBGL2_BUDGET`] on WebGL 2 unless `pipelines=` says), the
    /// compile log always (the page log's early lines).
    pub fn render_switches(&self, webgl2: bool) -> RenderSwitches {
        let mut r = self.render;
        r.compile_budget = match self.pipelines {
            Some(p) => p,
            None => webgl2.then_some(WEBGL2_BUDGET),
        };
        r.log_pipelines = true;
        r
    }

    /// What is turned off, for the log (`None`: nothing).
    pub fn summary(&self) -> Option<String> {
        let mut names = self.render.names();
        if self.no_voice {
            names.push("novoice".into());
        }
        if self.no_audio {
            names.push("noaudio".into());
        }
        match self.pipelines {
            Some(Some(n)) => names.push(format!("pipelines={n} per frame")),
            Some(None) => names.push("pipelines=all".into()),
            None => {}
        }
        match self.photos {
            Some(true) => names.push("photos=1".into()),
            Some(false) => names.push("photos=0".into()),
            None => {}
        }
        (!names.is_empty()).then(|| names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(q: &str) -> (Bisect, Vec<String>) {
        let pairs: Vec<(String, String)> = q
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (k.to_string(), v.to_string())
            })
            .collect();
        let mut w = Vec::new();
        (Bisect::from_pairs(&pairs, &mut w), w)
    }

    #[test]
    fn nothing_without_switches() {
        let (b, w) = parse("route=hills&gpu=webgl2&quality=low&log=1");
        assert_eq!(b, Bisect::default());
        assert!(w.is_empty());
        assert!(b.summary().is_none());
        // WebGL 2 paces the pipelines anyway; WebGPU does not.
        assert_eq!(b.render_switches(true).compile_budget, Some(WEBGL2_BUDGET));
        assert_eq!(b.render_switches(false).compile_budget, None);
        assert!(b.render_switches(false).log_pipelines);
    }

    #[test]
    fn each_switch_turns_off_one_thing() {
        let one = |q: &str| parse(q).0;
        assert!(one("nosee=1").render.no_see);
        assert!(one("noarrays=1").render.no_arrays);
        assert!(one("plain=1").render.plain);
        assert!(one("noshadow=1").render.no_shadow);
        assert!(one("nomsaa=1").render.no_msaa);
        assert!(one("nosky=1").render.no_sky);
        assert!(one("noegui=1").render.no_egui);
        assert!(one("novoice=1").no_voice);
        assert!(one("noaudio=1").no_audio);
        assert_eq!(one("frames=0").render.frames, Some(0));
        assert_eq!(one("frames=60").render.frames, Some(60));
        // A bare key is on too.
        assert!(one("noshadow").render.no_shadow);
        // Only that one.
        let b = one("noshadow=1");
        assert_eq!(
            b,
            Bisect {
                render: RenderSwitches {
                    no_shadow: true,
                    ..Default::default()
                },
                shadow_named: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn shadowmap_and_shadowfix() {
        let (b, w) = parse("shadowmap=512&shadowfix=hw2x2,near");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(b.render.shadow_map, Some(512));
        assert!(b.render.shadow_fix.hw2x2 && b.render.shadow_fix.near);
        assert!(!b.render.no_shadow);
        let s = b.summary().unwrap();
        assert!(
            s.contains("shadowmap=512") && s.contains("shadowfix=hw2x2,near"),
            "{s}"
        );
        for n in [256, 512, 1024, 2048, 4096] {
            assert_eq!(
                parse(&format!("shadowmap={n}")).0.render.shadow_map,
                Some(n)
            );
        }
        let (b, w) = parse("shadowmap=1000&shadowmap=8192&shadowfix=depth16");
        assert_eq!(b, Bisect::default());
        assert_eq!(w.len(), 3, "{w:?}");
    }

    /// iOS WebKit on WebGL 2: shadows off by default; `noshadow=0` and
    /// `shadowmap=N` turn them on; Android, desktops and WebGPU keep them.
    #[test]
    fn ios_webkit_on_webgl2_rides_without_shadows_by_default() {
        let with = |q: &str, ios: bool, webgl2: bool| {
            let mut b = parse(q).0;
            let off = b.platform_defaults(ios, webgl2);
            (b, off)
        };
        let (b, off) = with("route=hills", true, true);
        assert!(off && b.render.no_shadow);
        assert_eq!(b.render.shadows_off_why, ShadowsOffWhy::IosWebKit);
        // The other suspects stay on.
        assert!(!b.render.no_see && !b.render.no_arrays && !b.render.no_sky);
        // noshadow=0: on again; noshadow=1: off by the switch.
        let (b, off) = with("noshadow=0", true, true);
        assert!(!off && !b.render.no_shadow);
        let (b, off) = with("noshadow=1", true, true);
        assert!(!off && b.render.no_shadow);
        assert_eq!(b.render.shadows_off_why, ShadowsOffWhy::Switch);
        // shadowmap=N: on at that size.
        let (b, off) = with("shadowmap=1024", true, true);
        assert!(!off && !b.render.no_shadow);
        assert_eq!(b.render.shadow_map, Some(1024));
        // …unless noshadow=1 says off too.
        let (b, _) = with("shadowmap=512&noshadow=1", true, true);
        assert!(b.render.no_shadow);
        // ios=safe and a later noshadow=0.
        let (b, _) = with("ios=safe&noshadow=0", true, true);
        assert!(!b.render.no_shadow && b.render.no_see);
        // Not iOS, or WebGPU (only by gpu=webgpu on iOS): unchanged.
        for (ios, webgl2) in [(false, true), (false, false), (true, false)] {
            let (b, off) = with("route=hills", ios, webgl2);
            assert!(!off && !b.render.no_shadow, "{ios} {webgl2}");
        }
    }

    /// iOS WebKit on WebGL 2: the own ride photos off by default (10G);
    /// `photos=1` turns them on, `photos=0` off anywhere, `ios=safe` too;
    /// `photofix=…` reaches the renderer; Android, desktops and WebGPU
    /// keep them.
    #[test]
    fn ios_webkit_on_webgl2_takes_no_ride_photos_by_default() {
        use ocw_ui::photographer::PhotosOff;
        let off = |q: &str, ios: bool, webgl2: bool| parse(q).0.photos_off(ios, webgl2);
        assert_eq!(off("route=hills", true, true), Some(PhotosOff::IosWebKit));
        for (ios, webgl2) in [(false, true), (false, false), (true, false)] {
            assert_eq!(off("route=hills", ios, webgl2), None, "{ios} {webgl2}");
        }
        assert_eq!(off("photos=1", true, true), None);
        assert_eq!(off("photos=on", true, true), None);
        assert_eq!(off("photos=0", false, false), Some(PhotosOff::Switch));
        assert_eq!(off("ios=safe", false, true), Some(PhotosOff::Switch));
        assert_eq!(off("ios=safe&photos=1", true, true), None);
        // The shadows' default does not touch the photos and back.
        let mut b = parse("photos=1").0;
        assert!(b.platform_defaults(true, true));
        assert_eq!(b.photos_off(true, true), None);
        // photofix=…
        let (b, w) = parse("photos=1&photofix=small,once");
        assert!(w.is_empty(), "{w:?}");
        assert!(b.render.photo_fix.small && b.render.photo_fix.once);
        let s = b.summary().unwrap();
        assert!(
            s.contains("photofix=small,once") && s.contains("photos=1"),
            "{s}"
        );
        assert!(parse("photofix=screen").0.render.photo_fix.screen);
        assert!(parse("photofix=all").0.render.photo_fix.nosave);
        let (b, w) = parse("photofix=tiny&photos=maybe");
        assert_eq!(b, Bisect::default());
        assert_eq!(w.len(), 2, "{w:?}");
    }

    #[test]
    fn the_pipelines_pace() {
        let budget = |q: &str| parse(q).0.render_switches(true).compile_budget;
        assert_eq!(budget("pipelines=serial"), Some(1));
        assert_eq!(budget("pipelines=1"), Some(1));
        assert_eq!(budget("pipelines=8"), Some(8));
        assert_eq!(budget("pipelines=all"), None);
        assert_eq!(
            parse("pipelines=serial")
                .0
                .render_switches(false)
                .compile_budget,
            Some(1)
        );
    }

    #[test]
    fn ios_safe_turns_everything_off_and_a_later_zero_turns_one_on() {
        let (b, w) = parse("ios=safe");
        assert!(w.is_empty());
        let r = b.render;
        assert!(r.no_see && r.no_arrays && r.plain && r.no_shadow && r.no_msaa);
        assert!(r.no_sky && r.no_egui && b.no_voice && b.no_audio);
        assert_eq!(b.render_switches(true).compile_budget, Some(1));
        assert_eq!(r.frames, None);
        // The order in the address does not matter: ios first.
        let (b, _) = parse("nosee=0&ios=safe&frames=30");
        assert!(!b.render.no_see && b.render.no_shadow);
        assert_eq!(b.render.frames, Some(30));
        assert!(b.summary().unwrap().contains("noshadow"));
    }

    #[test]
    fn bad_values_are_ignored_with_a_warning() {
        let (b, w) = parse("nosee=maybe&pipelines=fast&frames=-1&ios=yes&pipelines=999");
        assert_eq!(b, Bisect::default());
        assert_eq!(w.len(), 5, "{w:?}");
        assert!(w[0].starts_with("ios=yes"), "{w:?}");
    }
}
