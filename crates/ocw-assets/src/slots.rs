//! Paint slots: the material names the assets use (spec §2.5 "visual part
//! … paint slots"). The app recolours the [`Tint`]ed slots from the rider's
//! look; the others keep the asset's colour.

use crate::model::{Material, srgb};

/// Where a slot's colour comes from at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    /// The asset's own colour.
    Fixed,
    /// `RiderLook::frame`.
    Frame,
    /// `RiderLook::jersey`.
    Jersey,
    /// `RiderLook::helmet`.
    Helmet,
    /// The kit pattern's colour (white on dark jerseys, black on light).
    Accent,
}

/// A paint slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub name: &'static str,
    pub tint: Tint,
    /// Default colour (sRGB `0xRRGGBB`).
    pub color: u32,
    pub metallic: f32,
    pub roughness: f32,
}

const fn slot(name: &'static str, tint: Tint, color: u32, metallic: f32, roughness: f32) -> Slot {
    Slot {
        name,
        tint,
        color,
        metallic,
        roughness,
    }
}

/// All slots.
pub const ALL: [Slot; 22] = [
    slot("frame", Tint::Frame, 0x1E1E22, 0.1, 0.32),
    slot("tyre", Tint::Fixed, 0x1A1A1A, 0.0, 0.85),
    slot("tyre_wall", Tint::Fixed, 0x8A6A45, 0.0, 0.8),
    slot("rim", Tint::Fixed, 0x202124, 0.3, 0.35),
    slot("metal", Tint::Fixed, 0xA8ABB0, 0.9, 0.3),
    slot("component", Tint::Fixed, 0x16161A, 0.2, 0.5),
    slot("bar_tape", Tint::Fixed, 0x111113, 0.0, 0.8),
    slot("decal", Tint::Fixed, 0xF2F2EE, 0.0, 0.4),
    slot("bottle", Tint::Fixed, 0xE8E8E2, 0.0, 0.45),
    slot("jersey", Tint::Jersey, 0xFF1472, 0.0, 0.75),
    slot("accent", Tint::Accent, 0xF5F5F0, 0.0, 0.75),
    slot("shorts", Tint::Fixed, 0x141416, 0.0, 0.7),
    slot("skin", Tint::Fixed, 0xD9A98C, 0.0, 0.65),
    slot("helmet", Tint::Helmet, 0xF2F2EE, 0.1, 0.4),
    slot("helmet_detail", Tint::Fixed, 0x1C1C20, 0.0, 0.6),
    slot("shoe", Tint::Fixed, 0xEDEDE8, 0.0, 0.5),
    slot("sock", Tint::Fixed, 0xF4F4F0, 0.0, 0.8),
    slot("lens", Tint::Fixed, 0x101418, 0.6, 0.15),
    slot("hair", Tint::Fixed, 0x3B2A20, 0.0, 0.9),
    // Part variants (spec §14 Phase 4, 4B): bare carbon, and the lenses
    // of the light set (the renderer makes them glow).
    slot("carbon", Tint::Fixed, 0x0C0C0E, 0.15, 0.22),
    slot("light_front", Tint::Fixed, 0xFFF6E0, 0.0, 0.2),
    slot("light_rear", Tint::Fixed, 0xE0201C, 0.0, 0.2),
];

/// The slot called `name`.
pub fn get(name: &str) -> Option<&'static Slot> {
    ALL.iter().find(|s| s.name == name)
}

/// The asset material of slot `name` (its default colour).
pub fn material(name: &str) -> Material {
    let s = get(name).unwrap_or_else(|| panic!("unknown paint slot {name}"));
    Material {
        name: s.name.into(),
        color: srgb(s.color),
        metallic: s.metallic,
        roughness: s.roughness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_names_are_unique_and_the_look_has_its_slots() {
        for (i, a) in ALL.iter().enumerate() {
            assert!(ALL[i + 1..].iter().all(|b| b.name != a.name), "{}", a.name);
        }
        for t in [Tint::Frame, Tint::Jersey, Tint::Helmet, Tint::Accent] {
            assert!(ALL.iter().any(|s| s.tint == t), "{t:?}");
        }
        assert_eq!(get("frame").unwrap().tint, Tint::Frame);
        assert!(get("nope").is_none());
        // sRGB to linear: white stays 1, mid grey is ~0.21.
        assert_eq!(srgb(0xFFFFFF), [1.0; 4]);
        assert!((srgb(0x808080)[0] - 0.2158).abs() < 1e-3);
    }
}
