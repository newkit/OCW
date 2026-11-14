//! The kit design (spec §2.6, §14 Phase 8 T6): the jersey designer's
//! pattern colours and race number, the free colours of shorts, socks,
//! shoes and gloves, and the helmet and shoe finishes. Free looks like the
//! character – never physics, never hearts – part of [`crate::RiderLook`]
//! and sent like the character, once per change (`RiderBuild.look`).
//!
//! The patterns (plain, chest band, sash, stripes, chevrons, gradient:
//! [`crate::KitPattern`]) are our own and drawn procedurally by the
//! renderer's gear shader (`ocw-render`, `gear.wesl`) from the rider
//! model's rest-pose coordinates; [`pattern_weights`] and [`number_ink`]
//! are their Rust twins (tests, the far LOD's colour). Coordinates: the
//! generated rider (`ocw_assets::rider`) standing in its rest pose, metres,
//! the pelvis at the origin, +Y up, facing −Z, +X its right.

use crate::bike::{KitPattern, Rgb, RiderLook};

/// The highest race number (three digits; 0 = no number).
pub const MAX_RACE_NUMBER: u16 = 999;

/// A surface finish (helmet shell, shoe uppers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Finish {
    /// Glossy: a clear coat over the colour.
    #[default]
    Gloss,
    /// Matte: rough, no clear coat.
    Matte,
}

impl Finish {
    pub const ALL: [Finish; 2] = [Finish::Gloss, Finish::Matte];

    pub fn id(self) -> &'static str {
        match self {
            Self::Gloss => "gloss",
            Self::Matte => "matte",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Gloss => "Gloss",
            Self::Matte => "Matte",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|f| f.id().eq_ignore_ascii_case(s.trim()))
    }

    pub fn wire(self) -> u32 {
        self as u32
    }

    /// Unknown numbers are gloss.
    pub fn from_wire(n: u32) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }
}

/// The kit design of a look (spec §14 Phase 8 T6). The pattern itself
/// and the first (jersey) and helmet colours are [`RiderLook`]'s
/// `pattern`, `jersey` and `helmet`; the default is the plain kit of the
/// app (and of anonymous riders).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KitDesign {
    /// The pattern's second colour (`None`: the automatic contrast,
    /// white on dark jerseys, near-black on light ones – [`contrast`]).
    pub second: Option<Rgb>,
    /// The third colour (`None`: a two-colour design).
    pub third: Option<Rgb>,
    /// Race number on the jersey's back and both hips of the shorts
    /// (0: none; 1 … [`MAX_RACE_NUMBER`]).
    pub number: u16,
    /// The pattern on the helmet too: the shell in the jersey colours.
    pub helmet_pattern: bool,
    /// Kit colours (`None`: the model's own colours: black shorts and
    /// gloves, white socks and shoes).
    pub shorts: Option<Rgb>,
    pub socks: Option<Rgb>,
    pub shoes: Option<Rgb>,
    pub gloves: Option<Rgb>,
    pub helmet_finish: Finish,
    pub shoe_finish: Finish,
}

impl KitDesign {
    /// Values from the network or a file, made valid: numbers past
    /// [`MAX_RACE_NUMBER`] are none, colours 24-bit.
    pub fn validated(self) -> Self {
        let c = |v: Option<Rgb>| v.map(|c| Rgb::from_wire(c.0));
        Self {
            second: c(self.second),
            third: c(self.third),
            number: if self.number > MAX_RACE_NUMBER {
                0
            } else {
                self.number
            },
            shorts: c(self.shorts),
            socks: c(self.socks),
            shoes: c(self.shoes),
            gloves: c(self.gloves),
            ..self
        }
    }

    /// "second #FFFFFF, third #1E88E5, number 42 (helmet too); shorts
    /// #1E1E22, gloss helmet, matte shoes" – the parts that differ from
    /// the default.
    pub fn summary(&self) -> String {
        let d = self.validated();
        let mut parts = Vec::new();
        let opt = |c: Option<Rgb>| c.map_or_else(|| "auto".to_string(), |c| c.to_string());
        parts.push(format!("second {}", opt(d.second)));
        if let Some(t) = d.third {
            parts.push(format!("third {t}"));
        }
        if d.number > 0 {
            parts.push(format!("number {}", d.number));
        }
        if d.helmet_pattern {
            parts.push("pattern on the helmet".into());
        }
        for (what, c) in [
            ("shorts", d.shorts),
            ("socks", d.socks),
            ("shoes", d.shoes),
            ("gloves", d.gloves),
        ] {
            if let Some(c) = c {
                parts.push(format!("{what} {c}"));
            }
        }
        parts.push(format!(
            "{} helmet, {} shoes",
            d.helmet_finish.id(),
            d.shoe_finish.id()
        ));
        parts.join(", ")
    }
}

/// The automatic second colour of a jersey: near-black on light colours,
/// off-white on dark ones (the accent of the plain kit).
pub fn contrast(c: Rgb) -> Rgb {
    if luminance(c) > 0.55 {
        Rgb(0x0F0F12)
    } else {
        Rgb(0xF5F5F0)
    }
}

/// Relative luminance of an sRGB colour (0–1, on the gamma-encoded
/// channels: what reads as light or dark).
pub fn luminance(c: Rgb) -> f32 {
    let [r, g, b] = c.channels().map(|v| f32::from(v) / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

impl RiderLook {
    /// The design's colours: first (the jersey), second (or the
    /// automatic contrast), third (or the second).
    pub fn kit_colours(&self) -> [Rgb; 3] {
        let second = self.design.second.unwrap_or_else(|| contrast(self.jersey));
        [self.jersey, second, self.design.third.unwrap_or(second)]
    }
}

/// The garments a pattern is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Garment {
    /// Jersey, sleeves and skinsuit: the pattern.
    Jersey,
    /// Shorts and tights: their own colour with side panels in the
    /// jersey colours (non-plain patterns) and the hip numbers.
    Shorts,
    /// The helmet shell: a centre stripe when the pattern is on it.
    Helmet,
}

impl Garment {
    pub fn wire(self) -> u32 {
        self as u32
    }
}

/// The body part a mesh hangs on (the rider rig's nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyRegion {
    /// Pelvis and torso.
    Torso,
    Arm,
    Leg,
    Head,
    Hand,
    Foot,
}

impl BodyRegion {
    pub fn code(self) -> u32 {
        self as u32
    }
}

/// Where the patterns sit (rest pose, m): the chest band's lower and
/// upper edge, the sash's centre and angle, the stripes' width, the
/// chevrons' first edge and height, the gradient's bottom and top.
pub mod layout {
    pub const BAND: [f32; 2] = [0.24, 0.34];
    /// Width of the third colour's pinstripes along the edges.
    pub const PIN: f32 = 0.014;
    pub const SASH_CENTRE: [f32; 2] = [0.0, 0.29];
    /// Rad from the horizontal (from the right shoulder to the left hip).
    pub const SASH_ANGLE: f32 = 0.62;
    pub const SASH_HALF: f32 = 0.042;
    pub const STRIPE: f32 = 0.045;
    pub const CHEVRON_FROM: f32 = 0.16;
    pub const CHEVRON: f32 = 0.07;
    pub const CHEVRON_SLOPE: f32 = 0.9;
    pub const GRADIENT: [f32; 2] = [0.02, 0.5];
    /// Shorts: the side panel from this |x| outwards.
    pub const SIDE_PANEL: f32 = 0.138;
    /// Helmet: the centre stripe's half width.
    pub const HELMET_STRIPE: f32 = 0.02;
    /// The race number on the back: the paper's centre, half size, the
    /// digits' height, and the depth from which a point is on the back.
    pub const BIB_CENTRE: [f32; 2] = [0.0, 0.29];
    pub const BIB_HALF: [f32; 2] = [0.1, 0.075];
    pub const BIB_DIGIT: f32 = 0.075;
    pub const BIB_BACK_Z: f32 = 0.07;
    /// The hip numbers: paper plates on the pelvis' sides (the rider
    /// model's `hip_number_*` nodes, outside every thigh at any pedal
    /// angle: they stay put and readable while riding). |x| from which a
    /// point is on a plate, the digits' centre (z, y) and height.
    pub const HIP_X: f32 = 0.174;
    pub const HIP_CENTRE: [f32; 2] = [0.0, -0.035];
    pub const HIP_DIGIT: f32 = 0.036;
}

/// Weights of the colours `[base, first, second, third]` of a garment's
/// pattern at the rest-pose point `p` (they add up to 1). `base` is the
/// garment's own colour: the jersey's first colour, the shorts' colour,
/// the helmet's colour (or the first colour with the pattern on it).
/// Hard edges here; the shader anti-aliases them.
pub fn pattern_weights(
    pattern: KitPattern,
    three: bool,
    garment: Garment,
    region: BodyRegion,
    p: [f32; 3],
) -> [f32; 4] {
    use layout::*;
    let [x, y, z] = p;
    let one = |i: usize| {
        let mut w = [0.0; 4];
        w[i] = 1.0;
        w
    };
    // Second colour, or the third along the edges of a band.
    let banded = |d: f32, half: f32| {
        if d.abs() < half {
            one(2)
        } else if three && d.abs() < half + PIN {
            one(3)
        } else {
            one(0)
        }
    };
    match garment {
        Garment::Jersey => {
            if region != BodyRegion::Torso && pattern != KitPattern::Gradient {
                // Sleeves in the main colour.
                return one(0);
            }
            match pattern {
                KitPattern::Plain => one(0),
                KitPattern::Band => {
                    banded(y - 0.5 * (BAND[0] + BAND[1]), 0.5 * (BAND[1] - BAND[0]))
                }
                KitPattern::Sash => {
                    let (s, c) = SASH_ANGLE.sin_cos();
                    let d = -(x - SASH_CENTRE[0]) * s + (y - SASH_CENTRE[1]) * c;
                    banded(d, SASH_HALF)
                }
                KitPattern::Stripes => {
                    let k = (x / STRIPE + 0.5).floor() as i32;
                    let n = if three { 3 } else { 2 };
                    match k.rem_euclid(n) {
                        0 => one(0),
                        1 => one(2),
                        _ => one(3),
                    }
                }
                KitPattern::Chevrons => {
                    let v = y - CHEVRON_SLOPE * x.abs() - CHEVRON_FROM;
                    if v < 0.0 {
                        return one(0);
                    }
                    match (v / CHEVRON).floor() as i32 {
                        0 | 2 => one(2),
                        1 if three => one(3),
                        _ => one(0),
                    }
                }
                KitPattern::Gradient => {
                    gradient((y - GRADIENT[0]) / (GRADIENT[1] - GRADIENT[0]), three)
                }
            }
        }
        Garment::Shorts => {
            if pattern == KitPattern::Plain
                || !matches!(region, BodyRegion::Torso | BodyRegion::Leg)
            {
                return one(0);
            }
            let side = x.abs();
            if side > SIDE_PANEL {
                one(1)
            } else if side > SIDE_PANEL - PIN * 0.5 {
                one(if three { 3 } else { 2 })
            } else {
                one(0)
            }
        }
        Garment::Helmet => match pattern {
            KitPattern::Plain => one(0),
            KitPattern::Gradient => {
                // Front (−z) to back.
                let w = gradient((z + 0.12) / 0.26, three);
                [0.0, w[0] + w[1], w[2], w[3]]
            }
            _ => {
                if x.abs() < HELMET_STRIPE {
                    one(2)
                } else if three && x.abs() < HELMET_STRIPE + PIN * 0.6 {
                    one(3)
                } else {
                    one(1)
                }
            }
        },
    }
}

/// Weights of a vertical blend over `t` (0 bottom … 1 top): first to
/// second, through the third in the middle for three colours.
fn gradient(t: f32, three: bool) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    if three {
        if t < 0.5 {
            let u = t * 2.0;
            [0.0, 1.0 - u, 0.0, u]
        } else {
            let u = t * 2.0 - 1.0;
            [0.0, 0.0, u, 1.0 - u]
        }
    } else {
        [0.0, 1.0 - t, t, 0.0]
    }
}

/// What the race number puts on a point: nothing, the paper of the
/// bib, or a digit's ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    None,
    Paper,
    Digit,
}

/// The digits 0–9 as 3 × 5 pixel glyphs: bit `row * 3 + col` (row 0 at
/// the top, col 0 on the left). The same table is in `gear.wesl`.
pub const GLYPHS: [u16; 10] = [
    glyph(["###", "#.#", "#.#", "#.#", "###"]),
    glyph([".#.", "##.", ".#.", ".#.", "###"]),
    glyph(["###", "..#", "###", "#..", "###"]),
    glyph(["###", "..#", "###", "..#", "###"]),
    glyph(["#.#", "#.#", "###", "..#", "..#"]),
    glyph(["###", "#..", "###", "..#", "###"]),
    glyph(["###", "#..", "###", "#.#", "###"]),
    glyph(["###", "..#", ".#.", ".#.", ".#."]),
    glyph(["###", "#.#", "###", "#.#", "###"]),
    glyph(["###", "#.#", "###", "..#", "###"]),
];

const fn glyph(rows: [&str; 5]) -> u16 {
    let mut bits = 0u16;
    let mut r = 0;
    while r < 5 {
        let b = rows[r].as_bytes();
        let mut c = 0;
        while c < 3 {
            if b[c] == b'#' {
                bits |= 1 << (r * 3 + c);
            }
            c += 1;
        }
        r += 1;
    }
    bits
}

/// Whether the number `n` (1–999) has ink at `(u, v)` in a line of
/// digits `h` high centred at the origin (u to the right, v up): 3 × 5
/// pixel glyphs one pixel apart.
pub fn digits_ink(n: u16, u: f32, v: f32, h: f32) -> bool {
    if n == 0 || n > MAX_RACE_NUMBER {
        return false;
    }
    let px = h / 5.0;
    let count = if n >= 100 {
        3
    } else if n >= 10 {
        2
    } else {
        1
    };
    let width = (4 * count - 1) as f32 * px;
    let gx = (u + 0.5 * width) / px;
    let gy = (0.5 * h - v) / px;
    if gx < 0.0 || gy < 0.0 || gx >= width / px || gy >= 5.0 {
        return false;
    }
    let cell = (gx / 4.0).floor() as u32;
    let col = gx.floor() as u32 - cell * 4;
    if col >= 3 {
        return false;
    }
    let digit = (n / 10u16.pow(count - 1 - cell)) % 10;
    let row = gy.floor() as u32;
    GLYPHS[digit as usize] & (1 << (row * 3 + col)) != 0
}

/// The race number's ink at the rest-pose point `p` of a garment: the
/// paper bib with its digits on the jersey's back, the paper plates on
/// both hips (on the pelvis, not the thighs: read from the side, front to
/// back on the left hip, back to front on the right).
pub fn number_ink(number: u16, garment: Garment, region: BodyRegion, p: [f32; 3]) -> Ink {
    use layout::*;
    if number == 0 || number > MAX_RACE_NUMBER {
        return Ink::None;
    }
    let [x, y, z] = p;
    match (garment, region) {
        (Garment::Jersey, BodyRegion::Torso) => {
            let (u, v) = (x - BIB_CENTRE[0], y - BIB_CENTRE[1]);
            if z < BIB_BACK_Z || u.abs() > BIB_HALF[0] || v.abs() > BIB_HALF[1] {
                return Ink::None;
            }
            if digits_ink(number, u, v, BIB_DIGIT) {
                Ink::Digit
            } else {
                Ink::Paper
            }
        }
        // Only the pelvis carries them: on a thigh they turned with the
        // pedal stroke (unreadable).
        (Garment::Shorts, BodyRegion::Torso) => {
            if x.abs() < HIP_X {
                return Ink::None;
            }
            let u = if x < 0.0 { z } else { -z } - HIP_CENTRE[0];
            if digits_ink(number, u, y - HIP_CENTRE[1], HIP_DIGIT) {
                Ink::Digit
            } else {
                Ink::Paper
            }
        }
        _ => Ink::None,
    }
}

/// The colour that covers most of the jersey (the far LOD's jersey,
/// which has no pattern): the pattern sampled on a cylinder around the
/// torso. Deterministic.
pub fn dominant_jersey_colour(look: &RiderLook) -> Rgb {
    let colours = look.kit_colours();
    let three = look.design.third.is_some();
    let mut area = [0.0f32; 3];
    for j in 0..24 {
        let y = 0.0 + 0.45 * (j as f32 + 0.5) / 24.0;
        for i in 0..32 {
            let a = std::f32::consts::TAU * (i as f32 + 0.5) / 32.0;
            let p = [0.17 * a.sin(), y, 0.03 - 0.105 * a.cos()];
            let w = pattern_weights(look.pattern, three, Garment::Jersey, BodyRegion::Torso, p);
            area[0] += w[0] + w[1];
            area[1] += w[2];
            area[2] += w[3];
        }
    }
    let mut best = 0;
    for i in 1..3 {
        if area[i] > area[best] + 1e-3 {
            best = i;
        }
    }
    colours[best]
}

/// Area shares of the four colours over the torso's cylinder (tests).
pub fn torso_shares(pattern: KitPattern, three: bool) -> [f32; 4] {
    let mut s = [0.0f32; 4];
    let n = 24 * 32;
    for j in 0..24 {
        let y = 0.45 * (j as f32 + 0.5) / 24.0;
        for i in 0..32 {
            let a = std::f32::consts::TAU * (i as f32 + 0.5) / 32.0;
            let p = [0.17 * a.sin(), y, 0.03 - 0.105 * a.cos()];
            let w = pattern_weights(pattern, three, Garment::Jersey, BodyRegion::Torso, p);
            for k in 0..4 {
                s[k] += w[k] / n as f32;
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finishes_and_validation() {
        for f in Finish::ALL {
            assert_eq!(Finish::parse(f.id()), Some(f));
            assert_eq!(Finish::from_wire(f.wire()), f);
        }
        assert_eq!(Finish::from_wire(7), Finish::Gloss);
        let d = KitDesign {
            number: 1000,
            second: Some(Rgb(0xFF12_3456)),
            ..Default::default()
        }
        .validated();
        assert_eq!(d.number, 0);
        assert_eq!(d.second, Some(Rgb(0x123456)));
        assert_eq!(KitDesign::default().validated(), KitDesign::default());
        assert!(KitDesign::default().summary().contains("second auto"));
    }

    #[test]
    fn colours_of_a_design() {
        let look = RiderLook::default();
        // Pink is not light: white is its automatic second colour.
        assert_eq!(
            look.kit_colours(),
            [Rgb(0xFF1472), Rgb(0xF5F5F0), Rgb(0xF5F5F0)]
        );
        let light = RiderLook {
            jersey: Rgb(0xFDD835),
            ..look
        };
        assert_eq!(light.kit_colours()[1], Rgb(0x0F0F12));
        let three = RiderLook {
            design: KitDesign {
                second: Some(Rgb(0x1E88E5)),
                third: Some(Rgb(0xF2F2EE)),
                ..Default::default()
            },
            ..look
        };
        assert_eq!(
            three.kit_colours(),
            [Rgb(0xFF1472), Rgb(0x1E88E5), Rgb(0xF2F2EE)]
        );
    }

    /// Pattern generation is deterministic and each pattern has its
    /// own layout: same input, same weights; plain is one colour; the
    /// others have the second colour on 10–60 % of the torso, the third
    /// only when chosen.
    #[test]
    fn patterns_are_deterministic_and_distinct() {
        let mut seen = Vec::new();
        for p in KitPattern::ALL {
            for three in [false, true] {
                let a = torso_shares(p, three);
                assert_eq!(a, torso_shares(p, three), "{p:?}: deterministic");
                let sum: f32 = a.iter().sum();
                assert!((sum - 1.0).abs() < 1e-3, "{p:?} {a:?}");
                if p == KitPattern::Plain {
                    assert!((a[0] - 1.0).abs() < 1e-4 && a[1..].iter().all(|w| *w == 0.0));
                    continue;
                }
                assert!((0.08..0.65).contains(&a[2]), "{p:?} {three} {a:?}");
                assert_eq!(a[3] > 0.0, three, "{p:?} {a:?}");
                seen.push(a);
            }
        }
        // All non-plain layouts differ.
        for (i, a) in seen.iter().enumerate() {
            for b in &seen[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Spot checks: the band across the chest, the stripes' centre
        // stripe in the first colour, the chevron points down the middle.
        let at = |p: KitPattern, q: [f32; 3]| {
            pattern_weights(p, false, Garment::Jersey, BodyRegion::Torso, q)
        };
        assert_eq!(
            at(KitPattern::Band, [0.0, 0.29, -0.09]),
            [0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(
            at(KitPattern::Band, [0.0, 0.1, -0.09]),
            [1.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            at(KitPattern::Stripes, [0.0, 0.2, -0.09]),
            [1.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            at(KitPattern::Stripes, [0.045, 0.2, -0.09]),
            [0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(
            at(KitPattern::Chevrons, [0.0, 0.18, -0.09]),
            [0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(
            at(KitPattern::Chevrons, [0.15, 0.18, -0.09]),
            [1.0, 0.0, 0.0, 0.0]
        );
        let g = at(KitPattern::Gradient, [0.0, 0.0, -0.09]);
        assert_eq!(g, [0.0, 1.0, 0.0, 0.0], "first colour at the waist");
        let g = at(KitPattern::Gradient, [0.0, 0.6, -0.09]);
        assert_eq!(g, [0.0, 0.0, 1.0, 0.0], "second at the shoulders");
        // Sleeves keep the main colour except in the gradient.
        assert_eq!(
            pattern_weights(
                KitPattern::Stripes,
                true,
                Garment::Jersey,
                BodyRegion::Arm,
                [0.2, 0.3, 0.0]
            ),
            [1.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn shorts_and_helmet_carry_the_design() {
        let s = |p: KitPattern, x: f32| {
            pattern_weights(p, false, Garment::Shorts, BodyRegion::Leg, [x, -0.1, 0.0])
        };
        assert_eq!(s(KitPattern::Plain, -0.16), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            s(KitPattern::Sash, -0.16),
            [0.0, 1.0, 0.0, 0.0],
            "side panel"
        );
        assert_eq!(s(KitPattern::Sash, -0.05), [1.0, 0.0, 0.0, 0.0]);
        let h = |p: KitPattern, x: f32| {
            pattern_weights(p, true, Garment::Helmet, BodyRegion::Head, [x, 0.8, 0.0])
        };
        assert_eq!(
            h(KitPattern::Band, 0.0),
            [0.0, 0.0, 1.0, 0.0],
            "centre stripe"
        );
        assert_eq!(h(KitPattern::Band, 0.025), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(h(KitPattern::Band, 0.06), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(h(KitPattern::Plain, 0.0), [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn race_numbers_are_drawn_on_the_back_and_the_hips() {
        // The glyphs: "1" has its stem in the middle column.
        assert_eq!(GLYPHS[1] & (1 << 1), 1 << 1);
        assert_eq!(GLYPHS[8].count_ones(), 13);
        // "7": ink in the top row, nothing in the bottom-left pixel.
        let h = layout::BIB_DIGIT;
        let px = h / 5.0;
        assert!(digits_ink(7, -px, 0.5 * h - 0.5 * px, h));
        assert!(!digits_ink(7, -px, -0.5 * h + 0.5 * px, h));
        assert!(!digits_ink(0, 0.0, 0.0, h), "no number");
        // Three digits are wider than one.
        assert!(digits_ink(123, -4.5 * px, 0.0, h));
        assert!(!digits_ink(1, -4.5 * px, 0.0, h));
        let back = |n: u16, x: f32, y: f32| {
            number_ink(n, Garment::Jersey, BodyRegion::Torso, [x, y, 0.13])
        };
        assert_eq!(back(42, 0.09, 0.36), Ink::Paper, "corner of the bib");
        assert_eq!(back(42, 0.0, 0.1), Ink::None, "below the bib");
        assert_eq!(back(0, 0.0, 0.29), Ink::None);
        assert_eq!(
            number_ink(42, Garment::Jersey, BodyRegion::Torso, [0.0, 0.29, -0.09]),
            Ink::None,
            "not on the chest"
        );
        let ink: usize = (0..40)
            .flat_map(|i| (0..30).map(move |j| (i, j)))
            .filter(|&(i, j)| {
                back(42, -0.1 + 0.005 * i as f32, 0.215 + 0.005 * j as f32) == Ink::Digit
            })
            .count();
        assert!(ink > 100, "{ink}");
        // Hips: paper plates on the pelvis with the digits, read from the
        // side.
        let y = layout::HIP_CENTRE[1];
        let hip = |x: f32, z: f32| number_ink(8, Garment::Shorts, BodyRegion::Torso, [x, y, z]);
        assert_eq!(hip(-0.178, 0.0), Ink::Digit, "middle bar of the 8");
        assert_eq!(hip(0.178, 0.0), Ink::Digit);
        assert_eq!(hip(0.178, 0.045), Ink::Paper, "the plate beside the digit");
        assert_eq!(hip(0.05, 0.0), Ink::None, "not on the front");
        // Regression (spec §14 Phase 8 T6): never on the thighs, which
        // turn with the pedals.
        assert_eq!(
            number_ink(8, Garment::Shorts, BodyRegion::Leg, [0.178, y, 0.0]),
            Ink::None
        );
        assert_eq!(
            number_ink(8, Garment::Shorts, BodyRegion::Leg, [-0.15, -0.13, 0.0]),
            Ink::None
        );
    }

    #[test]
    fn the_far_colour_is_the_dominant_one() {
        let look = |pattern, second| RiderLook {
            jersey: Rgb(0xE53935),
            pattern,
            design: KitDesign {
                second,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            dominant_jersey_colour(&look(KitPattern::Plain, None)),
            Rgb(0xE53935)
        );
        assert_eq!(
            dominant_jersey_colour(&look(KitPattern::Band, Some(Rgb(0x1E88E5)))),
            Rgb(0xE53935)
        );
        let l = look(KitPattern::Stripes, Some(Rgb(0x1E88E5)));
        assert_eq!(dominant_jersey_colour(&l), dominant_jersey_colour(&l));
    }
}
