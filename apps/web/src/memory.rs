//! The browser app's memory on phones (spec 14 Phase 10 B2, B7; the
//! iPhone's restarts of 2026-11-22 and 2026-11-23). iOS kills a tab's
//! WebContent process well below the phone's RAM – on an iPhone 15 about
//! 400 – 500 MB of wasm memory, GPU buffers and textures (WebGL counts in
//! the same budget), the compiled module and the browser's own share
//! together – and Safari loads it again. A phone or tablet (the page's
//! `handheld`) therefore rides in the **Phone** mode: the renderer meshes
//! the 3 × 3 tiles around the rider (desktop: 5 × 5; beyond, the far
//! terrain stands in) one tile at a time ([`MemoryMode::tile_pace`]), the
//! streaming fetches two rings and keeps three (desktop: 3 / 5) with half
//! the tile budget, the sun's shadow map is 1024² and the unused point
//! light shadow cube tiny, and a guard ([`MemoryGuard`]) steps down near
//! the budget. The page draws at pixel ratio 1.5 there (`ocw-gpu.js`
//! `pixelRatioCap`: WebKit cannot take the preset's cap). After a page
//! died in a ride it rides on in `memory=saver`, after a second death in
//! `memory=minimal`, then it stops (`ocw-gpu.js` `nextMemoryStep`);
//! `memory=normal` is the desktop's. iPhones and iPads (iOS WebKit) start
//! in the saver mode (spec 14 Phase 10, 10G, v6.77). Desktops are
//! unchanged.

use bevy::prelude::*;
use ocw_client::quality::DeviceInfo;
use ocw_worldstore::stream::resident::Residency;

/// The memory guard (v5.42, the iPhone's memory): what a phone's tab may
/// use – the wasm memory (it never shrinks) and what the app holds on the
/// GPU – before iOS kills it (an iPhone 15 kills a tab well below its RAM;
/// ~450 MB is taken as safe). Near the budget the guard steps down instead
/// of letting the tab die: first the world keeps only the tiles around
/// the rider (the minimal mode's residency) and the smallest shadow map,
/// then it fetches no ring beyond the rider's own tiles. Each step once,
/// logged and printed to the console; never back up within a ride.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryGuard {
    /// The budget (bytes).
    pub budget: u64,
    /// Steps taken (0 – 2).
    pub stage: u8,
}

/// What the guard does at a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStep {
    /// 85 % of the budget: the minimal mode's residency and shadow map.
    Shrink,
    /// 95 %: no tiles beyond the rider's ring are fetched or kept.
    Least,
}

impl MemoryGuard {
    /// Phones and their saver modes get a guard; desktops none.
    pub fn for_mode(mode: MemoryMode) -> Option<Self> {
        let mb = match mode {
            MemoryMode::Normal => return None,
            MemoryMode::Phone => 450,
            MemoryMode::Saver => 400,
            MemoryMode::Minimal => 350,
        };
        Some(Self {
            budget: mb << 20,
            stage: 0,
        })
    }

    /// The step for `used` bytes (wasm memory + GPU), if one is due now.
    pub fn check(&mut self, used: u64) -> Option<GuardStep> {
        let share = used as f64 / self.budget.max(1) as f64;
        let step = match self.stage {
            0 | 1 if share >= 0.95 => GuardStep::Least,
            0 if share >= 0.85 => GuardStep::Shrink,
            _ => return None,
        };
        self.stage = match step {
            GuardStep::Shrink => 1,
            GuardStep::Least => 2,
        };
        Some(step)
    }

    /// The residency after `step` (never more than before).
    pub fn residency(step: GuardStep, now: Residency) -> Residency {
        let (fetch, keep, budget) = match step {
            GuardStep::Shrink => (1, 1, 16 << 20),
            GuardStep::Least => (1, 1, 8 << 20),
        };
        Residency {
            fetch_radius: now.fetch_radius.min(fetch),
            keep_radius: now.keep_radius.min(keep).max(now.fetch_radius.min(fetch)),
            budget_bytes: now.budget_bytes.min(budget),
        }
    }
}

/// How often the guard looks (s).
const GUARD_INTERVAL_S: f64 = 2.0;

/// The guard's system: every [`GUARD_INTERVAL_S`] the wasm memory and the
/// GPU sums against the budget.
pub fn guard_memory(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let Some(mut guard) = world.get_resource::<MemoryGuard>().copied() else {
        return;
    };
    let last = world.get_resource::<GuardClock>().map_or(f64::MIN, |c| c.0);
    if now - last < GUARD_INTERVAL_S {
        return;
    }
    world.insert_resource(GuardClock(now));
    let Some(wasm) = crate::wasm_memory_bytes() else {
        return;
    };
    let g = crate::gpu_memory::GpuMemory::now();
    let used = wasm + g.slab_bytes.max(g.mesh_bytes) + g.image_bytes;
    let Some(step) = guard.check(used) else {
        return;
    };
    world.insert_resource(guard);
    if let Some(mut stream) = world.get_resource_mut::<crate::world::WorldStream>() {
        stream.options.residency = MemoryGuard::residency(step, stream.options.residency);
    }
    world.insert_resource(ocw_render::quality::ShadowMapCap(Some(512)));
    if step == GuardStep::Least {
        // New tiles with a third of the trees.
        world.insert_resource(ocw_render::package::PlantThinning(3));
    }
    let line = format!(
        "memory guard: {:.0} of {:.0} MB used – {}",
        used as f64 / 1048576.0,
        guard.budget as f64 / 1048576.0,
        match step {
            GuardStep::Shrink => "fewer world tiles kept, smaller shadows",
            GuardStep::Least => "only the tiles around the rider",
        }
    );
    warn!("{line}");
    if let Some(mut c) = world.get_resource_mut::<ocw_render::console::ConsoleState>() {
        c.print(line);
    }
}

/// When the guard looked last (s of app time).
#[derive(Resource, Debug, Clone, Copy)]
struct GuardClock(f64);

/// How much memory the world may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMode {
    /// The desktop's (as before).
    Normal,
    /// A phone or tablet.
    Phone,
    /// Less: after the page died in a ride.
    Saver,
    /// The least: after the page died in the saver mode too (one ring of
    /// tiles, the smallest shadow map; the page draws at pixel ratio 1).
    Minimal,
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
            "minimal" => Some(Some(Self::Minimal)),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Phone => "phone",
            Self::Saver => "saver",
            Self::Minimal => "minimal",
        }
    }

    /// The largest sun shadow map (`None`: the quality preset's). A
    /// 2048² map takes 16 MB of GPU memory, 1024² 4 MB.
    pub fn shadow_map_cap(self) -> Option<usize> {
        match self {
            Self::Normal => None,
            Self::Phone | Self::Saver => Some(1024),
            Self::Minimal => Some(512),
        }
    }

    /// How many tiles are meshed at once (each holds 14 – 28 MB of heap
    /// until its upload, and the wasm memory keeps its highest mark).
    pub fn tile_pace(self) -> ocw_render::package::TileBuildPace {
        match self {
            Self::Normal => ocw_render::package::TileBuildPace::default(),
            _ => ocw_render::package::TileBuildPace::PHONE,
        }
    }

    /// The rings around the course within which the world's per-tile
    /// parts are kept (`None`: the whole world): one more than the tiles
    /// kept around the rider (the renderer reads their neighbours).
    pub fn course_rings(self, residency: Residency) -> Option<i32> {
        (self != Self::Normal).then_some(residency.keep_radius + 1)
    }

    /// Every how many trees one is drawn (0: all; phones every second, the
    /// least every third).
    pub fn plant_thinning(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Phone | Self::Saver => 2,
            Self::Minimal => 3,
        }
    }

    /// The ride photos' size (`None`: 2560 × 1440).
    pub fn photo_size(self) -> Option<ocw_render::photographer::PhotoSize> {
        match self {
            Self::Normal => None,
            _ => Some(ocw_render::photographer::PhotoSize::PHONE),
        }
    }

    /// The point lights' shadow cube map (`None`: Bevy's 1024², 24 MB of
    /// GPU memory that Bevy allocates although no light of ours casts a
    /// point shadow).
    pub fn point_shadow_px(self) -> Option<usize> {
        match self {
            Self::Normal => None,
            _ => Some(16),
        }
    }

    /// The page's choice, else Saver on iOS / iPadOS (WebKit: the iPhone
    /// of 2026-11-27 rode with shadows only in the saver mode, spec 14
    /// Phase 10, 10G), Phone on another phone or tablet, else Normal.
    pub fn resolve(page: Option<Self>, device: &DeviceInfo) -> Self {
        page.unwrap_or(if device.ios {
            Self::Saver
        } else if device.handheld() {
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
            Self::Phone | Self::Saver | Self::Minimal => Some(1),
        }
    }

    /// The streaming's policy: the page's, made smaller on phones (never
    /// larger than the page asked for).
    pub fn residency(self, page: Residency) -> Residency {
        let (fetch, keep, budget) = match self {
            Self::Normal => return page,
            Self::Phone => (2, 3, 48 << 20),
            Self::Saver => (1, 2, 32 << 20),
            Self::Minimal => (1, 1, 16 << 20),
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
        let android = device(r#"{"mobile": true, "coarse_pointer": true, "max_touch_points": 5}"#);
        let tablet = device(r#"{"coarse_pointer": true, "max_touch_points": 5}"#);
        let pc =
            device(r#"{"mobile": false, "coarse_pointer": false, "cores": 12, "memory_gb": 8}"#);
        assert_eq!(MemoryMode::resolve(None, &android), MemoryMode::Phone);
        assert_eq!(MemoryMode::resolve(None, &tablet), MemoryMode::Phone);
        // iOS / iPadOS (WebKit): the saver mode from the start (10G).
        let iphone = device(
            r#"{"mobile": true, "coarse_pointer": true, "max_touch_points": 5, "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/145.0 Mobile/15E148 Safari/605.1.15"}"#,
        );
        let ipad = device(
            r#"{"coarse_pointer": true, "max_touch_points": 5, "ipad_as_mac": true, "platform": "MacIntel", "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Safari/605.1.15"}"#,
        );
        assert_eq!(MemoryMode::resolve(None, &iphone), MemoryMode::Saver);
        assert_eq!(MemoryMode::resolve(None, &ipad), MemoryMode::Saver);
        assert_eq!(
            MemoryMode::resolve(Some(MemoryMode::Phone), &iphone),
            MemoryMode::Phone
        );
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
        // The minimal mode after a second death: one ring, the least.
        let minimal = MemoryMode::Minimal.residency(page);
        assert_eq!(
            (
                minimal.fetch_radius,
                minimal.keep_radius,
                minimal.budget_bytes
            ),
            (1, 1, 16 << 20)
        );
        // Shadow maps: the desktop's untouched, phones smaller.
        assert_eq!(MemoryMode::Normal.shadow_map_cap(), None);
        assert_eq!(MemoryMode::Normal.point_shadow_px(), None);
        assert_eq!(MemoryMode::Phone.shadow_map_cap(), Some(1024));
        assert_eq!(MemoryMode::Minimal.shadow_map_cap(), Some(512));
        assert_eq!(MemoryMode::Saver.point_shadow_px(), Some(16));
        // Tiles meshed one at a time on phones, as before on desktops.
        use ocw_render::package::TileBuildPace;
        assert_eq!(MemoryMode::Normal.tile_pace(), TileBuildPace::default());
        assert_eq!(MemoryMode::Phone.tile_pace(), TileBuildPace::PHONE);
        // The world's per-tile parts along the course on phones only.
        assert_eq!(MemoryMode::Normal.course_rings(page), None);
        assert_eq!(MemoryMode::Phone.course_rings(phone), Some(4));
        assert_eq!(MemoryMode::Minimal.course_rings(minimal), Some(2));
        // Every second tree on phones, all on desktops.
        assert_eq!(MemoryMode::Normal.plant_thinning(), 0);
        assert_eq!(MemoryMode::Phone.plant_thinning(), 2);
        // Photos at a quarter of the pixels on phones.
        assert_eq!(MemoryMode::Normal.photo_size(), None);
        let p = MemoryMode::Phone.photo_size().expect("a phone's");
        assert_eq!((p.width, p.height), (1280, 720));
    }

    /// The guard steps down near the budget, once per step, and never on
    /// a desktop.
    #[test]
    fn the_guard_steps_down_near_the_budget() {
        assert_eq!(MemoryGuard::for_mode(MemoryMode::Normal), None);
        let mut g = MemoryGuard::for_mode(MemoryMode::Phone).expect("a phone");
        assert_eq!(g.budget, 450 << 20);
        let mb = |m: u64| m << 20;
        assert_eq!(g.check(mb(300)), None);
        assert_eq!(g.check(mb(390)), Some(GuardStep::Shrink));
        assert_eq!(g.check(mb(395)), None, "each step once");
        assert_eq!(g.check(mb(430)), Some(GuardStep::Least));
        assert_eq!(g.check(mb(900)), None);
        assert_eq!(g.check(mb(100)), None, "never back up");
        // Straight to the last step when the memory jumps.
        let mut g = MemoryGuard::for_mode(MemoryMode::Saver).expect("saver");
        assert_eq!(g.check(mb(399)), Some(GuardStep::Least));
        // The residency only shrinks.
        let page = MemoryMode::Phone.residency(Residency::default());
        let r = MemoryGuard::residency(GuardStep::Shrink, page);
        assert_eq!(
            (r.fetch_radius, r.keep_radius, r.budget_bytes),
            (1, 1, 16 << 20)
        );
        let small = Residency {
            fetch_radius: 1,
            keep_radius: 1,
            budget_bytes: 4 << 20,
        };
        assert_eq!(MemoryGuard::residency(GuardStep::Least, small), small);
    }

    #[test]
    fn the_page_option_parses() {
        assert_eq!(MemoryMode::parse("auto"), Some(None));
        assert_eq!(MemoryMode::parse("saver"), Some(Some(MemoryMode::Saver)));
        assert_eq!(MemoryMode::parse("phone"), Some(Some(MemoryMode::Phone)));
        assert_eq!(MemoryMode::parse("normal"), Some(Some(MemoryMode::Normal)));
        assert_eq!(MemoryMode::parse("lots"), None);
        for m in [
            MemoryMode::Normal,
            MemoryMode::Phone,
            MemoryMode::Saver,
            MemoryMode::Minimal,
        ] {
            assert_eq!(MemoryMode::parse(m.name()), Some(Some(m)));
        }
    }
}
