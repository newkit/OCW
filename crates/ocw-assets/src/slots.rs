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
    /// The character's skin tone (spec §14 Phase 4, 4C).
    Skin,
    /// The character's hair colour as drawn (greyed by the age, spec
    /// §14 Phase 4, 4N).
    Hair,
    /// The moustache's and beard's colour (4N N6: the hair's, or own).
    FacialHair,
    /// The face's lines with the age (4N N3): the skin, darker.
    SkinLine,
    /// The lips: the skin, redder and darker (4N, the face).
    Lips,
    /// The kit colours of the jersey designer (spec §14 Phase 8 T6; free):
    /// `KitDesign::{shorts, socks, shoes, gloves}`, the asset's colour
    /// when not chosen.
    Shorts,
    Socks,
    Shoes,
    Gloves,
    /// The middle and outer finger joints (spec §14 Phase 4, 4N, the
    /// hands): the gloves' colour under full and winter gloves, the skin
    /// out of fingerless mitts.
    FingerTip,
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
pub const ALL: [Slot; 87] = [
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
    slot("shorts", Tint::Shorts, 0x141416, 0.0, 0.7),
    slot("skin", Tint::Skin, 0xD9A98C, 0.0, 0.65),
    slot("helmet", Tint::Helmet, 0xF2F2EE, 0.1, 0.4),
    slot("helmet_detail", Tint::Fixed, 0x18181B, 0.0, 0.8),
    slot("shoe", Tint::Shoes, 0xEDEDE8, 0.0, 0.5),
    slot("sock", Tint::Socks, 0xF4F4F0, 0.0, 0.8),
    slot("lens", Tint::Fixed, 0x101418, 0.6, 0.15),
    slot("hair", Tint::Hair, 0x3B2A20, 0.0, 0.9),
    // The character of 4N: moustache and beard, the face's lines.
    slot("facial_hair", Tint::FacialHair, 0x3B2A20, 0.0, 0.95),
    slot("skin_line", Tint::SkinLine, 0xA87C62, 0.0, 0.8),
    // The face (2026-11-17, "give the rider a mouth … and ears, eyes"):
    // the eyes' dark shape and the lips.
    slot("eye", Tint::Fixed, 0x1E1814, 0.0, 0.3),
    slot("lips", Tint::Lips, 0xB5786A, 0.0, 0.55),
    // The kit's variants (spec 4C): aero socks, race shoes, overshoes,
    // gloves and the padded back of gel gloves.
    slot("sock_aero", Tint::Socks, 0x1E1F22, 0.0, 0.75),
    slot("shoe_race", Tint::Shoes, 0x141416, 0.25, 0.3),
    slot("overshoe", Tint::Shoes, 0x0F0F11, 0.05, 0.45),
    slot("glove", Tint::Gloves, 0x18181A, 0.0, 0.7),
    slot("glove_pad", Tint::Fixed, 0x6A6E76, 0.0, 0.8),
    slot("fingertip", Tint::FingerTip, 0x18181A, 0.0, 0.7),
    // Part variants (spec §14 Phase 4, 4B): bare carbon, and the lenses
    // of the light set (the renderer makes them glow).
    slot("carbon", Tint::Fixed, 0x0C0C0E, 0.15, 0.22),
    slot("light_front", Tint::Fixed, 0xFFF6E0, 0.0, 0.2),
    slot("light_rear", Tint::Fixed, 0xE0201C, 0.0, 0.2),
    // Plants (spec §14 Phase 15, 15B): the crown is tinted per plant and
    // by the season (`ocw-render::flora`; the file's colour is the summer
    // default, its vertex colours the shading), bark per species, `dry`
    // parts (dead fronds, stalks) by their vertex colours.
    slot("leaf", Tint::Fixed, 0x3F6B2C, 0.0, 0.85),
    slot("needle", Tint::Fixed, 0x2B4A2E, 0.0, 0.85),
    slot("bark", Tint::Fixed, 0x5A4636, 0.0, 0.9),
    slot("dry", Tint::Fixed, 0xFFFFFF, 0.0, 0.9),
    // The rider status signs (spec §2.6): the X over a muted rider's mouth
    // and the phone's screen (the renderer makes it glow).
    slot("status_mark", Tint::Fixed, 0xD8261E, 0.0, 0.6),
    slot("phone_screen", Tint::Fixed, 0x9EC4F0, 0.0, 0.15),
    // Props in the world (spec §14 Phase 15, 15B / 15H MX): the point
    // obstacles and the street lamps; `bulb` glows from dusk to dawn
    // (`ocw-render::street_lamps`). The files carry their own colours.
    slot("concrete", Tint::Fixed, 0x9E9C94, 0.0, 0.9),
    slot("steel", Tint::Fixed, 0xA8ADB3, 0.6, 0.45),
    slot("wood", Tint::Fixed, 0x735438, 0.0, 0.85),
    slot("paint_red", Tint::Fixed, 0xC71F1A, 0.0, 0.6),
    slot("paint_white", Tint::Fixed, 0xEBEBE6, 0.0, 0.6),
    slot("paint_dark", Tint::Fixed, 0x383B3D, 0.1, 0.6),
    slot("bulb", Tint::Fixed, 0xE1DFDA, 0.0, 0.4),
    // The unicorn (spec §14 Phase 15 M4, `creatures/unicorn-winged.glb`):
    // the renderer draws its own materials per slot (`ocw-render`'s
    // `unicorn::paint_material`); `mane_rider` is the mane's first band in
    // the rider's colour, `rainbow_*` the bands of `unicorn::RAINBOW`.
    slot("coat", Tint::Fixed, 0xF7F5FA, 0.0, 0.55),
    slot("horn", Tint::Fixed, 0xFFC740, 0.85, 0.28),
    slot("muzzle", Tint::Fixed, 0xF5DBE6, 0.0, 0.6),
    slot("feather", Tint::Fixed, 0xFCF7FF, 0.0, 0.45),
    slot("mane_rider", Tint::Fixed, 0xFF1472, 0.0, 0.5),
    slot("rainbow_0", Tint::Fixed, 0xE53935, 0.0, 0.5),
    slot("rainbow_1", Tint::Fixed, 0xFB8C00, 0.0, 0.5),
    slot("rainbow_2", Tint::Fixed, 0xFDD835, 0.0, 0.5),
    slot("rainbow_3", Tint::Fixed, 0x43A047, 0.0, 0.5),
    slot("rainbow_4", Tint::Fixed, 0x00ACC1, 0.0, 0.5),
    slot("rainbow_5", Tint::Fixed, 0x1E88E5, 0.0, 0.5),
    slot("rainbow_6", Tint::Fixed, 0x5E35B1, 0.0, 0.5),
    slot("rainbow_7", Tint::Fixed, 0xD81B60, 0.0, 0.5),
    // The workshop pieces (spec §14 Phase 15 M3, `workshop/*.glb`): never
    // recoloured; the renderer makes `lamp` and `window_glass` glow.
    slot("floor_concrete", Tint::Fixed, 0x8E8B85, 0.0, 0.92),
    slot("concrete_dark", Tint::Fixed, 0x77746F, 0.0, 0.95),
    slot("wall", Tint::Fixed, 0xEAE3D6, 0.0, 0.88),
    slot("wall_accent", Tint::Fixed, 0x5E7A6E, 0.0, 0.85),
    slot("skirting", Tint::Fixed, 0x6B5645, 0.0, 0.7),
    slot("ceiling", Tint::Fixed, 0xF1EEE8, 0.0, 0.9),
    slot("lamp", Tint::Fixed, 0xFFF4DC, 0.0, 0.4),
    slot("workshop_wood", Tint::Fixed, 0xB57E48, 0.0, 0.62),
    slot("wood_dark", Tint::Fixed, 0x5C4030, 0.0, 0.7),
    slot("pegboard", Tint::Fixed, 0xC9A27A, 0.0, 0.8),
    slot("peg_hole", Tint::Fixed, 0x3A2E24, 0.0, 0.9),
    slot("workshop_steel", Tint::Fixed, 0xA2A7AC, 0.85, 0.32),
    slot("steel_dark", Tint::Fixed, 0x2C2E33, 0.4, 0.45),
    slot("rubber", Tint::Fixed, 0x262626, 0.0, 0.9),
    slot("tool_red", Tint::Fixed, 0xC0392B, 0.0, 0.45),
    slot("tool_blue", Tint::Fixed, 0x2E6DB4, 0.0, 0.45),
    slot("tool_yellow", Tint::Fixed, 0xE3B11F, 0.0, 0.5),
    slot("cardboard", Tint::Fixed, 0xB58B5A, 0.0, 0.85),
    slot("bin_blue", Tint::Fixed, 0x3D6F95, 0.0, 0.55),
    slot("bin_grey", Tint::Fixed, 0x8C9196, 0.0, 0.6),
    slot("locker", Tint::Fixed, 0x5B7F95, 0.5, 0.45),
    slot("mirror", Tint::Fixed, 0xDDE3E8, 1.0, 0.04),
    slot("window_glass", Tint::Fixed, 0xCFE4F2, 0.0, 0.08),
    slot("window_frame", Tint::Fixed, 0xF4F4F0, 0.0, 0.5),
    slot("card", Tint::Fixed, 0xF4F1EA, 0.0, 0.7),
    slot("stand", Tint::Fixed, 0x3A3F47, 0.6, 0.38),
    // The VR bike computer (spec §14 Phase 11, v4.68 on `agent/vr-x1`;
    // WebXR 14I W2): its body, the screen window under the live screen,
    // the LED bar (lit by the renderer).
    slot("computer_body", Tint::Fixed, 0x2B2E34, 0.05, 0.55),
    slot("computer_screen", Tint::Fixed, 0x14201C, 0.0, 0.2),
    slot("computer_led", Tint::Fixed, 0x3AD0FF, 0.0, 0.3),
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
        emissive: [0.0; 3],
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
        for t in [
            Tint::Frame,
            Tint::Jersey,
            Tint::Helmet,
            Tint::Accent,
            Tint::Skin,
            Tint::Hair,
            Tint::FacialHair,
            Tint::SkinLine,
            Tint::Lips,
            Tint::Shorts,
            Tint::Socks,
            Tint::Shoes,
            Tint::Gloves,
            Tint::FingerTip,
        ] {
            assert!(ALL.iter().any(|s| s.tint == t), "{t:?}");
        }
        assert_eq!(get("frame").unwrap().tint, Tint::Frame);
        assert!(get("nope").is_none());
        // sRGB to linear: white stays 1, mid grey is ~0.21.
        assert_eq!(srgb(0xFFFFFF), [1.0; 4]);
        assert!((srgb(0x808080)[0] - 0.2158).abs() < 1e-3);
    }
}
