//! Headset profiles and the frame budget (spec §14 Phase 11, 11C, Q1).
//!
//! The code stays OpenXR-only and vendor-neutral: a profile only fills in
//! what a runtime may not tell (the panel, the rates it can run) and the
//! render scale we start with. The development headset is the **Oculus
//! Quest (first generation)**: 1440 × 1600 per eye, **72 Hz** (13.9 ms a
//! frame) – over Oculus Link on Windows (X1), later standalone (Phase 14,
//! Snapdragon 835: the same 72 Hz, a lower render scale). Every other
//! headset is postponed (v3.29) and runs on the generic profile with the
//! runtime's numbers.

/// What we know about a headset model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadsetProfile {
    pub id: &'static str,
    pub name: &'static str,
    /// The display's pixels per eye.
    pub panel_px: (u32, u32),
    /// The refresh rates it runs (Hz), lowest first.
    pub refresh_hz: &'static [f32],
    /// The render scale over the runtime's recommended eye size, PC VR.
    pub render_scale_pc: f32,
    /// The same on the headset's own chip (Phase 14 standalone).
    pub render_scale_standalone: f32,
}

/// The Oculus Quest (first generation, 2019).
pub const QUEST_1: HeadsetProfile = HeadsetProfile {
    id: "quest1",
    name: "Oculus Quest (first generation)",
    panel_px: (1440, 1600),
    refresh_hz: &[72.0],
    render_scale_pc: 1.0,
    render_scale_standalone: 0.8,
};

/// The render scale on PC: the runtime's recommended eye size, but not
/// more pixels than the panel has (v4.70). Meta Quest Link recommends
/// 1808 × 2000 for the Quest 1's 1440 × 1600 panel (supersampled 1.25×);
/// at that size Weela drew 36 fps for the 72 Hz headset – every other frame
/// missed, reprojected, seen as flicker. The panel's size is ≈ 0.8 of it.
/// Unknown panels: the profile's scale.
pub fn pc_render_scale(profile: &HeadsetProfile, recommended: (u32, u32)) -> f32 {
    let (pw, ph) = profile.panel_px;
    let (rw, rh) = recommended;
    if pw == 0 || ph == 0 || rw == 0 || rh == 0 {
        return profile.render_scale_pc;
    }
    let to_panel = (pw as f32 / rw as f32).max(ph as f32 / rh as f32);
    profile.render_scale_pc.min(to_panel)
}

/// Any other headset: the runtime's own numbers.
pub const GENERIC: HeadsetProfile = HeadsetProfile {
    id: "generic",
    name: "OpenXR headset",
    panel_px: (0, 0),
    refresh_hz: &[72.0, 80.0, 90.0],
    render_scale_pc: 1.0,
    render_scale_standalone: 0.8,
};

/// The profile for the runtime's system name (`XrSystemProperties
/// .systemName`; Meta's runtime says "Oculus Quest" for the Quest 1 over
/// Link, ALVR / SteamVR forward the headset's name). Unknown names, and
/// the later Quests (postponed), get [`GENERIC`].
pub fn detect(system_name: &str) -> &'static HeadsetProfile {
    let n = system_name.to_ascii_lowercase();
    let later = ["quest 2", "quest 3", "quest pro", "quest2", "quest3"];
    if n.contains("quest") && !later.iter().any(|l| n.contains(l)) {
        &QUEST_1
    } else {
        &GENERIC
    }
}

/// The comfort budget (spec 11B / Q4 note): 72 Hz minimum, 90 Hz is
/// enough for a seated ride.
pub const MIN_HZ: f32 = 72.0;
pub const MAX_HZ: f32 = 90.0;

/// The refresh rate to ask for from those the runtime offers: the highest
/// within 72–90 Hz, else the closest to that range (`None`: nothing
/// offered, keep the runtime's).
pub fn target_rate(offered: &[f32]) -> Option<f32> {
    let ok = offered
        .iter()
        .copied()
        .filter(|r| r.is_finite() && *r > 0.0);
    if let Some(best) = ok
        .clone()
        .filter(|r| (MIN_HZ - 0.5..=MAX_HZ + 0.5).contains(r))
        .reduce(f32::max)
    {
        return Some(best);
    }
    ok.min_by(|a, b| {
        let d = |r: f32| (r - r.clamp(MIN_HZ, MAX_HZ)).abs();
        d(*a).total_cmp(&d(*b))
    })
}

/// The time per frame (ms) at `hz`.
pub fn frame_budget_ms(hz: f32) -> f32 {
    if hz > 0.0 { 1000.0 / hz } else { f32::INFINITY }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quest_1_is_found_and_runs_72_hz() {
        for name in ["Oculus Quest", "Oculus Quest (Link)", "ALVR Oculus Quest"] {
            assert_eq!(detect(name).id, "quest1", "{name}");
        }
        for name in [
            "Oculus Quest2",
            "Meta Quest 3",
            "Valve Index",
            "Monado: Simulated HMD",
            "",
        ] {
            assert_eq!(detect(name).id, "generic", "{name}");
        }
        assert_eq!(QUEST_1.panel_px, (1440, 1600));
        assert_eq!(target_rate(QUEST_1.refresh_hz), Some(72.0));
        assert!((frame_budget_ms(72.0) - 13.89).abs() < 0.01);
    }

    #[test]
    fn rates_stay_within_72_to_90_hz() {
        assert_eq!(target_rate(&[60.0, 72.0, 80.0, 90.0, 120.0]), Some(90.0));
        assert_eq!(target_rate(&[72.0, 120.0]), Some(72.0));
        // Only rates outside: the closest.
        assert_eq!(target_rate(&[60.0, 144.0]), Some(60.0));
        assert_eq!(target_rate(&[120.0, 144.0]), Some(120.0));
        assert_eq!(target_rate(&[]), None);
        assert_eq!(target_rate(&[f32::NAN]), None);
        assert_eq!(frame_budget_ms(0.0), f32::INFINITY);
    }

    /// The Quest 1 over Link: the recommended 1808 × 2000 is rendered at
    /// the panel's 1440 × 1600; a runtime that recommends less keeps its
    /// size; an unknown panel keeps the profile's scale.
    #[test]
    fn the_pc_render_scale_stops_at_the_panel() {
        let s = pc_render_scale(&QUEST_1, (1808, 2000));
        assert!((s - 0.8).abs() < 0.01, "{s}");
        assert_eq!(pc_render_scale(&QUEST_1, (1344, 1504)), 1.0);
        assert_eq!(pc_render_scale(&GENERIC, (2000, 2000)), 1.0);
        assert_eq!(pc_render_scale(&QUEST_1, (0, 0)), 1.0);
    }
}
